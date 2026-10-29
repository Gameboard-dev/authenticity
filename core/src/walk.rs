//! Stepping through the AI characters the AI Detector pane lists, one prompt at a time: clicking an entry selects its
//! first occurrence in the editor and asks "Replace this character?", saying what it is and what it would become under
//! the rules and settings of "Replace unusual characters" (clean.rs). Enter replaces it (one edit, one undo), Esc skips
//! it, and the prompt moves on to the next occurrence in the list's order until the list ends or Stop is clicked.
//!
//! This module is the model, without the editor: the list's order (`groups`, which the pane draws), the walk through
//! it (`Walk`: what comes next, and the places of the rest after a replacement), finding an occurrence again after the
//! note changed (`find`), and what the prompt offers (`prompt`). The prompt itself is in app.rs.

use std::collections::{BTreeMap, VecDeque};

use crate::chars::{Hit, Kind};
use crate::clean::{self, Edit, Options};

/// The prompt decides one occurrence at a time, so it always offers the plain equivalent of a typographic character
/// (curly quotes, dashes, the ellipsis ...): the setting "Replace typographic characters too" governs only the bulk
/// command, Replace unusual characters, which changes a whole selection at once.
pub const PROMPT_OPTIONS: Options = Options { typography: true };

/// One entry of the pane's list: a character and its places in the note (UTF-16), in the order the pane shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub kind: Kind,
    pub code: u32,
    pub name: &'static str,
    pub short: &'static str,
    pub at: Vec<(u32, u32)>,
}

/// The pane's entries: the characters found, by kind and then code point, each with its places in the note's order.
pub fn groups(hits: &[Hit]) -> Vec<Group> {
    let mut by: BTreeMap<(u8, u32), Group> = BTreeMap::new();
    for h in hits {
        by.entry((h.kind as u8, h.code))
            .or_insert_with(|| Group { kind: h.kind, code: h.code, name: h.name, short: h.short, at: Vec::new() })
            .at
            .push((h.from, h.to));
    }
    by.into_values().collect()
}

/// One place to ask about.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub from: u32,
    pub to: u32,
    pub code: u32,
    pub kind: Kind,
    pub name: &'static str,
}

/// The places still to ask about, the current one first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Walk {
    queue: VecDeque<Occurrence>,
}

impl Walk {
    /// From the first occurrence of entry `group` to the end of the list.
    pub fn new(groups: &[Group], group: usize) -> Walk {
        let queue = groups
            .iter()
            .skip(group)
            .flat_map(|g| g.at.iter().map(|&(from, to)| Occurrence { from, to, code: g.code, kind: g.kind, name: g.name }))
            .collect();
        Walk { queue }
    }

    pub fn current(&self) -> Option<&Occurrence> {
        self.queue.front()
    }

    /// How many are left, the current one included.
    pub fn left(&self) -> usize {
        self.queue.len()
    }

    /// Esc: on to the next.
    pub fn skip(&mut self) {
        self.queue.pop_front();
    }

    /// The current one was found at another place (the note changed).
    pub fn moved(&mut self, from: u32, to: u32) {
        if let Some(o) = self.queue.front_mut() {
            (o.from, o.to) = (from, to);
        }
    }

    /// Enter: the current one was replaced, [from, to) by `insert`. Those inside that range went with it; those after it
    /// move by the change in length (an em dash's "--" is one unit longer).
    pub fn replaced(&mut self, from: u32, to: u32, insert: &str) {
        self.queue.pop_front();
        let grew = insert.encode_utf16().count() as i64 - (to - from) as i64;
        self.queue.retain(|o| o.to <= from || o.from >= to);
        for o in self.queue.iter_mut().filter(|o| o.from >= to) {
            o.from = (o.from as i64 + grew) as u32;
            o.to = (o.to as i64 + grew) as u32;
        }
    }
}

/// Where an occurrence is in the note now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// Still at its place.
    Here,
    /// Not there any more, but the same character is elsewhere: the nearest place.
    Moved(u32, u32),
    /// No longer in the note.
    Gone,
}

pub fn find(text: &str, o: &Occurrence) -> Found {
    let mut at = 0u32;
    let mut best: Option<(u32, u32)> = None;
    for c in text.chars() {
        let w = c.len_utf16() as u32;
        if c as u32 == o.code {
            if at == o.from {
                return Found::Here;
            }
            if best.is_none_or(|(b, _)| at.abs_diff(o.from) < b.abs_diff(o.from)) {
                best = Some((at, at + w));
            }
        }
        at += w;
    }
    best.map_or(Found::Gone, |(f, t)| Found::Moved(f, t))
}

/// What the prompt says and offers for one occurrence.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    /// "Arabic comma (U+060C) → ,", "zero-width space (U+200B) → remove", or "…: no automatic replacement …".
    pub line: String,
    /// The replacement, if there is one; without one, the prompt asks for the character to put in its place.
    pub edit: Option<Edit>,
}

/// How a replacement reads after the arrow.
fn becomes(e: &Edit, own: u32) -> String {
    let wider = e.to - e.from > own;
    let what = match e.insert.as_str() {
        "" => "remove".to_string(),
        " " => "a space".to_string(),
        s if wider || s.trim() != s => format!("\u{201c}{s}\u{201d}"),
        s => s.to_string(),
    };
    if wider { format!("{what} (with the spaces beside it)") } else { what }
}

pub fn prompt(text: &str, o: &Occurrence) -> Prompt {
    let mut name = o.name.to_string();
    if let Some(first) = name.get(..1) {
        name.replace_range(..1, &first.to_uppercase());
    }
    let shown = match char::from_u32(o.code) {
        Some(c) if matches!(o.kind, Kind::LookAlike) => format!("\u{201c}{c}\u{201d}, {}", name.to_lowercase()),
        _ => name,
    };
    let what = format!("{shown} (U+{:04X})", o.code);
    // the en dash's plain equivalent is the hyphen-minus wherever it stands, as in the bulk command
    let found = if o.code == 0x2013 {
        Some(Edit { from: o.from, to: o.to, insert: "-".into(), change: clean::Change::Typographic })
    } else {
        clean::edit_at(text, o.from, PROMPT_OPTIONS)
    };
    match found {
        Some(e) => Prompt { line: format!("{what} \u{2192} {}", becomes(&e, o.to - o.from)), edit: Some(e) },
        None => Prompt { line: format!("{what}: no automatic replacement here. Type the character to put in its place, or skip it."), edit: None },
    }
}

/// Characters that join the one before them into what a reader sees as one character (a grapheme): combining marks,
/// variation selectors, the zero-width joiner, emoji skin tones, tag characters.
fn joins(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A | 0x064B..=0x065F | 0x0900..=0x0903
        | 0x093A..=0x094F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200D | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F
        | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
}

/// What the prompt's box keeps of what was typed: the last character as a reader sees it (one grapheme: a letter with
/// its accents, an emoji with its modifiers), so typing another key replaces the one there.
pub fn last_grapheme(typed: &str) -> String {
    let cs: Vec<char> = typed.chars().collect();
    let Some(mut start) = cs.len().checked_sub(1) else { return String::new() };
    // back over the joining characters to the one they join, and over a zero-width joiner's sequence
    while start > 0 && (joins(cs[start]) || cs[start - 1] == '\u{200D}') {
        start -= 1;
    }
    // a flag is a pair of regional indicators
    let regional = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
    if start > 0 && regional(cs[start]) && cs[..start].iter().rev().take_while(|c| regional(**c)).count() % 2 == 1 {
        start -= 1;
    }
    cs[start..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chars::scan;

    #[test]
    fn the_list_s_order_and_the_walk() {
        // two zero-width spaces, an Arabic comma, a no-break space (UTF-16 places)
        let text = "a\u{200B}b x\u{060C} c\u{00A0}d e\u{200B}f";
        let g = groups(&scan(text, false));
        assert_eq!(g.iter().map(|g| (g.code, g.at.len())).collect::<Vec<_>>(), vec![(0x200B, 2), (0x00A0, 1), (0x060C, 1)]);
        let mut w = Walk::new(&g, 0);
        assert_eq!(w.left(), 4);
        assert_eq!(w.current().map(|o| o.from), Some(1));
        w.skip();
        assert_eq!(w.current().map(|o| o.from), Some(12), "the next of the same entry first");
        // starting at an entry leaves out the ones above it
        assert_eq!(Walk::new(&g, 2).left(), 1);
        assert_eq!(Walk::new(&g, 3).current(), None);
    }

    #[test]
    fn a_replacement_moves_the_rest() {
        let text = "a\u{200B}b x\u{060C} c\u{00A0}d e\u{200B}f";
        let g = groups(&scan(text, false));
        let mut w = Walk::new(&g, 0);
        let e = prompt(text, w.current().unwrap()).edit.unwrap();
        assert_eq!((e.from, e.to, e.insert.as_str()), (1, 2, ""));
        w.replaced(e.from, e.to, &e.insert);
        // the second zero-width space was at 12; one unit was removed before it
        assert_eq!(w.current().map(|o| (o.from, o.to)), Some((11, 12)));
        let text = "ab x\u{060C} c\u{00A0}d e\u{200B}f";
        assert_eq!(find(text, w.current().unwrap()), Found::Here);
        // a character typed in: two UTF-16 units for one, so those after it move on by one
        w.replaced(11, 12, "\u{1F600}");
        assert_eq!(w.left(), 2);
        // an em dash becomes two characters in its place: the no-break space before it stays, and those after move on
        let t = "a\u{00A0}\u{2014} b\u{200B}";
        let mut w = Walk { queue: [(2, 3, 0x2014, Kind::Dash), (1, 2, 0x00A0, Kind::Space), (5, 6, 0x200B, Kind::Hidden)].map(|(from, to, code, kind)| Occurrence { from, to, code, kind, name: "x" }).into() };
        let e = prompt(t, w.current().unwrap()).edit.unwrap();
        assert_eq!((e.from, e.to, e.insert.as_str()), (2, 3, "--"));
        w.replaced(e.from, e.to, &e.insert);
        assert_eq!(w.left(), 2);
        assert_eq!(w.current().map(|o| (o.from, o.to)), Some((1, 2)));
    }

    #[test]
    fn found_again_after_the_note_changed() {
        let o = Occurrence { from: 3, to: 4, code: 0x200B, kind: Kind::Hidden, name: "zero-width space" };
        assert_eq!(find("abc\u{200B}", &o), Found::Here);
        assert_eq!(find("xxabc\u{200B} \u{200B}", &o), Found::Moved(5, 6), "the nearest");
        assert_eq!(find("abcd", &o), Found::Gone);
    }

    #[test]
    fn what_the_prompt_offers() {
        let text = "x\u{060C} y\u{200B}z d\u{0430}ta \u{201c}q\u{201d} and a \u{2014} b, pp. 3\u{2013}5, so \u{2013} it\u{2026}";
        let at = |from: u32, kind, name| Occurrence { from, to: from + 1, code: text.encode_utf16().nth(from as usize).unwrap() as u32, kind, name };
        let p = prompt(text, &at(1, Kind::Punctuation, "Arabic comma"));
        assert_eq!(p.line, "Arabic comma (U+060C) \u{2192} ,");
        assert!(p.edit.is_some());
        assert_eq!(prompt(text, &at(4, Kind::Hidden, "zero-width space")).line, "Zero-width space (U+200B) \u{2192} remove");
        assert_eq!(prompt(text, &at(8, Kind::LookAlike, "Cyrillic letter in a Latin word")).line, "\u{201c}\u{0430}\u{201d}, cyrillic letter in a latin word (U+0430) \u{2192} a");
        // typographic characters: always offered here, whatever the bulk command's setting
        assert_eq!(prompt(text, &at(12, Kind::Typographic, "curly double quote")).line, "Curly double quote (U+201C) \u{2192} \"");
        let d = prompt(text, &at(22, Kind::Dash, "em dash"));
        assert_eq!(d.line, "Em dash (U+2014) \u{2192} --");
        assert_eq!(d.edit.as_ref().map(|e| (e.from, e.to, e.insert.as_str())), Some((22, 23, "--")), "in its own place, the spaces kept");
        // the en dash is a hyphen-minus here, between digits or between spaces alike
        for at_ in [32, 39] {
            let e = prompt(text, &at(at_, Kind::Dash, "en dash"));
            assert_eq!(e.line, "En dash (U+2013) \u{2192} -");
            assert_eq!(e.edit.map(|e| (e.from, e.to, e.insert)), Some((at_, at_ + 1, "-".to_string())));
        }
        assert_eq!(prompt(text, &at(43, Kind::Typographic, "ellipsis character")).line, "Ellipsis character (U+2026) \u{2192} ...");
        // a character with no plain equivalent where it is (an Arabic comma in Arabic text): the user types one
        let ar = "\u{0645}\u{060C} \u{0645}";
        let o = Occurrence { from: 1, to: 2, code: 0x060C, kind: Kind::Punctuation, name: "Arabic comma" };
        let p = prompt(ar, &o);
        assert_eq!(p.line, "Arabic comma (U+060C): no automatic replacement here. Type the character to put in its place, or skip it.");
        assert_eq!(p.edit, None);
    }

    #[test]
    fn the_box_keeps_one_character() {
        assert_eq!(last_grapheme(""), "");
        assert_eq!(last_grapheme(","), ",");
        assert_eq!(last_grapheme("ab"), "b", "typing another key replaces the one there");
        assert_eq!(last_grapheme("xe\u{0301}"), "e\u{0301}", "a letter with its accent");
        assert_eq!(last_grapheme("a\u{1F44D}\u{1F3FD}"), "\u{1F44D}\u{1F3FD}", "an emoji with its skin tone");
        assert_eq!(last_grapheme("a\u{1F468}\u{200D}\u{1F4BB}"), "\u{1F468}\u{200D}\u{1F4BB}", "a joined emoji");
        assert_eq!(last_grapheme("a\u{1F1EC}\u{1F1E7}"), "\u{1F1EC}\u{1F1E7}", "a flag");
        assert_eq!(last_grapheme("\u{1F1EC}\u{1F1E7}\u{1F1EB}\u{1F1F7}"), "\u{1F1EB}\u{1F1F7}", "the last of two flags");
    }

    /// Every character the AI characters list can show, in a Latin word ("xa?a x"), with the prompt's replacement or
    /// none; printed with --nocapture. The ones with none get the box.
    #[test]
    fn the_characters_with_no_automatic_replacement() {
        let mut none = Vec::new();
        for u in (0..0x30000u32).chain(0xE0000..0xE01F0) {
            let Some(c) = char::from_u32(u) else { continue };
            let text = format!("xa{c}a x");
            let Some(h) = scan(&text, true).into_iter().find(|h| h.code == u) else { continue };
            let o = Occurrence { from: h.from, to: h.to, code: u, kind: h.kind, name: h.name };
            if prompt(&text, &o).edit.is_none() {
                none.push((u, h.name));
            }
        }
        for (u, name) in &none {
            println!("U+{u:04X} {name}");
        }
        let has = |u: u32| none.iter().any(|(x, _)| *x == u);
        // offered in the prompt: the en and em dash, curly quotes, the ellipsis, the no-break space, primes
        for u in [0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x2026, 0x00A0, 0x2032, 0x2212] {
            assert!(!has(u), "U+{u:04X} should have a replacement");
        }
    }
}
