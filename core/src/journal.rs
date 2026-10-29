//! Authenticity's own records on this computer, outside the vault, in its local data folder
//! (`<local data>/authenticity/`, `%LOCALAPPDATA%\authenticity` on Windows):
//!
//! - **History** (`history/<vault id>.jsonl`): one JSON line per AI check, so a past run can be compared with the
//!   current one: when, the note, the device, the helper's version, and for each paragraph (or group, or sentence) its
//!   hash, its first words, its score, every quick detector's result and the detectors that failed; for a full
//!   analysis, every detector's result and weight. The checked text itself only when the setting *Keep the checked text
//!   in the history* is on (off by default). Runs older than the retention (*Keep the history for*, 30 days by
//!   default) are dropped, and the file is kept under `HISTORY_CAP` bytes (the oldest runs go first).
//! - **Debug log** (`logs/authenticity-debug.log`), only while the setting *Debug log* is on (off by default):
//!   requests (sizes, hashes, grouping, concurrency, device), answers (status, time, per-detector results) and errors
//!   with their context. Never a key; the text only when *Keep the checked text* is also on. The file is moved to
//!   `.1` when it passes `LOG_CAP` bytes (one old file kept).
//!
//! What to write and what to keep are pure functions (tested); the files are written with Node's `fs` (desktop only).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::app::App;
use crate::js::{self, get, set, str};

/// The history file's and the debug log's largest size, in bytes.
pub const HISTORY_CAP: usize = 5 * 1024 * 1024;
pub const LOG_CAP: usize = 5 * 1024 * 1024;
/// The words of a paragraph kept when its text is not.
pub const FIRST_WORDS: usize = 8;

/// One scored stretch of a run: a paragraph, a group of short ones, or a sentence with its neighbours.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    /// The hash of the text scored (ai::key), its first words, and the text itself when the setting keeps it.
    pub key: String,
    pub first: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub score: f64,
    /// Each detector's own result (0 to 100) and, for a full analysis, its weight.
    pub detectors: Vec<(String, f64, Option<f64>)>,
    pub failed: Vec<String>,
    /// For a group: what the pane says of it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub together: String,
}

/// One check, as kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    /// Seconds since 1970.
    pub t: f64,
    pub note: String,
    /// "paragraphs", "sentences" or "analysis".
    pub kind: String,
    /// The engine that scored it: "gptzero", else SlopTotal (blank).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub engine: String,
    pub device: String,
    pub helper: String,
    pub items: Vec<Item>,
}

/// The first `FIRST_WORDS` words of a text.
pub fn first_words(text: &str) -> String {
    let w: Vec<&str> = text.split_whitespace().collect();
    let mut s = w.iter().take(FIRST_WORDS).copied().collect::<Vec<_>>().join(" ");
    if w.len() > FIRST_WORDS {
        s.push('\u{2026}');
    }
    s
}

/// An item for `text` as the settings allow: its text kept only with `keep_text`.
pub fn item(text: &str, score: f64, keep_text: bool) -> Item {
    Item { key: crate::ai::key(text), first: first_words(text), text: keep_text.then(|| text.to_string()), score, ..Item::default() }
}

/// The history file's lines to keep, oldest first, after adding `new`: those within `days` of `now` (seconds), and no
/// more than fit in `cap` bytes, newest kept. A line that is not a run is dropped.
pub fn prune(lines: &[&str], new: Option<&str>, now: f64, days: f64, cap: usize) -> Vec<String> {
    let oldest = now - days.max(0.0) * 86_400.0;
    let mut keep: Vec<String> = lines
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok().filter(|v| v["t"].as_f64().is_some_and(|t| t >= oldest)).map(|_| l.to_string()))
        .chain(new.map(str::to_string))
        .collect();
    let mut size: usize = keep.iter().map(|l| l.len() + 1).sum();
    while size > cap && !keep.is_empty() {
        size -= keep[0].len() + 1;
        keep.remove(0);
    }
    keep
}

/// A debug log line, or None when the debug log is off (nothing is written then). The text is never in it unless
/// `keep_text`; `fields` must hold no key (the callers pass sizes, hashes, statuses and results only).
pub fn debug_line(on: bool, now_ms: f64, event: &str, fields: Value, keep_text: bool, text: Option<&str>) -> Option<String> {
    if !on {
        return None;
    }
    let mut v = serde_json::json!({ "t": now_ms, "event": event });
    if let (Value::Object(o), Value::Object(f)) = (&mut v, fields) {
        o.extend(f);
    }
    if let (true, Some(t)) = (keep_text, text) {
        v["text"] = Value::String(t.to_string());
    }
    Some(v.to_string())
}

// ----- the files

/// `<local data>/authenticity` and the path separator, or None (no local data folder).
fn base() -> Option<(String, &'static str)> {
    let process = get(&js::global(), "process");
    let windows = str(&get(&process, "platform")) == "win32";
    let env = get(&process, "env");
    let var = |k: &str| Some(str(&get(&env, k))).filter(|v| !v.is_empty());
    let root = var("LOCALAPPDATA").or_else(|| var("XDG_DATA_HOME")).or_else(|| var("HOME").map(|h| format!("{h}/.local/share")))?;
    let sep = if windows { "\\" } else { "/" };
    Some((format!("{root}{sep}authenticity"), sep))
}

/// The folder of the logs (the settings' log pane reads them).
pub fn log_dir() -> Option<String> {
    base().map(|(b, sep)| format!("{b}{sep}logs"))
}

fn history_file(a: &App) -> Option<String> {
    let (b, sep) = base()?;
    let vault = call_base_path(a)?;
    let windows = sep == "\\";
    Some(format!("{b}{sep}history{sep}{}.jsonl", crate::library::vault_id(&vault, windows)))
}

fn call_base_path(a: &App) -> Option<String> {
    let adapter = get(&get(&a.app, "vault"), "adapter");
    let f = get(&adapter, "getBasePath");
    f.is_function().then(|| f.dyn_ref::<js_sys::Function>().and_then(|f| f.call0(&adapter).ok())).flatten().filter(|b| b.is_string()).map(|b| str(&b))
}

fn fs(a: &App) -> JsValue {
    js::apply(&a.require, &JsValue::NULL, &["fs".into()])
}

fn fs_call(fs: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    get(fs, name).dyn_into::<js_sys::Function>()?.apply(fs, &args.iter().collect::<js_sys::Array>())
}

fn mkdir(fs: &JsValue, dir: &str) {
    let o = js_sys::Object::new();
    set(&o, "recursive", true);
    let _ = fs_call(fs, "mkdirSync", &[dir.into(), o.into()]);
}

fn read(fs: &JsValue, path: &str) -> Option<String> {
    fs_call(fs, "readFileSync", &[path.into(), "utf8".into()]).ok().filter(|v| v.is_string()).map(|v| str(&v))
}

/// Add a run to the history (and drop what the retention and the cap say), inside `with`.
pub fn record(a: &App, run: &Run) {
    if !js::desktop() {
        return;
    }
    let Some(path) = history_file(a) else { return };
    let fs = fs(a);
    let Some(dir) = path.rsplit_once(['\\', '/']).map(|(d, _)| d.to_string()) else { return };
    mkdir(&fs, &dir);
    let old = read(&fs, &path).unwrap_or_default();
    let lines: Vec<&str> = old.lines().filter(|l| !l.trim().is_empty()).collect();
    let new = serde_json::to_string(run).unwrap_or_default();
    let keep = prune(&lines, Some(&new), js_sys::Date::now() / 1000.0, a.settings.history_days, HISTORY_CAP);
    let tmp = format!("{path}.tmp");
    let mut text = keep.join("\n");
    text.push('\n');
    if fs_call(&fs, "writeFileSync", &[tmp.clone().into(), text.into()]).is_ok() {
        let _ = fs_call(&fs, "renameSync", &[tmp.into(), path.into()]);
    }
}

/// The runs kept for `note`, newest first (the History view).
pub fn runs_of(a: &App, note: &str) -> Vec<Run> {
    if !js::desktop() {
        return Vec::new();
    }
    let Some(path) = history_file(a) else { return Vec::new() };
    let text = read(&fs(a), &path).unwrap_or_default();
    let mut out: Vec<Run> = text.lines().filter_map(|l| serde_json::from_str::<Run>(l).ok()).filter(|r| r.note == note).collect();
    out.reverse();
    out
}

/// Write a debug line when the debug log is on (`fields`: sizes, hashes, results; never a key), inside `with`.
pub fn debug(a: &App, event: &str, fields: Value, text: Option<&str>) {
    let Some(line) = debug_line(a.settings.debug_log, js_sys::Date::now(), event, fields, a.settings.keep_text, text) else { return };
    if !js::desktop() {
        return;
    }
    let Some(dir) = log_dir() else { return };
    let sep = if dir.contains('\\') { "\\" } else { "/" };
    let path = format!("{dir}{sep}authenticity-debug.log");
    let fs = fs(a);
    mkdir(&fs, &dir);
    let size = fs_call(&fs, "statSync", &[path.clone().into()]).ok().map(|s| get(&s, "size").as_f64().unwrap_or(0.0) as usize).unwrap_or(0);
    if size > LOG_CAP {
        let _ = fs_call(&fs, "renameSync", &[path.clone().into(), format!("{path}.1").into()]);
    }
    let _ = fs_call(&fs, "appendFileSync", &[path.into(), format!("{line}\n").into()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_keeps_its_retention_and_its_cap() {
        let run = |t: f64| serde_json::to_string(&Run { t, note: "a.md".into(), ..Run::default() }).unwrap();
        let (old, mid, now) = (run(0.0), run(20.0 * 86_400.0), 40.0 * 86_400.0);
        let lines = [old.as_str(), mid.as_str(), "not a run"];
        let new = run(now);
        // 30 days: the 40-day-old run goes, the 20-day-old stays, the new one is added, the bad line dropped
        assert_eq!(prune(&lines, Some(&new), now, 30.0, 1 << 20), [mid.clone(), new.clone()]);
        // the cap: the oldest go first until it fits
        assert_eq!(prune(&lines, Some(&new), now, 365.0, new.len() + mid.len() + 2), [mid.clone(), new.clone()]);
        assert_eq!(prune(&lines, Some(&new), now, 365.0, new.len() + 1), [new.clone()]);
    }

    #[test]
    fn the_debug_log_writes_nothing_when_off_and_no_text_unless_kept() {
        let f = || serde_json::json!({ "bytes": 120, "key": "0123abcd" });
        assert_eq!(debug_line(false, 1.0, "request", f(), true, Some("my text")), None, "off: nothing written");
        let line = debug_line(true, 1.0, "request", f(), false, Some("my text")).unwrap();
        assert!(line.contains("\"event\":\"request\"") && line.contains("\"bytes\":120") && !line.contains("my text"), "{line}");
        assert!(debug_line(true, 1.0, "request", f(), true, Some("my text")).unwrap().contains("my text"));
    }

    #[test]
    fn items_keep_the_text_only_when_asked() {
        let t = "one two three four five six seven eight nine ten";
        let i = item(t, 21.0, false);
        assert_eq!((i.first.as_str(), i.text.as_deref()), ("one two three four five six seven eight\u{2026}", None));
        assert_eq!(item(t, 21.0, true).text.as_deref(), Some(t));
    }
}
