// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Looking the finished file over before it is posted.
//!
//! Everything else in this crate makes a decision and trusts it. The camera
//! decides where the head is, the captions decide where the phrase breaks,
//! the cutaways decide when to look away, and then the export writes a file
//! and nobody checks it. The person watching finds the chopped forehead, and
//! by then it is a re-render.
//!
//! This reads the written file back and says what is wrong with it. Not what
//! is *good* about it, which is a matter of taste and no business of a
//! program: what is measurably wrong. A head that leaves the frame, a face
//! smaller than a thumb, a picture that goes black, sound too quiet for a
//! feed that normalises everything to the same loudness, an opening half
//! second with nothing in it.
//!
//! Every check here is arithmetic on the pixels and the samples. No model is
//! asked whether the clip is any good, no service is called, nothing leaves
//! the machine. That is a deliberate limit and not a shortcut: the faults
//! listed below are the ones that have actually shipped, and all of them are
//! numbers. Taste stays with the person.
//!
//! The reading is split the way [`super::gaps`] and [`super::punch`] are
//! split: the rules are functions over plain slices, which is what the tests
//! exercise, and [`look_over`] is the one place that opens a file.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use concat_core::{FrameRate, Rational};
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_vision::face::Detector;
use concat_vision::reframe::Face;

/// How many times a second the picture is read back.
///
/// Three. A fault worth reporting lasts longer than a third of a second by
/// definition, because anything shorter is a flicker nobody can act on, and
/// reading a one minute clip is then a hundred and eighty inferences.
pub const LOOK_RATE: u32 = 3;

/// The shortest run of bad frames worth a note, in seconds.
///
/// Under this it is one frame the detector lost, not a fault in the edit.
/// Reporting those would bury the real notes in noise, which is the only way
/// a check like this fails: by being ignored.
pub const SPELL: f64 = 0.6;

/// The loudness feeds normalise to, in LUFS.
///
/// TikTok, Reels and Shorts all turn everything down to roughly this, so a
/// file mastered louder is quietly turned down and only sounds squashed,
/// and a file mastered quieter is left quiet next to everything around it.
pub const TARGET_LOUDNESS: f64 = -14.0;

/// How far from [`TARGET_LOUDNESS`] is close enough.
pub const LOUDNESS_SLACK: f64 = 2.0;

/// The sample value counted as full scale.
pub const CLIPPED: f32 = 0.999;

/// How many full scale samples in a row make it clipping rather than one
/// loud transient landing exactly on the ceiling.
pub const CLIPPED_RUN: usize = 4;

/// How much of the opening has to carry sound, in seconds.
///
/// The first moment is the only one everybody sees. Opening on a breath
/// gives the thumb somewhere to go.
pub const DEAD_OPEN: f64 = 0.4;

/// The smallest a face may be, as a fraction of the frame's height.
///
/// A tenth of a tall frame is about 190 pixels at 1920, which is a face
/// that reads on a phone. Below it the reframe has cropped a wide shot
/// without ever going in.
pub const FACE_LEAST: f64 = 0.10;

/// How far past the frame's edge a face may sit before it counts as cut.
///
/// Two hundredths. An ear at the edge is a shot; a chin off the bottom is a
/// mistake, and the difference between them is about this much.
pub const EDGE: f64 = 0.02;

/// Where a head should sit vertically, as fractions of the frame.
///
/// Eyes near the upper third is the ordinary portrait rule and the reframe
/// aims for it. A centre outside this band means the camera settled on
/// somebody's chest or on the ceiling above them.
pub const BAND: (f64, f64) = (0.16, 0.66);

/// How fast the subject may drift, in frame widths a second, before the
/// shot reads as unsteady.
///
/// A twelfth of the frame a second is a slow pan. Faster than that in a
/// clip that is supposed to be locked off is the camera hunting, which is
/// the complaint that started this module.
pub const WANDER: f64 = 0.08;

/// Luma, zero to one, under which a frame counts as black.
pub const DARK: f64 = 0.06;

/// Mean change between frames, zero to one, under which the picture counts
/// as frozen.
pub const STILL: f64 = 0.0015;

/// The lengths that hold, in seconds.
///
/// Under the first the hook has no room to land; over the second the
/// retention curve has usually already gone. Both ends are a note to look,
/// never a fault: a good long clip beats a bad short one.
pub const SWEET: (f64, f64) = (12.0, 75.0);

/// How severely a note wants attention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    /// It will show. Worth another pass before posting.
    Stop,
    /// Worth a look, but a judgement rather than a fault.
    Look,
}

/// What was found.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// The file is not the shape a feed wants, width over height.
    NotTall { aspect: f64 },
    /// Shorter than [`SWEET`].
    Short { seconds: f64 },
    /// Longer than [`SWEET`].
    Long { seconds: f64 },
    /// No face found at all through this stretch.
    NoFace,
    /// A face sitting past the frame's edge by more than [`EDGE`].
    FaceCut,
    /// A face smaller than [`FACE_LEAST`], given as the height found.
    FaceSmall { height: f64 },
    /// The subject's centre above [`BAND`], given as the height found.
    FaceHigh { at: f64 },
    /// And below it.
    FaceLow { at: f64 },
    /// The subject moving faster than [`WANDER`], in frame widths a second.
    Wander { per_second: f64 },
    /// Frames under [`DARK`].
    Dark,
    /// Frames that do not change.
    Frozen,
    /// Quieter than [`TARGET_LOUDNESS`] by more than the slack.
    Quiet { lufs: f64 },
    /// And louder.
    Loud { lufs: f64 },
    /// Samples pinned at full scale.
    Clipped,
    /// The opening with nothing in it, in seconds.
    DeadOpen { seconds: f64 },
}

/// One thing found, and where.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Note {
    /// Seconds into the file where it starts.
    pub at: f64,
    /// And ends. The same as `at` for a note about the whole file.
    pub until: f64,
    /// How much it matters.
    pub weight: Weight,
    /// What it is.
    pub kind: Kind,
}

impl Note {
    /// A note about the file as a whole rather than a moment in it.
    fn whole(weight: Weight, kind: Kind) -> Note {
        Note {
            at: 0.0,
            until: 0.0,
            weight,
            kind,
        }
    }
}

/// The stretches where `bad` holds for at least [`SPELL`], in seconds.
///
/// `rate` is how many entries there are to a second.
fn spells(bad: &[bool], rate: f64) -> Vec<(f64, f64)> {
    if rate <= 0.0 {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut from: Option<usize> = None;
    for (i, wrong) in bad.iter().chain(std::iter::once(&false)).enumerate() {
        match (from, wrong) {
            (None, true) => from = Some(i),
            (Some(start), false) => {
                let (at, until) = (start as f64 / rate, i as f64 / rate);
                if until - at >= SPELL {
                    found.push((at, until));
                }
                from = None;
            }
            _ => {}
        }
    }
    found
}

/// The middle value of a set. Zero for an empty one.
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    match values.len() {
        0 => 0.0,
        n if n % 2 == 1 => values[n / 2],
        n => f64::midpoint(values[n / 2 - 1], values[n / 2]),
    }
}

/// The biggest face in a frame, which is the one the shot is about.
fn subject(faces: &[Face]) -> Option<&Face> {
    faces.iter().max_by(|a, b| a.area().total_cmp(&b.area()))
}

/// What the file's shape and length say.
pub fn shape_notes(width: u32, height: u32, seconds: f64) -> Vec<Note> {
    let mut notes = Vec::new();
    if width > 0 && height > 0 {
        let aspect = f64::from(width) / f64::from(height);
        // Nine by sixteen is 0.5625. A whole percent of slack covers the odd
        // 1082 a scaler leaves behind without letting a square through.
        if (aspect - 0.5625).abs() > 0.01 {
            notes.push(Note::whole(Weight::Stop, Kind::NotTall { aspect }));
        }
    }
    if seconds > 0.0 && seconds < SWEET.0 {
        notes.push(Note::whole(Weight::Look, Kind::Short { seconds }));
    }
    if seconds > SWEET.1 {
        notes.push(Note::whole(Weight::Look, Kind::Long { seconds }));
    }
    notes
}

/// What the faces found through the file say about the framing.
///
/// `seen` is every face in every frame read, `rate` how many of those
/// frames there are to a second. The coordinates are the detector's:
/// fractions of the frame, which here is the exported frame, so a box
/// crossing zero or one is a head genuinely leaving the picture rather than
/// one leaving the crop.
pub fn face_notes(seen: &[Vec<Face>], rate: f64) -> Vec<Note> {
    if seen.is_empty() || rate <= 0.0 {
        return Vec::new();
    }
    let mut notes = Vec::new();

    let none: Vec<bool> = seen.iter().map(Vec::is_empty).collect();
    // A clip with no face anywhere is a cutaway or a screen recording, not
    // a badly framed talking head, and saying so every third of a second
    // would be worse than saying nothing.
    if !none.iter().all(|empty| *empty) {
        for (at, until) in spells(&none, rate) {
            notes.push(Note {
                at,
                until,
                weight: Weight::Stop,
                kind: Kind::NoFace,
            });
        }
    }

    let cut: Vec<bool> = seen
        .iter()
        .map(|faces| {
            subject(faces).is_some_and(|f| {
                f.x < -EDGE || f.y < -EDGE || f.x + f.w > 1.0 + EDGE || f.y + f.h > 1.0 + EDGE
            })
        })
        .collect();
    for (at, until) in spells(&cut, rate) {
        notes.push(Note {
            at,
            until,
            weight: Weight::Stop,
            kind: Kind::FaceCut,
        });
    }

    // Size and placing are judged on the middle of the clip rather than
    // frame by frame: a head turning away shrinks its own box for a moment
    // and that is not a framing fault.
    let heights: Vec<f64> = seen
        .iter()
        .filter_map(|f| subject(f))
        .map(|f| f.h)
        .collect();
    let middles: Vec<f64> = seen
        .iter()
        .filter_map(|f| subject(f))
        .map(|f| f.centre().1)
        .collect();
    if !heights.is_empty() {
        let height = median(heights);
        if height < FACE_LEAST {
            notes.push(Note::whole(Weight::Stop, Kind::FaceSmall { height }));
        }
        let at = median(middles);
        if at < BAND.0 {
            notes.push(Note::whole(Weight::Stop, Kind::FaceHigh { at }));
        } else if at > BAND.1 {
            notes.push(Note::whole(Weight::Stop, Kind::FaceLow { at }));
        }
    }

    // Drift, measured between one reading and the next and then taken at
    // the middle, so a genuine cut to another shot counts once and does not
    // decide the answer.
    let centres: Vec<Option<(f64, f64)>> = seen
        .iter()
        .map(|faces| subject(faces).map(Face::centre))
        .collect();
    let steps: Vec<f64> = centres
        .windows(2)
        .filter_map(|pair| match (pair[0], pair[1]) {
            (Some(a), Some(b)) => Some(((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt() * rate),
            _ => None,
        })
        .collect();
    if steps.len() >= 4 {
        let per_second = median(steps);
        if per_second > WANDER {
            notes.push(Note::whole(Weight::Look, Kind::Wander { per_second }));
        }
    }
    notes
}

/// What the pictures themselves say.
///
/// `luma` is the average brightness of each frame read, zero to one, and
/// `change` the average difference from the frame before it on the same
/// scale, with the first entry zero.
pub fn picture_notes(luma: &[f64], change: &[f64], rate: f64) -> Vec<Note> {
    let mut notes = Vec::new();
    let dark: Vec<bool> = luma.iter().map(|level| *level < DARK).collect();
    for (at, until) in spells(&dark, rate) {
        notes.push(Note {
            at,
            until,
            weight: Weight::Stop,
            kind: Kind::Dark,
        });
    }
    // The first entry has nothing before it, so it is never frozen.
    let still: Vec<bool> = change
        .iter()
        .enumerate()
        .map(|(i, diff)| i > 0 && *diff < STILL)
        .collect();
    for (at, until) in spells(&still, rate) {
        notes.push(Note {
            at,
            until,
            weight: Weight::Stop,
            kind: Kind::Frozen,
        });
    }
    notes
}

/// One biquad, run one sample at a time.
///
/// Written out rather than taken from a crate because it is six
/// multiplications and the alternative is a dependency for them.
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Biquad {
        Biquad {
            b,
            a,
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn step(&mut self, input: f64) -> f64 {
        let out = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [out, self.y[0]];
        out
    }
}

/// How loud the sound is, in LUFS, by ITU-R BS.1770.
///
/// The two shaping filters, then four hundred millisecond blocks with the
/// quiet ones dropped: first everything under seventy below silence, then
/// everything ten below what is left. The gating is the part that matters.
/// Without it a clip that opens on a breath measures quieter than the same
/// clip trimmed, and the number stops meaning anything.
///
/// The filter coefficients are defined at forty eight thousand samples a
/// second, so [`sound_notes`] reads at that rate and this assumes it.
pub fn loudness_lufs(samples: &[f32], rate: u32) -> f64 {
    if samples.is_empty() || rate == 0 {
        return f64::NEG_INFINITY;
    }
    let mut shelf = Biquad::new(
        [
            1.535_124_859_586_97,
            -2.691_696_189_406_38,
            1.198_392_810_852_85,
        ],
        [-1.690_659_293_182_41, 0.732_480_774_215_85],
    );
    let mut pass = Biquad::new(
        [1.0, -2.0, 1.0],
        [-1.990_047_454_833_98, 0.990_072_250_366_21],
    );
    let shaped: Vec<f64> = samples
        .iter()
        .map(|sample| pass.step(shelf.step(f64::from(*sample))))
        .collect();

    // Four hundred milliseconds, moved on by a hundred: the overlap the
    // standard asks for.
    let block = (f64::from(rate) * 0.4).round() as usize;
    let hop = (f64::from(rate) * 0.1).round() as usize;
    if block == 0 || hop == 0 || shaped.len() < block {
        return f64::NEG_INFINITY;
    }
    let power: Vec<f64> = (0..=(shaped.len() - block))
        .step_by(hop)
        .map(|start| {
            let window = &shaped[start..start + block];
            window.iter().map(|v| v * v).sum::<f64>() / block as f64
        })
        .collect();
    let level = |mean: f64| -0.691 + 10.0 * mean.max(1e-20).log10();

    let loud: Vec<f64> = power
        .iter()
        .copied()
        .filter(|mean| level(*mean) > -70.0)
        .collect();
    if loud.is_empty() {
        return f64::NEG_INFINITY;
    }
    let ungated = level(loud.iter().sum::<f64>() / loud.len() as f64);
    let kept: Vec<f64> = loud
        .into_iter()
        .filter(|mean| level(*mean) > ungated - 10.0)
        .collect();
    if kept.is_empty() {
        return ungated;
    }
    level(kept.iter().sum::<f64>() / kept.len() as f64)
}

/// What the sound says. `rate` should be forty eight thousand; see
/// [`loudness_lufs`].
pub fn sound_notes(samples: &[f32], rate: u32) -> Vec<Note> {
    if samples.is_empty() || rate == 0 {
        return Vec::new();
    }
    let mut notes = Vec::new();

    let lufs = loudness_lufs(samples, rate);
    if lufs.is_finite() {
        if lufs < TARGET_LOUDNESS - LOUDNESS_SLACK {
            notes.push(Note::whole(Weight::Stop, Kind::Quiet { lufs }));
        } else if lufs > TARGET_LOUDNESS + LOUDNESS_SLACK {
            notes.push(Note::whole(Weight::Look, Kind::Loud { lufs }));
        }
    }

    // Clipping, reported once with the first place it happens: a file that
    // clips usually clips in dozens of places and the fix is the same one.
    let mut run = 0usize;
    for (i, sample) in samples.iter().enumerate() {
        if sample.abs() >= CLIPPED {
            run += 1;
            if run >= CLIPPED_RUN {
                let at = (i + 1 - run) as f64 / f64::from(rate);
                notes.push(Note {
                    at,
                    until: at,
                    weight: Weight::Stop,
                    kind: Kind::Clipped,
                });
                break;
            }
        } else {
            run = 0;
        }
    }

    // An opening with nothing in it, judged against the clip's own speech
    // rather than against silence, the same way `gaps` judges a pause.
    let level = super::punch::loudness(samples, rate);
    let speaking: Vec<f64> = level.iter().copied().filter(|v| *v > 1e-5).collect();
    let floor = median(speaking) * super::gaps::QUIET;
    if floor > 0.0 {
        let quiet =
            level.iter().take_while(|value| **value < floor).count() as f64 * super::punch::WINDOW;
        if quiet >= DEAD_OPEN {
            notes.push(Note {
                at: 0.0,
                until: quiet,
                weight: Weight::Stop,
                kind: Kind::DeadOpen { seconds: quiet },
            });
        }
    }
    notes
}

/// Reads `path` back and says what is wrong with it.
///
/// The file is the exported one, so the frames are the frames somebody will
/// see and the numbers mean what they say. `progress` runs from zero to one
/// over the picture, which is the slow half.
pub fn look_over(
    path: &str,
    detector: &Mutex<Detector>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<Note>, String> {
    let info = concat_media::probe(path).map_err(|error| error.to_string())?;
    let video = info.require_video().map_err(|error| error.to_string())?;
    let (width, height) = (video.width, video.height);
    let seconds = info.duration.map_or(0.0, |d| d.as_f64());

    let mut notes = shape_notes(width, height, seconds);
    if seconds <= 0.0 || width == 0 || height == 0 {
        return Ok(notes);
    }

    // The longer side fitted to the model's square, so a tall frame keeps a
    // face the size the model was trained to find. Decoding larger is read
    // and thrown away; decoding by width, as the reframe does for a wide
    // source, would letterbox this one down to a third of the square.
    let aspect = f64::from(width) / f64::from(height);
    let input = concat_vision::face::INPUT_W as f64;
    let (read_w, read_h) = if aspect >= 1.0 {
        (input, input / aspect)
    } else {
        (input * aspect, input)
    };
    let options = DecodeOptions::default()
        .scaled_to(
            (read_w.round() as u32).max(1),
            (read_h.round() as u32).max(1),
        )
        .at_rate(FrameRate::new(Rational::new(i64::from(LOOK_RATE), 1)));
    let mut decoder = Decoder::open(path, &options).map_err(|error| error.to_string())?;

    let wanted = ((seconds * f64::from(LOOK_RATE)).round() as usize).max(1);
    let mut seen: Vec<Vec<Face>> = Vec::with_capacity(wanted);
    let mut luma: Vec<f64> = Vec::with_capacity(wanted);
    let mut change: Vec<f64> = Vec::with_capacity(wanted);
    let mut before: Option<Vec<u8>> = None;
    while seen.len() < wanted {
        if cancel.load(Ordering::Relaxed) {
            return Err("review cancelled".to_owned());
        }
        let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? else {
            break;
        };
        let faces = detector
            .lock()
            .map_err(|_| "detector poisoned")?
            .faces(&frame)?;
        let (bright, moved) = picture_of(&frame, before.as_deref());
        before = Some(frame.pixels().to_vec());
        seen.push(faces);
        luma.push(bright);
        change.push(moved);
        progress(seen.len() as f32 / wanted as f32);
    }
    progress(1.0);

    let rate = f64::from(LOOK_RATE);
    notes.extend(face_notes(&seen, rate));
    notes.extend(picture_notes(&luma, &change, rate));

    if info.audio.is_some() {
        notes.extend(sound_of(path, seconds)?);
    }
    notes.sort_by(|a, b| a.at.total_cmp(&b.at));
    Ok(notes)
}

/// The average brightness of a frame, and how far it is from the one
/// before, both zero to one.
///
/// Brightness weighted the way an eye weighs it, because a frame that is
/// pure blue is dark to look at and a mean of the three channels calls it
/// a third lit.
fn picture_of(frame: &concat_core::Frame, before: Option<&[u8]>) -> (f64, f64) {
    use concat_core::frame::BYTES_PER_PIXEL;
    let pixels = frame.pixels();
    if pixels.is_empty() {
        return (0.0, 0.0);
    }
    let count = pixels.len() / BYTES_PER_PIXEL;
    if count == 0 {
        return (0.0, 0.0);
    }
    // Every ninth pixel: brightness is an average and an average does not
    // need all two hundred thousand of them.
    let step = BYTES_PER_PIXEL * 9;
    let mut lit = 0.0;
    let mut moved = 0.0;
    let mut taken = 0.0;
    // Only comparable against a frame of the same size, which after the
    // first one it always is.
    let earlier = before.filter(|old| old.len() == pixels.len());
    let mut at = 0;
    while at + 2 < pixels.len() {
        let now = 0.2126 * f64::from(pixels[at])
            + 0.7152 * f64::from(pixels[at + 1])
            + 0.0722 * f64::from(pixels[at + 2]);
        lit += now / 255.0;
        if let Some(old) = earlier {
            let was = 0.2126 * f64::from(old[at])
                + 0.7152 * f64::from(old[at + 1])
                + 0.0722 * f64::from(old[at + 2]);
            moved += (now - was).abs() / 255.0;
        }
        taken += 1.0;
        at += step;
    }
    if taken == 0.0 {
        return (0.0, 0.0);
    }
    (lit / taken, moved / taken)
}

/// The sound notes for one file.
fn sound_of(path: &str, seconds: f64) -> Result<Vec<Note>, String> {
    use concat_media::{AudioDecoder, AudioOptions, SampleFormat};

    // Forty eight thousand because the loudness filters are defined there,
    // and mono because a note about a mix is a note about the whole mix.
    const RATE: u32 = 48_000;
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            start: None,
            duration: Some(seconds),
            filters: Vec::new(),
            rate: RATE,
            channels: 1,
            format: SampleFormat::F32,
            stream: None,
        },
    )
    .map_err(|error| error.to_string())?;
    let samples = decoder.collect_f32().map_err(|error| error.to_string())?;
    Ok(sound_notes(&samples, RATE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face_at(x: f64, y: f64, w: f64, h: f64) -> Face {
        Face {
            x,
            y,
            w,
            h,
            score: 0.9,
            mouth: 0.0,
        }
    }

    /// A well framed head: a fifth of the frame tall, eyes high, still.
    fn framed(n: usize) -> Vec<Vec<Face>> {
        (0..n)
            .map(|_| vec![face_at(0.35, 0.28, 0.3, 0.2)])
            .collect()
    }

    #[test]
    fn a_well_framed_still_head_has_nothing_said_about_it() {
        assert!(
            face_notes(&framed(60), f64::from(LOOK_RATE)).is_empty(),
            "a good shot was complained about"
        );
    }

    #[test]
    fn a_chin_off_the_bottom_is_reported_once_it_lasts() {
        let mut seen = framed(60);
        // Two seconds of it, which is six readings at three a second.
        for frame in seen.iter_mut().skip(20).take(6) {
            frame[0] = face_at(0.35, 0.85, 0.3, 0.2);
        }
        let notes = face_notes(&seen, f64::from(LOOK_RATE));
        assert!(
            notes.iter().any(|n| n.kind == Kind::FaceCut),
            "a cut chin went unnoticed: {notes:?}"
        );
    }

    #[test]
    fn one_lost_frame_is_not_a_fault() {
        let mut seen = framed(60);
        seen[30] = Vec::new();
        let notes = face_notes(&seen, f64::from(LOOK_RATE));
        assert!(
            !notes.iter().any(|n| n.kind == Kind::NoFace),
            "a single dropped detection was reported: {notes:?}"
        );
    }

    #[test]
    fn a_face_the_size_of_a_thumbnail_is_a_fault() {
        let seen: Vec<Vec<Face>> = (0..60)
            .map(|_| vec![face_at(0.45, 0.3, 0.06, 0.05)])
            .collect();
        let notes = face_notes(&seen, f64::from(LOOK_RATE));
        assert!(
            notes
                .iter()
                .any(|n| matches!(n.kind, Kind::FaceSmall { .. })),
            "a tiny face passed: {notes:?}"
        );
    }

    #[test]
    fn a_hunting_camera_is_reported_and_a_locked_one_is_not() {
        let locked = framed(60);
        assert!(
            !face_notes(&locked, f64::from(LOOK_RATE))
                .iter()
                .any(|n| matches!(n.kind, Kind::Wander { .. })),
            "a locked shot was called unsteady"
        );

        // Moving a twentieth of the frame every reading, which at three a
        // second is well past WANDER.
        let hunting: Vec<Vec<Face>> = (0..60)
            .map(|i| {
                let drift = (i % 8) as f64 * 0.05;
                vec![face_at(0.2 + drift, 0.28, 0.3, 0.2)]
            })
            .collect();
        assert!(
            face_notes(&hunting, f64::from(LOOK_RATE))
                .iter()
                .any(|n| matches!(n.kind, Kind::Wander { .. })),
            "a hunting camera passed"
        );
    }

    #[test]
    fn the_shape_check_knows_a_tall_frame_from_a_wide_one() {
        assert!(shape_notes(1080, 1920, 30.0).is_empty(), "9:16 refused");
        assert!(
            shape_notes(1920, 1080, 30.0)
                .iter()
                .any(|n| matches!(n.kind, Kind::NotTall { .. })),
            "16:9 accepted"
        );
    }

    /// A tone at a known level, which BS.1770 has a known answer for.
    fn tone(rate: u32, seconds: f64, amplitude: f64) -> Vec<f32> {
        let n = (seconds * f64::from(rate)).round() as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                (amplitude * (std::f64::consts::TAU * 1000.0 * t).sin()) as f32
            })
            .collect()
    }

    #[test]
    fn the_loudness_of_a_known_tone_is_the_known_answer() {
        // A 1 kHz sine at full scale is -3.01 dBFS in mean square, and the
        // shaping filters are near enough flat there, so BS.1770 answers
        // about -3.0 LUFS. Half a decibel of room for the filters.
        let full = loudness_lufs(&tone(48_000, 5.0, 1.0), 48_000);
        assert!((full + 3.0).abs() < 0.5, "full scale measured {full}");
        // Halving the amplitude is six decibels down, exactly.
        let half = loudness_lufs(&tone(48_000, 5.0, 0.5), 48_000);
        assert!(
            (full - half - 6.02).abs() < 0.1,
            "half scale measured {half} against {full}"
        );
    }

    #[test]
    fn sound_too_quiet_for_a_feed_is_reported() {
        // About -26 LUFS: a long way under the target.
        let quiet = tone(48_000, 5.0, 0.05);
        let notes = sound_notes(&quiet, 48_000);
        assert!(
            notes.iter().any(|n| matches!(n.kind, Kind::Quiet { .. })),
            "quiet sound passed: {notes:?}"
        );
    }

    #[test]
    fn a_black_stretch_is_reported_and_an_ordinary_picture_is_not() {
        let rate = f64::from(LOOK_RATE);
        // Brightness that wobbles, so nothing reads as frozen.
        let lit: Vec<f64> = (0..60).map(|i| 0.4 + (i % 3) as f64 * 0.01).collect();
        let moved: Vec<f64> = (0..60).map(|i| if i == 0 { 0.0 } else { 0.02 }).collect();
        assert!(
            picture_notes(&lit, &moved, rate).is_empty(),
            "an ordinary picture was complained about"
        );

        let mut dark = lit.clone();
        for level in dark.iter_mut().skip(10).take(6) {
            *level = 0.01;
        }
        assert!(
            picture_notes(&dark, &moved, rate)
                .iter()
                .any(|n| n.kind == Kind::Dark),
            "a black stretch passed"
        );
    }

    #[test]
    fn a_frozen_stretch_is_reported() {
        let rate = f64::from(LOOK_RATE);
        let lit: Vec<f64> = (0..60).map(|_| 0.4).collect();
        let mut moved: Vec<f64> = (0..60).map(|i| if i == 0 { 0.0 } else { 0.02 }).collect();
        for diff in moved.iter_mut().skip(20).take(8) {
            *diff = 0.0;
        }
        assert!(
            picture_notes(&lit, &moved, rate)
                .iter()
                .any(|n| n.kind == Kind::Frozen),
            "a frozen stretch passed"
        );
    }
}
