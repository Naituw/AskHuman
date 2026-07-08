//! MCP 配置集成：把 `askhuman` server 写入三家 Agent 的**用户级全局** MCP 配置，
//! 供 MCP 模式（`AskHuman mcp`）被客户端拉起。
//!
//! 落点（spec D7）：
//! - Cursor：`~/.cursor/mcp.json`（JSON，`mcpServers.askhuman`；其 MCP 超时硬编码 ~60s 且不可配置，
//!   不写 `timeout`）。
//! - Claude Code：`~/.claude.json`（JSON，top-level `mcpServers.askhuman`；文件大、含项目历史，必须最小化
//!   编辑。写 `timeout`(毫秒) 覆盖其 60s 默认，否则长等待会被取消）。
//! - Codex：`~/.codex/config.toml`（TOML，`[mcp_servers.askhuman]`，含大 `tool_timeout_sec`）。
//!
//! 一律沿用现有 hook/rule 集成的**纯函数 + 最小化编辑（CST/`toml_edit`）+ 解析失败即中止不覆盖 + 单测**
//! 范式：只触碰自有 `askhuman` 条目，保留用户其它内容 / 注释 / 键序。`command` 写当前可执行文件绝对路径
//! （D16，部分客户端不继承 shell PATH）。

use crate::integrations::agent_rules::AgentTarget;
use crate::paths;
use anyhow::{anyhow, Context, Result};
use jsonc_parser::cst::CstRootNode;
use jsonc_parser::json;
use jsonc_parser::ParseOptions;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 各家配置中本 server 的名字（恒 `askhuman`，spec D15）。
pub const SERVER_NAME: &str = "askhuman";
/// 启动子命令（`AskHuman mcp`）。
pub const ARG_MCP: &str = "mcp";
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

/// 该目标 JSON 条目是否需要写入 `timeout`（毫秒）。仅 Claude Code 支持并需要；Cursor 不认该字段。
fn json_timeout_ms(target: AgentTarget) -> Option<i64> {
    match target {
        AgentTarget::ClaudeCode => Some(CLAUDE_TOOL_TIMEOUT_MS),
        _ => None,
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
        AgentTarget::Cursor | AgentTarget::ClaudeCode => Format::Json,
    }
}

/// 目标配置文件路径。
fn config_path(target: AgentTarget) -> PathBuf {
    match target {
        AgentTarget::Cursor => paths::cursor_mcp_json(),
        AgentTarget::ClaudeCode => paths::claude_json(),
        AgentTarget::Codex => paths::codex_config_toml(),
        AgentTarget::Grok => paths::grok_config_toml(),
    }
}

/// 当前平台是否支持（三家配置读写均跨平台）。
pub fn supported(_target: AgentTarget) -> bool {
    true
}

/// 配置展示路径（home 前缀折叠为 `~`）。
pub fn display_path(target: AgentTarget) -> String {
    collapse_home(&config_path(target))
}

/// 是否已写入本 server 条目。
pub fn is_installed(target: AgentTarget) -> bool {
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
    if !is_installed(target) {
        return false;
    }
    let Ok(exe) = current_exe_string() else {
        return false;
    };
    let path = config_path(target);
    match format_of(target) {
        Format::Json => read_json_value(&path)
            .map(|v| !json_entry_matches(&v, &exe, json_timeout_ms(target)))
            .unwrap_or(false),
        Format::Toml => std::fs::read_to_string(&path)
            .map(|t| !toml_entry_matches(target, &t, &exe))
            .unwrap_or(false),
    }
}

/// 安装：写入 / 更新本 server 条目（最小化编辑，保留用户其它内容）。
pub fn install(target: AgentTarget) -> Result<String> {
    write_entry(target)?;
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigInstalled").to_string())
}

/// 更新：与安装同样写入逻辑，仅反馈文案不同。
pub fn update(target: AgentTarget) -> Result<String> {
    write_entry(target)?;
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigUpdated").to_string())
}

/// 卸载：移除本 server 条目（保留用户其它条目）；条目本就不存在则 no-op。
pub fn uninstall(target: AgentTarget) -> Result<String> {
    let path = config_path(target);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let updated = match format_of(target) {
            Format::Json => apply_uninstall_json(&text)?,
            Format::Toml => apply_uninstall_toml(&text)?,
        };
        write_text(&path, &updated)?;
    }
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.mcpConfigRemoved").to_string())
}

fn write_entry(target: AgentTarget) -> Result<()> {
    let exe = current_exe_string()?;
    let path = config_path(target);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = match format_of(target) {
        Format::Json => apply_install_json(&text, &exe, json_timeout_ms(target))?,
        Format::Toml => apply_install_toml(target, &text, &exe)?,
    };
    write_text(&path, &updated)
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

// MARK: - JSON（Cursor / Claude）：CST 保留格式最小化编辑

/// upsert `mcpServers.askhuman = { command, args:["mcp"], timeout? }`；仅触碰本条目，其余字节保留。
/// `timeout_ms`（毫秒）仅 Claude 传入（覆盖其 60s 默认）；Cursor 传 `None`、不写该字段。
/// 解析失败返回 Err（调用方据此中止、绝不整写覆盖）。
fn apply_install_json(text: &str, command: &str, timeout_ms: Option<i64>) -> Result<String> {
    let source = if text.trim().is_empty() { "{}" } else { text };
    let root = CstRootNode::parse(source, &ParseOptions::default())
        .map_err(|e| anyhow!("解析 MCP 配置 JSON 失败，已中止（不覆盖原文件）：{e}"))?;
    let root_obj = root
        .object_value_or_create()
        .ok_or_else(|| anyhow!("MCP 配置根不是 JSON 对象，已中止"))?;
    let servers = root_obj
        .object_value_or_create("mcpServers")
        .ok_or_else(|| anyhow!("MCP 配置的 `mcpServers` 不是对象，已中止"))?;

    let entry = match timeout_ms {
        Some(ms) => json!({
            "command": command,
            "args": [ARG_MCP],
            "timeout": ms,
        }),
        None => json!({
            "command": command,
            "args": [ARG_MCP],
        }),
    };
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

fn json_entry_matches(value: &Value, command: &str, timeout_ms: Option<i64>) -> bool {
    let Some(entry) = json_entry(value) else {
        return false;
    };
    let cmd_ok = entry.get("command").and_then(|v| v.as_str()) == Some(command);
    let args_ok = entry
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.len() == 1 && a[0].as_str() == Some(ARG_MCP))
        .unwrap_or(false);
    // Claude 需精确匹配 timeout（旧条目无 timeout → 视为需更新）；Cursor 不该有该字段。
    let timeout_ok = match timeout_ms {
        Some(ms) => entry.get("timeout").and_then(|v| v.as_i64()) == Some(ms),
        None => entry.get("timeout").is_none(),
    };
    cmd_ok && args_ok && timeout_ok
}

// MARK: - TOML（Codex）：toml_edit 保留格式最小化编辑

/// upsert `[mcp_servers.askhuman]`（command/args/startup_timeout_sec/tool_timeout_sec，
/// Grok 额外写 `tool_timeouts = { ask = 86400 }`）。
fn apply_install_toml(target: AgentTarget, text: &str, command: &str) -> Result<String> {
    use toml_edit::{value, Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};
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

    entry.insert("command", value(command));
    let mut args = Array::new();
    args.push(ARG_MCP);
    entry.insert("args", value(args));
    entry.insert("startup_timeout_sec", value(startup));
    entry.insert("tool_timeout_sec", value(tool));
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
    Ok(doc.to_string())
}

/// 移除 `[mcp_servers.askhuman]`；若 `mcp_servers` 因此变空则删除该表。
fn apply_uninstall_toml(text: &str) -> Result<String> {
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
    Ok(doc.to_string())
}

fn toml_installed(text: &str) -> bool {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    toml_entry(&doc).is_some()
}

fn toml_entry<'a>(doc: &'a toml_edit::DocumentMut) -> Option<&'a dyn toml_edit::TableLike> {
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
        .map(|a| a.len() == 1 && a.get(0).and_then(|x| x.as_str()) == Some(ARG_MCP))
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
    cmd_ok && args_ok && startup_ok && tool_ok && ask_ok
}

// MARK: - 私有 IO / 工具

fn current_exe_string() -> Result<String> {
    let p = std::env::current_exe().context("failed to resolve current exe path")?;
    Ok(p.to_string_lossy().to_string())
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(path, text.as_bytes())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/Users/u/.local/bin/AskHuman";

    fn to_value(text: &str) -> Value {
        jsonc_parser::parse_to_serde_value::<Value>(text, &ParseOptions::default()).unwrap()
    }

    // ── JSON ──

    #[test]
    fn json_install_into_empty_creates_entry() {
        let out = apply_install_json("", EXE, None).unwrap();
        let v = to_value(&out);
        assert_eq!(v["mcpServers"][SERVER_NAME]["command"], EXE);
        assert_eq!(v["mcpServers"][SERVER_NAME]["args"][0], ARG_MCP);
        // Cursor 风格（无 timeout）不写该字段。
        assert!(v["mcpServers"][SERVER_NAME].get("timeout").is_none());
        assert!(json_entry_matches(&v, EXE, None));
    }

    #[test]
    fn json_install_claude_writes_timeout() {
        let out = apply_install_json("", EXE, Some(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
        let v = to_value(&out);
        assert_eq!(
            v["mcpServers"][SERVER_NAME]["timeout"].as_i64(),
            Some(CLAUDE_TOOL_TIMEOUT_MS)
        );
        assert!(json_entry_matches(&v, EXE, Some(CLAUDE_TOOL_TIMEOUT_MS)));
        // 缺 timeout 的预期（Cursor）与之不匹配。
        assert!(!json_entry_matches(&v, EXE, None));
    }

    #[test]
    fn json_claude_old_entry_without_timeout_needs_update() {
        // 模拟旧版（无 timeout）安装后，按 Claude 预期校验应判定需更新。
        let old = apply_install_json("{}", EXE, None).unwrap();
        let v = to_value(&old);
        assert!(!json_entry_matches(&v, EXE, Some(CLAUDE_TOOL_TIMEOUT_MS)));
    }

    #[test]
    fn json_install_is_idempotent_fixpoint() {
        let a = apply_install_json("{}", EXE, Some(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
        let b = apply_install_json(&a, EXE, Some(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
        let c = apply_install_json(&b, EXE, Some(CLAUDE_TOOL_TIMEOUT_MS)).unwrap();
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
        let out = apply_install_json(input, EXE, None).unwrap();
        assert!(out.contains("// 用户注释，勿动"), "注释应原样保留");
        let v = to_value(&out);
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"][SERVER_NAME]["command"], EXE);
    }

    #[test]
    fn json_install_updates_command_in_place() {
        let old = apply_install_json("{}", "/old/AskHuman", None).unwrap();
        let new = apply_install_json(&old, EXE, None).unwrap();
        let v = to_value(&new);
        assert_eq!(v["mcpServers"][SERVER_NAME]["command"], EXE);
        assert!(json_entry_matches(&v, EXE, None));
        assert!(!json_entry_matches(&v, "/old/AskHuman", None));
    }

    #[test]
    fn json_install_aborts_on_non_object_root() {
        assert!(apply_install_json("[]", EXE, None).is_err());
    }

    #[test]
    fn json_install_aborts_on_parse_error() {
        assert!(apply_install_json("{ \"mcpServers\": ", EXE, None).is_err());
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
        let only = apply_install_json("{}", EXE, None).unwrap();
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
            "[mcp_servers.askhuman]\ncommand = \"{EXE}\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 30.0\ntool_timeout_sec = 86400.0\n"
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
        let out = apply_uninstall_toml(&input).unwrap();
        assert!(!toml_installed(&out));
        assert!(out.contains("[mcp_servers.other]"), "他人 server 应保留");
    }

    #[test]
    fn toml_uninstall_drops_empty_servers_table() {
        let only = apply_install_toml(CODEX, "", EXE).unwrap();
        let out = apply_uninstall_toml(&only).unwrap();
        assert!(!out.contains("mcp_servers"), "空 mcp_servers 表应删除");
    }

    #[test]
    fn toml_uninstall_noop_when_absent() {
        let input = "model = \"gpt-5\"\n";
        let out = apply_uninstall_toml(input).unwrap();
        assert!(out.contains("model = \"gpt-5\""));
        assert!(!toml_installed(&out));
    }

    #[test]
    fn toml_uninstall_aborts_on_parse_error() {
        assert!(apply_uninstall_toml("[mcp_servers").is_err());
    }
}
