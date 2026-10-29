//! Authenticity's native programs, sharing the plugin's pure modules (../core, ../similar):
//!
//! - `authenticity-helper` (helper.rs): runs SlopTotal for the plugin on 127.0.0.1, deciding the device (device.rs),
//!   taking part in the GPU claims of the user's other services (claims.rs), starting and stopping it (launch.rs);
//! - `authenticity-mcp` (mcp.rs): the MCP server for research assistants;
//! - `check-overlap` (overlap.rs): the close-copying check of a draft.
//!
//! All of them speak HTTP only to 127.0.0.1 (http.rs) and read the vault from disk (vault.rs).

pub mod claims;
pub mod device;
pub mod helper;
pub mod http;
pub mod launch;
pub mod mcp;
pub mod overlap;
pub mod vault;

/// The value after `flag` in `args`.
pub fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// A port from `flag`, else `default`.
pub fn port_arg(args: &[String], flag: &str, default: u16) -> Result<u16, String> {
    match arg(args, flag) {
        None => Ok(default),
        Some(p) => p.parse::<u16>().ok().filter(|p| *p >= 1024).ok_or(format!("{flag}: a port from 1024 to 65535")),
    }
}
