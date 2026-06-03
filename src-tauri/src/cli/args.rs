//! 提问参数解析（纯逻辑，可单测）。

#[derive(Debug, Clone, PartialEq)]
pub struct AskArgs {
    pub message: String,
    pub options: Vec<String>,
    pub is_markdown: bool,
    /// `-f`/`--file` 给出的原始路径（按出现顺序，未做解析/校验）。
    pub files: Vec<String>,
}

/// 解析 `AskHuman <message> [-o <opt> ...] [-f <path> ...] [--no-markdown]`。
/// 失败时返回中文错误描述。
pub fn parse_ask(args: &[String]) -> Result<AskArgs, String> {
    let mut message: Option<String> = None;
    let mut options: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut is_markdown = true;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-o" | "--option" => {
                if i + 1 >= args.len() {
                    return Err(format!("{} 选项缺少参数值", arg));
                }
                options.push(args[i + 1].clone());
                i += 2;
            }
            "-f" | "--file" => {
                if i + 1 >= args.len() {
                    return Err(format!("{} 选项缺少参数值", arg));
                }
                files.push(args[i + 1].clone());
                i += 2;
            }
            "--no-markdown" => {
                is_markdown = false;
                i += 1;
            }
            a if a.starts_with('-') => {
                return Err(format!("未知选项: {}", a));
            }
            _ => {
                if message.is_some() {
                    return Err("仅允许一个提问内容参数".to_string());
                }
                message = Some(arg.clone());
                i += 1;
            }
        }
    }

    let message = message.ok_or_else(|| "缺少提问内容".to_string())?;

    Ok(AskArgs {
        message,
        options,
        is_markdown,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn message_only() {
        let p = parse_ask(&v(&["hello"])).unwrap();
        assert_eq!(p.message, "hello");
        assert!(p.options.is_empty());
        assert!(p.is_markdown);
    }

    #[test]
    fn multiple_options() {
        let p = parse_ask(&v(&["msg", "-o", "A", "--option", "B"])).unwrap();
        assert_eq!(p.message, "msg");
        assert_eq!(p.options, v(&["A", "B"]));
    }

    #[test]
    fn multiple_files() {
        let p = parse_ask(&v(&["msg", "-f", "a.md", "--file", "b.png"])).unwrap();
        assert_eq!(p.message, "msg");
        assert_eq!(p.files, v(&["a.md", "b.png"]));
    }

    #[test]
    fn mixed_options_and_files() {
        let p = parse_ask(&v(&["msg", "-o", "A", "-f", "a.md", "--no-markdown"])).unwrap();
        assert_eq!(p.message, "msg");
        assert_eq!(p.options, v(&["A"]));
        assert_eq!(p.files, v(&["a.md"]));
        assert!(!p.is_markdown);
    }

    #[test]
    fn requires_file_value() {
        assert!(parse_ask(&v(&["msg", "-f"])).is_err());
    }

    #[test]
    fn no_markdown_flag() {
        let p = parse_ask(&v(&["msg", "--no-markdown"])).unwrap();
        assert!(!p.is_markdown);
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(parse_ask(&v(&["--foo"])).is_err());
    }

    #[test]
    fn rejects_multiple_messages() {
        assert!(parse_ask(&v(&["a", "b"])).is_err());
    }

    #[test]
    fn requires_option_value() {
        assert!(parse_ask(&v(&["msg", "-o"])).is_err());
    }

    #[test]
    fn requires_message() {
        assert!(parse_ask(&v(&["-o", "A"])).is_err());
    }
}
