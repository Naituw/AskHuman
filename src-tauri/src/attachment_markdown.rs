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
/// Longest link destination the popup keeps as the path a click will name.
const MAX_LOCAL_DESTINATION: usize = 4096;

/// A link destination that names a file on this machine: no scheme, a `file:` URL or a drive path.
/// The popup keeps it as inert data (`data-local-link`) for the panel's click handler, never as a URL.
/// Everything with another scheme (`javascript:`, `data:`, `tauri:`, `ftp:` …) stays dropped.
pub fn local_file_destination(url: &str) -> bool {
    if url.is_empty()
        || url.len() > MAX_LOCAL_DESTINATION
        || url.chars().any(|c| c.is_control())
        || url.starts_with("//")
        || url.starts_with('#')
        || url.starts_with('?')
    {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("file:") {
        return true;
    }
    // A drive path is a letter, a colon and a separator, which the author may have percent-encoded.
    let bytes = url.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (matches!(bytes[2], b'/' | b'\\')
            || lower[2..].starts_with("%5c")
            || lower[2..].starts_with("%2f"))
    {
        return true;
    }
    !has_scheme(url)
}

/// `scheme:` in front of anything that could end the first path segment.
fn has_scheme(url: &str) -> bool {
    let mut chars = url.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    for c in chars {
        match c {
            ':' => return true,
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-') => {}
            _ => return false,
        }
    }
    false
}

fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The opening tag of a popup link to a local file: no URL semantics, the authored destination as data.
fn local_link_open(dest: &str, title: &str) -> String {
    let title = if title.is_empty() {
        String::new()
    } else {
        format!(" title=\"{}\"", escape_attr(title))
    };
    format!(
        "<a href=\"\" data-local-link=\"{}\"{title}>",
        escape_attr(dest)
    )
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
            dest_url, title, ..
        }) if popup && !allowed_url(&dest_url, false) && local_file_destination(&dest_url) => {
            Event::Html(local_link_open(&dest_url, &title).into())
        }
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
        // Neither the active link nor the local image keeps anything a WebView could request.
        assert!(!html.contains("href=\"javascript") && !html.contains("src=\"file"));
        assert!(html.contains("<a href=\"\">x</a>"));
        assert!(html.contains("<img src=\"\" alt=\"x\" />"));
    }

    #[test]
    fn popup_keeps_local_file_links_as_inert_data_for_the_click_handler() {
        let html = render(
            "[a](docs/a.md:12) [b](/Users/me/b.md \"Title\") [c](C:\\proj\\c.md) [d](file:///tmp/d.md#L3) [e](<../my notes/e.md>) [f](報告.md)",
            true,
        );
        assert!(html.contains("<a href=\"\" data-local-link=\"docs/a.md:12\">a</a>"));
        assert!(
            html.contains("<a href=\"\" data-local-link=\"/Users/me/b.md\" title=\"Title\">b</a>")
        );
        assert!(html.contains("<a href=\"\" data-local-link=\"C:\\proj\\c.md\">c</a>"));
        assert!(html.contains("<a href=\"\" data-local-link=\"file:///tmp/d.md#L3\">d</a>"));
        assert!(html.contains("<a href=\"\" data-local-link=\"../my notes/e.md\">e</a>"));
        // The destination is data as the author wrote it, not percent-encoded into a URL.
        assert!(html.contains("data-local-link=\"報告.md\""));
        // Nothing carries URL semantics: every anchor has an empty href.
        assert_eq!(html.matches("href=\"\"").count(), 6);
        assert_eq!(html.matches("<a ").count(), 6);
    }

    #[test]
    fn popup_drops_every_other_destination_and_escapes_the_kept_ones() {
        for dest in [
            "javascript:alert(1)",
            "data:text/html,x",
            "tauri://localhost/x",
            "ftp://host/x",
            "vbscript:x",
            "localhost:8080/x",
            "//host/share",
        ] {
            let html = render(&format!("[x](<{dest}>)"), true);
            assert!(html.contains("<a href=\"\">x</a>"), "{dest}: {html}");
            assert!(!html.contains("data-local-link"), "{dest}: {html}");
        }
        let long = format!("[x](<{}>)", "a".repeat(MAX_LOCAL_DESTINATION + 1));
        assert!(!render(&long, true).contains("data-local-link"));
        let at_limit = format!("[x](<{}>)", "a".repeat(MAX_LOCAL_DESTINATION));
        assert!(render(&at_limit, true).contains("data-local-link"));
        // A destination cannot break out of its attribute.
        let html = render("[x](a\"onmouseover=\"alert(1))", true);
        assert!(
            html.contains("data-local-link=\"a&quot;onmouseover=&quot;alert(1)\""),
            "{html}"
        );
        assert!(!html.contains("\" onmouseover") && !html.contains(" onmouseover="));
        let html = render("[x](<a'b&c> \"t<i>\")", true);
        assert!(
            html.contains("data-local-link=\"a&#39;b&amp;c\" title=\"t&lt;i&gt;\""),
            "{html}"
        );
        assert!(!html.contains("<i>"));
        // Images keep the strict policy, and the browser snapshot keeps its links as they were.
        assert!(!render("![x](docs/a.png)", true).contains("docs/a.png"));
        let snapshot = render("[a](docs/a.md)", false);
        assert!(snapshot.contains("href=\"docs/a.md\"") && !snapshot.contains("data-local-link"));
    }

    #[test]
    fn local_file_destinations_are_paths_not_urls() {
        for local in [
            "docs/a.md",
            "./a.md",
            "../a.md:3:4",
            "/Users/me/a.md",
            "C:\\a\\b.md",
            "c:/a/b.md",
            "C:%5Ca.md",
            "\\\\server\\share\\a.md",
            "file:///tmp/a.md",
            "FILE:///tmp/a.md",
            "docs/a:b.md",
            "報告 最終.md",
        ] {
            assert!(local_file_destination(local), "{local}");
        }
        for other in [
            "",
            "#frag",
            "?q=1",
            "//host/x",
            "https://example.com",
            "mailto:a@b.c",
            "javascript:void(0)",
            "a:b.md",
            "x-app://open",
            "docs/a\u{7}.md",
            "docs/a\n.md",
        ] {
            assert!(!local_file_destination(other), "{other:?}");
        }
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
