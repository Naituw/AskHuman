//! CLI 调用参考提示词（供设置界面展示与复制）。
//!
//! 该提示词始终为英文（面向 AI 的契约），且**不内嵌** help 文本，
//! 而是指引 AI 执行 `<prog> --agent-help` 获取实时、随界面语言本地化的用法。
//!
//! 协作风格（aligned / autonomous / custom）见 `docs/specs/collaboration-style.md`：
//! 通道纪律固定；`Interview…` / 改方案确认段随配置替换。

use crate::config::{AppConfig, CollaborationStyle};

pub const USER_CONFIRMED_END_TURN_MARKER: &str = "[user_confirmed_end_turn]";
pub const SUBAGENT_PROTOCOL_RULES: &str = r#"**This protocol does not apply to subagents. If you are a subagent, do not use AskHuman.**
**When starting a subagent, tell it that it is a subagent and must not use AskHuman.**"#;

pub const fn subagent_guard_context() -> &'static str {
    "You are a subagent. Do not use AskHuman."
}

/// Default **aligned** collaboration body (tool-agnostic). Used as the custom-style editor default.
pub fn default_aligned_collaboration_text() -> &'static str {
    r#"- Interview me relentlessly about every aspect of the requirements until we reach a shared understanding. Use AskHuman as instructed in the interaction protocol above.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through AskHuman before making the change."#
}

/// **Autonomous** collaboration body (tool-agnostic).
pub fn default_autonomous_collaboration_text() -> &'static str {
    r#"- Prefer reasonable defaults and keep making progress. Do **not** interview relentlessly on every design branch.
- Ask via AskHuman only when you are blocked, the choice is irreversible or security-sensitive, the blast radius is high, or I explicitly asked to decide. Prefer exploring the codebase over asking.
- You may decide minor implementation details without asking. If you would change the agreed plan, scope, or user-visible behavior in a material way, ask first.
- When you finish a task and call whats-next, briefly note any important defaults you took."#
}

/// Resolve collaboration body for `style` + optional custom text (empty custom → aligned default).
pub fn collaboration_body(style: CollaborationStyle, custom_text: &str) -> String {
    match style {
        CollaborationStyle::Aligned => default_aligned_collaboration_text().to_string(),
        CollaborationStyle::Autonomous => default_autonomous_collaboration_text().to_string(),
        CollaborationStyle::Custom => {
            let t = custom_text.trim();
            if t.is_empty() {
                default_aligned_collaboration_text().to_string()
            } else {
                t.to_string()
            }
        }
    }
}

fn wrap_collaboration_style(style: CollaborationStyle, body: &str) -> String {
    format!(
        "<collaboration_style name=\"{}\">\n{}\n</collaboration_style>",
        style.as_str(),
        body.trim_end()
    )
}

fn collaboration_section_from_config() -> String {
    let cfg = AppConfig::load_without_secrets();
    let body = collaboration_body(
        cfg.general.collaboration_style,
        &cfg.general.collaboration_style_custom_text,
    );
    wrap_collaboration_style(cfg.general.collaboration_style, &body)
}

/// CLI 版：行为约束 + 当前协作风格段。
pub fn cli_reference() -> String {
    let program = crate::cli::help::program_name();
    let collab = collaboration_section_from_config();

    format!(
        r#"<mandatory_interaction_protocol>
{subagent_rules}

**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**
**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by invoking the `{program}` command through the Shell/Bash tool. DO NOT ask questions in direct output, and DO NOT ask by ending the turn.
- When invoking `{program}` through the Shell/Bash tool, you MUST set that tool call's timeout to 24 hours (86400000 ms) to give me enough time to respond.
- Before asking, run `{program} --agent-help` to learn its usage.

- When asking through `{program}`, provide predefined options whenever applicable, mark your recommended option(s) with `-o!`, and briefly explain your rationale.
- I can ONLY see what is delivered through `{program}`. Anything I need to review , or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through `{program}`, inline or attached with `-f`. Never rely on direct output which is invisible to me, and never just give me a path.
- After fully completing the current task—and never for questions, decisions, or next steps within it—you MUST run `{program} --whats-next` for the end-of-task handoff before ending, to ask me for a separate next task.
- If it returns a task, start working on it immediately and repeat this protocol when done. Do NOT end the turn/conversation or mark the request as complete unless `{program} --whats-next` returned that I approved ending the turn and there are no more tasks.
- After the user explicitly approves ending the turn, you MUST append the `{end_marker}` marker on a new final line at the end of your final output. Without that approval, you MUST NEVER output this marker.
</mandatory_interaction_protocol>

- When I ask for a project todo or defer a concrete task or suggestion until later, add it with `{program} todo add "<concise task>"`. Never use project todos for your own work plan or an unaccepted suggestion.
{collab}"#,
        program = program,
        end_marker = USER_CONFIRMED_END_TURN_MARKER,
        subagent_rules = SUBAGENT_PROTOCOL_RULES,
        collab = collab,
    )
}

/// MCP 版参考提示词：交互纪律与 CLI 版一致，工具改为 AskHuman MCP `ask`；协作风格同配置。
pub fn mcp_reference() -> String {
    let collab = collaboration_section_from_config();
    format!(
        r#"<mandatory_interaction_protocol>
{subagent_rules}

**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**
**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by calling the `ask` tool provided by the AskHuman MCP server (referred to below as the AskHuman `ask` tool). DO NOT ask questions in direct output, and DO NOT ask by ending the turn.
- The AskHuman `ask` tool blocks until I reply, which may take a long time; always wait for its result instead of giving up or proceeding on assumptions.

- When asking through the AskHuman `ask` tool, provide predefined options whenever applicable, mark your recommended option(s) as recommended, and briefly explain your rationale.
- I can ONLY see what is delivered through the AskHuman `ask` tool. Anything I need to review, or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through the AskHuman `ask` tool, inline or attached as files. Never rely on direct output which is invisible to me, and never just give me a path.
- After fully completing the current task—and never for questions, decisions, or next steps within it—you MUST call the AskHuman `whats_next` tool for the end-of-task handoff before ending, to ask me for a separate next task.
- If it returns a task, start working on it immediately and repeat this protocol when done. Do NOT end the turn/conversation or mark the request as complete unless the `whats_next` result says I approved ending the turn and there are no more tasks.
- After the user explicitly approves ending the turn, you MUST append the `{end_marker}` marker on a new final line at the end of your final output. Without that approval, you MUST NEVER output this marker.
</mandatory_interaction_protocol>

- When I ask for a project todo or defer a concrete task or suggestion until later, call the AskHuman MCP `todo_add` tool with the task text (optional `auto: true` for auto-run). Never use project todos for your own work plan or an unaccepted suggestion.
{collab}"#,
        end_marker = USER_CONFIRMED_END_TURN_MARKER,
        subagent_rules = SUBAGENT_PROTOCOL_RULES,
        collab = collab,
    )
}

/// Grok skill 正文：装进 `~/.grok/skills/interaction-protocol/SKILL.md` 的 Markdown 主体（不含 YAML
/// frontmatter，由 `grok_skill.rs` 拼接）。
///
/// **复用 [`mcp_reference`] + 追加一小段 Grok 说明**（单一来源，避免协议措辞漂移）：正文 = MCP 版参考
/// 提示词原样 + 末尾一段「在 Grok 里怎么联系我」。为何还要那段：Grok 默认模型 Composer **不读**
/// `~/.grok/AGENTS.md` 全局 rules（调研证实），交互协议只能经 skill 承载；且 Grok 把 MCP 工具的暴露/发现
/// 方式做得与「直接可见工具」不同，需点明「工具没列出时先搜再调」。
///
/// **刻意保持通用、不写死具体 harness / 工具名**（如 Composer / Grok Build / `CallMcpTool` / `search_tool` /
/// `use_tool`）：Grok 后续版本会改这些名字与机制，写死会过时误导。故只声明一条「联系人类」的降级阶梯：
/// 1）**优先** MCP `ask` 工具（P2 定案：MCP 优先于 shell/CLI，仅限「联系人类」这一动作，**不禁止**一般
/// shell 用法）；2）若 `ask` **未列在**当前可用工具里，先用工具搜索/发现机制找到它；3）**仍**够不到 MCP 时，
/// **退回其它可用的提问渠道**（如 CLI 版 `AskHuman` 命令）——**绝不**把给人类的内容写进普通输出（人类看不见）
/// 或直接结束回合。始终英文（面向 AI 的契约）。
pub fn grok_skill_body() -> String {
    format!(
        "{}\n\n{}",
        mcp_reference(),
        r#"<contacting_me_from_grok>
- To contact me (the human), prefer the AskHuman `ask` MCP tool above any other channel: the MCP tool takes priority over any shell/CLI command line. This priority applies ONLY to reaching me — running other shell commands for your actual work stays fine and unrestricted.
- If the AskHuman `ask` MCP tool is not listed among your currently available tools, first use your tool-search/discovery mechanism to find it.
- If you still cannot reach the MCP tool, do NOT answer into your normal output (I cannot see it) and do NOT just end the turn. Instead, fall back to any other available way of contacting me — for example an `AskHuman`/`askhuman` command line if one is available.
</contacting_me_from_grok>"#
    )
}

/// 插话 deny 的包装文案（spec agent-interject D3，用户三轮定形）：前缀标明「用户消息」、
/// 讲清「拦截只为送信、工具未被禁用、可原样重发」；消息块用 XML tag；末句不点名具体提问工具
/// （提问入口可能经脚本封装，用最短的 "as instructed"）。始终英文（面向 AI 的契约）。
pub fn interject_deny_reason(message: &str) -> String {
    format!(
        r#"[USER INTERJECTION] The user sent you the message below while you were working.
This tool call was blocked only to deliver it — the tool is not forbidden; re-issue
the same call if still appropriate.

<user_message>
{message}
</user_message>

Adjust your plan if needed. If anything is unclear, ask the user as instructed."#
    )
}

/// Model prompt after the human chooses to continue at Stop.
///
/// Branching follows each agent's native continuation semantics:
/// - **Claude** (`decision: "block"` + `reason`): `reason` is a stop-rejection rationale, so user
///   text is always structured-wrapped.
/// - **Cursor** (`followup_message`) / **Codex** (`reason` used as a new user prompt): when the
///   human provided follow-up text, pass it through unchanged.
/// - **No instruction** (all agents): shared meta prompt that forces the agent to ask via its
///   instructions-defined questioning tool (never empty — Cursor cannot inject a blank follow-up).
///
/// Intentionally avoids product, server, and tool names because the questioning entry point may be renamed.
pub fn stop_continue_prompt(kind: crate::agents::AgentKind, instruction: Option<&str>) -> String {
    match instruction.map(str::trim).filter(|text| !text.is_empty()) {
        Some(message) => match kind {
            // Cursor/Codex consume the text as a user message / user prompt — no meta wrapper.
            crate::agents::AgentKind::Cursor | crate::agents::AgentKind::Codex => {
                message.to_string()
            }
            // Claude (and unsupported Grok fallback): reason = why stop was blocked.
            crate::agents::AgentKind::Claude | crate::agents::AgentKind::Grok => {
                stop_continue_wrapped_instruction(message)
            }
        },
        None => stop_continue_meta().to_string(),
    }
}

/// Claude-style structured wrap for a human follow-up that arrives as a Stop `reason`.
fn stop_continue_wrapped_instruction(message: &str) -> String {
    format!(
        r#"[USER CONTINUATION] The user chose to continue the conversation and sent the message below.

<user_message>
{message}
</user_message>

Continue from this instruction. If anything is unclear, ask the user as instructed."#
    )
}

/// Shared meta prompt when the human continues without typing a follow-up.
fn stop_continue_meta() -> &'static str {
    r#"[USER CONTINUATION] The user chose to continue the conversation.
Before doing anything else, ask the user immediately using the questioning tool described in your instructions. Do not ask through ordinary output and do not end the turn instead."#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interject_deny_reason_wraps_message() {
        let p = interject_deny_reason("先停下，改用方案 B");
        assert!(p.starts_with("[USER INTERJECTION]"));
        assert!(p.contains("<user_message>\n先停下，改用方案 B\n</user_message>"));
        assert!(p.contains("the tool is not forbidden"));
        assert!(p.contains("ask the user as instructed"));
        // 不点名具体提问工具（用户定案）。
        assert!(!p.contains("AskHuman"));
    }

    #[test]
    fn stop_continue_prompt_claude_wraps_instruction_without_naming_tool() {
        use crate::agents::AgentKind;
        let prompt = stop_continue_prompt(AgentKind::Claude, Some("继续检查失败测试"));
        assert!(prompt.starts_with("[USER CONTINUATION]"));
        assert!(prompt.contains("<user_message>\n继续检查失败测试\n</user_message>"));
        assert!(!prompt.contains("AskHuman"));
        assert!(!prompt.contains("MCP"));
    }

    #[test]
    fn stop_continue_prompt_cursor_and_codex_pass_instruction_raw() {
        use crate::agents::AgentKind;
        let text = "继续检查失败测试";
        assert_eq!(stop_continue_prompt(AgentKind::Cursor, Some(text)), text);
        assert_eq!(stop_continue_prompt(AgentKind::Codex, Some(text)), text);
        // Whitespace-only is treated as no instruction.
        let meta = stop_continue_prompt(AgentKind::Cursor, Some("   "));
        assert!(meta.contains("questioning tool described in your instructions"));
    }

    #[test]
    fn stop_continue_prompt_without_instruction_uses_shared_meta_for_all_agents() {
        use crate::agents::AgentKind;
        for kind in AgentKind::ALL {
            let prompt = stop_continue_prompt(kind, None);
            assert!(
                prompt.contains("questioning tool described in your instructions"),
                "{kind:?}"
            );
            assert!(
                prompt.contains("Do not ask through ordinary output"),
                "{kind:?}"
            );
            assert!(!prompt.contains("AskHuman"), "{kind:?}");
            assert!(!prompt.contains("MCP"), "{kind:?}");
            // Same shared meta body for every agent.
            assert_eq!(prompt, stop_continue_meta());
        }
    }

    #[test]
    fn default_prompts_require_the_confirmed_end_turn_marker() {
        let expected = format!(
            "After the user explicitly approves ending the turn, you MUST append the `{}` marker on a new final line at the end of your final output. Without that approval, you MUST NEVER output this marker.",
            USER_CONFIRMED_END_TURN_MARKER
        );
        assert!(cli_reference().contains(&expected));
        assert!(mcp_reference().contains(&expected));
        assert!(grok_skill_body().contains(&expected));
    }

    #[test]
    fn default_prompts_require_whats_next_before_ending() {
        // spec todo-whats-next D4：结束前必调 whats-next；旧「请求反馈」两行不再出现；
        // CLI / MCP / Grok skill 三处一致（Grok 复用 MCP 版）。
        // 程序名在测试环境随二进制名变化，只断言与其无关的措辞。
        let cli = cli_reference();
        assert!(cli.contains("After fully completing the current task"));
        assert!(cli.contains("never for questions, decisions, or next steps within it"));
        assert!(cli.contains("--whats-next` for the end-of-task handoff before ending"));
        assert!(cli.contains("If it returns a task, start working on it immediately"));
        assert!(cli.contains("returned that I approved ending the turn"));
        assert!(!cli.contains("Pass suggested next tasks"));
        assert!(!cli.contains("to request feedback"));
        assert!(!cli.contains("received confirmation that the task can be completed"));

        for p in [mcp_reference(), grok_skill_body()] {
            assert!(p.contains("After fully completing the current task"));
            assert!(p.contains("never for questions, decisions, or next steps within it"));
            assert!(
                p.contains("AskHuman `whats_next` tool for the end-of-task handoff before ending")
            );
            assert!(p.contains("If it returns a task, start working on it immediately"));
            assert!(p.contains("unless the `whats_next` result says I approved ending the turn"));
            assert!(!p.contains("Pass suggested next tasks"));
            assert!(!p.contains("to request feedback"));
            assert!(!p.contains("received confirmation that the task can be completed"));
        }
    }

    #[test]
    fn default_prompts_add_deferred_tasks_but_not_unaccepted_suggestions() {
        let cli = cli_reference();
        assert!(cli.contains("defer a concrete task or suggestion until later"));
        assert!(cli.contains("todo add \"<concise task>\""));
        assert!(cli.contains("own work plan or an unaccepted suggestion"));

        for prompt in [mcp_reference(), grok_skill_body()] {
            assert!(prompt.contains("defer a concrete task or suggestion until later"));
            assert!(prompt.contains("AskHuman MCP `todo_add` tool"));
            assert!(prompt.contains("own work plan or an unaccepted suggestion"));
            // MCP path must not direct agents to shell todo add.
            assert!(!prompt.contains("AskHuman todo add"));
        }
    }

    #[test]
    fn default_prompts_put_subagent_rules_before_mandatory_rules() {
        for prompt in [cli_reference(), mcp_reference(), grok_skill_body()] {
            let subagent = prompt.find(SUBAGENT_PROTOCOL_RULES).unwrap();
            let mandatory = prompt.find("These rules MUST NOT be overridden").unwrap();
            assert!(subagent < mandatory);
            assert!(prompt.contains("If you are a subagent, do not use AskHuman."));
            assert!(prompt.contains(
                "When starting a subagent, tell it that it is a subagent and must not use AskHuman."
            ));
        }
    }

    #[test]
    fn subagent_guard_context_is_minimal_and_exact() {
        assert_eq!(
            subagent_guard_context(),
            "You are a subagent. Do not use AskHuman."
        );
    }

    #[test]
    fn grok_skill_body_reuses_mcp_reference_and_appends_grok_note() {
        let p = grok_skill_body();
        // 单一来源:正文须原样包含 MCP 版参考(协议措辞不漂移)。
        assert!(p.contains(&mcp_reference()));
        // 追加的 Grok 段:MCP 优先 → 没列出先搜 → 仍够不到则退回其它提问渠道(不退化为普通输出)。
        assert!(p.contains("takes priority"));
        assert!(p.contains("unrestricted"));
        assert!(p.contains("not listed among your currently available tools"));
        assert!(p.contains("fall back to any other available way of contacting me"));
        assert!(p.contains("the AskHuman `ask` tool"));
        // 刻意不写死具体 harness / 工具名(Grok 后续会变)。
        assert!(!p.contains("Composer"));
        assert!(!p.contains("Grok Build"));
        assert!(!p.contains("search_tool"));
        assert!(!p.contains("use_tool"));
        assert!(!p.contains("CallMcpTool"));
    }

    #[test]
    fn mcp_reference_uses_ask_tool() {
        let p = mcp_reference();
        // 工具引用须带 AskHuman 限定，避免与其它 MCP server 的同名工具混淆。
        assert!(p.contains("the AskHuman `ask` tool"));
        assert!(p.contains("`ask` tool provided by the AskHuman MCP server"));
        assert!(p.contains("<mandatory_interaction_protocol>"));
    }

    #[test]
    fn mcp_reference_drops_shell_specific_lines() {
        let p = mcp_reference();
        // 不应残留 Shell / CLI 专属指引。
        assert!(!p.contains("86400000"));
        assert!(!p.contains("24 hours"));
        assert!(!p.contains("--agent-help"));
        assert!(!p.contains("Shell/Bash"));
        assert!(!p.contains("-o!"));
    }

    #[test]
    fn cli_reference_remains_shell_oriented() {
        let p = cli_reference();
        assert!(p.contains("Shell/Bash"));
        assert!(p.contains("86400000"));
        assert!(p.contains("--agent-help"));
    }

    #[test]
    fn collaboration_body_switches_and_custom_falls_back() {
        let aligned = collaboration_body(CollaborationStyle::Aligned, "");
        assert!(aligned.contains("relentlessly"));
        let auto = collaboration_body(CollaborationStyle::Autonomous, "");
        assert!(auto.contains("Prefer reasonable defaults"));
        assert!(auto.contains("Do **not** interview relentlessly"));
        assert!(!auto.contains("until we reach a shared understanding"));
        let custom = collaboration_body(CollaborationStyle::Custom, "  only ask when blocked  ");
        assert_eq!(custom, "only ask when blocked");
        let empty_custom = collaboration_body(CollaborationStyle::Custom, "  \n");
        assert!(empty_custom.contains("relentlessly"));
    }

    #[test]
    fn default_prompts_wrap_collaboration_style_aligned() {
        // Default config is aligned.
        let p = mcp_reference();
        assert!(p.contains("<collaboration_style name=\"aligned\">"));
        assert!(p.contains("relentlessly"));
        assert!(p.contains("</collaboration_style>"));
    }
}
