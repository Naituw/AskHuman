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

- When asking through `{program}`, provide predefined options whenever applicable, mark your recommended option(s) with `-o!` (instead of writing "recommended" in the option text), and briefly explain your rationale.
- I can ONLY see what is delivered through `{program}`. Anything I need to review , or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through `{program}`, inline or attached with `-f`. Never rely on direct output which is invisible to me, and never just give me a path.
- Before completing the turn/request, you MUST call `{program}` to request feedback.
- Do NOT end the turn/conversation or mark the request as complete unless you have explicitly asked via `{program}` and received confirmation that the task can be completed or ended.
</mandatory_interaction_protocol>

- Interview me with `{program}` relentlessly about every aspect of the requirements until we reach a shared understanding.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through `{program}` before making the change."#,
        program = program,
    )
}