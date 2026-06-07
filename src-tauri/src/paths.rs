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
