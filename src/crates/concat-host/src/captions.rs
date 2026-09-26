// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Captions from what the site already knows.
//!
//! YouTube times its automatic captions to the millisecond, word by word,
//! and hands them over for the asking. A transcriber would need a model
//! downloaded, minutes of a processor, and would still be reading a
//! re-encoded copy of the sound; this reads the original's own answer in
//! a couple of seconds. Whisper stays for everything else - a file from a
//! camera, a site with no captions, a language YouTube did not hear.
//!
//! **Two or three words at a time.** One word alone is ninety pieces on
//! the timeline for thirty seconds of speech, too many to correct by hand
//! and too jumpy to read; a whole sentence is a wall that arrives before
//! it is spoken. [`chunks`] cuts on the three things that make a caption
//! feel spoken rather than typed: a pause, the end of a sentence, and
//! running out of room.
//!
//! Everything here is text and arithmetic, tested without a network.

use serde_json::Value;

/// One word, and when it is said.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// Seconds from the start of the video.
    pub at: f64,
    /// The word, without the space that separated it.
    pub text: String,
}

/// A caption as it goes on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    /// Seconds from the start of the video.
    pub start: f64,
    /// When it comes off.
    pub end: f64,
    /// What it says.
    pub text: String,
}

/// At most this many words in one caption.
pub const WORDS: usize = 3;

/// And at most this many characters, so three long words do not run off a
/// tall frame's narrow side.
pub const CHARS: usize = 20;

/// A silence this long ends a caption: it is a breath, and a caption that
/// runs through one reads as though the words were joined.
pub const PAUSE: f64 = 0.4;

/// How long the last caption of a run stays up when nothing follows it to
/// say when it should go.
pub const TAIL: f64 = 1.2;

/// The words out of YouTube's `json3` captions.
///
/// The shape is `events[].segs[]`, each segment a piece of text with an
/// offset from its event's start. Events without segments are the timing
/// marks YouTube puts between lines and carry no words. Pieces that are
/// only whitespace are the spaces between words, not words.
pub fn words_of_json3(text: &str) -> Vec<Word> {
    let json: Value = match serde_json::from_str(text) {
        Ok(json) => json,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let Some(events) = json["events"].as_array() else {
        return out;
    };
    for event in events {
        let base = event["tStartMs"].as_f64().unwrap_or(0.0);
        let Some(segs) = event["segs"].as_array() else {
            continue;
        };
        for seg in segs {
            let Some(text) = seg["utf8"].as_str() else {
                continue;
            };
            let word = text.trim();
            if word.is_empty() {
                continue;
            }
            out.push(Word {
                at: (base + seg["tOffsetMs"].as_f64().unwrap_or(0.0)) / 1000.0,
                text: word.to_owned(),
            });
        }
    }
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    out
}

/// Whether a word ends a sentence, and so ends a caption.
fn closes(word: &str) -> bool {
    word.ends_with('.') || word.ends_with('?') || word.ends_with('!')
}

/// The words as captions of two or three.
///
/// A caption ends when the next word would make it too long, when the
/// word just added closed a sentence, or when the gap before the next
/// word is a pause. Each caption comes off when the next one goes on, so
/// there is never a gap with nothing on screen mid-sentence; the last of
/// a run stays [`TAIL`] seconds.
pub fn chunks(words: &[Word]) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut held: Vec<&Word> = Vec::new();

    for (i, word) in words.iter().enumerate() {
        held.push(word);
        let text: String = held
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let next = words.get(i + 1);
        let full = held.len() >= WORDS || text.chars().count() >= CHARS;
        let paused = next.is_some_and(|n| n.at - word.at > PAUSE);
        if full || closes(&word.text) || paused || next.is_none() {
            let start = held[0].at;
            // Up to the next word, or a moment past the last one.
            let end = next.map_or(word.at + TAIL, |n| n.at);
            out.push(Chunk {
                start,
                end: end.max(start + 0.1),
                text,
            });
            held.clear();
        }
    }
    out
}

/// How much of a silence to keep after the last word, so it finishes
/// rather than being clipped on its final sound.
pub const SETTLE: f64 = 0.35;

/// The furthest a clip's end is allowed to move on to find a pause.
///
/// Four seconds is a sentence's tail. Past that the clip is no longer the
/// one that was asked for, and a person who typed an end meant it.
pub const REACH: f64 = 4.0;

/// The end moved on to the next pause in the speech.
///
/// A clip cut at the second someone typed stops mid-word, which is the
/// one thing that makes a clip look unfinished however good the rest of
/// it is. The words carry their own times, so the silences are known: this
/// finds the first one at or after `wanted` and ends there instead.
///
/// `wanted` is returned unchanged when there is no pause within [`REACH`],
/// when the speech runs on without one, or when there are no words at all.
/// Moving a clip's end is a small liberty; moving it far is not.
pub fn ends_at(words: &[Word], wanted: f64, reach: f64) -> f64 {
    for pair in words.windows(2) {
        let (word, next) = (&pair[0], &pair[1]);
        let gap = next.at - word.at;
        if gap <= PAUSE {
            continue;
        }
        // Just past the last word before the silence, not into it.
        let settled = word.at + gap.min(SETTLE);
        if settled >= wanted {
            return if settled <= wanted + reach {
                settled
            } else {
                wanted
            };
        }
    }
    // Past the last word: nothing is said after `wanted`, so there is
    // nothing to be cut off mid-way.
    match words.last() {
        Some(last) if last.at >= wanted && last.at + SETTLE <= wanted + reach => last.at + SETTLE,
        _ => wanted,
    }
}

/// The captions that fall inside a stretch of the video, with their times
/// moved to start from zero.
///
/// A downloaded section begins at zero in its own file while the captions
/// are timed to the whole video, so without this every caption would be
/// three quarters of an hour late. A caption straddling an edge is kept
/// and clipped: half a sentence on screen beats none.
pub fn within(chunks: &[Chunk], from: f64, to: f64) -> Vec<Chunk> {
    chunks
        .iter()
        .filter(|c| c.end > from && c.start < to)
        .map(|c| Chunk {
            start: (c.start.max(from) - from).max(0.0),
            end: c.end.min(to) - from,
            text: c.text.clone(),
        })
        .filter(|c| c.end > c.start)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn say(pairs: &[(f64, &str)]) -> Vec<Word> {
        pairs
            .iter()
            .map(|(at, text)| Word {
                at: *at,
                text: (*text).to_owned(),
            })
            .collect()
    }

    #[test]
    fn words_come_out_of_the_shape_youtube_sends() {
        let json = r#"{"events":[
            {"tStartMs":1000,"segs":[{"utf8":"one"},{"utf8":" two","tOffsetMs":300}]},
            {"tStartMs":2000,"segs":[{"utf8":"three"}]}
        ]}"#;
        let words = words_of_json3(json);
        assert_eq!(words.len(), 3);
        assert_eq!(
            words[0],
            Word {
                at: 1.0,
                text: "one".to_owned()
            }
        );
        assert_eq!(
            words[1],
            Word {
                at: 1.3,
                text: "two".to_owned()
            }
        );
        assert_eq!(words[2].at, 2.0);
    }

    #[test]
    fn the_spaces_between_words_are_not_words() {
        let json = r#"{"events":[{"tStartMs":0,"segs":[
            {"utf8":"a"},{"utf8":" "},{"utf8":"\n"},{"utf8":"b"}]}]}"#;
        let words = words_of_json3(json);
        assert_eq!(words.len(), 2, "{words:?}");
    }

    #[test]
    fn a_timing_mark_carries_no_words() {
        // YouTube puts events with no segs between lines.
        let json = r#"{"events":[{"tStartMs":0},{"tStartMs":10,"segs":[{"utf8":"x"}]}]}"#;
        assert_eq!(words_of_json3(json).len(), 1);
    }

    #[test]
    fn anything_that_is_not_json3_gives_nothing_rather_than_a_panic() {
        for text in ["", "not json", "{}", r#"{"events":"no"}"#] {
            assert!(words_of_json3(text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn three_words_at_a_time_when_nothing_else_cuts_it() {
        let out = chunks(&say(&[
            (0.0, "one"),
            (0.2, "two"),
            (0.4, "six"),
            (0.6, "four"),
            (0.8, "five"),
            (1.0, "six"),
        ]));
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0].text, "one two six");
        assert_eq!(out[1].text, "four five six");
    }

    #[test]
    fn a_pause_ends_a_caption_even_mid_phrase() {
        // Half a second of silence after "two".
        let out = chunks(&say(&[(0.0, "one"), (0.2, "two"), (1.0, "three")]));
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0].text, "one two");
        assert_eq!(out[1].text, "three");
    }

    #[test]
    fn a_full_stop_ends_a_caption() {
        let out = chunks(&say(&[(0.0, "stop."), (0.2, "next"), (0.4, "one")]));
        assert_eq!(out[0].text, "stop.");
        assert_eq!(out.len(), 2, "{out:?}");
    }

    #[test]
    fn long_words_break_before_three_of_them() {
        let out = chunks(&say(&[
            (0.0, "extraordinary"),
            (0.2, "circumstances"),
            (0.4, "prevailed"),
        ]));
        assert!(out.len() >= 2, "one line would run off the frame: {out:?}");
    }

    #[test]
    fn one_caption_comes_off_as_the_next_goes_on() {
        let out = chunks(&say(&[
            (0.0, "a"),
            (0.1, "b"),
            (0.2, "c"),
            (0.5, "d"),
            (0.6, "e"),
            (0.7, "f"),
        ]));
        assert_eq!(out.len(), 2);
        assert!((out[0].end - out[1].start).abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn the_last_caption_stays_up_rather_than_vanishing() {
        let out = chunks(&say(&[(10.0, "end.")]));
        assert_eq!(out.len(), 1);
        assert!((out[0].end - (10.0 + TAIL)).abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn no_words_is_no_captions() {
        assert!(chunks(&[]).is_empty());
    }

    #[test]
    fn the_end_moves_on_to_the_next_silence() {
        // Speech, then a gap after "three", then more.
        let words = say(&[
            (0.0, "one"),
            (0.3, "two"),
            (0.6, "three"),
            (2.0, "four"),
            (2.3, "five"),
        ]);
        // Asked to end at 1.0, mid-silence-ward: ends just after "three".
        let end = ends_at(&words, 1.0, REACH);
        assert!((end - (0.6 + SETTLE)).abs() < 1e-9, "{end}");
    }

    #[test]
    fn an_end_already_at_a_silence_is_left_alone() {
        let words = say(&[(0.0, "one"), (2.0, "two")]);
        let end = ends_at(&words, 0.1, REACH);
        assert!((end - (0.0 + SETTLE)).abs() < 1e-9, "{end}");
    }

    #[test]
    fn speech_that_runs_on_keeps_the_end_that_was_asked_for() {
        // No gap longer than a breath anywhere, and the last word is far
        // past the wanted end.
        let words: Vec<Word> = (0..100)
            .map(|i| Word {
                at: f64::from(i) * 0.2,
                text: "on".to_owned(),
            })
            .collect();
        assert!((ends_at(&words, 5.0, REACH) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_pause_too_far_away_does_not_drag_the_end_to_it() {
        // The only silence is twenty seconds past the wanted end.
        let words = say(&[(0.0, "a"), (20.0, "b")]);
        assert!((ends_at(&words, 1.0, 0.5) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn no_words_means_the_end_stands() {
        assert!((ends_at(&[], 7.0, REACH) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn silence_after_the_last_word_ends_just_after_it() {
        let words = say(&[(0.0, "a"), (1.0, "done.")]);
        let end = ends_at(&words, 1.2, REACH);
        assert!((end - (1.0 + SETTLE)).abs() < 1e-9, "{end}");
    }

    #[test]
    fn a_downloaded_stretch_has_its_captions_moved_to_start_from_zero() {
        let all = vec![
            Chunk {
                start: 100.0,
                end: 101.0,
                text: "before".to_owned(),
            },
            Chunk {
                start: 2565.0,
                end: 2566.0,
                text: "inside".to_owned(),
            },
            Chunk {
                start: 2600.0,
                end: 2601.0,
                text: "after".to_owned(),
            },
        ];
        let cut = within(&all, 2565.0, 2585.0);
        assert_eq!(cut.len(), 1, "{cut:?}");
        assert_eq!(cut[0].text, "inside");
        assert!((cut[0].start - 0.0).abs() < 1e-9, "{cut:?}");
        assert!((cut[0].end - 1.0).abs() < 1e-9, "{cut:?}");
    }

    #[test]
    fn a_caption_across_the_edge_is_kept_and_clipped() {
        let all = vec![Chunk {
            start: 2564.0,
            end: 2567.0,
            text: "straddles".to_owned(),
        }];
        let cut = within(&all, 2565.0, 2585.0);
        assert_eq!(cut.len(), 1);
        assert!((cut[0].start - 0.0).abs() < 1e-9);
        assert!((cut[0].end - 2.0).abs() < 1e-9, "{cut:?}");
    }
}
