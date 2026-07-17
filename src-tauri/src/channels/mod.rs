//! Channel 抽象：并行运行、首个终态结果由协调器采纳，其余被 `interrupt` 收尾。

pub mod confirm;
pub mod conversation;
pub mod dingding;
pub mod feishu;
pub mod health;
pub mod popup;
pub mod slack;
pub mod telegram;

use crate::app::coordinator::Coordinator;
use crate::models::AskRequest;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 投递结果的句柄（协调器，线程安全）。
pub type ResultSink = Arc<Coordinator>;

/// Per-request caller context rendered in ordinary IM message/question titles. It stays outside
/// `AskRequest` because source, Agent, and project describe delivery context rather than content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationOrigin {
    source: String,
    agent_label: Option<String>,
    project_name: Option<String>,
}

impl ConversationOrigin {
    pub fn new(source: &str, agent_kind: Option<&str>, project: &str) -> Self {
        let agent_label = agent_kind
            .and_then(crate::agents::AgentKind::parse)
            .map(|kind| kind.label().to_string());
        let source = source.trim();
        // MCP requests initially use the generic fallback. Once daemon resolution identifies an
        // Agent, mirror Popup's inline badge by treating that Agent as the effective source.
        let source = if source.is_empty()
            || source.eq_ignore_ascii_case(crate::models::DEFAULT_SOURCE_NAME)
        {
            agent_label
                .clone()
                .unwrap_or_else(|| crate::models::DEFAULT_SOURCE_NAME.to_string())
        } else {
            source.to_string()
        };
        let project_name = crate::project::display_name(project);
        Self {
            source,
            agent_label,
            project_name: (!project_name.trim().is_empty()).then_some(project_name),
        }
    }

    /// `Message from …` / `Question from …`, followed by distinct Agent and project labels.
    pub fn source_title(&self, lang: crate::i18n::Lang, key: &'static str) -> String {
        let base = if self.is_default_source() {
            // Preserve the fallback's historical English base while omitting the uninformative
            // "from the Loop" segment. Project context is still appended below.
            let title_key = match key {
                "channel.questionFrom" => "channel.questionTitle",
                "channel.messageFrom" => "channel.messageTitle",
                _ => key,
            };
            crate::i18n::tr(crate::i18n::Lang::En, title_key).to_string()
        } else {
            crate::i18n::source_header(lang, key, &self.source)
        };
        self.append_context(base, false)
    }

    /// `Question` / `Question i/n`, followed by source, Agent, and project labels.
    pub fn question_title(&self, base: String) -> String {
        self.append_context(base, true)
    }

    fn append_context(&self, base: String, include_source: bool) -> String {
        let mut used = vec![self.source.as_str()];
        let mut parts: Vec<&str> = Vec::new();
        if include_source && !self.is_default_source() {
            parts.push(&self.source);
        }
        for value in [self.agent_label.as_deref(), self.project_name.as_deref()]
            .into_iter()
            .flatten()
        {
            if !used.iter().any(|seen| seen.eq_ignore_ascii_case(value)) {
                used.push(value);
                parts.push(value);
            }
        }
        if parts.is_empty() {
            base
        } else {
            format!("{} · {}", base, parts.join(" · "))
        }
    }

    fn is_default_source(&self) -> bool {
        self.source
            .eq_ignore_ascii_case(crate::models::DEFAULT_SOURCE_NAME)
    }
}

/// Why a session is interrupted before it produced an answer. A single reason covers both
/// "another channel answered first" and "the whole request was cancelled" — they only differ
/// in the wording of the card's terminal state.
#[derive(Clone)]
pub enum Interruption {
    /// Another channel answered first; carries the winner's display name.
    AnsweredBy(String),
    /// The whole request was cancelled; carries the cancel source display name (empty = generic).
    Cancelled(String),
}

/// Interrupt signal: set when a session is interrupted before answering, carrying the reason
/// (so the terminal card text can name the winner or the cancel source). Shared (Arc) between
/// the outer Channel / finalizer and the session task.
pub struct Preemption {
    cancelled: AtomicBool,
    reason: Mutex<Option<Interruption>>,
}

impl Preemption {
    pub fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            reason: Mutex::new(None),
        }
    }

    /// Mark this session interrupted with the given reason.
    pub fn interrupt(&self, reason: Interruption) {
        if let Ok(mut r) = self.reason.lock() {
            *r = Some(reason);
        }
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// The interruption reason (None until interrupted).
    pub fn reason(&self) -> Option<Interruption> {
        self.reason.lock().ok().and_then(|r| r.clone())
    }
}

impl Default for Preemption {
    fn default() -> Self {
        Self::new()
    }
}

pub trait Channel: Send + Sync {
    fn id(&self) -> &str;
    /// 启动 Channel；到达终态（发送/取消）时向 sink 投递一次结果。
    fn start(&self, request: &AskRequest, origin: &ConversationOrigin, sink: ResultSink);
    /// Interrupt this channel before it produced a result, finalizing its UI per `reason`
    /// (preempted by a winner, or the whole request cancelled). Does not deliver a result.
    fn interrupt(&self, reason: &Interruption);
}
