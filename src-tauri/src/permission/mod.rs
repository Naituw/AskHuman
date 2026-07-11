//! Permission hook: intercept Claude Code / Codex PermissionRequest events,
//! route to AskHuman daemon for human approval, and output the decision JSON.

pub mod input;
pub mod output;
pub mod summarize;

use crate::ipc::{self, ClientHello, ClientMsg, ConfirmTask, ServerMsg};

const STDIN_MAX_BYTES: usize = 512 * 1024;
const FIELD_MAX_CHARS: usize = 8192;

/// Parsed permission request (agent-agnostic).
#[derive(Debug, Clone)]
pub struct PermissionInput {
    pub agent: AgentFamily,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub cwd: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFamily {
    Claude,
    Codex,
}

impl AgentFamily {
    pub fn from_arg(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

impl PermissionInput {
    /// Build a `ConfirmTask` suitable for sending to the daemon.
    pub fn to_confirm_task(&self) -> ConfirmTask {
        let summary = summarize::summarize(&self.tool_name, &self.tool_input);
        let agent_label = match self.agent {
            AgentFamily::Claude => "Claude Code",
            AgentFamily::Codex => "Codex",
        };
        let title = format!("{} — {}", agent_label, &self.tool_name);

        // Body: cwd + tool summary
        let mut body = String::new();
        if let Some(cwd) = &self.cwd {
            body.push_str(&format!("📁 `{}`\n\n", cwd));
        }
        body.push_str(&summary.body);

        ConfirmTask {
            title,
            body_md: body,
            details: summary.details,
            confirm_action_id: "approve_once".to_string(),
            confirm_label: "Allow once".to_string(),
            cancel_action_id: "deny".to_string(),
            cancel_label: "Deny".to_string(),
            confirm_role: "primary".to_string(),
            cancel_role: "destructive".to_string(),
            dismiss_action_id: Some("deny".to_string()),
            record_history: false,
            expires_at_ms: 0,
            source: format!("permission-hook-{}", self.agent.as_str()),
            agent_session_id: self.session_id.clone(),
            caller_pid: std::process::id(),
        }
    }
}

/// Entry point for `AskHuman __permission-hook <claude|codex>`.
/// Returns the process exit code (0 = success or fallback; never non-zero).
pub fn run_hook(args: &[String]) -> i32 {
    let agent_arg = args.first().map(|s| s.as_str()).unwrap_or("");
    let forced_agent = AgentFamily::from_arg(agent_arg);

    let Some((_detected, perm_input)) = input::parse_stdin() else {
        return 0;
    };

    let agent = forced_agent.unwrap_or(perm_input.agent);
    let task = PermissionInput {
        agent,
        ..perm_input
    }
    .to_confirm_task();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let result = rt.block_on(run_confirm(task));
    match result {
        ConfirmDecision::Allow => {
            println!("{}", output::allow_json(agent));
        }
        ConfirmDecision::Deny => {
            println!("{}", output::deny_json(agent));
        }
        ConfirmDecision::Fallback => {
            // stdout stays empty → Agent falls back to native permission prompt.
        }
    }
    0
}

enum ConfirmDecision {
    Allow,
    Deny,
    Fallback,
}

/// Connect to daemon, submit confirm, wait for result.
async fn run_confirm(task: ConfirmTask) -> ConfirmDecision {
    use tokio::io::BufReader;

    let stream = match ipc::transport::connect().await {
        Ok(s) => s,
        Err(_) => return ConfirmDecision::Fallback,
    };
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);

    let hello = ClientHello {
        protocol_version: ipc::PROTOCOL_VERSION,
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        binary_path: std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        fingerprint: crate::daemon::lifecycle::current_fingerprint(),
        pid: std::process::id(),
    };

    if ipc::write_msg(&mut w, &ClientMsg::Hello(hello))
        .await
        .is_err()
    {
        return ConfirmDecision::Fallback;
    }

    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::HelloAck(_))) => {}
        _ => return ConfirmDecision::Fallback,
    }

    if ipc::write_msg(&mut w, &ClientMsg::SubmitConfirm(task))
        .await
        .is_err()
    {
        return ConfirmDecision::Fallback;
    }

    // Wait for ConfirmAccepted, then ConfirmFinal or ConfirmFallback.
    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::ConfirmAccepted { .. })) => {}
        _ => return ConfirmDecision::Fallback,
    }

    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::ConfirmFinal { action_id, .. })) => {
            if action_id == "approve_once" {
                ConfirmDecision::Allow
            } else {
                ConfirmDecision::Deny
            }
        }
        Ok(Some(ServerMsg::ConfirmFallback { .. })) => ConfirmDecision::Fallback,
        _ => ConfirmDecision::Fallback,
    }
}
