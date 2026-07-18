pub mod adapters;
mod build;
pub mod model;
pub mod patch;
pub(crate) mod safety;
pub mod worker;

pub use model::{
    PermissionDiffModel, PermissionDiffWorkerInput, PermissionEditIntent, PermissionEditOperation,
    SnapshotStatus,
};

pub const MAX_FILES: usize = 64;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_TOTAL_READ_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_DIFF_LINES_PER_FILE: usize = 400;
pub const MAX_DIFF_LINES_TOTAL: usize = 3000;
pub const WORKER_TIMEOUT_MS: u64 = 300;
pub const MAX_WORKER_STDOUT_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_INTENT_BYTES: usize = 768 * 1024;

pub fn validate_intent(
    intent: &PermissionEditIntent,
    agent_kind: &str,
    workspace: &str,
) -> Result<(), String> {
    if intent.agent_kind != agent_kind {
        return Err("permission edit agent mismatch".to_string());
    }
    if intent.workspace != workspace {
        return Err("permission edit workspace mismatch".to_string());
    }
    let tool_matches = matches!(
        (agent_kind, intent.native_tool.as_str(), &intent.operation),
        (
            "claude",
            "Edit",
            PermissionEditOperation::TextReplace { .. }
        ) | (
            "claude",
            "Write",
            PermissionEditOperation::WholeFileWrite { .. }
        ) | (
            "claude",
            "NotebookEdit",
            PermissionEditOperation::Unsupported { .. }
        ) | (
            "codex",
            "apply_patch",
            PermissionEditOperation::PatchSet { .. }
        ) | (
            "codex",
            "apply_patch",
            PermissionEditOperation::Unsupported { .. }
        )
    );
    if !tool_matches {
        return Err("permission edit tool mismatch".to_string());
    }
    if let PermissionEditOperation::PatchSet { files } = &intent.operation {
        if files.is_empty() || files.len() > MAX_FILES {
            return Err("permission edit file count is invalid".to_string());
        }
    }
    let bytes = serde_json::to_vec(intent)
        .map_err(|_| "permission edit serialization failed".to_string())?;
    if bytes.len() > MAX_INTENT_BYTES {
        return Err("permission edit payload is too large".to_string());
    }
    Ok(())
}
