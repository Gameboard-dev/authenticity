//! The library's index and its two searches: word sequences (shingles) for copied wording, and content words for
//! close paraphrase. Each check runs in two steps (`plan`, then `finish` with the passages the plan names, which the
//! plugin reads asynchronously); the index is kept as bytes as well as in a file (`to_bytes`, `from_bytes`, for the
//! plugin, which has no file system of its own).
//!
//! Every passage of every source is read as its words (`text::tokens`); each run of `K` consecutive words (a shingle)
//! is hashed to 32 bits, and the index keeps one sorted array of (shingle, passage) pairs, eight bytes each (the user's
//! library: 31,000 passages, 5.7 million words). Looking a shingle up is a binary search. Sources are added, replaced
//! and removed one by one (`update`): their passages' shingles are made and merged into the array in one linear pass,
//! with no other source read again, so a few changed notes cost a few notes' work. The index is kept on disk between
//! runs (`save`, `load`).
//!
//! The check (`find`) looks the text's shingles up: a source with shingles close together in the text (a run of
//! `min_run` words gives `min_run - K + 1` in a row) is a candidate, and only the candidates' passages are read again
//! and aligned word by word with the text. The 32-bit hash only chooses candidates; the alignment compares 64-bit
//! hashes of whole words, so a collision costs a passage read, never a false finding.
//!
//! - **verbatim**: a run of at least `min_run` words of the text, in order, in the source (normalised as
//!   `text::tokens` says);
//! - **near-verbatim**: exact runs of at least `K` words separated by at most `NEAR_GAP` words changed, inserted or
//!   left out, on both sides, that together match at least `min_run` words and `NEAR_COVERAGE` of the stretch;
//! - **fixed phrasing**: a run whose shingles all occur in `common` sources or more is the field's wording, listed
//!   apart; a run with fewer than `content` content words (not function words, not part of a fixed term) is left out.
//!
//! **Close paraphrase** (`para_plan`, `para_finish`; the rule is para.rs's). The index also keeps each passage's content
//! words (their stems, each once) as a second sorted array of (stem, passage) pairs, with each passage's number of
//! them. A sentence's candidates are the passages that share the most of its stems, each stem weighed by its inverse
//! document frequency and a passage's score by its length (BM25 with every word counted once:
//! <https://en.wikipedia.org/wiki/Okapi_BM25>), so a passage that shares the sentence's rarer words comes first and a
//! long passage does not win by its length alone. Only the best few passages per sentence are read again; their
//! sentences are compared with the sentence word by word (`para::in_order`, `para::weighted_share`). No model, no
//! network: a sentence costs one pass over its stems' postings.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;

use crate::text::{self, Token};

/// Words per shingle: the shortest run the index can find.
pub const K: usize = 4;
/// The version of the words, the hashes and the file: an index of another version is made again (2: the content
/// words for close paraphrase).
pub const VERSION: u32 = 2;
/// Candidate passages read per sentence for close paraphrase, and BM25's two constants.
pub const PARA_CANDIDATES: usize = 16;
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
/// The most words changed, inserted or left out between two exact runs of a near-verbatim stretch.
pub const NEAR_GAP: usize = 3;
/// The share of a near-verbatim stretch's words that must match exactly.
pub const NEAR_COVERAGE: f64 = 0.7;

/// Authenticity's own file; any other format (another magic) is refused.
const MAGIC: &[u8; 4] = b"AUSI";
const LOW: u64 = 0xffff_ffff;

/// A shingle's hash from its words' hashes (64 bits; the index keeps the top 32).
pub fn gram_hash(words: &[u64]) -> u64 {
    let mut h: u64 = 0x9e37_79b9_7f4a_7c15;
    for w in words {
        h = (h.rotate_left(23) ^ w).wrapping_mul(0x2545_f491_4f6c_dd1d);
    }
    // splitmix64's finaliser
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// The index's keys for a source passage's text (its own text, nothing left out): each shingle's top 32 bits, sorted
/// and without repeats.
pub fn keys_of(passage: &str) -> Vec<u32> {
    let hashes: Vec<u64> = text::tokens(passage).iter().map(|t| t.hash).collect();
    let mut out: Vec<u32> = hashes.windows(K).map(|w| (gram_hash(w) >> 32) as u32).collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[derive(Clone, Debug, PartialEq)]
struct Source {
    key: String,
    sig: [u64; 3],
    first: u32,
    count: u32,
}

/// A source to add (or put in place of the one with its key): its key, the caller's signature of what it was made
/// from (compared at the next update), and its passages, each the caller's reference (a passage id) with its keys
/// (`keys_of`); `words` gives each passage's content-word keys (`para::word_keys`), in the same order (missing: none).
pub struct NewSource {
    pub key: String,
    pub sig: [u64; 3],
    pub passages: Vec<(i64, Vec<u32>)>,
    pub words: Vec<Vec<u32>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    sources: Vec<Source>,
    by_key: HashMap<String, u32>,
    /// Each passage's reference, and its source.
    refs: Vec<i64>,
    owner: Vec<u32>,
    /// (shingle << 32 | passage), sorted.
    postings: Vec<u64>,
    /// Each passage's number of distinct content words, and (stem << 32 | passage), sorted.
    wlen: Vec<u32>,
    words: Vec<u64>,
}

/// What the index holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub sources: usize,
    pub passages: usize,
    pub shingles: usize,
    pub words: usize,
    pub bytes: usize,
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    pub fn stats(&self) -> Stats {
        let keys: usize = self.sources.iter().map(|s| s.key.len() + 40).sum();
        Stats { sources: self.sources.len(), passages: self.refs.len(), shingles: self.postings.len(), words: self.words.len(), bytes: (self.postings.len() + self.words.len()) * 8 + self.refs.len() * 16 + keys }
    }

    /// Each source's key and signature.
    pub fn signatures(&self) -> impl Iterator<Item = (&str, [u64; 3])> {
        self.sources.iter().map(|s| (s.key.as_str(), s.sig))
    }

    /// A source's passage references, in order.
    pub fn passages_of(&self, key: &str) -> &[i64] {
        match self.by_key.get(key) {
            Some(&s) => {
                let s = &self.sources[s as usize];
                &self.refs[s.first as usize..(s.first + s.count) as usize]
            }
            None => &[],
        }
    }

    /// Whether a source is held.
    pub fn has(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    /// A source's signature, if it is held.
    pub fn signature(&self, key: &str) -> Option<[u64; 3]> {
        self.by_key.get(key).map(|&s| self.sources[s as usize].sig)
    }

    /// A held source's signature replaced (its file was touched but its text is the same); whether it is held.
    pub fn set_signature(&mut self, key: &str, sig: [u64; 3]) -> bool {
        match self.by_key.get(key) {
            Some(&s) => {
                self.sources[s as usize].sig = sig;
                true
            }
            None => false,
        }
    }

    /// Sources removed (`remove`) and added or replaced (`add`, a source already held is replaced), in one linear
    /// pass over the array; no other source is read again.
    pub fn update(&mut self, remove: &[String], add: Vec<NewSource>) {
        if remove.is_empty() && add.is_empty() {
            return;
        }
        let gone: HashSet<&str> = remove.iter().map(String::as_str).chain(add.iter().map(|s| s.key.as_str())).collect();
        let mut remap = vec![u32::MAX; self.refs.len()];
        let (mut sources, mut refs, mut owner) = (Vec::with_capacity(self.sources.len() + add.len()), Vec::with_capacity(self.refs.len()), Vec::with_capacity(self.refs.len()));
        let mut wlen = Vec::with_capacity(self.refs.len());
        for s in &self.sources {
            if gone.contains(s.key.as_str()) {
                continue;
            }
            let first = refs.len() as u32;
            for slot in s.first..s.first + s.count {
                remap[slot as usize] = refs.len() as u32;
                refs.push(self.refs[slot as usize]);
                owner.push(sources.len() as u32);
                wlen.push(self.wlen.get(slot as usize).copied().unwrap_or(0));
            }
            sources.push(Source { key: s.key.clone(), sig: s.sig, first, count: s.count });
        }
        let (mut fresh, mut fresh_words): (Vec<u64>, Vec<u64>) = (Vec::new(), Vec::new());
        let mut seen = HashSet::new();
        for ns in add.into_iter().rev() {
            // the same key twice: the later one
            if !seen.insert(ns.key.clone()) {
                continue;
            }
            let first = refs.len() as u32;
            for (i, (r, keys)) in ns.passages.iter().enumerate() {
                let slot = refs.len() as u64;
                refs.push(*r);
                owner.push(sources.len() as u32);
                fresh.extend(keys.iter().map(|k| (u64::from(*k) << 32) | slot));
                let words = ns.words.get(i).map(Vec::as_slice).unwrap_or(&[]);
                wlen.push(words.len() as u32);
                fresh_words.extend(words.iter().map(|k| (u64::from(*k) << 32) | slot));
            }
            sources.push(Source { key: ns.key, sig: ns.sig, first, count: ns.passages.len() as u32 });
        }
        let postings = merge(&self.postings, &remap, fresh);
        let words = merge(&self.words, &remap, fresh_words);
        self.by_key = sources.iter().enumerate().map(|(i, s)| (s.key.clone(), i as u32)).collect();
        (self.sources, self.refs, self.owner, self.postings, self.wlen, self.words) = (sources, refs, owner, postings, wlen, words);
    }

    /// The entries of one shingle (its top 32 bits).
    fn postings_of(&self, key: u32) -> &[u64] {
        entries(&self.postings, key)
    }

    // -------------------------------------------------------------------- on disk

    /// The index written to `path` (whole, then renamed into place, so a crash never leaves half a file).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        {
            let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
            self.write_to(&mut w)?;
            w.flush()?;
        }
        std::fs::rename(&tmp, path)
    }

    /// The index as the bytes `save` writes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.postings.len() * 8 + self.refs.len() * 8 + 64);
        self.write_to(&mut out).expect("writing to memory does not fail");
        out
    }

    /// An index from `to_bytes`; an error for another version or damaged bytes.
    pub fn from_bytes(bytes: &[u8]) -> std::io::Result<Index> {
        Index::read_from(&mut std::io::Cursor::new(bytes), "the stored index")
    }

    fn write_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        {
            w.write_all(MAGIC)?;
            for x in [VERSION, K as u32, self.sources.len() as u32] {
                w.write_all(&x.to_le_bytes())?;
            }
            for s in &self.sources {
                w.write_all(&(s.key.len() as u32).to_le_bytes())?;
                w.write_all(s.key.as_bytes())?;
                for x in s.sig {
                    w.write_all(&x.to_le_bytes())?;
                }
                w.write_all(&s.first.to_le_bytes())?;
                w.write_all(&s.count.to_le_bytes())?;
            }
            w.write_all(&(self.refs.len() as u64).to_le_bytes())?;
            for r in &self.refs {
                w.write_all(&r.to_le_bytes())?;
            }
            let mut buf = Vec::with_capacity(8 * 65536);
            for list in [&self.postings, &self.words] {
                w.write_all(&(list.len() as u64).to_le_bytes())?;
                for chunk in list.chunks(65536) {
                    buf.clear();
                    for e in chunk {
                        buf.extend_from_slice(&e.to_le_bytes());
                    }
                    w.write_all(&buf)?;
                }
            }
            buf.clear();
            for n in &self.wlen {
                buf.extend_from_slice(&n.to_le_bytes());
            }
            w.write_all(&buf)?;
        }
        Ok(())
    }

    /// The index kept at `path`; an error for a missing file, another version or a damaged one (the caller makes it
    /// again).
    pub fn load(path: &Path) -> std::io::Result<Index> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Index::read_limited(&mut std::io::BufReader::with_capacity(1 << 20, file), &path.display().to_string(), len)
    }

    fn read_from(r: &mut std::io::Cursor<&[u8]>, name: &str) -> std::io::Result<Index> {
        let len = r.get_ref().len() as u64;
        Index::read_limited(r, name, len)
    }

    /// The index from `r`, which holds `len` bytes: a count that would need more than that is damage, not a reason to
    /// ask for the memory.
    fn read_limited(r: &mut impl Read, name: &str, total: u64) -> std::io::Result<Index> {
        let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{name}: {why}"));
        let mut magic = [0u8; 4];
        r.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(bad("not a shingle index"));
        }
        let mut u32_ = || -> std::io::Result<u32> {
            let mut b = [0u8; 4];
            r.read_exact(&mut b)?;
            Ok(u32::from_le_bytes(b))
        };
        let (version, k, n) = (u32_()?, u32_()?, u32_()?);
        if version != VERSION || k as usize != K {
            return Err(bad(&format!("version {version} with {k}-word shingles, not {VERSION} with {K}")));
        }
        // each source takes at least 36 bytes, each passage and shingle 8
        if u64::from(n) * 36 > total {
            return Err(bad("damaged (more sources than the file holds)"));
        }
        let fits = |count: usize| (count as u64).checked_mul(8).is_some_and(|b| b <= total);
        let mut ix = Index::new();
        let mut b4 = [0u8; 4];
        let mut b8 = [0u8; 8];
        for _ in 0..n {
            r.read_exact(&mut b4)?;
            let len = u32::from_le_bytes(b4) as usize;
            if len > 4096 {
                return Err(bad("a source key too long"));
            }
            let mut key = vec![0u8; len];
            r.read_exact(&mut key)?;
            let mut sig = [0u64; 3];
            for x in &mut sig {
                r.read_exact(&mut b8)?;
                *x = u64::from_le_bytes(b8);
            }
            r.read_exact(&mut b4)?;
            let first = u32::from_le_bytes(b4);
            r.read_exact(&mut b4)?;
            let count = u32::from_le_bytes(b4);
            ix.sources.push(Source { key: String::from_utf8(key).map_err(|_| bad("a source key not UTF-8"))?, sig, first, count });
        }
        r.read_exact(&mut b8)?;
        let slots = u64::from_le_bytes(b8) as usize;
        if !fits(slots) {
            return Err(bad("damaged (more passages than the file holds)"));
        }
        let mut raw = vec![0u8; slots * 8];
        r.read_exact(&mut raw)?;
        ix.refs = raw.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c)).collect();
        for (list, what) in [(&mut ix.postings, "shingles"), (&mut ix.words, "words")] {
            r.read_exact(&mut b8)?;
            let entries = u64::from_le_bytes(b8) as usize;
            if !fits(entries) {
                return Err(bad(&format!("damaged (more {what} than the file holds)")));
            }
            let mut raw = vec![0u8; entries * 8];
            r.read_exact(&mut raw)?;
            *list = raw.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
        }
        let mut raw = vec![0u8; slots * 4];
        r.read_exact(&mut raw)?;
        ix.wlen = raw.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        // the passages' sources from the sources' ranges, checked
        ix.owner = vec![u32::MAX; slots];
        for (i, s) in ix.sources.iter().enumerate() {
            let end = s.first as usize + s.count as usize;
            if end > slots {
                return Err(bad("a source's passages out of range"));
            }
            ix.owner[s.first as usize..end].fill(i as u32);
        }
        let bad_list = |l: &[u64]| l.iter().any(|e| (e & LOW) as usize >= slots) || !l.is_sorted();
        if ix.owner.contains(&u32::MAX) || bad_list(&ix.postings) || bad_list(&ix.words) {
            return Err(bad("damaged"));
        }
        ix.by_key = ix.sources.iter().enumerate().map(|(i, s)| (s.key.clone(), i as u32)).collect();
        Ok(ix)
    }

    // -------------------------------------------------------------------- the check

    /// The verbatim and near-verbatim runs `q` shares with the sources `allowed` accepts. `texts` gives the passages'
    /// texts by reference (only candidates' passages are asked for, in one call).
    pub fn find(&self, q: &Query, o: &Options, allowed: &dyn Fn(&str) -> bool, texts: &mut Texts) -> Result<Found, String> {
        let plan = self.plan(q, o, allowed);
        let texts = if plan.refs.is_empty() { HashMap::new() } else { texts(&plan.refs)? };
        Ok(self.finish(q, o, &plan, &texts))
    }

    /// The first step of `find`: the candidates, and the passages (`Plan::refs`) whose texts `finish` needs.
    pub fn plan(&self, q: &Query, o: &Options, allowed: &dyn Fn(&str) -> bool) -> Plan {
        let n = o.min_run.max(K);
        let t = &q.tokens;
        let hashes: Vec<u64> = t.iter().map(|x| x.hash).collect();
        // the text's shingles, where K words share a stretch between breaks
        let qg: Vec<Option<u64>> = (0..t.len()).map(|i| (i + K <= t.len() && t[i].seg == t[i + K - 1].seg).then(|| gram_hash(&hashes[i..i + K]))).collect();
        let mut ok: Vec<Option<bool>> = vec![None; self.sources.len()];
        let mut hits: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for (i, g) in qg.iter().enumerate() {
            let Some(g) = g else { continue };
            let ps = self.postings_of((g >> 32) as u32);
            if ps.len() > o.max_postings {
                continue; // in too many passages to choose candidates by (still counted for fixed phrasing)
            }
            for e in ps {
                let slot = (e & LOW) as u32;
                let src = self.owner[slot as usize];
                let yes = *ok[src as usize].get_or_insert_with(|| allowed(&self.sources[src as usize].key));
                if yes {
                    hits.entry(src).or_default().push((i as u32, slot));
                }
            }
        }
        // candidates: a run of n words makes n - K + 1 hits in a row; near-verbatim makes hits a little apart. A very
        // common shingle inside a run is not looked up, so hits up to K apart still pair.
        let m = n - K + 1;
        let window = if o.near { (K + NEAR_GAP) as u32 } else { K as u32 };
        let mut wanted: Vec<(u32, Vec<u32>)> = Vec::new();
        for (src, hs) in &hits {
            let mut chosen = BTreeSet::new();
            if m == 1 {
                chosen.extend(hs.iter().map(|h| h.1));
            } else {
                let mut lo = 0;
                for j in 0..hs.len() {
                    while hs[j].0 - hs[lo].0 > window {
                        lo += 1;
                    }
                    for k in lo..j {
                        if hs[k].0 < hs[j].0 {
                            chosen.insert(hs[k].1);
                            chosen.insert(hs[j].1);
                        }
                    }
                }
            }
            if !chosen.is_empty() {
                wanted.push((*src, chosen.into_iter().collect()));
            }
        }
        wanted.sort_by_key(|w| w.0);
        let refs: Vec<i64> = wanted.iter().flat_map(|(_, slots)| slots.iter().map(|s| self.refs[*s as usize])).collect();
        Plan { qg, wanted, refs }
    }

    /// The second step of `find`: the candidates of `plan` aligned with the text, given their passages' texts by
    /// reference (a passage missing from `texts` reads as empty and so matches nothing).
    pub fn finish(&self, q: &Query, o: &Options, plan: &Plan, texts: &HashMap<i64, String>) -> Found {
        let n = o.min_run.max(K);
        let (qg, wanted, refs) = (&plan.qg, &plan.wanted, &plan.refs);
        let mut found: Vec<Finding> = Vec::new();
        let mut commons: Vec<Common> = Vec::new();
        for (src, slots) in wanted {
            // consecutive passages of a source read as one stretch, so a run across their boundary is found whole
            let mut blocks: Vec<Vec<u32>> = Vec::new();
            for s in slots {
                match blocks.last_mut() {
                    Some(b) if *b.last().expect("not empty") + 1 == *s => b.push(*s),
                    _ => blocks.push(vec![*s]),
                }
            }
            for block in blocks {
                let doc = Doc::read(&block, |s| {
                    let r = self.refs[s as usize];
                    (r, texts.get(&r).map(String::as_str).unwrap_or(""))
                });
                for seeds in doc.chains(qg, o.near) {
                    let Some(f) = self.judge(q, &doc, &seeds, n, o, *src, &mut commons) else { continue };
                    found.push(f);
                }
            }
        }
        Found { findings: group(found), common: dedup(commons), candidates: wanted.len(), passages_read: refs.len() }
    }

    /// One chain of exact runs made a finding, a fixed phrase (into `commons`), or nothing.
    #[allow(clippy::too_many_arguments)]
    fn judge(&self, q: &Query, doc: &Doc, seeds: &[Seed], n: usize, o: &Options, src: u32, commons: &mut Vec<Common>) -> Option<Finding> {
        let first = seeds.first()?;
        let last = seeds.last()?;
        let (qs, qe) = (first.q, last.q + last.len);
        let matched: usize = seeds.iter().map(|s| s.len).sum();
        let longest = seeds.iter().map(|s| s.len).max().unwrap_or(0);
        let words = qe - qs;
        let kind = if seeds.len() == 1 && first.len >= n {
            Kind::Verbatim
        } else if seeds.len() > 1 && matched >= n && matched as f64 / words as f64 >= NEAR_COVERAGE {
            Kind::Near
        } else {
            return None;
        };
        let content = seeds.iter().flat_map(|s| &q.tokens[s.q..s.q + s.len]).filter(|t| t.content).count();
        if content < o.content {
            return None; // function words with a term or two: not copied wording
        }
        let (start, end) = (q.tokens[qs].start, q.tokens[qe - 1].end);
        if o.common > 0 {
            let spread = self.spread(q, seeds, o.common);
            if spread >= o.common {
                commons.push(Common { start, end, first: qs, last: qe, sources: spread });
                return None;
            }
        }
        let (pa, pb) = (first.p, last.p + last.len - 1);
        let (sa, sb) = (doc.at[pa], doc.at[pb]);
        let source_text = if sa.0 == sb.0 {
            doc.texts[sa.0][sa.1..sb.2].to_string()
        } else {
            let mut s = doc.texts[sa.0][sa.1..].to_string();
            for t in &doc.texts[sa.0 + 1..sb.0] {
                s.push(' ');
                s.push_str(t);
            }
            s.push(' ');
            s.push_str(&doc.texts[sb.0][..sb.2]);
            s
        };
        Some(Finding {
            kind,
            start,
            end,
            first: qs,
            last: qe,
            words,
            matched,
            longest,
            score: if kind == Kind::Verbatim { 1.0 } else { matched as f64 / words as f64 },
            source: self.sources[src as usize].key.clone(),
            passage: doc.refs[sa.0],
            source_start: sa.1,
            source_end: if sa.0 == sb.0 { sb.2 } else { doc.texts[sa.0].len() },
            source_text,
            also: Vec::new(),
        })
    }

    /// How many sources hold every shingle of the runs (at least; counting stops below `need`).
    fn spread(&self, q: &Query, seeds: &[Seed], need: usize) -> usize {
        let hashes: Vec<u64> = q.tokens.iter().map(|t| t.hash).collect();
        let mut keys: Vec<u32> = seeds.iter().flat_map(|s| (s.q..=s.q + s.len - K).map(|i| (gram_hash(&hashes[i..i + K]) >> 32) as u32)).collect();
        keys.sort_unstable();
        keys.dedup();
        let mut lists: Vec<&[u64]> = keys.iter().map(|k| self.postings_of(*k)).collect();
        lists.sort_by_key(|l| l.len());
        let Some(first) = lists.first() else { return 0 };
        let mut set: HashSet<u32> = first.iter().map(|e| self.owner[(e & LOW) as usize]).collect();
        for l in &lists[1..] {
            if set.len() < need {
                break;
            }
            let here: HashSet<u32> = l.iter().map(|e| self.owner[(e & LOW) as usize]).collect();
            set.retain(|s| here.contains(s));
        }
        set.len()
    }
}

/// Where the check reads the candidates' passages: their texts by reference, all asked for in one call.
pub type Texts<'a> = dyn FnMut(&[i64]) -> Result<HashMap<i64, String>, String> + 'a;

/// The entries of one key in a sorted (key << 32 | passage) array.
fn entries(list: &[u64], key: u32) -> &[u64] {
    let k = u64::from(key);
    let a = list.partition_point(|e| (e >> 32) < k);
    let b = a + list[a..].partition_point(|e| (e >> 32) <= k);
    &list[a..b]
}

/// The kept entries of `old`, renumbered by `remap` (which keeps their order; `u32::MAX`: removed), merged in one pass
/// with the sorted new ones.
fn merge(old: &[u64], remap: &[u32], mut fresh: Vec<u64>) -> Vec<u64> {
    fresh.sort_unstable();
    let kept = old.iter().filter_map(|e| {
        let s = remap[(e & LOW) as usize];
        (s != u32::MAX).then_some((e & !LOW) | u64::from(s))
    });
    let mut merged = Vec::with_capacity(old.len() + fresh.len());
    let mut f = fresh.into_iter().peekable();
    for e in kept {
        while let Some(&n) = f.peek().filter(|n| **n < e) {
            merged.push(n);
            f.next();
        }
        merged.push(e);
    }
    merged.extend(f);
    merged
}

/// The first step of the paraphrase search: each sentence's candidate passages (slots, best first), and the passages
/// whose texts `para_finish` needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParaPlan {
    pub per: Vec<Vec<u32>>,
    pub refs: Vec<i64>,
}

/// A library sentence a sentence may closely paraphrase: its source (key), its passage (reference), the sentence as
/// written, the share of the sentence's content words it keeps in order and the share of their weight it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct ParaHit {
    pub key: String,
    pub passage: i64,
    pub sentence: String,
    pub kept: f64,
    pub weight: f64,
    /// The shared words' weight in all (inverse document frequency summed).
    pub shared: f64,
}

impl Index {
    /// A stem's inverse document frequency over the passages (BM25's, never below 0).
    fn idf(&self, df: usize) -> f64 {
        let n = self.refs.len() as f64;
        (1.0 + (n - df as f64 + 0.5) / (df as f64 + 0.5)).ln()
    }

    /// A stem's weight in the library (`para::weighted_share`).
    pub fn stem_weight(&self, stem: &str) -> f64 {
        self.idf(entries(&self.words, crate::para::stem_key(stem)).len())
    }

    /// Each sentence's (its content stems, `para::content`) best `top` passages among the sources `allowed` accepts, by
    /// BM25 over its distinct stems; a sentence of fewer than `para::MIN_CONTENT` stems has none.
    pub fn para_plan(&self, sentences: &[Vec<String>], allowed: &dyn Fn(&str) -> bool, top: usize) -> ParaPlan {
        let n = self.refs.len();
        let avg = if n == 0 { 1.0 } else { (self.wlen.iter().map(|x| u64::from(*x)).sum::<u64>() as f64 / n as f64).max(1.0) };
        // each passage's length factor, once: (k1 + 1) / (1 + k1 (1 - b + b len / avg))
        let norm: Vec<f32> = self.wlen.iter().map(|l| ((BM25_K1 + 1.0) / (1.0 + BM25_K1 * (1.0 - BM25_B + BM25_B * f64::from(*l) / avg))) as f32).collect();
        let mut ok: Vec<Option<bool>> = vec![None; self.sources.len()];
        let mut score = vec![0f32; n];
        let mut touched: Vec<u32> = Vec::new();
        let mut per = Vec::with_capacity(sentences.len());
        let mut wanted: BTreeSet<u32> = BTreeSet::new();
        for mine in sentences {
            let mut stems: Vec<u32> = mine.iter().map(|s| crate::para::stem_key(s)).collect();
            stems.sort_unstable();
            stems.dedup();
            if mine.len() < crate::para::MIN_CONTENT {
                per.push(Vec::new());
                continue;
            }
            for k in stems {
                let list = entries(&self.words, k);
                // a stem in half the passages or more says nothing about which one
                if list.is_empty() || list.len() * 2 > n {
                    continue;
                }
                let idf = self.idf(list.len()) as f32;
                for e in list {
                    let slot = (e & LOW) as u32;
                    let src = self.owner[slot as usize] as usize;
                    if !*ok[src].get_or_insert_with(|| allowed(&self.sources[src].key)) {
                        continue;
                    }
                    if score[slot as usize] == 0.0 {
                        touched.push(slot);
                    }
                    score[slot as usize] += idf * norm[slot as usize];
                }
            }
            let mut best: Vec<(f32, u32)> = touched.iter().map(|&s| (score[s as usize], s)).collect();
            let k = top.min(best.len());
            if k > 0 && k < best.len() {
                best.select_nth_unstable_by(k - 1, |a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                best.truncate(k);
            }
            best.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            for &s in &touched {
                score[s as usize] = 0.0;
            }
            touched.clear();
            let slots: Vec<u32> = best.into_iter().map(|(_, s)| s).collect();
            wanted.extend(slots.iter().copied());
            per.push(slots);
        }
        ParaPlan { per, refs: wanted.into_iter().map(|s| self.refs[s as usize]).collect() }
    }

    /// The second step: each sentence's library sentences, in its candidate passages (texts by reference), that keep at
    /// least `floor` of its content stems in order, one per source sentence, the best first (kept plus weight). The
    /// rule (para::close) is the caller's, at its closeness.
    pub fn para_finish(&self, sentences: &[Vec<String>], plan: &ParaPlan, texts: &HashMap<i64, String>, floor: f64) -> Vec<Vec<ParaHit>> {
        sentences
            .iter()
            .zip(&plan.per)
            .map(|(mine, slots)| {
                // the sentence's stems' weights, once
                let weights: HashMap<&str, f64> = mine.iter().map(|m| (m.as_str(), self.stem_weight(m))).collect();
                let total: f64 = weights.values().sum();
                let mut out: Vec<ParaHit> = Vec::new();
                for &slot in slots {
                    let r = self.refs[slot as usize];
                    let Some(text) = texts.get(&r) else { continue };
                    for s in crate::para::passage_sentences(text) {
                        let src = crate::para::content(s);
                        let kept = crate::para::in_order(mine, &src);
                        if kept < floor {
                            continue;
                        }
                        let weight = crate::para::weighted_share(mine, &src, |w| weights.get(w).copied().unwrap_or(0.0));
                        let key = self.sources[self.owner[slot as usize] as usize].key.clone();
                        let sentence = s.split_whitespace().collect::<Vec<_>>().join(" ");
                        let round = |x: f64| (x * 1000.0).round() / 1000.0;
                        let hit = ParaHit { key, passage: r, sentence, kept: round(kept), weight: round(weight), shared: round(weight * total) };
                        match out.iter_mut().find(|h| h.key == hit.key && h.sentence == hit.sentence) {
                            Some(h) if h.kept + h.weight < hit.kept + hit.weight => *h = hit,
                            Some(_) => {}
                            None => out.push(hit),
                        }
                    }
                }
                out.sort_by(|a, b| (b.kept + b.weight).total_cmp(&(a.kept + a.weight)));
                out
            })
            .collect()
    }
}

/// A check half done (`Index::plan`): the text's shingles, the candidates, and the passages `Index::finish` reads.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    qg: Vec<Option<u64>>,
    wanted: Vec<(u32, Vec<u32>)>,
    /// The passages whose texts the check needs, by reference.
    pub refs: Vec<i64>,
}

/// A text to check: its words, with what was left out (`text::blank`) and its fixed terms marked.
pub struct Query {
    pub blanked: String,
    pub tokens: Vec<Token>,
}

impl Query {
    pub fn new(text: &str, masks: text::Masks, fixed_terms: &[String]) -> Query {
        let blanked = text::blank(text, masks);
        let mut tokens = text::tokens(&blanked);
        let terms: Vec<Vec<u64>> = fixed_terms.iter().map(|t| text::phrase(t)).collect();
        text::mark_terms(&mut tokens, &terms);
        Query { blanked, tokens }
    }
}

/// The check's settings.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The shortest run, in words, that counts (at least `K`).
    pub min_run: usize,
    /// The fewest content words a finding needs.
    pub content: usize,
    /// A run found in this many sources or more is fixed phrasing (0: never).
    pub common: usize,
    /// Whether near-verbatim stretches are looked for.
    pub near: bool,
    /// A shingle in more passages than this chooses no candidates (it is still counted for fixed phrasing).
    pub max_postings: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { min_run: 6, content: 4, common: 5, near: true, max_postings: 2000 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Verbatim,
    Near,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Verbatim => "verbatim",
            Kind::Near => "near_verbatim",
        }
    }
}

/// A stretch of the text found in a source.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub kind: Kind,
    /// Its place in the text, in bytes, and in words (`first..last`).
    pub start: usize,
    pub end: usize,
    pub first: usize,
    pub last: usize,
    /// Its words; how many match exactly; the longest exact run.
    pub words: usize,
    pub matched: usize,
    pub longest: usize,
    /// 1 for verbatim; the share of its words matched exactly for near-verbatim.
    pub score: f64,
    /// The source's key, the passage the match starts in (its reference), the match's place in that passage's text
    /// (bytes; to its end when the match goes on into the next), and the source's words that match.
    pub source: String,
    pub passage: i64,
    pub source_start: usize,
    pub source_end: usize,
    pub source_text: String,
    /// Other sources with the same stretch.
    pub also: Vec<String>,
}

/// A run that is the field's fixed phrasing: its place in the text and how many sources hold it.
#[derive(Clone, Debug, PartialEq)]
pub struct Common {
    pub start: usize,
    pub end: usize,
    pub first: usize,
    pub last: usize,
    pub sources: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub findings: Vec<Finding>,
    pub common: Vec<Common>,
    /// Sources and passages read again to align them with the text.
    pub candidates: usize,
    pub passages_read: usize,
}

/// An exact run shared by the text (from word `q`) and a stretch of a source (from word `p`), `len` words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seed {
    q: usize,
    p: usize,
    len: usize,
}

/// A stretch of consecutive passages of one source, as words.
struct Doc<'a> {
    refs: Vec<i64>,
    texts: Vec<&'a str>,
    /// Each word: (passage within the stretch, start, end in that passage's text).
    at: Vec<(usize, usize, usize)>,
    /// (shingle hash, word), sorted.
    grams: Vec<(u64, u32)>,
}

impl<'a> Doc<'a> {
    /// The passages `slots`, each's reference and text from `get`.
    fn read(slots: &[u32], get: impl Fn(u32) -> (i64, &'a str)) -> Doc<'a> {
        let (mut refs, mut texts, mut at, mut hashes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (k, s) in slots.iter().enumerate() {
            let (r, t) = get(*s);
            for w in text::tokens(t) {
                at.push((k, w.start, w.end));
                hashes.push(w.hash);
            }
            refs.push(r);
            texts.push(t);
        }
        let mut grams: Vec<(u64, u32)> = hashes.windows(K).enumerate().map(|(i, w)| (gram_hash(w), i as u32)).collect();
        grams.sort_unstable();
        Doc { refs, texts, at, grams }
    }

    fn positions(&self, g: u64) -> impl Iterator<Item = usize> + '_ {
        let a = self.grams.partition_point(|x| x.0 < g);
        self.grams[a..].iter().take_while(move |x| x.0 == g).map(|x| x.1 as usize)
    }

    /// The exact runs (at least K words) shared with the text (`qg`: the text's shingles), as chains: runs close
    /// together on both sides one chain (near-verbatim), each other run a chain of its own.
    fn chains(&self, qg: &[Option<u64>], near: bool) -> Vec<Vec<Seed>> {
        // maximal runs along each diagonal (source word - text word)
        let mut active: HashMap<i64, (usize, usize, usize)> = HashMap::new();
        let mut seeds = Vec::new();
        for (i, g) in qg.iter().enumerate() {
            let Some(g) = g else { continue };
            for p in self.positions(*g) {
                let d = p as i64 - i as i64;
                match active.get_mut(&d) {
                    Some(a) if a.2 + 1 == i => a.2 = i,
                    Some(a) => {
                        seeds.push(Seed { q: a.0, p: a.1, len: a.2 - a.0 + K });
                        *a = (i, p, i);
                    }
                    None => {
                        active.insert(d, (i, p, i));
                    }
                }
            }
        }
        seeds.extend(active.into_values().map(|a| Seed { q: a.0, p: a.1, len: a.2 - a.0 + K }));
        seeds.sort_by_key(|s| (s.q, s.p));
        seeds.dedup();
        if !near {
            return seeds.into_iter().map(|s| vec![s]).collect();
        }
        // the best chain ending at each run (most words matched), then the chains taken best first, disjoint
        let fits = |a: &Seed, b: &Seed| {
            let (qa, pa) = (a.q + a.len, a.p + a.len);
            b.q >= qa && b.p >= pa && b.q - qa <= NEAR_GAP && b.p - pa <= NEAR_GAP && (b.q - qa).abs_diff(b.p - pa) <= 2
        };
        let (mut best, mut prev) = (vec![0usize; seeds.len()], vec![usize::MAX; seeds.len()]);
        for j in 0..seeds.len() {
            best[j] = seeds[j].len;
            for i in 0..j {
                if fits(&seeds[i], &seeds[j]) && best[i] + seeds[j].len > best[j] {
                    best[j] = best[i] + seeds[j].len;
                    prev[j] = i;
                }
            }
        }
        let mut order: Vec<usize> = (0..seeds.len()).collect();
        order.sort_by_key(|&j| (std::cmp::Reverse(best[j]), j));
        let mut used = vec![false; seeds.len()];
        let mut out = Vec::new();
        for j in order {
            let mut chain = vec![j];
            while let Some(&k) = chain.last().filter(|&&k| prev[k] != usize::MAX) {
                chain.push(prev[k]);
            }
            if chain.iter().any(|k| used[*k]) {
                continue;
            }
            for k in &chain {
                used[*k] = true;
            }
            chain.reverse();
            let runs: Vec<Seed> = chain.iter().map(|k| seeds[*k]).collect();
            // a chain that is not near-verbatim gives its runs back one by one
            let matched: usize = runs.iter().map(|s| s.len).sum();
            let words = runs.last().map_or(0, |l| l.q + l.len) - runs[0].q;
            if runs.len() > 1 && (matched as f64) / (words as f64) < NEAR_COVERAGE {
                out.extend(runs.into_iter().map(|s| vec![s]));
            } else {
                out.push(runs);
            }
        }
        out
    }
}

/// The findings of all sources made one per stretch of the text: the one matching most words first; one of the same
/// source that overlaps it extends it (two runs found in different places of the source, overlapping in the text);
/// one of another source that overlaps it by half its words or more adds its source to `also`.
fn group(mut all: Vec<Finding>) -> Vec<Finding> {
    all.sort_by(|a, b| b.matched.cmp(&a.matched).then(a.start.cmp(&b.start)).then(a.source.cmp(&b.source)));
    let mut kept: Vec<Finding> = Vec::new();
    for f in all {
        let overlap = |k: &Finding| k.last.min(f.last).saturating_sub(k.first.max(f.first));
        if let Some(k) = kept.iter_mut().find(|k| k.source == f.source && overlap(k) > 0) {
            let ov = overlap(k);
            if f.first < k.first {
                (k.first, k.start) = (f.first, f.start);
            }
            if f.last > k.last {
                (k.last, k.end) = (f.last, f.end);
            }
            k.words = k.last - k.first;
            k.matched = (k.matched + f.matched - ov).min(k.words);
            k.longest = k.longest.max(f.longest);
            k.score = k.matched as f64 / k.words as f64;
            continue;
        }
        match kept.iter_mut().max_by_key(|k| overlap(k)).filter(|k| overlap(k) * 2 >= f.words) {
            Some(k) => {
                if k.source != f.source && !k.also.contains(&f.source) && k.also.len() < 10 {
                    k.also.push(f.source);
                }
            }
            None => kept.push(f),
        }
    }
    kept.sort_by_key(|f| (f.start, f.end));
    kept
}

fn dedup(mut c: Vec<Common>) -> Vec<Common> {
    c.sort_by_key(|x| (x.start, std::cmp::Reverse(x.end), std::cmp::Reverse(x.sources)));
    c.dedup_by(|b, a| a.start <= b.start && b.end <= a.end);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Masks;

    /// A small library: sources by key, each passages of text; references are 100 * source + passage.
    fn library(sources: &[(&str, &[&str])]) -> (Index, HashMap<i64, String>) {
        let mut ix = Index::new();
        let mut texts = HashMap::new();
        let add = sources.iter().enumerate().map(|(si, (key, ps))| NewSource {
            key: key.to_string(),
            sig: [si as u64, 0, 0],
            passages: ps.iter().enumerate().map(|(pi, p)| {
                let r = (100 * si + pi) as i64;
                texts.insert(r, p.to_string());
                (r, keys_of(p))
            }).collect(),
            words: ps.iter().map(|p| crate::para::word_keys(p)).collect(),
        }).collect();
        ix.update(&[], add);
        (ix, texts)
    }

    /// Close paraphrase by content words: the source sentence found among many, a restatement in other words not.
    #[test]
    fn paraphrase_candidates_by_content_words() {
        use crate::para::content;
        let filler: Vec<String> = (0..40).map(|i| format!("Unrelated passage number {i} discusses weather, rivers, valleys and the colour of autumn leaves in the hills.")).collect();
        let mut sources: Vec<(String, Vec<&str>)> = filler.iter().enumerate().map(|(i, f)| (format!("F{i}"), vec![f.as_str()])).collect();
        sources.push(("P".into(), vec!["We describe the setting. Error correction is weakened if the errors are correlated, in other words, if many bit positions are simultaneously incorrect."]));
        let refs: Vec<(&str, &[&str])> = sources.iter().map(|(k, ps)| (k.as_str(), ps.as_slice())).collect();
        let (ix, texts) = library(&refs);
        let mine = vec![
            content("Error correction is weakened when the errors are correlated, that is, when many bit positions are wrong at the same time."),
            content("Errors that occur together, flipping several bits at once, undermine the code's ability to fix them."),
            content("Too short here."),
        ];
        let plan = ix.para_plan(&mine, &|_| true, PARA_CANDIDATES);
        assert_eq!(plan.per[0].first().map(|s| ix.refs[*s as usize]), Some(4000), "the source passage first");
        assert!(plan.per[2].is_empty(), "too few content words");
        let hits = ix.para_finish(&mine, &plan, &texts, 0.3);
        let h = &hits[0][0];
        assert_eq!(h.key, "P");
        assert!(h.sentence.starts_with("Error correction is weakened if"));
        assert!(h.kept >= 0.75 && h.weight > 0.7, "{h:?}");
        assert!(h.kept >= crate::para::CLOSENESS && h.weight >= crate::para::WEIGHT && h.shared > 0.0, "{h:?}");
        assert!(hits[1].iter().all(|h| !crate::para::close(&mine[1], h.kept, h.weight, h.shared, crate::para::CLOSENESS)), "{:?}", hits[1]);
        // stored and loaded with the word index
        let back = Index::from_bytes(&ix.to_bytes()).unwrap();
        assert_eq!(back, ix);
        // the source removed: no candidate from it
        let mut ix2 = ix.clone();
        ix2.update(&["P".to_string()], vec![]);
        assert!(ix2.para_plan(&mine, &|_| true, PARA_CANDIDATES).per[0].iter().all(|s| ix2.refs[*s as usize] != 4000));
    }

    fn check(ix: &Index, texts: &HashMap<i64, String>, draft: &str, o: &Options) -> Found {
        let q = Query::new(draft, Masks { latex: false, ..Masks::ALL }, &[]);
        ix.find(&q, o, &|_| true, &mut |refs| Ok(refs.iter().filter_map(|r| texts.get(r).map(|t| (*r, t.clone()))).collect())).unwrap()
    }

    const SOURCE: &str = "Earlier studies reported that the proposed sampling procedure reduces the variance of every estimator considerably, \
        although the improvement depends on the size of the training corpus.";

    #[test]
    fn verbatim_runs_with_case_punctuation_and_hyphens() {
        let (ix, texts) = library(&[("A", &[SOURCE]), ("B", &["Nothing in common here at all, only other words about gardens and rivers."])]);
        let o = Options::default();
        // exact
        let f = check(&ix, &texts, "We note that the proposed sampling procedure reduces the variance of every estimator, as others did.", &o);
        assert_eq!(f.findings.len(), 1, "{f:?}");
        let x = &f.findings[0];
        assert_eq!((x.kind, x.source.as_str(), x.longest), (Kind::Verbatim, "A", 11));
        assert_eq!(x.source_text, "that the proposed sampling procedure reduces the variance of every estimator");
        assert_eq!(x.passage, 0);
        // case, punctuation, white space and a hyphen differ
        let f = check(&ix, &texts, "THE PROPOSED Sampling-Procedure, reduces   the\nvariance; of every estimator!", &o);
        assert_eq!(f.findings.len(), 1, "{f:?}");
        assert_eq!(f.findings[0].kind, Kind::Verbatim);
        // shorter than the run asked for: nothing
        let f = check(&ix, &texts, "the proposed sampling procedure was new", &o);
        assert!(f.findings.is_empty(), "{f:?}");
    }

    #[test]
    fn quoted_text_is_left_out() {
        let (ix, texts) = library(&[("A", &[SOURCE])]);
        let o = Options::default();
        let f = check(&ix, &texts, "They wrote that \"the proposed sampling procedure reduces the variance of every estimator\" (Smith, 2020).", &o);
        assert!(f.findings.is_empty(), "{f:?}");
        let f = check(&ix, &texts, "They wrote that “the proposed sampling procedure reduces the variance of every estimator”.", &o);
        assert!(f.findings.is_empty(), "{f:?}");
        // without the mask, it is found
        let q = Query::new("They wrote that \"the proposed sampling procedure reduces the variance of every estimator\".", Masks::NONE, &[]);
        let f = ix.find(&q, &o, &|_| true, &mut |refs| Ok(refs.iter().map(|r| (*r, texts[r].clone())).collect())).unwrap();
        assert_eq!(f.findings.len(), 1);
    }

    #[test]
    fn near_verbatim_with_a_word_changed() {
        let (ix, texts) = library(&[("A", &[SOURCE])]);
        let o = Options::default();
        let draft = "Earlier studies found that the proposed sampling procedure lowers the variance of every estimator considerably.";
        let f = check(&ix, &texts, draft, &o);
        assert_eq!(f.findings.len(), 1, "{f:?}");
        let x = &f.findings[0];
        assert_eq!(x.kind, Kind::Near);
        assert!(x.score >= NEAR_COVERAGE && x.score < 1.0);
        assert_eq!(&draft[x.start..x.end], "that the proposed sampling procedure lowers the variance of every estimator considerably");
        // off: only the exact runs of six words or more
        let f = check(&ix, &texts, draft, &Options { near: false, ..o });
        assert!(f.findings.iter().all(|x| x.kind == Kind::Verbatim));
    }

    #[test]
    fn runs_across_passages_and_fixed_phrasing() {
        let (ix, texts) = library(&[("A", &["The first passage ends with the words of a long", "sentence that goes on into the next passage of the same source."])]);
        let f = check(&ix, &texts, "Here the words of a long sentence that goes on into the next passage appear.", &Options::default());
        assert_eq!(f.findings.len(), 1, "{f:?}");
        assert_eq!(f.findings[0].longest, 13);
        assert_eq!(f.findings[0].source_text, "the words of a long sentence that goes on into the next passage");
        // the same run in five sources: fixed phrasing
        let p = "results are reported as the mean and standard deviation over five independent runs";
        let (ix, texts) = library(&[("A", &[p]), ("B", &[p]), ("C", &[p]), ("D", &[p]), ("E", &[p])]);
        let f = check(&ix, &texts, &format!("All {p} here."), &Options::default());
        assert!(f.findings.is_empty() && f.common.len() == 1, "{f:?}");
        assert_eq!(f.common[0].sources, 5);
        let f = check(&ix, &texts, &format!("All {p} here."), &Options { common: 0, ..Options::default() });
        assert_eq!(f.findings.len(), 1);
        assert_eq!(f.findings[0].also.len(), 4, "one finding for the stretch, the other sources with it");
    }

    /// Two runs from different places of one source that overlap in the text are one stretch.
    #[test]
    fn overlapping_runs_of_one_source_are_one_finding() {
        let src = "Article 9 of the general regulation applies to records. Elsewhere: the general regulation applies to every public body in the region.";
        let (ix, texts) = library(&[("A", &[src])]);
        let draft = "Article 9 of the general regulation applies to every public body in the region.";
        let f = check(&ix, &texts, draft, &Options::default());
        assert_eq!(f.findings.len(), 1, "{f:?}");
        let x = &f.findings[0];
        assert_eq!(&draft[x.start..x.end], "Article 9 of the general regulation applies to every public body in the region");
        assert_eq!((x.longest, x.words, x.matched), (11, 14, 14));
    }

    #[test]
    fn function_words_and_terms_do_not_make_a_run() {
        let (ix, texts) = library(&[("A", &["it is one of the most widely used methods in the field of the study of language"])]);
        let f = check(&ix, &texts, "This is one of the most widely used methods in the field today.", &Options::default());
        assert_eq!(f.findings.len(), 1, "content words: widely, used, methods, field");
        let q = Query::new("This is one of the most widely used methods in the field today.", Masks::ALL, &["widely used methods".into()]);
        let f2 = ix.find(&q, &Options::default(), &|_| true, &mut |refs| Ok(refs.iter().map(|r| (*r, texts[r].clone())).collect())).unwrap();
        assert!(f2.findings.is_empty(), "with a fixed term, a single content word is left: {f2:?}");
    }

    #[test]
    fn offsets_are_right_for_non_ascii() {
        let src = "Die Größe der Stichprobe beeinflusst die Varianz jedes Schätzers erheblich, wie frühere Arbeiten zeigten.";
        let (ix, texts) = library(&[("A", &[src])]);
        let draft = "😀 Ünd: die Größe der Stichprobe beeinflusst die Varianz jedes Schätzers sehr.";
        let f = check(&ix, &texts, draft, &Options::default());
        assert_eq!(f.findings.len(), 1, "{f:?}");
        let x = &f.findings[0];
        assert_eq!(&draft[x.start..x.end], "die Größe der Stichprobe beeinflusst die Varianz jedes Schätzers");
        let u = crate::text::utf16_offsets(draft, &[x.start, x.end]);
        let units: Vec<u16> = draft.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[u[0]..u[1]]).unwrap(), &draft[x.start..x.end]);
        assert_eq!(x.source_text, "Die Größe der Stichprobe beeinflusst die Varianz jedes Schätzers");
    }

    #[test]
    fn scope_and_incremental_updates() {
        let (mut ix, mut texts) = library(&[("A", &[SOURCE]), ("B", &["Another source about rivers, mountains and the weather of distant valleys in spring."])]);
        let draft = "the proposed sampling procedure reduces the variance of every estimator";
        assert!(check(&ix, &texts, draft, &Options::default()).findings.len() == 1);
        // out of scope: not found
        let q = Query::new(draft, Masks::ALL, &[]);
        let f = ix.find(&q, &Options::default(), &|k| k != "A", &mut |refs| Ok(refs.iter().map(|r| (*r, texts[r].clone())).collect())).unwrap();
        assert!(f.findings.is_empty());
        // A replaced by a corrected note; B untouched; C added; then A removed
        let before = ix.stats();
        let corrected = "Later studies showed the sampling procedure changed nothing at all.";
        texts.insert(900, corrected.into());
        ix.update(&[], vec![NewSource { key: "A".into(), sig: [9, 0, 0], passages: vec![(900, keys_of(corrected))], words: vec![] }]);
        assert!(check(&ix, &texts, draft, &Options::default()).findings.is_empty(), "the old text is gone");
        assert_eq!(ix.passages_of("B"), &[100]);
        assert_eq!(ix.signatures().find(|s| s.0 == "A").unwrap().1, [9, 0, 0]);
        texts.insert(901, SOURCE.into());
        ix.update(&[], vec![NewSource { key: "C".into(), sig: [1, 2, 3], passages: vec![(901, keys_of(SOURCE))], words: vec![] }]);
        let f = check(&ix, &texts, draft, &Options::default());
        assert_eq!(f.findings[0].source, "C");
        ix.update(&["A".into(), "C".into()], vec![]);
        assert_eq!(ix.stats().sources, 1);
        assert!(ix.stats().shingles < before.shingles);
        // the same index as one made from scratch with B alone
        let (fresh, _) = library(&[("B", &["Another source about rivers, mountains and the weather of distant valleys in spring."])]);
        assert_eq!(ix.postings.iter().map(|e| e >> 32).collect::<Vec<_>>(), fresh.postings.iter().map(|e| e >> 32).collect::<Vec<_>>());
    }

    #[test]
    fn saved_and_loaded() {
        let (ix, _) = library(&[("A", &[SOURCE, "a second passage"]), ("B", &["Another source."])]);
        let path = std::env::temp_dir().join(format!("authenticity-similar-{}.bin", std::process::id()));
        ix.save(&path).unwrap();
        let back = Index::load(&path).unwrap();
        assert_eq!(back, ix);
        std::fs::write(&path, b"AUSI\x09\x00\x00\x00").unwrap();
        assert!(Index::load(&path).is_err(), "another version is made again");
        std::fs::write(&path, b"QPSI\x01\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00\x00").unwrap();
        assert!(Index::load(&path).is_err(), "another program's file is not read as Authenticity's");
        _ = std::fs::remove_file(&path);
        // as bytes, the same; damaged bytes never ask for more memory than they hold
        let bytes = ix.to_bytes();
        assert_eq!(Index::from_bytes(&bytes).unwrap(), ix);
        let mut big = bytes.clone();
        big.truncate(12);
        big.extend_from_slice(&u32::MAX.to_le_bytes()); // four billion sources
        assert!(Index::from_bytes(&big).unwrap_err().to_string().contains("more sources"));
        let mut lying = Index::new().to_bytes();
        let at = lying.len() - 24; // the passage count of an empty index (then the shingle and word counts)
        lying[at..at + 8].copy_from_slice(&(u64::MAX / 16).to_le_bytes());
        assert!(Index::from_bytes(&lying).unwrap_err().to_string().contains("more passages"));
    }

    /// The check in two steps gives what `find` gives, and asks only for the candidates' passages.
    #[test]
    fn plan_then_finish() {
        let (ix, texts) = library(&[("A", &[SOURCE]), ("B", &["Nothing in common here at all, only other words about gardens and rivers."])]);
        let q = Query::new("We note that the proposed sampling procedure reduces the variance of every estimator.", Masks::ALL, &[]);
        let o = Options::default();
        let plan = ix.plan(&q, &o, &|_| true);
        assert_eq!(plan.refs, vec![0], "only source A's passage is read");
        let wanted: HashMap<i64, String> = plan.refs.iter().map(|r| (*r, texts[r].clone())).collect();
        let two = ix.finish(&q, &o, &plan, &wanted);
        let one = ix.find(&q, &o, &|_| true, &mut |refs| Ok(refs.iter().map(|r| (*r, texts[r].clone())).collect())).unwrap();
        assert_eq!(two, one);
        // a passage that could not be read matches nothing (never a guess)
        assert!(ix.finish(&q, &o, &plan, &HashMap::new()).findings.is_empty());
        // signatures change in place
        let mut ix = ix;
        assert!(ix.has("A") && ix.set_signature("A", [7, 7, 7]) && ix.signature("A") == Some([7, 7, 7]));
        assert!(!ix.set_signature("Z", [1, 1, 1]));
    }
}
