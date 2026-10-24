//! 三态模式编排：把每家 Agent 的「Rule + Subagent Guard +（超时 Hook | MCP 配置）」聚合为
//! **None / Cli / Mcp** 三态互斥，
//! 供设置 UI 与 `agents`/`doctor` CLI 复用。
//!
//! - **Cli** 模式绑定：CLI 版 Rule + Guard + 超时 Hook（Codex 无超时 Hook）。
//! - **Mcp** 模式绑定：MCP 版 Rule + Guard + MCP 配置（用户级全局）。
//! - Mode changes reconcile the target bundle idempotently. Antigravity prepares config first
//!   and conditionally restores it if its existing rule journal restores the previous rule.
//!
//! Lifecycle tracking is an optional capability owned by an active automatic integration. New
//! integrations adopt the default-on preference; explicit opt-out survives mode updates. None
//! mode always removes the on-disk lifecycle artifact.

use crate::integrations::agent_rules::{self, AgentTarget, Variant};
use crate::integrations::{
    agent_ask_question, agent_context_recovery, agent_lifecycle, agent_permission, agent_stop,
    agent_subagent_guard, claude_hook, cursor_hook, mcp_config, mutation_lock,
};
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

/// Whether this agent has a managed timeout runtime artifact. Pi uses an Extension rather than a
/// native hook, but keeps this compatibility surface while the UI wire format is migrated.
pub fn timeout_hook_supported(target: AgentTarget) -> bool {
    match target {
        AgentTarget::Cursor => cursor_hook::supported(),
        AgentTarget::ClaudeCode => claude_hook::supported(),
        AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => false,
        AgentTarget::Pi => true,
    }
}

/// 超时 Hook 是否已安装（Codex / Grok 恒 false）。
pub fn timeout_hook_is_installed(target: AgentTarget) -> bool {
    match target {
        AgentTarget::Cursor => cursor_hook::is_installed(),
        AgentTarget::ClaudeCode => claude_hook::is_installed(),
        AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => false,
        AgentTarget::Pi => super::pi_extension::status().cli,
    }
}

/// 超时 Hook 是否需更新（Codex / Grok 恒 false）。
pub fn timeout_hook_needs_update(target: AgentTarget) -> bool {
    match target {
        AgentTarget::Cursor => cursor_hook::needs_update(),
        AgentTarget::ClaudeCode => claude_hook::needs_update(),
        AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => false,
        AgentTarget::Pi => {
            let status = super::pi_extension::status();
            status.cli && status.outdated
        }
    }
}

fn timeout_hook_install(target: AgentTarget) -> Result<()> {
    match target {
        AgentTarget::Cursor => cursor_hook::install().map(|_| ()),
        AgentTarget::ClaudeCode => claude_hook::install().map(|_| ()),
        AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => Ok(()),
        AgentTarget::Pi => super::pi_extension::set_cli_enabled(true),
    }
}

fn timeout_hook_uninstall(target: AgentTarget) -> Result<()> {
    match target {
        AgentTarget::Cursor => cursor_hook::uninstall().map(|_| ()),
        AgentTarget::ClaudeCode => claude_hook::uninstall().map(|_| ()),
        AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => Ok(()),
        AgentTarget::Pi => super::pi_extension::set_cli_enabled(false),
    }
}

/// 在文件管理器中定位超时 Hook 的配置文件（Codex / Grok 无 Hook，no-op）。
pub fn timeout_hook_reveal(target: AgentTarget) {
    match target {
        AgentTarget::Cursor => cursor_hook::reveal(),
        AgentTarget::ClaudeCode => claude_hook::reveal(),
        AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => {}
        AgentTarget::Pi => super::pi_extension::reveal(),
    }
}

/// 用系统默认程序打开超时 Hook 的配置文件（Codex / Grok 无 Hook，no-op）。
pub fn timeout_hook_open(target: AgentTarget) {
    match target {
        AgentTarget::Cursor => cursor_hook::open(),
        AgentTarget::ClaudeCode => claude_hook::open(),
        AgentTarget::Codex | AgentTarget::Grok | AgentTarget::Antigravity => {}
        AgentTarget::Pi => super::pi_extension::open(),
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
    current_from_artifacts(mcp, hook, agent_rules::installed_variant(target))
}

fn current_from_artifacts(mcp: bool, runtime: bool, rule: Option<Variant>) -> Mode {
    match (mcp, runtime) {
        (true, false) => Mode::Mcp,
        (false, true) => Mode::Cli,
        // Runtime artifacts are absent (for example Codex CLI) or conflict after a manual edit;
        // fall back to the managed Rule variant. Lifecycle is intentionally not an input.
        _ => match rule {
            Some(Variant::Mcp) => Mode::Mcp,
            Some(Variant::Cli) => Mode::Cli,
            None => Mode::None,
        },
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
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ArtifactUpdates {
    pub rule: bool,
    pub hook: bool,
    pub mcp: bool,
}

#[derive(Clone, Copy, Default)]
struct ArtifactState {
    rule_installed: bool,
    rule_outdated: bool,
    guard_outdated: bool,
    timeout_supported: bool,
    timeout_installed: bool,
    timeout_outdated: bool,
    permission_outdated: bool,
    recovery_outdated: bool,
    mcp_installed: bool,
    mcp_outdated: bool,
}

/// 逐产物计算当前模式下的过期 / 缺失情况。None 模式仅报告需要清理的残留 Permission Hook。
pub fn artifact_updates(target: AgentTarget) -> ArtifactUpdates {
    let mode = current(target);
    let updates = artifact_updates_for(
        mode,
        ArtifactState {
            rule_installed: agent_rules::is_installed(target),
            rule_outdated: match mode {
                Mode::Cli => agent_rules::needs_update_variant(target, Variant::Cli),
                Mode::Mcp => agent_rules::needs_update_variant(target, Variant::Mcp),
                Mode::None => false,
            },
            guard_outdated: agent_subagent_guard::needs_update(target, mode),
            timeout_supported: timeout_hook_supported(target),
            timeout_installed: timeout_hook_is_installed(target),
            timeout_outdated: timeout_hook_needs_update(target),
            permission_outdated: permission_needs_reconcile(target, mode),
            recovery_outdated: agent_context_recovery::needs_update(target, mode),
            mcp_installed: mcp_config::is_installed(target),
            mcp_outdated: mcp_config::needs_update(target),
        },
    );
    merge_lifecycle_update(
        updates,
        agent_lifecycle::status_for_mode(kind_for_target(target), mode).needs_update,
    )
}

fn merge_lifecycle_update(
    mut updates: ArtifactUpdates,
    lifecycle_needs_update: bool,
) -> ArtifactUpdates {
    updates.hook |= lifecycle_needs_update;
    updates
}

fn artifact_updates_for(mode: Mode, state: ArtifactState) -> ArtifactUpdates {
    match mode {
        Mode::None => ArtifactUpdates {
            hook: state.permission_outdated || state.recovery_outdated,
            ..ArtifactUpdates::default()
        },
        Mode::Cli => ArtifactUpdates {
            rule: !state.rule_installed || state.rule_outdated || state.guard_outdated,
            hook: (state.timeout_supported && (!state.timeout_installed || state.timeout_outdated))
                || state.permission_outdated
                || state.recovery_outdated,
            mcp: false,
        },
        Mode::Mcp => ArtifactUpdates {
            rule: !state.rule_installed || state.rule_outdated || state.guard_outdated,
            hook: state.permission_outdated,
            mcp: !state.mcp_installed || state.mcp_outdated || state.recovery_outdated,
        },
    }
}

fn permission_needs_reconcile(target: AgentTarget, _mode: Mode) -> bool {
    let status = agent_permission::status(target);
    if !status.supported {
        return false;
    }
    status.needs_update
}

/// 当前模式下是否有产物过期 / 缺失（含 Rule / Guard、超时 Hook 与 MCP 配置）。
pub fn needs_update(target: AgentTarget) -> bool {
    let u = artifact_updates(target);
    u.rule || u.hook || u.mcp
}

// MARK: - 切换

/// 一键设为目标模式并完整 reconcile 该模式的托管产物。重复设置同一 mode 也会更新磁盘；
/// Permission 是否安装只读取其独立 preference，本操作绝不改写该 preference。
///
/// Grok 只提供 `None | Mcp` 两态（Composer 的 CLI 会自动后台化、不可靠，见调研）：请求 `Cli` 直接报错，
/// 避免留下「装了 skill 却没 MCP 配置」的半残状态。Grok 的 `Mcp` 产物 = skill（经 `agent_rules` 委托）+
/// MCP 配置，正好复用下方 Mcp 分支（无超时 Hook）。
pub fn set(target: AgentTarget, mode: Mode) -> Result<()> {
    set_with_env(target, mode, &std::collections::BTreeMap::new())
}

pub(crate) fn set_with_env(
    target: AgentTarget,
    mode: Mode,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    if !env.is_empty() && (target != AgentTarget::Codex || mode != Mode::Mcp) {
        return Err(anyhow::anyhow!("environment overrides require Codex MCP"));
    }

    for (name, value) in env {
        if !mcp_config::valid_env_name(name) || value.contains('\0') {
            return Err(anyhow::anyhow!("invalid environment override"));
        }
    }
    if target == AgentTarget::Grok && mode == Mode::Cli {
        return Err(anyhow::anyhow!(
            "Grok only supports None | Mcp (no CLI mode)"
        ));
    }
    if target == AgentTarget::Pi && mode == Mode::Mcp {
        return Err(anyhow::anyhow!("Pi only supports None | Cli (no MCP mode)"));
    }
    let lock = mutation_lock::IntegrationMutationLock::acquire()?;
    set_unlocked_with_env(target, mode, env)?;
    drop(lock);
    if target == AgentTarget::Cursor {
        crate::cursor_desktop::integration_changed()?;
    }
    Ok(())
}

fn set_unlocked_with_env(
    target: AgentTarget,
    mode: Mode,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    if target == AgentTarget::Antigravity {
        let config = mcp_config::prepare_antigravity_config(mode == Mode::Mcp)?;
        let variant = if mode == Mode::Mcp {
            Variant::Mcp
        } else {
            Variant::Cli
        };
        let context = agent_rules::antigravity_context_for_variant(variant);
        return set_antigravity_prepared(&context, &config, mode);
    }
    match mode {
        Mode::None => uninstall_all_unlocked(target),
        Mode::Cli => {
            // 卸 MCP 产物 → 装 CLI Rule + 超时 Hook（Codex 跳过 Hook）。
            mcp_config::uninstall(target)?;
            agent_rules::install_variant_unlocked(target, Variant::Cli)?;
            agent_subagent_guard::reconcile_unlocked(target, mode)?;
            if timeout_hook_supported(target) {
                timeout_hook_install(target)?;
            }
            agent_context_recovery::reconcile_unlocked(target, mode)?;
            agent_lifecycle::reconcile_unlocked(kind_for_target(target), mode, true)?;
            agent_permission::reconcile_unlocked(target, mode)?;
            agent_stop::reconcile_unlocked(kind_for_target(target), mode)?;
            agent_ask_question::reconcile_unlocked(kind_for_target(target), mode)?;
            Ok(())
        }
        Mode::Mcp => {
            // 卸超时 Hook → 装 MCP Rule + MCP 配置。
            if timeout_hook_supported(target) {
                timeout_hook_uninstall(target)?;
            }
            agent_rules::install_variant_unlocked(target, Variant::Mcp)?;
            agent_subagent_guard::reconcile_unlocked(target, mode)?;
            mcp_config::install_with_env(target, env)?;
            agent_context_recovery::reconcile_unlocked(target, mode)?;
            agent_lifecycle::reconcile_unlocked(kind_for_target(target), mode, true)?;
            agent_permission::reconcile_unlocked(target, mode)?;
            agent_stop::reconcile_unlocked(kind_for_target(target), mode)?;
            agent_ask_question::reconcile_unlocked(kind_for_target(target), mode)?;
            Ok(())
        }
    }
}

fn same_logical_rule(
    before: &super::antigravity_rule::RuleSnapshot,
    after: &super::antigravity_rule::RuleSnapshot,
) -> bool {
    use super::antigravity_rule::RuleState;
    if matches!(before.state, RuleState::Conflict | RuleState::NeedsRepair)
        || matches!(after.state, RuleState::Conflict | RuleState::NeedsRepair)
        || (before.state == RuleState::Missing) != (after.state == RuleState::Missing)
    {
        return false;
    }
    match (&before.ownership, &after.ownership) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.path == b.path
                && a.generation_version == b.generation_version
                && a.variant == b.variant
                && a.sha256 == b.sha256
        }
        _ => false,
    }
}

#[cfg(test)]
thread_local! {
    static BEFORE_ANTIGRAVITY_CONFIG_RESTORE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

fn set_antigravity_prepared(
    context: &super::antigravity_rule::RuleContext,
    config: &mcp_config::PreparedAntigravityConfig,
    mode: Mode,
) -> Result<()> {
    use super::antigravity_rule::{self, RuleState};
    let retry = format!(
        "retry explicitly: AskHuman agents mode antigravity {}",
        mode.as_str()
    );
    antigravity_rule::recover_unlocked(context).map_err(|error| anyhow::anyhow!(
        "Antigravity rule recovery failed: {error:#}; rule {}; ledger {}; MCP config {} unchanged; {retry}",
        context.target_path.display(), context.integration_state_file.display(), config.path().display()
    ))?;
    let before = antigravity_rule::probe(context)?;
    if matches!(before.state, RuleState::Conflict | RuleState::NeedsRepair) {
        anyhow::bail!(
            "cannot confirm shared Antigravity rule {} using current instance ledger {}: {}; MCP config unchanged",
            context.target_path.display(), context.integration_state_file.display(),
            before.reason.as_deref().unwrap_or("rule needs repair")
        );
    }
    config.apply().map_err(|error| {
        anyhow::anyhow!(
            "MCP config publication failed at {}: {error:#}; rule unchanged; {retry}",
            config.path().display()
        )
    })?;
    let operation = match mode {
        Mode::None => antigravity_rule::uninstall_unlocked(context),
        Mode::Cli | Mode::Mcp => antigravity_rule::install_unlocked(context).map(|_| ()),
    };
    if let Err(error) = operation {
        let recovered = antigravity_rule::recover_unlocked(context)
            .and_then(|_| antigravity_rule::probe(context));
        let outcome = match recovered {
            Ok(after) if same_logical_rule(&before, &after) => {
                #[cfg(test)]
                BEFORE_ANTIGRAVITY_CONFIG_RESTORE.with(|slot| {
                    let hook = slot.borrow_mut().take();
                    if let Some(hook) = hook {
                        hook();
                    }
                });
                match config.restore() {
                    Ok(()) => "prior rule and MCP config restored".to_string(),
                    Err(restore) => format!(
                        "prior rule restored, but MCP config restoration refused/failed: {restore:#}"
                    ),
                }
            }
            Ok(_) => {
                "rule changed; preserving published MCP config; inspect current state".to_string()
            }
            Err(recovery) => {
                format!("rule recovery uncertain; preserving published MCP config: {recovery:#}")
            }
        };
        anyhow::bail!(
            "Antigravity mode {} failed: {error:#}; {outcome}; rule {}; ledger {}; MCP config {}; {retry}",
            mode.as_str(), context.target_path.display(), context.integration_state_file.display(), config.path().display()
        );
    }
    Ok(())
}

/// 更新当前模式的全部产物到最新（不切换模式）。None 有残留时清理，clean None 幂等 no-op。
pub fn update(target: AgentTarget) -> Result<()> {
    agent_rules::validate_status(target)?;
    set(target, current(target))
}

/// 把当前模式下的**单个产物**刷新到最新（不切换模式、不动其它产物）。各底层 install 均幂等，
/// 故「重装即更新」；与当前模式不相干的产物（如 None、或在 Cli 模式更新 Mcp）为 no-op。
pub fn update_artifact(target: AgentTarget, artifact: Artifact) -> Result<()> {
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    agent_rules::validate_status(target)?;
    let mode = current(target);
    match (mode, artifact) {
        (Mode::Cli, Artifact::Rule) => {
            agent_rules::install_variant_unlocked(target, Variant::Cli)?;
            agent_subagent_guard::reconcile_unlocked(target, mode)
        }
        (Mode::Mcp, Artifact::Rule) => {
            agent_rules::install_variant_unlocked(target, Variant::Mcp)?;
            agent_subagent_guard::reconcile_unlocked(target, mode)
        }
        (Mode::Cli, Artifact::Hook) => {
            if timeout_hook_supported(target) {
                timeout_hook_install(target)?;
            }
            agent_context_recovery::reconcile_unlocked(target, mode)?;
            agent_lifecycle::reconcile_unlocked(kind_for_target(target), mode, true)?;
            agent_permission::reconcile_unlocked(target, mode)
        }
        (Mode::Mcp, Artifact::Hook) | (Mode::None, Artifact::Hook) => {
            agent_context_recovery::reconcile_unlocked(target, mode)?;
            agent_lifecycle::reconcile_unlocked(kind_for_target(target), mode, true)?;
            agent_permission::reconcile_unlocked(target, mode)
        }
        (Mode::Mcp, Artifact::Mcp) => {
            mcp_config::install(target)?;
            agent_context_recovery::reconcile_unlocked(target, mode)
        }
        _ => Ok(()),
    }
}

/// 卸载当前 / 全部模式产物（Rule + Guard + 超时 Hook + MCP 配置），保留用户其它内容。
fn uninstall_all_unlocked(target: AgentTarget) -> Result<()> {
    agent_context_recovery::reconcile_unlocked(target, Mode::None)?;
    agent_rules::uninstall_unlocked(target)?;
    agent_subagent_guard::reconcile_unlocked(target, Mode::None)?;
    if timeout_hook_supported(target) {
        timeout_hook_uninstall(target)?;
    }
    mcp_config::uninstall(target)?;
    agent_lifecycle::reconcile_unlocked(kind_for_target(target), Mode::None, false)?;
    agent_permission::reconcile_unlocked(target, Mode::None)?;
    agent_stop::reconcile_unlocked(kind_for_target(target), Mode::None)?;
    agent_ask_question::reconcile_unlocked(kind_for_target(target), Mode::None)?;
    Ok(())
}

pub(crate) fn kind_for_target(target: AgentTarget) -> crate::agents::AgentKind {
    match target {
        AgentTarget::Cursor => crate::agents::AgentKind::Cursor,
        AgentTarget::ClaudeCode => crate::agents::AgentKind::Claude,
        AgentTarget::Codex => crate::agents::AgentKind::Codex,
        AgentTarget::Grok => crate::agents::AgentKind::Grok,
        AgentTarget::Pi => crate::agents::AgentKind::Pi,
        AgentTarget::Antigravity => crate::agents::AgentKind::Antigravity,
    }
}

/// One strict snapshot for public mode/state views; do not re-read through lossy bool helpers.
pub(crate) fn antigravity_status() -> Result<(Mode, ArtifactUpdates, bool)> {
    let snapshot = agent_rules::antigravity_snapshot()?;
    let (mcp_installed, mcp_outdated) = mcp_config::antigravity_config_status()?;
    Ok(antigravity_status_from_snapshot(
        snapshot,
        mcp_installed,
        mcp_outdated,
    ))
}

fn antigravity_status_from_snapshot(
    snapshot: super::antigravity_rule::RuleSnapshot,
    mcp_installed: bool,
    mcp_outdated: bool,
) -> (Mode, ArtifactUpdates, bool) {
    let rule_installed = matches!(
        snapshot.state,
        super::antigravity_rule::RuleState::OwnedCurrent
            | super::antigravity_rule::RuleState::OwnedOutdated
    );
    let rule_outdated = snapshot.state == super::antigravity_rule::RuleState::OwnedOutdated;
    let rule_variant = snapshot
        .ownership
        .as_ref()
        .and_then(|r| match r.variant.as_str() {
            "cli" => Some(Variant::Cli),
            "mcp" => Some(Variant::Mcp),
            _ => None,
        });
    let mode = current_from_artifacts(mcp_installed, false, rule_variant);
    let updates = match mode {
        Mode::None => ArtifactUpdates::default(),
        Mode::Cli => ArtifactUpdates {
            rule: !rule_installed || rule_outdated,
            hook: false,
            mcp: false,
        },
        Mode::Mcp => ArtifactUpdates {
            rule: !rule_installed || rule_outdated,
            hook: false,
            mcp: !mcp_installed || mcp_outdated,
        },
    };
    (mode, updates, rule_installed)
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
    fn mode_detection_uses_only_core_integration_artifacts() {
        assert_eq!(current_from_artifacts(true, false, None), Mode::Mcp);
        assert_eq!(current_from_artifacts(false, true, None), Mode::Cli);
        assert_eq!(
            current_from_artifacts(false, false, Some(Variant::Cli)),
            Mode::Cli
        );
        assert_eq!(current_from_artifacts(false, false, None), Mode::None);
    }

    #[test]
    fn antigravity_status_does_not_report_mcp_as_an_installed_rule() {
        let snapshot = super::super::antigravity_rule::RuleSnapshot {
            state: super::super::antigravity_rule::RuleState::Missing,
            ownership: None,
            reason: None,
        };
        let (mode, updates, rule_installed) =
            antigravity_status_from_snapshot(snapshot, true, false);

        assert_eq!(mode, Mode::Mcp);
        assert!(!rule_installed);
        assert!(updates.rule);
        assert!(!updates.mcp);
    }

    #[test]
    fn codex_has_no_timeout_hook() {
        assert!(!timeout_hook_supported(AgentTarget::Codex));
        assert!(!timeout_hook_is_installed(AgentTarget::Codex));
        assert!(!timeout_hook_needs_update(AgentTarget::Codex));
    }

    #[test]
    fn kind_for_target_covers_every_agent_target() {
        assert_eq!(
            kind_for_target(AgentTarget::Cursor),
            crate::agents::AgentKind::Cursor
        );
        assert_eq!(
            kind_for_target(AgentTarget::ClaudeCode),
            crate::agents::AgentKind::Claude
        );
        assert_eq!(
            kind_for_target(AgentTarget::Codex),
            crate::agents::AgentKind::Codex
        );
        assert_eq!(
            kind_for_target(AgentTarget::Grok),
            crate::agents::AgentKind::Grok
        );
        assert_eq!(
            kind_for_target(AgentTarget::Pi),
            crate::agents::AgentKind::Pi
        );
    }

    #[test]
    fn artifact_updates_route_recovery_to_the_mode_owned_artifact() {
        let clean = ArtifactState {
            rule_installed: true,
            timeout_installed: true,
            mcp_installed: true,
            ..ArtifactState::default()
        };
        assert_eq!(
            artifact_updates_for(Mode::None, clean),
            ArtifactUpdates::default()
        );
        assert_eq!(
            artifact_updates_for(
                Mode::None,
                ArtifactState {
                    recovery_outdated: true,
                    ..clean
                }
            ),
            ArtifactUpdates {
                hook: true,
                ..ArtifactUpdates::default()
            }
        );
        assert_eq!(
            artifact_updates_for(
                Mode::Cli,
                ArtifactState {
                    recovery_outdated: true,
                    ..clean
                }
            ),
            ArtifactUpdates {
                hook: true,
                ..ArtifactUpdates::default()
            }
        );
        assert_eq!(
            artifact_updates_for(
                Mode::Mcp,
                ArtifactState {
                    recovery_outdated: true,
                    ..clean
                }
            ),
            ArtifactUpdates {
                mcp: true,
                ..ArtifactUpdates::default()
            }
        );
    }

    #[test]
    fn artifact_updates_keep_rule_timeout_permission_and_mcp_independent() {
        let clean = ArtifactState {
            rule_installed: true,
            timeout_supported: true,
            timeout_installed: true,
            mcp_installed: true,
            ..ArtifactState::default()
        };
        assert_eq!(
            artifact_updates_for(Mode::Cli, clean),
            ArtifactUpdates::default()
        );
        assert_eq!(
            artifact_updates_for(Mode::Mcp, clean),
            ArtifactUpdates::default()
        );

        for state in [
            ArtifactState {
                rule_installed: false,
                ..clean
            },
            ArtifactState {
                rule_outdated: true,
                ..clean
            },
            ArtifactState {
                guard_outdated: true,
                ..clean
            },
        ] {
            assert!(artifact_updates_for(Mode::Cli, state).rule);
            assert!(artifact_updates_for(Mode::Mcp, state).rule);
        }
        for state in [
            ArtifactState {
                timeout_installed: false,
                ..clean
            },
            ArtifactState {
                timeout_outdated: true,
                ..clean
            },
            ArtifactState {
                permission_outdated: true,
                ..clean
            },
        ] {
            assert!(artifact_updates_for(Mode::Cli, state).hook);
        }
        assert!(
            !artifact_updates_for(
                Mode::Cli,
                ArtifactState {
                    timeout_supported: false,
                    timeout_installed: false,
                    ..clean
                }
            )
            .hook
        );
        assert!(
            artifact_updates_for(
                Mode::Mcp,
                ArtifactState {
                    permission_outdated: true,
                    ..clean
                }
            )
            .hook
        );
        for state in [
            ArtifactState {
                mcp_installed: false,
                ..clean
            },
            ArtifactState {
                mcp_outdated: true,
                ..clean
            },
        ] {
            assert!(artifact_updates_for(Mode::Mcp, state).mcp);
        }
    }

    #[test]
    fn lifecycle_drift_is_classified_as_a_hook_update() {
        let base = ArtifactUpdates {
            rule: true,
            hook: false,
            mcp: false,
        };
        assert_eq!(merge_lifecycle_update(base, false), base);
        assert_eq!(
            merge_lifecycle_update(base, true),
            ArtifactUpdates { hook: true, ..base }
        );
    }
}

#[cfg(test)]
mod env_mode_tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn rejects_unsupported_env_modes_before_acquiring_mutation_lock() {
        let env = BTreeMap::from([("DISPLAY".into(), ":1".into())]);
        for (target, mode) in [
            (AgentTarget::ClaudeCode, Mode::Mcp),
            (AgentTarget::Cursor, Mode::Mcp),
            (AgentTarget::Grok, Mode::Mcp),
            (AgentTarget::Pi, Mode::Mcp),
            (AgentTarget::Antigravity, Mode::Cli),
            (AgentTarget::Antigravity, Mode::Mcp),
            (AgentTarget::Codex, Mode::Cli),
            (AgentTarget::Codex, Mode::None),
        ] {
            assert!(set_with_env(target, mode, &env)
                .unwrap_err()
                .to_string()
                .contains("require Codex MCP"));
        }
        let invalid = BTreeMap::from([("".into(), "x".into())]);
        assert!(set_with_env(AgentTarget::Codex, Mode::Mcp, &invalid)
            .unwrap_err()
            .to_string()
            .contains("invalid environment"));
    }
}

#[cfg(test)]
mod antigravity_capability_tests {
    use super::*;
    #[test]
    fn advanced_adapters_remain_closed_except_mcp() {
        let kind = crate::agents::AgentKind::Antigravity;
        let target = AgentTarget::Antigravity;
        assert!(!timeout_hook_supported(target));
        assert!(mcp_config::supported(target));
        assert!(!agent_permission::supported(target));
        assert!(!agent_stop::supported(kind));
        assert!(!agent_ask_question::supported(kind));
        let lifecycle = agent_lifecycle::status_for_mode(kind, Mode::Cli);
        assert!(!lifecycle.supported && !lifecycle.enabled && !lifecycle.needs_update);
        assert!(!super::super::agent_launch::fork_readiness(kind).ready);
    }
}

#[cfg(all(test, unix))]
mod antigravity_transition_tests {
    use super::*;
    use crate::integrations::antigravity_rule::{self, RuleContext, RuleState};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    struct Fixture {
        _root: tempfile::TempDir,
        context: RuleContext,
        config: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let state = root.path().join("state");
            std::fs::create_dir(&state).unwrap();
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
            let context = RuleContext {
                target_path: root
                    .path()
                    .join(".gemini/config/rules/askhuman-antigravity.md"),
                integration_state_file: state.join("integration-state.json"),
                generation_version: "1".into(),
                variant: "cli".into(),
                rendered_bytes: Vec::new(),
            };
            std::fs::write(
                &context.integration_state_file,
                b"{\"codex\":{\"kept\":true},\"future\":9}",
            )
            .unwrap();
            let config = root.path().join(".gemini/config/mcp_config.json");
            Self {
                _root: root,
                context,
                config,
            }
        }
        fn context(&self, mode: Mode) -> RuleContext {
            let mut context = self.context.clone();
            context.variant = if mode == Mode::Mcp { "mcp" } else { "cli" }.into();
            context.rendered_bytes = format!(
                "---\ntrigger: always_on\ndescription: AskHuman\n---\n{}\n<!-- AskHuman:begin DO NOT EDIT (managed by AskHuman) -->\n{}\n<!-- AskHuman:end -->\n",
                antigravity_rule::MANAGED_FILE_MARK, context.variant
            ).into_bytes();
            context
        }
        fn prepare(&self, mode: Mode) -> Result<mcp_config::PreparedAntigravityConfig> {
            mcp_config::PreparedAntigravityConfig::prepare(
                self.config.clone(),
                mode == Mode::Mcp,
                "/test/AskHuman",
            )
        }
        fn set(&self, mode: Mode) -> Result<()> {
            set_antigravity_prepared(&self.context(mode), &self.prepare(mode)?, mode)
        }
        fn bytes(&self) -> Vec<Option<Vec<u8>>> {
            [
                &self.config,
                &self.context.target_path,
                &self.context.integration_state_file,
            ]
            .map(|path| std::fs::read(path).ok())
            .to_vec()
        }
        fn seed_config(&self) {
            std::fs::create_dir_all(self.config.parent().unwrap()).unwrap();
            std::fs::write(&self.config, "{\n // keep comment\n \"mcpServers\": {\"other\": {\"command\":\"other\"}}, \"extra\":17\n}\n").unwrap();
        }
        fn assert_mode(&self, mode: Mode) {
            let snapshot = antigravity_rule::probe(&self.context(mode)).unwrap();
            if mode == Mode::None {
                assert_eq!(snapshot.state, RuleState::Missing);
                assert!(snapshot.ownership.is_none());
            } else {
                assert_eq!(snapshot.state, RuleState::OwnedCurrent);
                assert_eq!(snapshot.ownership.unwrap().variant, mode.as_str());
            }
        }
    }
    struct Failure;
    impl Failure {
        fn at(first: &'static str, second: Option<&'static str>) -> Self {
            antigravity_rule::set_test_failures(Some(first), second);
            Self
        }
    }
    impl Drop for Failure {
        fn drop(&mut self) {
            antigravity_rule::set_test_failures(None, None);
            BEFORE_ANTIGRAVITY_CONFIG_RESTORE.with(|slot| slot.borrow_mut().take());
        }
    }

    #[test]
    fn transitions_are_idempotent_and_preserve_unrelated_configuration() {
        let fixture = Fixture::new();
        fixture.seed_config();
        for mode in [
            Mode::Cli,
            Mode::Mcp,
            Mode::Mcp,
            Mode::Cli,
            Mode::None,
            Mode::None,
        ] {
            fixture.set(mode).unwrap();
            fixture.assert_mode(mode);
            let text = std::fs::read_to_string(&fixture.config).unwrap();
            assert!(text.contains("// keep comment"));
            let config: serde_json::Value =
                jsonc_parser::parse_to_serde_value(&text, &Default::default()).unwrap();
            assert_eq!(config["extra"], 17);
            assert_eq!(config["mcpServers"]["other"]["command"], "other");
            assert_eq!(
                config["mcpServers"].get("askhuman").is_some(),
                mode == Mode::Mcp
            );
            if mode == Mode::Mcp {
                assert_eq!(config["mcpServers"]["askhuman"]["timeoutSeconds"], 86400);
                assert_eq!(
                    config["mcpServers"]["askhuman"]["command"],
                    "/test/AskHuman"
                );
            }
            let ledger: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&fixture.context.integration_state_file).unwrap(),
            )
            .unwrap();
            assert_eq!(ledger["codex"]["kept"], true);
            assert_eq!(ledger["future"], 9);
        }
    }

    #[test]
    fn malformed_or_unreadable_config_preserves_rule_and_ledger_in_every_mode() {
        for bad in [b"{".as_slice(), b"[]", b"{\"mcpServers\":[]}", b"\xff"] {
            let fixture = Fixture::new();
            fixture.set(Mode::Cli).unwrap();
            std::fs::write(&fixture.config, bad).unwrap();
            for mode in [Mode::Cli, Mode::Mcp, Mode::None] {
                let before = fixture.bytes();
                assert!(fixture.set(mode).is_err());
                assert_eq!(fixture.bytes(), before);
            }
        }
        let fixture = Fixture::new();
        fixture.set(Mode::Cli).unwrap();
        std::fs::create_dir(&fixture.config).unwrap();
        let before = fixture.bytes();
        assert!(fixture.set(Mode::Mcp).is_err());
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn conflicting_rule_does_not_remove_working_mcp_configuration() {
        for mode in [Mode::Cli, Mode::None] {
            let fixture = Fixture::new();
            fixture.set(Mode::Mcp).unwrap();
            std::fs::write(&fixture.context.target_path, b"user edited rule").unwrap();
            let before = fixture.bytes();
            let error = fixture.set(mode).unwrap_err().to_string();
            assert!(error.contains("current instance ledger"));
            assert_eq!(fixture.bytes(), before);
        }
    }

    #[test]
    fn config_publication_failure_leaves_rule_untouched() {
        // A root process does not honor the owner write-bit fixture.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let fixture = Fixture::new();
        fixture.seed_config();
        fixture.set(Mode::Cli).unwrap();
        let before = fixture.bytes();
        let parent = fixture.config.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = fixture.set(Mode::Mcp);
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("publication failed"));
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn prepared_publication_and_rollback_preserve_external_edits() {
        let fixture = Fixture::new();
        fixture.seed_config();
        fixture.set(Mode::Cli).unwrap();
        let plan = fixture.prepare(Mode::Mcp).unwrap();
        let old_rule = std::fs::read(&fixture.context.target_path).unwrap();
        std::fs::write(&fixture.config, b"{\"external\":1}").unwrap();
        assert!(set_antigravity_prepared(&fixture.context(Mode::Mcp), &plan, Mode::Mcp).is_err());
        assert_eq!(
            std::fs::read(&fixture.context.target_path).unwrap(),
            old_rule
        );
        assert_eq!(std::fs::read(&fixture.config).unwrap(), b"{\"external\":1}");
        let plan = fixture.prepare(Mode::Mcp).unwrap();
        plan.apply().unwrap();
        std::fs::write(&fixture.config, b"{\"external\":2}").unwrap();
        assert!(plan.restore().is_err());
        assert_eq!(std::fs::read(&fixture.config).unwrap(), b"{\"external\":2}");
    }

    #[test]
    fn precommit_rule_failures_restore_logical_rule_and_exact_config() {
        for point in [
            "prepared",
            "file-applied",
            "ledger-applied-before-journal",
            "ledger-applied",
        ] {
            let fixture = Fixture::new();
            fixture.seed_config();
            fixture.set(Mode::Cli).unwrap();
            let before = fixture.bytes();
            let failure = Failure::at(point, None);
            let error = fixture.set(Mode::Mcp).unwrap_err().to_string();
            drop(failure);
            assert!(
                error.contains("prior rule and MCP config restored"),
                "{error}"
            );
            assert_eq!(std::fs::read(&fixture.config).ok(), before[0]);
            assert_eq!(std::fs::read(&fixture.context.target_path).ok(), before[1]);
            fixture.assert_mode(Mode::Cli);
        }
    }

    #[test]
    fn failed_new_install_restores_original_config_absence() {
        let fixture = Fixture::new();
        let failure = Failure::at("file-applied", None);
        assert!(fixture.set(Mode::Mcp).is_err());
        drop(failure);
        assert!(!fixture.config.exists());
        fixture.assert_mode(Mode::None);
        fixture.set(Mode::Mcp).unwrap();
        fixture.assert_mode(Mode::Mcp);
    }

    #[test]
    fn failed_removal_restores_mcp_rule_and_configuration() {
        let fixture = Fixture::new();
        fixture.set(Mode::Mcp).unwrap();
        let before = fixture.bytes();
        let failure = Failure::at("file-applied", None);
        assert!(fixture.set(Mode::None).is_err());
        drop(failure);
        assert_eq!(std::fs::read(&fixture.config).ok(), before[0]);
        fixture.assert_mode(Mode::Mcp);
    }

    #[test]
    fn committed_rule_error_keeps_published_config_and_explicit_retry_converges() {
        let fixture = Fixture::new();
        fixture.set(Mode::Cli).unwrap();
        let failure = Failure::at("committed", None);
        let error = fixture.set(Mode::Mcp).unwrap_err().to_string();
        drop(failure);
        assert!(error.contains("preserving published MCP config"), "{error}");
        assert!(error.contains("retry explicitly: AskHuman agents mode antigravity mcp"));
        fixture.assert_mode(Mode::Mcp);
        fixture.set(Mode::Mcp).unwrap();
        fixture.assert_mode(Mode::Mcp);
        assert!(std::fs::read_to_string(&fixture.config)
            .unwrap()
            .contains("86400"));
    }

    #[test]
    fn persistent_committed_cleanup_failure_preserves_evidence_until_explicit_retry() {
        let fixture = Fixture::new();
        fixture.seed_config();
        fixture.set(Mode::Cli).unwrap();
        let private = fixture
            .context
            .integration_state_file
            .parent()
            .unwrap()
            .join(".antigravity-rule-state");
        let journal_path = private.join(".antigravity-rule.pending.json");
        let failure = Failure::at("cleanup", None);
        let error = fixture.set(Mode::Mcp).unwrap_err().to_string();
        assert!(error.contains("rule recovery uncertain"), "{error}");
        assert!(error.contains("injected failure at cleanup"), "{error}");
        assert!(error.contains("retry explicitly: AskHuman agents mode antigravity mcp"));
        let journal_bytes = std::fs::read(&journal_path).unwrap();
        let journal: serde_json::Value = serde_json::from_slice(&journal_bytes).unwrap();
        assert_eq!(journal["phase"], "committed");
        let transaction = private.join(journal["transaction_dir"].as_str().unwrap());
        let preimages: Vec<_> = ["target_preimage", "ledger_preimage"]
            .iter()
            .map(|key| {
                let path = transaction.join(journal[key]["file"].as_str().unwrap());
                let bytes = std::fs::read(&path).unwrap();
                assert!(!bytes.is_empty());
                (path, bytes)
            })
            .collect();
        let published = fixture.bytes();
        assert!(std::str::from_utf8(published[0].as_ref().unwrap())
            .unwrap()
            .contains("86400"));
        assert_eq!(
            antigravity_rule::probe(&fixture.context(Mode::Mcp))
                .unwrap()
                .state,
            RuleState::NeedsRepair
        );
        for _ in 0..2 {
            let error = fixture.set(Mode::Mcp).unwrap_err().to_string();
            assert!(error.contains("rule recovery failed"), "{error}");
            assert!(error.contains("injected failure at cleanup"), "{error}");
            assert!(
                error.contains("MCP config") && error.contains("unchanged"),
                "{error}"
            );
            assert_eq!(fixture.bytes(), published);
            assert_eq!(std::fs::read(&journal_path).unwrap(), journal_bytes);
            for (path, bytes) in &preimages {
                assert_eq!(&std::fs::read(path).unwrap(), bytes);
            }
        }
        drop(failure);
        fixture.set(Mode::Mcp).unwrap();
        fixture.assert_mode(Mode::Mcp);
        assert_eq!(fixture.bytes(), published);
        assert!(!journal_path.exists());
        assert!(!transaction.exists());
        let config = std::fs::read_to_string(&fixture.config).unwrap();
        assert!(
            config.contains("keep comment") && config.contains("other") && config.contains("17")
        );
    }

    #[test]
    fn orchestration_preserves_external_config_edits_before_rollback() {
        for (before, requested) in [
            (Mode::Cli, Mode::Mcp),
            (Mode::Mcp, Mode::Cli),
            (Mode::Mcp, Mode::None),
        ] {
            let fixture = Fixture::new();
            fixture.seed_config();
            fixture.set(before).unwrap();
            let old_rule = std::fs::read(&fixture.context.target_path).unwrap();
            let external =
                br#"{"mcpServers":{"external":{"command":"keep-me"}},"externalRevision":23}"#;
            let config_path = fixture.config.clone();
            let failure = Failure::at("file-applied", None);
            BEFORE_ANTIGRAVITY_CONFIG_RESTORE.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    std::fs::write(config_path, external).unwrap();
                }));
            });
            let error = fixture.set(requested).unwrap_err().to_string();
            assert!(
                error.contains("prior rule restored, but MCP config restoration refused/failed"),
                "{error}"
            );
            assert!(error.contains("changed outside this operation"), "{error}");
            assert!(
                error.contains(&fixture.config.display().to_string()),
                "{error}"
            );
            assert!(
                error.contains(&format!(
                    "retry explicitly: AskHuman agents mode antigravity {}",
                    requested.as_str()
                )),
                "{error}"
            );
            drop(failure);
            assert_eq!(std::fs::read(&fixture.config).unwrap(), external);
            assert_eq!(
                std::fs::read(&fixture.context.target_path).unwrap(),
                old_rule
            );
            fixture.assert_mode(before);
            fixture.set(requested).unwrap();
            fixture.assert_mode(requested);
            let config: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&fixture.config).unwrap()).unwrap();
            assert_eq!(config["externalRevision"], 23);
            assert_eq!(config["mcpServers"]["external"]["command"], "keep-me");
            assert_eq!(
                config["mcpServers"].get("askhuman").is_some(),
                requested == Mode::Mcp
            );
        }
    }

    #[test]
    fn failed_recovery_preserves_evidence_and_explicit_retry_converges() {
        let fixture = Fixture::new();
        fixture.set(Mode::Cli).unwrap();
        let failure = Failure::at("file-applied", Some("recovery-target-applied"));
        let error = fixture.set(Mode::Mcp).unwrap_err().to_string();
        drop(failure);
        assert!(error.contains("rule recovery uncertain"), "{error}");
        assert!(std::fs::read_to_string(&fixture.config)
            .unwrap()
            .contains("86400"));
        assert_eq!(
            antigravity_rule::probe(&fixture.context(Mode::Mcp))
                .unwrap()
                .state,
            RuleState::NeedsRepair
        );
        fixture.set(Mode::Mcp).unwrap();
        fixture.assert_mode(Mode::Mcp);
    }

    #[test]
    fn interruption_between_config_and_rule_requires_explicit_requested_mode() {
        let fixture = Fixture::new();
        fixture.set(Mode::Mcp).unwrap();
        fixture.prepare(Mode::Cli).unwrap().apply().unwrap();
        fixture.assert_mode(Mode::Mcp);
        fixture.set(Mode::Cli).unwrap();
        fixture.assert_mode(Mode::Cli);
        assert!(!std::fs::read_to_string(&fixture.config)
            .unwrap()
            .contains("askhuman"));
    }

    #[test]
    fn pending_recovery_rejects_writable_state_parent_before_config_publication() {
        let fixture = Fixture::new();
        fixture.set(Mode::Cli).unwrap();
        let failure = Failure::at("file-applied", Some("recovery-target-applied"));
        assert!(fixture.set(Mode::Mcp).is_err());
        drop(failure);
        let before = fixture.bytes();
        let parent = fixture.context.integration_state_file.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        let error = fixture.set(Mode::Mcp).unwrap_err().to_string();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(error.contains("not writable by other users"), "{error}");
        assert!(error.contains(&fixture.context.integration_state_file.display().to_string()));
        assert_eq!(fixture.bytes(), before);
        fixture.set(Mode::Mcp).unwrap();
        fixture.assert_mode(Mode::Mcp);
    }

    #[test]
    fn none_clears_stale_ownership_when_rule_is_missing() {
        let fixture = Fixture::new();
        fixture.set(Mode::Mcp).unwrap();
        std::fs::remove_file(&fixture.context.target_path).unwrap();
        fixture.set(Mode::None).unwrap();
        fixture.assert_mode(Mode::None);
    }
}
