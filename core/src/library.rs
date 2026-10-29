//! Which notes are the library a check compares with: the notes (`.md`) in the library folder (Authenticity's setting
//! *Library folder*; there is no default: the plagiarism check asks for it first), never the user's own writing. Notes
//! under the excluded folders (default `Drafts/` and `LaTeX/`, plus the user's list) and the note being checked are
//! never sources.
//!
//! Every folder here is a vault path, checked before use (it comes from settings synchronised with the vault):
//! relative, with `/`, no `..`, no drive or leading slash.

/// What the plagiarism check says when the library folder is not set (or not a plain vault folder).
pub const SET_LIBRARY: &str = "set the Library folder first (Settings \u{2192} Authenticity \u{2192} Plagiarism): the vault folder of the notes your writing is compared with, such as Sources/ or Library/";
/// Folders whose notes are the user's own writing, never sources.
pub const DEFAULT_EXCLUDE: &[&str] = &["Drafts/", "LaTeX/"];

/// A vault folder as `a/b/`, or None if it is not a plain relative path. An empty string is the vault's root ("").
pub fn folder(s: &str) -> Option<String> {
    let t = s.trim().replace('\\', "/");
    let t = t.trim_matches('/');
    if t.is_empty() {
        return Some(String::new());
    }
    let bad = t.contains(':') || t.split('/').any(|p| p.is_empty() || p == "." || p == ".." || p.starts_with('.')) || t.len() > 400 || t.chars().any(char::is_control);
    (!bad).then(|| format!("{t}/"))
}

/// The library and what is left out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub library: String,
    pub exclude: Vec<String>,
}

impl Scope {
    /// From the library folder and the excluded folders (comma or line separated) as set; a folder that is not a plain
    /// vault path is left out (an unusable or empty library folder leaves the library empty: `is_set` is false).
    pub fn new(library: &str, exclude: &str) -> Scope {
        let library = folder(library).unwrap_or_default();
        let mut ex: Vec<String> = DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect();
        ex.extend(exclude.split([',', '\n']).filter_map(folder).filter(|f| !f.is_empty()));
        ex.sort();
        ex.dedup();
        Scope { library, exclude: ex }
    }

    /// Whether a library folder is set (a vault folder other than the vault's root).
    pub fn is_set(&self) -> bool {
        !self.library.is_empty()
    }

    /// Whether a note (vault path) is in the library and not left out.
    pub fn holds(&self, path: &str) -> bool {
        self.is_set() && path.ends_with(".md") && path.starts_with(&self.library) && !self.exclude.iter().any(|e| path.starts_with(e.as_str()) || path.contains(&format!("/{e}")))
    }
}

/// A vault's id, for where its library index is kept on this computer: FNV-1a of its folder on disk (`/`
/// separators, no trailing one, lower case on Windows). The plugin and the native programs both use this.
pub fn vault_id(base: &str, windows: bool) -> String {
    let mut p = base.replace('\\', "/");
    while p.ends_with('/') && p.len() > 1 {
        p.pop();
    }
    if windows {
        p = p.to_lowercase();
    }
    format!("{:016x}", authenticity_similar::library::fnv(p.as_bytes()))
}

/// A library note as a reader knows it: its source folder (the citation key) and its name, `smith2020 › Results`.
pub fn title(path: &str, library: &str) -> String {
    let rest = path.strip_prefix(library).unwrap_or(path).trim_end_matches(".md");
    match rest.split_once('/') {
        Some((key, name)) => format!("{key} \u{203a} {}", name.rsplit('/').next().unwrap_or(name)),
        None => rest.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_plain_vault_paths() {
        assert_eq!(folder("Sources\\Library"), Some("Sources/Library/".into()));
        assert_eq!(folder("/Papers/"), Some("Papers/".into()));
        assert_eq!(folder(""), Some(String::new()));
        for bad in ["../x", "a/../b", "C:/x", ".obsidian", "a//b", "a/.hidden/b"] {
            assert_eq!(folder(bad), None, "{bad}");
        }
    }

    #[test]
    fn drafts_and_own_writing_are_never_sources() {
        let s = Scope::new("Sources/Library/", "Notes/Mine, Sources/Library/own2024/");
        assert!(s.holds("Sources/Library/smith2020/A Title.md"));
        assert!(!s.holds("Sources/Library/smith2020/attachments/fig.png"));
        assert!(!s.holds("Drafts/Chapter.md") && !s.holds("LaTeX/Chapter 1.md"));
        assert!(!s.holds("Sources/Library/own2024/My paper.md"), "the user's list");
        assert!(!s.holds("Elsewhere/Note.md"));
        // no library folder, or one that is not a plain vault path: nothing is the library, and the check asks for it
        for unset in ["", "/", "../../etc"] {
            let s = Scope::new(unset, "");
            assert!(!s.is_set() && !s.holds("Anything/A Title.md"), "{unset:?}");
        }
    }

    #[test]
    fn titles() {
        assert_eq!(title("Sources/Library/smith2020/05 Results.md", "Sources/Library/"), "smith2020 \u{203a} 05 Results");
    }
}
