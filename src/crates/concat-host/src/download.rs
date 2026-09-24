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
    /// Sound only, as an m4a.
    pub audio_only: bool,
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

        let clean = if request.clean { CLEAN_TIKTOK } else { "" };
        let format = if request.audio_only {
            format!("ba{clean}/b{clean}/ba/b")
        } else {
            // Best picture with best sound, then any single file that has
            // both: the second is what the sites that serve one file offer.
            format!("bv*{clean}+ba/b{clean}/bv*+ba/b")
        };
        let template = request
            .into
            .join("%(title).80B [%(id)s].%(ext)s")
            .to_string_lossy()
            .into_owned();

        let mut child = Command::new(&tool)
            .arg("--no-playlist")
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
        if !status.success() {
            let mut why = String::new();
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                let _ = err.read_to_string(&mut why);
            }
            let why = why
                .lines()
                .rev()
                .find(|l| l.contains("ERROR"))
                .unwrap_or("");
            return Err(if why.is_empty() {
                "the download failed".to_owned()
            } else {
                why.trim().to_owned()
            });
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
