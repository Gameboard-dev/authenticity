//! The two side panes, for the note last active:
//!
//! - **Authenticity: AI Detector** (`authenticity-ai`): the AI check's paragraphs (most AI first; click one to go to
//!   it), and the characters Reveal AI characters found (they are the other sign of text pasted from a model);
//! - **Authenticity: Plagiarism** (`authenticity-plagiarism`): the thresholds (the close-paraphrase closeness and the
//!   shortest run, the same settings as the settings tab), a count line, and the comparison table (table.rs): *My
//!   sentence* beside *Similar source sentence* with their shared words marked, the source with a link that opens the
//!   library note at the line, the kind and the closeness; the field's fixed phrasing listed apart, not counted.
//!
//! Each is a subclass of ItemView made at run time (obsidian_kit's `item_view`), so no JavaScript class is needed. The
//! combined panel of 0.2 (`authenticity-panel`) is still registered, so a workspace that saved one loads without an
//! error: its leaf turns into the AI Detector pane.
//!
//! Everything is drawn as DOM nodes with text (createEl), never as HTML.
//!
//! Obsidian's API: <https://docs.obsidian.md/Reference/TypeScript+API/ItemView>, getRightLeaf, revealLeaf, setTooltip,
//! Workspace.getLeaf, WorkspaceLeaf.openFile.

use js_sys::Promise;
use wasm_bindgen::prelude::*;

use crate::app::{with, App, CLOSENESS_KEY, MIN_RUN_KEY};
use crate::js::{self, call, f1, get, obj, set, str};
use crate::table::{self, Sort};

/// The view types: the two panes, and 0.2's combined panel (turned into the AI Detector pane when a workspace has one).
pub const AI_VIEW: &str = "authenticity-ai";
pub const PLAG_VIEW: &str = "authenticity-plagiarism";
pub const OLD_VIEW: &str = "authenticity-panel";
pub const VIEWS: [&str; 3] = [AI_VIEW, PLAG_VIEW, OLD_VIEW];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Ai,
    Plagiarism,
}

impl Pane {
    pub fn view(self) -> &'static str {
        match self {
            Pane::Ai => AI_VIEW,
            Pane::Plagiarism => PLAG_VIEW,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Pane::Ai => "Authenticity: AI Detector",
            Pane::Plagiarism => "Authenticity: Plagiarism",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Pane::Ai => "shield-check",
            Pane::Plagiarism => "copy-check",
        }
    }
}

/// The panes open now (their views).
#[derive(Default)]
pub struct Panes {
    pub ai: Option<JsValue>,
    pub plagiarism: Option<JsValue>,
}

impl Panes {
    pub fn any(&self) -> bool {
        self.ai.is_some() || self.plagiarism.is_some()
    }

    fn slot(&mut self, p: Pane) -> &mut Option<JsValue> {
        match p {
            Pane::Ai => &mut self.ai,
            Pane::Plagiarism => &mut self.plagiarism,
        }
    }
}

/// A pane's view for `leaf`.
pub fn create(obs: &JsValue, leaf: JsValue, pane: Pane) -> JsValue {
    let v = js::item_view(obs, leaf, pane.view(), pane.title(), pane.icon());
    set(&v, "authenticityPane", pane.view());
    let me = v.clone();
    set(&v, "onOpen", f1(move |_| {
        let attach = {
            let me = me.clone();
            move || {
                with(|a| {
                    *a.panes.slot(pane) = Some(me.clone());
                    render(a);
                })
                .is_some()
            }
        };
        if !attach.clone()() {
            js::timeout(0.0, move || _ = attach());
        }
        Promise::resolve(&JsValue::UNDEFINED).into()
    }));
    set(&v, "onClose", f1(move |_| {
        with(|a| *a.panes.slot(pane) = None);
        Promise::resolve(&JsValue::UNDEFINED).into()
    }));
    v
}

/// 0.2's combined panel, from a saved workspace: its leaf becomes the AI Detector pane as soon as it opens.
pub fn create_old(obs: &JsValue, leaf: JsValue) -> JsValue {
    let v = js::item_view(obs, leaf.clone(), OLD_VIEW, "Authenticity", Pane::Ai.icon());
    set(&v, "onOpen", f1(move |_| {
        let leaf = leaf.clone();
        js::timeout(0.0, move || {
            let state = obj! { active: false };
            set(&state, "type", AI_VIEW);
            call!(leaf, setViewState, state);
        });
        Promise::resolve(&JsValue::UNDEFINED).into()
    }));
    v
}

/// Open (or reveal) a pane in the right sidebar.
pub fn open(pane: Pane) {
    let Some(ws) = with(|a| get(&a.app, "workspace")) else { return };
    let leaves = js_sys::Array::from(&call!(ws, getLeavesOfType, pane.view()));
    let leaf = if leaves.length() > 0 {
        let leaf = leaves.get(0);
        if str(&get(&get(&leaf, "view"), "authenticityPane")) != pane.view() {
            let state = obj! { active: false };
            set(&state, "type", pane.view());
            call!(leaf, setViewState, state);
        }
        leaf
    } else {
        let leaf = call!(ws, getRightLeaf, false);
        let state = obj! { active: false };
        set(&state, "type", pane.view());
        call!(leaf, setViewState, state);
        leaf
    };
    call!(ws, revealLeaf, leaf);
}

fn el(parent: &JsValue, tag: &str, cls: &str, text: &str) -> JsValue {
    call!(parent, createEl, tag, obj! { cls: cls, text: text })
}

fn on(e: &JsValue, event: &str, f: impl FnMut(JsValue) -> JsValue + 'static) {
    call!(e, addEventListener, event, f1(f));
}

fn on_click(e: &JsValue, mut f: impl FnMut() + 'static) {
    on(e, "click", move |_| {
        f();
        JsValue::UNDEFINED
    });
}

fn button(parent: &JsValue, text: &str, f: impl FnMut() + 'static) -> JsValue {
    let b = el(parent, "button", "", text);
    on_click(&b, f);
    b
}

/// A tooltip, as Obsidian draws them (setTooltip), else the browser's.
fn tooltip(a: &App, e: &JsValue, text: &str) {
    let f = get(&a.obs, "setTooltip");
    if f.is_function() {
        js::apply(&f, &JsValue::NULL, &[e.clone(), text.into()]);
    } else {
        call!(e, setAttribute, "title", text);
    }
}

/// The note's text as its editor has it now, if it is open.
pub fn note_text(a: &App, path: &str) -> Option<String> {
    let leaf = a.markdown_leaf(path)?;
    Some(str(&call!(get(&get(&leaf, "view"), "editor"), getValue)))
}

/// Draw the panes that are open.
pub fn render(a: &App) {
    if let Some(v) = &a.panes.ai {
        render_ai(a, &get(v, "contentEl"));
    }
    if let Some(v) = &a.panes.plagiarism {
        render_plagiarism(a, &get(v, "contentEl"));
    }
}

/// The top of a pane: the note's name, the buttons, what is running and the last failure.
fn head(a: &App, root: &JsValue, pane: Pane) -> String {
    call!(root, empty);
    call!(get(root, "classList"), add, "auth-panel");
    let path = a.note.clone();
    let name = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".md").to_string();
    el(root, "h4", "auth-title", if name.is_empty() { "No note open" } else { &name });
    let row = el(root, "div", "auth-buttons", "");
    match pane {
        Pane::Ai => {
            button(&row, "Check for AI writing", || js::spawn(crate::app::check_ai(false)));
            button(&row, if a.history_open { "Hide history" } else { "History" }, || {
                with(|a| {
                    a.history_open = !a.history_open;
                    a.history_pick = None;
                    render(a);
                });
            });
            button(&row, "Reveal AI characters", || {
                let _ = with(|a| a.reveal.insert(a.note.clone(), None));
                with(|a| a.changed(Some(a.note.clone())));
            });
        }
        Pane::Plagiarism => {
            button(&row, "Check for plagiarism", || js::spawn(crate::app::check_plagiarism(false)));
        }
    }
    button(&row, if a.settings.show_marks { "Hide marks" } else { "Show marks" }, crate::app::toggle_marks);
    if let Some(activity) = &a.activity {
        let row = el(root, "div", "auth-activity", "");
        el(&row, "span", "auth-muted", activity);
        let b = button(&row, "Cancel", crate::app::cancel_check);
        tooltip(a, &b, "Stop this check now: nothing more is sent to SlopTotal or read from the library, and nothing from the check is kept. SlopTotal keeps running (Stop SlopTotal now stops it).");
    }
    if let Some(problem) = a.problems.get(&path) {
        el(root, "p", "auth-callout auth-problem", problem);
    }
    path
}

// ----- the AI Detector pane

/// The rows a pane marks and scrolls to: a place (or places) in a note (UTF-16), from Show in AI check, Show in
/// similarity check, or a check of a selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Focus {
    pub pane: Pane,
    pub path: String,
    pub ranges: Vec<(u32, u32)>,
}

/// Whether a row at [from, to) of `path` is one `pane` marks.
fn focused(a: &App, pane: Pane, path: &str, from: u32, to: u32) -> bool {
    a.focus.as_ref().is_some_and(|f| f.pane == pane && f.path == path && f.ranges.iter().any(|r| table::meets((from, to), *r)))
}

/// Mark a row as focused, and scroll the first of them into view once drawn.
fn mark_focused(tr: &JsValue, first: &mut bool) {
    call!(get(tr, "classList"), add, "is-focused");
    if *first {
        *first = false;
        let tr = tr.clone();
        js::timeout(0.0, move || _ = call!(tr, scrollIntoView, obj! { block: "center" }));
    }
}

/// The AI Detector pane: the characters found first (they are what Reveal AI characters shows), then the table of
/// the paragraphs' scores.
fn render_ai(a: &App, root: &JsValue) {
    let path = head(a, root, Pane::Ai);
    let text = note_text(a, &path);
    if a.history_open {
        history(a, root, &path);
    }
    characters(a, root, &path);
    ai_section(a, root, &path, text.as_deref());
}

fn ai_section(a: &App, root: &JsValue, path: &str, text: Option<&str>) {
    el(root, "h5", "", "AI writing");
    if a.device.sloptotal_folder.trim().is_empty() {
        el(root, "p", "auth-callout", "Set the SlopTotal folder (Settings → Authenticity → Writing checks) to check for AI writing. The installation guide says how to install SlopTotal.");
        return;
    }
    let Some(text) = text else {
        el(root, "p", "auth-muted", "Open the note to see its paragraphs' scores.");
        return;
    };
    let mut rows = a.scored(path, text);
    if rows.is_empty() {
        el(root, "p", "auth-muted", "Not checked yet. A check sends this note's prose (without code, maths, comments, tables and citation keys) to SlopTotal on this computer. Paragraphs are tinted from green (reads as written by a person) to red (reads as AI).");
        return;
    }
    let n = rows.len();
    let ai = rows.iter().filter(|(_, s)| s.score.score > crate::sloptotal::AI_MIN).count();
    let human = rows.iter().filter(|(_, s)| s.score.score <= crate::sloptotal::CLEAN_MAX).count();
    let card = el(root, "div", &format!("auth-card auth-card-{}", if ai * 2 > n { "ai" } else if human * 2 > n { "human" } else { "mixed" }), "");
    el(&card, "div", "auth-headline", &format!("{ai} of {n} paragraphs read as AI"));
    el(&card, "div", "auth-muted", &format!("{human} read as written by a person, {} mixed.", n - ai - human));
    el(root, "p", "auth-legend", crate::ai_table::LEGEND);
    el(root, "p", "auth-caveat", "A detector measures how much the wording resembles model output; it does not establish who wrote it, it is unreliable on short paragraphs (under about 80 words) and more often wrong on non-native English, and its detectors can disagree strongly with each other. Read it as a lead, never as proof.");
    let by_score = a.settings.ai_sort != "position";
    if by_score {
        rows.sort_by(|x, y| y.1.score.score.total_cmp(&x.1.score.score));
    }
    sort_select(root, "aiSort", &[("score", "score"), ("position", "position in the note")], if by_score { "score" } else { "position" });
    let weights = a.helper.weights.as_ref();
    let wrap = el(root, "div", "auth-table-wrap", "");
    let t = el(&wrap, "table", "auth-table auth-ai-table", "");
    let head = el(&el(&t, "thead", "", ""), "tr", "", "");
    for h in ["My paragraph", "AI-likeness", "Verdict", "Why (SlopTotal's detectors)", "Hints"] {
        el(&head, "th", "", h);
    }
    let body = el(&t, "tbody", "", "");
    let mut first = true;
    let all = crate::ai::paragraphs(&crate::prose::extract(text));
    for (p, s) in rows {
        let key = crate::ai::key(&p.text);
        let an = a.ai.analyses.get(&key).map(|(_, an)| an);
        let r = crate::ai_table::paragraph_row(&p.text, &s, an, weights);
        let tr = el(&body, "tr", "auth-row", "");
        if focused(a, Pane::Ai, path, p.from, p.to) {
            mark_focused(&tr, &mut first);
        }
        let mine = el(&tr, "td", "auth-cell-mine", "");
        marked(&el(&mine, "span", "", ""), &r.short, &r.marks);
        let open = a.expanded.contains(&key);
        let toggle = el(&mine, "a", "auth-mini", if open { "Hide sentences" } else { "Sentences" });
        let k2 = key.clone();
        on(&toggle, "click", move |e| {
            call!(e, stopPropagation);
            with(|a| {
                if !a.expanded.remove(&k2) {
                    a.expanded.insert(k2.clone());
                }
                render(a);
            });
            JsValue::UNDEFINED
        });
        let pill_cell = el(&tr, "td", "auth-cell-pct", "");
        let pill = el(&pill_cell, "span", &format!("auth-pill {}", r.class), &r.label);
        let verdict = el(&tr, "td", "", &r.verdict);
        if !r.note.is_empty() {
            el(&verdict, "div", "auth-note", &r.note);
        }
        if !r.caveat.is_empty() {
            tooltip(a, &pill, &r.caveat);
            el(&verdict, "div", "auth-muted", &format!("{}.", r.caveat));
        }
        let det = el(&tr, "td", "auth-cell-list", "");
        if s.score.engine == crate::gptzero::ENGINE {
            // GPTZero's answer: its message; its reasons only through AI Patterns, for keys it allow-lists
            if !s.score.message.is_empty() {
                el(&det, "div", "auth-detector-says", &s.score.message);
            }
            el(&det, "div", "auth-muted", "GPTZero gives its reasons (AI Patterns) only to keys it has allow-listed; Sentences shows which sentences it highlights as AI, each with its own probability.");
        } else if an.is_none() {
            let b = el(&det, "button", "auth-mini-button", "Load reasons");
            tooltip(a, &b, "Ask SlopTotal for each detector's reason for this paragraph (starts SlopTotal if it is not running).");
            let t2 = p.text.clone();
            on(&b, "click", move |e| {
                call!(e, stopPropagation);
                js::spawn(load_reasons(t2.clone()));
                JsValue::UNDEFINED
            });
        } else {
            if !r.overall.is_empty() {
                el(&det, "div", "auth-callout auth-problem", &r.overall);
            }
            for d in &r.detectors {
                detector_block(&det, d);
            }
            for f in &r.failed {
                el(&det, "div", "auth-muted", f);
            }
            if !r.uncounted.is_empty() {
                let more = el(&det, "details", "auth-uncounted", "");
                el(&more, "summary", "", &format!("Not counted for this text's language ({})", r.uncounted.len()));
                for d in &r.uncounted {
                    detector_block(&more, d);
                }
            }
        }
        list_cell(&tr, &r.hints);
        // a paragraph scored with its neighbours: the whole group is selected (and glows)
        let (from, to) = s.together.as_ref().and_then(|t| crate::ai::span_range(&all, t)).unwrap_or((p.from, p.to));
        let pth = path.to_string();
        on_click(&tr, move || go_to(&pth, from, to, None));
        if open {
            sentence_rows(a, &body, path, text, &p);
        }
    }
}

/// The History view: the past runs of this note's AI checks (journal.rs), newest first; a run picked shows its scores
/// beside the current ones.
fn history(a: &App, root: &JsValue, path: &str) {
    el(root, "h5", "", "History");
    let runs = crate::journal::runs_of(a, path);
    if runs.is_empty() {
        el(root, "p", "auth-muted", "No past checks of this note are kept (each AI check is recorded on this computer; Settings \u{2192} History).");
        return;
    }
    let list = el(root, "div", "auth-list", "");
    for r in runs.iter().take(30) {
        let date = js_sys::Date::new(&JsValue::from(r.t * 1000.0));
        let when = str(&date.to_locale_string("default", &JsValue::UNDEFINED));
        let scores: Vec<f64> = r.items.iter().map(|i| i.score).collect();
        let mean = if scores.is_empty() { 0.0 } else { scores.iter().sum::<f64>() / scores.len() as f64 };
        let picked = a.history_pick == Some(r.t);
        let item = el(&list, "div", if picked { "auth-item auth-focused" } else { "auth-item" }, "");
        el(&item, "span", "auth-item-text", &format!("{when} \u{b7} {} \u{b7} {} scored, mean {:.0} / 100{}", r.kind, r.items.len(), mean, if r.device.is_empty() { String::new() } else { format!(" \u{b7} {}", r.device) }));
        let t = r.t;
        on_click(&item, move || {
            with(|a| {
                a.history_pick = if a.history_pick == Some(t) { None } else { Some(t) };
                render(a);
            });
        });
        if picked {
            let tbl = el(root, "table", "auth-table", "");
            let head = el(&el(&tbl, "thead", "", ""), "tr", "", "");
            for h in ["Then: the text", "Then", "Now"] {
                el(&head, "th", "", h);
            }
            let body = el(&tbl, "tbody", "", "");
            for i in &r.items {
                let tr = el(&body, "tr", "", "");
                el(&tr, "td", "", i.text.as_deref().unwrap_or(&i.first));
                el(&tr, "td", "auth-cell-pct", &crate::ai_table::label(i.score));
                let now = a.ai.paragraphs.get(&i.key).map(|s| crate::ai_table::label(s.score.score)).unwrap_or_else(|| "\u{2014}".into());
                el(&tr, "td", "auth-cell-pct", &now);
            }
        }
    }
}

/// `text` into `parent`, with the byte ranges `marks` (the words a hint found) highlighted.
fn marked(parent: &JsValue, text: &str, marks: &[std::ops::Range<usize>]) {
    let mut at = 0;
    for m in marks {
        if m.start < at || m.end > text.len() || !text.is_char_boundary(m.start) || !text.is_char_boundary(m.end) {
            continue;
        }
        if m.start > at {
            el(parent, "span", "", &text[at..m.start]);
        }
        el(parent, "mark", "auth-hint-mark", &text[m.clone()]);
        at = m.end;
    }
    if at < text.len() {
        el(parent, "span", "", &text[at..]);
    }
}

/// One detector of the full analysis as its own block: its name in plain words, its result on the scale (a bar and a
/// number), its share of the score, and one sentence.
fn detector_block(parent: &JsValue, d: &crate::ai_table::Detector) {
    let b = el(parent, "div", "auth-detector", "");
    let top = el(&b, "div", "auth-detector-top", "");
    el(&top, "span", "auth-detector-name", &d.plain);
    el(&top, "span", "auth-detector-score", &format!("{:.0} / 100", d.score));
    let bar = el(&b, "div", "auth-bar", "");
    let fill = el(&bar, "div", &format!("auth-bar-fill {}", crate::ai::class(crate::ai::tint(d.score))), "");
    set(&get(&fill, "style"), "width", format!("{:.0}%", d.score.clamp(0.0, 100.0)));
    if let Some(share) = d.share {
        el(&b, "div", "auth-muted", &format!("Counts for {:.0}% of the score.", share * 100.0));
    }
    if !d.says.is_empty() {
        el(&b, "div", "auth-detector-says", &d.says);
    }
}

fn list_cell(tr: &JsValue, lines: &[String]) {
    let td = el(tr, "td", "auth-cell-list", "");
    if lines.is_empty() {
        el(&td, "span", "auth-muted", "\u{2014}");
    }
    for l in lines {
        el(&td, "div", "", l);
    }
}

/// A paragraph's sentences as rows under it, each with its score with its neighbours when asked for (the button asks).
fn sentence_rows(a: &App, body: &JsValue, path: &str, text: &str, p: &crate::ai::Paragraph) {
    // a GPTZero score brings its own sentences
    if let Some(s) = a.ai.of(path, &p.text).filter(|s| s.score.engine == crate::gptzero::ENGINE) {
        let (para, _) = crate::app::slice16(text, p.from, p.to);
        for r in crate::ai_table::gptzero_sentence_rows(&s.score) {
            let tr = el(body, "tr", "auth-row auth-subrow", "");
            el(&tr, "td", "auth-cell-mine", &r.short);
            el(&el(&tr, "td", "auth-cell-pct", ""), "span", &format!("auth-pill {}", r.class), &r.label);
            let verdict = el(&tr, "td", "", &r.verdict);
            if !r.note.is_empty() {
                el(&verdict, "div", "auth-note", &r.note);
            }
            el(&tr, "td", "auth-cell-list", "");
            el(&tr, "td", "auth-cell-list", "");
            let at = crate::align::find_utf16(&para, &r.text).map(|(x, y)| (p.from + x, p.from + y)).unwrap_or((p.from, p.to));
            let pth = path.to_string();
            on_click(&tr, move || go_to(&pth, at.0, at.1, None));
        }
        return;
    }
    let rows = crate::ai_table::sentence_rows(&p.text, |ctx| a.ai.sentences.get(&crate::ai::key(ctx)).map(|s| s.score.clone()));
    let missing = rows.iter().any(|(r, ctx)| ctx.is_some() && r.score.is_nan());
    let (para, _) = crate::app::slice16(text, p.from, p.to);
    for (k, (r, _)) in rows.iter().enumerate() {
        let tr = el(body, "tr", "auth-row auth-subrow", "");
        marked(&el(&tr, "td", "auth-cell-mine", ""), &r.short, &r.marks);
        let pct = el(&tr, "td", "auth-cell-pct", "");
        el(&pct, "span", &format!("auth-pill {}", r.class), &r.label);
        let verdict = el(&tr, "td", "", &r.verdict);
        if !r.note.is_empty() {
            el(&verdict, "div", "auth-note", &r.note);
        }
        let det = el(&tr, "td", "auth-cell-list", "");
        if k == 0 && missing {
            let b = el(&det, "button", "auth-mini-button", "Score sentences");
            tooltip(a, &b, "Score each sentence with its neighbours (less reliable than the paragraph's score: short text). Starts SlopTotal if it is not running.");
            let t2 = p.text.clone();
            on(&b, "click", move |e| {
                call!(e, stopPropagation);
                js::spawn(score_sentences(t2.clone()));
                JsValue::UNDEFINED
            });
        }
        list_cell(&tr, &r.hints);
        let at = crate::align::find_utf16(&para, &r.text).map(|(x, y)| (p.from + x, p.from + y)).unwrap_or((p.from, p.to));
        let pth = path.to_string();
        on_click(&tr, move || go_to(&pth, at.0, at.1, None));
    }
}

/// Ask SlopTotal for each detector's reason for a paragraph (an explicit request: SlopTotal is started if needed).
async fn load_reasons(paragraph: String) {
    let run = crate::app::run_now();
    if let Err(e) = crate::helper::ensure_sloptotal("the detectors' reasons").await {
        if !crate::app::cancelled(run) {
            with(|a| {
                a.busy(None);
                a.notice(&e);
            });
        }
        return;
    }
    with(|a| a.busy(Some("AI check: asking each detector's reason\u{2026}".into())));
    crate::helper::load_weights().await;
    let r = crate::sloptotal::analyze(&crate::helper::proxy(), &paragraph).await;
    if crate::app::cancelled(run) {
        return;
    }
    with(|a| {
        a.busy(None);
        match r {
            Ok(mut an) => {
                crate::sloptotal::settle(&mut an, a.helper.weights.as_ref(), &paragraph);
                // the history: every detector's result and weight
                let lang = a.helper.weights.as_ref().map(|w| w.for_language(&an.language).clone()).unwrap_or_default();
                let names = a.helper.weights.as_ref().map(|w| w.names.clone()).unwrap_or_default();
                let mut it = crate::journal::item(&paragraph, an.score.unwrap_or(an.overall), a.settings.keep_text);
                it.detectors = an.reasons.iter().map(|r| (r.engine.clone(), r.score * 100.0, lang.get(names.get(&r.engine).unwrap_or(&r.engine)).copied())).collect();
                it.failed = an.failed.iter().map(|f| format!("{}: {}", f.engine, f.details)).collect();
                let run = crate::journal::Run { t: js_sys::Date::now() / 1000.0, note: a.note.clone(), kind: "analysis".into(), engine: String::new(), device: a.helper.device_in_use(), helper: a.helper.last.as_ref().map(|h| h.version.clone()).unwrap_or_default(), items: vec![it] };
                crate::journal::debug(a, "analysis", serde_json::json!({ "key": run.items[0].key, "detectors": run.items[0].detectors, "failed": run.items[0].failed, "score": an.score }), None);
                crate::journal::record(a, &run);
                a.ai.analyses.insert(crate::ai::key(&paragraph), (js_sys::Date::now(), an));
                a.save_ai();
                render(a);
            }
            Err(e) => a.notice(&format!("the reasons could not be had: {e}")),
        }
    });
}

/// Score a paragraph's sentences, each with its neighbours (those scored before are not sent again).
async fn score_sentences(paragraph: String) {
    let sents = crate::ai::sentences(&paragraph);
    let todo: Vec<String> = (0..sents.len())
        .filter_map(|k| crate::ai::context(&paragraph, &sents, k))
        .filter(|c| with(|a| !a.ai.sentences.contains_key(&crate::ai::key(c))).unwrap_or(false))
        .collect();
    if todo.is_empty() {
        return;
    }
    let run = crate::app::run_now();
    if let Err(e) = crate::helper::ensure_sloptotal("the sentences' scores").await {
        if !crate::app::cancelled(run) {
            with(|a| {
                a.busy(None);
                a.notice(&e);
            });
        }
        return;
    }
    with(|a| a.busy(Some(format!("AI check: scoring {} sentences\u{2026}", todo.len()))));
    crate::helper::load_weights().await;
    let weights = with(|a| a.helper.weights.clone()).flatten();
    // kept only when the whole paragraph's sentences were scored or failed: a cancelled run keeps nothing
    let mut got = Vec::new();
    for ctx in todo {
        match crate::sloptotal::quick(&crate::helper::proxy(), &ctx, weights.as_ref()).await {
            _ if crate::app::cancelled(run) => return,
            Ok(s) => got.push((ctx, s)),
            Err(e) => {
                with(|a| a.notice(&format!("a sentence could not be scored: {e}")));
                break;
            }
        }
    }
    with(|a| {
        // the history: each sentence's context scored
        let items = got.iter().map(|(ctx, s)| {
            let mut it = crate::journal::item(ctx, s.score, a.settings.keep_text);
            it.detectors = s.engines.iter().map(|(k, x)| (k.clone(), *x, None)).collect();
            it.failed = s.failed.clone();
            it
        }).collect();
        crate::journal::record(a, &crate::journal::Run { t: js_sys::Date::now() / 1000.0, note: a.note.clone(), kind: "sentences".into(), engine: String::new(), device: a.helper.device_in_use(), helper: a.helper.last.as_ref().map(|h| h.version.clone()).unwrap_or_default(), items });
        for (ctx, s) in got {
            a.ai.sentences.insert(crate::ai::key(&ctx), crate::ai::Scored { score: s, when: js_sys::Date::now(), device: String::new(), together: None });
        }
        a.busy(None);
        a.save_ai();
        render(a);
    });
}

/// A sort choice kept in the settings (`key`).
fn sort_select(root: &JsValue, key: &'static str, options: &[(&str, &str)], now: &str) {
    let row = el(root, "label", "auth-sort", "Sort by ");
    let select = call!(row, createEl, "select", obj! { cls: "dropdown" });
    for (k, text) in options {
        let o = el(&select, "option", "", text);
        set(&o, "value", *k);
    }
    set(&select, "value", now);
    on(&select, "change", move |e| {
        let v = str(&get(&get(&e, "target"), "value"));
        with(|a| a.set_field(key, v.into()));
        JsValue::UNDEFINED
    });
}

fn characters(a: &App, root: &JsValue, path: &str) {
    let Some(hits) = a.hits.get(path) else { return };
    if hits.is_empty() && !a.reveal.contains_key(path) {
        return;
    }
    el(root, "h5", "", "AI characters");
    if hits.is_empty() {
        el(root, "p", "auth-muted", "None found.");
        return;
    }
    let list = el(root, "div", "auth-list", "");
    for (i, g) in crate::walk::groups(hits).into_iter().enumerate() {
        let item = el(&list, "div", "auth-item", "");
        let ch = char::from_u32(g.code).unwrap_or(' ');
        let shown = if g.short.is_empty() { ch.to_string() } else { g.short.to_string() };
        el(&item, "span", &format!("auth-pill {}", g.kind.class()), &shown);
        el(&item, "span", "auth-item-text", &format!("{} \u{d7} {} (U+{:04X}, {})", crate::app::format_count(g.at.len()), g.name, g.code, g.kind.label().to_lowercase()));
        tooltip(a, &item, "Go through them one by one: OK (Enter) replaces the selected one (one undo), Skip (Esc) skips it, Stop ends.");
        let p = path.to_string();
        on_click(&item, move || js::spawn(crate::app::step_through(p.clone(), i)));
    }
}

// ----- the Plagiarism pane

const CLOSENESS_TIP: &str = "Closeness of a paraphrase: the share of my sentence's content words (the words other than the, of, and \u{2026}) found in the source sentence in the same order. A sentence is flagged from this share up when the words it shares are the source sentence's particular ones, not only common ground (and, with the optional embedding search, when the two also say the same). Lower flags more; never below 30%. The same setting as Settings \u{2192} Authenticity \u{2192} Close paraphrase from.";
const MIN_RUN_TIP: &str = "Verbatim and near-verbatim wording counts from this many words in a row shared with a library note (6, the usual rule; case and punctuation aside, quotations left out). The same setting as Settings \u{2192} Authenticity \u{2192} Shortest run of words.";

fn render_plagiarism(a: &App, root: &JsValue) {
    let path = head(a, root, Pane::Plagiarism);
    thresholds(a, root);
    let Some(c) = a.found.get(&path) else {
        el(root, "p", "auth-muted", "Not checked yet. A check compares this note with your library notes (never your drafts), on this computer, for copied wording and close paraphrase.");
        return;
    };
    if let Some(why) = a.para_note.get(&path) {
        el(root, "p", "auth-muted", why);
    }
    let t = a.thresholds();
    let outside = c.outside(t);
    if !outside.is_empty() {
        let box_ = el(root, "div", "auth-callout auth-recheck", "");
        el(&box_, "div", "", &format!("{} Lower than that needs a new check.", outside.join(" ")));
        button(&box_, "Check the note again", || js::spawn(crate::app::check_plagiarism(false)));
    }
    let shown: &[crate::similarity::Match] = a.matches.get(&path).map(Vec::as_slice).unwrap_or(&[]);
    el(root, "p", "auth-summary", &format!("{}.", table::summary(shown, c.sentences, c.selection)));
    let sort = Sort::parse(&a.settings.plag_sort);
    let rows = table::rows(shown, sort);
    if !rows.is_empty() {
        sort_select(root, "plagSort", &[("closeness", "closeness"), ("position", "position in the note")], sort.key());
        comparison(a, root, &path, &rows);
        el(root, "p", "auth-caveat", "Marked words: those my sentence shares with the source sentence in the same order (for a paraphrase, the content words the closeness counts). Click a row to go to the sentence; the source opens the library note at the line. Wording copied without quotation marks is plagiarism even when cited: quote it exactly, with the page, or say it in your own words and structure.");
    }
    let common: Vec<&crate::similarity::Match> = shown.iter().filter(|m| m.kind == "common").collect();
    if !common.is_empty() {
        let shown: Vec<String> = common.iter().take(15).map(|m| format!("\u{201c}{}\u{201d} ({} notes)", m.text.split_whitespace().collect::<Vec<_>>().join(" "), m.sources)).collect();
        el(root, "h5", "", &format!("The field's fixed phrasing, not counted ({})", common.len()));
        el(root, "p", "auth-muted", &shown.join("; "));
    }
}

/// The two thresholds at the top of the pane: changing either re-filters at once (app.rs, set_field).
fn thresholds(a: &App, root: &JsValue) {
    let box_ = el(root, "div", "auth-thresholds", "");
    let t = a.thresholds();
    // the close-paraphrase closeness: a slider, its value beside it while it moves, applied on release
    let row = el(&box_, "label", "auth-threshold", "");
    let label = el(&row, "span", "auth-threshold-label", "");
    el(&label, "span", "auth-threshold-term", "Close paraphrase from");
    tooltip(a, &label, CLOSENESS_TIP);
    let slider = call!(row, createEl, "input");
    call!(slider, setAttribute, "type", "range");
    for (k, v) in [("min", crate::para::MIN_CLOSENESS), ("max", crate::para::MAX_CLOSENESS), ("step", 0.05)] {
        call!(slider, setAttribute, k, v.to_string());
    }
    set(&slider, "value", t.closeness.to_string());
    tooltip(a, &slider, CLOSENESS_TIP);
    let value = el(&row, "span", "auth-threshold-value", &format!("{:.0}%", t.closeness * 100.0));
    let v2 = value.clone();
    on(&slider, "input", move |e| {
        let x = str(&get(&get(&e, "target"), "value")).parse::<f64>().unwrap_or(0.0);
        call!(v2, setText, format!("{:.0}%", x * 100.0));
        JsValue::UNDEFINED
    });
    on(&slider, "change", |e| {
        if let Ok(x) = str(&get(&get(&e, "target"), "value")).parse::<f64>() {
            with(|a| a.set_field(CLOSENESS_KEY, ((x * 100.0).round() / 100.0).into()));
        }
        JsValue::UNDEFINED
    });
    // back to the default, shown only when the closeness is not it; re-filters at once like the slider
    if (t.closeness - crate::para::CLOSENESS).abs() > 1e-9 {
        let reset = el(&row, "button", "auth-threshold-reset", &format!("Reset to {:.0}%", crate::para::CLOSENESS * 100.0));
        call!(reset, setAttribute, "type", "button");
        tooltip(a, &reset, "Back to the default paraphrase closeness; what the last check found is filtered again at once, with no new search.");
        on(&reset, "click", |e| {
            call!(e, preventDefault);
            with(|a| a.set_field(CLOSENESS_KEY, crate::para::CLOSENESS.into()));
            JsValue::UNDEFINED
        });
    }
    // the shortest run, in words
    let row = el(&box_, "label", "auth-threshold", "");
    let label = el(&row, "span", "auth-threshold-label", "");
    el(&label, "span", "auth-threshold-term", "Verbatim from");
    tooltip(a, &label, MIN_RUN_TIP);
    let number = call!(row, createEl, "input");
    call!(number, setAttribute, "type", "number");
    for (k, v) in [("min", authenticity_similar::K), ("max", 30), ("step", 1)] {
        call!(number, setAttribute, k, v.to_string());
    }
    set(&number, "value", t.min_run.to_string());
    tooltip(a, &number, MIN_RUN_TIP);
    el(&row, "span", "auth-threshold-value", "words");
    on(&number, "change", |e| {
        if let Ok(n) = str(&get(&get(&e, "target"), "value")).trim().parse::<f64>() {
            with(|a| a.set_field(MIN_RUN_KEY, n.round().clamp(authenticity_similar::K as f64, 30.0).into()));
        }
        JsValue::UNDEFINED
    });
}

/// A cell's pieces, the shared ones in a mark.
fn cell(parent: &JsValue, cls: &str, segs: &[table::Seg]) {
    let td = el(parent, "td", cls, "");
    for s in segs {
        el(&td, if s.shared { "mark" } else { "span" }, if s.shared { "auth-shared" } else { "" }, &s.text);
    }
}

fn comparison(a: &App, root: &JsValue, path: &str, rows: &[table::Row]) {
    let wrap = el(root, "div", "auth-table-wrap", "");
    let tableel = el(&wrap, "table", "auth-table", "");
    let head = el(&el(&tableel, "thead", "", ""), "tr", "", "");
    for h in ["My sentence", "Similar source sentence", "Source", "Kind", "Closeness"] {
        el(&head, "th", "", h);
    }
    let body = el(&tableel, "tbody", "", "");
    let mut first = true;
    for r in rows {
        let tr = el(&body, "tr", "auth-row", "");
        if focused(a, Pane::Plagiarism, path, r.from, r.to) {
            mark_focused(&tr, &mut first);
        }
        cell(&tr, "auth-cell-mine", &r.mine);
        cell(&tr, "auth-cell-source", &r.source);
        let src = el(&tr, "td", "auth-cell-ref", "");
        let link = el(&src, "a", "auth-source-link", &r.title);
        if r.note.is_empty() {
            call!(get(&link, "classList"), add, "is-unresolved");
        } else {
            tooltip(a, &link, &format!("Open {}{}", r.note, r.line.map(|l| format!(" at line {}", l + 1)).unwrap_or_default()));
            let (note, line) = (r.note.clone(), r.line);
            on(&link, "click", move |e| {
                call!(e, stopPropagation);
                call!(e, preventDefault);
                open_source(&note, line);
                JsValue::UNDEFINED
            });
        }
        if let Some(p) = r.page {
            el(&src, "div", "auth-muted", &format!("p. {p}"));
        }
        el(&tr, "td", "auth-cell-kind", r.kind);
        let pct = el(&tr, "td", "auth-cell-pct", &format!("{}%", r.closeness));
        tooltip(a, &pct, &r.tip);
        let (path, from, to, words) = (path.to_string(), r.from, r.to, r.text.clone());
        on_click(&tr, move || go_to(&path, from, to, Some(&words)));
    }
}

/// Go to [from, to) of the note (found again by `words` if it moved since), select it and flash it.
fn go_to(path: &str, from: u32, to: u32, words: Option<&str>) {
    let at = match words {
        Some(w) => with(|a| note_text(a, path).and_then(|t| crate::render::locate(&t, from, to, w))).flatten(),
        None => Some((from, to)),
    };
    let Some((x, y)) = at else {
        with(|a| a.notice("that passage has changed since the check; check again to find it."));
        return;
    };
    crate::render::jump(path, x, y);
    // a brief flash on the editor, so the eye finds the place (styles.css, .auth-flash)
    let Some(Some(dom)) = with(|a| a.markdown_leaf(path).map(|l| get(&get(&get(&get(&l, "view"), "editor"), "cm"), "dom"))) else { return };
    if dom.is_object() {
        let cls = get(&dom, "classList");
        call!(cls, remove, "auth-flash");
        call!(cls, add, "auth-flash");
        js::timeout(1300.0, move || _ = call!(cls, remove, "auth-flash"));
    }
}

/// Open a library note in a new tab, at `line` (from 0) when known.
fn open_source(note: &str, line: Option<u32>) {
    let Some((ws, file)) = with(|a| (get(&a.app, "workspace"), call!(get(&a.app, "vault"), getAbstractFileByPath, note))) else { return };
    if file.is_null() || file.is_undefined() {
        with(|a| a.notice(&format!("{note} is not in the vault any more.")));
        return;
    }
    let leaf = call!(ws, getLeaf, "tab");
    let state = match line {
        Some(l) => obj! { eState: obj! { line: l } },
        None => obj! {},
    };
    call!(leaf, openFile, file, state);
}
