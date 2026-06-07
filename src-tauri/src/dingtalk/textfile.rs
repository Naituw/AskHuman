//! 钉钉文本类附件的判定与路由：短文本内联、长文本转 docx、其余原样发送。
//!
//! 方案见 `docs/plans/dingtalk-attachment-preview.md`（决策 D2–D16）。

use crate::config::DingTalkChannelConfig;
use crate::dingtalk::docx;

/// 短/长阈值：内容字符数 ≤ 此值视为“短”。
const INLINE_CHAR_THRESHOLD: usize = 3000;

/// 钉钉可原生预览的文件类型（这些原样发送，不做处理）。
const DINGTALK_WHITELIST: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "zip", "rar",
];

/// 视为“文本类”的扩展名（命中且内容为 UTF-8 才进入处理）。
const TEXT_EXTS: &[&str] = &[
    "md", "markdown", "txt", "text", "log", "csv", "tsv", "json", "json5", "yaml", "yml", "toml",
    "ini", "conf", "cfg", "env", "properties", "xml", "html", "htm", "css", "scss", "less", "svg",
    "js", "jsx", "ts", "tsx", "mjs", "cjs", "vue", "svelte", "py", "rb", "go", "rs", "java", "kt",
    "kts", "scala", "c", "h", "cpp", "cc", "cxx", "hpp", "hh", "cs", "m", "mm", "swift", "php",
    "pl", "lua", "dart", "r", "sql", "graphql", "gql", "proto", "sh", "bash", "zsh", "fish", "ps1",
    "bat", "gradle", "dockerfile", "makefile", "mk", "cmake", "diff", "patch",
];

/// 路由决策结果，由调用方执行实际发送。
pub enum TextAction {
    /// 短文本：内联为一条 sampleMarkdown 消息（title, text），不再发文件。
    Inline { title: String, text: String },
    /// 转 docx：上传后以 sampleFile(fileType=docx, fileName) 发送。
    Docx { file_name: String, bytes: Vec<u8> },
    /// 不处理：调用方按原样发送源文件。
    PassThrough,
}

/// 小写扩展名（无则空串）。
fn ext_lower(name: &str) -> String {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_whitelist(ext: &str) -> bool {
    DINGTALK_WHITELIST.contains(&ext)
}

fn is_text(ext: &str) -> bool {
    TEXT_EXTS.contains(&ext)
}

fn is_markdown(ext: &str) -> bool {
    ext == "md" || ext == "markdown"
}

/// 扩展名 → 围栏代码块语言标识（用于内联高亮）。未知则用扩展名原文。
fn lang_of(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "go" => "go",
        "kt" | "kts" => "kotlin",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "m" | "mm" => "objectivec",
        "sh" | "bash" | "zsh" | "fish" => "bash",
        "yml" => "yaml",
        "json5" => "json",
        "conf" | "cfg" => "ini",
        "htm" => "html",
        "svg" => "xml",
        "gql" => "graphql",
        "proto" => "protobuf",
        "mk" | "makefile" => "makefile",
        "ps1" => "powershell",
        "patch" => "diff",
        other => other,
    }
}

fn read_utf8(path: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    // 去掉可能的 UTF-8 BOM。
    let s = String::from_utf8(bytes).ok()?;
    Some(s.strip_prefix('\u{feff}').map(|s| s.to_string()).unwrap_or(s))
}

fn human_size(n: usize) -> String {
    if n < 1024 {
        format!("{} B", n)
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

/// 选一个足够长的围栏（避免内容里本身含 ``` 截断）。
fn fence(content: &str) -> String {
    let mut cur = 0usize;
    let mut max = 0usize;
    for c in content.chars() {
        if c == '`' {
            cur += 1;
            max = max.max(cur);
        } else {
            cur = 0;
        }
    }
    "`".repeat(max.max(2) + 1)
}

/// 组装内联消息正文：加粗 header + 分割线 + 内容（md 原文 / 非 md 代码块）。
fn build_inline_text(name: &str, ext: &str, content: &str, byte_len: usize) -> String {
    let lines = content.lines().count().max(1);
    let header = format!("**{} · {} · {} 行**", name, human_size(byte_len), lines);
    let body = if is_markdown(ext) {
        content.to_string()
    } else {
        let f = fence(content);
        format!("{f}{lang}\n{content}\n{f}", f = f, lang = lang_of(ext), content = content)
    };
    format!("{header}\n\n---\n\n{body}", header = header, body = body)
}

/// 根据配置与文件，决定如何发送。纯函数（仅读文件 + 生成 docx 字节），不做网络。
pub fn plan(cfg: &DingTalkChannelConfig, path: &str, name: &str) -> TextAction {
    let ext = ext_lower(name);

    // 已可预览类型 / 非文本类：原样发送。
    if is_whitelist(&ext) || !is_text(&ext) {
        return TextAction::PassThrough;
    }
    // 命中文本清单但内容非 UTF-8：判为非文本，原样发送。
    let content = match read_utf8(path) {
        Some(c) => c,
        None => return TextAction::PassThrough,
    };
    let byte_len = content.len();
    let char_count = content.chars().count();

    // 短文本 + 开关① → 内联。
    if char_count <= INLINE_CHAR_THRESHOLD && cfg.inline_small_text {
        let text = build_inline_text(name, &ext, &content, byte_len);
        return TextAction::Inline {
            title: name.to_string(),
            text,
        };
    }

    // 长文本（或内联关）→ 开关② ? 转 docx : 原样。
    if cfg.convert_text_to_docx {
        let bytes = if is_markdown(&ext) {
            docx::build_markdown_docx(&content)
        } else {
            docx::build_plaincode_docx(name, &content)
        };
        match bytes {
            Ok(b) => TextAction::Docx {
                file_name: format!("{}.docx", name),
                bytes: b,
            },
            Err(_) => TextAction::PassThrough, // 生成失败：静默退回原样
        }
    } else {
        TextAction::PassThrough
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(inline: bool, convert: bool) -> DingTalkChannelConfig {
        DingTalkChannelConfig {
            inline_small_text: inline,
            convert_text_to_docx: convert,
            ..Default::default()
        }
    }

    #[test]
    fn whitelist_and_nontext_pass_through() {
        let c = cfg(true, true);
        assert!(matches!(plan(&c, "/x/a.pdf", "a.pdf"), TextAction::PassThrough));
        assert!(matches!(plan(&c, "/x/a.png", "a.png"), TextAction::PassThrough));
        assert!(matches!(plan(&c, "/x/a.bin", "a.bin"), TextAction::PassThrough));
    }

    #[test]
    fn lang_mapping() {
        assert_eq!(lang_of("rs"), "rust");
        assert_eq!(lang_of("py"), "python");
        assert_eq!(lang_of("unknownext"), "unknownext");
    }

    #[test]
    fn fence_grows_with_backticks() {
        assert_eq!(fence("no ticks"), "```");
        assert_eq!(fence("a ``` b"), "````");
    }

    #[test]
    fn short_text_inlines_with_header_and_hr() {
        let c = cfg(true, true);
        let dir = std::env::temp_dir();
        let p = dir.join("ha_tf_test.txt");
        std::fs::write(&p, "hello\nworld\n").unwrap();
        let action = plan(&c, p.to_str().unwrap(), "note.txt");
        match action {
            TextAction::Inline { text, .. } => {
                assert!(text.contains("note.txt"));
                assert!(text.contains("---"));
                assert!(text.contains("```")); // 非 md 用代码块
            }
            _ => panic!("expected inline"),
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn long_text_converts_to_docx() {
        let c = cfg(true, true);
        let dir = std::env::temp_dir();
        let p = dir.join("ha_tf_long.md");
        let big = "# t\n\n".to_string() + &"中文行内容 abc\n\n".repeat(800);
        std::fs::write(&p, &big).unwrap();
        let action = plan(&c, p.to_str().unwrap(), "big.md");
        match action {
            TextAction::Docx { file_name, bytes } => {
                assert_eq!(file_name, "big.md.docx");
                assert!(bytes.len() > 300);
            }
            _ => panic!("expected docx"),
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn convert_off_passes_through_long() {
        let c = cfg(true, false);
        let dir = std::env::temp_dir();
        let p = dir.join("ha_tf_off.txt");
        std::fs::write(&p, "x".repeat(5000)).unwrap();
        let action = plan(&c, p.to_str().unwrap(), "big.txt");
        assert!(matches!(action, TextAction::PassThrough));
        let _ = std::fs::remove_file(p);
    }
}
