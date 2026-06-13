//! Agent 生命周期 hook 集成（实验性功能，spec D16/D17）：为 Claude Code / Codex / Cursor 安装
//! **用户级** lifecycle hook，调用隐藏子命令 `AskHuman __agent-hook <agent> <event>` 上报事件。
//!
//! 设计要点：
//! - 与既有 timeout hook（`askhuman-timeout.sh`）**完全独立**：本模块只增删命令里含 `__agent-hook`
//!   标记的条目，保留用户其它 hook 与文件格式（jsonc CST / toml_edit 做保留格式的最小化编辑）。
//! - Claude/Cursor 是 JSON 配置（无信任）；Codex 是 `~/.codex/hooks.json` 定义 + `~/.codex/config.toml`
//!   `[hooks.state]` 写信任哈希（复刻 codex `version_for_toml`，见 `demo/.../codex-trust.cjs` 与
//!   FINDINGS §6.2）。Codex 无 SessionEnd 事件。
//! - 去重（Cursor 双触发）由 reporter 运行时按 env 判定真实家族解决，故三家可同时安装、互不影响。

use crate::agents::AgentKind;
use crate::paths;
use anyhow::{anyhow, Context, Result};
use jsonc_parser::cst::{CstNode, CstRootNode};
use jsonc_parser::json;
use jsonc_parser::ParseOptions;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// 识别本功能注入条目的命令标记。
pub const MARKER: &str = "__agent-hook";

/// 生命周期 hook 安装状态（前端实验区开关据此渲染）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleStatus {
    /// 是否已安装（至少一个事件已注册本功能条目）。
    pub installed: bool,
    /// 已安装但需更新（命令路径变化 / 事件缺失 / Codex 信任缺失或不匹配）。
    pub outdated: bool,
    /// 当前平台是否支持（仅 unix）。
    pub supported: bool,
}

/// 某家族要注册的事件：(配置文件里的事件键, 归一化 lifecycle 事件)。
/// Codex 额外需要 snake_case 标签用于信任键/身份（见 `codex_label`）。
fn events(kind: AgentKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        // Claude：SessionStart/UserPromptSubmit/Stop/SessionEnd。
        AgentKind::Claude => &[
            ("SessionStart", "session-start"),
            ("UserPromptSubmit", "turn-start"),
            ("Stop", "turn-end"),
            ("SessionEnd", "session-end"),
        ],
        // Codex：无 SessionEnd（FINDINGS §6）。
        AgentKind::Codex => &[
            ("SessionStart", "session-start"),
            ("UserPromptSubmit", "turn-start"),
            ("Stop", "turn-end"),
        ],
        // Cursor：原生 camelCase 事件。
        AgentKind::Cursor => &[
            ("sessionStart", "session-start"),
            ("beforeSubmitPrompt", "turn-start"),
            ("stop", "turn-end"),
            ("sessionEnd", "session-end"),
        ],
    }
}

/// Codex 事件键（PascalCase）→ 信任用 snake_case 标签（hooks/src/lib.rs::hook_event_key_label）。
fn codex_label(event_key: &str) -> Option<&'static str> {
    match event_key {
        "SessionStart" => Some("session_start"),
        "UserPromptSubmit" => Some("user_prompt_submit"),
        "Stop" => Some("stop"),
        _ => None,
    }
}

pub fn supported() -> bool {
    cfg!(unix)
}

/// 当前可执行文件绝对路径（hook 命令调用它）。
fn exe_path() -> Result<String> {
    let p = std::env::current_exe().context("failed to resolve current exe path")?;
    Ok(p.to_string_lossy().to_string())
}

/// hook 命令字符串：`"<exe>" __agent-hook <agent> <lifecycle-event>`。
fn hook_command(exe: &str, kind: AgentKind, lc_event: &str) -> String {
    format!("\"{}\" {} {} {}", exe, MARKER, kind.as_str(), lc_event)
}

// ===== 对外：状态 / 安装 / 卸载 =====

pub fn status(kind: AgentKind) -> LifecycleStatus {
    if !supported() {
        return LifecycleStatus {
            installed: false,
            outdated: false,
            supported: false,
        };
    }
    match kind {
        AgentKind::Claude => json_status(kind, &paths::claude_settings_json(), Shape::Nested),
        AgentKind::Cursor => json_status(kind, &paths::cursor_hooks_json(), Shape::Flat),
        AgentKind::Codex => codex_status(),
    }
}

pub fn install(kind: AgentKind) -> Result<String> {
    let exe = exe_path()?;
    match kind {
        AgentKind::Claude => json_install(kind, &exe, &paths::claude_settings_json(), Shape::Nested)?,
        AgentKind::Cursor => json_install(kind, &exe, &paths::cursor_hooks_json(), Shape::Flat)?,
        AgentKind::Codex => codex_install(&exe)?,
    }
    Ok(message("cmd.lifecycleInstalled"))
}

pub fn uninstall(kind: AgentKind) -> Result<String> {
    match kind {
        AgentKind::Claude => json_uninstall(&paths::claude_settings_json(), Shape::Nested)?,
        AgentKind::Cursor => json_uninstall(&paths::cursor_hooks_json(), Shape::Flat)?,
        AgentKind::Codex => codex_uninstall()?,
    }
    Ok(message("cmd.lifecycleRemoved"))
}

fn message(key: &'static str) -> String {
    let lang = crate::i18n::Lang::current();
    crate::i18n::tr(lang, key).to_string()
}

// ===== JSON（Claude=Nested / Cursor=Flat） =====

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// `{ "Event": [ { "hooks": [ { "type":"command", "command": ... } ] } ] }`（Claude/Codex）。
    Nested,
    /// `{ "event": [ { "command": ... } ] }`（Cursor，含顶层 `version`）。
    Flat,
}

/// 元素（事件数组的一项）是否含本功能命令（按 shape 解析）。
fn elem_has_marker(elem: &Value, shape: Shape) -> bool {
    match shape {
        Shape::Nested => elem
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|arr| arr.iter().any(|h| cmd_has_marker(h.get("command"))))
            .unwrap_or(false),
        Shape::Flat => cmd_has_marker(elem.get("command")),
    }
}

fn cmd_has_marker(cmd: Option<&Value>) -> bool {
    cmd.and_then(|c| c.as_str())
        .map(|c| c.contains(MARKER))
        .unwrap_or(false)
}

fn elem_node_has_marker(node: &CstNode, shape: Shape) -> bool {
    node.to_serde_value()
        .map(|v| elem_has_marker(&v, shape))
        .unwrap_or(false)
}

fn read_value(path: &std::path::Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    jsonc_parser::parse_to_serde_value(&text, &ParseOptions::default()).ok()
}

fn json_status(kind: AgentKind, path: &std::path::Path, shape: Shape) -> LifecycleStatus {
    let exe = exe_path().unwrap_or_default();
    let Some(root) = read_value(path) else {
        return LifecycleStatus {
            installed: false,
            outdated: false,
            supported: true,
        };
    };
    let hooks = root.get("hooks");
    let mut any = false;
    let mut complete = true;
    for (event_key, lc) in events(kind) {
        let want = hook_command(&exe, kind, lc);
        let arr = hooks.and_then(|h| h.get(event_key)).and_then(|a| a.as_array());
        let has_ours = arr
            .map(|a| a.iter().any(|e| elem_has_marker(e, shape)))
            .unwrap_or(false);
        let has_exact = arr
            .map(|a| a.iter().any(|e| elem_cmd_equals(e, shape, &want)))
            .unwrap_or(false);
        if has_ours {
            any = true;
        }
        if !has_exact {
            complete = false;
        }
    }
    LifecycleStatus {
        installed: any,
        outdated: any && !complete,
        supported: true,
    }
}

/// 元素是否恰好含给定命令（用于 outdated：路径变化检测）。
fn elem_cmd_equals(elem: &Value, shape: Shape, want: &str) -> bool {
    match shape {
        Shape::Nested => elem
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|arr| arr.iter().any(|h| h.get("command").and_then(|c| c.as_str()) == Some(want)))
            .unwrap_or(false),
        Shape::Flat => elem.get("command").and_then(|c| c.as_str()) == Some(want),
    }
}

fn json_install(kind: AgentKind, exe: &str, path: &std::path::Path, shape: Shape) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let updated = apply_json_install(kind, exe, &text, shape)?;
    write_text(path, &updated)
}

fn json_uninstall(path: &std::path::Path, shape: Shape) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let updated = apply_json_uninstall(&text, shape)?;
    write_text(path, &updated)
}

/// 在 JSON 文本中插入/更新本功能各事件条目（CST 保留格式）。仅触碰本功能条目。
fn apply_json_install(kind: AgentKind, exe: &str, text: &str, shape: Shape) -> Result<String> {
    let source = if text.trim().is_empty() { "{}" } else { text };
    let root = CstRootNode::parse(source, &ParseOptions::default())
        .map_err(|e| anyhow!("解析配置失败，已中止（不覆盖原文件）：{e}"))?;
    let root_obj = root
        .object_value_or_create()
        .ok_or_else(|| anyhow!("配置根不是 JSON 对象，已中止"))?;

    // Cursor hooks.json 需要顶层 version=1。
    if shape == Shape::Flat && root_obj.get("version").is_none() {
        root_obj.append("version", json!(1));
    }

    let hooks = root_obj
        .object_value_or_create("hooks")
        .ok_or_else(|| anyhow!("配置的 'hooks' 不是对象，已中止"))?;

    for (event_key, lc) in events(kind) {
        let command = hook_command(exe, kind, lc);
        let cmd = command.as_str();
        let arr = hooks
            .array_value_or_create(event_key)
            .ok_or_else(|| anyhow!("配置的 '{event_key}' 不是数组，已中止"))?;
        let mut replaced = false;
        for e in arr.elements() {
            if !elem_node_has_marker(&e, shape) {
                continue;
            }
            if !replaced {
                if let Some(obj) = e.as_object() {
                    match shape {
                        Shape::Nested => {
                            obj.replace_with(json!({ "hooks": [ { "type": "command", "command": cmd } ] }));
                        }
                        Shape::Flat => {
                            obj.replace_with(json!({ "command": cmd }));
                        }
                    }
                    replaced = true;
                    continue;
                }
            }
            e.remove();
        }
        if !replaced {
            arr.ensure_multiline();
            match shape {
                Shape::Nested => {
                    arr.append(json!({ "hooks": [ { "type": "command", "command": cmd } ] }));
                }
                Shape::Flat => {
                    arr.append(json!({ "command": cmd }));
                }
            }
        }
    }
    Ok(root.to_string())
}

/// 在 JSON 文本中移除本功能各事件条目；事件数组变空则删除该键。仅触碰本功能条目。
fn apply_json_uninstall(text: &str, shape: Shape) -> Result<String> {
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|e| anyhow!("解析配置失败，已中止（不覆盖原文件）：{e}"))?;
    let Some(root_obj) = root.object_value() else {
        return Ok(root.to_string());
    };
    let Some(hooks) = root_obj.object_value("hooks") else {
        return Ok(root.to_string());
    };
    // 事件键从同文本的 serde 解析取（避免依赖 CST 的属性枚举 API）；逐个清理本功能元素，空数组删键。
    let parsed: Option<Value> =
        jsonc_parser::parse_to_serde_value(text, &ParseOptions::default()).ok();
    let keys: Vec<String> = parsed
        .as_ref()
        .and_then(|v| v.get("hooks"))
        .and_then(|h| h.as_object())
        .map(|o| o.keys().cloned().collect::<Vec<String>>())
        .unwrap_or_default();
    for key in keys {
        if let Some(arr) = hooks.array_value(&key) {
            for e in arr.elements() {
                if elem_node_has_marker(&e, shape) {
                    e.remove();
                }
            }
            if arr.elements().is_empty() {
                if let Some(prop) = hooks.get(&key) {
                    prop.remove();
                }
            }
        }
    }
    Ok(root.to_string())
}

// ===== Codex（hooks.json 定义 + config.toml 信任哈希） =====

fn codex_install(exe: &str) -> Result<()> {
    // 1) 写 ~/.codex/hooks.json（Nested shape，与 Claude 同构）。
    let path = paths::codex_hooks_json();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let updated = apply_json_install(AgentKind::Codex, exe, &text, Shape::Nested)?;
    write_text(&path, &updated)?;

    // 2) 由最终文件计算信任键与哈希，写入 ~/.codex/config.toml [hooks.state]。
    let entries = codex_trust_entries(&path)?;
    write_codex_trust(&entries)
}

fn codex_uninstall() -> Result<()> {
    let path = paths::codex_hooks_json();
    if let Ok(text) = std::fs::read_to_string(&path) {
        let updated = apply_json_uninstall(&text, Shape::Nested)?;
        write_text(&path, &updated)?;
    }
    // 移除 config.toml 中所有以本 hooks.json 绝对路径为前缀的信任条目。
    remove_codex_trust_for(&path)
}

fn codex_status() -> LifecycleStatus {
    let base = json_status(AgentKind::Codex, &paths::codex_hooks_json(), Shape::Nested);
    if !base.installed {
        return base;
    }
    // 信任校验：每个期望键的 trusted_hash 必须存在且匹配。
    let trust_ok = match codex_trust_entries(&paths::codex_hooks_json()) {
        Ok(entries) if !entries.is_empty() => {
            let have = read_codex_trust();
            entries
                .iter()
                .all(|(k, h)| have.get(k).map(|v| v == h).unwrap_or(false))
        }
        _ => false,
    };
    LifecycleStatus {
        installed: true,
        outdated: base.outdated || !trust_ok,
        supported: true,
    }
}

/// 由 hooks.json 实际结构计算本功能各 handler 的 (信任键, trusted_hash)。
/// 信任键 = `<abs hooks.json>:<snake_label>:<group_index>:<handler_index>`。
fn codex_trust_entries(hooks_json: &std::path::Path) -> Result<Vec<(String, String)>> {
    // 信任键前缀必须与 codex 构造的 `hooks_config_folder/hooks.json` 显示路径逐字一致。
    // codex 用 `$CODEX_HOME(=~/.codex)/hooks.json` 且不做符号链接规范化，故这里也用原始绝对路径
    // （不 canonicalize，以免解析符号链接后与 codex 的路径分叉）。
    let abs_str = hooks_json.to_string_lossy().to_string();
    let Some(root) = read_value(hooks_json) else {
        return Ok(Vec::new());
    };
    let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (event_key, groups) in hooks {
        let Some(label) = codex_label(event_key) else {
            continue;
        };
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for (gi, group) in groups.iter().enumerate() {
            let Some(handlers) = group.get("hooks").and_then(|h| h.as_array()) else {
                continue;
            };
            for (hi, handler) in handlers.iter().enumerate() {
                let cmd = handler.get("command").and_then(|c| c.as_str());
                let is_command = handler.get("type").and_then(|t| t.as_str()) == Some("command");
                let Some(cmd) = cmd else { continue };
                if !is_command || !cmd.contains(MARKER) {
                    continue;
                }
                let key = format!("{abs_str}:{label}:{gi}:{hi}");
                let hash = codex_trusted_hash(label, cmd);
                out.push((key, hash));
            }
        }
    }
    Ok(out)
}

/// 复刻 codex `version_for_toml(NormalizedHookIdentity)`：
/// `"sha256:" + hex(sha256(canonical_compact_json(identity)))`。
/// identity = { event_name, hooks:[{type:"command", command, timeout:600, async:false}] }
/// （我们的事件 matcher 恒 None → 省略；timeout 默认 600；async 默认 false）。
fn codex_trusted_hash(label: &str, command: &str) -> String {
    let identity = serde_json::json!({
        "event_name": label,
        "hooks": [ {
            "type": "command",
            "command": command,
            "timeout": 600,
            "async": false,
        } ],
    });
    let mut buf = String::new();
    canonical_compact(&identity, &mut buf);
    let mut hasher = Sha256::new();
    hasher.update(buf.as_bytes());
    let hex = hasher.finalize();
    format!("sha256:{}", hex_encode(&hex))
}

/// 递归输出「键排序 + 紧凑」的规范化 JSON（与 codex `canonical_json` 一致，不依赖 Map 排序实现）。
fn canonical_compact(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                canonical_compact(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, e) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_compact(e, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 读取 config.toml 中现有的 `[hooks.state.<key>] trusted_hash`（key → hash）。
fn read_codex_trust() -> std::collections::HashMap<String, String> {
    use toml_edit::DocumentMut;
    let mut map = std::collections::HashMap::new();
    let Ok(text) = std::fs::read_to_string(paths::codex_config_toml()) else {
        return map;
    };
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return map;
    };
    if let Some(state) = doc
        .get("hooks")
        .and_then(|h| h.as_table())
        .and_then(|h| h.get("state"))
        .and_then(|s| s.as_table())
    {
        for (key, item) in state.iter() {
            if let Some(hash) = item
                .as_table()
                .and_then(|t| t.get("trusted_hash"))
                .and_then(|v| v.as_str())
            {
                map.insert(key.to_string(), hash.to_string());
            }
        }
    }
    map
}

/// 写入/更新 config.toml 的 `[hooks.state.<key>] trusted_hash`（toml_edit 保留格式最小化编辑）。
fn write_codex_trust(entries: &[(String, String)]) -> Result<()> {
    use toml_edit::{DocumentMut, Item, Table};
    let path = paths::codex_config_toml();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc = existing
        .parse::<DocumentMut>()
        .map_err(|e| anyhow!("解析 config.toml 失败，已中止（不覆盖原文件）：{e}"))?;

    fn ensure_intermediate<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table> {
        if !parent.contains_key(key) {
            let mut t = Table::new();
            t.set_implicit(true);
            parent.insert(key, Item::Table(t));
        }
        parent
            .get_mut(key)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| anyhow!("config.toml 中 `{key}` 不是表，已中止"))
    }

    let hooks = ensure_intermediate(doc.as_table_mut(), "hooks")?;
    let state = ensure_intermediate(hooks, "state")?;
    for (key, hash) in entries {
        if !state.contains_key(key) {
            state.insert(key, Item::Table(Table::new()));
        }
        let entry = state
            .get_mut(key)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| anyhow!("config.toml 中信任条目不是表，已中止"))?;
        entry.insert("trusted_hash", toml_edit::value(hash.clone()));
    }
    write_toml(&path, &doc.to_string())
}

/// 移除 config.toml 中所有以 `<abs hooks.json>:` 为前缀的信任条目；空表则删除。
fn remove_codex_trust_for(hooks_json: &std::path::Path) -> Result<()> {
    use toml_edit::{DocumentMut, Item};
    let path = paths::codex_config_toml();
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut doc = existing
        .parse::<DocumentMut>()
        .map_err(|e| anyhow!("解析 config.toml 失败，已中止（不覆盖原文件）：{e}"))?;
    let prefix = format!("{}:", hooks_json.to_string_lossy());

    let mut changed = false;
    if let Some(hooks) = doc.get_mut("hooks").and_then(Item::as_table_mut) {
        if let Some(state) = hooks.get_mut("state").and_then(Item::as_table_mut) {
            let before = state.len();
            state.retain(|k, _| !k.starts_with(&prefix));
            changed = state.len() != before;
            if state.is_empty() {
                hooks.remove("state");
            }
        }
        if hooks.is_empty() {
            doc.as_table_mut().remove("hooks");
        }
    }
    if changed {
        write_toml(&path, &doc.to_string())?;
    }
    Ok(())
}

// ===== 私有 IO =====

fn write_text(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(path, text.as_bytes())
}

fn write_toml(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(path, text.as_bytes())
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/opt/AskHuman";

    fn to_value(text: &str) -> Value {
        jsonc_parser::parse_to_serde_value(text, &ParseOptions::default()).unwrap()
    }

    #[test]
    fn claude_install_adds_four_events_nested() {
        let out = apply_json_install(AgentKind::Claude, EXE, "{}", Shape::Nested).unwrap();
        let v = to_value(&out);
        for ev in ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"] {
            let arr = v["hooks"][ev].as_array().unwrap();
            assert_eq!(arr.len(), 1);
            let cmd = arr[0]["hooks"][0]["command"].as_str().unwrap();
            assert!(cmd.contains(MARKER) && cmd.contains("claude"));
        }
    }

    #[test]
    fn cursor_install_flat_with_version() {
        let out = apply_json_install(AgentKind::Cursor, EXE, "{}", Shape::Flat).unwrap();
        let v = to_value(&out);
        assert_eq!(v["version"], 1);
        let arr = v["hooks"]["beforeSubmitPrompt"].as_array().unwrap();
        assert_eq!(arr[0]["command"].as_str().unwrap().contains(MARKER), true);
        assert!(arr[0].get("hooks").is_none(), "flat 形状无嵌套 hooks");
    }

    #[test]
    fn install_is_idempotent_fixpoint() {
        let a = apply_json_install(AgentKind::Claude, EXE, "{}", Shape::Nested).unwrap();
        let b = apply_json_install(AgentKind::Claude, EXE, &a, Shape::Nested).unwrap();
        let c = apply_json_install(AgentKind::Claude, EXE, &b, Shape::Nested).unwrap();
        assert_eq!(b, c);
        let v = to_value(&c);
        assert_eq!(v["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn install_preserves_existing_timeout_hook() {
        // 既有 timeout hook（PreToolUse/Bash + askhuman-timeout.sh）应原样保留。
        let input = "{ \"hooks\": { \"PreToolUse\": [ { \"matcher\": \"Bash\", \"hooks\": [ { \"type\": \"command\", \"command\": \"x/askhuman-timeout.sh\" } ] } ] } }";
        let out = apply_json_install(AgentKind::Claude, EXE, input, Shape::Nested).unwrap();
        let v = to_value(&out);
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "x/askhuman-timeout.sh"
        );
        assert!(v["hooks"]["SessionStart"].as_array().unwrap().len() == 1);
    }

    #[test]
    fn uninstall_removes_only_ours() {
        let input = "{ \"hooks\": { \"PreToolUse\": [ { \"matcher\": \"Bash\", \"hooks\": [ { \"type\": \"command\", \"command\": \"x/askhuman-timeout.sh\" } ] } ], \"SessionStart\": [ { \"hooks\": [ { \"type\": \"command\", \"command\": \"a __agent-hook claude session-start\" } ] } ] } }";
        let out = apply_json_uninstall(input, Shape::Nested).unwrap();
        let v = to_value(&out);
        assert!(v["hooks"].get("SessionStart").is_none(), "空数组应删键");
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "x/askhuman-timeout.sh",
            "timeout hook 应保留"
        );
    }

    #[test]
    fn parse_error_aborts() {
        assert!(apply_json_install(AgentKind::Claude, EXE, "{ \"hooks\": ", Shape::Nested).is_err());
        assert!(apply_json_uninstall("{ \"hooks\": ", Shape::Nested).is_err());
    }

    #[test]
    fn codex_trusted_hash_matches_reference_algorithm() {
        // 与 codex-trust.cjs 同输入应得同输出（键排序紧凑 JSON 的 sha256）。
        let cmd = "\"/opt/AskHuman\" __agent-hook codex session-start";
        let h = codex_trusted_hash("session_start", cmd);
        // 独立计算参考值。
        let serialized = format!(
            "{{\"event_name\":\"session_start\",\"hooks\":[{{\"async\":false,\"command\":{},\"timeout\":600,\"type\":\"command\"}}]}}",
            serde_json::to_string(cmd).unwrap()
        );
        let mut hasher = Sha256::new();
        hasher.update(serialized.as_bytes());
        let want = format!("sha256:{}", hex_encode(&hasher.finalize()));
        assert_eq!(h, want);
    }

    #[test]
    fn canonical_compact_sorts_keys() {
        let v = serde_json::json!({ "b": 1, "a": [ { "y": 2, "x": 1 } ] });
        let mut s = String::new();
        canonical_compact(&v, &mut s);
        assert_eq!(s, "{\"a\":[{\"x\":1,\"y\":2}],\"b\":1}");
    }
}
