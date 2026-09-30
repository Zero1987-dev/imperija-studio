// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Deciding where a cutaway goes and what it should show.
//!
//! Searching for footage by hand and dropping it in is half the work of a
//! clip. The captions are already on the timeline with their times on them,
//! which is everything needed to guess: what is being talked about, and
//! when.
//!
//! **A guess, and it says so.** Picking the word to search for without a
//! model is a heuristic - the longest word nobody uses for grammar - and it
//! will sometimes put a picture of a bridge under a sentence about getting
//! over something. The cutaways land as ordinary clips on a lane of their
//! own, which is one key to delete; that is the whole answer to a guess
//! that missed, and it is a better answer than not guessing.
//!
//! Language is handled by knowing what to *throw away* rather than by
//! knowing what to keep: the words a sentence is built out of are few and
//! listable, in either language this is cut in, and what is left over is
//! mostly what the sentence is about.

//! One thing here does open a file: [`opening`], which looks at the stock
//! footage to find a moment worth cutting to. Everything else is text.

use concat_media::{DecodeOptions, Decoder, FrameSource};

/// The words that carry grammar rather than meaning, in the two languages
/// this is cut in.
///
/// Listing what to discard rather than what to keep: the first list is
/// short and closed, the second is every noun there is. Anything left after
/// this is far more likely to be worth a picture.
pub const GRAMMAR: &[&str] = &[
    // English
    "about",
    "after",
    "again",
    "against",
    "because",
    "been",
    "before",
    "being",
    "between",
    "both",
    "came",
    "could",
    "does",
    "doing",
    "done",
    "down",
    "during",
    "each",
    "even",
    "ever",
    "every",
    "from",
    "gets",
    "getting",
    "going",
    "gonna",
    "have",
    "having",
    "here",
    "into",
    "just",
    "know",
    "like",
    "little",
    "made",
    "make",
    "many",
    "more",
    "most",
    "much",
    "must",
    "never",
    "only",
    "other",
    "over",
    "really",
    "right",
    "said",
    "same",
    "says",
    "should",
    "since",
    "some",
    "something",
    "still",
    "such",
    "than",
    "that",
    "thats",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "things",
    "think",
    "this",
    "those",
    "through",
    "time",
    "under",
    "until",
    "very",
    "want",
    "well",
    "went",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "with",
    "without",
    "would",
    "yeah",
    "your",
    "youre",
    "always",
    "actually",
    "anything",
    "everything",
    "nothing",
    "people",
    "someone",
    "maybe",
    "basically",
    "literally",
    // Bosnian, Croatian, Serbian
    "ali",
    "ako",
    "bez",
    "bilo",
    "bila",
    "bili",
    "biti",
    "bude",
    "cega",
    "cemu",
    "dobro",
    "gdje",
    "iako",
    "ipak",
    "isto",
    "jedan",
    "jedna",
    "jer",
    "jos",
    "kada",
    "kako",
    "kao",
    "koje",
    "koji",
    "koja",
    "kroz",
    "malo",
    "mnogo",
    "mogu",
    "moze",
    "nakon",
    "nego",
    "neka",
    "neki",
    "nema",
    "nije",
    "onda",
    "ono",
    "opet",
    "ovo",
    "poslije",
    "prije",
    "puno",
    "samo",
    "sada",
    "svaki",
    "sve",
    "tako",
    "tamo",
    "treba",
    "uvijek",
    "vec",
    "vise",
    "zato",
    "zbog",
    "znaci",
    "znam",
    "sam",
    "smo",
    "ste",
    "sta",
    "sto",
    "tu",
    "ti",
    "mi",
    "vi",
    "oni",
    "one",
    "ona",
    "on",
];

/// How long a cutaway runs.
///
/// Long enough to register, short enough that the person talking is not
/// gone. Anything past three seconds and the viewer starts wondering where
/// the speaker went.
pub const LENGTH: f64 = 1.8;

/// And the least time between two of them.
///
/// A cutaway every other sentence is a slideshow. Eight seconds is roughly
/// where they stop reading as decoration and start reading as illustration.
pub const APART: f64 = 8.0;

/// The shortest word worth a picture.
///
/// Six, not four. Four lets through every short concrete noun in the
/// language, and a stock library will happily answer one of those with
/// something that has nothing to do with the sentence: asked for "corn" it
/// came back with a full-screen photograph of hazelnuts, laid over a man
/// talking about social media. A word that short carries too little of
/// what a sentence is about to be worth searching on, and a cutaway that
/// means nothing is worse than no cutaway - it does not decorate the
/// point, it hides the person making it.
pub const SHORTEST_WORD: usize = 6;

/// How long the speaker is left alone at the start, in seconds.
///
/// Nobody has met the speaker yet. Cutting away from a face before anyone
/// has looked at it spends the only seconds that decide whether there are
/// any others - and it is exactly what happened: the first cutaway landed
/// at three quarters of a second, over the hook.
pub const SETTLE: f64 = 2.5;

/// One cutaway to find and place.
#[derive(Clone, PartialEq, Debug)]
pub struct Cue {
    /// Where it starts, in timeline seconds.
    pub at: f64,
    /// And ends.
    pub until: f64,
    /// What to search for.
    pub words: String,
}

/// The word in a line most likely to be worth a picture.
///
/// The longest one that is not grammar. Length stands in for meaning
/// because it usually can: the short words in any sentence are the ones
/// holding it together.
pub fn subject_of(line: &str) -> Option<String> {
    line.split(|c: char| !c.is_alphabetic())
        .filter(|word| word.chars().count() >= SHORTEST_WORD)
        .map(str::to_lowercase)
        .filter(|word| !GRAMMAR.contains(&word.as_str()))
        .max_by_key(|word| word.chars().count())
}

/// Where the cutaways go, from the captions already on the timeline.
///
/// `captions` is every title with its start, its end and its words, in any
/// order. What comes back is in order, spaced by at least [`APART`], and
/// never twice on the same word - the same picture twice in a minute is
/// worse than one picture.
pub fn cues(captions: &[(f64, f64, String)], spacing: f64, length: f64) -> Vec<Cue> {
    let mut lines: Vec<&(f64, f64, String)> = captions.iter().collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut cues: Vec<Cue> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for (start, end, text) in lines {
        // The opening belongs to whoever is talking; see `SETTLE`.
        if *start < SETTLE {
            continue;
        }
        if cues
            .last()
            .is_some_and(|last| *start - last.at < spacing.max(0.1))
        {
            continue;
        }
        let Some(word) = subject_of(text) else {
            continue;
        };
        if used.contains(&word) {
            continue;
        }
        // Never past the line that prompted it by more than its own length:
        // a cutaway outliving the sentence it illustrates is just a cut.
        let until = (start + length).min(end.max(*start) + length);
        cues.push(Cue {
            at: *start,
            until,
            words: word.clone(),
        });
        used.push(word);
    }
    cues
}

/// How far into a stock clip the search for an opening begins, as a
/// fraction of the file.
///
/// Stock footage is sold with its own front matter. A slow fade up from
/// black is the commonest, a held title card the next, and either way the
/// first moment of the file is the one moment of it worth the least. A
/// cutaway taken from the very head therefore shows the fade rather than
/// the shot - and where the fade is longer than the cutaway, shows nothing
/// at all.
pub const INTO: f64 = 0.12;

/// How lit a frame has to be to count as a picture, zero to one.
pub const LIT: f64 = 0.07;

/// And how much it has to vary across itself, on the same scale.
///
/// Brightness alone passes a white title card, which is as useless a
/// cutaway as a black one. A picture of something has light and dark in
/// it; a card does not.
pub const VARIED: f64 = 0.035;

/// How far apart the frames tried are, in seconds.
pub const STEP: f64 = 0.4;

/// How many are tried before giving up.
///
/// Twelve steps is a little under five seconds of searching, which covers
/// any front matter worth the name. Past that the file is probably dark on
/// purpose, and starting at the head is as good an answer as any.
pub const TRIES: usize = 12;

/// How wide the frames are decoded, in pixels.
///
/// This is a question about the average of a picture, and the average of a
/// picture survives being made small. Sixteenth of a frame each way is a
/// four-hundredth of the pixels and the same answer.
pub const LOOK_WIDTH: u32 = 120;

/// Where in `path` a cutaway of `wanted` seconds should start.
///
/// Answers with seconds into the file: the first moment from [`INTO`]
/// onwards that has a picture in it, or zero if the file is too short to
/// choose or nothing in it passes. Never answers with a start so late that
/// `wanted` would run off the end.
///
/// The whole point is that a stock clip's own opening is usually its worst
/// part, and nothing else in the program was looking. One cutaway came out
/// black from end to end because the file it came from faded up over three
/// seconds and the cutaway was two and a half.
pub fn opening(path: &str, wanted: f64) -> f64 {
    let Ok(info) = concat_media::probe(path) else {
        return 0.0;
    };
    let Some(seconds) = info.duration.map(|d| d.as_f64()) else {
        return 0.0;
    };
    // The last moment a cutaway may begin at and still be whole.
    let latest = seconds - wanted;
    if latest <= STEP {
        return 0.0;
    }
    let from = (seconds * INTO).min(latest);

    let Ok(video) = info.require_video() else {
        return from;
    };
    let aspect = if video.height > 0 {
        f64::from(video.width) / f64::from(video.height)
    } else {
        1.0
    };
    let options = DecodeOptions::default()
        .starting_at(concat_core::Rational::new((from * 1000.0) as i64, 1000))
        .scaled_to(
            LOOK_WIDTH,
            ((f64::from(LOOK_WIDTH) / aspect.max(0.01)).round() as u32).max(1),
        )
        // Exactly one frame every STEP, as a fraction rather than a whole
        // number of frames a second: a step of 0.4 is two and a half a
        // second, and rounding that to three would put every answer below
        // out by as much as three quarters of a second by the last try.
        .at_rate(concat_core::FrameRate::new(concat_core::Rational::new(
            1000,
            (STEP * 1000.0).round() as i64,
        )))
        .limited_to(TRIES as u64);
    let Ok(mut decoder) = Decoder::open(path, &options) else {
        return from;
    };
    for step in 0..TRIES {
        let at = from + step as f64 * STEP;
        if at > latest {
            break;
        }
        match decoder.next_frame() {
            Ok(Some(frame)) => {
                let (lit, varied) = picture(&frame);
                if lit >= LIT && varied >= VARIED {
                    return at;
                }
            }
            // The file ended or would not decode: what was asked for at the
            // start is as good as anything now.
            _ => break,
        }
    }
    from
}

/// A frame's average brightness and how much it varies across itself, both
/// zero to one.
///
/// Its own loop rather than the one in [`super::review`], because that one
/// answers a different question - how far this frame is from the one before
/// it - and folding both into one function would mean every caller paying
/// for the half it did not ask for.
fn picture(frame: &concat_core::Frame) -> (f64, f64) {
    use concat_core::frame::BYTES_PER_PIXEL;

    let pixels = frame.pixels();
    if pixels.len() < BYTES_PER_PIXEL {
        return (0.0, 0.0);
    }
    let mut levels = Vec::with_capacity(pixels.len() / BYTES_PER_PIXEL);
    let mut at = 0;
    while at + 2 < pixels.len() {
        // Weighted the way an eye weighs it: a frame of pure blue is dark
        // to look at, and a mean of the three channels calls it a third
        // lit.
        levels.push(
            (0.2126 * f64::from(pixels[at])
                + 0.7152 * f64::from(pixels[at + 1])
                + 0.0722 * f64::from(pixels[at + 2]))
                / 255.0,
        );
        at += BYTES_PER_PIXEL;
    }
    if levels.is_empty() {
        return (0.0, 0.0);
    }
    let mean = levels.iter().sum::<f64>() / levels.len() as f64;
    let spread = levels.iter().map(|level| (level - mean).abs()).sum::<f64>() / levels.len() as f64;
    (mean, spread)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at: f64, text: &str) -> (f64, f64, String) {
        (at, at + 1.5, text.to_owned())
    }

    #[test]
    fn the_word_worth_a_picture_is_not_the_one_holding_the_sentence_together() {
        assert_eq!(
            subject_of("and then I bought a motorcycle with it"),
            Some("motorcycle".to_owned())
        );
        assert_eq!(
            subject_of("to je bila najbolja investicija"),
            Some("investicija".to_owned())
        );
        // Nothing but grammar is nothing to search for.
        assert_eq!(subject_of("and so it was that they were"), None);
        assert_eq!(subject_of("ali ako je to tako"), None);
        assert_eq!(subject_of(""), None);
        // Numbers are not pictures.
        assert_eq!(subject_of("2026 and 1999"), None);
    }

    #[test]
    fn cutaways_are_spaced_out_rather_than_one_a_sentence() {
        let captions = vec![
            line(3.0, "I bought a motorcycle"),
            line(5.0, "the weather was terrible"),
            line(7.0, "we drove to the mountains"),
            line(23.0, "and the restaurant was closed"),
        ];
        let found = cues(&captions, APART, LENGTH);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!((found[0].at - 3.0).abs() < 1e-9);
        assert!((found[1].at - 23.0).abs() < 1e-9);
        assert!(found[1].at - found[0].at >= APART);
    }

    #[test]
    fn the_same_picture_is_never_asked_for_twice() {
        let captions = vec![
            line(3.0, "I bought a motorcycle"),
            line(13.0, "the motorcycle was red"),
            line(23.0, "and then the restaurant"),
        ];
        let found = cues(&captions, APART, LENGTH);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].words, "motorcycle");
        assert_eq!(found[1].words, "restaurant");
    }

    #[test]
    fn a_cutaway_runs_its_own_length_and_lands_on_the_line() {
        let captions = vec![line(5.0, "I bought a motorcycle")];
        let found = cues(&captions, APART, LENGTH);
        assert_eq!(found.len(), 1);
        assert!((found[0].at - 5.0).abs() < 1e-9);
        assert!((found[0].until - (5.0 + LENGTH)).abs() < 1e-9);
    }

    #[test]
    fn captions_in_any_order_come_out_in_time_order() {
        let captions = vec![
            line(23.0, "and then the restaurant"),
            line(3.0, "I bought a motorcycle"),
            line(43.0, "the mountains were quiet"),
        ];
        let found = cues(&captions, APART, LENGTH);
        assert_eq!(found.len(), 3);
        for pair in found.windows(2) {
            assert!(pair[0].at < pair[1].at, "{pair:?}");
        }
    }

    /// The opening belongs to whoever is talking.
    #[test]
    fn nothing_cuts_away_before_the_speaker_has_been_met() {
        let captions = vec![
            line(0.5, "I bought a motorcycle"),
            line(SETTLE + 0.5, "and then the restaurant"),
        ];
        let found = cues(&captions, APART, LENGTH);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].at >= SETTLE, "cut away at {}", found[0].at);
    }

    /// A word too short to mean anything is not searched for.
    #[test]
    fn a_short_concrete_noun_is_not_worth_a_picture() {
        // The one that shipped: "corn" fetched a full-screen photograph of
        // hazelnuts and laid it over the speaker.
        assert_eq!(subject_of("a quick jump to get into corn"), None);
        // While the same sentence with something to picture still works.
        assert_eq!(
            subject_of("a quick jump to get into gambling"),
            Some("gambling".to_owned())
        );
    }

    #[test]
    fn nothing_to_read_is_no_cutaways_rather_than_a_panic() {
        assert!(cues(&[], APART, LENGTH).is_empty());
        assert!(cues(&[line(3.0, "and so it was")], APART, LENGTH).is_empty());
        // A spacing of nothing is still a spacing, not a division by zero.
        assert_eq!(cues(&[line(3.0, "motorcycle")], 0.0, LENGTH).len(), 1);
    }
}
