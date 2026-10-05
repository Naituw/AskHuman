//! Standalone styled HTML shared by attachment snapshots and Quick Look.
use std::path::{Path, PathBuf};

pub fn write(path: &str, doc: &str) -> Result<PathBuf, String> {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("preview");
    let dir = std::env::temp_dir()
        .join("askhuman")
        .join("preview")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{}.html", stem));
    std::fs::write(&out, doc.as_bytes()).map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn document(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title><style>{css}</style></head>\
<body><article class=\"markdown-body\">{body}</article></body></html>",
        title = escape_html_min(title),
        css = PREVIEW_CSS,
        body = body,
    )
}

// Escape the title independently of the rendered body.
fn escape_html_min(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// Reading styles follow the system appearance.
const PREVIEW_CSS: &str = r#"
:root { color-scheme: light dark; }
body {
  margin: 0;
  background: #ffffff;
  color: #1f2328;
  font: 15px/1.65 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Arial, "PingFang SC", "Hiragino Sans GB", sans-serif;
}
.markdown-body { max-width: 820px; margin: 0 auto; padding: 28px 32px 48px; word-wrap: break-word; }
.markdown-body h1, .markdown-body h2 { border-bottom: 1px solid #d8dee4; padding-bottom: .3em; }
.markdown-body h1 { font-size: 1.9em; } .markdown-body h2 { font-size: 1.5em; }
.markdown-body h3 { font-size: 1.25em; } .markdown-body h4 { font-size: 1em; }
.markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4, .markdown-body h5, .markdown-body h6 {
  margin: 1.4em 0 .6em; font-weight: 600; line-height: 1.3;
}
.markdown-body p, .markdown-body ul, .markdown-body ol, .markdown-body blockquote, .markdown-body table, .markdown-body pre { margin: 0 0 1em; }
.markdown-body a { color: #0969da; text-decoration: none; }
.markdown-body a:hover { text-decoration: underline; }
.markdown-body code {
  font: .88em/1.5 ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  background: rgba(129,139,152,.18); padding: .2em .4em; border-radius: 6px;
}
.markdown-body pre {
  background: #f6f8fa; padding: 14px 16px; border-radius: 8px; overflow: auto;
}
.markdown-body pre code { background: none; padding: 0; }
.markdown-body blockquote { color: #59636e; border-left: .25em solid #d0d7de; padding: 0 1em; }
.markdown-body table { border-collapse: collapse; display: block; overflow: auto; }
.markdown-body th, .markdown-body td { border: 1px solid #d0d7de; padding: 6px 13px; }
.markdown-body tr:nth-child(2n) { background: #f6f8fa; }
.markdown-body img { max-width: 100%; }
.markdown-body hr { border: 0; border-top: 1px solid #d8dee4; margin: 1.6em 0; }
.markdown-body ul.contains-task-list { list-style: none; padding-left: 1.2em; }
.markdown-body li input[type=checkbox] { margin: 0 .4em 0 -1.2em; }
@media (prefers-color-scheme: dark) {
  body { background: #1e1e1e; color: #e6edf3; }
  .markdown-body h1, .markdown-body h2 { border-bottom-color: #3d444d; }
  .markdown-body a { color: #4493f8; }
  .markdown-body code { background: rgba(101,108,118,.32); }
  .markdown-body pre { background: #161b22; }
  .markdown-body blockquote { color: #9198a1; border-left-color: #3d444d; }
  .markdown-body th, .markdown-body td { border-color: #3d444d; }
  .markdown-body tr:nth-child(2n) { background: #161b22; }
  .markdown-body hr { border-top-color: #3d444d; }
}
"#;
