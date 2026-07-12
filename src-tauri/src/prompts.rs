//! CLI 调用参考提示词（供设置界面展示与复制）。
//!
//! 该提示词始终为英文（面向 AI 的契约），且**不内嵌** help 文本，
//! 而是指引 AI 执行 `<prog> --agent-help` 获取实时、随界面语言本地化的用法。

pub const USER_CONFIRMED_END_TURN_MARKER: &str = "[user_confirmed_end_turn]";

/// 组装参考提示词：行为约束规则（英文固定）+ 一行执行指引。
pub fn cli_reference() -> String {
    let program = crate::cli::help::program_name();

    format!(
        r#"<mandatory_interaction_protocol>
**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**
**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by invoking the `{program}` command through the Shell/Bash tool. DO NOT ask questions in direct output, and DO NOT ask by ending the turn.
- When invoking `{program}` through the Shell/Bash tool, you MUST set that tool call's timeout to 24 hours (86400000 ms) to give me enough time to respond.
- Before asking, run `{program} --agent-help` to learn its usage.

- When asking through `{program}`, provide predefined options whenever applicable, mark your recommended option(s) with `-o!`, and briefly explain your rationale.
- I can ONLY see what is delivered through `{program}`. Anything I need to review , or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through `{program}`, inline or attached with `-f`. Never rely on direct output which is invisible to me, and never just give me a path.
- Before completing the turn/request, you MUST call `{program}` to request feedback.
- Do NOT end the turn/conversation or mark the request as complete unless you have explicitly asked via `{program}` and received confirmation that the task can be completed or ended, and that there are no more tasks to do.
- After the user agrees, you MUST append the `{end_marker}` marker on a new final line at the end of your final output.
</mandatory_interaction_protocol>

- Interview me with `{program}` relentlessly about every aspect of the requirements until we reach a shared understanding.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through `{program}` before making the change."#,
        program = program,
        end_marker = USER_CONFIRMED_END_TURN_MARKER,
    )
}

/// MCP 版参考提示词：交互纪律与 CLI 版一致，但工具用法改为「调用 AskHuman MCP server 的 `ask` 工具」。
///
/// 与 [`cli_reference`] 的差异（spec D10）：去掉 Shell 专属的「设 24h 超时」「先跑 `--agent-help`」等句
/// （MCP 工具调用本身可长超时、用法由工具 schema 自带），把「经 Shell 调 `AskHuman`」改为「调用 AskHuman
/// 的 `ask` 工具」。**工具引用须带 AskHuman 限定**——agent 可能挂载多个 MCP server，单说「the `ask`
/// tool」会有歧义，故全文统一为「the AskHuman `ask` tool」并在首句点明它由 AskHuman MCP server 提供。
/// 其余纪律（必须提问、不在直接输出/结束回合提问、提供预定义选项 + 标推荐、附件经工具、结束前回执、
/// relentless interview、不擅自改方案）全部保留。始终英文（面向 AI 的契约）。
pub fn mcp_reference() -> String {
    format!(
        r#"<mandatory_interaction_protocol>
**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**
**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by calling the `ask` tool provided by the AskHuman MCP server (referred to below as the AskHuman `ask` tool). DO NOT ask questions in direct output, and DO NOT ask by ending the turn.
- The AskHuman `ask` tool blocks until I reply, which may take a long time; always wait for its result instead of giving up or proceeding on assumptions.

- When asking through the AskHuman `ask` tool, provide predefined options whenever applicable, mark your recommended option(s) as recommended, and briefly explain your rationale.
- I can ONLY see what is delivered through the AskHuman `ask` tool. Anything I need to review, or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through the AskHuman `ask` tool, inline or attached as files. Never rely on direct output which is invisible to me, and never just give me a path.
- Before completing the turn/request, you MUST call the AskHuman `ask` tool to request feedback.
- Do NOT end the turn/conversation or mark the request as complete unless you have explicitly asked via the AskHuman `ask` tool and received confirmation that the task can be completed or ended, and that there are no more tasks to do.
- After the user agrees, you MUST append the `{end_marker}` marker on a new final line at the end of your final output.
</mandatory_interaction_protocol>

- Interview me with the AskHuman `ask` tool relentlessly about every aspect of the requirements until we reach a shared understanding.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through the AskHuman `ask` tool before making the change."#,
        end_marker = USER_CONFIRMED_END_TURN_MARKER,
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

/// Model prompt after the human chooses to continue at Stop. It intentionally avoids product,
/// server, and tool names because the installed questioning entry point may be renamed.
pub fn stop_continue_prompt(instruction: Option<&str>) -> String {
    match instruction.map(str::trim).filter(|text| !text.is_empty()) {
        Some(message) => format!(
            r#"[USER CONTINUATION] The user chose to continue the conversation and sent the message below.

<user_message>
{message}
</user_message>

Continue from this instruction. If anything is unclear, ask the user as instructed."#
        ),
        None => r#"[USER CONTINUATION] The user chose to continue the conversation.
Before doing anything else, ask the user immediately using the questioning tool described in your instructions. Do not ask through ordinary output and do not end the turn instead."#
            .to_string(),
    }
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
    fn stop_continue_prompt_wraps_instruction_without_naming_tool() {
        let prompt = stop_continue_prompt(Some("继续检查失败测试"));
        assert!(prompt.starts_with("[USER CONTINUATION]"));
        assert!(prompt.contains("<user_message>\n继续检查失败测试\n</user_message>"));
        assert!(!prompt.contains("AskHuman"));
        assert!(!prompt.contains("MCP"));
    }

    #[test]
    fn stop_continue_prompt_without_instruction_uses_instructions_tool() {
        let prompt = stop_continue_prompt(None);
        assert!(prompt.contains("questioning tool described in your instructions"));
        assert!(prompt.contains("Do not ask through ordinary output"));
        assert!(!prompt.contains("AskHuman"));
        assert!(!prompt.contains("MCP"));
    }

    #[test]
    fn default_prompts_require_the_confirmed_end_turn_marker() {
        let expected = format!(
            "After the user agrees, you MUST append the `{}` marker on a new final line at the end of your final output.",
            USER_CONFIRMED_END_TURN_MARKER
        );
        assert!(cli_reference().contains(&expected));
        assert!(mcp_reference().contains(&expected));
        assert!(grok_skill_body().contains(&expected));
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
}
