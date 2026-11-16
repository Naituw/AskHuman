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
use serde::{Deserialize, Serialize};

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

/// What a new card receives from the draft of an abandoned ask (spec D7): one entry per question,
/// the pictures read back from the files they were saved to.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftLoad {
    pub answers: Vec<LoadedAnswer>,
    pub annotations: Vec<UserAnnotation>,
    /// Pictures the draft had whose files are gone (the system clears its temporary directory
    /// after a while). The card says how many, so the user knows something is missing.
    pub missing_images: usize,
}

/// One question of a loaded draft, shaped like what the form holds.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedAnswer {
    pub selected_options: Vec<String>,
    pub user_input: String,
    pub images: Vec<ImageAttachment>,
    pub files: Vec<String>,
}

/// A picture larger than this is not read back: a draft is only a head start, and a file that big
/// was not saved by a form that took it from the clipboard.
const MAX_DRAFT_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// Read a saved picture back into the form (a data URL, like one pasted or dropped).
pub fn read_image(path: &str) -> std::io::Result<ImageAttachment> {
    if std::fs::metadata(path)?.len() > MAX_DRAFT_IMAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "saved picture is too large",
        ));
    }
    Ok(ImageAttachment {
        data: crate::commands::image_data_url(path)?,
        media_type: crate::commands::image_mime_from_path(path).to_string(),
        filename: std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
    })
}

/// Whether a reply file is certainly gone. Only "not found" counts: a path that cannot be examined
/// for another reason (a refused look at a protected folder) is still offered, since a wrong "gone"
/// costs the user a file they could have used.
pub fn file_gone(path: &str) -> bool {
    matches!(
        std::fs::metadata(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// The page's form of a claimed draft. `read_image` brings a saved picture back (any error counts
/// it as missing); `file_gone` says whether a reply file no longer exists (such a file is left out
/// quietly, as the spec says: it is only offered while the path still leads somewhere).
pub fn load(
    draft: HistoryDraft,
    mut read_image: impl FnMut(&str) -> std::io::Result<ImageAttachment>,
    file_gone: impl Fn(&str) -> bool,
) -> DraftLoad {
    let mut missing_images = 0;
    let answers = draft
        .answers
        .into_iter()
        .map(|answer| {
            let mut images = Vec::with_capacity(answer.images.len());
            for path in &answer.images {
                match read_image(path) {
                    Ok(image) => images.push(image),
                    Err(_) => missing_images += 1,
                }
            }
            LoadedAnswer {
                selected_options: answer.selected_options,
                user_input: answer.user_input.unwrap_or_default(),
                images,
                files: answer
                    .files
                    .into_iter()
                    .filter(|path| !file_gone(path))
                    .collect(),
            }
        })
        .collect();
    DraftLoad {
        answers,
        annotations: draft.annotations,
        missing_images,
    }
}

/// Take the draft an abandoned ask of the same session and question left, for the card of
/// `request_id` (spec D6). A card whose question names no session gets nothing. At most once per
/// draft; asking again with the same request id hands back the draft it already took.
///
/// Blocking (file locks, image files): call it off the async executor.
pub fn claim(key: &DraftKey, request_id: &str) -> Option<DraftLoad> {
    let session = key.session_key.as_deref()?;
    let draft = history::claim_draft(session, &key.fingerprint, request_id)?;
    Some(load(draft, read_image, file_gone))
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

    fn stored(answers: Vec<HistoryAnswer>) -> HistoryDraft {
        HistoryDraft {
            key: Some(key()),
            answers,
            annotations: vec![UserAnnotation {
                quote: "q".into(),
                note: "n".into(),
            }],
            claimed_by: None,
        }
    }

    fn picture(path: &str) -> std::io::Result<ImageAttachment> {
        if path.contains("gone") {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        Ok(ImageAttachment {
            data: format!("data:image/png;base64,{path}"),
            media_type: "image/png".into(),
            filename: Some(path.into()),
        })
    }

    /// Spec D7: every part comes back in its question; a picture whose file is gone is counted, not
    /// fatal; a reply file whose path leads nowhere is left out quietly.
    #[test]
    fn a_draft_comes_back_question_by_question_and_counts_what_is_gone() {
        let draft = stored(vec![
            HistoryAnswer {
                selected_options: vec!["Yes".into()],
                user_input: Some("because".into()),
                images: vec!["a.png".into(), "gone-1.png".into(), "b.png".into()],
                files: vec!["/there".into(), "/gone".into()],
            },
            HistoryAnswer {
                images: vec!["gone-2.png".into()],
                ..Default::default()
            },
        ]);
        let loaded = load(draft, picture, |path| path.contains("gone"));
        assert_eq!(loaded.answers.len(), 2);
        assert_eq!(loaded.answers[0].selected_options, vec!["Yes"]);
        assert_eq!(loaded.answers[0].user_input, "because");
        let names: Vec<_> = loaded.answers[0]
            .images
            .iter()
            .map(|image| image.filename.as_deref().unwrap())
            .collect();
        assert_eq!(names, vec!["a.png", "b.png"]);
        assert_eq!(loaded.answers[0].files, vec!["/there"]);
        assert_eq!(
            loaded.answers[1].user_input, "",
            "no text is an empty string"
        );
        assert!(loaded.answers[1].images.is_empty());
        assert_eq!(loaded.missing_images, 2);
        assert_eq!(loaded.annotations.len(), 1);
    }

    #[test]
    fn the_page_reads_a_loaded_draft_in_the_shape_of_its_form() {
        let loaded = load(
            stored(vec![HistoryAnswer {
                selected_options: vec!["Yes".into()],
                images: vec!["a.png".into()],
                ..Default::default()
            }]),
            picture,
            |_| false,
        );
        let wire = serde_json::to_value(&loaded).unwrap();
        assert_eq!(wire["missingImages"], 0);
        assert_eq!(wire["answers"][0]["selectedOptions"][0], "Yes");
        assert_eq!(wire["answers"][0]["userInput"], "");
        assert_eq!(wire["answers"][0]["images"][0]["mediaType"], "image/png");
        assert_eq!(wire["answers"][0]["files"], serde_json::json!([]));
        assert_eq!(wire["annotations"][0]["quote"], "q");
    }

    /// What is kept is what comes back: a picture is written the way a reply's is, and read back
    /// byte for byte.
    #[test]
    fn a_saved_picture_comes_back_byte_for_byte() {
        use base64::Engine;
        let dir = tempfile::tempdir().unwrap();
        let original = image("shot.png");
        let save = DraftSave {
            answers: vec![QuestionAnswer {
                images: vec![original.clone()],
                ..Default::default()
            }],
            annotations: Vec::new(),
        };
        let draft = build(Some(key()), save, |_, images| {
            crate::cli::image_writer::save_into_dir(images, dir.path(), Lang::En)
        });
        let loaded = load(draft, read_image, file_gone);
        assert_eq!(loaded.missing_images, 0);
        let back = &loaded.answers[0].images[0];
        assert_eq!(back.media_type, "image/png");
        assert_eq!(back.filename.as_deref(), Some("shot.png"));
        let encoded = back.data.strip_prefix("data:image/png;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let sent = base64::engine::general_purpose::STANDARD
            .decode(&original.data)
            .unwrap();
        assert_eq!(bytes, sent);
    }

    #[test]
    fn a_picture_that_is_gone_or_too_large_is_not_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("cleared.png");
        let error = read_image(missing.to_str().unwrap()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        // A sparse file: large on paper, nothing on disk.
        let huge = dir.path().join("huge.png");
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_DRAFT_IMAGE_BYTES + 1)
            .unwrap();
        let error = read_image(huge.to_str().unwrap()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn only_a_file_that_is_not_found_counts_as_gone() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().join("report.txt");
        std::fs::write(&here, "x").unwrap();
        assert!(!file_gone(here.to_str().unwrap()));
        assert!(file_gone(dir.path().join("cleared.txt").to_str().unwrap()));
    }

    /// A card whose question names no session gets nothing, and the history is not even looked at.
    #[test]
    fn a_question_without_a_session_claims_nothing() {
        let anonymous = DraftKey {
            session_key: None,
            fingerprint: "f".into(),
        };
        assert!(claim(&anonymous, "r").is_none());
    }
}
