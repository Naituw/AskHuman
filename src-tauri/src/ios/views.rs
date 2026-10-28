//! Projections of daemon requests onto the wire views the phone renders (plan §4.4).

use std::path::PathBuf;

use crate::channels::ConversationOrigin;
use crate::confirm::ActionRole;
use crate::models::{
    AskRequest, ConfirmFieldKind, ConfirmRequest, FileAttachment, DEFAULT_SOURCE_NAME,
};

use super::crypto;
use super::wire::{
    AgentRef, AskView, AssetRef, ConfirmChoiceView, ConfirmDetailView, ConfirmFieldView,
    ConfirmInputView, ConfirmView, MessageView, OptionView, ProjectRef, QuestionView, RequestView,
    MAX_ASSET_BYTES, MAX_REQUEST_IMAGES, SESSION_TITLE_MAX_CHARS, THUMB_MAX_BYTES,
};

/// A file the phone may fetch with `asset.get` while the request is pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSource {
    pub asset_id: String,
    pub path: PathBuf,
    pub name: String,
    pub mime: String,
    pub size: u64,
}

pub struct BuiltView {
    pub view: RequestView,
    pub assets: Vec<AssetSource>,
}

fn agent_ref(origin: &ConversationOrigin) -> AgentRef {
    AgentRef {
        kind: origin.agent_kind().unwrap_or("other").to_string(),
        label: origin
            .agent_label()
            .unwrap_or_else(|| origin.source())
            .to_string(),
    }
}

fn project_ref(origin: &ConversationOrigin) -> ProjectRef {
    ProjectRef {
        key: origin.project_key().to_string(),
        name: origin.project_name().unwrap_or_default().to_string(),
    }
}

/// Source label shown next to the agent: only a custom `ASKHUMAN_SOURCE` name, since the Agent
/// itself already has its own badge.
fn source_label(origin: &ConversationOrigin) -> String {
    let source = origin.source();
    if source.eq_ignore_ascii_case(DEFAULT_SOURCE_NAME) || origin.agent_label() == Some(source) {
        String::new()
    } else {
        source.to_string()
    }
}

pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" | "heif" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

/// Small JPEG preview (<= 24 KB) of an image file; `None` when it cannot be decoded.
pub fn thumbnail(path: &std::path::Path) -> Option<Vec<u8>> {
    let image = image::open(path).ok()?;
    for (edge, quality) in [(240, 70), (200, 60), (160, 50), (120, 40)] {
        let small = image.thumbnail(edge, edge).to_rgb8();
        let mut out = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        if small.write_with_encoder(encoder).is_err() {
            return None;
        }
        if out.len() <= THUMB_MAX_BYTES {
            return Some(out);
        }
    }
    None
}

/// A session title as a request carries it: one line (whitespace runs collapse into one space) of
/// at most `SESSION_TITLE_MAX_CHARS` characters, `None` when nothing is left.
pub fn clean_session_title(raw: &str) -> Option<String> {
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return None;
    }
    if one_line.chars().count() <= SESSION_TITLE_MAX_CHARS {
        return Some(one_line);
    }
    let cut: String = one_line.chars().take(SESSION_TITLE_MAX_CHARS).collect();
    Some(format!("{}…", cut.trim_end()))
}

fn asset_for(file: &FileAttachment, with_thumb: bool) -> (AssetRef, Option<AssetSource>) {
    let asset_id = crypto::new_id();
    let mime = mime_for(&file.name).to_string();
    let too_large = file.size > MAX_ASSET_BYTES;
    let thumb = (with_thumb && file.is_image && !too_large)
        .then(|| thumbnail(std::path::Path::new(&file.path)))
        .flatten()
        .map(|bytes| crypto::b64u(&bytes));
    let reference = AssetRef {
        asset_id: asset_id.clone(),
        name: file.name.clone(),
        size: file.size,
        mime: mime.clone(),
        is_image: file.is_image,
        thumb,
        too_large,
    };
    let source = (!too_large).then(|| AssetSource {
        asset_id,
        path: PathBuf::from(&file.path),
        name: file.name.clone(),
        mime,
        size: file.size,
    });
    (reference, source)
}

/// The phone view of a question. Blocking (reads and decodes attachments): call it from a
/// blocking context.
pub fn ask_view(
    request: &AskRequest,
    origin: &ConversationOrigin,
    created_at_ms: i64,
) -> BuiltView {
    let mut assets = Vec::new();
    let mut files = Vec::new();
    let mut thumbs_left = MAX_REQUEST_IMAGES;
    for file in &request.message.files {
        let with_thumb = thumbs_left > 0;
        if with_thumb && file.is_image {
            thumbs_left -= 1;
        }
        let (reference, source) = asset_for(file, with_thumb);
        files.push(reference);
        assets.extend(source);
    }
    let questions = request
        .questions
        .iter()
        .map(|question| QuestionView {
            text: question.message.clone(),
            options: question
                .predefined_options
                .iter()
                .map(|option| OptionView {
                    text: option.text.clone(),
                    recommended: option.recommended,
                    todo: option.todo_id.is_some(),
                })
                .collect(),
        })
        .collect();
    let view = RequestView {
        id: request.id.clone(),
        kind: "ask".into(),
        created_at: created_at_ms,
        agent: agent_ref(origin),
        project: project_ref(origin),
        session_title: None,
        source: source_label(origin),
        content_hash: String::new(),
        truncated: false,
        ask: Some(AskView {
            is_markdown: request.is_markdown,
            message: MessageView {
                text: request.message.text.clone(),
                files,
            },
            questions,
            select_only: request.select_only,
            single: request.single,
            whats_next: request.whats_next,
        }),
        confirm: None,
    };
    BuiltView { view, assets }
}

fn role_name(role: ActionRole) -> &'static str {
    match role {
        ActionRole::Primary => "primary",
        ActionRole::Destructive => "destructive",
        ActionRole::Default => "default",
    }
}

fn field_kind_name(kind: ConfirmFieldKind) -> &'static str {
    match kind {
        ConfirmFieldKind::Text => "text",
        ConfirmFieldKind::Path => "path",
        ConfirmFieldKind::Timestamp => "timestamp",
    }
}

/// Prefix-tier variants have no level selector on the phone: only the recommended variant of a
/// group is offered (wire indices still refer to the full list).
fn variant_visible(choice: &crate::models::ConfirmChoice) -> bool {
    choice
        .variant
        .as_ref()
        .is_none_or(|variant| variant.recommended)
}

pub fn confirm_view(request: &ConfirmRequest, origin: &ConversationOrigin) -> RequestView {
    let form = request.choice_form_view();
    let choices: Vec<ConfirmChoiceView> = request
        .choices
        .iter()
        .enumerate()
        .filter(|(_, choice)| variant_visible(choice))
        .map(|(wire_index, choice)| ConfirmChoiceView {
            wire_index,
            label: choice.label.clone(),
            description: choice.description.clone(),
            role: role_name(choice.role).into(),
            recommended: false,
        })
        .collect();
    let offered = |index: usize| choices.iter().any(|c| c.wire_index == index);
    let default_index = form.default_index.filter(|index| offered(*index));
    let input = form.input.as_ref().map(|input| {
        let visible_when_index = match &input.visibility {
            crate::models::ChoiceFormInputVisibility::Always => None,
            crate::models::ChoiceFormInputVisibility::WhenIndex(index) => Some(*index),
        };
        ConfirmInputView {
            id: input.id.clone(),
            label: input.label.clone(),
            placeholder: input.placeholder.clone(),
            max_chars: input.max_chars,
            visible_when_index,
            required: request
                .presentation
                .input()
                .is_some_and(|spec| spec.requires_value()),
        }
    });
    RequestView {
        id: request.id.clone(),
        kind: "confirm".into(),
        created_at: request.created_at_ms as i64,
        agent: agent_ref(origin),
        project: project_ref(origin),
        session_title: None,
        source: source_label(origin),
        content_hash: String::new(),
        truncated: false,
        ask: None,
        confirm: Some(ConfirmView {
            title: request.title.clone(),
            context: request
                .context
                .iter()
                .map(|field| ConfirmFieldView {
                    id: field.id.clone(),
                    label: field.label.clone(),
                    value: field.value.clone(),
                    kind: field_kind_name(field.kind).into(),
                })
                .collect(),
            detail: ConfirmDetailView {
                summary: request.detail.summary.clone(),
                body_md: request.detail.body_md.clone(),
            },
            choices,
            default_index,
            input,
            dismiss_index: Some(request.dismiss_index()),
            submit_label: form.submit_label,
            expires_at: request.expires_at_ms as i64,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ChoiceVariant, ConfirmChoice, ConfirmDetail, ConfirmField, ConfirmInput,
        ConfirmPresentation, MessagePrompt, OptionItem, Question,
    };

    fn origin() -> ConversationOrigin {
        ConversationOrigin::new("Codex", Some("codex"), "/Users/me/work/HumanInLoop")
    }

    #[test]
    fn ask_view_carries_origin_options_and_flags() {
        let mut request = AskRequest::new(
            MessagePrompt {
                text: "Pick one".into(),
                files: Vec::new(),
            },
            vec![Question {
                message: "Which?".into(),
                predefined_options: vec![OptionItem::new("a", false), OptionItem::new("b", true)],
            }],
            true,
        );
        request.single = true;
        let built = ask_view(&request, &origin(), 42);
        let view = built.view;
        assert_eq!(view.kind, "ask");
        assert_eq!(view.created_at, 42);
        assert_eq!(view.agent.kind, "codex");
        assert_eq!(view.agent.label, "Codex");
        assert_eq!(view.project.name, "HumanInLoop");
        assert_eq!(view.project.key, "/Users/me/work/HumanInLoop");
        let ask = view.ask.unwrap();
        assert!(ask.is_markdown && ask.single && !ask.select_only);
        assert_eq!(ask.questions[0].options[1].text, "b");
        assert!(ask.questions[0].options[1].recommended);
        assert!(built.assets.is_empty());
    }

    #[test]
    fn default_source_without_an_agent_is_labelled_by_the_source_name() {
        let origin = ConversationOrigin::new(DEFAULT_SOURCE_NAME, None, "");
        let request = AskRequest::new(MessagePrompt::default(), vec![], false);
        let view = ask_view(&request, &origin, 0).view;
        assert_eq!(view.agent.kind, "other");
        assert_eq!(view.agent.label, DEFAULT_SOURCE_NAME);
        assert_eq!(view.source, "");
    }

    #[test]
    fn a_session_title_is_one_short_line_or_nothing() {
        assert_eq!(clean_session_title(""), None);
        assert_eq!(clean_session_title(" \n\t "), None);
        assert_eq!(
            clean_session_title("  Fix the\n flaky   test\t"),
            Some("Fix the flaky test".into()),
            "whitespace runs collapse into one space"
        );
        let exact = "字".repeat(SESSION_TITLE_MAX_CHARS);
        assert_eq!(clean_session_title(&exact), Some(exact.clone()));
        let long = format!("{exact}尾巴");
        let cut = clean_session_title(&long).unwrap();
        assert_eq!(cut.chars().count(), SESSION_TITLE_MAX_CHARS + 1);
        assert!(cut.ends_with('…'));
        // A cut never leaves a space in front of the ellipsis.
        let spaced = format!(
            "{} {}",
            "a".repeat(SESSION_TITLE_MAX_CHARS - 1),
            "b".repeat(30)
        );
        assert!(!clean_session_title(&spaced).unwrap().ends_with(" …"));
    }

    #[test]
    fn views_start_without_a_session_title() {
        let request = AskRequest::new(MessagePrompt::default(), vec![], false);
        assert_eq!(ask_view(&request, &origin(), 0).view.session_title, None);
    }

    #[test]
    fn attachments_become_assets_with_thumbnails_for_images() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        image::RgbImage::from_pixel(640, 480, image::Rgb([200, 30, 30]))
            .save(&png)
            .unwrap();
        let txt = dir.path().join("notes.txt");
        std::fs::write(&txt, "hello").unwrap();
        let request = AskRequest::new(
            MessagePrompt {
                text: String::new(),
                files: vec![
                    FileAttachment {
                        path: png.display().to_string(),
                        name: "shot.png".into(),
                        size: std::fs::metadata(&png).unwrap().len(),
                        is_image: true,
                    },
                    FileAttachment {
                        path: txt.display().to_string(),
                        name: "notes.txt".into(),
                        size: 5,
                        is_image: false,
                    },
                    FileAttachment {
                        path: "/nonexistent/huge.bin".into(),
                        name: "huge.bin".into(),
                        size: MAX_ASSET_BYTES + 1,
                        is_image: false,
                    },
                ],
            },
            vec![Question {
                message: "q".into(),
                predefined_options: vec![],
            }],
            false,
        );
        let built = ask_view(&request, &origin(), 0);
        let files = built.view.ask.unwrap().message.files;
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].mime, "image/png");
        let thumb = crypto::b64u_decode(files[0].thumb.as_deref().unwrap()).unwrap();
        assert!(thumb.len() <= THUMB_MAX_BYTES);
        assert_eq!(&thumb[..2], &[0xff, 0xd8], "JPEG");
        assert!(files[1].thumb.is_none());
        assert_eq!(files[1].mime, "text/plain");
        assert!(files[2].too_large);
        // Oversized files are listed but cannot be fetched.
        assert_eq!(built.assets.len(), 2);
        assert_eq!(built.assets[0].asset_id, files[0].asset_id);
    }

    fn confirm_request() -> ConfirmRequest {
        let choice = |id: &str, role, variant| ConfirmChoice {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            role,
            variant,
        };
        let variant = |group: &str, level, recommended| {
            Some(ChoiceVariant {
                group: group.into(),
                level,
                level_label: String::new(),
                segment_label: String::new(),
                recommended,
            })
        };
        ConfirmRequest {
            id: "c1".into(),
            title: "Permission".into(),
            context: vec![ConfirmField {
                id: "tool".into(),
                label: "Tool".into(),
                value: "shell".into(),
                kind: ConfirmFieldKind::Path,
            }],
            detail: ConfirmDetail {
                summary: "Run it".into(),
                body_md: "```sh\nls\n```".into(),
            },
            choices: vec![
                choice("approve", ActionRole::Primary, None),
                choice("prefix0", ActionRole::Default, variant("p", 0, false)),
                choice("prefix1", ActionRole::Default, variant("p", 1, true)),
                choice("deny", ActionRole::Destructive, None),
            ],
            presentation: ConfirmPresentation::SingleSelectSubmit {
                input: Some(ConfirmInput {
                    id: "note".into(),
                    visible_when_action_id: "deny".into(),
                    always_visible: false,
                    required: true,
                    prefix_chars_by_action_id: Default::default(),
                    label: "Why".into(),
                    placeholder: "reason".into(),
                    max_chars: 300,
                }),
                submit_label: "Send".into(),
                default_action_id: Some("approve".into()),
            },
            dismiss_action_id: "deny".into(),
            created_at_ms: 1_000,
            expires_at_ms: 61_000,
        }
    }

    #[test]
    fn confirm_view_offers_only_recommended_variants_and_keeps_wire_indices() {
        let view = confirm_view(&confirm_request(), &origin());
        assert_eq!(view.kind, "confirm");
        assert_eq!(view.created_at, 1_000);
        let confirm = view.confirm.unwrap();
        let wire_indices: Vec<usize> = confirm.choices.iter().map(|c| c.wire_index).collect();
        assert_eq!(
            wire_indices,
            [0, 2, 3],
            "non-recommended variant 1 is hidden"
        );
        assert_eq!(confirm.choices[2].role, "destructive");
        assert_eq!(confirm.default_index, Some(0));
        assert_eq!(confirm.dismiss_index, Some(3));
        assert_eq!(confirm.expires_at, 61_000);
        assert_eq!(confirm.submit_label, "Send");
        assert_eq!(confirm.context[0].kind, "path");
        let input = confirm.input.unwrap();
        assert_eq!(input.visible_when_index, Some(3));
        assert!(input.required);
        assert_eq!(input.max_chars, 300);
    }

    #[test]
    fn a_hidden_default_choice_is_not_offered_as_the_default() {
        let mut request = confirm_request();
        let ConfirmPresentation::SingleSelectSubmit {
            default_action_id, ..
        } = &mut request.presentation;
        *default_action_id = Some("prefix0".into());
        let confirm = confirm_view(&request, &origin()).confirm.unwrap();
        assert_eq!(confirm.default_index, None);
    }

    #[test]
    fn mime_types_follow_the_extension() {
        assert_eq!(mime_for("a.PNG"), "image/png");
        assert_eq!(mime_for("report.pdf"), "application/pdf");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }
}
