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
pub const LENGTH: f64 = 2.4;

/// And the least time between two of them.
///
/// A cutaway every other sentence is a slideshow. Eight seconds is roughly
/// where they stop reading as decoration and start reading as illustration.
pub const APART: f64 = 8.0;

/// The shortest word worth a picture. Under this it is grammar the list
/// above happened to miss.
pub const SHORTEST_WORD: usize = 4;

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
            line(0.0, "I bought a motorcycle"),
            line(2.0, "the weather was terrible"),
            line(4.0, "we drove to the mountains"),
            line(20.0, "and the restaurant was closed"),
        ];
        let found = cues(&captions, APART, LENGTH);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!((found[0].at - 0.0).abs() < 1e-9);
        assert!((found[1].at - 20.0).abs() < 1e-9);
        assert!(found[1].at - found[0].at >= APART);
    }

    #[test]
    fn the_same_picture_is_never_asked_for_twice() {
        let captions = vec![
            line(0.0, "I bought a motorcycle"),
            line(10.0, "the motorcycle was red"),
            line(20.0, "and then the restaurant"),
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
            line(20.0, "and then the restaurant"),
            line(0.0, "I bought a motorcycle"),
            line(40.0, "the mountains were quiet"),
        ];
        let found = cues(&captions, APART, LENGTH);
        assert_eq!(found.len(), 3);
        for pair in found.windows(2) {
            assert!(pair[0].at < pair[1].at, "{pair:?}");
        }
    }

    #[test]
    fn nothing_to_read_is_no_cutaways_rather_than_a_panic() {
        assert!(cues(&[], APART, LENGTH).is_empty());
        assert!(cues(&[line(0.0, "and so it was")], APART, LENGTH).is_empty());
        // A spacing of nothing is still a spacing, not a division by zero.
        assert_eq!(cues(&[line(0.0, "motorcycle")], 0.0, LENGTH).len(), 1);
    }
}
