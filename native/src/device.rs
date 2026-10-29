//! Which device SlopTotal runs on: the graphics card when the hardware has the capacity, so users who have it are not
//! held back; else the processor. Settings → Device: Automatic (the default), GPU or CPU.
//!
//! The graphics card is used only when all of these hold: SlopTotal's Python has a CUDA build of PyTorch (its
//! `torch/version.py` names a CUDA version), an NVIDIA card is found, the card is large enough for SlopTotal's models
//! (SLOPTOTAL_GPU_MB plus the margin every service of the workspace leaves), and that much is free, after the user's
//! other GPU services have been asked to let theirs go when the user agrees (the question every plugin of the
//! workspace asks, shared/src/gpu.rs). With GPU chosen, a shortage always asks; with Automatic, it asks only when the
//! user's other services hold enough to make room, and otherwise uses the processor without asking.
//!
//! The decision is a pure function of what was found (`decide`), so every row of the table is tested.

use std::path::Path;

use serde_json::{json, Value};

/// What SlopTotal's models take on the GPU, all loaded at start in fp32: about 8.3 GB (an estimate from the models'
/// sizes, 2026-10-29). SlopTotal cannot unload them, so it holds this until it is stopped.
pub const SLOPTOTAL_GPU_MB: u64 = 8300;
/// Left free for the display and other programs (native-kit's GPU_MARGIN_MB).
pub const MARGIN_MB: u64 = native_kit::hw::GPU_MARGIN_MB;

/// What the computer is doing now, for how many paragraph requests to send to SlopTotal at once (`parallel`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Load {
    /// Logical CPU cores, and how busy the CPU is (all programs, 0 to 100).
    pub cores: usize,
    pub busy: f32,
    /// The GPU's free memory (MB), and what the other services' claims hold now and held when SlopTotal started.
    pub gpu_free_mb: Option<u64>,
    pub others_mb: u64,
    pub others_mb_at_start: u64,
}

/// How many paragraph requests SlopTotal is sent at once, on `device` ("cuda", else the processor). Measured on
/// 2026-10-29 on a 16-core CPU with SlopTotal on the processor (24 paragraphs, 3,053 words): 12.0 s one at a time,
/// 10.5 s two at a time, 10.5 s four at a time (each request then slower): SlopTotal runs each request's classifiers on
/// one thread and holds most of the work in its own queue, so two in flight is all that pays. On the processor: two,
/// one when the computer has fewer than four cores (two left for Obsidian) or is already busy (70% or more). On the
/// GPU: two, one when its free memory runs low (under 1 GB) or another service has claimed it since SlopTotal started
/// (it backs off, as the claims protocol asks). Never more than SlopTotal's quick lane takes.
pub fn parallel(device: &str, l: Load) -> usize {
    let n = if device == "cuda" {
        if l.gpu_free_mb.is_some_and(|f| f < 1024) || l.others_mb > l.others_mb_at_start { 1 } else { 2 }
    } else if l.cores < 4 || l.busy >= 70.0 {
        1
    } else {
        2
    };
    n.clamp(1, authenticity_core::sloptotal::QUICK_LANE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Gpu,
    Cpu,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "gpu" => Mode::Gpu,
            "cpu" => Mode::Cpu,
            _ => Mode::Auto,
        }
    }
}

/// What was found about this computer.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    /// Whether SlopTotal's Python has PyTorch for CUDA (Err: it could not be told).
    pub cuda_torch: Result<bool, String>,
    pub total_mb: Option<u64>,
    pub free_mb: Option<u64>,
    /// What the user's other GPU services hold, and their names.
    pub others_mb: u64,
    pub holders: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Gpu,
    /// The processor, and why.
    Cpu(String),
    /// Too little free: ask the user (the shortage, as the shared question takes it).
    Ask(Value),
    /// The user agreed: ask the other services to let go, then look again.
    Release,
}

impl Default for Facts {
    fn default() -> Facts {
        Facts { cuda_torch: Ok(false), total_mb: None, free_mb: None, others_mb: 0, holders: Vec::new() }
    }
}

/// The device, from the setting, what was found and the user's answer to the question ("" none yet, "free", "cpu").
pub fn decide(mode: Mode, f: &Facts, answer: &str) -> Decision {
    let need = SLOPTOTAL_GPU_MB + MARGIN_MB;
    if mode == Mode::Cpu {
        return Decision::Cpu("set to the processor".into());
    }
    match &f.cuda_torch {
        Ok(true) => {}
        Ok(false) => return Decision::Cpu("SlopTotal's Python has PyTorch for the processor only (the graphics card needs a CUDA build of PyTorch: the installation guide, step 3)".into()),
        Err(e) => return Decision::Cpu(format!("SlopTotal's PyTorch could not be read ({e})")),
    }
    let Some(total) = f.total_mb else { return Decision::Cpu("no NVIDIA graphics card was found".into()) };
    if total < need {
        return Decision::Cpu(format!("the graphics card has {total} MB, and SlopTotal's models need about {need} MB"));
    }
    let free = f.free_mb.unwrap_or(0);
    if free >= need {
        return Decision::Gpu;
    }
    match answer {
        "cpu" => return Decision::Cpu("you chose the processor".into()),
        "free" => return Decision::Release,
        _ => {}
    }
    let room = f.others_mb > 0 && free + f.others_mb >= need;
    if room || mode == Mode::Gpu {
        return Decision::Ask(json!({ "what": "SlopTotal's detectors", "need_mb": need, "free_mb": free, "others_mb": f.others_mb, "others": f.holders.len(), "holders": f.holders }));
    }
    Decision::Cpu(format!("the graphics card has only {free} MB free, and SlopTotal's models need about {need} MB"))
}

/// Whether the PyTorch in SlopTotal's own Python (`<folder>/venv`) was built for CUDA: its `torch/version.py` sets
/// `cuda` to a version ("12.1"), or to None for a CPU build.
pub fn cuda_torch(folder: &Path) -> Result<bool, String> {
    let venv = folder.join("venv");
    let mut candidates = vec![venv.join("Lib").join("site-packages").join("torch").join("version.py")];
    if let Ok(lib) = std::fs::read_dir(venv.join("lib")) {
        for e in lib.flatten() {
            candidates.push(e.path().join("site-packages").join("torch").join("version.py"));
        }
    }
    let file = candidates.into_iter().find(|p| p.is_file()).ok_or("PyTorch is not installed in SlopTotal's venv")?;
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    Ok(cuda_in(&text))
}

/// `cuda: Optional[str] = '12.1'` or `cuda = None` in torch/version.py.
pub fn cuda_in(version_py: &str) -> bool {
    version_py.lines().filter(|l| l.trim_start().starts_with("cuda")).any(|l| {
        let name = l.trim_start().split(|c: char| c == ':' || c == '=' || c.is_whitespace()).next();
        let value = l.rsplit('=').next().unwrap_or("").trim();
        name == Some("cuda") && (value.starts_with('\'') || value.starts_with('"')) && value.trim_matches(['\'', '"']).chars().next().is_some_and(|c| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_at_once_follow_the_hardware() {
        let idle = Load { cores: 16, busy: 10.0, gpu_free_mb: Some(4000), others_mb: 0, others_mb_at_start: 0 };
        assert_eq!(parallel("cpu", idle), 2);
        assert_eq!(parallel("cpu", Load { busy: 85.0, ..idle }), 1, "a busy computer");
        assert_eq!(parallel("cpu", Load { cores: 2, ..idle }), 1, "few cores: room left for Obsidian");
        assert_eq!(parallel("", idle), 2, "a SlopTotal started outside Authenticity: as on the processor");
        assert_eq!(parallel("cuda", idle), 2);
        assert_eq!(parallel("cuda", Load { gpu_free_mb: Some(600), ..idle }), 1, "little GPU memory left");
        assert_eq!(parallel("cuda", Load { others_mb: 2000, others_mb_at_start: 0, ..idle }), 1, "another service claimed the GPU since");
        assert_eq!(parallel("cuda", Load { others_mb: 2000, others_mb_at_start: 2000, ..idle }), 2, "the same claims as at the start");
    }

    fn facts(torch: bool, total: Option<u64>, free: u64, others: u64) -> Facts {
        Facts { cuda_torch: Ok(torch), total_mb: total, free_mb: Some(free), others_mb: others, holders: if others > 0 { vec!["Another plugin".into()] } else { vec![] } }
    }

    fn cpu(d: Decision) -> bool {
        matches!(d, Decision::Cpu(_))
    }

    /// The decision table, row by row.
    #[test]
    fn device_decision_table() {
        let big = Some(16_000);
        // CPU chosen: always the processor
        assert!(cpu(decide(Mode::Cpu, &facts(true, big, 16_000, 0), "")));
        // no CUDA PyTorch, no card, or a card too small: the processor, whatever the setting
        for m in [Mode::Auto, Mode::Gpu] {
            assert!(cpu(decide(m, &facts(false, big, 16_000, 0), "")));
            assert!(cpu(decide(m, &facts(true, None, 0, 0), "")));
            assert!(cpu(decide(m, &facts(true, Some(8188), 8000, 0), "")), "an 8 GB card never fits");
            assert!(cpu(decide(m, &Facts { cuda_torch: Err("unreadable".into()), ..facts(true, big, 16_000, 0) }, "")));
        }
        // room: the GPU
        assert_eq!(decide(Mode::Auto, &facts(true, big, 9000, 0), ""), Decision::Gpu);
        // short, but the user's other services hold enough: ask (both settings)
        for m in [Mode::Auto, Mode::Gpu] {
            let Decision::Ask(s) = decide(m, &facts(true, big, 4000, 6000), "") else { panic!() };
            assert_eq!((s["need_mb"].as_u64(), s["free_mb"].as_u64(), s["others_mb"].as_u64()), (Some(8600), Some(4000), Some(6000)));
            assert_eq!(s["holders"][0], "Another plugin");
        }
        // short, and nothing to free: Automatic uses the processor without asking; GPU asks
        assert!(cpu(decide(Mode::Auto, &facts(true, big, 4000, 0), "")));
        assert!(matches!(decide(Mode::Gpu, &facts(true, big, 4000, 0), ""), Decision::Ask(_)));
        // the user's answers
        assert_eq!(decide(Mode::Auto, &facts(true, big, 4000, 6000), "free"), Decision::Release);
        assert!(cpu(decide(Mode::Auto, &facts(true, big, 4000, 6000), "cpu")));
    }

    /// This computer, as the user has it: an 8 GB card and PyTorch for the processor. Automatic picks the processor.
    #[test]
    fn eight_gigabytes_picks_the_cpu() {
        let d = decide(Mode::Auto, &facts(true, Some(8188), 7400, 0), "");
        assert!(matches!(&d, Decision::Cpu(why) if why.contains("8188 MB")), "{d:?}");
        assert!(cpu(decide(Mode::Auto, &facts(false, Some(8188), 7400, 0), "")));
    }

    #[test]
    fn torch_builds_are_told_apart() {
        assert!(cuda_in("__version__ = '2.5.1+cu121'\ncuda: Optional[str] = '12.1'\nhip = None\n"));
        assert!(!cuda_in("__version__ = '2.15.0+cpu'\ncuda: Optional[str] = None\n"));
        assert!(!cuda_in("cudnn = '9'\n"));
        let dir = std::env::temp_dir().join(format!("auth-torch-{}", std::process::id()));
        let p = dir.join("venv").join("Lib").join("site-packages").join("torch");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("version.py"), "cuda = None\n").unwrap();
        assert_eq!(cuda_torch(&dir), Ok(false));
        assert!(cuda_torch(&dir.join("nowhere")).is_err());
        _ = std::fs::remove_dir_all(&dir);
    }
}
