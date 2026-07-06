//! 通用「单选卡」的 Telegram 渲染（HTML 正文 + inline keyboard）。
//!
//! 用户定案布局：正文逐个列出 agent（两行：圆点+`[编号]` 类型·目录 / 标题），正文下方每个 agent 一枚
//! 按钮「<动作> [编号]」，点击即触发（`callback_data=sel:<idx>`，idx=选项下标；daemon 侧按下标映射回
//! session_id，避开 64 字节 callback_data 上限与 seq 漂移）。
//!
//! 点选后（daemon 侧）：`/watch` 就地把本消息编辑成实时 watch 卡（editMessageText，与飞书就地变身一致）；
//! `/status` 回文本详情、卡不动；`/unwatch` 旧卡定格 + 就地刷新本卡（移除该项 / 取 0 定格）。

use crate::i18n::Lang;
use crate::select::{SelectDot, SelectView};
use serde_json::{json, Value};

/// 按钮回调 data 前缀（`sel:<idx>`）。
pub const CB_PREFIX: &str = "sel:";

/// 状态圆点 emoji（无彩色字体：工作中🟢 / 空闲⚪，与 watch 卡同风格）。
fn dot_emoji(dot: Option<SelectDot>) -> &'static str {
    match dot {
        Some(SelectDot::Working) => "🟢",
        Some(SelectDot::Idle) => "⚪",
        None => "▫️",
    }
}

/// 渲染正文 HTML（`parse_mode=HTML`）：标题 + 每个选项两行。
pub fn render_select_html(view: &SelectView) -> String {
    use super::markdown::escape_html as esc;
    let mut out = format!("<b>{}</b>", esc(&view.title));
    if let Some(note) = &view.truncated_note {
        out.push_str(&format!(" <i>{}</i>", esc(note)));
    }
    for opt in &view.options {
        out.push_str("\n\n");
        out.push_str(dot_emoji(opt.dot));
        if let Some(seq) = opt.seq {
            out.push_str(&format!(" <b>[{}]</b>", seq));
        }
        out.push_str(&format!(" {}", esc(&opt.primary)));
        if let Some(badge) = &opt.badge {
            out.push_str(&format!(" {}", esc(badge)));
        }
        if let Some(sub) = &opt.secondary {
            out.push_str(&format!("\n<i>{}</i>", esc(sub)));
        }
    }
    out
}

/// inline keyboard：每个选项一枚按钮「<动作> [编号]」，每按钮独占一行，`callback_data=sel:<idx>`。
pub fn inline_keyboard(view: &SelectView, lang: Lang) -> Value {
    let label = view.action.button_label(lang);
    let rows: Vec<Value> = view
        .options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            let text = match opt.seq {
                Some(seq) => format!("{} [{}]", label, seq),
                None => label.clone(),
            };
            json!([{ "text": text, "callback_data": format!("{}{}", CB_PREFIX, idx) }])
        })
        .collect();
    json!({ "inline_keyboard": rows })
}

/// 定格 HTML（编辑时不带 markup 即移除按钮）：标题 + 定格文案。
pub fn render_select_final_html(title: &str, final_label: &str) -> String {
    use super::markdown::escape_html as esc;
    format!("<b>{}</b>\n<i>{}</i>", esc(title), esc(final_label))
}

/// 解析按钮回调 data → 选项下标（非本卡回调返回 None）。
pub fn parse_select_action(data: &str) -> Option<usize> {
    data.strip_prefix(CB_PREFIX).and_then(|s| s.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::{build_view, SelectAction, SelectDot, SelectOption};

    fn opt(id: &str, dot: Option<SelectDot>, seq: u64, primary: &str, badge: Option<&str>, sub: &str) -> SelectOption {
        SelectOption {
            id: id.to_string(),
            dot,
            seq: Some(seq),
            primary: primary.to_string(),
            badge: badge.map(|b| b.to_string()),
            secondary: Some(sub.to_string()),
        }
    }

    fn view() -> SelectView {
        build_view(
            "选择要实时关注的 Agent：".into(),
            vec![
                opt("s-work", Some(SelectDot::Working), 2, "Cursor · <api>", Some("· 关注中"), "重构 & 测试"),
                opt("s-idle", Some(SelectDot::Idle), 5, "Claude Code · web", None, "写文档"),
            ],
            SelectAction::Watch,
            Lang::Zh,
        )
    }

    #[test]
    fn html_two_lines_and_escapes() {
        let html = render_select_html(&view());
        assert!(html.contains("<b>选择要实时关注的 Agent：</b>"));
        // 第一行圆点 + 加粗编号 + 主文本（用户内容转义）+ 徽标；第二行斜体标题。
        assert!(html.contains("🟢 <b>[2]</b> Cursor · &lt;api&gt; · 关注中"));
        assert!(html.contains("<i>重构 &amp; 测试</i>"));
        assert!(html.contains("⚪ <b>[5]</b> Claude Code · web"));
    }

    #[test]
    fn keyboard_one_button_per_option() {
        let kb = inline_keyboard(&view(), Lang::Zh);
        let rows = kb["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0]["text"], "关注 [2]");
        assert_eq!(rows[0][0]["callback_data"], "sel:0");
        assert_eq!(rows[1][0]["callback_data"], "sel:1");
    }

    #[test]
    fn parse_roundtrip() {
        assert_eq!(parse_select_action("sel:3"), Some(3));
        assert_eq!(parse_select_action("watch:refresh"), None);
        assert_eq!(parse_select_action("sel:x"), None);
    }
}
