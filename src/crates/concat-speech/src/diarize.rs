// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Who is speaking, and when.
//!
//! Two networks through sherpa-onnx, entirely on this machine. The first
//! cuts the recording into stretches of one voice; the second turns each
//! stretch into the numbers that identify a voice, so stretches of the same
//! person can be gathered into one pile. What comes back is a list of
//! [`Turn`]s: from here to there, voice number two.
//!
//! **It numbers voices, it does not name them, and it has never seen the
//! picture.** "Voice 0" is whoever the clustering happened to put first,
//! and nothing here knows which face on screen that is. Reframing joins the
//! two with `concat_vision::reframe::bind`, which asks whose mouth was
//! working while each voice held the floor.
//!
//! Called from inside the reframe job rather than being a job of its own:
//! it is one step of reframing a clip, and reframing already refuses to run
//! twice at once.

use std::path::Path;

use concat_media::{AudioDecoder, AudioOptions, SampleFormat};
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};

/// One stretch of a recording in which one voice was speaking.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Turn {
    /// Seconds from the start of the window that was read.
    pub start: f64,
    /// Seconds from the start of the window that was read.
    pub end: f64,
    /// Which voice, numbered from zero.
    pub speaker: usize,
}

/// How near two voices have to measure before they are taken for one
/// person.
///
/// sherpa-onnx's own recommendation for this pair of networks. Lower splits
/// one person into two when their voice changes - shouting, laughing - and
/// higher folds two people into one, which for our purposes is the worse
/// of the two: a reframe that thinks there is one speaker simply falls back
/// to following the largest face, which is where it started.
pub const SAME_VOICE: f32 = 0.5;

/// Speech shorter than this is not a turn, it is a noise.
pub const SHORTEST_SPEECH: f32 = 0.3;

/// And a gap shorter than this is not a silence, it is a breath.
pub const SHORTEST_GAP: f32 = 0.5;

/// How many voices to look for, or `None` to let the clustering decide from
/// [`SAME_VOICE`].
///
/// A podcast is usually two, but saying so would be a guess about somebody
/// else's video, and being wrong about it is worse than not knowing: told
/// there are two, the clustering will find two in a monologue.
pub const HOW_MANY: Option<i32> = None;

/// The turns in one window of one file.
///
/// `start` and `duration` are seconds into the file, the same window the
/// clip uses; the turns that come back are relative to that window, so a
/// clip that begins forty minutes into a podcast still counts from zero.
///
/// An empty answer is the ordinary answer for a clip with one speaker, no
/// speech, or no audio at all, and means the caller should carry on without
/// it rather than stop.
pub fn turns(
    path: &str,
    start: f64,
    duration: f64,
    audio_stream: Option<usize>,
    segmentation: &Path,
    voiceprint: &Path,
) -> Result<Vec<Turn>, String> {
    if duration <= 0.0 {
        return Ok(Vec::new());
    }
    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(segmentation.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        },
        embedding: SpeakerEmbeddingExtractorConfig {
            model: Some(voiceprint.to_string_lossy().into_owned()),
            ..Default::default()
        },
        clustering: FastClusteringConfig {
            num_clusters: HOW_MANY.unwrap_or(-1),
            threshold: SAME_VOICE,
            ..Default::default()
        },
        min_duration_on: SHORTEST_SPEECH,
        min_duration_off: SHORTEST_GAP,
    };
    let diarizer =
        OfflineSpeakerDiarization::create(&config).ok_or("the speaker models would not load")?;

    // At the rate the networks were trained for, whatever that is, rather
    // than at one assumed here.
    let rate = diarizer.sample_rate();
    if rate <= 0 {
        return Err("the speaker models report no sample rate".to_owned());
    }
    let mut decoder = AudioDecoder::open(
        path,
        &AudioOptions {
            start: Some(start),
            duration: Some(duration),
            filters: Vec::new(),
            rate: rate as u32,
            channels: 1,
            format: SampleFormat::F32,
            stream: audio_stream,
        },
    )
    .map_err(|error| error.to_string())?;
    let samples = decoder.collect_f32().map_err(|error| error.to_string())?;
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let Some(found) = diarizer.process(&samples) else {
        return Ok(Vec::new());
    };
    Ok(found
        .sort_by_start_time()
        .into_iter()
        .map(|segment| Turn {
            start: f64::from(segment.start),
            end: f64::from(segment.end),
            speaker: segment.speaker.max(0) as usize,
        })
        .collect())
}

/// How many different voices a set of turns holds.
pub fn voices(turns: &[Turn]) -> usize {
    turns.iter().map(|turn| turn.speaker + 1).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_of_no_length_is_not_worth_opening_a_file_for() {
        let answer = turns(
            "/nowhere.mp4",
            0.0,
            0.0,
            None,
            Path::new("/nowhere-a.onnx"),
            Path::new("/nowhere-b.onnx"),
        );
        assert_eq!(answer, Ok(Vec::new()));
    }

    #[test]
    fn counting_voices_ignores_how_they_are_ordered() {
        let turns = [
            Turn {
                start: 0.0,
                end: 1.0,
                speaker: 1,
            },
            Turn {
                start: 1.0,
                end: 2.0,
                speaker: 0,
            },
            Turn {
                start: 2.0,
                end: 3.0,
                speaker: 1,
            },
        ];
        assert_eq!(voices(&turns), 2);
        assert_eq!(voices(&[]), 0);
    }
}
