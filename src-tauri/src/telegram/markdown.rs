//! 把标准 Markdown 处理为 Telegram MarkdownV2（移植自 Swift `TelegramMarkdown`）。
//!
//! 步骤：保护代码块/行内代码 → 标题/粗体转换 → 转义特殊字符 → 还原占位符。

/// 处理入口。
pub fn process(text: &str) -> String {
    let mut protected: Vec<String> = Vec::new();
    let s = protect_code_blocks(text, &mut protected);
    let s = protect_inline_code(&s, &mut protected);
    let s = convert_markdown(&s);
    let mut s = escape_special(&s);
    // 倒序还原，避免 "CODEBLOCK1" 命中 "CODEBLOCK10"。
    for (i, seg) in protected.iter().enumerate().rev() {
        s = s.replace(&format!("CODEBLOCK{}", i), seg);
    }
    s
}

fn protect_code_blocks(text: &str, protected: &mut Vec<String>) -> String {
    let mut text = text.to_string();
    loop {
        let Some(start) = text.find("```") else { break };
        let after = start + 3;
        let Some(rel_end) = text[after..].find("```") else { break };
        let end = after + rel_end + 3;
        let placeholder = format!("CODEBLOCK{}", protected.len());
        protected.push(text[start..end].to_string());
        text.replace_range(start..end, &placeholder);
    }
    text
}

fn protect_inline_code(text: &str, protected: &mut Vec<String>) -> String {
    let mut text = text.to_string();
    let mut search = 0usize;
    loop {
        let Some(open_rel) = text[search..].find('`') else { break };
        let open = search + open_rel;
        let after = open + 1;
        let Some(close_rel) = text[after..].find('`') else { break };
        let end = after + close_rel + 1;
        let placeholder = format!("CODEBLOCK{}", protected.len());
        protected.push(text[open..end].to_string());
        text.replace_range(open..end, &placeholder);
        search = open + placeholder.len();
    }
    text
}

fn convert_markdown(text: &str) -> String {
    let joined: Vec<String> = text.split('\n').map(convert_header).collect();
    convert_bold(&joined.join("\n"))
}

/// `^#{1,6}\s+(.+)$` → `>$1`
fn convert_header(line: &str) -> String {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &line[hashes..];
        let title = rest.trim_start_matches([' ', '\t']);
        // 必须存在分隔空白且标题非空。
        if title.len() < rest.len() && !title.is_empty() {
            return format!(">{}", title);
        }
    }
    line.to_string()
}

/// `\*\*([^*]+)\*\*` → `*$1*`
fn convert_bold(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            let start = i + 2;
            let mut j = start;
            while j < chars.len() && chars[j] != '*' {
                j += 1;
            }
            if j > start && j + 1 < chars.len() && chars[j] == '*' && chars[j + 1] == '*' {
                out.push('*');
                out.extend(&chars[start..j]);
                out.push('*');
                i = j + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 转义 MarkdownV2 特殊字符（不转义 `*`、`>`、`` ` ``）。
fn escape_special(text: &str) -> String {
    const SPECIAL: &[char] = &[
        '_', '[', ']', '(', ')', '~', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if SPECIAL.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_to_single_star() {
        assert_eq!(process("**bold**"), "*bold*");
    }

    #[test]
    fn header_to_quote() {
        assert_eq!(process("# Title"), ">Title");
        assert_eq!(process("### Sub"), ">Sub");
    }

    #[test]
    fn escapes_special_chars() {
        assert_eq!(process("a.b-c!"), "a\\.b\\-c\\!");
        assert_eq!(process("(x)"), "\\(x\\)");
    }

    #[test]
    fn inline_code_not_escaped() {
        assert_eq!(process("`a.b`"), "`a.b`");
    }

    #[test]
    fn code_block_preserved() {
        assert_eq!(process("```\na.b\n```"), "```\na.b\n```");
    }

    #[test]
    fn mixed() {
        // 普通文本中的点被转义，代码内的点不转义。
        assert_eq!(process("see `x.y` end."), "see `x.y` end\\.");
    }

    #[test]
    fn non_header_hash_escaped() {
        // 行内 # 不是标题（无空白分隔）→ 作为普通字符被转义。
        assert_eq!(process("a#b"), "a\\#b");
    }
}
