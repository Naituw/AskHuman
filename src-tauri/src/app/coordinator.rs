//! 抢答协调器：并行 Channel 的首个终态结果生效，其余被 `interrupt` 收尾。
//!
//! 收到首个结果后不立即退出，而是给落败渠道一个**收尾窗口**（最多 ~2s，事件驱动、提前结束）
//! 把卡片改成终态（钉钉灰显「已提交」、Telegram 编辑卡片为「已回答」等），随后输出结果并退出。

use super::{RenderOutcome, RequestOutcome};
use crate::channels::{Channel, Interruption, Preemption};
use crate::i18n::{self, Lang};
use crate::models::{AskRequest, ChannelAction, ChannelResult};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc::UnboundedSender;

/// 收尾窗口上限：超过即强制退出，保证进程不会因某端收尾卡住而挂起。
/// 事件驱动为主（落败端收尾完成即提前退出），此上限仅为兜底；取值偏宽以容忍
/// 跨网络编辑卡片（如代理下访问 Telegram）较慢的情况。
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(5);

/// 拿到结果后如何退出进程。
#[derive(Clone)]
pub enum Exiter {
    /// GUI 模式：经 Tauri 事件循环退出（携带退出码）。
    Gui(AppHandle),
    /// headless 模式：直接退出进程。
    Process,
    /// Daemon 模式（Ask）：渲染结果经通道回传连接处理器。
    Ipc(UnboundedSender<RenderOutcome>),
    /// Daemon 模式（Confirm）：结构化终态经通道回传连接处理器。
    IpcConfirm(UnboundedSender<RequestOutcome>),
}

/// 协调器交互模式：决定 `finish()` 的行为。
#[derive(Clone)]
pub enum CoordinatorMode {
    /// 普通 Ask：渲染文本、写历史、输出 stdout。
    Ask { request: AskRequest },
    /// 权限确认：映射到 action_id、不写历史。
    Confirm {
        confirm_action_id: String,
        cancel_action_id: String,
    },
}

pub struct Coordinator {
    inner: Mutex<Inner>,
    /// 结果渲染 / 收尾文案使用的界面语言（Daemon 模式为调用方上送的 lang；单进程为 `Lang::current()`）。
    lang: Lang,
    /// 当前项目 key（用于回复历史归类；可空）。
    project: String,
    /// 调用方来源名（写入回复历史；可空）。
    source: String,
    /// 调用方 agent 家族（claude/codex/cursor/grok，写入回复历史；可空）。
    /// 内部可变：daemon 异步 walk 进程树解析完成后经 `set_agent_kind` 回填
    /// （MCP 模式 env 判不出家族，只有这条路能拿到），`finish` 落历史时取最新值。
    agent_kind: Mutex<Option<String>>,
    /// 仍在收尾的落败「消息渠道」数（弹窗瞬时关闭，不计入）。
    pending: Arc<AtomicUsize>,
    /// 已采纳的终态结果（首个 submit 写入）。
    result: Mutex<Option<ChannelResult>>,
    /// 赢家渠道 id（首个 submit 写入；与 `result` 不同，`finish` 不会取走，供作答后更新活跃槽读取）。
    winner: Mutex<Option<String>>,
    /// 是否已进入收尾阶段（首个 submit 后置位）。GUI 据此拦下「关窗即退出」，
    /// 仅放行协调器自身的 `app.exit`，确保结果先输出；收尾前不拦（Cmd+Q 等照常退出）。
    finalizing: AtomicBool,
    /// 结果是否已输出（保证只输出 / 退出一次）。
    emitted: AtomicBool,
}

struct Inner {
    finished: bool,
    exiter: Exiter,
    mode: CoordinatorMode,
    channels: Vec<Arc<dyn Channel>>,
    /// headless 模式：共享抢答信号 + 消息渠道总数（用于算落败数）。GUI 为 None。
    headless: Option<(Arc<Preemption>, usize)>,
}

impl Coordinator {
    /// GUI 模式协调器。`project` / `source` / `agent_kind` 写入回复历史（可空）。
    pub fn new(
        app: AppHandle,
        request: AskRequest,
        project: String,
        source: String,
        agent_kind: Option<String>,
    ) -> Arc<Self> {
        Self::build(
            Exiter::Gui(app),
            CoordinatorMode::Ask { request },
            None,
            Lang::current(),
            project,
            source,
            agent_kind,
        )
    }

    /// headless 模式协调器（无 GUI，结果到达后直接退出进程）。
    /// `preempt` 为各会话共享的抢答信号；`messaging_count` 为并行消息渠道数。
    pub fn new_headless(
        request: AskRequest,
        preempt: Arc<Preemption>,
        messaging_count: usize,
        project: String,
        source: String,
    ) -> Arc<Self> {
        Self::build(
            Exiter::Process,
            CoordinatorMode::Ask { request },
            Some((preempt, messaging_count)),
            Lang::current(),
            project,
            source,
            None,
        )
    }

    /// Daemon 模式协调器（Ask）：结果到达后渲染并经 `tx` 回传，不退出进程。
    /// `lang` 为调用方上送的界面语言（A11，使 `auto` 跟随调用方）。
    pub fn new_ipc(
        request: AskRequest,
        lang: Lang,
        tx: UnboundedSender<RenderOutcome>,
        project: String,
        source: String,
        agent_kind: Option<String>,
    ) -> Arc<Self> {
        Self::build(
            Exiter::Ipc(tx),
            CoordinatorMode::Ask { request },
            None,
            lang,
            project,
            source,
            agent_kind,
        )
    }

    /// Daemon 模式协调器（Confirm）：权限确认，不写历史，终态为结构化 action_id。
    pub fn new_confirm_ipc(
        confirm_action_id: String,
        cancel_action_id: String,
        lang: Lang,
        tx: UnboundedSender<RequestOutcome>,
    ) -> Arc<Self> {
        Self::build(
            Exiter::IpcConfirm(tx),
            CoordinatorMode::Confirm {
                confirm_action_id,
                cancel_action_id,
            },
            None,
            lang,
            String::new(),
            String::new(),
            None,
        )
    }

    fn build(
        exiter: Exiter,
        mode: CoordinatorMode,
        headless: Option<(Arc<Preemption>, usize)>,
        lang: Lang,
        project: String,
        source: String,
        agent_kind: Option<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                finished: false,
                exiter,
                mode,
                channels: Vec::new(),
                headless,
            }),
            lang,
            project,
            source,
            agent_kind: Mutex::new(agent_kind),
            pending: Arc::new(AtomicUsize::new(0)),
            result: Mutex::new(None),
            winner: Mutex::new(None),
            finalizing: AtomicBool::new(false),
            emitted: AtomicBool::new(false),
        })
    }

    /// 是否已进入收尾阶段（供 GUI 事件循环决定是否拦下关窗退出）。
    pub fn is_finalizing(&self) -> bool {
        self.finalizing.load(Ordering::SeqCst)
    }

    /// 回填调用方 agent 家族（daemon 异步 walk 解析完成后调用；覆盖 env 探测值或 None）。
    pub fn set_agent_kind(&self, kind: String) {
        *self.agent_kind.lock().unwrap() = Some(kind);
    }

    pub fn register(&self, channel: Arc<dyn Channel>) {
        self.inner.lock().unwrap().channels.push(channel);
    }

    /// 是否已登记某个渠道（按 id）。用于「补推在途」时避免对同一渠道重复挂接 / 重发卡片。
    pub fn has_channel(&self, id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .channels
            .iter()
            .any(|c| c.id() == id)
    }

    /// 赢家渠道 id（终态结果的来源；未作答 / 系统取消时为 None）。供作答后把活跃槽更新为该渠道。
    pub fn winner_channel_id(&self) -> Option<String> {
        self.winner.lock().unwrap().clone()
    }

    /// 投递终态结果：仅首个生效；随后取消其余 Channel 并启动收尾窗口，到时输出并退出。
    pub fn submit(self: &Arc<Self>, result: ChannelResult) {
        let (exiter, pending_count) = {
            let mut inner = self.inner.lock().unwrap();
            if inner.finished {
                return;
            }
            inner.finished = true;
            // 进入收尾：此后 GUI 拦下关窗退出，独占由协调器主动 `app.exit`。
            self.finalizing.store(true, Ordering::SeqCst);
            let source = result.source_channel_id.clone();
            let action = result.action;
            *self.winner.lock().unwrap() = Some(source.clone());
            *self.result.lock().unwrap() = Some(result);

            let lang = self.lang;
            let winner = display_name(&source, lang);
            // Reason for interrupting the losing channels: a real answer (Send) attributes the
            // winner ("Answered via X"); a popup Cancel means the whole request was cancelled by
            // that source ("Cancelled by Popup").
            let reason = match action {
                ChannelAction::Send => Interruption::AnsweredBy(winner.clone()),
                ChannelAction::Cancel => Interruption::Cancelled(winner.clone()),
            };

            let pending = match &inner.headless {
                // headless：取消共享信号；落败数 = 渠道数 - 1（赢家）。
                Some((preempt, count)) => {
                    preempt.interrupt(reason.clone());
                    count.saturating_sub(1)
                }
                // GUI：逐个取消落败渠道；弹窗瞬时关闭不计入收尾等待。
                None => {
                    let losers: Vec<Arc<dyn Channel>> = inner
                        .channels
                        .iter()
                        .filter(|c| c.id() != source)
                        .cloned()
                        .collect();
                    for ch in &losers {
                        ch.interrupt(&reason);
                    }
                    losers.iter().filter(|c| c.id() != "popup").count()
                }
            };
            (inner.exiter.clone(), pending)
        };

        self.pending.store(pending_count, Ordering::SeqCst);

        // GUI（单进程）：立即关闭弹窗（赢家是弹窗时它不在 losers 中，需显式关）。
        // Daemon 模式弹窗在独立 GUI Helper 进程，关窗由其自身收到 cancel / 连接断开处理，此处不涉及。
        if let Exiter::Gui(app) = &exiter {
            if let Some(w) = app.get_webview_window("popup") {
                let _ = w.close();
            }
        }

        // 收尾窗口：等落败端收尾完成（pending 归零）或 2s 超时后输出并退出。
        let me = Arc::clone(self);
        let pending = self.pending.clone();
        let waiter = async move {
            let deadline = Instant::now() + FINALIZE_TIMEOUT;
            while pending.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            me.finish();
        };
        match exiter {
            Exiter::Gui(_) => {
                tauri::async_runtime::spawn(waiter);
            }
            Exiter::Process | Exiter::Ipc(_) | Exiter::IpcConfirm(_) => {
                tokio::spawn(waiter);
            }
        }
    }

    /// Cancel the whole request (CLI disconnected / `daemon stop`): interrupt every channel as
    /// `Cancelled(source)` so all cards finalize to a cancelled state and the popup closes.
    /// Unlike `submit`, this does not render or deliver a result (no one is waiting). No-op if a
    /// result was already submitted. `source` is the localized cancel source ("Caller"; empty = generic).
    pub fn cancel_request(&self, source: String) {
        let mut inner = self.inner.lock().unwrap();
        if inner.finished {
            return;
        }
        inner.finished = true;
        let reason = Interruption::Cancelled(source);
        match &inner.headless {
            Some((preempt, _)) => preempt.interrupt(reason),
            None => {
                for ch in &inner.channels {
                    ch.interrupt(&reason);
                }
            }
        }
    }

    /// 一个落败渠道完成收尾时调用：未归零则减一（用于提前结束收尾窗口）。
    pub fn notify_finalized(&self) {
        let _ = self
            .pending
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                if v > 0 {
                    Some(v - 1)
                } else {
                    None
                }
            });
    }

    /// 输出已采纳结果并退出（只生效一次）。无结果时直接返回，交由调用方兜底。
    pub fn finish(&self) {
        if self.emitted.swap(true, Ordering::SeqCst) {
            return;
        }
        let (exiter, mode) = {
            let inner = self.inner.lock().unwrap();
            (inner.exiter.clone(), inner.mode.clone())
        };
        let result = self.result.lock().unwrap().take();
        let Some(result) = result else {
            return;
        };

        match mode {
            CoordinatorMode::Ask { request } => self.finish_ask(exiter, &request, result),
            CoordinatorMode::Confirm {
                confirm_action_id,
                cancel_action_id,
            } => self.finish_confirm(exiter, &confirm_action_id, &cancel_action_id, result),
        }
    }

    fn finish_ask(&self, exiter: Exiter, request: &AskRequest, result: ChannelResult) {
        let (outcome, image_paths) = super::render_result(request, &result, self.lang);
        self.record_history(request, &result, &image_paths);
        if let Exiter::Ipc(tx) = &exiter {
            let _ = tx.send(outcome);
            return;
        }
        if let Some(err) = &outcome.stderr {
            super::stderr_redirect::eprintln_real(err);
        } else {
            println!("{}", outcome.stdout);
        }
        let _ = std::io::stdout().flush();
        match exiter {
            Exiter::Gui(app) => app.exit(outcome.exit_code),
            Exiter::Process => std::process::exit(outcome.exit_code),
            Exiter::Ipc(_) => unreachable!("handled above"),
            Exiter::IpcConfirm(_) => unreachable!("Ask mode cannot use IpcConfirm exiter"),
        }
    }

    fn finish_confirm(
        &self,
        exiter: Exiter,
        confirm_action_id: &str,
        cancel_action_id: &str,
        result: ChannelResult,
    ) {
        let action_id = match result.action {
            ChannelAction::Send => confirm_action_id.to_string(),
            ChannelAction::Cancel => cancel_action_id.to_string(),
        };
        let outcome = RequestOutcome::Confirm {
            action_id,
            source_channel_id: result.source_channel_id,
        };
        if let Exiter::IpcConfirm(tx) = &exiter {
            let _ = tx.send(outcome);
        }
    }

    /// Append a reply-history entry for this terminal result (best-effort side channel).
    ///
    /// Every result reaching `finish` is user-initiated (a Send, or a popup/IM Cancel); system
    /// cancels go through `cancel_request` and carry no result, so they never get here — which is
    /// exactly the "only user-initiated cancels" policy. Image/file values are stored as paths.
    fn record_history(
        &self,
        request: &AskRequest,
        result: &ChannelResult,
        image_paths: &[Vec<String>],
    ) {
        // 仅需 history_limit（general）；用 load_without_secrets() 避免每条回答落历史都读钥匙串。
        let limit = crate::config::AppConfig::load_without_secrets()
            .general
            .history_limit;
        if limit == 0 {
            return;
        }
        let answers = match result.action {
            ChannelAction::Cancel => Vec::new(),
            ChannelAction::Send => result
                .answers
                .iter()
                .enumerate()
                .map(|(i, a)| crate::history::HistoryAnswer {
                    selected_options: a.selected_options.clone(),
                    user_input: a.user_input.clone(),
                    images: image_paths.get(i).cloned().unwrap_or_default(),
                    files: a.files.clone(),
                })
                .collect(),
        };
        let entry = crate::history::HistoryEntry {
            id: request.id.clone(),
            timestamp_ms: crate::history::now_ms(),
            project: self.project.clone(),
            source: self.source.clone(),
            agent_kind: self.agent_kind.lock().unwrap().clone(),
            channel: result.source_channel_id.clone(),
            action: result.action,
            is_markdown: request.is_markdown,
            message: request.message.clone(),
            questions: request.questions.clone(),
            answers,
        };
        crate::history::record(entry, limit);
    }
}

/// 渠道 id → 赢家端展示名（按界面语言）。
fn display_name(id: &str, lang: Lang) -> String {
    match id {
        "popup" => i18n::tr(lang, "channel.sourcePopup").to_string(),
        "telegram" => i18n::tr(lang, "channel.sourceTelegram").to_string(),
        "dingding" => i18n::tr(lang, "channel.sourceDingTalk").to_string(),
        "feishu" => i18n::tr(lang, "channel.sourceFeishu").to_string(),
        "slack" => i18n::tr(lang, "channel.sourceSlack").to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChannelAction, ChannelResult};

    /// Confirm coordinator: Send action maps to confirm_action_id.
    #[tokio::test]
    async fn confirm_send_maps_to_confirm_action_id() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let coord =
            Coordinator::new_confirm_ipc("allow".to_string(), "deny".to_string(), Lang::En, tx);

        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: Vec::new(),
            source_channel_id: "popup".to_string(),
        };
        coord.submit(result);

        tokio::time::sleep(Duration::from_millis(200)).await;
        let outcome = rx.try_recv().unwrap();
        match outcome {
            RequestOutcome::Confirm {
                action_id,
                source_channel_id,
            } => {
                assert_eq!(action_id, "allow");
                assert_eq!(source_channel_id, "popup");
            }
            other => panic!("expected Confirm, got {:?}", other),
        }
    }

    /// Confirm coordinator: Cancel action maps to cancel_action_id.
    #[tokio::test]
    async fn confirm_cancel_maps_to_cancel_action_id() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let coord =
            Coordinator::new_confirm_ipc("allow".to_string(), "deny".to_string(), Lang::En, tx);

        let result = ChannelResult {
            action: ChannelAction::Cancel,
            answers: Vec::new(),
            source_channel_id: "telegram".to_string(),
        };
        coord.submit(result);

        tokio::time::sleep(Duration::from_millis(200)).await;
        let outcome = rx.try_recv().unwrap();
        match outcome {
            RequestOutcome::Confirm {
                action_id,
                source_channel_id,
            } => {
                assert_eq!(action_id, "deny");
                assert_eq!(source_channel_id, "telegram");
            }
            other => panic!("expected Confirm, got {:?}", other),
        }
    }

    /// First-answer uniqueness: second submit is ignored.
    #[tokio::test]
    async fn confirm_first_answer_wins() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let coord =
            Coordinator::new_confirm_ipc("allow".to_string(), "deny".to_string(), Lang::En, tx);

        coord.submit(ChannelResult {
            action: ChannelAction::Send,
            answers: Vec::new(),
            source_channel_id: "popup".to_string(),
        });
        coord.submit(ChannelResult {
            action: ChannelAction::Cancel,
            answers: Vec::new(),
            source_channel_id: "telegram".to_string(),
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        let outcome = rx.try_recv().unwrap();
        assert!(matches!(
            outcome,
            RequestOutcome::Confirm {
                ref action_id,
                ..
            } if action_id == "allow"
        ));
        assert!(rx.try_recv().is_err());
    }
}
