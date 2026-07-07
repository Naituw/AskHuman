//! Agent 注册表：daemon 内存维护被追踪 agent 的状态，并持久化到 `~/.askhuman/agents.json`。
//!
//! 身份模型（spec D7）：**以 `session_id` 为身份**；`pid` 仅用于存活轮询。同一 pid 出现新
//! `session_id` ⇒ 旧 session 判「已结束」、新 session 复用该 pid（一个 pid 同时至多一个活动 session）。
//!
//! 状态推导（spec D5/D8/D12）：turn-start→工作中、turn-end→空闲；进程存活轮询是权威「已结束」
//! 判据；仅当 **拿不到 pid** 时用 1 小时 TTL 兜底（任意事件 / ask 调用都刷新活动时间）。
//!
//! **无 pid 路径（spec D25/D26）**：Codex 新版经共享 app-server 跑 agent，`detect` 把这类会话的
//! pid 归一成 `None`（walk 只会命中长寿共享守护、拿不到 TUI pid）；此时本注册表**不做 D7 轮换、
//! 不做存活轮询**，纯由 D12 TTL + `working_backstop_sweep` 治理——与 Claude 被 PID-scrub 时**同一
//! 条路径**。打断 / 关窗（Codex 无对应 hook，同 Claude）靠 `working_backstop_sweep` 超时兜底降级。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::detect::pid_alive;
use super::title::resolve_title;
use super::{AgentKind, LifecycleEvent};
use crate::paths;

/// 「已结束」记录的全局保留上限（spec D11）。
const MAX_ENDED: usize = 10;
/// TTL 兜底时长（spec D12）：仅对**无 pid**的活动记录生效。
pub const TTL_SECS: u64 = 3600;

/// 「工作中」兜底超时：某 agent 进程仍在、但距上次活动超过此时长且没有在途 AskHuman 请求，
/// 即把它从「工作中」降级为「空闲」。用于兜底 Claude「用户打断回合」这类**没有任何 hook**
/// 的场景（打断后会一直卡在「工作中」，直到下个回合/进程退出）。设得足够大（30 分钟），
/// 这样正常的长回合（编译/测试/长回复）几乎不会被误判——它只在「真卡住」时才触发。
/// 等待人类回答 AskHuman 期间由 daemon 按在途请求持续刷新活动，故等多久都不会被它降级。
pub const WORKING_BACKSTOP_SECS: u64 = 30 * 60;

/// agent 三态（spec D8，展示用词 工作中 / 空闲 / 已结束）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Working,
    Idle,
    Ended,
}

/// 单个被追踪 agent（一条 session）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecord {
    /// 稳定数字编号：**当前 daemon 生命周期内**单调递增、不复用、从 1 起，供 IM `/status <编号>` 寻址。
    /// 不跨 daemon 重启保留（`load()` 会对还原记录按序重排），故盘上旧值忽略（`serde(default)`）。
    #[serde(default)]
    pub seq: u64,
    pub kind: AgentKind,
    pub session_id: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    pub started_at: u64,
    pub last_activity: u64,
    pub state: AgentState,
    #[serde(default)]
    pub ended_at: Option<u64>,
    /// 所在终端类型（`apple-terminal`/`iterm2`/`vscode`/…/`other`）。由 pid 沿进程链惰性识别并缓存，
    /// 供状态窗口「聚焦终端」按钮按支持度显隐。无 pid / 未解析时为 None。
    #[serde(default)]
    pub terminal: Option<String>,
    /// 实时「当前工具」（PreToolUse 上报置位、PostToolUse/回合结束/会话结束清除）。**不落盘**
    /// （`serde(skip)`：既不入 `agents.json` 也不入默认 snapshot）；由 `snapshot()` 手动注入 `currentTool`，
    /// daemon 重启自然消失。供 `/status <编号>` 无滞后地反映「此刻在跑什么」。
    #[serde(skip)]
    pub current_tool: Option<CurrentTool>,
    /// 本回合累计工具步数（PreToolUse +1；turn-start 清零、turn-end 归零）。**不落盘**，
    /// `snapshot()` 注入 `turnSteps`，供 `/watch` 卡状态行「第 N 步」（依赖生命周期 hook，无则恒 0）。
    #[serde(skip)]
    pub turn_steps: u32,
    /// 本回合开始时刻（Unix 秒；turn-start 置位、首个工具心跳兜底补位、turn-end 清除）。**不落盘**，
    /// `snapshot()` 注入 `turnStartedAt`（`/watch` 卡时长已改用 `startedAt` 整体运行时长——
    /// 用户定案：回合时长迷惑；本字段暂无展示消费方，保留数据）。
    #[serde(skip)]
    pub turn_started_at: Option<u64>,
}

/// 实时「当前工具」快照：跨进程只存原始工具名 + 已归一化的短对象 + 上报时间（秒）。
#[derive(Debug, Clone)]
pub struct CurrentTool {
    pub name: String,
    pub object: Option<String>,
    pub at: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    active: Vec<AgentRecord>,
    #[serde(default)]
    ended: Vec<AgentRecord>,
}

#[derive(Default)]
struct Inner {
    /// 活动记录（工作中 / 空闲），按 session_id 索引（保持插入顺序用 Vec 即可，量小）。
    active: Vec<AgentRecord>,
    /// 最近结束（最多 MAX_ENDED，队尾最新）。
    ended: VecDeque<AgentRecord>,
    /// 下一个待分配的稳定编号（`seq`）。单调递增、不复用；`Default` 起始 0，`alloc_seq` 兜底从 1 起。
    next_seq: u64,
}

impl Inner {
    /// 分配一个稳定编号（从 1 起、单调、不复用）。
    fn alloc_seq(&mut self) -> u64 {
        if self.next_seq < 1 {
            self.next_seq = 1;
        }
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }
}

/// daemon 内唯一的 agent 注册表（线程安全）。
pub struct AgentRegistry {
    inner: Mutex<Inner>,
    /// PID 缓存：(session_id, hint_pid) → 已解析的 agent PID。
    /// hook 发 ppid 来，daemon 从 ppid 向上 walk 找到 agent PID 后缓存，避免重复 walk。
    pid_cache: Mutex<std::collections::HashMap<(String, u32), Option<u32>>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            pid_cache: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 从 `~/.askhuman/agents.json` 还原，并对每条活动记录用 kill-0 复核存活、剔除已死（spec D18）。
    pub fn load() -> Self {
        let reg = Self::new();
        let Ok(text) = std::fs::read_to_string(paths::agents_file()) else {
            return reg;
        };
        let Ok(parsed) = serde_json::from_str::<Persisted>(&text) else {
            return reg;
        };
        let now = now_secs();
        let mut inner = reg.inner.lock().unwrap();
        for mut rec in parsed.active {
            // 复核存活：有 pid 且已死 → 结束；无 pid 留给 TTL。
            if let Some(pid) = rec.pid {
                if !pid_alive(pid) {
                    rec.state = AgentState::Ended;
                    rec.ended_at = Some(now);
                    push_ended(&mut inner.ended, rec);
                    continue;
                }
            }
            inner.active.push(rec);
        }
        for rec in parsed.ended {
            push_ended(&mut inner.ended, rec);
        }
        // 盘上旧 seq 一律忽略：按序（活动在前、已结束在后）重排，保证「当前 daemon 生命周期内」稳定、从 1 起。
        let mut seq = 1u64;
        for r in inner.active.iter_mut() {
            r.seq = seq;
            seq += 1;
        }
        for r in inner.ended.iter_mut() {
            r.seq = seq;
            seq += 1;
        }
        inner.next_seq = seq;
        drop(inner);
        reg
    }

    /// 从 hint_pid（hook 的 ppid）解析 agent PID，按 (session_id, hint_pid) 缓存。
    /// Agent 可 resume 同一 session（session_id 不变但进程不同），hint_pid 变化触发重解析。
    pub fn resolve_pid(
        &self,
        session_id: &str,
        kind: AgentKind,
        hint_pid: Option<u32>,
    ) -> Option<u32> {
        let hint = hint_pid?;
        let key = (session_id.to_string(), hint);
        {
            let cache = self.pid_cache.lock().unwrap();
            if let Some(&cached) = cache.get(&key) {
                return cached;
            }
        }
        let resolved = super::detect::walk_agent_pid(kind, hint);
        let mut cache = self.pid_cache.lock().unwrap();
        cache.insert(key, resolved);
        resolved
    }

    /// 清除某 session 的所有 PID 缓存条目。
    pub fn clear_pid_cache(&self, session_id: &str) {
        let mut cache = self.pid_cache.lock().unwrap();
        cache.retain(|(sid, _), _| sid != session_id);
    }

    /// 处理一次生命周期事件（spec D5/D6/D7）。返回是否有状态变化（供广播）。
    pub fn apply_event(
        &self,
        kind: AgentKind,
        event: LifecycleEvent,
        session_id: &str,
        pid: Option<u32>,
        cwd: Option<String>,
        ts: u64,
    ) -> bool {
        if session_id.is_empty() {
            return false;
        }
        let now = if ts == 0 { now_secs() } else { ts };
        let mut inner = self.inner.lock().unwrap();

        // 轮换（spec D7）：同一 pid 上若已有「另一个 session」活动 → 旧的判结束。
        let mut changed = false;
        if let Some(pid) = pid {
            let rotated: Vec<AgentRecord> = drain_where(&mut inner.active, |r| {
                r.pid == Some(pid) && r.session_id != session_id
            });
            for mut r in rotated {
                r.state = AgentState::Ended;
                r.ended_at = Some(now);
                push_ended(&mut inner.ended, r);
                changed = true;
            }
        }

        // 幂等登记 + 更新（任何事件都能建，不依赖 session-start）。
        let idx = inner.active.iter().position(|r| r.session_id == session_id);
        let (idx, created) = match idx {
            Some(i) => {
                let r = &mut inner.active[i];
                if pid.is_some() {
                    r.pid = pid;
                }
                if cwd.is_some() {
                    r.cwd = cwd;
                }
                r.last_activity = now;
                (i, false)
            }
            None => {
                let seq = inner.alloc_seq();
                inner.active.push(AgentRecord {
                    seq,
                    kind,
                    session_id: session_id.to_string(),
                    pid,
                    title: None,
                    cwd,
                    started_at: now,
                    last_activity: now,
                    state: AgentState::Idle,
                    ended_at: None,
                    terminal: None,
                    current_tool: None,
                    turn_steps: 0,
                    turn_started_at: None,
                });
                (inner.active.len() - 1, true)
            }
        };
        changed |= created;

        // 事件 → 状态。`changed` 决定是否持久化 + 广播：**Activity（工具心跳）在状态不变时返回
        // false**，避免长回合里每次工具调用都落盘/广播（last_activity 仍已在内存刷新，喂兜底超时；
        // 相对时间由前端 ticker + 15s 轮询广播兜底）。
        let prev = inner.active[idx].state;
        match event {
            LifecycleEvent::SessionStart => { /* 已确保登记，保持 Idle */ }
            LifecycleEvent::TurnStart => {
                inner.active[idx].state = AgentState::Working;
                // 新回合：步数清零、记回合开始时刻（供 /watch「第 N 步 · 已 N 分钟」）。
                inner.active[idx].turn_steps = 0;
                inner.active[idx].turn_started_at = Some(now);
                changed |= prev != AgentState::Working;
            }
            LifecycleEvent::Activity => {
                inner.active[idx].state = AgentState::Working;
                // turn-start 缺失（hook 竞态 / 半装）时以首个心跳兜底记回合开始。
                if inner.active[idx].turn_started_at.is_none() {
                    inner.active[idx].turn_started_at = Some(now);
                }
                changed |= prev != AgentState::Working;
            }
            LifecycleEvent::TurnEnd => {
                inner.active[idx].state = AgentState::Idle;
                inner.active[idx].current_tool = None; // 回合结束不应残留在跑工具
                inner.active[idx].turn_steps = 0;
                inner.active[idx].turn_started_at = None;
                changed |= prev != AgentState::Idle;
            }
            LifecycleEvent::SessionEnd => {
                let mut r = inner.active.remove(idx);
                r.state = AgentState::Ended;
                r.ended_at = Some(now);
                r.current_tool = None; // 会话结束清除
                r.turn_steps = 0;
                r.turn_started_at = None;
                push_ended(&mut inner.ended, r);
                changed = true;
            }
        }
        changed
    }

    /// ask 调用顺带刷新活动 + 重置 TTL（spec D21）：仅刷新已存在的同家族 session，不新建。
    pub fn touch_activity(&self, kind: AgentKind, session_id: &str, pid: Option<u32>) -> bool {
        if session_id.is_empty() {
            return false;
        }
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        if let Some(r) = inner
            .active
            .iter_mut()
            .find(|r| r.session_id == session_id && r.kind == kind)
        {
            r.last_activity = now;
            if r.pid.is_none() && pid.is_some() {
                r.pid = pid;
            }
            true
        } else {
            false
        }
    }

    /// 拿不到 `session_id` 时按 **pid** 刷新活动（典型为 **MCP 模式**：agent 把 MCP server 的 env
    /// 清空，子进程只能靠进程树 walk 拿到 `(kind, pid)`，取不到会话 ID）。在活动记录里按 `(kind, pid)`
    /// 匹配**已存在**的 session 并刷新 `last_activity`；**只更新、绝不新建**——pid 是当次现取、真实存活的，
    /// 天然规避长寿 MCP server 旧 `session_id` 造成的「幽灵会话」。返回是否命中（供广播刷新相对时间）。
    pub fn touch_activity_by_pid(&self, kind: AgentKind, pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        let mut hit = false;
        for r in inner
            .active
            .iter_mut()
            .filter(|r| r.kind == kind && r.pid == Some(pid))
        {
            r.last_activity = now;
            hit = true;
        }
        hit
    }

    /// PreToolUse 实时上报：置「当前工具」。命中已存在的同家族 session（daemon 已先 `apply_event` 登记）；
    /// 顺带置工作中 + 刷新活动 + 补 pid。**不落盘、不广播**（current_tool 仅内存 + snapshot，IM `/status`
    /// 拉取时现取；避免每次工具调用都持久化/广播）。找不到记录则 no-op。
    pub fn set_current_tool(
        &self,
        kind: AgentKind,
        session_id: &str,
        pid: Option<u32>,
        name: String,
        object: Option<String>,
    ) {
        if session_id.is_empty() {
            return;
        }
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        if let Some(r) = inner
            .active
            .iter_mut()
            .find(|r| r.session_id == session_id && r.kind == kind)
        {
            r.state = AgentState::Working;
            r.last_activity = now;
            if r.pid.is_none() && pid.is_some() {
                r.pid = pid;
            }
            r.current_tool = Some(CurrentTool { name, object, at: now });
            // 每次 PreToolUse 记一步（回合内单调递增，turn-start/turn-end 清零）。
            r.turn_steps = r.turn_steps.saturating_add(1);
            if r.turn_started_at.is_none() {
                r.turn_started_at = Some(now);
            }
        }
    }

    /// PostToolUse 实时上报：清除「当前工具」（工具已跑完）。找不到记录则 no-op。
    pub fn clear_current_tool(&self, kind: AgentKind, session_id: &str) {
        if session_id.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if let Some(r) = inner
            .active
            .iter_mut()
            .find(|r| r.session_id == session_id && r.kind == kind)
        {
            r.current_tool = None;
        }
    }

    /// 「IM 会话期自动激活」无 hook 兜底：提问时把对应 session 标记为「工作中」。
    /// 不存在则新建；已存在则置为 Working 并刷新活动 / 补 pid（在途提问必然处于「工作中」turn 内）。
    /// 返回是否有状态变化（供广播）。
    pub fn upsert_working(
        &self,
        kind: AgentKind,
        session_id: &str,
        pid: Option<u32>,
        cwd: Option<String>,
    ) -> bool {
        if session_id.is_empty() {
            return false;
        }
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        if let Some(r) = inner
            .active
            .iter_mut()
            .find(|r| r.session_id == session_id && r.kind == kind)
        {
            let was_working = r.state == AgentState::Working;
            r.state = AgentState::Working;
            r.last_activity = now;
            if r.pid.is_none() && pid.is_some() {
                r.pid = pid;
            }
            if r.cwd.is_none() && cwd.is_some() {
                r.cwd = cwd;
            }
            !was_working
        } else {
            let seq = inner.alloc_seq();
            inner.active.push(AgentRecord {
                seq,
                kind,
                session_id: session_id.to_string(),
                pid,
                title: None,
                cwd,
                started_at: now,
                last_activity: now,
                state: AgentState::Working,
                ended_at: None,
                terminal: None,
                current_tool: None,
                turn_steps: 0,
                turn_started_at: None,
            });
            true
        }
    }

    /// 进程存活轮询（spec D5）：有 pid 且已死 → 结束。返回是否有变化。
    pub fn poll_liveness(&self) -> bool {
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        let dead: Vec<AgentRecord> = drain_where(&mut inner.active, |r| {
            r.pid.map(|p| !pid_alive(p)).unwrap_or(false)
        });
        let changed = !dead.is_empty();
        for mut r in dead {
            r.state = AgentState::Ended;
            r.ended_at = Some(now);
            push_ended(&mut inner.ended, r);
        }
        changed
    }

    /// TTL 兜底（spec D12）：仅对**无 pid**的活动记录，超 TTL 无活动 → 结束。返回是否有变化。
    pub fn ttl_sweep(&self) -> bool {
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        let expired: Vec<AgentRecord> = drain_where(&mut inner.active, |r| {
            r.pid.is_none() && now.saturating_sub(r.last_activity) > TTL_SECS
        });
        let changed = !expired.is_empty();
        for mut r in expired {
            r.state = AgentState::Ended;
            r.ended_at = Some(now);
            push_ended(&mut inner.ended, r);
        }
        changed
    }

    /// 在途 AskHuman 请求豁免：给「pid 命中在途请求集合」的活动记录刷新活动时间。
    /// daemon 每个轮询 tick 先调它（用所有在途请求的 agent pid），这样「等待人类回答 AskHuman」
    /// 期间对应 agent 的活动时间一直新鲜，`working_backstop_sweep` 永远不会把它降级为空闲。
    /// 返回是否命中（仅用于调试，不触发广播——刷新活动不改变状态）。
    pub fn refresh_by_pids(&self, pids: &[u32]) -> bool {
        if pids.is_empty() {
            return false;
        }
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        let mut hit = false;
        for r in inner.active.iter_mut() {
            if let Some(pid) = r.pid {
                if pids.contains(&pid) {
                    r.last_activity = now;
                    hit = true;
                }
            }
        }
        hit
    }

    /// 在途 AskHuman 豁免（**session_id 版**，与 `refresh_by_pids` 并列）：给「正等待人类回答」的
    /// **无 pid** agent（典型 Codex 共享 app-server / Claude 被 PID-scrub）按 `session_id` 刷新活动，
    /// 使其在等待期间不被 `working_backstop_sweep` 降级为空闲（有 pid 的由 `refresh_by_pids` 覆盖）。
    /// `session_id` 全局唯一，故不分家族匹配。返回是否命中（仅调试用，不触发广播——刷新不改状态）。
    pub fn refresh_by_session_ids(&self, session_ids: &[String]) -> bool {
        if session_ids.is_empty() {
            return false;
        }
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        let mut hit = false;
        for r in inner.active.iter_mut() {
            if session_ids.iter().any(|s| s == &r.session_id) {
                r.last_activity = now;
                hit = true;
            }
        }
        hit
    }

    /// 「工作中」兜底超时扫描：把「工作中」且距上次活动超过 `timeout_secs` 的记录降级为「空闲」。
    /// 兜底 Claude「用户打断回合」这类无 hook 场景（见 `WORKING_BACKSTOP_SECS`）。调用前应先用
    /// `refresh_by_pids` 豁免在途 AskHuman 的 agent。返回是否有变化（供广播）。
    pub fn working_backstop_sweep(&self, timeout_secs: u64) -> bool {
        let now = now_secs();
        let mut inner = self.inner.lock().unwrap();
        let mut changed = false;
        for r in inner.active.iter_mut() {
            if r.state == AgentState::Working
                && now.saturating_sub(r.last_activity) > timeout_secs
            {
                r.state = AgentState::Idle;
                changed = true;
            }
        }
        changed
    }

    /// 手动把指定 session 的「工作中」记录置为「空闲」（状态窗口用户纠正漏 hook 卡死场景）。
    /// 仅当该记录存在且为「工作中」时生效；置空闲后刷新活动时间（避免下个 tick 立即被兜底重扫，
    /// 行为上即「已纠正」）。不动 pid/会话、不结束。返回是否有变化（供广播）。
    pub fn force_idle(&self, session_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if let Some(r) = inner
            .active
            .iter_mut()
            .find(|r| r.session_id == session_id && r.state == AgentState::Working)
        {
            r.state = AgentState::Idle;
            r.last_activity = now_secs();
            return true;
        }
        false
    }

    /// 工作中 agent 数（spec D18：仅它与窗口连接阻止 daemon 空闲退出；空闲 agent 不算）。
    pub fn working_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .active
            .iter()
            .filter(|r| r.state == AgentState::Working)
            .count()
    }

    /// 空闲 agent 数（菜单栏状态展示用；不参与空闲退出判定）。
    pub fn idle_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .active
            .iter()
            .filter(|r| r.state == AgentState::Idle)
            .count()
    }

    /// 活动（工作中 / 空闲）会话的 session_id 集合（插话队列兜底清理用：不在此集合的会话
    /// 视为已结束，其待送达条目应清空）。
    pub fn active_session_ids(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.active.iter().map(|r| r.session_id.clone()).collect()
    }

    /// 托盘「Agent 状态」子菜单摘要（spec agent-interject D7）：仅活动会话，**工作中在前**、
    /// 组内按最近活动倒序；ended 不含。`pending_interject` 恒 false，由 daemon 按插话队列注入。
    pub fn tray_agent_infos(&self) -> Vec<crate::ipc::TrayAgentInfo> {
        let mut inner = self.inner.lock().unwrap();
        // 标题 / 终端类型惰性补齐（与 snapshot 同口径）：标题用于条目文案、终端决定「聚焦终端」显隐。
        for r in inner.active.iter_mut() {
            if r.title.is_none() {
                r.title = resolve_title(r.kind, &r.session_id);
            }
            if r.terminal.is_none() {
                if let Some(pid) = r.pid {
                    r.terminal =
                        Some(super::detect::terminal_kind(pid).unwrap_or("other").to_string());
                }
            }
        }
        let mut list: Vec<&AgentRecord> = inner.active.iter().collect();
        list.sort_by(|a, b| {
            let rank = |r: &AgentRecord| match r.state {
                AgentState::Working => 0u8,
                _ => 1u8,
            };
            rank(a)
                .cmp(&rank(b))
                .then(b.last_activity.cmp(&a.last_activity))
        });
        list.into_iter()
            .map(|r| crate::ipc::TrayAgentInfo {
                session_id: r.session_id.clone(),
                seq: r.seq,
                kind: r.kind.as_str().to_string(),
                title: r.title.clone().unwrap_or_default(),
                project_name: r
                    .cwd
                    .as_deref()
                    .map(crate::project::display_name)
                    .unwrap_or_default(),
                cwd: r.cwd.clone(),
                state: match r.state {
                    AgentState::Working => "working",
                    _ => "idle",
                }
                .to_string(),
                pending_interject: false,
                // 与前端 `lib/terminals.ts` 的支持清单一致（Terminal.app / iTerm2）。
                focusable: r.pid.is_some()
                    && matches!(r.terminal.as_deref(), Some("apple-terminal") | Some("iterm2")),
                pid: r.pid,
            })
            .collect()
    }

    /// 构造全量快照（解析缺失标题并缓存）。返回 agents 列表 Value（前端按类型分组、按状态排序）。
    pub fn snapshot(&self) -> Value {
        let mut inner = self.inner.lock().unwrap();
        // 惰性补齐标题（已解析的不再重复）；ended 记录的会话文件依然在盘上，同样补齐。
        for r in inner.active.iter_mut() {
            if r.title.is_none() {
                r.title = resolve_title(r.kind, &r.session_id);
            }
            // 惰性识别终端类型（活动记录、有 pid 时）；找不到记 "other"，避免每次快照重算。
            if r.terminal.is_none() {
                if let Some(pid) = r.pid {
                    r.terminal =
                        Some(super::detect::terminal_kind(pid).unwrap_or("other").to_string());
                }
            }
        }
        for r in inner.ended.iter_mut() {
            if r.title.is_none() {
                r.title = resolve_title(r.kind, &r.session_id);
            }
        }
        let mut list: Vec<AgentRecord> = inner.active.clone();
        for r in inner.ended.iter() {
            list.push(r.clone());
        }
        // current_tool / turn_steps / turn_started_at 标了 serde(skip)，默认序列化不含 →
        // 手动注入 `currentTool` / `turnSteps` / `turnStartedAt`（仅进 snapshot，不落盘）。
        let arr: Vec<Value> = list
            .iter()
            .map(|r| {
                let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
                if let Some(obj) = v.as_object_mut() {
                    if let Some(ct) = &r.current_tool {
                        obj.insert(
                            "currentTool".to_string(),
                            serde_json::json!({ "name": ct.name, "object": ct.object, "at": ct.at }),
                        );
                    }
                    if r.turn_steps > 0 {
                        obj.insert("turnSteps".to_string(), serde_json::json!(r.turn_steps));
                    }
                    if let Some(ts) = r.turn_started_at {
                        obj.insert("turnStartedAt".to_string(), serde_json::json!(ts));
                    }
                }
                v
            })
            .collect();
        Value::Array(arr)
    }

    /// 持久化到 `~/.askhuman/agents.json`（原子写）。best-effort，失败静默。
    pub fn persist(&self) {
        let inner = self.inner.lock().unwrap();
        let data = Persisted {
            active: inner.active.clone(),
            ended: inner.ended.iter().cloned().collect(),
        };
        drop(inner);
        let Ok(json) = serde_json::to_string_pretty(&data) else {
            return;
        };
        let path = paths::agents_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
        if std::fs::write(&tmp, json.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 把记录压入「已结束」队列，超出 MAX_ENDED 则从队首淘汰。
fn push_ended(ended: &mut VecDeque<AgentRecord>, rec: AgentRecord) {
    ended.push_back(rec);
    while ended.len() > MAX_ENDED {
        ended.pop_front();
    }
}

/// 从 Vec 中取出满足谓词的元素（保留其余），返回被取出的。
fn drain_where<T>(v: &mut Vec<T>, pred: impl Fn(&T) -> bool) -> Vec<T> {
    let mut taken = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if pred(&v[i]) {
            taken.push(v.remove(i));
        } else {
            i += 1;
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> AgentRegistry {
        AgentRegistry::new()
    }

    #[test]
    fn turn_events_toggle_working_idle() {
        let r = reg();
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::SessionStart,
            "s1",
            Some(111),
            None,
            100,
        );
        assert_eq!(r.working_count(), 0);
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "s1",
            Some(111),
            None,
            110,
        );
        assert_eq!(r.working_count(), 1);
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnEnd,
            "s1",
            Some(111),
            None,
            120,
        );
        assert_eq!(r.working_count(), 0);
    }

    #[test]
    fn activity_keeps_working_and_refreshes() {
        // Pre/PostToolUse → Activity：置工作中 + 刷新活动，不结束回合。
        let r = reg();
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "s1",
            Some(111),
            None,
            100,
        );
        assert_eq!(r.working_count(), 1);
        // 时间推进后来一次 Activity：仍工作中，且活动时间被刷新。
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::Activity,
            "s1",
            Some(111),
            None,
            500,
        );
        assert_eq!(r.working_count(), 1);
    }

    #[test]
    fn working_backstop_demotes_stale_working() {
        let r = reg();
        // 工作中、活动时间停在 t=1。
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "stuck",
            Some(std::process::id()),
            None,
            1,
        );
        assert_eq!(r.working_count(), 1);
        // 超时阈值很小 → 应被降级为空闲。
        assert!(r.working_backstop_sweep(10));
        assert_eq!(r.working_count(), 0);
        assert_eq!(r.idle_count(), 1);
        // 再扫一次无变化（已是空闲）。
        assert!(!r.working_backstop_sweep(10));
    }

    #[test]
    fn force_idle_demotes_only_working() {
        let r = reg();
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "s1",
            Some(111),
            None,
            1,
        );
        assert_eq!(r.working_count(), 1);
        // 命中工作中 → 置空闲。
        assert!(r.force_idle("s1"));
        assert_eq!(r.working_count(), 0);
        assert_eq!(r.idle_count(), 1);
        // 已空闲再置无变化；未知 session 无变化。
        assert!(!r.force_idle("s1"));
        assert!(!r.force_idle("nope"));
    }

    #[test]
    fn refresh_by_pids_exempts_inflight_from_backstop() {
        // 场景1：在途 ask（pid 命中）→ 刷新活动 → 不被兜底降级。
        let r = reg();
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "asking",
            Some(4242),
            None,
            1, // 活动时间停在很久以前
        );
        assert!(r.refresh_by_pids(&[4242])); // 刷新到 now
        assert!(!r.working_backstop_sweep(10)); // 新鲜 → 不降级
        assert_eq!(r.working_count(), 1);

        // 场景2：pid 不在在途集合 → 不刷新 → 陈旧 → 被降级。
        let r2 = reg();
        r2.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "stale",
            Some(4242),
            None,
            1,
        );
        assert!(!r2.refresh_by_pids(&[9999])); // pid 不匹配，未刷新
        assert!(r2.working_backstop_sweep(10)); // 陈旧 → 降级
        assert_eq!(r2.working_count(), 0);
    }

    #[test]
    fn refresh_by_session_ids_exempts_pidless_from_backstop() {
        // 无 pid agent（Codex app-server / Claude scrubbed）等人回答期间靠 session_id 豁免，不被降级。
        let r = reg();
        r.apply_event(
            AgentKind::Codex,
            LifecycleEvent::TurnStart,
            "asking",
            None, // 无 pid（共享 app-server 归一）
            None,
            1, // 活动时间停在很久以前
        );
        assert_eq!(r.working_count(), 1);
        // 命中在途 session → 刷新到 now → 不被兜底降级。
        assert!(r.refresh_by_session_ids(&["asking".to_string()]));
        assert!(!r.working_backstop_sweep(10));
        assert_eq!(r.working_count(), 1);

        // 不在在途集合 → 不刷新 → 陈旧 → 被降级。
        let r2 = reg();
        r2.apply_event(
            AgentKind::Codex,
            LifecycleEvent::TurnStart,
            "stale",
            None,
            None,
            1,
        );
        assert!(!r2.refresh_by_session_ids(&["other".to_string()]));
        assert!(r2.working_backstop_sweep(10));
        assert_eq!(r2.working_count(), 0);
        // 空集合 → 未命中。
        assert!(!r2.refresh_by_session_ids(&[]));
    }

    #[test]
    fn turn_start_without_session_start_registers() {
        // spec D6：缺 session-start 也能登记。
        let r = reg();
        r.apply_event(
            AgentKind::Codex,
            LifecycleEvent::TurnStart,
            "s1",
            Some(7),
            None,
            100,
        );
        assert_eq!(r.working_count(), 1);
    }

    #[test]
    fn pid_rotation_ends_previous_session() {
        // spec D7：同一 pid 新 session ⇒ 旧的判结束。
        let r = reg();
        r.apply_event(
            AgentKind::Codex,
            LifecycleEvent::SessionStart,
            "old",
            Some(42),
            None,
            100,
        );
        r.apply_event(
            AgentKind::Codex,
            LifecycleEvent::SessionStart,
            "new",
            Some(42),
            None,
            200,
        );
        let snap = r.snapshot();
        let arr = snap.as_array().unwrap();
        let old = arr.iter().find(|x| x["sessionId"] == "old").unwrap();
        let new = arr.iter().find(|x| x["sessionId"] == "new").unwrap();
        assert_eq!(old["state"], "ended");
        assert_eq!(new["state"], "idle");
    }

    #[test]
    fn session_end_moves_to_ended() {
        let r = reg();
        r.apply_event(
            AgentKind::Cursor,
            LifecycleEvent::SessionStart,
            "s1",
            Some(1),
            None,
            100,
        );
        r.apply_event(
            AgentKind::Cursor,
            LifecycleEvent::SessionEnd,
            "s1",
            Some(1),
            None,
            110,
        );
        let arr = r.snapshot();
        let arr = arr.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["state"], "ended");
    }

    #[test]
    fn ttl_only_affects_pidless_records() {
        let r = reg();
        // 有 pid 的不受 TTL 影响（但本测试进程 pid 一般存活，poll 不杀它）。
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "withpid",
            Some(std::process::id()),
            None,
            1,
        );
        // 无 pid 且活动很久以前 → TTL 结束。
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::TurnStart,
            "nopid",
            None,
            None,
            1,
        );
        let changed = r.ttl_sweep();
        assert!(changed);
        let arr = r.snapshot();
        let arr = arr.as_array().unwrap();
        let nopid = arr.iter().find(|x| x["sessionId"] == "nopid").unwrap();
        assert_eq!(nopid["state"], "ended");
        let withpid = arr.iter().find(|x| x["sessionId"] == "withpid").unwrap();
        assert_ne!(withpid["state"], "ended");
    }

    #[test]
    fn ended_capped_at_ten() {
        let r = reg();
        for i in 0..15 {
            let s = format!("s{i}");
            r.apply_event(
                AgentKind::Codex,
                LifecycleEvent::SessionStart,
                &s,
                None,
                None,
                1,
            );
            r.apply_event(
                AgentKind::Codex,
                LifecycleEvent::SessionEnd,
                &s,
                None,
                None,
                2,
            );
        }
        let arr = r.snapshot();
        let ended: Vec<_> = arr
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["state"] == "ended")
            .collect();
        assert_eq!(ended.len(), MAX_ENDED);
        // 最早的应被淘汰，保留最近 10（s5..s14）。
        assert!(ended.iter().any(|x| x["sessionId"] == "s14"));
        assert!(!ended.iter().any(|x| x["sessionId"] == "s0"));
    }

    #[test]
    fn touch_activity_only_existing() {
        let r = reg();
        assert!(!r.touch_activity(AgentKind::Claude, "missing", None));
        r.apply_event(
            AgentKind::Claude,
            LifecycleEvent::SessionStart,
            "s1",
            None,
            None,
            1,
        );
        assert!(r.touch_activity(AgentKind::Claude, "s1", Some(9)));
    }

    #[test]
    fn touch_activity_by_pid_matches_existing_only() {
        // MCP 模式兜底：拿不到 session_id，按 (kind, pid) 匹配已存在 session 刷新。
        let r = reg();
        // 无记录 → 未命中。
        assert!(!r.touch_activity_by_pid(AgentKind::Codex, 4242));
        r.apply_event(
            AgentKind::Codex,
            LifecycleEvent::TurnStart,
            "s1",
            Some(4242),
            None,
            1,
        );
        // 命中并刷新 last_activity。
        assert!(r.touch_activity_by_pid(AgentKind::Codex, 4242));
        // 家族不匹配 → 未命中（不跨家族污染）。
        assert!(!r.touch_activity_by_pid(AgentKind::Claude, 4242));
        // pid=0 / 不存在的 pid → 未命中。
        assert!(!r.touch_activity_by_pid(AgentKind::Codex, 0));
        assert!(!r.touch_activity_by_pid(AgentKind::Codex, 9999));
        // 只刷新、不新建 session。
        let arr = r.snapshot();
        assert_eq!(arr.as_array().unwrap().len(), 1);
    }

    #[test]
    fn seq_is_monotonic_and_exposed_in_snapshot() {
        let r = reg();
        r.apply_event(AgentKind::Cursor, LifecycleEvent::TurnStart, "s1", None, None, 1);
        r.apply_event(AgentKind::Claude, LifecycleEvent::TurnStart, "s2", None, None, 2);
        let arr = r.snapshot();
        let arr = arr.as_array().unwrap();
        let s1 = arr.iter().find(|x| x["sessionId"] == "s1").unwrap();
        let s2 = arr.iter().find(|x| x["sessionId"] == "s2").unwrap();
        assert_eq!(s1["seq"], 1);
        assert_eq!(s2["seq"], 2);
        // 结束一个再新建：编号不复用（单调）。
        r.apply_event(AgentKind::Codex, LifecycleEvent::SessionEnd, "s1", None, None, 3);
        r.apply_event(AgentKind::Codex, LifecycleEvent::TurnStart, "s3", None, None, 4);
        let arr = r.snapshot();
        let arr = arr.as_array().unwrap();
        let s3 = arr.iter().find(|x| x["sessionId"] == "s3").unwrap();
        assert_eq!(s3["seq"], 3);
    }

    #[test]
    fn current_tool_set_clear_and_snapshot_injection() {
        let r = reg();
        r.apply_event(AgentKind::Cursor, LifecycleEvent::TurnStart, "s1", None, None, 1);
        // set：snapshot 注入 currentTool。
        r.set_current_tool(
            AgentKind::Cursor,
            "s1",
            None,
            "Shell".into(),
            Some("cargo test".into()),
        );
        let snap = r.snapshot();
        let s1 = snap
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["sessionId"] == "s1")
            .unwrap()
            .clone();
        assert_eq!(s1["currentTool"]["name"], "Shell");
        assert_eq!(s1["currentTool"]["object"], "cargo test");
        assert!(s1["currentTool"]["at"].as_u64().is_some());
        // clear：currentTool 消失。
        r.clear_current_tool(AgentKind::Cursor, "s1");
        let snap = r.snapshot();
        let s1 = snap
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["sessionId"] == "s1")
            .unwrap()
            .clone();
        assert!(s1.get("currentTool").is_none());
    }

    #[test]
    fn turn_steps_count_and_reset() {
        let r = reg();
        r.apply_event(AgentKind::Cursor, LifecycleEvent::TurnStart, "s1", None, None, 10);
        // 两次 PreToolUse → 第 2 步；turnStartedAt 为 turn-start 时刻。
        r.set_current_tool(AgentKind::Cursor, "s1", None, "Read".into(), None);
        r.set_current_tool(AgentKind::Cursor, "s1", None, "Shell".into(), None);
        let snap = r.snapshot();
        let s1 = snap
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["sessionId"] == "s1")
            .unwrap()
            .clone();
        assert_eq!(s1["turnSteps"].as_u64(), Some(2));
        assert_eq!(s1["turnStartedAt"].as_u64(), Some(10));
        // turn-end 清零；再 turn-start 重新起算。
        r.apply_event(AgentKind::Cursor, LifecycleEvent::TurnEnd, "s1", None, None, 20);
        let snap = r.snapshot();
        let s1 = snap
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["sessionId"] == "s1")
            .unwrap()
            .clone();
        assert!(s1.get("turnSteps").is_none());
        assert!(s1.get("turnStartedAt").is_none());
        r.apply_event(AgentKind::Cursor, LifecycleEvent::TurnStart, "s1", None, None, 30);
        r.set_current_tool(AgentKind::Cursor, "s1", None, "Write".into(), None);
        let snap = r.snapshot();
        let s1 = snap
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["sessionId"] == "s1")
            .unwrap()
            .clone();
        assert_eq!(s1["turnSteps"].as_u64(), Some(1));
        assert_eq!(s1["turnStartedAt"].as_u64(), Some(30));
    }

    #[test]
    fn current_tool_cleared_on_turn_end_and_not_persisted() {
        let r = reg();
        r.apply_event(AgentKind::Claude, LifecycleEvent::TurnStart, "s1", None, None, 1);
        r.set_current_tool(AgentKind::Claude, "s1", None, "Read".into(), Some("a.rs".into()));
        // 回合结束清除。
        r.apply_event(AgentKind::Claude, LifecycleEvent::TurnEnd, "s1", None, None, 2);
        let snap = r.snapshot();
        let s1 = snap
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["sessionId"] == "s1")
            .unwrap()
            .clone();
        assert!(s1.get("currentTool").is_none());
        // 不落盘：Persisted 序列化不含 current_tool。
        r.set_current_tool(AgentKind::Claude, "s1", None, "Write".into(), None);
        let inner = r.inner.lock().unwrap();
        let data = Persisted {
            active: inner.active.clone(),
            ended: inner.ended.iter().cloned().collect(),
        };
        drop(inner);
        let json = serde_json::to_string(&data).unwrap();
        assert!(!json.contains("currentTool"));
        assert!(!json.contains("current_tool"));
    }
}
