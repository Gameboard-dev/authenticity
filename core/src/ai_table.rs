//! The AI Detector pane's table as plain data (the pane draws it with DOM nodes): one row per checked paragraph, *My
//! paragraph* (its hints' words highlighted) | the score | the verdict, with one reliability note | why: each detector
//! of SlopTotal's full analysis as its own block, in the order it moved the score | the model-free hints; and, on
//! demand, one row per sentence with its score taken with its neighbours.
//!
//! **One scale everywhere**: AI-likeness from 0 (reads as written by a person) to 100 (reads as AI), SlopTotal's own
//! scale; up to 35 reads as human, above 65 as AI ([`LEGEND`]). A row's number is SlopTotal's quick check (the score the
//! tints show), said so; the full analysis that Load reasons runs gives the detectors' reasons on the same scale.
//!
//! Nothing here asks SlopTotal: the detectors' reasons and the sentences' scores are what was asked for before (the
//! pane's buttons ask), and the hints are fixed rules (hints.rs). Nothing here suggests a rewrite.

use std::ops::Range;

use crate::ai::{self, Scored};
use crate::sloptotal::{Analysis, Reason, Score, Weights, AI_MIN, CLEAN_MAX};

/// The pane's legend for the scale.
pub const LEGEND: &str = "AI-likeness, 0 to 100: 0 reads as written by a person, 100 as AI (up to 35 reads as human, above 65 as AI). A resemblance to model output, not proof of authorship. SlopTotal is tuned to rarely call human text AI, and by its own benchmark misses about a third of AI text: a high score means something, a low one is weak evidence.";
/// The hints per row, and the paragraph's text before it is cut.
pub const HINTS: usize = 4;
const SHORT: usize = 280;

/// One detector of the full analysis, as the pane shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Detector {
    /// SlopTotal's name, and the same in plain words.
    pub engine: String,
    pub plain: String,
    /// Its own result on the scale (0 to 100).
    pub score: f64,
    /// Its share of SlopTotal's weight for the text's language (0 to 1); None when it does not count for the language
    /// or the weights are unknown.
    pub share: Option<f64>,
    /// One plain sentence: what it measures, and what it found (SlopTotal's own words for the number), or that the model
    /// gives a probability only.
    pub says: String,
    /// How far it moved the score (its weight times its result's log-odds; 0 when it does not count).
    pub moved: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AiRow {
    /// The text, and as much of it as the cell shows, with the byte ranges (in `short`) of the words a hint found.
    pub text: String,
    pub short: String,
    pub marks: Vec<Range<usize>>,
    pub score: f64,
    /// "21 / 100"; the pill's class.
    pub label: String,
    pub class: String,
    /// The verdict in words and what the score is from ("Reads as written by a person · quick check").
    pub verdict: String,
    /// One reliability note, or empty.
    pub note: String,
    /// The full analysis's detectors that count, in the order they moved the score, then those that do not count for
    /// the language (empty: not asked for yet).
    pub detectors: Vec<Detector>,
    pub uncounted: Vec<Detector>,
    /// With the reasons: what the analysis says about detectors that did not run (empty when all ran), and the list.
    pub overall: String,
    pub failed: Vec<String>,
    /// Quick detectors that failed (empty if none did).
    pub caveat: String,
    pub hints: Vec<String>,
}

/// A score as the table gives it, on the one scale.
pub fn label(score: f64) -> String {
    format!("{score:.0} / 100")
}

/// The band a score falls in, in words.
pub fn band(score: f64) -> &'static str {
    if score > AI_MIN {
        "Reads as AI"
    } else if score <= CLEAN_MAX {
        "Reads as written by a person"
    } else {
        "Mixed"
    }
}

/// Which engine a score is from, as each row says.
pub fn engine_label(s: &Score) -> &'static str {
    if s.engine == crate::gptzero::ENGINE { "GPTZero" } else { "SlopTotal, quick check" }
}

/// A GPTZero score's sentences as rows: each with its own probability on the scale, the ones GPTZero highlights as
/// AI said so.
pub fn gptzero_sentence_rows(s: &Score) -> Vec<AiRow> {
    s.sentences
        .iter()
        .map(|x| AiRow {
            text: x.text.clone(),
            short: short(&x.text),
            marks: Vec::new(),
            score: x.p * 100.0,
            label: label(x.p * 100.0),
            class: ai::class(ai::tint(x.p * 100.0)),
            verdict: format!("{} \u{b7} GPTZero", band(x.p * 100.0)),
            note: if x.highlight { "GPTZero highlights it as AI.".into() } else { String::new() },
            detectors: Vec::new(),
            uncounted: Vec::new(),
            overall: String::new(),
            failed: Vec::new(),
            caveat: String::new(),
            hints: Vec::new(),
        })
        .collect()
}

/// The one reliability note for a score: a group's label, else a short text, else SlopTotal's low confidence.
pub fn note(text_words: usize, s: &Scored) -> String {
    if let Some(t) = &s.together {
        return format!("Less reliable: {}.", t.label);
    }
    if text_words < crate::sloptotal::SHORT_WORDS {
        return format!("Less reliable: under {} words.", crate::sloptotal::SHORT_WORDS);
    }
    if s.score.confidence == "low" {
        return "Less reliable: SlopTotal's confidence in it is low.".into();
    }
    String::new()
}

fn one_line(s: &str, n: usize) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out: String = line.chars().take(n).collect();
    if line.chars().count() > n {
        out.push('\u{2026}');
    }
    out
}

/// A detector in plain words: its name for a reader, what it measures, and whether it is a trained model that gives a
/// probability only (SlopTotal's classifiers). Detectors not listed keep SlopTotal's name and description.
fn plain(engine: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match engine {
        "Perplexity" => ("Predictability", "how predictable the wording is to a language model (GPT-2)", false),
        "Burstiness" => ("Sentence rhythm", "how much predictability varies from sentence to sentence", false),
        "Binoculars" => ("Two-model comparison", "how much more a small model is surprised by the text than a larger one", false),
        "Cross-Perplexity" => ("Predictability, two models", "how predictable the text is to two language models", false),
        "DivEye" => ("Surprise variation", "how evenly surprising the words are", false),
        "Fast-DetectGPT" => ("Likelihood curvature", "whether the wording sits at a peak of a language model's likelihood", false),
        "GLTR" => ("Word-choice rank", "how often the words are among a language model's top predictions", false),
        "Log-Rank" => ("Word rank", "the average rank of each word among a language model's predictions", false),
        "Formulaic Patterns" => ("Formulaic phrases", "set phrases and templates common in model output", false),
        "Linguistic Markers" => ("Stock words and markers", "words and constructions models use more than people", false),
        "Readability Uniformity" => ("Even readability", "how uniform readability is from sentence to sentence", false),
        "Sentiment & Hedging" => ("Tone and hedging", "the tone and the hedging of the sentences", false),
        "Structural Analysis" => ("Structure", "patterns in how sentences and paragraphs are built", false),
        "Vocabulary Richness" => ("Vocabulary range", "how varied the vocabulary is", false),
        "BERT-tiny RAID" => ("Classifier: BERT-tiny", "", true),
        "ChatGPT Detector" => ("Classifier: ChatGPT detector", "", true),
        "Desklib DeBERTa" => ("Classifier: Desklib DeBERTa", "", true),
        "E5-Small" => ("Classifier: E5-small", "", true),
        "Fakespot" => ("Classifier: Fakespot RoBERTa", "", true),
        "OpenAI Detector" => ("Classifier: OpenAI detector", "", true),
        "ReMoDetect" => ("Classifier: ReMoDetect", "", true),
        "SuperAnnotate" => ("Classifier: SuperAnnotate", "", true),
        "TMR Detector" => ("Classifier: TMR RoBERTa", "", true),
        _ => return None,
    })
}

/// A detector's block: plain name, its result, and one sentence (never a rewrite).
fn detector(r: &Reason, share: Option<f64>, moved: f64) -> Detector {
    let found = one_line(if r.details.is_empty() { &r.description } else { &r.details }, 200);
    let (name, says) = match plain(&r.engine) {
        Some((name, _, true)) => (name.to_string(), "A trained model: it gives a probability only, no reason.".to_string()),
        Some((name, what, false)) => (name.to_string(), if found.is_empty() { format!("Measures {what}.") } else { format!("Measures {what}. SlopTotal: {found}") }),
        None => (r.engine.clone(), found),
    };
    Detector { engine: r.engine.clone(), plain: name, score: r.score * 100.0, share, says, moved }
}

/// The analysis's detectors as blocks: those that count for the text's language, by how far they moved the score
/// (weight times the log-odds of their result, either way), each with its share of the weight; then those that do not
/// count. Without weights, all by their own results, with no share.
pub fn detectors(an: &Analysis, weights: Option<&Weights>) -> (Vec<Detector>, Vec<Detector>) {
    let logit = |p: f64| {
        let p = p.clamp(0.01, 0.99);
        (p / (1.0 - p)).ln()
    };
    match weights.and_then(|w| w.order(an)) {
        Some((counted, uncounted)) => {
            let total: f64 = counted.iter().map(|(_, w)| w.max(0.0)).sum();
            let mut c: Vec<Detector> = counted.iter().map(|(r, w)| detector(r, (total > 0.0).then(|| w.max(0.0) / total), w * logit(r.score))).collect();
            c.sort_by(|a, b| b.moved.abs().total_cmp(&a.moved.abs()));
            (c, uncounted.iter().map(|r| detector(r, None, 0.0)).collect())
        }
        None => {
            let mut c: Vec<Detector> = an.reasons.iter().map(|r| detector(r, None, 0.0)).collect();
            c.sort_by(|a, b| b.score.total_cmp(&a.score));
            (c, Vec::new())
        }
    }
}

/// The hints of a stretch of sentences of a paragraph (`only`: one sentence's, else all), one per kind, at most HINTS,
/// with the byte ranges (in the paragraph) of the words they found.
pub fn hints_of(paragraph: &str, only: Option<usize>) -> (Vec<String>, Vec<Range<usize>>) {
    let sents = ai::sentences(paragraph);
    let texts: Vec<&str> = sents.iter().map(|r| &paragraph[r.clone()]).collect();
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut marks: Vec<Range<usize>> = Vec::new();
    for (k, t) in texts.iter().enumerate() {
        if only.is_some_and(|o| o != k) {
            continue;
        }
        for h in crate::hints::hints(t, &texts) {
            marks.extend(h.spans.iter().map(|r| sents[k].start + r.start..sents[k].start + r.end));
            if !out.iter().any(|(id, _)| *id == h.id) {
                out.push((h.id, format!("{}: {}", h.label, h.found)));
            }
        }
    }
    marks.sort_by_key(|r| (r.start, r.end));
    (out.into_iter().take(HINTS).map(|(_, s)| s).collect(), marks)
}

fn short(t: &str) -> String {
    let mut s: String = t.chars().take(SHORT).collect();
    if t.chars().count() > SHORT {
        s.push('\u{2026}');
    }
    s
}

/// Marks (byte ranges of `text`) that fall inside the first `len` bytes (the cut text the cell shows).
fn within(marks: Vec<Range<usize>>, len: usize) -> Vec<Range<usize>> {
    marks.into_iter().filter(|r| r.end <= len).collect()
}

/// A checked paragraph's row.
pub fn paragraph_row(text: &str, s: &Scored, analysis: Option<&Analysis>, weights: Option<&Weights>) -> AiRow {
    let caveat = crate::sloptotal::quick_caveat(&s.score);
    let (hints, marks) = hints_of(text, None);
    let short = short(text);
    let cut = text.len().min(short.trim_end_matches('\u{2026}').len());
    let (detectors, uncounted) = analysis.map(|an| detectors(an, weights)).unwrap_or_default();
    AiRow {
        text: text.to_string(),
        marks: within(marks, cut),
        short,
        score: s.score.score,
        label: label(s.score.score),
        class: ai::class(ai::tint(s.score.score)),
        verdict: format!("{} \u{b7} {}", band(s.score.score), engine_label(&s.score)),
        note: note(text.split_whitespace().count(), s),
        detectors,
        uncounted,
        overall: analysis.filter(|an| !an.failed.is_empty()).map(crate::sloptotal::overall_line).unwrap_or_default(),
        failed: analysis.map(|an| an.failed.iter().map(did_not_run).collect()).unwrap_or_default(),
        caveat,
        hints,
    }
}

/// A detector that did not run, with SlopTotal's reason.
pub fn did_not_run(f: &crate::sloptotal::Failed) -> String {
    if f.details.is_empty() { format!("Did not run: {}", f.engine) } else { format!("Did not run: {} ({})", f.engine, one_line(&f.details, 140)) }
}

/// A paragraph's sentences as rows: each with its score taken with its neighbours, when it was asked for (`scored`:
/// by the text scored), and its hints. `None` for a sentence too short to score even with the whole paragraph.
pub fn sentence_rows(paragraph: &str, scored: impl Fn(&str) -> Option<Score>) -> Vec<(AiRow, Option<String>)> {
    let sents = ai::sentences(paragraph);
    sents
        .iter()
        .enumerate()
        .map(|(k, r)| {
            let text = paragraph[r.clone()].to_string();
            let ctx = ai::context(paragraph, &sents, k);
            let s = ctx.as_deref().and_then(&scored);
            let (hints, marks) = hints_of(paragraph, Some(k));
            let short = short(&text);
            let cut = text.len().min(short.trim_end_matches('\u{2026}').len());
            let row = AiRow {
                marks: within(marks.into_iter().map(|m| m.start - r.start..m.end - r.start).collect(), cut),
                short,
                score: s.as_ref().map_or(f64::NAN, |s| s.score),
                label: s.as_ref().map_or_else(|| "\u{2014}".to_string(), |s| label(s.score)),
                class: s.as_ref().map_or_else(String::new, |s| ai::class(ai::tint(s.score))),
                verdict: s.as_ref().map_or_else(String::new, |s| format!("{} \u{b7} quick check", band(s.score))),
                note: if s.is_some() { "Less reliable: a sentence, scored with its neighbours.".into() } else { String::new() },
                detectors: Vec::new(),
                uncounted: Vec::new(),
                overall: String::new(),
                failed: Vec::new(),
                caveat: s.as_ref().map_or_else(String::new, crate::sloptotal::quick_caveat),
                hints,
                text,
            };
            (row, ctx)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(x: f64, v: &str) -> Score {
        Score { score: x, verdict: v.into(), confidence: "high".into(), ..Score::default() }
    }

    fn scored(s: Score) -> Scored {
        Scored { score: s, when: 0.0, device: String::new(), together: None }
    }

    #[test]
    fn one_scale_and_one_note() {
        assert_eq!(label(82.4), "82 / 100");
        assert_eq!(label(7.0), "7 / 100");
        assert_eq!((band(82.0), band(20.0), band(50.0)), ("Reads as AI", "Reads as written by a person", "Mixed"));
        let long = 100;
        assert_eq!(note(long, &scored(score(20.0, "clean"))), "");
        assert_eq!(note(long, &scored(Score { confidence: "low".into(), ..score(20.0, "clean") })), "Less reliable: SlopTotal's confidence in it is low.");
        // a short text says that, and only that, even with low confidence
        assert_eq!(note(30, &scored(Score { confidence: "low".into(), ..score(20.0, "clean") })), "Less reliable: under 80 words.");
        let grouped = Scored { together: Some(crate::ai::Together { span: vec![], label: "scored together with the next paragraph (too short alone)".into() }), ..scored(score(20.0, "clean")) };
        assert_eq!(note(30, &grouped), "Less reliable: scored together with the next paragraph (too short alone).");
    }

    #[test]
    fn detectors_as_blocks_in_the_order_they_moved_the_score() {
        let p = "It is important to note that the results are robust. Moreover, the method is efficient. Furthermore, it is simple.";
        let s = scored(score(78.0, "ai"));
        let r = |e: &str, x: f64, d: &str| Reason { engine: e.into(), score: x, verdict: String::new(), details: d.into(), description: String::new() };
        let an = Analysis { overall: 78.0, language: "en".into(), reasons: vec![r("Burstiness", 0.9, "Sentence perplexity CV 0.12"), r("Fakespot", 0.6, "AI probability: 60.0% (Fakespot APOLLO RoBERTa)"), r("E5-Small", 0.93, "AI probability: 93.0%")], ..Analysis::default() };
        let mut w = Weights::default();
        w.english.insert("Burstiness".into(), 0.1);
        w.english.insert("Fakespot".into(), 0.3);
        w.english.insert("E5-Small".into(), 0.0);
        let row = paragraph_row(p, &s, Some(&an), Some(&w));
        assert_eq!((row.label.as_str(), row.class.as_str(), row.verdict.as_str()), ("78 / 100", "auth-ai-2", "Reads as AI \u{b7} SlopTotal, quick check"));
        // Burstiness moved it more (0.1 x logit 0.9 = 0.22) than Fakespot (0.3 x logit 0.6 = 0.12)
        assert_eq!(row.detectors.iter().map(|d| d.plain.as_str()).collect::<Vec<_>>(), ["Sentence rhythm", "Classifier: Fakespot RoBERTa"]);
        assert_eq!(row.detectors[0].share, Some(0.25));
        assert_eq!(row.detectors[0].says, "Measures how much predictability varies from sentence to sentence. SlopTotal: Sentence perplexity CV 0.12");
        assert_eq!(row.detectors[1].says, "A trained model: it gives a probability only, no reason.");
        assert_eq!(row.uncounted.len(), 1, "E5 does not count for English");
        assert_eq!(row.overall, "", "all ran: nothing to say");
        // the hints' words are marked in the text
        assert!(!row.hints.is_empty() && !row.marks.is_empty(), "{:?}", row.hints);
        assert!(row.marks.iter().all(|m| p.get(m.clone()).is_some()));
        assert!(paragraph_row(p, &s, None, None).detectors.is_empty(), "not asked for yet");
        // a detector that did not run: listed, and the message says it is left out
        let failed = Analysis { failed: vec![crate::sloptotal::Failed { engine: "binoculars".into(), details: "Engine error: OverflowError".into() }], ..an.clone() };
        let row = paragraph_row(p, &s, Some(&failed), Some(&w));
        assert_eq!(row.failed, ["Did not run: binoculars (Engine error: OverflowError)"]);
        assert!(row.overall.starts_with("1 detector did not run (binoculars)."), "{}", row.overall);
    }

    #[test]
    fn a_gptzero_row_says_so_and_lists_its_sentences() {
        let s = Score { score: 99.7, engine: "gptzero".into(), sentences: vec![crate::gptzero::Sentence { text: "It is.".into(), p: 0.98, highlight: true }], ..score(99.7, "ai") };
        let row = paragraph_row("A paragraph of a hundred words would be here.", &scored(s.clone()), None, None);
        assert_eq!((row.label.as_str(), row.verdict.as_str()), ("100 / 100", "Reads as AI \u{b7} GPTZero"));
        let rows = gptzero_sentence_rows(&s);
        assert_eq!((rows[0].label.as_str(), rows[0].note.as_str()), ("98 / 100", "GPTZero highlights it as AI."));
        assert_eq!(paragraph_row("x", &scored(score(20.0, "clean")), None, None).verdict, "Reads as written by a person \u{b7} SlopTotal, quick check");
    }

    #[test]
    fn sentence_rows_on_demand() {
        let p = "The first sentence is long enough to be scored on its own. The second one too, with a few more words in it. And a third sentence closes it.";
        let rows = sentence_rows(p, |ctx| (!ctx.contains("third")).then(|| score(30.0, "clean")));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].0.text, "The first sentence is long enough to be scored on its own.");
        assert!(rows[0].1.is_some(), "the text it is scored with");
        assert_eq!(rows[0].0.label, "30 / 100");
        assert_eq!(rows[0].0.note, "Less reliable: a sentence, scored with its neighbours.");
        assert_eq!(rows[1].0.label, "\u{2014}", "not scored yet");
    }
}
