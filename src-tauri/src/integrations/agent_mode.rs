//! 三态模式编排：把每家 Agent 的「Rule + Subagent Guard +（超时 Hook | MCP 配置）」聚合为
//! **None / Cli / Mcp** 三态互斥，
//! 供设置 UI 与 `agents`/`doctor` CLI 复用。
//!
//! - **Cli** 模式绑定：CLI 版 Rule + Guard + 超时 Hook（Codex 无超时 Hook）。
//! - **Mcp** 模式绑定：MCP 版 Rule + Guard + MCP 配置（用户级全局）。
//! - Mode changes reconcile the target bundle idempotently. Antigravity is one managed plugin
//!   (rules, MCP declaration and lifecycle hooks, see `antigravity_plugin`); the standalone rule
//!   file and the global MCP entry of the earlier design are retired only after the plugin is in
//!   place, and a leftover the user changed is reported, never overwritten.
//!
//! For Codex the "Rule" is no longer a managed `AGENTS.md` block: the interaction protocol is
//! delivered by the mode-owned SessionStart hook (see `codex_protocol`), so delegated runs such as
//! `codex exec` receive no injection at all. A legacy managed block is migrated only by explicit
//! user actions (`set`, `update`, `update_artifact`, `agents update`, the Rules "update" button),
//! never automatically after an upgrade.
//!
//! Lifecycle tracking is an optional capability owned by an active automatic integration. New
//! integrations adopt the default-on preference; explicit opt-out survives mode updates. None
//! mode always removes the on-disk lifecycle artifact.

use crate::integrations::agent_rules::{self, AgentTarget, Variant};
use crate::integrations::{
    agent_ask_question, agent_context_recovery, agent_lifecycle, agent_permission, agent_stop,
    agent_subagent_guard, antigravity_plugin, claude_hook, codex_protocol, cursor_hook, mcp_config,
    mutation_lock,
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
    if target == AgentTarget::Antigravity {
        // One plugin carries every Antigravity artifact, so the generic artifact walk below does
        // not apply. Like the other lenient readers, an unreadable state reads as None.
        return antigravity_status()
            .map(|(mode, _, _)| mode)
            .unwrap_or(Mode::None);
    }
    let mcp = mcp_config::is_installed(target);
    if target == AgentTarget::Codex {
        // Codex has no timeout hook, and its protocol carrier (the SessionStart hook) is identical
        // in both modes, so the generic artifact rule below cannot be used: MCP config → Mcp, a
        // legacy AGENTS.md block → its variant, a lone protocol hook → Cli.
        return codex_protocol::current_mode(mcp);
    }
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
    if target == AgentTarget::Antigravity {
        return antigravity_status()
            .map(|(_, updates, _)| updates)
            .unwrap_or_default();
    }
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
            // Codex's SessionStart hook is its protocol carrier: its drift is reported through the
            // Rules flag (`rule_outdated`, via `codex_protocol::needs_update`), where the update
            // button performs the migration. Only leftover cleanup in None mode stays a hook flag.
            recovery_outdated: if target == AgentTarget::Codex && mode != Mode::None {
                false
            } else {
                agent_context_recovery::needs_update(target, mode)
            },
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
    ensure_protocol_deliverable(target, mode)?;
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
        return set_antigravity(mode);
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

/// Publish (or remove) the AskHuman plugin, then retire what the earlier design left behind.
///
/// The plugin goes first: until it exists, the earlier rule and MCP entry keep Antigravity
/// working, so a failure never leaves it without the protocol. Retiring the leftovers is a
/// separate step that stops at anything the user changed and then reports it; the plugin is
/// already in place by then.
fn set_antigravity(mode: Mode) -> Result<()> {
    match mode {
        Mode::None => antigravity_plugin::uninstall()?,
        Mode::Cli | Mode::Mcp => antigravity_plugin::install(antigravity_plugin::Request {
            variant: if mode == Mode::Mcp {
                Variant::Mcp
            } else {
                Variant::Cli
            },
            lifecycle: agent_lifecycle::tracking_wanted(crate::agents::AgentKind::Antigravity),
            // The Stop confirmation joins the shared Stop hook in a later phase.
            stop: false,
        })?,
    }
    agent_lifecycle::reconcile_unlocked(crate::agents::AgentKind::Antigravity, mode, true)?;
    retire_earlier_antigravity(mode)
}

/// Remove the standalone rule file and the global MCP entry of the earlier design. With neither
/// present nothing is read or written, so an unrelated, malformed global MCP configuration never
/// blocks the plugin.
fn retire_earlier_antigravity(requested: Mode) -> Result<()> {
    if !earlier_antigravity_left() {
        return Ok(());
    }
    let retry = format!(
        "retry explicitly: AskHuman agents mode antigravity {}",
        requested.as_str()
    );
    let config = mcp_config::prepare_antigravity_config(false)?;
    // The rule's variant does not matter for removal; the writer only needs its location.
    let context = agent_rules::antigravity_context_for_variant(Variant::Cli);
    set_antigravity_transaction(&context, &config, Mode::None, &retry).map_err(|error| {
        anyhow::anyhow!(
            "the AskHuman plugin is in place, but the earlier Antigravity rule or MCP entry could not be removed and may duplicate it: {error:#}"
        )
    })
}

/// The earlier design's transaction: the MCP configuration and the standalone rule are changed
/// together, and the configuration is restored when the rule change fails. Only the retirement
/// of the leftovers uses it now.
#[cfg(test)]
fn set_antigravity_prepared(
    context: &super::antigravity_rule::RuleContext,
    config: &mcp_config::PreparedAntigravityConfig,
    mode: Mode,
) -> Result<()> {
    let retry = format!(
        "retry explicitly: AskHuman agents mode antigravity {}",
        mode.as_str()
    );
    set_antigravity_transaction(context, config, mode, &retry)
}

fn set_antigravity_transaction(
    context: &super::antigravity_rule::RuleContext,
    config: &mcp_config::PreparedAntigravityConfig,
    mode: Mode,
    retry: &str,
) -> Result<()> {
    use super::antigravity_rule::{self, RuleState};
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

/// Codex receives its protocol from a SessionStart hook and there is no static fallback (spec
/// `codex-delegated-run-silence` D11), so below the version floor nothing may be (re)installed:
/// failing up front keeps a mode switch from leaving a half-applied integration. Removal
/// (`Mode::None`) is always allowed.
fn ensure_protocol_deliverable(target: AgentTarget, mode: Mode) -> Result<()> {
    if target == AgentTarget::Codex && mode != Mode::None {
        codex_protocol::ensure_supported()?;
    }
    Ok(())
}

/// 更新当前模式的全部产物到最新（不切换模式）。None 有残留时清理，clean None 幂等 no-op。
pub fn update(target: AgentTarget) -> Result<()> {
    if target == AgentTarget::Antigravity {
        let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
        let mode = mode_for_update(target)?;
        return set_unlocked_with_env(target, mode, &std::collections::BTreeMap::new());
    }
    agent_rules::validate_status(target)?;
    set(target, current(target))
}

fn mode_for_update(target: AgentTarget) -> Result<Mode> {
    if target == AgentTarget::Antigravity {
        // Preserve ledger-backed missing-rule recovery and every strict status error.
        antigravity_status().map(|(mode, _, _)| mode)
    } else {
        Ok(current(target))
    }
}

/// 把当前模式下的**单个产物**刷新到最新（不切换模式、不动其它产物）。各底层 install 均幂等，
/// 故「重装即更新」；与当前模式不相干的产物（如 None、或在 Cli 模式更新 Mcp）为 no-op。
pub fn update_artifact(target: AgentTarget, artifact: Artifact) -> Result<()> {
    let _lock = mutation_lock::IntegrationMutationLock::acquire()?;
    agent_rules::validate_status(target)?;
    let mode = mode_for_update(target)?;
    if target == AgentTarget::Antigravity {
        // The rules, the MCP declaration and the hooks are files of one plugin: refreshing any
        // of them refreshes all of it.
        return set_antigravity(mode);
    }
    ensure_protocol_deliverable(target, mode)?;
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
///
/// The plugin is the integration. Without one, an integration of the earlier design (a
/// standalone rule file and a global MCP entry) still defines the mode, with every strict error
/// preserved, and is reported as due for migration.
pub(crate) fn antigravity_status() -> Result<(Mode, ArtifactUpdates, bool)> {
    let (mode, updates, rule_installed) = antigravity_artifact_status()?;
    let lifecycle = agent_lifecycle::status_for_mode(crate::agents::AgentKind::Antigravity, mode);
    Ok((
        mode,
        merge_lifecycle_update(updates, lifecycle.needs_update),
        rule_installed,
    ))
}

fn antigravity_artifact_status() -> Result<(Mode, ArtifactUpdates, bool)> {
    let plugin = antigravity_plugin::snapshot()?;
    plugin.manageable()?;
    if plugin.present() {
        return Ok(antigravity_plugin_status(
            &plugin,
            earlier_antigravity_left(),
        ));
    }
    let snapshot = agent_rules::antigravity_snapshot()?;
    let (mcp_installed, mcp_outdated) = mcp_config::antigravity_config_status()?;
    let (mode, mut updates, rule_installed) =
        antigravity_status_from_snapshot(snapshot, mcp_installed, mcp_outdated);
    if mode != Mode::None {
        // The integration still lives in the earlier files; an update migrates it to the plugin.
        updates.rule = true;
        updates.mcp = mode == Mode::Mcp;
    }
    Ok((mode, updates, rule_installed))
}

/// Whether the earlier design left a standalone rule file or a global MCP entry behind. What
/// cannot be read or proven to be AskHuman's counts as left behind: retiring it reports why.
fn earlier_antigravity_left() -> bool {
    agent_rules::antigravity_snapshot().map_or(true, |snapshot| {
        snapshot.state != super::antigravity_rule::RuleState::Missing
    }) || mcp_config::antigravity_config_status().is_ok_and(|(installed, _)| installed)
}

/// Status of an integration the plugin carries. The plugin is one artifact: whatever is stale in
/// it is refreshed by the same update.
fn antigravity_plugin_status(
    plugin: &antigravity_plugin::Snapshot,
    earlier_left: bool,
) -> (Mode, ArtifactUpdates, bool) {
    use antigravity_plugin::State;
    let mode = match plugin.variant {
        Some(Variant::Cli) => Mode::Cli,
        Some(Variant::Mcp) => Mode::Mcp,
        None => Mode::None,
    };
    let stale = plugin.state != State::OwnedCurrent;
    let updates = if mode == Mode::None {
        ArtifactUpdates::default()
    } else {
        ArtifactUpdates {
            // Leftovers of the earlier design do not impair a current plugin, so they only make
            // the integration "due for an update"; the MCP flag also gates task readiness.
            rule: stale || earlier_left,
            hook: false,
            mcp: mode == Mode::Mcp && stale,
        }
    };
    let rule_installed = matches!(plugin.state, State::OwnedCurrent | State::OwnedOutdated);
    (mode, updates, rule_installed)
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
            rule: !rule_installed || rule_outdated || rule_variant != Some(Variant::Cli),
            hook: false,
            mcp: false,
        },
        Mode::Mcp => ArtifactUpdates {
            rule: !rule_installed || rule_outdated || rule_variant != Some(Variant::Mcp),
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
    fn antigravity_mcp_status_marks_a_current_cli_rule_for_update() {
        let snapshot = super::super::antigravity_rule::RuleSnapshot {
            state: super::super::antigravity_rule::RuleState::OwnedCurrent,
            ownership: Some(super::super::antigravity_rule::RuleOwnership {
                path: std::path::PathBuf::from(
                    "/home/test/.gemini/config/rules/askhuman-antigravity.md",
                ),
                generation_version: "1".into(),
                variant: "cli".into(),
                sha256: "owned-digest".into(),
                owner: Some(1),
                legacy_file_identity: None,
            }),
            reason: None,
        };
        let (mode, updates, installed) = antigravity_status_from_snapshot(snapshot, true, false);
        assert_eq!(mode, Mode::Mcp);
        assert!(installed && updates.rule);
        assert!(!updates.mcp && !updates.hook);
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
    fn advanced_adapters_remain_closed_except_mcp_and_lifecycle() {
        // Lifecycle tracking is open (the plugin's hooks); its on-disk state is covered by the
        // isolated-home tests below, so nothing here reads the real home directory.
        let kind = crate::agents::AgentKind::Antigravity;
        let target = AgentTarget::Antigravity;
        assert!(!timeout_hook_supported(target));
        assert!(mcp_config::supported(target));
        assert!(!agent_permission::supported(target));
        assert!(!agent_stop::supported(kind));
        assert!(!agent_ask_question::supported(kind));
        assert!(!super::super::agent_launch::fork_readiness(kind).ready);
    }

    #[test]
    fn plugin_status_reports_mode_and_what_needs_an_update() {
        use super::antigravity_plugin::{Snapshot, State};
        let snapshot = |state, variant, lifecycle| Snapshot {
            state,
            variant,
            lifecycle,
            reason: None,
            location: std::path::PathBuf::from("/plugin"),
        };
        let none = ArtifactUpdates::default();
        // A current plugin of either variant, with nothing left behind, needs nothing.
        for (variant, mode) in [(Variant::Cli, Mode::Cli), (Variant::Mcp, Mode::Mcp)] {
            let current = snapshot(State::OwnedCurrent, Some(variant), true);
            assert_eq!(
                antigravity_plugin_status(&current, false),
                (mode, none, true)
            );
        }
        // Whatever is stale in the plugin is one update: the rules, and the MCP declaration
        // when the plugin carries one.
        let outdated = |variant| snapshot(State::OwnedOutdated, Some(variant), true);
        assert_eq!(
            antigravity_plugin_status(&outdated(Variant::Cli), false),
            (
                Mode::Cli,
                ArtifactUpdates {
                    rule: true,
                    hook: false,
                    mcp: false
                },
                true
            )
        );
        assert_eq!(
            antigravity_plugin_status(&outdated(Variant::Mcp), false),
            (
                Mode::Mcp,
                ArtifactUpdates {
                    rule: true,
                    hook: false,
                    mcp: true
                },
                true
            )
        );
        // A plugin whose files vanished keeps its mode, but its rules are not installed.
        let repair = snapshot(State::NeedsRepair, Some(Variant::Cli), true);
        let (mode, updates, installed) = antigravity_plugin_status(&repair, false);
        assert_eq!(mode, Mode::Cli);
        assert!(updates.rule && !installed);
        // The earlier design's files, still present next to a current plugin, are due for
        // removal, but they do not make the working MCP declaration stale.
        let current = snapshot(State::OwnedCurrent, Some(Variant::Mcp), true);
        let (mode, updates, installed) = antigravity_plugin_status(&current, true);
        assert_eq!(mode, Mode::Mcp);
        assert!(updates.rule && !updates.mcp && installed);
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
            // Resolve only the trusted fixture root, preserving strict checks below it.
            let root_path = root.path().canonicalize().unwrap();
            let state = root_path.join("state");
            std::fs::create_dir(&state).unwrap();
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
            let context = RuleContext {
                target_path: root_path.join(".gemini/config/rules/askhuman-antigravity.md"),
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
            let config = root_path.join(".gemini/config/mcp_config.json");
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

    /// Runs `body` in a child process whose `HOME` and `ASKHUMAN_HOME` point into a fresh
    /// directory: the plugin and the earlier artifacts live at fixed global locations.
    fn in_isolated_home(test: &str, body: impl FnOnce()) {
        const CHILD: &str = "ASKHUMAN_TEST_ANTIGRAVITY_ISOLATED_CHILD";
        if std::env::var_os(CHILD).is_some() {
            body();
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let state = home.join("state");
        std::fs::create_dir(&state).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output =
            std::process::Command::new(crate::development::routing::program_path().unwrap())
                .args([
                    "--exact",
                    &format!("integrations::agent_mode::antigravity_transition_tests::{test}"),
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("HOME", &home)
                .env("ASKHUMAN_HOME", &state)
                .output()
                .unwrap();
        // A filter that matches nothing also exits successfully; insist that the one test ran.
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn plugin_dir() -> PathBuf {
        crate::paths::antigravity_plugin_dir_in(&crate::paths::integration_home())
    }

    /// Every file below `dir`, by relative path.
    fn tree(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(
            base: &std::path::Path,
            dir: &std::path::Path,
            out: &mut std::collections::BTreeMap<String, Vec<u8>>,
        ) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    out.insert(
                        path.strip_prefix(base)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        std::fs::read(&path).unwrap(),
                    );
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// The integration as the release before the plugin left it: a standalone rule file under
    /// strict ownership and, for MCP, an entry in the global MCP configuration.
    fn seed_earlier_integration(mode: Mode) {
        let config = mcp_config::prepare_antigravity_config(mode == Mode::Mcp).unwrap();
        let variant = if mode == Mode::Mcp {
            Variant::Mcp
        } else {
            Variant::Cli
        };
        let context = agent_rules::antigravity_context_for_variant(variant);
        set_antigravity_prepared(&context, &config, mode).unwrap();
    }

    #[test]
    fn plugin_updates_repair_missing_files_and_preserve_strict_errors() {
        in_isolated_home(
            "plugin_updates_repair_missing_files_and_preserve_strict_errors",
            || {
                let target = AgentTarget::Antigravity;
                let dir = plugin_dir();
                let rules = dir.join("rules/AGENTS.md");
                let config = crate::paths::antigravity_mcp_config_json();
                for artifact_only in [true, false] {
                    set(target, Mode::Cli).unwrap();
                    std::fs::remove_file(&rules).unwrap();
                    let (mode, updates, installed) = antigravity_status().unwrap();
                    assert_eq!(mode, Mode::Cli);
                    assert!(updates.rule && !installed);
                    if artifact_only {
                        update_artifact(target, Artifact::Rule).unwrap();
                    } else {
                        update(target).unwrap();
                    }
                    let (mode, updates, installed) = antigravity_status().unwrap();
                    assert_eq!(mode, Mode::Cli);
                    assert!(installed && updates == ArtifactUpdates::default());
                }

                // The plugin does not depend on the global MCP configuration: a malformed one is
                // neither read as AskHuman's nor rewritten.
                for malformed in [b"{".as_slice(), b"[]", b"{\"mcpServers\":[]}"] {
                    std::fs::write(&config, malformed).unwrap();
                    let before = tree(&dir);
                    assert_eq!(antigravity_status().unwrap().1, ArtifactUpdates::default());
                    update(target).unwrap();
                    update_artifact(target, Artifact::Rule).unwrap();
                    assert_eq!(std::fs::read(&config).unwrap(), malformed);
                    assert_eq!(tree(&dir), before);
                }
                std::fs::remove_file(&config).unwrap();

                // A file someone else edited is a conflict that nothing touches.
                let published = std::fs::read(&rules).unwrap();
                std::fs::write(&rules, b"user-edited rules").unwrap();
                let before = tree(&dir);
                assert!(antigravity_status().is_err());
                assert!(update(target).is_err());
                assert!(update_artifact(target, Artifact::Rule).is_err());
                assert!(set(target, Mode::Mcp).is_err());
                assert!(set(target, Mode::None).is_err());
                assert_eq!(tree(&dir), before);
                std::fs::write(&rules, published).unwrap();
                assert_eq!(antigravity_status().unwrap().1, ArtifactUpdates::default());
            },
        );
    }

    #[test]
    fn the_earlier_integration_migrates_to_the_plugin_and_is_then_retired() {
        in_isolated_home(
            "the_earlier_integration_migrates_to_the_plugin_and_is_then_retired",
            || {
                use super::antigravity_plugin::State;
                let target = AgentTarget::Antigravity;
                let config = crate::paths::antigravity_mcp_config_json();
                let earlier_rule =
                    crate::paths::antigravity_rule_file_in(&crate::paths::integration_home());
                std::fs::create_dir_all(config.parent().unwrap()).unwrap();
                std::fs::write(
                    &config,
                    "{\n // keep comment\n \"mcpServers\": {\"other\": {\"command\":\"other\"}}, \"extra\":17\n}\n",
                )
                .unwrap();
                seed_earlier_integration(Mode::Mcp);
                assert!(earlier_rule.exists());
                assert!(std::fs::read_to_string(&config)
                    .unwrap()
                    .contains("askhuman"));

                // The earlier files still define the mode, and the missing plugin is due.
                let (mode, updates, installed) = antigravity_status().unwrap();
                assert_eq!(mode, Mode::Mcp);
                assert!(installed && updates.rule && updates.mcp);
                assert_eq!(
                    antigravity_plugin::snapshot().unwrap().state,
                    State::Missing
                );

                update(target).unwrap();

                // The plugin carries everything now; nothing of the earlier design is left.
                let plugin = antigravity_plugin::snapshot().unwrap();
                assert_eq!(plugin.state, State::OwnedCurrent);
                assert_eq!(plugin.variant, Some(Variant::Mcp));
                assert!(plugin_dir().join("mcp_config.json").exists());
                assert!(!earlier_rule.exists());
                let text = std::fs::read_to_string(&config).unwrap();
                assert!(text.contains("// keep comment") && !text.contains("askhuman"));
                let value: serde_json::Value =
                    jsonc_parser::parse_to_serde_value(&text, &Default::default()).unwrap();
                assert_eq!(value["extra"], 17);
                assert_eq!(value["mcpServers"]["other"]["command"], "other");
                assert_eq!(
                    antigravity_status().unwrap(),
                    (Mode::Mcp, ArtifactUpdates::default(), true)
                );

                // A second update changes nothing.
                let plugin_before = tree(&plugin_dir());
                let config_before = std::fs::read(&config).unwrap();
                update(target).unwrap();
                assert_eq!(tree(&plugin_dir()), plugin_before);
                assert_eq!(std::fs::read(&config).unwrap(), config_before);

                // None removes the plugin and leaves the user's own configuration alone.
                set(target, Mode::None).unwrap();
                assert_eq!(
                    antigravity_plugin::snapshot().unwrap().state,
                    State::Missing
                );
                assert!(!plugin_dir().exists());
                assert_eq!(antigravity_status().unwrap().0, Mode::None);
                assert_eq!(std::fs::read(&config).unwrap(), config_before);
            },
        );
    }

    #[test]
    fn an_earlier_rule_the_user_changed_is_reported_while_the_plugin_stays() {
        in_isolated_home(
            "an_earlier_rule_the_user_changed_is_reported_while_the_plugin_stays",
            || {
                use super::antigravity_plugin::State;
                let target = AgentTarget::Antigravity;
                let earlier_rule =
                    crate::paths::antigravity_rule_file_in(&crate::paths::integration_home());
                seed_earlier_integration(Mode::Cli);
                std::fs::write(&earlier_rule, b"my own rules").unwrap();

                // Without a plugin the earlier rule still defines the mode: its conflict is a
                // strict status error and an update refuses to run.
                assert!(antigravity_status().is_err());
                assert!(update(target).is_err());
                assert!(antigravity_plugin::snapshot().unwrap().state == State::Missing);

                // An explicit mode change publishes the plugin, then stops at the changed rule.
                let error = set(target, Mode::Cli).unwrap_err().to_string();
                assert!(error.contains("plugin is in place"), "{error}");
                assert_eq!(std::fs::read(&earlier_rule).unwrap(), b"my own rules");
                assert_eq!(
                    antigravity_plugin::snapshot().unwrap().state,
                    State::OwnedCurrent
                );
                let (mode, updates, _) = antigravity_status().unwrap();
                assert_eq!(mode, Mode::Cli);
                assert!(updates.rule);

                // Updating again reports it again and still touches neither file.
                let plugin_before = tree(&plugin_dir());
                assert!(update(target).is_err());
                assert_eq!(std::fs::read(&earlier_rule).unwrap(), b"my own rules");
                assert_eq!(tree(&plugin_dir()), plugin_before);

                // Once the user removes it, the next update settles.
                std::fs::remove_file(&earlier_rule).unwrap();
                update(target).unwrap();
                assert_eq!(antigravity_status().unwrap().1, ArtifactUpdates::default());
            },
        );
    }

    #[test]
    fn the_lifecycle_switch_edits_the_plugin_hooks() {
        in_isolated_home("the_lifecycle_switch_edits_the_plugin_hooks", || {
            use crate::agents::AgentKind;
            let kind = AgentKind::Antigravity;
            let target = AgentTarget::Antigravity;
            let hooks = plugin_dir().join("hooks.json");

            // Without an integration there is nothing to carry the hooks.
            assert!(agent_lifecycle::set_enabled(kind, true).is_err());
            assert!(!agent_lifecycle::any_installed());

            set(target, Mode::Mcp).unwrap();
            let status = agent_lifecycle::status(kind);
            assert!(status.supported && status.enabled && status.installed);
            assert!(!status.needs_update && !status.cleanup_required);
            assert!(hooks.exists());
            assert!(agent_lifecycle::any_installed());
            assert!(agent_lifecycle::tracking_installed(kind));

            agent_lifecycle::set_enabled(kind, false).unwrap();
            let status = agent_lifecycle::status(kind);
            assert!(!status.enabled && status.preference_configured && !status.installed);
            assert!(!status.needs_update);
            assert!(!hooks.exists());
            assert_eq!(current(target), Mode::Mcp);
            assert!(!agent_lifecycle::any_installed());

            // The preference survives an update of the integration.
            update(target).unwrap();
            assert!(!hooks.exists());
            assert_eq!(antigravity_status().unwrap().1, ArtifactUpdates::default());

            agent_lifecycle::set_enabled(kind, true).unwrap();
            assert!(hooks.exists());
            assert_eq!(antigravity_status().unwrap().1, ArtifactUpdates::default());

            // Removing the integration removes the hooks with it and keeps the preference.
            set(target, Mode::None).unwrap();
            assert!(!hooks.exists());
            let status = agent_lifecycle::status(kind);
            assert!(!status.enabled && !status.installed && status.preference_configured);
            assert_eq!(agent_lifecycle::preference(kind), Some(true));
        });
    }

    #[test]
    fn declared_mcp_configuration_exposes_and_repairs_cli_rule_drift() {
        let fixture = Fixture::new();
        fixture.set(Mode::Cli).unwrap();
        fixture.prepare(Mode::Mcp).unwrap().apply().unwrap();
        let snapshot = antigravity_rule::probe(&fixture.context(Mode::Cli)).unwrap();
        assert_eq!(snapshot.state, RuleState::OwnedCurrent);
        let (mode, updates, installed) = antigravity_status_from_snapshot(snapshot, true, false);
        assert_eq!(mode, Mode::Mcp);
        assert!(installed && updates.rule);
        fixture.set(mode).unwrap();
        fixture.assert_mode(Mode::Mcp);
        let (_, updates, _) = antigravity_status_from_snapshot(
            antigravity_rule::probe(&fixture.context(Mode::Mcp)).unwrap(),
            true,
            false,
        );
        assert!(!updates.rule);
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
        assert_eq!(journal["format_version"], 1);
        for key in ["old_target", "old_ledger"] {
            assert!(!journal[key]["bytes"].as_array().unwrap().is_empty());
        }
        assert!(journal["transaction_dir"].is_null());
        assert_eq!(std::fs::read_dir(&private).unwrap().count(), 1);
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
            assert_eq!(std::fs::read_dir(&private).unwrap().count(), 1);
        }
        drop(failure);
        fixture.set(Mode::Mcp).unwrap();
        fixture.assert_mode(Mode::Mcp);
        assert_eq!(fixture.bytes(), published);
        assert!(!journal_path.exists());
        assert_eq!(std::fs::read_dir(&private).unwrap().count(), 0);
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
