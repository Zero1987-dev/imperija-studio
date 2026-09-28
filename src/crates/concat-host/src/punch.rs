// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Where a shot should push in.
//!
//! The single most recognisable move in the whole format: somebody makes a
//! point and the frame is suddenly closer on them, then eases back. Done by
//! hand it is two keyframes per sentence and nobody does it for a
//! twelve-minute podcast; done here it is one pass over the sound.
//!
//! **Read from loudness, not from the words.** A person leans on what they
//! mean, and that lean is louder than the line around it. Loudness needs no
//! model, no download and no language, so a clip in Bosnian is punched the
//! same as one in English and a clip with no transcript at all is punched
//! just the same.
//!
//! What comes back is a list of moments. Turning those into keys is the
//! window's business, because the scale a clip is already at is the window's
//! business.

/// The stretch of sound one loudness reading covers.
///
/// Fifty milliseconds: shorter than any syllable worth noticing and long
/// enough that one glottal click is not a reading of its own.
pub const WINDOW: f64 = 0.05;

/// How many readings are taken together to decide a moment is loud.
///
/// A fifth of a second. An emphasis is a word leaned on, not a spike.
pub const SPAN: usize = 4;

/// How much louder than the clip's own middle a moment has to be.
///
/// Measured against the median rather than the mean, so a clip with a
/// stretch of silence in it is judged by its speech and not by its gaps.
/// Half again is about where a leaned-on word sits above a level one.
pub const EMPHASIS: f64 = 1.5;

/// And how far apart two pushes have to be.
///
/// Pushing in every other sentence is a tic rather than an emphasis, and
/// the eye stops reading it as meaning anything. Four seconds is roughly
/// the shortest gap that still looks deliberate.
pub const APART: f64 = 4.0;

/// How long a push holds before it lets go.
pub const HOLD: f64 = 2.2;

/// How much closer the frame goes.
///
/// Twelve percent. Enough to be felt on a phone held at arm's length, and
/// small enough not to soften a picture that is very likely being enlarged
/// already - a tall crop of a wide shot is an enlargement before any of
/// this happens.
pub const PUNCH: f64 = 1.12;

/// How long the frame takes to arrive.
///
/// Near enough to nothing. The move reads as a cut rather than as a zoom,
/// which is the difference between it landing on the word and drifting
/// past it.
pub const SNAP: f64 = 0.08;

/// And how long it takes to leave, which is not the same thing at all: the
/// way out is a release, and a release that snaps is a flinch.
pub const EASE_OUT: f64 = 0.45;

/// A moment the shot should be closer on, in seconds from the start of the
/// window that was read.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Beat {
    /// When the frame arrives closer.
    pub at: f64,
    /// And when it has finished easing back out.
    pub until: f64,
}

/// The loudness of each [`WINDOW`] of the sound, in order.
///
/// Root mean square, which is what the ear is nearer to than to the peak:
/// one sharp click in a quiet passage is not a loud passage.
pub fn loudness(samples: &[f32], rate: u32) -> Vec<f64> {
    let span = ((f64::from(rate) * WINDOW).round() as usize).max(1);
    samples
        .chunks(span)
        .map(|chunk| {
            let sum: f64 = chunk.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
            (sum / chunk.len() as f64).sqrt()
        })
        .collect()
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

/// The moments worth pushing in on.
///
/// A run of [`SPAN`] readings averaging [`EMPHASIS`] above the clip's own
/// median is a lean; the first such run after a gap of [`APART`] is a push.
/// Everything is relative to this clip, so a quiet recording and a loud one
/// are punched the same number of times.
pub fn beats(samples: &[f32], rate: u32, seconds: f64) -> Vec<Beat> {
    if rate == 0 || seconds <= 0.0 {
        return Vec::new();
    }
    let level = loudness(samples, rate);
    if level.len() < SPAN {
        return Vec::new();
    }
    // The middle of the *speech*, not of the recording: readings at or near
    // nothing are silence between words and would drag the middle down.
    let speaking: Vec<f64> = level.iter().copied().filter(|v| *v > 1e-4).collect();
    let middle = median(speaking);
    if middle <= 0.0 {
        return Vec::new();
    }

    let mut beats: Vec<Beat> = Vec::new();
    let mut at = 0usize;
    while at + SPAN <= level.len() {
        let mean = level[at..at + SPAN].iter().sum::<f64>() / SPAN as f64;
        if mean < middle * EMPHASIS {
            at += 1;
            continue;
        }
        let when = at as f64 * WINDOW;
        // A push that would land on the tail of the clip has nowhere to
        // ease back out, and one too near the last is a tic.
        if when + HOLD > seconds || beats.last().is_some_and(|last| when - last.at < APART) {
            at += 1;
            continue;
        }
        beats.push(Beat {
            at: when,
            until: (when + HOLD).min(seconds),
        });
        // Past the hold before looking again, so one long shout is one push.
        at += ((HOLD / WINDOW).round() as usize).max(1);
    }
    beats
}

/// The beats in one window of one file.
///
/// `start` and `duration` are seconds into the file, the same window the
/// clip uses; the beats that come back are relative to that window.
pub fn beats_of(
    path: &str,
    start: f64,
    duration: f64,
    audio_stream: Option<usize>,
) -> Result<Vec<Beat>, String> {
    use concat_media::{AudioDecoder, AudioOptions, SampleFormat};

    if duration <= 0.0 {
        return Ok(Vec::new());
    }
    // Sixteen thousand is plenty: loudness is an envelope, not a waveform,
    // and mono because a push is a decision about the whole picture.
    const RATE: u32 = 16_000;
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            start: Some(start),
            duration: Some(duration),
            filters: Vec::new(),
            rate: RATE,
            channels: 1,
            format: SampleFormat::F32,
            stream: audio_stream,
        },
    )
    .map_err(|error| error.to_string())?;
    let samples = decoder.collect_f32().map_err(|error| error.to_string())?;
    Ok(beats(&samples, RATE, duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sound at `rate` for `seconds`, quiet except where `loud` says.
    fn speech(rate: u32, seconds: f64, loud: &[(f64, f64)]) -> Vec<f32> {
        let n = (seconds * f64::from(rate)).round() as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                let level = if loud.iter().any(|(from, to)| t >= *from && t < *to) {
                    0.5
                } else {
                    0.15
                };
                // A tone rather than noise, so the loudness is exactly what
                // the level says and the test is about the rule, not the
                // waveform.
                (level * (std::f64::consts::TAU * 200.0 * t).sin()) as f32
            })
            .collect()
    }

    #[test]
    fn a_leaned_on_word_is_a_push_and_level_speech_is_not() {
        let rate = 16_000;
        let quiet = speech(rate, 20.0, &[]);
        assert!(beats(&quiet, rate, 20.0).is_empty(), "level speech punched");

        let with_a_lean = speech(rate, 20.0, &[(6.0, 6.6)]);
        let found = beats(&with_a_lean, rate, 20.0);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!((found[0].at - 6.0).abs() < 0.2, "{:?}", found[0]);
        assert!(
            (found[0].until - (6.0 + HOLD)).abs() < 0.2,
            "{:?}",
            found[0]
        );
    }

    #[test]
    fn two_leans_close_together_are_one_push() {
        let rate = 16_000;
        // Half a second apart: one thought, leaned on twice.
        let samples = speech(rate, 20.0, &[(5.0, 5.4), (5.9, 6.3)]);
        let found = beats(&samples, rate, 20.0);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn leans_far_apart_are_both_pushes() {
        let rate = 16_000;
        let samples = speech(rate, 30.0, &[(4.0, 4.6), (14.0, 14.6), (24.0, 24.6)]);
        let found = beats(&samples, rate, 30.0);
        assert_eq!(found.len(), 3, "{found:?}");
        for pair in found.windows(2) {
            assert!(pair[1].at - pair[0].at >= APART, "{pair:?}");
        }
    }

    #[test]
    fn a_lean_with_no_room_left_to_ease_out_of_is_left_alone() {
        let rate = 16_000;
        // Loud right at the end: there is nowhere to come back from.
        let samples = speech(rate, 8.0, &[(7.4, 8.0)]);
        assert!(beats(&samples, rate, 8.0).is_empty());
    }

    #[test]
    fn a_quiet_recording_is_punched_like_a_loud_one() {
        // Everything relative to the clip's own middle, so the same shape
        // of performance answers the same however it was recorded.
        let rate = 16_000;
        let loud = speech(rate, 20.0, &[(6.0, 6.6)]);
        let quiet: Vec<f32> = loud.iter().map(|s| s * 0.05).collect();
        assert_eq!(beats(&loud, rate, 20.0), beats(&quiet, rate, 20.0));
    }

    #[test]
    fn nothing_to_read_is_no_pushes_rather_than_a_panic() {
        assert!(beats(&[], 16_000, 10.0).is_empty());
        assert!(beats(&[0.0; 1000], 16_000, 10.0).is_empty());
        assert!(beats(&[0.5; 16_000], 0, 10.0).is_empty());
        assert!(beats(&[0.5; 16_000], 16_000, 0.0).is_empty());
    }

    #[test]
    fn loudness_is_the_level_it_was_given() {
        let rate = 16_000;
        let steady: Vec<f32> = (0..rate).map(|_| 0.5f32).collect();
        for reading in loudness(&steady, rate) {
            assert!((reading - 0.5).abs() < 1e-6, "{reading}");
        }
        assert_eq!(loudness(&[], rate).len(), 0);
    }
}
