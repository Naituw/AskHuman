//! 生命周期上报器：由三家 Agent 的用户级 hook 调用，向 daemon 上报一条事件即退出（spec D20）。
//!
//! 调用形如 `AskHuman __agent-hook <agent> <event>`：
//! - `<agent>`：claude / codex / cursor（hook 安装时写死，**意图**家族）。
//! - `<event>`：session-start / turn-start / turn-end / session-end。
//!
//! 会话 ID 解析优先级：env 专用变量 → hook 经 stdin 传入的 JSON（`session_id` 等）。
//! pid 通过向上 walk 进程树定位到真实 Agent 进程；cwd 取 stdin / env / 当前目录。
//!
//! 去重（Cursor 双触发，FINDINGS §7.6）：Cursor 会同时按自身 hook 与兼容的 Claude hook 触发；
//! 若 env 探测出的「真实运行家族」与意图家族不一致，则**跳过**本次上报，避免重复登记。

use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

use super::detect;
use super::AgentKind;
use crate::ipc::ClientMsg;

/// 入口：`args` 为 `__agent-hook` 之后的参数（`[<agent>, <event>]`）。失败一律静默退出。
pub fn run(args: &[String]) {
    let Some(intended) = args.first().and_then(|s| AgentKind::parse(s)) else {
        return;
    };
    let Some(event) = args.get(1).and_then(|s| super::LifecycleEvent::parse(s)) else {
        return;
    };

    let env: HashMap<String, String> = std::env::vars().collect();

    // 去重：唯一需要跳过的误触发是「Cursor 兼容加载了 ~/.claude → Claude hook 在 cursor-agent 下双触发」。
    // 即仅当 intended=claude 且实际运行家族是 Cursor 时跳过（保留 cursor 自身那次）。
    // 其它情况一律不跳过：Codex/Cursor 只会执行自己的 hook，绝不能因 env 里残留 CURSOR_*
    // （例如从 cursor-agent 环境启动 Codex/Claude）而误杀其自身上报（FINDINGS §7.6 的本意）。
    let running = detect::detect_running_agent_from(&env);
    if intended == AgentKind::Claude && running == Some(AgentKind::Cursor) {
        return;
    }

    let stdin = read_stdin_json();
    let session_id = resolve_session_id(intended, &env, stdin.as_ref());
    // 无会话 ID 无法作为身份键（spec D7），直接放弃（best-effort）。
    if session_id.is_empty() {
        return;
    }
    let pid = detect::walk_agent_pid_from_self(intended);
    let cwd = resolve_cwd(&env, stdin.as_ref());

    let msg = ClientMsg::AgentEvent {
        agent: intended.as_str().to_string(),
        event: event.as_str().to_string(),
        session_id,
        pid,
        cwd,
        ts: 0,
    };
    crate::client::report_agent_event(msg);
}

/// 解析会话 ID：env 专用变量优先，其次 stdin JSON 的若干常见字段。
fn resolve_session_id(kind: AgentKind, env: &HashMap<String, String>, stdin: Option<&Value>) -> String {
    if let Some(s) = detect::session_id_from_env_map(kind, env) {
        return s;
    }
    if let Some(v) = stdin {
        for key in ["session_id", "sessionId", "conversation_id", "conversationId", "thread_id", "threadId"] {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                let s = s.trim();
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    String::new()
}

/// 解析工作目录：stdin JSON `cwd` → env 工程目录 → 当前目录。
fn resolve_cwd(env: &HashMap<String, String>, stdin: Option<&Value>) -> Option<String> {
    if let Some(v) = stdin {
        if let Some(s) = v.get("cwd").and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return Some(s.to_string());
            }
        }
    }
    for key in ["CURSOR_PROJECT_DIR", "CLAUDE_PROJECT_DIR"] {
        if let Some(s) = env.get(key) {
            if !s.trim().is_empty() {
                return Some(s.clone());
            }
        }
    }
    std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
}

/// 读取 hook 经 stdin 传入的 JSON（best-effort，带超时，避免在无 stdin 时挂起）。
fn read_stdin_json() -> Option<Value> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return None;
    }
    // 在独立线程读，主线程最多等 500ms：hook 通常瞬间写完并关闭 stdin。
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::stdin().read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    let buf = rx.recv_timeout(Duration::from_millis(500)).ok()?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}
