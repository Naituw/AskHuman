//! Types of the daemon-facing API that Agents, Watch, interjections and new tasks need on top of
//! the question flow (plan §4.4, §4.6, §4.7). The gateway never touches the Agent registry or
//! the terminal itself; it only talks in these shapes.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::ios::wire::{TaskOptions, TodoOption};

/// One Watch frame as the daemon computes it.
#[derive(Debug, Clone)]
pub struct FrameData {
    /// Frame fields (`phase`, `title`, `text`, `steps`, …). The gateway adds `failed`.
    pub frame: Value,
    /// Changes whenever the visible content changes; unchanged frames are not sent again.
    pub sig: String,
    /// The Agent session ended with this frame.
    pub ended: bool,
}

/// What the daemon did with an interjection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterjectResult {
    /// `queued`, `delivered` or `rejected`.
    pub status: &'static str,
    /// `notWorking`, `unsupported`, `empty`, `notFound`, … when rejected.
    pub reason: Option<String>,
    /// Interjections still waiting for the Agent (including this one when queued).
    pub pending: u64,
}

impl InterjectResult {
    pub fn rejected(reason: &str) -> Self {
        Self {
            status: "rejected",
            reason: Some(reason.to_string()),
            pending: 0,
        }
    }
}

/// A file the phone uploaded for an interjection, staged on disk by the gateway.
#[derive(Debug, Clone)]
pub struct StagedFile {
    pub name: String,
    pub mime: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_image: bool,
}

/// What a phone may pick when creating a task, and where each workspace id points.
#[derive(Debug, Clone)]
pub struct TaskChoices {
    pub options: TaskOptions,
    /// Workspace id (hash of the canonical path) → absolute path.
    pub workspaces: HashMap<String, String>,
}

impl TaskChoices {
    /// Creation is not possible (feature off, nothing ready, …).
    pub fn blocked(permission_mode: &str, reason: &str) -> Self {
        Self {
            options: TaskOptions {
                enabled: false,
                blocked: Some(reason.to_string()),
                permission_mode: permission_mode.to_string(),
                workspaces: Vec::new(),
                agents: Vec::new(),
            },
            workspaces: HashMap::new(),
        }
    }
}

/// A validated `task.create`.
#[derive(Debug, Clone)]
pub struct TaskLaunch {
    pub phone_id: String,
    pub client_msg_id: String,
    pub workspace: String,
    pub agent_kind: String,
    /// `default` or `yolo`.
    pub permission: String,
    pub todo_id: Option<String>,
    pub text: String,
    /// A model id the phone picked; `None` leaves it to the Agent. Well-formed, but not yet
    /// matched against the Agent's catalog: the host does that.
    pub model: Option<String>,
    /// A reasoning effort the phone picked; `None` leaves it to the Agent. Same status as `model`.
    pub effort: Option<String>,
}

/// Todos offered for a workspace.
#[derive(Debug, Clone, Default)]
pub struct TodoChoices {
    pub todos: Vec<TodoOption>,
    pub overflow: u64,
}

/// Lifecycle events the daemon reports about an Agent session (the subset Follow needs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEventKind {
    TurnStart,
    TurnEnd,
    /// Tool use, prompts and other signs of life.
    Activity,
    SessionEnd,
}

#[derive(Debug, Clone)]
pub struct AgentEventInfo {
    /// `{kind}:{sessionId}`.
    pub agent_key: String,
    pub event: AgentEventKind,
    /// The turn ended because of an error (never for a plain stop).
    pub failed: bool,
}
