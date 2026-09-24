// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Bringing a video in from a link.
//!
//! A person editing for TikTok starts with someone else's upload as often
//! as with their own camera: a podcast to cut, a clip to react to, their
//! own post they no longer have the file for. Doing that outside the
//! editor means a second program, a downloads folder and an import; doing
//! it here is a link pasted into the media bin.
//!
//! The fetching itself is [yt-dlp], which is the only honest answer - the
//! sites change what they serve every few weeks and keeping up with them
//! is a project of its own. It is run as a child process, not linked, and
//! it is downloaded on first use the way the models are: one pinned
//! release, checked against its published digest before it is allowed to
//! run. An unchecked executable is not something this program fetches.
//!
//! **Watermarks.** TikTok serves the same video twice - once plain and
//! once with the watermark burned in - and yt-dlp exposes the second as a
//! format named `download`. [`CLEAN_TIKTOK`] refuses it, which is the
//! whole of getting a clean file.
//!
//! [yt-dlp]: https://github.com/yt-dlp/yt-dlp

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::jobs::SingleFlight;

/// The release the tool is pinned to.
///
/// Pinned so that what runs is what was checked. It does go stale - the
/// sites move and yt-dlp follows them - so [`Downloads::update`] runs the
/// tool's own updater, which checks its next version the same way.
pub const TOOL_VERSION: &str = "2026.08.19";

/// The asset for this platform, and what it must hash to.
///
/// Digests are yt-dlp's own `SHA2-256SUMS` for [`TOOL_VERSION`].
pub const fn tool_asset() -> (&'static str, &'static str, &'static str) {
    #[cfg(target_os = "windows")]
    {
        (
            "yt-dlp.exe",
            "yt-dlp.exe",
            "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a",
        )
    }
    #[cfg(target_os = "macos")]
    {
        (
            "yt-dlp_macos",
            "yt-dlp",
            "0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202",
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        (
            "yt-dlp_linux",
            "yt-dlp",
            "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a",
        )
    }
}

/// Roughly how large the tool is, for the progress bar before the server
/// says. Being wrong only makes the bar jump once.
const TOOL_BYTES: u64 = 30_000_000;

/// The format rule that leaves TikTok's watermark behind.
///
/// TikTok offers the burned-in copy as a format whose id is `download` and
/// whose note says `watermarked`. The `?` on each test means a format that
/// carries neither field is kept rather than refused - most sites have no
/// such field at all, and this rule is applied to all of them.
pub const CLEAN_TIKTOK: &str = "[format_id!*=?download][format_note!*=?watermark]";

/// How far a fetch has got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// The tool is being downloaded: bytes so far, of about this many.
    Fetching {
        /// Bytes received.
        received: u64,
        /// Bytes expected.
        total: u64,
    },
    /// The video is coming down: `0..=1`, as yt-dlp reports it.
    Downloading(f32),
    /// Downloaded; the streams are being put together.
    Merging,
}

/// What is being taken from the page.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Wanted {
    /// The picture with its sound.
    #[default]
    Video,
    /// The sound alone, left in whatever the site serves - usually m4a or
    /// webm. Nothing is re-encoded, so nothing is lost and it is instant.
    Audio,
    /// The sound alone, turned into an MP3. Needs ffmpeg on the machine;
    /// [`Downloads::can_convert`] says whether it is there.
    AudioMp3,
}

/// What one fetch covers.
#[derive(Clone, Debug)]
pub struct FetchRequest {
    /// The page's address, as the person pasted it.
    pub url: String,
    /// The folder the file lands in - the project's own media folder.
    pub into: PathBuf,
    /// Refuse TikTok's watermarked copy. On for everything; it costs
    /// nothing where the fields do not exist.
    pub clean: bool,
    /// Picture, sound, or sound as an MP3.
    pub wanted: Wanted,
    /// The tallest picture to take, in pixels; 0 for whatever is best.
    /// 2160 is 4K, 1080 and 720 the usual two below it.
    pub max_height: u32,
    /// The most frames a second to take; 0 for whatever is best. A site
    /// that only has 60 still answers with it when 30 is asked for and
    /// nothing else exists - see [`format_for`].
    pub max_fps: u32,
}

impl Default for FetchRequest {
    fn default() -> FetchRequest {
        FetchRequest {
            url: String::new(),
            into: PathBuf::new(),
            clean: true,
            wanted: Wanted::Video,
            max_height: 0,
            max_fps: 0,
        }
    }
}

/// What a picture has to be encoded in to be worth editing.
///
/// H.264. Not because it is the best - AV1 is half the size for the same
/// picture, which is why YouTube serves it first - but because every
/// graphics chip made in the last fifteen years decodes it in hardware,
/// and almost none of them decode AV1 at all. An AV1 file plays back on
/// the processor alone: it downloads quickly, looks fine in a player that
/// buffers, and drags an editor's timeline to a crawl the moment anything
/// asks it to seek.
pub const EDITABLE: &str = "[vcodec^=avc1]";

/// And the sound: AAC in an MP4, the pair H.264 is normally carried with.
/// Opus and Vorbis are fine to play, but muxing them beside H.264 leaves
/// a file some tools will not open.
pub const EDITABLE_SOUND: &str = "[acodec^=mp4a]";

/// The format rule for a request, in yt-dlp's own language.
///
/// Read left to right, `/` meaning "or else". Four choices for a picture:
/// H.264 within the limits, anything within the limits, H.264 at any size,
/// and finally any single file. So the usual answer is an editable file at
/// the asked-for size, a video that exists only in AV1 still comes down,
/// and a video that has no copy that small comes down at what it has. A
/// limit is a ceiling, never a demand.
///
/// Every choice carries the watermark rule, the last one included: a
/// silently watermarked video is worse than a download that says it found
/// nothing clean.
pub fn format_for(request: &FetchRequest) -> String {
    let clean = if request.clean { CLEAN_TIKTOK } else { "" };
    if request.wanted != Wanted::Video {
        return format!("ba{EDITABLE_SOUND}{clean}/ba{clean}/b{clean}");
    }
    let mut limits = String::new();
    if request.max_height > 0 {
        limits.push_str(&format!("[height<={}]", request.max_height));
    }
    if request.max_fps > 0 {
        limits.push_str(&format!("[fps<={}]", request.max_fps));
    }
    format!(
        "bv*{limits}{EDITABLE}{clean}+ba{EDITABLE_SOUND}{clean}         /bv*{limits}{EDITABLE}{clean}+ba{clean}         /bv*{limits}{clean}+ba{clean}         /bv*{EDITABLE}{clean}+ba{clean}         /b{clean}"
    )
}

/// The fetching service: where the tool lives, and the one-job slot.
pub struct Downloads {
    gate: Arc<SingleFlight>,
    data: PathBuf,
}

impl Downloads {
    /// A service keeping its tool under `data`.
    pub fn new(data: &Path) -> Downloads {
        Downloads {
            gate: Arc::new(SingleFlight::new()),
            data: data.to_path_buf(),
        }
    }

    /// Whether a fetch is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running fetch to stop.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// Where the tool is kept.
    pub fn tool_path(&self) -> PathBuf {
        self.data.join("tools").join(tool_asset().1)
    }

    /// Whether the tool is already there.
    pub fn installed(&self) -> bool {
        self.tool_path().is_file()
    }

    /// The tool, downloaded and checked on first use.
    fn tool(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, String> {
        let file = self.tool_path();
        if file.is_file() {
            return Ok(file);
        }
        let (asset, _, digest) = tool_asset();
        let parent = file.parent().ok_or("no tools folder")?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        let partial = file.with_extension("part");
        let url =
            format!("https://github.com/yt-dlp/yt-dlp/releases/download/{TOOL_VERSION}/{asset}");
        progress(Progress::Fetching {
            received: 0,
            total: TOOL_BYTES,
        });
        crate::models::download(
            &url,
            &partial,
            TOOL_BYTES,
            cancel,
            "download cancelled",
            &mut |received, total| progress(Progress::Fetching { received, total }),
        )?;
        // Checked before it is ever run, and thrown away when it does not
        // match: this is an executable, not a model, and a wrong one is
        // worse than none.
        if let Err(error) = crate::models::verify(&partial, digest) {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))
                .map_err(|error| format!("could not make the tool runnable: {error}"))?;
        }
        std::fs::rename(&partial, &file)
            .map_err(|error| format!("could not finish {}: {error}", file.display()))?;
        Ok(file)
    }

    /// Whether MP3 is on offer: yt-dlp converts through ffmpeg, and asking
    /// for it without one fails after the download rather than before it.
    pub fn can_convert(&self) -> bool {
        which("ffmpeg").is_some()
    }

    /// Runs the tool's own updater, which checks the new version itself.
    ///
    /// The pin in [`TOOL_VERSION`] is what a first install is checked
    /// against; after that the sites move faster than this program's
    /// releases, so the escape hatch is the tool updating itself.
    pub fn update(&self) -> Result<String, String> {
        let tool = self.tool_path();
        if !tool.is_file() {
            return Err("nothing to update yet".to_owned());
        }
        let out = Command::new(&tool)
            .arg("-U")
            .output()
            .map_err(|error| format!("could not run the tool: {error}"))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    /// Fetches `request` and answers with the file that landed.
    pub fn fetch(
        &self,
        request: &FetchRequest,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, String> {
        let job = self.gate.begin("download")?;
        let cancel = job.cancel_handle();
        let tool = self.tool(&cancel, progress)?;
        std::fs::create_dir_all(&request.into)
            .map_err(|error| format!("could not create {}: {error}", request.into.display()))?;

        let format = format_for(request);
        let template = request
            .into
            .join("%(title).80B [%(id)s].%(ext)s")
            .to_string_lossy()
            .into_owned();

        let mut child = Command::new(&tool)
            .arg("--no-playlist")
            // YouTube hands out addresses that go stale, and a long file can
            // outlive the one it started on: the download then stops with
            // "403 Forbidden" partway through. These make the tool ask again
            // instead of giving up, and fetch the address afresh when the
            // old one is refused.
            .args(["--retries", "20"])
            .args(["--fragment-retries", "20"])
            .args(["--extractor-retries", "5"])
            // Take up where a stopped attempt left off rather than starting
            // the hour again.
            .arg("--continue")
            // Named rather than left to the default, which is deno alone.
            .args(match js_runtime() {
                Some(runtime) => vec!["--js-runtime".to_owned(), runtime.to_owned()],
                None => Vec::new(),
            })
            // One line per progress report rather than a carriage return
            // rewriting one, which cannot be read line by line.
            .arg("--newline")
            .arg("--no-colors")
            .args(["-f", &format])
            .args(match request.wanted {
                // Nothing to convert: whatever the site serves is kept.
                Wanted::Video | Wanted::Audio => Vec::new(),
                Wanted::AudioMp3 => vec![
                    "--extract-audio".to_owned(),
                    "--audio-format".to_owned(),
                    "mp3".to_owned(),
                    "--audio-quality".to_owned(),
                    "0".to_owned(),
                ],
            })
            .args(["-o", &template])
            // `--print` alone would only pretend to download; with this it
            // downloads and then says where the file went, which beats
            // guessing the name back out of the template.
            .arg("--no-simulate")
            .args(["--print", "after_move:filepath"])
            .arg(&request.url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run the downloader: {error}"))?;

        // Read on a thread of its own, never after the wait. A pipe holds
        // about sixty kilobytes; once it is full the tool blocks writing
        // its next warning, and a caller that reads only after the process
        // exits is then waiting for a process that is waiting for it.
        // YouTube alone warns enough to fill it, and the download simply
        // stops - which is what it did.
        let stderr = child.stderr.take().ok_or("the downloader said nothing")?;
        let complaints = std::thread::spawn(move || {
            BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<String>>()
        });

        let stdout = child.stdout.take().ok_or("the downloader said nothing")?;
        let mut landed: Option<PathBuf> = None;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                return Err("download cancelled".to_owned());
            }
            if let Some(percent) = percent_of(&line) {
                progress(Progress::Downloading(percent));
            } else if line.starts_with("[Merger]") || line.starts_with("[ExtractAudio]") {
                progress(Progress::Merging);
            } else {
                let path = Path::new(line.trim());
                if path.is_file() {
                    landed = Some(path.to_path_buf());
                }
            }
        }
        let status = child
            .wait()
            .map_err(|error| format!("the downloader did not finish: {error}"))?;
        let said = complaints.join().unwrap_or_default();
        for line in &said {
            log::warn!("download: {line}");
        }
        if !status.success() {
            // The last ERROR line is the one that stopped it; any above are
            // usually a format it tried first and could not have.
            let why = said
                .iter()
                .rev()
                .find(|line| line.contains("ERROR"))
                .map(|line| line.trim().to_owned());
            return Err(why.unwrap_or_else(|| "the download failed".to_owned()));
        }
        landed.ok_or_else(|| "the download finished but left no file".to_owned())
    }
}

/// The JavaScript runtimes yt-dlp can drive, in the order they are tried.
///
/// YouTube hides the address of its streams behind a script that has to be
/// run to be read, so without one of these the tool fetches the page, warns,
/// and hands back nothing usable. Deno is the one yt-dlp enables by itself;
/// the rest have to be named, which is what [`js_runtime`] does.
pub const JS_RUNTIMES: [&str; 4] = ["deno", "node", "bun", "quickjs"];

/// The first JavaScript runtime on the path, if any.
///
/// None is not fatal - most sites need no script at all, and the tool says
/// so itself when one is missing.
pub fn js_runtime() -> Option<&'static str> {
    JS_RUNTIMES.into_iter().find(|name| which(name).is_some())
}

/// Where a program of that name is on the path.
fn which(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(if cfg!(windows) { ';' } else { ':' })
        .map(|dir| Path::new(dir).join(&exe))
        .find(|path| path.is_file())
}

/// The percentage out of a yt-dlp progress line, as `0..=1`.
///
/// The line reads `[download]  45.2% of 12.34MiB at ...`; anything else,
/// including the ones about playlists and formats, is not progress.
pub fn percent_of(line: &str) -> Option<f32> {
    let rest = line.strip_prefix("[download]")?.trim_start();
    let (number, _) = rest.split_once('%')?;
    let percent: f32 = number.trim().parse().ok()?;
    Some((percent / 100.0).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_progress_line_reads_as_a_fraction() {
        // Compared with a margin: 45.2 divided by a hundred is not exactly
        // 0.452 in binary, and a progress bar does not care.
        let read = percent_of("[download]  45.2% of 12.34MiB at 1.00MiB/s ETA 00:07");
        assert!(
            (read.expect("a percentage") - 0.452).abs() < 1e-6,
            "{read:?}"
        );
        assert_eq!(percent_of("[download] 100% of 1.00MiB"), Some(1.0));
        assert_eq!(percent_of("[download]   0.0% of ~10.00MiB"), Some(0.0));
    }

    #[test]
    fn anything_else_is_not_progress() {
        assert_eq!(percent_of("[youtube] abc: Downloading webpage"), None);
        assert_eq!(percent_of("[download] Destination: video.mp4"), None);
        assert_eq!(percent_of("[Merger] Merging formats into \"v.mp4\""), None);
        assert_eq!(percent_of(""), None);
    }

    fn asking(wanted: Wanted, height: u32, fps: u32) -> String {
        format_for(&FetchRequest {
            wanted,
            max_height: height,
            max_fps: fps,
            ..FetchRequest::default()
        })
    }

    #[test]
    fn the_first_choice_is_one_the_graphics_chip_can_decode() {
        // AV1 is what YouTube offers first and what the timeline cannot
        // play; asking for H.264 ahead of it is the whole of this fix.
        let f = asking(Wanted::Video, 1080, 30);
        let first = f.split('/').next().expect("a first choice");
        assert!(first.contains("vcodec^=avc1"), "{first}");
        assert!(first.contains("acodec^=mp4a"), "{first}");
    }

    #[test]
    fn a_video_that_exists_only_in_av1_still_comes_down() {
        // Some choice has to be willing to take whatever there is, or a
        // video with no H.264 copy fails instead of arriving.
        let f = asking(Wanted::Video, 1080, 30);
        assert!(
            f.split('/').any(|choice| !choice.contains("avc1")),
            "every choice demands H.264: {f}"
        );
    }

    #[test]
    fn sound_alone_prefers_the_one_that_muxes_with_h264() {
        let f = asking(Wanted::Audio, 0, 0);
        assert!(f.starts_with("ba[acodec^=mp4a]"), "{f}");
        assert!(
            f.split('/').any(|c| !c.contains("mp4a")),
            "no fallback: {f}"
        );
    }

    #[test]
    fn no_limits_asks_for_the_best_there_is() {
        let f = asking(Wanted::Video, 0, 0);
        assert!(!f.contains("height"), "{f}");
        assert!(!f.contains("fps"), "{f}");
        assert!(f.starts_with("bv*"), "{f}");
    }

    #[test]
    fn a_height_and_a_rate_become_ceilings() {
        let f = asking(Wanted::Video, 1080, 30);
        assert!(f.contains("[height<=1080]"), "{f}");
        assert!(f.contains("[fps<=30]"), "{f}");
    }

    #[test]
    fn a_video_that_has_no_such_copy_still_comes_down() {
        // The last choice carries no limits, so 4K-only footage asked for
        // at 720 arrives as 4K rather than as an error.
        let f = asking(Wanted::Video, 720, 0);
        let last = f.rsplit('/').next().expect("a last choice");
        assert!(!last.contains("height"), "last choice is limited: {f}");
    }

    #[test]
    fn sound_alone_asks_for_sound_and_ignores_the_picture_limits() {
        for wanted in [Wanted::Audio, Wanted::AudioMp3] {
            let f = asking(wanted, 1080, 60);
            assert!(f.starts_with("ba"), "{f}");
            assert!(!f.contains("height") && !f.contains("fps"), "{f}");
        }
    }

    #[test]
    fn the_watermark_rule_is_in_every_choice_that_names_a_format() {
        let f = asking(Wanted::Video, 1080, 0);
        for choice in f.split('/') {
            assert!(choice.contains("format_id!*=?download"), "{choice} in {f}");
        }
    }

    #[test]
    fn the_watermark_rule_refuses_the_marked_copy_and_keeps_the_rest() {
        // Both tests are optional, so a format with neither field survives.
        assert!(CLEAN_TIKTOK.contains("format_id!*=?download"));
        assert!(CLEAN_TIKTOK.contains("format_note!*=?watermark"));
    }

    #[test]
    fn a_runtime_is_looked_for_on_the_path_and_missing_is_allowed() {
        // Whatever this machine has, the answer is one of the four or none,
        // and asking must not panic on a machine with no PATH at all.
        match js_runtime() {
            Some(name) => assert!(JS_RUNTIMES.contains(&name), "{name}"),
            None => {}
        }
        assert!(which("a-program-nobody-has-installed-3f9c").is_none());
    }

    #[test]
    fn this_platform_has_an_asset_and_a_digest() {
        let (asset, name, digest) = tool_asset();
        assert!(!asset.is_empty() && !name.is_empty());
        assert_eq!(digest.len(), 64, "a sha256 is 64 hex digits");
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
