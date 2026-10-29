//! Authenticity's part in the GPU claims the local GPU services share (native-kit's `hw::claims`: the same files, the
//! same protocol): while the SlopTotal this helper started holds the GPU, the helper claims it under its own name, so
//! the other services' question about the GPU names it; before SlopTotal loads on the GPU, the helper reads the others'
//! claims and, when the user agrees, asks them to let go. SlopTotal cannot unload its models, so Authenticity's release
//! path (`/gpu/release` on the helper) stops the SlopTotal the helper started.

use std::time::Duration;

use native_kit::hw::{self, claims as kit};

pub use kit::Claim;

pub const SERVICE: &str = "authenticity";
pub const NAME: &str = "Authenticity";

/// This helper holds `mb` of GPU memory (through the SlopTotal it started), freed by POST /gpu/release on `port`.
pub fn hold(port: u16, mb: u64) {
    kit::hold_as(SERVICE, Some(NAME), port, mb, "/gpu/release");
}

pub fn drop_claim() {
    kit::drop_claim(SERVICE);
}

/// The other services' live claims (a stale one is removed).
pub fn others() -> Vec<Claim> {
    kit::others(SERVICE)
}

/// Ask every other service holding GPU memory to let it go, then wait up to `wait` for `need_mb` (margin included) to
/// be free. Whether it is.
pub fn release_others(need_mb: u64, wait: Duration) -> bool {
    kit::release_others(SERVICE, need_mb.saturating_sub(hw::GPU_MARGIN_MB), wait)
}
