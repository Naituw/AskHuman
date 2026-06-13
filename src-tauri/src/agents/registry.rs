//! Agent 注册表：daemon 内存维护被追踪 agent 的状态，并持久化到 `~/.askhuman/agents.json`。
//!
//! 身份模型（spec D7）：**以 `session_id` 为身份**；`pid` 仅用于存活轮询。同一 pid 出现新
//! `session_id` ⇒ 旧 session 判「已结束」、新 session 复用该 pid（一个 pid 同时至多一个活动 session）。
//!
//! 状态推导（spec D5/D8/D12）：turn-start→工作中、turn-end→空闲；进程存活轮询是权威「已结束」
//! 判据；仅当 **拿不到 pid** 时用 1 小时 TTL 兜底（任意事件 / ask 调用都刷新活动时间）。

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
}

/// daemon 内唯一的 agent 注册表（线程安全）。
pub struct AgentRegistry {
    inner: Mutex<Inner>,
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
        drop(inner);
        reg
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
        if let Some(pid) = pid {
            let rotated: Vec<AgentRecord> = drain_where(&mut inner.active, |r| {
                r.pid == Some(pid) && r.session_id != session_id
            });
            for mut r in rotated {
                r.state = AgentState::Ended;
                r.ended_at = Some(now);
                push_ended(&mut inner.ended, r);
            }
        }

        // 幂等登记 + 更新（任何事件都能建，不依赖 session-start）。
        let idx = inner.active.iter().position(|r| r.session_id == session_id);
        let idx = match idx {
            Some(i) => {
                let r = &mut inner.active[i];
                if pid.is_some() {
                    r.pid = pid;
                }
                if cwd.is_some() {
                    r.cwd = cwd;
                }
                r.last_activity = now;
                i
            }
            None => {
                inner.active.push(AgentRecord {
                    kind,
                    session_id: session_id.to_string(),
                    pid,
                    title: None,
                    cwd,
                    started_at: now,
                    last_activity: now,
                    state: AgentState::Idle,
                    ended_at: None,
                });
                inner.active.len() - 1
            }
        };

        // 事件 → 状态。
        match event {
            LifecycleEvent::SessionStart => { /* 已确保登记，保持 Idle */ }
            LifecycleEvent::TurnStart => inner.active[idx].state = AgentState::Working,
            LifecycleEvent::TurnEnd => inner.active[idx].state = AgentState::Idle,
            LifecycleEvent::SessionEnd => {
                let mut r = inner.active.remove(idx);
                r.state = AgentState::Ended;
                r.ended_at = Some(now);
                push_ended(&mut inner.ended, r);
            }
        }
        true
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

    /// 工作中 agent 数（spec D18：仅它与窗口连接阻止 daemon 空闲退出；空闲 agent 不算）。
    pub fn working_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .active
            .iter()
            .filter(|r| r.state == AgentState::Working)
            .count()
    }

    /// 构造全量快照（解析缺失标题并缓存）。返回 agents 列表 Value（前端按类型分组、按状态排序）。
    pub fn snapshot(&self) -> Value {
        let mut inner = self.inner.lock().unwrap();
        // 惰性补齐标题（已解析的不再重复）；ended 记录的会话文件依然在盘上，同样补齐。
        for r in inner.active.iter_mut() {
            if r.title.is_none() {
                r.title = resolve_title(r.kind, &r.session_id);
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
        serde_json::to_value(&list).unwrap_or(Value::Array(vec![]))
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
        r.apply_event(AgentKind::Claude, LifecycleEvent::SessionStart, "s1", Some(111), None, 100);
        assert_eq!(r.working_count(), 0);
        r.apply_event(AgentKind::Claude, LifecycleEvent::TurnStart, "s1", Some(111), None, 110);
        assert_eq!(r.working_count(), 1);
        r.apply_event(AgentKind::Claude, LifecycleEvent::TurnEnd, "s1", Some(111), None, 120);
        assert_eq!(r.working_count(), 0);
    }

    #[test]
    fn turn_start_without_session_start_registers() {
        // spec D6：缺 session-start 也能登记。
        let r = reg();
        r.apply_event(AgentKind::Codex, LifecycleEvent::TurnStart, "s1", Some(7), None, 100);
        assert_eq!(r.working_count(), 1);
    }

    #[test]
    fn pid_rotation_ends_previous_session() {
        // spec D7：同一 pid 新 session ⇒ 旧的判结束。
        let r = reg();
        r.apply_event(AgentKind::Codex, LifecycleEvent::SessionStart, "old", Some(42), None, 100);
        r.apply_event(AgentKind::Codex, LifecycleEvent::SessionStart, "new", Some(42), None, 200);
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
        r.apply_event(AgentKind::Cursor, LifecycleEvent::SessionStart, "s1", Some(1), None, 100);
        r.apply_event(AgentKind::Cursor, LifecycleEvent::SessionEnd, "s1", Some(1), None, 110);
        let arr = r.snapshot();
        let arr = arr.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["state"], "ended");
    }

    #[test]
    fn ttl_only_affects_pidless_records() {
        let r = reg();
        // 有 pid 的不受 TTL 影响（但本测试进程 pid 一般存活，poll 不杀它）。
        r.apply_event(AgentKind::Claude, LifecycleEvent::TurnStart, "withpid", Some(std::process::id()), None, 1);
        // 无 pid 且活动很久以前 → TTL 结束。
        r.apply_event(AgentKind::Claude, LifecycleEvent::TurnStart, "nopid", None, None, 1);
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
            r.apply_event(AgentKind::Codex, LifecycleEvent::SessionStart, &s, None, None, 1);
            r.apply_event(AgentKind::Codex, LifecycleEvent::SessionEnd, &s, None, None, 2);
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
        r.apply_event(AgentKind::Claude, LifecycleEvent::SessionStart, "s1", None, None, 1);
        assert!(r.touch_activity(AgentKind::Claude, "s1", Some(9)));
    }
}
