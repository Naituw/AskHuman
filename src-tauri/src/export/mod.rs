//! Render export artifacts for IM commands (diff / transcript → HTML or docx).

pub mod diff_docx;
pub mod diff_html;
pub mod diff_md;
pub mod transcript_docx;
pub mod transcript_html;
pub mod transcript_md;

use crate::agents::transcript_full::TranscriptDoc;
use crate::gitutil::DiffModel;

/// Sanitize a path segment for filenames.
pub fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if out.len() >= max {
            break;
        }
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c == ' ' || c == '.' {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "file".into()
    } else {
        out
    }
}

pub fn diff_filename(seq: u64, project: &str, ext: &str) -> String {
    format!("diff-{}-{}.{}", seq, slug(project, 40), ext)
}

pub fn transcript_filename(seq: u64, title: &str, ext: &str) -> String {
    format!("transcript-{}-{}.{}", seq, slug(title, 40), ext)
}

pub fn render_diff_html(model: &DiffModel, meta_title: &str) -> String {
    diff_html::render(model, meta_title)
}

pub fn render_diff_md(model: &DiffModel, meta_title: &str) -> String {
    diff_md::render(model, meta_title)
}

pub fn render_diff_docx(model: &DiffModel, meta_title: &str) -> Result<Vec<u8>, String> {
    diff_docx::render(model, meta_title)
}

pub fn render_transcript_html(doc: &TranscriptDoc, meta_title: &str) -> String {
    transcript_html::render(doc, meta_title)
}

pub fn render_transcript_md(doc: &TranscriptDoc, meta_title: &str) -> String {
    transcript_md::render(doc, meta_title)
}

pub fn render_transcript_docx(doc: &TranscriptDoc, meta_title: &str) -> Result<Vec<u8>, String> {
    transcript_docx::render(doc, meta_title)
}
