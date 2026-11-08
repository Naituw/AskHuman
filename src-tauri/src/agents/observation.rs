//! Completed, publicly recorded text for the desktop and phone observation views.

use serde::Serialize;
use std::hash::{DefaultHasher, Hash, Hasher};

const MAX_TEXT_BYTES: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestText {
    pub id: String,
    pub revision: String,
    pub kind: &'static str,
    pub text: String,
    pub truncated: bool,
    pub at: Option<u64>,
}

pub fn fingerprint(text: &str) -> String {
    let mut hash = DefaultHasher::new();
    text.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

impl LatestText {
    pub fn new(id: String, kind: &'static str, text: &str, at: Option<u64>) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let revision = fingerprint(text);
        let truncated = text.len() > MAX_TEXT_BYTES;
        let mut start = text.len().saturating_sub(MAX_TEXT_BYTES);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        Some(Self {
            id,
            revision,
            kind,
            text: text[start..].to_string(),
            truncated,
            at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_bounded_by_bytes_and_retains_the_latest_words() {
        let text = format!("{}\nLatest action", "思考".repeat(4_000));
        let value = LatestText::new("one".into(), "thinking", &text, None).unwrap();
        assert!(value.truncated);
        assert!(value.text.len() <= MAX_TEXT_BYTES);
        assert!(value.text.ends_with("Latest action"));
        assert!(LatestText::new("one".into(), "thinking", "  ", None).is_none());
    }
}
