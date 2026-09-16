//! 渠道健康登记表（进程内存态，R7 渠道故障可见化）。
//!
//! 四个平台 client 的统一请求出口（`call` 系列）失败时登记、成功时清除，daemon 的
//! `ensure_*_router` 建连失败同样登记——覆盖建连 / 投放发送 / 卡片编辑（含 watch）全部路径，
//! 无须在每个 `eprintln!` 点各自插桩。daemon 经 `TrayState` / `StatusInfo` 把快照带给
//! 托盘与设置页；legacy in-process/test callers may also write this table without readers.
//!
//! 语义（用户定案）：该渠道下一次任何成功操作即清；纯内存态，daemon 重启即清。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 一条渠道故障记录（`channel` 为渠道 id："telegram" / "dingding" / "feishu" / "slack"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelIssue {
    pub channel: String,
    pub message: String,
    /// 首次出现的 Unix 毫秒时间戳（同一错误重复发生不刷新，避免托盘签名抖动）。
    pub at_ms: u64,
}

type Notifier = Box<dyn Fn() + Send + Sync>;

fn registry() -> &'static Mutex<HashMap<String, ChannelIssue>> {
    static R: OnceLock<Mutex<HashMap<String, ChannelIssue>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn notifier_slot() -> &'static Mutex<Option<Notifier>> {
    static N: OnceLock<Mutex<Option<Notifier>>> = OnceLock::new();
    N.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 登记变化时的回调（daemon 启动时挂 `broadcast_tray_state`）。仅在记录**内容**变化
/// （新错误 / 错误文案变化 / 清除）时触发，同一错误反复发生不重复通知。
pub fn set_notifier(f: impl Fn() + Send + Sync + 'static) {
    *notifier_slot().lock().unwrap() = Some(Box::new(f));
}

fn notify() {
    if let Some(f) = notifier_slot().lock().unwrap().as_ref() {
        f();
    }
}

/// 登记一条渠道故障。同渠道同文案已在表中则只保留原时间戳、不通知。
pub fn report(channel: &str, message: impl Into<String>) {
    let message = message.into();
    let changed = {
        let mut r = registry().lock().unwrap();
        match r.get(channel) {
            Some(prev) if prev.message == message => false,
            _ => {
                r.insert(
                    channel.to_string(),
                    ChannelIssue {
                        channel: channel.to_string(),
                        message,
                        at_ms: now_ms(),
                    },
                );
                true
            }
        }
    };
    if changed {
        notify();
    }
}

/// 清除某渠道的故障记录（任何成功操作后调用）。无记录时是廉价 no-op。
pub fn clear(channel: &str) {
    let changed = registry().lock().unwrap().remove(channel).is_some();
    if changed {
        notify();
    }
}

/// 当前全部故障记录的快照（按渠道 id 排序，输出稳定）。
pub fn snapshot() -> Vec<ChannelIssue> {
    let mut v: Vec<ChannelIssue> = registry().lock().unwrap().values().cloned().collect();
    v.sort_by(|a, b| a.channel.cmp(&b.channel));
    v
}

// ===== Long-connection reconnect policy (shared by the DingTalk / Feishu / Slack routers) =====

/// First retry delay after a long connection drops.
const RECONNECT_BASE: std::time::Duration = std::time::Duration::from_millis(500);
/// Ceiling for the exponential backoff. A daemon that lost its network keeps probing at this
/// cadence for as long as it lives; the in-flight cards stay valid and resume once it reconnects.
const RECONNECT_CAP: std::time::Duration = std::time::Duration::from_secs(30);

/// Delay before reconnect `attempt` (0-based): 0.5 s doubling up to the 30 s cap. The routers
/// never give up — a bounded budget used to turn every longer outage into a card falsely marked
/// "cancelled" while the request kept waiting.
pub fn reconnect_delay(attempt: u32) -> std::time::Duration {
    let factor = 1u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
    RECONNECT_BASE
        .checked_mul(factor)
        .map(|d| d.min(RECONNECT_CAP))
        .unwrap_or(RECONNECT_CAP)
}

/// Whether reconnect `attempt` (0-based) deserves a log line: the first three attempts, then
/// every tenth, so a multi-hour outage does not flood `daemon.log`.
pub fn should_log_reconnect(attempt: u32) -> bool {
    attempt < 3 || attempt % 10 == 0
}

/// Message recorded in the health table while a router is between connections.
pub fn reconnecting_message(attempt: u32, error: &str) -> String {
    format!(
        "long connection lost; reconnecting (attempt {}): {}",
        attempt + 1,
        error
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // 注册表是进程全局的，测试串行化以免互相污染。
    fn with_clean_registry(f: impl FnOnce()) {
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap();
        registry().lock().unwrap().clear();
        *notifier_slot().lock().unwrap() = None;
        f();
        registry().lock().unwrap().clear();
        *notifier_slot().lock().unwrap() = None;
    }

    #[test]
    fn report_clear_snapshot_roundtrip() {
        with_clean_registry(|| {
            report("telegram", "poll failed");
            report("dingding", "token expired");
            let snap = snapshot();
            assert_eq!(snap.len(), 2);
            // 输出按渠道 id 排序。
            assert_eq!(snap[0].channel, "dingding");
            assert_eq!(snap[1].channel, "telegram");
            clear("telegram");
            let snap = snapshot();
            assert_eq!(snap.len(), 1);
            assert_eq!(snap[0].channel, "dingding");
        });
    }

    #[test]
    fn repeated_same_error_keeps_timestamp_and_skips_notify() {
        with_clean_registry(|| {
            let hits = Arc::new(AtomicUsize::new(0));
            let h = hits.clone();
            set_notifier(move || {
                h.fetch_add(1, Ordering::SeqCst);
            });
            report("slack", "invalid_auth");
            let first = snapshot()[0].at_ms;
            report("slack", "invalid_auth");
            assert_eq!(snapshot()[0].at_ms, first);
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            // 文案变化 → 视为新错误，通知。
            report("slack", "channel_not_found");
            assert_eq!(hits.load(Ordering::SeqCst), 2);
            // 清除已有记录 → 通知；再清是 no-op。
            clear("slack");
            assert_eq!(hits.load(Ordering::SeqCst), 3);
            clear("slack");
            assert_eq!(hits.load(Ordering::SeqCst), 3);
        });
    }

    #[test]
    fn reconnect_delay_doubles_from_half_a_second_and_caps_at_thirty_seconds() {
        use std::time::Duration;
        assert_eq!(reconnect_delay(0), Duration::from_millis(500));
        assert_eq!(reconnect_delay(1), Duration::from_secs(1));
        assert_eq!(reconnect_delay(2), Duration::from_secs(2));
        assert_eq!(reconnect_delay(5), Duration::from_secs(16));
        assert_eq!(reconnect_delay(6), Duration::from_secs(30));
        assert_eq!(reconnect_delay(7), Duration::from_secs(30));
        // No overflow, no panic, still capped for absurd attempt counts.
        assert_eq!(reconnect_delay(40), Duration::from_secs(30));
        assert_eq!(reconnect_delay(u32::MAX), Duration::from_secs(30));
    }

    #[test]
    fn reconnect_logging_is_front_loaded_then_sparse() {
        assert!(should_log_reconnect(0));
        assert!(should_log_reconnect(1));
        assert!(should_log_reconnect(2));
        assert!(!should_log_reconnect(3));
        assert!(!should_log_reconnect(9));
        assert!(should_log_reconnect(10));
        assert!(!should_log_reconnect(11));
        assert!(should_log_reconnect(20));
        assert_eq!(
            reconnecting_message(0, "tls handshake eof"),
            "long connection lost; reconnecting (attempt 1): tls handshake eof"
        );
    }
}
