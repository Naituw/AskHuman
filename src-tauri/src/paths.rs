//! 跨平台路径：配置目录、临时目录、Cursor 相关路径。

use std::path::PathBuf;

/// 用户主目录（解析失败时回退到当前目录，保证不 panic）。
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 配置目录 `~/.askhuman`（写入与读取的规范位置）。
pub fn config_dir() -> PathBuf {
    home().join(".askhuman")
}

/// 旧版配置目录 `~/.humaninloop`（仅用于向后兼容读取）。
pub fn legacy_config_dir() -> PathBuf {
    home().join(".humaninloop")
}

/// 配置文件 `~/.askhuman/config.json`。
pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// 旧版配置文件 `~/.humaninloop/config.json`（仅用于向后兼容读取）。
pub fn legacy_config_file() -> PathBuf {
    legacy_config_dir().join("config.json")
}

/// 本次请求的图片落盘目录 `temp/askhuman/<request_id>/`。
pub fn request_temp_dir(request_id: &str) -> PathBuf {
    std::env::temp_dir().join("askhuman").join(request_id)
}

/// 回复历史文件 `~/.askhuman/history.jsonl`（每行一条 JSON）。
pub fn history_file() -> PathBuf {
    config_dir().join("history.jsonl")
}

/// 回复历史写入锁 `~/.askhuman/history.lock`（写/裁剪/清空时持有）。
pub fn history_lock() -> PathBuf {
    config_dir().join("history.lock")
}

/// 版本自更新状态文件 `~/.askhuman/update.json`（最新版本/检查时间/忽略集合/待生效）。
pub fn update_state_file() -> PathBuf {
    config_dir().join("update.json")
}

/// Agent 生命周期追踪状态文件 `~/.askhuman/agents.json`（daemon 持久化、重启复核用）。
pub fn agents_file() -> PathBuf {
    config_dir().join("agents.json")
}

/// Cursor 目录 `~/.cursor`。
pub fn cursor_dir() -> PathBuf {
    home().join(".cursor")
}

/// `~/.cursor/hooks.json`。
pub fn cursor_hooks_json() -> PathBuf {
    cursor_dir().join("hooks.json")
}

/// `~/.cursor/hooks/askhuman-timeout.sh`。
pub fn cursor_hook_script() -> PathBuf {
    cursor_dir().join("hooks").join("askhuman-timeout.sh")
}

/// 旧版 hook 脚本 `~/.cursor/hooks/humaninloop-timeout.sh`（仅用于向后兼容清理）。
pub fn legacy_cursor_hook_script() -> PathBuf {
    cursor_dir().join("hooks").join("humaninloop-timeout.sh")
}

/// Cursor 全局规则目录 `~/.cursor/rules`（用户级，跨项目，文件为 `*.mdc`）。
pub fn cursor_rules_dir() -> PathBuf {
    cursor_dir().join("rules")
}

/// 本应用独占的 Cursor 全局规则文件 `~/.cursor/rules/askhuman.mdc`。
pub fn cursor_rule_file() -> PathBuf {
    cursor_rules_dir().join("askhuman.mdc")
}

/// Claude Code 配置目录 `~/.claude`。
pub fn claude_dir() -> PathBuf {
    home().join(".claude")
}

/// Claude Code 全局 memory 文件 `~/.claude/CLAUDE.md`（用户级，跨项目）。
pub fn claude_md() -> PathBuf {
    claude_dir().join("CLAUDE.md")
}

/// Claude Code 用户级设置文件 `~/.claude/settings.json`（hooks / env 等都写在此）。
pub fn claude_settings_json() -> PathBuf {
    claude_dir().join("settings.json")
}

/// Claude Code hook 脚本 `~/.claude/hooks/askhuman-timeout.sh`。
pub fn claude_hook_script() -> PathBuf {
    claude_dir().join("hooks").join("askhuman-timeout.sh")
}

/// Codex 配置目录 `~/.codex`。
pub fn codex_dir() -> PathBuf {
    home().join(".codex")
}

/// Codex 全局指令文件 `~/.codex/AGENTS.md`（用户级，跨项目）。
pub fn codex_agents_md() -> PathBuf {
    codex_dir().join("AGENTS.md")
}

/// Codex 用户级配置文件 `~/.codex/config.toml`（生命周期 hook 的信任 `[hooks.state]` 写于此）。
pub fn codex_config_toml() -> PathBuf {
    codex_dir().join("config.toml")
}

/// Codex 用户级 hook 定义文件 `~/.codex/hooks.json`（生命周期 hook 写于此；信任键以其绝对路径为前缀）。
pub fn codex_hooks_json() -> PathBuf {
    codex_dir().join("hooks.json")
}
