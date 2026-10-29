//! SlopTotal (<https://github.com/pablocaeg/sloptotal>, MIT), the local AI-text detector Authenticity uses: its scoring
//! API on 127.0.0.1, as the plugin and the MCP server speak it (through Authenticity's helper, which starts SlopTotal,
//! or to a SlopTotal already running). Only these endpoints are used, always with a body of `{"text": ...}` alone (the
//! `url` field, which makes SlopTotal fetch a web page, is never sent):
//!
//! - `GET /health`: the detectors loaded, their number, and those that failed to load (`engines.failed`);
//! - `POST /api/quick-score`: one text, scored by SlopTotal's quick classifiers (0-100, a verdict, a confidence and
//!   each classifier's own result), one request per paragraph, a few at a time (`Env::parallel`); SlopTotal's
//!   paragraph-score is not used: it gives no classifier's result, so a failed one could not be left out;
//! - `POST /api/analyze`: every detector on one text, each with its score and a one-line reason (`details`): the
//!   reasons the AI Detector pane shows when asked (Load reasons);
//! - `GET /api/queue/ticket/{id}`: quick-score and analyze may answer 202 with a ticket to poll, or 429 with
//!   `retry_after` (seconds) when SlopTotal is busy.
//!
//! Fails closed where a score would mislead: a detector that could not load scores 0 and still counts in SlopTotal's
//! overall score, so nothing is scored while `/health` lists a failed detector. A detector that fails on one text is
//! shown, not hidden, and left out of the score: an analysis keeps the reasons of the detectors that ran and lists
//! those whose reason starts "Model loading failed:" or "Engine error:" apart, and its score is the weighted average
//! of those that ran (`Weights::without_failed`: SlopTotal's model with its weights renormalised over them; SlopTotal's
//! own number counts them as 0 and is not shown), or none when too few ran; a quick score with a quick detector in
//! error likewise, when SlopTotal gives each quick detector's result. Answers are read field by field (allow-listed
//! keys, strict types, lengths capped): another program may answer on the port.
//!
//! The flows are written against `Env` (a request, a pause), so they run in Obsidian (requestUrl, timers), in the MCP
//! server (blocking HTTP) and in tests (a fake).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// SlopTotal refuses shorter text (its MIN_TEXT_LENGTH).
pub const MIN_CHARS: usize = 50;
/// SlopTotal reads at most this many characters of a text (its MAX_ANALYSED_CHARS); one request of paragraphs is kept
/// below it.
pub const MAX_CHARS: usize = 100_000;
/// SlopTotal's quick lane runs at most this many requests at once (its queue's "quick" size); more wait in its queue.
pub const QUICK_LANE: usize = 4;
/// Under this many words a paragraph's score is shown as less reliable (SlopTotal's own short-text limit).
pub const SHORT_WORDS: usize = 80;
/// The quick score's bands (SlopTotal's QUICK_CLEAN_MAX and QUICK_AI_MIN): "clean" up to 35, "ai" above 65.
pub const CLEAN_MAX: f64 = 35.0;
pub const AI_MIN: f64 = 65.0;
/// How long a queued request is waited for, and how often its ticket is asked about.
pub const QUEUE_WAIT_MS: u32 = 15 * 60 * 1000;
pub const POLL_MS: u32 = 1000;
/// How often a busy SlopTotal (429) is asked again, and the longest pause it may ask for.
pub const BUSY_RETRIES: u32 = 5;
pub const BUSY_PAUSE_CAP_S: u32 = 30;
/// The reasons of detectors that did not run (SlopTotal's own wording).
pub const FAILED_PREFIXES: [&str; 2] = ["Model loading failed:", "Engine error:"];

/// An answer: its status (0 if nothing answered) and its text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

#[allow(async_fn_in_trait)]
pub trait Env {
    async fn get(&self, path: &str) -> Reply;
    async fn post(&self, path: &str, body: String) -> Reply;
    async fn sleep(&self, ms: u32);
    /// Whether the check was cancelled (the panes' Cancel): no request is sent after it, a queued one is no longer
    /// asked about (its ticket is dropped), and the flows end with `CANCELLED`.
    fn cancelled(&self) -> bool {
        false
    }
    /// How many paragraph requests to send at once now (the helper decides it from the device and the computer's
    /// load; 1 to QUICK_LANE).
    fn parallel(&self) -> usize {
        1
    }
}

/// The error a cancelled check ends with (the caller says nothing more: Cancel said it).
pub const CANCELLED: &str = "cancelled";

/// The only body ever sent: the text, nothing else.
pub fn body(text: &str) -> String {
    json!({ "text": text }).to_string()
}

fn short(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A name SlopTotal gives a detector (`classifier_e5`): letters, digits and `_`, at most 40.
fn engine_name(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    (!s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')).then(|| s.to_string())
}

fn finite(v: &Value, lo: f64, hi: f64) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite() && *x >= lo && *x <= hi)
}

/// SlopTotal's own words for an error, if its answer has them (`error`, else FastAPI's `detail`).
fn said(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let e = v.get("error").or_else(|| v.get("detail"))?;
    e.as_str().map(|s| short(s, 300))
}

/// What went wrong, from an answer that is not a 200, and what to do.
pub fn error(r: &Reply) -> String {
    let words = said(&r.body).map(|s| format!(": {s}")).unwrap_or_default();
    match r.status {
        0 => format!("SlopTotal is not answering{}. Check Settings → Authenticity → Writing checks (SlopTotal folder), then try again", if r.body.is_empty() { String::new() } else { format!(" ({})", short(&r.body, 200)) }),
        400 => format!("SlopTotal refused the text (400{words})"),
        404 => format!("SlopTotal did not know the request (404{words}); is the program on the port SlopTotal?"),
        429 => format!("SlopTotal is busy (429{words}); try again in a minute"),
        500..=599 => format!("SlopTotal had a problem ({}{words}); its window or log says more", r.status),
        s => format!("SlopTotal answered {s}{words}"),
    }
}

// ----- health

/// The detectors as `/health` reports them. `loaded` counts the detectors that have run since SlopTotal started, not
/// those in memory (seen 2026-10-29: 11 of 23 for fifteen minutes after "Model preloading complete — 23 engines
/// ready"), so readiness is "healthy" with no detector failed, never loaded == total.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub healthy: bool,
    pub loaded: u32,
    pub total: u32,
    pub failed: Vec<String>,
}

impl Health {
    /// Whether SlopTotal says it is healthy, knows its detectors, and none failed to load.
    pub fn ready(&self) -> bool {
        self.healthy && self.total > 0 && self.failed.is_empty()
    }
}

pub fn parse_health(body: &str) -> Result<Health, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "the program on SlopTotal's port did not answer as SlopTotal does (no JSON from /health)".to_string())?;
    let e = v.get("engines").filter(|e| e.is_object()).ok_or("the program on SlopTotal's port did not answer as SlopTotal does (no engines in /health)")?;
    let n = |k: &str| e.get(k).and_then(Value::as_u64).map(|x| x.min(1000) as u32);
    let failed: Vec<String> = e.get("failed").and_then(Value::as_array).map(|a| a.iter().take(50).map(|x| engine_name(x).unwrap_or_else(|| "(unnamed)".into())).collect()).unwrap_or_default();
    let healthy = v["status"].as_str() == Some("healthy");
    Ok(Health { healthy, loaded: n("loaded").unwrap_or(0), total: n("total").ok_or("SlopTotal's /health gave no number of detectors")?, failed })
}

/// Why nothing is scored while some detectors failed to load.
pub fn failed_message(failed: &[String]) -> String {
    format!(
        "SlopTotal could not load {} of its detectors ({}), and a detector that did not load scores 0 in its overall score, so nothing is scored. Start SlopTotal by hand once (the installation guide, step 4) and check its window for the reason",
        failed.len(),
        failed.join(", ")
    )
}

/// SlopTotal's health, failing closed: an error if it does not answer as SlopTotal or a detector failed to load.
pub async fn health<E: Env>(env: &E) -> Result<Health, String> {
    let r = env.get("/health").await;
    if r.status != 200 {
        return Err(error(&r));
    }
    let h = parse_health(&r.body)?;
    if !h.failed.is_empty() {
        return Err(failed_message(&h.failed));
    }
    Ok(h)
}

// ----- scores

/// A score of one text, 0 (reads as human) to 100 (reads as AI).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Score {
    pub score: f64,
    /// "clean", "mixed" or "ai" (SlopTotal's quick verdict).
    pub verdict: String,
    /// SlopTotal's confidence ("low", "medium", "high"), if given.
    pub confidence: String,
    /// The quick detectors' own scores (0-100), when given.
    pub engines: Vec<(String, f64)>,
    /// Quick detectors that failed on this text (SlopTotal counts each as undecided, 50), and whether the score was
    /// worked out again without them (`Weights::quick_without_failed`); if not, it is SlopTotal's, shown marked.
    pub failed: Vec<String>,
    pub rescored: bool,
    /// Who scored it: blank for SlopTotal, "gptzero" for GPTZero (gptzero.rs), with GPTZero's class ("human", "ai",
    /// "mixed"), its result message and its sentences.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub engine: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub class: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sentences: Vec<crate::gptzero::Sentence>,
}

/// The verdict from a score, by SlopTotal's quick bands (the answer's own verdict is checked against it).
pub fn verdict_of(score: f64) -> &'static str {
    if score <= CLEAN_MAX {
        "clean"
    } else if score <= AI_MIN {
        "mixed"
    } else {
        "ai"
    }
}

fn verdict(v: &Value, score: f64) -> String {
    match v.as_str() {
        Some(s @ ("clean" | "mixed" | "ai")) => s.to_string(),
        _ => verdict_of(score).to_string(),
    }
}

fn confidence(v: &Value) -> String {
    match v.as_str() {
        Some(s) if s.len() <= 20 && s.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'-') => s.to_string(),
        _ => String::new(),
    }
}

/// A quick score from quick-score's answer. A detector in error is counted by SlopTotal as undecided (50, not 0), so
/// the score stands on the others: it is kept, with the failed detectors listed for a caveat.
pub fn parse_quick(v: &Value) -> Result<Score, String> {
    let score = finite(&v["score"], 0.0, 100.0).ok_or("SlopTotal's answer had no score in it; nothing was scored")?;
    let mut engines = Vec::new();
    let mut broken = Vec::new();
    for e in v["engines"].as_array().into_iter().flatten().take(50) {
        let name = engine_name(&e["engine"]).unwrap_or_else(|| "(unnamed)".into());
        if e["verdict"].as_str() == Some("error") {
            broken.push(name);
            continue;
        }
        if let Some(s) = finite(&e["score"], 0.0, 100.0) {
            engines.push((name, s));
        }
    }
    Ok(Score { score, verdict: verdict(&v["verdict"], score), confidence: confidence(&v["confidence"]), engines, failed: broken, ..Score::default() })
}

/// What to say of a quick score some of whose detectors failed (empty if none did).
pub fn quick_caveat(s: &Score) -> String {
    let did = match s.failed.len() {
        0 => return String::new(),
        1 => format!("1 quick detector failed on this text ({})", s.failed[0]),
        n => format!("{n} quick detectors failed on this text ({})", s.failed.join(", ")),
    };
    if s.rescored {
        format!("{did}; the score is the weighted average of the others, by SlopTotal's weights")
    } else {
        format!("{did}; SlopTotal counted it as undecided (50), and the score could not be worked out without it")
    }
}

/// One detector's finding from analyze: its name, score (0-1), verdict and one-line reason.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Reason {
    pub engine: String,
    pub score: f64,
    /// "clean", "suspicious" or "slop".
    pub verdict: String,
    pub details: String,
    pub description: String,
}

/// analyze's answer: the overall score (0-100) and each detector's reason.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Analysis {
    pub overall: f64,
    pub verdict: String,
    pub flagged: u32,
    pub total: u32,
    /// "supported", "experimental" or "unsupported": how far SlopTotal trusts its score in the text's language.
    pub language_support: String,
    /// The text's language as SlopTotal detected it ("en", "de" ..., or "other").
    pub language: String,
    /// The detectors that ran, with their reasons.
    pub reasons: Vec<Reason>,
    /// The detectors that did not run on this text, with SlopTotal's words for why. SlopTotal's own overall score
    /// counts each as 0 (most human), which misleads, so it is not shown then.
    pub failed: Vec<Failed>,
    /// The score shown (`settle`): SlopTotal's overall score when every detector ran; with detectors failed, the
    /// weighted average over those that ran (`Weights::without_failed`), or None with the reason in `no_score`.
    pub score: Option<f64>,
    pub no_score: String,
}

/// A detector that did not run: its name and SlopTotal's reason.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Failed {
    pub engine: String,
    pub details: String,
}

/// How much each detector counts in SlopTotal's overall score: the weights of its fitted logistic model
/// (`app/calibration.json` in SlopTotal's folder, read-only: "english", "multilingual" and "quick", each with
/// `weights` by detector key), fitted by SlopTotal's author on their own benchmark (SlopBench). A weight reflects a
/// detector's accuracy and its overlap with the others, not its trustworthiness alone; a detector of weight 0 does not
/// count for that language at all, however high its own score. With every detector run, the score shown is SlopTotal's;
/// with detectors failed, its weights renormalised over those that ran.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Weights {
    pub english: std::collections::HashMap<String, f64>,
    pub multilingual: std::collections::HashMap<String, f64>,
    pub quick: std::collections::HashMap<String, f64>,
    /// Detector display names (as analyze gives them) to keys (as the weights name them), from `/api/engines`.
    pub names: std::collections::HashMap<String, String>,
    /// The rest of the full analysis's model (calibration.json), to work the overall score out again with failed
    /// detectors left out; None when a section is not in the format SlopTotal's calibration.py reads.
    pub english_model: Option<EnglishModel>,
    pub multilingual_model: Option<MultilingualModel>,
    /// The quick score's model ("quick": the same form as "english").
    pub quick_model: Option<EnglishModel>,
}

/// The least share of the language's total weight the detectors that ran must carry for a score: below it, too much
/// of what SlopTotal's model weighs is missing for the rest to stand for it, and no score is shown.
pub const MIN_RAN: f64 = 0.5;

/// calibration.json's "english" section besides the weights (SlopTotal's `calibrated_score` for English).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnglishModel {
    pub intercept: f64,
    pub length_k: f64,
    pub length_neutral: f64,
    /// (log-odds, score) knots, by rising log-odds.
    pub knots: Vec<(f64, f64)>,
}

/// calibration.json's "multilingual" section besides the weights.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MultilingualModel {
    pub intercept: f64,
    pub default_offset: f64,
    pub offsets: std::collections::HashMap<String, f64>,
    pub short_alpha: f64,
    pub short_ref_words: f64,
}

fn english_model(v: &Value) -> Option<EnglishModel> {
    let f = |k: &str| finite(&v[k], -1000.0, 1000.0);
    let knots: Vec<(f64, f64)> = v["band_knots"].as_array()?.iter().take(50).map(|k| Some((finite(&k[0], -1000.0, 1000.0)?, finite(&k[1], 0.0, 100.0)?))).collect::<Option<_>>()?;
    let rising = knots.len() >= 2 && knots.windows(2).all(|w| w[0].0 < w[1].0);
    rising.then_some(())?;
    Some(EnglishModel { intercept: f("intercept")?, length_k: finite(&v["length_k"], 0.0, 1000.0)?, length_neutral: f("length_neutral")?, knots })
}

fn multilingual_model(v: &Value) -> Option<MultilingualModel> {
    let f = |k: &str| finite(&v[k], -1000.0, 1000.0);
    let offsets = v["languages"].as_object()?.iter().take(200).filter_map(|(l, x)| Some((l.chars().take(8).collect(), finite(&x["offset"], -1000.0, 1000.0)?))).collect();
    Some(MultilingualModel { intercept: f("intercept")?, default_offset: f("default_offset")?, offsets, short_alpha: f("short_alpha")?, short_ref_words: finite(&v["short_ref_words"], 0.0, 100_000.0)? })
}

/// SlopTotal's `_logit`: the log-odds of a score, clamped to [0.01, 0.99].
fn logit(p: f64) -> f64 {
    let p = p.clamp(0.01, 0.99);
    (p / (1.0 - p)).ln()
}

/// SlopTotal's `_interpolate`: log-odds to 0-100 along the knots.
fn interpolate(z: f64, knots: &[(f64, f64)]) -> f64 {
    if z <= knots[0].0 {
        return knots[0].1;
    }
    for w in knots.windows(2) {
        let ((z0, s0), (z1, s1)) = (w[0], w[1]);
        if z <= z1 {
            return s0 + (s1 - s0) * (z - z0) / (z1 - z0);
        }
    }
    knots[knots.len() - 1].1
}

/// Letters of the scripts written without spaces between words, which SlopTotal counts as words by characters.
fn unspaced(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF | 0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0x3000..=0x303F | 0xFF00..=0xFFEF)
}

fn weights_of(v: &Value) -> std::collections::HashMap<String, f64> {
    v["weights"].as_object().map(|o| o.iter().take(100).filter_map(|(k, w)| Some((engine_name(&json!(k))?, finite(w, -100.0, 100.0)?))).collect()).unwrap_or_default()
}

/// The weights from calibration.json; a section in another format is left empty (the reasons are then ordered by
/// score and said to be unweighted), never guessed.
pub fn parse_calibration(v: &Value) -> Weights {
    Weights {
        english: weights_of(&v["english"]),
        multilingual: weights_of(&v["multilingual"]),
        quick: weights_of(&v["quick"]),
        names: Default::default(),
        english_model: english_model(&v["english"]),
        multilingual_model: multilingual_model(&v["multilingual"]),
        quick_model: english_model(&v["quick"]),
    }
}

/// Detector names to keys, from `/api/engines` (a list of `{key, name}`).
pub fn parse_engines(v: &Value) -> std::collections::HashMap<String, String> {
    v.as_array().into_iter().flatten().take(100).filter_map(|e| Some((e["name"].as_str()?.chars().take(60).collect(), engine_name(&e["key"])?))).collect()
}

impl Weights {
    /// The weights for a text's language ("en" is English; any other is the multilingual model's).
    pub fn for_language(&self, lang: &str) -> &std::collections::HashMap<String, f64> {
        if lang == "en" || lang.is_empty() { &self.english } else { &self.multilingual }
    }

    /// An analysis's reasons for showing: those that count for the text's language, by weight (highest first), with
    /// it; then those of weight 0 ("not counted for this language"). `None` when the weights are unknown.
    #[allow(clippy::type_complexity)]
    pub fn order<'a>(&self, a: &'a Analysis) -> Option<(Vec<(&'a Reason, f64)>, Vec<&'a Reason>)> {
        let w = self.for_language(&a.language);
        if w.is_empty() {
            return None;
        }
        let (mut counted, mut uncounted) = (Vec::new(), Vec::new());
        for r in &a.reasons {
            let key = self.names.get(&r.engine).cloned().unwrap_or_else(|| r.engine.clone());
            match w.get(&key) {
                Some(&x) if x != 0.0 => counted.push((r, x)),
                _ => uncounted.push(r),
            }
        }
        counted.sort_by(|x, y| y.1.total_cmp(&x.1));
        uncounted.sort_by(|x, y| y.score.total_cmp(&x.score));
        Some((counted, uncounted))
    }

    /// A detector's key (as the weights name it) from the name analyze gave it.
    fn key(&self, name: &str) -> String {
        self.names.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    /// SlopTotal's `calibrated_score` (app/calibration.py) from detector scores (0-1) by key, for a text of `words`
    /// words in `lang`; a detector missing from `scores` counts as undecided (0.5), as there. With `left_out`, the
    /// weights renormalised over the others (`weighted`). Rounded to 0.1; with the share of the weight that ran.
    fn calibrated(&self, lang: &str, scores: &HashMap<String, f64>, words: usize, left_out: &[String]) -> Option<(f64, f64)> {
        let w = self.for_language(lang);
        if w.is_empty() {
            return None;
        }
        let (sum, share) = weighted(w, scores, left_out);
        let score = if lang == "en" || lang.is_empty() {
            band(self.english_model.as_ref()?, sum, words)
        } else {
            let m = self.multilingual_model.as_ref()?;
            let words = words as f64;
            let mut z = m.intercept + sum + m.offsets.get(lang).copied().unwrap_or(m.default_offset);
            if words > 0.0 && words < m.short_ref_words {
                z -= m.short_alpha * (1.0 / words - 1.0 / m.short_ref_words);
            }
            100.0 / (1.0 + (-z).exp())
        };
        Some(((score * 10.0).round() / 10.0, share))
    }

    /// SlopTotal's `quick_score` likewise.
    fn quick_calibrated(&self, scores: &HashMap<String, f64>, words: usize, left_out: &[String]) -> Option<(f64, f64)> {
        if self.quick.is_empty() {
            return None;
        }
        let (sum, share) = weighted(&self.quick, scores, left_out);
        Some(((band(self.quick_model.as_ref()?, sum, words) * 10.0).round() / 10.0, share))
    }

    /// With detectors failed, the score over those that ran: SlopTotal's model with its weights renormalised over them.
    /// Only when the same model, with the failed detectors counted as SlopTotal counted them (`as_counted`), gives
    /// SlopTotal's own number (within a point: the detectors' scores come rounded), never for text in a script written
    /// without spaces between words (SlopTotal counts its words otherwise), and only when those that ran carry
    /// `MIN_RAN` of the weight.
    #[allow(clippy::too_many_arguments)]
    fn renormalised(&self, quick: bool, lang: &str, ran: &HashMap<String, f64>, failed: &[String], as_counted: f64, theirs: f64, text: &str) -> Result<f64, Why> {
        let unknown = || Why::Unknown("SlopTotal's score counts them as if they had found the text human, and it could not be worked out again without them".into());
        if text.chars().any(unspaced) {
            return Err(unknown());
        }
        let words = text.split_whitespace().count();
        let mut counted = ran.clone();
        for f in failed {
            counted.insert(f.clone(), as_counted);
        }
        let model = |scores: &HashMap<String, f64>, left_out: &[String]| if quick { self.quick_calibrated(scores, words, left_out) } else { self.calibrated(lang, scores, words, left_out) };
        let (check, _) = model(&counted, &[]).ok_or_else(unknown)?;
        if (check - theirs).abs() > 1.0 {
            return Err(unknown());
        }
        let (x, share) = model(ran, failed).ok_or_else(unknown)?;
        if share < MIN_RAN {
            return Err(Why::TooFew(format!("{TOO_FEW} for a meaningful score: those that ran carry {:.0}% of SlopTotal's weight for this text (at least {:.0}% is needed)", share * 100.0, MIN_RAN * 100.0)));
        }
        Ok(x)
    }

    /// The full analysis's score without its failed detectors (`renormalised`): SlopTotal counted them as 0.
    pub fn without_failed(&self, a: &Analysis, text: &str) -> Result<f64, Why> {
        if a.failed.is_empty() {
            return Ok(a.overall);
        }
        let ran: HashMap<String, f64> = a.reasons.iter().map(|r| (self.key(&r.engine), r.score)).collect();
        let failed: Vec<String> = a.failed.iter().map(|f| self.key(&f.engine)).collect();
        self.renormalised(false, &a.language, &ran, &failed, 0.0, a.overall, text)
    }

    /// The quick score without its failed detectors: SlopTotal counted them as 0.5.
    pub fn quick_without_failed(&self, s: &Score, text: &str) -> Result<f64, Why> {
        if s.failed.is_empty() {
            return Ok(s.score);
        }
        let ran: HashMap<String, f64> = s.engines.iter().map(|(k, x)| (k.clone(), x / 100.0)).collect();
        self.renormalised(true, "en", &ran, &s.failed, 0.5, s.score, text)
    }
}

/// How the reason for no score when too few detectors ran begins.
pub const TOO_FEW: &str = "too few detectors ran";

/// Why no score without the failed detectors: too few ran, or SlopTotal's model could not be reproduced.
#[derive(Clone, Debug, PartialEq)]
pub enum Why {
    TooFew(String),
    Unknown(String),
}

impl Why {
    pub fn text(&self) -> &str {
        match self {
            Why::TooFew(s) | Why::Unknown(s) => s,
        }
    }
}

/// The sum of weight times log-odds over the detectors of `w` (by key); a detector missing from `scores` counts as
/// 0.5. With `left_out`, those are left out and every other weight is scaled by total / ran (positive weights), so the
/// weights still add up to SlopTotal's total; returns the sum and ran / total.
fn weighted(w: &HashMap<String, f64>, scores: &HashMap<String, f64>, left_out: &[String]) -> (f64, f64) {
    let total: f64 = w.values().filter(|x| **x > 0.0).sum();
    let ran: f64 = w.iter().filter(|(k, x)| **x > 0.0 && !left_out.contains(k)).map(|(_, x)| *x).sum();
    let scale = if left_out.is_empty() || ran <= 0.0 { 1.0 } else { total / ran };
    let sum = w.iter().filter(|(k, _)| !left_out.contains(k)).map(|(k, x)| x * scale * logit(scores.get(k).copied().unwrap_or(0.5))).sum();
    (sum, if total > 0.0 { ran / total } else { 0.0 })
}

/// The English and quick models' last steps: short texts pulled toward a neutral log-odds, then the band's knots.
fn band(m: &EnglishModel, sum: f64, words: usize) -> f64 {
    let words = words as f64;
    let r = if m.length_k != 0.0 { words / (words + m.length_k) } else { 1.0 };
    interpolate(r * (m.intercept + sum) + (1.0 - r) * m.length_neutral, &m.knots)
}

/// The score an analysis shows (`Analysis::score`, or why none in `no_score`).
pub fn settle(a: &mut Analysis, weights: Option<&Weights>, text: &str) {
    let r = match weights {
        _ if a.failed.is_empty() => Ok(a.overall),
        Some(w) => w.without_failed(a, text).map_err(|e| e.text().to_string()),
        None => Err("SlopTotal's weights could not be read, so the score could not be worked out without them".to_string()),
    };
    (a.score, a.no_score) = match r {
        Ok(x) => (Some(x), String::new()),
        Err(e) => (None, e),
    };
}

/// What the full analysis's score is, in words: the score, and with detectors failed, which did not run and what the
/// score is then (or why there is none).
pub fn overall_line(a: &Analysis) -> String {
    if a.failed.is_empty() {
        return format!("Overall: {:.0} of 100.", a.score.unwrap_or(a.overall));
    }
    let names: Vec<&str> = a.failed.iter().map(|f| f.engine.as_str()).collect();
    let did = if names.len() == 1 { format!("1 detector did not run ({})", names[0]) } else { format!("{} detectors did not run ({})", names.len(), names.join(", ")) };
    match a.score {
        Some(x) => format!("{did}. The score is the weighted average of the {} that ran, by SlopTotal's weights: {x:.0} of 100.", a.reasons.len()),
        None if a.no_score.is_empty() => format!("{did}. No score is shown."),
        None => format!("{did}. No score is shown: {}.", a.no_score),
    }
}

pub fn parse_analysis(v: &Value) -> Result<Analysis, String> {
    let list = v["engine_results"].as_array().ok_or("SlopTotal's analysis had no detector results in it")?;
    let mut reasons = Vec::new();
    let mut broken = Vec::new();
    for e in list.iter().take(60) {
        let name = e["engine_name"].as_str().map(|s| short(s, 60)).unwrap_or_else(|| "(unnamed)".into());
        let details = e["details"].as_str().map(|s| short(s, 600)).unwrap_or_default();
        if FAILED_PREFIXES.iter().any(|p| details.starts_with(p)) {
            broken.push(Failed { engine: name, details: short(&details, 200) });
            continue;
        }
        let Some(score) = finite(&e["score"], 0.0, 1.0) else { continue };
        let verdict = match e["verdict"].as_str() {
            Some(s @ ("clean" | "suspicious" | "slop")) => s.to_string(),
            _ => String::new(),
        };
        reasons.push(Reason { engine: name, score, verdict, details, description: e["description"].as_str().map(|s| short(s, 400)).unwrap_or_default() });
    }
    if reasons.is_empty() {
        let names: Vec<&str> = broken.iter().map(|f| f.engine.as_str()).collect();
        return Err(format!("none of SlopTotal's detectors ran on this text ({} failed: {})", broken.len(), names.join(", ")));
    }
    let overall = finite(&v["overall_score"], 0.0, 100.0).ok_or("SlopTotal's analysis had no overall score in it")?;
    let support = match v["language_support"].as_str() {
        Some(s @ ("supported" | "experimental" | "unsupported")) => s.to_string(),
        _ => String::new(),
    };
    Ok(Analysis {
        overall,
        verdict: v["overall_verdict"].as_str().map(|s| short(s, 60)).unwrap_or_default(),
        flagged: v["engines_flagged"].as_u64().unwrap_or(0).min(1000) as u32,
        total: v["engines_total"].as_u64().unwrap_or(reasons.len() as u64).min(1000) as u32,
        language_support: support,
        language: v["language"].as_str().filter(|l| l.len() <= 8 && l.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')).unwrap_or_default().to_string(),
        reasons,
        failed: broken,
        score: None,
        no_score: String::new(),
    })
}

// ----- requests

/// A ticket id as SlopTotal makes them, before it goes into a path: letters, digits, `-` and `_`, at most 64.
fn ticket(v: &Value) -> Option<String> {
    let t = v["ticket_id"].as_str()?;
    (!t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).then(|| t.to_string())
}

/// A POST to an endpoint with a queue (quick-score, analyze): the result, waiting on a ticket (202) or a busy
/// SlopTotal (429) as it asks.
pub async fn queued<E: Env>(env: &E, path: &str, text: &str) -> Result<Value, String> {
    let mut busy = 0;
    loop {
        if env.cancelled() {
            return Err(CANCELLED.into());
        }
        let r = env.post(path, body(text)).await;
        match r.status {
            200 => return read(&r),
            202 => {
                let v = read(&r)?;
                let t = ticket(&v).ok_or("SlopTotal queued the request without a ticket to ask about")?;
                return wait_ticket(env, &t).await;
            }
            429 if busy < BUSY_RETRIES => {
                busy += 1;
                env.sleep(pause(&r)).await;
            }
            _ => return Err(error(&r)),
        }
    }
}

fn read(r: &Reply) -> Result<Value, String> {
    let v: Value = serde_json::from_str(&r.body).map_err(|_| "SlopTotal's answer could not be read (not JSON); nothing was scored".to_string())?;
    // a queued request that failed comes back as its result: {"error": ...}
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Err(format!("SlopTotal could not score the text: {}", short(e, 300)));
    }
    Ok(v)
}

/// The pause a 429 asks for, in ms (its `retry_after`, 1 to BUSY_PAUSE_CAP_S seconds).
fn pause(r: &Reply) -> u32 {
    let s = serde_json::from_str::<Value>(&r.body).ok().and_then(|v| v["retry_after"].as_u64()).unwrap_or(2);
    (s.clamp(1, BUSY_PAUSE_CAP_S as u64) as u32) * 1000
}

async fn wait_ticket<E: Env>(env: &E, t: &str) -> Result<Value, String> {
    let path = format!("/api/queue/ticket/{t}");
    let mut waited = 0;
    loop {
        env.sleep(POLL_MS).await;
        waited += POLL_MS;
        if env.cancelled() {
            return Err(CANCELLED.into());
        }
        let r = env.get(&path).await;
        match r.status {
            200 => return read(&r),
            202 if waited < QUEUE_WAIT_MS => {}
            202 => return Err(format!("SlopTotal had not scored the text after {} minutes; try a shorter selection", QUEUE_WAIT_MS / 60_000)),
            404 => return Err("SlopTotal no longer has the queued request (it expired); try again".into()),
            _ => return Err(error(&r)),
        }
    }
}

/// One text's quick score (at least MIN_CHARS characters). With quick detectors failed, the weighted average of the
/// others (`weights`: SlopTotal's calibration, when read), else SlopTotal's own (marked); an error when too few ran.
pub async fn quick<E: Env>(env: &E, text: &str, weights: Option<&Weights>) -> Result<Score, String> {
    let text = text.trim();
    if text.chars().count() < MIN_CHARS {
        return Err(format!("too short to score: SlopTotal needs at least {MIN_CHARS} characters"));
    }
    let mut s = parse_quick(&queued(env, "/api/quick-score", text).await?)?;
    if !s.failed.is_empty() {
        if let Some(w) = weights {
            match w.quick_without_failed(&s, text) {
                Ok(x) => {
                    s.score = x;
                    s.verdict = verdict_of(x).into();
                    s.rescored = true;
                }
                Err(Why::TooFew(e)) => return Err(e),
                Err(Why::Unknown(_)) => {}
            }
        }
    }
    Ok(s)
}

/// Every detector's reason for one text.
pub async fn analyze<E: Env>(env: &E, text: &str) -> Result<Analysis, String> {
    let text = text.trim();
    if text.chars().count() < MIN_CHARS {
        return Err(format!("too short to analyse: SlopTotal needs at least {MIN_CHARS} characters"));
    }
    parse_analysis(&queued(env, "/api/analyze", text).await?)
}

/// Scores for paragraphs (each a line of prose, without blank lines inside), one quick-score request each, `parallel`
/// at a time: None for one too short to score (under MIN_CHARS characters) or whose detectors that ran were too few for
/// a score. A paragraph under SHORT_WORDS words is scored, and shown as less reliable.
pub async fn paragraphs<E: Env>(env: &E, paras: &[&str], weights: Option<&Weights>) -> Result<Vec<Option<Score>>, String> {
    // white space made single, so a line break inside a paragraph never splits it at SlopTotal
    let owned: Vec<String> = paras.iter().map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    let mut out: Vec<Option<Score>> = vec![None; owned.len()];
    let scorable: Vec<usize> = (0..owned.len()).filter(|&i| owned[i].chars().count() >= MIN_CHARS).collect();
    let mut at = 0;
    while at < scorable.len() {
        if env.cancelled() {
            return Err(CANCELLED.into());
        }
        let wave = &scorable[at..(at + env.parallel().clamp(1, QUICK_LANE)).min(scorable.len())];
        let got = join_all(wave.iter().map(|&i| quick(env, &owned[i], weights)).collect()).await;
        for (&i, r) in wave.iter().zip(got) {
            out[i] = match r {
                Ok(s) => Some(s),
                Err(e) if e.starts_with(TOO_FEW) => None,
                Err(e) => return Err(e),
            };
        }
        at += wave.len();
    }
    Ok(out)
}

/// The futures run together, their outputs in order (one combinator, so no dependency for it).
async fn join_all<F: std::future::Future>(fs: Vec<F>) -> Vec<F::Output> {
    use std::task::Poll;
    let mut fs: Vec<std::pin::Pin<Box<F>>> = fs.into_iter().map(Box::pin).collect();
    let mut out: Vec<Option<F::Output>> = fs.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut waiting = false;
        for (f, o) in fs.iter_mut().zip(out.iter_mut()) {
            if o.is_none() {
                match f.as_mut().poll(cx) {
                    Poll::Ready(v) => *o = Some(v),
                    Poll::Pending => waiting = true,
                }
            }
        }
        if waiting { Poll::Pending } else { Poll::Ready(()) }
    })
    .await;
    out.into_iter().map(|o| o.expect("every future finished")).collect()
}

/// Runs a future that never waits (a blocking `Env`, as the MCP server's): polled until done, with no executor.
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn raw() -> RawWaker {
        fn clone(_: *const ()) -> RawWaker {
            raw()
        }
        fn nop(_: *const ()) {}
        static VT: RawWakerVTable = RawWakerVTable::new(clone, nop, nop, nop);
        RawWaker::new(std::ptr::null(), &VT)
    }
    // SAFETY: the waker's functions do nothing and use no data.
    let waker = unsafe { Waker::from_raw(raw()) };
    let mut f = Box::pin(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(&waker)) {
            return v;
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// A fake SlopTotal: canned replies in order, every request recorded (method, path, body), pauses added up.
    #[derive(Default)]
    pub struct Fake {
        pub replies: RefCell<VecDeque<Reply>>,
        pub sent: RefCell<Vec<(String, String, String)>>,
        pub slept: RefCell<u32>,
        /// Cancelled once this many requests were sent.
        pub cancel_after: Option<usize>,
        /// Paragraph requests at once (0: one).
        pub parallel: usize,
    }

    impl Fake {
        pub fn new(replies: Vec<Reply>) -> Fake {
            Fake { replies: RefCell::new(replies.into()), ..Fake::default() }
        }
        fn next(&self) -> Reply {
            self.replies.borrow_mut().pop_front().expect("an unexpected request")
        }
    }

    impl Env for Fake {
        async fn get(&self, path: &str) -> Reply {
            self.sent.borrow_mut().push(("GET".into(), path.into(), String::new()));
            self.next()
        }
        async fn post(&self, path: &str, body: String) -> Reply {
            self.sent.borrow_mut().push(("POST".into(), path.into(), body));
            self.next()
        }
        async fn sleep(&self, ms: u32) {
            *self.slept.borrow_mut() += ms;
        }
        fn cancelled(&self) -> bool {
            self.cancel_after.is_some_and(|n| self.sent.borrow().len() >= n)
        }
        fn parallel(&self) -> usize {
            self.parallel.max(1)
        }
    }

    pub fn ok(v: Value) -> Reply {
        Reply { status: 200, body: v.to_string() }
    }

    fn status(s: u16, v: Value) -> Reply {
        Reply { status: s, body: v.to_string() }
    }

    const P1: &str = "The first paragraph is long enough for SlopTotal to score it on its own merits.";
    const P2: &str = "A second paragraph follows, also long enough to be scored by the quick detectors.";

    fn quick_answer(score: f64) -> Value {
        json!({ "score": score, "verdict": verdict_of(score), "confidence": "medium", "engines": [{ "engine": "classifier_e5", "score": 71.5, "verdict": "slop" }], "elapsed_ms": 3.0 })
    }

    #[test]
    fn health_fails_closed_on_a_failed_detector() {
        let h = |v: Value| block_on(health(&Fake::new(vec![ok(v)])));
        // ready while only some detectors have run yet (loaded counts those that ran)
        let partly = h(json!({ "status": "healthy", "engines": { "total": 23, "loaded": 11, "failed": [] } })).unwrap();
        assert_eq!(partly, Health { healthy: true, loaded: 11, total: 23, failed: vec![] });
        assert!(partly.ready());
        assert!(!h(json!({ "status": "degraded", "engines": { "total": 23, "loaded": 23, "failed": [] } })).unwrap().ready());
        let e = h(json!({ "engines": { "total": 23, "loaded": 22, "failed": ["classifier_tmr"] } })).unwrap_err();
        assert!(e.contains("classifier_tmr") && e.contains("nothing is scored"), "{e}");
        assert!(h(json!({ "hello": "not sloptotal" })).unwrap_err().contains("did not answer as SlopTotal"));
        assert!(block_on(health(&Fake::new(vec![Reply { status: 0, body: "connect ECONNREFUSED".into() }]))).unwrap_err().contains("not answering"));
        assert!(!Health { healthy: true, loaded: 10, total: 0, failed: vec![] }.ready());
    }

    #[test]
    fn requests_carry_the_text_alone() {
        let fake = Fake::new(vec![ok(quick_answer(80.0))]);
        let s = block_on(quick(&fake, P1, None)).unwrap();
        assert_eq!((s.score, s.verdict.as_str(), s.confidence.as_str()), (80.0, "ai", "medium"));
        assert_eq!(s.engines, vec![("classifier_e5".to_string(), 71.5)]);
        let (method, path, body) = fake.sent.borrow()[0].clone();
        assert_eq!((method.as_str(), path.as_str()), ("POST", "/api/quick-score"));
        let b: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(b, json!({ "text": P1 }), "never a url field");
        // too short: nothing is sent
        let fake = Fake::new(vec![]);
        assert!(block_on(quick(&fake, "Too short.", None)).unwrap_err().contains("at least 50"));
        assert!(fake.sent.borrow().is_empty());
    }

    #[test]
    fn a_quick_score_with_a_detector_in_error_is_kept_with_a_caveat() {
        // SlopTotal counts a quick detector in error as undecided (50), so the score rests on the others
        let v = json!({ "score": 50.0, "verdict": "mixed", "engines": [{ "engine": "classifier_tmr", "score": 50.0, "verdict": "error" }, { "engine": "classifier_e5", "score": 61.0, "verdict": "mixed" }] });
        let s = parse_quick(&v).unwrap();
        assert_eq!((s.failed.as_slice(), s.engines.len()), (["classifier_tmr".to_string()].as_slice(), 1));
        assert!(quick_caveat(&s).contains("classifier_tmr") && quick_caveat(&s).contains("undecided (50)"));
        assert_eq!(quick_caveat(&parse_quick(&json!({ "score": 20.0 })).unwrap()), "");
        assert!(parse_quick(&json!({ "score": 140.0 })).is_err(), "out of range");
        assert!(parse_quick(&json!({ "score": "high" })).is_err());
        // an unknown verdict is taken from the score
        assert_eq!(parse_quick(&json!({ "score": 20.0, "verdict": "<b>" })).unwrap().verdict, "clean");
    }

    #[test]
    fn queued_requests_poll_their_ticket_and_wait_when_busy() {
        let fake = Fake::new(vec![
            status(429, json!({ "error": "Server overloaded", "retry_after": 3 })),
            status(202, json!({ "status": "queued", "ticket_id": "abc-123", "position": 2 })),
            status(202, json!({ "status": "queued", "position": 1 })),
            ok(quick_answer(10.0)),
        ]);
        let s = block_on(quick(&fake, P1, None)).unwrap();
        assert_eq!(s.verdict, "clean");
        let paths: Vec<String> = fake.sent.borrow().iter().map(|s| s.1.clone()).collect();
        assert_eq!(paths, ["/api/quick-score", "/api/quick-score", "/api/queue/ticket/abc-123", "/api/queue/ticket/abc-123"]);
        assert_eq!(*fake.slept.borrow(), 3000 + 2 * POLL_MS);
        // a ticket id that is not one is never put into a path
        let fake = Fake::new(vec![status(202, json!({ "ticket_id": "../../shutdown" }))]);
        assert!(block_on(quick(&fake, P1, None)).unwrap_err().contains("without a ticket"));
        assert_eq!(fake.sent.borrow().len(), 1);
        // an expired ticket, a queued request that failed, and a SlopTotal busy too long
        let fake = Fake::new(vec![status(202, json!({ "ticket_id": "t1" })), status(404, json!({ "detail": "Ticket not found or expired" }))]);
        assert!(block_on(quick(&fake, P1, None)).unwrap_err().contains("expired"));
        let fake = Fake::new(vec![status(202, json!({ "ticket_id": "t1" })), ok(json!({ "error": "boom" }))]);
        assert!(block_on(quick(&fake, P1, None)).unwrap_err().contains("boom"));
        let busy: Vec<Reply> = (0..=BUSY_RETRIES).map(|_| status(429, json!({ "retry_after": 999 }))).collect();
        let fake = Fake::new(busy);
        assert!(block_on(quick(&fake, P1, None)).unwrap_err().contains("busy"));
        assert_eq!(*fake.slept.borrow(), BUSY_RETRIES * BUSY_PAUSE_CAP_S * 1000, "pauses capped");
    }

    #[test]
    fn each_paragraph_goes_to_quick_score_on_its_own() {
        // three paragraphs, one too short: two quick-score requests, each with its paragraph alone, results in order
        let fake = Fake { parallel: 2, ..Fake::new(vec![ok(quick_answer(20.0)), ok(quick_answer(90.0))]) };
        let got = block_on(paragraphs(&fake, &[P1, "Tiny.", P2], None)).unwrap();
        assert_eq!(got.iter().map(|s| s.as_ref().map(|s| s.score)).collect::<Vec<_>>(), [Some(20.0), None, Some(90.0)]);
        let sent = fake.sent.borrow();
        assert_eq!(sent.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(), ["/api/quick-score", "/api/quick-score"]);
        assert_eq!(serde_json::from_str::<Value>(&sent[1].2).unwrap(), json!({ "text": P2 }));
        // a paragraph whose detectors that ran are too few: no score for it, the others scored
        let w = parse_calibration(&serde_json::from_str(CALIBRATION).unwrap());
        // SlopTotal's own number, with fakespot (most of the quick weight) failed and counted as 0.5
        let theirs = w.quick_calibrated(&[("classifier_tmr".to_string(), 0.4), ("classifier_fakespot".to_string(), 0.5)].into_iter().collect(), P1.split_whitespace().count(), &[]).unwrap().0;
        let few = json!({ "score": theirs, "verdict": "clean", "engines": [{ "engine": "classifier_fakespot", "score": 50.0, "verdict": "error" }, { "engine": "classifier_tmr", "score": 40.0, "verdict": "clean" }] });
        let fake = Fake::new(vec![ok(few), ok(quick_answer(70.0))]);
        let got = block_on(paragraphs(&fake, &[P1, P2], Some(&w))).unwrap();
        assert_eq!(got.iter().map(|s| s.as_ref().map(|s| s.score)).collect::<Vec<_>>(), [None, Some(70.0)]);
    }

    #[test]
    fn an_analysis_with_a_detector_that_did_not_run_is_shown_with_it_listed() {
        let good = json!({ "overall_score": 72.0, "overall_verdict": "Likely AI-generated", "engines_flagged": 2, "engines_total": 2, "language_support": "supported",
            "engine_results": [
                { "engine_name": "Burstiness", "score": 0.8, "verdict": "slop", "details": "Sentence lengths vary little (CV 0.12)", "description": "How much sentence length varies" },
                { "engine_name": "Linguistic", "score": 0.7, "verdict": "slop", "details": "3 stock phrases", "description": "" }
            ] });
        let a = parse_analysis(&good).unwrap();
        assert_eq!((a.overall, a.reasons.len(), a.language_support.as_str()), (72.0, 2, "supported"));
        assert_eq!(a.reasons[0].details, "Sentence lengths vary little (CV 0.12)");
        for prefix in FAILED_PREFIXES {
            let mut bad = good.clone();
            bad["engine_results"][1]["details"] = json!(format!("{prefix} OverflowError"));
            let a = parse_analysis(&bad).unwrap();
            assert_eq!(a.reasons.iter().map(|r| r.engine.as_str()).collect::<Vec<_>>(), ["Burstiness"], "the one that ran, with its reason");
            assert_eq!(a.failed, [Failed { engine: "Linguistic".into(), details: format!("{prefix} OverflowError") }], "{prefix}");
            // none ran: nothing to show
            bad["engine_results"][0]["details"] = json!(format!("{prefix} x"));
            assert!(parse_analysis(&bad).unwrap_err().contains("none of"));
        }
        let fake = Fake::new(vec![ok(good)]);
        assert!(block_on(analyze(&fake, P1)).is_ok());
        assert_eq!(fake.sent.borrow()[0].1, "/api/analyze");
    }

    #[test]
    fn reasons_are_ordered_by_slop_totals_own_weights() {
        let cal = json!({ "version": 3, "english": { "intercept": 0.9, "weights": { "structural": 0.41, "classifier_desklib": 0.37, "classifier_e5": 0.0 } },
            "multilingual": { "weights": { "classifier_e5": 0.24 } }, "quick": { "weights": { "classifier_fakespot": 0.57 } } });
        let mut w = parse_calibration(&cal);
        w.names = parse_engines(&json!([{ "key": "structural", "name": "Structural" }, { "key": "classifier_desklib", "name": "Desklib" }, { "key": "classifier_e5", "name": "E5 classifier" }]));
        let r = |name: &str, score: f64| Reason { engine: name.into(), score, verdict: "slop".into(), ..Reason::default() };
        let an = Analysis { language: "en".into(), reasons: vec![r("E5 classifier", 0.93), r("Desklib", 0.6), r("Structural", 0.5)], ..Analysis::default() };
        let (counted, uncounted) = w.order(&an).unwrap();
        assert_eq!(counted.iter().map(|(r, x)| (r.engine.as_str(), *x)).collect::<Vec<_>>(), [("Structural", 0.41), ("Desklib", 0.37)]);
        assert_eq!(uncounted.iter().map(|r| r.engine.as_str()).collect::<Vec<_>>(), ["E5 classifier"], "93% from a detector that does not count");
        // another language: the multilingual weights
        let de = Analysis { language: "de".into(), ..an.clone() };
        assert_eq!(w.order(&de).unwrap().0.len(), 1);
        // a calibration file in another format: no weights, never guessed
        assert!(parse_calibration(&json!({ "english": { "weights": [0.1, 0.2] } })).order(&an).is_none());
    }

    /// SlopTotal's calibration.json (version of 2026-10-29). The expected numbers are its own calibrated_score and
    /// quick_score run in its Python with the weights renormalised over the detectors that ran (each kept weight times
    /// total / ran, the failed ones 0): 36.3 in English with binoculars and cross_perplexity out (SlopTotal's own,
    /// counting them as 0: 32.6); in German they carry no weight (17.7 either way); with classifier_desklib out too,
    /// 37.1 (69% of the weight ran). The quick score with classifier_tmr out: 24.6 (SlopTotal's, counting it as 0.5: 24.7).
    const CALIBRATION: &str = r#"{"english": {"intercept": 0.9571, "weights": {"classifier_bert_raid": 0.0, "binoculars": 0.0951, "burstiness": 0.0074, "classifier_chatgpt": 0.0, "cross_perplexity": 0.0, "classifier_desklib": 0.3686, "diveye": 0.0, "classifier_e5": 0.0, "classifier_fakespot": 0.2621, "fast_detectgpt": 0.0, "formulaic": 0.1121, "gltr": 0.0, "linguistic": 0.0, "log_rank": 0.0, "classifier_openai": 0.0, "perplexity": 0.0, "classifier_remodetect": 0.0, "readability": 0.0991, "sentiment": 0.0, "structural": 0.4081, "classifier_superannotate": 0.1568, "classifier_tmr": 0.0, "vocabulary": 0.0}, "length_k": 5.0, "length_neutral": -3.6, "band_knots": [[-7.0416, 0.0], [-1.0416, 30.0], [0.2894, 45.0], [1.1572, 55.0], [2.1993, 80.0], [6.1993, 100.0]]}, "multilingual": {"short_alpha": 81.9, "short_ref_words": 150, "intercept": 5.5363, "weights": {"classifier_bert_raid": 0.0, "binoculars": 0.0, "burstiness": 0.1096, "classifier_chatgpt": 0.1155, "cross_perplexity": 0.0, "classifier_desklib": 0.0229, "diveye": 0.0, "classifier_e5": 0.2421, "classifier_fakespot": 0.0, "fast_detectgpt": 0.1581, "formulaic": 0.0, "gltr": 0.0, "linguistic": 0.015, "log_rank": 0.0, "classifier_openai": 0.031, "perplexity": 0.0, "classifier_remodetect": 0.9038, "readability": 0.0, "sentiment": 0.0, "structural": 0.2295, "classifier_superannotate": 0.1783, "classifier_tmr": 0.1802, "vocabulary": 0.0}, "default_offset": -0.5385, "languages": {"de": {"offset": -1.5715, "status": "supported"}, "fr": {"offset": -0.7151, "status": "supported"}}}, "quick": {"intercept": -0.5795, "weights": {"classifier_fakespot": 0.5705, "classifier_tmr": 0.0225, "classifier_bert_raid": 0.0, "classifier_e5": 0.0, "linguistic": 0.1145, "formulaic": 0.1132}, "length_k": 30.0, "length_neutral": -1.9, "band_knots": [[-6.776, 0.0], [-0.776, 30.0], [0.3466, 45.0], [0.8174, 55.0], [1.1213, 80.0], [5.1213, 100.0]]}}"#;
    const RAN: [(&str, f64); 14] = [("structural", 0.62), ("classifier_desklib", 0.31), ("classifier_fakespot", 0.18), ("formulaic", 0.4), ("readability", 0.55), ("classifier_superannotate", 0.07), ("burstiness", 0.7), ("classifier_e5", 0.9), ("classifier_remodetect", 0.35), ("fast_detectgpt", 0.5), ("classifier_tmr", 0.2), ("classifier_chatgpt", 0.6), ("linguistic", 0.3), ("classifier_openai", 0.1)];
    const EN: &str = "The results suggest that the method improves accuracy across all of the datasets we examined, although the gains are smaller on the noisier subsets. We discuss possible reasons for this and outline further work.";
    const DE: &str = "Die Ergebnisse zeigen, dass die Methode die Genauigkeit verbessert, auch wenn die Gewinne kleiner sind.";

    fn analysis(language: &str, overall: f64, failed: &[&str]) -> Analysis {
        let reasons = RAN.iter().filter(|(k, _)| !failed.contains(k)).map(|(k, s)| Reason { engine: k.to_string(), score: *s, ..Reason::default() }).collect();
        let failed = failed.iter().map(|e| Failed { engine: e.to_string(), details: "Engine error: OverflowError".into() }).collect();
        Analysis { overall, language: language.into(), reasons, failed, ..Analysis::default() }
    }

    #[test]
    fn failed_detectors_are_left_out_of_the_score() {
        let w = parse_calibration(&serde_json::from_str(CALIBRATION).unwrap());
        assert!(w.english_model.is_some() && w.multilingual_model.is_some() && w.quick_model.is_some());
        const TWO: [&str; 2] = ["binoculars", "cross_perplexity"];
        // SlopTotal counted them as 0 (32.6); left out, its weights renormalised over those that ran: its own Python's 36.3
        assert_eq!(w.without_failed(&analysis("en", 32.6, &TWO), EN), Ok(36.3));
        assert_eq!(w.without_failed(&analysis("de", 17.7, &TWO), DE), Ok(17.7), "no weight in German");
        // classifier_desklib out as well: 69% of the weight ran (SlopTotal's own number, with the three at 0, first)
        let three = ["binoculars", "cross_perplexity", "classifier_desklib"];
        let mut zero: HashMap<String, f64> = RAN.iter().map(|(k, s)| (k.to_string(), *s)).collect();
        for f in three {
            zero.insert(f.into(), 0.0);
        }
        let theirs = w.calibrated("en", &zero, EN.split_whitespace().count(), &[]).unwrap().0;
        assert_eq!(w.without_failed(&analysis("en", theirs, &three), EN), Ok(37.1));
        // too few ran: binoculars, structural, desklib and fakespot out leave 25% of the weight
        let most = ["binoculars", "structural", "classifier_desklib", "classifier_fakespot"];
        let mut zero: HashMap<String, f64> = RAN.iter().map(|(k, s)| (k.to_string(), *s)).collect();
        for f in most {
            zero.insert(f.into(), 0.0);
        }
        let theirs = w.calibrated("en", &zero, EN.split_whitespace().count(), &[]).unwrap().0;
        assert!(matches!(w.without_failed(&analysis("en", theirs, &most), EN), Err(Why::TooFew(e)) if e.contains("25%")));
        // SlopTotal's own number not reproduced, a script without spaces, no model: no score
        assert!(matches!(w.without_failed(&analysis("en", 41.0, &TWO), EN), Err(Why::Unknown(_))));
        assert!(matches!(w.without_failed(&analysis("en", 32.6, &TWO), &format!("{EN} \u{7814}\u{7a76}")), Err(Why::Unknown(_))));
        assert!(parse_calibration(&json!({ "english": { "weights": { "structural": 0.4 } } })).without_failed(&analysis("en", 32.6, &TWO), EN).is_err());
        // nothing failed: SlopTotal's own
        assert_eq!(w.without_failed(&analysis("en", 30.0, &[]), EN), Ok(30.0));
        // in words
        let mut a = analysis("en", 32.6, &TWO);
        settle(&mut a, Some(&w), EN);
        assert_eq!(a.score, Some(36.3));
        assert_eq!(overall_line(&a), "2 detectors did not run (binoculars, cross_perplexity). The score is the weighted average of the 14 that ran, by SlopTotal's weights: 36 of 100.");
        let mut a = analysis("en", theirs, &most);
        settle(&mut a, Some(&w), EN);
        assert_eq!(a.score, None);
        assert!(overall_line(&a).starts_with("4 detectors did not run (binoculars, structural, classifier_desklib, classifier_fakespot). No score is shown: too few detectors ran"), "{}", overall_line(&a));
        let mut a = analysis("en", 32.6, &TWO);
        settle(&mut a, None, EN);
        assert!(a.score.is_none() && a.no_score.contains("weights could not be read"));
        let mut a = analysis("en", 12.0, &[]);
        settle(&mut a, None, EN);
        assert_eq!(overall_line(&a), "Overall: 12 of 100.");
    }

    #[test]
    fn a_quick_score_without_its_failed_detector() {
        let w = parse_calibration(&serde_json::from_str(CALIBRATION).unwrap());
        // SlopTotal's quick score counted classifier_tmr as 0.5 (24.7); left out: 24.6
        let engines = [("classifier_fakespot", 12.3), ("classifier_bert_raid", 70.0), ("classifier_e5", 20.0), ("linguistic", 25.0), ("formulaic", 60.0)];
        let s = Score { score: 24.7, engines: engines.iter().map(|(k, x)| (k.to_string(), *x)).collect(), failed: vec!["classifier_tmr".into()], ..Score::default() };
        assert_eq!(w.quick_without_failed(&s, EN), Ok(24.6));
        // fakespot failed: 30% of the quick weight ran
        let s = Score { score: 30.0, engines: vec![("classifier_tmr".into(), 40.0)], failed: vec!["classifier_fakespot".into()], ..Score::default() };
        let theirs = w.quick_calibrated(&[("classifier_tmr".to_string(), 0.4), ("classifier_fakespot".to_string(), 0.5)].into_iter().collect(), EN.split_whitespace().count(), &[]).unwrap().0;
        assert!(matches!(w.quick_without_failed(&Score { score: theirs, ..s }, EN), Err(Why::TooFew(_))));
    }

    #[test]
    fn a_cancelled_check_sends_nothing_more() {
        // two at a time: cancelled after the first two requests, the third paragraph is not sent
        let fake = Fake { cancel_after: Some(2), parallel: 2, ..Fake::new(vec![ok(quick_answer(20.0)), ok(quick_answer(30.0)), ok(quick_answer(40.0))]) };
        let got = block_on(paragraphs(&fake, &[P1, P2, P1], None));
        assert_eq!(got.unwrap_err(), CANCELLED);
        assert_eq!(fake.sent.borrow().len(), 2, "no request after Cancel");
        // a queued request: its ticket is no longer asked about
        let fake = Fake { cancel_after: Some(1), ..Fake::new(vec![Reply { status: 202, body: json!({ "ticket_id": "t1" }).to_string() }]) };
        assert_eq!(block_on(quick(&fake, P1, None)).unwrap_err(), CANCELLED);
        assert_eq!(fake.sent.borrow().len(), 1);
        // cancelled before anything: nothing sent
        let fake = Fake { cancel_after: Some(0), ..Fake::new(vec![]) };
        assert_eq!(block_on(analyze(&fake, P1)).unwrap_err(), CANCELLED);
        assert!(fake.sent.borrow().is_empty());
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(error(&Reply { status: 400, body: json!({ "error": "Text must be at least 50 characters." }).to_string() }).contains("at least 50"));
        assert!(error(&Reply { status: 500, body: String::new() }).contains("problem"));
        assert!(error(&Reply { status: 0, body: String::new() }).contains("SlopTotal folder"));
    }
}
