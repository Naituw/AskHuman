//! 提问参数解析（纯逻辑，可单测）。
//!
//! 模型：第一个位置参数是共享 Message；`-q` 声明实际问题。
//! 完全没有 `-q` 时，第一个参数等价于 `-q`（`AskHuman "X"` ≡ `AskHuman -q "X"`），
//! 被提升为唯一问题，其前置 `-o` 归该问题；`-f` 始终归 Message（位置不限）。

/// 单个预定义选项的原始参数（解析层结构，避免依赖 models）。
#[derive(Debug, Clone, PartialEq)]
pub struct OptArg {
    pub text: String,
    /// 由 `-o!` / `--option!` 声明：该选项是提问方的推荐答案。
    pub recommended: bool,
}

/// 单个问题的原始参数（路径未解析/校验）。
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionArgs {
    pub message: String,
    pub options: Vec<OptArg>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AskArgs {
    /// 共享 Message 文本（无 `-q` 时为空——其内容已提升进 `questions`）。
    pub message_text: String,
    /// `-f`/`--file` 给出的原始路径（按出现顺序，归 Message，未做解析/校验）。
    pub message_files: Vec<String>,
    /// 问题列表（解析+归一化后恒 ≥1）。
    pub questions: Vec<QuestionArgs>,
    /// 是否按 Markdown 渲染（全局，对所有问题生效）。
    pub is_markdown: bool,
}

/// 解析 `AskHuman <Message> [-f <path>] [-q <text> [-o <opt>] ...] [--no-markdown]`。
///
/// 规则：
/// - 第一个位置参数 = Message 文本；无任何 `-q` 时它被提升为唯一问题。
/// - `-o` 归「最近声明的问题」；存在 `-q` 时不能出现在第一个 `-q` 之前。
///   `-o!` / `--option!` 同 `-o`，且把该选项标记为推荐（一题可多个）。
/// - `-f` 始终归 Message（位置不限）。
/// - `--no-markdown` 全局。
/// - `--stdin`：Message 文本取自 `stdin_message`（由调用方从 stdin 读好后注入，
///   保持本函数无 IO 副作用），等价于位置参数 Message，但可出现在任意位置；
///   与位置参数 `<Message>` 互斥。
///
/// `stdin_message`：`Some` 表示命令行含 `--stdin`（值为已读好的 stdin 内容）；
/// `None` 表示未给 `--stdin`。
///
/// 失败时返回按 `lang` 本地化的错误描述。
pub fn parse_ask(
    args: &[String],
    lang: crate::i18n::Lang,
    stdin_message: Option<String>,
) -> Result<AskArgs, String> {
    use crate::i18n::tr;
    let mut message_text = String::new();
    let mut message_files: Vec<String> = Vec::new();
    let mut questions: Vec<QuestionArgs> = Vec::new();
    // 无 `-q` 时，位于第一个参数之后的 `-o` 暂存于此，归一化时挂到被提升的问题。
    let mut lead_options: Vec<OptArg> = Vec::new();
    let mut is_markdown = true;

    let mut seen_positional = false;
    let mut seen_question_flag = false;
    // 预扫描：是否存在任一 `-q`。决定 `-o` 在题前是报错还是归 Message（被提升的问题）。
    let has_q = args.iter().any(|a| a == "-q" || a == "--question");

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-q" | "--question" => {
                if i + 1 >= args.len() {
                    return Err(tr(lang, "cli.optionMissingValue").replace("{opt}", arg));
                }
                questions.push(QuestionArgs {
                    message: args[i + 1].clone(),
                    options: Vec::new(),
                });
                seen_question_flag = true;
                i += 2;
            }
            "-o" | "--option" | "-o!" | "--option!" => {
                if i + 1 >= args.len() {
                    return Err(tr(lang, "cli.optionMissingValue").replace("{opt}", arg));
                }
                let value = OptArg {
                    text: args[i + 1].clone(),
                    recommended: arg.ends_with('!'),
                };
                match questions.last_mut() {
                    Some(q) => q.options.push(value),
                    None => {
                        if has_q {
                            // 存在 -q 时，-o 不能出现在第一个 -q 之前。
                            return Err(tr(lang, "cli.optionBeforeQuestion").replace("{opt}", arg));
                        }
                        // 无 `-q`：暂存，归一化时归属被提升的问题。
                        lead_options.push(value);
                    }
                }
                i += 2;
            }
            "-f" | "--file" => {
                if i + 1 >= args.len() {
                    return Err(tr(lang, "cli.optionMissingValue").replace("{opt}", arg));
                }
                message_files.push(args[i + 1].clone());
                i += 2;
            }
            "--no-markdown" => {
                is_markdown = false;
                i += 1;
            }
            "--stdin" => {
                // 等价于位置参数 Message，但可出现在任意位置；与位置参数互斥。
                if seen_positional {
                    return Err(tr(lang, "cli.stdinWithPositional").to_string());
                }
                // 内容由调用方（IO 层）读好注入；缺省兜底为空串。
                message_text = stdin_message.clone().unwrap_or_default();
                seen_positional = true;
                i += 1;
            }
            a if a.starts_with('-') => {
                return Err(tr(lang, "cli.unknownOptionColon").replace("{opt}", a));
            }
            _ => {
                // 位置参数：仅允许作为第一个 token（Message），且需在任何 -q 之前。
                if seen_positional || seen_question_flag {
                    return Err(tr(lang, "cli.positionalOnlyMessage").to_string());
                }
                message_text = arg.clone();
                seen_positional = true;
                i += 1;
            }
        }
    }

    // 有效性校验（归一化前）：至少有 Message 文本 / 一个 -q / 一个 -f；仅 -o 不算有效。
    if message_text.trim().is_empty() && questions.is_empty() && message_files.is_empty() {
        return Err(tr(lang, "cli.missingContent").to_string());
    }

    // 归一化：无 `-q` 时，第一个参数（含其前置 -o）提升为唯一问题，Message 文本清空。
    if !seen_question_flag {
        questions.push(QuestionArgs {
            message: std::mem::take(&mut message_text),
            options: lead_options,
        });
    }

    Ok(AskArgs {
        message_text,
        message_files,
        questions,
        is_markdown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::i18n::Lang;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// 普通选项（非推荐）。
    fn o(items: &[&str]) -> Vec<OptArg> {
        items
            .iter()
            .map(|s| OptArg { text: s.to_string(), recommended: false })
            .collect()
    }

    // 解析逻辑与语言无关：测试统一用英文（源语言）。
    fn pa(args: &[String]) -> Result<AskArgs, String> {
        parse_ask(args, Lang::En, None)
    }

    // 带 stdin 内容（模拟命令行含 --stdin、内容已由 IO 层读好）。
    fn pas(args: &[String], stdin: &str) -> Result<AskArgs, String> {
        parse_ask(args, Lang::En, Some(stdin.to_string()))
    }

    #[test]
    fn positional_promoted_equals_q() {
        let a = pa(&v(&["X"])).unwrap();
        let b = pa(&v(&["-q", "X"])).unwrap();
        assert_eq!(a.message_text, "");
        assert_eq!(a.questions, vec![QuestionArgs { message: "X".into(), options: vec![] }]);
        assert_eq!(a.questions, b.questions);
        assert_eq!(b.message_text, "");
        assert!(a.is_markdown);
    }

    #[test]
    fn single_with_options_promoted() {
        let p = pa(&v(&["X", "-o", "A", "--option", "B"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.questions, vec![QuestionArgs { message: "X".into(), options: o(&["A", "B"]) }]);
    }

    #[test]
    fn recommended_option_promoted() {
        // -o! 标记推荐，归属规则与 -o 一致（无 -q 时归被提升的问题）。
        let p = pa(&v(&["X", "-o!", "A", "-o", "B"])).unwrap();
        assert_eq!(
            p.questions[0].options,
            vec![
                OptArg { text: "A".into(), recommended: true },
                OptArg { text: "B".into(), recommended: false },
            ]
        );
    }

    #[test]
    fn recommended_long_form_and_multiple() {
        // --option! 与 -o! 等价；一题允许多个推荐。
        let p = pa(&v(&["M", "-q", "Q1", "--option!", "A", "-o!", "B", "-o", "C"])).unwrap();
        assert_eq!(
            p.questions[0].options,
            vec![
                OptArg { text: "A".into(), recommended: true },
                OptArg { text: "B".into(), recommended: true },
                OptArg { text: "C".into(), recommended: false },
            ]
        );
    }

    #[test]
    fn recommended_per_question_attribution() {
        let p = pa(&v(&["M", "-q", "Q1", "-o", "A", "-q", "Q2", "-o!", "B"])).unwrap();
        assert!(!p.questions[0].options[0].recommended);
        assert!(p.questions[1].options[0].recommended);
    }

    #[test]
    fn recommended_before_any_question_errors() {
        // 与 -o 一致：有 -q 时不得出现在第一个 -q 之前。
        assert!(pa(&v(&["M", "-o!", "A", "-q", "Q1"])).is_err());
    }

    #[test]
    fn recommended_requires_value() {
        assert!(pa(&v(&["msg", "-o!"])).is_err());
    }

    #[test]
    fn single_with_files_promoted() {
        let p = pa(&v(&["X", "-f", "f.png"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.message_files, v(&["f.png"]));
        assert_eq!(p.questions, vec![QuestionArgs { message: "X".into(), options: vec![] }]);
    }

    #[test]
    fn multi_message_and_questions() {
        let p = pa(&v(&[
            "M", "-q", "Q1", "-o", "A", "-q", "Q2", "-o", "B",
        ]))
        .unwrap();
        assert_eq!(p.message_text, "M");
        assert_eq!(p.questions.len(), 2);
        assert_eq!(p.questions[0], QuestionArgs { message: "Q1".into(), options: o(&["A"]) });
        assert_eq!(p.questions[1], QuestionArgs { message: "Q2".into(), options: o(&["B"]) });
    }

    #[test]
    fn file_after_q_belongs_to_message() {
        let p = pa(&v(&["M", "-q", "Q1", "-f", "x.png"])).unwrap();
        assert_eq!(p.message_text, "M");
        assert_eq!(p.message_files, v(&["x.png"]));
        assert_eq!(p.questions[0].options.len(), 0);
    }

    #[test]
    fn optional_message_questions_only() {
        let p = pa(&v(&["-q", "Q1", "-q", "Q2"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.questions.len(), 2);
    }

    #[test]
    fn optional_message_file_only() {
        let p = pa(&v(&["-f", "x.png", "-q", "Q1"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.message_files, v(&["x.png"]));
        assert_eq!(p.questions.len(), 1);
    }

    #[test]
    fn no_markdown_is_global() {
        let p = pa(&v(&["M", "-q", "Q1", "--no-markdown"])).unwrap();
        assert!(!p.is_markdown);
        assert_eq!(p.questions.len(), 1);
    }

    #[test]
    fn option_before_any_question_errors() {
        assert!(pa(&v(&["M", "-o", "A", "-q", "Q1"])).is_err());
    }

    #[test]
    fn option_only_is_invalid() {
        assert!(pa(&v(&["-o", "A"])).is_err());
    }

    #[test]
    fn second_positional_errors() {
        assert!(pa(&v(&["M", "extra"])).is_err());
        assert!(pa(&v(&["-q", "Q1", "extra"])).is_err());
    }

    #[test]
    fn question_requires_value() {
        assert!(pa(&v(&["-q"])).is_err());
        assert!(pa(&v(&["M", "-q"])).is_err());
    }

    #[test]
    fn requires_some_content() {
        assert!(pa(&v(&["--no-markdown"])).is_err());
        assert!(pa(&v(&[])).is_err());
    }

    #[test]
    fn requires_option_value() {
        assert!(pa(&v(&["msg", "-o"])).is_err());
    }

    #[test]
    fn requires_file_value() {
        assert!(pa(&v(&["msg", "-f"])).is_err());
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(pa(&v(&["msg", "--foo"])).is_err());
    }

    #[test]
    fn stdin_is_shared_message_with_q() {
        let p = pas(&v(&["--stdin", "-q", "Q1"]), "MSG").unwrap();
        assert_eq!(p.message_text, "MSG");
        assert_eq!(p.questions, vec![QuestionArgs { message: "Q1".into(), options: vec![] }]);
    }

    #[test]
    fn stdin_promoted_without_q() {
        // 无 -q 时 stdin 内容提升为唯一问题，message_text 清空。
        let p = pas(&v(&["--stdin"]), "the only question body").unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(
            p.questions,
            vec![QuestionArgs { message: "the only question body".into(), options: vec![] }]
        );
    }

    #[test]
    fn stdin_position_is_free() {
        // --stdin 可出现在 -q 之后（具名标志，不要求在题前）。
        let p = pas(&v(&["-q", "Q1", "--stdin"]), "MSG").unwrap();
        assert_eq!(p.message_text, "MSG");
        assert_eq!(p.questions.len(), 1);
    }

    #[test]
    fn stdin_conflicts_with_positional() {
        assert!(pas(&v(&["MSG", "--stdin"]), "X").is_err());
        assert!(pas(&v(&["--stdin", "MSG"]), "X").is_err());
    }

    #[test]
    fn stdin_empty_without_q_is_invalid() {
        assert!(pas(&v(&["--stdin"]), "   ").is_err());
        assert!(pas(&v(&["--stdin"]), "").is_err());
    }

    #[test]
    fn stdin_empty_with_q_is_ok() {
        let p = pas(&v(&["--stdin", "-q", "Q1"]), "").unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.questions.len(), 1);
    }
}
