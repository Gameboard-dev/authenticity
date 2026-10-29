//! check-overlap: the close-copying check of a draft against the library notes, from the command line. Wording taken
//! from a source without quotation marks is plagiarism even when cited, so a chapter is checked before it is handed
//! over.
//!
//! ```text
//! check-overlap "<vault>/Drafts/Chapter 2.md"                 # against the works the draft cites
//! check-overlap draft.md --library                            # against every library note
//! check-overlap draft.md --words 7 --term "neural network"    # longer runs only, a fixed term
//! ```
//!
//! What is compared: the draft's running text, without front matter, `%% comments %%`, inline comments `^[ ... ]`,
//! callouts and table rows, code, maths, links and `[@citations]`, LaTeX commands with their first argument, citations
//! in brackets with a year, and anything in quotation marks (a quotation is meant to match). Words are compared
//! normalised (case, punctuation, white space, hyphens, apostrophes, accents and ligatures do not count).
//!
//! By default the draft is compared with the notes of the works it cites (`[@key]`, `@key`): the notes in the library
//! folder's `<key>/` folder, and any library note whose `citekey` property is the key. `--library` compares it with
//! every library note. The library's notes are the only sources: the user's own writing (Drafts/, LaTeX/, the excluded
//! folders, the draft itself) never is. A run is at least `--words` words (default 6) in that order in one note;
//! near-verbatim stretches have up to three words changed between exact runs. Runs found in `--common` notes or more
//! (default 5) are the field's fixed phrasing, listed apart; runs with fewer than `--content` content words (default
//! 4) are ordinary language; fixed terms (`--term`, and Authenticity's setting *Fixed terms*) never make a run. Exit
//! status 1 when a run is found, 2 when the check could not run.

use std::collections::HashSet;
use std::path::Path;

use authenticity_similar::{library as lib, Kind, Masks, Options, Query, K};

use crate::vault::Vault;

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub note: String,
    pub path: String,
    pub length: usize,
    pub text: String,
    pub near: bool,
    pub matched: usize,
    pub words: usize,
    pub others: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub runs: Vec<Run>,
    pub commons: Vec<(String, usize)>,
    /// Cited keys with no library note.
    pub missing: Vec<String>,
    pub sources: usize,
}

pub struct Args {
    pub draft: String,
    pub library: bool,
    pub words: usize,
    pub common: usize,
    pub content: usize,
    pub terms: Vec<String>,
    pub vault: Option<String>,
    pub folder: Option<String>,
    pub exclude: Vec<String>,
    pub json: bool,
}

pub const USAGE: &str = "usage: check-overlap DRAFT [--library] [--words N (6)] [--common N (5)] [--content N (4)] [--term PHRASE]... [--vault DIR] [--library-folder FOLDER] [--exclude FOLDER]... [--json]";

pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args { draft: String::new(), library: false, words: 6, common: 5, content: 4, terms: Vec::new(), vault: None, folder: None, exclude: Vec::new(), json: false };
    let mut it = argv.iter();
    let num = |v: Option<&String>, flag: &str| v.and_then(|s| s.parse::<usize>().ok()).ok_or(format!("{flag}: a number"));
    while let Some(x) = it.next() {
        match x.as_str() {
            "--library" => a.library = true,
            "--json" => a.json = true,
            "--words" => a.words = num(it.next(), "--words")?,
            "--common" => a.common = num(it.next(), "--common")?,
            "--content" => a.content = num(it.next(), "--content")?,
            "--term" => a.terms.push(it.next().ok_or("--term: a phrase")?.clone()),
            "--vault" | "--notes" => a.vault = Some(it.next().ok_or("--vault: a folder")?.clone()),
            "--library-folder" => a.folder = Some(it.next().ok_or("--library-folder: a vault folder")?.clone()),
            "--exclude" => a.exclude.push(it.next().ok_or("--exclude: a vault folder")?.clone()),
            "-h" | "--help" => return Err(USAGE.into()),
            f if f.starts_with("--") => return Err(format!("unknown option {f}\n{USAGE}")),
            d if a.draft.is_empty() => a.draft = d.to_string(),
            _ => return Err(USAGE.into()),
        }
    }
    if a.draft.is_empty() {
        return Err(USAGE.into());
    }
    Ok(a)
}

/// The citation keys a draft cites: `[@key ...]`, `; @key`, and narrative `@key`.
pub fn cited_keys(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let b: Vec<char> = text.chars().collect();
    for i in 0..b.len() {
        if b[i] != '@' || (i > 0 && (b[i - 1].is_alphanumeric() || b[i - 1] == '.')) {
            continue;
        }
        let key: String = b[i + 1..].iter().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '-')).collect();
        let key = key.trim_end_matches(['.', ':', '-']).to_string();
        if key.chars().next().is_some_and(char::is_alphanumeric) {
            out.insert(key);
        }
    }
    out
}

/// A note's `citekey` property, from its first lines.
fn citekey(text: &str) -> Option<String> {
    let head: String = text.chars().take(3000).collect();
    head.lines().find_map(|l| l.strip_prefix("citekey:")).map(|v| v.trim().trim_matches('"').to_string()).filter(|k| !k.is_empty())
}

fn squeeze(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c == '\0').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

pub fn check(a: &Args) -> Result<Outcome, String> {
    let draft = Path::new(&a.draft);
    let raw = std::fs::read(draft).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| format!("{}: {e}", a.draft))?;
    let v = Vault::find(a.vault.as_deref(), draft.parent().unwrap_or(Path::new(".")), ".obsidian")?;
    let me = v.rel(draft).unwrap_or_default();
    let scope = v.scope(a.folder.as_deref(), &a.exclude);
    if !scope.is_set() {
        return Err(format!("{} (or give --library-folder FOLDER)", authenticity_core::library::SET_LIBRARY));
    }
    let (ix, keys) = v.index(&scope.library);
    let mut terms = v.fixed_terms();
    terms.extend(a.terms.iter().cloned());
    let q = Query::new(&raw, Masks::ALL, &terms);
    let held: Vec<String> = ix.signatures().map(|(k, _)| k.to_string()).filter(|k| scope.holds(k) && *k != me).collect();
    let mut missing = Vec::new();
    let targets: HashSet<String> = if a.library {
        held.iter().cloned().collect()
    } else {
        let cited = cited_keys(&raw);
        let mut t = HashSet::new();
        for key in &cited {
            let folder = format!("{}{key}/", scope.library);
            let found: Vec<&String> = held.iter().filter(|p| p.starts_with(&folder) || v.read(p).and_then(|x| citekey(&x)).as_deref() == Some(key.as_str())).collect();
            if found.is_empty() {
                missing.push(key.clone());
            }
            t.extend(found.into_iter().cloned());
        }
        missing.sort();
        t
    };
    let o = Options { min_run: a.words.max(K), content: a.content, common: a.common, near: true, max_postings: 2000 };
    let plan = ix.plan(&q, &o, &|k| targets.contains(k));
    let texts = lib::passage_texts(&plan.refs, &keys, |k| v.read(k));
    let found = ix.finish(&q, &o, &plan, &texts);
    let title = |p: &str| authenticity_core::library::title(p, &scope.library);
    let mut runs: Vec<Run> = found.findings.iter().map(|f| Run { note: title(&f.source), path: f.source.clone(), length: f.longest, text: squeeze(&raw[f.start..f.end]), near: f.kind == Kind::Near, matched: f.matched, words: f.words, others: f.also.len() }).collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.length.max(r.matched)));
    let mut commons: Vec<(String, usize)> = found.common.iter().map(|c| (squeeze(&raw[c.start..c.end]), c.sources)).collect();
    commons.sort_by_key(|c| std::cmp::Reverse(c.1));
    Ok(Outcome { runs, commons, missing, sources: targets.len() })
}

pub fn print(a: &Args, out: &Outcome) {
    if a.json {
        let v = serde_json::json!({
            "draft": a.draft, "scope": if a.library { "library" } else { "cited" }, "sources_compared": out.sources, "missing": out.missing,
            "runs": out.runs.iter().map(|r| serde_json::json!({ "note": r.note, "path": r.path, "longest": r.length, "text": r.text, "near_verbatim": r.near, "matched": r.matched, "words": r.words, "other_notes": r.others })).collect::<Vec<_>>(),
            "fixed_phrasing": out.commons.iter().map(|(t, n)| serde_json::json!({ "text": t, "notes": n })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return;
    }
    println!("{}", a.draft);
    println!("  compared with {} library note{} ({})", out.sources, if out.sources == 1 { "" } else { "s" }, if a.library { "the whole library" } else { "the works it cites" });
    if !out.missing.is_empty() {
        println!("  cited keys with no library note (not checked): {}", out.missing.join(", "));
    }
    println!("  possible close copying ({}):", out.runs.len());
    for r in &out.runs {
        let also = if r.others > 0 { format!(" (and {} other note{})", r.others, if r.others == 1 { "" } else { "s" }) } else { String::new() };
        let near = if r.near { format!(" (near-verbatim: {} of {} words the same)", r.matched, r.words) } else { String::new() };
        println!("    {:>3} words | {}{also} | {}{near}", r.length, r.note.chars().take(60).collect::<String>(), r.text);
    }
    let shown: Vec<&str> = out.commons.iter().take(20).map(|c| c.0.as_str()).collect();
    println!("  fixed phrasing, found in {}+ notes ({}): {}", a.common, out.commons.len(), shown.join("; "));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::vault;

    fn args(draft: &Path, library: bool) -> Args {
        Args { draft: draft.to_string_lossy().into_owned(), library, words: 6, common: 5, content: 4, terms: Vec::new(), vault: None, folder: Some("Library/".into()), exclude: Vec::new(), json: false }
    }

    #[test]
    fn the_works_cited_by_default_the_library_with_a_flag_never_own_writing() {
        // a cache in the test's own folder, never the user's
        let v = vault("overlap", &[
            ("Library/smith2020/A Sample Title.md", "---\ncitekey: \"smith2020\"\n---\n# Results\n\nThe proposed method reduced the error rate on every benchmark we tried by a wide margin.\n"),
            ("Library/jones2019/Uncited.md", "---\ncitekey: \"jones2019\"\n---\nOur survey shows that careful calibration of the sensors improves long term stability in every season.\n"),
            ("Drafts/Earlier (DRAFT).md", "My own earlier wording: we argue that the shared vocabulary of the field shapes every later chapter."),
        ]);
        std::env::set_var("LOCALAPPDATA", v.root.join("local"));
        std::env::set_var("XDG_CACHE_HOME", v.root.join("local"));
        let draft = v.root.join("Drafts").join("Chapter.md");
        std::fs::write(&draft, "The proposed method reduced the error rate on every benchmark we tried [@smith2020, p. 3]. Careful calibration of the sensors improves long term stability [@lee2021]. My own earlier wording: we argue that the shared vocabulary of the field shapes every later chapter.\n").unwrap();
        let cited = check(&args(&draft, false)).unwrap();
        assert_eq!(cited.missing, ["lee2021"]);
        assert_eq!(cited.runs.len(), 1, "{:?}", cited.runs);
        assert_eq!(cited.runs[0].note, "smith2020 \u{203a} A Sample Title");
        assert_eq!(cited.runs[0].text, "The proposed method reduced the error rate on every benchmark we tried");
        let whole = check(&args(&draft, true)).unwrap();
        assert_eq!(whole.runs.len(), 2, "{:?}", whole.runs);
        assert!(whole.runs.iter().any(|r| r.path == "Library/jones2019/Uncited.md"));
        assert!(!whole.runs.iter().any(|r| r.path.starts_with("Drafts/")), "own writing is never a source");
        // a fixed term makes the run ordinary language
        let mut a = args(&draft, false);
        a.terms = vec!["error rate on every benchmark".into(), "proposed method reduced".into()];
        assert!(check(&a).unwrap().runs.is_empty());
        // the index was kept: a second run reads nothing new and gives the same
        assert_eq!(check(&args(&draft, true)).unwrap().runs.len(), 2);
        _ = std::fs::remove_dir_all(&v.root);
    }

    #[test]
    fn keys_and_arguments() {
        let k = cited_keys("As [@smith2020, p. 3; @jones2019] and @lee2021. say; mail me@example.com.");
        assert!(k.contains("smith2020") && k.contains("jones2019") && k.contains("lee2021"));
        assert!(!k.iter().any(|x| x.contains("example")));
        let a = parse_args(&["d.md".into(), "--library".into(), "--term".into(), "x y".into(), "--words".into(), "7".into()]).unwrap();
        assert!(a.library && a.terms == ["x y"] && a.words == 7);
        assert!(parse_args(&["--bogus".into()]).is_err());
        assert!(parse_args(&[]).is_err());
    }
}
