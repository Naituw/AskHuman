//! 三态模式编排：把每家 Agent 的「Rule +（超时 Hook | MCP 配置）」聚合为 **None / Cli / Mcp** 三态互斥，
//! 供设置 UI 与 `agents`/`doctor` CLI 复用。
//!
//! - **Cli** 模式绑定：CLI 版 Rule + 超时 Hook（Codex 无超时 Hook，仅 Rule）。
//! - **Mcp** 模式绑定：MCP 版 Rule + MCP 配置（用户级全局）。
//! - 一键切换（[`set`]）：先卸掉「非目标模式」的全部产物，再装目标模式产物；天然幂等。
//!
//! 注意：实验性 lifecycle hook（turn 追踪）**不属于**任何模式，保持独立开关、与本编排正交（spec D9）。

use crate::integrations::agent_rules::{self, AgentTarget, Variant};
use crate::integrations::{claude_hook, cursor_hook, mcp_config};
use anyhow::Result;

/// 每家 Agent 的集成模式（互斥三态）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    None,
    Cli,
    Mcp,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::None => "none",
            Mode::Cli => "cli",
            Mode::Mcp => "mcp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Mode::None),
            "cli" => Some(Mode::Cli),
            "mcp" => Some(Mode::Mcp),
            _ => None,
        }
    }
}

// MARK: - 超时 Hook 分派（Codex 无超时 Hook）

/// 该 Agent 是否有「超时 Hook」概念（Codex 没有）。
pub fn timeout_hook_supported(target: AgentTarget) -> bool {
    match target {
        AgentTarget::Cursor => cursor_hook::supported(),
        AgentTarget::ClaudeCode => claude_hook::supported(),
        AgentTarget::Codex => false,
    }
}

/// 超时 Hook 是否已安装（Codex 恒 false）。
pub fn timeout_hook_is_installed(target: AgentTarget) -> bool {
    match target {
        AgentTarget::Cursor => cursor_hook::is_installed(),
        AgentTarget::ClaudeCode => claude_hook::is_installed(),
        AgentTarget::Codex => false,
    }
}

/// 超时 Hook 是否需更新（Codex 恒 false）。
pub fn timeout_hook_needs_update(target: AgentTarget) -> bool {
    match target {
        AgentTarget::Cursor => cursor_hook::needs_update(),
        AgentTarget::ClaudeCode => claude_hook::needs_update(),
        AgentTarget::Codex => false,
    }
}

fn timeout_hook_install(target: AgentTarget) -> Result<()> {
    match target {
        AgentTarget::Cursor => cursor_hook::install().map(|_| ()),
        AgentTarget::ClaudeCode => claude_hook::install().map(|_| ()),
        AgentTarget::Codex => Ok(()),
    }
}

fn timeout_hook_uninstall(target: AgentTarget) -> Result<()> {
    match target {
        AgentTarget::Cursor => cursor_hook::uninstall().map(|_| ()),
        AgentTarget::ClaudeCode => claude_hook::uninstall().map(|_| ()),
        AgentTarget::Codex => Ok(()),
    }
}

/// 在文件管理器中定位超时 Hook 的配置文件（Codex 无 Hook，no-op）。
pub fn timeout_hook_reveal(target: AgentTarget) {
    match target {
        AgentTarget::Cursor => cursor_hook::reveal(),
        AgentTarget::ClaudeCode => claude_hook::reveal(),
        AgentTarget::Codex => {}
    }
}

/// 用系统默认程序打开超时 Hook 的配置文件（Codex 无 Hook，no-op）。
pub fn timeout_hook_open(target: AgentTarget) {
    match target {
        AgentTarget::Cursor => cursor_hook::open(),
        AgentTarget::ClaudeCode => claude_hook::open(),
        AgentTarget::Codex => {}
    }
}

// MARK: - 状态

/// 当前模式：**以产物（MCP 配置 / 超时 Hook）为首要信号**，产物不明确时再回退到 Rule 正文变体。
///
/// 之所以产物优先：MCP 配置与超时 Hook 由 [`set`] 维护、彼此互斥，是稳定的模式标识；而 Rule 正文会随
/// 内置提示词版本演进而漂移，若以「正文是否精确等于当前 `mcp_reference()`」判定，一旦更新提示词，已装的
/// 旧正文就会失配并被错判成 CLI（曾导致「装了 MCP、改版后却显示 CLI 且提示需更新」的 bug）。
pub fn current(target: AgentTarget) -> Mode {
    let mcp = mcp_config::is_installed(target);
    let hook = timeout_hook_is_installed(target);
    match (mcp, hook) {
        (true, false) => return Mode::Mcp,
        (false, true) => return Mode::Cli,
        // 产物全无（如 Codex 的 CLI 模式：无超时 Hook 产物）或都有（用户手改）→ 以 Rule 变体兜底。
        _ => {}
    }
    match agent_rules::installed_variant(target) {
        Some(Variant::Mcp) => Mode::Mcp,
        Some(Variant::Cli) => Mode::Cli,
        None => Mode::None,
    }
}

/// 当前模式下的某类产物（供「按产物」过期判断与单项更新）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Artifact {
    Rule,
    Hook,
    Mcp,
}

impl Artifact {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rule" => Some(Artifact::Rule),
            "hook" => Some(Artifact::Hook),
            "mcp" => Some(Artifact::Mcp),
            _ => None,
        }
    }
}

/// 当前模式下各产物是否过期 / 缺失（与 [`needs_update`] 同口径，逐产物拆开供 UI 概览统计与单项更新）。
#[derive(Clone, Copy, Default, Debug)]
pub struct ArtifactUpdates {
    pub rule: bool,
    pub hook: bool,
    pub mcp: bool,
}

/// 逐产物计算当前模式下的过期 / 缺失情况。None 模式无产物，全 false。
pub fn artifact_updates(target: AgentTarget) -> ArtifactUpdates {
    match current(target) {
        Mode::None => ArtifactUpdates::default(),
        Mode::Cli => ArtifactUpdates {
            rule: !agent_rules::is_installed(target)
                || agent_rules::needs_update_variant(target, Variant::Cli),
            hook: timeout_hook_supported(target)
                && (!timeout_hook_is_installed(target) || timeout_hook_needs_update(target)),
            mcp: false,
        },
        Mode::Mcp => ArtifactUpdates {
            rule: !agent_rules::is_installed(target)
                || agent_rules::needs_update_variant(target, Variant::Mcp),
            hook: false,
            mcp: !mcp_config::is_installed(target) || mcp_config::needs_update(target),
        },
    }
}

/// 当前模式下是否有产物过期 / 缺失（含 Rule 漂移、超时 Hook 缺失/过期、MCP 配置缺失/过期）。
pub fn needs_update(target: AgentTarget) -> bool {
    let u = artifact_updates(target);
    u.rule || u.hook || u.mcp
}

// MARK: - 切换

/// 一键切到目标模式：先卸「非目标模式」的全部产物，再装目标模式产物。各底层 install/uninstall 已幂等。
pub fn set(target: AgentTarget, mode: Mode) -> Result<()> {
    match mode {
        Mode::None => uninstall_all(target),
        Mode::Cli => {
            // 卸 MCP 产物 → 装 CLI Rule + 超时 Hook（Codex 跳过 Hook）。
            mcp_config::uninstall(target)?;
            agent_rules::install_variant(target, Variant::Cli)?;
            if timeout_hook_supported(target) {
                timeout_hook_install(target)?;
            }
            Ok(())
        }
        Mode::Mcp => {
            // 卸超时 Hook → 装 MCP Rule + MCP 配置。
            if timeout_hook_supported(target) {
                timeout_hook_uninstall(target)?;
            }
            agent_rules::install_variant(target, Variant::Mcp)?;
            mcp_config::install(target)?;
            Ok(())
        }
    }
}

/// 更新当前模式的全部产物到最新（不切换模式）。当前为 None 时 no-op。
pub fn update(target: AgentTarget) -> Result<()> {
    match current(target) {
        Mode::None => Ok(()),
        Mode::Cli => set(target, Mode::Cli),
        Mode::Mcp => set(target, Mode::Mcp),
    }
}

/// 把当前模式下的**单个产物**刷新到最新（不切换模式、不动其它产物）。各底层 install 均幂等，
/// 故「重装即更新」；与当前模式不相干的产物（如 None、或在 Cli 模式更新 Mcp）为 no-op。
pub fn update_artifact(target: AgentTarget, artifact: Artifact) -> Result<()> {
    match (current(target), artifact) {
        (Mode::Cli, Artifact::Rule) => {
            agent_rules::install_variant(target, Variant::Cli).map(|_| ())
        }
        (Mode::Mcp, Artifact::Rule) => {
            agent_rules::install_variant(target, Variant::Mcp).map(|_| ())
        }
        (Mode::Cli, Artifact::Hook) => {
            if timeout_hook_supported(target) {
                timeout_hook_install(target).map(|_| ())
            } else {
                Ok(())
            }
        }
        (Mode::Mcp, Artifact::Mcp) => mcp_config::install(target).map(|_| ()),
        _ => Ok(()),
    }
}

/// 卸载当前 / 全部模式产物（Rule + 超时 Hook + MCP 配置），保留用户其它内容。
fn uninstall_all(target: AgentTarget) -> Result<()> {
    agent_rules::uninstall(target)?;
    if timeout_hook_supported(target) {
        timeout_hook_uninstall(target)?;
    }
    mcp_config::uninstall(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parse_as_str_roundtrip() {
        for m in [Mode::None, Mode::Cli, Mode::Mcp] {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        assert_eq!(Mode::parse("other"), None);
    }

    #[test]
    fn codex_has_no_timeout_hook() {
        assert!(!timeout_hook_supported(AgentTarget::Codex));
        assert!(!timeout_hook_is_installed(AgentTarget::Codex));
        assert!(!timeout_hook_needs_update(AgentTarget::Codex));
    }
}
