//! "Replace unusual characters": in the selected Markdown, characters with a clear plain equivalent are replaced by
//! it. The one command of the plugin that rewrites text: run only on the selection the user chose, after a dialog that
//! says how many characters of which kinds will change, as one edit (one undo).
//!
//! Always, when the plain equivalent is clear:
//!
//! - **invisible characters are removed**: zero-width spaces, word joiners, byte-order marks, soft hyphens, direction
//!   marks and embeddings, invisible mathematical operators, Unicode tag characters, Hangul fillers. Zero-width
//!   joiners and non-joiners and variation selectors are removed only between Latin text: beside an emoji or a letter
//!   of another script they do a job (an emoji sequence, a Persian or Indic word) and are kept;
//! - **unusual spaces become a space**: no-break, narrow no-break, figure, punctuation, thin, hair, em and en spaces,
//!   the medium mathematical and the ideographic space;
//! - **look-alike punctuation becomes the keyboard's**: Unicode hyphens become `-`; fullwidth, small-form, ideographic
//!   and Arabic commas, full stops, colons, semicolons, question and exclamation marks, and the Greek question mark
//!   (a semicolon's twin) become `, . : ; ? !` where the text around them is Latin (in Chinese, Japanese, Arabic or
//!   Greek text they are that language's own punctuation, and are kept); a modifier letter or acute accent used as an
//!   apostrophe between Latin letters becomes `'` (the ʻokina, a letter, is kept); the one-dot leader becomes `.`, the
//!   ratio sign `:` where it is not between digits;
//! - **look-alike letters become Latin ones**: a Cyrillic or Greek letter inside a Latin word ("dаta" with a Cyrillic
//!   а), styled mathematical letters and digits (𝐁𝐨𝐥𝐝), and fullwidth letters and digits in Latin text.
//!
//! Typography the writer may want is left as it is unless `typography` is on (the setting "Replace typographic
//! characters too", for the bulk command; the AI characters step-through, one character at a time, always has it on):
//! curly quotes and low quotes, primes, the ellipsis character, em and en dashes (also the vertical forms) and the
//! horizontal bar, the figure dash, the minus sign, bullets and arrows. Never touched: accented letters, ×, °, other
//! symbols, and letters of other scripts outside Latin words.
//!
//! What is not prose is left as it is: fenced code and maths blocks, inline code and inline maths, table delimiter
//! rows, and horizontal rules (including front matter's `---`).

/// The kinds of change, for the dialog's count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Invisible,
    Space,
    Punctuation,
    Letter,
    Typographic,
}

impl Change {
    pub const ALL: [Change; 5] = [Change::Invisible, Change::Space, Change::Punctuation, Change::Letter, Change::Typographic];

    /// How the dialog names `n` of them.
    pub fn describe(self, n: usize) -> String {
        let s = |one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
        match self {
            Change::Invisible => s("invisible character removed", "invisible characters removed"),
            Change::Space => s("unusual space made a space", "unusual spaces made spaces"),
            Change::Punctuation => s("look-alike punctuation mark made the keyboard's", "look-alike punctuation marks made the keyboard's"),
            Change::Letter => s("look-alike letter or digit made a Latin one", "look-alike letters or digits made Latin ones"),
            Change::Typographic => s("typographic character made the keyboard's", "typographic characters made the keyboard's"),
        }
    }
}

/// What to replace beyond the clear cases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Also curly quotes, dashes, the ellipsis character, primes, the minus sign, bullets and arrows.
    pub typography: bool,
}

/// What a replacement changed: how many characters of each kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts(pub [usize; 5]);

impl Counts {
    fn add(&mut self, c: Change) {
        self.0[c as usize] += 1;
    }

    pub fn total(&self) -> usize {
        self.0.iter().sum()
    }

    pub fn of(&self, c: Change) -> usize {
        self.0[c as usize]
    }

    /// "3 invisible characters removed, 2 unusual spaces made spaces" (the kinds with any).
    pub fn describe(&self) -> String {
        Change::ALL.iter().filter(|c| self.of(**c) > 0).map(|c| c.describe(self.of(*c))).collect::<Vec<_>>().join(", ")
    }
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c) || ('\u{1E00}'..='\u{1EFF}').contains(&c)
}

/// Always removed: invisible characters with no job in text.
fn invisible(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' | '\u{180E}' | '\u{034F}' | '\u{2061}'..='\u{2064}' | '\u{200E}' | '\u{200F}'
        | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{E0000}'..='\u{E007F}' | '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}')
}

/// Removed only between Latin text: joiners and variation selectors, which shape emoji and other scripts.
fn joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}')
}

fn odd_space(c: char) -> bool {
    matches!(c, '\u{00A0}' | '\u{202F}' | '\u{2000}'..='\u{200A}' | '\u{205F}' | '\u{3000}')
}

/// Punctuation of other scripts and forms, as the keyboard's (used where the text around is Latin).
fn foreign_punctuation(c: char) -> Option<char> {
    Some(match c {
        '\u{3001}' | '\u{060C}' | '\u{FE50}' | '\u{FE51}' => ',',
        '\u{3002}' | '\u{FE52}' => '.',
        '\u{037E}' | '\u{FE54}' | '\u{061B}' => ';',
        '\u{FE55}' => ':',
        '\u{FE56}' | '\u{061F}' => '?',
        '\u{FE57}' => '!',
        // fullwidth forms of ASCII punctuation and symbols (letters and digits are look-alike letters)
        '\u{FF01}'..='\u{FF5E}' if !char::from_u32(c as u32 - 0xFEE0).is_some_and(|a| a.is_ascii_alphanumeric()) => char::from_u32(c as u32 - 0xFEE0)?,
        _ => return None,
    })
}

/// A fullwidth letter or digit, or a styled mathematical one, as the plain one.
fn plain_letter(c: char) -> Option<(char, bool)> {
    let u = c as u32;
    match u {
        // (the plain letter, whether it counts only in Latin text)
        0xFF21..=0xFF3A | 0xFF41..=0xFF5A | 0xFF10..=0xFF19 => char::from_u32(u - 0xFEE0).map(|a| (a, true)),
        0x1D400..=0x1D6A3 => {
            let r = (u - 0x1D400) % 52;
            char::from_u32(if r < 26 { 'A' as u32 + r } else { 'a' as u32 + r - 26 }).map(|a| (a, false))
        }
        0x1D7CE..=0x1D7FF => char::from_u32('0' as u32 + (u - 0x1D7CE) % 10).map(|a| (a, false)),
        _ => None,
    }
}

/// The Latin letter a Cyrillic or Greek one imitates.
fn latin(c: char) -> Option<char> {
    Some(match c {
        'а' | 'α' => 'a', 'е' => 'e', 'о' | 'ο' => 'o', 'р' | 'ρ' => 'p', 'с' => 'c', 'у' => 'y', 'х' | 'χ' => 'x',
        'і' | 'ι' => 'i', 'ј' => 'j', 'ѕ' => 's', 'ԁ' => 'd', 'һ' => 'h', 'ӏ' => 'l', 'ν' => 'v', 'κ' => 'k', 'τ' => 't',
        'υ' => 'u', 'А' | 'Α' => 'A', 'В' | 'Β' => 'B', 'Е' | 'Ε' => 'E', 'К' | 'Κ' => 'K', 'М' | 'Μ' => 'M',
        'Н' | 'Η' => 'H', 'О' | 'Ο' => 'O', 'Р' | 'Ρ' => 'P', 'С' => 'C', 'Т' | 'Τ' => 'T', 'Х' | 'Χ' => 'X',
        'У' | 'Υ' => 'Y', 'І' | 'Ι' => 'I', 'Ј' => 'J', 'Ѕ' => 'S', 'Ζ' => 'Z', 'Ν' => 'N',
        _ => return None,
    })
}

/// Typographic characters as the keyboard's (only with `typography`); dashes are dash_out's.
fn typographic(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => "'",
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => "\"",
        '\u{2026}' => "...",
        '\u{2012}' | '\u{2212}' => "-",
        '\u{2022}' | '\u{2023}' | '\u{25E6}' | '\u{2043}' => "-",
        '\u{2192}' | '\u{21D2}' => "->",
        '\u{2190}' => "<-",
        '\u{2194}' => "<->",
        _ => return None,
    })
}

/// Whether the text around place `i` of a line is Latin: the nearest letter before it (or, with none, after it).
fn latin_around(cs: &[char], i: usize) -> bool {
    let before = cs[..i].iter().rev().find(|c| c.is_alphabetic() && plain_letter(**c).is_none());
    let after = || cs[i + 1..].iter().find(|c| c.is_alphabetic() && plain_letter(**c).is_none());
    before.or_else(after).is_none_or(|c| is_latin(*c))
}

/// Whether a neighbour of a joiner is plain Latin text (letters, digits, ASCII, white space) or nothing.
fn plain_neighbour(c: Option<&char>) -> bool {
    c.is_none_or(|c| c.is_ascii() || is_latin(*c) || c.is_whitespace())
}

/// One replacement in the text: [from, to) (UTF-16, as the editor counts) becomes `insert` (empty: removed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub from: u32,
    pub to: u32,
    pub insert: String,
    pub change: Change,
}

/// What one character (or an em dash with its spaces) does: the place after it, its kind of change if it changes,
/// and where its output starts (before its own when an em dash takes back the spaces written before it).
fn step(cs: &[char], i: usize, in_latin_word: &[bool], out: &mut String, o: Options) -> (usize, Option<Change>, usize) {
    let c = cs[i];
    let start = out.len();
    let prev = if i > 0 { cs.get(i - 1) } else { None };
    let next = cs.get(i + 1);
    if o.typography {
        // an em dash (and its relatives: the horizontal bar, the two- and three-em dashes, the small and vertical em
        // dashes) becomes "--" where it stands, the spaces around it as they were
        if matches!(c, '\u{2014}' | '\u{2015}' | '\u{2E3A}' | '\u{2E3B}' | '\u{FE58}' | '\u{FE31}') {
            out.push_str("--");
            return (i + 1, Some(Change::Typographic), start);
        }
        // an en dash is a hyphen-minus wherever it stands
        if matches!(c, '\u{2013}' | '\u{FE32}') {
            out.push('-');
            return (i + 1, Some(Change::Typographic), start);
        }
        if let Some(t) = typographic(c) {
            out.push_str(t);
            return (i + 1, Some(Change::Typographic), start);
        }
    }
    let between_latin = prev.is_some_and(|p| is_latin(*p)) && next.is_some_and(|d| is_latin(*d));
    let digits_around = prev.is_some_and(char::is_ascii_digit) && next.is_some_and(char::is_ascii_digit);
    let change = if invisible(c) || (joiner(c) && plain_neighbour(prev) && plain_neighbour(next)) {
        Some(Change::Invisible)
    } else if odd_space(c) {
        if !out.ends_with(' ') && next != Some(&' ') {
            out.push(' ');
        }
        Some(Change::Space)
    } else if matches!(c, '\u{2010}' | '\u{2011}' | '\u{FE63}' | '\u{FF0D}') {
        out.push('-');
        Some(Change::Punctuation)
    } else if matches!(c, '\u{02BC}' | '\u{02B9}' | '\u{00B4}') && between_latin {
        out.push('\'');
        Some(Change::Punctuation)
    } else if c == '\u{2024}' || (c == '\u{2236}' && !digits_around) {
        out.push(if c == '\u{2024}' { '.' } else { ':' });
        Some(Change::Punctuation)
    } else if let Some(p) = foreign_punctuation(c).filter(|_| latin_around(cs, i)) {
        out.push(p);
        Some(Change::Punctuation)
    } else if let Some(l) = in_latin_word[i].then(|| latin(c)).flatten() {
        out.push(l);
        Some(Change::Letter)
    } else if let Some((l, _)) = plain_letter(c).filter(|(_, latin_only)| !latin_only || latin_around(cs, i)) {
        out.push(l);
        Some(Change::Letter)
    } else {
        out.push(c);
        None
    };
    (i + 1, change, start)
}

/// One line of Markdown; inline code and inline maths are copied as they are. Each replacement is added to `edits`
/// with its place in the line (in characters).
fn line(cs: &[char], out: &mut String, n: &mut Counts, o: Options, edits: &mut Vec<(usize, usize, String, Change)>) {
    // letters from another script count only inside a word that also has Latin letters
    let mut in_latin_word = vec![false; cs.len()];
    let mut i = 0;
    while i < cs.len() {
        if cs[i].is_alphabetic() {
            let start = i;
            while i < cs.len() && cs[i].is_alphabetic() {
                i += 1;
            }
            let latin_word = cs[start..i].iter().any(|&c| is_latin(c));
            in_latin_word[start..i].iter_mut().for_each(|x| *x = latin_word);
        } else {
            i += 1;
        }
    }
    // where each character's output began, so an em dash that takes back spaces knows which characters they were
    let mut pos = vec![0usize; cs.len()];
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        let before = out.len();
        // `code` and $maths$: up to the closing mark, untouched
        if c == '`' || (c == '$' && cs.get(i + 1).is_some_and(|d| !d.is_whitespace())) {
            if let Some(e) = (i + 1..cs.len()).find(|&k| cs[k] == c) {
                out.extend(&cs[i..=e]);
                pos[i..=e].iter_mut().for_each(|p| *p = before);
                i = e + 1;
                continue;
            }
        }
        let (j, change, start) = step(cs, i, &in_latin_word, out, o);
        pos[i..j].iter_mut().for_each(|p| *p = before);
        if let Some(change) = change {
            n.add(change);
            // the characters whose output the em dash took back are part of its replacement
            let mut s = i;
            while s > 0 && pos[s - 1] >= start {
                s -= 1;
            }
            while edits.last().is_some_and(|e| e.0 >= s) {
                edits.pop();
            }
            edits.push((s, j, out[start..].to_string(), change));
        }
        i = j;
    }
}

/// The replacements in `text` (clean's), with their places, and how many of each kind.
fn run(text: &str, o: Options) -> (String, Counts, Vec<Edit>) {
    let mut out = String::with_capacity(text.len());
    let mut n = Counts::default();
    let mut edits = Vec::new();
    let mut fence: Option<&str> = None; // "```", "~~~" or "$$" while inside such a block
    let mut base = 0u32; // the line's start (UTF-16)
    for (k, l) in text.split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
            base += 1;
        }
        let width: u32 = l.chars().map(|c| c.len_utf16() as u32).sum();
        let t = l.trim();
        let opens = ["```", "~~~", "$$"].into_iter().find(|m| t.starts_with(m));
        let structural = t.len() >= 3 && t.chars().all(|c| c == '-' || c == ' ') // a rule or front matter's fence
            || t.starts_with('|') && t.chars().all(|c| "|-: ".contains(c)); // a table's delimiter row
        if fence.is_some() || opens.is_some() || structural {
            out.push_str(l);
            match fence {
                // a block opens, unless it is `$$ x $$` on one line
                None => fence = opens.filter(|m| !(*m == "$$" && t.len() > 2 && t[2..].contains("$$"))),
                Some(m) if t.starts_with(m) || (m == "$$" && t.ends_with("$$")) => fence = None,
                Some(_) => {}
            }
            base += width;
            continue;
        }
        let cs: Vec<char> = l.chars().collect();
        let mut at = Vec::with_capacity(cs.len() + 1); // each character's place (UTF-16) in the text
        let mut u = base;
        for c in &cs {
            at.push(u);
            u += c.len_utf16() as u32;
        }
        at.push(u);
        let mut found = Vec::new();
        line(&cs, &mut out, &mut n, o, &mut found);
        edits.extend(found.into_iter().map(|(s, e, insert, change)| Edit { from: at[s], to: at[e], insert, change }));
        base += width;
    }
    (out, n, edits)
}

/// The selected Markdown with the characters replaced, and how many of each kind were.
pub fn clean(text: &str, o: Options) -> (String, Counts) {
    let (out, n, _) = run(text, o);
    (out, n)
}

/// Every replacement `clean` makes in `text`, in order, with its place (UTF-16).
pub fn edits(text: &str, o: Options) -> Vec<Edit> {
    run(text, o).2
}

/// The replacement of the character at `at` (UTF-16) of `text`, if it has one under these options.
pub fn edit_at(text: &str, at: u32, o: Options) -> Option<Edit> {
    edits(text, o).into_iter().find(|e| e.from <= at && at < e.to.max(e.from + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEEP: Options = Options { typography: false };
    const ALL: Options = Options { typography: true };

    fn c(s: &str) -> String {
        clean(s, KEEP).0
    }

    #[test]
    fn invisible_characters_are_removed() {
        let (out, n) = clean("a\u{200B}b\u{FEFF}c soft\u{00AD}ware \u{202E}x\u{202C} t\u{E0041}ag \u{2061}", KEEP);
        assert_eq!(out, "abc software x tag ");
        assert_eq!(n.of(Change::Invisible), 7);
        // joiners and variation selectors between Latin text go; beside an emoji or another script they stay
        assert_eq!(c("da\u{200D}ta wo\u{FE0F}rd"), "data word");
        assert_eq!(c("👩\u{200D}💻 ok"), "👩\u{200D}💻 ok");
        assert_eq!(c("❤\u{FE0F}"), "❤\u{FE0F}");
        assert_eq!(c("می\u{200C}خواهم"), "می\u{200C}خواهم");
    }

    #[test]
    fn unusual_spaces_become_a_space() {
        let (out, n) = clean("p.\u{00A0}3, 5\u{202F}kg, a\u{2009}b, c\u{3000}d, e \u{00A0}f", KEEP);
        assert_eq!(out, "p. 3, 5 kg, a b, c d, e f");
        assert_eq!(n.of(Change::Space), 5);
    }

    #[test]
    fn look_alike_punctuation_in_latin_text() {
        let (out, n) = clean("one，two。 three；four： five？six！ x\u{3001}y z\u{060C}w Is it\u{037E} it\u{02BC}s don\u{00B4}t a\u{2010}b c\u{2011}d", KEEP);
        assert_eq!(out, "one,two. three;four: five?six! x,y z,w Is it; it's don't a-b c-d");
        assert_eq!(n.of(Change::Punctuation), 13);
        // in another script's text its own punctuation stays
        assert_eq!(c("我们，你们。"), "我们，你们。");
        assert_eq!(c("مرحبا، صديقي"), "مرحبا، صديقي");
        assert_eq!(c("Τι κάνεις;"), "Τι κάνεις;");
        assert_eq!(c("Τι κάνεις\u{037E}"), "Τι κάνεις\u{037E}");
        // the ʻokina is a letter; a ratio between digits is a ratio
        assert_eq!(c("Hawaiʻi 3∶2 note∶ x"), "Hawaiʻi 3∶2 note: x");
    }

    #[test]
    fn look_alike_letters() {
        let (out, n) = clean("d\u{0430}ta \u{1D401}\u{1D428}ld \u{FF21}\u{FF11} x\u{1D7CF}", KEEP);
        assert_eq!(out, "data Bold A1 x1");
        assert_eq!(n.of(Change::Letter), 6);
        assert_eq!(c("Greek αβγ stays, Cyrillic мир stays"), "Greek αβγ stays, Cyrillic мир stays");
        assert_eq!(c("日本ＡＢＣ"), "日本ＡＢＣ", "fullwidth letters in Japanese text stay");
    }

    #[test]
    fn typography_is_kept_unless_asked() {
        let t = "“q” ‘s’ it’s wait… a — b 3–5 −2 • x → y 5′ café × 2 °C naïve";
        let (out, n) = clean(t, KEEP);
        assert_eq!(out, t, "curly quotes, dashes, ellipsis, minus, bullets, arrows, primes, accents, ×, ° all kept");
        assert_eq!(n.total(), 0);
        let (out, n) = clean(t, ALL);
        assert_eq!(out, "\"q\" 's' it's wait... a -- b 3-5 -2 - x -> y 5' café × 2 °C naïve");
        assert_eq!(n.of(Change::Typographic), 12);
    }

    #[test]
    fn dashes_with_typography() {
        let d = |s: &str| clean(s, ALL).0;
        // an em dash is "--" where it stands, with the spaces around it as they were
        assert_eq!(d("a — b"), "a -- b");
        assert_eq!(d("a—b"), "a--b");
        assert_eq!(d("The model — small and fast — wins."), "The model -- small and fast -- wins.");
        assert_eq!(d("It works—mostly."), "It works--mostly.");
        assert_eq!(d("A list:\n— first"), "A list:\n-- first");
        assert_eq!(d("| a | — |"), "| a | -- |");
        // its relatives too: the horizontal bar, the two- and three-em dashes, the small and vertical em dashes
        assert_eq!(d("x \u{2015} y \u{2E3A} z \u{2E3B} w\u{FE58}v\u{FE31}u"), "x -- y -- z -- w--v--u");
        // an en dash is "-" wherever it stands; the figure dash and the minus sign too
        assert_eq!(d("pp. 3–5 and a well–known fact"), "pp. 3-5 and a well-known fact");
        assert_eq!(d("one – two"), "one - two");
        assert_eq!(d("tel 555\u{2012}0100, \u{2212}2"), "tel 555-0100, -2");
        assert_eq!(d("plain -- and --- stay"), "plain -- and --- stay");
    }

    #[test]
    fn code_maths_and_structure_are_left_alone() {
        let text = "---\ntitle: x\n---\n| a\u{00A0}b |\n|---|:-:|\n```\nx\u{200B}y\n```\n$$\na\u{00A0}b\n$$\nok\u{200B} `a\u{200B}b` and $x\u{00A0}y$ done";
        assert_eq!(c(text), "---\ntitle: x\n---\n| a b |\n|---|:-:|\n```\nx\u{200B}y\n```\n$$\na\u{00A0}b\n$$\nok `a\u{200B}b` and $x\u{00A0}y$ done");
    }

    #[test]
    fn the_dialog_counts() {
        let (_, n) = clean("a\u{200B}b c\u{00A0}d x，y d\u{0430}ta", KEEP);
        assert_eq!(n.describe(), "1 invisible character removed, 1 unusual space made a space, 1 look-alike punctuation mark made the keyboard's, 1 look-alike letter or digit made a Latin one");
        assert_eq!(Counts::default().describe(), "");
    }

    /// The text with `edits` made (from the end, so earlier places stay right), as the editor makes them.
    fn apply(text: &str, edits: &[Edit]) -> String {
        let mut u: Vec<u16> = text.encode_utf16().collect();
        for e in edits.iter().rev() {
            u.splice(e.from as usize..e.to as usize, e.insert.encode_utf16());
        }
        String::from_utf16(&u).unwrap()
    }

    #[test]
    fn the_edits_make_what_clean_makes() {
        let texts = [
            "a\u{200B}b c\u{00A0}d x，y d\u{0430}ta",
            "The model — small and fast — wins.\nIt works—mostly. 𝐁𝐨𝐥𝐝 😀 and a\u{00A0}— b",
            "A list:\n— first\n| a | — |\none – two, pp. 3–5 “quoted” … → done",
            "---\ntitle: x\n---\n```\nx\u{200B}y\n```\nok\u{200B} `a\u{200B}b` and $x\u{00A0}y$ done\u{00A0} \u{00A0}end",
        ];
        for t in texts {
            for o in [KEEP, ALL] {
                assert_eq!(apply(t, &edits(t, o)), clean(t, o).0, "{t:?} {o:?}");
            }
        }
    }

    #[test]
    fn one_character_s_replacement() {
        // an Arabic comma after Latin text, a zero-width space, a Cyrillic а in "dаta" (UTF-16 places)
        let t = "x\u{060C} y\u{200B}z d\u{0430}ta";
        assert_eq!(edit_at(t, 1, KEEP), Some(Edit { from: 1, to: 2, insert: ",".into(), change: Change::Punctuation }));
        assert_eq!(edit_at(t, 4, KEEP), Some(Edit { from: 4, to: 5, insert: String::new(), change: Change::Invisible }));
        assert_eq!(edit_at(t, 8, KEEP).map(|e| e.insert), Some("a".into()));
        assert_eq!(edit_at(t, 0, KEEP), None, "a plain letter has none");
        // a styled letter is two UTF-16 units; an em dash is "--" in its own place, and only with typography on
        let t = "𝐁 and a — b";
        assert_eq!(edit_at(t, 0, KEEP), Some(Edit { from: 0, to: 2, insert: "B".into(), change: Change::Letter }));
        assert_eq!(edit_at(t, 9, KEEP), None);
        assert_eq!(edit_at(t, 9, ALL), Some(Edit { from: 9, to: 10, insert: "--".into(), change: Change::Typographic }));
        // in code it is left alone
        assert_eq!(edit_at("`a\u{200B}b`", 2, KEEP), None);
    }
}
