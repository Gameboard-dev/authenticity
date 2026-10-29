//! Characters worth seeing, found by fixed rules: invisible ones (zero-width spaces and joiners, the byte-order mark,
//! soft hyphens, direction marks, Unicode tag characters), unusual spaces (no-break, narrow, thin, hair …), dashes
//! (the em dash above all, which language models use far more than people typing do), punctuation that looks like a
//! comma, full stop, colon, semicolon, apostrophe or hyphen but is another character (a fullwidth or ideographic comma,
//! a Greek question mark for a semicolon, a modifier letter for an apostrophe), letters from another script inside a
//! Latin word (a Cyrillic "а" in "dаta") and styled Unicode letters (mathematical bold, fullwidth), and typographic
//! characters (curly quotes, the ellipsis character, bullets, arrows). Invisible characters, unusual spaces and
//! look-alikes seldom come from typing: they arrive with text pasted from a model, a PDF or a web page. Typographic
//! characters are also produced by Word, so they are a milder signal and can be turned off.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hidden,
    Space,
    Dash,
    Punctuation,
    LookAlike,
    Typographic,
}

impl Kind {
    pub fn class(self) -> &'static str {
        match self {
            Kind::Hidden => "auth-char-hidden",
            Kind::Space => "auth-char-space",
            Kind::Dash => "auth-char-dash",
            Kind::Punctuation => "auth-char-punct",
            Kind::Typographic => "auth-char-typo",
            Kind::LookAlike => "auth-char-lookalike",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Hidden => "Invisible characters",
            Kind::Space => "Unusual spaces",
            Kind::Dash => "Dashes",
            Kind::Punctuation => "Look-alike punctuation",
            Kind::Typographic => "Typographic characters",
            Kind::LookAlike => "Look-alike letters",
        }
    }
}

/// One character found: its UTF-16 range in the note, its kind, a short label to draw and its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub from: u32,
    pub to: u32,
    pub kind: Kind,
    pub code: u32,
    pub short: &'static str,
    pub name: &'static str,
}

fn classify(c: char) -> Option<(Kind, &'static str, &'static str)> {
    use Kind::*;
    Some(match c {
        '\u{200B}' => (Hidden, "ZWSP", "zero-width space"),
        '\u{200C}' => (Hidden, "ZWNJ", "zero-width non-joiner"),
        '\u{200D}' => (Hidden, "ZWJ", "zero-width joiner"),
        '\u{2060}' => (Hidden, "WJ", "word joiner"),
        '\u{FEFF}' => (Hidden, "BOM", "byte-order mark (zero-width no-break space)"),
        '\u{00AD}' => (Hidden, "SHY", "soft hyphen"),
        '\u{180E}' => (Hidden, "MVS", "Mongolian vowel separator"),
        '\u{034F}' => (Hidden, "CGJ", "combining grapheme joiner"),
        '\u{2061}'..='\u{2064}' => (Hidden, "INV", "invisible mathematical operator"),
        '\u{200E}' => (Hidden, "LRM", "left-to-right mark"),
        '\u{200F}' => (Hidden, "RLM", "right-to-left mark"),
        '\u{202A}'..='\u{202E}' => (Hidden, "BIDI", "bidirectional embedding or override"),
        '\u{2066}'..='\u{2069}' => (Hidden, "BIDI", "bidirectional isolate"),
        '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}' => (Hidden, "FILL", "Hangul filler"),
        '\u{E0000}'..='\u{E007F}' => (Hidden, "TAG", "Unicode tag character (can carry hidden text)"),
        '\u{FE00}'..='\u{FE0F}' => (Hidden, "VS", "variation selector"),
        '\u{00A0}' => (Space, "NBSP", "no-break space"),
        '\u{202F}' => (Space, "NNBSP", "narrow no-break space"),
        '\u{2007}' => (Space, "FSP", "figure space"),
        '\u{2008}' => (Space, "PSP", "punctuation space"),
        '\u{2009}' => (Space, "THSP", "thin space"),
        '\u{200A}' => (Space, "HSP", "hair space"),
        '\u{2000}'..='\u{2006}' => (Space, "SP", "typographic space (quad or em/en space)"),
        '\u{205F}' => (Space, "MMSP", "medium mathematical space"),
        '\u{3000}' => (Space, "IDSP", "ideographic space"),
        '\u{2014}' => (Dash, "—", "em dash"),
        '\u{2013}' => (Dash, "–", "en dash"),
        '\u{2015}' => (Dash, "―", "horizontal bar"),
        '\u{2012}' => (Dash, "‒", "figure dash"),
        '\u{2E3A}' | '\u{2E3B}' => (Dash, "⸺", "two- or three-em dash"),
        '\u{FE58}' | '\u{FE31}' | '\u{FE32}' => (Dash, "﹘", "small or vertical em dash"),
        '\u{2010}' => (Punctuation, "‐", "Unicode hyphen (looks like the keyboard's -)"),
        '\u{2011}' => (Punctuation, "‑", "non-breaking hyphen"),
        '\u{2212}' => (Punctuation, "−", "minus sign"),
        '\u{FE63}' | '\u{FF0D}' => (Punctuation, "－", "small or fullwidth hyphen-minus"),
        '\u{FF0C}' => (Punctuation, "，", "fullwidth comma"),
        '\u{3001}' => (Punctuation, "、", "ideographic comma"),
        '\u{060C}' => (Punctuation, "،", "Arabic comma"),
        '\u{FE50}' | '\u{FE51}' => (Punctuation, "﹐", "small comma"),
        '\u{201A}' => (Punctuation, "‚", "low single quotation mark (looks like a comma)"),
        '\u{FF0E}' | '\u{3002}' | '\u{FE52}' => (Punctuation, "．", "fullwidth, ideographic or small full stop"),
        '\u{2024}' => (Punctuation, "․", "one-dot leader (looks like a full stop)"),
        '\u{037E}' => (Punctuation, ";", "Greek question mark (looks like a semicolon)"),
        '\u{FF1B}' | '\u{FE54}' => (Punctuation, "；", "fullwidth or small semicolon"),
        '\u{FF1A}' | '\u{FE55}' | '\u{2236}' => (Punctuation, "：", "fullwidth or small colon, or ratio sign"),
        '\u{FF01}' | '\u{FF1F}' => (Punctuation, "？", "fullwidth exclamation or question mark"),
        '\u{02BC}' | '\u{02BB}' | '\u{02B9}' => (Punctuation, "ʼ", "modifier letter used as an apostrophe"),
        '\u{2032}' | '\u{2033}' => (Punctuation, "′", "prime (looks like an apostrophe or quote)"),
        '\u{00B4}' => (Punctuation, "´", "acute accent used as an apostrophe"),
        '\u{FF07}' | '\u{FF02}' => (Punctuation, "＇", "fullwidth apostrophe or quote"),
        '\u{1D400}'..='\u{1D7FF}' => (LookAlike, "𝐀", "styled mathematical letter or digit (bold, italic …)"),
        '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}' | '\u{FF10}'..='\u{FF19}' => (LookAlike, "Ａ", "fullwidth letter or digit"),
        '\u{2018}' | '\u{2019}' => (Typographic, "’", "curly single quote or apostrophe"),
        '\u{201C}' | '\u{201D}' => (Typographic, "”", "curly double quote"),
        '\u{2026}' => (Typographic, "…", "ellipsis character"),
        '\u{2022}' | '\u{2023}' | '\u{25E6}' | '\u{2043}' => (Typographic, "•", "bullet character"),
        '\u{2192}' | '\u{21D2}' | '\u{2190}' | '\u{2194}' => (Typographic, "→", "arrow"),
        _ => return None,
    })
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c)
}

fn look_alike(c: char) -> Option<&'static str> {
    match c {
        '\u{0400}'..='\u{04FF}' => Some("Cyrillic letter in a Latin word"),
        '\u{0370}'..='\u{03FF}' => Some("Greek letter in a Latin word"),
        _ => None,
    }
}

/// Every such character in `text`; typographic ones only if `typographic`.
pub fn scan(text: &str, typographic: bool) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut pos = 0u32;
    let mut word: Vec<(char, u32)> = Vec::new(); // the current run of letters, for look-alikes
    let flush = |word: &mut Vec<(char, u32)>, hits: &mut Vec<Hit>| {
        if word.iter().any(|&(c, _)| is_latin(c)) {
            for &(c, at) in word.iter() {
                if let Some(name) = look_alike(c) {
                    hits.push(Hit { from: at, to: at + c.len_utf16() as u32, kind: Kind::LookAlike, code: c as u32, short: "", name });
                }
            }
        }
        word.clear();
    };
    for c in text.chars() {
        let w = c.len_utf16() as u32;
        if c.is_alphabetic() {
            word.push((c, pos));
        } else {
            flush(&mut word, &mut hits);
        }
        if let Some((kind, short, name)) = classify(c) {
            if kind != Kind::Typographic || typographic {
                hits.push(Hit { from: pos, to: pos + w, kind, code: c as u32, short, name });
            }
        }
        pos += w;
    }
    flush(&mut word, &mut hits);
    hits.sort_by_key(|h| h.from);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_kind() {
        let text = "a\u{200B}b c\u{00A0}d — “q” d\u{0430}ta αβ";
        let kinds: Vec<(Kind, &str)> = scan(text, true).iter().map(|h| (h.kind, h.short)).collect();
        assert_eq!(kinds, vec![(Kind::Hidden, "ZWSP"), (Kind::Space, "NBSP"), (Kind::Dash, "—"), (Kind::Typographic, "”"), (Kind::Typographic, "”"), (Kind::LookAlike, "")]);
        assert_eq!(scan(text, false).len(), 4); // Greek on its own (αβ) is not flagged; the em dash always is
    }

    #[test]
    fn look_alike_punctuation_and_styled_letters() {
        let kinds: Vec<(Kind, &str)> = scan("a，b c\u{037E}d it\u{02BC}s \u{1D401}old", false).iter().map(|h| (h.kind, h.short)).collect();
        assert_eq!(kinds, vec![(Kind::Punctuation, "，"), (Kind::Punctuation, ";"), (Kind::Punctuation, "ʼ"), (Kind::LookAlike, "𝐀")]);
    }

    #[test]
    fn utf16_positions() {
        let hits = scan("😀\u{200B}", false);
        assert_eq!((hits[0].from, hits[0].to), (2, 3));
    }
}

#[cfg(test)]
mod real {
    /// `AUTH_NOTE=<path> cargo test -p authenticity-core chars::real -- --ignored --nocapture`: a note's characters, counted.
    #[test]
    #[ignore]
    fn count_characters() {
        let path = std::env::var("AUTH_NOTE").expect("AUTH_NOTE");
        let mut counts = std::collections::BTreeMap::new();
        for h in super::scan(&std::fs::read_to_string(path).unwrap(), true) {
            *counts.entry((h.kind.label(), h.name)).or_insert(0) += 1;
        }
        for ((kind, name), n) in counts {
            println!("{kind}: {name} × {n}");
        }
    }
}
