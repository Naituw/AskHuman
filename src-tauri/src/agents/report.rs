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
use crate::ipc::{ClientMsg, ToolPhase, ToolReport};

/// 入口：`args` 为 `__agent-hook` 之后的参数（`[<agent>, <event>]`）。失败一律静默退出。
pub fn run(args: &[String]) {
    let Some(intended) = args.first().and_then(|s| AgentKind::parse(s)) else {
        return;
    };
    let Some(event) = args.get(1).and_then(|s| super::LifecycleEvent::parse(s)) else {
        return;
    };

    let env: HashMap<String, String> = std::env::vars().collect();

    // 去重：跳过「兼容加载他家 hook」造成的误触发。
    let running = detect::detect_running_agent_from(&env);
    // Grok 默认会合并触发 `~/.claude`/`~/.cursor` 的兼容 hook（见 grok hooks 文档）：这些兼容 hook 的
    // intended 是 claude/cursor，但真实运行家族是 Grok（env 有 GROK_HOOK_EVENT/GROK_SESSION_ID）→ 一律
    // 跳过，只认 Grok 原生 hook（intended==grok），避免把 Grok 会话错标成 Claude/Cursor 或重复登记。
    if running == Some(AgentKind::Grok) && intended != AgentKind::Grok {
        return;
    }
    // Cursor 兼容加载了 ~/.claude → Claude hook 在 cursor-agent 下双触发：仅当 intended=claude 且实际
    // 运行家族是 Cursor 时跳过（保留 cursor 自身那次）。其它情况一律不跳过：Codex/Cursor 只会执行自己的
    // hook，绝不能因 env 里残留 CURSOR_*（例如从 cursor-agent 环境启动 Codex/Claude）而误杀其自身上报。
    if intended == AgentKind::Claude && running == Some(AgentKind::Cursor) {
        return;
    }

    let stdin = read_stdin_json();
    let session_id = resolve_session_id(intended, &env, stdin.as_ref());
    // 无会话 ID 无法作为身份键（spec D7），直接放弃（best-effort）。
    if session_id.is_empty() {
        return;
    }
    // 不在 hook 侧 walk 进程树（~280ms），改发 ppid 给 daemon 缓存解析。
    let hint_pid = {
        #[cfg(unix)]
        {
            Some(unsafe { libc::getppid() } as u32)
        }
        #[cfg(not(unix))]
        {
            None::<u32>
        }
    };
    let cwd = resolve_cwd(&env, stdin.as_ref());

    // 仅 activity 事件（Pre/PostToolUse）才尝试解析工具信息；其余事件无工具。
    let tool = if matches!(event, super::LifecycleEvent::Activity) {
        extract_tool(stdin.as_ref())
    } else {
        None
    };

    // 插话轮询（spec agent-interject D3/D4）：仅 **PreToolUse**（stdin 判定阶段为 pre）、
    // 且已通过上方去重、且非 Grok（首期排除，无可靠传话通道）时，在上报的同一连接上
    // 读回一帧裁决；PostToolUse 与其余事件保持即发即走。
    let interject_poll = matches!(event, super::LifecycleEvent::Activity)
        && intended != AgentKind::Grok
        && stdin.as_ref().and_then(|v| detect_phase(v)) == Some(ToolPhase::Pre);

    let msg = ClientMsg::AgentEvent {
        agent: intended.as_str().to_string(),
        event: event.as_str().to_string(),
        session_id,
        pid: None,
        hint_pid,
        cwd,
        ts: 0,
        tool,
        interject_poll,
    };
    if interject_poll {
        if let crate::client::InterjectPollOutcome::Deny(text) =
            crate::client::report_agent_event_with_poll(msg)
        {
            print_deny_json(intended, &text);
        }
    } else {
        crate::client::report_agent_event(msg);
    }
}

/// 输出各家 PreToolUse 的 deny JSON（stdout，随后调用方 exit 0；spec agent-interject D3）。
/// 消息经 `prompts::interject_deny_reason` 包装（`[USER INTERJECTION]` 协议文案）。
fn print_deny_json(kind: AgentKind, message: &str) {
    let json = deny_json(kind, message);
    println!("{json}");
}

/// 构造 deny JSON（纯函数，供单测）。Claude / Codex 同构（`hookSpecificOutput`）；
/// Cursor 用 `permission` + `user_message`/`agent_message` **双字段同文**：live 实测 + bundle
/// 静态核对（IDE cursor-agent-exec 与 CLI hooks-exec 的 deny 分支）证实**模型看到的拒绝理由
/// 取自 `user_message`**（`agent_message` 仅透传 protobuf、未见进模型的消费点，与官方文档
/// 「fed back to the agent」不符）；两字段都放完整协议文本，兼容未来 Cursor 按文档语义改用
/// `agent_message`。代价：UI 拦截提示显示整段协议文本（内含用户原话），可接受。
fn deny_json(kind: AgentKind, message: &str) -> Value {
    let reason = crate::prompts::interject_deny_reason(message);
    match kind {
        AgentKind::Cursor => serde_json::json!({
            "permission": "deny",
            "agent_message": reason.clone(),
            "user_message": reason,
        }),
        // Claude / Codex（Grok 不会走到：上游已排除）。
        _ => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        }),
    }
}

/// 从 hook stdin 解析本次工具调用（best-effort）：判 pre/post，pre 需能取到工具名（否则退化为无工具）。
fn extract_tool(stdin: Option<&Value>) -> Option<ToolReport> {
    let v = stdin?;
    match detect_phase(v)? {
        // post：清除不依赖名字，尽力带上（daemon 只按 session 清）。
        ToolPhase::Post => Some(ToolReport {
            name: tool_name(v).unwrap_or_default(),
            object: None,
            phase: ToolPhase::Post,
        }),
        // pre：无工具名无法展示 → 退化为无工具（纯心跳）。
        ToolPhase::Pre => {
            let name = tool_name(v)?;
            let object = super::activity::classify_tool(&name, tool_input(v).as_ref()).object;
            Some(ToolReport {
                name,
                object,
                phase: ToolPhase::Pre,
            })
        }
    }
}

/// 判断工具阶段：优先看显式 hook 事件名，否则按「有无结果字段 / 有无工具输入」启发式。
fn detect_phase(v: &Value) -> Option<ToolPhase> {
    for k in ["hook_event_name", "hookEventName"] {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
            let l = s.to_ascii_lowercase();
            if l.contains("pretooluse") {
                return Some(ToolPhase::Pre);
            }
            if l.contains("posttooluse") {
                return Some(ToolPhase::Post);
            }
        }
    }
    // 结果字段（非空）→ post。
    for k in [
        "tool_response",
        "tool_result",
        "tool_output",
        "function_call_output",
        "response",
        "output",
    ] {
        if v.get(k).map(|x| !x.is_null()).unwrap_or(false) {
            return Some(ToolPhase::Post);
        }
    }
    // 有工具名 / 输入 → pre。
    let has_tool = [
        "tool_name",
        "toolName",
        "tool",
        "tool_input",
        "toolInput",
        "tool_calls",
    ]
    .iter()
    .any(|k| v.get(*k).map(|x| !x.is_null()).unwrap_or(false));
    has_tool.then_some(ToolPhase::Pre)
}

/// 取工具名（各家字段兼容）。
fn tool_name(v: &Value) -> Option<String> {
    for k in ["tool_name", "toolName", "tool"] {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// 取工具输入（对象或原始 JSON 字符串，`classify_tool` 内部再 `parse_args`）。
fn tool_input(v: &Value) -> Option<Value> {
    for k in ["tool_input", "toolInput", "input", "arguments"] {
        if let Some(x) = v.get(k) {
            if !x.is_null() {
                return Some(x.clone());
            }
        }
    }
    None
}

/// 解析会话 ID：env 专用变量优先，其次 stdin JSON 的若干常见字段。
fn resolve_session_id(
    kind: AgentKind,
    env: &HashMap<String, String>,
    stdin: Option<&Value>,
) -> String {
    if let Some(s) = detect::session_id_from_env_map(kind, env) {
        return s;
    }
    if let Some(v) = stdin {
        for key in [
            "session_id",
            "sessionId",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
        ] {
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
    for key in [
        "CURSOR_PROJECT_DIR",
        "GROK_WORKSPACE_ROOT",
        "CLAUDE_PROJECT_DIR",
    ] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn phase_pre_from_tool_input() {
        let v = json!({"tool_name":"Shell","tool_input":{"command":"cargo test"}});
        assert_eq!(detect_phase(&v), Some(ToolPhase::Pre));
        let t = extract_tool(Some(&v)).unwrap();
        assert_eq!(t.phase, ToolPhase::Pre);
        assert_eq!(t.name, "Shell");
        assert_eq!(t.object.as_deref(), Some("cargo test"));
    }

    #[test]
    fn phase_post_from_response_field() {
        let v = json!({"tool_name":"Read","tool_input":{"file_path":"/a/b.rs"},"tool_response":{"ok":true}});
        assert_eq!(detect_phase(&v), Some(ToolPhase::Post));
        assert_eq!(extract_tool(Some(&v)).unwrap().phase, ToolPhase::Post);
    }

    #[test]
    fn explicit_hook_event_name_wins() {
        // 显式事件名优先于「有 tool_input 像 pre」的启发式。
        let v = json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{}});
        assert_eq!(detect_phase(&v), Some(ToolPhase::Post));
    }

    #[test]
    fn no_tool_fields_is_none() {
        let v = json!({"session_id":"s","cwd":"/x"});
        assert_eq!(detect_phase(&v), None);
        assert!(extract_tool(Some(&v)).is_none());
    }

    #[test]
    fn pre_without_name_degrades() {
        let v = json!({"tool_input":{"command":"ls"}});
        assert_eq!(detect_phase(&v), Some(ToolPhase::Pre));
        assert!(extract_tool(Some(&v)).is_none());
    }

    #[test]
    fn deny_json_claude_codex_shape() {
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let v = deny_json(kind, "改用方案 B");
            let out = &v["hookSpecificOutput"];
            assert_eq!(out["hookEventName"], "PreToolUse");
            assert_eq!(out["permissionDecision"], "deny");
            let reason = out["permissionDecisionReason"].as_str().unwrap();
            assert!(reason.starts_with("[USER INTERJECTION]"));
            assert!(reason.contains("<user_message>\n改用方案 B\n</user_message>"));
            assert!(v.get("permission").is_none(), "不应混入 Cursor 字段");
        }
    }

    #[test]
    fn deny_json_cursor_shape() {
        let v = deny_json(AgentKind::Cursor, "停一下");
        assert_eq!(v["permission"], "deny");
        // live 实测：Cursor 喂回模型的拒绝理由取自 user_message（agent_message 未见消费）——
        // 两字段须同为完整协议文本，缺一即丢话。
        let user_msg = v["user_message"].as_str().unwrap();
        assert!(user_msg.starts_with("[USER INTERJECTION]"));
        assert!(user_msg.contains("停一下"));
        assert_eq!(v["agent_message"], v["user_message"]);
        assert!(
            v.get("hookSpecificOutput").is_none(),
            "不应混入 Claude 字段"
        );
    }
}
