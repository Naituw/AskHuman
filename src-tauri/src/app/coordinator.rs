//! 抢答协调器：并行 Channel 的首个终态结果生效，其余被 `cancel_by_other` 收尾。
//!
//! 收到首个结果后不立即退出，而是给落败渠道一个**收尾窗口**（最多 ~2s，事件驱动、提前结束）
//! 把卡片改成终态（钉钉灰显「已提交」、Telegram 编辑卡片为「已回答」等），随后输出结果并退出。

use crate::channels::{Channel, Preemption};
use crate::i18n::{self, Lang};
use crate::models::{AskRequest, ChannelResult};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

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
}

pub struct Coordinator {
    inner: Mutex<Inner>,
    /// 仍在收尾的落败「消息渠道」数（弹窗瞬时关闭，不计入）。
    pending: Arc<AtomicUsize>,
    /// 已采纳的终态结果（首个 submit 写入）。
    result: Mutex<Option<ChannelResult>>,
    /// 是否已进入收尾阶段（首个 submit 后置位）。GUI 据此拦下「关窗即退出」，
    /// 仅放行协调器自身的 `app.exit`，确保结果先输出；收尾前不拦（Cmd+Q 等照常退出）。
    finalizing: AtomicBool,
    /// 结果是否已输出（保证只输出 / 退出一次）。
    emitted: AtomicBool,
}

struct Inner {
    finished: bool,
    exiter: Exiter,
    request: AskRequest,
    channels: Vec<Arc<dyn Channel>>,
    /// headless 模式：共享抢答信号 + 消息渠道总数（用于算落败数）。GUI 为 None。
    headless: Option<(Arc<Preemption>, usize)>,
}

impl Coordinator {
    /// GUI 模式协调器。
    pub fn new(app: AppHandle, request: AskRequest) -> Arc<Self> {
        Self::build(Exiter::Gui(app), request, None)
    }

    /// headless 模式协调器（无 GUI，结果到达后直接退出进程）。
    /// `preempt` 为各会话共享的抢答信号；`messaging_count` 为并行消息渠道数。
    pub fn new_headless(
        request: AskRequest,
        preempt: Arc<Preemption>,
        messaging_count: usize,
    ) -> Arc<Self> {
        Self::build(Exiter::Process, request, Some((preempt, messaging_count)))
    }

    fn build(
        exiter: Exiter,
        request: AskRequest,
        headless: Option<(Arc<Preemption>, usize)>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                finished: false,
                exiter,
                request,
                channels: Vec::new(),
                headless,
            }),
            pending: Arc::new(AtomicUsize::new(0)),
            result: Mutex::new(None),
            finalizing: AtomicBool::new(false),
            emitted: AtomicBool::new(false),
        })
    }

    /// 是否已进入收尾阶段（供 GUI 事件循环决定是否拦下关窗退出）。
    pub fn is_finalizing(&self) -> bool {
        self.finalizing.load(Ordering::SeqCst)
    }

    pub fn register(&self, channel: Arc<dyn Channel>) {
        self.inner.lock().unwrap().channels.push(channel);
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
            *self.result.lock().unwrap() = Some(result);

            let lang = Lang::current();
            let winner = display_name(&source, lang);

            let pending = match &inner.headless {
                // headless：取消共享信号；落败数 = 渠道数 - 1（赢家）。
                Some((preempt, count)) => {
                    preempt.cancel(&winner);
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
                        ch.cancel_by_other(&winner);
                    }
                    losers.iter().filter(|c| c.id() != "popup").count()
                }
            };
            (inner.exiter.clone(), pending)
        };

        self.pending.store(pending_count, Ordering::SeqCst);

        // GUI：立即关闭弹窗（赢家是弹窗时它不在 losers 中，需显式关）。
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
            Exiter::Process => {
                tokio::spawn(waiter);
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
        let (exiter, request_id) = {
            let inner = self.inner.lock().unwrap();
            (inner.exiter.clone(), inner.request.id.clone())
        };
        let result = self.result.lock().unwrap().take();
        let Some(result) = result else {
            // 无结果（headless 全部会话结束仍未作答）：不退出，交由调用方报错。
            return;
        };
        let code = super::emit_result(&request_id, &result);
        let _ = std::io::stdout().flush();
        match exiter {
            Exiter::Gui(app) => app.exit(code),
            Exiter::Process => std::process::exit(code),
        }
    }
}

/// 渠道 id → 赢家端展示名（按界面语言）。
fn display_name(id: &str, lang: Lang) -> String {
    match id {
        "popup" => i18n::tr(lang, "channel.sourcePopup").to_string(),
        "telegram" => i18n::tr(lang, "channel.sourceTelegram").to_string(),
        "dingding" => i18n::tr(lang, "channel.sourceDingTalk").to_string(),
        "feishu" => i18n::tr(lang, "channel.sourceFeishu").to_string(),
        other => other.to_string(),
    }
}
