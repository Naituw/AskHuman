//! Channel 抽象：并行运行、首个终态结果由协调器采纳，其余被 `cancel_by_other` 收尾。

pub mod conversation;
pub mod dingding;
pub mod feishu;
pub mod popup;
pub mod telegram;

use crate::app::coordinator::Coordinator;
use crate::models::AskRequest;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 投递结果的句柄（协调器，线程安全）。
pub type ResultSink = Arc<Coordinator>;

/// 抢答信号：被其他端抢答时置位，并记录赢家的展示名（供收尾文案点名）。
/// 在「外层 Channel / 收尾」与「会话任务」之间共享（Arc）。
pub struct Preemption {
    cancelled: AtomicBool,
    winner: Mutex<String>,
}

impl Preemption {
    pub fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            winner: Mutex::new(String::new()),
        }
    }

    /// 标记被 `winner`（展示名）抢答。
    pub fn cancel(&self, winner: &str) {
        if let Ok(mut w) = self.winner.lock() {
            *w = winner.to_string();
        }
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 赢家展示名（未抢答时为空）。
    pub fn winner(&self) -> String {
        self.winner.lock().map(|w| w.clone()).unwrap_or_default()
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
    /// 被其他 Channel 抢答后收尾（不再投递）；`winner` 为赢家端展示名。
    fn cancel_by_other(&self, winner: &str);
}
