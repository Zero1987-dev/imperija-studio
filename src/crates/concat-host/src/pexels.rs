// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Cutaway footage, from Pexels.
//!
//! B-roll is the one thing the editor cannot make out of what it has: a
//! podcast clip has one shot of one person, and a cutaway has to come from
//! somewhere. Pexels gives it away - free to use, free to change, and free
//! commercially - which is rarer than it sounds among the sites that say
//! "free".
//!
//! **The key is the person's own.** Anyone with a Pexels account gets one
//! instantly, and it stays on this machine; nothing is proxied through
//! anywhere of ours and no account of ours is involved. Their terms ask for
//! a visible link back, which the sheet carries.
//!
//! Two hundred searches an hour and twenty thousand a month, which for one
//! person cutting clips is not a limit anybody meets.
//!
//! Everything here that can be tested without a network is: the address a
//! search builds, what comes back out of the answer, and which of the files
//! offered is the one worth taking.

use serde_json::Value;

/// Where a search goes.
pub const SEARCH: &str = "https://api.pexels.com/videos/search";

/// How many come back at a time. Enough to fill a sheet twice over without
/// making somebody wait for pictures they will not scroll to.
pub const PER_PAGE: u32 = 24;

/// The shortest side worth taking for a 1080-wide frame.
///
/// Anything under this is being enlarged on the way in, which is the fault
/// the whole of `Full width` exists to avoid.
pub const LEAST: u32 = 1080;

/// One video the search found.
#[derive(Clone, PartialEq, Debug)]
pub struct Found {
    /// Pexels' own id.
    pub id: u64,
    /// How long it runs.
    pub seconds: f64,
    /// Who shot it. Their terms do not require the credit; it costs
    /// nothing and it is the decent thing.
    pub by: String,
    /// The page it lives on, which is the link back their API terms ask
    /// for.
    pub page: String,
    /// A still, for the card.
    pub still: String,
    /// The file to fetch.
    pub file: String,
    /// And its size, so the card can say.
    pub width: u32,
    /// The file's height.
    pub height: u32,
}

/// The address one search goes to.
///
/// Portrait first: a clip that is already tall needs no cropping, and a
/// wide one laid over a tall frame is either letterboxed or enlarged.
pub fn url_for(words: &str, page: u32) -> String {
    let words = words.trim();
    let escaped: String = words
        .chars()
        .map(|c| match c {
            ' ' => "+".to_owned(),
            c if c.is_ascii_alphanumeric() => c.to_string(),
            c => c.to_string().bytes().map(|b| format!("%{b:02X}")).collect(),
        })
        .collect();
    format!(
        "{SEARCH}?query={escaped}&orientation=portrait&per_page={PER_PAGE}&page={}",
        page.max(1)
    )
}

/// The file worth taking out of the several Pexels offers.
///
/// The smallest that is still big enough, rather than the biggest there is:
/// a 4K cutaway shown for two seconds behind a caption is a download nobody
/// needed. Falls back to the largest when nothing reaches [`LEAST`].
pub fn best_file(files: &[Value]) -> Option<(String, u32, u32)> {
    let mp4s: Vec<(String, u32, u32)> = files
        .iter()
        .filter(|file| {
            file.get("file_type")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind.contains("mp4"))
        })
        .filter_map(|file| {
            Some((
                file.get("link")?.as_str()?.to_owned(),
                file.get("width")?.as_u64()? as u32,
                file.get("height")?.as_u64()? as u32,
            ))
        })
        .filter(|(_, w, h)| *w > 0 && *h > 0)
        .collect();
    mp4s.iter()
        .filter(|(_, w, h)| (*w).min(*h) >= LEAST)
        .min_by_key(|(_, w, h)| u64::from(*w) * u64::from(*h))
        .or_else(|| {
            mp4s.iter()
                .max_by_key(|(_, w, h)| u64::from(*w) * u64::from(*h))
        })
        .cloned()
}

/// What a search answered, read out of it.
pub fn found_in(json: &str) -> Result<Vec<Found>, String> {
    let root: Value = serde_json::from_str(json).map_err(|error| format!("{error}"))?;
    // Pexels says what went wrong in plain words rather than in a code.
    if let Some(trouble) = root.get("error").and_then(Value::as_str) {
        return Err(trouble.to_owned());
    }
    let Some(videos) = root.get("videos").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    Ok(videos
        .iter()
        .filter_map(|video| {
            let files = video.get("video_files")?.as_array()?;
            let (file, width, height) = best_file(files)?;
            Some(Found {
                id: video.get("id")?.as_u64()?,
                seconds: video.get("duration").and_then(Value::as_f64).unwrap_or(0.0),
                by: video
                    .get("user")
                    .and_then(|user| user.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                page: video
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                still: video
                    .get("image")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                file,
                width,
                height,
            })
        })
        .collect())
}

/// Asks Pexels, with the person's own key.
pub fn search(key: &str, words: &str, page: u32) -> Result<Vec<Found>, String> {
    if key.trim().is_empty() {
        return Err("no Pexels key".to_owned());
    }
    if words.trim().is_empty() {
        return Ok(Vec::new());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let answer = agent
        .get(&url_for(words, page))
        .set("Authorization", key.trim())
        .call();
    let body = match answer {
        Ok(response) => response.into_string().map_err(|error| format!("{error}"))?,
        // The two that mean something a person can act on, said plainly.
        Err(ureq::Error::Status(401, _)) => return Err("the Pexels key was refused".to_owned()),
        Err(ureq::Error::Status(429, _)) => {
            return Err("Pexels has had enough searches for now".to_owned());
        }
        Err(error) => return Err(format!("{error}")),
    };
    found_in(&body)
}

/// Fetches one file, answering with where it landed.
///
/// Written beside its own name under `into`, which for a cutaway is the
/// project's media folder and for a still is a cache nobody looks at. Not
/// resumable and not cancellable: a Pexels file is seconds of video and a
/// still is a few kilobytes, and machinery for stopping them would be
/// larger than they are.
pub fn fetch(
    url: &str,
    into: &std::path::Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<std::path::PathBuf, String> {
    use std::io::{Read, Write};

    let name = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && !name.contains('?'))
        .unwrap_or("pexels.mp4");
    std::fs::create_dir_all(into)
        .map_err(|error| format!("could not make {}: {error}", into.display()))?;
    let file = into.join(name);

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("{url} did not answer: {error}"))?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    let mut source = response.into_reader();
    let mut sink = std::fs::File::create(&file)
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut had = 0u64;
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| format!("{url} stopped: {error}"))?;
        if read == 0 {
            break;
        }
        sink.write_all(&buffer[..read])
            .map_err(|error| format!("could not write {}: {error}", file.display()))?;
        had += read as u64;
        progress(had, total);
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One answer, shaped the way Pexels shapes them.
    const ANSWER: &str = r#"{
      "page": 1, "per_page": 2, "total_results": 400,
      "videos": [
        {
          "id": 8551234, "width": 1080, "height": 1920, "duration": 12,
          "url": "https://www.pexels.com/video/a-city-at-night-8551234/",
          "image": "https://images.pexels.com/videos/8551234/still.jpg",
          "user": { "name": "Ayla Demir" },
          "video_files": [
            { "quality": "sd", "file_type": "video/mp4", "width": 360, "height": 640,
              "link": "https://player.vimeo.com/x/360.mp4" },
            { "quality": "hd", "file_type": "video/mp4", "width": 1080, "height": 1920,
              "link": "https://player.vimeo.com/x/1080.mp4" },
            { "quality": "uhd", "file_type": "video/mp4", "width": 2160, "height": 3840,
              "link": "https://player.vimeo.com/x/2160.mp4" }
          ]
        },
        {
          "id": 9000001, "width": 720, "height": 1280, "duration": 6,
          "url": "https://www.pexels.com/video/rain-9000001/",
          "image": "https://images.pexels.com/videos/9000001/still.jpg",
          "user": { "name": "Marko Ilić" },
          "video_files": [
            { "quality": "sd", "file_type": "video/mp4", "width": 720, "height": 1280,
              "link": "https://player.vimeo.com/x/720.mp4" }
          ]
        }
      ]
    }"#;

    #[test]
    fn a_search_asks_for_tall_pictures_and_escapes_what_was_typed() {
        let url = url_for("night city", 1);
        assert!(url.starts_with(SEARCH), "{url}");
        assert!(url.contains("query=night+city"), "{url}");
        assert!(url.contains("orientation=portrait"), "{url}");
        assert!(url.contains(&format!("per_page={PER_PAGE}")), "{url}");
        assert!(url.contains("page=1"), "{url}");
        // A page of zero is a page of one; nobody asks for the zeroth.
        assert!(url_for("x", 0).contains("page=1"));
        // Our own letters survive the trip.
        let ours = url_for("čaj", 1);
        assert!(ours.contains("query=%C4%8Daj"), "{ours}");
    }

    #[test]
    fn the_smallest_file_that_is_still_big_enough_is_the_one_taken() {
        let found = found_in(ANSWER).expect("reads");
        assert_eq!(found.len(), 2, "{found:?}");
        // 1080 wide is enough; the 4K copy beside it is a download nobody
        // needed and the 360 is too small to show.
        assert!(found[0].file.ends_with("1080.mp4"), "{:?}", found[0]);
        assert_eq!((found[0].width, found[0].height), (1080, 1920));
        // Nothing here reaches the floor, so the largest there is stands in
        // rather than the video being dropped.
        assert!(found[1].file.ends_with("720.mp4"), "{:?}", found[1]);
    }

    #[test]
    fn who_shot_it_and_where_it_lives_come_back_too() {
        let found = found_in(ANSWER).expect("reads");
        assert_eq!(found[0].by, "Ayla Demir");
        assert_eq!(found[1].by, "Marko Ilić");
        assert!(found[0].page.starts_with("https://www.pexels.com/video/"));
        assert!(found[0].still.ends_with(".jpg"));
        assert!((found[0].seconds - 12.0).abs() < 1e-9);
        assert_eq!(found[0].id, 8_551_234);
    }

    #[test]
    fn an_answer_that_is_a_complaint_is_an_error_and_not_an_empty_shelf() {
        let refused = found_in(r#"{"error":"Invalid API key"}"#);
        assert_eq!(refused, Err("Invalid API key".to_owned()));
        // And a shelf with nothing on it is not an error.
        assert_eq!(found_in(r#"{"videos":[]}"#), Ok(Vec::new()));
        assert_eq!(found_in(r#"{"total_results":0}"#), Ok(Vec::new()));
        assert!(found_in("not json at all").is_err());
    }

    #[test]
    fn a_video_with_nothing_to_download_is_left_out_rather_than_breaking_the_rest() {
        let mixed = r#"{"videos":[
          {"id":1,"duration":3,"video_files":[]},
          {"id":2,"duration":3,"user":{"name":"A"},"url":"u","image":"i",
           "video_files":[{"file_type":"video/mp4","width":1080,"height":1920,
                           "link":"https://x/ok.mp4"}]}
        ]}"#;
        let found = found_in(mixed).expect("reads");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, 2);
    }

    #[test]
    fn asking_with_no_key_or_no_words_never_reaches_the_network() {
        assert!(search("", "night", 1).is_err());
        assert_eq!(search("abc", "   ", 1), Ok(Vec::new()));
    }
}
