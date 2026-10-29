//! Model-free hints for a sentence: what in its wording, or in its paragraph, AI-text detectors are known to react to,
//! found by fixed rules (no model, no server, the same answer every time). Each hint names the habit, says what was
//! found and where (byte ranges in the sentence), and why a detector may react to it. They explain; they never suggest
//! or make a rewrite: the writer decides what, if anything, to change.
//!
//! The habits: words and phrases language models overuse (stock phrases), signposting openers ("Moreover,"), vague
//! wording (quantities and roles left unnamed), stacked hedges, em dashes, and sentences of very even length in the
//! paragraph (detectors measure how much sentence length varies, "burstiness").

use std::ops::Range;

/// One hint: its habit (a stable id and a label), what was found and why it matters to a detector, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub id: &'static str,
    pub label: &'static str,
    pub found: String,
    pub why: &'static str,
    /// Byte ranges in the sentence (empty for a hint about the paragraph).
    pub spans: Vec<Range<usize>>,
}

/// Words and phrases language models use far more than people do (lower case, whole words).
pub const STOCK: &[&str] = &[
    "delve", "delves", "delving", "tapestry", "pivotal", "underscore", "underscores", "underscoring", "seamless", "seamlessly", "leverage",
    "leveraging", "landscape", "realm", "multifaceted", "intricate", "intricacies", "holistic", "nuanced", "ever-evolving", "ever-changing",
    "shed light", "sheds light", "paving the way", "a testament to", "navigate the", "navigating the", "in today's", "game-changer",
    "game changer", "cutting-edge", "unlock the", "unlocking the", "harness the", "harnessing the", "embark", "embarking", "foster",
    "fostering", "elevate", "showcase", "showcasing", "commendable", "meticulous", "meticulously", "bustling", "vibrant", "profound",
    "it is important to note", "it is worth noting", "it's worth noting", "plays a crucial role", "plays a pivotal role", "plays a vital role",
    "in the realm of", "a myriad of", "myriad", "noteworthy",
];

/// Openers that announce the next sentence's role instead of making its point.
pub const SIGNPOSTS: &[&str] = &[
    "moreover", "furthermore", "additionally", "in addition", "in conclusion", "to conclude", "overall", "ultimately", "notably",
    "importantly", "consequently", "in summary", "to summarise", "to summarize", "in essence", "indeed",
];

/// Wording that leaves the quantity, the kind or the role unnamed.
pub const VAGUE: &[&str] = &[
    "various", "numerous", "a variety of", "a range of", "a wide range of", "a number of", "several aspects", "many aspects", "key factors",
    "various factors", "in many ways", "in various ways", "plays a role", "plays a key role", "plays a significant role", "a significant impact",
    "significant role", "it is important to", "it is essential to", "it is crucial to", "in today's world", "in recent years", "aspects of",
    "a key component", "an important aspect",
];

/// Hedges that stack with another in the same sentence ("may potentially").
pub const HEDGES: &[&str] = &["may", "might", "could", "potentially", "possibly", "perhaps", "arguably", "likely", "somewhat", "relatively"];

/// The fewest sentences a paragraph needs before evenness means anything, and the coefficient of variation of their
/// lengths (in words) below which it is called even.
pub const EVEN_MIN_SENTENCES: usize = 4;
pub const EVEN_CV: f64 = 0.25;

/// Where `phrase` (lower case) occurs as whole words in `lower` (the sentence lower-cased, same byte lengths assumed
/// only for ASCII; offsets are mapped back by `map`).
fn whole_words(lower: &str, phrase: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find(phrase) {
        let a = from + i;
        let b = a + phrase.len();
        let before = lower[..a].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = lower[b..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            out.push(a..b);
        }
        from = b;
    }
    out
}

/// The sentence lower-cased where that keeps every byte offset (ASCII letters only; anything else as it is).
fn lower(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii() { c.to_ascii_lowercase() } else { c }).collect::<String>().replace(['\u{2019}', '\u{2018}'], "'")
}

fn quote_list(found: &[String]) -> String {
    let mut f = found.to_vec();
    f.dedup();
    f.iter().map(|w| format!("\u{201c}{w}\u{201d}")).collect::<Vec<_>>().join(", ")
}

/// The words of a sentence (for its length).
fn words(s: &str) -> usize {
    s.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// Whether the sentences of a paragraph are of very even length: (lowest, highest, coefficient of variation).
pub fn evenness(sentences: &[&str]) -> Option<(usize, usize, f64)> {
    if sentences.len() < EVEN_MIN_SENTENCES {
        return None;
    }
    let lens: Vec<f64> = sentences.iter().map(|s| words(s) as f64).collect();
    let n = lens.len() as f64;
    let mean = lens.iter().sum::<f64>() / n;
    if mean <= 0.0 {
        return None;
    }
    let sd = (lens.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / n).sqrt();
    let cv = sd / mean;
    let (lo, hi) = lens.iter().fold((f64::MAX, 0.0f64), |(a, b), &l| (a.min(l), b.max(l)));
    (cv < EVEN_CV).then_some((lo as usize, hi as usize, cv))
}

/// The hints for `sentence`, one of the sentences of its paragraph (`paragraph`, all of them, the sentence among them).
#[allow(clippy::single_range_in_vec_init)] // a list of one span is meant
pub fn hints(sentence: &str, paragraph: &[&str]) -> Vec<Hint> {
    let l = lower(sentence);
    let byte_safe = l.len() == sentence.len();
    let mut out = Vec::new();
    let found_in = |list: &[&str]| -> (Vec<String>, Vec<Range<usize>>) {
        let (mut names, mut spans) = (Vec::new(), Vec::new());
        for w in list {
            let at = whole_words(&l, w);
            if !at.is_empty() {
                names.push(w.to_string());
                if byte_safe {
                    spans.extend(at);
                }
            }
        }
        // a phrase inside a longer one found too ("plays a role" in "plays a key role") is the same finding
        spans.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
        spans.dedup_by(|b, a| a.start <= b.start && b.end <= a.end);
        (names, spans)
    };

    let (stock, spans) = found_in(STOCK);
    if !stock.is_empty() {
        out.push(Hint { id: "stock-phrases", label: "Stock phrases", found: quote_list(&stock), why: "Language models use these words and phrases far more often than people do, and detectors that look for model vocabulary count them.", spans });
    }

    let opener = l.trim_start().trim_start_matches(['"', '\u{201c}', '(']);
    let lead = l.len() - opener.len();
    if let Some(w) = SIGNPOSTS.iter().find(|w| opener.starts_with(**w) && opener[w.len()..].starts_with([',', ' '])) {
        out.push(Hint { id: "signpost-opener", label: "Signposting opener", found: format!("opens with \u{201c}{}\u{201d}", &sentence[lead..lead + w.len()]), why: "Model-written prose often opens sentences with a connective that announces their role; detectors that read sentence openings react to it.", spans: if byte_safe { vec![lead..lead + w.len()] } else { Vec::new() } });
    }

    let (vague, spans) = found_in(VAGUE);
    if !vague.is_empty() {
        out.push(Hint { id: "vague-wording", label: "Vague wording", found: quote_list(&vague), why: "General words that leave the number, the kind or the role unnamed make text predictable; predictability (low perplexity) is what statistical detectors measure.", spans });
    }

    let hedges: Vec<Range<usize>> = HEDGES.iter().flat_map(|h| whole_words(&l, h)).collect();
    if hedges.len() >= 2 {
        let mut names: Vec<String> = hedges.iter().map(|r| l[r.clone()].to_string()).collect();
        names.sort();
        out.push(Hint { id: "stacked-hedges", label: "Stacked hedges", found: format!("{} hedges in one sentence: {}", hedges.len(), quote_list(&names)), why: "Several hedges on one claim are typical of model output, which qualifies statements by default.", spans: if byte_safe { hedges } else { Vec::new() } });
    }

    let dashes: Vec<Range<usize>> = sentence.match_indices('\u{2014}').map(|(i, d)| i..i + d.len()).collect();
    if !dashes.is_empty() {
        out.push(Hint { id: "em-dash", label: "Em dashes", found: format!("{} em dash{}", dashes.len(), if dashes.len() == 1 { "" } else { "es" }), why: "Models use the em dash (\u{2014}) much more than people typing do; some detectors and readers treat it as a sign.", spans: dashes });
    }

    if let Some((lo, hi, cv)) = evenness(paragraph) {
        out.push(Hint { id: "even-lengths", label: "Even sentence lengths", found: format!("the paragraph's {} sentences are all {lo} to {hi} words long (variation {:.0}%)", paragraph.len(), cv * 100.0), why: "People mix short and long sentences; detectors measure how much sentence length varies (\u{201c}burstiness\u{201d}), and little variation reads as model output.", spans: Vec::new() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(h: &[Hint]) -> Vec<&str> {
        h.iter().map(|h| h.id).collect()
    }

    #[test]
    fn finds_each_habit_with_its_place() {
        let s = "Moreover, this pivotal framework may potentially shed light on various factors \u{2014} a testament to the field.";
        let h = hints(s, &[s]);
        assert_eq!(ids(&h), ["stock-phrases", "signpost-opener", "vague-wording", "stacked-hedges", "em-dash"]);
        let stock = &h[0];
        assert_eq!(stock.found, "\u{201c}pivotal\u{201d}, \u{201c}shed light\u{201d}, \u{201c}a testament to\u{201d}");
        let marked: Vec<&str> = stock.spans.iter().map(|r| &s[r.clone()]).collect();
        assert_eq!(marked, ["pivotal", "shed light", "a testament to"]);
        assert_eq!(&s[h[1].spans[0].clone()], "Moreover");
        assert_eq!(h[3].found, "2 hedges in one sentence: \u{201c}may\u{201d}, \u{201c}potentially\u{201d}");
        assert_eq!(&s[h[4].spans[0].clone()], "\u{2014}");
    }

    #[test]
    fn whole_words_only_and_case_ignored() {
        assert!(hints("Robust and delivered results; the overallocation was fixed.", &[]).is_empty(), "delve in delivered, overall in overallocation");
        assert_eq!(ids(&hints("We DELVE into it.", &[])), ["stock-phrases"]);
        // a phrase inside a longer one: one span
        let s = "It plays a key role here.";
        let h = hints(s, &[]);
        assert_eq!(h[0].spans.iter().map(|r| &s[r.clone()]).collect::<Vec<_>>(), ["plays a key role"]);
    }

    #[test]
    fn even_lengths_need_enough_sentences() {
        let even = ["One two three four five six.", "Seven eight nine ten eleven twelve.", "A b c d e f.", "G h i j k l."];
        assert_eq!(evenness(&even).map(|e| (e.0, e.1)), Some((6, 6)));
        assert!(ids(&hints(even[0], &even)).contains(&"even-lengths"));
        let varied = ["Short.", "This one runs on for quite a lot longer than the first, with clauses.", "Mid length here, sort of.", "Yes."];
        assert!(evenness(&varied).is_none());
        assert!(evenness(&even[..3]).is_none(), "three sentences say nothing");
    }

    #[test]
    fn plain_specific_sentences_get_no_hints() {
        let s = "Calibrating on 200 indoor images clipped 4% of the brightest outdoor activations.";
        assert!(hints(s, &[s]).is_empty());
    }

    #[test]
    fn hints_never_suggest_a_rewrite() {
        // the texts explain; none tells the writer what to write instead
        let s = "Moreover, various pivotal factors may possibly matter \u{2014} indeed.";
        let even = [s, s, s, s];
        for h in hints(s, &even) {
            let text = format!("{} {}", h.found, h.why).to_lowercase();
            for advice in ["instead", "replace", "rewrite", "try ", "consider", "should", "use plainer", "say it"] {
                assert!(!text.contains(advice), "{}: {text}", h.id);
            }
        }
    }
}
