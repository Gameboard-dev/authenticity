//! The plugin on Obsidian's main thread: its settings (data.json, synchronised with the vault) and this computer's own
//! settings (Obsidian's local storage, never synchronised: the SlopTotal folder, the device, the helper), the commands
//! and the editor's context menu, the status bar, and the two checks:
//!
//! - **Check for AI writing**: the note's (or selection's) paragraphs, as prose, scored by SlopTotal through the helper
//!   (helper.rs, sloptotal.rs); paragraphs already scored are not sent again;
//! - **Check for plagiarism**: the text compared with the library notes (index.rs, similarity.rs): copied wording, and
//!   close paraphrase by the library index's content-word search (or, when the user turns it on, an embedding search,
//!   api.rs).
//!
//! **Reveal AI characters** marks the characters chars.rs finds in a selection or note. Nothing is ever rewritten.
//!
//! Obsidian's API: <https://docs.obsidian.md/Reference/TypeScript+API/Plugin> (loadData, saveData, addSettingTab,
//! addCommand, addStatusBarItem, addRibbonIcon, registerView, registerEditorExtension, registerEvent),
//! <https://docs.obsidian.md/Reference/TypeScript+API/App/loadLocalStorage>, requestUrl, Modal, Notice.
//!
//! Anything that can make CodeMirror or Obsidian call back into the plugin (a dispatch, opening a view, moving the
//! selection) is done outside `with`, since a call back into `with` while it is held finds nothing.

use std::cell::RefCell;
use std::collections::HashMap;

use obsidian_kit::settings::{self, Ctl, Row};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::chars::Hit;
use crate::js::{self, call, f0, f1, f2, f3, get, new, obj, set, str};
use crate::similarity::Match;

const README: &str = "https://github.com/Gameboard-dev/authenticity#readme";
/// Obsidian's local storage key for this computer's settings.
const LOCAL_KEY: &str = "authenticity-device";

/// The settings synchronised with the vault (data.json): no paths to programs, no addresses.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub ai_colour: String,
    pub human_colour: String,
    /// The opacity of the strongest tint, 0 to 1.
    pub strength: f64,
    /// Whether the plugin's marks are shown in notes at all; off hides them without losing any result.
    pub show_marks: bool,
    /// The AI check's paragraph tints, and the plagiarism check's underlines, each shown or hidden on its own (the
    /// ribbon menu); off keeps the results.
    pub show_ai_marks: bool,
    pub show_plag_marks: bool,
    /// Mark unusual characters in every note in scope as it is drawn (Reveal AI characters marks them on request).
    pub show_hidden: bool,
    pub show_typographic: bool,
    /// "Replace unusual characters" also replaces typographic characters (curly quotes, dashes, the ellipsis ...).
    pub replace_typographic: bool,
    /// Comma-separated folders whose notes have their characters marked; blank: every note.
    pub folders: String,
    /// Above this many words, an AI check asks first (SlopTotal on the CPU takes a few seconds a paragraph).
    pub confirm_over: f64,
    /// The library folder (a vault folder; blank: not set, and the plagiarism check asks for it).
    pub library_folder: String,
    /// More folders whose notes are never sources (Drafts/ and LaTeX/ always are).
    pub exclude: String,
    pub min_run: f64,
    pub content_words: f64,
    pub common_sources: f64,
    pub near_verbatim: bool,
    pub ignore_quotes: bool,
    pub fixed_terms: String,
    /// Look for close paraphrase, and the share of a sentence's content words kept in a source sentence's order from
    /// which it is one (para.rs); and whether its candidates come from the optional embedding search (api.rs).
    pub paraphrase: bool,
    pub paraphrase_closeness: f64,
    pub paraphrase_embeddings: bool,
    /// The Plagiarism pane's order: "closeness" or "position".
    pub plag_sort: String,
    /// The AI Detector pane's order: "score" or "position".
    pub ai_sort: String,
    /// The name of the secret in Obsidian's keychain that holds the GPTZero key (never the key itself).
    pub gptzero_secret: String,
    /// The history of checks (journal.rs): kept this many days; with the checked text itself or not. The debug log.
    pub history_days: f64,
    pub keep_text: bool,
    pub debug_log: bool,
}

/// The two thresholds the settings tab and the Plagiarism pane both set: one value in data.json, read by both.
pub const MIN_RUN_KEY: &str = "minRun";
pub const CLOSENESS_KEY: &str = "paraphraseCloseness";

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ai_colour: "#e5484d".into(),
            human_colour: "#30a46c".into(),
            strength: 0.6,
            show_marks: true,
            show_ai_marks: true,
            show_plag_marks: true,
            show_hidden: true,
            show_typographic: true,
            replace_typographic: false,
            folders: String::new(),
            confirm_over: 5000.0,
            library_folder: String::new(),
            exclude: String::new(),
            min_run: 6.0,
            content_words: 4.0,
            common_sources: 5.0,
            near_verbatim: true,
            ignore_quotes: true,
            fixed_terms: String::new(),
            paraphrase: true,
            paraphrase_closeness: crate::para::CLOSENESS,
            paraphrase_embeddings: false,
            plag_sort: "closeness".into(),
            ai_sort: "score".into(),
            gptzero_secret: String::new(),
            history_days: 30.0,
            keep_text: false,
            debug_log: false,
        }
    }
}

impl Settings {
    /// These settings with `key` (data.json's name, as the settings tab and the panes give it) set to `value`; None if
    /// the value does not fit the setting. The one way either place changes a setting.
    pub fn with_field(&self, key: &str, value: serde_json::Value) -> Option<Settings> {
        let mut o = serde_json::to_value(self).ok()?;
        o.as_object_mut()?.insert(key.to_string(), value);
        serde_json::from_value(o).ok()
    }

    /// The shortest run and the close-paraphrase closeness now, as the results are filtered with.
    pub fn thresholds(&self) -> crate::similarity::Thresholds {
        crate::similarity::Thresholds::new(self.min_run, self.paraphrase_closeness)
    }

    pub fn similarity(&self) -> crate::similarity::Settings {
        crate::similarity::Settings {
            min_run: self.min_run.max(0.0) as usize,
            content: self.content_words.max(0.0) as usize,
            common: self.common_sources.max(0.0) as usize,
            near: self.near_verbatim,
            ignore_quotes: self.ignore_quotes,
            fixed_terms: crate::similarity::terms(&self.fixed_terms),
        }
    }
}

/// This computer's settings (Obsidian's local storage): where SlopTotal is, which device it uses, and the helper.
/// Never in data.json, so a program's path never comes from the synchronised vault.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Device {
    pub sloptotal_folder: String,
    pub sloptotal_port: f64,
    /// "auto", "gpu" or "cpu".
    pub device: String,
    /// The helper program; blank: authenticity-helper in the plugin's folder.
    pub helper_path: String,
    pub helper_port: f64,
    pub idle_minutes: f64,
}

impl Default for Device {
    fn default() -> Self {
        Device { sloptotal_folder: String::new(), sloptotal_port: 8000.0, device: "auto".into(), helper_path: String::new(), helper_port: 8791.0, idle_minutes: 10.0 }
    }
}

pub fn port(p: f64, default: u16) -> u16 {
    if (1024.0..=65535.0).contains(&p) { p as u16 } else { default }
}

pub struct App {
    pub plugin: JsValue,
    pub obs: JsValue,
    pub app: JsValue,
    pub require: JsValue,
    pub settings: Settings,
    pub device: Device,
    /// The AI check's results (ai.json); what each note's plagiarism check found (found.json) and, of that, what
    /// passes the thresholds now (matches.json, which the marks, the panes, the status bar and the MCP server read).
    pub ai: crate::ai::Store,
    pub found: HashMap<String, crate::similarity::Checked>,
    pub matches: HashMap<String, Vec<Match>>,
    /// The characters found in each note, from the last time its editor was drawn.
    pub hits: HashMap<String, Vec<Hit>>,
    /// Characters revealed on request: the note's whole text (None) or a range of it.
    pub reveal: HashMap<String, Option<(u32, u32)>>,
    /// Counts the checks cancelled: a check takes it when it starts and stops when it has changed (`alive`).
    pub run: u64,
    /// The marks drawn in editors and Reading view (the shared view-marks crate), and what render.rs keeps for them.
    pub marks: view_marks::Marks,
    pub render: crate::render::State,
    pub status: JsValue,
    /// The two side panes, while open.
    pub panes: crate::panel::Panes,
    /// The Markdown note the panel and the status bar follow: the last one active.
    pub note: String,
    /// What is running now (shown in the status bar and the panel), and the last failure per note.
    pub activity: Option<String>,
    pub problems: HashMap<String, String>,
    /// Why paraphrase was not checked in the last plagiarism check of each note.
    pub para_note: HashMap<String, String>,
    /// What the embedding search was asked about each sentence, so that a check again asks only about changed ones.
    pub para_cache: crate::similarity::Cache,
    /// The rows a pane shows marked and scrolled to (Show in AI check, Show in similarity check, a check of text that
    /// had not changed), and the paragraphs whose sentences the AI Detector pane lists (by the paragraph's key).
    pub focus: Option<crate::panel::Focus>,
    pub expanded: std::collections::HashSet<String>,
    /// GPTZero's AI Patterns for a paragraph (by its key), asked for in the AI Detector pane; kept while Obsidian runs.
    pub gz_patterns: HashMap<String, Result<Vec<crate::gptzero::PatternHit>, String>>,
    /// The AI Detector pane's History view: open or not, and the run picked (its time).
    pub history_open: bool,
    pub history_pick: Option<f64>,
    /// The AI check's engine chosen "for this session" (never kept across restarts).
    pub session_engine: Option<String>,
    pub helper: crate::helper::State,
    pub index: crate::index::State,
    persist: Option<f64>,
    /// The minute timer that rewrites the status file (api.rs), and the one that asks whether the server still runs.
    pub status_timer: Option<f64>,
    poll_timer: Option<f64>,
    pub dir: String,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` on the app (`None` once the plugin is unloaded, or while it is already borrowed).
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

const UI: &[Row] = &[
    ("", "Documentation", "How to set up SlopTotal, what each check finds and does not, and what the colours mean: ", Ctl::Link(README)),
    ("", "Writing checks", "", Ctl::Head),
    ("", "", "Everything runs on this computer. The AI-writing check uses SlopTotal, a local detector you install once (the installation guide); the plagiarism check compares your text with the notes in your library folder. The settings below marked \"this computer\" are kept in this computer's Obsidian only, never in the vault.", Ctl::Note),
    ("device.sloptotalFolder", "SlopTotal folder (this computer)", "The folder you installed SlopTotal in, such as %LOCALAPPDATA%\\Programs\\SlopTotal (environment variables and a leading ~ are expanded; Status below shows the folder used). Authenticity runs its Python (venv) only while you check, on 127.0.0.1, with downloads switched off.", Ctl::Text),
    ("device.device", "Device (this computer)", "Automatic uses the graphics card when SlopTotal's Python has PyTorch for CUDA and the card has room for its models (about 8.3 GB with a margin), asking your other local GPU services to free theirs first if you agree; otherwise the processor.", Ctl::Choice(devices)),
    ("device.sloptotalPort", "SlopTotal port (this computer)", "Where SlopTotal answers on 127.0.0.1. A SlopTotal you started yourself on this port is used as it is and never stopped by Authenticity.", Ctl::Number),
    ("device.idleMinutes", "Stop SlopTotal after this many idle minutes (this computer)", "SlopTotal holds about 9 GB of memory while it runs; it is stopped this long after the last check.", Ctl::Number),
    ("device.helperPath", "Helper program (this computer)", "The full path of authenticity-helper, from the native release. Blank: authenticity-helper in the plugin's folder.", Ctl::Text),
    ("", "Status", "", Ctl::Custom(status_row)),
    ("confirmOver", "Ask before checking more than this many words", "On the processor SlopTotal takes a few seconds a paragraph, so a whole chapter asks first. 0: always ask.", Ctl::Number),
    ("", "Plagiarism", "", Ctl::Head),
    ("libraryFolder", "Library folder", "The vault folder of the notes your writing is compared with (your sources, as notes), such as Sources/ or Library/. Required: the plagiarism check asks for it until it is set.", Ctl::Text),
    ("exclude", "Never compare with", "More folders whose notes are your own writing, comma-separated. Drafts/ and LaTeX/ never count, and neither does the note being checked.", Ctl::Text),
    (MIN_RUN_KEY, "Copied wording: shortest run", "Wording shared with a source counts from this many words in a row (6, the usual rule). Case, punctuation and hyphens do not count, and text in quotation marks is left out. Also set at the top of the Plagiarism pane.", Ctl::Number),
    ("ignoreQuotes", "Leave out quoted text", "Text in quotation marks is not compared: a quotation with its citation is meant to match its source.", Ctl::Toggle),
    ("nearVerbatim", "Near-verbatim", "Also find stretches with a few words changed, inserted or left out.", Ctl::Toggle),
    ("contentWords", "Fewest content words", "A run with fewer words that are not function words (the, of, and) or fixed terms is ordinary language, not copied wording.", Ctl::Number),
    ("commonSources", "Fixed phrasing from", "A run found in this many library notes or more is the field's fixed phrasing, listed apart. 0: never.", Ctl::Number),
    ("fixedTerms", "Fixed terms", "Technical terms that are not copied wording however often they appear, one per line or comma-separated.", Ctl::Text),
    ("paraphrase", "Close paraphrase", "Sentences that keep a library sentence's own words in its order with a few changed (patchwriting), found by Authenticity's own search over the library's words (no model, nothing leaves this computer). Saying what a source says in your own structure, with its citation, is not flagged.", Ctl::Toggle),
    (CLOSENESS_KEY, "Close paraphrase from", "Paraphrase closeness: the share of your sentence's content words (the words other than the, of, and ...) found in a library sentence in the same order, from which the sentence is flagged; the same with either search. The source sentence must also share the sentence's particular words, not only common ones (or, with the embedding search, say the same). 50% by default; lower flags more, never below 30%. Also set at the top of the Plagiarism pane, where a change shows at once.", Ctl::Slider(crate::para::MIN_CLOSENESS, crate::para::MAX_CLOSENESS, 0.05)),
    ("", "Embedding search (optional)", "", Ctl::Custom(embeddings_row)),
    ("", "Library index", "", Ctl::Custom(index_row)),
    ("", "GPTZero (optional)", "", Ctl::Head),
    ("", "", "Optional and off without a key. GPTZero is an online AI detector. With its API key linked here, each AI check in Obsidian asks first whether to use GPTZero (which sends the checked text, and nothing else, to GPTZero's servers, counted against your GPTZero plan) or SlopTotal (offline). Nothing is sent without that answer. The MCP server always uses SlopTotal.", Ctl::Note),
    ("", "GPTZero API key", "Kept in Obsidian's keychain (Settings \u{2192} Keychain), never in this plugin's files. Create the key in your GPTZero account (gptzero.me/developers), then link it here. Optional: without a key nothing is sent.", Ctl::Custom(gptzero_key_row)),
    ("", "Check the key", "Asks GPTZero for the plan and the words left this billing cycle; uses no words.", Ctl::Custom(gptzero_check_row)),
    ("", "History", "", Ctl::Head),
    ("historyDays", "Keep the history for (days)", "Each AI check is recorded on this computer, outside the vault (Authenticity's local data folder), so a past run can be compared with the current one in the AI Detector pane (History). Older runs are dropped; the file is kept under 5 MB.", Ctl::Number),
    ("keepText", "Keep the checked text in the history", "Off: the history keeps each paragraph's hash and first words only. On: also the paragraphs' text (and the debug log may hold it too).", Ctl::Toggle),
    ("", "Logs", "", Ctl::Head),
    ("debugLog", "Debug log", "Write a detailed log of each check (request sizes, hashes, grouping, concurrency, the device, answers and their times, errors) to logs/authenticity-debug.log in Authenticity's local data folder, for finding a fault. Never a key; the text only if the history keeps it. Off by default.", Ctl::Toggle),
    ("", "Log", "", Ctl::Custom(crate::logview::row)),
    ("", "Colours", "", Ctl::Head),
    ("", "AI colour", "Paragraphs that read as written by AI, stronger the higher the score.", Ctl::Custom(ai_colour_row)),
    ("", "Human colour", "Paragraphs that read as written by a person, stronger the lower the score. Scores in between are orange.", Ctl::Custom(human_colour_row)),
    ("strength", "Strength", "The opacity of the strongest tint.", Ctl::Slider(0.1, 1.0, 0.05)),
    ("showMarks", "Show marks in notes", "The tints, underlines and character marks. Off hides them all and keeps every result; the command \"Show or hide Authenticity's marks\" does the same.", Ctl::Toggle),
    ("showAiMarks", "Show AI marks", "The paragraph tints, green to red. Also in the ribbon icon's menu.", Ctl::Toggle),
    ("showPlagMarks", "Show similarity marks", "The plagiarism check's underlines. Also in the ribbon icon's menu.", Ctl::Toggle),
    ("", "Characters", "", Ctl::Head),
    ("showHidden", "Mark unusual characters as you write", "Invisible characters (zero-width spaces and joiners, the byte-order mark, soft hyphens, direction marks), unusual spaces, dashes, look-alike punctuation and letters, and styled Unicode letters, in every note in the folders below. Off: only where you use Reveal AI characters.", Ctl::Toggle),
    ("showTypographic", "Mark typographic characters", "Curly quotes and apostrophes, the ellipsis character, bullets and arrows. Word produces them too, so they are a weak sign on their own.", Ctl::Toggle),
    ("replaceTypographic", "Replace typographic characters too", "\"Replace unusual characters\" (right-click a selection) replaces invisible characters, odd spaces and look-alike punctuation and letters. On: also curly quotes, en and em dashes, the ellipsis character, primes, the minus sign, bullets and arrows, which you may want to keep. (The AI characters list's step-through, which asks about one character at a time, always offers them.)", Ctl::Toggle),
    ("folders", "Only in these folders", "Comma-separated folders whose notes have their characters marked as you write; blank for every note.", Ctl::Text),
];

fn devices() -> Vec<(&'static str, &'static str)> {
    vec![("auto", "Automatic"), ("gpu", "Graphics card (GPU)"), ("cpu", "Processor (CPU)")]
}

#[wasm_bindgen]
pub struct Handle;

#[wasm_bindgen]
impl Handle {
    pub fn unload(&self) {
        let ws = with(|a| {
            let body = get(&get(&js::global(), "document"), "body");
            let style = get(&body, "style");
            for v in ["--auth-ai", "--auth-human", "--auth-strength"] {
                call!(style, removeProperty, v);
            }
            a.marks.clear();
            crate::render::remove_style();
            if let Some(t) = a.poll_timer.take() {
                call!(js::global(), clearInterval, t);
            }
            a.helper.release();
            a.index.cancel();
            crate::api::withdraw(a);
            get(&a.app, "workspace")
        });
        if let Some(ws) = ws {
            for view in crate::panel::VIEWS {
                call!(ws, detachLeavesOfType, view);
            }
        }
        APP.with(|a| a.borrow_mut().take());
    }
}

/// Called by main.js: the plugin object, the `obsidian` module, `require`, the compiled module and its bindings.
#[wasm_bindgen]
pub fn start(plugin: JsValue, obs: JsValue, require: JsValue, _module: JsValue, _glue: String) -> Handle {
    let req = |m: &str| js::apply(&require, &JsValue::NULL, &[m.into()]);
    let (cmv, cms) = (req("@codemirror/view"), req("@codemirror/state"));
    let local = call!(get(&plugin, "app"), loadLocalStorage, LOCAL_KEY);
    let device = serde_wasm_bindgen::from_value::<Device>(local).unwrap_or_default();
    let app = App {
        app: get(&plugin, "app"),
        dir: str(&get(&get(&plugin, "manifest"), "dir")),
        marks: view_marks::Marks::new::<App>(obs.clone(), cmv, cms, crate::render::config()),
        render: Default::default(),
        settings: Settings::default(),
        device,
        ai: Default::default(),
        found: HashMap::new(),
        matches: HashMap::new(),
        hits: HashMap::new(),
        reveal: HashMap::new(),
        run: 0,
        status: JsValue::UNDEFINED,
        panes: Default::default(),
        note: String::new(),
        activity: None,
        problems: HashMap::new(),
        para_note: HashMap::new(),
        para_cache: Default::default(),
        focus: None,
        expanded: Default::default(),
        gz_patterns: Default::default(),
        history_open: false,
        history_pick: None,
        session_engine: None,
        helper: Default::default(),
        index: Default::default(),
        persist: None,
        status_timer: None,
        poll_timer: None,
        require,
        plugin,
        obs,
    };
    let (p, o) = (app.plugin.clone(), app.obs.clone());
    APP.with(|a| *a.borrow_mut() = Some(app));
    // The panel's view, the editor extension and the post-processor are registered now, while main.js's onload is
    // still running: Obsidian restores the leaves and editors of a plugin only for what it registers during onload.
    // Registering makes Obsidian create them at once, and they call back into `with`, so the app is not borrowed
    // while they are registered (the extension is made inside, registered outside). Editors made before the results
    // have loaded are tagged again once they have (render::refresh).
    // the two panes, and the combined panel of 0.2 (a leaf of it saved in the workspace becomes the AI Detector pane)
    for pane in [crate::panel::Pane::Ai, crate::panel::Pane::Plagiarism] {
        let o = o.clone();
        call!(p, registerView, pane.view(), f1(move |leaf| crate::panel::create(&o, leaf, pane)));
    }
    let o2 = o.clone();
    call!(p, registerView, crate::panel::OLD_VIEW, f1(move |leaf| crate::panel::create_old(&o2, leaf)));
    if let Some(ext) = with(|a| view_marks::editor::extension(a)) {
        call!(p, registerEditorExtension, ext);
    }
    call!(p, registerMarkdownPostProcessor, f2(|el, ctx| {
        with(|a| view_marks::reading::process(a, el, ctx));
        JsValue::UNDEFINED
    }));
    crate::render::register_command(&p);
    js::spawn(async {
        let Some((plugin, adapter, dir)) = with(|a| (a.plugin.clone(), get(&get(&a.app, "vault"), "adapter"), a.dir.clone())) else { return };
        let data = js::wait(call!(plugin, loadData)).await.unwrap_or(JsValue::NULL);
        let settings = serde_wasm_bindgen::from_value::<Settings>(data).unwrap_or_default();
        let read = |name: &str| {
            let (adapter, file) = (adapter.clone(), format!("{dir}/{name}"));
            async move {
                match js::wait(call!(adapter, exists, file.clone())).await {
                    Ok(v) if v.is_truthy() => js::wait(call!(adapter, read, file)).await.ok().map(|t| str(&t)),
                    _ => None,
                }
            }
        };
        let ai = read("ai.json").await.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        // what the checks found; from 0.2's matches.json (what passed its thresholds then) when there is no found.json
        let found: Option<HashMap<String, crate::similarity::Checked>> = read("found.json").await.and_then(|t| serde_json::from_str(&t).ok());
        let found = match found {
            Some(f) => f,
            None => {
                let old: HashMap<String, Vec<Match>> = read("matches.json").await.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                let t = settings.thresholds();
                old.into_iter().map(|(k, v)| (k, crate::similarity::Checked::from_matches(v, t))).collect()
            }
        };
        // what the embedding search said about each sentence asked about, so an unchanged one is not sent again
        let para_cache = read("paraphrase.json").await.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        with(|a| {
            a.settings = settings;
            a.ai = ai;
            a.para_cache = para_cache;
            a.found = found;
            a.refilter(None);
            a.apply_style();
        });
        with(|a| {
            a.register();
            a.changed(None);
        });
        // ready: the API for other plugins, and the status file other local tools can read
        crate::api::publish();
    });
    Handle
}

impl App {
    /// The settings tab, status bar, ribbon icon, commands and events (none of which call back while being added).
    fn register(&mut self) {
        let p = self.plugin.clone();
        let tab = new(&get(&self.obs, "PluginSettingTab"), &[self.app.clone(), p.clone()]);
        let t = tab.clone();
        set(&tab, "display", f0(move || {
            let Some(obs) = with(|a| a.obs.clone()) else { return };
            settings::render_with(&t, &obs, UI, |k| with(|a| a.field(k)).unwrap_or(JsValue::UNDEFINED), |k, v| _ = with(|a| a.set_field(k, v)), |k| with(|a| a.row_state(k)).flatten());
        }));
        call!(p, addSettingTab, tab);
        self.status = call!(p, addStatusBarItem);
        call!(get(&self.status, "classList"), add, "auth-status", "mod-clickable");
        // the status bar and the ribbon icon open the same menu
        call!(p, registerDomEvent, self.status.clone(), "click", f1(|e| {
            with(|a| a.menu(&e));
            JsValue::UNDEFINED
        }));
        call!(p, addRibbonIcon, "shield-check", "Authenticity", f1(|e| {
            with(|a| a.menu(&e));
            JsValue::UNDEFINED
        }));
        // whether the server still runs (the helper stops SlopTotal when idle): asked every 20 s while it is believed to
        let id = call!(js::global(), setInterval, f0(|| {
            if with(|a| a.helper.last.is_some()).unwrap_or(false) {
                js::spawn(async {
                    crate::helper::refresh().await;
                    with(|a| a.show_status());
                });
            }
        }), 20_000);
        self.poll_timer = id.as_f64();
        let command = |id: &str, name: &str, f: JsValue| _ = call!(p, addCommand, obj! { id: id, name: name, callback: f });
        command("check-ai-note", "Check this note for AI writing", f0(|| js::spawn(check_ai(false))));
        command("check-ai-selection", "Check the selection for AI writing", f0(|| js::spawn(check_ai(true))));
        command("check-plagiarism-note", "Check this note for plagiarism", f0(|| js::spawn(check_plagiarism(false))));
        command("check-plagiarism-selection", "Check the selection for plagiarism", f0(|| js::spawn(check_plagiarism(true))));
        command("reveal-characters-note", "Reveal AI characters in this note", f0(|| reveal(false)));
        command("reveal-characters-selection", "Reveal AI characters in the selection", f0(|| reveal(true)));
        command("clear-results", "Clear this note's results", f0(|| _ = with(|a| {
            let note = a.note.clone();
            let ai = a.ai.clear(&note);
            let m = a.found.remove(&note).is_some() | a.matches.remove(&note).is_some();
            a.reveal.remove(&note);
            if ai {
                a.save_ai();
            }
            if m {
                a.save_matches();
            }
            a.changed(Some(note));
        })));
        // "open-panel" keeps 0.2's id, so a hotkey given to it still works
        command("open-panel", "Open the AI Detector pane", f0(|| crate::panel::open(crate::panel::Pane::Ai)));
        command("open-plagiarism-pane", "Open the Plagiarism pane", f0(|| crate::panel::open(crate::panel::Pane::Plagiarism)));
        command("toggle-marks", "Show or hide Authenticity's marks", f0(toggle_marks));
        command("stop-sloptotal", "Stop SlopTotal now", f0(|| js::spawn(crate::helper::stop_sloptotal())));
        command("rebuild-index", "Rebuild the library index from every library note", f0(|| {
            with(|a| {
                a.para_cache.clear(); // the embedding search is asked again too
                a.save_para_cache();
            });
            js::spawn(crate::index::rebuild())
        }));
        call!(p, addCommand, obj! { id: "remove-ai-characters", name: "Replace unusual characters in the selection", editorCallback: f2(|editor, _view| {
            js::spawn(replace_characters(editor));
            JsValue::UNDEFINED
        }) });
        let ws = get(&self.app, "workspace");
        call!(p, registerEvent, call!(ws, on, "editor-menu", f3(editor_menu)));
        call!(p, registerEvent, call!(ws, on, "active-leaf-change", f1(|leaf| {
            let view = get(&leaf, "view");
            if str(&call!(view, getViewType)) == "markdown" {
                follow(&get(&view, "file"));
            }
            JsValue::UNDEFINED
        })));
        call!(p, registerEvent, call!(ws, on, "file-open", f1(|file| {
            follow(&file);
            JsValue::UNDEFINED
        })));
        let vault = get(&self.app, "vault");
        call!(p, registerEvent, call!(vault, on, "rename", f2(|file, old| {
            let (from, to) = (str(&old), str(&get(&file, "path")));
            with(|a| {
                a.ai.rename(&from, &to);
                if let Some(f) = a.found.remove(&from) {
                    a.found.insert(to.clone(), f);
                }
                if let Some(m) = a.matches.remove(&from) {
                    a.matches.insert(to.clone(), m);
                    a.save_matches();
                }
                a.save_ai();
                if a.note == from {
                    a.note = to.clone();
                }
            });
            crate::index::changed(&to, Some(&from));
            JsValue::UNDEFINED
        })));
        call!(p, registerEvent, call!(vault, on, "delete", f1(|file| {
            let path = str(&get(&file, "path"));
            with(|a| {
                if a.ai.clear(&path) {
                    a.save_ai();
                }
                if a.found.remove(&path).is_some() | a.matches.remove(&path).is_some() {
                    a.save_matches();
                }
            });
            crate::index::changed(&path, None);
            JsValue::UNDEFINED
        })));
        for event in ["modify", "create"] {
            call!(p, registerEvent, call!(vault, on, event, f1(|file| {
                let path = str(&get(&file, "path"));
                crate::index::changed(&path, None);
                crate::render::file_changed(&path);
                JsValue::UNDEFINED
            })));
        }
        let active = call!(ws, getActiveFile);
        if !active.is_null() && !active.is_undefined() {
            self.note = str(&get(&active, "path"));
        }
        self.show_status();
        crate::panel::render(self);
    }

    // ----- settings

    pub fn field(&self, key: &str) -> JsValue {
        match key.strip_prefix("device.") {
            Some(k) => get(&serde_wasm_bindgen::to_value(&self.device).unwrap_or_default(), k),
            None => get(&serde_wasm_bindgen::to_value(&self.settings).unwrap_or_default(), key),
        }
    }

    pub fn set_field(&mut self, key: &str, v: JsValue) {
        if let Some(k) = key.strip_prefix("device.") {
            let o = serde_wasm_bindgen::to_value(&self.device).unwrap_or_default();
            set(&o, k, v);
            if let Ok(d) = serde_wasm_bindgen::from_value::<Device>(o) {
                self.device = d;
            }
            call!(self.app, saveLocalStorage, LOCAL_KEY, serde_wasm_bindgen::to_value(&self.device).unwrap_or_default());
            // a helper started with the old folder, device or ports is let go; the next check starts one with these
            self.helper.release();
            js::timeout(0.0, crate::api::refresh); // the features offered may have changed
            return;
        }
        let value: serde_json::Value = serde_wasm_bindgen::from_value(v).unwrap_or(serde_json::Value::Null);
        if let Some(s) = self.settings.with_field(key, value) {
            self.settings = s;
        }
        // the thresholds re-filter what the checks found, at once (no new check)
        if matches!(key, MIN_RUN_KEY | CLOSENESS_KEY) {
            self.refilter(None);
            self.save_matches();
        }
        if let Some(h) = self.persist.take() {
            js::clear_timeout(h);
        }
        // data.json after a pause, so typing in a text field writes once
        self.persist = Some(js::timeout(500.0, || {
            with(|a| {
                a.persist = None;
                call!(a.plugin, saveData, serde_wasm_bindgen::to_value(&a.settings).unwrap_or_default());
            });
        }));
        if matches!(key, "libraryFolder" | "exclude") {
            self.index.scope_changed();
        }
        self.apply_style();
        self.changed(None);
    }

    /// Settings that depend on what this computer has: greyed out, with the reason.
    fn row_state(&self, key: &str) -> Option<(bool, String)> {
        match key {
            _ => None,
        }
    }

    /// The colours and strength as CSS variables on the body (styles.css reads them).
    fn apply_style(&self) {
        let body = get(&get(&js::global(), "document"), "body");
        let style = get(&body, "style");
        call!(style, setProperty, "--auth-ai", self.settings.ai_colour.clone());
        call!(style, setProperty, "--auth-human", self.settings.human_colour.clone());
        call!(style, setProperty, "--auth-strength", self.settings.strength.to_string());
        crate::render::apply_style(self);
    }

    /// Something shown has changed (for one note, or all): editors and Reading view are drawn again after this call
    /// returns (render.rs), the status bar and the panel now.
    pub fn changed(&mut self, path: Option<String>) {
        crate::render::refresh(path);
        self.show_status();
        crate::panel::render(self);
    }

    /// Whether a note's characters are marked as it is drawn (the folders setting).
    pub fn in_scope(&self, path: &str) -> bool {
        let folders: Vec<&str> = self.settings.folders.split(',').map(|f| f.trim().trim_matches('/')).filter(|f| !f.is_empty()).collect();
        folders.is_empty() || folders.iter().any(|f| path.starts_with(&format!("{f}/")))
    }

    fn write(&self, name: &str, text: String) {
        let adapter = get(&get(&self.app, "vault"), "adapter");
        call!(adapter, write, format!("{}/{name}", self.dir), text);
    }

    pub fn save_ai(&mut self) {
        self.ai.trim();
        self.write("ai.json", serde_json::to_string(&self.ai).unwrap_or_default());
    }

    /// What the checks found (found.json) and what passes the thresholds (matches.json).
    pub fn save_matches(&self) {
        self.write("found.json", serde_json::to_string(&self.found).unwrap_or_default());
        self.write("matches.json", serde_json::to_string(&self.matches).unwrap_or_default());
    }

    pub fn thresholds(&self) -> crate::similarity::Thresholds {
        self.settings.thresholds()
    }

    /// What the embedding search said about each sentence (paraphrase.json).
    pub fn save_para_cache(&self) {
        self.write("paraphrase.json", serde_json::to_string(&self.para_cache).unwrap_or_default());
    }

    /// The matches shown (marks, panes, status, matches.json) made again from what was found, at the thresholds now:
    /// one note's or all.
    pub fn refilter(&mut self, path: Option<&str>) {
        let t = self.thresholds();
        match path {
            Some(p) => match self.found.get(p) {
                Some(c) => _ = self.matches.insert(p.to_string(), c.matches(t)),
                None => _ = self.matches.remove(p),
            },
            None => self.matches = self.found.iter().map(|(k, c)| (k.clone(), c.matches(t))).collect(),
        }
    }

    /// A check of `path` failed: say why in a notice and in the panel, until the next one.
    pub fn fail(&mut self, path: &str, why: &str) {
        if why == crate::sloptotal::CANCELLED {
            crate::logview::event(&format!("{path}: check cancelled"));
            return; // Cancel said so
        }
        crate::logview::event(&format!("{path}: check failed"));
        self.notice(why);
        self.problems.insert(path.to_string(), why.to_string());
        self.show_status();
        crate::panel::render(self);
    }

    /// Whether the check that took `run` when it started is still wanted (not cancelled since).
    pub fn alive(&self, run: u64) -> bool {
        self.run == run
    }

    pub fn busy(&mut self, what: Option<String>) {
        if let Some(w) = &what {
            crate::logview::event(w);
        }
        self.activity = what;
        self.show_status();
        crate::panel::render(self);
    }

    /// The paragraphs of a note with an AI score, in order: (paragraph, score).
    pub fn scored(&self, path: &str, text: &str) -> Vec<(crate::ai::Paragraph, crate::ai::Scored)> {
        if !self.ai.notes.contains_key(path) {
            return Vec::new();
        }
        self.ai.scored_in(path, crate::ai::paragraphs(&crate::prose::extract(text)))
    }

    /// SlopTotal's state as the helper last said: "running", "starting" or "stopped".
    pub fn server_state(&self) -> &'static str {
        match self.helper.last.as_ref().map(|h| h.state.as_str()) {
            Some("running") => "running",
            Some("starting") => "starting",
            _ => "stopped",
        }
    }

    /// The status bar: whether Authenticity's server (the helper and SlopTotal) runs, as a dot (green running, orange
    /// starting, grey stopped) and the name, as other plugins' status items show theirs; the tooltip says
    /// more, and a click opens the menu.
    pub fn show_status(&self) {
        if self.status.is_undefined() {
            return;
        }
        let state = self.server_state();
        // the device in use beside the name while SlopTotal runs, as the other plugins' items show theirs
        let device = self.helper.last.as_ref().filter(|_| state == "running").map(|h| h.device.as_str()).unwrap_or("");
        call!(self.status, setText, match device {
            "cuda" => "Authenticity \u{b7} GPU",
            "cpu" => "Authenticity \u{b7} CPU",
            _ => "Authenticity",
        });
        let cls = get(&self.status, "classList");
        call!(cls, toggle, "is-running", state == "running");
        call!(cls, toggle, "is-busy", state == "starting");
        let mut tip = vec![format!("Authenticity's server (SlopTotal): {state}")];
        if let Some(h) = self.helper.last.as_ref().filter(|_| state == "running") {
            if !h.device.is_empty() {
                tip.push(format!("On the {}", if h.device == "cuda" { "graphics card (CUDA)" } else { "processor" }));
            }
        }
        if let Some(a) = &self.activity {
            tip.push(a.clone());
        }
        tip.push("Click for the menu".into());
        call!(self.status, setAttribute, "aria-label", tip.join("\n"));
    }

    /// The menu of the ribbon icon and the status bar (as Augmented View's): the panes, the two kinds of marks, the
    /// server, and the strength of the AI tints and the close-paraphrase closeness (the same settings as the panes and
    /// the settings tab). <https://docs.obsidian.md/Reference/TypeScript+API/Menu>
    pub fn menu(&self, e: &JsValue) {
        let m = new(&get(&self.obs, "Menu"), &[]);
        let item = |title: String, checked: Option<bool>, icon: Option<&'static str>, f: Box<dyn FnOnce()>| {
            let mut f = Some(f);
            call!(m, addItem, f1(move |i| {
                call!(i, setTitle, title.clone());
                if let Some(c) = checked {
                    call!(i, setChecked, c);
                }
                if let Some(icon) = icon {
                    call!(i, setIcon, icon);
                }
                let mut g = f.take();
                call!(i, onClick, f0(move || if let Some(g) = g.take() { g() }))
            }));
        };
        let heading = |title: &'static str| _ = call!(m, addItem, f1(move |i| call!(call!(i, setTitle, title), setDisabled, true)));
        use crate::panel::Pane;
        if self.activity.is_some() {
            item("Cancel the check".into(), None, Some("x"), Box::new(cancel_check));
            call!(m, addSeparator);
        }
        item(Pane::Ai.title().into(), None, Some(Pane::Ai.icon()), Box::new(|| crate::panel::open(Pane::Ai)));
        item(Pane::Plagiarism.title().into(), None, Some(Pane::Plagiarism.icon()), Box::new(|| crate::panel::open(Pane::Plagiarism)));
        call!(m, addSeparator);
        item("Show AI marks".into(), Some(self.settings.show_ai_marks), None, Box::new(|| flip("showAiMarks")));
        item("Show similarity marks".into(), Some(self.settings.show_plag_marks), None, Box::new(|| flip("showPlagMarks")));
        call!(m, addSeparator);
        let state = self.server_state();
        if state == "stopped" {
            item("Start the server".into(), None, Some("play"), Box::new(|| js::spawn(start_server())));
        } else {
            item("Stop the server".into(), None, Some("square"), Box::new(|| js::spawn(stop_server())));
        }
        call!(m, addSeparator);
        heading("Strength of the AI marks");
        for level in [0.2, 0.4, 0.6, 0.8, 1.0] {
            let on = (self.settings.strength - level).abs() < 0.01;
            item(format!("{:.0}%", level * 100.0), Some(on), None, Box::new(move || _ = with(|a| a.set_field("strength", level.into()))));
        }
        call!(m, addSeparator);
        heading("Close paraphrase from");
        let now = self.thresholds().closeness;
        for level in [0.3, 0.4, 0.5, 0.6, 0.7] {
            let on = (now - level).abs() < 0.01;
            item(format!("{:.0}%", level * 100.0), Some(on), None, Box::new(move || _ = with(|a| a.set_field(CLOSENESS_KEY, level.into()))));
        }
        call!(m, showAtMouseEvent, e.clone());
    }

    pub fn notice(&self, text: &str) {
        crate::logview::event(text);
        new(&get(&self.obs, "Notice"), &[format!("Authenticity: {text}").into(), 8000.into()]);
    }

    /// The Markdown leaf showing `path`, if one is open.
    pub fn markdown_leaf(&self, path: &str) -> Option<JsValue> {
        let leaves = js_sys::Array::from(&call!(get(&self.app, "workspace"), getLeavesOfType, "markdown"));
        leaves.iter().find(|l| str(&get(&get(&get(l, "view"), "file"), "path")) == path)
    }

    /// The editor to check: the active Markdown view's, else the followed note's.
    pub fn target(&self) -> Option<(String, JsValue)> {
        let view = call!(get(&self.app, "workspace"), getActiveViewOfType, get(&self.obs, "MarkdownView"));
        let view = if view.is_null() || view.is_undefined() { get(&self.markdown_leaf(&self.note)?, "view") } else { view };
        let path = str(&get(&get(&view, "file"), "path"));
        (!path.is_empty()).then(|| (path, get(&view, "editor")))
    }
}

/// The AI-checked paragraphs (unchanged since) that meet `at` (a selection, or the cursor), as their places.
fn ai_at(a: &App, path: &str, text: &str, at: (u32, u32)) -> Vec<(u32, u32)> {
    if !a.ai.notes.contains_key(path) {
        return Vec::new();
    }
    a.scored(path, text).into_iter().map(|(p, _)| p).filter(|p| crate::table::meets((p.from, p.to), at)).map(|p| (p.from, p.to)).collect()
}

/// The flagged passages (verbatim, near-verbatim, close paraphrase; unchanged since: their words are still there) that
/// meet `at`, as their places now.
fn matches_at(a: &App, path: &str, text: &str, at: (u32, u32)) -> Vec<(u32, u32)> {
    a.matches.get(path).into_iter().flatten().filter(|m| m.kind != "common").filter_map(|m| crate::render::locate(text, m.from, m.to, &m.text)).filter(|r| crate::table::meets(*r, at)).collect()
}

/// Mark the rows of `ranges` in a pane, open it and scroll to them.
pub fn show_in(pane: crate::panel::Pane, path: &str, ranges: Vec<(u32, u32)>) {
    with(|a| a.focus = Some(crate::panel::Focus::new(pane, path, ranges, true)));
    crate::panel::open(pane);
    with(|a| crate::panel::render(a));
}

/// The editor's context menu: with text selected, the three checks of the selection (and Replace unusual
/// characters); over text that was checked and has not changed since, Show in AI check and Show in similarity check
/// (the latter only where something was flagged), each revealing its entry in its pane. The cursor alone counts as
/// the place for the Show entries. (<https://docs.obsidian.md/Reference/TypeScript+API/Workspace/on('editor-menu')>)
fn editor_menu(menu: JsValue, editor: JsValue, _view: JsValue) -> JsValue {
    let path = str(&get(&get(&_view, "file"), "path"));
    let selected = selection(&editor);
    let at = selected.unwrap_or_else(|| {
        let c = call!(editor, posToOffset, call!(editor, getCursor, "head")).as_f64().unwrap_or(0.0) as u32;
        (c, c)
    });
    let text = str(&call!(editor, getValue));
    let (ai, flagged) = with(|a| (ai_at(a, &path, &text, at), matches_at(a, &path, &text, at))).unwrap_or_default();
    let show = |title: &'static str, icon: &'static str, pane: crate::panel::Pane, ranges: Vec<(u32, u32)>| {
        let path = path.clone();
        call!(menu, addItem, f1(move |i| {
            let (path, ranges) = (path.clone(), ranges.clone());
            call!(call!(i, setTitle, title), setIcon, icon);
            call!(i, onClick, f0(move || show_in(pane, &path, ranges.clone())))
        }));
    };
    if !ai.is_empty() {
        show("Show in AI check", "shield-check", crate::panel::Pane::Ai, ai);
    }
    if !flagged.is_empty() {
        show("Show in similarity check", "copy-check", crate::panel::Pane::Plagiarism, flagged);
    }
    if selected.is_none() {
        return JsValue::UNDEFINED;
    }
    let item = |title: &'static str, icon: &'static str, run: fn()| {
        call!(menu, addItem, f1(move |i| {
            call!(call!(i, setTitle, title), setIcon, icon);
            call!(i, onClick, f0(run))
        }));
    };
    item("Check for plagiarism", "copy-check", || js::spawn(check_plagiarism(true)));
    item("Check for AI writing", "shield-check", || js::spawn(check_ai(true)));
    item("Reveal AI characters", "scan-eye", || reveal(true));
    let ed = editor.clone();
    call!(menu, addItem, f1(move |i| {
        let ed = ed.clone();
        call!(call!(i, setTitle, "Replace unusual characters\u{2026}"), setIcon, "replace");
        call!(i, onClick, f0(move || js::spawn(replace_characters(ed.clone()))))
    }));
    // copying with the marks, where the note has any
    if with(|a| a.scope_of(&path).is_some()).unwrap_or(false) {
        crate::render::menu_item(&menu, &editor, path);
    }
    JsValue::UNDEFINED
}

/// The selection's [from, to) in the editor (UTF-16), or None if nothing is selected.
fn selection(editor: &JsValue) -> Option<(u32, u32)> {
    let off = |which: &str| call!(editor, posToOffset, call!(editor, getCursor, which)).as_f64().unwrap_or(0.0) as u32;
    let (a, b) = (off("from"), off("to"));
    (a < b).then_some((a, b))
}

/// The text of [from, to) (UTF-16) of `text`, and the byte where it starts.
pub fn slice16(text: &str, from: u32, to: u32) -> (String, usize) {
    let (mut units, mut start, mut end) = (0u32, None, text.len());
    for (i, c) in text.char_indices() {
        if start.is_none() && units >= from {
            start = Some(i);
        }
        if units >= to {
            end = i;
            break;
        }
        units += c.len_utf16() as u32;
    }
    let s = start.unwrap_or(text.len());
    (text[s..end.max(s)].to_string(), s)
}

/// Replace the selection's unusual characters with their plain equivalents (clean.rs), after a dialog that says how
/// many of which kinds will change; one edit, so one undo puts them all back. Run only from its command or the
/// selection's menu, on the selection the user chose; the checks themselves never change the text.
async fn replace_characters(editor: JsValue) {
    let text = str(&call!(editor, getSelection));
    if text.is_empty() {
        with(|a| a.notice("select the text first."));
        return;
    }
    let typography = with(|a| a.settings.replace_typographic).unwrap_or(false);
    let (out, n) = crate::clean::clean(&text, crate::clean::Options { typography });
    if n.total() == 0 {
        with(|a| a.notice(if typography { "no unusual characters in the selection." } else { "no unusual characters in the selection (typographic ones are kept: Settings → Replace typographic characters too)." }));
        return;
    }
    let kept = if typography { "" } else { " Curly quotes, dashes, the ellipsis character and other typography are kept (Settings → Replace typographic characters too)." };
    let message = format!("{}: {}.{kept} Undo (Ctrl+Z) puts them back.", match n.total() {
        1 => "1 character will change".to_string(),
        t => format!("{t} characters will change"),
    }, n.describe());
    if !confirm("Replace unusual characters", &message, "Replace").await {
        return;
    }
    // the selection may have moved while the dialog was open: replace only the text that was checked
    if str(&call!(editor, getSelection)) != text {
        with(|a| a.notice("the selection changed; nothing was replaced. Select the text and try again."));
        return;
    }
    call!(editor, replaceSelection, out);
    with(|a| a.notice(&format!("replaced {} character{}.", n.total(), if n.total() == 1 { "" } else { "s" })));
}

/// What the step-through prompt was answered with.
#[derive(Clone, PartialEq, Eq)]
enum Answer {
    /// OK with the replacement the prompt offered.
    Offered,
    /// OK with the character typed in the box (no automatic replacement).
    Typed(String),
    Skip,
    Stop,
}

/// Go through the AI characters the AI Detector pane lists, from the first occurrence of entry `group` to the end of
/// the list (walk.rs): each is selected in the editor and a prompt asks whether to replace it, with what "Replace
/// unusual characters" would put there (typographic characters included, whatever its setting), or, with no
/// automatic replacement, a box for the character to put there. Enter is OK (one edit, so one undo), Esc skips it,
/// Stop ends. An occurrence no longer at its place (the note changed) is found again, or skipped with a notice.
pub async fn step_through(path: String, group: usize) {
    thread_local! {
        static WALKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    // one walk at a time (a click in the pane while a prompt is open would start a second)
    if WALKING.with(|w| w.replace(true)) {
        return;
    }
    walk_through(path, group).await;
    WALKING.with(|w| w.set(false));
}

async fn walk_through(path: String, group: usize) {
    let Some(Some(groups)) = with(|a| a.hits.get(&path).map(|h| crate::walk::groups(h))) else { return };
    let mut walk = crate::walk::Walk::new(&groups, group);
    let mut replaced = 0usize;
    while let Some(occ) = walk.current().cloned() {
        let Some(Some(editor)) = with(|a| a.markdown_leaf(&path).map(|l| get(&get(&l, "view"), "editor"))) else {
            with(|a| a.notice("the note is no longer open in an editor; stopped."));
            break;
        };
        let text = str(&call!(editor, getValue));
        match crate::walk::find(&text, &occ) {
            crate::walk::Found::Here => {}
            crate::walk::Found::Moved(from, to) => walk.moved(from, to),
            crate::walk::Found::Gone => {
                with(|a| a.notice(&format!("the {} (U+{:04X}) is no longer in the note; skipped.", occ.name, occ.code)));
                walk.skip();
                continue;
            }
        }
        let Some(occ) = walk.current().cloned() else { break };
        crate::render::jump(&path, occ.from, occ.to);
        let p = crate::walk::prompt(&text, &occ);
        let (from, to, insert) = match ask_step(&p, walk.left() - 1).await {
            Answer::Stop => break,
            Answer::Skip => {
                walk.skip();
                continue;
            }
            Answer::Offered => match &p.edit {
                Some(e) => (e.from, e.to, e.insert.clone()),
                None => {
                    walk.skip();
                    continue;
                }
            },
            Answer::Typed(c) => (occ.from, occ.to, c),
        };
        // the note may have changed while the prompt was open: then the occurrence is found again and asked about again
        if str(&call!(editor, getValue)) != text {
            continue;
        }
        let (a, b) = (call!(editor, offsetToPos, from), call!(editor, offsetToPos, to));
        call!(editor, replaceRange, insert.clone(), a, b);
        walk.replaced(from, to, &insert);
        replaced += 1;
        let p = path.clone();
        with(|a| a.changed(Some(p)));
        // the characters are found again as the editor is drawn: the pane after that
        js::timeout(50.0, || _ = with(|a| crate::panel::render(a)));
    }
    if replaced > 0 {
        with(|a| a.notice(&format!("replaced {replaced} character{}; Undo (Ctrl+Z) puts them back one at a time.", if replaced == 1 { "" } else { "s" })));
    }
}

/// The step-through prompt: "Replace this character?", what it is and what it would become, or, with no automatic
/// replacement, a box for one character (focused, so a key can be typed at once; a second key replaces the first).
/// Enter is OK (disabled while the box is empty), Esc is Skip, in the box as anywhere in the prompt; Stop ends the
/// walk. Drawn low and without the dimmed background, so the selected character stays in sight.
async fn ask_step(p: &crate::walk::Prompt, after: usize) -> Answer {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    let Some((obs, app)) = with(|a| (a.obs.clone(), a.app.clone())) else { return Answer::Stop };
    let modal = new(&get(&obs, "Modal"), &[app]);
    call!(get(&get(&modal, "containerEl"), "classList"), add, "auth-step");
    call!(get(&modal, "titleEl"), setText, "Replace this character?");
    let content = get(&modal, "contentEl");
    let doc = get(&js::global(), "document");
    let make = |tag: &str, text: &str, class: &str| {
        let e = call!(doc, createElement, tag);
        if !text.is_empty() {
            set(&e, "textContent", text);
        }
        if !class.is_empty() {
            set(&e, "className", class);
        }
        e
    };
    call!(content, appendChild, make("p", &p.line, "auth-step-line"));
    let typed = p.edit.is_none();
    let input = typed.then(|| {
        let i = make("input", "", "auth-step-input");
        set(&i, "type", "text");
        set(&i, "placeholder", "the character to put in its place");
        set(&i, "spellcheck", false);
        set(&i, "autocomplete", "off");
        call!(content, appendChild, i.clone());
        i
    });
    let more = match after {
        0 => "The last one.".to_string(),
        1 => "1 more after this one.".to_string(),
        n => format!("{n} more after this one."),
    };
    call!(content, appendChild, make("p", &more, "auth-muted"));
    let row = make("div", "", "modal-button-container");
    call!(content, appendChild, row.clone());
    let chosen = Rc::new(RefCell::new(Answer::Skip));
    let done = Rc::new(Cell::new(false));
    let ok_button = make("button", "OK (Enter)", "mod-cta");
    // OK: the offered replacement, or the character in the box (nothing while it is empty)
    let ok: Rc<dyn Fn()> = {
        let (m, c, d, i) = (modal.clone(), chosen.clone(), done.clone(), input.clone());
        Rc::new(move || {
            let answer = match &i {
                Some(i) => {
                    let v = crate::walk::last_grapheme(&str(&get(i, "value")));
                    if v.is_empty() {
                        return;
                    }
                    Answer::Typed(v)
                }
                None => Answer::Offered,
            };
            if !d.replace(true) {
                *c.borrow_mut() = answer;
                _ = call!(m, close);
            }
        })
    };
    let end = |answer: Answer| {
        let (m, c, d) = (modal.clone(), chosen.clone(), done.clone());
        move || {
            if !d.replace(true) {
                *c.borrow_mut() = answer.clone();
                _ = call!(m, close);
            }
        }
    };
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let o = ok.clone();
        call!(ok_button, addEventListener, "click", f1(move |_| {
            o();
            JsValue::UNDEFINED
        }));
        if typed {
            set(&ok_button, "disabled", true);
        }
        call!(row, appendChild, ok_button.clone());
        for (label, answer) in [("Skip (Esc)", Answer::Skip), ("Stop", Answer::Stop)] {
            let b = make("button", label, "");
            let f = end(answer);
            call!(b, addEventListener, "click", f1(move |_| {
                f();
                JsValue::UNDEFINED
            }));
            call!(row, appendChild, b);
        }
        // Enter is OK and Esc is Skip in the whole prompt (Obsidian's Esc would close it, which is also Skip)
        let o = ok.clone();
        call!(get(&modal, "scope"), register, js_sys::Array::new(), "Enter", f2(move |_, _| {
            o();
            JsValue::FALSE
        }));
        let skip = end(Answer::Skip);
        call!(get(&modal, "scope"), register, js_sys::Array::new(), "Escape", f2(move |_, _| {
            skip();
            JsValue::FALSE
        }));
        if let Some(i) = &input {
            // the box keeps one character, the last typed; OK is enabled while it holds one
            let (i2, b) = (i.clone(), ok_button.clone());
            call!(i, addEventListener, "input", f1(move |_| {
                let v = crate::walk::last_grapheme(&str(&get(&i2, "value")));
                if str(&get(&i2, "value")) != v {
                    set(&i2, "value", v.as_str());
                }
                set(&b, "disabled", v.is_empty());
                JsValue::UNDEFINED
            }));
            // the keys in the box itself, so a browser default never takes them first
            let (o, skip) = (ok.clone(), end(Answer::Skip));
            call!(i, addEventListener, "keydown", f1(move |e| {
                match str(&get(&e, "key")).as_str() {
                    "Enter" => {
                        call!(e, preventDefault);
                        call!(e, stopPropagation);
                        o();
                    }
                    "Escape" => {
                        call!(e, preventDefault);
                        call!(e, stopPropagation);
                        skip();
                    }
                    _ => {}
                }
                JsValue::UNDEFINED
            }));
        }
        set(&modal, "onClose", f0(move || {
            _ = resolve.call1(&JsValue::NULL, &JsValue::UNDEFINED);
        }));
    });
    call!(modal, open);
    if let Some(i) = input {
        call!(i, focus);
        js::timeout(0.0, move || _ = call!(i, focus));
    }
    _ = js::wait(promise.into()).await;
    let answer = chosen.borrow().clone();
    answer
}

/// Reveal the characters chars.rs finds in the selection or the note: marked in the editor and listed in the panel.
fn reveal(in_selection: bool) {
    let Some(Some((path, editor))) = with(|a| a.target()) else { return };
    let range = if in_selection {
        match selection(&editor) {
            Some(r) => Some(r),
            None => {
                with(|a| a.notice("select the text first."));
                return;
            }
        }
    } else {
        None
    };
    let text = str(&call!(editor, getValue));
    let (scope, base) = match range {
        Some((a, b)) => {
            let (s, _) = slice16(&text, a, b);
            (s, a)
        }
        None => (text, 0),
    };
    let typo = with(|a| a.settings.show_typographic).unwrap_or(true);
    let hits = crate::chars::scan(&scope, typo);
    with(|a| {
        a.reveal.insert(path.clone(), range);
        let what = if range.is_some() { "the selection" } else { "this note" };
        a.notice(&match hits.len() {
            0 => format!("no unusual characters in {what}."),
            1 => format!("1 unusual character in {what}, marked."),
            n => format!("{n} unusual characters in {what}, marked; the AI Detector pane lists them."),
        });
        let _ = base;
        a.changed(Some(path));
    });
    crate::panel::open(crate::panel::Pane::Ai);
}

/// Flip a setting that is on or off (the menu's toggles).
fn flip(key: &'static str) {
    with(|a| {
        let on = a.field(key).as_bool().unwrap_or(true);
        a.set_field(key, (!on).into());
    });
}

/// Start the server from the menu: the helper and SlopTotal, as a check starts them.
async fn start_server() {
    with(|a| a.show_status());
    let r = crate::helper::ensure_sloptotal("starting the server").await;
    with(|a| {
        a.busy(None);
        a.notice(&match r {
            Ok(()) => "SlopTotal is running; it stops after the idle time set in Settings.".to_string(),
            Err(e) => format!("the server did not start: {e}"),
        });
    });
}

/// Stop the server from the menu.
async fn stop_server() {
    crate::helper::stop_sloptotal().await;
    with(|a| a.show_status());
}

/// Show or hide all the plugin's marks in notes (a class on the body that styles.css reads; nothing is redrawn).
pub fn toggle_marks() {
    with(|a| {
        let on = !a.settings.show_marks;
        a.set_field("showMarks", on.into());
        a.notice(if on { "marks shown." } else { "marks hidden; results are kept." });
    });
}

/// The Markdown note now active, for the panel and the status bar.
fn follow(file: &JsValue) {
    if file.is_null() || file.is_undefined() || str(&get(file, "extension")) != "md" {
        return;
    }
    with(|a| {
        let path = str(&get(file, "path"));
        if a.note != path {
            a.note = path;
            a.show_status();
            crate::panel::render(a);
        }
    });
}

// ----- the settings rows the plugin fills itself

fn colour_row(s: &JsValue, key: &'static str) {
    let value = with(|a| a.field(key)).unwrap_or_default();
    call!(s, addColorPicker, f1(move |p| {
        call!(p, setValue, value.clone());
        call!(p, onChange, f1(move |v| {
            with(|a| a.set_field(key, v));
            JsValue::UNDEFINED
        }))
    }));
}

fn ai_colour_row(s: &JsValue) {
    colour_row(s, "aiColour");
}

fn human_colour_row(s: &JsValue) {
    colour_row(s, "humanColour");
}

/// What the helper and SlopTotal are doing, with a button to look again.
fn status_row(s: &JsValue) {
    let text = with(|a| a.helper.describe(&a.device)).unwrap_or_default();
    call!(s, setDesc, text);
    let s2 = s.clone();
    call!(s, addButton, f1(move |b| {
        let s3 = s2.clone();
        call!(b, setButtonText, "Check");
        call!(b, onClick, f0(move || {
            let s4 = s3.clone();
            js::spawn(async move {
                crate::helper::refresh().await;
                let text = with(|a| a.helper.describe(&a.device)).unwrap_or_default();
                call!(s4, setDesc, text);
            });
        }))
    }));
}

/// The optional embedding search's row: offered only when the Quarry plugin is installed; a toggle and a Check button
/// that says, in the row, whether the search can be used now.
fn embeddings_row(s: &JsValue) {
    let Some((installed, on)) = with(|a| (crate::api::embeddings_installed(a), a.settings.paraphrase_embeddings)) else { return };
    if !installed {
        call!(s, setDesc, "Offered when the Quarry plugin is installed. Authenticity's own search is used.");
        call!(s, setDisabled, true);
        return;
    }
    call!(s, setDesc, "If the Quarry plugin is installed, Authenticity can use its embedding model for the paraphrase check instead of its own search (which needs no model). Its service must be running; Authenticity never starts it, and uses its own search when it does not answer. Check tests it now.");
    call!(s, addToggle, f1(move |t| {
        call!(call!(t, setValue, on), onChange, f1(|v| {
            with(|a| a.set_field("paraphraseEmbeddings", v));
            JsValue::UNDEFINED
        }))
    }));
    let s2 = s.clone();
    call!(s, addButton, f1(move |b| {
        let s3 = s2.clone();
        call!(b, setButtonText, "Check");
        call!(b, onClick, f0(move || {
            let s4 = s3.clone();
            call!(s4, setDesc, "Checking\u{2026}");
            js::spawn(async move {
                let text = crate::api::check_embeddings().await;
                call!(s4, setDesc, text);
            });
        }))
    }));
}

/// The keychain row for the GPTZero key (Obsidian's SecretComponent stores the secret; the setting holds its name).
fn gptzero_key_row(s: &JsValue) {
    let Some((obs, app, name)) = with(|a| (a.obs.clone(), a.app.clone(), a.settings.gptzero_secret.clone())) else { return };
    let component = get(&obs, "SecretComponent");
    if !component.is_function() {
        call!(s, setDesc, "Needs Obsidian 1.11.4 or later, whose keychain keeps the key.");
        return;
    }
    let c = new(&component, &[app, get(s, "controlEl")]);
    call!(c, setValue, name);
    call!(c, onChange, f1(move |v| {
        with(|a| {
            a.set_field("gptzeroSecret", if v.is_null() { "".into() } else { v });
            a.session_engine = None;
        });
        JsValue::UNDEFINED
    }));
}

fn gptzero_check_row(s: &JsValue) {
    let s2 = s.clone();
    call!(s, addButton, f1(move |b| {
        let s3 = s2.clone();
        call!(b, setButtonText, "Check");
        call!(b, onClick, f0(move || {
            let s4 = s3.clone();
            js::spawn(async move {
                let text = match gptzero_key().await {
                    Some(key) => crate::gptzero::check_key(&GptZeroNet { run: None }, &key).await,
                    None => "No GPTZero key is linked (or its keychain entry is empty).".into(),
                };
                call!(s4, setDesc, text);
            });
        }))
    }));
}

/// The GPTZero key from Obsidian's keychain, by the name the setting holds; None if none is linked or it is empty.
/// It is passed to the request and never kept, logged or shown.
pub(crate) async fn gptzero_key() -> Option<String> {
    let (secrets, name) = with(|a| (get(&a.app, "secretStorage"), a.settings.gptzero_secret.clone()))?;
    if name.trim().is_empty() || secrets.is_undefined() {
        return None;
    }
    let value = str(&js::wait(call!(secrets, getSecret, name)).await.unwrap_or(JsValue::NULL));
    Some(value.trim().to_string()).filter(|v| !v.is_empty())
}

/// Requests to GPTZero through Obsidian's `requestUrl` (no cross-origin limits); a check's run number, so Cancel stops
/// the requests not yet sent.
struct GptZeroNet {
    run: Option<u64>,
}

async fn gptzero_request(method: &str, url: &str, key: &str, body: Option<String>) -> (u16, String) {
    let Some(request_url) = with(|a| get(&a.obs, "requestUrl")) else { return (0, String::new()) };
    let headers = obj! { Accept: "application/json" };
    set(&headers, "x-api-key", key);
    let req = obj! { url: url, method: method, headers: headers, throw: false };
    if let Some(b) = body {
        set(&req, "contentType", "application/json");
        set(&req, "body", b);
    }
    match js::wait(js::apply(&request_url, &JsValue::NULL, &[req])).await {
        Ok(res) => (get(&res, "status").as_f64().unwrap_or(0.0) as u16, str(&get(&res, "text"))),
        Err(_) => (0, String::new()),
    }
}

impl crate::gptzero::Net for GptZeroNet {
    async fn post(&self, url: &str, key: &str, body: String) -> (u16, String) {
        let (bytes, start) = (body.len(), js::now());
        let r = gptzero_request("POST", url, key, Some(body)).await;
        with(|a| crate::journal::debug(a, "gptzero-request", crate::gptzero::request_log(bytes, r.0, js::now() - start), None));
        r
    }
    async fn get(&self, url: &str, key: &str) -> (u16, String) {
        gptzero_request("GET", url, key, None).await
    }
    fn cancelled(&self) -> bool {
        self.run.is_some_and(cancelled)
    }
}

/// GPTZero's AI Patterns for a paragraph it scored, when the user asks (the pane's AI Patterns button): the paragraph is
/// sent again, with the key; the answer, or what went wrong, is kept for the pane.
pub async fn gptzero_patterns(paragraph: String) {
    let Some(key) = gptzero_key().await else {
        with(|a| a.notice("link a GPTZero API key first (Settings \u{2192} Authenticity \u{2192} GPTZero)."));
        return;
    };
    with(|a| a.busy(Some("GPTZero: asking for its AI patterns\u{2026}".into())));
    let r = crate::gptzero::patterns(&GptZeroNet { run: None }, &key, &paragraph).await;
    with(|a| {
        a.busy(None);
        crate::journal::debug(a, "gptzero-patterns", serde_json::json!({ "ok": r.is_ok(), "found": r.as_ref().map(|v| v.len()).unwrap_or(0) }), None);
        a.gz_patterns.insert(crate::ai::key(&paragraph), r);
        crate::panel::render(a);
    });
}

/// The question before a check when a GPTZero key is linked: GPTZero, SlopTotal or Cancel, with "Remember for this
/// session". The engine chosen and whether to remember it; None when cancelled (nothing is sent).
async fn ask_engine(text: &str) -> Option<(String, bool)> {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    let (obs, app) = with(|a| (a.obs.clone(), a.app.clone()))?;
    let modal = new(&get(&obs, "Modal"), &[app]);
    call!(get(&modal, "titleEl"), setText, "Check with GPTZero or SlopTotal?");
    let content = get(&modal, "contentEl");
    let doc = get(&js::global(), "document");
    let p = call!(doc, createElement, "p");
    set(&p, "textContent", text);
    call!(content, appendChild, p);
    let label = call!(doc, createElement, "label");
    let tick = call!(doc, createElement, "input");
    set(&tick, "type", "checkbox");
    call!(label, appendChild, tick.clone());
    call!(label, appendChild, call!(doc, createTextNode, " Remember for this session"));
    call!(content, appendChild, label);
    let row = call!(doc, createElement, "div");
    set(&row, "className", "modal-button-container");
    call!(content, appendChild, row.clone());
    let chosen: Rc<RefCell<Option<String>>> = Rc::default();
    let remember = Rc::new(Cell::new(false));
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        for (label, engine, cta) in [("GPTZero", Some(crate::gptzero::ENGINE), false), ("SlopTotal (offline)", Some("sloptotal"), true), ("Cancel", None, false)] {
            let b = call!(doc, createElement, "button");
            set(&b, "textContent", label);
            if cta {
                set(&b, "className", "mod-cta");
            }
            let (m, c, r, t) = (modal.clone(), chosen.clone(), remember.clone(), tick.clone());
            call!(b, addEventListener, "click", f1(move |_| {
                *c.borrow_mut() = engine.map(str::to_string);
                r.set(get(&t, "checked").as_bool().unwrap_or(false));
                _ = call!(m, close);
                JsValue::UNDEFINED
            }));
            call!(row, appendChild, b);
        }
        set(&modal, "onClose", f0(move || {
            _ = resolve.call1(&JsValue::NULL, &JsValue::UNDEFINED);
        }));
    });
    call!(modal, open);
    _ = js::wait(promise.into()).await;
    let engine = chosen.borrow().clone()?;
    Some((engine, remember.get()))
}

fn index_row(s: &JsValue) {
    let text = with(|a| a.index.describe()).unwrap_or_default();
    call!(s, setDesc, text);
    call!(s, addButton, f1(|b| {
        call!(b, setButtonText, "Rebuild");
        call!(b, onClick, f0(|| js::spawn(crate::index::rebuild())))
    }));
}

/// A dialog with Cancel and `ok`; true if `ok` was clicked.
pub async fn confirm(title: &str, text: &str, ok: &str) -> bool {
    let Some((obs, app)) = with(|a| (a.obs.clone(), a.app.clone())) else { return false };
    js::ask(&obs, &app, title, text, ok, "Cancel").await
}

/// 12345 → "12,345".
pub fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ----- the checks

/// Cancel the check that is running (the panes' Cancel, the menu's Cancel the check): no further request is sent to
/// SlopTotal and no further library notes are read; what the check had not finished is dropped (nothing of it is
/// kept), and the status is idle again. SlopTotal keeps running (Stop SlopTotal now stops it).
pub fn cancel_check() {
    with(|a| {
        if a.activity.is_none() {
            return;
        }
        a.run += 1;
        a.index.progress = None;
        a.busy(None);
        a.notice("check cancelled; nothing from it was kept.");
    });
}

/// The check's run number (`App::alive`).
pub fn run_now() -> u64 {
    with(|a| a.run).unwrap_or(0)
}

/// Whether a check that took `run` was cancelled since.
pub fn cancelled(run: u64) -> bool {
    with(|a| !a.alive(run)).unwrap_or(true)
}

/// Check the note's (or selection's) paragraphs for AI writing with SlopTotal: those already scored are not sent again.
pub async fn check_ai(in_selection: bool) {
    let Some(Some((path, editor))) = with(|a| if a.activity.is_some() { None } else { a.target() }) else { return };
    let run = run_now();
    let text = str(&call!(editor, getValue));
    let mut prose = crate::prose::extract(&text);
    let mut range = None;
    if in_selection {
        match selection(&editor) {
            Some((a, b)) => {
                prose = prose.within(a, b);
                range = Some((a, b));
            }
            None => {
                with(|a| a.notice("select the text to check first."));
                return;
            }
        }
    }
    // the selection's rows are marked in the pane
    with(|a| a.focus = range.map(|r| crate::panel::Focus::new(crate::panel::Pane::Ai, &path, vec![r], false)));
    // every paragraph is scored again, even one scored before and unchanged: the user re-checks on purpose (Show in
    // AI check shows the last scores without checking)
    // headings are left out and start a section; a paragraph under SHORT_WORDS words is scored with its neighbours
    let (paras, boundary) = crate::ai::sections(&text, crate::ai::paragraphs(&prose));
    let units = crate::ai::units(&paras, &boundary, crate::sloptotal::SHORT_WORDS);
    with(|a| {
        a.problems.remove(&path);
        let groups: Vec<serde_json::Value> = units.iter().map(|u| serde_json::json!({ "span": [u.span.start, u.span.end], "members": u.members, "key": crate::ai::key(&u.text), "words": u.text.split_whitespace().count() })).collect();
        crate::journal::debug(a, "ai-check", serde_json::json!({ "note": path, "paragraphs": paras.len(), "units": groups, "parallel": a.helper.last.as_ref().map(|h| h.parallel), "device": a.helper.device_in_use() }), None);
    });
    let scorable: Vec<&crate::ai::Unit> = units.iter().filter(|u| u.text.chars().count() >= crate::sloptotal::MIN_CHARS).collect();
    if scorable.is_empty() {
        with(|a| {
            a.notice(if paras.is_empty() { "there is no prose to check." } else { "the paragraphs are too short to score (SlopTotal needs at least 50 characters)." });
            a.save_ai();
            a.changed(Some(path.clone()));
        });
        crate::panel::open(crate::panel::Pane::Ai);
        return;
    }
    let words: usize = paras.iter().map(|p| p.text.split_whitespace().count()).sum();
    let limit = with(|a| a.settings.confirm_over).unwrap_or(5000.0);
    if words as f64 > limit {
        // what the check will hold, by the device SlopTotal runs on (measured on the processor: about 3.5 ms a word with
        // two paragraphs at a time)
        let device = with(|a| a.helper.device_in_use()).unwrap_or_default();
        let secs = (words as f64 * 0.0035).ceil() as u64;
        let what = match device.as_str() {
            "cuda" => format!("SlopTotal holds the graphics card while it checks {} paragraphs ({} words), one request each, a few at a time; your other GPU work waits or shares it meanwhile.", scorable.len(), format_count(words)),
            "cpu" => format!("Checking {} paragraphs ({} words) on the processor takes about {} (one request a paragraph, two at a time), and the computer is busy meanwhile.", scorable.len(), format_count(words), if secs < 60 { format!("{secs} seconds") } else { format!("{} minutes", secs.div_ceil(60)) }),
            _ => format!("Checking {} paragraphs ({} words) with SlopTotal takes a while (one request a paragraph), and the computer is busy meanwhile.", scorable.len(), format_count(words)),
        };
        if !confirm("A long check", &what, "Check").await {
            return;
        }
    }
    let texts: Vec<&str> = scorable.iter().map(|p| p.text.as_str()).collect();
    // the engine: SlopTotal, or GPTZero when its key is linked and the user chooses it for this check (asked before
    // anything is sent; a choice is remembered for this session only when asked)
    let key = gptzero_key().await;
    let session = with(|a| a.session_engine.clone()).flatten();
    let mut engine = match crate::gptzero::plan(key.is_some(), session.as_deref()) {
        crate::gptzero::Plan::SlopTotal => "sloptotal".to_string(),
        crate::gptzero::Plan::GptZero => crate::gptzero::ENGINE.to_string(),
        crate::gptzero::Plan::Ask => {
            let k = key.clone().unwrap_or_default();
            let (status, body) = gptzero_request("GET", crate::gptzero::USAGE, &k, None).await;
            // the plan's words left, for the question (usage-stats: the key alone, no text, no words used)
            let left = if status == 200 { crate::gptzero::parse_usage(&body).map(|u| u.1) } else { None };
            let chosen = ask_engine(&crate::gptzero::question(words, left)).await;
            match crate::gptzero::answer(chosen.as_ref().map(|(e, r)| (e.as_str(), *r))) {
                None => return, // Cancel: nothing is sent
                Some((e, remember)) => {
                    if remember.is_some() {
                        with(|a| a.session_engine = remember);
                    }
                    e
                }
            }
        }
    };
    let started = js::now();
    let mut gz: Option<Vec<Option<crate::sloptotal::Score>>> = None;
    if engine == crate::gptzero::ENGINE {
        with(|a| a.busy(Some(format!("AI check: GPTZero is scoring {} paragraph{}\u{2026}", scorable.len(), if scorable.len() == 1 { "" } else { "s" }))));
        let r = crate::gptzero::scores(&GptZeroNet { run: Some(run) }, &key.unwrap_or_default(), &texts).await;
        if cancelled(run) {
            return;
        }
        match r {
            Ok(s) => gz = Some(s),
            Err(e) => {
                with(|a| {
                    a.busy(None);
                    crate::journal::debug(a, "gptzero-error", serde_json::json!({ "note": path, "error": e }), None);
                });
                if !confirm("GPTZero did not score the text", &crate::gptzero::fallback(&e), "Check with SlopTotal").await {
                    with(|a| a.fail(&path, &e));
                    return;
                }
                engine = "sloptotal".into();
            }
        }
    }
    let (result, device) = match gz {
        Some(s) => (Ok(s), String::new()),
        None => {
            if let Err(e) = crate::helper::ensure_sloptotal("the AI-writing check").await {
                with(|a| {
                    a.busy(None);
                    a.fail(&path, &e);
                });
                return;
            }
            with(|a| a.busy(Some(format!("AI check: scoring {} paragraph{}\u{2026}", scorable.len(), if scorable.len() == 1 { "" } else { "s" }))));
            let env = crate::helper::proxy();
            // SlopTotal's weights, to leave out a quick detector that fails (sloptotal::quick)
            crate::helper::load_weights().await;
            let weights = with(|a| a.helper.weights.clone()).flatten();
            let result = crate::sloptotal::paragraphs(&env, &texts, weights.as_ref()).await;
            (result, with(|a| a.helper.device_in_use()).unwrap_or_default())
        }
    };
    if cancelled(run) {
        with(|a| crate::journal::debug(a, "ai-check-cancelled", serde_json::json!({ "note": path }), None));
        return; // nothing of a cancelled check is kept
    }
    with(|a| {
        a.busy(None);
        match result {
            Ok(scores) => {
                let now = js_sys::Date::now();
                // the history: each unit scored, with every quick detector's result
                let mut history = crate::journal::Run { t: now / 1000.0, note: path.clone(), kind: "paragraphs".into(), engine: if engine == crate::gptzero::ENGINE { engine.clone() } else { String::new() }, device: device.clone(), helper: a.helper.last.as_ref().map(|h| h.version.clone()).unwrap_or_default(), items: Vec::new() };
                for (u, s) in scorable.iter().zip(&scores) {
                    let Some(score) = s else { continue };
                    let mut it = crate::journal::item(&u.text, score.score, a.settings.keep_text);
                    it.detectors = score.engines.iter().map(|(k, x)| (k.clone(), *x, None)).collect();
                    it.failed = score.failed.clone();
                    it.together = u.members.iter().find_map(|&m| crate::ai::together_label(u, m, crate::sloptotal::SHORT_WORDS)).unwrap_or_default();
                    history.items.push(it);
                }
                crate::journal::debug(a, "ai-check-done", serde_json::json!({ "note": path, "ms": (js::now() - started).round(), "results": history.items.iter().map(|i| serde_json::json!({ "key": i.key, "score": i.score, "detectors": i.detectors, "failed": i.failed })).collect::<Vec<_>>() }), None);
                crate::journal::record(a, &history);
                for (u, s) in scorable.iter().zip(scores) {
                    let Some(score) = s else { continue };
                    let span: Vec<String> = paras[u.span.clone()].iter().map(|p| crate::ai::key(&p.text)).collect();
                    for &m in &u.members {
                        let p = &paras[m];
                        let key = crate::ai::key(&p.text);
                        // the reasons and the sentences' scores from before are asked again too, when wanted
                        a.ai.analyses.remove(&key);
                        for k in (0..crate::ai::sentences(&p.text).len()).filter_map(|i| crate::ai::context(&p.text, &crate::ai::sentences(&p.text), i)) {
                            a.ai.sentences.remove(&crate::ai::key(&k));
                        }
                        let together = crate::ai::together_label(u, m, crate::sloptotal::SHORT_WORDS).map(|label| crate::ai::Together { span: span.clone(), label });
                        a.ai.keep(&path, key, crate::ai::Scored { score: score.clone(), when: now, device: device.clone(), together });
                    }
                }
                a.save_ai();
                a.changed(Some(path.clone()));
            }
            Err(e) => {
                crate::journal::debug(a, "ai-check-error", serde_json::json!({ "note": path, "error": e, "ms": (js::now() - started).round() }), None);
                a.fail(&path, &e)
            }
        }
    });
    crate::helper::touched();
    crate::panel::open(crate::panel::Pane::Ai);
}

/// Check the note's (or selection's) wording against the library notes, and its sentences for close paraphrase.
/// Always a new check, even of text checked before and unchanged (the user re-checks on purpose): no cache is used.
/// Show in similarity check is what shows the last results without checking.
pub async fn check_plagiarism(in_selection: bool) {
    let Some(Some((path, editor))) = with(|a| if a.activity.is_some() { None } else { a.target() }) else { return };
    let run = run_now();
    let text = str(&call!(editor, getValue));
    let range = if in_selection {
        match selection(&editor) {
            Some(r) => Some(r),
            None => {
                with(|a| a.notice("select the text to check first."));
                return;
            }
        }
    } else {
        None
    };
    let (checked, base) = match range {
        Some((a, b)) => (slice16(&text, a, b).0, a),
        None => (text.clone(), 0),
    };
    with(|a| a.problems.remove(&path));
    let found = crate::index::check(&path, &checked, base).await;
    if cancelled(run) {
        return;
    }
    let mut matches = match found {
        Ok(m) => m,
        Err(e) => {
            with(|a| {
                a.busy(None);
                a.fail(&path, &e);
            });
            return;
        }
    };
    // close paraphrase, for the sentences not already found word for word: the content-word search, or the embedding
    // search when it is on and answers
    let mut why_not = String::new();
    let mut floor = None;
    if with(|a| a.settings.paraphrase).unwrap_or(false) {
        match crate::api::paraphrase(&path, &checked, base, &matches, true, Some(run)).await {
            Ok((mut p, f, note)) => {
                matches.append(&mut p);
                floor = Some(f);
                if let Some(n) = note {
                    with(|a| a.notice(&n));
                }
            }
            Err(e) if e == crate::sloptotal::CANCELLED => return,
            Err(e) => why_not = format!("Paraphrase was not checked: {e}"),
        }
    } else {
        why_not = "Paraphrase is off (Settings \u{2192} Authenticity \u{2192} Plagiarism).".into();
    }
    if cancelled(run) {
        return;
    }
    with(|a| {
        a.busy(None);
        let t = a.thresholds();
        let sentences = crate::similarity::count_sentences(&checked, &a.settings.similarity());
        let new = crate::similarity::Checked { min_run: t.min_run, floor, found: matches, sentences, selection: range.is_some() };
        // what passes the thresholds now, for the notice
        let n = new.matches(t).iter().filter(|m| m.kind != "common").count();
        // a selection's findings replace those it covers, and are layered over the rest
        a.found.entry(path.clone()).or_default().merge(new, range);
        a.refilter(Some(&path));
        if why_not.is_empty() {
            a.para_note.remove(&path);
        } else {
            a.para_note.insert(path.clone(), why_not.clone());
        }
        a.save_matches();
        a.save_para_cache();
        a.focus = range.map(|r| crate::panel::Focus::new(crate::panel::Pane::Plagiarism, &path, vec![r], false));
        a.notice(&match n {
            0 => "no wording shared with the library found.".to_string(),
            1 => "1 passage shares wording with the library; it is underlined, and the Plagiarism pane lists it.".to_string(),
            n => format!("{n} passages share wording with the library; they are underlined, and the Plagiarism pane lists them."),
        });
        a.changed(Some(path.clone()));
    });
    crate::panel::open(crate::panel::Pane::Plagiarism);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_by_utf16() {
        let t = "a😀b é c";
        assert_eq!(slice16(t, 1, 3), ("😀".to_string(), 1));
        assert_eq!(slice16(t, 3, 6).0, "b é");
        assert_eq!(slice16(t, 0, 99).0, t);
        assert_eq!(format_count(12345), "12,345");
        assert_eq!(port(80.0, 8000), 8000);
        assert_eq!(port(9000.0, 8000), 9000);
    }

    /// The pane and the settings tab set the same two settings, by the same names, in the one Settings (data.json),
    /// and the thresholds the results are filtered with follow.
    #[test]
    fn thresholds_are_one_setting_for_the_pane_and_the_tab() {
        let keys: Vec<&str> = UI.iter().map(|r| r.0).collect();
        assert!(keys.contains(&MIN_RUN_KEY) && keys.contains(&CLOSENESS_KEY), "the settings tab shows both");
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert!(json.get(MIN_RUN_KEY).is_some() && json.get(CLOSENESS_KEY).is_some() && json.get("plagSort").is_some(), "data.json's names");
        let s = Settings::default().with_field(CLOSENESS_KEY, serde_json::json!(0.65)).unwrap().with_field(MIN_RUN_KEY, serde_json::json!(8.0)).unwrap();
        assert_eq!((s.paraphrase_closeness, s.min_run), (0.65, 8.0));
        assert_eq!(s.thresholds(), crate::similarity::Thresholds { min_run: 8, closeness: 0.65 });
        assert_eq!(s.with_field("plagSort", serde_json::json!("position")).unwrap().plag_sort, "position");
        assert!(s.with_field(CLOSENESS_KEY, serde_json::json!("half")).is_none(), "a value that does not fit is refused");
        assert_eq!(serde_json::from_value::<Settings>(serde_json::json!({ "minRun": 7 })).unwrap().plag_sort, "closeness", "0.2's data.json loads");
    }
}
