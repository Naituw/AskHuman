//! Codex interaction-protocol delivery (spec `docs/specs/codex-delegated-run-silence.md`, D8–D11).
//!
//! The protocol reaches a Codex model through the SessionStart hook (`agents::context_recovery`),
//! not through `~/.codex/AGENTS.md`: Codex loads AGENTS.md into every run, delegated ones
//! included, and those must behave as if AskHuman were not integrated. The managed AGENTS.md block
//! that older AskHuman versions wrote (the *legacy block*) is recognized here only so it can be
//! migrated away.
//!
//! Everything works on a [`ProtocolFiles`] so migration can be tested on temporary files; the
//! parameterless wrappers bind it to the user's real `~/.codex`.

use anyhow::{anyhow, Result};
use std::path::PathBuf;

use super::agent_context_recovery::{self, CodexFiles};
use super::agent_mode::Mode;
use super::agent_rules::{self, AgentTarget, Variant};

/// Oldest Codex release known to run the SessionStart hook this module installs (the baseline of
/// AskHuman's other Codex hook features). A Codex whose version cannot be read counts as supported.
pub const MINIMUM_VERSION: &str = "0.144.1";
const MINIMUM: (u64, u64, u64) = (0, 144, 1);

pub fn version_supported(version: Option<(u64, u64, u64)>) -> bool {
    version.is_none_or(|version| version >= MINIMUM)
}

fn version_label((major, minor, patch): (u64, u64, u64)) -> String {
    format!("{major}.{minor}.{patch}")
}

/// Error for a Codex below [`MINIMUM_VERSION`]; no fallback carrier exists (D11).
pub(crate) fn ensure_supported_for(version: Option<(u64, u64, u64)>) -> Result<()> {
    match version {
        Some(version) if !version_supported(Some(version)) => Err(anyhow!(crate::i18n::tr(
            crate::i18n::Lang::current(),
            "cmd.codexProtocolUnsupported"
        )
        .replace("{version}", &version_label(version))
        .replace("{minimum}", MINIMUM_VERSION))),
        _ => Ok(()),
    }
}

/// [`ensure_supported_for`] against the installed Codex (cached login-shell probe).
pub fn ensure_supported() -> Result<()> {
    ensure_supported_for(super::agent_launch::codex_version())
}

/// The files protocol delivery touches.
#[derive(Clone, Debug)]
pub(crate) struct ProtocolFiles {
    pub codex: CodexFiles,
    pub agents_md: PathBuf,
}

impl ProtocolFiles {
    pub(crate) fn real() -> Self {
        Self {
            codex: CodexFiles::real(),
            agents_md: crate::paths::codex_agents_md(),
        }
    }
}

// MARK: - State

fn read_agents_md(files: &ProtocolFiles) -> Option<String> {
    std::fs::read_to_string(&files.agents_md).ok()
}

pub(crate) fn has_legacy_block_at(files: &ProtocolFiles) -> bool {
    read_agents_md(files).is_some_and(|text| agent_rules::has_block(&text))
}

/// Variant of the legacy AGENTS.md block; `None` when there is no block.
pub(crate) fn legacy_variant_at(files: &ProtocolFiles) -> Option<Variant> {
    let text = read_agents_md(files)?;
    let body = agent_rules::block_body(&text)?;
    Some(agent_rules::classify_body(&body, AgentTarget::Codex))
}

/// A managed SessionStart hook entry exists (no freshness or trust check).
pub(crate) fn hook_installed_at(files: &ProtocolFiles) -> bool {
    agent_context_recovery::hook_installed_at(AgentTarget::Codex, &files.codex)
}

pub(crate) fn is_installed_at(files: &ProtocolFiles) -> bool {
    hook_installed_at(files) || has_legacy_block_at(files)
}

/// Integration mode of a Codex install (pure).
///
/// The MCP configuration is the stable marker of MCP mode. Without it a legacy block is explicit
/// evidence of its own variant, and a lone protocol hook means CLI mode (the hook is identical in
/// both modes, so it cannot tell them apart by itself).
pub(crate) fn mode_from(mcp: bool, hook: bool, legacy: Option<Variant>) -> Mode {
    if mcp {
        return Mode::Mcp;
    }
    match legacy {
        Some(Variant::Mcp) => Mode::Mcp,
        Some(Variant::Cli) => Mode::Cli,
        None if hook => Mode::Cli,
        None => Mode::None,
    }
}

pub(crate) fn current_mode_at(files: &ProtocolFiles, mcp_installed: bool) -> Mode {
    mode_from(
        mcp_installed,
        hook_installed_at(files),
        legacy_variant_at(files),
    )
}

/// The delivery needs the user's attention: the hook is outdated or untrusted, or the legacy block
/// is still there (D10). A missing hook with no legacy block is "not installed", not "outdated".
/// `supported` is evaluated lazily because it may probe the Codex binary; below the version floor
/// there is nothing the user could update to.
pub(crate) fn needs_update_at(files: &ProtocolFiles, supported: &dyn Fn() -> bool) -> bool {
    let status = agent_context_recovery::status_at(AgentTarget::Codex, Mode::Cli, &files.codex);
    let stale = (status.installed && status.outdated) || has_legacy_block_at(files);
    stale && supported()
}

/// Whether the protocol actually reaches Codex sessions, as shown by `agents show` and `doctor`.
/// Hook delivery can fail silently (Codex skips an untrusted hook without a word), so this makes
/// the failure visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Neither the hook nor a legacy block exists.
    NotInstalled,
    /// The hook is installed, current, and trusted: interactive sessions receive the protocol.
    Trusted,
    /// The hook exists but Codex does not trust it, so it never runs.
    Untrusted,
    /// The hook is stale, or only the legacy AGENTS.md block exists: an update migrates it.
    NeedsUpdate,
    /// This Codex is older than [`MINIMUM_VERSION`]; no carrier exists for it.
    UnsupportedVersion,
}

impl Delivery {
    /// Stable identifier for machine-readable output (`doctor --json`).
    pub fn as_str(self) -> &'static str {
        match self {
            Delivery::NotInstalled => "notInstalled",
            Delivery::Trusted => "trusted",
            Delivery::Untrusted => "untrusted",
            Delivery::NeedsUpdate => "needsUpdate",
            Delivery::UnsupportedVersion => "unsupportedVersion",
        }
    }
}

/// Pure classification behind [`Delivery`]. `stale` is hook-shape drift and only matters once the
/// hook is trusted (an untrusted hook is reported as such first).
pub(crate) fn delivery_from(
    version_supported: bool,
    hook_installed: bool,
    trusted: bool,
    stale: bool,
    legacy_block: bool,
) -> Delivery {
    if !version_supported {
        return Delivery::UnsupportedVersion;
    }
    if hook_installed {
        if !trusted {
            return Delivery::Untrusted;
        }
        return if stale || legacy_block {
            Delivery::NeedsUpdate
        } else {
            Delivery::Trusted
        };
    }
    if legacy_block {
        Delivery::NeedsUpdate
    } else {
        Delivery::NotInstalled
    }
}

pub(crate) fn delivery_at(files: &ProtocolFiles, version: Option<(u64, u64, u64)>) -> Delivery {
    let hook_installed = hook_installed_at(files);
    let trusted = !hook_installed
        || super::agent_permission::codex_marker_trusted_at(
            &files.codex.hooks_json,
            &files.codex.config_toml,
            agent_context_recovery::MARKER,
        )
        .unwrap_or(false);
    let stale = hook_installed
        && agent_context_recovery::status_at(AgentTarget::Codex, Mode::Cli, &files.codex).outdated;
    delivery_from(
        version_supported(version),
        hook_installed,
        trusted,
        stale,
        has_legacy_block_at(files),
    )
}

// MARK: - Migration

type Snapshot = Vec<(PathBuf, Option<Vec<u8>>)>;

fn snapshot(files: &ProtocolFiles) -> Snapshot {
    [
        &files.codex.hooks_json,
        &files.codex.config_toml,
        &files.agents_md,
    ]
    .into_iter()
    .map(|path| (path.clone(), std::fs::read(path).ok()))
    .collect()
}

/// Put every file back exactly as snapshotted. Files that already match are left untouched.
fn rollback(snapshot: &Snapshot) {
    for (path, original) in snapshot {
        let current = std::fs::read(path).ok();
        if &current == original {
            continue;
        }
        match original {
            Some(bytes) => {
                let _ = super::hook_edit::atomic_write(path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// The three steps of [`install_at`], injectable so failure paths can be tested.
pub(crate) struct Steps<'a> {
    pub install_hook: &'a dyn Fn(&ProtocolFiles, Mode) -> Result<()>,
    /// The hook is installed, exact, and trusted.
    pub verify: &'a dyn Fn(&ProtocolFiles, Mode) -> bool,
    pub remove_block: &'a dyn Fn(&ProtocolFiles) -> Result<()>,
}

fn real_install_hook(files: &ProtocolFiles, mode: Mode) -> Result<()> {
    agent_context_recovery::reconcile_at(AgentTarget::Codex, mode, &files.codex)
}

fn real_verify(files: &ProtocolFiles, mode: Mode) -> bool {
    let status = agent_context_recovery::status_at(AgentTarget::Codex, mode, &files.codex);
    status.installed && !status.outdated
}

/// Atomic migration to hook delivery (D10): install and trust the hook first, confirm it, only then
/// delete the legacy block. Any failure puts `hooks.json`, `config.toml`, and `AGENTS.md` back
/// exactly as they were, so there is never a moment with neither carrier (a moment with both only
/// repeats the protocol once).
pub(crate) fn install_with(files: &ProtocolFiles, mode: Mode, steps: &Steps<'_>) -> Result<()> {
    if mode == Mode::None {
        return Err(anyhow!(
            "protocol delivery needs a CLI or MCP integration mode"
        ));
    }
    let before = snapshot(files);
    let fail = |error: anyhow::Error| {
        rollback(&before);
        Err(error)
    };
    if let Err(error) = (steps.install_hook)(files, mode) {
        return fail(error);
    }
    if !(steps.verify)(files, mode) {
        return fail(anyhow!(crate::i18n::tr(
            crate::i18n::Lang::current(),
            "cmd.codexProtocolNotTrusted"
        )));
    }
    if let Err(error) = (steps.remove_block)(files) {
        return fail(error);
    }
    Ok(())
}

pub(crate) fn install_at(files: &ProtocolFiles, mode: Mode) -> Result<()> {
    install_with(
        files,
        mode,
        &Steps {
            install_hook: &real_install_hook,
            verify: &real_verify,
            remove_block: &remove_legacy_block_at,
        },
    )
}

/// Install (or migrate to) hook delivery for the current user.
pub fn install(mode: Mode) -> Result<()> {
    ensure_supported()?;
    install_at(&ProtocolFiles::real(), mode)
}

fn remove_legacy_block_at(files: &ProtocolFiles) -> Result<()> {
    let Some(text) = read_agents_md(files) else {
        return Ok(());
    };
    if !agent_rules::has_block(&text) {
        return Ok(());
    }
    let updated = agent_rules::remove_block(&text);
    super::hook_edit::atomic_write(&files.agents_md, updated.as_bytes())
        .map_err(|error| error.context(format!("failed to write {}", files.agents_md.display())))
}

pub(crate) fn uninstall_at(files: &ProtocolFiles) -> Result<()> {
    agent_context_recovery::reconcile_at(AgentTarget::Codex, Mode::None, &files.codex)?;
    remove_legacy_block_at(files)
}

/// Remove both carriers. Session markers belong to the integration that wrote them, so they go too.
pub fn uninstall() -> Result<()> {
    uninstall_at(&ProtocolFiles::real())?;
    let _ = std::fs::remove_dir_all(crate::paths::codex_delegated_marker_dir());
    Ok(())
}

/// A collaboration-style change reaches the hook at run time, so only a legacy block, which holds a
/// copy of the text, needs rewriting. It is refreshed in place and **not** migrated: migration is an
/// explicit user action (D10).
pub(crate) fn refresh_legacy_block_at(files: &ProtocolFiles) -> Result<()> {
    let Some(text) = read_agents_md(files) else {
        return Ok(());
    };
    let Some(body) = agent_rules::block_body(&text) else {
        return Ok(());
    };
    let variant = agent_rules::classify_body(&body, AgentTarget::Codex);
    let updated = agent_rules::upsert_block(&text, &variant.body(AgentTarget::Codex));
    if updated == text {
        return Ok(());
    }
    super::hook_edit::atomic_write(&files.agents_md, updated.as_bytes())
        .map_err(|error| error.context(format!("failed to write {}", files.agents_md.display())))
}

// MARK: - Real-environment wrappers

pub fn has_legacy_block() -> bool {
    has_legacy_block_at(&ProtocolFiles::real())
}

pub fn legacy_variant() -> Option<Variant> {
    legacy_variant_at(&ProtocolFiles::real())
}

pub fn is_installed() -> bool {
    is_installed_at(&ProtocolFiles::real())
}

pub fn current_mode(mcp_installed: bool) -> Mode {
    current_mode_at(&ProtocolFiles::real(), mcp_installed)
}

pub fn needs_update() -> bool {
    needs_update_at(&ProtocolFiles::real(), &|| {
        version_supported(super::agent_launch::codex_version())
    })
}

pub fn delivery() -> Delivery {
    delivery_at(&ProtocolFiles::real(), super::agent_launch::codex_version())
}

pub fn apply_collaboration_style() -> Result<()> {
    let _lock = super::mutation_lock::IntegrationMutationLock::acquire()?;
    refresh_legacy_block_at(&ProtocolFiles::real())
}

/// File shown (and revealed / opened) for the Codex "Rules" row: the hooks file that carries the
/// protocol, not AGENTS.md.
pub fn display_file() -> PathBuf {
    crate::paths::codex_hooks_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, ProtocolFiles) {
        let dir = tempfile::tempdir().unwrap();
        let files = ProtocolFiles {
            codex: CodexFiles {
                hooks_json: dir.path().join("hooks.json"),
                config_toml: dir.path().join("config.toml"),
            },
            agents_md: dir.path().join("AGENTS.md"),
        };
        (dir, files)
    }

    /// AGENTS.md as an older AskHuman wrote it: user text plus the managed block.
    fn legacy_agents_md(user: &str, variant: Variant) -> String {
        agent_rules::upsert_block(user, &variant.body(AgentTarget::Codex))
    }

    fn read(path: &std::path::Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    fn everything(files: &ProtocolFiles) -> Vec<Option<Vec<u8>>> {
        vec![
            read(&files.codex.hooks_json),
            read(&files.codex.config_toml),
            read(&files.agents_md),
        ]
    }

    fn always_supported() -> bool {
        true
    }

    #[test]
    fn version_floor_accepts_unknown_and_the_baseline_and_newer() {
        assert!(version_supported(None));
        assert!(version_supported(Some((0, 144, 1))));
        assert!(version_supported(Some((0, 163, 0))));
        assert!(version_supported(Some((1, 0, 0))));
        assert!(!version_supported(Some((0, 144, 0))));
        assert!(!version_supported(Some((0, 98, 7))));
        assert_eq!(MINIMUM_VERSION, "0.144.1");
    }

    #[test]
    fn unsupported_versions_are_rejected_with_a_readable_message() {
        assert!(ensure_supported_for(None).is_ok());
        assert!(ensure_supported_for(Some((0, 144, 1))).is_ok());
        let message = ensure_supported_for(Some((0, 140, 2)))
            .unwrap_err()
            .to_string();
        assert!(message.contains("0.140.2"), "{message}");
        assert!(message.contains("0.144.1"), "{message}");
    }

    #[test]
    fn mode_prefers_mcp_config_then_legacy_variant_then_a_lone_hook() {
        use Variant::{Cli, Mcp};
        assert_eq!(mode_from(true, false, None), Mode::Mcp);
        assert_eq!(mode_from(true, true, None), Mode::Mcp);
        // The MCP config outranks a stale CLI block, as it always has.
        assert_eq!(mode_from(true, true, Some(Cli)), Mode::Mcp);
        assert_eq!(mode_from(false, false, Some(Mcp)), Mode::Mcp);
        assert_eq!(mode_from(false, true, Some(Mcp)), Mode::Mcp);
        assert_eq!(mode_from(false, false, Some(Cli)), Mode::Cli);
        assert_eq!(mode_from(false, true, Some(Cli)), Mode::Cli);
        // A migrated install has no block: the hook alone means CLI.
        assert_eq!(mode_from(false, true, None), Mode::Cli);
        assert_eq!(mode_from(false, false, None), Mode::None);
    }

    #[test]
    fn migration_installs_a_trusted_hook_and_removes_only_the_legacy_block() {
        let (_dir, files) = fixture();
        std::fs::write(
            &files.agents_md,
            legacy_agents_md("# My Codex rules\n\nbe kind\n", Variant::Cli),
        )
        .unwrap();
        assert!(has_legacy_block_at(&files));
        assert_eq!(legacy_variant_at(&files), Some(Variant::Cli));
        assert!(!hook_installed_at(&files));
        assert!(needs_update_at(&files, &always_supported));
        assert_eq!(current_mode_at(&files, false), Mode::Cli);

        install_at(&files, Mode::Cli).unwrap();

        assert!(hook_installed_at(&files));
        assert!(!has_legacy_block_at(&files));
        assert_eq!(legacy_variant_at(&files), None);
        let agents = std::fs::read_to_string(&files.agents_md).unwrap();
        assert!(agents.contains("# My Codex rules") && agents.contains("be kind"));
        assert!(!agents.contains("AskHuman"));
        assert!(
            real_verify(&files, Mode::Cli),
            "installed, exact, and trusted"
        );
        assert!(!needs_update_at(&files, &always_supported));
        assert!(is_installed_at(&files));
        // The hook alone now identifies the mode, and the MCP config still outranks it.
        assert_eq!(current_mode_at(&files, false), Mode::Cli);
        assert_eq!(current_mode_at(&files, true), Mode::Mcp);
    }

    #[test]
    fn migration_is_idempotent_and_never_creates_agents_md() {
        let (_dir, files) = fixture();
        install_at(&files, Mode::Mcp).unwrap();
        let first = everything(&files);
        assert!(!files.agents_md.exists(), "AGENTS.md was never there");
        install_at(&files, Mode::Mcp).unwrap();
        assert_eq!(everything(&files), first);
        assert!(!needs_update_at(&files, &always_supported));
    }

    #[test]
    fn a_failing_hook_install_leaves_every_file_untouched() {
        let (_dir, files) = fixture();
        std::fs::write(&files.agents_md, legacy_agents_md("keep\n", Variant::Mcp)).unwrap();
        // Codex trust lives in config.toml; an unparsable one makes the trust sync fail.
        std::fs::write(&files.codex.config_toml, "[[[ not toml").unwrap();
        std::fs::write(&files.codex.hooks_json, "{\n  \"hooks\": {}\n}\n").unwrap();
        let before = everything(&files);
        assert!(install_at(&files, Mode::Mcp).is_err());
        assert_eq!(everything(&files), before);
        assert!(has_legacy_block_at(&files));
    }

    #[test]
    fn an_untrusted_hook_after_install_rolls_everything_back() {
        let (_dir, files) = fixture();
        std::fs::write(&files.agents_md, legacy_agents_md("keep\n", Variant::Cli)).unwrap();
        std::fs::write(&files.codex.config_toml, "model = \"x\"\n").unwrap();
        let before = everything(&files);
        let result = install_with(
            &files,
            Mode::Cli,
            &Steps {
                install_hook: &real_install_hook,
                verify: &|_, _| false,
                remove_block: &remove_legacy_block_at,
            },
        );
        let message = result.unwrap_err().to_string();
        assert!(message.contains("nothing was changed") || message.contains("未做任何改动"));
        assert_eq!(
            everything(&files),
            before,
            "hooks.json/config.toml restored, block kept"
        );
        assert!(
            read(&files.codex.hooks_json).is_none(),
            "a hooks.json created by the attempt is removed"
        );
    }

    #[test]
    fn a_failure_while_removing_the_block_undoes_the_already_installed_hook() {
        let (_dir, files) = fixture();
        std::fs::write(&files.agents_md, legacy_agents_md("keep\n", Variant::Cli)).unwrap();
        std::fs::write(&files.codex.config_toml, "model = \"x\"\n").unwrap();
        std::fs::write(&files.codex.hooks_json, "{\"hooks\":{}}").unwrap();
        let before = everything(&files);
        let result = install_with(
            &files,
            Mode::Cli,
            &Steps {
                install_hook: &real_install_hook,
                verify: &real_verify,
                remove_block: &|_| Err(anyhow!("disk full")),
            },
        );
        assert_eq!(result.unwrap_err().to_string(), "disk full");
        assert_eq!(everything(&files), before);
        assert!(!hook_installed_at(&files));
        assert!(
            has_legacy_block_at(&files),
            "there is never a moment with neither carrier"
        );
    }

    #[test]
    fn the_hook_is_installed_before_the_block_is_removed() {
        let (_dir, files) = fixture();
        std::fs::write(&files.agents_md, legacy_agents_md("", Variant::Cli)).unwrap();
        let order = std::cell::RefCell::new(Vec::new());
        install_with(
            &files,
            Mode::Cli,
            &Steps {
                install_hook: &|files, mode| {
                    order.borrow_mut().push("hook");
                    // The block is still there while the hook goes in.
                    assert!(has_legacy_block_at(files));
                    real_install_hook(files, mode)
                },
                verify: &|files, mode| {
                    order.borrow_mut().push("verify");
                    assert!(has_legacy_block_at(files));
                    real_verify(files, mode)
                },
                remove_block: &|files| {
                    order.borrow_mut().push("block");
                    assert!(hook_installed_at(files), "hook first");
                    remove_legacy_block_at(files)
                },
            },
        )
        .unwrap();
        assert_eq!(order.into_inner(), ["hook", "verify", "block"]);
    }

    #[test]
    fn none_is_not_a_delivery_mode() {
        let (_dir, files) = fixture();
        assert!(install_at(&files, Mode::None).is_err());
        assert!(everything(&files).iter().all(Option::is_none));
    }

    #[test]
    fn below_the_version_floor_nothing_is_offered_to_update() {
        let (_dir, files) = fixture();
        std::fs::write(&files.agents_md, legacy_agents_md("", Variant::Cli)).unwrap();
        assert!(needs_update_at(&files, &|| true));
        assert!(!needs_update_at(&files, &|| false));
        assert!(
            is_installed_at(&files),
            "the legacy block still counts as installed"
        );
        assert_eq!(current_mode_at(&files, false), Mode::Cli);
    }

    #[test]
    fn the_support_probe_is_not_consulted_when_nothing_is_stale() {
        let (_dir, files) = fixture();
        install_at(&files, Mode::Cli).unwrap();
        let probed = std::cell::Cell::new(false);
        let probe = || {
            probed.set(true);
            true
        };
        assert!(!needs_update_at(&files, &probe));
        assert!(
            !probed.get(),
            "a clean install must not spawn a login shell"
        );
    }

    #[test]
    fn a_hook_that_codex_no_longer_trusts_needs_an_update() {
        let (_dir, files) = fixture();
        install_at(&files, Mode::Cli).unwrap();
        assert!(!needs_update_at(&files, &always_supported));
        std::fs::write(&files.codex.config_toml, "").unwrap();
        assert!(needs_update_at(&files, &always_supported));
        // Reinstalling restores trust.
        install_at(&files, Mode::Cli).unwrap();
        assert!(!needs_update_at(&files, &always_supported));
    }

    #[test]
    fn delivery_classification_covers_every_state() {
        // (version_supported, hook_installed, trusted, stale, legacy_block) -> state
        use Delivery::*;
        for (inputs, expected) in [
            ((true, false, false, false, false), NotInstalled),
            ((true, false, false, false, true), NeedsUpdate),
            ((true, true, true, false, false), Trusted),
            ((true, true, true, true, false), NeedsUpdate),
            ((true, true, true, false, true), NeedsUpdate),
            ((true, true, false, false, false), Untrusted),
            // An untrusted hook is reported as such even if its shape also drifted.
            ((true, true, false, true, true), Untrusted),
            // The version floor outranks everything, including a healthy hook.
            ((false, true, true, false, false), UnsupportedVersion),
            ((false, false, false, false, true), UnsupportedVersion),
            ((false, false, false, false, false), UnsupportedVersion),
        ] {
            let (version, hook, trusted, stale, legacy) = inputs;
            assert_eq!(
                delivery_from(version, hook, trusted, stale, legacy),
                expected,
                "{inputs:?}"
            );
        }
    }

    #[test]
    fn delivery_follows_the_files_through_migration_and_trust_loss() {
        let (_dir, files) = fixture();
        let current = Some((0, 150, 0));
        assert_eq!(delivery_at(&files, current), Delivery::NotInstalled);

        std::fs::write(&files.agents_md, legacy_agents_md("", Variant::Cli)).unwrap();
        assert_eq!(delivery_at(&files, current), Delivery::NeedsUpdate);
        assert_eq!(
            delivery_at(&files, Some((0, 140, 0))),
            Delivery::UnsupportedVersion
        );

        install_at(&files, Mode::Cli).unwrap();
        assert_eq!(delivery_at(&files, current), Delivery::Trusted);
        assert_eq!(
            delivery_at(&files, None),
            Delivery::Trusted,
            "unknown version is supported"
        );

        // Codex forgets the trust entry: the hook still exists but never runs.
        std::fs::write(&files.codex.config_toml, "").unwrap();
        assert_eq!(delivery_at(&files, current), Delivery::Untrusted);

        install_at(&files, Mode::Cli).unwrap();
        assert_eq!(delivery_at(&files, current), Delivery::Trusted);
        uninstall_at(&files).unwrap();
        assert_eq!(delivery_at(&files, current), Delivery::NotInstalled);
    }

    #[test]
    fn a_missing_protocol_is_not_installed_rather_than_outdated() {
        let (_dir, files) = fixture();
        assert!(!is_installed_at(&files));
        assert!(!needs_update_at(&files, &always_supported));
        assert_eq!(current_mode_at(&files, false), Mode::None);
    }

    #[test]
    fn uninstall_removes_hook_trust_and_block_but_not_user_text() {
        let (_dir, files) = fixture();
        std::fs::write(&files.codex.config_toml, "model = \"x\"\n").unwrap();
        std::fs::write(&files.agents_md, legacy_agents_md("# mine\n", Variant::Cli)).unwrap();
        install_at(&files, Mode::Cli).unwrap();
        // Simulate the pre-migration leftover too: a block that came back.
        std::fs::write(&files.agents_md, legacy_agents_md("# mine\n", Variant::Cli)).unwrap();
        uninstall_at(&files).unwrap();
        assert!(!hook_installed_at(&files));
        assert!(!has_legacy_block_at(&files));
        let agents = std::fs::read_to_string(&files.agents_md).unwrap();
        assert!(agents.contains("# mine") && !agents.contains("AskHuman"));
        let config = std::fs::read_to_string(&files.codex.config_toml).unwrap();
        assert!(config.contains("model = \"x\""));
        assert!(!config.contains("trusted_hash"), "{config}");
        // And it is idempotent.
        uninstall_at(&files).unwrap();
    }

    #[test]
    fn collaboration_style_refresh_rewrites_the_block_text_without_migrating() {
        let (_dir, files) = fixture();
        let stale =
            agent_rules::upsert_block("# mine\n", "old wording: invoke it via the Shell/Bash tool");
        std::fs::write(&files.agents_md, &stale).unwrap();

        refresh_legacy_block_at(&files).unwrap();

        let text = std::fs::read_to_string(&files.agents_md).unwrap();
        assert_eq!(
            agent_rules::block_body(&text).as_deref(),
            Some(Variant::Cli.body(AgentTarget::Codex).as_str())
        );
        assert!(text.starts_with("# mine"));
        assert!(!hook_installed_at(&files), "refreshing is not migrating");
        assert!(read(&files.codex.hooks_json).is_none());
        assert!(read(&files.codex.config_toml).is_none());

        // The MCP flavour keeps its variant.
        let stale_mcp = agent_rules::upsert_block("", "old wording: call the AskHuman `ask` tool");
        std::fs::write(&files.agents_md, stale_mcp).unwrap();
        refresh_legacy_block_at(&files).unwrap();
        assert_eq!(legacy_variant_at(&files), Some(Variant::Mcp));
        let text = std::fs::read_to_string(&files.agents_md).unwrap();
        assert_eq!(
            agent_rules::block_body(&text).as_deref(),
            Some(Variant::Mcp.body(AgentTarget::Codex).as_str())
        );
    }

    #[test]
    fn collaboration_style_refresh_is_a_no_op_after_migration() {
        let (_dir, files) = fixture();
        refresh_legacy_block_at(&files).unwrap();
        assert!(!files.agents_md.exists());
        std::fs::write(&files.agents_md, "# only user text\n").unwrap();
        refresh_legacy_block_at(&files).unwrap();
        assert_eq!(
            std::fs::read_to_string(&files.agents_md).unwrap(),
            "# only user text\n"
        );
    }
}
