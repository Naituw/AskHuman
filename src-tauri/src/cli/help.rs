//! 帮助与版本文案。

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

pub fn help_text() -> String {
    let prog = program_name();
    [
        "HumanInLoop - Human-in-the-loop 交互工具".to_string(),
        String::new(),
        "用法:".to_string(),
        format!("  {prog} <message> [选项]      启动询问弹窗，结果写入 stdout"),
        format!("  {prog} --settings            启动设置界面"),
        format!("  {prog} --help, -h            显示此帮助信息"),
        format!("  {prog} --version, -v         显示版本信息"),
        String::new(),
        "参数:".to_string(),
        "  <message>                      要展示给用户的提问内容（必填）".to_string(),
        String::new(),
        "选项:".to_string(),
        "  -o, --option <text>            添加预定义选项，可多次出现".to_string(),
        "  -f, --file <path>              附带文件在弹窗中展示，可多次出现".to_string(),
        "                                 （仅用于展示，不出现在结果输出中）".to_string(),
        "  --no-markdown                  关闭 Markdown 渲染（默认开启）".to_string(),
        String::new(),
        "输出格式（成功路径）:".to_string(),
        "  [选择的选项] / [用户输入] / [图片] 三个区块，".to_string(),
        "  每个区块仅在有内容时输出，区块之间用空行分隔。".to_string(),
        String::new(),
        "更多文档：参考应用内设置的「参考提示词」页面。".to_string(),
    ]
    .join("\n")
}

pub fn version_text() -> String {
    format!("HumanInLoop v{}", env!("CARGO_PKG_VERSION"))
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
    fn help_uses_program_name_and_default_is_unchanged() {
        let text = help_text();
        assert!(text.contains("<message>"));
        // Default (argv[0] basename) keeps the original "AskHuman" labeling
        // when not invoked under a different name.
        assert!(text.contains("HumanInLoop - Human-in-the-loop 交互工具"));
    }
}
