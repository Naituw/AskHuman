//! CLI 调用参考提示词（供设置界面展示与复制）。
//!
//! 该提示词始终为英文（面向 AI 的契约），且**不内嵌** help 文本，
//! 而是指引 AI 执行 `<prog> --agent-help` 获取实时、随界面语言本地化的用法。

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
</mandatory_interaction_protocol>

- Interview me with `{program}` relentlessly about every aspect of the requirements until we reach a shared understanding.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through `{program}` before making the change."#,
        program = program,
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
    r#"<mandatory_interaction_protocol>
**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**
**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by calling the `ask` tool provided by the AskHuman MCP server (referred to below as the AskHuman `ask` tool). DO NOT ask questions in direct output, and DO NOT ask by ending the turn.
- The AskHuman `ask` tool blocks until I reply, which may take a long time; always wait for its result instead of giving up or proceeding on assumptions.

- When asking through the AskHuman `ask` tool, provide predefined options whenever applicable, mark your recommended option(s) as recommended, and briefly explain your rationale.
- I can ONLY see what is delivered through the AskHuman `ask` tool. Anything I need to review, or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through the AskHuman `ask` tool, inline or attached as files. Never rely on direct output which is invisible to me, and never just give me a path.
- Before completing the turn/request, you MUST call the AskHuman `ask` tool to request feedback.
- Do NOT end the turn/conversation or mark the request as complete unless you have explicitly asked via the AskHuman `ask` tool and received confirmation that the task can be completed or ended, and that there are no more tasks to do.
</mandatory_interaction_protocol>

- Interview me with the AskHuman `ask` tool relentlessly about every aspect of the requirements until we reach a shared understanding.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through the AskHuman `ask` tool before making the change."#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
