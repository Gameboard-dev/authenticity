//! The text a check reads: what is left out of it, its words with their places, and its sentences.
//!
//! What is left out is blanked in place (`blank`): the text keeps its length in bytes, so every word found in it has
//! its place in the text as given (the editor's highlight, the source's span). A blanked stretch is a break (NUL bytes:
//! no run of words spans it) or a gap (spaces: removed without a break, as a TeX comment at the end of a line). The
//! rules are applied in order to a copy of the text instead of rewriting it.
//!
//! Words (`tokens`) are compared normalised: case, punctuation and white space do not count; a hyphen splits a word
//! ("state-of-the-art" reads as four words, as "state of the art" does), a word broken across a line by a hyphen
//! ("exam-\nple") reads whole, apostrophes inside a word are dropped ("don't" and "don’t" read "dont"), and letters
//! are compared without accents and compatibility forms (a PDF's "ﬁ" ligature reads "fi", "é" reads "e").

use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// What a check leaves out of the text it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Masks {
    /// Obsidian's markdown: the properties, code blocks, `%% comments %%`, maths, inline review comments `^[ ... ]`,
    /// callouts and table rows, links and citations in brackets (`[[...]]`, `[@key]`), and inline code.
    pub markdown: bool,
    /// LaTeX: `%` comments and commands with their first argument (`\cite{...}`). Off for plain text, where a `%`
    /// is a percent sign.
    pub latex: bool,
    /// Citations in brackets with a year: "(Smith et al., 2020)".
    pub citations: bool,
    /// Anything inside quotation marks (`"..."`, `“...”`, `‘...’`, on one line): a quotation with its citation is
    /// meant to match its source.
    pub quotes: bool,
}

impl Masks {
    /// Everything left out, as a draft is read.
    pub const ALL: Masks = Masks { markdown: true, latex: true, citations: true, quotes: true };
    /// Nothing left out: a source's own text.
    pub const NONE: Masks = Masks { markdown: false, latex: false, citations: false, quotes: false };
}

/// A break: no run of words spans it.
const BREAK: u8 = 0;
/// A gap: removed, without a break.
const GAP: u8 = b' ';

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid pattern")
}

static FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)```.*?```"));
static COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)%%.*?%%"));
static DISPLAY: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)\$\$.*?\$\$"));
/// `(?<![\\$])\$[^$\n]+\$` without its look-behind, which is checked in code (the regex crate has none).
static INLINE_MATH: LazyLock<Regex> = LazyLock::new(|| re(r"\$[^$\n]+\$"));
static LINKS: LazyLock<Regex> = LazyLock::new(|| re(r"\[\[[^\]]*\]\]|\[@[^\]]*\]|`[^`]*`"));
/// `(?<!\\)%.*`, the look-behind checked in code.
static TEX_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"%.*"));
static TEX_CMD: LazyLock<Regex> = LazyLock::new(|| re(r"\\[A-Za-z]+\*?(\[[^\]]*\])?(\{[^}]*\})?"));
static HIGHLIGHT: LazyLock<Regex> = LazyLock::new(|| re(r"=="));
/// Citations in brackets with a year.
static YEAR_PAREN: LazyLock<Regex> = LazyLock::new(|| re(r"\([^()]*\b(?:1[89]|20)\d\d[a-z]?\b[^()]*\)"));
static QUOTED: LazyLock<Regex> = LazyLock::new(|| re("\"[^\"\n]*\"|“[^”\n]*”|‘[^’\n]*’"));

/// The text with what `m` leaves out blanked (see the module's notes); the same length in bytes.
pub fn blank(text: &str, m: Masks) -> String {
    // a carriage return reads as a space (the rules were written for `\n` line ends)
    let mut w = if text.contains('\r') { text.replace('\r', " ") } else { text.to_string() };
    if m.markdown {
        front_matter(&mut w);
        fill_all(&mut w, &FENCE, BREAK);
        fill_all(&mut w, &COMMENT, BREAK);
        fill_all(&mut w, &DISPLAY, BREAK);
        inline_comments(&mut w);
        box_lines(&mut w);
        fill_where(&mut w, &INLINE_MATH, |t, s| !matches!(before(t, s), Some('\\' | '$')), BREAK);
        fill_all(&mut w, &LINKS, BREAK);
    }
    if m.latex {
        fill_where(&mut w, &TEX_COMMENT, |t, s| before(t, s) != Some('\\'), GAP);
        fill_all(&mut w, &TEX_CMD, BREAK);
    }
    if m.markdown {
        fill_all(&mut w, &HIGHLIGHT, GAP);
    }
    if m.citations {
        fill_all(&mut w, &YEAR_PAREN, BREAK);
    }
    if m.quotes {
        fill_all(&mut w, &QUOTED, BREAK);
    }
    w
}

/// Each range filled with `b` (an ASCII byte over whole characters, so the text stays UTF-8).
fn apply(w: &mut String, ranges: &[Range<usize>], b: u8) {
    if ranges.is_empty() {
        return;
    }
    let mut bytes = std::mem::take(w).into_bytes();
    for r in ranges {
        bytes[r.clone()].fill(b);
    }
    *w = String::from_utf8(bytes).expect("ASCII over whole characters keeps the text UTF-8");
}

fn fill_all(w: &mut String, rx: &Regex, b: u8) {
    let ranges: Vec<Range<usize>> = rx.find_iter(w).map(|m| m.range()).collect();
    apply(w, &ranges, b);
}

/// The matches `ok` accepts (given the text and the match's start); a refused match is tried again one character
/// later, as a backtracking engine with the look-around would.
fn fill_where(w: &mut String, rx: &Regex, ok: impl Fn(&str, usize) -> bool, b: u8) {
    let (mut ranges, mut at) = (Vec::new(), 0);
    while at <= w.len() {
        let Some(m) = rx.find_at(w, at) else { break };
        let next = |i: usize| i + w[i..].chars().next().map_or(1, char::len_utf8);
        if ok(w, m.start()) {
            ranges.push(m.range());
            at = if m.end() > m.start() { m.end() } else { next(m.end()) };
        } else {
            at = next(m.start());
        }
    }
    apply(w, &ranges, b);
}

/// The character before byte `i`, if any.
fn before(text: &str, i: usize) -> Option<char> {
    text[..i].chars().next_back()
}

/// The properties: from a first line `---` to the next line that starts with `---`.
fn front_matter(w: &mut String) {
    let first_line_end = w.find('\n');
    let Some(nl) = first_line_end.filter(|&nl| w.starts_with("---") && w[3..nl].trim().is_empty()) else { return };
    if let Some(end) = w[nl..].find("\n---").map(|e| e + nl) {
        let whole = 0..end + 4;
        apply(w, std::slice::from_ref(&whole), BREAK);
    }
}

/// Inline review comments `^[ ... ]`, brackets nested, to the end of the text when one is not closed.
fn inline_comments(w: &mut String) {
    let (mut ranges, mut i) = (Vec::new(), 0);
    let b = w.as_bytes();
    while let Some(j) = w[i..].find("^[").map(|j| j + i) {
        let (mut depth, mut k) = (0i32, j + 1);
        while k < b.len() {
            match b[k] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            k += 1;
        }
        ranges.push(j..(k + 1).min(b.len()));
        i = k + 1;
        if i >= w.len() {
            break;
        }
    }
    apply(w, &ranges, BREAK);
}

/// Callouts and table rows: lines that start with `>` or `|`, removed without a break.
fn box_lines(w: &mut String) {
    let mut ranges = Vec::new();
    let mut start = 0;
    for line in w.split('\n') {
        if line.trim_start().starts_with(['>', '|']) {
            ranges.push(start..start + line.len());
        }
        start += line.len() + 1;
    }
    apply(w, &ranges, GAP);
}

// ------------------------------------------------------------------------ words

/// One word of a text: its place (bytes, `start..end`), the stretch of the text between breaks it is in (`seg`), its
/// normalised form's hash, and whether it counts as a content word (not a function word, not part of a fixed term).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub seg: u32,
    pub hash: u64,
    pub content: bool,
}

/// Function words: a run made mostly of them with a term or two is not copied wording (a fixed list).
pub const FUNCTION_WORDS: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "at", "to", "for", "from", "by", "with", "as", "and", "or", "but", "nor", "so", "that", "this", "these",
    "those", "which", "who", "whom", "whose", "is", "are", "was", "were", "be", "been", "being", "has", "have", "had", "do", "does", "did", "it",
    "its", "their", "there", "here", "than", "then", "not", "no", "into", "onto", "over", "under", "across", "between", "within", "without",
    "about", "also", "each", "every", "all", "any", "some", "such", "one", "two", "more", "most", "less", "only", "very", "can", "could", "may",
    "might", "will", "would", "should", "must",
];

static FUNCTION: LazyLock<HashSet<u64>> = LazyLock::new(|| FUNCTION_WORDS.iter().map(|w| word_hash(w)).collect());

/// FNV-1a, 64 bits, of a normalised word (<http://www.isthe.com/chongo/tech/comp/fnv/>): fixed across runs and
/// builds, so the index can be kept on disk.
pub fn word_hash(w: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in w.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’' | 'ʼ')
}

/// A character added to a word's normalised form: lower case, without accents, compatibility forms decomposed.
fn push_norm(out: &mut String, c: char) {
    if c.is_ascii() {
        out.push(c.to_ascii_lowercase());
        return;
    }
    for d in std::iter::once(c).nfkd() {
        if !is_combining_mark(d) {
            out.extend(d.to_lowercase());
        }
    }
}

/// The words of a blanked text (`blank`), with their places.
pub fn tokens(w: &str) -> Vec<Token> {
    let (mut out, mut seg, mut broke, mut i) = (Vec::new(), 0u32, false, 0usize);
    let mut norm = String::new();
    let at = |i: usize| w[i..].chars().next();
    while let Some(c) = at(i) {
        if c == '\0' {
            broke = true;
            i += 1;
            continue;
        }
        if !c.is_alphanumeric() {
            i += c.len_utf8();
            continue;
        }
        if broke && !out.is_empty() {
            seg += 1;
        }
        broke = false;
        norm.clear();
        let start = i;
        let mut end = i;
        while let Some(d) = at(i) {
            if d.is_alphanumeric() {
                push_norm(&mut norm, d);
                i += d.len_utf8();
                end = i;
                continue;
            }
            // an apostrophe or a soft hyphen inside a word: dropped, the word goes on
            if is_apostrophe(d) || d == '\u{AD}' {
                if at(i + d.len_utf8()).is_some_and(char::is_alphanumeric) {
                    i += d.len_utf8();
                    continue;
                }
                break;
            }
            // a word broken across a line: a lower-case letter, "-", white space holding a line break, a lower-case letter
            if d == '-' && norm.chars().next_back().is_some_and(char::is_lowercase) {
                let rest = &w[i + 1..];
                let space = rest.len() - rest.trim_start_matches([' ', '\t', '\n']).len();
                if rest[..space].contains('\n') && rest[space..].chars().next().is_some_and(char::is_lowercase) {
                    i += 1 + space;
                    continue;
                }
            }
            break;
        }
        let hash = word_hash(&norm);
        out.push(Token { start, end, seg, hash, content: !FUNCTION.contains(&hash) });
    }
    out
}

/// The normalised words of a phrase (a fixed term), as hashes.
pub fn phrase(text: &str) -> Vec<u64> {
    tokens(text).iter().map(|t| t.hash).collect()
}

/// The words covered by an occurrence of a fixed term (`terms`, each a phrase's hashes) made not content words: a run
/// made only of such terms and function words is the field's vocabulary, not copied wording.
pub fn mark_terms(tokens: &mut [Token], terms: &[Vec<u64>]) {
    for term in terms.iter().filter(|t| !t.is_empty()) {
        let n = term.len();
        for i in 0..(tokens.len() + 1).saturating_sub(n) {
            let w = &tokens[i..i + n];
            if w[0].seg == w[n - 1].seg && w.iter().zip(term).all(|(t, h)| t.hash == *h) {
                for t in &mut tokens[i..i + n] {
                    t.content = false;
                }
            }
        }
    }
}

// ------------------------------------------------------------------------ sentences

/// Words before a full stop that do not end a sentence (lower case, without the final stop).
const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "al", "fig", "figs", "eq", "eqs", "no", "nos", "vol", "vols", "pp", "p", "cf", "vs", "etc", "approx", "dr", "prof", "mr", "mrs",
    "ms", "sec", "secs", "ch", "chap", "ref", "refs", "resp", "ed", "eds", "inc", "ltd", "co", "jr", "sr", "st", "tab", "viz", "ca", "cit", "op",
];

/// The sentences of a blanked text, as byte ranges trimmed of white space and breaks: split after `.`, `!` or `?` (and
/// any closing quotation mark or bracket) followed by white space and a capital, a digit or an opening mark (not
/// after an abbreviation or an initial), at a blank line, and before a line that starts a heading, a list item, a
/// callout or a table row.
pub fn sentences(w: &str) -> Vec<Range<usize>> {
    let b = w.as_bytes();
    let mut cuts = vec![0usize];
    let (mut i, mut line_start) = (0, 0);
    while i < b.len() {
        match b[i] {
            b'\n' => {
                // a heading is a sentence of its own
                if w[line_start..i].trim_start().starts_with('#') {
                    cuts.push(i + 1);
                }
                line_start = i + 1;
                let rest = &w[i + 1..];
                let line = rest.trim_start_matches([' ', '\t', '\0']);
                let blank_line = line.starts_with('\n') || line.is_empty();
                let starts_block = line.starts_with(['#', '>', '|', '-', '*', '+']) || {
                    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                    digits > 0 && line[digits..].starts_with(['.', ')'])
                };
                if blank_line || starts_block {
                    cuts.push(i + 1);
                }
            }
            b'.' | b'!' | b'?' => {
                let mut j = i + 1;
                while j < b.len() && matches!(w[j..].chars().next(), Some(')' | ']' | '"' | '\'' | '”' | '’')) {
                    j += w[j..].chars().next().map_or(1, char::len_utf8);
                }
                let rest = &w[j..];
                let space = rest.len() - rest.trim_start_matches([' ', '\t', '\0']).len();
                let next = rest[space..].chars().next();
                // a blanked span (a citation or link such as `@key` that starts the next sentence) opens one too
                let blanked = rest[..space].contains('\0');
                let opens = blanked || next.is_some_and(|c| c.is_uppercase() || c.is_ascii_digit() || matches!(c, '"' | '“' | '‘' | '(' | '[' | '@'));
                if space > 0 && opens && (b[i] != b'.' || !abbreviation(&w[..i])) {
                    cuts.push(j);
                }
                i = j.max(i + 1);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    cuts.push(b.len());
    cuts.dedup();
    let trim = |c: char| c.is_whitespace() || c == '\0';
    cuts.windows(2)
        .filter_map(|p| {
            let s = &w[p[0]..p[1]];
            let lead = s.len() - s.trim_start_matches(trim).len();
            let body = s.trim_matches(trim);
            (!body.is_empty()).then(|| p[0] + lead..p[0] + lead + body.len())
        })
        .collect()
}

/// Whether the text before a full stop ends in an abbreviation or an initial ("J. Smith").
fn abbreviation(before: &str) -> bool {
    let word: String = before.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '.').collect::<Vec<_>>().into_iter().rev().collect();
    let mut cs = word.chars();
    if let (Some(c), None) = (cs.next(), cs.next()) {
        return c.is_uppercase();
    }
    ABBREVIATIONS.contains(&word.to_lowercase().as_str())
}

/// UTF-16 offsets (what an editor in a browser counts) of byte offsets in `text`, each on a character boundary; `at`
/// in any order, converted in one pass.
pub fn utf16_offsets(text: &str, at: &[usize]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..at.len()).collect();
    order.sort_by_key(|&k| at[k]);
    let mut out = vec![0; at.len()];
    let (mut byte, mut units) = (0usize, 0usize);
    let mut chars = text.chars();
    for k in order {
        while byte < at[k] {
            let Some(c) = chars.next() else { break };
            byte += c.len_utf8();
            units += c.len_utf16();
        }
        out[k] = units;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        tokens(text).iter().map(|t| text[t.start..t.end].to_string()).collect()
    }

    #[test]
    fn words_are_normalised() {
        let h = |s: &str| phrase(s);
        assert_eq!(h("The State-of-the-art"), h("the state of the art"));
        assert_eq!(h("don't"), h("don’t"));
        assert_eq!(h("don't"), h("dont"));
        assert_eq!(h("ﬁnding naïve CAFÉ"), h("finding naive cafe"));
        assert_eq!(h("an exam-\nple of it"), h("an example of it"));
        assert_ne!(h("well-\nKnown"), h("wellknown"), "a capital after the break: a real hyphen");
        assert_eq!(words("a, b; (c) d."), ["a", "b", "c", "d"]);
    }

    #[test]
    fn places_are_the_texts() {
        let text = "Ünïcode — “quoted” text, and more.";
        for t in tokens(text) {
            assert!(text.is_char_boundary(t.start) && text.is_char_boundary(t.end));
        }
        assert_eq!(words(text), ["Ünïcode", "quoted", "text", "and", "more"]);
        let b = blank(text, Masks { quotes: true, ..Masks::NONE });
        assert_eq!(b.len(), text.len());
        assert_eq!(words(&b), ["Ünïcode", "text", "and", "more"]);
    }

    #[test]
    fn breaks_make_segments() {
        let text = "one two \"three four\" five six";
        let t = tokens(&blank(text, Masks { quotes: true, ..Masks::NONE }));
        assert_eq!(t.iter().map(|t| t.seg).collect::<Vec<_>>(), [0, 0, 1, 1]);
    }

    #[test]
    fn markdown_is_left_out() {
        let text = "---\ntitle: x\n---\nKept words here.\n%% a comment %%\n> a callout line\n```\ncode\n```\nSee [[Note|link]] and `code` and $x$ (Smith, 2020) %50 percent\n";
        let b = blank(text, Masks { latex: false, ..Masks::ALL });
        assert_eq!(b.len(), text.len());
        let w = words(&b);
        assert_eq!(w, ["Kept", "words", "here", "See", "and", "and", "50", "percent"]);
        let b = blank(text, Masks::ALL);
        assert_eq!(words(&b), ["Kept", "words", "here", "See", "and", "and"], "with LaTeX on, % starts a comment");
    }

    #[test]
    fn sentences_split_where_they_end() {
        let text = "First one, e.g. with an abbreviation. Second (J. Smith) here! Third?\n\n# Heading\nFourth 3.5 times.";
        let s: Vec<&str> = sentences(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(s, ["First one, e.g. with an abbreviation.", "Second (J. Smith) here!", "Third?", "# Heading", "Fourth 3.5 times."]);
        // a sentence that starts with a narrative citation, or with a link left out of the check, is one of its own
        let text = "The first ends here. @smith2020 found the second. [[note|Term]] is the third.";
        let b = blank(text, Masks::ALL);
        let s: Vec<&str> = sentences(&b).into_iter().map(|r| &text[r]).collect();
        assert_eq!(s, ["The first ends here.", "@smith2020 found the second.", "is the third."]);
    }

    #[test]
    fn utf16() {
        let text = "a😀b é";
        let at = [0, 1, 5, 6, text.len()];
        assert_eq!(utf16_offsets(text, &at), [0, 1, 3, 4, 6]);
        assert_eq!(utf16_offsets(text, &[6, 1]), [4, 1]);
    }

    #[test]
    fn terms_are_not_content() {
        let text = "machine learning systems are used";
        let mut t = tokens(text);
        mark_terms(&mut t, &[phrase("Machine Learning")]);
        assert_eq!(t.iter().map(|t| t.content).collect::<Vec<_>>(), [false, false, true, false, true]);
    }
}
