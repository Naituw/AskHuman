//! Self-contained HTML for unstaged git diffs.

use crate::gitutil::{DiffLine, DiffModel, FileChangeKind, LineKind};

pub fn render(model: &DiffModel, meta_title: &str) -> String {
    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>\n", esc(meta_title)));
    body.push_str(&format!(
        "<p class=\"meta\">{} · {} file(s){}</p>\n",
        esc(&model.git_root.display().to_string()),
        model.total_paths,
        if model.truncated {
            " · <strong>truncated</strong>"
        } else {
            ""
        }
    ));

    if !model.files.is_empty() {
        body.push_str("<nav class=\"toc\"><ul>\n");
        for f in &model.files {
            let id = anchor(&f.path);
            body.push_str(&format!(
                "<li><a href=\"#{}\">{}</a> <span class=\"kind\">{:?}</span></li>\n",
                id,
                esc(&f.path),
                f.kind
            ));
        }
        body.push_str("</ul></nav>\n");
    }

    for f in &model.files {
        let id = anchor(&f.path);
        body.push_str(&format!("<section id=\"{}\" class=\"file\">\n", id));
        body.push_str(&format!(
            "<h2>{} <span class=\"kind\">{:?}</span></h2>\n",
            esc(&f.path),
            f.kind
        ));
        if f.skipped {
            let reason = f.skip_reason.as_deref().unwrap_or("skipped");
            body.push_str(&format!("<p class=\"skip\">({})</p>\n", esc(reason)));
            if f.lines.is_empty() {
                body.push_str("</section>\n");
                continue;
            }
        }
        body.push_str("<pre class=\"diff\">");
        for line in &f.lines {
            body.push_str(&format_line(line));
        }
        body.push_str("</pre>\n</section>\n");
    }

    wrap_html(meta_title, &body)
}

fn format_line(line: &DiffLine) -> String {
    let (cls, prefix) = match line.kind {
        LineKind::Insert => ("ins", "+"),
        LineKind::Delete => ("del", "-"),
        LineKind::Equal => ("eq", " "),
        LineKind::Header => ("hdr", ""),
    };
    if line.kind == LineKind::Header {
        format!("<span class=\"hdr\">{}</span>\n", esc(&line.text))
    } else {
        format!(
            "<span class=\"{}\">{}{}</span>\n",
            cls,
            prefix,
            esc(&line.text)
        )
    }
}

fn anchor(path: &str) -> String {
    let mut s = String::from("f-");
    for c in path.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

fn esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            _ => c.to_string(),
        })
        .collect()
}

fn wrap_html(title: &str, body: &str) -> String {
    // Explicit fg/bg on every block so Telegram dark mode cannot leave light
    // text on a white code pane (prefers-color-scheme alone is unreliable there).
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<meta name="color-scheme" content="light dark"/>
<title>{title}</title>
<style>
:root {{ color-scheme: light dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  margin: 0 auto; max-width: 960px; padding: 1.25rem; line-height: 1.45;
  background: #0d1117; color: #e6edf3; }}
h1 {{ font-size: 1.35rem; margin-bottom: 0.25rem; color: #e6edf3; }}
h2 {{ font-size: 1.05rem; margin: 1.5rem 0 0.5rem; border-bottom: 1px solid #30363d;
  padding-bottom: 0.25rem; color: #e6edf3; }}
.meta {{ color: #8b949e; font-size: 0.9rem; }}
.toc ul {{ padding-left: 1.2rem; }}
.toc a {{ color: #58a6ff; }}
.kind {{ font-size: 0.75rem; color: #8b949e; font-weight: normal; }}
.skip {{ color: #d29922; font-style: italic; }}
.diff {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px; background: #161b22; color: #e6edf3; border: 1px solid #30363d;
  border-radius: 6px; padding: 0.5rem; overflow-x: auto; white-space: pre; line-height: 1.35; }}
.ins {{ background: #033a16; color: #e6edf3; display: block; }}
.del {{ background: #67060c; color: #e6edf3; display: block; }}
.eq {{ color: #e6edf3; display: block; }}
.hdr {{ display: block; color: #8b949e; }}
@media (prefers-color-scheme: light) {{
  body {{ background: #f6f8fa; color: #1f2328; }}
  h1, h2 {{ color: #1f2328; }}
  h2 {{ border-color: #d0d7de; }}
  .meta, .kind, .hdr {{ color: #656d76; }}
  .skip {{ color: #9a6700; }}
  .toc a {{ color: #0969da; }}
  .diff {{ background: #ffffff; color: #1f2328; border-color: #d0d7de; }}
  .ins {{ background: #e6ffec; color: #1f2328; }}
  .del {{ background: #ffebe9; color: #1f2328; }}
  .eq {{ color: #1f2328; }}
}}
</style>
</head>
<body>
{body}
</body>
</html>
"#,
        title = esc(title),
        body = body
    )
}

// silence unused import warning if FileChangeKind only used in Debug
#[allow(dead_code)]
fn _kind_label(k: FileChangeKind) -> &'static str {
    match k {
        FileChangeKind::Modified => "modified",
        FileChangeKind::Deleted => "deleted",
        FileChangeKind::Untracked => "untracked",
        FileChangeKind::Binary => "binary",
    }
}
