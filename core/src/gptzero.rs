//! GPTZero, the optional online AI detector (<https://docs.gptzero.me/>, OpenAPI at
//! <https://docs.gptzero.me/openapi/public.yaml>): used only when its API key is set in Obsidian's keychain and the user
//! chooses it for a check (the plugin asks each time; app.rs). `POST https://api.gptzero.me/v2/predict/text` with the
//! key in `x-api-key` and `{"document": text}` (nothing else) returns probabilities for the document, its paragraphs
//! and its sentences; `GET /v3/usage-stats` the plan and the words used and left this billing cycle (it uses no words).
//!
//! On the pane's scale, a text's AI-likeness is 100 times GPTZero's `completely_generated_prob` (the probability it
//! gives that the text was written by AI), a sentence's 100 times its `generated_prob`. GPTZero says why only through
//! AI Patterns, which it opens to allow-listed keys only; its scan answers give the probabilities, the class, a
//! confidence, a result message, and which sentences it would highlight as AI.
//!
//! Each unit of a check (a paragraph, or a group of short ones: ai.rs) is one request, as with SlopTotal, so every row
//! has its own result. The flows are written against `Net` (a request), so they run in Obsidian (requestUrl) and in
//! tests (a fake server). The key is passed through to the request and never kept, logged or returned.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sloptotal::{verdict_of, Score, CANCELLED};

pub const PREDICT: &str = "https://api.gptzero.me/v2/predict/text";
pub const USAGE: &str = "https://api.gptzero.me/v3/usage-stats";
/// AI Patterns: GPTZero's explanations, sentence by sentence (Server-Sent Events; only for keys GPTZero allow-lists).
pub const PATTERNS: &str = "https://api.gptzero.me/v3/ai/patterns/stream";
/// The longest text GPTZero accepts on API plans, in characters.
pub const MAX_CHARS: usize = 150_000;
/// The engine's name in a score, the history and the pane.
pub const ENGINE: &str = "gptzero";

/// One sentence of GPTZero's answer: its text, the probability it was written by AI, and whether GPTZero would
/// highlight it as AI.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sentence {
    pub text: String,
    pub p: f64,
    pub highlight: bool,
}

/// A request to GPTZero: (status, body); status 0 when it could not be made.
#[allow(async_fn_in_trait)]
pub trait Net {
    async fn post(&self, url: &str, key: &str, body: String) -> (u16, String);
    async fn get(&self, url: &str, key: &str) -> (u16, String);
    fn cancelled(&self) -> bool {
        false
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(400).collect()
}

fn p(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite() && (0.0..=1.0).contains(x))
}

/// A score from `/v2/predict/text`'s answer, on the pane's scale.
pub fn parse(body: &Value) -> Result<Score, String> {
    let doc = body.get("documents").and_then(|d| d.get(0)).ok_or("GPTZero's answer had no document in it")?;
    let prob = p(doc, "completely_generated_prob").ok_or("GPTZero's answer had no probability in it")?;
    let score = (prob * 1000.0).round() / 10.0;
    let sentences = doc.get("sentences").and_then(Value::as_array).into_iter().flatten().take(2000).filter_map(|x| {
        Some(Sentence { text: s(x, "sentence"), p: p(x, "generated_prob")?, highlight: x.get("highlight_sentence_for_ai").and_then(Value::as_bool).unwrap_or(false) })
    }).collect();
    let class = match s(doc, "predicted_class").as_str() {
        c @ ("human" | "ai" | "mixed") => c.to_string(),
        _ => String::new(),
    };
    let confidence = match s(doc, "confidence_category").as_str() {
        c @ ("low" | "medium" | "high") => c.to_string(),
        _ => String::new(),
    };
    Ok(Score {
        score,
        verdict: verdict_of(score).into(),
        confidence,
        engine: ENGINE.into(),
        class,
        message: s(doc, "result_message"),
        sentences,
        ..Score::default()
    })
}

/// What went wrong, in plain words, from an answer that was not a 200 (GPTZero's `error` when it gives one).
pub fn error(status: u16, body: &str) -> String {
    let said = serde_json::from_str::<Value>(body).ok().map(|v| s(&v, "error")).filter(|e| !e.is_empty());
    let what = match status {
        400 => "GPTZero refused the text",
        401 => "the GPTZero key is missing or has expired",
        402 => "GPTZero asks for payment (the plan or subscription)",
        403 => "GPTZero does not recognise the key",
        429 => "the GPTZero plan's word limit or hourly rate limit is reached",
        500..=599 => "GPTZero had a problem on its side; try again later",
        0 => "GPTZero could not be reached (no connection?)",
        _ => "GPTZero answered with an error",
    };
    match said {
        Some(e) => format!("{what} ({status}: {e})"),
        None => format!("{what} ({status})"),
    }
}

/// The plan, and the words left this cycle (None: no word limit), from usage-stats (no words used); None when the
/// key does not work.
pub fn parse_usage(body: &str) -> Option<(String, Option<f64>)> {
    let v: Value = serde_json::from_str(body).ok()?;
    let d = v.get("data")?;
    Some((s(d, "plan"), d.get("words_left").and_then(Value::as_f64)))
}

/// The key checked with usage-stats (no words used): what to tell the user.
pub async fn check_key<N: Net>(net: &N, key: &str) -> String {
    let (status, body) = net.get(USAGE, key).await;
    if status != 200 {
        return without_key(error(status, &body), key);
    }
    match parse_usage(&body) {
        Some((plan, Some(left))) => format!("The key works. Plan: {plan}; {left:.0} words left this billing cycle (checking used none)."),
        Some((plan, None)) => format!("The key works. Plan: {plan}, billed by the word with no fixed allowance (checking used none)."),
        None => "GPTZero answered, but not as its usage API does.".into(),
    }
}

/// What a check does about GPTZero before anything is sent: no key means SlopTotal, with no question; a choice made
/// "for this session" is used again (never across restarts); otherwise the user is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    SlopTotal,
    GptZero,
    Ask,
}

pub fn plan(key_linked: bool, session: Option<&str>) -> Plan {
    match (key_linked, session) {
        (false, _) => Plan::SlopTotal,
        (true, Some(ENGINE)) => Plan::GptZero,
        (true, Some(_)) => Plan::SlopTotal,
        (true, None) => Plan::Ask,
    }
}

/// The question's text: what each choice does, with the words the check would send and what the plan has left.
pub fn question(words: usize, left: Option<Option<f64>>) -> String {
    let plan = match left {
        Some(Some(l)) => format!(" Your plan has {l:.0} words left this billing cycle; this check would use about {words}."),
        Some(None) => " Your plan is billed by the word.".to_string(),
        None => String::new(),
    };
    format!("GPTZero is an online detector: it sends this text (about {words} words, the text alone) to GPTZero's servers, and the words count against your GPTZero plan.{plan} SlopTotal checks it on this computer, offline.")
}

/// What the question's answer means: None when cancelled (nothing is sent); else the engine for this check and, when
/// "Remember for this session" was ticked, the engine to remember until Obsidian restarts.
pub fn answer(chosen: Option<(&str, bool)>) -> Option<(String, Option<String>)> {
    let (engine, remember) = chosen?;
    let engine = if engine == ENGINE { ENGINE } else { "sloptotal" };
    Some((engine.to_string(), remember.then(|| engine.to_string())))
}

/// The offer after GPTZero failed: what went wrong, that nothing of it was kept, and SlopTotal instead.
pub fn fallback(error: &str) -> String {
    format!("{error}. Nothing was kept from GPTZero. Check with SlopTotal (offline) instead?")
}

/// What the debug log keeps of a request to GPTZero: its size, status and time; never the key or the text.
pub fn request_log(bytes: usize, status: u16, ms: f64) -> Value {
    serde_json::json!({ "bytes": bytes, "status": status, "ms": ms.round() })
}

/// One AI writing pattern GPTZero found in a sentence, with its plain-language explanation (AI Patterns).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternHit {
    /// The sentence's place in the text (from 0) and its words.
    pub sentence_index: usize,
    pub sentence: String,
    /// The pattern's name as GPTZero shows it, its category, and its description.
    pub name: String,
    pub category: String,
    pub description: String,
    /// Why this sentence matched it.
    pub explanation: String,
    /// How many times more often the pattern shows up in AI text than in human text, when measured.
    pub k_times: Option<f64>,
}

/// AI Patterns' answer (`data: <json>` lines; a final `{"error": …}` event when it failed after starting) as the
/// patterns found, in the order of the sentences.
pub fn parse_patterns(body: &str) -> Result<Vec<PatternHit>, String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data:") else { continue };
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else { continue };
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            return Err(format!("GPTZero's AI Patterns stopped: {}", e.chars().take(200).collect::<String>()));
        }
        let index = v.get("sentence_index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let sentence = s(&v, "sentence");
        for p in v.get("patterns").and_then(Value::as_array).into_iter().flatten().take(50) {
            let name = Some(s(p, "display_name")).filter(|x| !x.is_empty()).unwrap_or_else(|| s(p, "pattern_name"));
            let description = Some(s(p, "display_description")).filter(|x| !x.is_empty()).unwrap_or_else(|| s(p, "description"));
            let k_times = p.get("k_times").and_then(Value::as_f64).filter(|x| x.is_finite());
            out.push(PatternHit { sentence_index: index, sentence: sentence.clone(), name, category: s(p, "category"), description, explanation: s(p, "explanation"), k_times });
        }
    }
    Ok(out)
}

/// Ask AI Patterns about `text` (the user asked, for a paragraph GPTZero scored): the patterns found, or what went
/// wrong, with the allow-listing GPTZero requires said when the key is refused.
pub async fn patterns<N: Net>(net: &N, key: &str, text: &str) -> Result<Vec<PatternHit>, String> {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() || t.chars().count() > MAX_CHARS {
        return Err("the text is empty or too long for GPTZero".into());
    }
    let (status, body) = net.post(PATTERNS, key, serde_json::json!({ "documents": [{ "inputText": t }] }).to_string()).await;
    match status {
        200 => parse_patterns(&body).map_err(|e| without_key(e, key)),
        401 | 403 => Err(without_key(format!("{}. AI Patterns works only for keys GPTZero has allow-listed for it", error(status, &body)), key)),
        _ => Err(without_key(error(status, &body), key)),
    }
}

/// An error message with the key taken out, should GPTZero's answer quote it.
fn without_key(msg: String, key: &str) -> String {
    if key.len() >= 4 && msg.contains(key) { msg.replace(key, "[key]") } else { msg }
}

/// One score per text (a check's units), one request each, in order: None for a text GPTZero cannot take (empty, or
/// over MAX_CHARS); an error stops the check (nothing after it is sent).
pub async fn scores<N: Net>(net: &N, key: &str, texts: &[&str]) -> Result<Vec<Option<Score>>, String> {
    let mut out = Vec::with_capacity(texts.len());
    for t in texts {
        if net.cancelled() {
            return Err(CANCELLED.into());
        }
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() || t.chars().count() > MAX_CHARS {
            out.push(None);
            continue;
        }
        let (status, body) = net.post(PREDICT, key, serde_json::json!({ "document": t }).to_string()).await;
        if status != 200 {
            return Err(without_key(error(status, &body), key));
        }
        let v: Value = serde_json::from_str(&body).map_err(|_| "GPTZero's answer could not be read".to_string())?;
        out.push(Some(parse(&v)?));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sloptotal::block_on;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// GPTZero, faked: the requests it was sent (url, key, body) and its answers in turn.
    #[derive(Default)]
    struct Fake {
        answers: RefCell<VecDeque<(u16, String)>>,
        sent: RefCell<Vec<(String, String, String)>>,
    }

    impl Net for Fake {
        async fn post(&self, url: &str, key: &str, body: String) -> (u16, String) {
            self.sent.borrow_mut().push((url.into(), key.into(), body));
            self.answers.borrow_mut().pop_front().expect("an unexpected request")
        }
        async fn get(&self, url: &str, key: &str) -> (u16, String) {
            self.sent.borrow_mut().push((url.into(), key.into(), String::new()));
            self.answers.borrow_mut().pop_front().expect("an unexpected request")
        }
    }

    fn answer(prob: f64) -> String {
        serde_json::json!({ "version": "2026-10-07-base", "documents": [{ "completely_generated_prob": prob, "predicted_class": "ai", "confidence_category": "high", "result_message": "Our detector is highly confident that the text is written by AI.",
            "sentences": [{ "sentence": "First sentence.", "generated_prob": 0.99, "highlight_sentence_for_ai": true }, { "sentence": "Second one.", "generated_prob": 0.2, "highlight_sentence_for_ai": false }] }] }).to_string()
    }

    #[test]
    fn answers_map_to_the_scale_with_their_sentences() {
        let net = Fake { answers: RefCell::new([(200, answer(0.997)), (200, answer(0.12))].into()), ..Fake::default() };
        let got = block_on(scores(&net, "secret-key", &["A paragraph  of text.", "Another paragraph."])).unwrap();
        let a = got[0].as_ref().unwrap();
        assert_eq!((a.score, a.verdict.as_str(), a.engine.as_str(), a.class.as_str(), a.confidence.as_str()), (99.7, "ai", "gptzero", "ai", "high"));
        assert_eq!(a.sentences[0], Sentence { text: "First sentence.".into(), p: 0.99, highlight: true });
        assert_eq!(got[1].as_ref().unwrap().score, 12.0);
        // each unit its own request, the text alone (white space made single), the key only in its header
        let sent = net.sent.borrow();
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].0.as_str(), sent[0].2.as_str()), (PREDICT, r#"{"document":"A paragraph of text."}"#));
        assert!(sent.iter().all(|s| s.1 == "secret-key" && !s.2.contains("secret-key")));
        // the key is in no result
        assert!(!format!("{got:?}").contains("secret-key"));
    }

    #[test]
    fn errors_are_said_plainly_and_stop_the_check() {
        for (status, body, says) in [(401, "{}", "missing or has expired"), (429, r#"{"error":"Monthly limit of 10000 words has been reached"}"#, "word limit"), (0, "", "could not be reached")] {
            let net = Fake { answers: RefCell::new([(status, body.to_string())].into()), ..Fake::default() };
            let e = block_on(scores(&net, "k", &["One.", "Two."])).unwrap_err();
            assert!(e.contains(says), "{e}");
            assert_eq!(net.sent.borrow().len(), 1, "nothing sent after the error");
        }
    }

    #[test]
    fn no_key_no_question_and_never_remembered_across_restarts() {
        assert_eq!(plan(false, None), Plan::SlopTotal, "no key: SlopTotal, no question, nothing sent");
        assert_eq!(plan(false, Some(ENGINE)), Plan::SlopTotal, "a remembered choice needs the key too");
        assert_eq!(plan(true, None), Plan::Ask);
        assert_eq!(plan(true, Some(ENGINE)), Plan::GptZero, "chosen for this session");
        assert_eq!(plan(true, Some("sloptotal")), Plan::SlopTotal);
        let q = question(1234, Some(Some(5000.0)));
        assert!(q.contains("about 1234 words") && q.contains("5000 words left") && q.contains("offline"), "{q}");
        assert!(question(10, Some(None)).contains("billed by the word"));
    }

    /// The question's three answers: GPTZero or SlopTotal for this check (remembered only when ticked), and Cancel,
    /// after which nothing is sent.
    #[test]
    fn the_question_s_answers_and_cancel_sends_nothing() {
        assert_eq!(super::answer(Some((ENGINE, false))), Some((ENGINE.to_string(), None)));
        assert_eq!(super::answer(Some((ENGINE, true))), Some((ENGINE.to_string(), Some(ENGINE.to_string()))), "for this session");
        assert_eq!(super::answer(Some(("sloptotal", true))), Some(("sloptotal".to_string(), Some("sloptotal".to_string()))));
        assert_eq!(super::answer(Some(("anything else", false))).unwrap().0, "sloptotal", "never GPTZero unless chosen");
        assert_eq!(super::answer(None), None, "Cancel");
        // the flow as app.rs runs it: asked, cancelled, so no request at all
        let net = Fake::default();
        if plan(true, None) == Plan::Ask && super::answer(None).is_some() {
            _ = block_on(scores(&net, "k", &["Some text."]));
        }
        assert!(net.sent.borrow().is_empty(), "Cancel sends nothing");
        // after a session's SlopTotal choice, nothing goes to GPTZero either
        assert_eq!(plan(true, super::answer(Some(("sloptotal", true))).unwrap().1.as_deref()), Plan::SlopTotal);
    }

    /// A failure is said plainly and offers SlopTotal; the key appears in no message, log line or history line, even
    /// when GPTZero's answer quotes it.
    #[test]
    fn errors_offer_sloptotal_and_the_key_is_never_shown_or_logged() {
        let key = "gz-secret-0123456789";
        for (status, says) in [(401, "missing or has expired"), (403, "does not recognise the key"), (429, "word limit or hourly rate limit")] {
            let body = format!(r#"{{"error":"bad key {key}"}}"#);
            let net = Fake { answers: RefCell::new([(status, body.clone())].into()), ..Fake::default() };
            let e = block_on(scores(&net, key, &["One."])).unwrap_err();
            assert!(e.contains(says) && !e.contains(key) && e.contains("[key]"), "{e}");
            let offer = fallback(&e);
            assert!(offer.contains("Nothing was kept from GPTZero") && offer.contains("SlopTotal (offline)") && !offer.contains(key), "{offer}");
            let net = Fake { answers: RefCell::new([(status, body)].into()), ..Fake::default() };
            assert!(!block_on(check_key(&net, key)).contains(key));
        }
        // the debug log: size, status, time only
        let line = crate::journal::debug_line(true, 0.0, "gptzero-request", request_log(812, 200, 431.6), false, Some("The checked text.")).unwrap();
        assert!(line.contains("\"bytes\":812") && !line.contains(key) && !line.contains("checked text"), "{line}");
        // the history: the engine, the score, a hash and the first words; never the key; the text only if kept
        let run = crate::journal::Run { t: 0.0, note: "a.md".into(), kind: "paragraphs".into(), engine: ENGINE.into(), device: String::new(), helper: String::new(), items: vec![crate::journal::item("One two three four five six seven eight nine ten.", 99.7, false)] };
        let h = serde_json::to_string(&run).unwrap();
        assert!(h.contains("\"engine\":\"gptzero\"") && !h.contains(key) && !h.contains("nine ten"), "{h}");
    }

    /// AI Patterns, against a fake server: each pattern with its sentence and explanation; an error event, a refused
    /// key (not allow-listed) and an unreadable answer said plainly; the request holds the text alone.
    #[test]
    fn ai_patterns_are_read_from_the_stream() {
        let stream = concat!(
            ":keepalive\n\n",
            "data: {\"document_index\":0,\"sentence_index\":2,\"sentence\":\"It is fast, cheap and simple.\",\"interpretability_designation\":\"ai\",",
            "\"patterns\":[{\"domain_id\":\"ai-patterns\",\"pattern_id\":\"rule_of_three\",\"pattern_name\":\"Rule of three\",\"display_name\":\"Rule of three\",",
            "\"category\":\"Language and grammar\",\"description\":\"d\",\"display_description\":\"Lists of three.\",\"explanation\":\"Three adjectives in a row.\",\"relevance\":0.9,\"k_times\":3.2}]}\n\n",
            "data: {\"document_index\":0,\"sentence_index\":5,\"sentence\":\"In conclusion, it works.\",\"patterns\":[{\"pattern_name\":\"Signposting\",\"explanation\":\"Opens with a stock phrase.\",\"k_times\":null}]}\n\n");
        let net = Fake { answers: RefCell::new([(200, stream.to_string())].into()), ..Fake::default() };
        let got = block_on(patterns(&net, "k", "A paragraph  of text.")).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].sentence_index, got[0].name.as_str(), got[0].explanation.as_str(), got[0].k_times), (2, "Rule of three", "Three adjectives in a row.", Some(3.2)));
        assert_eq!((got[1].name.as_str(), got[1].k_times), ("Signposting", None), "the canonical name when no display name");
        assert_eq!((net.sent.borrow()[0].0.as_str(), net.sent.borrow()[0].2.as_str()), (PATTERNS, r#"{"documents":[{"inputText":"A paragraph of text."}]}"#));
        assert!(parse_patterns("data: {\"error\": \"Error in streaming AI patterns\"}\n").unwrap_err().contains("stopped"));
        let key = "gz-secret-0123456789";
        let net = Fake { answers: RefCell::new([(403, format!(r#"{{"error":"bad {key}"}}"#))].into()), ..Fake::default() };
        let e = block_on(patterns(&net, key, "Text.")).unwrap_err();
        assert!(e.contains("allow-listed") && !e.contains(key), "{e}");
        assert!(parse_patterns("not a stream").unwrap().is_empty());
    }

    #[test]
    fn the_key_is_checked_without_using_words() {
        let net = Fake { answers: RefCell::new([(200, r#"{"data":{"plan":"Professional","words_left":429569,"words_used":70431}}"#.into())].into()), ..Fake::default() };
        let t = block_on(check_key(&net, "k"));
        assert!(t.starts_with("The key works. Plan: Professional; 429569 words left"), "{t}");
        assert_eq!(net.sent.borrow()[0].0, USAGE);
        let net = Fake { answers: RefCell::new([(401, "{}".into())].into()), ..Fake::default() };
        assert!(block_on(check_key(&net, "k")).contains("expired"));
    }
}
