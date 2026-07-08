//! 通用「单选卡」（spec `docs/specs/im-select-card.md`）：与传输无关的纯逻辑。
//!
//! 一张单选卡 = `标题 + 一组选项 + 一个动作`。每个选项 = `{稳定 id, 状态圆点, 展示编号, 主文本,
//! 徽标, 次行}`；命令侧只负责「给出选项列表 + 选中后做什么」，各渠道渲染器（飞书
//! `feishu/card.rs::build_select_card`，后续 TG/Slack/钉钉）把它渲染成一行行「信息 + 触发按钮」，
//! 单击即触发（回调 value `{select:<idx>}`）。
//!
//! 渲染布局（飞书，用户定稿「方案A」）：每个选项一行 = 左侧小字号两行富文本
//! （第一行 `圆点 [编号] 主文本 · 徽标`，第二行灰色次行＝标题）+ 右侧一枚紧凑按钮（文案随动作）。
//! 数据保持「无标记语言」：圆点/加粗/颜色等标记由各渠道渲染器自行拼装（跨渠道可各异）。

use crate::i18n::{self, Lang};
use serde_json::Value;
use std::collections::HashSet;

/// 单卡最多渲染的选项数（超出截断并在标题追加说明）。日常 agent 数一般 < 10。
pub const SELECT_MAX_OPTIONS: usize = 20;

/// 选项状态圆点（agent 场景）。渲染器映射到各渠道的颜色/字符（飞书 = markdown 彩色 `●`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectDot {
    /// 工作中（绿）。
    Working,
    /// 空闲（灰）。
    Idle,
}

/// 单选卡的动作种类（整卡统一）：决定每行触发按钮的文案与样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectAction {
    Watch,
    Status,
    Unwatch,
    /// 发送插话（`/msg` 无编号时的选择卡；按钮「发送」，点它把预存内容发给该 agent）。
    Msg,
}

impl SelectAction {
    /// 按钮本地化文案。
    pub fn button_label(self, lang: Lang) -> String {
        let key = match self {
            SelectAction::Watch => "select.btnWatch",
            SelectAction::Status => "select.btnStatus",
            SelectAction::Unwatch => "select.btnUnwatch",
            SelectAction::Msg => "select.btnMsg",
        };
        i18n::tr(lang, key).to_string()
    }
}

/// 一个可点选项（传输无关，无标记语言）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    /// 稳定标识（agent 场景 = session_id）。点击后据此定位领域对象（不用会漂移的展示编号）。
    pub id: String,
    /// 状态圆点（None = 不显示圆点）。
    pub dot: Option<SelectDot>,
    /// 展示编号（`[n]`，None = 不显示）。
    pub seq: Option<u64>,
    /// 主文本（编号之后、徽标之前）：agent 场景 = `类型 · 工作目录名`。
    pub primary: String,
    /// 主行末徽标（如「· 关注中」，None = 无）。
    pub badge: Option<String>,
    /// 主行末「· 已运行 X」运行时长（发卡那一刻的快照值，便于区分 agent；None = 无 `startedAt`）。
    /// 渲染在徽标之后。时长复用 watch 卡算法（`watch::fmt_duration`），始终显示（含 <60 秒）。
    pub elapsed: Option<String>,
    /// 次行（灰、可换行）：agent 场景 = 标题。
    pub secondary: Option<String>,
}

/// 一张单选卡的渲染数据（渲染器消费）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectView {
    /// 卡片标题 / 提示。
    pub title: String,
    /// 选项列表（已按上限截断）。
    pub options: Vec<SelectOption>,
    /// 截断说明（选项超上限时的「（仅列前 N 个）」，否则 None）。
    pub truncated_note: Option<String>,
    /// 动作（决定每行按钮文案/样式）。
    pub action: SelectAction,
}

/// 组装单选卡视图：超 `SELECT_MAX_OPTIONS` 时截断并置截断说明。
pub fn build_view(
    title: String,
    mut options: Vec<SelectOption>,
    action: SelectAction,
    lang: Lang,
) -> SelectView {
    let truncated_note = if options.len() > SELECT_MAX_OPTIONS {
        options.truncate(SELECT_MAX_OPTIONS);
        Some(i18n::tr(lang, "select.truncated").replace("{n}", &SELECT_MAX_OPTIONS.to_string()))
    } else {
        None
    };
    SelectView {
        title,
        options,
        truncated_note,
        action,
    }
}

// ── 命令种类标题（本地化）──

pub fn title_watch(lang: Lang) -> String {
    i18n::tr(lang, "select.titleWatch").to_string()
}
pub fn title_status(lang: Lang) -> String {
    i18n::tr(lang, "select.titleStatus").to_string()
}
pub fn title_unwatch(lang: Lang) -> String {
    i18n::tr(lang, "select.titleUnwatch").to_string()
}
pub fn title_msg(lang: Lang) -> String {
    i18n::tr(lang, "select.titleMsg").to_string()
}

/// 由一条注册表快照记录组装选项字段（`dot / seq / primary=类型·工作目录名 / elapsed=已运行时长 /
/// secondary=标题`）；`sid` 已由调用方取好。`watching` 命中则加「· 关注中」徽标。`now`＝当前 epoch 秒，
/// 用于据 `startedAt` 算运行时长。
fn option_from_record(
    rec: &Value,
    sid: String,
    watching: &HashSet<String>,
    now: u64,
    lang: Lang,
) -> SelectOption {
    let dot = match rec.get("state").and_then(|v| v.as_str()) {
        Some("working") => Some(SelectDot::Working),
        Some("idle") => Some(SelectDot::Idle),
        _ => None,
    };
    let seq = rec.get("seq").and_then(|v| v.as_u64());
    let badge = if watching.contains(&sid) {
        Some(i18n::tr(lang, "select.watchingBadge").trim().to_string())
    } else {
        None
    };
    // 运行时长只对「工作中」显示（用户定案：空闲 agent 显示「已运行」易误导，直接不显示）。
    let elapsed = (dot == Some(SelectDot::Working))
        .then(|| elapsed_badge(rec, now, lang))
        .flatten();
    SelectOption {
        id: sid,
        dot,
        seq,
        primary: primary_text(rec, lang),
        badge,
        elapsed,
        secondary: Some(title_text(rec, lang)),
    }
}

/// 主行末「· 已运行 X」时长徽标：据 `startedAt` 起算，复用 watch 卡算法（`fmt_duration`），始终显示
/// （含 <60 秒的「X 秒」，最利于区分）。无 `startedAt` → None（不显示）。仅工作中 agent 用（调用方门控）。
fn elapsed_badge(rec: &Value, now: u64, lang: Lang) -> Option<String> {
    let start = rec.get("startedAt").and_then(|v| v.as_u64())?;
    let secs = now.saturating_sub(start);
    Some(format!(
        "· {}",
        i18n::tr(lang, "watch.statsElapsed")
            .replace("{t}", &crate::watch::fmt_duration(secs, lang))
    ))
}

/// 主文本 = `类型 · 工作目录名`（工作目录名取 cwd 末段，缺省仅类型）。
fn primary_text(rec: &Value, lang: Lang) -> String {
    let kind = rec.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let kind_label = crate::agents::AgentKind::parse(kind)
        .map(|k| k.label().to_string())
        .unwrap_or_else(|| kind.to_string());
    match rec
        .get("cwd")
        .and_then(|v| v.as_str())
        .and_then(crate::autochannel::project_name)
    {
        Some(dir) => format!("{} · {}", kind_label, dir),
        None => {
            if kind_label.is_empty() {
                i18n::tr(lang, "autoChannel.noProject").to_string()
            } else {
                kind_label
            }
        }
    }
}

/// 次行标题（缺省 → noTitle）。
fn title_text(rec: &Value, lang: Lang) -> String {
    rec.get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| i18n::tr(lang, "autoChannel.noTitle").to_string())
}

/// 由注册表快照（`AgentRegistry::snapshot()` 的 Value 数组）组装 agent 选项：仅取「工作中 / 空闲」
/// （已结束不列），工作中在前。`watching` = 本渠道已在关注的 session_id 集合（命中加「· 关注中」徽标）。
pub fn agent_options(
    snapshot: &Value,
    watching: &HashSet<String>,
    now: u64,
    lang: Lang,
) -> Vec<SelectOption> {
    let empty = Vec::new();
    let list = snapshot.as_array().unwrap_or(&empty);
    let mut working: Vec<SelectOption> = Vec::new();
    let mut idle: Vec<SelectOption> = Vec::new();
    for rec in list {
        let bucket = match rec.get("state").and_then(|v| v.as_str()) {
            Some("working") => &mut working,
            Some("idle") => &mut idle,
            _ => continue, // ended / 未知：不列
        };
        let sid = rec
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if sid.is_empty() {
            continue;
        }
        bucket.push(option_from_record(rec, sid, watching, now, lang));
    }
    working.extend(idle);
    working
}

/// 由注册表快照组装 `/watch` 单选卡选项：**仅列「工作中」** 的 agent（含 grok，区别于
/// `msg_options` 排除 grok）。空闲 agent 关注没有实际意义，故不列出。
pub fn watch_options(
    snapshot: &Value,
    watching: &HashSet<String>,
    now: u64,
    lang: Lang,
) -> Vec<SelectOption> {
    let empty = Vec::new();
    let list = snapshot.as_array().unwrap_or(&empty);
    let mut out: Vec<SelectOption> = Vec::new();
    for rec in list {
        if rec.get("state").and_then(|v| v.as_str()) != Some("working") {
            continue;
        }
        let sid = rec
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if sid.is_empty() {
            continue;
        }
        out.push(option_from_record(rec, sid, watching, now, lang));
    }
    out
}

/// 由注册表快照组装「可发送插话」的候选选项（`/msg` 无编号单选卡）：**仅列「工作中」且非 grok**
/// 的 agent（插话只对工作中有意义、grok 无可靠传话通道）。`watching` 命中仍加「· 关注中」徽标。
pub fn msg_options(
    snapshot: &Value,
    watching: &HashSet<String>,
    now: u64,
    lang: Lang,
) -> Vec<SelectOption> {
    let empty = Vec::new();
    let list = snapshot.as_array().unwrap_or(&empty);
    let mut out: Vec<SelectOption> = Vec::new();
    for rec in list {
        if rec.get("state").and_then(|v| v.as_str()) != Some("working") {
            continue; // 仅工作中。
        }
        if rec.get("kind").and_then(|v| v.as_str()) == Some("grok") {
            continue; // grok 无法插话。
        }
        let sid = rec
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if sid.is_empty() {
            continue;
        }
        out.push(option_from_record(rec, sid, watching, now, lang));
    }
    out
}

/// 组装单个 agent 选项（`/unwatch` 单选卡按订阅列举时用）：按 session_id 在快照定位记录；
/// 记录已消失（agent 结束/被清理）时以 `seq` 兜底、圆点/主文本降级。
pub fn agent_option_by_session(
    snapshot: &Value,
    session_id: &str,
    seq: u64,
    now: u64,
    lang: Lang,
) -> SelectOption {
    if let Some(rec) = snapshot.as_array().and_then(|l| {
        l.iter()
            .find(|r| r.get("sessionId").and_then(|v| v.as_str()) == Some(session_id))
    }) {
        let mut opt = option_from_record(rec, session_id.to_string(), &HashSet::new(), now, lang);
        // 订阅侧的稳定展示编号优先（快照 seq 与订阅 seq 一致，缺省时兜底）。
        opt.seq = opt.seq.or(Some(seq));
        opt
    } else {
        SelectOption {
            id: session_id.to_string(),
            dot: None,
            seq: Some(seq),
            primary: i18n::tr(lang, "autoChannel.noProject").to_string(),
            badge: None,
            elapsed: None,
            secondary: Some(i18n::tr(lang, "autoChannel.noTitle").to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 固定「当前时刻」，配合 `startedAt` 断言运行时长。
    const NOW: u64 = 1_000_000;

    fn snap() -> Value {
        json!([
            {"seq":1,"kind":"cursor","sessionId":"s-idle","state":"idle","title":"闲着","cwd":"/tmp/my-proj","startedAt": NOW - 600},
            {"seq":2,"kind":"claude","sessionId":"s-work","state":"working","title":"忙着","cwd":"/tmp/api-server","startedAt": NOW - 360},
            {"seq":3,"kind":"codex","sessionId":"s-end","state":"ended","title":"完了","cwd":"/tmp/proj","startedAt": NOW - 100},
        ])
    }

    #[test]
    fn agent_options_working_first_and_skips_ended() {
        let opts = agent_options(&snap(), &HashSet::new(), NOW, Lang::Zh);
        // 只列工作中 + 空闲；工作中在前。
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].id, "s-work");
        assert_eq!(opts[0].dot, Some(SelectDot::Working));
        assert_eq!(opts[0].seq, Some(2));
        // 主文本 = 类型 · 工作目录名。
        assert_eq!(opts[0].primary, "Claude Code · api-server");
        assert_eq!(opts[0].secondary.as_deref(), Some("忙着"));
        // 运行时长（6 分钟 = NOW-360）。
        assert_eq!(opts[0].elapsed.as_deref(), Some("· 已运行 6 分钟"));
        assert_eq!(opts[1].id, "s-idle");
        assert_eq!(opts[1].dot, Some(SelectDot::Idle));
        assert_eq!(opts[1].primary, "Cursor · my-proj");
        // 空闲态不显示运行时长（用户定案：易误导）。
        assert_eq!(opts[1].elapsed, None);
        // 无徽标。
        assert!(opts[0].badge.is_none());
    }

    #[test]
    fn elapsed_shows_seconds_under_a_minute_and_none_without_start() {
        let snap = json!([
            {"seq":1,"kind":"claude","sessionId":"s1","state":"working","title":"t","cwd":"/tmp/a","startedAt": NOW - 30},
            {"seq":2,"kind":"cursor","sessionId":"s2","state":"working","title":"t","cwd":"/tmp/b"},
        ]);
        let opts = agent_options(&snap, &HashSet::new(), NOW, Lang::Zh);
        // <60 秒仍显示「X 秒」（用户定案：始终显示，最利于区分）。
        assert_eq!(opts[0].elapsed.as_deref(), Some("· 已运行 30 秒"));
        // 无 startedAt → 不显示时长。
        assert_eq!(opts[1].elapsed, None);
    }

    #[test]
    fn watching_badge_applied() {
        let mut watching = HashSet::new();
        watching.insert("s-work".to_string());
        let opts = agent_options(&snap(), &watching, NOW, Lang::Zh);
        assert_eq!(opts[0].id, "s-work");
        assert_eq!(opts[0].badge.as_deref(), Some("· 关注中"));
        assert!(opts[1].badge.is_none());
    }

    #[test]
    fn agent_option_by_session_falls_back_when_missing() {
        let opt = agent_option_by_session(&snap(), "s-gone", 7, NOW, Lang::Zh);
        assert_eq!(opt.id, "s-gone");
        assert_eq!(opt.seq, Some(7));
        assert!(opt.dot.is_none());
        assert!(opt.elapsed.is_none());
        // 命中时用快照字段。
        let opt2 = agent_option_by_session(&snap(), "s-work", 2, NOW, Lang::Zh);
        assert_eq!(opt2.dot, Some(SelectDot::Working));
        assert_eq!(opt2.primary, "Claude Code · api-server");
        assert_eq!(opt2.elapsed.as_deref(), Some("· 已运行 6 分钟"));
    }

    #[test]
    fn build_view_truncates_over_limit() {
        let mk = |i: usize| SelectOption {
            id: i.to_string(),
            dot: None,
            seq: Some(i as u64),
            primary: format!("opt {i}"),
            badge: None,
            elapsed: None,
            secondary: None,
        };
        let many: Vec<SelectOption> = (0..(SELECT_MAX_OPTIONS + 5)).map(mk).collect();
        let v = build_view("T".into(), many, SelectAction::Watch, Lang::Zh);
        assert_eq!(v.options.len(), SELECT_MAX_OPTIONS);
        assert!(v.truncated_note.is_some());
        // 未超上限不截断。
        let v2 = build_view("T".into(), vec![mk(0)], SelectAction::Status, Lang::Zh);
        assert_eq!(v2.options.len(), 1);
        assert!(v2.truncated_note.is_none());
    }

    #[test]
    fn action_button_labels() {
        assert_eq!(SelectAction::Watch.button_label(Lang::Zh), "关注");
        assert_eq!(SelectAction::Status.button_label(Lang::Zh), "查看");
        assert_eq!(SelectAction::Unwatch.button_label(Lang::Zh), "取消");
        assert_eq!(SelectAction::Msg.button_label(Lang::Zh), "发送");
    }

    #[test]
    fn watch_options_only_working_including_grok() {
        let mut snap = snap();
        snap.as_array_mut().unwrap().push(json!({
            "seq":4,"kind":"grok","sessionId":"s-grok","state":"working","title":"g","cwd":"/tmp/g","startedAt": NOW - 120
        }));
        let opts = watch_options(&snap, &HashSet::new(), NOW, Lang::Zh);
        // working cursor (s-work) 不在 snap() 默认里——snap() 里 claude s-work 是 working。
        // snap() = idle cursor + working claude + ended codex + 追加 working grok。
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].id, "s-work"); // claude，working
        assert_eq!(opts[1].id, "s-grok"); // grok，working（不排除）
                                          // idle 的 s-idle 不在列表中。
        assert!(!opts.iter().any(|o| o.id == "s-idle"));
        // ended 的 s-end 不在列表中。
        assert!(!opts.iter().any(|o| o.id == "s-end"));
        // 关注徽标生效。
        let mut watching = HashSet::new();
        watching.insert("s-grok".to_string());
        let opts2 = watch_options(&snap, &watching, NOW, Lang::Zh);
        assert_eq!(opts2[1].badge.as_deref(), Some("· 关注中"));
    }

    #[test]
    fn msg_options_only_working_non_grok() {
        // 快照含：working cursor / idle claude / ended codex（见 snap）+ 追加一个 working grok。
        let mut snap = snap();
        snap.as_array_mut().unwrap().push(json!({
            "seq":4,"kind":"grok","sessionId":"s-grok","state":"working","title":"g","cwd":"/tmp/g"
        }));
        let opts = msg_options(&snap, &HashSet::new(), NOW, Lang::Zh);
        // 仅剩工作中·非 grok 的那一个（claude s-work）。
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].id, "s-work");
        // 运行时长随选项显示。
        assert_eq!(opts[0].elapsed.as_deref(), Some("· 已运行 6 分钟"));
        // 关注徽标仍生效。
        let mut watching = HashSet::new();
        watching.insert("s-work".to_string());
        let opts2 = msg_options(&snap, &watching, NOW, Lang::Zh);
        assert_eq!(opts2[0].badge.as_deref(), Some("· 关注中"));
    }
}
