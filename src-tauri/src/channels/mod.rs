//! Channel 抽象：并行运行、首个终态结果由协调器采纳，其余被 `interrupt` 收尾。

pub mod confirm;
pub mod conversation;
pub mod dingding;
pub mod feishu;
pub mod popup;
pub mod slack;
pub mod telegram;

use crate::app::coordinator::Coordinator;
use crate::models::AskRequest;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 投递结果的句柄（协调器，线程安全）。
pub type ResultSink = Arc<Coordinator>;

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
    fn start(&self, request: &AskRequest, sink: ResultSink);
    /// Interrupt this channel before it produced a result, finalizing its UI per `reason`
    /// (preempted by a winner, or the whole request cancelled). Does not deliver a result.
    fn interrupt(&self, reason: &Interruption);
}
