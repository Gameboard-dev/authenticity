//! Starting and stopping SlopTotal. Its own Python (`<folder>/venv`) runs uvicorn with SlopTotal's app, on 127.0.0.1
//! only, in SlopTotal's folder, as an argument array (no shell, nothing built from text), with the settings that keep
//! it on this computer:
//!
//! - `--host 127.0.0.1` (SlopTotal's own scripts default to 0.0.0.0, the whole network) and the port;
//! - `SLOPTOTAL_DEVICE` (`cpu` or `cuda`, as device.rs decided), `HF_HOME=<folder>/models` (the models the user
//!   downloaded once), `HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1` (no model library asks huggingface.co about its
//!   models at each start) and `HF_HUB_DISABLE_TELEMETRY=1`;
//! - `SLOPTOTAL_RETENTION_DAYS=1`: SlopTotal keeps each text it scores in its own database, and this keeps it one
//!   day, the shortest it allows;
//! - `PYTHONIOENCODING=utf-8`, `PYTHONUTF8=1`: its output goes to a log file, which Python on Windows would otherwise
//!   write in the ANSI code page, failing on SlopTotal's start-up banner.
//!
//! The folder comes only from this computer's settings (the plugin's local storage, or the MCP server's arguments),
//! never from the vault, and is checked: its environment variables expanded (`%LOCALAPPDATA%\Programs\SlopTotal`, as
//! the setting's description suggests, or `~/...`; core's paths.rs), an absolute folder holding SlopTotal's
//! `app/main.py` and its venv's Python.
//! SlopTotal's output goes to `<local data>/authenticity/logs/sloptotal.log` (begun again at each start).
//!
//! On Windows the venv's python.exe starts the real Python as its child, so stopping SlopTotal stops the whole tree
//! this helper started, and nothing else.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// What SlopTotal is started with.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

/// SlopTotal's Python in its folder.
pub fn python(folder: &Path) -> PathBuf {
    if cfg!(windows) {
        folder.join("venv").join("Scripts").join("python.exe")
    } else {
        folder.join("venv").join("bin").join("python")
    }
}

/// The folder as set, with its environment variables expanded from this process's environment (`%LOCALAPPDATA%`,
/// `~`; core's paths.rs).
pub fn expand_folder(folder: &str) -> Result<PathBuf, String> {
    authenticity_core::paths::expand(folder, |k| std::env::var(k).ok().filter(|v| !v.is_empty()), cfg!(windows)).map(PathBuf::from)
}

/// The folder, checked: its variables expanded, absolute, SlopTotal's app in it, its Python there.
pub fn check_folder(folder: &str) -> Result<PathBuf, String> {
    if folder.trim().is_empty() {
        return Err("no SlopTotal folder is set (Settings → Authenticity → Writing checks)".into());
    }
    let f = expand_folder(folder).map_err(|e| format!("the SlopTotal folder \"{}\": {e}", folder.trim()))?;
    if !f.is_absolute() {
        return Err(format!("the SlopTotal folder must be a full path, such as %LOCALAPPDATA%\\Programs\\SlopTotal ({})", f.display()));
    }
    if !f.join("app").join("main.py").is_file() {
        return Err(format!("{} does not hold SlopTotal (no app/main.py): set the folder you installed it in", f.display()));
    }
    if !python(&f).is_file() {
        return Err(format!("SlopTotal's Python is missing ({}): make its venv (the installation guide, step 2)", python(&f).display()));
    }
    Ok(f)
}

pub fn spec(folder: &Path, port: u16, device: &str) -> Spec {
    let device = if device == "cuda" { "cuda" } else { "cpu" };
    Spec {
        program: python(folder),
        args: ["-m", "uvicorn", "app.main:app", "--host", "127.0.0.1", "--port"].iter().map(|s| s.to_string()).chain([port.to_string()]).collect(),
        cwd: folder.to_path_buf(),
        env: vec![
            ("SLOPTOTAL_DEVICE".into(), device.into()),
            ("HF_HOME".into(), folder.join("models").to_string_lossy().into_owned()),
            ("HF_HUB_OFFLINE".into(), "1".into()),
            ("TRANSFORMERS_OFFLINE".into(), "1".into()),
            ("HF_HUB_DISABLE_TELEMETRY".into(), "1".into()),
            ("SLOPTOTAL_RETENTION_DAYS".into(), "1".into()),
            // its output goes to a log file, which Python on Windows writes in the ANSI code page: SlopTotal's start-up
            // banner (box-drawing characters) then stopped it with a UnicodeEncodeError (2026-10-29). UTF-8 throughout.
            ("PYTHONIOENCODING".into(), "utf-8".into()),
            ("PYTHONUTF8".into(), "1".into()),
        ],
    }
}

pub fn log_file() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("authenticity").join("logs").join("sloptotal.log"))
}

/// Whether SlopTotal's log says its models are loaded ("Model preloading complete", its own line once every model it
/// preloads is in memory): requests sent before it race the loading, which SlopTotal guards with a lock but which
/// has been seen to leave a model broken (docs/setup.md).
pub fn preloaded_in(log: &str) -> bool {
    log.contains("Model preloading complete")
}

/// The same for the SlopTotal this helper started (its log file).
pub fn preloaded() -> bool {
    log_file().and_then(|f| std::fs::read(f).ok()).is_some_and(|b| preloaded_in(&String::from_utf8_lossy(&b)))
}

/// The last lines of SlopTotal's log, for a message.
pub fn log_tail(n: usize) -> String {
    let text = log_file().and_then(|f| std::fs::read(f).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" | ").chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect()
}

pub fn start(s: &Spec) -> Result<Child, String> {
    let mut c = Command::new(&s.program);
    c.args(&s.args).current_dir(&s.cwd).stdin(Stdio::null());
    for (k, v) in &s.env {
        c.env(k, v);
    }
    match log_file().and_then(|f| {
        std::fs::create_dir_all(f.parent()?).ok()?;
        std::fs::File::create(f).ok()
    }) {
        Some(out) => {
            let err = out.try_clone().map_err(|e| e.to_string())?;
            c.stdout(out).stderr(err);
        }
        None => {
            c.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window
    }
    c.spawn().map_err(|e| format!("SlopTotal's Python could not start ({}): {e}", s.program.display()))
}

/// The processes started from `pid` (its children, theirs, ...), deepest first.
fn descendants(pid: u32) -> Vec<sysinfo::Pid> {
    let mut s = sysinfo::System::new();
    s.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut out = Vec::new();
    let mut frontier = vec![sysinfo::Pid::from_u32(pid)];
    while let Some(p) = frontier.pop() {
        for (q, proc_) in s.processes() {
            if proc_.parent() == Some(p) && !out.contains(q) {
                out.push(*q);
                frontier.push(*q);
            }
        }
    }
    out.reverse();
    out
}

/// Stop the SlopTotal this helper started: its children first, then it.
pub fn stop(child: &mut Child) {
    let kids = descendants(child.id());
    let s = {
        let mut s = sysinfo::System::new();
        s.refresh_processes(sysinfo::ProcessesToUpdate::Some(&kids), true);
        s
    };
    for k in &kids {
        if let Some(p) = s.process(*k) {
            p.kill();
        }
    }
    _ = child.kill();
    _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_only_when_the_models_are_loaded() {
        assert!(!preloaded_in("INFO: Application startup complete.\nINFO sloptotal: Preloaded GPT-2 Medium"));
        assert!(preloaded_in("INFO sloptotal: Preloaded SuperAnnotate RoBERTa\n2026-10-29 15:03:59 INFO sloptotal: Model preloading complete \u{2014} 23 engines ready"));
    }

    #[test]
    fn started_on_this_computer_only_and_offline() {
        let folder = if cfg!(windows) { PathBuf::from("C:\\Tools\\SlopTotal") } else { PathBuf::from("/opt/sloptotal") };
        let s = spec(&folder, 8000, "cpu");
        assert_eq!(s.args, ["-m", "uvicorn", "app.main:app", "--host", "127.0.0.1", "--port", "8000"]);
        assert_eq!(s.cwd, folder);
        assert!(s.program.ends_with(if cfg!(windows) { "venv\\Scripts\\python.exe" } else { "venv/bin/python" }));
        let env: std::collections::HashMap<&str, &str> = s.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env["SLOPTOTAL_DEVICE"], "cpu");
        assert_eq!(env["HF_HUB_OFFLINE"], "1");
        assert_eq!(env["TRANSFORMERS_OFFLINE"], "1");
        assert_eq!(env["HF_HUB_DISABLE_TELEMETRY"], "1");
        assert_eq!(env["SLOPTOTAL_RETENTION_DAYS"], "1");
        assert_eq!((env["PYTHONIOENCODING"], env["PYTHONUTF8"]), ("utf-8", "1"), "its log is written in UTF-8");
        assert_eq!(PathBuf::from(env["HF_HOME"]), folder.join("models"));
        assert!(!s.args.iter().any(|a| a == "0.0.0.0"));
        assert_eq!(spec(&folder, 8000, "cuda").env[0].1, "cuda");
        assert_eq!(spec(&folder, 8000, "anything; else").env[0].1, "cpu", "only cpu or cuda");
    }

    #[test]
    fn the_folder_is_checked() {
        assert!(check_folder("").unwrap_err().contains("no SlopTotal folder"));
        assert!(check_folder("relative/path").unwrap_err().contains("full path"));
        let dir = std::env::temp_dir().join(format!("auth-slop-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let d = dir.to_string_lossy().into_owned();
        assert!(check_folder(&d).unwrap_err().contains("app/main.py"));
        std::fs::write(dir.join("app").join("main.py"), "").unwrap();
        assert!(check_folder(&d).unwrap_err().contains("Python is missing"));
        std::fs::create_dir_all(python(&dir).parent().unwrap()).unwrap();
        std::fs::write(python(&dir), "").unwrap();
        assert_eq!(check_folder(&d).unwrap(), dir);
        // a hostile folder name is a path, never a command: it simply is not SlopTotal
        assert!(check_folder("C:\\x\" & calc & \"").is_err());
        _ = std::fs::remove_dir_all(&dir);
    }

    /// The form the setting's description suggests (`%LOCALAPPDATA%\Programs\SlopTotal`, or `~/...`) is expanded from
    /// the environment before it is checked; a variable that is not set is named in the error.
    #[test]
    fn the_folder_may_name_environment_variables() {
        let (var, written) = if cfg!(windows) { ("LOCALAPPDATA", "%LOCALAPPDATA%") } else { ("HOME", "$HOME") };
        let base = PathBuf::from(std::env::var(var).unwrap());
        let name = format!("auth-slop-env-{}", std::process::id());
        let dir = base.join(&name);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        std::fs::write(dir.join("app").join("main.py"), "").unwrap();
        std::fs::create_dir_all(python(&dir).parent().unwrap()).unwrap();
        std::fs::write(python(&dir), "").unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(check_folder(&format!("{written}{sep}{name}")).unwrap(), dir);
        assert_eq!(expand_folder(&format!("{written}{sep}{name}")).unwrap(), dir);
        let e = check_folder(&format!("{}AUTH_SURELY_UNSET_VAR{}{sep}SlopTotal", if cfg!(windows) { "%" } else { "$" }, if cfg!(windows) { "%" } else { "" })).unwrap_err();
        assert!(e.contains("AUTH_SURELY_UNSET_VAR") && e.contains("not set"), "{e}");
        _ = std::fs::remove_dir_all(&dir);
    }
}
