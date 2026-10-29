//! The Plagiarism pane's comparison table as plain data (the pane draws it with DOM nodes, never as HTML): one row per
//! finding that passes the thresholds, *My sentence* beside *Similar source sentence*, with the words they share
//! marked in both, the source (its title, page and the line to open), the kind and the closeness.
//!
//! The words marked are those the two have in common in the same order (the longest common subsequence): for a
//! verbatim or near-verbatim run, every word of the run that is the source's (case and punctuation aside); for a close
//! paraphrase, the content words it keeps in the source sentence's order (the words the closeness counts), so the
//! correspondence can be seen.

use std::ops::Range;

use crate::para;
use crate::similarity::Match;

/// A piece of a cell's text: shared with the other cell or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub shared: bool,
}

/// One row of the table.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub mine: Vec<Seg>,
    pub source: Vec<Seg>,
    /// The source as a reader knows it (citation key › title), its page when known, and the library note to open at
    /// `line` (from 0, as Obsidian's editor counts lines).
    pub title: String,
    pub page: Option<u32>,
    pub note: String,
    pub line: Option<u32>,
    pub kind: &'static str,
    /// The closeness in percent, and what it means for this row.
    pub closeness: u32,
    pub tip: String,
    /// Where the finding was in the note when checked (UTF-16) and its words, to find it again after edits.
    pub from: u32,
    pub to: u32,
    pub text: String,
}

/// The table's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Closeness,
    Position,
}

impl Sort {
    pub fn parse(s: &str) -> Sort {
        if s == "position" { Sort::Position } else { Sort::Closeness }
    }

    pub fn key(self) -> &'static str {
        match self {
            Sort::Closeness => "closeness",
            Sort::Position => "position",
        }
    }
}

/// The most words of a cell compared (a sentence is far shorter; the cap only bounds the work).
const MAX_WORDS: usize = 400;

/// A word's byte range in `t` and what it is compared by (None: not compared, a function word in content mode).
fn keyed(t: &str, region: &Range<usize>, content_only: bool) -> Vec<(Range<usize>, Option<String>)> {
    let base = t.as_ptr() as usize;
    para::words(t)
        .into_iter()
        .take(MAX_WORDS * 4)
        .map(|w| {
            let at = w.as_ptr() as usize - base;
            let r = at..at + w.len();
            let inside = r.start >= region.start && r.end <= region.end;
            let key = if !inside {
                None
            } else if content_only {
                para::is_content(w).then(|| para::stem(w))
            } else {
                Some(w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect())
            };
            (r, key)
        })
        .collect()
}

/// The words `a` and `b` share in the same order (the longest common subsequence of their keys), as byte ranges of
/// each, words next to each other joined into one range.
pub fn shared(a: &str, a_in: Range<usize>, b: &str, b_in: Range<usize>, content_only: bool) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let wa = keyed(a, &a_in, content_only);
    let wb = keyed(b, &b_in, content_only);
    // the compared words only, keeping their place among all the words (so neighbours can be joined)
    let ia: Vec<usize> = (0..wa.len()).filter(|&i| wa[i].1.is_some()).take(MAX_WORDS).collect();
    let ib: Vec<usize> = (0..wb.len()).filter(|&i| wb[i].1.is_some()).take(MAX_WORDS).collect();
    let (n, m) = (ia.len(), ib.len());
    let mut t = vec![0u16; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * (m + 1) + j] = if wa[ia[i]].1 == wb[ib[j]].1 { t[(i + 1) * (m + 1) + j + 1] + 1 } else { t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let (mut pa, mut pb) = (Vec::new(), Vec::new());
    while i < n && j < m {
        if wa[ia[i]].1 == wb[ib[j]].1 {
            pa.push(ia[i]);
            pb.push(ib[j]);
            i += 1;
            j += 1;
        } else if t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (join(&wa, &pa), join(&wb, &pb))
}

/// The ranges of the words `picked` (places among `words`, ascending), neighbours joined.
fn join(words: &[(Range<usize>, Option<String>)], picked: &[usize]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut last: Option<usize> = None;
    for &k in picked {
        let r = words[k].0.clone();
        match (last, out.last_mut()) {
            (Some(l), Some(prev)) if l + 1 == k => prev.end = r.end,
            _ => out.push(r),
        }
        last = Some(k);
    }
    out
}

/// A cell's text cut into pieces at `spans` (byte ranges, ascending, not overlapping).
pub fn segments(text: &str, spans: &[Range<usize>]) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut at = 0;
    for r in spans {
        if r.start < at || r.end > text.len() || !text.is_char_boundary(r.start) || !text.is_char_boundary(r.end) {
            continue;
        }
        if r.start > at {
            out.push(Seg { text: text[at..r.start].to_string(), shared: false });
        }
        out.push(Seg { text: text[r.clone()].to_string(), shared: true });
        at = r.end;
    }
    if at < text.len() {
        out.push(Seg { text: text[at..].to_string(), shared: false });
    }
    out
}

/// The kind as the table names it.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "verbatim" => "Verbatim",
        "near_verbatim" => "Near-verbatim",
        "paraphrase" => "Close paraphrase",
        _ => "Fixed phrasing",
    }
}

/// A finding as a row.
pub fn row(m: &Match) -> Row {
    let para = m.kind == "paraphrase";
    let (mine, ma, mb) = if para { (m.text.as_str(), 0, m.text.len()) } else { m.in_sentence() };
    let (source, sa, sb) = if para { (m.source_text.as_str(), 0, m.source_text.len()) } else { m.in_source_sentence() };
    let (x, y) = shared(mine, ma..mb, source, sa..sb, para);
    let closeness = match m.kind.as_str() {
        "verbatim" => 100,
        _ => (m.score * 100.0).round().clamp(0.0, 100.0) as u32,
    };
    let tip = match m.kind.as_str() {
        "verbatim" => format!("The same {} words in a row as the source (case and punctuation aside).", m.run_words()),
        "near_verbatim" => format!("{closeness}% of this stretch's words are the source's, in its order, with a few words changed, added or left out ({} words the same).", m.run_words()),
        _ => format!(
            "{closeness}% of your sentence's content words (words other than the, of, and \u{2026}) are found in the source sentence in the same order, and the two say the same (similarity {:.2}).",
            m.cosine.unwrap_or(0.0)
        ),
    };
    Row {
        mine: segments(mine, &x),
        source: segments(source, &y),
        title: if m.title.is_empty() { m.source.clone() } else { m.title.clone() },
        page: m.page,
        note: m.source.clone(),
        line: m.line,
        kind: kind_label(&m.kind),
        closeness,
        tip,
        from: m.from,
        to: m.to,
        text: m.text.clone(),
    }
}

/// The table's rows for a note's findings (fixed phrasing left out: it is listed apart), in `sort`'s order: closest
/// first (then by place), or by place in the note.
pub fn rows(found: &[Match], sort: Sort) -> Vec<Row> {
    let mut out: Vec<Row> = found.iter().filter(|m| m.kind != "common").map(row).collect();
    match sort {
        Sort::Closeness => out.sort_by(|a, b| b.closeness.cmp(&a.closeness).then(a.from.cmp(&b.from))),
        Sort::Position => out.sort_by_key(|r| (r.from, r.to)),
    }
    out
}

/// Whether a row's place in the note, [from, to), meets `at`: a selection, or the cursor (from == to), which meets
/// the row it is in or at either end of.
pub fn meets(row: (u32, u32), at: (u32, u32)) -> bool {
    if at.0 == at.1 { row.0 <= at.0 && at.0 <= row.1 } else { row.0 < at.1 && at.0 < row.1 }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The count line: "2 close paraphrases, 4 verbatim runs in 126 sentences".
pub fn summary(found: &[Match], sentences: usize, selection: bool) -> String {
    let n = |k: &str| found.iter().filter(|m| m.kind == k).count();
    let parts: Vec<String> = [
        (n("paraphrase"), "close paraphrase", "close paraphrases"),
        (n("verbatim"), "verbatim run", "verbatim runs"),
        (n("near_verbatim"), "near-verbatim run", "near-verbatim runs"),
    ]
    .into_iter()
    .filter(|p| p.0 > 0)
    .map(|(k, one, many)| count(k, one, many))
    .collect();
    let head = match parts.len() {
        0 => "No wording shared with the library and no close paraphrase".to_string(),
        1 => parts[0].clone(),
        _ => format!("{} and {}", parts[..parts.len() - 1].join(", "), parts[parts.len() - 1]),
    };
    match (sentences, selection) {
        (0, _) => head,
        (s, false) => format!("{head} in {}", count(s, "sentence", "sentences")),
        (s, true) => format!("{head} in the {} of the selection checked", count(s, "sentence", "sentences")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(segs: &[Seg]) -> Vec<&str> {
        segs.iter().filter(|s| s.shared).map(|s| s.text.as_str()).collect()
    }

    fn whole(segs: &[Seg]) -> String {
        segs.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn a_verbatim_row_marks_the_run_in_both_sentences() {
        let mine = "As shown before, the proposed sampling procedure reduces the variance of every estimator here.";
        let src = "Earlier studies reported that the Proposed sampling procedure reduces the variance of every estimator considerably.";
        let text = "the proposed sampling procedure reduces the variance of every estimator";
        let st = "the Proposed sampling procedure reduces the variance of every estimator";
        let m = Match {
            kind: "verbatim".into(), text: text.into(), from: 17, to: 88, score: 1.0, matched: 10, title: "a2020 \u{203a} Paper A".into(), source: "Lib/a2020/Paper A.md".into(),
            source_text: st.into(), line: Some(3), sentence: mine.into(), sentence_at: mine.find(text).unwrap(), source_sentence: src.into(), source_at: src.find(st).unwrap(), ..Match::default()
        };
        let r = row(&m);
        assert_eq!(whole(&r.mine), mine, "the cell is the whole sentence");
        assert_eq!(whole(&r.source), src);
        assert_eq!(marked(&r.mine), [text], "the run, as one piece");
        assert_eq!(marked(&r.source), [st], "case aside");
        assert_eq!((r.kind, r.closeness, r.note.as_str(), r.line, r.title.as_str()), ("Verbatim", 100, "Lib/a2020/Paper A.md", Some(3), "a2020 \u{203a} Paper A"));
        assert!(r.tip.starts_with("The same 10 words"), "{}", r.tip);
        // the words outside the run are never marked, even where they recur ("the")
        assert!(!r.mine[0].shared && r.mine[0].text == "As shown before, ");
    }

    #[test]
    fn a_paraphrase_row_marks_the_content_words_kept_in_order() {
        let mine = "The suggested sampling method lowers the variance of each estimator considerably across all benchmarks.";
        let src = "The proposed sampling procedure reduces the variance of every estimator considerably on all benchmarks.";
        let m = Match { kind: "paraphrase".into(), text: mine.into(), score: 0.56, cosine: Some(0.86), source_text: src.into(), page: Some(4), ..Match::default() };
        let r = row(&m);
        assert_eq!(marked(&r.mine), ["sampling", "variance", "estimator considerably", "benchmarks"]);
        assert_eq!(marked(&r.source), ["sampling", "variance", "estimator considerably", "benchmarks"]);
        assert_eq!((r.kind, r.closeness, r.page), ("Close paraphrase", 56, Some(4)));
        assert!(r.tip.contains("56% of your sentence's content words") && r.tip.contains("0.86"), "{}", r.tip);
        assert_eq!(whole(&r.mine), mine);
    }

    #[test]
    fn near_verbatim_marks_only_the_words_the_same() {
        let a = "we found that the error rate rises sharply with the length of the input";
        let b = "they found that the error rate grows sharply with the length of the input";
        let (x, y) = shared(a, 0..a.len(), b, 0..b.len(), false);
        let xs: Vec<&str> = x.iter().map(|r| &a[r.clone()]).collect();
        let ys: Vec<&str> = y.iter().map(|r| &b[r.clone()]).collect();
        assert_eq!(xs, ["found that the error rate", "sharply with the length of the input"]);
        assert_eq!(ys, xs);
    }

    #[test]
    fn rows_sort_and_the_count_line() {
        let f = |kind: &str, from: u32, score: f64| Match { kind: kind.into(), text: "x y".into(), from, to: from + 3, score, ..Match::default() };
        let found = vec![f("paraphrase", 50, 0.55), f("verbatim", 10, 1.0), f("near_verbatim", 30, 0.8), f("common", 5, 1.0), f("paraphrase", 70, 0.62)];
        let by_closeness: Vec<(u32, u32)> = rows(&found, Sort::Closeness).iter().map(|r| (r.from, r.closeness)).collect();
        assert_eq!(by_closeness, [(10, 100), (30, 80), (70, 62), (50, 55)], "fixed phrasing is not a row");
        let by_place: Vec<u32> = rows(&found, Sort::Position).iter().map(|r| r.from).collect();
        assert_eq!(by_place, [10, 30, 50, 70]);
        assert_eq!(summary(&found, 126, false), "2 close paraphrases, 1 verbatim run and 1 near-verbatim run in 126 sentences");
        assert_eq!(summary(&found[..2], 1, false), "1 close paraphrase and 1 verbatim run in 1 sentence");
        assert_eq!(summary(&found[3..4], 12, true), "No wording shared with the library and no close paraphrase in the 12 sentences of the selection checked");
        assert_eq!(summary(&found[1..2], 0, false), "1 verbatim run");
        assert_eq!((Sort::parse("position"), Sort::parse("anything"), Sort::Position.key()), (Sort::Position, Sort::Closeness, "position"));
    }

    #[test]
    fn a_selection_or_the_cursor_meets_a_row() {
        assert!(meets((10, 20), (15, 15)) && meets((10, 20), (20, 20)) && !meets((10, 20), (21, 21)), "the cursor");
        assert!(meets((10, 20), (0, 11)) && meets((10, 20), (19, 40)) && !meets((10, 20), (20, 30)) && !meets((10, 20), (0, 10)), "a selection");
    }

    #[test]
    fn segments_skip_bad_spans() {
        let t = "héllo world";
        assert_eq!(segments(t, &[0..2]), vec![Seg { text: t.into(), shared: false }], "not a char boundary: left unmarked");
        assert_eq!(marked(&segments(t, &[7..12, 0..3])), ["world"], "out of order: the later one skipped");
        assert!(segments("", &[]).is_empty());
    }
}
