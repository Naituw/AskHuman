//! The draft of an ask whose caller went away (spec `popup-draft-on-cancel`): what the page
//! reports when the card is closed, turned into the history record that outlives the card.
//!
//! The popup host does this itself rather than handing it to the daemon: the draft lives in the
//! page, the history file is shared by a cross-process lock, and a new `ClientMsg` would move the
//! protocol number for nothing.
use crate::annotations::{sanitize, UserAnnotation};
use crate::history::{self, DraftKey, HistoryAnswer, HistoryDraft};
use crate::i18n::Lang;
use crate::models::{ImageAttachment, QuestionAnswer};
use serde::Deserialize;

/// What a card held when its caller went away, as the page reports it (images still inline).
/// Shaped like a submission so the page builds it from the same state; the todo fields a
/// submission carries are ignored (spec D2: not part of a draft).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSave {
    #[serde(default)]
    pub answers: Vec<QuestionAnswer>,
    #[serde(default)]
    pub annotations: Vec<UserAnnotation>,
}

impl DraftSave {
    /// Whether the user typed anything worth keeping, by the same rule the stored draft uses.
    fn is_empty(&self) -> bool {
        self.annotations.is_empty()
            && self.answers.iter().all(|answer| {
                answer.selected_options.is_empty()
                    && answer
                        .user_input
                        .as_deref()
                        .is_none_or(|text| text.trim().is_empty())
                    && answer.images.is_empty()
                    && answer.files.is_empty()
            })
    }
}

/// The stored form of `save`. Each question's images go through `write_images(question, images)`,
/// which returns where they were written; an image that cannot be written is left out rather
/// than losing the text next to it. Notes are cleaned the way a sent answer's are.
pub fn build(
    key: Option<DraftKey>,
    save: DraftSave,
    mut write_images: impl FnMut(usize, &[ImageAttachment]) -> anyhow::Result<Vec<String>>,
) -> HistoryDraft {
    let answers = save
        .answers
        .into_iter()
        .enumerate()
        .map(|(question, answer)| HistoryAnswer {
            images: write_images(question, &answer.images).unwrap_or_default(),
            selected_options: answer.selected_options,
            user_input: answer.user_input,
            files: answer.files,
        })
        .collect();
    HistoryDraft {
        key,
        answers,
        annotations: sanitize(save.annotations, true),
        claimed_by: None,
    }
}

/// Keep the draft of the ask `request_id` on its cancel entry in the history. Best-effort: with
/// nothing typed, with history off or trimmed away (no cancel entry to hang it on), or when the
/// write fails, nothing is kept and nothing is reported to the user. Returns whether it was kept.
///
/// Blocking (file locks, image files): call it off the async executor.
pub fn keep(request_id: &str, key: DraftKey, mut save: DraftSave, lang: Lang) -> bool {
    save.annotations = sanitize(save.annotations, true);
    if save.is_empty() {
        return false;
    }
    // Ask first, so no image is written for a draft that nothing would take.
    if !history::accepts_draft(request_id) {
        return false;
    }
    let draft = build(Some(key), save, |question, images| {
        crate::cli::image_writer::save(images, request_id, question, lang)
    });
    history::attach_draft(request_id, draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answer(text: &str) -> QuestionAnswer {
        serde_json::from_value(json!({ "userInput": text })).unwrap()
    }

    fn image(name: &str) -> ImageAttachment {
        ImageAttachment {
            data: "iVBORw0KGgo=".into(),
            media_type: "image/png".into(),
            filename: Some(name.into()),
        }
    }

    fn key() -> DraftKey {
        DraftKey {
            session_key: Some("sid:a".into()),
            fingerprint: "f".into(),
        }
    }

    /// The page reports an answer the way it submits one; todo fields and an empty list of images
    /// are accepted and the draft keeps none of the todo parts.
    #[test]
    fn a_report_reads_like_a_submission() {
        let save: DraftSave = serde_json::from_value(json!({
            "answers": [{
                "selectedOptions": ["Yes"], "userInput": "why",
                "images": [{ "data": "AAAA", "mediaType": "image/png" }],
                "files": ["/tmp/a.txt"],
                "todoIds": ["t1"], "todoSelections": [{ "id": "t1", "attachments": [] }]
            }],
            "annotations": [{ "quote": "q", "note": "n" }]
        }))
        .unwrap();
        assert_eq!(save.answers.len(), 1);
        assert_eq!(save.annotations.len(), 1);
        assert!(!save.is_empty());
        let bare: DraftSave = serde_json::from_value(json!({})).unwrap();
        assert!(bare.is_empty());
    }

    #[test]
    fn only_something_typed_is_worth_keeping() {
        let mut save = DraftSave {
            answers: vec![answer(""), answer("  \n ")],
            annotations: Vec::new(),
        };
        assert!(save.is_empty(), "untouched and blank questions");
        save.answers[1] = answer("text");
        assert!(!save.is_empty());
        for filled in [
            serde_json::from_value::<QuestionAnswer>(json!({ "selectedOptions": ["A"] })).unwrap(),
            serde_json::from_value::<QuestionAnswer>(
                json!({ "images": [{ "data": "AA", "mediaType": "image/png" }] }),
            )
            .unwrap(),
            serde_json::from_value::<QuestionAnswer>(json!({ "files": ["/tmp/x"] })).unwrap(),
        ] {
            let one = DraftSave {
                answers: vec![filled],
                annotations: Vec::new(),
            };
            assert!(!one.is_empty());
        }
        let notes_only = DraftSave {
            answers: vec![answer("")],
            annotations: vec![UserAnnotation {
                quote: "q".into(),
                note: "n".into(),
            }],
        };
        assert!(!notes_only.is_empty());
    }

    /// Spec D2/D4: the stored draft has the same shape as a sent answer, with images as the paths
    /// they were written to, one question at a time (the writer is told which one).
    #[test]
    fn the_stored_draft_keeps_each_question_and_where_its_images_went() {
        let save = DraftSave {
            answers: vec![
                QuestionAnswer {
                    selected_options: vec!["Yes".into()],
                    user_input: Some("because".into()),
                    images: vec![image("a.png"), image("b.png")],
                    files: vec!["/tmp/a.txt".into()],
                    ..Default::default()
                },
                answer("second"),
            ],
            annotations: vec![UserAnnotation {
                quote: "  the   quoted\npassage ".into(),
                note: " a note ".into(),
            }],
        };
        let mut asked = Vec::new();
        let draft = build(Some(key()), save, |question, images| {
            asked.push((question, images.len()));
            Ok((0..images.len())
                .map(|n| format!("/tmp/askhuman/r/q{}/img-{n}.png", question + 1))
                .collect())
        });
        assert_eq!(asked, vec![(0, 2), (1, 0)]);
        assert_eq!(draft.key, Some(key()));
        assert_eq!(draft.claimed_by, None);
        assert_eq!(draft.answers.len(), 2);
        assert_eq!(draft.answers[0].selected_options, vec!["Yes"]);
        assert_eq!(draft.answers[0].user_input.as_deref(), Some("because"));
        assert_eq!(
            draft.answers[0].images,
            vec![
                "/tmp/askhuman/r/q1/img-0.png",
                "/tmp/askhuman/r/q1/img-1.png"
            ]
        );
        assert_eq!(draft.answers[0].files, vec!["/tmp/a.txt"]);
        assert_eq!(draft.answers[1].user_input.as_deref(), Some("second"));
        assert!(draft.answers[1].images.is_empty());
        // Notes are cleaned like a sent answer's: the quote folded to one line, the note trimmed.
        assert_eq!(draft.annotations[0].quote, "the quoted passage");
        assert_eq!(draft.annotations[0].note, "a note");
        assert!(draft.has_content());
    }

    /// An image that cannot be written must not take the text with it.
    #[test]
    fn an_image_that_cannot_be_written_does_not_lose_the_text() {
        let save = DraftSave {
            answers: vec![QuestionAnswer {
                user_input: Some("keep me".into()),
                images: vec![image("a.png")],
                ..Default::default()
            }],
            annotations: Vec::new(),
        };
        let draft = build(Some(key()), save, |_, _| anyhow::bail!("disk full"));
        assert_eq!(draft.answers[0].user_input.as_deref(), Some("keep me"));
        assert!(draft.answers[0].images.is_empty());
        assert!(draft.has_content());
    }

    /// With no history to hang it on, nothing is written: `keep` asks the history first, so the
    /// images are never written either. (The history here is whatever file the test process
    /// resolves, so the request id is one nothing has ever recorded.)
    #[test]
    fn nothing_is_written_for_an_ask_the_history_does_not_know() {
        let id = format!("draft-test-{}", uuid::Uuid::new_v4());
        let save = DraftSave {
            answers: vec![QuestionAnswer {
                user_input: Some("typed".into()),
                images: vec![image("a.png")],
                ..Default::default()
            }],
            annotations: Vec::new(),
        };
        assert!(!keep(&id, key(), save, Lang::En));
        assert!(
            !crate::paths::request_temp_dir(&id).exists(),
            "no image directory for a draft nothing would take"
        );
    }

    #[test]
    fn nothing_typed_is_never_kept() {
        let id = format!("draft-test-{}", uuid::Uuid::new_v4());
        let save = DraftSave {
            answers: vec![answer("   ")],
            annotations: Vec::new(),
        };
        assert!(!keep(&id, key(), save, Lang::En));
    }
}
