//! The library's shingle index in the plugin (../similar). It is made the first time a plagiarism check needs it, from
//! the library notes read through Obsidian's vault, and kept on this computer outside the vault (so it is not
//! synchronised): `<local data>/authenticity/<vault id>/library.idx`, the same file Authenticity's native programs
//! (check-overlap, the MCP server) read and keep. After that it is kept up to date note by note:
//!
//! - each check first compares the library notes' modification times and sizes with the index's and reads only those
//!   that differ (a note whose text is unchanged only has its time noted);
//! - a note changed, added, moved or deleted in the library folder while Obsidian runs is indexed again a few seconds
//!   later, with any others changed meanwhile, in one update of the index;
//!
//! so a few changed notes never rebuild the index. "Rebuild the library index" makes it again from every note.
//!
//! The index holds every note of the library folder; which of them count as sources (never the excluded folders, never
//! the note being checked) is decided at each check.

use std::collections::{BTreeSet, HashMap};

use authenticity_similar::{library as lib, Index, NewSource};
use wasm_bindgen::prelude::*;

use crate::app::with;
use crate::js::{self, call, get, str};
use crate::library::Scope;
use crate::similarity::Match;

/// Notes read before the plugin pauses for the editor (a first index reads every library note).
const BATCH: usize = 25;
/// Quiet time after a library note changes before it is indexed again, and before the index is written out.
const SETTLE_MS: f64 = 3000.0;
const SAVE_MS: f64 = 20_000.0;

#[derive(Default)]
pub struct State {
    ix: Option<Index>,
    /// Each held note's key by its hash (passage numbers carry the hash).
    keys: HashMap<i64, String>,
    building: bool,
    /// The library folder the index was last brought up to date with.
    pub library: String,
    /// Notes changed while Obsidian ran, waiting to be indexed again.
    dirty: BTreeSet<String>,
    timer: Option<f64>,
    save_timer: Option<f64>,
    pub progress: Option<String>,
}

impl State {
    pub fn cancel(&mut self) {
        for t in [self.timer.take(), self.save_timer.take()].into_iter().flatten() {
            js::clear_timeout(t);
        }
    }

    /// The library folder or the exclusions changed: the next check syncs with the new scope.
    pub fn scope_changed(&mut self) {
        self.library.clear();
    }

    pub fn describe(&self) -> String {
        if let Some(p) = &self.progress {
            return p.clone();
        }
        match &self.ix {
            None => "Made the first time you check for plagiarism, from the notes in the library folder; then kept up to date as they change, and stored on this computer outside the vault.".into(),
            Some(ix) => {
                let s = ix.stats();
                format!("{} notes, {} passages, {:.1} MB, kept up to date as the library changes (folder {}).", s.sources, s.passages, s.bytes as f64 / 1_048_576.0, self.library)
            }
        }
    }

    fn refresh_keys(&mut self) {
        self.keys = self.ix.as_ref().map(|ix| ix.signatures().map(|(k, _)| (lib::key_hash(k), k.to_string())).collect()).unwrap_or_default();
    }
}

fn node(m: &str) -> JsValue {
    with(|a| js::apply(&a.require, &JsValue::NULL, &[m.into()])).unwrap_or(JsValue::UNDEFINED)
}

use crate::library::vault_id;

/// Where the index is kept on this computer, or None (no vault on disk, no local data folder).
fn cache_file() -> Option<String> {
    let base = with(|a| call!(get(&get(&a.app, "vault"), "adapter"), getBasePath)).filter(|b| b.is_string()).map(|b| str(&b))?;
    let process = get(&js::global(), "process");
    let windows = str(&get(&process, "platform")) == "win32";
    let env = get(&process, "env");
    let var = |k: &str| Some(str(&get(&env, k))).filter(|v| !v.is_empty());
    let root = var("LOCALAPPDATA").or_else(|| var("XDG_CACHE_HOME")).or_else(|| var("HOME").map(|h| format!("{h}/.cache")))?;
    let sep = if windows { "\\" } else { "/" };
    Some(format!("{root}{sep}authenticity{sep}{}{sep}library.idx", vault_id(&base, windows)))
}

fn load_cache() -> Option<Index> {
    let path = cache_file()?;
    let fs = node("fs");
    if !call!(fs, existsSync, path.clone()).is_truthy() {
        return None;
    }
    let read = get(&fs, "readFileSync").dyn_into::<js_sys::Function>().ok()?;
    let buf = read.call1(&fs, &path.into()).ok()?;
    Index::from_bytes(&js_sys::Uint8Array::new(&buf).to_vec()).ok()
}

fn save_cache(bytes: Vec<u8>) {
    let Some(path) = cache_file() else { return };
    let fs = node("fs");
    let dir = path.rsplit_once(['\\', '/']).map(|(d, _)| d.to_string()).unwrap_or_default();
    let tmp = format!("{path}.tmp");
    let options = js_sys::Object::new();
    js::set(&options, "recursive", true);
    let write = || -> Result<(), JsValue> {
        let f = |name: &str| get(&fs, name).dyn_into::<js_sys::Function>();
        f("mkdirSync")?.call2(&fs, &dir.clone().into(), &options)?;
        f("writeFileSync")?.call2(&fs, &tmp.clone().into(), &js_sys::Uint8Array::from(&bytes[..]))?;
        f("renameSync")?.call2(&fs, &tmp.clone().into(), &path.clone().into())?;
        Ok(())
    };
    if let Err(e) = write() {
        with(|a| a.notice(&format!("the library index could not be stored on this computer ({}); it is made again next time.", str(&get(&e, "message")))));
    }
}

/// Write the index out once changes have settled.
fn save_later() {
    with(|a| {
        if let Some(t) = a.index.save_timer.take() {
            js::clear_timeout(t);
        }
        a.index.save_timer = Some(js::timeout(SAVE_MS, || {
            let bytes = with(|a| {
                a.index.save_timer = None;
                a.index.ix.as_ref().map(Index::to_bytes)
            })
            .flatten();
            if let Some(b) = bytes {
                save_cache(b);
            }
        }));
    });
}

/// The library folder now: this plugin's setting (Settings → Plagiarism), or an error that asks for it.
fn library_folder() -> Result<String, String> {
    let set = with(|a| a.settings.library_folder.clone()).unwrap_or_default();
    let scope = Scope::new(&set, "");
    if scope.is_set() { Ok(scope.library) } else { Err(crate::library::SET_LIBRARY.into()) }
}

/// The library notes now: (path, modification time in ms, size).
fn files(library: &str) -> Vec<(String, u64, u64)> {
    let Some(list) = with(|a| call!(get(&a.app, "vault"), getMarkdownFiles)) else { return Vec::new() };
    js_sys::Array::from(&list)
        .iter()
        .filter_map(|f| {
            let path = str(&get(&f, "path"));
            path.starts_with(library).then(|| {
                let stat = get(&f, "stat");
                (path, get(&stat, "mtime").as_f64().unwrap_or(0.0) as u64, get(&stat, "size").as_f64().unwrap_or(0.0) as u64)
            })
        })
        .collect()
}

async fn read(path: &str) -> Option<String> {
    let vault = with(|a| get(&a.app, "vault"))?;
    let file = call!(vault, getAbstractFileByPath, path);
    if file.is_null() || file.is_undefined() || get(&file, "stat").is_undefined() {
        return None;
    }
    js::wait(call!(vault, cachedRead, file)).await.ok().map(|t| str(&t))
}

/// Bring the index up to date with the library notes (made from nothing when `fresh`): read what changed, in batches,
/// then one update.
async fn sync(fresh: bool) -> Result<(), String> {
    if with(|a| a.index.building).unwrap_or(true) {
        // another sync is running: wait for it
        while with(|a| a.index.building).unwrap_or(false) {
            js::sleep(200.0).await;
        }
        return Ok(());
    }
    with(|a| a.index.building = true);
    let library = match library_folder() {
        Ok(l) => l,
        Err(e) => {
            with(|a| a.index.building = false);
            return Err(e);
        }
    };
    let mut ix = with(|a| if fresh { None } else { a.index.ix.take() }).flatten().or_else(|| if fresh { None } else { load_cache() }).unwrap_or_default();
    let now = files(&library);
    let plan = lib::plan_sync(ix.signatures(), &now);
    let stats: HashMap<&str, (u64, u64)> = now.iter().map(|(p, t, s)| (p.as_str(), (*t, *s))).collect();
    let mut add: Vec<NewSource> = Vec::new();
    let mut remove = plan.remove.clone();
    let total = plan.read.len();
    let run = crate::app::run_now();
    for (n, path) in plan.read.iter().enumerate() {
        // Cancel stops the reading; what was read is indexed (the rest is read at the next check)
        if crate::app::cancelled(run) {
            break;
        }
        if n % BATCH == 0 && total > BATCH {
            with(|a| {
                a.index.progress = Some(format!("Indexing the library: {n} of {total} notes\u{2026}"));
                a.busy(a.index.progress.clone());
            });
            js::sleep(0.0).await;
        }
        let (t, s) = stats.get(path.as_str()).copied().unwrap_or_default();
        match read(path).await {
            Some(text) => {
                let sig = lib::signature(t, s, &text);
                if ix.signature(path).is_some_and(|old| old[2] == sig[2]) {
                    ix.set_signature(path, sig); // touched, not changed
                } else {
                    add.push(lib::source(path, &text, sig));
                }
            }
            None => remove.push(path.clone()),
        }
    }
    let changed = !add.is_empty() || !remove.is_empty() || total > 0;
    ix.update(&remove, add);
    with(|a| {
        a.index.ix = Some(ix);
        a.index.refresh_keys();
        a.index.library = library;
        a.index.building = false;
        a.index.progress = None;
        a.busy(None);
    });
    if changed || fresh {
        if fresh {
            if let Some(Some(b)) = with(|a| a.index.ix.as_ref().map(Index::to_bytes)) {
                save_cache(b);
            }
        } else {
            save_later();
        }
    }
    Ok(())
}

/// Make the index again from every library note.
pub async fn rebuild() {
    if let Err(e) = sync(true).await {
        with(|a| a.notice(&e));
        return;
    }
    with(|a| a.notice(&a.index.describe()));
}

/// A note changed (or was deleted, or moved from `old`): if it is (or was) in the library and the index is in use, it
/// is indexed again once changes settle.
pub fn changed(path: &str, old: Option<&str>) {
    with(|a| {
        let lib = a.index.library.clone();
        if a.index.ix.is_none() || lib.is_empty() {
            return; // not made yet: the next check syncs
        }
        let mut any = false;
        for p in [Some(path), old].into_iter().flatten() {
            if p.starts_with(&lib) && p.ends_with(".md") {
                a.index.dirty.insert(p.to_string());
                any = true;
            }
        }
        if !any {
            return;
        }
        if let Some(t) = a.index.timer.take() {
            js::clear_timeout(t);
        }
        a.index.timer = Some(js::timeout(SETTLE_MS, || js::spawn(flush())));
    });
}

/// Index again the notes changed while Obsidian ran, in one update.
async fn flush() {
    let Some(dirty) = with(|a| {
        a.index.timer = None;
        std::mem::take(&mut a.index.dirty)
    }) else { return };
    if dirty.is_empty() || with(|a| a.index.building).unwrap_or(true) {
        with(|a| a.index.dirty.extend(dirty));
        return;
    }
    let (mut add, mut remove) = (Vec::new(), Vec::new());
    for path in &dirty {
        let stat = with(|a| {
            let f = call!(get(&a.app, "vault"), getAbstractFileByPath, path.as_str());
            (!f.is_null() && !f.is_undefined()).then(|| get(&f, "stat")).filter(|s| !s.is_undefined()).map(|s| (get(&s, "mtime").as_f64().unwrap_or(0.0) as u64, get(&s, "size").as_f64().unwrap_or(0.0) as u64))
        })
        .flatten();
        match (stat, read(path).await) {
            (Some((t, s)), Some(text)) => add.push(lib::source(path, &text, lib::signature(t, s, &text))),
            _ => remove.push(path.clone()),
        }
    }
    with(|a| {
        if let Some(ix) = a.index.ix.as_mut() {
            ix.update(&remove, add);
        }
        a.index.refresh_keys();
    });
    save_later();
}

/// The plagiarism check of `text` (the note `path`'s text, or a selection of it starting at UTF-16 place `base`)
/// against the library: verbatim, near-verbatim and fixed phrasing.
pub async fn check(path: &str, text: &str, base: u32) -> Result<Vec<Match>, String> {
    sync(false).await?;
    let Some((settings, exclude)) = with(|a| (a.settings.similarity(), a.settings.exclude.clone())) else { return Err("the plugin is unloading".into()) };
    let library = with(|a| a.index.library.clone()).unwrap_or_default();
    let scope = Scope::new(&library, &exclude);
    let q = settings.query(text);
    let o = settings.options();
    let me = path.to_string();
    let Some(Some((plan, keys, empty))) = with(|a| a.index.ix.as_ref().map(|ix| (ix.plan(&q, &o, &|k| k != me && scope.holds(k)), a.index.keys.clone(), ix.stats().sources == 0))) else {
        return Err("the library index could not be made".into());
    };
    if empty {
        return Err(format!("there are no notes in the library folder ({library}) to compare with: set the Library folder (Settings \u{2192} Authenticity \u{2192} Plagiarism) to the folder of your source notes"));
    }
    with(|a| a.busy(Some("Plagiarism: comparing with the library\u{2026}".into())));
    // the candidates' notes, each read once
    let mut wanted: HashMap<String, Option<String>> = HashMap::new();
    let run = crate::app::run_now();
    for r in &plan.refs {
        if crate::app::cancelled(run) {
            return Err(crate::sloptotal::CANCELLED.into());
        }
        if let Some(k) = keys.get(&lib::ref_parts(*r).0) {
            if !wanted.contains_key(k) {
                wanted.insert(k.clone(), read(k).await);
            }
        }
    }
    let texts = lib::passage_texts(&plan.refs, &keys, |k| wanted.get(k).cloned().flatten());
    let found = with(|a| a.index.ix.as_ref().map(|ix| ix.finish(&q, &o, &plan, &texts))).flatten().unwrap_or_default();
    Ok(crate::similarity::verbatim(text, base, &found, |k| crate::library::title(k, &library), &texts, |k| wanted.get(k).cloned().flatten()))
}

/// The byte ranges of `text` (at UTF-16 place `base` in the note) that the verbatim findings cover.
pub fn covered(text: &str, base: u32, verbatim: &[Match]) -> Vec<std::ops::Range<usize>> {
    verbatim
        .iter()
        .filter(|m| m.kind != "common")
        .filter_map(|m| {
            let (a, _) = crate::app::slice16(text, m.from.saturating_sub(base), m.to.saturating_sub(base));
            let start = text.find(&a)?;
            Some(start..start + a.len())
        })
        .collect()
}

/// Close paraphrases of `text`'s sentences (those the verbatim findings do not mostly cover) in the library, by the
/// index's content-word search (no model, nothing leaves the computer; para.rs's rule), each sentence with its
/// candidates from closeness `floor` up; `check` has brought the index up to date first.
pub async fn paraphrase(path: &str, text: &str, base: u32, verbatim: &[Match], floor: f64) -> Result<Vec<Match>, String> {
    let Some((settings, exclude)) = with(|a| (a.settings.similarity(), a.settings.exclude.clone())) else { return Err("the plugin is unloading".into()) };
    let library = with(|a| a.index.library.clone()).unwrap_or_default();
    let scope = Scope::new(&library, &exclude);
    let sentences = crate::similarity::paraphrase_sentences(text, &settings, &covered(text, base, verbatim));
    if sentences.is_empty() {
        return Ok(Vec::new());
    }
    let me = path.to_string();
    let Some(Some((stems, plan, keys))) = with(|a| {
        a.index.ix.as_ref().map(|ix| {
            let (stems, plan) = crate::similarity::local_plan(ix, &sentences, &|k| k != me && scope.holds(k));
            (stems, plan, a.index.keys.clone())
        })
    }) else {
        return Err("the library index could not be made".into());
    };
    with(|a| a.busy(Some(format!("Plagiarism: looking for close paraphrase of {} sentences\u{2026}", sentences.len()))));
    // the candidates' notes, each read once
    let mut wanted: HashMap<String, Option<String>> = HashMap::new();
    let run = crate::app::run_now();
    for r in &plan.refs {
        if crate::app::cancelled(run) {
            return Err(crate::sloptotal::CANCELLED.into());
        }
        if let Some(k) = keys.get(&lib::ref_parts(*r).0) {
            if !wanted.contains_key(k) {
                wanted.insert(k.clone(), read(k).await);
            }
        }
    }
    let texts = lib::passage_texts(&plan.refs, &keys, |k| wanted.get(k).cloned().flatten());
    let hits = with(|a| a.index.ix.as_ref().map(|ix| crate::similarity::local_hits(ix, &stems, &plan, &texts, floor, |k| crate::library::title(k, &library), |k| wanted.get(k).cloned().flatten()))).flatten().unwrap_or_default();
    Ok(crate::similarity::paraphrase_matches(text, base, &sentences, &hits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_ids_ignore_separators_and_case_on_windows() {
        assert_eq!(vault_id("C:\\Users\\Me\\Vault\\", true), vault_id("c:/users/me/vault", true));
        assert_ne!(vault_id("/home/me/Vault", false), vault_id("/home/me/vault", false));
    }
}
