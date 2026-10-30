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
//! search and says so. Its address and its token come from Quarry's plugin API (`serviceUrl()`, `serviceToken()`;
//! Quarry's docs/companion-plugins.md): since Quarry 2026-10-30 every request to its service carries the token
//! (`Authorization: Bearer`), and in a browser (Ignis) the address is the relay on the page's own origin.

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

/// Where the embedding service is and the token its requests carry.
#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    /// The address without a trailing slash: `http://127.0.0.1:8766`, or in a browser `https://<page>/relay/quarry`.
    pub url: String,
    /// The service's token (Quarry's `serviceToken()`); None from a Quarry older than the token, or before Quarry has
    /// reached its service.
    pub token: Option<String>,
}

/// A string a function of the plugin's API returns (or the property itself, from an older API), trimmed; None when
/// missing or empty.
fn api_text(api: &JsValue, name: &str) -> Option<String> {
    let f = get(api, name);
    let v = if f.is_function() { js::apply(&f, api, &[]) } else { f };
    v.as_string().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The service as Quarry's plugin API gives it: `serviceUrl()` and `serviceToken()`; from an older Quarry,
/// `servicePort` on 127.0.0.1, else the usual port, without a token.
fn embeddings_service(plugin: &JsValue) -> Service {
    let api = get(plugin, "api");
    service_from(api_text(&api, "serviceUrl"), get(&api, "servicePort").as_f64().or_else(|| {
        let f = get(&api, "servicePort");
        f.is_function().then(|| js::apply(&f, &api, &[]).as_f64()).flatten()
    }), api_text(&api, "serviceToken"))
}

/// The service from what the API gave (testable without JavaScript): an http(s) address as given (a trailing slash
/// taken off), else 127.0.0.1 on the port (checked), else on the usual port.
pub fn service_from(url: Option<String>, port: Option<f64>, token: Option<String>) -> Service {
    let url = url.filter(|u| u.starts_with("http://") || u.starts_with("https://")).map(|u| u.trim_end_matches('/').to_string());
    let url = url.unwrap_or_else(|| format!("http://127.0.0.1:{}", port.map_or(EMBEDDINGS_PORT, |p| crate::app::port(p, EMBEDDINGS_PORT))));
    Service { url, token }
}

/// Whether a check uses the embedding search now: the setting on and the plugin there; its service, or why not.
/// Whether the service answers is learnt when it is asked (a refused connection says so, and the check falls back).
pub fn embeddings_status(a: &App) -> Result<Service, String> {
    if !a.settings.paraphrase_embeddings {
        return Err("the embedding search is off".into());
    }
    Ok(embeddings_service(&embeddings_plugin(a)?))
}

/// Whether `url` is on the page's own origin (a browser through Ignis): such a request goes with the page's own
/// fetch, which carries the Cloudflare Access sign-in; Obsidian's requestUrl would send it from Ignis's server instead.
fn same_origin(url: &str) -> bool {
    let origin = str(&get(&get(&js::global(), "location"), "origin"));
    origin.starts_with("http") && (url == origin || url.starts_with(&format!("{origin}/")))
}

/// A POST to the embedding service, with its token: (status, the answer's text).
async fn send(service: &Service, path: &str, body: &Value) -> Result<(u16, String), JsValue> {
    let url = format!("{}{path}", service.url);
    let headers = JsValue::from(js_sys::Object::new());
    set(&headers, "Content-Type", "application/json");
    if let Some(t) = &service.token {
        set(&headers, "Authorization", format!("Bearer {t}"));
    }
    if same_origin(&url) {
        let init = obj! { method: "POST", headers: headers, body: body.to_string() };
        let res = js::wait(js::apply(&get(&js::global(), "fetch"), &js::global(), &[url.into(), init])).await?;
        let text = js::wait(call!(res, text)).await?;
        return Ok((get(&res, "status").as_f64().unwrap_or(0.0) as u16, str(&text)));
    }
    let request_url = with(|a| get(&a.obs, "requestUrl")).ok_or(JsValue::NULL)?;
    let req = obj! { url: url, method: "POST", headers: headers, body: body.to_string(), throw: false };
    let res = js::wait(js::apply(&request_url, &JsValue::NULL, &[req])).await?;
    Ok((get(&res, "status").as_f64().unwrap_or(0.0) as u16, str(&get(&res, "text"))))
}

/// A POST to the embedding service: its JSON answer, or what went wrong in a sentence.
async fn post_similar(service: &Service, path: &str, body: &Value) -> Result<Value, String> {
    let (status, text) = send(service, path, body).await.map_err(|_| format!("its service does not answer at {} (it is started from its own plugin)", service.url))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    answer(status, v, service.token.is_some())
}

/// The service's answer as a result (testable without JavaScript).
pub fn answer(status: u16, v: Value, had_token: bool) -> Result<Value, String> {
    match status {
        200 => Ok(v),
        401 if !had_token => Err("its service asks for its token, which the Quarry plugin has not given yet (update Quarry, or let it reach its service first)".into()),
        401 => Err("its service refused the token the Quarry plugin gave (in a browser, paste the token again in Quarry's settings)".into()),
        404 => Err("its service has no similarity API (update the Quarry plugin)".into()),
        _ => Err(format!("its service answered {status}{}", v["error"].as_str().map(|e| format!(": {}", e.chars().take(300).collect::<String>())).unwrap_or_default())),
    }
}

/// The embedding service, for similarity::ask.
struct Embeddings(Service);

impl similarity::SimilarApi for Embeddings {
    async fn similar(&self, body: &Value) -> Result<Value, String> {
        post_similar(&self.0, "/similar", body).await
    }
}

/// The Check button beside the setting: whether the embedding search can be used now, in a sentence. It looks at the
/// plugin (installed, enabled), reaches its service on 127.0.0.1, checks the similarity API's version, and sends one
/// short probe sentence. It never starts the service.
pub async fn check_embeddings() -> String {
    let Some(service) = with(|a| embeddings_plugin(a).map(|p| embeddings_service(&p))) else { return "the plugin is unloading.".into() };
    let service = match service {
        Ok(s) => s,
        Err(e) => return format!("Not available: {e}."),
    };
    let at = &service.url;
    match post_similar(&service, "/similar", &similarity::first_request(&[PROBE], "cpu")).await.and_then(|v| similarity::check_api(&v).map(|_| v)) {
        Ok(v) if v["gpu_shortage"].is_object() => format!("Answers at {at} (similarity API {}); its models would need room on the graphics card, which it asks about when used.", similarity::SIMILAR_API),
        Ok(_) => format!("OK: the Quarry plugin's embedding search answers at {at} (similarity API {}).", similarity::SIMILAR_API),
        Err(e) => format!("Not available: {e}."),
    }
}

/// Close-paraphrase candidates for `text` (at `base` in the note) from the embedding search: its sentences not already
/// matched word for word, asked unless asked before (the cache, by sentence; `ask_all`: every sentence asked, as Check
/// for plagiarism does), each with its candidates from a closeness a little below the setting
/// (`similarity::fetch_floor`, returned with them) so the Plagiarism pane can change the setting without asking again;
/// `similarity::filter` applies the setting. The GPU question is asked if the service's models have no room.
pub async fn paraphrase_embeddings(service: Service, text: &str, base: u32, verbatim: &[Match], ask_all: bool, run: Option<u64>) -> Result<(Vec<Match>, f64), String> {
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
        match similarity::ask(&Embeddings(service.clone()), &asked, floor, &gpu).await? {
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
    if let Some(Ok(service)) = with(|a| embeddings_status(a)) {
        match paraphrase_embeddings(service, text, base, verbatim, ask_all, run).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_from_quarrys_api() {
        let t = Some("ab".repeat(32));
        assert_eq!(service_from(Some("http://127.0.0.1:9000/".into()), Some(8766.0), t.clone()), Service { url: "http://127.0.0.1:9000".into(), token: t.clone() }, "serviceUrl() wins, its trailing slash off");
        assert_eq!(service_from(Some("https://vault.example.org/relay/quarry".into()), None, t.clone()).url, "https://vault.example.org/relay/quarry", "Ignis's relay, as given");
        assert_eq!(service_from(None, Some(9001.0), None), Service { url: "http://127.0.0.1:9001".into(), token: None }, "an older Quarry: its servicePort");
        assert_eq!(service_from(None, None, None).url, "http://127.0.0.1:8766");
        for odd in ["javascript:alert(1)", "file:///etc/passwd", "127.0.0.1:8766", "/relay/quarry"] {
            assert_eq!(service_from(Some(odd.into()), None, None).url, "http://127.0.0.1:8766", "{odd}: not a web address, the usual one instead");
        }
        assert_eq!(service_from(None, Some(80.0), None).url, "http://127.0.0.1:8766", "a port below 1024 is not the service's");
    }

    #[test]
    fn a_refusal_says_what_to_do() {
        assert_eq!(answer(200, json!({"a": 1}), true), Ok(json!({"a": 1})));
        assert!(answer(401, Value::Null, false).unwrap_err().contains("not given yet"));
        assert!(answer(401, Value::Null, true).unwrap_err().contains("paste the token again"));
        assert!(answer(404, Value::Null, true).unwrap_err().contains("no similarity API"));
        assert_eq!(answer(500, json!({"error": "boom"}), true).unwrap_err(), "its service answered 500: boom");
    }
}
