//! MCP 配置集成：把 `askhuman` server 写入三家 Agent 的**用户级全局** MCP 配置，
//! 供 MCP 模式（`AskHuman mcp`）被客户端拉起。
//!
//! 落点（spec D7）：
//! - Cursor：`~/.cursor/mcp.json`（JSON，`mcpServers.askhuman`；其 MCP 超时硬编码 ~60s 且不可配置，
//!   不写 `timeout`）。
//! - Claude Code：`~/.claude.json`（JSON，top-level `mcpServers.askhuman`；文件大、含项目历史，必须最小化
//!   编辑。写 `timeout`(毫秒) 覆盖其 60s 默认，否则长等待会被取消）。
//! - Codex：`~/.codex/config.toml`（TOML，`[mcp_servers.askhuman]`，含大 `tool_timeout_sec`；另将
//!   `mcp__askhuman` 追加进 `[features.code_mode].direct_only_tool_namespaces`，强制模型顶层直调）。
//!
//! 一律沿用现有 hook/rule 集成的**纯函数 + 最小化编辑（CST/`toml_edit`）+ 解析失败即中止不覆盖 + 单测**
//! 范式：只触碰自有 `askhuman` 条目，保留用户其它内容 / 注释 / 键序。Codex direct-only 的所有权单独记在
//! `~/.askhuman/integration-state.json`：预存同名项不认领，卸载只移除本应用实际追加的项。`command` 写当前
//! 可执行文件绝对路径（D16，部分客户端不继承 shell PATH）。

use crate::integrations::agent_rules::AgentTarget;
use crate::paths;
use anyhow::{anyhow, Context, Result};
use jsonc_parser::cst::CstRootNode;
use jsonc_parser::json;
use jsonc_parser::ParseOptions;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 各家配置中本 server 的名字（恒 `askhuman`，spec D15）。
pub const SERVER_NAME: &str = "askhuman";
/// 启动子命令（`AskHuman mcp`）。
pub const ARG_MCP: &str = "mcp";
/// Codex Code Mode 中必须由模型顶层直调的 MCP namespace。
pub const CODEX_DIRECT_ONLY_NAMESPACE: &str = "mcp__askhuman";
/// Codex MCP server 启动超时（秒）。
pub const CODEX_STARTUP_TIMEOUT_SEC: i64 = 30;
/// Codex MCP 工具调用超时（秒）：取很大值，等待人类回应不被取消（spec D6）。
pub const CODEX_TOOL_TIMEOUT_SEC: i64 = 86400;
/// Claude Code（CLI）MCP 工具调用超时（**毫秒**）：写入 `mcpServers.askhuman.timeout`，覆盖其
/// 默认 60s（MCP TS SDK `DEFAULT_REQUEST_TIMEOUT_MSEC`），否则等待人类超过 60s 会被 `-32001` 取消。
/// 取 24h，与 Codex 的 `tool_timeout_sec`(86400s) 对齐。Cursor 的 MCP 超时硬编码 ~60s、不可配置，
/// 故仅给 Claude 写。
pub const CLAUDE_TOOL_TIMEOUT_MS: i64 = 86_400_000;
/// Grok MCP server 启动超时（秒）。
pub const GROK_STARTUP_TIMEOUT_SEC: i64 = 30;
/// Grok MCP 工具调用总超时（秒），24h。
pub const GROK_TOOL_TIMEOUT_SEC: i64 = 86400;
/// Grok 针对 `ask` 工具的 per-tool 超时（秒），24h：`tool_timeouts = { ask = 86400 }`。
/// 对默认模型 Composer 的 per-tool 语义更精准（spec D6 / 调研结论）。
pub const GROK_ASK_TOOL_TIMEOUT_SEC: i64 = 86400;
/// Antigravity MCP timeout budget in seconds: 24 hours.
/// Write the native timeoutSeconds field to allow extended human waits.
pub const ANTIGRAVITY_TOOL_TIMEOUT_SEC: i64 = 86400;

/// A bounded config edit used while the Antigravity rule transaction is reconciled.
/// Comparisons protect ordinary external edits, not an atomic compare-and-swap against writers.
pub(crate) struct PreparedAntigravityConfig {
    path: PathBuf,
    before: Option<String>,
    after: Option<String>,
}

fn read_optional_config(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("cannot read MCP config {}", path.display()))
        }
    }
}

fn validate_antigravity_json(text: &str) -> Result<Value> {
    // An empty file has historically been treated as an empty configuration.
    let value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        jsonc_parser::parse_to_serde_value::<Value>(text, &ParseOptions::default())
            .context("cannot parse Antigravity MCP config; original file preserved")?
    };
    if !value.is_object()
        || value
            .get("mcpServers")
            .is_some_and(|servers| !servers.is_object())
    {
        return Err(anyhow!(
            "Antigravity MCP config and mcpServers must be JSON objects"
        ));
    }
    Ok(value)
}

impl PreparedAntigravityConfig {
    pub(crate) fn prepare(path: PathBuf, install: bool, command: &str) -> Result<Self> {
        let before = read_optional_config(&path)?;
        let value = validate_antigravity_json(before.as_deref().unwrap_or_default())
            .with_context(|| format!("MCP config: {}", path.display()))?;
        let after = if install {
            json_entry_env(&value)?;
            let timeout = JsonTimeout::TimeoutSeconds(ANTIGRAVITY_TOOL_TIMEOUT_SEC);
            // A matching declarative config needs no publication, regardless of formatting.
            if json_entry_matches(&value, command, timeout) {
                before.clone()
            } else {
                Some(apply_install_json(
                    before.as_deref().unwrap_or_default(),
                    command,
                    timeout,
                )?)
            }
        } else if json_entry(&value).is_none() {
            before.clone()
        } else {
            before.as_deref().map(apply_uninstall_json).transpose()?
        };
        Ok(Self {
            path,
            before,
            after,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    fn replace_if_unchanged(
        &self,
        expected: &Option<String>,
        desired: &Option<String>,
    ) -> Result<()> {
        if read_optional_config(&self.path)? != *expected {
            return Err(anyhow!(
                "MCP config changed outside this operation; preserving {}",
                self.path.display()
            ));
        }
        if expected == desired {
            return Ok(());
        }
        match desired {
            Some(text) => write_antigravity_text(&self.path, text, expected),
            None => crate::development::migration::remove_file(&self.path).map_err(Into::into),
        }
    }

    pub(crate) fn apply(&self) -> Result<()> {
        self.replace_if_unchanged(&self.before, &self.after)
    }

    pub(crate) fn restore(&self) -> Result<()> {
        self.replace_if_unchanged(&self.after, &self.before)
    }
}

#[cfg(unix)]
fn write_antigravity_text(path: &Path, text: &str, expected: &Option<String>) -> Result<()> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let identity = |meta: &std::fs::Metadata| {
        (
            meta.dev(),
            meta.ino(),
            meta.uid(),
            meta.gid(),
            meta.mode(),
            meta.nlink(),
        )
    };
    let before = match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            // Preserve link semantics: automatic publication never replaces a linked config.
            if meta.file_type().is_symlink() {
                return Err(anyhow!(
                    "refusing to modify symlinked MCP config {}; link and target preserved; edit mcpServers.askhuman in your configuration manager's source (stow/chezmoi), or declare it in Home Manager and rebuild for Nix store targets; required fields: command=<absolute AskHuman path>, args=[\"mcp\"], timeoutSeconds=86400",
                    path.display()
                ));
            }
            if !meta.is_file() {
                return Err(anyhow!(
                    "MCP config {} is not a regular file; provide a user-owned regular configuration file",
                    path.display()
                ));
            }
            if meta.uid() != unsafe { libc::geteuid() } {
                return Err(anyhow!(
                    "MCP config {} is not owned by the current user; ask its owner to manage mcpServers.askhuman or provide a user-owned regular file; do not run AskHuman with sudo",
                    path.display()
                ));
            }
            if meta.nlink() != 1 {
                return Err(anyhow!(
                    "refusing to replace hard-linked MCP config {}; all links preserved; manage mcpServers.askhuman manually or provide a separate user-owned regular file with a single link",
                    path.display()
                ));
            }
            Some(meta)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("MCP config has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    // NamedTempFile creates a private 0600 file; explicitly set it before writing any secret bytes.
    let mut staged = tempfile::Builder::new()
        .prefix(".askhuman-mcp-")
        .tempfile_in(parent)?;
    staged
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    if let Some(meta) = &before {
        // Preserve the validated owner/group; failure leaves the original file in place.
        if unsafe { libc::fchown(staged.as_file().as_raw_fd(), meta.uid(), meta.gid()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // Preserve stricter owner permissions and never grant group/other access.
        staged
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(meta.mode() & 0o600))?;
    }
    staged.write_all(text.as_bytes())?;
    staged.as_file().sync_all()?;
    let current = match std::fs::symlink_metadata(path) {
        Ok(meta) => Some(meta),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current.as_ref().map(identity) != before.as_ref().map(identity)
        || read_optional_config(path)? != *expected
    {
        return Err(anyhow!(
            "MCP config changed before publication; preserving {}",
            path.display()
        ));
    }
    crate::development::migration::before_write(path, Some(text.as_bytes()))?;
    staged
        .persist(path)
        .with_context(|| format!("cannot publish private MCP config {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_antigravity_text(_: &Path, _: &str, _: &Option<String>) -> Result<()> {
    Err(anyhow!(
        "Antigravity MCP config mutation is unsupported on this platform"
    ))
}

pub(crate) fn prepare_antigravity_config(install: bool) -> Result<PreparedAntigravityConfig> {
    let path = paths::antigravity_mcp_config_json();
    PreparedAntigravityConfig::prepare(path.clone(), install, &current_exe_string()?).map_err(
        |error| {
            anyhow!(
                "cannot prepare Antigravity MCP config {}: {error:#}",
                path.display()
            )
        },
    )
}

/// Unlike compatibility bool helpers, public Antigravity status must not hide malformed files.
pub(crate) fn antigravity_config_status() -> Result<(bool, bool)> {
    let path = paths::antigravity_mcp_config_json();
    let text = read_optional_config(&path)
        .map_err(|error| anyhow!("cannot read Antigravity MCP config: {error:#}"))?;
    let value = validate_antigravity_json(text.as_deref().unwrap_or_default())
        .map_err(|error| anyhow!("MCP config {}: {error:#}", path.display()))?;
    let installed = json_entry(&value).is_some();
    let outdated = installed
        && !json_entry_matches(
            &value,
            &current_exe_string()?,
            json_timeout(AgentTarget::Antigravity),
        );
    Ok((installed, outdated))
}

/// AskHuman 自有的自动集成产物所有权账本。
///
/// `flatten` 保留未来版本增加的未知字段；该文件不是用户设置，不参与 `AppConfig` 的读写。
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct IntegrationState {
    codex: CodexIntegrationState,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CodexIntegrationState {
    direct_only_namespace_added_by_askhuman: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug)]
struct TomlInstallOutcome {
    text: String,
    direct_only_added: bool,
}

/// 某 TOML 目标（Codex / Grok）要写入的超时字段集：`(startup_timeout_sec, tool_timeout_sec, tool_timeouts.ask?)`。
/// Grok 额外写 per-tool `ask` 超时；Codex 不写（`ask` 位为 None）。
fn toml_profile(target: AgentTarget) -> (i64, i64, Option<i64>) {
    match target {
        AgentTarget::Grok => (
            GROK_STARTUP_TIMEOUT_SEC,
            GROK_TOOL_TIMEOUT_SEC,
            Some(GROK_ASK_TOOL_TIMEOUT_SEC),
        ),
        _ => (CODEX_STARTUP_TIMEOUT_SEC, CODEX_TOOL_TIMEOUT_SEC, None),
    }
}

/// Timeout field and units for each JSON client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum JsonTimeout {
    None,
    TimeoutMs(i64),
    TimeoutSeconds(i64),
}

fn json_timeout(target: AgentTarget) -> JsonTimeout {
    match target {
        AgentTarget::ClaudeCode => JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS),
        AgentTarget::Antigravity => JsonTimeout::TimeoutSeconds(ANTIGRAVITY_TOOL_TIMEOUT_SEC),
        _ => JsonTimeout::None,
    }
}

/// 配置文件格式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Json,
    Toml,
}

fn format_of(target: AgentTarget) -> Format {
    match target {
        AgentTarget::Codex | AgentTarget::Grok => Format::Toml,
        AgentTarget::Cursor | AgentTarget::ClaudeCode | AgentTarget::Antigravity => Format::Json,
        AgentTarget::Pi => Format::Json,
    }
}

/// 目标配置文件路径。
fn config_path(target: AgentTarget) -> PathBuf {
    match target {
        AgentTarget::Cursor => paths::cursor_mcp_json(),
        AgentTarget::ClaudeCode => paths::claude_json(),
        AgentTarget::Codex => paths::codex_config_toml(),
        AgentTarget::Grok => paths::grok_config_toml(),
        AgentTarget::Pi => paths::config_dir().join("unsupported-pi-mcp.json"),
        AgentTarget::Antigravity => paths::antigravity_mcp_config_json(),
    }
}

/// Whether MCP integration is available for this target (Pi has no MCP mode).
pub fn supported(target: AgentTarget) -> bool {
    !matches!(target, AgentTarget::Pi)
}

/// 配置展示路径（home 前缀折叠为 `~`）。
pub fn display_path(target: AgentTarget) -> String {
    collapse_home(&config_path(target))
}

/// 是否已写入本 server 条目。
pub fn is_installed(target: AgentTarget) -> bool {
    if !supported(target) {
        return false;
    }
    let path = config_path(target);
    match format_of(target) {
        Format::Json => read_json_value(&path)
            .map(|v| json_entry(&v).is_some())
            .unwrap_or(false),
        Format::Toml => std::fs::read_to_string(&path)
            .ok()
            .map(|t| toml_installed(&t))
            .unwrap_or(false),
    }
}

/// 已安装但内容（command 绝对路径 / args / Codex 超时）与最新模板不一致 → 需更新。
pub fn needs_update(target: AgentTarget) -> bool {
    if !supported(target) {
        return false;
    }
    if !is_installed(target) {
        return false;
    }
    let Ok(exe) = current_exe_string() else {
        return false;
    };
    let path = config_path(target);
    match format_of(target) {
        Format::Json => read_json_value(&path)
            .map(|v| !json_entry_matches(&v, &exe, json_timeout(target)))
            .unwrap_or(false),
        Format::Toml => std::fs::read_to_string(&path)
            .map(|t| !toml_entry_matches(target, &t, &exe))
            .unwrap_or(false),
    }
}

/// 安装：写入 / 更新本 server 条目（最小化编辑，保留用户其它内容）。
pub fn install(target: AgentTarget) -> Result<String> {
    if !supported(target) {
        return Err(anyhow!("MCP integration is unsupported for Pi"));
    }
    write_entry(target, &std::collections::BTreeMap::new())?;
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigInstalled").to_string())
}

pub(crate) fn install_with_env(
    target: AgentTarget,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<String> {
    if target != AgentTarget::Codex && !env.is_empty() {
        return Err(anyhow!("environment overrides require Codex MCP"));
    }
    write_entry(target, env)?;
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigInstalled").to_string())
}

/// 更新：与安装同样写入逻辑，仅反馈文案不同。
pub fn update(target: AgentTarget) -> Result<String> {
    if !supported(target) {
        return Err(anyhow!("MCP integration is unsupported for Pi"));
    }
    write_entry(target, &std::collections::BTreeMap::new())?;
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigUpdated").to_string())
}

/// 卸载：移除本 server 条目（保留用户其它条目）；条目本就不存在则 no-op。
pub fn uninstall(target: AgentTarget) -> Result<String> {
    if !supported(target) {
        return Ok(
            crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigRemoved").to_string(),
        );
    }
    let path = config_path(target);
    if target == AgentTarget::Antigravity {
        prepare_antigravity_config(false)?.apply()?;
        return Ok(
            crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigRemoved").to_string(),
        );
    }
    if target == AgentTarget::Codex && matches!(path.try_exists(), Ok(false)) {
        clear_codex_ownership_without_config(&paths::integration_state_file())?;
    }
    if let Ok(text) = std::fs::read_to_string(&path) {
        let updated = match format_of(target) {
            Format::Json => apply_uninstall_json(&text)?,
            Format::Toml if target == AgentTarget::Codex => {
                uninstall_codex_toml(&path, &text)?;
                return Ok(
                    crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigRemoved")
                        .to_string(),
                );
            }
            Format::Toml => apply_uninstall_toml(&text, false)?,
        };
        write_text(&path, &updated)?;
    }
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigRemoved").to_string())
}

fn write_entry(
    target: AgentTarget,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    if target == AgentTarget::Antigravity {
        return prepare_antigravity_config(true)?.apply();
    }
    let exe = current_exe_string()?;
    let path = config_path(target);
    let original = std::fs::read_to_string(&path);
    let original_exists = original.is_ok();
    let text = original.unwrap_or_default();
    let updated = match format_of(target) {
        Format::Json => apply_install_json(&text, &exe, json_timeout(target))?,
        Format::Toml if target == AgentTarget::Codex => {
            if env.is_empty() {
                install_codex_toml(&path, &text, original_exists, &exe)?;
            } else {
                install_codex_toml_with_state_env(
                    &path,
                    &paths::integration_state_file(),
                    &text,
                    original_exists,
                    &exe,
                    env,
                )?;
            }
            return Ok(());
        }
        Format::Toml => apply_install_toml(target, &text, &exe)?,
    };
    write_text(&path, &updated)
}

/// Codex 的 MCP server 与 direct-only 是一个安装事务；只有本次实际追加 direct-only 时才认领所有权。
fn install_codex_toml(
    path: &Path,
    original: &str,
    original_exists: bool,
    command: &str,
) -> Result<()> {
    let state_path = paths::integration_state_file();
    install_codex_toml_with_state(path, &state_path, original, original_exists, command)
}

fn install_codex_toml_with_state(
    path: &Path,
    state_path: &Path,
    original: &str,
    original_exists: bool,
    command: &str,
) -> Result<()> {
    install_codex_toml_with_state_env(
        path,
        state_path,
        original,
        original_exists,
        command,
        &std::collections::BTreeMap::new(),
    )
}

fn install_codex_toml_with_state_env(
    path: &Path,
    state_path: &Path,
    original: &str,
    original_exists: bool,
    command: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    // 安装前必须能读取所有权账本。损坏时中止，避免误认领用户的同名配置。
    let mut state = load_integration_state(state_path)?;
    let outcome = apply_install_toml_outcome_env(AgentTarget::Codex, original, command, env)?;

    write_text(path, &outcome.text)?;
    if outcome.direct_only_added && !state.codex.direct_only_namespace_added_by_askhuman {
        state.codex.direct_only_namespace_added_by_askhuman = true;
        if let Err(error) = save_integration_state(state_path, &state) {
            rollback_text(path, &outcome.text, original, original_exists)
                .with_context(|| format!("保存集成所有权失败（{error}），且回滚 Codex 配置失败"))?;
            return Err(error.context("保存集成所有权失败，已回滚 Codex 配置"));
        }
    }
    Ok(())
}

/// Codex 卸载只在账本明确归 AskHuman 所有时移除 direct-only；账本损坏/缺失一律保守保留。
fn uninstall_codex_toml(path: &Path, original: &str) -> Result<()> {
    let state_path = paths::integration_state_file();
    uninstall_codex_toml_with_state(path, &state_path, original)
}

fn uninstall_codex_toml_with_state(path: &Path, state_path: &Path, original: &str) -> Result<()> {
    let loaded = load_integration_state(state_path);
    let owned = loaded
        .as_ref()
        .map(|state| state.codex.direct_only_namespace_added_by_askhuman)
        .unwrap_or(false);
    let updated = apply_uninstall_toml(original, owned)?;
    write_text(path, &updated)?;

    if let Ok(mut state) = loaded {
        if owned {
            state.codex.direct_only_namespace_added_by_askhuman = false;
            if let Err(error) = save_integration_state(state_path, &state) {
                rollback_text(path, &updated, original, true).with_context(|| {
                    format!("清除集成所有权失败（{error}），且回滚 Codex 配置失败")
                })?;
                return Err(error.context("清除集成所有权失败，已回滚 Codex 配置"));
            }
        }
    }
    Ok(())
}

/// Codex 配置已不存在时 direct-only 也不存在，可安全清除旧所有权；损坏账本仍不覆盖。
fn clear_codex_ownership_without_config(state_path: &Path) -> Result<()> {
    let Ok(mut state) = load_integration_state(state_path) else {
        return Ok(());
    };
    if state.codex.direct_only_namespace_added_by_askhuman {
        state.codex.direct_only_namespace_added_by_askhuman = false;
        save_integration_state(state_path, &state)?;
    }
    Ok(())
}

/// 在文件管理器中定位配置文件。
pub fn reveal(target: AgentTarget) {
    let path = config_path(target);
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", &path.to_string_lossy()])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.clone());
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .spawn();
    }
}

/// 用系统默认程序打开配置文件。
pub fn open(target: AgentTarget) {
    let path = config_path(target);
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(&path)
            .spawn();
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    }
}

// MARK: - JSON (Cursor / Claude / Antigravity): minimal format-preserving CST edits

/// upsert `mcpServers.askhuman = { command, args:["mcp"], timeout? }`；仅触碰本条目，其余字节保留。
/// Claude uses timeout in milliseconds, Antigravity uses timeoutSeconds, and Cursor uses None.
/// 解析失败返回 Err（调用方据此中止、绝不整写覆盖）。
fn apply_install_json(text: &str, command: &str, timeout: JsonTimeout) -> Result<String> {
    let source = if text.trim().is_empty() { "{}" } else { text };
    let root = CstRootNode::parse(source, &ParseOptions::default())
        .map_err(|e| anyhow!("解析 MCP 配置 JSON 失败，已中止（不覆盖原文件）：{e}"))?;
    let root_obj = root
        .object_value_or_create()
        .ok_or_else(|| anyhow!("MCP 配置根不是 JSON 对象，已中止"))?;
    let servers = root_obj
        .object_value_or_create("mcpServers")
        .ok_or_else(|| anyhow!("MCP 配置的 `mcpServers` 不是对象，已中止"))?;

    let mut entry = match timeout {
        JsonTimeout::TimeoutMs(ms) => json!({
            "command": command,
            "args": [ARG_MCP],
            "timeout": ms,
        }),
        JsonTimeout::TimeoutSeconds(sec) => json!({
            "command": command,
            "args": [ARG_MCP],
            "timeoutSeconds": sec,
        }),
        JsonTimeout::None => json!({
            "command": command,
            "args": [ARG_MCP],
        }),
    };
    if let jsonc_parser::cst::CstInputValue::Object(ref mut properties) = entry {
        let args = properties
            .iter_mut()
            .find(|(name, _)| name == "args")
            .unwrap();
        args.1 = jsonc_parser::cst::CstInputValue::Array(
            crate::development::routing::integration_args(&[ARG_MCP])
                .into_iter()
                .map(jsonc_parser::cst::CstInputValue::String)
                .collect(),
        );
    }
    let parsed: Value = jsonc_parser::parse_to_serde_value(source, &ParseOptions::default())?;
    if let Some(obj) = json_entry_env(&parsed)? {
        if let jsonc_parser::cst::CstInputValue::Object(ref mut properties) = entry {
            properties.push((
                "env".into(),
                jsonc_parser::cst::CstInputValue::Object(
                    obj.iter()
                        .map(|(name, value)| {
                            (
                                name.clone(),
                                jsonc_parser::cst::CstInputValue::String(
                                    value.as_str().unwrap().to_string(),
                                ),
                            )
                        })
                        .collect(),
                ),
            ));
        }
    }
    match servers.get(SERVER_NAME) {
        Some(prop) => {
            prop.set_value(entry);
        }
        None => {
            servers.ensure_multiline();
            servers.append(SERVER_NAME, entry);
        }
    }
    Ok(root.to_string())
}

/// 移除 `mcpServers.askhuman`；若 `mcpServers` 因此变空则删除该键。其余内容原样保留。
fn apply_uninstall_json(text: &str) -> Result<String> {
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|e| anyhow!("解析 MCP 配置 JSON 失败，已中止（不覆盖原文件）：{e}"))?;
    let Some(root_obj) = root.object_value() else {
        return Ok(root.to_string());
    };
    if let Some(servers) = root_obj.object_value("mcpServers") {
        if let Some(prop) = servers.get(SERVER_NAME) {
            prop.remove();
        }
        if servers.properties().is_empty() {
            if let Some(prop) = root_obj.get("mcpServers") {
                prop.remove();
            }
        }
    }
    Ok(root.to_string())
}

/// 以 JSONC 解析为 serde 值（供状态查询，与客户端解析语义一致）。
fn read_json_value(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    jsonc_parser::parse_to_serde_value::<Value>(&text, &ParseOptions::default()).ok()
}

fn json_entry(value: &Value) -> Option<&Value> {
    value.get("mcpServers")?.get(SERVER_NAME)
}

fn json_entry_env(value: &Value) -> Result<Option<&serde_json::Map<String, Value>>> {
    let Some(env) = json_entry(value).and_then(|entry| entry.get("env")) else {
        return Ok(None);
    };
    let object = env
        .as_object()
        .ok_or_else(|| anyhow!("MCP env must be an object"))?;
    for (name, value) in object {
        if !valid_env_name(name) || value.as_str().is_none_or(|v| v.contains('\0')) {
            return Err(anyhow!(
                "MCP env must contain valid names and string values without NUL"
            ));
        }
    }
    Ok(Some(object))
}

fn json_entry_matches(value: &Value, command: &str, timeout: JsonTimeout) -> bool {
    let Some(entry) = json_entry(value) else {
        return false;
    };
    let cmd_ok = entry.get("command").and_then(|v| v.as_str()) == Some(command);
    let args_ok = entry
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            let expected = crate::development::routing::integration_args(&[ARG_MCP]);
            a.len() == expected.len()
                && a.iter()
                    .zip(expected.iter())
                    .all(|(actual, expected)| actual.as_str() == Some(expected.as_str()))
        })
        .unwrap_or(false);
    // Require the client's exact timeout field and units; Cursor must omit timeout fields.
    let timeout_ok = match timeout {
        JsonTimeout::TimeoutMs(ms) => entry.get("timeout").and_then(|v| v.as_i64()) == Some(ms),
        JsonTimeout::TimeoutSeconds(sec) => {
            entry.get("timeoutSeconds").and_then(|v| v.as_i64()) == Some(sec)
        }
        JsonTimeout::None => {
            entry.get("timeout").is_none() && entry.get("timeoutSeconds").is_none()
        }
    };
    cmd_ok && args_ok && timeout_ok
}

// MARK: - TOML（Codex）：toml_edit 保留格式最小化编辑

/// upsert `[mcp_servers.askhuman]`（command/args/startup_timeout_sec/tool_timeout_sec，
/// Grok 额外写 `tool_timeouts = { ask = 86400 }`）。
fn apply_install_toml(target: AgentTarget, text: &str, command: &str) -> Result<String> {
    Ok(apply_install_toml_outcome(target, text, command)?.text)
}

fn apply_install_toml_outcome(
    target: AgentTarget,
    text: &str,
    command: &str,
) -> Result<TomlInstallOutcome> {
    apply_install_toml_outcome_env(target, text, command, &std::collections::BTreeMap::new())
}

fn apply_install_toml_outcome_env(
    target: AgentTarget,
    text: &str,
    command: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<TomlInstallOutcome> {
    use toml_edit::{value, Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};
    for (name, val) in env {
        if !valid_env_name(name) || val.contains('\0') {
            return Err(anyhow!("invalid environment override"));
        }
    }
    let (startup, tool, ask) = toml_profile(target);
    let mut doc = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse::<DocumentMut>()
            .map_err(|e| anyhow!("解析 config.toml 失败，已中止（不覆盖原文件）：{e}"))?
    };

    if !doc.as_table().contains_key("mcp_servers") {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.as_table_mut().insert("mcp_servers", Item::Table(t));
    }
    let servers = doc
        .as_table_mut()
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("config.toml 中 `mcp_servers` 不是表，已中止"))?;

    if !servers.contains_key(SERVER_NAME) {
        servers.insert(SERVER_NAME, Item::Table(Table::new()));
    }
    let entry = servers
        .get_mut(SERVER_NAME)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("config.toml 中 `mcp_servers.askhuman` 不是表，已中止"))?;

    if let Some(existing) = entry.get("env") {
        let table = existing
            .as_table_like()
            .ok_or_else(|| anyhow!("MCP env must be a table"))?;
        for (name, value) in table.iter() {
            if !valid_env_name(name) || value.as_str().is_none_or(|v| v.contains('\0')) {
                return Err(anyhow!(
                    "MCP env must contain valid names and string values without NUL"
                ));
            }
        }
    }
    if !env.is_empty() {
        if !entry.contains_key("env") {
            entry.insert("env", Item::Table(Table::new()));
        }
        let table = entry
            .get_mut("env")
            .and_then(Item::as_table_like_mut)
            .unwrap();
        for (name, val) in env {
            table.insert(name, value(val.as_str()));
        }
    }
    entry.insert("command", value(command));
    let mut args = Array::new();
    for arg in crate::development::routing::integration_args(&[ARG_MCP]) {
        args.push(arg);
    }
    entry.insert("args", value(args));
    entry.insert("startup_timeout_sec", value(startup));
    entry.insert("tool_timeout_sec", value(tool));
    if target == AgentTarget::Codex {
        // AskHuman before v0.15 wrote `approval_mode` straight under the server table. Codex
        // only reads `default_tools_approval_mode` there (`approval_mode` is a per-tool key
        // under `tools.<name>`), so it ignores the old key and warns about it at every start.
        // The key was ours, so drop it; the tool tables and other servers keep theirs. Its
        // presence alone is not flagged as outdated either: `needs_update` gates task readiness.
        entry.remove("approval_mode");
        // Skip per-call approval for our own ask tools. Written only when the key is absent —
        // a user-edited value is their call and is never touched or flagged as outdated
        // (`toml_entry_matches` deliberately ignores this key).
        if !entry.contains_key("default_tools_approval_mode") {
            entry.insert("default_tools_approval_mode", value("approve"));
        }
    }
    // Grok：per-tool 超时 `tool_timeouts = { ask = 86400 }`（内联表，对 Composer 更精准）。
    match ask {
        Some(secs) => {
            let mut t = InlineTable::new();
            t.insert("ask", TomlValue::from(secs));
            entry.insert("tool_timeouts", value(t));
        }
        None => {
            entry.remove("tool_timeouts");
        }
    }
    let direct_only_added = if target == AgentTarget::Codex {
        ensure_codex_direct_only(&mut doc)?
    } else {
        false
    };
    Ok(TomlInstallOutcome {
        text: doc.to_string(),
        direct_only_added,
    })
}

/// 移除 `[mcp_servers.askhuman]`；若 `mcp_servers` 因此变空则删除该表。
/// `remove_codex_direct_only` 只可来自所有权账本的明确 true；移除后保留数组键和父表。
fn apply_uninstall_toml(text: &str, remove_codex_direct_only: bool) -> Result<String> {
    use toml_edit::{DocumentMut, Item};
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }
    let mut doc = text
        .parse::<DocumentMut>()
        .map_err(|e| anyhow!("解析 config.toml 失败，已中止（不覆盖原文件）：{e}"))?;
    if let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) {
        servers.remove(SERVER_NAME);
        if servers.is_empty() {
            doc.as_table_mut().remove("mcp_servers");
        }
    }
    if remove_codex_direct_only {
        remove_owned_codex_direct_only(&mut doc)?;
    }
    Ok(doc.to_string())
}

/// 确保 `[features.code_mode].direct_only_tool_namespaces` 含 AskHuman namespace。
/// 返回 true 仅表示本次确实追加，供所有权账本认领；预存同名项返回 false。
fn ensure_codex_direct_only(doc: &mut toml_edit::DocumentMut) -> Result<bool> {
    use toml_edit::{value, Array, Item, Table};

    if !doc.as_table().contains_key("features") {
        let mut table = Table::new();
        table.set_implicit(true);
        doc.as_table_mut().insert("features", Item::Table(table));
    }
    let features = doc
        .as_table_mut()
        .get_mut("features")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("config.toml 中 `features` 不是表，已中止"))?;

    if !features.contains_key("code_mode") {
        features.insert("code_mode", Item::Table(Table::new()));
    }
    let code_mode = features
        .get_mut("code_mode")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("config.toml 中 `features.code_mode` 不是表，已中止"))?;

    const KEY: &str = "direct_only_tool_namespaces";
    if let Some(item) = code_mode.get_mut(KEY) {
        let array = item.as_array_mut().ok_or_else(|| {
            anyhow!("config.toml 中 `features.code_mode.{KEY}` 不是字符串数组，已中止")
        })?;
        if array.iter().any(|value| value.as_str().is_none()) {
            return Err(anyhow!(
                "config.toml 中 `features.code_mode.{KEY}` 不是字符串数组，已中止"
            ));
        }
        if array
            .iter()
            .any(|value| value.as_str() == Some(CODEX_DIRECT_ONLY_NAMESPACE))
        {
            return Ok(false);
        }
        array.push(CODEX_DIRECT_ONLY_NAMESPACE);
        return Ok(true);
    }

    let mut array = Array::new();
    array.push(CODEX_DIRECT_ONLY_NAMESPACE);
    code_mode.insert(KEY, value(array));
    Ok(true)
}

fn codex_direct_only_matches(doc: &toml_edit::DocumentMut) -> bool {
    doc.get("features")
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|features| features.get("code_mode"))
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|code_mode| code_mode.get("direct_only_tool_namespaces"))
        .and_then(toml_edit::Item::as_array)
        .map(|array| {
            array.iter().all(|value| value.as_str().is_some())
                && array
                    .iter()
                    .any(|value| value.as_str() == Some(CODEX_DIRECT_ONLY_NAMESPACE))
        })
        .unwrap_or(false)
}

fn remove_owned_codex_direct_only(doc: &mut toml_edit::DocumentMut) -> Result<bool> {
    use toml_edit::Item;
    let Some(code_mode) = doc
        .get_mut("features")
        .and_then(Item::as_table_mut)
        .and_then(|features| features.get_mut("code_mode"))
        .and_then(Item::as_table_mut)
    else {
        return Ok(false);
    };
    let Some(item) = code_mode.get_mut("direct_only_tool_namespaces") else {
        return Ok(false);
    };
    let array = item.as_array_mut().ok_or_else(|| {
        anyhow!(
            "config.toml 中 `features.code_mode.direct_only_tool_namespaces` 不是字符串数组，已中止"
        )
    })?;
    if array.iter().any(|value| value.as_str().is_none()) {
        return Err(anyhow!(
            "config.toml 中 `features.code_mode.direct_only_tool_namespaces` 不是字符串数组，已中止"
        ));
    }
    let Some(index) = array
        .iter()
        .position(|value| value.as_str() == Some(CODEX_DIRECT_ONLY_NAMESPACE))
    else {
        return Ok(false);
    };
    array.remove(index);
    Ok(true)
}

fn toml_installed(text: &str) -> bool {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    toml_entry(&doc).is_some()
}

fn toml_entry(doc: &toml_edit::DocumentMut) -> Option<&dyn toml_edit::TableLike> {
    doc.get("mcp_servers")?
        .as_table_like()?
        .get(SERVER_NAME)?
        .as_table_like()
}

/// 把 TOML 数值读成 i64，**同时容忍整数与整值浮点**：Codex / Grok 的 CLI 会在改写 `config.toml`
/// 时把 `30` 归一化成 `30.0`。若只按 `as_integer()` 比较，写进去的整数被对方改成浮点后就会被判「需更新」，
/// 陷入「更新→被归一化→又需更新」的死循环。故整值浮点（如 86400.0）视为等于对应整数。
fn toml_int(item: &toml_edit::Item) -> Option<i64> {
    if let Some(i) = item.as_integer() {
        return Some(i);
    }
    let f = item.as_float()?;
    if f.fract() == 0.0 {
        Some(f as i64)
    } else {
        None
    }
}

fn toml_entry_matches(target: AgentTarget, text: &str, command: &str) -> bool {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    let Some(entry) = toml_entry(&doc) else {
        return false;
    };
    let (startup, tool, ask) = toml_profile(target);
    let cmd_ok = entry.get("command").and_then(|i| i.as_str()) == Some(command);
    let args_ok = entry
        .get("args")
        .and_then(|i| i.as_array())
        .map(|a| {
            let expected = crate::development::routing::integration_args(&[ARG_MCP]);
            a.len() == expected.len()
                && a.iter()
                    .zip(expected.iter())
                    .all(|(actual, expected)| actual.as_str() == Some(expected.as_str()))
        })
        .unwrap_or(false);
    let startup_ok = entry.get("startup_timeout_sec").and_then(toml_int) == Some(startup);
    let tool_ok = entry.get("tool_timeout_sec").and_then(toml_int) == Some(tool);
    // Grok 需精确匹配 `tool_timeouts.ask`（旧条目缺失 → 视为需更新）；Codex 不该有该键。
    let ask_ok = match ask {
        Some(secs) => {
            entry
                .get("tool_timeouts")
                .and_then(|i| i.as_table_like())
                .and_then(|t| t.get("ask"))
                .and_then(toml_int)
                == Some(secs)
        }
        None => entry.get("tool_timeouts").is_none(),
    };
    let direct_only_ok = target != AgentTarget::Codex || codex_direct_only_matches(&doc);
    cmd_ok && args_ok && startup_ok && tool_ok && ask_ok && direct_only_ok
}

// MARK: - 私有 IO / 工具

fn load_integration_state(path: &Path) -> Result<IntegrationState> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("解析集成所有权账本失败：{}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(IntegrationState::default())
        }
        Err(error) => {
            Err(error).with_context(|| format!("读取集成所有权账本失败：{}", path.display()))
        }
    }
}

fn save_integration_state(path: &Path, state: &IntegrationState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state).context("序列化集成所有权账本失败")?;
    crate::development::migration::before_write(path, Some(&bytes))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        harden_dir(dir);
    }
    let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    harden_file(&tmp);
    std::fs::rename(&tmp, path)?;
    harden_file(path);
    Ok(())
}

/// 仅当磁盘仍是本次刚写入的内容时回滚，避免覆盖期间发生的用户/其它进程修改。
fn rollback_text(
    path: &Path,
    expected_current: &str,
    original: &str,
    original_exists: bool,
) -> Result<()> {
    let current = std::fs::read_to_string(path)
        .with_context(|| format!("读取待回滚配置失败：{}", path.display()))?;
    if current != expected_current {
        return Err(anyhow!(
            "配置在事务期间已被其它进程修改，拒绝覆盖回滚：{}",
            path.display()
        ));
    }
    if original_exists {
        write_text(path, original)
    } else {
        crate::development::migration::remove_file(path)
            .with_context(|| format!("删除事务中新建的配置失败：{}", path.display()))
    }
}

#[cfg(unix)]
fn harden_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn harden_dir(_path: &Path) {}

#[cfg(unix)]
fn harden_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn harden_file(_path: &Path) {}

fn current_exe_string() -> Result<String> {
    let p = crate::executable::launch_path().context("failed to resolve current exe path")?;
    Ok(p.to_string_lossy().to_string())
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(path, text.as_bytes())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::development::migration::before_write(path, Some(bytes))?;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn collapse_home(p: &Path) -> String {
    let home = paths::home();
    if let Ok(rest) = p.strip_prefix(&home) {
        format!("~/{}", rest.display())
    } else {
        p.display().to_string()
    }
}

pub(crate) fn valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0'])
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/Users/u/.local/bin/AskHuman";

    #[cfg(unix)]
    #[test]
    fn antigravity_config_publication_preserves_private_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        for mode in [None, Some(0o600), Some(0o640), Some(0o644), Some(0o400)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("mcp_config.json");
            if let Some(mode) = mode {
                std::fs::write(
                    &path,
                    r#"{"mcpServers":{"other":{"env":{"KEY":"secret"}}}}"#,
                )
                .unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            let before = std::fs::metadata(&path).ok();
            let plan = PreparedAntigravityConfig::prepare(path.clone(), true, EXE).unwrap();
            plan.apply().unwrap();
            let after = std::fs::metadata(&path).unwrap();
            assert_eq!(after.mode() & 0o777, mode.unwrap_or(0o600) & 0o600);
            if let Some(before) = before {
                assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
                assert!(std::fs::read_to_string(&path).unwrap().contains("secret"));
            }
            plan.restore().unwrap();
            if let Some(mode) = mode {
                assert_eq!(
                    std::fs::metadata(&path).unwrap().mode() & 0o777,
                    mode & 0o600
                );
            } else {
                assert!(!path.exists());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_config_publication_rejects_links_without_changing_secrets() {
        use std::os::unix::fs::symlink;
        for hard_link in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let original = dir.path().join("secrets.json");
            let path = dir.path().join("mcp_config.json");
            let contents = r#"{"mcpServers":{"other":{"env":{"KEY":"secret"}}}}"#;
            std::fs::write(&original, contents).unwrap();
            if hard_link {
                std::fs::hard_link(&original, &path).unwrap();
            } else {
                symlink(&original, &path).unwrap();
            }
            let plan = PreparedAntigravityConfig::prepare(path.clone(), true, EXE).unwrap();
            let error = plan.apply().unwrap_err().to_string();
            if hard_link {
                assert!(error.contains("hard-linked MCP config"));
                assert!(error.contains("single link"));
                use std::os::unix::fs::MetadataExt;
                assert_eq!(std::fs::metadata(&original).unwrap().nlink(), 2);
            } else {
                assert!(error.contains("symlinked MCP config"));
                assert!(error.contains("Home Manager"));
                assert!(error.contains("timeoutSeconds=86400"));
                assert_eq!(std::fs::read_link(&path).unwrap(), original);
            }
            assert!(!error.contains("KEY"));
            assert!(!error.contains("secret\""));
            assert_eq!(std::fs::read_to_string(&original).unwrap(), contents);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_read_only_symlink_target_is_preserved_with_actionable_error() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("declarative.json");
        let path = dir.path().join("mcp_config.json");
        std::fs::write(&original, "{}").unwrap();
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o444)).unwrap();
        symlink(&original, &path).unwrap();
        let plan = PreparedAntigravityConfig::prepare(path.clone(), true, EXE).unwrap();
        let error = plan.apply().unwrap_err().to_string();
        assert!(error.contains("Home Manager and rebuild"));
        assert_eq!(std::fs::read_link(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "{}");
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_matching_symlink_config_remains_an_idempotent_noop() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("managed.json");
        let path = dir.path().join("mcp_config.json");
        let contents = apply_install_json("{}", EXE, JsonTimeout::TimeoutSeconds(86400)).unwrap();
        std::fs::write(&original, &contents).unwrap();
        symlink(&original, &path).unwrap();
        PreparedAntigravityConfig::prepare(path.clone(), true, EXE)
            .unwrap()
            .apply()
            .unwrap();
        assert_eq!(std::fs::read_link(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&original).unwrap(), contents);
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_semantic_noops_preserve_declarative_bytes_and_links() {
        use std::os::unix::fs::{symlink, MetadataExt};
        let matching = format!(
            "{{\n // keep root comment\n \"mcpServers\": {{\"askhuman\": {{\n\
             \"args\": [\"mcp\"], // keep entry comment\n\
             \"timeoutSeconds\": 86400, \"command\": \"{EXE}\",\n\
             \"env\": {{\"DISPLAY\": \":1\"}} }} }}, \"unrelated\": true\n}}\n"
        );
        for (install, contents) in [
            (true, matching.as_str()),
            (false, "{\"mcpServers\": { /* retain empty servers */ }}\n"),
        ] {
            for link in ["regular", "symbolic", "hard"] {
                let root = tempfile::tempdir().unwrap();
                let source = root.path().join("source.json");
                let path = root.path().join("mcp_config.json");
                std::fs::write(&source, contents).unwrap();
                match link {
                    "symbolic" => symlink(&source, &path).unwrap(),
                    "hard" => std::fs::hard_link(&source, &path).unwrap(),
                    _ => std::fs::write(&path, contents).unwrap(),
                }
                let before = std::fs::symlink_metadata(&path).unwrap();
                let plan = PreparedAntigravityConfig::prepare(path.clone(), install, EXE).unwrap();
                assert_eq!(plan.before, plan.after, "{link}, install={install}");
                plan.apply().unwrap();
                plan.restore().unwrap();
                let after = std::fs::symlink_metadata(&path).unwrap();
                assert_eq!((before.ino(), before.nlink()), (after.ino(), after.nlink()));
                assert_eq!(before.file_type(), after.file_type());
                assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
                assert_eq!(std::fs::read_to_string(&source).unwrap(), contents);
            }
        }
    }

    #[test]
    fn antigravity_semantic_noop_still_validates_environment() {
        for env in [
            serde_json::json!([]),
            serde_json::json!({"KEY": 1}),
            serde_json::json!({"BAD=KEY": "value"}),
            serde_json::json!({"KEY": "a\0b"}),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("mcp_config.json");
            let contents = serde_json::json!({"mcpServers": {"askhuman": {
                "command": EXE, "args": ["mcp"], "timeoutSeconds": 86400, "env": env
            }}})
            .to_string();
            std::fs::write(&path, &contents).unwrap();
            assert!(PreparedAntigravityConfig::prepare(path.clone(), true, EXE).is_err());
            assert_eq!(std::fs::read_to_string(path).unwrap(), contents);
        }
    }

    fn to_value(text: &str) -> Value {
        jsonc_parser::parse_to_serde_value::<Value>(text, &ParseOptions::default()).unwrap()
    }

    #[test]
    fn antigravity_mcp_properties() {
        assert!(supported(AgentTarget::Antigravity));
        assert_eq!(format_of(AgentTarget::Antigravity), Format::Json);
        assert_eq!(
            config_path(AgentTarget::Antigravity),
            paths::antigravity_mcp_config_json()
        );
        assert_eq!(
            json_timeout(AgentTarget::Antigravity),
            JsonTimeout::TimeoutSeconds(ANTIGRAVITY_TOOL_TIMEOUT_SEC)
        );
    }

    // ── JSON ──

    #[test]
    fn json_install_into_empty_creates_entry() {
        let out = apply_install_json("", EXE, JsonTimeout::None).unwrap();
        let v = to_value(&out);
        assert_eq!(v["mcpServers"][SERVER_NAME]["command"], EXE);
        assert_eq!(v["mcpServers"][SERVER_NAME]["args"][0], ARG_MCP);
        // Cursor 风格（无 timeout）不写该字段。
        assert!(v["mcpServers"][SERVER_NAME].get("timeout").is_none());
        assert!(v["mcpServers"][SERVER_NAME].get("timeoutSeconds").is_none());
        assert!(json_entry_matches(&v, EXE, JsonTimeout::None));
    }

    #[test]
    fn json_install_claude_writes_timeout() {
        let out =
            apply_install_json("", EXE, JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
        let v = to_value(&out);
        assert_eq!(
            v["mcpServers"][SERVER_NAME]["timeout"].as_i64(),
            Some(CLAUDE_TOOL_TIMEOUT_MS)
        );
        assert!(json_entry_matches(
            &v,
            EXE,
            JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS)
        ));
        // 缺 timeout 的预期（Cursor）与之不匹配。
        assert!(!json_entry_matches(&v, EXE, JsonTimeout::None));
    }

    #[test]
    fn json_install_antigravity_writes_timeout_seconds() {
        let out = apply_install_json(
            "",
            EXE,
            JsonTimeout::TimeoutSeconds(ANTIGRAVITY_TOOL_TIMEOUT_SEC),
        )
        .unwrap();
        let v = to_value(&out);
        assert_eq!(
            v["mcpServers"][SERVER_NAME]["timeoutSeconds"].as_i64(),
            Some(ANTIGRAVITY_TOOL_TIMEOUT_SEC)
        );
        assert!(json_entry_matches(
            &v,
            EXE,
            JsonTimeout::TimeoutSeconds(ANTIGRAVITY_TOOL_TIMEOUT_SEC)
        ));
        // A profile without timeoutSeconds must not match this entry.
        assert!(!json_entry_matches(&v, EXE, JsonTimeout::None));
        assert!(!json_entry_matches(
            &v,
            EXE,
            JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS)
        ));
    }

    #[test]
    fn json_claude_old_entry_without_timeout_needs_update() {
        // 模拟旧版（无 timeout）安装后，按 Claude 预期校验应判定需更新。
        let old = apply_install_json("{}", EXE, JsonTimeout::None).unwrap();
        let v = to_value(&old);
        assert!(!json_entry_matches(
            &v,
            EXE,
            JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS)
        ));
    }

    #[test]
    fn json_install_is_idempotent_fixpoint() {
        let a =
            apply_install_json("{}", EXE, JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
        let b =
            apply_install_json(&a, EXE, JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
        let c =
            apply_install_json(&b, EXE, JsonTimeout::TimeoutMs(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
        assert_eq!(b, c, "已安装态再安装应为稳定不动点");
        let v = to_value(&c);
        assert_eq!(
            v["mcpServers"].as_object().unwrap().len(),
            1,
            "不应产生重复条目"
        );
    }

    #[test]
    fn json_install_preserves_other_servers_and_comments() {
        let input = "{\n  // 用户注释，勿动\n  \"mcpServers\": {\n    \"other\": { \"command\": \"x\", \"args\": [] }\n  }\n}";
        let out = apply_install_json(input, EXE, JsonTimeout::None).unwrap();
        assert!(out.contains("// 用户注释，勿动"), "注释应原样保留");
        let v = to_value(&out);
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"][SERVER_NAME]["command"], EXE);
    }

    #[test]
    fn json_install_updates_command_in_place() {
        let old = apply_install_json("{}", "/old/AskHuman", JsonTimeout::None).unwrap();
        let new = apply_install_json(&old, EXE, JsonTimeout::None).unwrap();
        let v = to_value(&new);
        assert_eq!(v["mcpServers"][SERVER_NAME]["command"], EXE);
        assert!(json_entry_matches(&v, EXE, JsonTimeout::None));
        assert!(!json_entry_matches(&v, "/old/AskHuman", JsonTimeout::None));
    }

    #[test]
    fn json_install_aborts_on_non_object_root() {
        assert!(apply_install_json("[]", EXE, JsonTimeout::None).is_err());
    }

    #[test]
    fn json_install_aborts_on_parse_error() {
        assert!(apply_install_json("{ \"mcpServers\": ", EXE, JsonTimeout::None).is_err());
    }

    #[test]
    fn json_uninstall_removes_only_ours() {
        let input = "{ \"mcpServers\": { \"other\": { \"command\": \"x\" }, \"askhuman\": { \"command\": \"y\", \"args\": [\"mcp\"] } } }";
        let out = apply_uninstall_json(input).unwrap();
        let v = to_value(&out);
        assert!(json_entry(&v).is_none());
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
    }

    #[test]
    fn json_uninstall_drops_empty_servers_key() {
        let only = apply_install_json("{}", EXE, JsonTimeout::None).unwrap();
        let out = apply_uninstall_json(&only).unwrap();
        let v = to_value(&out);
        assert!(v.get("mcpServers").is_none(), "空 mcpServers 应删除该键");
    }

    #[test]
    fn json_uninstall_noop_when_absent() {
        let input = "{ \"mcpServers\": { \"other\": { \"command\": \"x\" } } }";
        let out = apply_uninstall_json(input).unwrap();
        let v = to_value(&out);
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert!(json_entry(&v).is_none());
    }

    #[test]
    fn json_uninstall_aborts_on_parse_error() {
        assert!(apply_uninstall_json("{ \"mcpServers\": ").is_err());
    }

    // ── TOML（Codex） ──
    const CODEX: AgentTarget = AgentTarget::Codex;
    const GROK: AgentTarget = AgentTarget::Grok;

    #[test]
    fn toml_install_into_empty_creates_table() {
        let out = apply_install_toml(CODEX, "", EXE).unwrap();
        assert!(out.contains("[mcp_servers.askhuman]"));
        assert!(toml_installed(&out));
        assert!(toml_entry_matches(CODEX, &out, EXE));
    }

    #[test]
    fn toml_install_writes_timeouts() {
        let out = apply_install_toml(CODEX, "", EXE).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        let entry = toml_entry(&doc).unwrap();
        assert_eq!(
            entry.get("tool_timeout_sec").and_then(|i| i.as_integer()),
            Some(CODEX_TOOL_TIMEOUT_SEC)
        );
        assert_eq!(
            entry
                .get("startup_timeout_sec")
                .and_then(|i| i.as_integer()),
            Some(CODEX_STARTUP_TIMEOUT_SEC)
        );
        // Codex 不写 per-tool tool_timeouts。
        assert!(entry.get("tool_timeouts").is_none());
    }

    #[test]
    fn codex_install_adds_direct_only_and_reports_ownership_once() {
        let first = apply_install_toml_outcome(CODEX, "", EXE).unwrap();
        assert!(first.direct_only_added);
        let doc = first.text.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(codex_direct_only_matches(&doc));

        let second = apply_install_toml_outcome(CODEX, &first.text, EXE).unwrap();
        assert!(!second.direct_only_added, "重复安装不能重复认领");
        assert_eq!(first.text, second.text);
    }

    #[test]
    fn codex_install_appends_without_overwriting_user_namespaces() {
        let input = "# keep\n[features.code_mode]\ndirect_only_tool_namespaces = [\"mcp__history\", \"mcp__notes\"] # user\n";
        let outcome = apply_install_toml_outcome(CODEX, input, EXE).unwrap();
        assert!(outcome.direct_only_added);
        assert!(outcome.text.contains("# keep"));
        assert!(outcome.text.contains("# user"));
        let doc = outcome.text.parse::<toml_edit::DocumentMut>().unwrap();
        let array = doc["features"]["code_mode"]["direct_only_tool_namespaces"]
            .as_array()
            .unwrap();
        let values: Vec<_> = array.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(
            values,
            ["mcp__history", "mcp__notes", CODEX_DIRECT_ONLY_NAMESPACE]
        );
    }

    #[test]
    fn codex_install_does_not_claim_preexisting_same_namespace() {
        let input = "[features.code_mode]\ndirect_only_tool_namespaces = [\"mcp__askhuman\", \"mcp__notes\"]\n";
        let outcome = apply_install_toml_outcome(CODEX, input, EXE).unwrap();
        assert!(!outcome.direct_only_added);
        let doc = outcome.text.parse::<toml_edit::DocumentMut>().unwrap();
        let array = doc["features"]["code_mode"]["direct_only_tool_namespaces"]
            .as_array()
            .unwrap();
        assert_eq!(
            array
                .iter()
                .filter(|v| v.as_str() == Some(CODEX_DIRECT_ONLY_NAMESPACE))
                .count(),
            1
        );
    }

    #[test]
    fn codex_install_aborts_on_invalid_direct_only_type() {
        let scalar = "[features.code_mode]\ndirect_only_tool_namespaces = true\n";
        assert!(apply_install_toml(CODEX, scalar, EXE).is_err());
        let mixed = "[features.code_mode]\ndirect_only_tool_namespaces = [\"mcp__notes\", 1]\n";
        assert!(apply_install_toml(CODEX, mixed, EXE).is_err());
    }

    #[test]
    fn codex_entry_without_direct_only_needs_update() {
        let old = format!(
            "[mcp_servers.askhuman]\ncommand = \"{EXE}\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 30\ntool_timeout_sec = 86400\n"
        );
        assert!(!toml_entry_matches(CODEX, &old, EXE));
    }

    #[test]
    fn codex_install_sets_approval_mode_only_when_absent() {
        // Fresh install writes approve (skip per-call approval for our own ask tools).
        let out = apply_install_toml(CODEX, "", EXE).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            toml_entry(&doc)
                .unwrap()
                .get("default_tools_approval_mode")
                .and_then(|i| i.as_str()),
            Some("approve")
        );
        // A user-edited value survives reinstall/update and never flags "needs update".
        let user_edited = out.replace(
            "default_tools_approval_mode = \"approve\"",
            "default_tools_approval_mode = \"prompt\"",
        );
        let rewritten = apply_install_toml(CODEX, &user_edited, EXE).unwrap();
        let doc = rewritten.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            toml_entry(&doc)
                .unwrap()
                .get("default_tools_approval_mode")
                .and_then(|i| i.as_str()),
            Some("prompt")
        );
        assert!(toml_entry_matches(CODEX, &rewritten, EXE));
        // A user-deleted key does not flag "needs update" either.
        let deleted = out.replace("default_tools_approval_mode = \"approve\"\n", "");
        assert!(toml_entry_matches(CODEX, &deleted, EXE));
        // Grok config never gets the Codex-specific key.
        let grok = apply_install_toml(GROK, "", EXE).unwrap();
        let doc = grok.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(toml_entry(&doc)
            .unwrap()
            .get("default_tools_approval_mode")
            .is_none());
    }

    /// What AskHuman before v0.15 left in a Codex config: today's entry with the approval key
    /// under its old name.
    fn legacy_codex_config() -> String {
        let current = apply_install_toml(CODEX, "", EXE).unwrap();
        let legacy = current.replace("default_tools_approval_mode", "approval_mode");
        assert_ne!(legacy, current);
        legacy
    }

    #[test]
    fn codex_install_drops_the_legacy_server_level_approval_mode() {
        let legacy = legacy_codex_config();
        // The old key alone does not flag "needs update": that would block task readiness.
        assert!(toml_entry_matches(CODEX, &legacy, EXE));

        let out = apply_install_toml(CODEX, &legacy, EXE).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        let entry = toml_entry(&doc).unwrap();
        assert!(entry.get("approval_mode").is_none());
        assert_eq!(
            entry
                .get("default_tools_approval_mode")
                .and_then(|i| i.as_str()),
            Some("approve")
        );
        // The migrated file is what a fresh install writes, and it stays put.
        assert_eq!(out, apply_install_toml(CODEX, "", EXE).unwrap());
        assert_eq!(apply_install_toml(CODEX, &out, EXE).unwrap(), out);
    }

    #[test]
    fn codex_install_drops_the_legacy_key_whatever_its_value() {
        // Codex ignored the key whatever it said, so an edited value carried no behavior.
        let edited = legacy_codex_config()
            .replace("approval_mode = \"approve\"", "approval_mode = \"prompt\"");
        let out = apply_install_toml(CODEX, &edited, EXE).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        let entry = toml_entry(&doc).unwrap();
        assert!(entry.get("approval_mode").is_none());
        assert_eq!(
            entry
                .get("default_tools_approval_mode")
                .and_then(|i| i.as_str()),
            Some("approve")
        );

        // With the current key present too, the user's value survives and only the old key goes.
        let both = edited.replace(
            "approval_mode = \"prompt\"",
            "approval_mode = \"prompt\"\ndefault_tools_approval_mode = \"writes\"",
        );
        let out = apply_install_toml(CODEX, &both, EXE).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        let entry = toml_entry(&doc).unwrap();
        assert!(entry.get("approval_mode").is_none());
        assert_eq!(
            entry
                .get("default_tools_approval_mode")
                .and_then(|i| i.as_str()),
            Some("writes")
        );
    }

    #[test]
    fn codex_install_keeps_every_other_approval_mode() {
        // `approval_mode` is a valid per-tool key (the permission memory writes it), and another
        // server's config is not ours to edit.
        let text = format!(
            "{}\n[mcp_servers.askhuman.tools.ask]\napproval_mode = \"approve\"\n\n\
             [mcp_servers.github]\ncommand = \"gh-mcp\"\napproval_mode = \"prompt\"\n",
            legacy_codex_config()
        );
        let out = apply_install_toml(CODEX, &text, EXE).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(toml_entry(&doc).unwrap().get("approval_mode").is_none());
        assert_eq!(
            doc["mcp_servers"]["askhuman"]["tools"]["ask"]["approval_mode"].as_str(),
            Some("approve")
        );
        assert_eq!(
            doc["mcp_servers"]["github"]["approval_mode"].as_str(),
            Some("prompt")
        );
    }

    #[test]
    fn grok_install_does_not_touch_an_approval_mode_key() {
        // AskHuman never wrote it for Grok, so any such key is the user's.
        let text = "[mcp_servers.askhuman]\ncommand = \"x\"\napproval_mode = \"approve\"\n";
        let out = apply_install_toml(GROK, text, EXE).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            toml_entry(&doc)
                .unwrap()
                .get("approval_mode")
                .and_then(|i| i.as_str()),
            Some("approve")
        );
    }

    #[test]
    fn toml_install_is_idempotent_fixpoint() {
        let a = apply_install_toml(CODEX, "", EXE).unwrap();
        let b = apply_install_toml(CODEX, &a, EXE).unwrap();
        let c = apply_install_toml(CODEX, &b, EXE).unwrap();
        assert_eq!(b, c, "已安装态再安装应为稳定不动点");
    }

    #[test]
    fn toml_install_preserves_other_tables_and_comments() {
        let input = "# 用户配置，勿动\nmodel = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"x\"\nargs = []\n";
        let out = apply_install_toml(CODEX, input, EXE).unwrap();
        assert!(out.contains("# 用户配置，勿动"), "注释应保留");
        assert!(out.contains("model = \"gpt-5\""), "用户键应保留");
        assert!(out.contains("[mcp_servers.other]"), "他人 server 应保留");
        assert!(toml_entry_matches(CODEX, &out, EXE));
    }

    #[test]
    fn toml_install_updates_command_in_place() {
        let old = apply_install_toml(CODEX, "", "/old/AskHuman").unwrap();
        let new = apply_install_toml(CODEX, &old, EXE).unwrap();
        assert!(toml_entry_matches(CODEX, &new, EXE));
        assert!(!toml_entry_matches(CODEX, &new, "/old/AskHuman"));
    }

    #[test]
    fn toml_install_aborts_on_parse_error() {
        assert!(apply_install_toml(CODEX, "[mcp_servers", EXE).is_err());
    }

    // ── TOML（Grok：额外的 tool_timeouts.ask） ──

    #[test]
    fn grok_install_writes_ask_tool_timeout() {
        let out = apply_install_toml(GROK, "", EXE).unwrap();
        assert!(out.contains("[mcp_servers.askhuman]"));
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        let entry = toml_entry(&doc).unwrap();
        assert_eq!(
            entry
                .get("startup_timeout_sec")
                .and_then(|i| i.as_integer()),
            Some(GROK_STARTUP_TIMEOUT_SEC)
        );
        assert_eq!(
            entry.get("tool_timeout_sec").and_then(|i| i.as_integer()),
            Some(GROK_TOOL_TIMEOUT_SEC)
        );
        assert_eq!(
            entry
                .get("tool_timeouts")
                .and_then(|i| i.as_table_like())
                .and_then(|t| t.get("ask"))
                .and_then(|v| v.as_integer()),
            Some(GROK_ASK_TOOL_TIMEOUT_SEC)
        );
        assert!(toml_entry_matches(GROK, &out, EXE));
    }

    #[test]
    fn grok_install_is_idempotent_fixpoint() {
        let a = apply_install_toml(GROK, "", EXE).unwrap();
        let b = apply_install_toml(GROK, &a, EXE).unwrap();
        let c = apply_install_toml(GROK, &b, EXE).unwrap();
        assert_eq!(b, c, "Grok 已安装态再安装应为稳定不动点");
    }

    #[test]
    fn grok_old_entry_without_ask_timeout_needs_update() {
        // 用 Codex 档写（无 tool_timeouts）后，按 Grok 预期校验应判需更新。
        let old = apply_install_toml(CODEX, "", EXE).unwrap();
        assert!(!toml_entry_matches(GROK, &old, EXE));
        // 补齐后匹配。
        let fixed = apply_install_toml(GROK, &old, EXE).unwrap();
        assert!(toml_entry_matches(GROK, &fixed, EXE));
    }

    #[test]
    fn toml_entry_matches_tolerates_float_normalized_timeouts() {
        // Codex/Grok CLI 会把 30 改写成 30.0：整值浮点应仍判定为「已是最新」，避免死循环需更新。
        let codex_float = format!(
            "[mcp_servers.askhuman]\ncommand = \"{EXE}\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 30.0\ntool_timeout_sec = 86400.0\n\n[features.code_mode]\ndirect_only_tool_namespaces = [\"mcp__askhuman\"]\n"
        );
        assert!(toml_entry_matches(CODEX, &codex_float, EXE));

        let grok_float = format!(
            "[mcp_servers.askhuman]\ncommand = \"{EXE}\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 30.0\ntool_timeout_sec = 86400.0\ntool_timeouts = {{ ask = 86400.0 }}\n"
        );
        assert!(toml_entry_matches(GROK, &grok_float, EXE));
    }

    #[test]
    fn toml_uninstall_removes_only_ours() {
        let input = apply_install_toml(
            CODEX,
            "[mcp_servers.other]\ncommand = \"x\"\nargs = []\n",
            EXE,
        )
        .unwrap();
        let out = apply_uninstall_toml(&input, true).unwrap();
        assert!(!toml_installed(&out));
        assert!(out.contains("[mcp_servers.other]"), "他人 server 应保留");
    }

    #[test]
    fn toml_uninstall_drops_empty_servers_table() {
        let only = apply_install_toml(CODEX, "", EXE).unwrap();
        let out = apply_uninstall_toml(&only, true).unwrap();
        assert!(!out.contains("mcp_servers"), "空 mcp_servers 表应删除");
    }

    #[test]
    fn toml_uninstall_noop_when_absent() {
        let input = "model = \"gpt-5\"\n";
        let out = apply_uninstall_toml(input, false).unwrap();
        assert!(out.contains("model = \"gpt-5\""));
        assert!(!toml_installed(&out));
    }

    #[test]
    fn toml_uninstall_aborts_on_parse_error() {
        assert!(apply_uninstall_toml("[mcp_servers", false).is_err());
    }

    #[test]
    fn codex_uninstall_only_removes_direct_only_when_owned() {
        let installed = apply_install_toml(CODEX, "", EXE).unwrap();
        let preserved = apply_uninstall_toml(&installed, false).unwrap();
        let doc = preserved.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(codex_direct_only_matches(&doc));

        let removed = apply_uninstall_toml(&installed, true).unwrap();
        let doc = removed.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(!codex_direct_only_matches(&doc));
        assert!(doc.get("features").is_some(), "父表必须保留");
        assert!(
            doc["features"]["code_mode"]["direct_only_tool_namespaces"]
                .as_array()
                .is_some_and(|a| a.is_empty()),
            "数组键必须保留"
        );
    }

    #[test]
    fn integration_state_roundtrip_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integration-state.json");
        std::fs::write(
            &path,
            r#"{"codex":{"direct_only_namespace_added_by_askhuman":true,"future":1},"other":{"x":2}}"#,
        )
        .unwrap();
        let state = load_integration_state(&path).unwrap();
        assert!(state.codex.direct_only_namespace_added_by_askhuman);
        save_integration_state(&path, &state).unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(value["codex"]["future"], 1);
        assert_eq!(value["other"]["x"], 2);
    }

    #[cfg(unix)]
    #[test]
    fn integration_state_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integration-state.json");
        save_integration_state(&path, &IntegrationState::default()).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn codex_install_rolls_back_when_state_save_fails() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let original = "model = \"gpt-5\"\n";
        std::fs::write(&config, original).unwrap();
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, "x").unwrap();
        let state = blocker.join("integration-state.json");

        assert!(install_codex_toml_with_state(&config, &state, original, true, EXE).is_err());
        assert_eq!(std::fs::read_to_string(config).unwrap(), original);
    }

    #[test]
    fn codex_owned_direct_only_roundtrips_through_install_and_uninstall() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let state_path = dir.path().join("integration-state.json");
        let original = "model = \"gpt-5\"\n";
        std::fs::write(&config, original).unwrap();

        install_codex_toml_with_state(&config, &state_path, original, true, EXE).unwrap();
        let installed = std::fs::read_to_string(&config).unwrap();
        assert!(toml_installed(&installed));
        assert!(codex_direct_only_matches(&installed.parse().unwrap()));
        assert!(
            load_integration_state(&state_path)
                .unwrap()
                .codex
                .direct_only_namespace_added_by_askhuman
        );

        uninstall_codex_toml_with_state(&config, &state_path, &installed).unwrap();
        let removed = std::fs::read_to_string(&config).unwrap();
        assert!(!toml_installed(&removed));
        assert!(!codex_direct_only_matches(&removed.parse().unwrap()));
        assert!(
            !load_integration_state(&state_path)
                .unwrap()
                .codex
                .direct_only_namespace_added_by_askhuman
        );
    }

    #[test]
    fn codex_uninstall_preserves_preexisting_namespace_without_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let state = dir.path().join("missing-state.json");
        let installed = apply_install_toml(CODEX, "", EXE).unwrap();
        std::fs::write(&config, &installed).unwrap();

        uninstall_codex_toml_with_state(&config, &state, &installed).unwrap();
        let out = std::fs::read_to_string(config).unwrap();
        assert!(!toml_installed(&out));
        assert!(codex_direct_only_matches(&out.parse().unwrap()));
    }

    #[test]
    fn missing_codex_config_clears_stale_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("integration-state.json");
        let mut state = IntegrationState::default();
        state.codex.direct_only_namespace_added_by_askhuman = true;
        save_integration_state(&state_path, &state).unwrap();

        clear_codex_ownership_without_config(&state_path).unwrap();
        assert!(
            !load_integration_state(&state_path)
                .unwrap()
                .codex
                .direct_only_namespace_added_by_askhuman
        );
    }
}

#[cfg(test)]
mod env_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn toml_env_merge_preserves_table_and_inline_values_and_is_stable() {
        for source in [
            "[mcp_servers.askhuman]\nenv = { KEEP = 'old', DISPLAY = ':0' }\n",
            "[mcp_servers.askhuman.env]\nKEEP = 'old'\nDISPLAY = ':0'\n",
        ] {
            let env = BTreeMap::from([
                ("DISPLAY".into(), ":1".into()),
                ("EMPTY".into(), "".into()),
                ("ESCAPE".into(), "a=\"b\\c\n".into()),
            ]);
            let merged =
                apply_install_toml_outcome_env(AgentTarget::Codex, source, "AskHuman", &env)
                    .unwrap()
                    .text;
            let doc = merged.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(
                doc["mcp_servers"]["askhuman"]["env"]["KEEP"].as_str(),
                Some("old")
            );
            assert_eq!(
                doc["mcp_servers"]["askhuman"]["env"]["ESCAPE"].as_str(),
                Some("a=\"b\\c\n")
            );
            assert_eq!(
                apply_install_toml(AgentTarget::Codex, &merged, "AskHuman").unwrap(),
                merged
            );
            assert_eq!(
                apply_install_toml_outcome_env(AgentTarget::Codex, &merged, "AskHuman", &env)
                    .unwrap()
                    .text,
                merged
            );
        }
    }

    #[test]
    fn invalid_env_aborts_toml_and_json() {
        for env in ["42", "{ DISPLAY = 42 }", "{ 'BAD=NAME' = 'x' }"] {
            assert!(apply_install_toml(
                AgentTarget::Codex,
                &format!("[mcp_servers.askhuman]\nenv = {env}\n"),
                "AskHuman"
            )
            .is_err());
        }
        for timeout in [
            JsonTimeout::None,
            JsonTimeout::TimeoutMs(86400000),
            JsonTimeout::TimeoutSeconds(86400),
        ] {
            assert!(apply_install_json(
                r#"{"mcpServers":{"askhuman":{"env":{"DISPLAY":42}}}}"#,
                "AskHuman",
                timeout
            )
            .is_err());
        }
    }

    #[test]
    fn json_env_survives_both_timeout_profiles_only() {
        let source =
            r#"{"mcpServers":{"askhuman":{"env":{"DISPLAY":":1","EMPTY":""},"custom":"discard"}}}"#;
        for timeout in [
            JsonTimeout::None,
            JsonTimeout::TimeoutMs(86400000),
            JsonTimeout::TimeoutSeconds(86400),
        ] {
            let updated = apply_install_json(source, "AskHuman", timeout).unwrap();
            let value: Value = serde_json::from_str(&updated).unwrap();
            assert_eq!(value["mcpServers"]["askhuman"]["env"]["DISPLAY"], ":1");
            assert!(value["mcpServers"]["askhuman"].get("custom").is_none());
            assert_eq!(
                apply_install_json(&updated, "AskHuman", timeout).unwrap(),
                updated
            );
        }
    }
}

#[cfg(test)]
mod env_transaction_tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn unreadable_ownership_prevents_environment_write() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let state = dir.path().join("blocked").join("state.json");
        std::fs::write(dir.path().join("blocked"), "file").unwrap();
        let original = "[mcp_servers.askhuman.env]\nKEEP = 'old'\n";
        std::fs::write(&config, original).unwrap();
        let env = BTreeMap::from([("DISPLAY".into(), ":1".into())]);
        assert!(install_codex_toml_with_state_env(
            &config, &state, original, true, "AskHuman", &env
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
    }
    #[test]
    fn malformed_env_does_not_write_config_or_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let state = dir.path().join("state.json");
        let original = "[mcp_servers.askhuman]\nenv = 42\n";
        std::fs::write(&config, original).unwrap();
        assert!(install_codex_toml_with_state_env(
            &config,
            &state,
            original,
            true,
            "AskHuman",
            &BTreeMap::new()
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(!state.exists());
    }
}
