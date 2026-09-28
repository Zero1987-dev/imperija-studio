// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Where the speaking stops.
//!
//! Dead air is what makes a clip feel long. A podcast is recorded at the
//! pace of a conversation and posted at the pace of a feed, and the whole
//! difference is often nothing more than the pauses taken out - the same
//! words, a third less time, and it holds.
//!
//! **Read from loudness, like [`super::punch`], and for the same reasons.**
//! No model to download, no transcript to have, no language to be in.
//!
//! What comes back is the stretches worth removing. Removing them is the
//! window's business, because what else is on the timeline at that moment -
//! captions, a sound effect, music - is the window's business, and all of
//! it has to move together or the clip comes apart.

use super::punch::{WINDOW, loudness};

/// How quiet a stretch has to be, against the clip's own speech.
///
/// A third of the middle of the speaking. Not silence in the absolute -
/// a room has a floor, a microphone has a hiss, and asking for true
/// silence finds none.
pub const QUIET: f64 = 0.33;

/// And how long, before it is worth cutting.
///
/// Under this it is a breath, and a conversation with its breaths taken
/// out sounds like a machine reading. Half a second is about where a pause
/// stops being part of the sentence.
pub const SHORTEST: f64 = 0.5;

/// How much of the quiet is left at each end.
///
/// The consonant at the end of a word is quieter than the vowel before it,
/// so a cut placed where the loudness falls lands slightly inside the word
/// and clips it. Leaving a tenth of a second at each end costs nothing and
/// saves every "t" in the clip.
pub const KEEP: f64 = 0.1;

/// A stretch worth removing, in seconds from the start of the window read.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Gap {
    /// Where the cut starts.
    pub from: f64,
    /// And ends.
    pub to: f64,
}

impl Gap {
    /// How much time it takes out.
    pub fn length(&self) -> f64 {
        (self.to - self.from).max(0.0)
    }
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

/// The stretches of quiet worth cutting out.
pub fn gaps(samples: &[f32], rate: u32, seconds: f64) -> Vec<Gap> {
    if rate == 0 || seconds <= 0.0 {
        return Vec::new();
    }
    let level = loudness(samples, rate);
    if level.is_empty() {
        return Vec::new();
    }
    // The middle of the speaking, so the threshold follows the recording
    // rather than a number decided here.
    let speaking: Vec<f64> = level.iter().copied().filter(|v| *v > 1e-5).collect();
    let floor = median(speaking) * QUIET;
    if floor <= 0.0 {
        return Vec::new();
    }

    let mut gaps = Vec::new();
    let mut run: Option<usize> = None;
    for (i, reading) in level.iter().enumerate() {
        if *reading < floor {
            run.get_or_insert(i);
            continue;
        }
        if let Some(start) = run.take() {
            push_gap(&mut gaps, start, i, seconds);
        }
    }
    if let Some(start) = run {
        push_gap(&mut gaps, start, level.len(), seconds);
    }
    gaps
}

/// One run of quiet readings, as a gap - once it is long enough to be one,
/// and pulled in at both ends so no word is clipped.
fn push_gap(gaps: &mut Vec<Gap>, start: usize, end: usize, seconds: f64) {
    let from = start as f64 * WINDOW + KEEP;
    let to = (end as f64 * WINDOW - KEEP).min(seconds);
    if to - from >= SHORTEST {
        gaps.push(Gap { from, to });
    }
}

/// The gaps in one window of one file.
///
/// `start` and `duration` are seconds into the file, the same window the
/// clip uses; the gaps that come back are relative to that window.
pub fn gaps_of(
    path: &str,
    start: f64,
    duration: f64,
    audio_stream: Option<usize>,
) -> Result<Vec<Gap>, String> {
    use concat_media::{AudioDecoder, AudioOptions, SampleFormat};

    if duration <= 0.0 {
        return Ok(Vec::new());
    }
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
    Ok(gaps(&samples, RATE, duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech at `rate` for `seconds`, silent where `quiet` says.
    fn talk(rate: u32, seconds: f64, quiet: &[(f64, f64)]) -> Vec<f32> {
        let n = (seconds * f64::from(rate)).round() as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                // A room floor rather than true silence, because that is
                // what a recording has and what this has to see past.
                let level = if quiet.iter().any(|(a, b)| t >= *a && t < *b) {
                    0.01
                } else {
                    0.3
                };
                (level * (std::f64::consts::TAU * 180.0 * t).sin()) as f32
            })
            .collect()
    }

    #[test]
    fn a_real_pause_is_a_cut_and_a_breath_is_not() {
        let rate = 16_000;
        // Two seconds of quiet in the middle, and a quarter-second breath.
        let samples = talk(rate, 20.0, &[(5.0, 7.0), (12.0, 12.25)]);
        let found = gaps(&samples, rate, 20.0);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].from > 5.0 && found[0].from < 5.3, "{:?}", found[0]);
        assert!(found[0].to > 6.7 && found[0].to < 7.0, "{:?}", found[0]);
    }

    #[test]
    fn the_cut_stops_short_of_the_words_at_both_ends() {
        let rate = 16_000;
        let samples = talk(rate, 20.0, &[(8.0, 10.0)]);
        let found = gaps(&samples, rate, 20.0);
        assert_eq!(found.len(), 1);
        // Inside the quiet, never into the speech either side.
        assert!(found[0].from >= 8.0, "{:?}", found[0]);
        assert!(found[0].to <= 10.0, "{:?}", found[0]);
        // And by about the allowance, not by half the pause.
        assert!(
            (found[0].from - (8.0 + KEEP)).abs() < 0.12,
            "{:?}",
            found[0]
        );
    }

    #[test]
    fn unbroken_speech_has_nothing_to_cut() {
        let rate = 16_000;
        assert!(gaps(&talk(rate, 15.0, &[]), rate, 15.0).is_empty());
    }

    #[test]
    fn several_pauses_come_back_in_order_and_do_not_overlap() {
        let rate = 16_000;
        let samples = talk(rate, 30.0, &[(4.0, 5.5), (11.0, 13.0), (20.0, 21.2)]);
        let found = gaps(&samples, rate, 30.0);
        assert_eq!(found.len(), 3, "{found:?}");
        for pair in found.windows(2) {
            assert!(pair[0].to < pair[1].from, "{pair:?}");
        }
        let saved: f64 = found.iter().map(Gap::length).sum();
        assert!(saved > 3.0 && saved < 4.7, "takes out {saved:.2}s");
    }

    #[test]
    fn a_quiet_recording_is_cut_like_a_loud_one() {
        let rate = 16_000;
        let loud = talk(rate, 20.0, &[(6.0, 8.0)]);
        let quiet: Vec<f32> = loud.iter().map(|s| s * 0.04).collect();
        assert_eq!(gaps(&loud, rate, 20.0), gaps(&quiet, rate, 20.0));
    }

    #[test]
    fn nothing_to_read_is_no_cuts_rather_than_a_panic() {
        assert!(gaps(&[], 16_000, 10.0).is_empty());
        assert!(gaps(&[0.0; 16_000], 16_000, 1.0).is_empty());
        assert!(gaps(&[0.3; 16_000], 0, 1.0).is_empty());
        assert!(gaps(&[0.3; 16_000], 16_000, 0.0).is_empty());
    }
}
