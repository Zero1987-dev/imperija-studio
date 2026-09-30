// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Reading what the clip finder already decided.
//!
//! A separate tool watches a podcast and writes down which stretches are
//! worth posting: where each one starts and ends, which line inside it is
//! the shocking one, what to put on the screen, and what to write under the
//! post. It is careful work and it is already done by the time anybody
//! opens the editor.
//!
//! And until now the editor could not read a word of it. Every one of those
//! decisions had to be typed in again by hand, and the first clip cut this
//! way went out with `THE HOOK GOES HERE` still on the screen while the
//! line the finder had picked - a mayor posting ninety-five thousand
//! addresses - played at the twenty-third second, where nobody was left to
//! hear it.
//!
//! So this reads the sheet. It is a parser and nothing else: it makes no
//! decisions, it only stops them being thrown away.
//!
//! The format is the finder's Markdown, and it is read forgivingly - a
//! field that is missing is missing, not an error, because a sheet that
//! half-parses is worth more than one that refuses.

/// A stretch of the source, in seconds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Span {
    /// Where it starts in the source file.
    pub from: f64,
    /// And ends.
    pub to: f64,
}

impl Span {
    /// How long it runs.
    pub fn length(&self) -> f64 {
        (self.to - self.from).max(0.0)
    }
}

/// One clip the finder picked out.
#[derive(Clone, PartialEq, Debug)]
pub struct Pick {
    /// Its place in the sheet, from one.
    pub rank: usize,
    /// What the finder scored it out of ten. Zero when it did not say.
    pub score: u32,
    /// The heading, which is also the finder's suggested on-screen line.
    pub title: String,
    /// The line worth opening on, when the finder named one.
    ///
    /// Often inside `body` rather than before it: the shocking sentence
    /// usually arrives late, and putting it first is the whole of a cold
    /// open.
    pub hook: Option<Span>,
    /// What the finder wants on the screen. Falls back to the title.
    pub on_screen: String,
    /// The stretch to post.
    pub body: Span,
    /// The words to post under it, hashtags and all.
    pub caption: String,
}

/// A whole sheet: what it is about, and what it picked.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Sheet {
    /// The source's title, from the first heading.
    pub title: String,
    /// The address the finder watched, when the sheet names one.
    pub source: String,
    /// The picks, in the order they were written.
    pub picks: Vec<Pick>,
}

/// Seconds from `H:MM:SS` or `M:SS`. None for anything else.
fn clock(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut seconds = 0.0;
    for part in &parts {
        let value: f64 = part.trim().parse().ok()?;
        if value < 0.0 {
            return None;
        }
        seconds = seconds * 60.0 + value;
    }
    Some(seconds)
}

/// The two times in `(6:08 → 6:13, …)`, taking either arrow.
fn span_in(line: &str) -> Option<Span> {
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    let inside = &line[open + 1..close];
    let (left, right) = inside.split_once('→').or_else(|| inside.split_once("->"))?;
    // The right-hand side carries the rest of the parenthesis after a
    // comma: "6:13, host (Michael Sartain)".
    let right = right.split(',').next().unwrap_or(right);
    let from = clock(left)?;
    let to = clock(right)?;
    (to > from).then_some(Span { from, to })
}

/// Whatever follows `**LABEL:**` on a line, trimmed.
fn field(line: &str, label: &str) -> Option<String> {
    let mark = format!("**{label}:**");
    let at = line.find(&mark)?;
    Some(line[at + mark.len()..].trim().to_owned())
}

/// Drops a trailing parenthesis the finder adds for alternatives, and the
/// quotation marks it wraps a spoken line in.
fn plain(text: &str) -> String {
    let mut out = text.trim();
    for opener in ["(alternative:", "(alternativa:"] {
        if let Some(at) = out.find(opener) {
            out = out[..at].trim_end();
        }
    }
    out.trim()
        .trim_matches(|c| c == '"' || c == '“' || c == '”')
        .trim()
        .to_owned()
}

/// The first web address on a line, if any.
fn address_in(line: &str) -> Option<String> {
    let at = line.find("http")?;
    let rest = &line[at..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ')' || c == '>')
        .unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// The heading's rank, score and title, from `## 1. [9/10] Something`.
fn heading(line: &str) -> Option<(usize, u32, String)> {
    let rest = line.strip_prefix("## ")?.trim();
    let (number, rest) = rest.split_once('.')?;
    let rank: usize = number.trim().parse().ok()?;
    let rest = rest.trim();
    let (score, title) = match rest.strip_prefix('[') {
        Some(after) => match after.split_once(']') {
            Some((inside, title)) => {
                let value = inside.split('/').next().unwrap_or("0");
                (value.trim().parse().unwrap_or(0), title)
            }
            None => (0, rest),
        },
        None => (0, rest),
    };
    Some((rank, score, title.trim().to_owned()))
}

/// Reads a finder's sheet.
///
/// Forgiving on purpose: a pick with no body is dropped, and everything
/// else missing is simply absent. One malformed entry does not cost the
/// sheet.
pub fn read(text: &str) -> Sheet {
    let mut sheet = Sheet::default();
    let mut open: Option<Pick> = None;

    // Anything still being built, if it has somewhere to cut.
    fn keep(sheet: &mut Sheet, pick: Option<Pick>) {
        if let Some(pick) = pick
            && pick.body.length() > 0.0
        {
            sheet.picks.push(pick);
        }
    }

    for line in text.lines() {
        let trimmed = line.trim();
        if sheet.title.is_empty()
            && let Some(title) = trimmed.strip_prefix("# ")
        {
            sheet.title = title.trim().to_owned();
            continue;
        }
        if let Some((rank, score, title)) = heading(trimmed) {
            keep(&mut sheet, open.take());
            open = Some(Pick {
                rank,
                score,
                on_screen: title.clone(),
                title,
                hook: None,
                body: Span { from: 0.0, to: 0.0 },
                caption: String::new(),
            });
            continue;
        }
        if sheet.source.is_empty()
            && open.is_none()
            && let Some(address) = address_in(trimmed)
        {
            sheet.source = address;
        }
        let Some(pick) = open.as_mut() else {
            continue;
        };
        if trimmed.contains("**HOOK") {
            pick.hook = span_in(trimmed);
        } else if trimmed.contains("**TIJELO") || trimmed.contains("**BODY") {
            if let Some(span) = span_in(trimmed) {
                pick.body = span;
            }
        } else if let Some(text) = field(trimmed, "TEKST NA EKRANU")
            .or_else(|| field(trimmed, "ON SCREEN"))
            .or_else(|| field(trimmed, "ON-SCREEN"))
        {
            let text = plain(&text);
            if !text.is_empty() {
                pick.on_screen = text;
            }
        } else if let Some(text) = field(trimmed, "Caption") {
            pick.caption = plain(&text);
        }
    }
    keep(&mut sheet, open);
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sheet in the shape the finder actually writes.
    const SHEET: &str = "\
# Michael Reacts to Mamdani on AI in Schools
Michael Sartain · objavljeno 21.09.2026 · 3k pregleda · https://youtu.be/kfouoBrOxDY
Režim: **growth**

## 1. [9/10] Mayor Doxxed 95,000 People?!
- **HOOK (6:08 → 6:13, host (Michael Sartain)):** \"Mayor Mandami had doxed him\"  (https://youtu.be/kfouoBrOxDY?t=368)
- **TEKST NA EKRANU:** Mayor Doxxed 95,000 People?!   (alternative: The Mayor Leaked 95K Addresses)
- **TIJELO (5:49 → 6:18, 29s):** počinje sa \"I don't know if you guys saw\"  (https://youtu.be/kfouoBrOxDY?t=349)
- **Caption:** The mayor doxxed his OWN supporter 👀 #Mamdani
- **Region:** US · **Rizik:** needs context

## 2. [7/10] Over 40s Have The AI Edge
- **HOOK (16:37 → 16:41, host):** \"most people like me who are over the age of 40\"
- **TEKST NA EKRANU:** Over 40s Have The AI Edge
- **TIJELO (16:37 → 16:58, 21s):** počinje sa \"And and most people\"
- **Caption:** He says everyone over 40 has an advantage 🤔 #AIinSchools
";

    #[test]
    fn a_sheet_is_read_down_to_the_second() {
        let sheet = read(SHEET);
        assert_eq!(sheet.title, "Michael Reacts to Mamdani on AI in Schools");
        assert_eq!(sheet.source, "https://youtu.be/kfouoBrOxDY");
        assert_eq!(sheet.picks.len(), 2, "{:?}", sheet.picks);

        let first = &sheet.picks[0];
        assert_eq!(first.rank, 1);
        assert_eq!(first.score, 9);
        assert_eq!(first.title, "Mayor Doxxed 95,000 People?!");
        // 5:49 and 6:18.
        assert!((first.body.from - 349.0).abs() < 1e-9, "{:?}", first.body);
        assert!((first.body.to - 378.0).abs() < 1e-9, "{:?}", first.body);
        // 6:08 and 6:13, and the host's name in the same parenthesis did
        // not confuse the second time.
        let hook = first.hook.expect("a hook");
        assert!((hook.from - 368.0).abs() < 1e-9, "{hook:?}");
        assert!((hook.to - 373.0).abs() < 1e-9, "{hook:?}");
    }

    #[test]
    fn the_alternatives_are_not_part_of_the_line() {
        let sheet = read(SHEET);
        assert_eq!(sheet.picks[0].on_screen, "Mayor Doxxed 95,000 People?!");
        assert!(
            sheet.picks[0]
                .caption
                .starts_with("The mayor doxxed his OWN"),
            "{}",
            sheet.picks[0].caption
        );
    }

    /// The whole point: the shocking line is not at the start of the body.
    #[test]
    fn a_hook_inside_the_body_is_kept_as_its_own_span() {
        let sheet = read(SHEET);
        let first = &sheet.picks[0];
        let hook = first.hook.expect("a hook");
        assert!(hook.from > first.body.from, "the hook was at the start");
        assert!(hook.to <= first.body.to, "the hook ran past the body");

        // While the second one's hook *is* its opening, and a cold open
        // there would only say the same thing twice.
        let second = &sheet.picks[1];
        let hook = second.hook.expect("a hook");
        assert!((hook.from - second.body.from).abs() < 1e-9);
    }

    #[test]
    fn clocks_in_either_shape_and_nothing_else() {
        assert_eq!(clock("6:08"), Some(368.0));
        assert_eq!(clock("1:02:03"), Some(3723.0));
        assert_eq!(clock("0:00"), Some(0.0));
        assert_eq!(clock("nonsense"), None);
        assert_eq!(clock("5"), None);
        assert_eq!(clock("1:2:3:4"), None);
    }

    #[test]
    fn a_sheet_of_nothing_is_a_sheet_of_nothing() {
        assert_eq!(read("").picks.len(), 0);
        assert_eq!(read("just some words\nand more").picks.len(), 0);
        // A heading with no body to cut is dropped rather than laid at zero.
        assert_eq!(read("## 1. [9/10] Nothing here").picks.len(), 0);
    }
}
