//! authenticity-helper: runs SlopTotal for Authenticity on 127.0.0.1 (see the library's helper.rs).
//!
//! `authenticity-helper --sloptotal <folder> [--port 8791] [--sloptotal-port 8000] [--device auto|gpu|cpu]
//! [--idle-minutes 10] [--exit-with-stdin]`. Prints `{"ready": true, "port": N}` when it serves. With
//! `--exit-with-stdin` (the plugin's), it stops SlopTotal and exits when its input closes (Obsidian closed).

use std::io::Read;
use std::time::Duration;

use authenticity_native::{arg, device, helper, port_arg};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let run = || -> Result<(), String> {
        let folder = arg(&args, "--sloptotal").unwrap_or_default();
        let cfg = helper::Config {
            port: port_arg(&args, "--port", 8791)?,
            folder,
            sloptotal_port: port_arg(&args, "--sloptotal-port", 8000)?,
            mode: device::Mode::parse(&arg(&args, "--device").unwrap_or_default()),
            idle: Duration::from_secs(60 * arg(&args, "--idle-minutes").and_then(|m| m.parse::<u64>().ok()).unwrap_or(10).clamp(1, 24 * 60)),
        };
        let h = helper::Helper::new(cfg.clone());
        let server = h.serve()?;
        println!("{{\"ready\":true,\"port\":{}}}", cfg.port);
        if args.iter().any(|a| a == "--exit-with-stdin") {
            let mut sink = [0u8; 256];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
            h.stop("Obsidian closed");
            std::process::exit(0);
        }
        _ = server.join();
        Ok(())
    };
    if let Err(e) = run() {
        eprintln!("authenticity-helper: {e}");
        std::process::exit(3);
    }
}
