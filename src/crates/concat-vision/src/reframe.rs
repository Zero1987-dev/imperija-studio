// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Reframing a wide shot to a tall one, following whoever is talking.
//!
//! A podcast is shot wide and posted tall. Done by hand that is a keyframe
//! every time someone leans, and it is the reason the clip never gets cut.
//! Done here it is one pass: a face detector says where the subject is in
//! each sampled frame, and this module turns that into the `scale`,
//! `offsetX` and `offsetY` the clip already understands.
//!
//! Those three and not a new effect on purpose. The renderer, the exporter
//! and the Keyframes tab all read them today, so a reframe is an edit the
//! user can see, drag and undo rather than a black box. If the tracker put
//! the camera somewhere silly, the fix is moving a key.
//!
//! Everything here is geometry and arithmetic, tested without a model. The
//! detector is [`super::face`], behind the `infer` feature.
//!
//! **Fractions, not pixels.** A face arrives as fractions of the source
//! picture - `(0, 0)` its top-left, `(1, 1)` its bottom-right - which is
//! the same language masks and strokes are stored in, so a crop or a change
//! of export size changes nothing here.

/// Where a face is in the source picture, in source fractions.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Face {
    /// Left edge, `0..1`.
    pub x: f64,
    /// Top edge, `0..1`.
    pub y: f64,
    /// Width as a fraction of the source's width.
    pub w: f64,
    /// Height as a fraction of the source's height.
    pub h: f64,
    /// How sure the detector is, `0..1`.
    pub score: f64,
    /// How far the mouth sits below the eyes, in eye-widths.
    ///
    /// The detector draws five points on every face - both eyes, the nose
    /// and the two corners of the mouth - and this is the one number worth
    /// keeping from them: the drop from the eye line to the mouth line,
    /// divided by the distance between the eyes. Dividing makes it the
    /// same number whether the face is near or far, which is what lets two
    /// people in one shot be compared at all.
    ///
    /// It grows as the jaw opens. On its own a single reading says very
    /// little, since a face turned aside reads much the same as a face
    /// mid-word; but *how much it varies* over a second or two separates a
    /// mouth that is working from one that is not. See [`bind`], which is the
    /// only thing that uses it, and uses it over a whole clip rather than a
    /// moment.
    ///
    /// Zero when the detector gave no usable points.
    pub mouth: f64,
}

impl Face {
    /// The middle of the face, in source fractions.
    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// How much of the picture it covers. The subject is usually the
    /// largest face; a face in the background is smaller by the same
    /// amount it is unimportant.
    pub fn area(&self) -> f64 {
        (self.w * self.h).max(0.0)
    }
}

/// Where the camera is, in the clip's own units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shot {
    /// `Clip::scale`: multiplier over the fitted size, 1 filling the frame.
    pub scale: f64,
    /// `Clip::offset_x`: from centred, as a fraction of frame width.
    pub offset_x: f64,
    /// `Clip::offset_y`: the same vertically; positive moves down.
    pub offset_y: f64,
}

/// The scale at which a source of one aspect covers a frame of another,
/// leaving no bars.
///
/// At `scale` 1 the source is *fitted*: the whole of it is inside the frame
/// and the rest is empty. Filling instead means growing it until its short
/// side reaches the frame's, which is the ratio of the two aspects - 3.16
/// for 16:9 into 9:16, because a tall slice of a wide picture is under a
/// third of its width.
pub fn cover_scale(source_aspect: f64, frame_aspect: f64) -> f64 {
    if source_aspect <= 0.0 || frame_aspect <= 0.0 {
        return 1.0;
    }
    (source_aspect / frame_aspect).max(frame_aspect / source_aspect)
}

/// How far the camera may travel before it shows past the picture's edge,
/// as a fraction of the frame, horizontally and vertically.
///
/// At scale `s` the source is `s` frame-widths across, so half of `s - 1`
/// hangs off each side and that is exactly how far it may move. Vertically
/// the source is `s * frame_aspect / source_aspect` frame-heights tall, and
/// a wide source filled into a tall frame has nothing to spare there: the
/// limit is zero and the camera only pans sideways, which is what a podcast
/// wants anyway.
pub fn travel(scale: f64, source_aspect: f64, frame_aspect: f64) -> (f64, f64) {
    let across = ((scale - 1.0) / 2.0).max(0.0);
    let down = if source_aspect > 0.0 {
        ((scale * frame_aspect / source_aspect - 1.0) / 2.0).max(0.0)
    } else {
        0.0
    };
    (across, down)
}

/// The camera that puts a point of the source under the middle of the frame.
///
/// A point at source fraction `u` sits `(u - 0.5)` of the *displayed* width
/// from its middle, and the displayed width is `scale` frames across, so
/// bringing it to the middle is an offset of `-(u - 0.5) * scale`. The
/// vertical is the same with the displayed height, which is shorter by the
/// ratio of the aspects. Both are then held inside [`travel`], so the frame
/// never shows past the edge of the picture.
pub fn shot_for(u: f64, v: f64, scale: f64, source_aspect: f64, frame_aspect: f64) -> Shot {
    shot_placing(u, v, (0.5, 0.5), scale, source_aspect, frame_aspect)
}

/// The camera that puts a point of the source at a chosen place in the
/// frame, rather than always at its middle.
///
/// `place` is where the point should land, `(0, 0)` the frame's top-left
/// and `(1, 1)` its bottom-right. Wanting it a tenth of a frame higher is
/// a tenth taken off the offset, which is all the difference between this
/// and [`shot_for`] - see [`HEADROOM`] for why a face wants that.
pub fn shot_placing(
    u: f64,
    v: f64,
    place: (f64, f64),
    scale: f64,
    source_aspect: f64,
    frame_aspect: f64,
) -> Shot {
    let height_in_frames = if source_aspect > 0.0 {
        scale * frame_aspect / source_aspect
    } else {
        scale
    };
    let (max_x, max_y) = travel(scale, source_aspect, frame_aspect);
    Shot {
        scale,
        offset_x: (-(u - 0.5) * scale + (place.0 - 0.5)).clamp(-max_x, max_x),
        offset_y: (-(v - 0.5) * height_in_frames + (place.1 - 0.5)).clamp(-max_y, max_y),
    }
}

/// How far the subject may drift before the camera answers, as a fraction
/// of the **exported frame's** width.
///
/// A head moves a little all the time. Following every twitch reads as a
/// hand-held camera in a room where there is none, so inside this the
/// camera holds still.
///
/// Measured on the frame and not on the source, which is what makes the
/// number mean anything: a tall slice of a wide shot is under a third of
/// it, so the source is magnified over three times on the way to the
/// screen, and four percent of the source - what this used to be - was
/// thirteen percent of the picture anyone actually sees.
///
/// Five and a half percent is where it sits because either side of that
/// is worse. Simulated against a still face with the detector's box
/// wandering up to two percent of the source, tighter values let that
/// wander through and the camera crept about; wider ones bought no more
/// stillness and only let the head sit further off-centre. At this value
/// the camera does not move at all for a still subject, and lands within
/// about one percent of centre after a real one.
pub const DEADZONE: f64 = 0.055;

/// How much of the way the camera closes on the subject each sampled frame.
///
/// Low enough that a turn of the head is a glide and not a snap, high
/// enough that the camera has arrived before the sentence ends. At the
/// old eighth it took three and a half seconds to answer someone leaning
/// across a table, which reads as a camera asleep at the wheel; a quarter
/// does it in under two. Stillness costs nothing here - [`DEADZONE`]
/// decides whether the camera moves at all, and this only decides how
/// quickly once it has.
pub const EASING: f64 = 0.25;

/// How much larger another face has to be before it is worth leaving the
/// one being followed.
///
/// Two people sat the same distance from one camera measure within a few
/// percent of each other, and which of them is "largest" then changes with
/// the detector's own noise. Answering that noise is what made the picture
/// dance between them. A third larger is a real difference - someone
/// leaning in, or a single speaker filling the frame - not a wobble.
pub const TAKEOVER: f64 = 1.35;

/// And for how many samples in a row before the camera believes it.
///
/// At [`super::SAMPLE_RATE`] this is about a second: long enough that a
/// flicker cannot move the camera, short enough that a real change of
/// speaker is not missed.
pub const INSIST: usize = 6;

/// How far the followed face may move between samples and still be taken
/// for the same person.
///
/// Past this it is not that they moved, it is that they are gone - out of
/// frame, turned away - and the camera takes the largest face there is
/// instead.
pub const LOST: f64 = 0.25;

/// How near the camera has to get before it stops following again.
///
/// The deadzone decides when the camera *sets out*, not when it gives up.
/// Without this pair the camera would stop the moment it came inside
/// [`DEADZONE`] and sit that far off-centre for the rest of the clip;
/// with it a real move is seen through to the end, and only then does the
/// camera go back to ignoring small ones.
pub const ARRIVED: f64 = DEADZONE / 5.0;

/// How many samples the subject's position is taken over before the camera
/// believes it.
///
/// The detector redraws its box every frame and the box wanders a few
/// pixels even when nobody moves. That wander is small in the source and
/// three times larger on screen, and it is what makes an otherwise still
/// picture creep about. The middle value of the last nine sightings - a
/// median, not an average, so one bad frame moves nothing - is steady
/// where the raw box is not. At [`super::SAMPLE_RATE`] nine samples is a
/// second and a half, which a seated guest is easily worth.
pub const STEADY: usize = 9;

/// How much of the frame's height the detector's box should fill.
///
/// YuNet draws a box around the face itself, brow to chin, so the whole
/// head is about half again as tall as it. A fifth of the frame puts the
/// head at roughly a third of it, which is how a talking head is framed
/// when a person does it. A podcast shot wide leaves the face far smaller
/// than that, and merely slicing a tall piece out of the middle keeps it
/// small - the reason an automatic reframe so often looks like a crop
/// rather than a shot.
pub const FACE_HEIGHT: f64 = 0.22;

/// And how far in it is worth going to get there.
///
/// Past this the source is being enlarged more than it has detail for and
/// the picture goes soft, which is worse than a face that is a little
/// small.
pub const ZOOM_MAX: f64 = 1.5;

/// Where down the frame the face's box is placed.
///
/// Dead centre leaves as much room above the head as below the chest and
/// reads as a mistake; the eye expects a little headroom and a lot of
/// body. Slightly above the middle gives that, and leaves the bottom of
/// the frame - where captions go - clear of the face.
pub const HEADROOM: f64 = 0.42;

/// The face nearest a point, and how far it is.
fn nearest(faces: &[Face], to: (f64, f64)) -> Option<(Face, f64)> {
    faces
        .iter()
        .map(|face| {
            let (u, v) = face.centre();
            (*face, ((u - to.0).powi(2) + (v - to.1).powi(2)).sqrt())
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The largest face in a frame.
fn largest(faces: &[Face]) -> Option<Face> {
    faces
        .iter()
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .copied()
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

/// Where the subject really is, from the last few sightings of them.
fn steady(recent: &[(f64, f64)]) -> (f64, f64) {
    (
        median(recent.iter().map(|p| p.0).collect()),
        median(recent.iter().map(|p| p.1).collect()),
    )
}

/// How tall the subject's face is, over the whole clip, in source
/// fractions.
///
/// The median rather than the mean, so a frame where the detector drew a
/// box around a lamp does not decide how far the camera zooms in. Zero
/// when no face was found at all, which [`framing`] reads as "do not
/// zoom".
pub fn subject_height(samples: &[Vec<Face>]) -> f64 {
    median(
        samples
            .iter()
            .filter_map(|faces| largest(faces))
            .map(|face| face.h)
            .filter(|h| *h > 0.0)
            .collect(),
    )
}

/// The scale that frames a face of this height properly, never showing
/// past the edge of the picture and never enlarging past [`ZOOM_MAX`].
///
/// At scale `s` the source stands `s * frame_aspect / source_aspect`
/// frame-heights tall, so a face `face_height` of the source fills that
/// much of it again; setting the product to [`FACE_HEIGHT`] and solving
/// for `s` is the whole of this. The answer is then held between merely
/// covering the frame - below which black bars appear - and the zoom
/// limit.
pub fn framing(face_height: f64, source_aspect: f64, frame_aspect: f64) -> f64 {
    let cover = cover_scale(source_aspect, frame_aspect);
    if face_height <= 0.0 || source_aspect <= 0.0 || frame_aspect <= 0.0 {
        return cover;
    }
    let wanted = FACE_HEIGHT / face_height * source_aspect / frame_aspect;
    wanted.clamp(cover, cover * ZOOM_MAX)
}

/// How much of the frame one source-fraction of subject movement crosses,
/// sideways and up-and-down, both as fractions of the frame's **width**.
///
/// This is what lets [`DEADZONE`] mean the same thing whatever the zoom.
/// An axis the camera cannot move along at all - and a wide shot filled
/// into a tall frame has no vertical room whatever - answers zero, so
/// that a head bobbing up and down, which changes nothing anyone can see,
/// no longer counts as the subject having moved and sets the camera
/// sliding sideways for it.
pub fn sensitivity(scale: f64, source_aspect: f64, frame_aspect: f64) -> (f64, f64) {
    let (max_x, max_y) = travel(scale, source_aspect, frame_aspect);
    let height_in_frames = if source_aspect > 0.0 {
        scale * frame_aspect / source_aspect
    } else {
        scale
    };
    let across = if max_x > 0.0 { scale } else { 0.0 };
    // Vertical offsets are fractions of the frame's height; as fractions
    // of its width they are larger by the frame's own aspect.
    let down = if max_y > 0.0 && frame_aspect > 0.0 {
        height_in_frames / frame_aspect
    } else {
        0.0
    };
    (across, down)
}

/// The camera's path over a clip, from every face found in each sampled
/// frame.
///
/// **It follows a person, not a position.** The camera keeps hold of whom
/// it is watching and looks for that same face again in the next frame;
/// it only changes to somebody else when they are [`TAKEOVER`] larger for
/// [`INSIST`] samples running. Choosing the largest face afresh each time
/// is what made a two-person podcast dance: the two measure within a few
/// percent of each other, so the detector's noise decided who was
/// "largest", and every flicker became a cut.
///
/// Having settled on whom to watch, it takes their position as the median
/// of the last [`STEADY`] sightings rather than the box the detector just
/// drew, because that box wanders a few pixels a frame on a subject sat
/// perfectly still. It then holds inside [`DEADZONE`] - measured on the
/// exported frame, so it means the same after the zoom - and glides
/// [`EASING`] of the way per sample otherwise. The
/// one time it moves at once is a change of subject - that is a vision
/// mixer cutting to the other guest, and gliding across the frame for it
/// would look like the camera lost them.
///
/// A frame nobody was found in keeps the camera where it was: a subject
/// who turns away or is briefly hidden should not send it to the middle.
pub fn follow(samples: &[Vec<Face>], sensitivity: (f64, f64)) -> Vec<(f64, f64)> {
    let mut watching: Option<Face> = None;
    let mut rival = 0usize;
    let mut recent: Vec<(f64, f64)> = Vec::with_capacity(STEADY);
    let mut camera = samples
        .iter()
        .flat_map(|faces| largest(faces))
        .next()
        .map_or((0.5, 0.5), |face| face.centre());
    let mut following = false;
    let mut path = Vec::with_capacity(samples.len());

    for faces in samples {
        if faces.is_empty() {
            path.push(camera);
            continue;
        }
        // The same person again, by where they were; or the largest when
        // nobody is being watched or the one being watched has gone.
        let held = watching
            .and_then(|face| nearest(faces, face.centre()))
            .filter(|(_, away)| *away <= LOST)
            .map(|(face, _)| face)
            .or_else(|| largest(faces));
        let Some(held) = held else {
            path.push(camera);
            continue;
        };

        let biggest = largest(faces).unwrap_or(held);
        let cut = if watching.is_none() {
            watching = Some(held);
            true
        } else if biggest.area() > held.area() * TAKEOVER {
            rival += 1;
            if rival >= INSIST {
                watching = Some(biggest);
                rival = 0;
                true
            } else {
                watching = Some(held);
                false
            }
        } else {
            rival = 0;
            watching = Some(held);
            false
        };

        // Where they are, taken over the last few sightings rather than
        // this one alone. A cut is a different person, so their history
        // starts over.
        if cut {
            recent.clear();
        }
        recent.push(watching.unwrap_or(held).centre());
        if recent.len() > STEADY {
            recent.remove(0);
        }
        let (u, v) = steady(&recent);
        if cut {
            camera = (u, v);
            following = false;
        } else {
            let (dx, dy) = (u - camera.0, v - camera.1);
            // On the screen, not in the source: see `DEADZONE`.
            let away = (dx * sensitivity.0).hypot(dy * sensitivity.1);
            if away > DEADZONE {
                following = true;
            } else if away <= ARRIVED {
                following = false;
            }
            if following {
                camera = (camera.0 + dx * EASING, camera.1 + dy * EASING);
            }
        }
        path.push(camera);
    }
    path
}

/// One stretch of a clip in which one person was speaking.
///
/// Comes from the audio, not the picture - see `concat_speech::diarize` -
/// so it is right about *when* somebody spoke and says nothing at all about
/// *which face on screen* that was. Joining the two is [`bind`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Turn {
    /// Seconds from the start of the clip.
    pub start: f64,
    /// Seconds from the start of the clip.
    pub end: f64,
    /// Which voice, numbered from zero, as the diarizer separated them.
    pub speaker: usize,
}

/// How many readings of a mouth are taken together before it means
/// anything.
///
/// Eight, which at [`super::SAMPLE_RATE`] is a second and a third. Speech
/// moves a jaw around four times a second and this samples six, so the
/// readings alias badly and say nothing about rhythm - but the *spread* of
/// them still separates a mouth that is working from one that is not, and
/// spread is all [`bind`] ever asks for.
pub const MOUTH_WINDOW: usize = 8;

/// Turns shorter than this are not worth moving a camera for.
///
/// A diarizer marks every "mhm" as a turn. Cutting to the other person for
/// three quarters of a second and straight back is the twitch that makes an
/// automatic edit obvious, so short turns are left to whoever holds the
/// shot.
pub const SHORTEST_TURN: f64 = 0.8;

/// How many faces the clip usually shows at once.
///
/// The commonest count rather than the largest: one frame where the
/// detector found a face in the bookshelf should not convince the camera
/// there are three people in the room.
fn crowd(samples: &[Vec<Face>]) -> usize {
    let mut tally = [0usize; 8];
    for faces in samples {
        if (1..=tally.len()).contains(&faces.len()) {
            tally[faces.len() - 1] += 1;
        }
    }
    tally
        .iter()
        .enumerate()
        .max_by_key(|(_, seen)| **seen)
        .filter(|(_, seen)| **seen > 0)
        .map_or(0, |(i, _)| i + 1)
}

/// The faces of one sample, left to right, when that sample shows the
/// usual number of them. `None` otherwise - a frame with somebody missing
/// cannot say who is who by position alone.
fn seated(faces: &[Face], crowd: usize) -> Option<Vec<Face>> {
    if crowd == 0 || faces.len() != crowd {
        return None;
    }
    let mut row = faces.to_vec();
    row.sort_by(|a, b| a.centre().0.total_cmp(&b.centre().0));
    Some(row)
}

/// Where each person sits, left to right, as a fraction across the source.
///
/// People in a podcast do not swap chairs, so their order across the frame
/// *is* their identity, and it survives the detector losing one of them for
/// a moment far better than following boxes about does.
pub fn places(samples: &[Vec<Face>]) -> Vec<f64> {
    let crowd = crowd(samples);
    let mut seen: Vec<Vec<f64>> = vec![Vec::new(); crowd];
    for faces in samples {
        let Some(row) = seated(faces, crowd) else {
            continue;
        };
        for (person, face) in row.iter().enumerate() {
            seen[person].push(face.centre().0);
        }
    }
    seen.into_iter().map(median).collect()
}

/// How hard each person's mouth was working, sample by sample.
///
/// The spread of the last [`MOUTH_WINDOW`] readings of [`Face::mouth`].
/// Zero where that person was not seen, or too few readings have arrived.
fn working(samples: &[Vec<Face>]) -> Vec<Vec<f64>> {
    let crowd = crowd(samples);
    let mut out = vec![vec![0.0; samples.len()]; crowd];
    let mut recent: Vec<Vec<f64>> = vec![Vec::new(); crowd];
    for (at, faces) in samples.iter().enumerate() {
        let Some(row) = seated(faces, crowd) else {
            continue;
        };
        for (person, face) in row.iter().enumerate() {
            let seen = &mut recent[person];
            seen.push(face.mouth);
            if seen.len() > MOUTH_WINDOW {
                seen.remove(0);
            }
            if seen.len() >= 3 {
                out[person][at] = spread(seen);
            }
        }
    }
    out
}

/// The mean square distance from the middle of a set.
fn spread(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
}

/// Which face belongs to which voice: for each speaker, the person the
/// camera should be on while that speaker talks.
///
/// **One decision per person for the whole clip, and that is the point.**
/// A single reading of a mouth is nearly worthless - a face turned aside
/// looks much like a face mid-word - and deciding who is talking from one
/// moment's readings is what a first attempt at this does and why it fails.
/// But the audio already says *when* each voice spoke. All that is left is
/// to ask, over the whole clip, whose mouth worked hardest during voice
/// nought's turns and whose during voice one's. Twenty seconds of a weak
/// signal answers that far more surely than a second of it answers anything.
///
/// The answer is an index into [`places`] per speaker. An empty answer
/// means it could not tell - one face, no turns, or two voices that both
/// picked the same face - and the caller should fall back to [`follow`].
pub fn bind(samples: &[Vec<Face>], turns: &[Turn], rate: f64) -> Vec<usize> {
    let crowd = crowd(samples);
    let voices = turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0);
    if crowd < 2 || voices < 2 || rate <= 0.0 {
        return Vec::new();
    }
    let effort = working(samples);
    let speaking_at = |at: usize| -> Option<usize> {
        let t = at as f64 / rate;
        turns
            .iter()
            .find(|turn| turn.start <= t && t < turn.end)
            .map(|turn| turn.speaker)
    };

    // For every pairing, how much harder that person's mouth worked during
    // that voice's turns than during everyone else's.
    let mut leaning = vec![vec![f64::NEG_INFINITY; crowd]; voices];
    for (voice, row) in leaning.iter_mut().enumerate() {
        for (person, cell) in row.iter_mut().enumerate() {
            let (mut theirs, mut n_theirs) = (0.0, 0usize);
            let (mut others, mut n_others) = (0.0, 0usize);
            for (at, work) in effort[person].iter().enumerate() {
                let Some(talking) = speaking_at(at) else {
                    continue;
                };
                if talking == voice {
                    theirs += work;
                    n_theirs += 1;
                } else {
                    others += work;
                    n_others += 1;
                }
            }
            if n_theirs == 0 || n_others == 0 {
                continue;
            }
            *cell = theirs / n_theirs as f64 - others / n_others as f64;
        }
    }

    // Strongest pairing first, and nobody claimed twice: two voices that
    // both look like the left-hand face mean the weaker claim is wrong.
    let mut answer = vec![usize::MAX; voices];
    let mut taken = vec![false; crowd];
    for _ in 0..voices.min(crowd) {
        let mut best: Option<(f64, usize, usize)> = None;
        for (voice, row) in leaning.iter().enumerate() {
            if answer[voice] != usize::MAX {
                continue;
            }
            for (person, score) in row.iter().enumerate() {
                if taken[person] || !score.is_finite() {
                    continue;
                }
                if best.is_none_or(|(top, _, _)| *score > top) {
                    best = Some((*score, voice, person));
                }
            }
        }
        let Some((_, voice, person)) = best else {
            break;
        };
        answer[voice] = person;
        taken[person] = true;
    }
    if answer.contains(&usize::MAX) {
        return Vec::new();
    }
    answer
}

/// The camera's path when the audio says who is speaking.
///
/// Same camera as [`follow`] - still, then a glide, and a cut when the
/// subject changes - but the subject is chosen by *who is talking* rather
/// than by whose face is biggest. Whose face is biggest was always the weak
/// part: two people sat the same distance from one lens measure the same
/// forever, so the camera either never left the first of them or left on
/// the detector's noise.
///
/// Falls back to [`follow`] whenever the audio cannot help: one face, one
/// voice, or a binding that did not come out.
pub fn follow_speaking(
    samples: &[Vec<Face>],
    sensitivity: (f64, f64),
    turns: &[Turn],
    rate: f64,
) -> Vec<(f64, f64)> {
    let voice_of = bind(samples, turns, rate);
    if voice_of.is_empty() {
        return follow(samples, sensitivity);
    }
    let seats = places(samples);
    let mut watching: Option<usize> = None;
    let mut recent: Vec<(f64, f64)> = Vec::with_capacity(STEADY);
    let mut camera = (0.5, 0.5);
    let mut started = false;
    let mut following = false;
    let mut path = Vec::with_capacity(samples.len());

    for (at, faces) in samples.iter().enumerate() {
        let t = at as f64 / rate;
        // Who has the floor, ignoring the interjections too short to move a
        // camera for. Between turns the camera stays where it was.
        let speaker = turns
            .iter()
            .find(|turn| turn.start <= t && t < turn.end && turn.end - turn.start >= SHORTEST_TURN)
            .map(|turn| turn.speaker);
        let wanted = speaker.and_then(|voice| voice_of.get(voice).copied());
        let Some(person) = wanted.or(watching) else {
            path.push(camera);
            continue;
        };
        // Their chair, and the face nearest it in this frame.
        let seat = seats.get(person).copied().unwrap_or(0.5);
        let Some((face, _)) = nearest(faces, (seat, 0.5)) else {
            path.push(camera);
            continue;
        };

        let cut = watching != Some(person);
        if cut {
            recent.clear();
        }
        watching = Some(person);
        recent.push(face.centre());
        if recent.len() > STEADY {
            recent.remove(0);
        }
        let (u, v) = steady(&recent);
        if cut || !started {
            camera = (u, v);
            following = false;
            started = true;
        } else {
            let (dx, dy) = (u - camera.0, v - camera.1);
            let away = (dx * sensitivity.0).hypot(dy * sensitivity.1);
            if away > DEADZONE {
                following = true;
            } else if away <= ARRIVED {
                following = false;
            }
            if following {
                camera = (camera.0 + dx * EASING, camera.1 + dy * EASING);
            }
        }
        path.push(camera);
    }
    path
}

/// The path as keys, with the ones that say nothing left out.
///
/// A key per sampled frame is thousands of keys on a clip nobody can then
/// edit. A point is kept when the straight line between the last kept key
/// and the next one would miss it by more than `tolerance` - the
/// Ramer-Douglas-Peucker rule - so a still camera is two keys and a pan is
/// as many as its shape needs. The first and last are always kept.
pub fn reduce(path: &[(f64, f64)], tolerance: f64) -> Vec<usize> {
    if path.len() <= 2 {
        return (0..path.len()).collect();
    }
    let mut keep = vec![false; path.len()];
    keep[0] = true;
    keep[path.len() - 1] = true;
    simplify(path, 0, path.len() - 1, tolerance, &mut keep);
    (0..path.len()).filter(|i| keep[*i]).collect()
}

/// The recursive half of [`reduce`]: the point furthest from the chord
/// between two kept ones, kept in turn when it is further than the
/// tolerance, and the two halves either side done the same way.
fn simplify(path: &[(f64, f64)], first: usize, last: usize, tolerance: f64, keep: &mut [bool]) {
    if last <= first + 1 {
        return;
    }
    let (ax, ay) = path[first];
    let (bx, by) = path[last];
    let (dx, dy) = (bx - ax, by - ay);
    let length = (dx * dx + dy * dy).sqrt();
    let mut worst = (0.0_f64, first);
    for (i, &(px, py)) in path.iter().enumerate().take(last).skip(first + 1) {
        // With no chord to speak of, distance from the point is the measure.
        let d = if length <= f64::EPSILON {
            ((px - ax).powi(2) + (py - ay).powi(2)).sqrt()
        } else {
            ((bx - ax) * (ay - py) - (ax - px) * (by - ay)).abs() / length
        };
        if d > worst.0 {
            worst = (d, i);
        }
    }
    if worst.0 > tolerance {
        simplify(path, first, worst.1, tolerance, keep);
        keep[worst.1] = true;
        simplify(path, worst.1, last, tolerance, keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: f64 = 16.0 / 9.0;
    const TALL: f64 = 9.0 / 16.0;

    /// A wide shot filled into a tall frame: the shape all of this is for.
    fn sense() -> (f64, f64) {
        sensitivity(cover_scale(WIDE, TALL), WIDE, TALL)
    }

    #[test]
    fn filling_a_tall_frame_from_a_wide_one_is_the_ratio_of_the_aspects() {
        // A 9:16 slice of a 16:9 picture is 31.6% of its width.
        let s = cover_scale(WIDE, TALL);
        assert!((s - (WIDE / TALL)).abs() < 1e-9);
        assert!((s - 3.160_493).abs() < 1e-5, "{s}");
        // The same source and frame needs no growing at all.
        assert!((cover_scale(WIDE, WIDE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_filled_tall_frame_pans_sideways_and_not_up() {
        let s = cover_scale(WIDE, TALL);
        let (across, down) = travel(s, WIDE, TALL);
        assert!(across > 1.0, "sideways room: {across}");
        assert!(down.abs() < 1e-9, "no vertical room once filled: {down}");
    }

    #[test]
    fn the_camera_puts_the_subject_in_the_middle() {
        let s = cover_scale(WIDE, TALL);
        // Someone a quarter of the way in from the left.
        let shot = shot_for(0.25, 0.5, s, WIDE, TALL);
        // Offset is measured in frame widths: a quarter of the source is a
        // quarter of `scale` frames.
        assert!((shot.offset_x - 0.25 * s).abs() < 1e-9, "{:?}", shot);
        assert!(shot.offset_y.abs() < 1e-9);
    }

    #[test]
    fn the_camera_never_shows_past_the_edge() {
        let s = cover_scale(WIDE, TALL);
        let (across, _) = travel(s, WIDE, TALL);
        for u in [0.0, 0.02, 0.98, 1.0] {
            let shot = shot_for(u, 0.5, s, WIDE, TALL);
            assert!(shot.offset_x.abs() <= across + 1e-9, "u={u} {:?}", shot);
        }
    }

    /// A face of width `w`, centred at `u` across and halfway down.
    fn seen(u: f64, w: f64) -> Face {
        Face {
            x: u - w / 2.0,
            y: 0.5 - w / 2.0,
            w,
            h: w,
            score: 0.9,
            mouth: 0.85,
        }
    }

    #[test]
    fn a_still_head_holds_the_camera_still() {
        // Drift well inside the deadzone, every sample.
        // A hundredth of the source is over three hundredths of the
        // screen once it is magnified, so the drift that counts as small
        // is smaller than it looks here.
        let samples: Vec<Vec<Face>> = (0..40)
            .map(|i| vec![seen(0.4 + f64::from(i % 2) * 0.004, 0.12)])
            .collect();
        let path = follow(&samples, sense());
        for p in &path {
            assert!((p.0 - path[0].0).abs() < 1e-9, "the camera moved to {p:?}");
        }
    }

    #[test]
    fn two_speakers_of_a_size_never_make_the_picture_dance() {
        // The bug this exists for. Two people the same distance from one
        // camera measure within a percent of each other, and which is
        // "largest" then flips with the detector's noise. Choosing afresh
        // each frame turned every flip into a cut, and the picture danced
        // between them for the whole clip.
        let samples: Vec<Vec<Face>> = (0..60)
            .map(|i| {
                let wobble = f64::from(i % 2) * 0.002;
                vec![seen(0.25, 0.100 + wobble), seen(0.75, 0.101 - wobble)]
            })
            .collect();
        let path = follow(&samples, sense());
        for (i, p) in path.iter().enumerate() {
            assert!(
                (p.0 - path[0].0).abs() < 1e-9,
                "sample {i}: the camera left {:?} for {p:?}",
                path[0]
            );
        }
    }

    #[test]
    fn a_real_change_of_speaker_cuts_but_only_once_it_insists() {
        // One guest leans in and fills the frame, and stays that way.
        let samples: Vec<Vec<Face>> = (0..20)
            .map(|_| vec![seen(0.25, 0.10), seen(0.75, 0.20)])
            .collect();
        let path = follow(&samples, sense());
        assert!((path[0].0 - 0.75).abs() < 1e-9, "starts on the larger one");

        // Now the other way round: the camera is already on the small one.
        let mut samples = vec![vec![seen(0.25, 0.10)]];
        samples.extend((0..20).map(|_| vec![seen(0.25, 0.10), seen(0.75, 0.20)]));
        let path = follow(&samples, sense());
        assert!((path[0].0 - 0.25).abs() < 1e-9);
        for (i, p) in path.iter().enumerate().take(INSIST) {
            assert!(
                (p.0 - 0.25).abs() < 1e-9,
                "sample {i} left too early: {p:?}"
            );
        }
        assert!(
            (path[INSIST].0 - 0.75).abs() < 1e-9,
            "should have cut by now: {:?}",
            path[INSIST]
        );
    }

    #[test]
    fn a_walk_is_followed_but_never_snapped_to() {
        // One face, starting at the left and then standing at the middle.
        let mut samples = vec![vec![seen(0.2, 0.12)]];
        samples.extend((0..60).map(|_| vec![seen(0.5, 0.12)]));
        let path = follow(&samples, sense());
        assert!((path[0].0 - 0.2).abs() < 1e-9);
        // A glide, not a jump: partway on the first step.
        assert!(path[1].0 > 0.2 && path[1].0 < 0.5, "{:?}", path[1]);
        // And arrived by the end.
        assert!((path.last().unwrap().0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_frame_with_nobody_in_it_leaves_the_camera_where_it_was() {
        let samples = vec![
            vec![seen(0.8, 0.12)],
            Vec::new(),
            Vec::new(),
            vec![seen(0.8, 0.12)],
        ];
        let path = follow(&samples, sense());
        assert_eq!(path[1], path[0]);
        assert_eq!(path[2], path[0]);
    }

    #[test]
    fn a_shaky_detector_does_not_shake_the_camera() {
        // The other half of the dancing picture, and the one left after
        // the subject stopped being chosen afresh: the box the detector
        // draws wanders a few pixels a frame on a person sat still, and
        // the camera was answering every wander.
        let mut seed = 12_345u64;
        let mut noise = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            f64::from((seed >> 40) as u32) / f64::from(1u32 << 24) - 0.5
        };
        // Two percent of the source is about twenty pixels of a 1920-wide
        // frame, which is more than YuNet actually wobbles by.
        let wander = 0.02;
        let samples: Vec<Vec<Face>> = (0..160)
            .map(|_| vec![seen(0.5 + noise() * wander, 0.12)])
            .collect();
        let raw: Vec<f64> = samples.iter().map(|f| f[0].centre().0).collect();
        let spread = |v: &[f64]| {
            v.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b))
                - v.iter().fold(f64::INFINITY, |a, b| a.min(*b))
        };
        // Worth checking the test tests something: on the screen the box
        // really does wander further than the camera is allowed to ignore,
        // and it is the median that swallows it.
        assert!(
            spread(&raw) * sense().0 > DEADZONE / 2.0,
            "the box barely moved; this proves nothing"
        );
        let path = follow(&samples, sense());
        let moved: Vec<f64> = path.iter().map(|p| p.0).collect();
        // Not "moved little" - did not move at all. The constants were
        // chosen by simulating exactly this.
        assert!(
            spread(&moved) < 1e-12,
            "the camera wandered {:.5} of the source while the box wandered {:.5}",
            spread(&moved),
            spread(&raw)
        );
    }

    #[test]
    fn the_subjects_size_is_the_usual_one_and_not_a_stray_frame() {
        let mut samples: Vec<Vec<Face>> = (0..20).map(|_| vec![seen(0.5, 0.12)]).collect();
        // One frame where the detector drew a box around half the room.
        samples.push(vec![seen(0.5, 0.60)]);
        assert!((subject_height(&samples) - 0.12).abs() < 1e-9);
        assert_eq!(subject_height(&[]), 0.0);
        assert_eq!(subject_height(&[Vec::new(), Vec::new()]), 0.0);
    }

    #[test]
    fn a_small_face_is_zoomed_to_but_never_past_the_limit() {
        let cover = cover_scale(WIDE, TALL);
        // At the covering scale the source stands exactly one frame tall,
        // so a face already the wanted size asks for no zoom at all.
        assert!((framing(FACE_HEIGHT, WIDE, TALL) - cover).abs() < 1e-9);
        // Half that size would want twice the zoom, and is capped.
        let s = framing(FACE_HEIGHT / 2.0, WIDE, TALL);
        assert!((s - cover * ZOOM_MAX).abs() < 1e-9, "{s}");
        // A face larger than wanted never zooms out past covering: that
        // would put black bars down the sides.
        assert!((framing(FACE_HEIGHT * 2.0, WIDE, TALL) - cover).abs() < 1e-9);
        // And no face at all is no zoom, not a division by zero.
        assert!((framing(0.0, WIDE, TALL) - cover).abs() < 1e-9);
    }

    #[test]
    fn a_zoomed_shot_puts_the_head_above_the_middle() {
        let scale = framing(FACE_HEIGHT / 2.0, WIDE, TALL);
        let centred = shot_for(0.5, 0.5, scale, WIDE, TALL);
        let placed = shot_placing(0.5, 0.5, (0.5, HEADROOM), scale, WIDE, TALL);
        assert!(placed.offset_y < centred.offset_y, "{placed:?} {centred:?}");
        // Higher, but never so high that the picture's edge shows.
        let (_, down) = travel(scale, WIDE, TALL);
        assert!(placed.offset_y.abs() <= down + 1e-9, "{placed:?}");
        // Sideways it is the plain shot: headroom is a vertical idea.
        assert!((placed.offset_x - centred.offset_x).abs() < 1e-9);
    }

    #[test]
    fn a_filled_tall_frame_only_answers_to_sideways_movement() {
        let (across, down) = sense();
        assert!(across > 3.0, "{across}");
        assert!(down.abs() < 1e-12, "nothing to see up and down: {down}");
        // Zoomed in there is room above and below, so it answers to both.
        let (across, down) = sensitivity(cover_scale(WIDE, TALL) * 1.2, WIDE, TALL);
        assert!(across > 3.0 && down > 0.0, "{across} {down}");
    }

    #[test]
    fn a_head_bobbing_up_and_down_does_not_move_the_camera_sideways() {
        // Vertical movement a wide-into-tall frame cannot show at all. It
        // used to count towards the distance that woke the camera, and
        // then the camera slid sideways for a nod.
        let samples: Vec<Vec<Face>> = (0..60)
            .map(|i| {
                let mut face = seen(0.4, 0.12);
                face.y += f64::from(i % 2) * 0.08;
                vec![face]
            })
            .collect();
        let path = follow(&samples, sense());
        for (i, p) in path.iter().enumerate() {
            assert!((p.0 - path[0].0).abs() < 1e-9, "sample {i} slid to {p:?}");
        }
    }

    /// A face at `u` whose mouth reads `mouth` this sample.
    fn lips(u: f64, mouth: f64) -> Face {
        Face {
            mouth,
            ..seen(u, 0.12)
        }
    }

    /// Six samples a second of two people sat at 0.25 and 0.75, where
    /// whoever has the floor has a mouth that moves and the other does not.
    ///
    /// `left_voice` is which voice the left-hand chair holds, so the same
    /// turns can be run with the two of them sat the other way round.
    fn two_speakers(turns: &[Turn], seconds: f64, left_voice: usize) -> Vec<Vec<Face>> {
        let rate = 6.0;
        let n = (seconds * rate) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / rate;
                let talking = turns
                    .iter()
                    .find(|turn| turn.start <= t && t < turn.end)
                    .map(|turn| turn.speaker);
                // A mouth that is working reads differently frame to frame;
                // a mouth at rest reads the same every time.
                let moving = if i % 2 == 0 { 0.85 } else { 1.0 };
                let left = talking == Some(left_voice);
                let right = talking.is_some() && !left;
                vec![
                    lips(0.25, if left { moving } else { 0.85 }),
                    lips(0.75, if right { moving } else { 0.85 }),
                ]
            })
            .collect()
    }

    #[test]
    fn the_usual_number_of_faces_is_the_commonest_one() {
        let mut samples = vec![vec![lips(0.25, 0.85), lips(0.75, 0.85)]; 40];
        // One frame where the detector found a face in the bookshelf.
        samples.push(vec![lips(0.1, 0.85), lips(0.5, 0.85), lips(0.9, 0.85)]);
        samples.push(Vec::new());
        assert_eq!(crowd(&samples), 2);
        assert_eq!(places(&samples).len(), 2);
        assert!((places(&samples)[0] - 0.25).abs() < 1e-9);
        assert!((places(&samples)[1] - 0.75).abs() < 1e-9);
        assert_eq!(crowd(&[]), 0);
    }

    #[test]
    fn each_voice_is_matched_to_the_face_that_was_moving() {
        // Voice 0 has the floor first, voice 1 second. The left-hand face
        // moves during the first stretch, the right-hand one during the
        // second, so voice 0 is the left face and voice 1 the right.
        let turns = [
            Turn {
                start: 0.0,
                end: 10.0,
                speaker: 0,
            },
            Turn {
                start: 10.0,
                end: 20.0,
                speaker: 1,
            },
        ];
        let samples = two_speakers(&turns, 20.0, 0);
        assert_eq!(bind(&samples, &turns, 6.0), vec![0, 1]);

        // The same turns with the two of them sat the other way round. Only
        // the mouths say so - the audio is identical - so an answer that
        // merely handed out chairs in order would not change here, and this
        // is the assertion that would catch it.
        let samples = two_speakers(&turns, 20.0, 1);
        assert_eq!(bind(&samples, &turns, 6.0), vec![1, 0]);
    }

    #[test]
    fn it_admits_when_it_cannot_tell() {
        let turns = [
            Turn {
                start: 0.0,
                end: 10.0,
                speaker: 0,
            },
            Turn {
                start: 10.0,
                end: 20.0,
                speaker: 1,
            },
        ];
        // One face on screen: nothing to choose between.
        let alone: Vec<Vec<Face>> = (0..120).map(|_| vec![lips(0.5, 0.9)]).collect();
        assert!(bind(&alone, &turns, 6.0).is_empty());
        // Two faces but only one voice: no comparison to draw.
        let one_voice = [Turn {
            start: 0.0,
            end: 20.0,
            speaker: 0,
        }];
        assert!(bind(&two_speakers(&turns, 20.0, 0), &one_voice, 6.0).is_empty());
        // No audio at all.
        assert!(bind(&two_speakers(&turns, 20.0, 0), &[], 6.0).is_empty());
    }

    #[test]
    fn the_camera_cuts_to_whoever_takes_the_floor() {
        let turns = [
            Turn {
                start: 0.0,
                end: 10.0,
                speaker: 0,
            },
            Turn {
                start: 10.0,
                end: 20.0,
                speaker: 1,
            },
        ];
        let samples = two_speakers(&turns, 20.0, 0);
        let path = follow_speaking(&samples, sense(), &turns, 6.0);
        // On the left-hand speaker while they hold the floor.
        assert!((path[30].0 - 0.25).abs() < 0.01, "{:?}", path[30]);
        // And on the right-hand one within a sample of the handover - a
        // cut, because gliding across the frame for a change of speaker
        // looks like the camera lost them.
        let handover = (10.0 * 6.0) as usize;
        assert!(
            (path[handover].0 - 0.75).abs() < 0.01,
            "still at {:?} after the handover",
            path[handover]
        );
        // Two shots, not a drift between them.
        let moves = path
            .windows(2)
            .filter(|pair| (pair[1].0 - pair[0].0).abs() > 1e-9)
            .count();
        assert!(moves <= 2, "the camera moved {moves} times, not once");
    }

    #[test]
    fn a_two_second_interjection_does_not_move_the_camera() {
        // The diarizer marks every "mhm". Cutting away for one and back is
        // the twitch that makes an automatic edit obvious.
        let turns = [
            Turn {
                start: 0.0,
                end: 8.0,
                speaker: 0,
            },
            Turn {
                start: 8.0,
                end: 8.4,
                speaker: 1,
            },
            Turn {
                start: 8.4,
                end: 16.0,
                speaker: 0,
            },
        ];
        let samples = two_speakers(&turns, 16.0, 0);
        let path = follow_speaking(&samples, sense(), &turns, 6.0);
        for (i, p) in path.iter().enumerate() {
            assert!(
                (p.0 - 0.25).abs() < 0.01,
                "sample {i} left the speaker for {p:?}"
            );
        }
    }

    #[test]
    fn without_audio_it_behaves_exactly_as_before() {
        let turns = [
            Turn {
                start: 0.0,
                end: 10.0,
                speaker: 0,
            },
            Turn {
                start: 10.0,
                end: 20.0,
                speaker: 1,
            },
        ];
        let samples = two_speakers(&turns, 20.0, 0);
        assert_eq!(
            follow_speaking(&samples, sense(), &[], 6.0),
            follow(&samples, sense())
        );
    }

    #[test]
    fn a_still_camera_reduces_to_its_two_ends() {
        let path = vec![(0.5, 0.5); 300];
        assert_eq!(reduce(&path, 0.002), vec![0, 299]);
    }

    #[test]
    fn a_pan_keeps_the_points_that_carry_its_shape() {
        // Straight travel needs no middle keys.
        let straight: Vec<_> = (0..=100).map(|i| (i as f64 / 100.0, 0.5)).collect();
        assert_eq!(reduce(&straight, 0.002), vec![0, 100]);
        // A corner has to survive.
        let mut bent: Vec<(f64, f64)> = (0..=50).map(|i| (i as f64 / 100.0, 0.5)).collect();
        bent.extend((51..=100).map(|i| (0.5, 0.5 + (i - 50) as f64 / 100.0)));
        let kept = reduce(&bent, 0.002);
        assert!(kept.len() >= 3 && kept.len() < 20, "kept {kept:?}");
        assert!(kept.contains(&0) && kept.contains(&100));
    }

    #[test]
    fn two_points_or_fewer_are_all_kept() {
        assert_eq!(reduce(&[], 0.01), Vec::<usize>::new());
        assert_eq!(reduce(&[(0.5, 0.5)], 0.01), vec![0]);
        assert_eq!(reduce(&[(0.1, 0.1), (0.9, 0.9)], 0.01), vec![0, 1]);
    }
}
