//! Drawing the results in notes, through the workspace's shared crate `view-marks` (the same system Augmented View
//! draws its colours with): the editor (Live Preview and Source mode) gets its marks over the range it renders, as the
//! text comes into view; Reading view gets segments of the blocks near the screen; and "Copy with Authenticity marks"
//! copies a selection with its marks as Markdown markers. There is no pop-up on hover: what a mark means is in the panes, reached from the editor's menu (Show in AI check, Show in similarity
//! check).
//!
//! The marks come from the stored results (the checks themselves run in app.rs): checked paragraphs tinted by their
//! SlopTotal score (red for AI, green for a person, five strengths; orange between), plagiarism matches underlined (solid for copied wording, dashed for close paraphrase; the field's
//! fixed phrasing is not marked, only listed in the panel), and characters (as the note is drawn, if set, or where Reveal AI characters was used). A note with
//! none of these is not marked at all (`Host::scope` is None) and nothing of it is handled.
//! Nothing is ever rewritten.

use std::collections::HashMap;

use view_marks::{copy, editor, highlights, locate, reading, style, Event, Host, Mark, Marks, Seg};
use wasm_bindgen::prelude::*;

use crate::app::{with, App};
use crate::js::{self, call, get, obj, str};
use crate::{ai, align, chars, prose};

pub const BASE: &str = "auth";
pub const ATTR: &str = "data-auth";
const SCOPE: &str = "on";
const STYLESHEET: &str = "authenticity-colours";

/// The mark classes, by index (view-marks' `Config::classes`).
pub const CLASSES: &[&str] = &[
    "auth-ai", "auth-human", "auth-mixed", "auth-sent", "auth-plag-match", "auth-plag-near", "auth-plag-para", "auth-plag-common",
    "auth-char-hidden", "auth-char-space", "auth-char-dash", "auth-char-punct", "auth-char-lookalike", "auth-char-typo",
];
const AI: u8 = 0;
const HUMAN: u8 = 1;
const MIXED: u8 = 2;
// 3, "auth-sent", marked the sentences the hover found (the hover is gone; the class keeps its place)
const PLAG: u8 = 4;
const CHAR0: u8 = 8;

pub fn config() -> view_marks::Config {
    view_marks::Config { base: BASE.into(), attr: ATTR.into(), classes: CLASSES.iter().map(|s| s.to_string()).collect() }
}

fn char_class(k: chars::Kind) -> u8 {
    CHAR0
        + match k {
            chars::Kind::Hidden => 0,
            chars::Kind::Space => 1,
            chars::Kind::Dash => 2,
            chars::Kind::Punctuation => 3,
            chars::Kind::LookAlike => 4,
            chars::Kind::Typographic => 5,
        }
}

/// One mark of a note: its range (UTF-16), class and strength, its lines, and the text Reading view finds it by.
#[derive(Clone, Debug, PartialEq)]
pub struct RMark {
    pub mark: Mark,
    pub lines: (u32, u32),
    pub find: String,
    pub every: bool,
}

/// A Reading-view block: its note, its section's lines, its text.
type Block = (String, Option<(u32, u32)>, String);

/// What the main thread keeps for drawing.
#[derive(Default)]
pub struct State {
    /// Each note's text as last sent (an editor's, or the file's for Reading view), with its version.
    docs: HashMap<String, (u32, String)>,
    /// Editors (by view-marks' id): their note and rendered range.
    views: HashMap<u32, (String, (u32, u32))>,
    /// Reading-view blocks: note, section lines, text.
    blocks: HashMap<u32, Block>,
    version: u32,
    /// The marks of each note for its text version, worked out once.
    made: HashMap<String, (u32, u64, Vec<RMark>)>,
    pub results_version: u64,
}

/// The 0-based line of each UTF-16 place in `text` (a function over sorted line starts).
fn line_starts(text: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    let mut pos = 0u32;
    for c in text.chars() {
        pos += c.len_utf16() as u32;
        if c == '\n' {
            starts.push(pos);
        }
    }
    starts
}

/// The marks of a note's text, from the stored results (also notes the characters for the AI Detector pane).
pub fn note_marks(a: &mut App, path: &str, text: &str) -> Vec<RMark> {
    let starts = line_starts(text);
    let line = |o: u32| (starts.partition_point(|&s| s <= o) as u32).saturating_sub(1);
    let mut out = Vec::new();
    let push = |m: Mark, find: String, every: bool, out: &mut Vec<RMark>| {
        if m.from < m.to {
            out.push(RMark { mark: m, lines: (line(m.from), line(m.to.saturating_sub(1))), find, every });
        }
    };
    let p = prose::extract(text);
    if a.ai.notes.contains_key(path) {
        for (para, s) in a.ai.scored_in(path, ai::paragraphs(&p)) {
            let s = &s;
            let (class, strength) = match ai::tint(s.score.score) {
                ai::Tint::Ai(l) => (AI, l),
                ai::Tint::Human(l) => (HUMAN, l),
                ai::Tint::Mixed => (MIXED, 0),
            };
            push(Mark { from: para.from, to: para.to, class, strength }, para.text.clone(), false, &mut out);
        }
    }
    if let Some(ms) = a.matches.get(path) {
        // the field's fixed phrasing is not plagiarism: listed apart in the panel, never marked in the note
        for m in ms.iter().filter(|m| m.kind != "common") {
            let Some((x, y)) = locate(text, m.from, m.to, &m.text) else { continue };
            let class = match m.kind.as_str() {
                "verbatim" => PLAG,
                "near_verbatim" => PLAG + 1,
                "paraphrase" => PLAG + 2,
                _ => PLAG + 3,
            };
            push(Mark::new(x, y, class), prose::extract(&m.text).text().replace('\n', " "), false, &mut out);
        }
    }
    let typo = a.settings.show_typographic;
    let hits: Vec<chars::Hit> = match a.reveal.get(path) {
        Some(Some((x, y))) => chars::scan(&crate::app::slice16(text, *x, *y).0, typo).into_iter().map(|mut h| {
            h.from += x;
            h.to += x;
            h
        }).collect(),
        Some(None) => chars::scan(text, typo),
        None if a.settings.show_hidden && a.in_scope(path) => chars::scan(text, typo),
        None => Vec::new(),
    };
    for h in &hits {
        push(Mark::new(h.from, h.to, char_class(h.kind)), char::from_u32(h.code).map(String::from).unwrap_or_default(), true, &mut out);
    }
    a.hits.insert(path.to_string(), hits);
    // sorted by start, the wider first at one start (a paragraph holds its sentences)
    out.sort_by_key(|m| (m.mark.from, std::cmp::Reverse(m.mark.to), m.mark.class));
    out
}

/// Where a match is in the note now: where it was, if its text is still there, else its text found again.
pub fn locate(text: &str, from: u32, to: u32, words: &str) -> Option<(u32, u32)> {
    let (at, _) = crate::app::slice16(text, from, to);
    if at == words {
        return Some((from, to));
    }
    align::find_utf16(text, words)
}

impl App {
    /// The note's marks for its text now (worked out once per text and results).
    fn marks_of(&mut self, path: &str) -> Vec<RMark> {
        let Some((v, text)) = self.render.docs.get(path).cloned() else { return Vec::new() };
        let rv = self.render.results_version;
        if let Some((mv, mr, m)) = self.render.made.get(path) {
            if *mv == v && *mr == rv {
                return m.clone();
            }
        }
        let m = note_marks(self, path, &text);
        self.render.made.insert(path.to_string(), (v, rv, m.clone()));
        m
    }

    /// Whether a note is marked: it has results, revealed characters, or its characters are marked as it is drawn.
    pub fn scope_of(&self, path: &str) -> Option<String> {
        let marked = self.ai.notes.contains_key(path) || self.matches.contains_key(path) || self.reveal.contains_key(path) || (self.settings.show_hidden && self.in_scope(path));
        marked.then(|| SCOPE.to_string())
    }

    /// An editor's marks over its rendered range, and the version they are for.
    fn editor_marks(&mut self, id: u32) -> Option<(u32, Vec<Mark>)> {
        let (path, (a, b)) = self.render.views.get(&id).cloned()?;
        let v = self.render.docs.get(&path)?.0;
        let marks = self.marks_of(&path).into_iter().filter(|m| m.mark.to > a && m.mark.from < b).map(|m| m.mark).collect();
        Some((v, marks))
    }

    /// A Reading-view block's segments: the note's marks over its lines, found in its text.
    fn block_segs(&mut self, id: u32) -> Option<Vec<Seg>> {
        let (path, lines, text) = self.render.blocks.get(&id).cloned()?;
        let marks = self.marks_of(&path);
        let norm = locate::Norm::new(&text);
        let mut ranges = Vec::new();
        for m in marks.iter().filter(|m| lines.is_none_or(|(a, b)| m.lines.0 <= b && m.lines.1 >= a)) {
            if m.find.trim().is_empty() {
                continue;
            }
            for (x, y) in norm.find_all(&m.find, m.every) {
                ranges.push((x, y, m.mark.class, m.mark.strength));
            }
        }
        Some(locate::segments_with_strength(&ranges))
    }
}

/// Draw again, after this call returns: the editors and Reading-view blocks of `path` (or all), with their notes'
/// scopes looked at again (a note may have become marked, or stopped being).
pub fn refresh(path: Option<String>) {
    js::timeout(0.0, move || {
        let todo = with(|a| {
            a.render.results_version += 1;
            for id in editor::editor_ids(a) {
                editor::tag_editor(a, id);
            }
            let attr = ATTR.to_string();
            let app: &App = a;
            app.marks.reading.tag(&attr, |p| app.scope_of(p));
            let eds: Vec<u32> = a.render.views.iter().filter(|(_, v)| path.as_deref().is_none_or(|p| p == v.0)).map(|(k, _)| *k).collect();
            let blocks: Vec<u32> = a.render.blocks.iter().filter(|(_, b)| path.as_deref().is_none_or(|p| p == b.0)).map(|(k, _)| *k).collect();
            (eds, blocks)
        });
        let Some((eds, blocks)) = todo else { return };
        for id in eds {
            draw_editor(id);
        }
        for id in blocks {
            with(|a| {
                if let Some(s) = a.block_segs(id) {
                    reading::queue(a, id, s);
                }
            });
        }
    });
}

fn draw_editor(id: u32) {
    if let Some(Some((v, marks))) = with(|a| a.editor_marks(id)) {
        editor::apply::<App>(id, v, &marks);
    }
}

impl Host for App {
    fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
        with(f)
    }

    fn marks(&mut self) -> &mut Marks {
        &mut self.marks
    }

    fn scope(&self, path: &str) -> Option<String> {
        self.scope_of(path)
    }

    fn send_doc(&mut self, path: &str, text: String) -> u32 {
        self.render.version += 1;
        let v = self.render.version;
        self.render.docs.insert(path.to_string(), (v, text));
        let ids: Vec<u32> = self.render.views.iter().filter(|(_, x)| x.0 == path).map(|(k, _)| *k).collect();
        js::timeout(0.0, move || ids.into_iter().for_each(draw_editor));
        crate::panel::render(self);
        v
    }

    fn ensure_doc(&mut self, path: &str) {
        if self.render.docs.contains_key(path) {
            return;
        }
        let (vault, path) = (get(&self.app, "vault"), path.to_string());
        js::spawn(async move {
            let file = call!(vault, getAbstractFileByPath, path.clone());
            if file.is_null() || file.is_undefined() {
                return;
            }
            let text = str(&js::wait(call!(vault, cachedRead, file)).await.unwrap_or_default());
            with(|a| {
                if !a.render.docs.contains_key(&path) {
                    a.render.version += 1;
                    let v = a.render.version;
                    a.render.docs.insert(path.clone(), (v, text));
                }
            });
            refresh(Some(path));
        });
    }

    fn send(&mut self, e: Event) {
        match e {
            Event::View { id, path, editor: true, render, .. } => {
                self.render.views.insert(id, (path, render));
                js::timeout(0.0, move || draw_editor(id));
            }
            Event::View { .. } => {}
            Event::DropView(id) => _ = self.render.views.remove(&id),
            Event::Block { id, path, lines, text, .. } => {
                self.render.blocks.insert(id, (path, lines, text));
                js::timeout(0.0, move || {
                    with(|a| {
                        if let Some(s) = a.block_segs(id) {
                            reading::queue(a, id, s);
                        }
                    });
                });
            }
            Event::DropBlock(id) => _ = self.render.blocks.remove(&id),
        }
    }
}

/// A note file changed: its text for Reading view is read again (an open editor sends its own).
pub fn file_changed(path: &str) {
    let shown = with(|a| a.marks.reading.shows(path) && !a.render.views.values().any(|v| v.0 == path)).unwrap_or(false);
    if shown {
        with(|a| a.render.docs.remove(path));
        with(|a| a.ensure_doc(path));
    }
}

// ----- colours

/// The stylesheet: Authenticity's colours from the settings, scoped to marked notes.
pub fn apply_style(a: &App) {
    let scope = style::scope(ATTR, SCOPE);
    let op = a.settings.strength.clamp(0.1, 1.0) as f32;
    let colour = |hex: &str, fallback: (u8, u8, u8)| style::rgb(hex).unwrap_or(fallback);
    let (red, green, orange, blue) = (colour(&a.settings.ai_colour, (0xe5, 0x48, 0x4d)), colour(&a.settings.human_colour, (0x30, 0xa4, 0x6c)), (0xf7, 0x6b, 0x15), (0x25, 0x63, 0xeb));
    let mut css = String::new();
    // Show marks in notes off: nothing is painted (the marks stay, unpainted, with every result kept)
    if !a.settings.show_marks {
        style::set_stylesheet(STYLESHEET, &css);
        return;
    }
    // the AI tints and the similarity underlines, each left out while hidden (the ribbon menu): the marks stay, unpainted
    if a.settings.show_ai_marks {
        css += &style::stepped(&scope, BASE, "auth-ai", style::Fill::Background, red, style::dark(red), op);
        css += &style::stepped(&scope, BASE, "auth-human", style::Fill::Background, green, style::dark(green), op);
        css += &style::rule(&scope, "auth-mixed", &style::paint(style::Fill::Background, orange, op * 0.6), &style::paint(style::Fill::Background, style::dark(orange), op * 0.6));
    }
    let line = |kind: &str, width: u8, (r, g, b): (u8, u8, u8), a: f32| format!("text-decoration: underline {kind} rgba({r}, {g}, {b}, {a:.2}); text-decoration-thickness: {width}px; text-underline-offset: 3px;");
    let (lb, db) = (blue, (0x6b, 0x9b, 0xff));
    if a.settings.show_plag_marks {
        for (class, kind, width, alpha) in [("auth-plag-match", "solid", 3, 1.0), ("auth-plag-near", "solid", 3, 0.75), ("auth-plag-para", "dashed", 2, 1.0), ("auth-plag-common", "dotted", 2, 0.45)] {
            css += &style::rule(&scope, class, &line(kind, width, lb, alpha), &line(kind, width, db, alpha));
        }
    }
    let o = "var(--color-orange)";
    let r = "var(--color-red)";
    let p = "var(--color-purple)";
    css += &style::rule(&scope, "auth-char-hidden::before", &format!("content: \"INV\"; margin: 0 1px; padding: 0 3px; border-radius: var(--radius-s); background-color: {o}; color: var(--text-on-accent); font-family: var(--font-monospace); font-size: 0.65em; vertical-align: 0.15em;"), &format!("content: \"INV\"; margin: 0 1px; padding: 0 3px; border-radius: var(--radius-s); background-color: {o}; color: var(--text-on-accent); font-family: var(--font-monospace); font-size: 0.65em; vertical-align: 0.15em;"));
    let space = format!("background-color: color-mix(in srgb, {o} 40%, transparent); outline: 1px dashed {o};");
    css += &style::rule(&scope, "auth-char-space", &space, &space);
    let dash = format!("background-color: color-mix(in srgb, {r} 30%, transparent); border-radius: 2px;");
    css += &style::rule(&scope, "auth-char-dash", &dash, &dash);
    let punct = format!("outline: 1px solid {o}; background-color: color-mix(in srgb, {o} 25%, transparent); border-radius: 2px;");
    css += &style::rule(&scope, "auth-char-punct", &punct, &punct);
    let look = format!("text-decoration: underline wavy {r};");
    css += &style::rule(&scope, "auth-char-lookalike", &look, &look);
    let typo = format!("text-decoration: underline dotted {p}; text-decoration-thickness: 2px; text-underline-offset: 3px;");
    css += &style::rule(&scope, "auth-char-typo", &typo, &typo);
    style::set_stylesheet(STYLESHEET, &css);
}

pub fn remove_style() {
    style::remove_stylesheet(STYLESHEET);
}

// ----- copy with marks

/// The colours a copy names, by index.
fn named() -> Vec<highlights::Named> {
    let n = |c: &str, name: &str| highlights::Named { colour: c.into(), name: name.into() };
    vec![n("#e5484d", "Reads as AI"), n("#30a46c", "Reads as written by a person"), n("#f76b15", "Mixed"), n("#2563eb", "Shared wording"), n("#2563eb", "Paraphrase"), n("#f76b15", "Unusual character")]
}

/// The selection of `editor` with its marks as markers, for the clipboard.
pub fn copy_selection(editor: &JsValue, path: &str) -> Option<(String, usize)> {
    let (_, from, to) = copy::selection(editor)?;
    let text = str(&call!(editor, getValue));
    let marks = with(|a| {
        a.render.version += 1;
        let v = a.render.version;
        a.render.docs.insert(path.to_string(), (v, text.clone()));
        a.marks_of(path)
    })?;
    let hl: Vec<highlights::Mark> = marks
        .iter()
        .filter_map(|m| {
            let colour = match m.mark.class {
                AI => 0,
                HUMAN => 1,
                MIXED => 2,
                c if c == PLAG || c == PLAG + 1 => 3,
                c if c == PLAG + 2 => 4,
                c if c >= CHAR0 => 5,
                _ => return None,
            };
            Some(highlights::Mark { from: m.mark.from, to: m.mark.to, colour, phrase: m.mark.class >= PLAG })
        })
        .collect();
    Some(highlights::write_range(&text, &hl, &named(), highlights::Format::Tilde, from, to))
}

pub fn copy_now(editor: JsValue, path: String) {
    let Some((out, n)) = copy_selection(&editor, &path) else {
        with(|a| a.notice("select the text to copy first."));
        return;
    };
    js::spawn(async move {
        let r = copy::to_clipboard(out).await;
        with(|a| a.notice(&match r {
            Ok(()) => format!("copied with {n} mark{}.", if n == 1 { "" } else { "s" }),
            Err(e) => format!("could not copy ({e})."),
        }));
    });
}

pub fn menu_item(menu: &JsValue, editor: &JsValue, path: String) {
    let e = editor.clone();
    copy::menu_item(menu, "Copy with Authenticity marks", "clipboard-copy", move || copy_now(e.clone(), path.clone()));
}

pub fn register_command(plugin: &JsValue) {
    copy::editor_command(plugin, "copy-with-marks", "Copy the selection with Authenticity's marks", |editor, view, checking| {
        if !copy::has_selection(editor) {
            return false;
        }
        if !checking {
            copy_now(editor.clone(), str(&get(&get(view, "file"), "path")));
        }
        true
    });
}

/// Select [from, to) of `path` in its editor and scroll to it.
pub fn jump(path: &str, from: u32, to: u32) {
    let Some(Some((ws, leaf))) = with(|a| a.markdown_leaf(path).map(|l| (get(&a.app, "workspace"), l))) else { return };
    call!(ws, setActiveLeaf, leaf.clone(), obj! { focus: true });
    let editor = get(&get(&leaf, "view"), "editor");
    let (a, b) = (call!(editor, offsetToPos, from), call!(editor, offsetToPos, to));
    call!(editor, setSelection, a.clone(), b.clone());
    call!(editor, scrollIntoView, obj! { from: a, to: b }, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_and_lines() {
        assert_eq!(CLASSES.len(), 14);
        assert_eq!(CLASSES[char_class(chars::Kind::Typographic) as usize], "auth-char-typo");
        assert_eq!(CLASSES[(PLAG + 2) as usize], "auth-plag-para");
        assert_eq!(line_starts("a\nb\r\nc"), vec![0, 2, 5]);
        assert_eq!(locate("one two three", 4, 7, "two"), Some((4, 7)));
        assert_eq!(locate("one TWO three", 0, 3, "two"), Some((4, 7)), "found again after an edit");
    }
}
