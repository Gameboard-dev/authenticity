//! The plagiarism check's results: wording a text shares with the library notes (verbatim and near-verbatim, from the
//! shingle index of ../similar), the field's fixed phrasing (listed apart), and close paraphrases, judged by para.rs's
//! rule. Paraphrase candidates come from the library index's content-word search (`local_plan`, `local_hits`: no
//! model, nothing leaves the computer), or, when the user turns it on and it answers, from an embedding service's
//! similarity API on this computer (`ask`, `POST /similar`). Each match keeps the note's words as written, so it can be
//! found again after edits (align.rs), and its source: the library note (vault path), the source's words, and for a
//! paraphrase the source's sentence it keeps the words of.
//!
//! The text checked is the note's own markdown (or a selection of it): quotations, citations, maths, code, comments,
//! tables and links are left out (`Masks`), so a quotation with its citation never counts. Offsets are bytes of that
//! text, given back as UTF-16 places in the note.
//!
//! **Thresholds applied afterwards.** A check keeps more than it shows (`Checked`): the runs found at the shortest run
//! it was made with, and each sentence's paraphrase candidates down to a closeness a little below the setting
//! (`fetch_floor`). `filter` applies the shortest run and the closeness the user has now, so changing either in the
//! Plagiarism pane shows its effect at once, without searching again; only a value below what was fetched needs a
//! new check (`Checked::outside`).

use std::collections::HashMap;
use std::ops::Range;

use authenticity_similar::{library as lib, text, Found, Index, Kind, Masks, Options, ParaPlan, Query, K, PARA_CANDIDATES};
use crate::para;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The check's settings (Settings → Authenticity → Plagiarism).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub min_run: usize,
    pub content: usize,
    pub common: usize,
    pub near: bool,
    pub ignore_quotes: bool,
    pub fixed_terms: Vec<String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { min_run: 6, content: 4, common: 5, near: true, ignore_quotes: true, fixed_terms: Vec::new() }
    }
}

impl Settings {
    pub fn options(&self) -> Options {
        Options { min_run: self.min_run.clamp(K, 100), content: self.content.min(50), common: self.common.min(1000), near: self.near, max_postings: 2000 }
    }

    pub fn masks(&self) -> Masks {
        Masks { markdown: true, latex: false, citations: true, quotes: self.ignore_quotes }
    }

    pub fn query(&self, text: &str) -> Query {
        Query::new(text, self.masks(), &self.fixed_terms)
    }
}

/// Fixed terms as the user lists them (one per line or comma), trimmed, at most 500 of at most 100 characters.
pub fn terms(list: &str) -> Vec<String> {
    list.split([',', '\n']).map(str::trim).filter(|t| !t.is_empty() && t.chars().count() <= 100).take(500).map(str::to_string).collect()
}

/// One finding.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Match {
    /// "verbatim", "near_verbatim", "paraphrase" or "common" (the field's fixed phrasing).
    pub kind: String,
    /// The checked text's words as written, and where they were (UTF-16, in the note) when checked.
    pub text: String,
    pub from: u32,
    pub to: u32,
    /// 1 for verbatim; the share matched exactly for near-verbatim; for a close paraphrase, the share of the
    /// sentence's content words kept in the source sentence's order (para.rs).
    pub score: f64,
    /// For a close paraphrase, the gate it passed besides the closeness (para.rs): the share of the sentence's
    /// content-word weight the source sentence holds (the content-word search), or the cosine of the embedding model's
    /// vectors for the two (the embedding search).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    pub cosine: Option<f64>,
    /// The source note (vault path), as a reader knows it, and its words.
    pub source: String,
    pub title: String,
    pub source_text: String,
    /// Other library notes with the same stretch (verbatim), or the number of sources holding it (common).
    pub also: Vec<String>,
    pub sources: usize,
    /// For a paraphrase: the source's page, when known. The line in `source` where its words are (the
    /// passage's line, from 0, as Obsidian's editor counts lines), when known.
    pub page: Option<u32>,
    pub line: Option<u32>,
    /// The run's words matched exactly (verbatim and near-verbatim; for fixed phrasing, its words), so that a longer
    /// shortest run can be applied without checking again. 0 when not recorded (results of an older version).
    #[serde(skip_serializing_if = "is_zero")]
    pub matched: usize,
    /// The note's sentence holding a verbatim or near-verbatim run, as written, and where the run starts in it (bytes);
    /// empty for a paraphrase, whose `text` is the sentence.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sentence: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub sentence_at: usize,
    /// The source's sentence holding the run's words, and where they start in it (bytes); empty when only the words
    /// are known (a paraphrase's `source_text` is the source's sentence).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_sentence: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub source_at: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Match {
    /// The words of a verbatim or near-verbatim run matched exactly (for the shortest run), or of a fixed phrase.
    pub fn run_words(&self) -> usize {
        if self.matched > 0 { self.matched } else { text::tokens(&self.text).len() }
    }

    /// The note's sentence and the run's place in it (bytes): (sentence, start, end).
    pub fn in_sentence(&self) -> (&str, usize, usize) {
        let s = &self.sentence;
        if !s.is_empty() && s.get(self.sentence_at..self.sentence_at + self.text.len()) == Some(self.text.as_str()) {
            return (s, self.sentence_at, self.sentence_at + self.text.len());
        }
        (&self.text, 0, self.text.len())
    }

    /// The source's sentence and its matching words' place in it (bytes): (sentence, start, end).
    pub fn in_source_sentence(&self) -> (&str, usize, usize) {
        let s = &self.source_sentence;
        if !s.is_empty() && s.get(self.source_at..self.source_at + self.source_text.len()) == Some(self.source_text.as_str()) {
            return (s, self.source_at, self.source_at + self.source_text.len());
        }
        (&self.source_text, 0, self.source_text.len())
    }

    /// The class that marks it (styles.css): copied wording solid, paraphrase dashed, fixed phrasing dotted and faint.
    pub fn class(&self) -> &'static str {
        match self.kind.as_str() {
            "paraphrase" => "auth-plag-para",
            "common" => "auth-plag-common",
            _ => "auth-plag-match",
        }
    }

    pub fn describe(&self) -> String {
        match self.kind.as_str() {
            "verbatim" => format!("Same wording as {}:\n\u{201c}{}\u{201d}", self.title, self.source_text),
            "near_verbatim" => format!("Nearly the same wording as {} ({:.0}% of the words the same):\n\u{201c}{}\u{201d}", self.title, self.score * 100.0, self.source_text),
            "paraphrase" => format!(
                "Close paraphrase of {}{}: {:.0}% of this sentence's content words in the source sentence's order{}. Quote it, or say it in your own structure:\n\u{201c}{}\u{201d}",
                self.title,
                self.page.map(|p| format!(", p. {p}")).unwrap_or_default(),
                self.score * 100.0,
                match (self.weight, self.cosine) {
                    (Some(w), _) => format!(", sharing its particular words ({:.0}% of their weight)", w * 100.0),
                    (_, Some(c)) => format!(", saying the same (embedding similarity {c:.2})"),
                    _ => String::new(),
                },
                self.source_text
            ),
            _ => format!("The field's fixed phrasing: found in {} library notes", self.sources),
        }
    }
}

/// UTF-16 places of byte offsets in `text`, plus `base`.
fn places(text: &str, at: &[usize], base: u32) -> Vec<u32> {
    text::utf16_offsets(text, at).into_iter().map(|u| base + u as u32).collect()
}

/// The most bytes of context kept on each side of a run, in the note's sentence and the source's.
const CONTEXT: usize = 400;

/// White space made single (a run of it becomes one space, kept at either end).
fn single_spaced(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// The nearest char boundary of `s` at or below `i`, and at or above it.
fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// The stretch of `cuts` (sentences, byte ranges of the same text) holding [start, end), at most CONTEXT bytes on
/// either side of it; [start, end) itself when no sentence holds it.
fn sentence_span(s: &str, cuts: &[Range<usize>], start: usize, end: usize) -> Range<usize> {
    let over = cuts.iter().filter(|r| r.end > start && r.start < end);
    let a = over.clone().map(|r| r.start).min().unwrap_or(start).min(start);
    let b = over.map(|r| r.end).max().unwrap_or(end).max(end);
    floor_char(s, a.max(start.saturating_sub(CONTEXT)))..ceil_char(s, b.min(end + CONTEXT))
}

/// A passage's sentences as byte ranges of it (para::passage_sentences, placed).
fn passage_cuts(p: &str) -> Vec<Range<usize>> {
    let base = p.as_ptr() as usize;
    crate::para::passage_sentences(p).into_iter().map(|s| {
        let at = s.as_ptr() as usize - base;
        at..at + s.len()
    }).collect()
}

/// The findings of a verbatim check of `text` (which starts at UTF-16 place `base` in the note), with each source's
/// title from `title`, the sentence of the note and of the source each run is in (`texts`: the passages the check
/// read, by reference), and the run's line in its source note (`note`: a library note's text).
pub fn verbatim(text: &str, base: u32, found: &Found, title: impl Fn(&str) -> String, texts: &HashMap<i64, String>, mut note: impl FnMut(&str) -> Option<String>) -> Vec<Match> {
    let mut out = Vec::new();
    let mine = text::sentences(&text::blank(text, Masks { markdown: true, latex: false, citations: true, quotes: false }));
    let mut notes: HashMap<String, Option<String>> = HashMap::new();
    for f in &found.findings {
        let p = places(text, &[f.start, f.end], base);
        let s = sentence_span(text, &mine, f.start, f.end);
        let source_text = f.source_text.split_whitespace().collect::<Vec<_>>().join(" ");
        // the source's sentence, when the run is within one passage (one that runs on into the next is shown alone)
        let passage = texts.get(&f.passage).map(String::as_str).unwrap_or("");
        let (source_sentence, source_at) = match passage.get(f.source_start..f.source_end) {
            Some(run) if f.source_end - f.source_start == f.source_text.len() && run == f.source_text => {
                let r = sentence_span(passage, &passage_cuts(passage), f.source_start, f.source_end);
                let before = single_spaced(&passage[r.start..f.source_start]).trim_start().to_string();
                let after = single_spaced(&passage[f.source_end..r.end]).trim_end().to_string();
                (format!("{before}{source_text}{after}"), before.len())
            }
            _ => (String::new(), 0),
        };
        // the line of the run in its note: the passage's place in the note, and the run's in the passage
        let held = notes.entry(f.source.clone()).or_insert_with(|| note(&f.source));
        let line = held.as_deref().and_then(|t| {
            let i = authenticity_similar::library::ref_parts(f.passage).1;
            let r = authenticity_similar::library::passages(t).into_iter().nth(i)?;
            let upto = t.get(..r.start + f.source_start)?;
            u32::try_from(upto.matches('\n').count()).ok()
        });
        out.push(Match {
            kind: if f.kind == Kind::Verbatim { "verbatim" } else { "near_verbatim" }.into(),
            text: text[f.start..f.end].to_string(),
            from: p[0],
            to: p[1],
            score: f.score,
            weight: None,
            cosine: None,
            title: title(&f.source),
            source: f.source.clone(),
            source_text,
            also: f.also.clone(),
            sources: 1 + f.also.len(),
            page: None,
            line,
            matched: f.matched,
            sentence: text[s.clone()].to_string(),
            sentence_at: f.start - s.start,
            source_sentence,
            source_at,
        });
    }
    for c in &found.common {
        let p = places(text, &[c.start, c.end], base);
        out.push(Match { kind: "common".into(), text: text[c.start..c.end].to_string(), from: p[0], to: p[1], sources: c.sources, matched: c.last - c.first, ..Match::default() });
    }
    out.sort_by_key(|m| (m.from, m.to));
    out
}

/// The sentences of `text` with at least four words: the count the Plagiarism pane gives ("in 126 sentences").
pub fn count_sentences(text: &str, s: &Settings) -> usize {
    let blanked = text::blank(text, s.masks());
    text::sentences(&blanked).into_iter().filter(|r| crate::para::words(&blanked[r.clone()]).len() >= 4).count()
}

/// ----- paraphrase (the rule is para.rs's)

/// The version of the embedding service's similarity API this plugin speaks.
pub const SIMILAR_API: u64 = 1;
/// The least cosine of the embedding model's vectors for a sentence and its source for "says the same" (the gate of the
/// embedding search; the content-word search's is para::WEIGHT).
pub const MEANING: f64 = 0.75;
/// Sentences shorter than this are not checked for paraphrase.
pub const MIN_SENTENCE_WORDS: usize = 8;
/// The passages asked for per sentence, and the least cosine a passage needs: in para.rs's calibration the source of
/// a close paraphrase was among the 8 nearest passages in 16 of 18 cases (more did not find the other two), with a
/// cosine of 0.58 or more.
pub const CANDIDATES: usize = 8;
pub const FLOOR: f64 = 0.55;

/// The sentences of `text` to check for paraphrase: those of at least MIN_SENTENCE_WORDS words that verbatim matches
/// cover less than half of. Byte ranges of `text`, with the sentence as sent (white space made single).
pub fn paraphrase_sentences(text: &str, s: &Settings, verbatim: &[Range<usize>]) -> Vec<(Range<usize>, String)> {
    let blanked = text::blank(text, s.masks());
    text::sentences(&blanked)
        .into_iter()
        .filter_map(|r| {
            // the sentence's words, without narrative citations (`@key`), which are not prose
            let words: Vec<&str> = blanked[r.clone()].split(|c: char| c.is_whitespace() || c == '\0').filter(|w| w.chars().any(char::is_alphanumeric) && !w.starts_with('@')).collect();
            if words.len() < MIN_SENTENCE_WORDS || words.join(" ").len() > 4000 {
                return None;
            }
            let covered: usize = verbatim.iter().map(|v| v.end.min(r.end).saturating_sub(v.start.max(r.start))).sum();
            (covered * 2 < r.len()).then(|| (r, words.join(" ")))
        })
        .collect()
}

/// The embedding service, as the plugin (requestUrl) or a native program (HTTP on 127.0.0.1) reaches it:
/// `POST /similar` with `body`, the answer's JSON (status 200), or why not.
#[allow(async_fn_in_trait)]
pub trait SimilarApi {
    async fn similar(&self, body: &Value) -> Result<Value, String>;
}

/// The first request: each sentence's nearest passages by the service's search model alone (no reranker, no nearest
/// sentence: the passages' words are compared here), so the whole note costs about one embedding pass.
pub fn first_request(sentences: &[&str], gpu: &str) -> Value {
    json!({ "sentences": sentences, "k": CANDIDATES, "candidates": CANDIDATES, "threshold": FLOOR, "floor": FLOOR, "rerank": false, "sentence": false,
        "min_words": MIN_SENTENCE_WORDS, "gpu": gpu })
}

/// The second request, for the few sentences that keep enough of a passage's words: within those passages' items,
/// each match with the passage's sentence nearest the caller's and its cosine (the "says the same" half of the rule).
pub fn second_request(sentences: &[&str], scope: &[String], gpu: &str) -> Value {
    json!({ "sentences": sentences, "k": CANDIDATES, "candidates": CANDIDATES, "threshold": FLOOR, "floor": FLOOR, "rerank": false, "sentence": true,
        "min_words": MIN_SENTENCE_WORDS, "scope": scope, "gpu": gpu })
}

/// The answer is the similarity API's, in the version spoken here.
pub fn check_api(v: &Value) -> Result<(), String> {
    match v["api"].as_u64() {
        Some(SIMILAR_API) => Ok(()),
        Some(n) => Err(format!("its similarity API is version {n}; this version of Authenticity speaks version {SIMILAR_API}. Update whichever is older")),
        None => Err("the service did not answer as a similarity API does (no version)".into()),
    }
}

/// Nothing was compared: the GPU had no room for the service's models (what it says about it, to ask the user).
fn shortage(v: &Value) -> Option<Value> {
    (v["gpu_shortage"].is_object() && v["done"].as_bool() != Some(true)).then(|| v["gpu_shortage"].clone())
}

/// An item key as the service gives it (8 letters and digits), else None: an untrusted answer's value.
fn item_key(v: &Value) -> Option<String> {
    v.as_str().filter(|k| k.len() == 8 && k.bytes().all(|b| b.is_ascii_alphanumeric())).map(str::to_string)
}

/// From the first answer: the sentences (their places in `sentences`) one of whose passages has a sentence keeping
/// at least `closeness` of their content words in order, with the items of those passages.
pub fn shortlist(v: &Value, sentences: &[&str], closeness: f64) -> Result<Vec<(usize, Vec<String>)>, String> {
    let list = v["results"].as_array().ok_or("the service's answer had no results in it")?;
    let mut out: Vec<(usize, Vec<String>)> = Vec::new();
    for r in list {
        let Some(i) = r["index"].as_u64().map(|i| i as usize).filter(|i| *i < sentences.len()) else { continue };
        let mine = para::content(sentences[i]);
        if mine.len() < para::MIN_CONTENT {
            continue;
        }
        let items: Vec<String> = r["matches"].as_array().into_iter().flatten().take(50).filter_map(|m| {
            let text = m["source"]["passage"]["text"].as_str()?;
            (para::closest(&mine, &short(text, 20_000)).0 >= closeness).then(|| item_key(&m["source"]["item_key"])).flatten()
        }).collect();
        if !items.is_empty() && !out.iter().any(|(j, _)| *j == i) {
            out.push((i, items));
        }
    }
    Ok(out)
}

/// A close paraphrase found for a sentence, without its place in the note (kept in the cache by the sentence).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    /// The share of the sentence's content words kept in the source sentence's order, and the gate it passed: the
    /// source sentence's share of their weight (content-word search) or the two's cosine (embedding search).
    pub kept: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default)]
    pub cosine: Option<f64>,
    pub title: String,
    pub source: String,
    pub source_text: String,
    pub page: Option<u32>,
    pub line: Option<u32>,
}

/// Every candidate among a sentence's matches in the second answer that keeps at least `floor` of its content words
/// in order and says the same (para.rs's rule at `floor`), the best first (kept words plus cosine), one per source
/// sentence. The pane's closeness then chooses among them (`filter`).
pub fn candidates(matches: &Value, sentence: &str, floor: f64) -> Vec<Hit> {
    let mine = para::content(sentence);
    let vault_path = |p: &Value| p.as_str().and_then(|n| crate::library::folder(n.trim_end_matches(".md")).map(|_| n.to_string()));
    let mut out: Vec<Hit> = Vec::new();
    for m in matches.as_array().into_iter().flatten().take(50) {
        let src = &m["source"];
        let Some(text) = src["sentence"]["text"].as_str() else { continue };
        let Some(cosine) = src["sentence"]["cosine"].as_f64().filter(|c| c.is_finite() && (-1.0..=1.0).contains(c)) else { continue };
        let kept = para::in_order(&mine, &para::content(&short(text, 4000)));
        if mine.len() < para::MIN_CONTENT || kept < floor || cosine < MEANING {
            continue;
        }
        // the file holding the passage (a long source's section note), else the source's note
        let note = vault_path(&src["file"]["path"]).or_else(|| vault_path(&src["note"])).unwrap_or_default();
        let line = src["file"]["line"].as_u64().and_then(|n| u32::try_from(n).ok()).filter(|_| !note.is_empty());
        let title = [src["citekey"].as_str(), src["title"].as_str()].into_iter().flatten().map(|t| short(t, 120)).collect::<Vec<_>>().join(" \u{203a} ");
        let hit = Hit {
            kept: (kept * 1000.0).round() / 1000.0,
            weight: None,
            cosine: Some((cosine * 1000.0).round() / 1000.0),
            title: if title.is_empty() { note.clone() } else { title },
            source: note,
            source_text: short(text, 1500).split_whitespace().collect::<Vec<_>>().join(" "),
            page: src["passage"]["page"].as_u64().and_then(|n| u32::try_from(n).ok()).filter(|p| *p > 0),
            line,
        };
        match out.iter_mut().find(|h| h.source == hit.source && h.source_text == hit.source_text) {
            Some(h) if h.rank() < hit.rank() => *h = hit,
            Some(_) => {}
            None => out.push(hit),
        }
    }
    out.sort_by(|a, b| b.rank().total_cmp(&a.rank()));
    out
}

impl Hit {
    /// How good a candidate is, to choose the best per sentence: kept words plus the gate's value.
    pub fn rank(&self) -> f64 {
        self.kept + self.weight.or(self.cosine).unwrap_or(0.0)
    }
}

/// The best close paraphrase among a sentence's matches in the second answer (para.rs's rule), if any.
pub fn judge(matches: &Value, sentence: &str, closeness: f64) -> Option<Hit> {
    candidates(matches, sentence, closeness).into_iter().next()
}

/// What asking the embedding service gave: each sentence's candidates (none, one or a few), or a GPU shortage to ask
/// the user about.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Hits(Vec<Vec<Hit>>),
    Shortage(Value),
}

/// How far below the closeness setting candidates are fetched, so the pane can lower it a little without a new check.
pub const FETCH_MARGIN: f64 = 0.1;

/// The closeness candidates are fetched from for a setting of `closeness`: FETCH_MARGIN lower, never below the
/// least the settings offer (nor above the setting itself), to two decimals.
pub fn fetch_floor(closeness: f64) -> f64 {
    let f = (closeness - FETCH_MARGIN).max(para::MIN_CLOSENESS).min(closeness);
    (f * 100.0).round() / 100.0
}

/// Ask the embedding service about `sentences` (two requests: see first_request and second_request) and keep each sentence's
/// candidates from closeness `floor` up.
pub async fn ask<Q: SimilarApi>(q: &Q, sentences: &[String], floor: f64, gpu: &str) -> Result<Outcome, String> {
    let mut hits = vec![Vec::new(); sentences.len()];
    if sentences.is_empty() {
        return Ok(Outcome::Hits(hits));
    }
    let refs: Vec<&str> = sentences.iter().map(String::as_str).collect();
    let v = q.similar(&first_request(&refs, gpu)).await?;
    check_api(&v)?;
    if let Some(s) = shortage(&v) {
        return Ok(Outcome::Shortage(s));
    }
    let short = shortlist(&v, &refs, floor)?;
    if short.is_empty() {
        return Ok(Outcome::Hits(hits));
    }
    let subset: Vec<&str> = short.iter().map(|(i, _)| refs[*i]).collect();
    let mut scope: Vec<String> = short.iter().flat_map(|(_, items)| items.iter().cloned()).collect();
    scope.sort();
    scope.dedup();
    let v = q.similar(&second_request(&subset, &scope, gpu)).await?;
    check_api(&v)?;
    if let Some(s) = shortage(&v) {
        return Ok(Outcome::Shortage(s));
    }
    for r in v["results"].as_array().ok_or("the service's answer had no results in it")? {
        let Some(k) = r["index"].as_u64().map(|k| k as usize).filter(|k| *k < subset.len()) else { continue };
        let i = short[k].0;
        hits[i] = candidates(&r["matches"], &sentences[i], floor);
    }
    Ok(Outcome::Hits(hits))
}

/// The candidates as matches on `text` (at UTF-16 place `base` in the note): one per candidate, so a sentence may have
/// several (`filter` keeps the best that passes the closeness).
pub fn paraphrase_matches(text: &str, base: u32, sent: &[(Range<usize>, String)], hits: &[Vec<Hit>]) -> Vec<Match> {
    sent.iter().zip(hits).flat_map(|((r, _), hs)| {
        let p = places(text, &[r.start, r.end], base);
        hs.iter().map(move |h| Match {
            kind: "paraphrase".into(),
            text: text[r.clone()].to_string(),
            from: p[0],
            to: p[1],
            score: h.kept,
            weight: h.weight,
            cosine: h.cosine,
            title: h.title.clone(),
            source: h.source.clone(),
            source_text: h.source_text.clone(),
            page: h.page,
            line: h.line,
            ..Match::default()
        })
    }).collect()
}

/// The content-word search's first step for `sentences` (as `paraphrase_sentences` gives them): each sentence's content
/// stems, and its candidate passages among the sources `allowed` accepts (`Index::para_plan`); the plan's `refs` are the
/// passages whose texts `local_hits` needs.
pub fn local_plan(ix: &Index, sentences: &[(Range<usize>, String)], allowed: &dyn Fn(&str) -> bool) -> (Vec<Vec<String>>, ParaPlan) {
    let stems: Vec<Vec<String>> = sentences.iter().map(|(_, s)| para::content(s)).collect();
    let plan = ix.para_plan(&stems, allowed, PARA_CANDIDATES);
    (stems, plan)
}

/// The second step: each sentence's candidates from closeness `floor` up that pass the content-word gate
/// (para::WEIGHT), the best first, as `Hit`s: `texts` are the candidate passages' texts by reference, `title` names a
/// source, and `note` gives a source note's text (for the line of the passage).
pub fn local_hits(ix: &Index, stems: &[Vec<String>], plan: &ParaPlan, texts: &HashMap<i64, String>, floor: f64, title: impl Fn(&str) -> String, mut note: impl FnMut(&str) -> Option<String>) -> Vec<Vec<Hit>> {
    let mut lines: HashMap<i64, Option<u32>> = HashMap::new();
    ix.para_finish(stems, plan, texts, floor)
        .into_iter()
        .zip(stems)
        .map(|(found, mine)| {
            found
                .into_iter()
                .filter(|h| para::close(mine, h.kept, h.weight, h.shared, floor))
                .map(|h| {
                    let line = *lines.entry(h.passage).or_insert_with(|| {
                        let (_, i) = lib::ref_parts(h.passage);
                        let t = note(&h.key)?;
                        let at = lib::passages(&t).get(i)?.start;
                        u32::try_from(t[..at].matches('\n').count()).ok()
                    });
                    Hit { kept: h.kept, weight: Some(h.weight), cosine: None, title: title(&h.key), source: h.key, source_text: short(&h.sentence, 1500), page: None, line }
                })
                .collect()
        })
        .collect()
}

/// The content-word search in one step, for a program that reads the notes itself (the native programs): `text`'s
/// sentences (verbatim matches covering less than half of them) against the sources `allowed` accepts, the candidates'
/// notes read with `read` (each once), each sentence's candidates from closeness `floor` up, as matches on `text`.
#[allow(clippy::too_many_arguments)]
pub fn paraphrase_local(ix: &Index, keys: &HashMap<i64, String>, text: &str, s: &Settings, covered: &[Range<usize>], allowed: &dyn Fn(&str) -> bool, floor: f64, library: &str, mut read: impl FnMut(&str) -> Option<String>) -> Vec<Match> {
    let sentences = paraphrase_sentences(text, s, covered);
    if sentences.is_empty() {
        return Vec::new();
    }
    let (stems, plan) = local_plan(ix, &sentences, allowed);
    let mut notes: HashMap<String, Option<String>> = HashMap::new();
    for r in &plan.refs {
        if let Some(k) = keys.get(&lib::ref_parts(*r).0) {
            if !notes.contains_key(k) {
                notes.insert(k.clone(), read(k));
            }
        }
    }
    let texts = lib::passage_texts(&plan.refs, keys, |k| notes.get(k).cloned().flatten());
    let hits = local_hits(ix, &stems, &plan, &texts, floor, |k| crate::library::title(k, library), |k| notes.get(k).cloned().flatten());
    paraphrase_matches(text, 0, &sentences, &hits)
}

/// What was found for sentences already asked about, by the sentence, with the closeness its candidates were fetched
/// from, so that checking a note again asks the embedding service only about the sentences that changed (or that a lower closeness
/// needs). Kept across restarts (paraphrase.json beside the plugin). Used by the plugin API's checkSimilarity only: a
/// check the user chooses (Check for plagiarism) always asks again, so it never returns stale results.
#[derive(Default, Serialize, Deserialize)]
pub struct Cache {
    map: HashMap<u64, (f64, Vec<Hit>)>,
}

impl Cache {
    /// The most sentences kept; past it the cache starts again.
    const MAX: usize = 20_000;

    fn key(sentence: &str) -> u64 {
        text::word_hash(sentence)
    }

    /// The sentence's candidates from closeness `floor` up, if it was asked about from that closeness or lower.
    pub fn get(&self, sentence: &str, floor: f64) -> Option<Vec<Hit>> {
        let (had, hits) = self.map.get(&Self::key(sentence))?;
        (*had <= floor + 1e-9).then(|| hits.iter().filter(|h| h.kept + 1e-9 >= floor).cloned().collect())
    }

    pub fn put(&mut self, sentence: &str, floor: f64, hits: Vec<Hit>) {
        if self.map.len() >= Self::MAX {
            self.map.clear();
        }
        self.map.insert(Self::key(sentence), (floor, hits));
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Forget one sentence (a check asked to start afresh).
    pub fn forget(&mut self, sentence: &str) {
        self.map.remove(&Self::key(sentence));
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

// ----- the thresholds, applied to what a check found

/// The thresholds the user has now: the shortest run (words) and the close-paraphrase closeness (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub min_run: usize,
    pub closeness: f64,
}

impl Thresholds {
    /// From the settings' values, kept within what the check can do.
    pub fn new(min_run: f64, closeness: f64) -> Thresholds {
        let closeness = if closeness.is_finite() { closeness } else { para::CLOSENESS };
        Thresholds { min_run: (min_run.max(0.0) as usize).clamp(K, 100), closeness: closeness.clamp(para::MIN_CLOSENESS, 1.0) }
    }
}

const EPS: f64 = 1e-9;

/// The findings that pass `t`: verbatim and near-verbatim runs (and fixed phrasing) of at least `t.min_run` words
/// matched exactly, and for each sentence the best paraphrase candidate (kept words plus its gate's value) that keeps
/// at least `t.closeness` of its content words in order and passes its search's gate (para::WEIGHT for the
/// content-word search, MEANING for the embedding search). In the note's order.
pub fn filter(found: &[Match], t: Thresholds) -> Vec<Match> {
    let mut out: Vec<Match> = Vec::new();
    let mut best: Vec<&Match> = Vec::new();
    let gate = |m: &Match| match (m.weight, m.cosine) {
        (Some(w), _) => (w >= para::WEIGHT).then_some(w),
        (_, Some(c)) => (c >= MEANING).then_some(c),
        _ => None,
    };
    for m in found {
        if m.kind == "paraphrase" {
            let Some(g) = gate(m) else { continue };
            if m.score + EPS < t.closeness {
                continue;
            }
            match best.iter_mut().find(|b| b.from == m.from && b.to == m.to) {
                Some(b) if b.score + gate(b).unwrap_or(0.0) < m.score + g => *b = m,
                Some(_) => {}
                None => best.push(m),
            }
        } else if m.run_words() >= t.min_run.max(K) {
            out.push(m.clone());
        }
    }
    out.extend(best.into_iter().cloned());
    out.sort_by_key(|m| (m.from, m.to));
    out
}

/// What a plagiarism check of a note found, before the thresholds are applied (kept in found.json).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Checked {
    /// The shortest run the check looked for: a longer one is applied at once, a shorter one needs a new check.
    pub min_run: usize,
    /// The closeness paraphrase candidates were kept from (None: paraphrase was not checked).
    pub floor: Option<f64>,
    /// Every run found, the fixed phrasing, and every paraphrase candidate from `floor` up.
    pub found: Vec<Match>,
    /// The sentences checked (of four words or more), and whether the last check was of a selection only.
    pub sentences: usize,
    pub selection: bool,
}

impl Checked {

    /// The findings that pass `t` (`filter`).
    pub fn matches(&self, t: Thresholds) -> Vec<Match> {
        filter(&self.found, t)
    }

    /// Results of an older version (only what passed its thresholds then): kept as found at `t`.
    pub fn from_matches(found: Vec<Match>, t: Thresholds) -> Checked {
        Checked { min_run: t.min_run, floor: Some(t.closeness), found, sentences: 0, selection: false }
    }

    /// The thresholds of `t` the last check cannot answer without checking again, said for the pane.
    pub fn outside(&self, t: Thresholds) -> Vec<String> {
        let mut out = Vec::new();
        if self.min_run > 0 && t.min_run < self.min_run {
            out.push(format!("Runs shorter than {} words were not looked for by the last check.", self.min_run));
        }
        if let Some(f) = self.floor {
            if t.closeness + EPS < f {
                out.push(format!("Paraphrases keeping less than {:.0}% were not fetched by the last check.", f * 100.0));
            }
        }
        out
    }

    /// A newer check of the note (`range`: of a selection, whose findings replace those it covers) taken in. The
    /// limits that hold for all of it are the stricter of the two.
    pub fn merge(&mut self, new: Checked, range: Option<(u32, u32)>) {
        let Some((s, e)) = range else {
            *self = new;
            return;
        };
        if self.found.is_empty() && self.min_run == 0 {
            *self = Checked { selection: true, ..new };
            return;
        }
        self.found.retain(|m| m.to <= s || m.from >= e);
        self.found.extend(new.found);
        self.found.sort_by_key(|m| (m.from, m.to));
        self.min_run = self.min_run.max(new.min_run);
        self.floor = match (self.floor, new.floor) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if self.selection || self.sentences == 0 {
            self.sentences = new.sentences;
            self.selection = true;
        }
    }
}

fn short(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use authenticity_similar::{library, Index};

    const SOURCE: &str = "Earlier studies reported that the proposed sampling procedure reduces the variance of every estimator considerably.";

    fn index() -> Index {
        let mut ix = Index::new();
        ix.update(&[], vec![library::source("Library/a2020/Paper A.md", SOURCE, [0; 3])]);
        ix
    }

    #[test]
    fn verbatim_matches_are_placed_in_the_note() {
        let ix = index();
        let s = Settings::default();
        let draft = "😀 We see that the proposed sampling procedure reduces the variance of every estimator, \"quoted words are left out\" here.";
        let q = s.query(draft);
        let found = ix.find(&q, &s.options(), &|_| true, &mut |refs| Ok(refs.iter().map(|r| (*r, SOURCE.to_string())).collect())).unwrap();
        let texts: HashMap<i64, String> = found.findings.iter().map(|f| (f.passage, SOURCE.to_string())).collect();
        let m = verbatim(draft, 100, &found, |k| crate::library::title(k, "Library/"), &texts, |k| (k == "Library/a2020/Paper A.md").then(|| SOURCE.to_string()));
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].kind.as_str(), m[0].title.as_str()), ("verbatim", "a2020 \u{203a} Paper A"));
        assert_eq!(m[0].text, "that the proposed sampling procedure reduces the variance of every estimator");
        let units: Vec<u16> = draft.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[(m[0].from - 100) as usize..(m[0].to - 100) as usize]).unwrap(), m[0].text);
        assert!(m[0].describe().starts_with("Same wording as a2020"));
        assert_eq!(m[0].class(), "auth-plag-match");
        // the sentences around the run, on both sides, and the run's line in the source note
        assert_eq!(m[0].matched, 11);
        assert_eq!(m[0].in_sentence(), (draft, draft.find("that the").unwrap(), draft.find(", \"quoted").unwrap()));
        let (ss, a, b) = m[0].in_source_sentence();
        assert_eq!((ss, &ss[a..b]), (SOURCE, m[0].text.as_str()));
        assert_eq!(m[0].line, Some(0));
    }

    fn run(kind: &str, from: u32, words: usize) -> Match {
        Match { kind: kind.into(), text: "w ".repeat(words).trim().into(), from, to: from + 10, score: 1.0, matched: words, ..Match::default() }
    }

    fn cand(from: u32, kept: f64, cosine: f64, source: &str) -> Match {
        Match { kind: "paraphrase".into(), text: "a sentence".into(), from, to: from + 10, score: kept, cosine: Some(cosine), source: source.into(), ..Match::default() }
    }

    /// The pane's thresholds re-applied to what a check found, without a new check.
    #[test]
    fn thresholds_filter_what_was_found() {
        let found = vec![run("verbatim", 0, 6), run("near_verbatim", 20, 9), run("common", 40, 7), cand(60, 0.42, 0.9, "A"), cand(60, 0.55, 0.8, "B"), cand(60, 0.7, 0.7, "C"), cand(80, 0.45, 0.95, "D")];
        let kinds = |t: Thresholds| filter(&found, t).iter().map(|m| format!("{}@{}{}", m.kind, m.from, m.source)).collect::<Vec<_>>();
        assert_eq!(kinds(Thresholds::new(6.0, 0.5)), ["verbatim@0", "near_verbatim@20", "common@40", "paraphrase@60B"], "C does not say the same (cosine under 0.75)");
        assert_eq!(kinds(Thresholds::new(8.0, 0.5)), ["near_verbatim@20", "paraphrase@60B"], "a longer shortest run drops the 6- and 7-word runs");
        assert_eq!(kinds(Thresholds::new(6.0, 0.4)), ["verbatim@0", "near_verbatim@20", "common@40", "paraphrase@60B", "paraphrase@80D"], "lower: the best candidate per sentence (B: 0.55 + 0.8 over A: 0.42 + 0.9)");
        assert_eq!(kinds(Thresholds::new(6.0, 0.6)), ["verbatim@0", "near_verbatim@20", "common@40"]);
        // results of an older version, without the words matched: counted from the text
        let old = Match { matched: 0, ..run("verbatim", 0, 7) };
        assert_eq!(old.run_words(), 7);
        assert_eq!(filter(&[old.clone()], Thresholds::new(7.0, 0.5)).len(), 1);
        assert!(filter(&[old], Thresholds::new(8.0, 0.5)).is_empty());
        // kept within what the check can do
        assert_eq!(Thresholds::new(1.0, f64::NAN), Thresholds { min_run: K, closeness: para::CLOSENESS });
    }

    /// The paraphrase closeness: 50% by default, never below 30% (whatever data.json says), the same for both searches.
    #[test]
    fn the_closeness_has_a_default_and_a_floor() {
        assert_eq!(para::CLOSENESS, 0.5);
        assert_eq!(crate::app::Settings::default().thresholds().closeness, 0.5, "the default setting");
        assert_eq!(Thresholds::new(6.0, 0.1).closeness, para::MIN_CLOSENESS, "below the floor: the floor");
        assert_eq!(Thresholds::new(6.0, 0.3).closeness, 0.3);
        assert_eq!(Thresholds::new(6.0, f64::INFINITY).closeness, para::CLOSENESS, "not a number: the default");
        assert_eq!(fetch_floor(0.3), para::MIN_CLOSENESS, "a check never fetches below the floor");
    }

    /// The content-word search's candidates are re-filtered by the closeness as they were found, without a new search,
    /// by the same rule as the embedding search's (kept words in order), each with its own gate.
    #[test]
    fn local_candidates_are_refiltered_without_a_new_search() {
        let local = |from: u32, kept: f64, weight: f64, source: &str| Match { kind: "paraphrase".into(), text: "a sentence".into(), from, to: from + 10, score: kept, weight: Some(weight), source: source.into(), ..Match::default() };
        let found = vec![local(0, 0.62, 0.7, "A"), local(20, 0.48, 0.9, "B"), local(40, 0.8, 0.3, "C"), cand(60, 0.55, 0.8, "D")];
        let kinds = |c: f64| filter(&found, Thresholds::new(6.0, c)).iter().map(|m| format!("{}{}", m.from, m.source)).collect::<Vec<_>>();
        assert_eq!(kinds(0.5), ["0A", "60D"], "C shares only common ground (weight under 0.45)");
        assert_eq!(kinds(0.45), ["0A", "20B", "60D"], "lower flags more");
        assert_eq!(kinds(0.6), ["0A"]);
        assert_eq!(kinds(0.7), Vec::<String>::new());
        assert_eq!(kinds(0.1), kinds(0.3), "never below the floor");
    }

    #[test]
    fn what_was_fetched_says_when_a_new_check_is_needed() {
        assert_eq!(fetch_floor(0.5), 0.4);
        assert_eq!(fetch_floor(0.35), 0.3);
        assert_eq!(fetch_floor(0.3), 0.3);
        assert_eq!(fetch_floor(0.25), 0.25, "never above the setting");
        let c = Checked { min_run: 6, floor: Some(0.4), found: vec![run("verbatim", 0, 6), cand(60, 0.45, 0.9, "A")], sentences: 12, selection: false };
        assert!(c.outside(Thresholds::new(6.0, 0.4)).is_empty());
        assert!(c.outside(Thresholds::new(9.0, 0.9)).is_empty(), "higher is always answered");
        let why = c.outside(Thresholds::new(5.0, 0.35));
        assert_eq!(why.len(), 2);
        assert!(why[0].contains("shorter than 6 words") && why[1].contains("less than 40%"), "{why:?}");
        assert_eq!(c.matches(Thresholds::new(6.0, 0.45)).len(), 2);
        assert!(Checked { floor: None, ..c.clone() }.outside(Thresholds::new(6.0, 0.2)).is_empty(), "paraphrase not checked: nothing to say about closeness");
    }

    #[test]
    fn a_selection_check_is_layered_over_the_note() {
        let mut c = Checked { min_run: 6, floor: Some(0.4), found: vec![run("verbatim", 0, 6), run("verbatim", 100, 6)], sentences: 50, selection: false };
        let sel = Checked { min_run: 8, floor: Some(0.3), found: vec![run("verbatim", 95, 8)], sentences: 3, selection: true };
        c.merge(sel.clone(), Some((90, 120)));
        assert_eq!(c.found.iter().map(|m| m.from).collect::<Vec<_>>(), [0, 95]);
        assert_eq!((c.min_run, c.floor, c.sentences, c.selection), (8, Some(0.4), 50, false), "the stricter limits; the note's count stays");
        let mut fresh = Checked::default();
        fresh.merge(sel.clone(), Some((90, 120)));
        assert_eq!((fresh.sentences, fresh.selection), (3, true));
        c.merge(Checked { selection: false, ..sel }, None);
        assert_eq!(c.found.len(), 1, "a check of the whole note replaces everything");
    }

    #[test]
    fn sentences_for_paraphrase_skip_short_and_copied_ones() {
        let s = Settings::default();
        let text = "Too short to send. This sentence has well over eight words in it and is checked. The copied sentence here is entirely covered by a verbatim match today. @smith2020 showed that the narrative citation is left out of the sentence sent.";
        let copied = text.find("The copied").unwrap();
        let got = paraphrase_sentences(text, &s, &[copied..text.find(" @smith").unwrap()]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].1, "This sentence has well over eight words in it and is checked.");
        assert_eq!(got[1].1, "showed that the narrative citation is left out of the sentence sent.");
        let body = first_request(&[got[0].1.as_str()], "");
        assert_eq!((body["sentences"][0].as_str(), body["rerank"].as_bool(), body["sentence"].as_bool()), (Some(got[0].1.as_str()), Some(false), Some(false)));
    }

    const PARA_SRC: &str = "The proposed sampling procedure reduces the variance of every estimator considerably on all benchmarks.";
    const CLOSE: &str = "The suggested sampling method lowers the variance of each estimator considerably across all benchmarks.";
    const OWN: &str = "Every estimator became far less variable once we sampled in the new way, on each benchmark tried.";

    /// The embedding service, faked: the requests it was sent, and its answers in turn.
    struct Fake {
        seen: std::cell::RefCell<Vec<Value>>,
        answers: std::cell::RefCell<Vec<Value>>,
    }

    impl SimilarApi for Fake {
        async fn similar(&self, body: &Value) -> Result<Value, String> {
            self.seen.borrow_mut().push(body.clone());
            let mut a = self.answers.borrow_mut();
            if a.is_empty() { Err("no more answers".into()) } else { Ok(a.remove(0)) }
        }
    }

    fn fake(answers: Vec<Value>) -> Fake {
        Fake { seen: Default::default(), answers: std::cell::RefCell::new(answers) }
    }

    fn passage(key: &str, text: &str, sentence: Option<(&str, f64)>) -> Value {
        json!({ "score": 0.7, "cosine": 0.7, "scored_by": "cosine", "source": { "item_key": key, "citekey": "b2019", "title": "Paper B", "note": "Library/b2019/Paper B.md",
            "file": { "path": "Library/b2019/03 Results.md", "line": 41 }, "passage": { "page": 4, "text": text },
            "sentence": sentence.map(|(t, c)| json!({ "text": t, "cosine": c })) } })
    }

    fn answer(results: Vec<Value>) -> Value {
        json!({ "api": 1, "done": true, "gpu_shortage": null, "results": results })
    }

    /// The two requests: the first finds the passages, the sentences keeping enough of a passage's words go to the
    /// second (within those items), whose nearest sentences decide.
    #[test]
    fn close_paraphrase_is_found_and_a_restatement_is_not() {
        let text = format!("{CLOSE} {OWN}");
        let sent = vec![(0..CLOSE.len(), CLOSE.to_string()), (CLOSE.len() + 1..text.len(), OWN.to_string())];
        let ptext = format!("We describe the setting first. {PARA_SRC}");
        let first = answer(vec![json!({ "index": 0, "matches": [passage("AB12CD34", &ptext, None), passage("../etc/x", &ptext, None)] }), json!({ "index": 1, "matches": [passage("EF56GH78", &ptext, None)] })]);
        let second = answer(vec![json!({ "index": 0, "matches": [passage("AB12CD34", &ptext, Some((OWN, 0.99))), passage("AB12CD34", &ptext, Some((PARA_SRC, 0.86)))] })]);
        let q = fake(vec![first, second]);
        let sentences: Vec<String> = sent.iter().map(|s| s.1.clone()).collect();
        let Outcome::Hits(hits) = crate::sloptotal::block_on(ask(&q, &sentences, para::CLOSENESS, "")).unwrap() else { panic!() };
        let seen = q.seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1]["sentences"], json!([CLOSE]), "only the sentence that keeps a passage's words is asked about again");
        assert_eq!(seen[1]["scope"], json!(["AB12CD34"]), "within its passages' items; a key that is not one is dropped");
        assert_eq!(seen[1]["sentence"], true);
        let h = hits[0].first().expect("the close paraphrase is found");
        assert!(hits[1].is_empty(), "the restatement in another structure is not");
        assert!(h.kept >= 0.5 && h.cosine == Some(0.86), "{h:?}");
        assert_eq!((h.source_text.as_str(), h.source.as_str(), h.page, h.line, h.title.as_str()), (PARA_SRC, "Library/b2019/03 Results.md", Some(4), Some(41), "b2019 \u{203a} Paper B"));
        let m = paraphrase_matches(&text, 10, &sent, &hits);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].kind.as_str(), m[0].from, m[0].to, m[0].class()), ("paraphrase", 10, 10 + CLOSE.len() as u32, "auth-plag-para"));
        assert!(m[0].describe().starts_with("Close paraphrase of b2019 \u{203a} Paper B, p. 4: 56% "), "{}", m[0].describe());
        // the same words but not the same statement (cosine under the rule's least): not a finding
        assert!(judge(&json!([passage("AB12CD34", &ptext, Some((PARA_SRC, 0.7)))]), CLOSE, para::CLOSENESS).is_none());
    }

    #[test]
    fn the_service_s_answers_are_checked() {
        let sentences = vec![CLOSE.to_string()];
        let run = |answers: Vec<Value>| crate::sloptotal::block_on(ask(&fake(answers), &sentences, para::CLOSENESS, ""));
        assert!(run(vec![json!({ "api": 2 })]).unwrap_err().contains("version 2"));
        assert!(run(vec![json!({ "results": [] })]).is_err());
        assert!(matches!(run(vec![json!({ "api": 1, "done": false, "gpu_shortage": { "need_mb": 3000 } })]), Ok(Outcome::Shortage(_))));
        assert_eq!(run(vec![answer(vec![json!({ "index": 0, "matches": [] })])]), Ok(Outcome::Hits(vec![vec![]])), "no passage near: one request only");
        // hostile values: an index out of range, a cosine out of range, a path that is not a vault path
        let ptext = PARA_SRC.to_string();
        let mut odd = passage("AB12CD34", &ptext, Some((PARA_SRC, 0.9)));
        odd["source"]["file"]["path"] = json!("../../.obsidian/x");
        odd["source"]["note"] = json!("../../.obsidian/x");
        let first = answer(vec![json!({ "index": 0, "matches": [passage("AB12CD34", &ptext, None)] })]);
        let second = answer(vec![json!({ "index": 7, "matches": [passage("AB12CD34", &ptext, Some((PARA_SRC, 0.9)))] }), json!({ "index": 0, "matches": [passage("AB12CD34", &ptext, Some((PARA_SRC, 3.0))), odd] })]);
        let Ok(Outcome::Hits(h)) = run(vec![first, second]) else { panic!() };
        assert_eq!(h[0].first().map(|h| (h.cosine, h.source.as_str(), h.line)), Some((Some(0.9), "", None)));
    }

    #[test]
    fn findings_are_kept_by_sentence() {
        let hit = |kept: f64| Hit { kept, weight: None, cosine: Some(0.8), title: String::new(), source: String::new(), source_text: format!("{kept}"), page: None, line: None };
        let mut c = Cache::default();
        assert_eq!(c.get(CLOSE, 0.5), None);
        c.put(CLOSE, 0.4, vec![]);
        assert_eq!(c.get(CLOSE, 0.4), Some(vec![]), "asked, nothing found");
        c.put(CLOSE, 0.4, vec![hit(0.45), hit(0.6)]);
        assert_eq!(c.get(CLOSE, 0.5), Some(vec![hit(0.6)]), "a higher closeness is answered from what was fetched");
        assert_eq!(c.get(CLOSE, 0.3), None, "a lower one is asked again");
        c.clear();
        assert_eq!(c.get(CLOSE, 0.4), None);
    }

    /// What the embedding search said survives a restart (JSON), and a sentence can be forgotten.
    #[test]
    fn the_paraphrase_cache_is_kept() {
        // the cache of what the embedding search said survives a restart (JSON)
        let mut cache = Cache::default();
        cache.put(CLOSE, 0.4, vec![Hit { kept: 0.6, weight: None, cosine: Some(0.8), title: "t".into(), source: "s.md".into(), source_text: "x".into(), page: Some(2), line: None }]);
        let back: Cache = serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert_eq!(back.get(CLOSE, 0.5).map(|h| h.len()), Some(1));
        let mut back = back;
        back.forget(CLOSE);
        assert!(back.is_empty());
    }

    #[test]
    fn fixed_terms_are_listed_by_line_or_comma() {
        assert_eq!(terms("machine learning, neural network\n\n  deep learning "), ["machine learning", "neural network", "deep learning"]);
    }
}
