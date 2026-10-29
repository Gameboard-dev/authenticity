//! The vault from disk, for the native programs: its library notes, the library folder (the argument, else
//! Authenticity's setting; with neither, the checks ask for it), Authenticity's settings that matter here (fixed terms, the
//! excluded folders), and the library's shingle index, kept in the same file and format as the plugin keeps it
//! (`<local data>/authenticity/<vault id>/library.idx`) and brought up to date the same way: only notes whose time or
//! size changed are read, only those whose text changed are indexed again.
//!
//! Paths read from the vault's settings are vault folders, checked (authenticity_core::library::folder); nothing in
//! them is run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use authenticity_core::library::{self, Scope};
use authenticity_similar::{library as lib, Index};
use serde_json::Value;

pub struct Vault {
    pub root: PathBuf,
    pub config: String,
}

impl Vault {
    /// The vault at `dir`, or the nearest folder above `from` that holds `.obsidian`.
    pub fn find(dir: Option<&str>, from: &Path, config: &str) -> Result<Vault, String> {
        let root = match dir {
            Some(d) => PathBuf::from(d),
            None => {
                let start = std::path::absolute(from).map_err(|e| e.to_string())?;
                start.ancestors().find(|a| a.join(config).is_dir()).map(Path::to_path_buf).ok_or("no vault above the file (no .obsidian folder): give --vault DIR")?
            }
        };
        let root = std::path::absolute(&root).map_err(|e| e.to_string())?;
        if !root.is_dir() {
            return Err(format!("{} is not a folder", root.display()));
        }
        Ok(Vault { root, config: config.into() })
    }

    fn json(&self, rel: &str) -> Value {
        std::fs::read(self.root.join(&self.config).join(rel)).ok().filter(|b| b.len() < (8 << 20)).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
    }

    /// Authenticity's settings (data.json).
    pub fn settings(&self) -> Value {
        self.json("plugins/authenticity/data.json")
    }

    /// The library folder (vault path): `explicit`, else Authenticity's setting; empty when neither is a vault folder.
    pub fn library(&self, explicit: Option<&str>) -> String {
        explicit
            .and_then(library::folder)
            .filter(|f| !f.is_empty())
            .or_else(|| self.settings()["libraryFolder"].as_str().and_then(library::folder).filter(|f| !f.is_empty()))
            .unwrap_or_default()
    }

    /// The scope: the library folder, Authenticity's excluded folders, and `extra`.
    pub fn scope(&self, explicit: Option<&str>, extra: &[String]) -> Scope {
        let mut ex = self.settings()["exclude"].as_str().unwrap_or("").to_string();
        for e in extra {
            ex.push(',');
            ex.push_str(e);
        }
        Scope::new(&self.library(explicit), &ex)
    }

    /// Authenticity's fixed terms.
    pub fn fixed_terms(&self) -> Vec<String> {
        authenticity_core::similarity::terms(self.settings()["fixedTerms"].as_str().unwrap_or(""))
    }

    /// A vault path of a file under the vault (with `/`), or None if it is not under it.
    pub fn rel(&self, p: &Path) -> Option<String> {
        let a = std::path::absolute(p).ok()?;
        Some(a.strip_prefix(&self.root).ok()?.to_string_lossy().replace('\\', "/"))
    }

    pub fn read(&self, rel: &str) -> Option<String> {
        if rel.split('/').any(|p| p == "..") {
            return None;
        }
        std::fs::read(self.root.join(rel)).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// The notes of the library folder: (vault path, modification time in ms, size).
    pub fn library_files(&self, folder: &str) -> Vec<(String, u64, u64)> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.join(folder)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let Ok(m) = e.metadata() else { continue };
                if m.is_dir() {
                    if !e.file_name().to_string_lossy().starts_with('.') {
                        stack.push(p);
                    }
                } else if p.extension().is_some_and(|x| x == "md") {
                    if let Some(rel) = self.rel(&p) {
                        let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
                        out.push((rel, t, m.len()));
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// The plugin's id for this vault (index.rs `vault_id`).
    pub fn id(&self) -> String {
        library::vault_id(&self.root.to_string_lossy(), cfg!(windows))
    }

    /// Where the plugin keeps the library index on this computer.
    pub fn cache_file(&self) -> Option<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        Some(base.join("authenticity").join(self.id()).join("library.idx"))
    }

    /// The library index, up to date: the stored one, with the notes that changed read again; stored back if changed.
    pub fn index(&self, folder: &str) -> (Index, HashMap<i64, String>) {
        let cache = self.cache_file();
        let mut ix = cache.as_ref().and_then(|c| Index::load(c).ok()).unwrap_or_default();
        let now = self.library_files(folder);
        let plan = lib::plan_sync(ix.signatures(), &now);
        let stats: HashMap<&str, (u64, u64)> = now.iter().map(|(p, t, s)| (p.as_str(), (*t, *s))).collect();
        let (mut add, mut remove) = (Vec::new(), plan.remove.clone());
        for path in &plan.read {
            let (t, s) = stats.get(path.as_str()).copied().unwrap_or_default();
            match self.read(path) {
                Some(text) => {
                    let sig = lib::signature(t, s, &text);
                    if ix.signature(path).is_some_and(|old| old[2] == sig[2]) {
                        ix.set_signature(path, sig);
                    } else {
                        add.push(lib::source(path, &text, sig));
                    }
                }
                None => remove.push(path.clone()),
            }
        }
        let changed = !plan.read.is_empty() || !remove.is_empty();
        ix.update(&remove, add);
        if changed {
            if let Some(c) = &cache {
                if let Err(e) = ix.save(c) {
                    eprintln!("the library index could not be stored at {} ({e}); it is made again next time", c.display());
                }
            }
        }
        let keys = ix.signatures().map(|(k, _)| (lib::key_hash(k), k.to_string())).collect();
        (ix, keys)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A vault in the temporary folder: `.obsidian`, and the files given.
    pub fn vault(name: &str, files: &[(&str, &str)]) -> Vault {
        let root = std::env::temp_dir().join(format!("auth-vault-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        for (p, t) in files {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        }
        Vault { root, config: ".obsidian".into() }
    }

    #[test]
    fn the_library_folder_from_the_argument_else_the_setting() {
        let v = vault("folder", &[]);
        assert_eq!(v.library(None), "", "not set: the checks ask for it");
        assert!(!v.scope(None, &[]).is_set());
        std::fs::create_dir_all(v.root.join(".obsidian/plugins/authenticity")).unwrap();
        std::fs::write(v.root.join(".obsidian/plugins/authenticity/data.json"), r#"{"libraryFolder":"Papers/","fixedTerms":"neural network","exclude":"Mine/"}"#).unwrap();
        assert_eq!(v.library(None), "Papers/");
        assert_eq!(v.library(Some("Other")), "Other/");
        assert_eq!(v.library(Some("../../etc")), "Papers/", "not a vault path: ignored");
        assert_eq!(v.fixed_terms(), ["neural network"]);
        assert!(v.scope(None, &[]).exclude.contains(&"Mine/".to_string()));
        _ = std::fs::remove_dir_all(&v.root);
    }

    /// The index is kept between runs, and only changed notes are read again.
    #[test]
    fn the_index_is_incremental() {
        let v = vault("index", &[("Library/a2020/A.md", "Earlier studies reported that the proposed sampling procedure reduces the variance."), ("Library/b2021/B.md", "Something else entirely about rivers and valleys.")]);
        // a cache of its own, so the test never touches the user's
        let cache = v.root.join("library.idx");
        let index = |v: &Vault| {
            let mut ix = Index::load(&cache).unwrap_or_default();
            let now = v.library_files("Library/");
            let plan = lib::plan_sync(ix.signatures(), &now);
            let read = plan.read.clone();
            let add = plan.read.iter().map(|p| {
                let (_, t, s) = now.iter().find(|n| &n.0 == p).unwrap();
                lib::source(p, &v.read(p).unwrap(), lib::signature(*t, *s, &v.read(p).unwrap()))
            }).collect();
            ix.update(&plan.remove, add);
            ix.save(&cache).unwrap();
            (ix, read, plan.remove)
        };
        let (ix, read, _) = index(&v);
        assert_eq!((ix.stats().sources, read.len()), (2, 2));
        let (_, read, _) = index(&v);
        assert!(read.is_empty(), "nothing changed: nothing read");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(v.root.join("Library/b2021/B.md"), "Changed text about mountains, valleys and the weather of spring.").unwrap();
        std::fs::remove_file(v.root.join("Library/a2020/A.md")).unwrap();
        let (ix, read, removed) = index(&v);
        assert_eq!((read, removed), (vec!["Library/b2021/B.md".to_string()], vec!["Library/a2020/A.md".to_string()]));
        assert_eq!(ix.stats().sources, 1);
        _ = std::fs::remove_dir_all(&v.root);
    }

    /// The id the plugin computes from Obsidian's base path (backslashes, its own case) is this one.
    #[test]
    fn vault_ids_match_the_plugins() {
        if cfg!(windows) {
            let v = Vault { root: PathBuf::from("C:\\Users\\Me\\Vault"), config: ".obsidian".into() };
            assert_eq!(v.id(), library::vault_id("c:\\users\\me\\vault\\", true));
        } else {
            let v = Vault { root: PathBuf::from("/home/me/Vault"), config: ".obsidian".into() };
            assert_eq!(v.id(), library::vault_id("/home/me/Vault/", false));
        }
    }
}
