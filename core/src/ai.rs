//! The AI-writing check's results and how they are shown. A note's prose (prose.rs: markup removed, one line per
//! paragraph, each character knowing its place in the note) is cut into paragraphs; each is scored by SlopTotal
//! (sloptotal.rs) and kept by the hash of its text, so a paragraph checked once is not sent again and an edited one
//! loses its colour rather than keeping one that no longer applies. Detector reasons (analyze) and a sentence's own
//! score (the sentence with its neighbours, less reliable) are asked for lazily, the first time they are wanted, and
//! kept the same way.
//!
//! Colours: SlopTotal's quick score runs from 0 (reads as written by a person) to 100 (reads as AI). Up to 35 is its
//! "clean" band: green, stronger the lower the score; above 65 its "ai" band: red, stronger the higher; in between,
//! "mixed": orange. Five strengths each way (classes auth-human-1..5, auth-ai-1..5, auth-mixed; styles.css).

use std::collections::HashMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::prose::Prose;
use crate::sloptotal::{Analysis, Score, AI_MIN, CLEAN_MAX, MIN_CHARS};

/// Kept at most (the oldest go first): paragraph scores, analyses, sentence scores.
pub const KEEP: usize = 5000;

/// A paragraph of a note: its prose and its place (UTF-16 range) in the note.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub text: String,
    pub from: u32,
    pub to: u32,
}

/// The paragraphs of a note's prose (its lines), with their places.
pub fn paragraphs(p: &Prose) -> Vec<Paragraph> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..=p.chars.len() {
        if i == p.chars.len() || p.chars[i] == '\n' {
            if i > start {
                let text: String = p.chars[start..i].iter().collect();
                if !text.trim().is_empty() {
                    out.push(Paragraph { text, from: p.src[start].0, to: p.src[i - 1].1 });
                }
            }
            start = i + 1;
        }
    }
    out
}

/// The key a text is kept by: FNV-1a of its words joined by single spaces, as hex.
pub fn key(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    format!("{:016x}", authenticity_similar::library::fnv(words.join(" ").as_bytes()))
}

/// A paragraph's score as kept: the score, when, and on which device SlopTotal ran; for a short paragraph scored
/// together with its neighbours, which (`together`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Scored {
    pub score: Score,
    pub when: f64,
    pub device: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub together: Option<Together>,
}

/// A short paragraph's group: the keys of the paragraphs whose text was scored (its own among them; the group is
/// valid only while all of them are in the note unchanged), and what the pane says ("scored together with the next
/// paragraph (too short alone)").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Together {
    pub span: Vec<String>,
    pub label: String,
}

/// One quick-score request of a check: the paragraphs whose text is sent (`span`, consecutive, of the paragraphs the
/// check was given), those that take its score (`members`), and the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub span: Range<usize>,
    pub members: Vec<usize>,
    pub text: String,
}

/// The note's paragraphs without its headings, and before each, whether a heading (a section's start) lies between it
/// and the one before. A heading is a paragraph whose line in the note starts with `#`.
pub fn sections(note: &str, paras: Vec<Paragraph>) -> (Vec<Paragraph>, Vec<bool>) {
    let units: Vec<u16> = note.encode_utf16().collect();
    let (mut out, mut boundary) = (Vec::new(), Vec::new());
    let mut heading_since = false;
    for p in paras {
        let start = (p.from as usize).min(units.len());
        let line_start = units[..start].iter().rposition(|&u| u == u16::from(b'\n')).map_or(0, |i| i + 1);
        let line_end = units[start..].iter().position(|&u| u == u16::from(b'\n')).map_or(units.len(), |i| start + i);
        let line = String::from_utf16_lossy(&units[line_start..line_end]);
        if line.trim_start().starts_with('#') {
            heading_since = true;
            continue;
        }
        boundary.push(heading_since && !out.is_empty());
        heading_since = false;
        out.push(p);
    }
    (out, boundary)
}

/// The requests for `paras` (`boundary[i]`: a section starts before paragraph i). A paragraph of `min_words` words or
/// more is scored on its own. A shorter one is scored together with the paragraphs after it in its section until the
/// group has `min_words` (the paragraphs it takes in that are long enough are still scored on their own as well);
/// short of that at the section's end, with those before it in the section too; only when its section has no more,
/// across the section's boundary (the next section first, as a lone short paragraph under a heading goes with the
/// paragraph after the next heading). A group that cannot reach `min_words` (a short note or selection) is scored as it is.
pub fn units(paras: &[Paragraph], boundary: &[bool], min_words: usize) -> Vec<Unit> {
    let words: Vec<usize> = paras.iter().map(|p| p.text.split_whitespace().count()).collect();
    let starts = |i: usize| boundary.get(i).copied().unwrap_or(false);
    let n = paras.len();
    let mut out: Vec<Unit> = Vec::new();
    let mut done = vec![false; n];
    for i in 0..n {
        if done[i] {
            continue;
        }
        if words[i] >= min_words {
            out.push(Unit { span: i..i + 1, members: vec![i], text: paras[i].text.trim().to_string() });
            done[i] = true;
            continue;
        }
        let (mut a, mut b, mut total) = (i, i + 1, words[i]);
        // forward within the section, then back within it, then forward across sections, then back across
        while total < min_words && b < n && !starts(b) {
            total += words[b];
            b += 1;
        }
        while total < min_words && a > 0 && !starts(a) {
            a -= 1;
            total += words[a];
        }
        while total < min_words && b < n {
            total += words[b];
            b += 1;
        }
        while total < min_words && a > 0 {
            a -= 1;
            total += words[a];
        }
        let members: Vec<usize> = (i..b).filter(|&k| words[k] < min_words && !done[k]).collect();
        for &k in &members {
            done[k] = true;
        }
        let text = paras[a..b].iter().map(|p| p.text.trim()).collect::<Vec<_>>().join(" ");
        out.push(Unit { span: a..b, members, text });
    }
    out
}

/// Scores for a plain list of paragraphs (another plugin's, or the MCP server's): grouped as a note's are (`units`,
/// with no sections), each with what the pane says of its group, if it was scored with others.
pub async fn score_list<E: crate::sloptotal::Env>(env: &E, paras: &[String], weights: Option<&crate::sloptotal::Weights>) -> Result<Vec<Option<(Score, Option<String>)>>, String> {
    let ps: Vec<Paragraph> = paras.iter().map(|t| Paragraph { text: t.clone(), from: 0, to: 0 }).collect();
    let us = units(&ps, &[], crate::sloptotal::SHORT_WORDS);
    let texts: Vec<&str> = us.iter().map(|u| u.text.as_str()).collect();
    let scores = crate::sloptotal::paragraphs(env, &texts, weights).await?;
    let mut out = vec![None; paras.len()];
    for (u, s) in us.iter().zip(scores) {
        let Some(s) = s else { continue };
        for &m in &u.members {
            out[m] = Some((s.clone(), together_label(u, m, crate::sloptotal::SHORT_WORDS)));
        }
    }
    Ok(out)
}

/// The place in the note of a group's paragraphs (`paras`: the note's paragraphs now), from the first to the last.
pub fn span_range(paras: &[Paragraph], t: &Together) -> Option<(u32, u32)> {
    let mut at = paras.iter().filter(|p| t.span.contains(&key(&p.text))).map(|p| (p.from, p.to));
    let first = at.next()?;
    Some(at.fold(first, |(a, b), (x, y)| (a.min(x), b.max(y))))
}

/// What the pane says of a member of a unit that holds more than its own paragraph.
pub fn together_label(u: &Unit, member: usize, min_words: usize) -> Option<String> {
    if u.span.len() <= 1 {
        return None;
    }
    let (before, after) = (member - u.span.start, u.span.end - member - 1);
    let with = match (before, after) {
        (0, 1) => "the next paragraph".to_string(),
        (0, k) => format!("the next {k} paragraphs"),
        (1, 0) => "the previous paragraph".to_string(),
        (k, 0) => format!("the previous {k} paragraphs"),
        _ => "the paragraphs around it".to_string(),
    };
    let short = if u.text.split_whitespace().count() < min_words { format!("; still under {min_words} words, less reliable") } else { String::new() };
    Some(format!("scored together with {with} (too short alone){short}"))
}

/// Which colour a score gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Human(u8),
    Mixed,
    Ai(u8),
}

/// The colour of a score, five strengths each way.
pub fn tint(score: f64) -> Tint {
    let level = |d: f64, span: f64| ((d / span * 5.0).ceil() as u8).clamp(1, 5);
    if score <= CLEAN_MAX {
        Tint::Human(level(CLEAN_MAX - score, CLEAN_MAX))
    } else if score > AI_MIN {
        Tint::Ai(level(score - AI_MIN, 100.0 - AI_MIN))
    } else {
        Tint::Mixed
    }
}

pub fn class(t: Tint) -> String {
    match t {
        Tint::Human(l) => format!("auth-human-{l}"),
        Tint::Ai(l) => format!("auth-ai-{l}"),
        Tint::Mixed => "auth-mixed".into(),
    }
}

/// A score in words, for a tooltip and the panel.
pub fn describe(s: &Score) -> String {
    let band = match s.verdict.as_str() {
        "ai" => "reads as AI",
        "clean" => "reads as written by a person",
        _ => "mixed",
    };
    let conf = if s.confidence.is_empty() { String::new() } else { format!(", {} confidence", s.confidence) };
    format!("AI-likeness {:.0} / 100: {band} (SlopTotal's quick check{conf})", s.score)
}

/// The sentences of a paragraph, as byte ranges (the same splitting as the similarity check's).
pub fn sentences(paragraph: &str) -> Vec<Range<usize>> {
    authenticity_similar::text::sentences(paragraph)
}

/// The text a sentence is scored with: the sentence and its neighbours, one more on each side until it is long enough
/// for SlopTotal (MIN_CHARS) or the paragraph runs out. None when even the whole paragraph is too short.
pub fn context(paragraph: &str, sentences: &[Range<usize>], i: usize) -> Option<String> {
    if i >= sentences.len() {
        return None;
    }
    let (mut a, mut b) = (i.saturating_sub(1), (i + 1).min(sentences.len() - 1));
    loop {
        let text = paragraph[sentences[a].start..sentences[b].end].trim();
        if text.chars().count() >= MIN_CHARS {
            return Some(text.to_string());
        }
        if a == 0 && b + 1 >= sentences.len() {
            return None;
        }
        a = a.saturating_sub(1);
        b = (b + 1).min(sentences.len() - 1);
    }
}

/// What the AI check keeps (ai.json beside the plugin): scores by text, and which paragraphs were checked in each note.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    /// SlopTotal's scores by the text's key, and GPTZero's apart (the two are never mixed).
    pub paragraphs: HashMap<String, Scored>,
    pub gptzero: HashMap<String, Scored>,
    /// For each note, the engine of its last check ("gptzero", else SlopTotal): the one whose scores it shows.
    pub engine: HashMap<String, String>,
    pub analyses: HashMap<String, (f64, Analysis)>,
    /// By the key of the sentence's context (the text scored).
    pub sentences: HashMap<String, Scored>,
    /// For each note, the keys of the paragraphs checked in it.
    pub notes: HashMap<String, Vec<String>>,
}

impl Store {
    /// Keep a note's paragraph score (in its engine's store), and show that engine's scores for the note.
    pub fn keep(&mut self, note: &str, key: String, s: Scored) {
        let list = self.notes.entry(note.to_string()).or_default();
        if !list.contains(&key) {
            list.push(key.clone());
        }
        self.engine.insert(note.to_string(), s.score.engine.clone());
        if s.score.engine == crate::gptzero::ENGINE { self.gptzero.insert(key, s) } else { self.paragraphs.insert(key, s) };
    }

    /// The store of the engine a note shows.
    fn store_of(&self, note: &str) -> &HashMap<String, Scored> {
        if self.engine.get(note).is_some_and(|e| e == crate::gptzero::ENGINE) { &self.gptzero } else { &self.paragraphs }
    }

    /// A note's checked paragraph with this text, if any (by the engine the note shows).
    pub fn of(&self, note: &str, text: &str) -> Option<&Scored> {
        let k = key(text);
        self.notes.get(note).filter(|l| l.contains(&k)).and(self.store_of(note).get(&k))
    }

    /// The note's paragraphs (`paras`, as the note is now) that have a score still valid: a paragraph's own, or its
    /// group's while every paragraph of the group is still there unchanged (editing one invalidates the group).
    pub fn scored_in(&self, note: &str, paras: Vec<Paragraph>) -> Vec<(Paragraph, Scored)> {
        if !self.notes.contains_key(note) {
            return Vec::new();
        }
        let now: std::collections::HashSet<String> = paras.iter().map(|p| key(&p.text)).collect();
        paras
            .into_iter()
            .filter_map(|p| {
                let s = self.of(note, &p.text)?;
                s.together.as_ref().is_none_or(|t| t.span.iter().all(|k| now.contains(k))).then(|| (p, s.clone()))
            })
            .collect()
    }

    /// Forget a note's results (its paragraphs' scores stay for other notes that hold the same text).
    pub fn clear(&mut self, note: &str) -> bool {
        let Some(keys) = self.notes.remove(note) else { return false };
        let used: std::collections::HashSet<&String> = self.notes.values().flatten().collect();
        for k in keys.iter().filter(|k| !used.contains(k)) {
            self.paragraphs.remove(k);
            self.gptzero.remove(k);
            self.analyses.remove(k);
        }
        self.engine.remove(note);
        true
    }

    pub fn rename(&mut self, from: &str, to: &str) {
        if let Some(l) = self.notes.remove(from) {
            self.notes.insert(to.to_string(), l);
        }
    }

    /// Drop the oldest beyond KEEP of each kind (a note's list keeps only keys still held).
    pub fn trim(&mut self) {
        fn oldest<V>(m: &mut HashMap<String, V>, when: impl Fn(&V) -> f64) {
            if m.len() <= KEEP {
                return;
            }
            let mut ages: Vec<(f64, String)> = m.iter().map(|(k, v)| (when(v), k.clone())).collect();
            ages.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, k) in ages.into_iter().take(m.len() - KEEP) {
                m.remove(&k);
            }
        }
        oldest(&mut self.paragraphs, |s| s.when);
        oldest(&mut self.gptzero, |s| s.when);
        oldest(&mut self.sentences, |s| s.when);
        oldest(&mut self.analyses, |a| a.0);
        let (held, gz) = (&self.paragraphs, &self.gptzero);
        self.notes.values_mut().for_each(|l| l.retain(|k| held.contains_key(k) || gz.contains_key(k)));
        self.notes.retain(|_, l| !l.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prose::extract;

    fn para(text: &str, from: u32) -> Paragraph {
        Paragraph { text: text.into(), from, to: from + text.len() as u32 }
    }

    fn words(n: usize, w: &str) -> String {
        vec![w; n].join(" ")
    }

    #[test]
    fn short_paragraphs_are_scored_with_their_neighbours() {
        let p = [para(&words(10, "a"), 0), para(&words(100, "b"), 100), para(&words(30, "c"), 400), para(&words(60, "d"), 600)];
        let u = units(&p, &[false; 4], 80);
        let show = |u: &[Unit]| u.iter().map(|u| (u.span.clone(), u.members.clone())).collect::<Vec<_>>();
        // a: with the long b (b still scored alone); c: with d, which reaches 90 and is short itself, so takes the score too
        assert_eq!(show(&u), [(0..2, vec![0]), (1..2, vec![1]), (2..4, vec![2, 3])]);
        assert_eq!(together_label(&u[0], 0, 80).as_deref(), Some("scored together with the next paragraph (too short alone)"));
        assert_eq!(together_label(&u[1], 1, 80), None, "long enough: alone, no label");
        assert_eq!(together_label(&u[2], 3, 80).as_deref(), Some("scored together with the previous paragraph (too short alone)"));
        // a short last paragraph goes with the one before it
        let q = [para(&words(90, "a"), 0), para(&words(20, "b"), 200)];
        assert_eq!(show(&units(&q, &[false; 2], 80)), [(0..1, vec![0]), (0..2, vec![1])]);
        // a whole selection under the limit: one group, said to be less reliable
        let r = [para(&words(20, "a"), 0), para(&words(20, "b"), 100)];
        let u = units(&r, &[false; 2], 80);
        assert_eq!(show(&u), [(0..2, vec![0, 1])]);
        assert!(together_label(&u[0], 0, 80).unwrap().ends_with("still under 80 words, less reliable"));
    }

    #[test]
    fn groups_stay_within_their_section_when_they_can() {
        // sections: [a short, b long] | [c short, d short, e long]
        let p = [para(&words(20, "a"), 0), para(&words(90, "b"), 100), para(&words(30, "c"), 300), para(&words(30, "d"), 400), para(&words(90, "e"), 500)];
        let boundary = [false, false, true, false, false];
        let u = units(&p, &boundary, 80);
        let show = |u: &[Unit]| u.iter().map(|u| (u.span.clone(), u.members.clone())).collect::<Vec<_>>();
        assert_eq!(show(&u), [(0..2, vec![0]), (1..2, vec![1]), (2..5, vec![2, 3]), (4..5, vec![4])]);
        // a lone short paragraph under a heading: with the paragraph after the next heading
        let p = [para(&words(90, "a"), 0), para(&words(10, "b"), 200), para(&words(90, "c"), 300)];
        let u = units(&p, &[false, true, true], 80);
        assert_eq!(show(&u)[1], (1..3, vec![1]), "across the next heading, not back into the section before");
        // headings are not paragraphs to score, and mark where a section starts
        let note = "# One\n\nFirst para here.\n\n## Two\n\nSecond para.";
        let (ps, b) = sections(note, paragraphs(&crate::prose::extract(note)));
        assert_eq!(ps.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["First para here.", "Second para."]);
        assert_eq!(b, [false, true]);
    }

    #[test]
    fn a_group_is_valid_while_its_paragraphs_are_unchanged() {
        let (a, b) = (para("short one", 0), para("its long neighbour", 20));
        let mut st = Store::default();
        let together = Some(Together { span: vec![key(&a.text), key(&b.text)], label: "scored together with the next paragraph (too short alone)".into() });
        st.keep("n.md", key(&a.text), Scored { together, ..Scored::default() });
        st.keep("n.md", key(&b.text), Scored::default());
        assert_eq!(st.scored_in("n.md", vec![a.clone(), b.clone()]).len(), 2);
        // the neighbour edited: the short one's group no longer holds; the neighbour's own score is gone with its text
        let edited = para("its long neighbour, edited", 20);
        assert!(st.scored_in("n.md", vec![a, edited]).is_empty());
    }

    #[test]
    fn paragraphs_keep_their_place_in_the_note() {
        let note = "# Title\n\nFirst **para** here.\n\nSecond para.";
        let ps = paragraphs(&extract(note));
        assert_eq!(ps.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Title", "First para here.", "Second para."]);
        let utf16: Vec<u16> = note.encode_utf16().collect();
        let at = |p: &Paragraph| String::from_utf16(&utf16[p.from as usize..p.to as usize]).unwrap();
        assert_eq!(at(&ps[1]), "First **para** here.");
        assert_eq!(at(&ps[2]), "Second para.");
    }

    #[test]
    fn tints_follow_sloptotal_bands() {
        assert_eq!(tint(0.0), Tint::Human(5));
        assert_eq!(tint(34.0), Tint::Human(1));
        assert_eq!(tint(35.0), Tint::Human(1));
        assert_eq!(tint(35.5), Tint::Mixed);
        assert_eq!(tint(65.0), Tint::Mixed);
        assert_eq!(tint(66.0), Tint::Ai(1));
        assert_eq!(tint(100.0), Tint::Ai(5));
        assert_eq!(class(tint(90.0)), "auth-ai-4");
        assert_eq!(class(tint(50.0)), "auth-mixed");
    }

    #[test]
    fn a_sentence_is_scored_with_its_neighbours() {
        let p = "Short one. Then a second, longer sentence with more words in it. Third. Fourth sentence here.";
        let s = sentences(p);
        assert_eq!(s.len(), 4);
        assert_eq!(context(p, &s, 0).unwrap(), "Short one. Then a second, longer sentence with more words in it.");
        assert_eq!(context(p, &s, 2).unwrap(), "Then a second, longer sentence with more words in it. Third. Fourth sentence here.");
        assert!(context("Tiny. Small.", &sentences("Tiny. Small."), 0).is_none());
    }

    #[test]
    fn the_store_keeps_by_text_and_by_note() {
        let mut st = Store::default();
        let s = Scored { score: Score { score: 80.0, verdict: "ai".into(), ..Score::default() }, when: 1.0, device: "cpu".into(), together: None };
        st.keep("a.md", key("Some  text."), s.clone());
        assert_eq!(st.of("a.md", "Some text."), Some(&s), "white space does not count");
        assert!(st.of("b.md", "Some text.").is_none(), "only where it was checked");
        assert!(st.of("a.md", "Some text, edited.").is_none());
        st.keep("b.md", key("Some text."), s.clone());
        assert!(st.clear("a.md"));
        assert!(st.of("b.md", "Some text.").is_some(), "still used by b");
        st.rename("b.md", "c.md");
        assert!(st.of("c.md", "Some text.").is_some());
    }
}
