//! Notes a person wrote on passages of an Agent's message (spec `message-annotations`).
//!
//! The popup and the phone let the user select text in the message, write a note, and send the
//! notes together with the answer. Everything funnels through one choke point,
//! `Coordinator::try_submit`, which calls [`sanitize`] once; rendering (text block, JSON,
//! What's next), the reply history and `--show-last` all read the cleaned list.
//!
//! The daemon passes the notes through without interpreting them: like `[user_input]` they are
//! untrusted human input to the Agent.

use crate::cli::output::MARKER_USER_ANNOTATIONS;
use serde::{Deserialize, Serialize};

/// Notes per request.
pub const MAX_ANNOTATIONS: usize = 20;
/// Characters in one note.
pub const MAX_NOTE_CHARS: usize = 2000;
/// Characters kept of a quoted passage (the selection is cut in the middle beyond this).
pub const MAX_QUOTE_CHARS: usize = 2000;
/// Characters of a quote shown in the text block and in `--show-last`; JSON and history keep the
/// whole stored quote.
pub const TEXT_QUOTE_CHARS: usize = 300;

/// One note: the passage it is about and what the user wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAnnotation {
    /// The selected text as seen (rendered, not Markdown source), whitespace folded to one line.
    pub quote: String,
    pub note: String,
}

/// Cleans the notes of a submission; the single place every source goes through.
///
/// - `allowed` is false for a cancellation and for a request that never offered notes: all notes
///   are dropped.
/// - Notes with an empty quote or an empty note (after trimming) are dropped.
/// - At most [`MAX_ANNOTATIONS`] are kept, in the order given (the sender sorts by position).
/// - A note is cut to [`MAX_NOTE_CHARS`] characters; a quote has its whitespace folded to single
///   spaces and is cut in the middle to [`MAX_QUOTE_CHARS`].
///
/// Idempotent: cleaning a cleaned list changes nothing.
pub fn sanitize(annotations: Vec<UserAnnotation>, allowed: bool) -> Vec<UserAnnotation> {
    if !allowed {
        return Vec::new();
    }
    annotations
        .into_iter()
        .filter_map(|annotation| {
            let quote = fold_whitespace(&annotation.quote);
            let note = annotation.note.trim();
            if quote.is_empty() || note.is_empty() {
                return None;
            }
            Some(UserAnnotation {
                quote: middle_truncate(&quote, MAX_QUOTE_CHARS),
                note: note.chars().take(MAX_NOTE_CHARS).collect(),
            })
        })
        .take(MAX_ANNOTATIONS)
        .collect()
}

/// Folds every run of whitespace (newlines included) to one space and trims both ends.
pub fn fold_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Keeps at most `max` characters by cutting the middle out and putting `…` in its place; the
/// result is exactly `max` characters long when something was cut, so cutting again is a no-op.
pub fn middle_truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max || max == 0 {
        return text.to_string();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let mut out: String = text.chars().take(head).collect();
    out.push('…');
    out.extend(text.chars().skip(count - tail));
    out
}

/// The quote as the text block and `--show-last` show it: at most [`TEXT_QUOTE_CHARS`].
pub fn quote_for_text(quote: &str) -> String {
    middle_truncate(quote, TEXT_QUOTE_CHARS)
}

/// The `[user_annotations]` block of the text output: per note a `> quote` line followed by the
/// note, a blank line between notes. Empty when there are no notes.
pub fn render_block(annotations: &[UserAnnotation]) -> String {
    if annotations.is_empty() {
        return String::new();
    }
    let entries = annotations
        .iter()
        .map(|a| format!("> {}\n{}", quote_for_text(&a.quote), a.note))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("{MARKER_USER_ANNOTATIONS}\n{entries}")
}

/// The same notes the way `--show-last` writes them: a `user_annotations:` section, every line
/// indented, a blank line between notes. Empty when there are no notes.
pub fn render_show_last(annotations: &[UserAnnotation]) -> String {
    if annotations.is_empty() {
        return String::new();
    }
    let mut out = String::from("user_annotations:");
    for (index, annotation) in annotations.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str("\n  > ");
        out.push_str(&quote_for_text(&annotation.quote));
        for line in annotation.note.lines() {
            out.push_str("\n  ");
            out.push_str(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(quote: &str, text: &str) -> UserAnnotation {
        UserAnnotation {
            quote: quote.into(),
            note: text.into(),
        }
    }

    #[test]
    fn sanitize_drops_everything_when_not_allowed() {
        assert!(sanitize(vec![note("a", "b")], false).is_empty());
    }

    #[test]
    fn sanitize_drops_empty_quotes_and_notes() {
        let cleaned = sanitize(
            vec![
                note("", "x"),
                note("   \n ", "x"),
                note("a", ""),
                note("a", " \t\n"),
                note("keep", "me"),
            ],
            true,
        );
        assert_eq!(cleaned, vec![note("keep", "me")]);
    }

    #[test]
    fn sanitize_folds_the_quote_and_trims_the_note() {
        let cleaned = sanitize(
            vec![note("  one\n\ttwo   three ", "  why?\nreally  ")],
            true,
        );
        assert_eq!(cleaned, vec![note("one two three", "why?\nreally")]);
    }

    #[test]
    fn sanitize_keeps_at_most_twenty_in_order() {
        let many: Vec<UserAnnotation> = (0..25).map(|i| note(&format!("q{i}"), "n")).collect();
        let cleaned = sanitize(many, true);
        assert_eq!(cleaned.len(), MAX_ANNOTATIONS);
        assert_eq!(cleaned[0].quote, "q0");
        assert_eq!(cleaned[19].quote, "q19");
    }

    #[test]
    fn sanitize_cuts_long_notes_and_quotes_by_characters() {
        let long_note = "注".repeat(2500);
        let long_quote = format!("{}{}{}", "头".repeat(1500), "中", "尾".repeat(1500));
        let cleaned = sanitize(vec![note(&long_quote, &long_note)], true);
        assert_eq!(cleaned[0].note.chars().count(), MAX_NOTE_CHARS);
        assert_eq!(cleaned[0].quote.chars().count(), MAX_QUOTE_CHARS);
        assert!(cleaned[0].quote.starts_with("头"));
        assert!(cleaned[0].quote.contains('…'));
        assert!(cleaned[0].quote.ends_with("尾"));
        // Exactly at the limits nothing is cut.
        let exact = sanitize(
            vec![note(
                &"q".repeat(MAX_QUOTE_CHARS),
                &"n".repeat(MAX_NOTE_CHARS),
            )],
            true,
        );
        assert_eq!(exact[0].quote.chars().count(), MAX_QUOTE_CHARS);
        assert!(!exact[0].quote.contains('…'));
        assert_eq!(exact[0].note.chars().count(), MAX_NOTE_CHARS);
    }

    #[test]
    fn sanitize_counts_characters_not_bytes_or_utf16_units() {
        // An emoji is four bytes and two UTF-16 units but one character.
        let quote = "😀".repeat(MAX_QUOTE_CHARS);
        let note_text = "😀".repeat(MAX_NOTE_CHARS);
        let cleaned = sanitize(vec![note(&quote, &note_text)], true);
        assert_eq!(cleaned[0].quote.chars().count(), MAX_QUOTE_CHARS);
        assert!(!cleaned[0].quote.contains('…'));
        assert_eq!(cleaned[0].note.chars().count(), MAX_NOTE_CHARS);
    }

    #[test]
    fn sanitize_is_idempotent() {
        let messy = vec![
            note("  a \n b ", "  x  "),
            note(&"长".repeat(3000), &"文".repeat(2100)),
            note("", "dropped"),
        ];
        let once = sanitize(messy, true);
        let twice = sanitize(once.clone(), true);
        assert_eq!(once, twice);
    }

    #[test]
    fn middle_truncate_keeps_head_and_tail_around_an_ellipsis() {
        assert_eq!(middle_truncate("abcdef", 6), "abcdef");
        assert_eq!(middle_truncate("abcdefg", 6), "ab…efg");
        assert_eq!(middle_truncate("abcdefghij", 5), "ab…ij");
        assert_eq!(middle_truncate("abc", 0), "abc");
        let cut = middle_truncate(&"x".repeat(5000), MAX_QUOTE_CHARS);
        assert_eq!(cut.chars().count(), MAX_QUOTE_CHARS);
        assert_eq!(middle_truncate(&cut, MAX_QUOTE_CHARS), cut);
    }

    #[test]
    fn text_quote_boundary_is_three_hundred_characters() {
        let at = |n: usize| "字".repeat(n);
        assert_eq!(quote_for_text(&at(299)), at(299));
        assert_eq!(quote_for_text(&at(300)), at(300));
        let cut = quote_for_text(&at(301));
        assert_eq!(cut.chars().count(), 300);
        assert_eq!(cut.chars().filter(|c| *c == '…').count(), 1);
        assert!(cut.starts_with(&at(149)));
        assert!(cut.ends_with(&at(150)));
    }

    #[test]
    fn block_lists_quote_then_note_with_a_blank_line_between_notes() {
        let block = render_block(&[
            note("需要同时调整三处表单", "是哪三处？\n把文件名列一下。"),
            note("回归测试也已补充", "补在哪个文件里？"),
        ]);
        assert_eq!(
            block,
            "[user_annotations]\n> 需要同时调整三处表单\n是哪三处？\n把文件名列一下。\n\n> 回归测试也已补充\n补在哪个文件里？"
        );
        assert_eq!(render_block(&[]), "");
    }

    #[test]
    fn block_shortens_long_quotes_but_not_the_stored_ones() {
        let stored = "字".repeat(500);
        let notes = [note(&stored, "n")];
        let block = render_block(&notes);
        let quote_line = block.lines().nth(1).unwrap();
        assert_eq!(quote_line.chars().count(), 2 + 300);
        assert_eq!(notes[0].quote.chars().count(), 500);
    }

    #[test]
    fn show_last_section_indents_every_line() {
        let text = render_show_last(&[note("q1", "a\nb"), note("q2", "c")]);
        assert_eq!(text, "user_annotations:\n  > q1\n  a\n  b\n\n  > q2\n  c");
        assert_eq!(render_show_last(&[]), "");
    }

    #[test]
    fn wire_form_is_quote_and_note() {
        let value = serde_json::to_value(note("q", "n")).unwrap();
        assert_eq!(value, serde_json::json!({"quote": "q", "note": "n"}));
        let back: UserAnnotation = serde_json::from_value(value).unwrap();
        assert_eq!(back, note("q", "n"));
    }
}
