//! Bounded, read-only unified diff previews, independent of the working tree.

use std::fmt::Write as _;
use std::io::Read;
use std::path::Path;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_LINES: usize = 20_000;
const MAX_LINE_BYTES: usize = 16_384;

pub fn supports(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("diff") || ext.eq_ignore_ascii_case("patch"))
}

pub fn render_file(path: &str) -> String {
    let title = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("diff");
    let read = || -> std::io::Result<Vec<u8>> {
        if !std::fs::metadata(path)?.is_file() {
            return Err(std::io::Error::other("not a regular file"));
        }
        let file = std::fs::File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("not a regular file"));
        }
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    match read() {
        Ok(bytes) if bytes.len() > MAX_BYTES => document(title, "", Some("preview.diffLimit")),
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => render(&text, title),
            _ => document(title, "", Some("preview.diffEncoding")),
        },
        Err(_) => document(title, "", Some("preview.diffReadFailed")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Add,
    Delete,
    Context,
    Meta,
}

#[derive(Debug, PartialEq, Eq)]
struct Row<'a> {
    text: &'a str,
    kind: Kind,
    old: Option<u64>,
    new: Option<u64>,
}

// Ranges must fit before any numbers are displayed. Empty ranges legitimately start at zero.
fn range(text: &str, prefix: char) -> Option<(u64, u64)> {
    let text = text.strip_prefix(prefix)?;
    let (start, count) = text.split_once(',').unwrap_or((text, "1"));
    if start.is_empty()
        || count.is_empty()
        || !start.bytes().all(|b| b.is_ascii_digit())
        || !count.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let start = start.parse::<u64>().ok()?;
    let count = count.parse::<u64>().ok()?;
    if (count > 0 && start == 0) || count > MAX_LINES as u64 {
        return None;
    }
    start.checked_add(count)?;
    Some((start, count))
}

fn hunk_header(text: &str) -> Option<((u64, u64), (u64, u64))> {
    let text = text.strip_prefix("@@ ")?;
    let (ranges, suffix) = text.split_once(" @@")?;
    if !suffix.is_empty() && !suffix.starts_with(' ') {
        return None;
    }
    let (old, new) = ranges.split_once(' ')?;
    Some((range(old, '-')?, range(new, '+')?))
}

// Validate the whole hunk first. Malformed/truncated hunks remain verbatim, without invented numbers.
fn hunk<'a>(lines: &[&'a str], start: usize) -> Option<(Vec<Row<'a>>, usize)> {
    let ((mut old, mut old_left), (mut new, mut new_left)) = hunk_header(lines[start])?;
    let mut rows = Vec::new();
    let mut i = start + 1;
    let mut can_mark_no_newline = false;
    while i < lines.len() {
        let text = lines[i];
        if text == "\\ No newline at end of file" && can_mark_no_newline {
            rows.push(Row {
                text,
                kind: Kind::Meta,
                old: None,
                new: None,
            });
            can_mark_no_newline = false;
            i += 1;
            continue;
        }
        if old_left == 0 && new_left == 0 {
            break;
        }
        let (kind, old_no, new_no) = match text.as_bytes().first()? {
            b'+' if new_left > 0 => {
                let number = new;
                new += 1;
                new_left -= 1;
                (Kind::Add, None, Some(number))
            }
            b'-' if old_left > 0 => {
                let number = old;
                old += 1;
                old_left -= 1;
                (Kind::Delete, Some(number), None)
            }
            b' ' if old_left > 0 && new_left > 0 => {
                let numbers = (old, new);
                old += 1;
                new += 1;
                old_left -= 1;
                new_left -= 1;
                (Kind::Context, Some(numbers.0), Some(numbers.1))
            }
            _ => return None,
        };
        rows.push(Row {
            text,
            kind,
            old: old_no,
            new: new_no,
        });
        can_mark_no_newline = true;
        i += 1;
    }
    (old_left == 0 && new_left == 0).then_some((rows, i))
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct DiffRow {
    pub text: String,
    pub kind: &'static str,
    #[serde(serialize_with = "serialize_line_number")]
    pub old: Option<u64>,
    #[serde(serialize_with = "serialize_line_number")]
    pub new: Option<u64>,
}
fn serialize_line_number<S: serde::Serializer>(
    value: &Option<u64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serde::Serialize::serialize(&value.map(|n| n.to_string()), serializer)
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct DiffSection {
    pub title: Option<String>,
    pub rows: Vec<DiffRow>,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct ParsedDiff {
    pub sections: Vec<DiffSection>,
    pub notice: Option<&'static str>,
}
pub fn parse(src: &str) -> Result<ParsedDiff, &'static str> {
    if src.len() > MAX_BYTES {
        return Err("preview.diffLimit");
    }
    let lines: Vec<_> = src
        .strip_prefix('\u{feff}')
        .unwrap_or(src)
        .lines()
        .take(MAX_LINES + 1)
        .collect();
    if lines.len() > MAX_LINES || lines.iter().any(|line| line.len() > MAX_LINE_BYTES) {
        return Err("preview.diffLimit");
    }
    let mut sections = vec![DiffSection {
        title: None,
        rows: Vec::new(),
    }];
    let (mut i, mut in_file, mut git_file, mut binary, mut supported, mut unparsed) =
        (0, false, false, false, false, false);
    while i < lines.len() {
        let text = lines[i];
        let git_header = text.starts_with("diff --git ")
            || text.starts_with("diff --cc ")
            || text.starts_with("diff --combined ");
        let file_pair = text.starts_with("--- ")
            && lines
                .get(i + 1)
                .is_some_and(|line| line.starts_with("+++ "));
        if git_header || (file_pair && !git_file) {
            sections.push(DiffSection {
                title: Some(if git_header { text } else { &lines[i + 1][4..] }.to_owned()),
                rows: Vec::new(),
            });
            git_file = git_header;
            in_file = file_pair || text.starts_with("diff --git ");
            binary = false;
        }
        if file_pair {
            git_file = false;
        }
        if text == "GIT binary patch" || text.starts_with("Binary files ") {
            binary = true;
        }
        let rows = &mut sections.last_mut().unwrap().rows;
        if text.starts_with("@@") {
            if in_file && !binary {
                if let Some((parsed, end)) = hunk(&lines, i) {
                    rows.push(DiffRow {
                        text: text.to_owned(),
                        kind: "hunk",
                        old: None,
                        new: None,
                    });
                    rows.extend(parsed.into_iter().map(|row| DiffRow {
                        text: row.text.to_owned(),
                        kind: match row.kind {
                            Kind::Add => "add",
                            Kind::Delete => "delete",
                            Kind::Context => "context",
                            Kind::Meta => "meta",
                        },
                        old: row.old,
                        new: row.new,
                    }));
                    supported = true;
                    i = end;
                    continue;
                }
            }
            unparsed = true;
        }
        rows.push(DiffRow {
            text: text.to_owned(),
            kind: "meta",
            old: None,
            new: None,
        });
        i += 1;
    }
    Ok(ParsedDiff {
        sections,
        notice: (unparsed || !supported).then_some("preview.diffPlain"),
    })
}
pub fn render(src: &str, title: &str) -> String {
    let parsed = match parse(src) {
        Ok(parsed) => parsed,
        Err(note) => return document(title, "", Some(note)),
    };
    let mut body = String::new();
    for section in parsed.sections {
        if let Some(title) = section.title {
            body.push_str("<section class=\"file\"><h2>");
            escape_into(&mut body, &title);
            body.push_str("</h2>");
        } else {
            body.push_str("<section>");
        }
        body.push_str("<div class=\"diff-scroll\"><div class=\"diff\">");
        for row in section.rows {
            render_row(&mut body, &row.text, row.kind, row.old, row.new);
        }
        body.push_str("</div></div></section>");
    }
    document(title, &body, parsed.notice)
}

fn render_row(out: &mut String, text: &str, class: &str, old: Option<u64>, new: Option<u64>) {
    let _ = write!(out, "<div class=\"line {class}\"><span class=\"number\">");
    if let Some(n) = old {
        let _ = write!(out, "{n}");
    }
    out.push_str("</span><span class=\"number\">");
    if let Some(n) = new {
        let _ = write!(out, "{n}");
    }
    out.push_str("</span><code>");
    escape_into(out, text);
    out.push_str("</code></div>");
}

fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

fn document(title: &str, body: &str, note: Option<&'static str>) -> String {
    let mut out = String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>");
    escape_into(&mut out, title);
    out.push_str("</title><style>");
    out.push_str(include_str!("attachment_diff.css"));
    out.push_str("</style></head><body><h1>");
    escape_into(&mut out, title);
    out.push_str("</h1>");
    if let Some(key) = note {
        out.push_str("<p class=\"notice\">");
        escape_into(&mut out, crate::i18n::tr(crate::i18n::Lang::current(), key));
        out.push_str("</p>");
    }
    out.push_str(body);
    out.push_str("</body></html>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/attachment-preview.patch");

    #[test]
    fn popup_line_numbers_keep_precision_beyond_javascript_integer_range() {
        let row = DiffRow {
            text: " context".into(),
            kind: "context",
            old: Some(9_007_199_254_740_993),
            new: None,
        };
        let json = serde_json::to_value(row).unwrap();
        assert_eq!(json["old"], "9007199254740993");
        assert!(json["new"].is_null());
    }

    #[test]
    fn only_explicit_diff_extensions_are_supported() {
        for name in ["a.diff", "a.PATCH", "/path with space/change.DiFf"] {
            assert!(supports(name));
        }
        for name in ["a.diff.txt", "patch", "a.md", "/folder.diff/a.txt"] {
            assert!(!supports(name));
        }
    }

    #[test]
    fn hunk_numbers_follow_each_side_and_reset_at_new_hunks() {
        let lines = [
            "@@ -8,3 +12,4 @@ fn main",
            " context",
            "-old",
            "+new",
            "+extra",
            " end",
        ];
        let (rows, end) = hunk(&lines, 0).unwrap();
        assert_eq!(end, lines.len());
        assert_eq!(
            rows.iter().map(|r| (r.old, r.new)).collect::<Vec<_>>(),
            vec![
                (Some(8), Some(12)),
                (Some(9), None),
                (None, Some(13)),
                (None, Some(14)),
                (Some(10), Some(15)),
            ]
        );
        let html = render(SAMPLE, "changes.patch");
        assert!(html.contains(
            "class=\"number\">20</span><span class=\"number\"></span><code>--- old punctuation"
        ));
        assert!(html.contains(
            "class=\"number\"></span><span class=\"number\">21</span><code>+++ new punctuation"
        ));
    }

    #[test]
    fn header_like_content_inside_hunks_is_code() {
        let (rows, _) = hunk(&["@@ -1 +1 @@", "--- a/code", "+++ b/code"], 0).unwrap();
        assert_eq!(rows[0].kind, Kind::Delete);
        assert_eq!(rows[1].kind, Kind::Add);
    }

    #[test]
    fn additions_deletions_and_no_newline_markers() {
        let (added, _) = hunk(&["@@ -0,0 +1,2 @@", "+a", "+b"], 0).unwrap();
        assert_eq!(added[1].new, Some(2));
        assert!(added.iter().all(|r| r.old.is_none()));
        let (removed, end) =
            hunk(&["@@ -1 +0,0 @@", "-a", "\\ No newline at end of file"], 0).unwrap();
        assert_eq!(end, 3);
        assert_eq!(removed[1].kind, Kind::Meta);
        let (both, _) = hunk(
            &[
                "@@ -1 +1 @@",
                "-a",
                "\\ No newline at end of file",
                "+b",
                "\\ No newline at end of file",
            ],
            0,
        )
        .unwrap();
        assert_eq!(both[2].new, Some(1));
    }

    #[test]
    fn malformed_ranges_and_incomplete_hunks_have_no_invented_numbers() {
        for header in [
            "@@ -x +1 @@",
            "@@ -1,-1 +1 @@",
            "@@ -0 +1 @@",
            "@@ -18446744073709551615 +1 @@",
            "@@ -1 +1 @@bad",
            "@@@ -1 -1 +1 @@@",
        ] {
            assert!(hunk_header(header).is_none(), "{header}");
        }
        for lines in [
            vec!["@@ -1,2 +1,2 @@", "-a", "+b"],
            vec!["@@ -1 +1 @@", "+b", "+extra"],
            vec!["@@ -1 +1 @@", "invalid"],
        ] {
            assert!(hunk(&lines, 0).is_none());
            let src = format!("--- a/a\n+++ b/a\n{}", lines.join("\n"));
            let html = render(&src, "broken.diff");
            assert!(!html.contains("class=\"line add\""));
            assert!(!html.contains("class=\"line delete\""));
            assert!(html.contains("class=\"notice\""));
            for line in lines {
                assert!(html.contains(line));
            }
        }
    }

    #[test]
    fn plain_unified_files_are_grouped_and_crlf_bom_are_accepted() {
        let html = render("\u{feff}--- a/one\r\n+++ b/one\r\n@@ -1 +1 @@\r\n-a\r\n+b\r\n--- a/two\r\n+++ b/two\r\n@@ -1 +1 @@\r\n-c\r\n+d", "plain.diff");
        assert_eq!(html.matches("<section class=\"file\">").count(), 2);
        assert_eq!(html.matches("class=\"line add\"").count(), 2);
        assert!(!html.contains("class=\"notice\""));
    }

    #[test]
    fn mail_metadata_binary_and_rename_are_preserved() {
        let html = render(SAMPLE, "sample.patch");
        for text in [
            "Subject: [PATCH]",
            "This message is preserved",
            "rename from old-name.txt",
            "Binary files a/icon.png",
            "\\ No newline at end of file",
            "2.50.0",
        ] {
            assert!(html.contains(text), "{text}");
        }
        assert_eq!(html.matches("<section class=\"file\">").count(), 5);
        assert_eq!(html.matches("class=\"line add\"").count(), 6);
        assert_eq!(html.matches("class=\"line delete\"").count(), 3);
        assert!(!html.contains("class=\"notice\""));
        let binary = render(
            "diff --git a/a b/a\nGIT binary patch\nliteral 1\n@@ -1 +1 @@\n-a\n+b",
            "binary.patch",
        );
        assert!(!binary.contains("class=\"line add\""));
        assert!(binary.contains("literal 1"));
    }

    #[test]
    fn combined_and_unknown_formats_remain_plain() {
        let html = render("diff --cc a\n@@@ -1 -1 +1 @@@\n++value", "merge.diff");
        assert!(html.contains("class=\"notice\""));
        assert!(html.contains("++value"));
        assert!(!html.contains("class=\"line add\""));
        let plain = render("hello\n+this is not a diff\n", "plain.patch");
        assert!(plain.contains("+this is not a diff"));
        assert!(!plain.contains("class=\"line add\""));
    }

    #[test]
    fn all_content_and_titles_are_escaped_without_active_resources() {
        let html = render("--- a/<img src=x>\n+++ b/<img src=x>\n@@ -1 +1 @@\n-<script>alert(1)</script>\n+<a href=\"file:///etc/passwd\">&</a>", "</title><script>x</script>");
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("<a href"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("default-src 'none'"));
    }

    #[test]
    fn oversized_content_never_looks_like_a_complete_partial_diff() {
        for text in [
            "x".repeat(MAX_BYTES + 1),
            "\n".repeat(MAX_LINES + 1),
            "x".repeat(MAX_LINE_BYTES + 1),
        ] {
            let html = render(&text, "large.diff");
            assert!(html.contains("class=\"notice\""));
            assert!(!html.contains("class=\"line "));
        }
    }

    #[test]
    fn file_reader_bounds_bytes_and_rejects_bad_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("change.patch");
        for bytes in [
            vec![0xff, 0xfe],
            vec![b'a', 0, b'b'],
            vec![b' '; MAX_BYTES + 1],
        ] {
            std::fs::write(&path, bytes).unwrap();
            let html = render_file(path.to_str().unwrap());
            assert!(html.contains("class=\"notice\""));
            assert!(!html.contains("class=\"line "));
        }
        std::fs::write(&path, SAMPLE).unwrap();
        assert!(render_file(path.to_str().unwrap()).contains("class=\"line add\""));
        std::fs::remove_file(&path).unwrap();
        assert!(render_file(path.to_str().unwrap()).contains("class=\"notice\""));
    }
}
