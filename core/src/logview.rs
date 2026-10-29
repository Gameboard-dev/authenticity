//! The live log pane in the settings tab (section "Logs"): Authenticity's own log (what it told you and what it
//! started, kept in memory while Obsidian runs), the debug log (`logs/authenticity-debug.log`, while *Debug log* is
//! on) and SlopTotal's output log (`logs/sloptotal.log`), as they grow.
//!
//! The files are Authenticity's own, in its local data folder outside the vault; they are only read, from the end,
//! with Node's `fs` (desktop only). The pane follows new lines, filters, copies what it shows (it writes the clipboard
//! on a click and never reads it), wraps lines or scrolls sideways (kept per device in Obsidian's local storage), keeps
//! at most `CAP` lines, and starts again from the end when a file is rotated or truncated. Every line is redacted
//! (`redact`): the GPTZero key never shows, and a debug line's checked text shows only while *Keep the checked text in
//! the history* is on.
//!
//! What to keep and show are pure functions (tested); the rest is the pane.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::app::with;
use crate::js::{self, call, f0, f1, get, obj, set, str};

/// The most lines the pane keeps and shows.
pub const CAP: usize = 2000;
/// How much of a file's end is read when the pane opens or the file was rotated.
pub const START_BYTES: u64 = 256 * 1024;
/// The per-device key of the *Wrap lines* choice.
const WRAP_KEY: &str = "authenticity-log-wrap";

// ----- what to keep and show

/// Lines as they arrive in chunks: whole lines kept (at most `cap`, the oldest dropped), a line not yet ended held back.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Lines {
    pub lines: VecDeque<String>,
    partial: String,
}

impl Lines {
    /// Add a chunk of a file. `from_middle`: the chunk starts inside a line (read from the middle of a file), whose
    /// first, cut line is dropped.
    pub fn push(&mut self, chunk: &str, from_middle: bool, cap: usize) {
        let mut text = std::mem::take(&mut self.partial);
        text.push_str(chunk);
        let mut parts: Vec<&str> = text.split('\n').collect();
        let last = parts.pop().unwrap_or_default().to_string();
        if from_middle && !parts.is_empty() {
            parts.remove(0);
        }
        for p in parts {
            let p = p.trim_end_matches('\r');
            if !p.is_empty() {
                self.lines.push_back(p.to_string());
            }
        }
        while self.lines.len() > cap {
            self.lines.pop_front();
        }
        self.partial = last;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.partial.clear();
    }
}

/// The lines that match `filter` (case-insensitive; blank: all).
pub fn shown<'a>(lines: impl IntoIterator<Item = &'a String>, filter: &str) -> Vec<&'a String> {
    let f = filter.trim().to_lowercase();
    lines.into_iter().filter(|l| f.is_empty() || l.to_lowercase().contains(&f)).collect()
}

/// A line as the pane shows it: the GPTZero key (if any) replaced, and a debug line's checked text hidden unless
/// `keep_text`.
pub fn redact(line: &str, key: Option<&str>, keep_text: bool) -> String {
    let mut out = line.to_string();
    if let Some(k) = key.filter(|k| k.len() >= 4) {
        if out.contains(k) {
            out = out.replace(k, "[key]");
        }
    }
    if !keep_text && out.starts_with('{') {
        if let Ok(mut v) = serde_json::from_str::<Value>(&out) {
            if let Some(o) = v.as_object_mut() {
                if o.contains_key("text") {
                    o.insert("text".into(), Value::String("[text not kept]".into()));
                    out = v.to_string();
                }
            }
        }
    }
    out
}

/// Where to start reading a file of `size` bytes after `offset` (and the file's identity, `ino`) were last seen:
/// (start, from the middle of a line?). A file smaller than before, or another file under the same name, was rotated or
/// truncated: read its end again.
pub fn next_read(size: u64, offset: Option<u64>, same_file: bool) -> (u64, bool) {
    match offset {
        Some(o) if same_file && size >= o => (o, false),
        _ => {
            let start = size.saturating_sub(START_BYTES);
            (start, start > 0)
        }
    }
}

// ----- Authenticity's own log, in memory

thread_local! {
    static EVENTS: RefCell<Lines> = RefCell::new(Lines::default());
    static VERSION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Add a line to Authenticity's own log (what it told you, what it started). Never a key or checked text.
pub fn event(text: &str) {
    let t = js_sys::Date::new_0();
    let stamp = format!("{:02}:{:02}:{:02}", t.get_hours(), t.get_minutes(), t.get_seconds());
    let line = format!("{stamp} {}\n", text.replace('\n', " "));
    EVENTS.with(|e| e.borrow_mut().push(&line, false, CAP));
    VERSION.with(|v| v.set(v.get() + 1));
}

// ----- the pane

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Events,
    Debug,
    SlopTotal,
}

struct Pane {
    source: Source,
    lines: Lines,
    offset: Option<u64>,
    ino: f64,
    seen: u64,
    filter: String,
    follow: bool,
    key: Option<String>,
    keep_text: bool,
    box_: JsValue,
    note: JsValue,
}

fn file_of(source: Source) -> Option<String> {
    let dir = crate::journal::log_dir()?;
    let sep = if dir.contains('\\') { "\\" } else { "/" };
    match source {
        Source::Events => None,
        Source::Debug => Some(format!("{dir}{sep}authenticity-debug.log")),
        Source::SlopTotal => Some(format!("{dir}{sep}sloptotal.log")),
    }
}

fn node(name: &str) -> JsValue {
    with(|a| js::apply(&a.require, &JsValue::NULL, &[name.into()])).unwrap_or(JsValue::UNDEFINED)
}

fn fs_call(fs: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    get(fs, name).dyn_into::<js_sys::Function>()?.apply(fs, &args.iter().collect::<js_sys::Array>())
}

/// Read `path` from byte `start` to its end, read-only: (text, size, identity); None when it cannot be read.
fn read_from(path: &str, start: u64, size: u64) -> Option<String> {
    let fs = node("fs");
    let fd = fs_call(&fs, "openSync", &[path.into(), "r".into()]).ok()?;
    let len = size.saturating_sub(start).min(8 * 1024 * 1024) as f64;
    let buffer = call!(get(&js::global(), "Buffer"), alloc, len);
    let read = fs_call(&fs, "readSync", &[fd.clone(), buffer.clone(), 0.into(), len.into(), (start as f64).into()]).ok();
    let _ = fs_call(&fs, "closeSync", &[fd]);
    let n = read?.as_f64().unwrap_or(0.0);
    let used = call!(buffer, subarray, 0, n);
    Some(str(&call!(used, toString, "utf8")))
}

fn stat(path: &str) -> Option<(u64, f64)> {
    let s = fs_call(&node("fs"), "statSync", &[path.into()]).ok()?;
    Some((get(&s, "size").as_f64().unwrap_or(0.0) as u64, get(&s, "ino").as_f64().unwrap_or(0.0)))
}

impl Pane {
    /// Bring the lines up to date; true when something changed.
    fn update(&mut self) -> bool {
        match self.source {
            Source::Events => {
                let v = VERSION.with(|v| v.get());
                if v == self.seen {
                    return false;
                }
                self.seen = v;
                self.lines = EVENTS.with(|e| e.borrow().clone());
                true
            }
            source => {
                let Some(path) = file_of(source) else { return false };
                let Some((size, ino)) = stat(&path) else {
                    let had = !self.lines.lines.is_empty() || self.offset.is_some();
                    self.lines.clear();
                    self.offset = None;
                    return had;
                };
                let same = self.offset.is_some() && ino == self.ino;
                let (start, middle) = next_read(size, self.offset, same);
                if !same || self.offset.is_none() {
                    self.lines.clear();
                }
                self.ino = ino;
                if start == size {
                    self.offset = Some(size);
                    return !same;
                }
                let Some(text) = read_from(&path, start, size) else { return false };
                self.lines.push(&text, middle, CAP);
                self.offset = Some(size);
                true
            }
        }
    }

    fn render(&self) {
        call!(self.box_, empty);
        let lines = shown(self.lines.lines.iter(), &self.filter);
        if lines.is_empty() {
            let why = if self.lines.lines.is_empty() { "No log lines yet." } else { "No line matches the filter." };
            call!(self.box_, createDiv, obj! { cls: "auth-log-empty", text: why });
        }
        for l in lines {
            call!(self.box_, createDiv, obj! { cls: "auth-log-line", text: redact(l, self.key.as_deref(), self.keep_text) });
        }
        if self.follow {
            set(&self.box_, "scrollTop", get(&self.box_, "scrollHeight"));
        }
    }

    fn describe(&self) -> String {
        match (self.source, file_of(self.source)) {
            (Source::Events, _) => "Authenticity's log: what it told you and what it started, since Obsidian started (kept in memory, not in a file).".into(),
            (_, Some(p)) if js::desktop() => format!("Read from {p} (read-only; the file is not changed)."),
            _ => "Log files are read on desktop Obsidian only.".into(),
        }
    }
}

fn wrap_now(app: &JsValue) -> bool {
    call!(app, loadLocalStorage, WRAP_KEY).as_bool().unwrap_or(true)
}

/// The settings row of the "Logs" section: the pane under it.
pub fn row(s: &JsValue) {
    let Some((app, debug_on, keep_text)) = with(|a| (a.app.clone(), a.settings.debug_log, a.settings.keep_text)) else { return };
    call!(s, setName, "Log");
    call!(s, setDesc, "Authenticity's log, the debug log and SlopTotal's log, as they grow. The files are only read; keys never show, and checked text only when the history keeps it.");
    let setting_el = get(s, "settingEl");
    call!(get(&setting_el, "classList"), add, "auth-log-setting");
    let wrap = call!(setting_el, createDiv, obj! { cls: "auth-log-pane" });
    let bar = call!(wrap, createDiv, obj! { cls: "auth-log-bar" });

    let select = call!(bar, createEl, "select", obj! { cls: "dropdown" });
    call!(select, setAttribute, "aria-label", "Which log");
    let mut sources = vec![("events", "Authenticity")];
    if debug_on {
        sources.push(("debug", "Debug log"));
    }
    if js::desktop() {
        sources.push(("sloptotal", "SlopTotal"));
    }
    for (value, text) in &sources {
        call!(select, createEl, "option", obj! { value: *value, text: *text });
    }
    let filter = call!(bar, createEl, "input", obj! { cls: "auth-log-filter", placeholder: "Filter" });
    call!(filter, setAttribute, "type", "search");
    call!(filter, setAttribute, "aria-label", "Show only lines containing");
    let checkbox = |parent: &JsValue| {
        let c = call!(parent, createEl, "input");
        call!(c, setAttribute, "type", "checkbox");
        c
    };
    let follow_label = call!(bar, createEl, "label", obj! { cls: "auth-log-check" });
    let follow = checkbox(&follow_label);
    set(&follow, "checked", true);
    call!(follow_label, appendText, " Follow new lines");
    let wrap_label = call!(bar, createEl, "label", obj! { cls: "auth-log-check" });
    let wrap_box = checkbox(&wrap_label);
    set(&wrap_box, "checked", wrap_now(&app));
    call!(wrap_label, appendText, " Wrap lines");
    let copy = call!(bar, createEl, "button", obj! { cls: "clickable-icon" });
    call!(copy, setAttribute, "aria-label", "Copy the lines shown");
    with(|a| call!(a.obs, setIcon, copy.clone(), "copy"));

    let box_ = call!(wrap, createDiv, obj! { cls: "auth-log" });
    call!(box_, setAttribute, "tabindex", "0");
    call!(box_, setAttribute, "role", "log");
    let set_wrap = {
        let box_ = box_.clone();
        move |on: bool| {
            let cl = get(&box_, "classList");
            call!(cl, toggle, "is-wrap", on);
            call!(cl, toggle, "is-nowrap", !on);
        }
    };
    set_wrap(wrap_now(&app));
    let note = call!(wrap, createDiv, obj! { cls: "auth-log-source" });

    let pane = Rc::new(RefCell::new(Pane { source: Source::Events, lines: Lines::default(), offset: None, ino: 0.0, seen: u64::MAX, filter: String::new(), follow: true, key: None, keep_text, box_: box_.clone(), note: note.clone() }));
    let refresh = {
        let pane = pane.clone();
        move |all: bool| {
            let mut p = pane.borrow_mut();
            if p.update() || all {
                p.render();
            }
        }
    };
    {
        let p = pane.borrow();
        call!(p.note, setText, p.describe());
    }
    refresh(true);

    // the GPTZero key, to take out of any line that held it
    {
        let pane = pane.clone();
        js::spawn(async move {
            let key = crate::app::gptzero_key().await;
            let mut p = pane.borrow_mut();
            p.key = key;
            p.render();
        });
    }
    {
        let (pane, r) = (pane.clone(), refresh.clone());
        call!(select, addEventListener, "change", f1(move |e| {
            {
                let mut p = pane.borrow_mut();
                p.source = match str(&get(&get(&e, "target"), "value")).as_str() {
                    "debug" => Source::Debug,
                    "sloptotal" => Source::SlopTotal,
                    _ => Source::Events,
                };
                p.lines.clear();
                p.offset = None;
                p.seen = u64::MAX;
                call!(p.note, setText, p.describe());
            }
            r(true);
            JsValue::UNDEFINED
        }));
    }
    {
        let pane = pane.clone();
        call!(filter, addEventListener, "input", f1(move |e| {
            let mut p = pane.borrow_mut();
            p.filter = str(&get(&get(&e, "target"), "value"));
            p.render();
            JsValue::UNDEFINED
        }));
    }
    {
        let pane = pane.clone();
        call!(follow, addEventListener, "change", f1(move |e| {
            let mut p = pane.borrow_mut();
            p.follow = get(&get(&e, "target"), "checked").as_bool().unwrap_or(true);
            p.render();
            JsValue::UNDEFINED
        }));
    }
    {
        let app = app.clone();
        call!(wrap_box, addEventListener, "change", f1(move |e| {
            let on = get(&get(&e, "target"), "checked").as_bool().unwrap_or(true);
            call!(app, saveLocalStorage, WRAP_KEY, on);
            set_wrap(on);
            JsValue::UNDEFINED
        }));
    }
    {
        let pane = pane.clone();
        call!(copy, addEventListener, "click", f1(move |_| {
            let text = {
                let p = pane.borrow();
                shown(p.lines.lines.iter(), &p.filter).into_iter().map(|l| redact(l, p.key.as_deref(), p.keep_text)).collect::<Vec<_>>().join("\n")
            };
            // the clipboard is written on this click only, and never read
            let clip = get(&get(&js::global(), "navigator"), "clipboard");
            let done = call!(clip, writeText, text);
            js::spawn(async move {
                let msg = match js::wait(done).await {
                    Ok(_) => "log copied. Check it for paths before sharing it.".to_string(),
                    Err(e) => format!("could not copy the log ({}).", str(&e)),
                };
                with(|a| a.notice(&msg));
            });
            JsValue::UNDEFINED
        }));
    }
    // every second while the pane is in the page; the timer stops itself once the settings are closed
    let timer = Rc::new(std::cell::Cell::new(0.0));
    let tick = {
        let (timer, box_, r) = (timer.clone(), box_.clone(), refresh.clone());
        f0(move || {
            if get(&box_, "isConnected").as_bool() != Some(true) {
                let id = timer.get();
                call!(js::global(), clearInterval, id);
                return;
            }
            r(false);
        })
    };
    let id = call!(js::global(), setInterval, tick, 1000).as_f64().unwrap_or(0.0);
    timer.set(id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_arrive_in_chunks_and_keep_the_cap() {
        let mut l = Lines::default();
        l.push("one\ntw", false, 3);
        assert_eq!(l.lines, ["one"]);
        l.push("o\r\nthree\nfour\nfive\n", false, 3);
        assert_eq!(l.lines, ["three", "four", "five"], "the oldest dropped at the cap; CR LF read as one line end");
        let mut m = Lines::default();
        m.push("t of a line\nwhole\n", true, 10);
        assert_eq!(m.lines, ["whole"], "read from the middle: the cut first line is dropped");
    }

    #[test]
    fn the_filter_ignores_case() {
        let lines: Vec<String> = vec!["Started SlopTotal".into(), "GPU free".into(), "stopped sloptotal".into()];
        assert_eq!(shown(lines.iter(), "sloptotal").len(), 2);
        assert_eq!(shown(lines.iter(), "  ").len(), 3);
        assert!(shown(lines.iter(), "nothing").is_empty());
    }

    #[test]
    fn keys_and_unkept_text_are_never_shown() {
        let key = "gz-secret-0123456789";
        assert_eq!(redact(&format!("error: bad key {key}"), Some(key), true), "error: bad key [key]");
        let line = r#"{"t":1,"event":"ai-request","bytes":12,"text":"My checked paragraph."}"#;
        let hidden = redact(line, None, false);
        assert!(!hidden.contains("My checked paragraph") && hidden.contains("[text not kept]") && hidden.contains("\"bytes\":12"), "{hidden}");
        assert_eq!(redact(line, None, true), line, "kept when the history keeps the text");
        assert_eq!(redact("plain line", Some("ab"), false), "plain line", "a key too short to be one is not looked for");
    }

    #[test]
    fn a_rotated_or_truncated_file_is_read_again_from_its_end() {
        assert_eq!(next_read(1000, None, false), (0, false), "first read of a small file: all of it");
        assert_eq!(next_read(START_BYTES + 10, None, false), (10, true), "a large one: its end, from inside a line");
        assert_eq!(next_read(1500, Some(1000), true), (1000, false), "grown: the new bytes");
        assert_eq!(next_read(200, Some(1000), true), (0, false), "smaller: truncated");
        assert_eq!(next_read(5000, Some(1000), false), (0, false), "another file under the name: rotated");
    }
}
