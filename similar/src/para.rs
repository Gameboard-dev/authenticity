//! When a sentence is a close paraphrase of a library sentence, by its content words alone (no language model): the
//! words of a text, their stems, and the rule the plagiarism check applies to the library sentences the index's word
//! search finds (index.rs, `Index::para_plan`).
//!
//! A sentence that says what a source says is not plagiarism by itself: a literature review restates its sources, with
//! citations, all the time. What makes a restatement a *close* paraphrase (patchwriting) is that it keeps the source's
//! own words in the source's order with a few swapped. So a sentence is flagged when, against one library sentence:
//!
//! - it keeps at least `closeness` (default 0.5) of its content words in that sentence's order (the longest common
//!   subsequence of their stems, over the sentence's content words), and
//! - the words it shares with that sentence carry at least [`WEIGHT`] of its content words' weight, and at least
//!   [`MIN_SHARED`] in all, each word weighed by how rare it is in the library (its inverse document frequency): sharing
//!   "results", "method" and "data" in order is common ground, sharing the source's particular terms is not, and a
//!   fragment of a few common words does not make a case however much of it is shared; and
//! - it has at least [`MIN_CONTENT`] content words (shorter sentences share half their words by chance).
//!
//! Calibrated on 2026-10-29 on a review chapter (126 sentences judged) against an 821-note library (192,279 passages):
//! its two known close paraphrases keep 60% (weight 68%, 49.6 shared) and 50% (weight 49%, 20.5 shared) of their
//! content words; the gates were set to keep both with the fewest other flags (a stray fragment of six common words
//! kept 67% with 62% of its weight but only 18.6 shared). The margins are small.

/// The least share of a sentence's content-word weight (inverse document frequency) found in the source sentence.
pub const WEIGHT: f64 = 0.45;
/// The least weight the shared words carry in all (inverse document frequency summed: about four of the sentence's
/// rarer words).
pub const MIN_SHARED: f64 = 20.0;
/// The fewest content words a sentence needs to be judged.
pub const MIN_CONTENT: usize = 6;
/// The default share of a sentence's content words kept in the source's order, and the range the settings and the
/// Plagiarism pane offer.
pub const CLOSENESS: f64 = 0.5;
pub const MIN_CLOSENESS: f64 = 0.3;
pub const MAX_CLOSENESS: f64 = 0.9;

/// Words that carry no content of their own.
const FUNCTION: &[&str] = &[
    "a", "an", "the", "of", "and", "or", "but", "nor", "for", "to", "in", "on", "at", "by", "with", "from", "as", "into", "onto", "than", "then",
    "that", "this", "these", "those", "it", "its", "is", "are", "was", "were", "be", "been", "being", "has", "have", "had", "do", "does", "did",
    "not", "no", "so", "such", "which", "who", "whom", "whose", "what", "when", "where", "while", "how", "why", "their", "them", "they", "there",
    "our", "we", "us", "i", "you", "your", "he", "she", "his", "her", "him", "also", "only", "both", "each", "either", "more", "most", "other",
    "same", "some", "any", "all", "can", "could", "may", "might", "must", "shall", "should", "will", "would", "one", "two", "via", "per", "upon",
    "about", "over", "under", "between", "among", "within", "without", "against", "through", "during", "before", "after", "above", "below", "up",
    "down", "out", "off", "again", "further", "once", "here", "very", "just",
];

/// A word's stem for comparing: lower case, British and American spellings made one (-isation, -ise, -yse), and a
/// common ending taken off a long enough word.
pub fn stem(w: &str) -> String {
    let mut s = w.to_lowercase().replace('\u{2019}', "'").replace("isation", "ization").replace("ise", "ize").replace("yse", "yze");
    if s.ends_with("'s") {
        s.truncate(s.len() - 2);
    }
    for end in ["ations", "ation", "ings", "ing", "edly", "ed", "ies", "es", "s", "ly"] {
        if s.chars().count() > end.len() + 3 && s.ends_with(end) {
            s.truncate(s.len() - end.len());
            break;
        }
    }
    s
}

/// The words of a text: runs of letters and digits, joined across a hyphen or an apostrophe inside a word.
pub fn words(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let cs: Vec<(usize, char)> = t.char_indices().collect();
    for (k, &(i, c)) in cs.iter().enumerate() {
        let inner = matches!(c, '-' | '\'' | '\u{2019}') && start.is_some() && cs.get(k + 1).is_some_and(|n| n.1.is_alphanumeric());
        if c.is_alphanumeric() || inner {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push(&t[s..i]);
        }
    }
    if let Some(s) = start {
        out.push(&t[s..]);
    }
    out
}

/// Whether a word carries content: not a function word, not one letter.
pub fn is_content(w: &str) -> bool {
    w.chars().count() > 1 && !FUNCTION.contains(&w.to_lowercase().as_str())
}

/// The content words of a text (no function words, none of one letter), as stems, in order.
pub fn content(t: &str) -> Vec<String> {
    words(t).into_iter().filter(|w| is_content(w)).map(stem).collect()
}

/// The share of `mine` (a sentence's content stems) found in `source` in the same order.
pub fn in_order(mine: &[String], source: &[String]) -> f64 {
    if mine.is_empty() || source.is_empty() {
        return 0.0;
    }
    let mut prev = vec![0usize; source.len() + 1];
    for x in mine {
        let mut cur = vec![0usize; source.len() + 1];
        for (j, y) in source.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { prev[j + 1].max(cur[j]) };
        }
        prev = cur;
    }
    prev[source.len()] as f64 / mine.len() as f64
}

/// A library passage's sentences (cut after `.`, `!` or `?` and white space before a capital, a digit or an opening
/// bracket, and at line breaks), those of three words or more.
pub fn passage_sentences(p: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let cs: Vec<(usize, char)> = p.char_indices().collect();
    let mut start = 0;
    for (k, &(i, c)) in cs.iter().enumerate() {
        let end = if c == '\n' {
            Some((i, i + 1))
        } else if matches!(c, '.' | '!' | '?') && cs.get(k + 1).is_some_and(|n| n.1 == ' ') && cs.get(k + 2).is_some_and(|n| n.1.is_uppercase() || n.1.is_ascii_digit() || matches!(n.1, '(' | '[')) {
            Some((i + 1, i + 1))
        } else {
            None
        };
        if let Some((e, next)) = end {
            out.push(p[start..e].trim());
            start = next;
        }
    }
    out.push(p[start..].trim());
    out.into_iter().filter(|s| words(s).len() >= 3).collect()
}

/// The closest any sentence of `passage` comes to `mine` in kept words (in_order), and that sentence.
pub fn closest<'a>(mine: &[String], passage: &'a str) -> (f64, &'a str) {
    passage_sentences(passage).into_iter().map(|s| (in_order(mine, &content(s)), s)).fold((0.0, ""), |best, x| if x.0 > best.0 { x } else { best })
}

/// The rule (see the module's notes): a close paraphrase when it keeps `closeness` of its words in the source
/// sentence's order and the words shared carry `WEIGHT` of their weight, and `MIN_SHARED` in all (`shared`).
pub fn close(mine: &[String], kept: f64, weight: f64, shared: f64, closeness: f64) -> bool {
    mine.len() >= MIN_CONTENT && kept >= closeness && weight >= WEIGHT && shared >= MIN_SHARED
}

/// A stem's key in the index's word postings (the top 32 bits of its FNV-1a hash).
pub fn stem_key(stem: &str) -> u32 {
    (crate::library::fnv(stem.as_bytes()) >> 32) as u32
}

/// The index's keys for a passage: its content words' stems, each once, sorted.
pub fn word_keys(passage: &str) -> Vec<u32> {
    let mut out: Vec<u32> = content(passage).iter().map(|s| stem_key(s)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The share of `mine`'s weight (each distinct stem weighed by `weigh`) carried by the stems also in `source`.
pub fn weighted_share(mine: &[String], source: &[String], weigh: impl Fn(&str) -> f64) -> f64 {
    let mut seen: Vec<&str> = mine.iter().map(String::as_str).collect();
    seen.sort_unstable();
    seen.dedup();
    let total: f64 = seen.iter().map(|s| weigh(s)).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let shared: f64 = seen.iter().filter(|s| source.iter().any(|t| t == *s)).map(|s| weigh(s)).sum();
    shared / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_join_spellings_and_endings() {
        assert_eq!(stem("Quantisation"), stem("quantization"));
        assert_eq!(stem("analysed"), stem("analyzed"));
        assert_eq!(stem("networks"), "network");
        assert_eq!(stem("predicting"), stem("predicts"));
        assert_eq!(stem("is"), "is", "short words keep their ending");
    }

    #[test]
    fn words_and_content() {
        assert_eq!(words("state-of-the-art, it's 3x (two)"), ["state-of-the-art", "it's", "3x", "two"]);
        assert_eq!(content("The model's errors are reported in a table"), ["model", "error", "report", "table"]);
    }

    #[test]
    fn kept_in_order() {
        let mine = content("Error correction is weakened when the errors are correlated, that is, when many bit positions are wrong at the same time.");
        let source = content("Error correction is weakened if the errors are correlated, in other words, if many bit positions are simultaneously incorrect.");
        let kept = in_order(&mine, &source);
        assert!(kept >= 0.75, "{kept}");
        let own = content("Errors that occur together, flipping several bits at once, undermine the code's ability to fix them.");
        assert!(in_order(&own, &source) < 0.35);
        // order counts: the same words shuffled keep less
        let a: Vec<String> = ["alpha", "beta", "gamma", "delta"].map(String::from).to_vec();
        let b: Vec<String> = ["delta", "gamma", "beta", "alpha"].map(String::from).to_vec();
        assert_eq!(in_order(&a, &a), 1.0);
        assert_eq!(in_order(&a, &b), 0.25);
        assert_eq!(in_order(&[], &a), 0.0);
    }

    #[test]
    fn passages_are_cut_into_sentences_and_the_closest_found() {
        let p = "We describe the setting first. The proposed sampling procedure reduces the variance considerably.\nA table follows (Table 2).\nx y";
        assert_eq!(passage_sentences(p), ["We describe the setting first.", "The proposed sampling procedure reduces the variance considerably.", "A table follows (Table 2)."]);
        let (kept, s) = closest(&content("Our proposed sampling procedure reduces variance a great deal."), p);
        assert!(s.starts_with("The proposed") && kept > 0.6, "{kept} {s}");
    }

    #[test]
    fn the_rule() {
        let six = content("one alpha beta gamma delta epsilon zeta");
        assert_eq!(six.len(), 6);
        assert!(close(&six, 0.5, WEIGHT, MIN_SHARED, CLOSENESS));
        assert!(!close(&six, 0.49, 0.9, 40.0, CLOSENESS), "too few words kept");
        assert!(!close(&six, 0.9, WEIGHT - 0.01, 40.0, CLOSENESS), "only common words shared");
        assert!(!close(&six, 0.9, 0.9, MIN_SHARED - 0.1, CLOSENESS), "a fragment of common words");
        assert!(!close(&six[..5], 0.9, 0.9, 40.0, CLOSENESS), "too short to judge");
    }

    #[test]
    fn rare_words_weigh_more() {
        let mine = content("The sampling procedure reduces estimator variance in the results");
        let source = content("Results show the method reduces the time");
        let weigh = |s: &str| if ["result", "reduc"].contains(&s) { 0.5 } else { 4.0 };
        let w = weighted_share(&mine, &source, weigh);
        assert!(w > 0.0 && w < 0.1, "only common words shared: {w}");
        assert_eq!(weighted_share(&mine, &mine, weigh), 1.0);
        assert_eq!(word_keys("alpha beta alpha"), {
            let mut k = vec![stem_key("alpha"), stem_key("beta")];
            k.sort_unstable();
            k
        });
    }
}
