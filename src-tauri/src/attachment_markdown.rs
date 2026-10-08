//! Shared static Markdown renderer. Popup policy limits generated URLs in its trusted WebView.
use pulldown_cmark::{html, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
pub fn supports(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| {
            ["md", "markdown", "mdown", "mkd", "mdwn"]
                .iter()
                .any(|e| x.eq_ignore_ascii_case(e))
        })
}
pub fn allowed_url(url: &str, image: bool) -> bool {
    if url.chars().any(|c| c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || (!image && (lower.starts_with("mailto:") || lower.starts_with('#')))
}
pub fn render(src: &str, popup: bool) -> String {
    render_bounded(src, popup, usize::MAX).unwrap()
}
pub fn render_popup(src: &str) -> Result<String, &'static str> {
    render_bounded(src, true, 50_000)
}
fn render_bounded(src: &str, popup: bool, max_events: usize) -> Result<String, &'static str> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(src, opts).map(|ev| match ev {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) if popup && !allowed_url(&dest_url, false) => Event::Start(Tag::Link {
            link_type,
            dest_url: "".into(),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) if popup && !allowed_url(&dest_url, true) => Event::Start(Tag::Image {
            link_type,
            dest_url: "".into(),
            title,
            id,
        }),
        other => other,
    });
    let mut mermaid = false;
    let events: Vec<_> = parser
        .flat_map(|event| match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ref info)))
                if info
                    .split_whitespace()
                    .next()
                    .is_some_and(|language| language.eq_ignore_ascii_case("mermaid")) =>
            {
                mermaid = true;
                vec![
                    Event::Html("<div class=\"mermaid-block\" data-mermaid-pending>".into()),
                    event,
                ]
            }
            Event::End(TagEnd::CodeBlock) if std::mem::take(&mut mermaid) => {
                vec![event, Event::Html("</div>".into())]
            }
            other => vec![other],
        })
        .take(max_events.saturating_add(1))
        .collect();
    if events.len() > max_events {
        return Err("limit");
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    if popup && out.len() > 8 * 1024 * 1024 {
        return Err("limit");
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_escapes_html_and_rejects_active_and_local_urls() {
        let html = render("# Title\n\n<script>x</script>\n\n[x](javascript:alert%281%29) ![x](file:///secret) [safe](https://example.com)\n\n- [x] task", true);
        assert!(html.contains("<h1>Title</h1>"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("file:///"));
        assert!(html.contains("https://example.com"));
        assert!(html.contains("type=\"checkbox\""));
    }
    #[test]
    fn marks_only_explicit_mermaid_fences_and_preserves_escaped_source() {
        let html = render("``` Mermaid extra\nflowchart TD\nA[<script>]-->B\n```\n\n```less\nflowchart TD\nA-->B\n```\n\n    flowchart TD", true);
        assert_eq!(html.matches("data-mermaid-pending").count(), 1);
        assert!(html.contains("A[&lt;script&gt;]--&gt;B"));
        assert!(html.contains("class=\"language-less\""));
        assert!(!render("```not-mermaid mermaid\nx\n```", true).contains("data-mermaid-pending"));
        assert!(render_bounded("```mermaid\nA\n```", true, 3).is_err());
    }
}
