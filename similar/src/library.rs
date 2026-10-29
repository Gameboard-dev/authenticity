//! Library notes as the index holds them. Each note of the library folder is a source (its key: the note's path in the
//! vault); its passages are its blocks of text between blank lines, without its properties (front matter). A passage is
//! named in the index by a number made from the note's key and the passage's place, so whoever reads the notes (the
//! plugin through Obsidian's vault, the native programs from disk) can find a passage's text again: read the note, cut
//! it the same way, take the passage. Runs across two passages of a note are still found whole (the index reads
//! consecutive passages as one stretch).
//!
//! A source's signature is (modification time in ms, size in bytes, hash of its text): when the first two differ from
//! the index's, the note is read and its hash compared, so a note that was only touched is not indexed again.

use std::collections::HashMap;
use std::ops::Range;

use crate::index::{keys_of, NewSource};

/// The most passages a note can have (the low bits of a passage's number); passages beyond are not indexed.
pub const MAX_PASSAGES: usize = 1 << 20;
const KEY_BITS: u32 = 43;

/// FNV-1a, 64 bits (<http://www.isthe.com/chongo/tech/comp/fnv/>): fixed across runs, builds and platforms.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A note's key (its vault path, with `/`) as the high bits of its passages' numbers.
pub fn key_hash(key: &str) -> i64 {
    (fnv(key.as_bytes()) & ((1u64 << KEY_BITS) - 1)) as i64
}

/// The number of a note's `i`th passage, or None past `MAX_PASSAGES`.
pub fn passage_ref(key: &str, i: usize) -> Option<i64> {
    (i < MAX_PASSAGES).then(|| (key_hash(key) << 20) | i as i64)
}

/// A passage's number as (its note's key hash, its place).
pub fn ref_parts(r: i64) -> (i64, usize) {
    (r >> 20, (r & (MAX_PASSAGES as i64 - 1)) as usize)
}

/// The byte range after a note's properties (`---` on the first line to the next line of `---`).
fn after_front_matter(text: &str) -> usize {
    let first_end = text.find('\n').unwrap_or(text.len());
    if text[..first_end].trim_end() != "---" {
        return 0;
    }
    let mut at = first_end + 1;
    while at < text.len() {
        let end = text[at..].find('\n').map_or(text.len(), |e| at + e);
        let line = text[at..end].trim_end();
        if line == "---" || line == "..." {
            return (end + 1).min(text.len());
        }
        at = end + 1;
    }
    0 // never closed: not properties
}

/// A note's passages: its blocks of text between blank lines (a line of white space only is blank), as byte ranges,
/// trimmed, without the properties.
pub fn passages(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut at = after_front_matter(text);
    while at <= text.len() {
        let line_end = text[at..].find('\n').map_or(text.len(), |e| at + e);
        let line = &text[at..line_end];
        if line.trim().is_empty() {
            if let Some(s) = start.take() {
                out.push(s..end);
            }
        } else {
            let lead = line.len() - line.trim_start().len();
            start.get_or_insert(at + lead);
            end = at + line.trim_end().len();
        }
        if line_end >= text.len() {
            break;
        }
        at = line_end + 1;
    }
    if let Some(s) = start {
        out.push(s..end);
    }
    out
}

/// The signature of a note: (modification time in ms, size in bytes, hash of its text).
pub fn signature(mtime_ms: u64, size: u64, text: &str) -> [u64; 3] {
    [mtime_ms, size, fnv(text.as_bytes())]
}

/// A note as a source to add to the index: its passages' shingles and content words.
pub fn source(key: &str, text: &str, sig: [u64; 3]) -> NewSource {
    let cut: Vec<(i64, &str)> = passages(text).into_iter().take(MAX_PASSAGES).enumerate().filter_map(|(i, r)| Some((passage_ref(key, i)?, &text[r]))).collect();
    let words = cut.iter().map(|(_, t)| crate::para::word_keys(t)).collect();
    NewSource { key: key.to_string(), sig, passages: cut.into_iter().map(|(r, t)| (r, keys_of(t))).collect(), words }
}

/// The texts of the passages `refs` names, from the notes they belong to: `read(key)` gives a note's text (None: it
/// could not be read, and its passages are left out). Each note is read once.
pub fn passage_texts(refs: &[i64], keys: &HashMap<i64, String>, mut read: impl FnMut(&str) -> Option<String>) -> HashMap<i64, String> {
    let mut by_note: HashMap<i64, Vec<(i64, usize)>> = HashMap::new();
    for r in refs {
        let (k, i) = ref_parts(*r);
        by_note.entry(k).or_default().push((*r, i));
    }
    let mut out = HashMap::new();
    for (k, wanted) in by_note {
        let Some(key) = keys.get(&k) else { continue };
        let Some(text) = read(key) else { continue };
        let cut = passages(&text);
        for (r, i) in wanted {
            if let Some(range) = cut.get(i) {
                out.insert(r, text[range.clone()].to_string());
            }
        }
    }
    out
}

/// What to do to bring the index up to date with the notes now in the library (path, modification time in ms, size).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sync {
    /// Notes the index holds that are gone (or no longer in scope).
    pub remove: Vec<String>,
    /// Notes new to the index, or whose time or size changed: to read (and, if their text changed, index again).
    pub read: Vec<String>,
}

/// The work `Sync` describes, from the index's signatures (`held`) and the notes now there.
pub fn plan_sync<'a>(held: impl Iterator<Item = (&'a str, [u64; 3])>, now: &[(String, u64, u64)]) -> Sync {
    let held: HashMap<&str, [u64; 3]> = held.collect();
    let here: std::collections::HashSet<&str> = now.iter().map(|n| n.0.as_str()).collect();
    let mut remove: Vec<String> = held.keys().filter(|k| !here.contains(**k)).map(|k| k.to_string()).collect();
    remove.sort();
    let read = now.iter().filter(|(p, t, s)| held.get(p.as_str()).is_none_or(|sig| sig[0] != *t || sig[1] != *s)).map(|n| n.0.clone()).collect();
    Sync { remove, read }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Index, Masks, Options, Query};

    #[test]
    fn passages_are_blocks_without_properties() {
        let note = "---\ncitekey: x\ntags: [a]\n---\n# Title\n\nFirst para\ncontinues here.\n  \n\n  Second para.  \r\n\r\nThird";
        let got: Vec<&str> = passages(note).into_iter().map(|r| &note[r]).collect();
        assert_eq!(got, ["# Title", "First para\ncontinues here.", "Second para.", "Third"]);
        // a first line of --- that never closes is text, not properties
        assert_eq!(passages("---\nnot closed").len(), 1);
        assert!(passages("").is_empty() && passages("\n\n \n").is_empty());
    }

    #[test]
    fn passage_numbers_round_trip() {
        let r = passage_ref("Library/a2020/A.md", 17).unwrap();
        assert!(r > 0);
        assert_eq!(ref_parts(r), (key_hash("Library/a2020/A.md"), 17));
        assert_ne!(key_hash("a.md"), key_hash("b.md"));
        assert!(passage_ref("x", MAX_PASSAGES).is_none());
    }

    #[test]
    fn sync_reads_only_what_changed() {
        let held = [("a.md", [1, 10, 5]), ("b.md", [2, 20, 6]), ("gone.md", [3, 30, 7])];
        let now = vec![("a.md".to_string(), 1, 10), ("b.md".to_string(), 9, 20), ("new.md".to_string(), 4, 40)];
        let s = plan_sync(held.iter().map(|(k, s)| (*k, *s)), &now);
        assert_eq!(s.remove, ["gone.md"]);
        assert_eq!(s.read, ["b.md", "new.md"]);
    }

    /// The whole route: notes cut into passages, indexed, a draft checked, the passages read back by number.
    #[test]
    fn notes_indexed_and_read_back() {
        let a = "---\ncitekey: a\n---\nAn unrelated opening paragraph about weather.\n\nEarlier studies reported that the proposed sampling procedure reduces the variance of every estimator considerably.";
        let mut ix = Index::new();
        ix.update(&[], vec![source("Lib/a/A.md", a, signature(1, a.len() as u64, a)), source("Lib/b/B.md", "Nothing alike.", [0; 3])]);
        let keys: HashMap<i64, String> = ix.signatures().map(|(k, _)| (key_hash(k), k.to_string())).collect();
        let q = Query::new("We know the proposed sampling procedure reduces the variance of every estimator.", Masks::ALL, &[]);
        let o = Options::default();
        let plan = ix.plan(&q, &o, &|_| true);
        let mut reads = 0;
        let texts = passage_texts(&plan.refs, &keys, |k| {
            reads += 1;
            (k == "Lib/a/A.md").then(|| a.to_string())
        });
        assert_eq!(reads, 1);
        let found = ix.finish(&q, &o, &plan, &texts);
        assert_eq!(found.findings.len(), 1);
        assert_eq!(found.findings[0].source, "Lib/a/A.md");
        assert_eq!(found.findings[0].source_text, "the proposed sampling procedure reduces the variance of every estimator");
    }
}
