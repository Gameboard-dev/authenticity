//! The paraphrase search judged on a real draft (a developer's check, not part of the release): every candidate the
//! content-word search finds for the draft's sentences, with its closeness (content words kept in order) and its weight
//! (the share of their weight the source sentence holds), the counts the closeness and weight gates would flag, and,
//! when an embedding service's similarity API answers on `--similar-port`, what it finds beside them. Quarry's service
//! asks for its token (`--token`, the 64 hex digits of `service-token` in its data folder).
//!
//! `cargo run --release --example para_eval -- --vault DIR --library FOLDER --note NOTE [--similar-port PORT] [--token TOKEN]`
//! (FOLDER and NOTE are vault paths). The index is made in memory; nothing is written.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use authenticity_core::similarity::{self, Settings};
use authenticity_core::sloptotal::block_on;
use authenticity_native::{http, vault::Vault};
use authenticity_similar::{library as lib, Index};
use serde_json::Value;

struct Similar(u16, Option<String>);

impl similarity::SimilarApi for Similar {
    async fn similar(&self, body: &Value) -> Result<Value, String> {
        let bearer = self.1.as_ref().map(|t| format!("Bearer {t}"));
        let headers: Vec<(&str, &str)> = bearer.iter().map(|b| ("Authorization", b.as_str())).collect();
        let r = http::request_with(self.0, "POST", "/similar", Some(&body.to_string()), Duration::from_secs(900), &headers)?;
        if r.status != 200 {
            return Err(format!("answered {}", r.status));
        }
        serde_json::from_str(&r.body).map_err(|e| e.to_string())
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (root, folder, note) = (arg(&args, "--vault").expect("--vault"), arg(&args, "--library").expect("--library"), arg(&args, "--note").expect("--note"));
    let v = Vault::find(Some(&root), std::path::Path::new("."), ".obsidian").expect("the vault");
    let t = Instant::now();
    let files = v.library_files(&folder);
    let mut add = Vec::new();
    for (p, mt, sz) in &files {
        if let Some(text) = v.read(p) {
            add.push(lib::source(p, &text, lib::signature(*mt, *sz, &text)));
        }
    }
    let read_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let mut ix = Index::new();
    ix.update(&[], add);
    let st = ix.stats();
    println!("index: {} notes, {} passages, {} shingle and {} word entries, {:.1} MB; read {read_s:.2} s, made {:.2} s", st.sources, st.passages, st.shingles, st.words, st.bytes as f64 / 1_048_576.0, t.elapsed().as_secs_f64());
    let keys: HashMap<i64, String> = ix.signatures().map(|(k, _)| (lib::key_hash(k), k.to_string())).collect();
    let text = v.read(&note).expect("the note");
    let s = Settings { fixed_terms: v.fixed_terms(), ..Settings::default() };

    // verbatim first, as the check does: sentences mostly covered by a copied run are not judged for paraphrase
    let t = Instant::now();
    let q = s.query(&text);
    let plan = ix.plan(&q, &s.options(), &|k| k != note);
    let texts = lib::passage_texts(&plan.refs, &keys, |k| v.read(k));
    let found = ix.finish(&q, &s.options(), &plan, &texts);
    let verbatim = similarity::verbatim(&text, 0, &found, |k| authenticity_core::library::title(k, &folder), &texts, |k| v.read(k));
    let covered: Vec<std::ops::Range<usize>> = verbatim.iter().filter(|m| m.kind != "common").filter_map(|m| text.find(&m.text).map(|a| a..a + m.text.len())).collect();
    let sentences = similarity::paraphrase_sentences(&text, &s, &covered);
    println!("verbatim: {} findings in {:.2} s; {} sentences judged for paraphrase", verbatim.iter().filter(|m| m.kind != "common").count(), t.elapsed().as_secs_f64(), sentences.len());

    let t = Instant::now();
    let top: usize = arg(&args, "--candidates").and_then(|c| c.parse().ok()).unwrap_or(authenticity_similar::PARA_CANDIDATES);
    let stems: Vec<Vec<String>> = sentences.iter().map(|(_, s)| authenticity_similar::para::content(s)).collect();
    let plan = ix.para_plan(&stems, &|k| k != note, top);
    let texts = lib::passage_texts(&plan.refs, &keys, |k| v.read(k));
    let floor = 0.3;
    let all = ix.para_finish(&stems, &plan, &texts, floor);
    println!("content-word search: {} passages read, {:.3} s", plan.refs.len(), t.elapsed().as_secs_f64());
    // every sentence's best candidate (kept + weight), whatever its weight
    let mut best: Vec<(usize, f64, f64, String, String)> = Vec::new();
    for (i, hits) in all.iter().enumerate() {
        if stems[i].len() < authenticity_similar::para::MIN_CONTENT {
            continue;
        }
        if let Some(h) = hits.iter().max_by(|a, b| (a.kept + a.weight).total_cmp(&(b.kept + b.weight))) {
            best.push((i, h.kept, h.weight, h.key.clone(), h.sentence.clone()));
        }
    }
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!("\nbest candidate per sentence (kept >= {floor}):");
    for (i, kept, w, key, src) in &best {
        let mine: String = sentences[*i].1.chars().take(160).collect();
        let src: String = src.chars().take(160).collect();
        let mut uniq = stems[*i].clone();
        uniq.sort();
        uniq.dedup();
        let total: f64 = uniq.iter().map(|x| ix.stem_weight(x)).sum();
        println!("  [{i}] kept {:.0}% weight {:.0}% ({} stems, idf {:.1} of {:.1})  {}\n      MINE: {mine}\n      SRC:  {src}", kept * 100.0, w * 100.0, stems[*i].len(), total * w, total, key.rsplit('/').nth(1).unwrap_or(key));
    }
    // with the rule as shipped (para::close: the weight gates), at each closeness: the best passing candidate per sentence
    println!("\nflagged with the shipped gates (weight >= {:.0}%, shared >= {:.0}):", authenticity_similar::para::WEIGHT * 100.0, authenticity_similar::para::MIN_SHARED);
    for c in [0.4, 0.5, 0.6, 0.7] {
        let flagged: Vec<String> = all.iter().enumerate().filter_map(|(i, hits)| {
            let h = hits.iter().filter(|h| authenticity_similar::para::close(&stems[i], h.kept, h.weight, h.shared, c)).max_by(|a, b| (a.kept + a.weight).total_cmp(&(b.kept + b.weight)))?;
            Some(format!("[{i}] {} {:.0}%", h.key.rsplit('/').nth(1).unwrap_or(&h.key), h.kept * 100.0))
        }).collect();
        println!("  closeness {:.0}%: {} flagged: {}", c * 100.0, flagged.len(), flagged.join("; "));
    }

    if let Some(port) = arg(&args, "--similar-port").and_then(|p| p.parse::<u16>().ok()) {
        let t = Instant::now();
        let asked: Vec<String> = sentences.iter().map(|s| s.1.clone()).collect();
        match block_on(similarity::ask(&Similar(port, arg(&args, "--token")), &asked, floor, "")) {
            Ok(similarity::Outcome::Hits(h)) => {
                println!("\nembedding search ({:.1} s): candidates passing its cosine gate", t.elapsed().as_secs_f64());
                for (i, hs) in h.iter().enumerate() {
                    if let Some(x) = hs.first() {
                        let mine: String = sentences[i].1.chars().take(160).collect();
                        println!("  [{i}] kept {:.0}% cosine {:.2}  {}\n      MINE: {mine}\n      SRC:  {}", x.kept * 100.0, x.cosine.unwrap_or(0.0), x.source.rsplit('/').nth(1).unwrap_or(&x.source), x.source_text.chars().take(160).collect::<String>());
                    }
                }
            }
            Ok(similarity::Outcome::Shortage(s)) => println!("embedding search: GPU shortage {s}"),
            Err(e) => println!("embedding search: {e}"),
        }
    }
}
