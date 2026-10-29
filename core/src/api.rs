//! What Authenticity offers other plugins, and the optional embedding search for close paraphrase.
//!
//! **What Authenticity offers** (`app.plugins.plugins.authenticity.api`, docs/assistant-tools.md): `version`, `available()` (which
//! checks can run now, and why not), `checkAi(paragraphs)`, `explainAi(text)`, `checkSimilarity(text, options)`,
//! `revealCharacters(text, options)` and `hints(sentence, paragraph)`. Each check returns a Promise of plain JSON;
//! none changes a note, and none starts SlopTotal unless asked to (`checkAi` and `explainAi` do: they are checks).
//!
//! **The optional embedding search** (Settings → Plagiarism, off by default, offered only when the Quarry plugin is
//! installed): close-paraphrase candidates from that plugin's embedding model, through its service's similarity API
//! on this computer (`POST /similar` on 127.0.0.1, behind a version check), instead of Authenticity's own content-word
//! search (index.rs). The service is never started from here; when it does not answer, a check falls back to the own
//! search and says so.

use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::future_to_promise;

use crate::app::{with, App};
use crate::js::{self, call, f1, f2, get, obj, set, str};
use crate::similarity::{self, Hit, Match, Outcome};

/// The version of this object's contract (docs/assistant-tools.md).
pub const VERSION: u32 = 1;
/// The plugin whose embedding model the optional search uses, and its service's usual port.
const EMBEDDINGS_PLUGIN: &str = "quarry";
const EMBEDDINGS_PORT: u16 = 8766;
/// The sentence the Check button sends (eight words or more, as any sentence asked about).
const PROBE: &str = "This short sentence checks that the similarity service answers here.";

// ----- the optional embedding search

fn plugins(a: &App) -> JsValue {
    get(&a.app, "plugins")
}

/// Whether the plugin with the embedding model is installed (the setting is offered only then).
pub fn embeddings_installed(a: &App) -> bool {
    !get(&get(&plugins(a), "manifests"), EMBEDDINGS_PLUGIN).is_undefined()
}

/// The plugin with the embedding model, when it is installed, enabled and loaded.
fn embeddings_plugin(a: &App) -> Result<JsValue, String> {
    let p = plugins(a);
    if !embeddings_installed(a) {
        return Err("the Quarry plugin is not installed".into());
    }
    if !call!(get(&p, "enabledPlugins"), has, EMBEDDINGS_PLUGIN).is_truthy() {
        return Err("the Quarry plugin is not enabled".into());
    }
    let plugin = get(&get(&p, "plugins"), EMBEDDINGS_PLUGIN);
    if plugin.is_undefined() || plugin.is_null() {
        return Err("the Quarry plugin has not loaded".into());
    }
    Ok(plugin)
}

/// The service's port, as the plugin's API gives it, else the usual one.
fn embeddings_port(plugin: &JsValue) -> u16 {
    let api = get(plugin, "api");
    let from_api = if get(&api, "servicePort").is_function() { call!(api, servicePort).as_f64() } else { get(&api, "servicePort").as_f64() };
    from_api.map_or(EMBEDDINGS_PORT, |p| crate::app::port(p, EMBEDDINGS_PORT))
}

/// Whether a check uses the embedding search now: the setting on and the plugin there; its service's port, or why not.
/// Whether the service answers is learnt when it is asked (a refused connection says so, and the check falls back).
pub fn embeddings_status(a: &App) -> Result<u16, String> {
    if !a.settings.paraphrase_embeddings {
        return Err("the embedding search is off".into());
    }
    Ok(embeddings_port(&embeddings_plugin(a)?))
}

/// A POST to the embedding service on this computer.
async fn post_similar(port: u16, path: &str, body: &Value) -> Result<Value, String> {
    let request_url = with(|a| get(&a.obs, "requestUrl")).ok_or("the plugin is unloading")?;
    let req = obj! { url: format!("http://127.0.0.1:{port}{path}"), method: "POST", contentType: "application/json", body: body.to_string(), throw: false };
    let res = js::wait(js::apply(&request_url, &JsValue::NULL, &[req])).await.map_err(|_| format!("its service does not answer on port {port} (it is started from its own plugin)"))?;
    let status = get(&res, "status").as_f64().unwrap_or(0.0) as u16;
    let v: Value = serde_json::from_str(&str(&get(&res, "text"))).unwrap_or(Value::Null);
    match status {
        200 => Ok(v),
        404 => Err("its service has no similarity API (update the Quarry plugin)".into()),
        _ => Err(format!("its service answered {status}{}", v["error"].as_str().map(|e| format!(": {}", e.chars().take(300).collect::<String>())).unwrap_or_default())),
    }
}

/// The embedding service on this computer, for similarity::ask.
struct Embeddings(u16);

impl similarity::SimilarApi for Embeddings {
    async fn similar(&self, body: &Value) -> Result<Value, String> {
        post_similar(self.0, "/similar", body).await
    }
}

/// The Check button beside the setting: whether the embedding search can be used now, in a sentence. It looks at the
/// plugin (installed, enabled), reaches its service on 127.0.0.1, checks the similarity API's version, and sends one
/// short probe sentence. It never starts the service.
pub async fn check_embeddings() -> String {
    let Some(port) = with(|a| embeddings_plugin(a).map(|p| embeddings_port(&p))) else { return "the plugin is unloading.".into() };
    let port = match port {
        Ok(p) => p,
        Err(e) => return format!("Not available: {e}."),
    };
    match post_similar(port, "/similar", &similarity::first_request(&[PROBE], "cpu")).await.and_then(|v| similarity::check_api(&v).map(|_| v)) {
        Ok(v) if v["gpu_shortage"].is_object() => format!("Answers on port {port} (similarity API {}); its models would need room on the graphics card, which it asks about when used.", similarity::SIMILAR_API),
        Ok(_) => format!("OK: the Quarry plugin's embedding search answers on port {port} (similarity API {}).", similarity::SIMILAR_API),
        Err(e) => format!("Not available: {e}."),
    }
}

/// Close-paraphrase candidates for `text` (at `base` in the note) from the embedding search: its sentences not already
/// matched word for word, asked unless asked before (the cache, by sentence; `ask_all`: every sentence asked, as Check
/// for plagiarism does), each with its candidates from a closeness a little below the setting
/// (`similarity::fetch_floor`, returned with them) so the Plagiarism pane can change the setting without asking again;
/// `similarity::filter` applies the setting. The GPU question is asked if the service's models have no room.
pub async fn paraphrase_embeddings(port: u16, text: &str, base: u32, verbatim: &[Match], ask_all: bool, run: Option<u64>) -> Result<(Vec<Match>, f64), String> {
    let (settings, closeness) = with(|a| (a.settings.similarity(), a.thresholds().closeness)).ok_or("the plugin is unloading")?;
    let floor = similarity::fetch_floor(closeness);
    let sentences = similarity::paraphrase_sentences(text, &settings, &crate::index::covered(text, base, verbatim));
    if sentences.is_empty() {
        return Ok((Vec::new(), floor));
    }
    // the sentences asked about before (unchanged since) are not asked again
    let known: Vec<Option<Vec<Hit>>> = with(|a| sentences.iter().map(|s| if ask_all { None } else { a.para_cache.get(&s.1, floor) }).collect()).ok_or("the plugin is unloading")?;
    let todo: Vec<usize> = (0..sentences.len()).filter(|&i| known[i].is_none()).collect();
    let asked: Vec<String> = todo.iter().map(|&i| sentences[i].1.clone()).collect();
    if !asked.is_empty() {
        let n = sentences.len() - asked.len();
        with(|a| a.busy(Some(format!("Plagiarism: the embedding search on {} sentences{}\u{2026}", asked.len(), if n > 0 { format!(" ({n} unchanged, known)") } else { String::new() }))));
    }
    let mut gpu = String::new();
    let got = loop {
        match similarity::ask(&Embeddings(port), &asked, floor, &gpu).await? {
            Outcome::Hits(h) => break h,
            Outcome::Shortage(s) => {
                let short = obsidian_kit::gpu::Shortage {
                    need_mb: s["need_mb"].as_u64().unwrap_or(0),
                    free_mb: s["free_mb"].as_u64().unwrap_or(0),
                    others_mb: s["others_mb"].as_u64().unwrap_or(0),
                    holders: s["holders"].as_array().map(|a| a.iter().take(8).filter_map(|h| h.as_str()).map(|h| h.chars().take(40).collect()).collect()).unwrap_or_default(),
                };
                let asking = obsidian_kit::gpu::Asking { plugin: "Quarry", models: "its embedding model", task: "the paraphrase check", on_cpu: "Check on the CPU", decline: "Use Authenticity's own search" };
                let (obs, app) = with(|a| (a.obs.clone(), a.app.clone())).ok_or("the plugin is unloading")?;
                match obsidian_kit::gpu::choose(&obs, &app, &short, &asking).await {
                    Some(choice) => gpu = choice.to_string(),
                    None => return Err("its models had no room on the graphics card".into()),
                }
            }
        }
    };
    // a cancelled check keeps nothing, not even in the cache
    if run.is_some_and(crate::app::cancelled) {
        return Err(crate::sloptotal::CANCELLED.into());
    }
    with(|a| {
        for (k, &i) in todo.iter().enumerate() {
            a.para_cache.put(&sentences[i].1, floor, got.get(k).cloned().unwrap_or_default());
        }
    });
    let mut fresh = got.into_iter();
    let hits: Vec<Vec<Hit>> = known.into_iter().map(|k| k.unwrap_or_else(|| fresh.next().unwrap_or_default())).collect();
    Ok((similarity::paraphrase_matches(text, base, &sentences, &hits), floor))
}

/// Close paraphrase for a check: the embedding search when it is on and answers, else (or when it does not answer)
/// Authenticity's own content-word search. The candidates, the closeness they were kept from, and a note when the
/// embedding search was wanted but did not answer.
pub async fn paraphrase(path: &str, text: &str, base: u32, verbatim: &[Match], ask_all: bool, run: Option<u64>) -> Result<(Vec<Match>, f64, Option<String>), String> {
    let mut fell_back = None;
    if let Some(Ok(port)) = with(|a| embeddings_status(a)) {
        match paraphrase_embeddings(port, text, base, verbatim, ask_all, run).await {
            Ok((m, floor)) => return Ok((m, floor, None)),
            Err(e) if e == crate::sloptotal::CANCELLED => return Err(e),
            Err(e) => fell_back = Some(format!("The embedding search did not answer ({e}); Authenticity's own search was used.")),
        }
    }
    let floor = with(|a| similarity::fetch_floor(a.thresholds().closeness)).ok_or("the plugin is unloading")?;
    let m = crate::index::paraphrase(path, text, base, verbatim, floor).await?;
    Ok((m, floor, fell_back))
}

// ----- the object other plugins call

fn promise(f: impl std::future::Future<Output = Result<Value, String>> + 'static) -> JsValue {
    future_to_promise(async move {
        match f.await {
            Ok(v) => Ok(serde_wasm_bindgen::to_value(&v).unwrap_or(JsValue::NULL)),
            Err(e) => Err(JsValue::from(js_sys::Error::new(&e))),
        }
    })
    .into()
}

fn strings(v: &JsValue, max: usize) -> Vec<String> {
    js_sys::Array::from(v).iter().take(max).filter_map(|x| x.as_string()).collect()
}

/// The features offered now (the status file's and the API's list): the similarity finder and the character check
/// always, the AI score once this computer has a SlopTotal folder.
pub fn features(a: &App) -> Vec<&'static str> {
    let mut f = Vec::new();
    if !a.device.sloptotal_folder.trim().is_empty() {
        f.push("ai_score");
    }
    f.extend(["similarity", "invisible_characters"]);
    f
}

/// The MCP server's name as registered and its tools (in the status file, for local tools that read it).
pub const MCP_SERVER: &str = "authenticity";
pub const MCP_TOOLS: &[&str] = &["check_ai", "explain_ai", "check_similarity", "reveal_characters", "check_overlap", "status", "list_notes", "ai_results", "similarity_results", "unusual_characters"];

/// A status file other local tools can read (whether Authenticity is enabled, its version, its features and its MCP
/// tools; no text): written on load and every minute while enabled, deleted on unload.
/// `<local data>/obsidian-kit/companions/authenticity.json`.
pub fn status_json(a: &App, now_s: u64) -> Value {
    let version = str(&get(&get(&a.plugin, "manifest"), "version"));
    let port = if a.helper.last.is_some() { crate::app::port(a.device.helper_port, 8791) } else { 0 };
    json!({ "name": "authenticity", "version": version, "api": VERSION, "enabled": true, "updated": now_s, "features": features(a),
        "mcp": { "server": MCP_SERVER, "tools": MCP_TOOLS }, "port": port })
}

fn status_file() -> Option<String> {
    let process = get(&js::global(), "process");
    let windows = str(&get(&process, "platform")) == "win32";
    let env = get(&process, "env");
    let var = |k: &str| Some(str(&get(&env, k))).filter(|v| !v.is_empty());
    let root = var("LOCALAPPDATA").or_else(|| var("XDG_DATA_HOME")).or_else(|| var("HOME").map(|h| format!("{h}/.local/share")))?;
    let sep = if windows { "\\" } else { "/" };
    Some(format!("{root}{sep}obsidian-kit{sep}companions{sep}authenticity.json"))
}

fn fs_call(a: &App, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let fs = js::apply(&a.require, &JsValue::NULL, &["fs".into()]);
    get(&fs, name).dyn_into::<js_sys::Function>()?.apply(&fs, &args.iter().collect::<js_sys::Array>())
}

fn write_status() {
    with(|a| {
        let Some(path) = status_file() else { return };
        let dir = path.rsplit_once(['\\', '/']).map(|(d, _)| d.to_string()).unwrap_or_default();
        let options = js_sys::Object::new();
        set(&options, "recursive", true);
        let text = status_json(a, (js_sys::Date::now() / 1000.0) as u64).to_string();
        let tmp = format!("{path}.tmp");
        let _ = fs_call(a, "mkdirSync", &[dir.into(), options.into()]).and_then(|_| fs_call(a, "writeFileSync", &[tmp.clone().into(), text.into()])).and_then(|_| fs_call(a, "renameSync", &[tmp.into(), path.into()]));
    });
}

/// Offer the API (set `api` on the plugin and say so with `authenticity:api-changed`) and start the status file.
pub fn publish() {
    let Some((plugin, ws)) = with(|a| (a.plugin.clone(), get(&a.app, "workspace"))) else { return };
    set(&plugin, "api", object());
    call!(ws, trigger, "authenticity:api-changed");
    if js::desktop() {
        write_status();
        let id = call!(js::global(), setInterval, crate::js::f0(write_status), 60_000);
        with(|a| a.status_timer = id.as_f64());
    }
}

/// The features changed (this computer's settings): the API's list, and the status file.
pub fn refresh() {
    let Some((plugin, f)) = with(|a| (a.plugin.clone(), features(a))) else { return };
    let api = get(&plugin, "api");
    if api.is_object() {
        set(&api, "features", serde_wasm_bindgen::to_value(&f).unwrap_or(JsValue::NULL));
        if js::desktop() {
            write_status();
        }
    }
}

/// Withdraw the API and the status file (on unload), inside `with`.
pub fn withdraw(a: &mut App) {
    if let Some(t) = a.status_timer.take() {
        call!(js::global(), clearInterval, t);
    }
    js_sys::Reflect::delete_property(a.plugin.unchecked_ref::<js_sys::Object>(), &"api".into()).ok();
    call!(get(&a.app, "workspace"), trigger, "authenticity:api-changed");
    if js::desktop() {
        if let Some(path) = status_file() {
            let _ = fs_call(a, "rmSync", &[path.into(), crate::js::obj! { force: true }]);
        }
    }
}

/// What can run now, and why not (the API's `status()`; `available()` is its plain yes or no).
fn status() -> Value {
    with(|a| {
        let ai = if a.device.sloptotal_folder.trim().is_empty() { json!({ "ok": false, "why": "the SlopTotal folder is not set (Settings → Authenticity → Writing checks)" }) } else { json!({ "ok": true, "sloptotal": a.helper.last.as_ref().map(|h| h.state.clone()) }) };
        let para = match (a.settings.paraphrase, embeddings_status(a)) {
            (false, _) => json!({ "ok": false, "why": "paraphrase is off in Authenticity's settings" }),
            (true, Ok(_)) => json!({ "ok": true, "search": "embeddings" }),
            (true, Err(_)) => json!({ "ok": true, "search": "content words" }),
        };
        json!({ "version": VERSION, "ai": ai, "similarity": { "ok": true }, "paraphrase": para, "characters": { "ok": true } })
    })
    .unwrap_or(Value::Null)
}

pub fn object() -> JsValue {
    let o = js_sys::Object::new();
    set(&o, "version", VERSION);
    // synchronous, no request, no model load
    set(&o, "available", f1(|_| JsValue::from(with(|a| a.panes.any() || !a.status.is_undefined()).unwrap_or(false))));
    set(&o, "features", serde_wasm_bindgen::to_value(&with(|a| features(a)).unwrap_or_default()).unwrap_or(JsValue::NULL));
    set(&o, "status", f1(|_| serde_wasm_bindgen::to_value(&status()).unwrap_or(JsValue::NULL)));
    set(&o, "checkAi", f1(|paras| {
        let paras = strings(&paras, 500);
        promise(async move {
            crate::helper::ensure_sloptotal_for("a check for another plugin", None).await?;
            crate::helper::load_weights().await;
            let weights = with(|a| a.helper.weights.clone()).flatten();
            let scores = crate::ai::score_list(&crate::helper::proxy_detached(), &paras, weights.as_ref()).await?;
            Ok(json!({ "paragraphs": scores.into_iter().map(|s| match s {
                Some((s, together)) => json!({ "score": s.score, "verdict": s.verdict, "confidence": s.confidence, "class": crate::ai::class(crate::ai::tint(s.score)), "describe": crate::ai::describe(&s), "scoredTogether": together }),
                None => json!({ "skipped": "not scored (too short even with the others, or too few detectors ran)" }),
            }).collect::<Vec<_>>() }))
        })
    }));
    set(&o, "explainAi", f1(|text| {
        let text = text.as_string().unwrap_or_default();
        promise(async move {
            crate::helper::ensure_sloptotal_for("a check for another plugin", None).await?;
            let mut a = crate::sloptotal::analyze(&crate::helper::proxy_detached(), &text).await?;
            crate::helper::load_weights().await;
            let weights = with(|x| x.helper.weights.clone()).flatten();
            crate::sloptotal::settle(&mut a, weights.as_ref(), &text);
            Ok(serde_json::to_value(a).unwrap_or(Value::Null))
        })
    }));
    set(&o, "checkSimilarity", f2(|text, options| {
        let text = text.as_string().unwrap_or_default();
        let mode = get(&options, "mode").as_string().unwrap_or_else(|| "both".into());
        let note = get(&options, "note").as_string().unwrap_or_default();
        promise(async move {
            let mut m = if mode == "paraphrase" { Vec::new() } else { crate::index::check(&note, &text, 0).await? };
            let mut why = Value::Null;
            if mode != "verbatim" {
                match paraphrase(&note, &text, 0, &m, false, None).await {
                    Ok((mut p, _, note)) => {
                        m.append(&mut p);
                        if let Some(n) = note {
                            why = json!(n);
                        }
                    }
                    Err(e) => why = json!(e),
                }
            }
            // the user's thresholds, as the Plagiarism pane shows them
            let t = with(|a| a.thresholds()).ok_or("the plugin is unloading")?;
            Ok(json!({ "matches": similarity::filter(&m, t), "paraphrase_not_checked": why }))
        })
    }));
    set(&o, "revealCharacters", f2(|text, options| {
        let text = text.as_string().unwrap_or_default();
        let typo = get(&options, "typographic").as_bool().unwrap_or(false);
        let hits: Vec<Value> = crate::chars::scan(&text, typo).iter().map(|h| json!({ "from": h.from, "to": h.to, "code": format!("U+{:04X}", h.code), "name": h.name, "kind": h.kind.label() })).collect();
        serde_wasm_bindgen::to_value(&hits).unwrap_or(JsValue::NULL)
    }));
    set(&o, "hints", f2(|sentence, paragraph| {
        let s = sentence.as_string().unwrap_or_default();
        let p = strings(&paragraph, 200);
        let refs: Vec<&str> = p.iter().map(String::as_str).collect();
        let h: Vec<Value> = crate::hints::hints(&s, &refs).into_iter().map(|h| json!({ "id": h.id, "label": h.label, "found": h.found, "why": h.why, "spans": h.spans.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>() })).collect();
        serde_wasm_bindgen::to_value(&h).unwrap_or(JsValue::NULL)
    }));
    o.into()
}
