// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Reframing a clip: a wide shot read once, and keys that follow the face.
//!
//! A podcast is shot wide and posted tall, and doing that by hand is a
//! keyframe every time someone leans. The job here reads the clip through
//! at [`SAMPLE_RATE`] frames a second, asks [`concat_vision::face`] where
//! the faces are, and hands back the `scale`, `offsetX` and `offsetY` keys
//! that put the speaker in the middle of a tall frame.
//!
//! It writes no file and touches no document. The window takes the keys it
//! answers with and applies them as an ordinary edit, so the reframe lands
//! in the undo stack, shows up in the Keyframes tab, and can be dragged
//! about afterwards. A tracker that puts the camera somewhere silly is then
//! a key to move rather than a result to throw away.
//!
//! One job at a time through a [`SingleFlight`], like every long job the
//! host runs, with the model fetched on first use and kept loaded.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use concat_core::{FrameRate, Rational};
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_vision::face::Detector;
use concat_vision::reframe::{self, Shot};
use concat_vision::{ModelId, models};

pub use crate::cutout::Progress;
use crate::cutout::fetch;
use crate::jobs::SingleFlight;

/// Faces are looked for this many times a second of source.
///
/// A head does not move the way an outline does, so this is well under the
/// mask rate: six a second is a look every 167ms, which is finer than the
/// camera's own easing can answer anyway, and a ten-minute podcast is three
/// and a half thousand inferences rather than six thousand.
pub const SAMPLE_RATE: u32 = 6;

/// How far a key may sit from the straight line between its neighbours
/// before it has to be kept, in frame widths.
///
/// A hundredth of the frame is under two pixels at 1080 wide - past what
/// anyone sees in a moving picture, and enough to turn a still camera into
/// two keys instead of thousands.
pub const KEY_TOLERANCE: f64 = 0.01;

/// What one reframe covers.
#[derive(Clone, Debug)]
pub struct ReframeRequest {
    /// The media file to read.
    pub media_path: String,
    /// Where the clip starts in that file, in seconds.
    pub start: f64,
    /// How much of it the clip uses, in seconds.
    pub duration: f64,
    /// The source picture's shape, width over height.
    pub source_aspect: f64,
    /// The shape being exported to, width over height - 0.5625 for 9:16.
    pub frame_aspect: f64,
    /// Who was speaking and when, in seconds from the clip's own start.
    ///
    /// Empty is the ordinary case and means the camera falls back to
    /// following whoever's face is largest. Filled in by the caller rather
    /// than worked out here: hearing voices apart needs sherpa-onnx, which
    /// is deliberately not a dependency of this crate, so the window asks
    /// `concat_speech::diarize` and hands the answer over. Getting the two
    /// models it needs is [`Reframers::speaker_models`].
    pub turns: Vec<reframe::Turn>,
}

/// One key the window applies: where in the clip, and the camera there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReframeKey {
    /// Where in the clip, as a fraction of its length, `0..=1`.
    pub at: f64,
    /// The camera at that moment.
    pub shot: Shot,
}

/// The reframing service: the model, loaded once, and the one-job slot.
pub struct Reframers {
    gate: Arc<SingleFlight>,
    /// The app's data directory, where the downloaded model lives.
    data: PathBuf,
    model: Mutex<Option<Arc<Mutex<Detector>>>>,
}

impl Reframers {
    /// A service with nothing loaded yet, keeping its model under `data`.
    pub fn new(data: &Path) -> Reframers {
        Reframers {
            gate: Arc::new(SingleFlight::new()),
            data: data.to_path_buf(),
            model: Mutex::new(None),
        }
    }

    /// Whether a reframe is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running reframe to stop after the frame in hand.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// The two networks that say who is speaking, fetched on first use and
    /// left on disk.
    ///
    /// Not loaded here, only downloaded: they are run through sherpa-onnx,
    /// which this crate does not link. The caller loads them.
    pub fn speaker_models(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(PathBuf, PathBuf), String> {
        let segmentation = models::model_file(&self.data, ModelId::Speakers);
        if !models::installed(&self.data, ModelId::Speakers) {
            fetch(ModelId::Speakers, &segmentation, cancel, progress)?;
        }
        let voiceprint = models::model_file(&self.data, ModelId::Voiceprint);
        if !models::installed(&self.data, ModelId::Voiceprint) {
            fetch(ModelId::Voiceprint, &voiceprint, cancel, progress)?;
        }
        Ok((segmentation, voiceprint))
    }

    /// The detector, fetched on first use and kept.
    fn detector(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Arc<Mutex<Detector>>, String> {
        if let Some(loaded) = self
            .model
            .lock()
            .map_err(|_| "model slot poisoned")?
            .as_ref()
        {
            return Ok(Arc::clone(loaded));
        }
        let file = models::model_file(&self.data, ModelId::Face);
        if !models::installed(&self.data, ModelId::Face) {
            fetch(ModelId::Face, &file, cancel, progress)?;
        }
        let detector = Arc::new(Mutex::new(Detector::from_file(&file)?));
        *self.model.lock().map_err(|_| "model slot poisoned")? = Some(Arc::clone(&detector));
        Ok(detector)
    }

    /// Reads the clip through and answers with the keys that follow the
    /// speaker. An empty answer means no face was found anywhere in it,
    /// which the window should say rather than silently do nothing.
    pub fn reframe(
        &self,
        request: &ReframeRequest,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<ReframeKey>, String> {
        let job = self.gate.begin("reframe")?;
        let cancel = job.cancel_handle();
        let detector = self.detector(&cancel, progress)?;

        let samples = ((request.duration * f64::from(SAMPLE_RATE)).round() as usize).max(1);
        progress(Progress::Analysing(0.0));

        // Decoded at the model's own input width: the detector letterboxes
        // into 640 x 640 regardless, so anything larger is read and thrown
        // away. The aspect is the source's, and `fit` puts the bars in.
        let options = DecodeOptions::default()
            .starting_at(Rational::new((request.start * 1000.0) as i64, 1000))
            .scaled_to(
                concat_vision::face::INPUT_W as u32,
                ((concat_vision::face::INPUT_W as f64 / request.source_aspect.max(0.01)).round()
                    as u32)
                    .max(1),
            )
            .at_rate(FrameRate::new(Rational::new(i64::from(SAMPLE_RATE), 1)))
            .limited_to(samples as u64);
        let mut decoder =
            Decoder::open(&request.media_path, &options).map_err(|error| error.to_string())?;

        // Every face, not just the one picked here: choosing the subject is
        // the camera's own job and it needs memory of whom it was watching.
        let mut seen: Vec<Vec<concat_vision::reframe::Face>> = Vec::with_capacity(samples);
        while seen.len() < samples {
            if job.cancelled() {
                return Err("reframe cancelled".to_owned());
            }
            let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? else {
                // The file ran out before its stated length: what was read
                // is what there is.
                break;
            };
            let faces = detector
                .lock()
                .map_err(|_| "detector poisoned")?
                .faces(&frame)?;
            seen.push(faces);
            progress(Progress::Analysing(seen.len() as f32 / samples as f32));
        }
        progress(Progress::Analysing(1.0));

        if seen.iter().all(Vec::is_empty) {
            return Ok(Vec::new());
        }

        // How far in to go, from how big the face actually is. Merely
        // covering the frame is the widest legal shot and leaves a podcast
        // face tiny in a tall picture, which is what makes an automatic
        // reframe look like a crop rather than a shot.
        let scale = reframe::framing(
            reframe::subject_height(&seen),
            request.source_aspect,
            request.frame_aspect,
        );
        // The camera judges drift on the exported frame, so it has to know
        // how much of that frame a step in the source crosses.
        let sensitivity = reframe::sensitivity(scale, request.source_aspect, request.frame_aspect);
        // Who has the floor when the audio said so, and whose face is
        // largest when it did not.
        let path =
            reframe::follow_speaking(&seen, sensitivity, &request.turns, f64::from(SAMPLE_RATE));
        // Reduced on the camera's path rather than on the offsets, so the
        // tolerance means the same thing whatever the zoom: a fraction of
        // the source, not of a number that grows with `scale`.
        let last = path.len().saturating_sub(1);
        Ok(reframe::reduce(&path, KEY_TOLERANCE / scale.max(1.0))
            .into_iter()
            .map(|i| ReframeKey {
                at: if last == 0 {
                    0.0
                } else {
                    i as f64 / last as f64
                },
                shot: reframe::shot_placing(
                    path[i].0,
                    path[i].1,
                    (0.5, reframe::HEADROOM),
                    scale,
                    request.source_aspect,
                    request.frame_aspect,
                ),
            })
            .collect())
    }
}
