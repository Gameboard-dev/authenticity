//! authenticity-mcp: Authenticity's MCP server, for research assistants. It sends chosen text for
//! AI-writing and similarity markup and reads the results, all on this computer:
//!
//! - `check_ai(paragraphs)`: each paragraph's SlopTotal score (0-100), band and colour, with the model-free hints of
//!   its sentences; `explain_ai(text)`: every detector's reason, ordered by SlopTotal's own weights for the language;
//! - `check_similarity(text, mode)`: wording shared with the library notes (verbatim, near-verbatim; fixed phrasing
//!   apart) and close paraphrases (the library index's content-word search), with UTF-16 spans in the text and each
//!   source;
//! - `check_overlap(draft)`: the close-copying check of a draft file (check-overlap's);
//! - `reveal_characters(text)`: invisible and look-alike characters, by fixed rules;
//! - `status`; and, read-only, what the plugin found in notes: `list_notes`, `ai_results`, `similarity_results`,
//!   `unusual_characters`.
//!
//! SlopTotal runs through Authenticity's helper: the one Obsidian started if it answers, else one this server runs in
//! its own process (stopped when this server stops). When the GPU has no room, a check answers `gpu_shortage`: ask the
//! user, then call again with `gpu: "free"` or `"cpu"`. It never rewrites text.
//!
//! `authenticity-mcp --vault <vault> [--config-dir .obsidian] [--sloptotal <folder>] [--sloptotal-port 8000]
//! [--helper-port 8791] [--device auto|gpu|cpu] [--idle-minutes 10]`. JSON-RPC 2.0 on stdin and
//! stdout, one message per line (<https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#stdio>).

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use authenticity_core::{ai, chars, hints, para, similarity, sloptotal};
use serde_json::{json, Value};

use crate::{device, helper, http, overlap, vault::Vault};

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "Authenticity's offline writing checks for the user's own writing, on this computer. \
check_ai scores paragraphs with SlopTotal, on this computer, never an online detector (0 reads as written by a person, 100 as AI; up to 35 'clean', above 65 'ai', \
between 'mixed'); explain_ai gives each detector's reason, ordered by SlopTotal's own weights (detectors of weight 0 do \
not count for the text's language). Scores are unreliable on short text (under about 80 words) and more often wrong on \
non-native English, and single detectors disagree strongly on short text: rely on the paragraph's overall score, never \
on one detector, and state findings as resemblance to model output, not authorship. check_similarity finds wording \
shared with the library notes (runs of 6+ words outside quotation marks; fixed phrasing listed apart) and close \
paraphrases by its content-word search. check_overlap checks a draft file (run it on a chapter before handing it in). \
reveal_characters lists invisible and look-alike characters. Nothing is ever rewritten; suggest no rewrites unless the \
user asks. If a check answers gpu_shortage, ask the user, then call again with gpu 'free' or 'cpu'. The first AI check \
starts SlopTotal (about 9 GB of memory; tens of seconds).";

pub struct Server {
    pub vault: PathBuf,
    pub config: String,
    pub folder: String,
    pub sloptotal_port: u16,
    pub helper_port: u16,
    pub mode: device::Mode,
    pub idle: Duration,
    /// The helper this server runs itself, when none answered.
    own: Mutex<Option<Arc<helper::Helper>>>,
}

fn tools() -> Value {
    let gpu = json!({ "type": "string", "enum": ["", "free", "cpu"], "description": "Only after a gpu_shortage answer and the user's choice: free (the user's other GPU services let go) or cpu." });
    let note = json!({ "type": "string", "description": "A note: its vault path (e.g. Drafts/Chapter.md), with or without .md, or its name." });
    json!([
        { "name": "status", "description": "What can run now: the helper and SlopTotal (running, device), the library folder and its notes, the paraphrase search.", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } },
        { "name": "check_ai", "description": "SlopTotal's score for each paragraph (0-100: up to 35 reads as written by a person, above 65 as AI), its band and colour class, and model-free hints for its sentences (stock phrases, signposting openers, vague wording, stacked hedges, em dashes, even sentence lengths). Paragraphs under 50 characters are skipped. Starts SlopTotal if needed. Always SlopTotal, never an online detector. More words than the user's limit (5,000 unless set) answers needs_confirmation: ask the user, then call again with confirm true.", "inputSchema": { "type": "object", "properties": {
            "paragraphs": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 200, "description": "The paragraphs, as prose (no markup)." }, "gpu": gpu.clone(), "confirm": { "type": "boolean", "description": "true once the user agreed to a check over their word limit" } }, "required": ["paragraphs"] } },
        { "name": "explain_ai", "description": "Every SlopTotal detector's score and one-line reason for one text, ordered by SlopTotal's weights for the text's language, with those of weight 0 listed apart as not counted, and those that did not run on the text listed apart (did_not_run) with SlopTotal's reason; and the overall score (use that, not any single detector). When detectors did not run, the overall score is the weighted average of those that ran, by SlopTotal's weights renormalised over them (SlopTotal's own number counts them as 0 and is not given); it is null, with no_score saying why, when too few ran or the weights could not be read. Slow on the processor (seconds to a minute).", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" }, "gpu": gpu.clone() }, "required": ["text"] } },
        { "name": "check_similarity", "description": "Wording the text shares with the library notes (verbatim and near-verbatim runs of 6+ words outside quotation marks) and, with mode both or paraphrase, close paraphrases (a sentence keeping at least the user's paraphrase closeness, 50% by default, of a library sentence's content words in its order, sharing that sentence's particular words; found by the library index's content-word search, no model; restating a source in another structure is not one). Each match: kind, UTF-16 span in the text, score (verbatim 1; paraphrase: the share of words kept in order), the source note (vault path, citation key, title, page) and its words. Fixed phrasing (in 5+ notes) is listed apart.", "inputSchema": { "type": "object", "properties": {
            "text": { "type": "string", "description": "Markdown or prose; quotations, citations, code, maths and comments are left out." }, "mode": { "type": "string", "enum": ["verbatim", "paraphrase", "both"] }, "note": note.clone(), "gpu": gpu }, "required": ["text"] } },
        { "name": "check_overlap", "description": "The close-copying check of a draft file against the works it cites (or the whole library): runs shared with library notes outside quotation marks, as check-overlap prints them.", "inputSchema": { "type": "object", "properties": {
            "draft": { "type": "string", "description": "The draft's path (absolute, or relative to the vault)." }, "library": { "type": "boolean" }, "words": { "type": "integer", "minimum": 4, "maximum": 100 }, "terms": { "type": "array", "items": { "type": "string" } } }, "required": ["draft"] }, "annotations": { "readOnlyHint": true } },
        { "name": "reveal_characters", "description": "Invisible characters, unusual spaces, dashes, look-alike punctuation and letters, styled letters (and typographic ones if asked) in a text, by fixed rules, with UTF-16 spans.", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" }, "typographic": { "type": "boolean" } }, "required": ["text"] }, "annotations": { "readOnlyHint": true } },
        { "name": "list_notes", "description": "The notes the plugin has results for (AI paragraphs checked, plagiarism matches).", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } },
        { "name": "ai_results", "description": "The AI check's results the plugin keeps for a note: each checked paragraph still in the note, with its score, line and band.", "inputSchema": { "type": "object", "properties": { "note": note.clone() }, "required": ["note"] }, "annotations": { "readOnlyHint": true } },
        { "name": "similarity_results", "description": "The plagiarism check's matches the plugin keeps for a note, with their lines and sources.", "inputSchema": { "type": "object", "properties": { "note": note.clone() }, "required": ["note"] }, "annotations": { "readOnlyHint": true } },
        { "name": "unusual_characters", "description": "Unusual characters in a note, with line and column.", "inputSchema": { "type": "object", "properties": { "note": note, "typographic": { "type": "boolean" } }, "required": ["note"] }, "annotations": { "readOnlyHint": true } }
    ])
}

/// SlopTotal's API through the helper, blocking (sloptotal::block_on runs the flows).
struct Proxy(u16);

impl sloptotal::Env for Proxy {
    async fn get(&self, path: &str) -> sloptotal::Reply {
        let r = http::request(self.0, "GET", &format!("/sloptotal{path}"), None, Duration::from_secs(60));
        r.map(|r| sloptotal::Reply { status: r.status, body: r.body }).unwrap_or_else(|e| sloptotal::Reply { status: 0, body: e })
    }
    async fn post(&self, path: &str, body: String) -> sloptotal::Reply {
        let r = http::request(self.0, "POST", &format!("/sloptotal{path}"), Some(&body), Duration::from_secs(900));
        r.map(|r| sloptotal::Reply { status: r.status, body: r.body }).unwrap_or_else(|e| sloptotal::Reply { status: 0, body: e })
    }
    async fn sleep(&self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
}

/// 1-based line and column (characters) of a UTF-16 place.
fn line_col(text: &str, at: u32) -> (usize, usize) {
    let (mut line, mut col, mut pos) = (1, 1, 0u32);
    for c in text.chars() {
        if pos >= at {
            break;
        }
        pos += c.len_utf16() as u32;
        if c == '\n' {
            (line, col) = (line + 1, 1);
        } else {
            col += 1;
        }
    }
    (line, col)
}

impl Server {
    #[allow(clippy::too_many_arguments)]
    pub fn new(vault: PathBuf, config: String, folder: String, sloptotal_port: u16, helper_port: u16, mode: device::Mode, idle: Duration) -> Server {
        Server { vault, config, folder, sloptotal_port, helper_port, mode, idle, own: Mutex::new(None) }
    }

    fn v(&self) -> Vault {
        Vault { root: self.vault.clone(), config: self.config.clone() }
    }

    /// A helper answering: Obsidian's, else one run here.
    fn helper(&self) -> Result<u16, String> {
        if helper::answering(self.helper_port)? {
            return Ok(self.helper_port);
        }
        let mut own = self.own.lock().unwrap_or_else(|e| e.into_inner());
        if own.is_none() {
            let h = helper::Helper::new(helper::Config { port: self.helper_port, folder: self.folder.clone(), sloptotal_port: self.sloptotal_port, mode: self.mode, idle: self.idle });
            h.serve()?;
            *own = Some(h);
        }
        Ok(self.helper_port)
    }

    /// SlopTotal running through the helper; Ok(Some(shortage)) when the user must be asked about the GPU first.
    fn sloptotal(&self, gpu: &str) -> Result<Option<Value>, String> {
        let port = self.helper()?;
        let r = http::request(port, "POST", "/sloptotal/start", Some(&json!({ "gpu": gpu }).to_string()), Duration::from_secs(60))?;
        let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
        if r.status != 200 {
            return Err(v["error"].as_str().unwrap_or("the helper could not start SlopTotal").to_string());
        }
        if v["gpu_shortage"].is_object() {
            return Ok(Some(v["gpu_shortage"].clone()));
        }
        for _ in 0..600 {
            let h = http::request(port, "GET", "/health", None, Duration::from_secs(5))?;
            let hv: Value = serde_json::from_str(&h.body).unwrap_or(Value::Null);
            let s = &hv["sloptotal"];
            match s["state"].as_str() {
                Some("running") => {
                    let failed: Vec<String> = s["engines"]["failed"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                    return if failed.is_empty() { Ok(None) } else { Err(sloptotal::failed_message(&failed)) };
                }
                Some("failed") | Some("stopped") => return Err(format!("SlopTotal could not start: {}", s["why"].as_str().unwrap_or(""))),
                _ => std::thread::sleep(Duration::from_secs(1)),
            }
        }
        Err("SlopTotal did not become ready in 10 minutes".into())
    }

    fn shortage(s: Value) -> Value {
        json!({ "gpu_shortage": s, "ask_the_user": "The graphics card has too little free memory for SlopTotal's models. Ask the user whether to free it (their other GPU services let go) or use the processor (slower), then call again with gpu 'free' or 'cpu'." })
    }

    fn check_ai(&self, args: &Value) -> Result<Value, String> {
        let paras: Vec<String> = args["paragraphs"].as_array().ok_or("paragraphs: a list of strings")?.iter().take(200).filter_map(|p| p.as_str().map(String::from)).collect();
        if paras.is_empty() {
            return Err("paragraphs: at least one".into());
        }
        // a long check asks first, as the plugin does (its setting "Ask before checking more than this many words")
        let words: usize = paras.iter().map(|p| p.split_whitespace().count()).sum();
        let limit = self.v().settings()["confirmOver"].as_f64().unwrap_or(5000.0);
        if words as f64 > limit && args["confirm"].as_bool() != Some(true) {
            return Ok(json!({ "needs_confirmation": format!("{words} words: SlopTotal takes a few seconds a paragraph on the processor. Ask the user, then call again with confirm true."), "words": words, "limit": limit }));
        }
        if let Some(s) = self.sloptotal(args["gpu"].as_str().unwrap_or(""))? {
            return Ok(Self::shortage(s));
        }
        let w = self.weights();
        let scores = sloptotal::block_on(ai::score_list(&Proxy(self.helper_port), &paras, Some(&w)))?;
        let out: Vec<Value> = paras.iter().zip(scores).enumerate().map(|(i, (p, s))| {
            let sents = ai::sentences(p);
            let texts: Vec<&str> = sents.iter().map(|r| &p[r.clone()]).collect();
            let hints: Vec<Value> = texts.iter().enumerate().flat_map(|(k, t)| {
                let start16 = p[..sents[k].start].encode_utf16().count();
                hints::hints(t, &texts).into_iter().map(move |h| json!({ "sentence": k, "habit": h.label, "found": h.found, "why": h.why,
                    "spans": h.spans.iter().map(|r| [start16 + t[..r.start].encode_utf16().count(), start16 + t[..r.end].encode_utf16().count()]).collect::<Vec<_>>() }))
            }).collect();
            match s {
                Some((s, together)) => json!({ "index": i, "score": s.score, "verdict": s.verdict, "confidence": s.confidence, "class": ai::class(ai::tint(s.score)), "describe": ai::describe(&s), "words": p.split_whitespace().count(), "hints": hints, "caveat": sloptotal::quick_caveat(&s), "scored_together": together }),
                None => json!({ "index": i, "skipped": "not scored (all the paragraphs together are under SlopTotal's 50 characters, or too few of its detectors ran)", "hints": hints }),
            }
        }).collect();
        Ok(json!({ "paragraphs": out, "scale": "0 reads as written by a person, 100 as AI; clean up to 35, ai above 65, mixed between", "caution": "Resemblance to model output, not authorship; unreliable under about 80 words and on non-native English." }))
    }

    fn weights(&self) -> sloptotal::Weights {
        let r = http::request(self.helper_port, "GET", "/weights", None, Duration::from_secs(10)).ok();
        let v: Value = r.and_then(|r| serde_json::from_str(&r.body).ok()).unwrap_or(Value::Null);
        let mut w = sloptotal::parse_calibration(&v["calibration"]);
        w.names = sloptotal::parse_engines(&v["engines"]);
        w
    }

    fn explain_ai(&self, args: &Value) -> Result<Value, String> {
        let text = args["text"].as_str().ok_or("text: a string")?;
        if let Some(s) = self.sloptotal(args["gpu"].as_str().unwrap_or(""))? {
            return Ok(Self::shortage(s));
        }
        let mut a = sloptotal::block_on(sloptotal::analyze(&Proxy(self.helper_port), text))?;
        let w = self.weights();
        sloptotal::settle(&mut a, Some(&w), text);
        let reason = |r: &sloptotal::Reason| json!({ "detector": r.engine, "score": (r.score * 1000.0).round() / 1000.0, "verdict": r.verdict, "reason": r.details });
        let detectors = match w.order(&a) {
            Some((counted, uncounted)) => json!({
                "counted": counted.iter().map(|(r, x)| { let mut v = reason(r); v["weight"] = json!(x); v }).collect::<Vec<_>>(),
                "not_counted_for_this_language": uncounted.iter().map(|r| reason(r)).collect::<Vec<_>>() }),
            None => json!({ "unweighted": a.reasons.iter().map(reason).collect::<Vec<_>>(), "note": "SlopTotal's weights could not be read" }),
        };
        let failed: Vec<Value> = a.failed.iter().map(|f| json!({ "detector": f.engine, "reason": f.details })).collect();
        let verdict = if a.failed.is_empty() { a.verdict.clone() } else { a.score.map(|x| sloptotal::verdict_of(x).to_string()).unwrap_or_default() };
        Ok(json!({ "overall_score": a.score, "no_score": a.no_score, "verdict": verdict, "language": a.language, "language_support": a.language_support, "detectors": detectors,
            "did_not_run": failed, "overall_note": sloptotal::overall_line(&a),
            "weights_note": "Weights are SlopTotal's own, fitted by its author on their benchmark (SlopBench); they reflect accuracy and overlap between detectors, not trustworthiness alone. Use the overall score." }))
    }

    fn check_similarity(&self, args: &Value) -> Result<Value, String> {
        let text = args["text"].as_str().ok_or("text: a string")?;
        let mode = args["mode"].as_str().unwrap_or("both");
        let v = self.v();
        let me = args["note"].as_str().map(|n| n.trim_start_matches('/').replace('\\', "/")).unwrap_or_default();
        let scope = v.scope(None, &[]);
        if !scope.is_set() {
            return Err(format!("no library folder: {}", authenticity_core::library::SET_LIBRARY));
        }
        let mut s = similarity::Settings { fixed_terms: v.fixed_terms(), ..similarity::Settings::default() };
        let set = v.settings();
        if let Some(n) = set["minRun"].as_f64() {
            s.min_run = n as usize;
        }
        let mut matches = Vec::new();
        if mode != "paraphrase" {
            let (ix, keys) = v.index(&scope.library);
            let q = s.query(text);
            let o = s.options();
            let plan = ix.plan(&q, &o, &|k| k != me && scope.holds(k));
            let texts = authenticity_similar::library::passage_texts(&plan.refs, &keys, |k| v.read(k));
            let found = ix.finish(&q, &o, &plan, &texts);
            matches = similarity::verbatim(text, 0, &found, |k| authenticity_core::library::title(k, &scope.library), &texts, |k| v.read(k));
        }
        let mut paraphrase = Value::Null;
        if mode != "verbatim" {
            let covered: Vec<std::ops::Range<usize>> = matches.iter().filter(|m| m.kind != "common").filter_map(|m| text.find(&m.text).map(|a| a..a + m.text.len())).collect();
            let closeness = set["paraphraseCloseness"].as_f64().unwrap_or(para::CLOSENESS).clamp(para::MIN_CLOSENESS, 1.0);
            let (ix, keys) = v.index(&scope.library);
            let found = similarity::paraphrase_local(&ix, &keys, text, &s, &covered, &|k| k != me && scope.holds(k), similarity::fetch_floor(closeness), &scope.library, |k| v.read(k));
            paraphrase = json!(format!("checked by the content-word search: a close paraphrase keeps at least {:.0}% of a sentence's content words in a library sentence's order, and the words it shares are that sentence's particular ones", closeness * 100.0));
            matches.extend(found);
        }
        // the best candidate per sentence at the user's closeness (and the runs at the shortest run)
        let closeness = set["paraphraseCloseness"].as_f64().unwrap_or(para::CLOSENESS);
        let mut matches = similarity::filter(&matches, similarity::Thresholds::new(s.min_run as f64, closeness));
        matches.sort_by_key(|m| (m.from, m.to));
        let (common, found): (Vec<_>, Vec<_>) = matches.into_iter().partition(|m| m.kind == "common");
        Ok(json!({ "matches": found, "fixed_phrasing": common, "paraphrase": paraphrase, "library": scope.library,
            "rule": "Wording copied without quotation marks is plagiarism even when cited: quote exactly with the page, or paraphrase in the user's own structure." }))
    }

    fn check_overlap(&self, args: &Value) -> Result<Value, String> {
        let d = args["draft"].as_str().ok_or("draft: a file")?;
        let p = PathBuf::from(d);
        let draft = if p.is_absolute() { p } else { self.vault.join(d) };
        let a = overlap::Args {
            draft: draft.to_string_lossy().into_owned(),
            library: args["library"].as_bool().unwrap_or(false),
            words: args["words"].as_u64().unwrap_or(6) as usize,
            common: 5,
            content: 4,
            terms: args["terms"].as_array().map(|t| t.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            vault: Some(self.vault.to_string_lossy().into_owned()),
            folder: None,
            exclude: Vec::new(),
            json: true,
        };
        let o = overlap::check(&a)?;
        Ok(json!({ "draft": d, "scope": if a.library { "library" } else { "cited" }, "sources_compared": o.sources, "missing": o.missing,
            "runs": o.runs.iter().map(|r| json!({ "note": r.note, "path": r.path, "longest": r.length, "text": r.text, "near_verbatim": r.near, "matched": r.matched, "words": r.words, "other_notes": r.others })).collect::<Vec<_>>(),
            "fixed_phrasing": o.commons.iter().map(|(t, n)| json!({ "text": t, "notes": n })).collect::<Vec<_>>() }))
    }

    fn status(&self) -> Value {
        let v = self.v();
        let lib = v.library(None);
        let helper = http::request(self.helper_port, "GET", "/health", None, Duration::from_secs(3)).ok().and_then(|r| serde_json::from_str::<Value>(&r.body).ok());
        json!({ "helper": helper.map(|h| h["sloptotal"].clone()).unwrap_or(json!("not running (started by the first AI check)")), "sloptotal_folder": if self.folder.is_empty() { "not set" } else { "set" },
            "library_folder": if lib.is_empty() { json!(null) } else { json!(lib) }, "library_notes": if lib.is_empty() { 0 } else { v.library_files(&lib).len() }, "paraphrase": "the content-word search (no model)" })
    }

    // ----- what the plugin keeps

    fn plugin_file(&self, name: &str) -> Value {
        std::fs::read(self.vault.join(&self.config).join("plugins").join("authenticity").join(name)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
    }

    fn resolve(&self, note: &str, known: &[String]) -> String {
        let n = note.trim().trim_start_matches('/').replace('\\', "/");
        let with_md = if n.ends_with(".md") { n } else { format!("{n}.md") };
        if known.contains(&with_md) {
            return with_md;
        }
        let name = with_md.rsplit('/').next().unwrap_or(&with_md).to_string();
        known.iter().find(|k| k.rsplit('/').next() == Some(name.as_str())).cloned().unwrap_or(with_md)
    }

    fn list_notes(&self) -> Value {
        let ai: ai::Store = serde_json::from_value(self.plugin_file("ai.json")).unwrap_or_default();
        let m: HashMap<String, Vec<similarity::Match>> = serde_json::from_value(self.plugin_file("matches.json")).unwrap_or_default();
        let mut notes: Vec<&String> = ai.notes.keys().chain(m.keys()).collect();
        notes.sort();
        notes.dedup();
        json!({ "notes": notes.iter().map(|n| json!({ "note": n, "ai_paragraphs": ai.notes.get(*n).map_or(0, Vec::len), "plagiarism_matches": m.get(*n).map_or(0, |l| l.iter().filter(|x| x.kind != "common").count()) })).collect::<Vec<_>>() })
    }

    fn ai_results(&self, args: &Value) -> Result<Value, String> {
        let st: ai::Store = serde_json::from_value(self.plugin_file("ai.json")).unwrap_or_default();
        let known: Vec<String> = st.notes.keys().cloned().collect();
        let path = self.resolve(args["note"].as_str().unwrap_or_default(), &known);
        let text = self.v().read(&path).ok_or(format!("no note at {path}"))?;
        let rows: Vec<Value> = ai::paragraphs(&authenticity_core::prose::extract(&text)).into_iter().filter_map(|p| st.of(&path, &p.text).map(|s| json!({ "line": line_col(&text, p.from).0, "score": s.score.score, "verdict": s.score.verdict, "class": ai::class(ai::tint(s.score.score)), "opens": p.text.chars().take(120).collect::<String>() }))).collect();
        if rows.is_empty() {
            return Err(format!("no AI results for {path}: check it in Obsidian (Check this note for AI writing) or with check_ai"));
        }
        Ok(json!({ "note": path, "paragraphs": rows }))
    }

    fn similarity_results(&self, args: &Value) -> Result<Value, String> {
        let m: HashMap<String, Vec<similarity::Match>> = serde_json::from_value(self.plugin_file("matches.json")).unwrap_or_default();
        let known: Vec<String> = m.keys().cloned().collect();
        let path = self.resolve(args["note"].as_str().unwrap_or_default(), &known);
        let list = m.get(&path).ok_or(format!("no plagiarism results for {path}: check it in Obsidian or with check_similarity"))?;
        let text = self.v().read(&path).unwrap_or_default();
        let rows: Vec<Value> = list.iter().map(|x| {
            let line = authenticity_core::align::find_utf16(&text, &x.text).map(|(a, _)| line_col(&text, a).0);
            json!({ "kind": x.kind, "line": line, "text": x.text, "score": x.score, "source": x.source, "title": x.title, "source_text": x.source_text, "page": x.page })
        }).collect();
        Ok(json!({ "note": path, "matches": rows }))
    }

    fn unusual_characters(&self, args: &Value) -> Result<Value, String> {
        let path = self.resolve(args["note"].as_str().unwrap_or_default(), &[]);
        let text = self.v().read(&path).ok_or(format!("no note at {path}"))?;
        Ok(json!({ "note": path, "characters": self.characters(&text, args["typographic"].as_bool().unwrap_or(false), true) }))
    }

    fn characters(&self, text: &str, typo: bool, lines: bool) -> Vec<Value> {
        chars::scan(text, typo).iter().map(|h| {
            let mut v = json!({ "from": h.from, "to": h.to, "char": char::from_u32(h.code).map(String::from), "code": format!("U+{:04X}", h.code), "name": h.name, "kind": h.kind.label() });
            if lines {
                let (l, c) = line_col(text, h.from);
                v["line"] = json!(l);
                v["column"] = json!(c);
            }
            v
        }).collect()
    }

    pub fn call(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "status" => Ok(self.status()),
            "check_ai" => self.check_ai(args),
            "explain_ai" => self.explain_ai(args),
            "check_similarity" => self.check_similarity(args),
            "check_overlap" => self.check_overlap(args),
            "reveal_characters" => Ok(json!({ "characters": self.characters(args["text"].as_str().unwrap_or_default(), args["typographic"].as_bool().unwrap_or(false), false) })),
            "list_notes" => Ok(self.list_notes()),
            "ai_results" => self.ai_results(args),
            "similarity_results" => self.similarity_results(args),
            "unusual_characters" => self.unusual_characters(args),
            _ => Err(format!("Unknown tool {name}")),
        }
    }

    pub fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let method = msg["method"].as_str().unwrap_or_default();
        let result = match method {
            "initialize" => json!({ "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL), "capabilities": { "tools": {} },
                "serverInfo": { "name": "authenticity", "version": env!("CARGO_PKG_VERSION") }, "instructions": INSTRUCTIONS }),
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tools() }),
            "tools/call" => match self.call(msg["params"]["name"].as_str().unwrap_or_default(), &msg["params"]["arguments"]) {
                Ok(v) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "structuredContent": v }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            },
            _ => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {method}") } })),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    /// Serve stdin until it closes, then stop what this server started.
    pub fn run(&self) {
        let stdin = std::io::stdin();
        let mut out = std::io::stdout().lock();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("Parse error: {e}") } })),
            };
            if let Some(r) = reply {
                let _ = writeln!(out, "{r}");
                let _ = out.flush();
            }
        }
        if let Some(h) = self.own.lock().unwrap_or_else(|e| e.into_inner()).take() {
            h.stop("the MCP server stopped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::vault;

    fn server(root: &std::path::Path) -> Server {
        // three ports where nothing listens (held together so they differ, then let go)
        let ls: Vec<std::net::TcpListener> = (0..3).map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap()).collect();
        let p: Vec<u16> = ls.iter().map(|l| l.local_addr().unwrap().port()).collect();
        drop(ls);
        Server::new(root.to_path_buf(), ".obsidian".into(), String::new(), p[0], p[1], device::Mode::Cpu, Duration::from_secs(60))
    }

    #[test]
    fn tools_and_offline_checks_on_a_fake_vault() {
        let v = vault("mcp", &[("Library/a2020/Paper A.md", "Earlier studies reported that the proposed sampling procedure reduces the variance of every estimator considerably.")]);
        std::env::set_var("XDG_CACHE_HOME", v.root.join("cache"));
        std::env::set_var("LOCALAPPDATA", v.root.join("cache"));
        let s = server(&v.root);
        // no library folder set: the similarity check asks for it
        let e = s.call("check_similarity", &json!({ "text": "Any text at all." })).unwrap_err();
        assert!(e.contains("Library folder"), "{e}");
        std::fs::create_dir_all(v.root.join(".obsidian/plugins/authenticity")).unwrap();
        std::fs::write(v.root.join(".obsidian/plugins/authenticity/data.json"), r#"{"libraryFolder":"Library/"}"#).unwrap();
        let list = s.handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        for t in ["check_ai", "explain_ai", "check_similarity", "check_overlap", "reveal_characters", "status"] {
            assert!(names.contains(&t), "{t}");
        }
        // similarity, verbatim, with spans in the text
        let text = "We know that the proposed sampling procedure reduces the variance of every estimator today.";
        let r = s.call("check_similarity", &json!({ "text": text, "mode": "verbatim" })).unwrap();
        let m = &r["matches"][0];
        assert_eq!((m["kind"].as_str(), m["source"].as_str()), (Some("verbatim"), Some("Library/a2020/Paper A.md")));
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[m["from"].as_u64().unwrap() as usize..m["to"].as_u64().unwrap() as usize]).unwrap(), m["text"].as_str().unwrap());
        // paraphrase: the content-word search, on this computer
        let r = s.call("check_similarity", &json!({ "text": "This sentence has more than eight words and is checked for paraphrase here.", "mode": "paraphrase" })).unwrap();
        assert!(r["paraphrase"].as_str().unwrap().contains("content-word search"), "{r}");
        // characters
        let r = s.call("reveal_characters", &json!({ "text": "a\u{200B}b" })).unwrap();
        assert_eq!(r["characters"][0]["code"], "U+200B");
        // the AI check without a SlopTotal folder: refused with what to do, and no SlopTotal started
        let e = s.call("check_ai", &json!({ "paragraphs": ["A paragraph long enough to be scored by the detectors of SlopTotal."] })).unwrap_err();
        assert!(e.contains("SlopTotal folder"), "{e}");
        let bad = s.handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "nope", "arguments": {} } })).unwrap();
        assert_eq!(bad["result"]["isError"], true);
        _ = std::fs::remove_dir_all(&v.root);
    }

    #[test]
    fn lines() {
        assert_eq!(line_col("a\nbc\nd", 3), (2, 2));
        assert_eq!(line_col("😀\nx", 3), (2, 1));
    }
}
