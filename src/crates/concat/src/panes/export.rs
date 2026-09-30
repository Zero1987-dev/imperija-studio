// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The export sheet: the form, the running render, and the result.
//!
//! The first pane cut out of the window's controller, and the shape every
//! later one follows. The pane owns its state; every way it can change -
//! a field edited, a button pressed, a worker reporting - is one
//! [`ExportMsg`]; [`ExportPane::update`] is the only code that changes the
//! state; and [`ExportPane::data`] is the one place its Slint rows are
//! built. The controller it needs for context - the session, the output
//! size, the job slots - is handed to `update` as the studio, which the
//! pane reads and asks things of but never reaches into for its own
//! fields.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use concat_host::export::{self, ExportSpec};
use concat_host::review::{self, Kind, Note, Weight};
use concat_media::ColorRange;

use crate::format::{bytes, eta};
use crate::host::{on_ui, spawn};
use crate::i18n::{self, t, tf};
use crate::panes::Msg;
use crate::platform;
use crate::studio::{
    AUDIO_BPS, EXPORT_CRF, EXPORT_RATES, EXPORT_SHORT_SIDES, EXPORT_TIERS, Studio, home_folder,
};
use crate::ui::{CheckNote, ExportData, ExportPhase};

/// Everything that can happen to the export sheet.
#[derive(Clone, Debug)]
pub enum ExportMsg {
    /// The sheet is asked for: the menu, the tray button or ⌘E.
    Open,
    /// The sheet is dismissed.
    Close,
    NameEdited(String),
    ResolutionChanged(i32),
    RateChanged(i32),
    QualityChanged(i32),
    CodecChanged(i32),
    TenBitChanged(bool),
    /// Limited or full range, as a row of the Advanced section's list.
    ColorRangeChanged(i32),
    /// The Advanced section is opened or closed.
    AdvancedToggled(bool),
    /// VBR or CBR.
    RateModeChanged(i32),
    /// The target bitrate field: digits only, in kbps.
    BitrateChanged(String),
    /// Back to the form after a finished or failed render.
    Again,
    /// Pick the destination folder.
    Browse,
    /// Show the finished file in the file manager.
    Reveal,
    Start,
    Cancel,
    /// The render's worker reporting where it is.
    Progress {
        /// Of the whole, `0..=1`.
        fraction: f32,
        /// What it is doing, in the person's language.
        stage: String,
    },
    /// The render's worker is done: the file written, or why not.
    Finished(Result<String, String>),
    /// The check reading the written file back, `0..=1`.
    Checking(f32),
    /// The check is done: what it found, or why it could not look.
    Checked(Result<Vec<Note>, String>),
}

/// The export sheet's state.
pub struct ExportPane {
    pub open: bool,
    pub name: String,
    pub folder: String,
    pub resolution: usize,
    pub rate: usize,
    pub quality: usize,
    /// Index into `VideoCodec::ALL`.
    pub codec: usize,
    pub ten_bit: bool,
    /// Index into `ColorRange::ALL`: 0 limited, 1 full. Read only while
    /// the Advanced section is open, like the bitrate.
    /// https://github.com/jub0t/Concat/issues/103
    pub color_range: usize,
    /// The Advanced section is open: bitrate controls show, and the size
    /// estimate reads the chosen bitrate.
    pub advanced: bool,
    /// Index into `RateMode::ALL`.
    pub rate_mode: usize,
    /// Target bitrate in kbps, used when `rate_mode` is CBR.
    pub bitrate: u32,
    pub phase: ExportPhase,
    pub progress: f32,
    pub stage: String,
    pub message: String,
    /// Where the finished file is, for Reveal.
    pub written: String,
    /// The written file is being read back.
    pub checking: bool,
    /// How far through that reading, `0..=1`.
    pub check_progress: f32,
    /// The reading finished, whatever it found. Apart from `notes` being
    /// empty, which is the good answer, and being empty because nothing has
    /// looked yet, which is not the same thing.
    pub checked: bool,
    /// What the check found, worst first is not wanted here: in the order
    /// they happen, which is the order somebody watching meets them.
    pub notes: Vec<Note>,
    /// When the render started, for a real ETA.
    started_at: Option<std::time::Instant>,
}

impl Default for ExportPane {
    fn default() -> Self {
        Self {
            open: false,
            name: "Untitled".into(),
            folder: home_folder("Movies"),
            resolution: 2,
            rate: 1,
            quality: 1,
            codec: 0,
            ten_bit: false,
            color_range: 0,
            advanced: false,
            rate_mode: 0,
            bitrate: 8000,
            phase: ExportPhase::Idle,
            progress: 0.0,
            stage: String::new(),
            message: String::new(),
            written: String::new(),
            checking: false,
            check_progress: 0.0,
            checked: false,
            notes: Vec::new(),
            started_at: None,
        }
    }
}

impl ExportPane {
    /// Applies one message. The studio is the rest of the window, for
    /// what the sheet needs to know and to start; the pane's own state is
    /// `self`, and while this runs the studio's copy of it is a blank the
    /// pane must not read.
    pub fn update(&mut self, msg: ExportMsg, studio: &mut Studio) {
        match msg {
            ExportMsg::Open => {
                self.open = true;
                self.phase = ExportPhase::Idle;
                self.message.clear();
            }
            ExportMsg::Close => self.open = false,
            ExportMsg::NameEdited(name) => self.name = name,
            ExportMsg::ResolutionChanged(index) => {
                self.resolution = (index.max(0) as usize).min(3);
            }
            ExportMsg::RateChanged(index) => self.rate = (index.max(0) as usize).min(2),
            ExportMsg::QualityChanged(index) => self.quality = (index.max(0) as usize).min(2),
            ExportMsg::CodecChanged(index) => self.codec = (index.max(0) as usize).min(2),
            ExportMsg::TenBitChanged(on) => self.ten_bit = on,
            ExportMsg::ColorRangeChanged(index) => {
                self.color_range = (index.max(0) as usize).min(ColorRange::ALL.len() - 1);
            }
            ExportMsg::AdvancedToggled(on) => self.advanced = on,
            ExportMsg::RateModeChanged(index) => self.rate_mode = index.max(0) as usize,
            ExportMsg::BitrateChanged(text) => {
                let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(value) = digits.parse::<u32>() {
                    self.bitrate = value.clamp(100, 200_000);
                }
            }
            ExportMsg::Again => {
                self.phase = ExportPhase::Idle;
                self.progress = 0.0;
                self.checking = false;
                self.checked = false;
                self.notes.clear();
            }
            ExportMsg::Browse => {
                if let Some(folder) = platform::pick_folder(&i18n::t("Export to"), &self.folder) {
                    self.folder = folder.to_string_lossy().into_owned();
                }
            }
            ExportMsg::Reveal => {
                if !self.written.is_empty()
                    && let Err(error) = platform::reveal(&self.written)
                {
                    studio.notify(&i18n::tf("Could not show the file: {0}", &[&error]), true);
                }
            }
            ExportMsg::Start => self.start(studio),
            ExportMsg::Cancel => {
                studio.host.exporter.cancel();
                self.phase = ExportPhase::Idle;
                self.progress = 0.0;
            }
            ExportMsg::Progress { fraction, stage } => {
                if self.phase == ExportPhase::Running {
                    self.progress = fraction.clamp(0.0, 1.0);
                    self.stage = stage;
                }
            }
            ExportMsg::Finished(Ok(written)) => {
                self.phase = ExportPhase::Done;
                self.progress = 1.0;
                self.written = written.clone();
                studio.notify(&t("Export finished"), false);
                self.check(&written, studio);
            }
            ExportMsg::Checking(fraction) => {
                if self.checking {
                    self.check_progress = fraction.clamp(0.0, 1.0);
                }
            }
            ExportMsg::Checked(found) => {
                self.checking = false;
                self.check_progress = 1.0;
                match found {
                    Ok(notes) => {
                        self.checked = true;
                        let faults = notes.iter().filter(|n| n.weight == Weight::Stop).count();
                        self.notes = notes;
                        if faults > 0 {
                            studio.notify(&tf("Checked: {0} to fix", &[&faults]), true);
                        }
                    }
                    // A file that cannot be read back is not a file that is
                    // wrong: say so quietly and leave the export alone.
                    Err(error) => {
                        self.checked = false;
                        studio.notify(&tf("Could not check the file: {0}", &[&error]), true);
                    }
                }
            }
            ExportMsg::Finished(Err(error)) => {
                if self.phase == ExportPhase::Idle {
                    // Cancelled: the sheet already went back to idle.
                    return;
                }
                self.phase = ExportPhase::Failed;
                self.message = error.clone();
                studio.notify(&tf("Export failed: {0}", &[&error]), true);
            }
        }
    }

    /// The frame the export renders at: the sheet's short side, scaled
    /// along the project's aspect and rounded to even dimensions, which is
    /// what the encoder's chroma subsampling needs.
    pub fn size(&self, studio: &Studio) -> (u32, u32) {
        let short = EXPORT_SHORT_SIDES[self.resolution.min(EXPORT_SHORT_SIDES.len() - 1)];
        let (project_w, project_h) = studio.output_size();
        let (project_w, project_h) = (project_w.max(1) as f64, project_h.max(1) as f64);
        let even = |side: f64| ((side / 2.0).round() as u32 * 2).max(2);
        if project_w >= project_h {
            (even(short as f64 * project_w / project_h), short)
        } else {
            (short, even(short as f64 * project_h / project_w))
        }
    }

    /// A rough size of the file at one quality tier, in bytes.
    pub fn size_bytes(&self, studio: &Studio, tier: usize) -> f32 {
        let (width, height) = self.size(studio);
        let (num, den) = EXPORT_RATES[self.rate.min(2)];
        let rate = num as f32 / den as f32;
        let pixels = (width as f32 * height as f32) / (1920.0 * 1080.0);
        // In CBR the bitrate is the number, not the tier; the pixels, rate
        // and codec factors no longer apply because the encoder is pinned.
        let video = if self.advanced && self.rate_mode == 1 {
            self.bitrate as f32 * 1000.0
        } else {
            EXPORT_TIERS[tier.min(2)]
                * 1_000_000.0
                * pixels
                * (rate / 30.0)
                * self.codec().size_factor()
                * if self.ten_bit { 1.05 } else { 1.0 }
        };
        (video + AUDIO_BPS) * studio.duration().max(1.0) / 8.0
    }

    /// The codec the sheet has chosen.
    pub fn codec(&self) -> concat_media::VideoCodec {
        concat_media::VideoCodec::ALL[self.codec.min(concat_media::VideoCodec::ALL.len() - 1)]
    }

    /// The range the file is written in: the Advanced section's choice
    /// while the section is open, video range otherwise - so a sheet with
    /// Advanced off exports what it always did.
    pub fn color_range(&self) -> ColorRange {
        if self.advanced {
            ColorRange::ALL[self.color_range.min(ColorRange::ALL.len() - 1)]
        } else {
            ColorRange::Limited
        }
    }

    /// Starts the render on a worker. Its reports come back as messages.
    /// Reads the file just written back and says what is wrong with it.
    ///
    /// Started without being asked for, because a check somebody has to
    /// remember to run is a check nobody runs. It costs a few seconds on a
    /// machine that has just spent minutes rendering, and it is the only
    /// thing in the program that looks at what was actually made rather
    /// than at what was meant.
    fn check(&mut self, written: &str, studio: &mut Studio) {
        self.checking = true;
        self.checked = false;
        self.check_progress = 0.0;
        self.notes.clear();

        let path = written.to_owned();
        let reframers = Arc::clone(&studio.host.reframers);
        spawn(
            // The return type is spelled out because the body asks a
            // question with `?` before it answers one, and inference will
            // not take the error type from a line it has not reached.
            move || -> Result<Vec<Note>, String> {
                // Never set: the read is seconds long and there is nothing
                // to press. It is here because `look_over` polls it between
                // frames, which is what lets it be cancellable later.
                let cancel = AtomicBool::new(false);
                let mut last = -1.0f32;
                let mut fetching = |_: concat_host::reframe::Progress| {};
                let detector = reframers.detector(&cancel, &mut fetching)?;
                let mut report = |fraction: f32| {
                    if fraction - last >= 0.02 {
                        last = fraction;
                        on_ui(move |studio, _, _| {
                            studio.handle(Msg::Export(ExportMsg::Checking(fraction)));
                        });
                    }
                };
                review::look_over(&path, &detector, &cancel, &mut report)
            },
            |studio, _, _, found| studio.handle(Msg::Export(ExportMsg::Checked(found))),
        );
    }

    fn start(&mut self, studio: &mut Studio) {
        let Some(session) = studio.session.as_ref() else {
            return;
        };
        if studio.timeline().clips.is_empty() {
            self.phase = ExportPhase::Failed;
            self.message = t("There is nothing on the timeline to export");
            return;
        }
        let job = match studio.host.exporter.begin() {
            Ok(job) => job,
            Err(error) => {
                self.phase = ExportPhase::Failed;
                self.message = error;
                return;
            }
        };
        let output = format!(
            "{}/{}.mp4",
            self.folder.trim_end_matches('/'),
            self.name.trim()
        );
        let spec = ExportSpec {
            output: output.clone(),
            crf: EXPORT_CRF[self.quality.min(2)],
            preset: "veryfast".into(),
            codec: self.codec(),
            ten_bit: self.ten_bit,
            rate_mode: if self.advanced && self.rate_mode == 1 {
                concat_media::RateMode::Cbr
            } else {
                concat_media::RateMode::Vbr
            },
            bitrate_kbps: if self.advanced && self.rate_mode == 1 {
                self.bitrate
            } else {
                0
            },
            color_range: self.color_range(),
        };
        let (frame_w, frame_h) = studio.output_size();
        let titles = studio
            .host
            .titles
            .clips(session.project(), frame_w, frame_h)
            .into_iter()
            .map(|title| title.clip)
            .collect();
        let mut request = export::request(session, &spec, titles);
        let (width, height) = self.size(studio);
        let (num, den) = EXPORT_RATES[self.rate.min(2)];
        request.width = width;
        request.height = height;
        request.rate_num = num;
        request.rate_den = den;

        studio.pause();
        self.phase = ExportPhase::Running;
        self.progress = 0.0;
        self.stage = t("Rendering video");
        self.message.clear();
        self.written.clear();
        self.started_at = Some(std::time::Instant::now());

        spawn(
            move || {
                let job = job;
                export::run(&request, job.cancel_flag(), |progress| {
                    let fraction = if progress.total > 0 {
                        progress.frame as f32 / progress.total as f32
                    } else {
                        0.0
                    };
                    let stage = match progress.stage {
                        "rendering" => t("Rendering video"),
                        "mixing audio" => t("Mixing audio"),
                        "muxing" => t("Finalising file"),
                        other => other.to_owned(),
                    };
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Export(ExportMsg::Progress { fraction, stage }));
                    });
                })
            },
            |studio, _, _, result| studio.handle(Msg::Export(ExportMsg::Finished(result))),
        );
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> ExportData {
        let (width, height) = self.size(studio);
        let (num, den) = EXPORT_RATES[self.rate.min(2)];
        let rate = num as f32 / den as f32;
        let clips = studio.timeline().clips.len();
        let titles = studio
            .timeline()
            .clips
            .iter()
            .filter(|clip| clip.kind == concat_project::model::ClipKind::Text)
            .count();
        ExportData {
            open: self.open,
            name: self.name.as_str().into(),
            path: format!("{}/{}.mp4", self.folder.trim_end_matches('/'), self.name).into(),
            format: format!("{width} × {height} · {rate:.2} fps").into(),
            duration: {
                let whole = studio.duration().max(0.0) as i32;
                format!("{}:{:02}", whole / 60, whole % 60).into()
            },
            contents: if titles > 0 {
                format!("{clips} clips · {titles} titles")
            } else {
                format!("{clips} clips")
            }
            .into(),
            resolution: self.resolution as i32,
            rate: self.rate as i32,
            quality: self.quality as i32,
            codec: self.codec as i32,
            ten_bit: self.ten_bit,
            color_range: self.color_range as i32,
            advanced: self.advanced,
            rate_mode: self.rate_mode as i32,
            bitrate: self.bitrate as i32,
            encoding: {
                // "HEVC 10-bit · hardware": the standard, the depth when it
                // is the deeper one, and whether the platform's own encoder
                // will be doing it.
                let codec = self.codec();
                let mut words = vec![codec.label().to_owned()];
                if self.ten_bit {
                    words.push("10-bit".to_owned());
                }
                if self.color_range() == ColorRange::Full {
                    words.push(t("full range"));
                }
                if codec
                    .encoders(true)
                    .first()
                    .is_some_and(|name| name.ends_with("_videotoolbox"))
                {
                    words.push(format!("· {}", t("hardware")));
                }
                words.join(" ")
            }
            .into(),
            size_high: bytes(self.size_bytes(studio, 0)).into(),
            size_balanced: bytes(self.size_bytes(studio, 1)).into(),
            size_small: bytes(self.size_bytes(studio, 2)).into(),
            phase: self.phase,
            progress: self.progress,
            stage: self.stage.as_str().into(),
            eta: if self.phase == ExportPhase::Running && self.progress > 0.02 {
                self.started_at
                    .map(|started| {
                        let elapsed = started.elapsed().as_secs_f32();
                        eta(elapsed / self.progress * (1.0 - self.progress)).into()
                    })
                    .unwrap_or_default()
            } else {
                slint::SharedString::new()
            },
            message: self.message.as_str().into(),
            done_size: bytes(self.size_bytes(studio, self.quality)).into(),
            empty: clips == 0,
            checking: self.checking,
            check_progress: self.check_progress,
            checked: self.checked,
            notes: slint::ModelRc::from(
                self.notes
                    .iter()
                    .map(|note| CheckNote {
                        stop: note.weight == Weight::Stop,
                        at: moment(note).into(),
                        said: said(note.kind).into(),
                    })
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
        }
    }
}

/// Where a note is, as a clock reading. Empty for one about the whole file.
///
/// A note about the whole file is the one with no span at all, which is how
/// `review` marks them: a real note at the very start still runs to
/// somewhere.
fn moment(note: &Note) -> String {
    if note.at == 0.0 && note.until == 0.0 {
        return String::new();
    }
    let whole = note.at.max(0.0) as i64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// What a note says, in the person's language.
///
/// Worded here rather than in `concat-host` for the reason every other
/// string is: the host does not know what language the window is in, and
/// should not have to.
fn said(kind: Kind) -> String {
    match kind {
        Kind::NotTall { .. } => t("The picture is not 9:16"),
        Kind::Short { seconds } => tf("Short for a feed: {0}s", &[&seconds.round()]),
        Kind::Long { seconds } => tf("Long for a feed: {0}s", &[&seconds.round()]),
        Kind::NoFace => t("No face in shot here"),
        Kind::FaceCut => t("The frame cuts the face"),
        Kind::FaceSmall { .. } => t("The face is small for a phone"),
        Kind::FaceHigh { .. } => t("The head sits too high in frame"),
        Kind::FaceLow { .. } => t("The head sits too low in frame"),
        Kind::Wander { .. } => t("The shot drifts rather than holds"),
        Kind::Dark => t("The picture goes black"),
        Kind::Frozen => t("The picture stops moving"),
        Kind::Quiet { lufs } => tf("Quiet for a feed: {0} LUFS", &[&format!("{lufs:.0}")]),
        Kind::Loud { lufs } => tf(
            "Louder than a feed keeps: {0} LUFS",
            &[&format!("{lufs:.0}")],
        ),
        Kind::Clipped => t("The sound clips"),
        Kind::DeadOpen { seconds } => tf(
            "Nothing is heard for the first {0}s",
            &[&format!("{seconds:.1}")],
        ),
    }
}
