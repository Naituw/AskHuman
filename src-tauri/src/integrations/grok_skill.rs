//! Grok「指令载体」：把必读交互协议装进 Grok 的 **skill**
//! `~/.grok/skills/interaction-protocol/SKILL.md`（AskHuman 独占该 skill 目录，整文件拥有）。
//!
//! 为何用 skill 而非全局 rules：Grok 默认模型 Composer **不读** `~/.grok/AGENTS.md`（调研证实，见
//! `docs/specs/grok-cli-integration-research.md`），skill 是唯一能同时覆盖 Composer 与 Grok Build 的
//! 入口。skill 属**弱约束**——模型需先判定相关才加载；本轮把 skill 重定位为「**无条件必读的交互协议**」，
//! 在 `frontmatter` 的 `description` 第一句无条件要求「每个 session 先读本 skill」（消解「需要提问才加载」的
//! 自指悖论），协议正文只对「调用 AskHuman 的 `ask`」声明「MCP 优先于 shell」，不禁止一般 shell 用法
//! （见 `prompts::grok_skill_body`）。
//!
//! 落盘为「整文件拥有」：安装/更新即覆盖为最新内容；卸载删除文件（目录空则一并删）。内容含托管标记，
//! 避免误删用户在同路径放置的无关文件。

use crate::paths;
use anyhow::{Context, Result};
use std::path::Path;

/// skill 文件内的托管标记（识别本应用拥有的文件；用户不应手改）。
pub const SKILL_MARK: &str = "<!-- AskHuman managed skill: DO NOT EDIT (managed by AskHuman) -->";

/// skill 的 YAML frontmatter（`name` + `description`）。
///
/// **定位（本轮重构）**：把 skill 从「需要提问时才加载的 AskHuman 技能」重定位为「**无条件必读的交互
/// 协议**」。原因：Grok skill 属懒加载 + 相关性门控，而「向人提问」这个动作对模型是内建行为、不存在能力
/// 缺口，故按「需要提问时加载」写会陷入自指悖论——最需要它的时刻恰恰意识不到需要它。改为在 description
/// 第一句无条件要求「每个 session 先读本 skill」，消解该悖论。
///
/// **为何全写进 `description`、不用 `when-to-use`**：实测（grok 0.2.82，Composer 与 Grok Build 两 harness）
/// 证实 `when-to-use` 仅以 `Use when:` 标签拼在 `description` 之后、进同一段常驻文本，对「无条件必读」策略
/// 无额外价值，故合并为单一 `description`。description 是 skill 唯一常驻注入 prompt 的字段（正文 body 需被
/// 判定相关后才加载），因此把协议最关键的事实（普通输出人类不可见）前置到此处兜底。
fn frontmatter() -> String {
    "---\n\
name: interaction-protocol\n\
description: >-\n  \
Mandatory interaction protocol for this environment. You MUST read this skill FIRST in EVERY\n  \
session, before doing anything else — unconditionally, for every task and topic, even if it seems\n  \
unrelated, and do NOT wait until you have a question. It ALWAYS applies, with one exception: if\n  \
you are a subagent, it does not apply to you. Key fact: the human user can ONLY see what is\n  \
delivered through the AskHuman `ask` MCP tool;\n  \
your normal chat output is invisible to them.\n\
---\n"
        .to_string()
}

/// 最新的 SKILL.md 完整内容（frontmatter + 托管标记 + 交互协议正文）。
pub fn content() -> String {
    format!(
        "{}{}\n\n{}\n",
        frontmatter(),
        SKILL_MARK,
        crate::prompts::grok_skill_body()
    )
}

/// 当前平台是否支持（skill 为纯文件读写，跨平台）。
pub fn supported() -> bool {
    true
}

/// skill 文件展示路径（home 前缀折叠为 `~`）。
pub fn display_path() -> String {
    collapse_home(&paths::grok_skill_md())
}

/// 是否已安装（文件存在且含本应用托管标记）。
pub fn is_installed() -> bool {
    std::fs::read_to_string(paths::grok_skill_md())
        .map(|t| t.contains(SKILL_MARK))
        .unwrap_or(false)
}

/// 已安装但内容与最新版本不一致 → 需更新。
pub fn needs_update() -> bool {
    match std::fs::read_to_string(paths::grok_skill_md()) {
        Ok(t) => t.contains(SKILL_MARK) && t != content(),
        Err(_) => false,
    }
}

/// 安装：写入最新 skill（整文件覆盖）。
pub fn install() -> Result<String> {
    write_skill()?;
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.skillInstalled").to_string())
}

/// 更新：与安装同样写入，仅反馈文案不同。
pub fn update() -> Result<String> {
    write_skill()?;
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.skillUpdated").to_string())
}

fn write_skill() -> Result<()> {
    let path = paths::grok_skill_md();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create directory: {}", dir.display()))?;
    }
    atomic_write(&path, content().as_bytes())
        .with_context(|| format!("failed to write skill file: {}", path.display()))?;
    Ok(())
}

/// 卸载：仅当文件由本应用拥有（含托管标记）时删除；skill 目录随之为空则一并删除。
pub fn uninstall() -> Result<String> {
    let path = paths::grok_skill_md();
    if let Ok(text) = std::fs::read_to_string(&path) {
        if text.contains(SKILL_MARK) {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove skill file: {}", path.display()))?;
            // 目录若已空（AskHuman 独占该 skill 目录），一并清理。
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
    Ok(crate::i18n::tr(crate::i18n::Lang::current(), "cmd.skillRemoved").to_string())
}

/// 在文件管理器中定位 skill 文件。
pub fn reveal() {
    let path = paths::grok_skill_md();
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

/// 用系统默认程序打开 skill 文件。
pub fn open() {
    let path = paths::grok_skill_md();
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

    #[test]
    fn content_has_frontmatter_marker_and_protocol() {
        let c = content();
        assert!(c.starts_with("---\n"));
        assert!(c.contains("name: interaction-protocol"));
        // description 第一句须为「无条件必读」定位，且含「普通输出人类不可见」这条兜底事实。
        assert!(c.contains("You MUST read this skill FIRST in EVERY"));
        assert!(c.contains("It ALWAYS applies, with one exception: if"));
        assert!(c.contains("you are a subagent, it does not apply to you."));
        assert!(!c.contains("It ALWAYS applies, with no exceptions."));
        assert!(c.contains("your normal chat output is invisible to them"));
        assert!(c.contains(SKILL_MARK));
        assert!(c.contains("<mandatory_interaction_protocol>"));
        assert!(c.contains("the AskHuman `ask` tool"));
    }
}
