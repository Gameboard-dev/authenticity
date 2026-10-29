//! Finding the detector's sentences in the note. GPTZero returns each sentence as text, not as an offset, and the
//! note may have been edited since the scan, so each sentence is looked for in the current prose (prose.rs): exactly
//! with white space collapsed, then ignoring case and quote style, then by its first and last four words (a long
//! sentence with a small edit inside). The search runs forward from the previous sentence and falls back to the whole
//! note. A sentence that is no longer there is reported as `None`: edited since the scan.

use crate::prose::Prose;

/// A text with white space collapsed and quotes made straight, lower-cased, and the index in the original of each
/// character kept.
struct Norm {
    chars: Vec<char>,
    at: Vec<usize>,
}

fn fold(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => '"',
        '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
        c => c.to_lowercase().next().unwrap_or(c),
    }
}

fn normalize(cs: &[char]) -> Norm {
    let mut n = Norm { chars: Vec::new(), at: Vec::new() };
    let mut space = true;
    for (i, &c) in cs.iter().enumerate() {
        if c.is_whitespace() {
            if !space {
                n.chars.push(' ');
                n.at.push(i);
            }
            space = true;
        } else {
            n.chars.push(fold(c));
            n.at.push(i);
            space = false;
        }
    }
    if n.chars.last() == Some(&' ') {
        n.chars.pop();
        n.at.pop();
    }
    n
}

fn find(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() || from > hay.len() - needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// [start, end) in `hay` of `needle`, from `from`: whole, or by its first and last four words within a span not much
/// longer than the sentence.
fn locate(hay: &[char], needle: &[char], from: usize) -> Option<(usize, usize)> {
    if let Some(i) = find(hay, needle, from) {
        return Some((i, i + needle.len()));
    }
    let words: Vec<&[char]> = needle.split(|&c| c == ' ').filter(|w| !w.is_empty()).collect();
    if words.len() < 10 {
        return None;
    }
    let head: Vec<char> = words[..4].join(&' ');
    let tail: Vec<char> = words[words.len() - 4..].join(&' ');
    let a = find(hay, &head, from)?;
    let b = find(hay, &tail, a + head.len())?;
    let end = b + tail.len();
    (end - a <= needle.len() + needle.len() / 4 + 20).then_some((a, end))
}

/// For each sentence, the [from, to) UTF-16 range in the note where it is now, or `None`.
pub fn locate_all(prose: &Prose, sentences: &[&str]) -> Vec<Option<(u32, u32)>> {
    let hay = normalize(&prose.chars);
    let mut cursor = 0;
    sentences
        .iter()
        .map(|s| {
            let cs: Vec<char> = s.chars().collect();
            let needle = normalize(&cs);
            if needle.chars.is_empty() {
                return None;
            }
            let (a, e) = locate(&hay.chars, &needle.chars, cursor).or_else(|| locate(&hay.chars, &needle.chars, 0))?;
            cursor = e;
            let (first, last) = (hay.at[a], hay.at[e - 1]);
            Some((prose.src[first].0, prose.src[last].1))
        })
        .collect()
}

/// Where `needle` is in `hay` (both plain text, such as a rendered block's), as a [from, to) UTF-16 range in `hay`.
pub fn find_utf16(hay: &str, needle: &str) -> Option<(u32, u32)> {
    let cs: Vec<char> = hay.chars().collect();
    let h = normalize(&cs);
    let n = normalize(&needle.chars().collect::<Vec<char>>());
    let (a, e) = locate(&h.chars, &n.chars, 0)?;
    let o16 = |i: usize| cs[..i].iter().map(|c| c.len_utf16() as u32).sum::<u32>();
    let (first, last) = (h.at[a], h.at[e - 1]);
    Some((o16(first), o16(last + 1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prose::extract;

    #[test]
    fn sentences_are_found_in_the_source() {
        let note = "# Title\n\nThe model is **small** [@a, p. 3]. It runs “fast”.\n\nIt runs fast.";
        let p = extract(note);
        let got = locate_all(&p, &["The model is small.", "It runs \"fast\".", "It runs fast.", "Gone sentence."]);
        let slice = |r: Option<(u32, u32)>| r.map(|(a, b)| note.encode_utf16().skip(a as usize).take((b - a) as usize).collect::<Vec<u16>>()).map(|u| String::from_utf16(&u).unwrap());
        assert_eq!(slice(got[0]).as_deref(), Some("The model is **small** [@a, p. 3]."));
        assert_eq!(slice(got[1]).as_deref(), Some("It runs “fast”."));
        assert_eq!(slice(got[2]).as_deref(), Some("It runs fast."));
        assert_eq!(got[3], None);
    }

    #[test]
    fn found_in_rendered_text_as_utf16() {
        let hay = "😀 Intro. The model is  small. Outro.";
        assert_eq!(find_utf16(hay, "the model is small."), Some((10, 30)));
        assert_eq!(find_utf16(hay, "Missing."), None);
    }

    #[test]
    fn a_small_edit_inside_a_long_sentence_still_matches() {
        let note = "One two three four five six seven eight nine ten eleven twelve thirteen.";
        let p = extract(note);
        let got = locate_all(&p, &["One two three four five SIX-ish seven eight nine ten eleven twelve thirteen."]);
        assert_eq!(got[0], Some((0, note.len() as u32)));
    }
}
