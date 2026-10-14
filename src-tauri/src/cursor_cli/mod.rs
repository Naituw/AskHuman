//! Managed interactive Cursor CLI startup and background-completion policy.
#[cfg(target_os = "macos")]
mod runtime;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessStatus {
    pub pid: u32,
    pub revision: String,
    pub version: String,
    pub phase: String,
    pub active: bool,
    pub suppressed: u64,
    pub updated_at: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Status {
    pub supported: bool,
    pub enabled: bool,
    pub phase: String,
    pub entries: Vec<String>,
    pub processes: Vec<ProcessStatus>,
    pub error: Option<String>,
    pub updated_at: u64,
}

pub fn start() {
    #[cfg(target_os = "macos")]
    tokio::spawn(runtime::run());
}

pub fn needs_cleanup() -> bool {
    #[cfg(target_os = "macos")]
    return runtime::needs_cleanup();
    #[cfg(not(target_os = "macos"))]
    false
}

pub fn status() -> Status {
    #[cfg(target_os = "macos")]
    return runtime::status();
    #[cfg(not(target_os = "macos"))]
    Status {
        phase: "unsupported".into(),
        ..Status::default()
    }
}

pub fn launch(args: &[std::ffi::OsString]) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return runtime::launch(args);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = args;
        anyhow::bail!("Managed Cursor CLI is available on macOS only")
    }
}
