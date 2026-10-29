//! Authenticity's helper, from the plugin's side. The helper (../native, `authenticity-helper`) is a small program on
//! 127.0.0.1 that starts SlopTotal when a check needs it, stops it when idle (or when another of the user's plugins
//! needs the GPU it holds), decides the device, and passes the plugin's requests to SlopTotal with the text alone.
//!
//! The plugin starts the helper itself (Node's child_process.spawn, an argument array, no shell) the first time a
//! check needs it, from the helper program in the plugin's folder or the one set for this computer; or adopts a helper
//! already answering on its port (the MCP server's, say) once it has said it is Authenticity's helper. The helper's
//! input stays open while the plugin runs: when Obsidian closes, the helper stops what it started and exits. Every
//! setting it starts with is this computer's (local storage), never the vault's.
//!
//! References: <https://nodejs.org/api/child_process.html#child_processspawncommand-args-options>,
//! <https://docs.obsidian.md/Reference/TypeScript+API/requestUrl>.

use js_sys::Array;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

use crate::app::{port, with, App, Device};
use crate::js::{self, call, f1, get, obj, set, str};
use crate::sloptotal::{self, Reply};

/// The helper protocol this plugin speaks (the helper's PROTOCOL).
pub const PROTOCOL: u64 = 1;
/// How long SlopTotal may take to load its models before a check gives up (the first start on the CPU is slow).
const LOAD_WAIT_MS: f64 = 10.0 * 60.0 * 1000.0;

#[derive(Default)]
pub struct State {
    /// The helper this plugin started (a ChildProcess), if any.
    child: Option<JsValue>,
    /// The helper's last /health, as read (allow-listed fields only).
    pub last: Option<Health>,
    /// Why the helper could not be used, the last time it was tried.
    pub problem: Option<String>,
    /// SlopTotal's detector weights (its calibration), once the helper has given them.
    pub weights: Option<sloptotal::Weights>,
}

/// What the helper says about itself and SlopTotal (read field by field: the port may be someone else's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Health {
    pub version: String,
    /// "stopped", "starting", "running" or "failed".
    pub state: String,
    /// "cuda", "cpu" or "" (a SlopTotal the helper did not start: its own choice).
    pub device: String,
    pub why: String,
    pub ours: bool,
    pub loaded: u32,
    pub total: u32,
    pub failed: Vec<String>,
    /// How many paragraph requests to send at once now (the helper's choice from the device and the computer's load).
    pub parallel: u32,
}

fn word(v: &Value, allowed: &[&str]) -> String {
    v.as_str().filter(|s| allowed.contains(s)).unwrap_or_default().to_string()
}

/// The helper's /health, if it is Authenticity's helper speaking this protocol.
pub fn parse_health(v: &Value) -> Result<Health, String> {
    if v["service"].as_str() != Some("authenticity-helper") {
        return Err("another program answers on the helper's port (Settings → Authenticity → Helper port)".into());
    }
    if v["protocol"].as_u64() != Some(PROTOCOL) {
        return Err(format!("the helper speaks protocol {}, this plugin {PROTOCOL}: update the helper program", v["protocol"]));
    }
    let s = &v["sloptotal"];
    let e = &s["engines"];
    Ok(Health {
        version: v["version"].as_str().unwrap_or_default().chars().take(20).collect(),
        state: word(&s["state"], &["stopped", "starting", "running", "failed"]),
        device: word(&s["device"], &["cuda", "cpu"]),
        why: s["why"].as_str().unwrap_or_default().chars().take(400).collect(),
        ours: s["ours"].as_bool().unwrap_or(false),
        loaded: e["loaded"].as_u64().unwrap_or(0).min(1000) as u32,
        total: e["total"].as_u64().unwrap_or(0).min(1000) as u32,
        failed: e["failed"].as_array().map(|a| a.iter().take(50).filter_map(|x| x.as_str()).map(|x| x.chars().take(40).collect()).collect()).unwrap_or_default(),
        parallel: s["parallel"].as_u64().unwrap_or(1).clamp(1, sloptotal::QUICK_LANE as u64) as u32,
    })
}

impl State {
    /// Stop the helper this plugin started: its input is closed (it then stops SlopTotal and exits), and it is killed
    /// if it is still there a few seconds later.
    pub fn release(&mut self) {
        if let Some(child) = self.child.take() {
            let stdin = get(&child, "stdin");
            if !stdin.is_null() && !stdin.is_undefined() {
                call!(stdin, end);
            }
            js::timeout(5000.0, move || {
                if get(&child, "exitCode").is_null() {
                    call!(child, kill);
                }
            });
        }
        self.last = None;
    }

    pub fn device_in_use(&self) -> String {
        self.last.as_ref().map(|h| h.device.clone()).unwrap_or_default()
    }

    /// The status line under Settings → Writing checks.
    pub fn describe(&self, d: &Device) -> String {
        if d.sloptotal_folder.trim().is_empty() {
            return "SlopTotal folder not set: the AI-writing check is unavailable until it is (the installation guide says how to install SlopTotal).".into();
        }
        // the folder as it will be used, when the setting names variables (%LOCALAPPDATA%, ~)
        let folder = match expanded_folder(d) {
            Err(e) => return format!("The SlopTotal folder cannot be used: {e}."),
            Ok(f) if crate::paths::has_variables(&d.sloptotal_folder, windows()) => format!("Folder: {f}. "),
            Ok(_) => String::new(),
        };
        format!("{folder}{}", self.describe_state())
    }

    fn describe_state(&self) -> String {
        if let Some(p) = &self.problem {
            return format!("Not available: {p}");
        }
        match &self.last {
            None => "SlopTotal is started when you run a check, and stopped when idle.".into(),
            Some(h) => {
                let dev = match h.device.as_str() {
                    "cuda" => " on the graphics card",
                    "cpu" => " on the processor",
                    _ => "",
                };
                let who = if h.ours { "" } else { " (started outside Authenticity: used as it is, never stopped)" };
                match h.state.as_str() {
                    "running" => format!("SlopTotal is running{dev}{who}, with {} detectors.", h.total),
                    "starting" => format!("SlopTotal is starting{dev} (loading its models)."),
                    "failed" => format!("SlopTotal could not start: {}", h.why),
                    _ => if h.why.is_empty() { "SlopTotal is stopped; it starts when you run a check.".to_string() } else { format!("SlopTotal is stopped ({}).", h.why) },
                }
            }
        }
    }
}

fn windows() -> bool {
    str(&get(&get(&js::global(), "process"), "platform")) == "win32"
}

/// The SlopTotal folder as set, its variables expanded (paths.rs) from this computer's environment (Node's
/// process.env), as the helper will expand them from the same environment.
pub fn expanded_folder(d: &Device) -> Result<String, String> {
    let env = get(&get(&js::global(), "process"), "env");
    crate::paths::expand(&d.sloptotal_folder, |k| Some(str(&get(&env, k))).filter(|v| !v.is_empty()), windows())
}

/// The helper's address on this computer.
fn base(d: &Device) -> String {
    format!("http://127.0.0.1:{}", port(d.helper_port, 8791))
}

/// A request to the helper (requestUrl: no cross-origin limits, and Obsidian sends no browser origin).
async fn request(method: &str, url: &str, body: Option<String>, timeout_ms: f64) -> Reply {
    let Some(request_url) = with(|a| get(&a.obs, "requestUrl")) else { return Reply { status: 0, body: "the plugin is unloading".into() } };
    let req = obj! { url: url, method: method, throw: false };
    if let Some(b) = body {
        set(&req, "contentType", "application/json");
        set(&req, "body", b);
    }
    let call = js::apply(&request_url, &JsValue::NULL, &[req]);
    let timer = js_sys::Promise::new(&mut |resolve, _| {
        js::timeout(timeout_ms, move || _ = resolve.call1(&JsValue::NULL, &JsValue::from_str("\u{0}timeout")));
    });
    match js::wait(js_sys::Promise::race(&Array::of2(&call, &timer)).into()).await {
        Ok(v) if v.as_string().as_deref() == Some("\u{0}timeout") => Reply { status: 0, body: "no answer in time".into() },
        Ok(res) => Reply { status: get(&res, "status").as_f64().unwrap_or(0.0) as u16, body: str(&get(&res, "text")) },
        Err(e) => Reply { status: 0, body: str(&get(&e, "message")) },
    }
}

/// The helper's health, if Authenticity's helper answers.
async fn health(d: &Device) -> Result<Health, String> {
    let r = request("GET", &format!("{}/health", base(d)), None, 3000.0).await;
    if r.status != 200 {
        return Err(if r.status == 0 { "not running".into() } else { format!("answered {}", r.status) });
    }
    parse_health(&serde_json::from_str(&r.body).unwrap_or(Value::Null))
}

/// Ask the helper how things are, for the settings' status line.
pub async fn refresh() {
    let Some(d) = with(|a| a.device.clone()) else { return };
    let h = health(&d).await;
    with(|a| match h {
        Ok(h) => {
            a.helper.last = Some(h);
            a.helper.problem = None;
        }
        Err(_) => a.helper.last = None,
    });
}

/// The helper program for this computer: the one set (this computer's setting), else the one in the plugin's folder.
fn program(a: &App) -> Result<String, String> {
    if !a.device.helper_path.trim().is_empty() {
        return Ok(a.device.helper_path.trim().to_string());
    }
    let adapter = get(&get(&a.app, "vault"), "adapter");
    let basepath = call!(adapter, getBasePath);
    if basepath.is_undefined() {
        return Err("this vault is not on this computer's disk".into());
    }
    let windows = str(&get(&get(&js::global(), "process"), "platform")) == "win32";
    let sep = if windows { "\\" } else { "/" };
    let dir = a.dir.replace('/', sep);
    Ok(format!("{}{sep}{dir}{sep}authenticity-helper{}", str(&basepath), if windows { ".exe" } else { "" }))
}

/// The arguments the helper starts with: this computer's settings only.
pub fn args(d: &Device) -> Vec<String> {
    let device = match d.device.as_str() {
        "gpu" | "cpu" => d.device.clone(),
        _ => "auto".into(),
    };
    vec![
        "--port".into(),
        port(d.helper_port, 8791).to_string(),
        "--sloptotal".into(),
        d.sloptotal_folder.trim().to_string(),
        "--sloptotal-port".into(),
        port(d.sloptotal_port, 8000).to_string(),
        "--device".into(),
        device,
        "--idle-minutes".into(),
        (d.idle_minutes.clamp(1.0, 24.0 * 60.0) as u32).to_string(),
        "--exit-with-stdin".into(),
    ]
}

/// The helper, answering: one already running, else one this plugin starts now.
async fn ensure_helper() -> Result<Device, String> {
    let Some(d) = with(|a| a.device.clone()) else { return Err("the plugin is unloading".into()) };
    if d.sloptotal_folder.trim().is_empty() {
        return Err("set the SlopTotal folder first (Settings → Authenticity → Writing checks); the installation guide says how to install SlopTotal".into());
    }
    expanded_folder(&d).map_err(|e| format!("the SlopTotal folder (Settings → Authenticity → Writing checks): {e}"))?;
    match health(&d).await {
        Ok(h) => {
            with(|a| a.helper.last = Some(h));
            return Ok(d);
        }
        Err(e) if e != "not running" => return Err(format!("the helper's port {}: {e}", port(d.helper_port, 8791))),
        Err(_) => {}
    }
    if !js::desktop() {
        return Err("the helper runs only in desktop Obsidian".into());
    }
    let spawned = with(|a| -> Result<(), String> {
        let path = program(a)?;
        let fs = js::apply(&a.require, &JsValue::NULL, &["fs".into()]);
        if !call!(fs, existsSync, path.clone()).is_truthy() {
            return Err(format!("no helper program at {path}: put authenticity-helper from the release there, or set Helper program for this computer"));
        }
        let cp = js::apply(&a.require, &JsValue::NULL, &["child_process".into()]);
        let argv: Array = args(&a.device).into_iter().map(JsValue::from).collect();
        let stdio: Array = ["pipe", "ignore", "pipe"].iter().map(|s| JsValue::from(*s)).collect();
        let child = call!(cp, spawn, path, argv, obj! { windowsHide: true, stdio: stdio });
        let tail = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let t = tail.clone();
        call!(get(&child, "stderr"), on, "data", f1(move |d| {
            let mut e = t.borrow_mut();
            e.push_str(&str(&d));
            let cut = e.ceil_char_boundary(e.len().saturating_sub(2000));
            e.drain(..cut);
            JsValue::UNDEFINED
        }));
        let me = child.clone();
        call!(child, on, "exit", f1(move |code| {
            let last = tail.borrow().lines().last().unwrap_or("").to_string();
            with(|a| {
                if a.helper.child.as_ref() == Some(&me) {
                    a.helper.child = None;
                    a.helper.last = None;
                    if code.as_f64().is_some_and(|c| c != 0.0) {
                        a.helper.problem = Some(format!("the helper stopped (exit {}): {last}", code.as_f64().unwrap_or(0.0)));
                    }
                }
            });
            JsValue::UNDEFINED
        }));
        call!(child, on, "error", f1(|e| {
            with(|a| a.helper.problem = Some(format!("the helper could not start: {}", str(&get(&e, "message")))));
            JsValue::UNDEFINED
        }));
        a.helper.child = Some(child);
        a.helper.problem = None;
        Ok(())
    });
    spawned.unwrap_or(Err("the plugin is unloading".into()))?;
    // it answers within a second or two
    for _ in 0..40 {
        js::sleep(250.0).await;
        if let Ok(h) = health(&d).await {
            with(|a| a.helper.last = Some(h));
            return Ok(d);
        }
        if let Some(Some(p)) = with(|a| a.helper.problem.clone()) {
            return Err(p);
        }
    }
    Err("the helper did not answer within 10 seconds".into())
}

/// SlopTotal running and ready: started through the helper (asking the GPU question if the helper asks it), then
/// waited for until its detectors are loaded. `task` names the check, for the GPU question.
pub async fn ensure_sloptotal(task: &str) -> Result<(), String> {
    ensure_sloptotal_for(task, Some(crate::app::run_now())).await
}

/// The same, for a check the panes' Cancel stops (`run`) or not (None: another plugin's).
pub async fn ensure_sloptotal_for(task: &str, run: Option<u64>) -> Result<(), String> {
    let d = ensure_helper().await?;
    let url = format!("{}/sloptotal/start", base(&d));
    let mut gpu = "";
    loop {
        with(|a| a.busy(Some("AI check: starting SlopTotal\u{2026}".into())));
        let r = request("POST", &url, Some(json!({ "gpu": gpu }).to_string()), 60_000.0).await;
        let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
        if r.status != 200 {
            return Err(v["error"].as_str().map(|s| s.chars().take(400).collect()).unwrap_or_else(|| sloptotal::error(&r)));
        }
        if let Some(short) = v.get("gpu_shortage").filter(|s| s.is_object()) {
            // the shared question, as other GPU plugins ask it
            let s = obsidian_kit::gpu::Shortage {
                need_mb: short["need_mb"].as_u64().unwrap_or(0),
                free_mb: short["free_mb"].as_u64().unwrap_or(0),
                others_mb: short["others_mb"].as_u64().unwrap_or(0),
                holders: short["holders"].as_array().map(|a| a.iter().take(8).filter_map(|h| h.as_str()).map(|h| h.chars().take(40).collect()).collect()).unwrap_or_default(),
            };
            let asking = obsidian_kit::gpu::Asking { plugin: "Authenticity", models: "SlopTotal's detectors", task, on_cpu: "Check on the CPU", decline: "Cancel" };
            let Some((obs, app)) = with(|a| (a.obs.clone(), a.app.clone())) else { return Err("the plugin is unloading".into()) };
            match obsidian_kit::gpu::choose(&obs, &app, &s, &asking).await {
                Some(choice) => {
                    gpu = choice;
                    continue;
                }
                None => return Err("cancelled".into()),
            }
        }
        break;
    }
    // wait for the detectors to load
    let start = js::now();
    loop {
        let h = health(&d).await?;
        let (state, why) = (h.state.clone(), h.why.clone());
        let failed = h.failed.clone();
        with(|a| a.helper.last = Some(h));
        match state.as_str() {
            "running" if !failed.is_empty() => return Err(sloptotal::failed_message(&failed)),
            "running" => return Ok(()),
            "failed" => return Err(format!("SlopTotal could not start: {why}")),
            "stopped" => return Err(if why.is_empty() { "SlopTotal stopped while starting; its window or log says why".into() } else { why }),
            _ => {}
        }
        // Cancel stops the waiting, not SlopTotal (it goes on loading; Stop SlopTotal now stops it)
        if run.is_some_and(crate::app::cancelled) {
            return Err(sloptotal::CANCELLED.into());
        }
        if js::now() - start > LOAD_WAIT_MS {
            return Err("SlopTotal had not loaded its detectors after 10 minutes; start it by hand once (the installation guide, step 4) to see why".into());
        }
        let secs = ((js::now() - start) / 1000.0) as u32;
        with(|a| a.busy(Some(format!("AI check: SlopTotal is loading its models ({secs} s)\u{2026}"))));
        js::sleep(1500.0).await;
    }
}

/// A check has just used SlopTotal (the helper counts its idle time from its last request anyway).
pub fn touched() {}

/// SlopTotal's detector weights, from the helper (which reads SlopTotal's calibration.json and its detector names),
/// once; read field by field like every answer.
pub async fn load_weights() {
    let Some((d, have)) = with(|a| (a.device.clone(), a.helper.weights.is_some())) else { return };
    if have {
        return;
    }
    let r = request("GET", &format!("{}/weights", base(&d)), None, 5000.0).await;
    if r.status != 200 {
        return;
    }
    let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
    let mut w = sloptotal::parse_calibration(&v["calibration"]);
    w.names = sloptotal::parse_engines(&v["engines"]);
    if !w.english.is_empty() || !w.multilingual.is_empty() {
        with(|a| a.helper.weights = Some(w));
    }
}

/// Stop SlopTotal now, if the helper started it.
pub async fn stop_sloptotal() {
    let Some(d) = with(|a| a.device.clone()) else { return };
    let r = request("POST", &format!("{}/sloptotal/stop", base(&d)), Some("{}".into()), 15_000.0).await;
    let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
    with(|a| {
        a.notice(match (r.status, v["stopped"].as_bool()) {
            (0, _) => "SlopTotal is not running.",
            (200, Some(true)) => "SlopTotal stopped.",
            (200, _) => "SlopTotal was not started by Authenticity, so it is left running.",
            _ => "the helper could not stop SlopTotal.",
        });
        a.helper.last = None;
    });
}

/// SlopTotal's API through the helper, which sends the text alone.
pub struct Proxy {
    base: String,
    /// The run number of the check it serves (None: one the panes' Cancel does not stop).
    run: Option<u64>,
}

/// The helper's way to SlopTotal for a check the panes' Cancel can stop (it takes the run number now).
pub fn proxy() -> Proxy {
    Proxy { base: with(|a| format!("{}/sloptotal", base(&a.device))).unwrap_or_default(), run: Some(crate::app::run_now()) }
}

/// The same for a check another plugin asked for (the API): the panes' Cancel does not stop it.
pub fn proxy_detached() -> Proxy {
    Proxy { run: None, ..proxy() }
}

impl sloptotal::Env for Proxy {
    async fn get(&self, path: &str) -> Reply {
        request("GET", &format!("{}{path}", self.base), None, 30_000.0).await
    }
    async fn post(&self, path: &str, body: String) -> Reply {
        // analysis on the processor can take minutes; the helper answers 202 with a ticket long before that
        let (bytes, start) = (body.len(), js::now());
        let r = request("POST", &format!("{}{path}", self.base), Some(body), 15.0 * 60.0 * 1000.0).await;
        with(|a| crate::journal::debug(a, "request", json!({ "path": path, "bytes": bytes, "status": r.status, "ms": (js::now() - start).round(), "answer_bytes": r.body.len() }), None));
        r
    }
    async fn sleep(&self, ms: u32) {
        js::sleep(ms as f64).await
    }
    fn cancelled(&self) -> bool {
        self.run.is_some_and(crate::app::cancelled)
    }
    fn parallel(&self) -> usize {
        with(|a| a.helper.last.as_ref().map(|h| h.parallel as usize)).flatten().unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_is_told_only_this_computers_settings() {
        let d = Device { sloptotal_folder: " C:\\Tools\\SlopTotal \"; calc".into(), sloptotal_port: 8000.0, device: "weird".into(), helper_path: String::new(), helper_port: 70000.0, idle_minutes: 0.0 };
        let a = args(&d);
        assert_eq!(a, ["--port", "8791", "--sloptotal", "C:\\Tools\\SlopTotal \"; calc", "--sloptotal-port", "8000", "--device", "auto", "--idle-minutes", "1", "--exit-with-stdin"]);
    }

    #[test]
    fn health_is_read_field_by_field() {
        let v = json!({ "service": "authenticity-helper", "protocol": 1, "version": "0.2.0", "sloptotal": { "state": "running", "device": "cpu", "ours": true, "engines": { "loaded": 23, "total": 23, "failed": [] }, "path": "C:\\evil.exe" } });
        let h = parse_health(&v).unwrap();
        assert_eq!((h.state.as_str(), h.device.as_str(), h.ours, h.loaded), ("running", "cpu", true, 23));
        assert!(parse_health(&json!({ "service": "another-service" })).unwrap_err().contains("another program"));
        assert!(parse_health(&json!({ "service": "authenticity-helper", "protocol": 9 })).unwrap_err().contains("protocol 9"));
        let odd = json!({ "service": "authenticity-helper", "protocol": 1, "sloptotal": { "state": "<script>", "device": "tpu" } });
        let h = parse_health(&odd).unwrap();
        assert_eq!((h.state.as_str(), h.device.as_str()), ("", ""));
    }
}
