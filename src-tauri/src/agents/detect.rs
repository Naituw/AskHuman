//! 运行时识别真实 Agent、从 env 取会话 ID、向上 walk 进程树定位 Agent 进程、kill-0 探活。
//!
//! 复刻 `demo/agent-lifecycle/harness/common.cjs` 的实测逻辑（FINDINGS §7.6 / §1.1）：
//! - `detect_running_agent`：从 hook/ask 子进程 env 判定真实 Agent（解决 Cursor 双触发去重）。
//!   顺序 **必须** 先判 Cursor（它也会设 `CLAUDE_PROJECT_DIR`），再 Codex，再 Claude。
//! - `walk_agent_pid`：从本进程向上回溯进程树，取第一个命中 Agent token、且非自身的祖先 pid。
//! - `pid_alive`：`kill(pid, 0)`（unix）判存活。

use std::collections::HashMap;

use super::AgentKind;

/// 进程链节点：pid / 父 pid / 可执行名(comm) / 完整命令行(command)。
#[derive(Debug, Clone)]
struct ProcEntry {
    pid: u32,
    ppid: u32,
    comm: String,
    command: String,
}

/// 本程序自身在进程树里的标记（避免把 reporter / daemon 自身误判成 Agent）。
const SELF_MARKERS: [&str; 3] = ["askhuman", "__agent-hook", "humaninloop"];

/// 从一个 env map 判定真实运行的 Agent（去重判据，见 FINDINGS §7.6）。
///
/// `CLAUDE_PROJECT_DIR` **不可** 作判据——Cursor 兼容性也会设它，故必须先判 Cursor。
/// 判不出返回 `None`（调用方应按 intended 处理，避免漏报）。
pub fn detect_running_agent_from(env: &HashMap<String, String>) -> Option<AgentKind> {
    let has = |k: &str| env.contains_key(k);
    if has("CURSOR_AGENT") || has("CURSOR_VERSION") || has("CURSOR_PROJECT_DIR") {
        return Some(AgentKind::Cursor);
    }
    if env.keys().any(|k| k.starts_with("CODEX_")) {
        return Some(AgentKind::Codex);
    }
    if has("CLAUDECODE") || has("CLAUDE_CODE_SESSION_ID") {
        return Some(AgentKind::Claude);
    }
    None
}

/// 读取本进程 env 判定真实 Agent。
pub fn detect_running_agent() -> Option<AgentKind> {
    detect_running_agent_from(&current_env())
}

/// 各家会话 ID 的 env 变量名（shell 工具子进程注入；hook 子进程通常无，靠 stdin）。
pub fn session_id_env_var(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "CLAUDE_CODE_SESSION_ID",
        AgentKind::Codex => "CODEX_THREAD_ID",
        AgentKind::Cursor => "CURSOR_CONVERSATION_ID",
    }
}

/// 从一个 env map 取指定家族的会话 ID。
pub fn session_id_from_env_map(kind: AgentKind, env: &HashMap<String, String>) -> Option<String> {
    env.get(session_id_env_var(kind))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 读取本进程 env 取会话 ID。
pub fn session_id_from_env(kind: AgentKind) -> Option<String> {
    session_id_from_env_map(kind, &current_env())
}

fn current_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

/// 识别一个进程节点是否「属于」指定家族的 Agent 进程。
///
/// - Claude / Codex：可执行名(comm) 子串含 `claude` / `codex`，或 argv0 basename 等于之。
/// - Cursor：cursor-agent 的可执行名是 `agent`（argv0 basename == `agent`），或命令行特异含 `cursor-agent`。
fn matches_agent(entry: &ProcEntry, kind: AgentKind) -> bool {
    let comm = entry.comm.to_ascii_lowercase();
    let command = entry.command.to_ascii_lowercase();
    let argv0_base = command
        .split_whitespace()
        .next()
        .map(basename)
        .unwrap_or_default();
    match kind {
        AgentKind::Claude => comm.contains("claude") || argv0_base == "claude",
        AgentKind::Codex => comm.contains("codex") || argv0_base == "codex",
        AgentKind::Cursor => {
            argv0_base == "agent"
                || comm.contains("cursor-agent")
                || command.contains("cursor-agent")
        }
    }
}

fn is_self(entry: &ProcEntry) -> bool {
    let hay = format!("{} {}", entry.comm, entry.command).to_ascii_lowercase();
    SELF_MARKERS.iter().any(|m| hay.contains(m))
}

fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// 从 `start_pid` 向上回溯进程树，返回第一个命中指定家族、且非自身的祖先 pid。
/// 找不到（或非 unix）返回 `None` → 调用方落 TTL 兜底。
pub fn walk_agent_pid(kind: AgentKind, start_pid: u32) -> Option<u32> {
    let chain = process_chain(start_pid);
    chain
        .into_iter()
        .filter(|e| !is_self(e))
        .find(|e| matches_agent(e, kind))
        .map(|e| e.pid)
}

/// 从当前进程向上 walk 定位指定家族的 Agent pid。
pub fn walk_agent_pid_from_self(kind: AgentKind) -> Option<u32> {
    walk_agent_pid(kind, std::process::id())
}

/// 进程是否存活（`kill(pid, 0)`：Ok / EPERM 视为存活，ESRCH 为已死）。
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if r == 0 {
        return true;
    }
    // errno: EPERM(1) 存在但无权限 → 存活；ESRCH(3) → 已死。
    matches!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM))
}

#[cfg(not(unix))]
pub fn pid_alive(_pid: u32) -> bool {
    false
}

// ── 进程链回溯（unix：调用 `ps`；非 unix：空） ──

#[cfg(unix)]
fn process_chain(start_pid: u32) -> Vec<ProcEntry> {
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut pid = start_pid;
    while pid > 1 && seen.insert(pid) {
        let Some((ppid, comm)) = ps_ppid_comm(pid) else {
            break;
        };
        let command = ps_command(pid).unwrap_or_default();
        chain.push(ProcEntry {
            pid,
            ppid,
            comm,
            command,
        });
        if ppid == 0 {
            break;
        }
        pid = ppid;
    }
    chain
}

#[cfg(not(unix))]
fn process_chain(_start_pid: u32) -> Vec<ProcEntry> {
    Vec::new()
}

/// `ps -o ppid=,comm= -p <pid>` → (ppid, comm)。
#[cfg(unix)]
fn ps_ppid_comm(pid: u32) -> Option<(u32, String)> {
    let out = run_ps(&["-o", "ppid=,comm=", "-p", &pid.to_string()])?;
    let trimmed = out.trim();
    let mut it = trimmed.splitn(2, char::is_whitespace);
    let ppid = it.next()?.trim().parse::<u32>().ok()?;
    let comm = it.next().unwrap_or("").trim().to_string();
    Some((ppid, comm))
}

#[cfg(unix)]
fn ps_command(pid: u32) -> Option<String> {
    run_ps(&["-o", "command=", "-p", &pid.to_string()]).map(|s| s.trim().to_string())
}

#[cfg(unix)]
fn run_ps(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("ps").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).to_string();
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn detect_cursor_takes_priority_over_claude_project_dir() {
        // Cursor 兼容性会设 CLAUDE_PROJECT_DIR，但必须判成 cursor。
        let env = env_of(&[("CURSOR_AGENT", "1"), ("CLAUDE_PROJECT_DIR", "/x")]);
        assert_eq!(detect_running_agent_from(&env), Some(AgentKind::Cursor));
    }

    #[test]
    fn detect_codex_by_prefix() {
        let env = env_of(&[("CODEX_MANAGED_BY_NPM", "1")]);
        assert_eq!(detect_running_agent_from(&env), Some(AgentKind::Codex));
    }

    #[test]
    fn detect_claude_when_only_claude_markers() {
        let env = env_of(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "abc")]);
        assert_eq!(detect_running_agent_from(&env), Some(AgentKind::Claude));
    }

    #[test]
    fn detect_none_when_ambiguous() {
        // 仅 CLAUDE_PROJECT_DIR 不足以判定（Cursor 也设它）。
        let env = env_of(&[("CLAUDE_PROJECT_DIR", "/x")]);
        assert_eq!(detect_running_agent_from(&env), None);
    }

    #[test]
    fn session_id_from_env_reads_per_kind_var() {
        let env = env_of(&[("CODEX_THREAD_ID", " tid ")]);
        assert_eq!(
            session_id_from_env_map(AgentKind::Codex, &env),
            Some("tid".to_string())
        );
        assert_eq!(session_id_from_env_map(AgentKind::Claude, &env), None);
    }

    #[test]
    fn matches_agent_recognizes_cursor_agent_named_agent() {
        let e = ProcEntry {
            pid: 1,
            ppid: 0,
            comm: "/Users/u/.local/bin/agent".to_string(),
            command: "agent --use-system-ca /x/index.js --yolo".to_string(),
        };
        assert!(matches_agent(&e, AgentKind::Cursor));
        assert!(!matches_agent(&e, AgentKind::Claude));
    }

    #[test]
    fn self_marker_excluded() {
        let e = ProcEntry {
            pid: 1,
            ppid: 0,
            comm: "AskHuman".to_string(),
            command: "AskHuman __agent-hook cursor turn-start".to_string(),
        };
        assert!(is_self(&e));
    }
}
