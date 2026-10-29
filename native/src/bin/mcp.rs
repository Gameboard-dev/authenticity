//! authenticity-mcp: Authenticity's MCP server (see the library's mcp.rs for the tools).
//!
//! `authenticity-mcp --vault <vault> [--config-dir .obsidian] [--sloptotal <folder>] [--sloptotal-port 8000]
//! [--helper-port 8791] [--device auto|gpu|cpu] [--idle-minutes 10]`. Without --sloptotal, the
//! installation guide's folder (%LOCALAPPDATA%\Programs\SlopTotal) when SlopTotal is there.

use std::path::PathBuf;
use std::time::Duration;

use authenticity_native::{arg, device, helper, mcp, port_arg};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let run = || -> Result<(), String> {
        let vault = arg(&args, "--vault").map(PathBuf::from).ok_or("usage: authenticity-mcp --vault <vault folder> [--sloptotal <folder>] [--device auto|gpu|cpu] ...")?;
        let folder = arg(&args, "--sloptotal").or_else(|| helper::default_folder().map(|f| f.to_string_lossy().into_owned())).unwrap_or_default();
        let s = mcp::Server::new(
            std::path::absolute(&vault).map_err(|e| e.to_string())?,
            arg(&args, "--config-dir").unwrap_or_else(|| ".obsidian".into()),
            folder,
            port_arg(&args, "--sloptotal-port", 8000)?,
            port_arg(&args, "--helper-port", 8791)?,
            device::Mode::parse(&arg(&args, "--device").unwrap_or_default()),
            Duration::from_secs(60 * arg(&args, "--idle-minutes").and_then(|m| m.parse::<u64>().ok()).unwrap_or(10).clamp(1, 24 * 60)),
        );
        s.run();
        Ok(())
    };
    if let Err(e) = run() {
        eprintln!("authenticity-mcp: {e}");
        std::process::exit(2);
    }
}
