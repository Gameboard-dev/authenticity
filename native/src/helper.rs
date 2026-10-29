//! Authenticity's helper: a small server on 127.0.0.1 that runs SlopTotal for the plugin and the MCP server.
//!
//! - `GET /health`: the helper (`service: authenticity-helper`, `protocol`) and SlopTotal's state;
//! - `POST /sloptotal/start` `{gpu: "" | "free" | "cpu"}`: start SlopTotal if it is not running (a SlopTotal already
//!   answering on its port is used as it is and never stopped); the device as device.rs decides, answering
//!   `{gpu_shortage}` when the user must be asked first;
//! - `POST /sloptotal/stop`: stop the SlopTotal this helper started;
//! - `GET /sloptotal/health`, `POST /sloptotal/api/{quick-score,analyze}`,
//!   `GET /sloptotal/api/queue/ticket/{id}`, `GET /sloptotal/api/engines`: passed to SlopTotal, a POST's body rebuilt
//!   as `{"text": ...}` alone, so nothing else (a `url`, which makes SlopTotal fetch a page) ever reaches it;
//! - `GET /weights`: SlopTotal's calibration.json (read-only, from its folder) and its detectors' names;
//! - `POST /gpu/release`: another of the user's services needs the GPU: SlopTotal, which cannot unload its models, is
//!   stopped if this helper started it on the GPU (the claim protocol of claims.rs).
//!
//! It stops SlopTotal after the idle time, when its input closes (`--exit-with-stdin`: the plugin's pipe, so Obsidian
//! closing stops it), and never runs anything but SlopTotal's own Python from the folder it was started with.
//! Every request is checked by http::allowed (this computer only, no web page).

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use authenticity_core::sloptotal;
use serde_json::{json, Value};

use crate::{claims, device, http, launch};

pub const PROTOCOL: u64 = 1;
/// How long SlopTotal may take to become healthy after it starts.
const START_WAIT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub folder: String,
    pub sloptotal_port: u16,
    pub mode: device::Mode,
    pub idle: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Starting,
    Running,
    Failed,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Stopped => "stopped",
            State::Starting => "starting",
            State::Running => "running",
            State::Failed => "failed",
        }
    }
}

struct Slop {
    state: State,
    device: String,
    /// What the other services' GPU claims held when SlopTotal started (device::parallel backs off past it).
    others_mb_at_start: u64,
    why: String,
    child: Option<Child>,
    last_used: Instant,
    health: sloptotal::Health,
}

pub struct Helper {
    pub cfg: Config,
    slop: Mutex<Slop>,
}

fn lock(h: &Helper) -> std::sync::MutexGuard<'_, Slop> {
    h.slop.lock().unwrap_or_else(|e| e.into_inner())
}

impl Helper {
    pub fn new(cfg: Config) -> Arc<Helper> {
        Arc::new(Helper { cfg, slop: Mutex::new(Slop { state: State::Stopped, device: String::new(), others_mb_at_start: 0, why: String::new(), child: None, last_used: Instant::now(), health: Default::default() }) })
    }

    fn sloptotal(&self, method: &str, path: &str, body: Option<&str>, timeout: Duration) -> Result<http::Reply, String> {
        http::request(self.cfg.sloptotal_port, method, path, body, timeout)
    }

    /// SlopTotal's health if it answers (as SlopTotal); Err(None) if nothing answers, Err(Some) if something else does.
    fn probe(&self) -> Result<sloptotal::Health, Option<String>> {
        let r = self.sloptotal("GET", "/health", None, Duration::from_secs(5)).map_err(|_| None)?;
        if r.status != 200 {
            return Err(Some(format!("the program on port {} answered {} to /health: it is not SlopTotal (set another SlopTotal port)", self.cfg.sloptotal_port, r.status)));
        }
        sloptotal::parse_health(&r.body).map_err(Some)
    }

    pub fn health_json(&self) -> Value {
        let parallel = self.parallel();
        let s = lock(self);
        json!({ "service": "authenticity-helper", "protocol": PROTOCOL, "version": env!("CARGO_PKG_VERSION"),
            "sloptotal": { "state": s.state.name(), "device": s.device, "why": s.why, "ours": s.child.is_some(), "port": self.cfg.sloptotal_port,
                "engines": { "loaded": s.health.loaded, "total": s.health.total, "failed": s.health.failed }, "parallel": parallel },
            "folder_set": !self.cfg.folder.trim().is_empty() })
    }

    /// How many paragraph requests the plugin sends at once now (device::parallel), from readings at most 5 s old (the
    /// CPU reading takes 200 ms, and /health is asked often).
    fn parallel(&self) -> usize {
        static LAST: Mutex<Option<(Instant, usize)>> = Mutex::new(None);
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, n)) = *last {
            if t.elapsed() < Duration::from_secs(5) {
                return n;
            }
        }
        let (device, at_start) = {
            let s = lock(self);
            (s.device.clone(), s.others_mb_at_start)
        };
        let gpu = device == "cuda";
        let load = device::Load {
            cores: native_kit::hw::cpu_cores(),
            busy: if gpu { 0.0 } else { native_kit::hw::cpu_busy() },
            gpu_free_mb: if gpu { native_kit::hw::device_free_mb() } else { None },
            others_mb: if gpu { claims::others().iter().map(|c| c.mb).sum() } else { 0 },
            others_mb_at_start: at_start,
        };
        let n = device::parallel(&device, load);
        *last = Some((Instant::now(), n));
        n
    }

    /// The facts device.rs decides by.
    fn facts(&self, folder: &std::path::Path) -> device::Facts {
        let others = claims::others();
        let total = native_kit::hw::stats(true)["gpu"]["mem_total_mb"].as_u64();
        device::Facts { cuda_torch: device::cuda_torch(folder), total_mb: total, free_mb: native_kit::hw::device_free_mb(), others_mb: others.iter().map(|c| c.mb).sum(), holders: others.iter().map(|c| c.display().to_string()).collect() }
    }

    /// Start SlopTotal (see the module's notes). The answer for /sloptotal/start.
    pub fn start(self: &Arc<Self>, answer: &str) -> Result<Value, String> {
        {
            let mut s = lock(self);
            s.last_used = Instant::now();
            if matches!(s.state, State::Starting | State::Running) {
                return Ok(json!({ "state": s.state.name(), "device": s.device }));
            }
        }
        match self.probe() {
            Ok(h) => {
                let mut s = lock(self);
                *s = Slop { state: State::Running, device: String::new(), others_mb_at_start: 0, why: "started outside Authenticity: used as it is".into(), child: None, last_used: Instant::now(), health: h };
                return Ok(json!({ "state": "running", "device": "" }));
            }
            Err(Some(e)) => return Err(e),
            Err(None) => {}
        }
        let folder = launch::check_folder(&self.cfg.folder)?;
        let mut decision = device::decide(self.cfg.mode, &self.facts(&folder), answer);
        if decision == device::Decision::Release {
            let need = device::SLOPTOTAL_GPU_MB + device::MARGIN_MB;
            decision = if claims::release_others(need, Duration::from_secs(20)) {
                device::Decision::Gpu
            } else {
                device::Decision::Cpu("the graphics card still had too little free memory after your other services were asked to free theirs".into())
            };
        }
        let (dev, why) = match decision {
            device::Decision::Ask(short) => return Ok(json!({ "gpu_shortage": short })),
            device::Decision::Gpu => ("cuda", String::new()),
            device::Decision::Cpu(why) => ("cpu", why),
            device::Decision::Release => unreachable!("decided above"),
        };
        let spec = launch::spec(&folder, self.cfg.sloptotal_port, dev);
        let child = launch::start(&spec)?;
        if dev == "cuda" {
            claims::hold(self.cfg.port, device::SLOPTOTAL_GPU_MB);
        }
        eprintln!("authenticity-helper: started SlopTotal (process {}) on the {}{}", child.id(), if dev == "cuda" { "GPU" } else { "CPU" }, if why.is_empty() { String::new() } else { format!(": {why}") });
        {
            let mut s = lock(self);
            let others_mb_at_start = claims::others().iter().map(|c| c.mb).sum();
            *s = Slop { state: State::Starting, device: dev.into(), others_mb_at_start, why, child: Some(child), last_used: Instant::now(), health: Default::default() };
        }
        let me = self.clone();
        std::thread::spawn(move || me.wait_ready());
        Ok(json!({ "state": "starting", "device": dev }))
    }

    /// Wait until SlopTotal says it is healthy with no failed detector (not until "loaded" reaches "total": that counts
    /// the detectors that have run), or it exits, or START_WAIT passes.
    fn wait_ready(&self) {
        let t = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            {
                let mut s = lock(self);
                if s.state != State::Starting {
                    return;
                }
                if let Some(Ok(Some(code))) = s.child.as_mut().map(|c| c.try_wait()) {
                    s.child = None;
                    s.state = State::Failed;
                    s.why = format!("SlopTotal stopped as it started (exit {}): {}", code.code().unwrap_or(-1), launch::log_tail(4));
                    claims::drop_claim();
                    return;
                }
            }
            if let Ok(h) = self.probe() {
                let mut s = lock(self);
                if !h.failed.is_empty() {
                    s.why = sloptotal::failed_message(&h.failed);
                }
                // healthy is not enough for a SlopTotal this helper started: its models must also be loaded (its log
                // says so), so no check races the loading
                let loaded = s.child.is_none() || launch::preloaded();
                if (h.healthy && loaded) || !h.failed.is_empty() {
                    s.state = State::Running;
                    s.health = h;
                    s.last_used = Instant::now();
                    return;
                }
            }
            if t.elapsed() > START_WAIT {
                self.stop("SlopTotal did not become ready in 10 minutes");
                lock(self).state = State::Failed;
                return;
            }
        }
    }

    /// Stop the SlopTotal this helper started (a SlopTotal it found running is left alone). Whether one was stopped.
    pub fn stop(&self, why: &str) -> bool {
        let mut s = lock(self);
        let Some(mut child) = s.child.take() else {
            if s.state == State::Running {
                s.state = State::Stopped; // forget an adopted one; it keeps running
            }
            return false;
        };
        launch::stop(&mut child);
        claims::drop_claim();
        eprintln!("authenticity-helper: stopped SlopTotal ({why})");
        *s = Slop { state: State::Stopped, device: String::new(), others_mb_at_start: 0, why: why.into(), child: None, last_used: Instant::now(), health: Default::default() };
        true
    }

    /// Stop SlopTotal once it has been idle for the idle time; notice when an adopted or started one has gone.
    pub fn watch(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(10));
            let (idle, running, ours) = {
                let mut s = lock(&me);
                if let Some(Ok(Some(_))) = s.child.as_mut().map(|c| c.try_wait()) {
                    s.child = None;
                    s.state = State::Stopped;
                    s.why = format!("SlopTotal stopped by itself: {}", launch::log_tail(3));
                    claims::drop_claim();
                }
                (s.last_used.elapsed(), s.state == State::Running, s.child.is_some())
            };
            if ours && running && idle > me.cfg.idle {
                me.stop("idle");
            }
        });
    }

    /// The helper's own GPU release: SlopTotal cannot unload, so it is stopped (if this helper started it there).
    pub fn release_gpu(&self) -> bool {
        let on_gpu = lock(self).device == "cuda";
        on_gpu && self.stop("another service needs the GPU")
    }

    fn touch(&self) {
        lock(self).last_used = Instant::now();
    }

    /// A request passed to SlopTotal: only the allow-listed paths, a POST's body rebuilt as the text alone.
    pub fn proxy(&self, method: &str, path: &str, body: &str) -> (u16, String) {
        let refuse = |s: u16, e: &str| (s, json!({ "error": e }).to_string());
        let rest = path.strip_prefix("/sloptotal").unwrap_or("");
        let ok_path = match (method, rest) {
            ("GET", "/health") | ("GET", "/api/engines") => true,
            ("POST", "/api/quick-score" | "/api/analyze") => true,
            ("GET", p) if p.starts_with("/api/queue/ticket/") => p["/api/queue/ticket/".len()..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') && p.len() > 18 && p.len() <= 18 + 64,
            _ => false,
        };
        if !ok_path {
            return refuse(404, "not something the helper passes to SlopTotal");
        }
        let body = if method == "POST" {
            let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            match v["text"].as_str() {
                Some(t) => Some(sloptotal::body(t)),
                None => return refuse(400, "send {\"text\": ...}"),
            }
        } else {
            None
        };
        if rest != "/health" && lock(self).state != State::Running {
            return refuse(503, "SlopTotal is not running: start it first (POST /sloptotal/start)");
        }
        self.touch();
        let timeout = if rest == "/api/analyze" { Duration::from_secs(900) } else { Duration::from_secs(120) };
        let r = self.sloptotal(method, rest, body.as_deref(), timeout);
        self.touch();
        match r {
            Ok(r) => (r.status, r.body),
            Err(e) => refuse(502, &format!("SlopTotal is not answering: {e}")),
        }
    }

    /// SlopTotal's calibration and detector names, for ordering the reasons (read-only).
    pub fn weights(&self) -> Value {
        let cal = launch::check_folder(&self.cfg.folder).ok().and_then(|f| std::fs::read(f.join("app").join("calibration.json")).ok()).filter(|b| b.len() < (1 << 20)).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
        let engines = self.sloptotal("GET", "/api/engines", None, Duration::from_secs(10)).ok().filter(|r| r.status == 200).and_then(|r| serde_json::from_str::<Value>(&r.body).ok()).unwrap_or(Value::Null);
        json!({ "calibration": cal, "engines": engines })
    }

    /// Answer one request.
    pub fn handle(self: &Arc<Self>, req: &http::Request) -> (u16, String) {
        if let Err(e) = http::allowed(req, self.cfg.port) {
            return (403, json!({ "error": e }).to_string());
        }
        let path = req.path.split('?').next().unwrap_or("");
        match (req.method.as_str(), path) {
            ("GET", "/health") => (200, self.health_json().to_string()),
            ("GET", "/weights") => (200, self.weights().to_string()),
            ("POST", "/sloptotal/start") => {
                let v: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
                let answer = v["gpu"].as_str().filter(|a| matches!(*a, "free" | "cpu")).unwrap_or_default();
                match self.start(answer) {
                    Ok(v) => (200, v.to_string()),
                    Err(e) => {
                        let mut s = lock(self);
                        s.state = State::Failed;
                        s.why = e.clone();
                        (409, json!({ "error": e }).to_string())
                    }
                }
            }
            ("POST", "/sloptotal/stop") => (200, json!({ "stopped": self.stop("asked to stop") }).to_string()),
            ("POST", "/gpu/release") => (200, json!({ "released": self.release_gpu() }).to_string()),
            (m, p) if p.starts_with("/sloptotal/") => self.proxy(m, p, &req.body),
            _ => (404, json!({ "error": "no such request" }).to_string()),
        }
    }

    fn connection(self: &Arc<Self>, mut s: TcpStream) {
        let (status, body) = match http::read_request(&mut s) {
            Ok(req) => self.handle(&req),
            Err((status, e)) => (status, json!({ "error": e }).to_string()),
        };
        http::respond(&mut s, status, &body);
    }

    /// Serve on 127.0.0.1:port, in threads. Err if the port is taken.
    pub fn serve(self: &Arc<Self>) -> Result<std::thread::JoinHandle<()>, String> {
        let l = TcpListener::bind(("127.0.0.1", self.cfg.port)).map_err(|e| format!("port {} on 127.0.0.1: {e}", self.cfg.port))?;
        self.watch();
        let me = self.clone();
        Ok(std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let me = me.clone();
                std::thread::spawn(move || me.connection(s));
            }
        }))
    }
}

/// A helper's health, if one answers on `port` (another program there: Err).
pub fn answering(port: u16) -> Result<bool, String> {
    match http::request(port, "GET", "/health", None, Duration::from_secs(3)) {
        Err(_) => Ok(false),
        Ok(r) => {
            let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
            if v["service"] == "authenticity-helper" && v["protocol"] == PROTOCOL {
                Ok(true)
            } else {
                Err(format!("another program answers on port {port}"))
            }
        }
    }
}

/// SlopTotal's usual folder on this computer (the installation guide's), if it is there.
pub fn default_folder() -> Option<PathBuf> {
    let f = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("SlopTotal");
    f.join("app").join("main.py").is_file().then_some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper() -> Arc<Helper> {
        // a SlopTotal port where nothing listens
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        Helper::new(Config { port: 8791, folder: String::new(), sloptotal_port: free, mode: device::Mode::Auto, idle: Duration::from_secs(600) })
    }

    fn get(h: &Arc<Helper>, path: &str) -> (u16, Value) {
        let req = http::Request { method: "GET".into(), path: path.into(), headers: vec![("host".into(), "127.0.0.1:8791".into())], body: String::new() };
        let (s, b) = h.handle(&req);
        (s, serde_json::from_str(&b).unwrap())
    }

    fn post(h: &Arc<Helper>, path: &str, body: &str) -> (u16, Value) {
        let req = http::Request { method: "POST".into(), path: path.into(), headers: vec![("host".into(), "127.0.0.1:8791".into()), ("content-type".into(), "application/json".into())], body: body.into() };
        let (s, b) = h.handle(&req);
        (s, serde_json::from_str(&b).unwrap())
    }

    #[test]
    fn health_and_refusals() {
        let h = helper();
        let (s, v) = get(&h, "/health");
        assert_eq!((s, v["service"].as_str(), v["protocol"].as_u64(), v["sloptotal"]["state"].as_str()), (200, Some("authenticity-helper"), Some(PROTOCOL), Some("stopped")));
        // without a folder: refused with what to do, and nothing started
        let (s, v) = post(&h, "/sloptotal/start", "{}");
        assert_eq!(s, 409);
        assert!(v["error"].as_str().unwrap().contains("no SlopTotal folder"));
        // only allow-listed paths reach SlopTotal; a url field never does
        assert_eq!(post(&h, "/sloptotal/api/scan-urls", "{\"urls\":[]}").0, 404);
        assert_eq!(post(&h, "/sloptotal/api/quick-score", "{\"url\":\"https://example.com\"}").0, 400);
        assert_eq!(get(&h, "/sloptotal/api/queue/ticket/..%2F..%2Fx").0, 404);
        assert_eq!(post(&h, "/sloptotal/api/quick-score", "{\"text\":\"some words\"}").0, 503, "not running");
        // nothing to release, nothing to stop
        assert_eq!(post(&h, "/gpu/release", "").1["released"], false);
        assert_eq!(post(&h, "/sloptotal/stop", "{}").1["stopped"], false);
    }

    #[test]
    fn requests_from_web_pages_are_refused() {
        let h = helper();
        let req = http::Request { method: "GET".into(), path: "/health".into(), headers: vec![("host".into(), "127.0.0.1:8791".into()), ("origin".into(), "https://evil.example".into())], body: String::new() };
        assert_eq!(h.handle(&req).0, 403);
    }

    /// A fake SlopTotal on a port: the helper adopts it (never started by the helper, so never stopped), and passes a
    /// request with the text alone.
    #[test]
    fn a_sloptotal_already_running_is_used_as_it_is() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let r = http::read_request(&mut s).unwrap();
                s2.lock().unwrap().push(format!("{} {} {}", r.method, r.path, r.body));
                let body = if r.path == "/health" { json!({ "status": "healthy", "engines": { "total": 23, "loaded": 4, "failed": [] } }) } else { json!({ "score": 12.0, "verdict": "clean", "engines": [] }) };
                http::respond(&mut s, 200, &body.to_string());
            }
        });
        let h = Helper::new(Config { port: 8791, folder: String::new(), sloptotal_port: port, mode: device::Mode::Auto, idle: Duration::from_secs(600) });
        let (s, v) = post(&h, "/sloptotal/start", "{}");
        assert_eq!((s, v["state"].as_str()), (200, Some("running")));
        assert_eq!(get(&h, "/health").1["sloptotal"]["ours"], false);
        let (s, v) = post(&h, "/sloptotal/api/quick-score", "{\"text\":\"Some text to score here.\",\"url\":\"https://example.com\"}");
        assert_eq!((s, v["score"].as_f64()), (200, Some(12.0)));
        assert!(seen.lock().unwrap().iter().any(|l| l == "POST /api/quick-score {\"text\":\"Some text to score here.\"}"), "{:?}", seen.lock().unwrap());
        assert!(!h.stop("test"), "not ours: never stopped");
    }
}
