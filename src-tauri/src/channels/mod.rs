//! Channel 抽象：并行运行、首个终态结果由协调器采纳，其余被 `cancel_by_other` 收尾。

pub mod popup;
pub mod telegram;

use crate::app::coordinator::Coordinator;
use crate::models::AskRequest;
use std::sync::Arc;

/// 投递结果的句柄（协调器，线程安全）。
pub type ResultSink = Arc<Coordinator>;

pub trait Channel: Send + Sync {
    fn id(&self) -> &str;
    /// 启动 Channel；到达终态（发送/取消）时向 sink 投递一次结果。
    fn start(&self, request: &AskRequest, sink: ResultSink);
    /// 被其他 Channel 抢答后收尾（不再投递）。
    fn cancel_by_other(&self);
}
