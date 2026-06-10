//! 帮助与版本文案（按界面语言本地化，源语言英文）。

use crate::i18n::{tr, Lang};
use std::path::Path;

/// 用于帮助文案的程序名，取自 argv[0] 的 basename。
/// 为空或异常时回退到 "AskHuman"。这样任何包装器/软链/改名调用
/// 都会显示对应的名字，无需调用方额外配合。
pub fn program_name() -> String {
    program_name_from(std::env::args().next().as_deref())
}

fn program_name_from(arg0: Option<&str>) -> String {
    arg0.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            Path::new(s)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "AskHuman".to_string())
}

pub fn help_text(lang: Lang) -> String {
    let prog = program_name();
    match lang {
        Lang::En => [
            format!("{prog} - Human-in-the-loop interaction tool"),
            String::new(),
            "Usage:".to_string(),
            format!("  {prog} <message> [options]   Open the ask popup; see --agent-help for arguments"),
            format!("  {prog} --settings            Open the settings window"),
            format!("  {prog} --history [--all]      Open the reply history window (current project; --all for every project)"),
            format!("  {prog} daemon <sub>          Manage the background daemon: status/stop/restart/start/logs"),
            format!("  {prog} --agent-help          Show concise AI-facing usage (asking)"),
            format!("  {prog} --help, -h            Show this help"),
            format!("  {prog} --version, -v         Show version"),
        ]
        .join("\n"),
        Lang::Zh => [
            format!("{prog} - Human-In-The-Loop 交互工具"),
            String::new(),
            "用法:".to_string(),
            format!("  {prog} <message> [选项]       启动询问弹窗，参数见 --agent-help 说明"),
            format!("  {prog} --settings            启动设置界面"),
            format!("  {prog} --history [--all]      启动回复历史窗口（默认当前项目；--all 查看全部项目）"),
            format!("  {prog} daemon <子命令>        管理后台 daemon：status/stop/restart/start/logs"),
            format!("  {prog} --agent-help          显示面向 AI 的精简用法（提问相关）"),
            format!("  {prog} --help, -h            显示此帮助信息"),
            format!("  {prog} --version, -v         显示版本信息"),
        ]
        .join("\n"),
    }
}

/// 面向 AI 的精简用法：仅含提问相关的调用方式、参数、结果区块与示例。
/// 结果区块标记取自 `i18n::tr`，确保「文档」与 `output.rs` 的「实际输出」一致。
pub fn agent_help_text(lang: Lang) -> String {
    let prog = program_name();
    let m_opts = tr(lang, "marker.options");
    let m_input = tr(lang, "marker.input");
    let m_images = tr(lang, "marker.images");
    let m_files = tr(lang, "marker.files");
    let m_status = tr(lang, "marker.status");
    match lang {
        Lang::En => [
            format!("{prog} — ask a human and collect their response."),
            String::new(),
            "Invocation:".to_string(),
            format!("  {prog} \"<Message>\" [-f \"<file>\" ...] [-q \"<question>\" [-o \"<option>\" ...] ...] [--no-markdown]"),
            String::new(),
            "Arguments:".to_string(),
            "  <Message>             Shared description for all questions (optional)".to_string(),
            "  --stdin               Read the <Message> from stdin instead of the argument (use a quoted heredoc to avoid shell quoting)".to_string(),
            "  -f, --file <path>     Attach a file or image to the Message (absolute/relative/~); repeatable".to_string(),
            "  -q, --question <text> Ask a question; repeatable; -q may be omitted when there is only one".to_string(),
            "  -o, --option <text>   Add a predefined answer option after a question".to_string(),
            "  -o!, --option! <text> Same as -o, and marks that option as your recommended answer".to_string(),
            "  --no-markdown         Disable Markdown rendering (applies to all descriptions/questions)".to_string(),
            String::new(),
            "User response (returned only when present, separated by blank lines):".to_string(),
            format!("  {m_opts}  Predefined options the user checked"),
            format!("  {m_input}    Free-form text the user typed"),
            format!("  {m_images}        Local paths of images the user attached (readable directly)"),
            format!("  {m_files}        Local paths of non-image files the user dropped (readable directly)"),
            format!("  {m_status}       Shown when the user cancels; follow its instructions to keep asking"),
            String::new(),
            "Multi-question output:".to_string(),
            "  Each question is grouped under \"# Qn\", with questions separated by \"---\"".to_string(),
            String::new(),
            "Examples:".to_string(),
            format!("  {prog} \"Proceed with deploy?\" -o! \"Proceed\" -o \"Stop\""),
            format!("  {prog} \"Review this change?\" -f ./diff.patch -q \"Continue?\" -o \"Continue\" -o \"Stop\""),
            format!("  {prog} \"A few things to confirm\" -q \"Keep logs?\" -o \"Keep\" -o \"Clear\" -q \"Enable cache?\" -o \"On\" -o \"Off\""),
            format!("  {prog} \"Plain text (no Markdown)\" --no-markdown"),
            format!("  {prog} --stdin -q \"Continue?\" -o \"Continue\" -o \"Stop\" <<'EOF'"),
            "# A long Markdown message with `backticks`, $vars and \"quotes\"".to_string(),
            "EOF".to_string(),
        ]
        .join("\n"),
        Lang::Zh => [
            format!("{prog} —— 向人类发起提问并收集回应。"),
            String::new(),
            "调用方式:".to_string(),
            format!("  {prog} \"<Message>\" [-f \"<文件>\" ...] [-q \"<问题>\" [-o \"<选项>\" ...] ...] [--no-markdown]"),
            String::new(),
            "参数说明:".to_string(),
            "  <Message>             所有问题的共享描述（可选）；".to_string(),
            "  --stdin               从标准输入读取 <Message>（用带引号的 heredoc 规避 shell 引号转义）".to_string(),
            "  -f, --file <path>     为 Message 附带文件或图片（绝对/相对/~），可多次出现".to_string(),
            "  -q, --question <text> 提出问题，可多次出现，只有一个问题时可省略 -q".to_string(),
            "  -o, --option <text>   跟随在问题后，添加预定义回答选项".to_string(),
            "  -o!, --option! <text> 同 -o，并把该选项标记为你的推荐答案".to_string(),
            "  --no-markdown         关闭 Markdown 渲染（对所有描述/问题生效）".to_string(),
            String::new(),
            "用户回应（仅在有内容时返回，空行分隔）:".to_string(),
            format!("  {m_opts}  用户勾选的预定义选项"),
            format!("  {m_input}    用户输入的自由文本"),
            format!("  {m_images}        用户附带图片的本地路径（可直接读取）"),
            format!("  {m_files}        用户拖入的非图片文件本地路径（可直接读取）"),
            format!("  {m_status}        用户取消时出现，请按其中说明继续询问"),
            String::new(),
            "多问题输出:".to_string(),
            "  每题以「# Qn」分组，题与题之间用「---」分隔".to_string(),
            String::new(),
            "使用示例:".to_string(),
            format!("  {prog} \"要继续部署吗？\" -o! \"继续\" -o \"停止\""),
            format!("  {prog} \"看看这个改动？\" -f ./diff.patch -q \"要继续吗？\" -o \"继续\" -o \"停止\""),
            format!("  {prog} \"以下是几处待确认\" -q \"保留日志？\" -o \"保留\" -o \"清除\" -q \"开启缓存？\" -o \"开\" -o \"关\""),
            format!("  {prog} \"纯文本内容（不渲染 Markdown）\" --no-markdown"),
            format!("  {prog} --stdin -q \"要继续吗？\" -o \"继续\" -o \"停止\" <<'EOF'"),
            "# 含 `反引号`、$VAR 与 \"引号\" 的长 Markdown 消息".to_string(),
            "EOF".to_string(),
        ]
        .join("\n"),
    }
}

pub fn version_text() -> String {
    format!("AskHuman v{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_basename_from_path() {
        assert_eq!(program_name_from(Some("/usr/local/bin/AskHuman")), "AskHuman");
        assert_eq!(program_name_from(Some("./AskHuman")), "AskHuman");
    }

    #[test]
    fn keeps_name_without_separator() {
        assert_eq!(program_name_from(Some("AskHuman")), "AskHuman");
        assert_eq!(program_name_from(Some("wblra ask")), "wblra ask");
    }

    #[test]
    fn falls_back_when_missing_or_empty() {
        assert_eq!(program_name_from(None), "AskHuman");
        assert_eq!(program_name_from(Some("")), "AskHuman");
        assert_eq!(program_name_from(Some("   ")), "AskHuman");
    }

    #[test]
    fn help_localized_by_lang() {
        let en = help_text(Lang::En);
        assert!(en.contains("<message>"));
        assert!(en.contains("Human-in-the-loop interaction tool"));
        let zh = help_text(Lang::Zh);
        assert!(zh.contains("用法:"));
    }

    #[test]
    fn agent_help_markers_match_output() {
        // agent-help 文档里的标记必须与 output.rs 实际输出一致。
        let en = agent_help_text(Lang::En);
        assert!(en.contains("[Selected options]"));
        assert!(en.contains("[Status]"));
        let zh = agent_help_text(Lang::Zh);
        assert!(zh.contains("[选择的选项]"));
        assert!(zh.contains("[状态]"));
    }
}
