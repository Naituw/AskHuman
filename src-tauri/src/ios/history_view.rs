//! What the phone sees of one recorded exchange (docs/specs/ios-session-history.md §5.3).
//!
//! A pure projection of a [`HistoryEntry`]: it keeps the Agent's message, the questions and what was
//! chosen or written to them, cuts every text to the limits of the wire, and turns everything that
//! lives on disk (attachments, saved images, reply files, notes) into a count. No path ever goes on
//! the wire (the same stance as the todos, docs/specs/ios-todos.md I6).
//!
//! The one exception to "a count" is the images (docs/specs/ios-history-pictures.md): [`with_pictures`]
//! adds a reference to each of them, with a name, a size, a thumbnail and an id that is a hash of where
//! the image sits in the exchange ([`picture_id`]) — never its path. What the Mac finds out about a file
//! goes through [`PictureProbe`], so the projection stays free of disk access.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::crypto;
use super::views::mime_for;
use super::wire::{
    AssetRef, SessionExchange, SessionQuestion, HISTORY_INPUT_MAX_CHARS, HISTORY_MESSAGE_MAX_CHARS,
    HISTORY_OPTION_MAX_CHARS, HISTORY_PICTURES_MAX, HISTORY_QUESTION_MAX_CHARS, MAX_ASSET_BYTES,
};
use crate::history::{HistoryAnswer, HistoryEntry};

/// Cuts `text` to at most `max` characters; a cut text ends in `…` (which counts toward `max`).
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn question_of(text: &str, answer: Option<&HistoryAnswer>) -> SessionQuestion {
    let mut question = SessionQuestion {
        text: clip(text, HISTORY_QUESTION_MAX_CHARS),
        selected: Vec::new(),
        input: None,
        images: 0,
        files: 0,
        pictures: Vec::new(),
    };
    if let Some(answer) = answer {
        question.selected = answer
            .selected_options
            .iter()
            .map(|option| clip(option, HISTORY_OPTION_MAX_CHARS))
            .collect();
        question.input = answer
            .user_input
            .as_deref()
            .filter(|input| !input.trim().is_empty())
            .map(|input| clip(input, HISTORY_INPUT_MAX_CHARS));
        question.images = answer.images.len() as u64;
        question.files = answer.files.len() as u64;
    }
    question
}

/// The exchange a phone shows for one history entry. Question `i` gets `answers[i]`; an automatic reply
/// records a single answer, which therefore lands on the first question.
pub fn exchange_of(entry: &HistoryEntry) -> SessionExchange {
    SessionExchange {
        id: entry.id.clone(),
        at: entry.timestamp_ms,
        channel: entry.channel.clone(),
        markdown: entry.is_markdown,
        message: clip(&entry.message.text, HISTORY_MESSAGE_MAX_CHARS),
        attachments: entry.message.files.len() as u64,
        pictures: Vec::new(),
        qs: entry
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| question_of(&question.message, entry.answers.get(index)))
            .collect(),
        notes: entry.annotations.len() as u64,
    }
}

// ---------------------------------------------------------------- pictures

/// Where a picture sits in an exchange. The slot, with the id of the exchange, is all that makes the id
/// of the picture, so it must never change for a recorded entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureSlot {
    /// The `index`-th file of the Agent's message (counting every file, not only the images).
    Message(usize),
    /// The `index`-th image of the answer to question number `question`.
    Reply { question: usize, index: usize },
}

impl PictureSlot {
    fn label(self) -> String {
        match self {
            Self::Message(index) => format!("m{index}"),
            Self::Reply { question, index } => format!("q{question}.{index}"),
        }
    }
}

/// The id a phone asks a picture for with `asset.get` (docs/specs/ios-history-pictures.md HP7): the 26
/// base32 characters every asset id has, here the first 16 bytes of a hash of the exchange and the slot.
/// The same exchange and slot always give the same id, so a thumbnail the phone already shows still
/// opens after the Mac restarted; and nothing about the path is in it.
pub fn picture_id(exchange_id: &str, slot: PictureSlot) -> String {
    let digest = Sha256::digest(
        format!("askhuman.history.picture\0{exchange_id}\0{}", slot.label()).as_bytes(),
    );
    crypto::base32_lower(&digest[..16])
}

/// One picture an entry holds: where it sits, where the file is and what the phone calls it.
struct Held<'a> {
    slot: PictureSlot,
    path: &'a str,
    name: String,
}

/// The name of a file on disk, for a picture that was recorded as a path only.
fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "image".to_string())
}

/// The images among the files of the Agent's message, in the order they were attached, at most
/// `HISTORY_PICTURES_MAX`. This is the only place that decides which files are pictures, for both the
/// projection and the lookup by id.
fn message_pictures(entry: &HistoryEntry) -> Vec<Held<'_>> {
    entry
        .message
        .files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.is_image)
        .take(HISTORY_PICTURES_MAX)
        .map(|(index, file)| Held {
            slot: PictureSlot::Message(index),
            path: &file.path,
            name: file.name.clone(),
        })
        .collect()
}

/// The images the user attached to the answer of question `question`, at most `HISTORY_PICTURES_MAX`.
/// Question `i` has `answers[i]`, as in [`exchange_of`].
fn reply_pictures(entry: &HistoryEntry, question: usize) -> Vec<Held<'_>> {
    let Some(answer) = entry.answers.get(question) else {
        return Vec::new();
    };
    answer
        .images
        .iter()
        .enumerate()
        .take(HISTORY_PICTURES_MAX)
        .map(|(index, path)| Held {
            slot: PictureSlot::Reply { question, index },
            path,
            name: file_name(path),
        })
        .collect()
}

/// What the Mac finds out about a picture file when it describes the picture to a phone.
pub trait PictureProbe {
    /// The size of the file in bytes, or `None` when it is not there any more (or is not a regular file).
    fn size(&self, path: &Path) -> Option<u64>;
    /// A small JPEG of it, or `None` when it cannot be decoded. Only asked for files within the limit of
    /// what a phone may download.
    fn thumbnail(&self, path: &Path) -> Option<Vec<u8>>;
}

/// The reference to one picture, as far as the probe can tell.
fn picture_ref(entry: &HistoryEntry, held: &Held<'_>, probe: &dyn PictureProbe) -> AssetRef {
    let path = Path::new(held.path);
    let mut reference = AssetRef {
        asset_id: picture_id(&entry.id, held.slot),
        name: held.name.clone(),
        size: 0,
        mime: mime_for(&held.name).to_string(),
        is_image: true,
        thumb: None,
        too_large: false,
        gone: false,
    };
    match probe.size(path) {
        None => reference.gone = true,
        Some(size) => {
            reference.size = size;
            reference.too_large = size > MAX_ASSET_BYTES;
            if !reference.too_large {
                reference.thumb = probe.thumbnail(path).map(|bytes| crypto::b64u(&bytes));
            }
        }
    }
    reference
}

/// [`exchange_of`] with the pictures: a reference to each image of the message and of every answer
/// (docs/specs/ios-history-pictures.md §5). The counts stay what they were, so a phone can tell how many
/// files and images there are besides the pictures it shows.
pub fn exchange_with_pictures(entry: &HistoryEntry, probe: &dyn PictureProbe) -> SessionExchange {
    let mut exchange = exchange_of(entry);
    exchange.pictures = message_pictures(entry)
        .iter()
        .map(|held| picture_ref(entry, held, probe))
        .collect();
    for (question, projected) in exchange.qs.iter_mut().enumerate() {
        projected.pictures = reply_pictures(entry, question)
            .iter()
            .map(|held| picture_ref(entry, held, probe))
            .collect();
    }
    exchange
}

/// Where a picture's file is and what it is called: what the Mac needs to serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureLocation {
    pub path: PathBuf,
    pub name: String,
}

/// The picture of one of `entries` that [`picture_id`] gave `asset_id`, if there is one. It only finds
/// what [`exchange_with_pictures`] hands out, so an id that was never given out is not found.
pub fn find_picture<'a>(
    entries: impl IntoIterator<Item = &'a HistoryEntry>,
    asset_id: &str,
) -> Option<PictureLocation> {
    for entry in entries {
        let replies =
            (0..entry.questions.len()).flat_map(|question| reply_pictures(entry, question));
        for held in message_pictures(entry).into_iter().chain(replies) {
            if picture_id(&entry.id, held.slot) == asset_id {
                return Some(PictureLocation {
                    path: PathBuf::from(held.path),
                    name: held.name,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::UserAnnotation;
    use crate::models::{ChannelAction, FileAttachment, MessagePrompt, OptionItem, Question};

    fn entry() -> HistoryEntry {
        HistoryEntry {
            id: "req-1".into(),
            timestamp_ms: 1_700_000_000_123,
            project: "/work/app".into(),
            source: "Codex".into(),
            agent_kind: Some("codex".into()),
            agent_session_id: Some("s1".into()),
            mcp_instance_id: None,
            channel: "popup".into(),
            action: ChannelAction::Send,
            is_markdown: true,
            message: MessagePrompt::new("Plan ready".into(), Vec::new()),
            questions: vec![
                Question::new(
                    "Which database?".into(),
                    vec![
                        OptionItem::new("Postgres", true),
                        OptionItem::new("SQLite", false),
                    ],
                ),
                Question::new("Anything else?".into(), Vec::new()),
            ],
            answers: vec![
                HistoryAnswer {
                    selected_options: vec!["Postgres".into()],
                    ..Default::default()
                },
                HistoryAnswer {
                    user_input: Some("Keep the migration reversible".into()),
                    ..Default::default()
                },
            ],
            annotations: Vec::new(),
        }
    }

    #[test]
    fn an_exchange_keeps_the_questions_and_only_what_was_chosen_or_written() {
        let exchange = exchange_of(&entry());
        assert_eq!(exchange.id, "req-1");
        assert_eq!(exchange.at, 1_700_000_000_123);
        assert_eq!(exchange.channel, "popup");
        assert!(exchange.markdown);
        assert_eq!(exchange.message, "Plan ready");
        assert_eq!(exchange.qs.len(), 2);
        assert_eq!(exchange.qs[0].text, "Which database?");
        // The option that was not chosen does not travel.
        assert_eq!(exchange.qs[0].selected, ["Postgres"]);
        assert_eq!(exchange.qs[0].input, None);
        assert_eq!(exchange.qs[1].selected, Vec::<String>::new());
        assert_eq!(
            exchange.qs[1].input.as_deref(),
            Some("Keep the migration reversible")
        );
        assert_eq!((exchange.attachments, exchange.notes), (0, 0));
    }

    #[test]
    fn files_images_and_notes_become_counts_and_no_path_travels() {
        let mut recorded = entry();
        recorded.message.files = vec![FileAttachment {
            path: "/Users/me/secret/plan.pdf".into(),
            name: "plan.pdf".into(),
            size: 10,
            is_image: false,
        }];
        recorded.answers[0].images = vec!["/tmp/a.png".into(), "/tmp/b.png".into()];
        recorded.answers[0].files = vec!["/Users/me/notes.txt".into()];
        recorded.annotations = vec![UserAnnotation {
            quote: "a passage".into(),
            note: "why?".into(),
        }];

        let exchange = exchange_of(&recorded);
        assert_eq!(exchange.attachments, 1);
        assert_eq!((exchange.qs[0].images, exchange.qs[0].files), (2, 1));
        assert_eq!(exchange.notes, 1);

        let json = serde_json::to_string(&exchange).unwrap();
        for leaked in [
            "/Users",
            "/tmp",
            "plan.pdf",
            "notes.txt",
            "a passage",
            "why?",
        ] {
            assert!(!json.contains(leaked), "{leaked} is on the wire");
        }
    }

    #[test]
    fn long_texts_are_cut_to_the_limits_and_end_in_an_ellipsis() {
        let mut recorded = entry();
        recorded.message.text = "m".repeat(HISTORY_MESSAGE_MAX_CHARS + 50);
        recorded.questions[0].message = "q".repeat(HISTORY_QUESTION_MAX_CHARS + 1);
        recorded.answers[0].selected_options = vec!["o".repeat(HISTORY_OPTION_MAX_CHARS * 2)];
        recorded.answers[1].user_input = Some("语".repeat(HISTORY_INPUT_MAX_CHARS + 7));

        let exchange = exchange_of(&recorded);
        let count = |text: &str| text.chars().count();
        assert_eq!(count(&exchange.message), HISTORY_MESSAGE_MAX_CHARS);
        assert!(exchange.message.ends_with('…'));
        assert_eq!(count(&exchange.qs[0].text), HISTORY_QUESTION_MAX_CHARS);
        assert!(exchange.qs[0].text.ends_with('…'));
        assert_eq!(count(&exchange.qs[0].selected[0]), HISTORY_OPTION_MAX_CHARS);
        let input = exchange.qs[1].input.as_deref().unwrap();
        assert_eq!(count(input), HISTORY_INPUT_MAX_CHARS);
        assert!(input.ends_with('…'));

        // Texts at the limit are left alone.
        recorded.message.text = "m".repeat(HISTORY_MESSAGE_MAX_CHARS);
        let exact = exchange_of(&recorded);
        assert_eq!(count(&exact.message), HISTORY_MESSAGE_MAX_CHARS);
        assert!(!exact.message.ends_with('…'));
    }

    #[test]
    fn an_automatic_reply_answers_the_first_question_only() {
        let mut recorded = entry();
        recorded.channel = "away".into();
        recorded.answers = vec![HistoryAnswer {
            user_input: Some("Decide for me, I am away.".into()),
            ..Default::default()
        }];
        let exchange = exchange_of(&recorded);
        assert_eq!(exchange.channel, "away");
        assert_eq!(
            exchange.qs[0].input.as_deref(),
            Some("Decide for me, I am away.")
        );
        assert_eq!(exchange.qs[1].input, None);
        assert!(exchange.qs[1].selected.is_empty());
    }

    #[test]
    fn a_blank_answer_and_a_missing_answer_read_as_not_answered() {
        let mut recorded = entry();
        recorded.answers = vec![HistoryAnswer {
            user_input: Some("  \n ".into()),
            ..Default::default()
        }];
        let exchange = exchange_of(&recorded);
        assert_eq!(exchange.qs[0].input, None);
        assert_eq!(exchange.qs[1].input, None);

        // More answers than questions cannot index past the questions.
        let mut extra = entry();
        extra.answers.push(HistoryAnswer {
            user_input: Some("stray".into()),
            ..Default::default()
        });
        assert_eq!(exchange_of(&extra).qs.len(), 2);
    }

    #[test]
    fn a_message_less_ask_has_an_empty_message_and_its_question() {
        let mut recorded = entry();
        recorded.message.text = String::new();
        recorded.questions.truncate(1);
        let exchange = exchange_of(&recorded);
        assert_eq!(exchange.message, "");
        assert_eq!(exchange.qs.len(), 1);
    }

    // ------------------------------------------------------------ pictures

    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A probe that knows a few files by path: their size and, for some, a thumbnail. It records the
    /// files it was asked a thumbnail of.
    #[derive(Default)]
    struct Probe {
        sizes: HashMap<String, u64>,
        thumbs: HashMap<String, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl Probe {
        fn with(mut self, path: &str, size: u64, thumb: Option<&[u8]>) -> Self {
            self.sizes.insert(path.into(), size);
            if let Some(bytes) = thumb {
                self.thumbs.insert(path.into(), bytes.to_vec());
            }
            self
        }
    }

    impl PictureProbe for Probe {
        fn size(&self, path: &Path) -> Option<u64> {
            self.sizes.get(path.to_str().unwrap()).copied()
        }
        fn thumbnail(&self, path: &Path) -> Option<Vec<u8>> {
            let key = path.to_str().unwrap().to_string();
            self.asked.borrow_mut().push(key.clone());
            self.thumbs.get(&key).cloned()
        }
    }

    fn attachment(path: &str, name: &str, is_image: bool) -> crate::models::FileAttachment {
        crate::models::FileAttachment {
            path: path.into(),
            name: name.into(),
            size: 1,
            is_image,
        }
    }

    /// An entry with pictures: two images and a PDF on the message (the PDF in the middle), two reply
    /// images on the first question, none on the second.
    fn entry_with_pictures() -> HistoryEntry {
        let mut recorded = entry();
        recorded.message.files = vec![
            attachment("/work/a.png", "a.png", true),
            attachment("/work/plan.pdf", "plan.pdf", false),
            attachment("/work/b.jpg", "b.jpg", true),
        ];
        recorded.answers[0].images =
            vec!["/tmp/askhuman/r/q0/one.png".into(), "/tmp/two.heic".into()];
        recorded
    }

    #[test]
    fn a_picture_id_is_a_stable_asset_id_that_differs_by_exchange_and_slot() {
        // Pinned: the phone keeps showing thumbnails across a restart of the Mac, so the ids may never change.
        assert_eq!(
            picture_id("req-1", PictureSlot::Message(0)),
            "bymlf6obzcgrlotgyepjdpjlxi"
        );
        assert_eq!(
            picture_id("req-1", PictureSlot::Message(3)),
            "hinmutsyspw5lssot23dm3holu"
        );
        assert_eq!(
            picture_id(
                "req-1",
                PictureSlot::Reply {
                    question: 1,
                    index: 2
                }
            ),
            "bldf7zxddersvmitjazfe2rvo4"
        );
        assert_eq!(
            picture_id("other", PictureSlot::Message(0)),
            "qx6rerw4nctgc3tt2u67hsr5au"
        );
        for id in [
            picture_id("req-1", PictureSlot::Message(0)),
            picture_id(
                "x",
                PictureSlot::Reply {
                    question: 0,
                    index: 0,
                },
            ),
        ] {
            assert!(crypto::is_valid_id(&id), "{id}");
        }
        // The slots do not collide: message file 1 and the first reply image are different pictures.
        assert_ne!(
            picture_id("req-1", PictureSlot::Message(1)),
            picture_id(
                "req-1",
                PictureSlot::Reply {
                    question: 0,
                    index: 1
                }
            )
        );
    }

    #[test]
    fn the_images_of_the_message_and_of_the_answers_become_references_in_order() {
        let probe = Probe::default()
            .with("/work/a.png", 344_000, Some(b"thumb-a"))
            .with("/work/b.jpg", 20_000, None)
            .with("/tmp/askhuman/r/q0/one.png", 9_000, Some(b"thumb-one"))
            .with("/tmp/two.heic", 5_000, None);
        let recorded = entry_with_pictures();
        let exchange = exchange_with_pictures(&recorded, &probe);

        // The counts are what exchange_of says: three files, two reply images.
        assert_eq!(exchange.attachments, 3);
        assert_eq!(exchange.qs[0].images, 2);

        let names: Vec<_> = exchange.pictures.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a.png", "b.jpg"], "the PDF is only counted");
        let first = &exchange.pictures[0];
        assert_eq!(first.asset_id, picture_id("req-1", PictureSlot::Message(0)));
        assert_eq!(
            exchange.pictures[1].asset_id,
            picture_id("req-1", PictureSlot::Message(2)),
            "the slot counts every file, not only the images"
        );
        assert_eq!((first.size, first.mime.as_str()), (344_000, "image/png"));
        assert!(first.is_image && !first.too_large && !first.gone);
        assert_eq!(
            crypto::b64u_decode(first.thumb.as_deref().unwrap()).unwrap(),
            b"thumb-a"
        );
        assert!(
            exchange.pictures[1].thumb.is_none(),
            "an undecodable picture has none"
        );

        // The reply images come with the name of the file, on the question they were attached to.
        let replies: Vec<_> = exchange.qs[0]
            .pictures
            .iter()
            .map(|p| (p.name.as_str(), p.mime.as_str()))
            .collect();
        assert_eq!(
            replies,
            [("one.png", "image/png"), ("two.heic", "image/heic")]
        );
        assert_eq!(
            exchange.qs[0].pictures[1].asset_id,
            picture_id(
                "req-1",
                PictureSlot::Reply {
                    question: 0,
                    index: 1
                }
            )
        );
        assert!(exchange.qs[1].pictures.is_empty());
    }

    #[test]
    fn a_picture_whose_file_is_gone_is_marked_and_a_huge_one_has_no_thumbnail() {
        let mut recorded = entry_with_pictures();
        recorded.message.files = vec![
            attachment("/work/gone.png", "gone.png", true),
            attachment("/work/huge.png", "huge.png", true),
            attachment("/work/ok.png", "ok.png", true),
        ];
        let probe = Probe::default()
            .with("/work/huge.png", MAX_ASSET_BYTES + 1, Some(b"never"))
            .with("/work/ok.png", MAX_ASSET_BYTES, Some(b"ok"));
        let exchange = exchange_with_pictures(&recorded, &probe);

        let gone = &exchange.pictures[0];
        assert!(gone.gone);
        assert_eq!((gone.size, gone.thumb.as_deref()), (0, None));
        assert_eq!(gone.name, "gone.png", "a missing file keeps its name");
        assert_eq!(gone.mime, "image/png");

        let huge = &exchange.pictures[1];
        assert!(huge.too_large && !huge.gone);
        assert_eq!(huge.size, MAX_ASSET_BYTES + 1);
        assert!(huge.thumb.is_none());
        assert!(
            !probe.asked.borrow().contains(&"/work/huge.png".to_string()),
            "a file over the limit is not decoded"
        );

        // Exactly at the limit is still fetchable.
        let ok = &exchange.pictures[2];
        assert!(!ok.too_large && ok.thumb.is_some());
    }

    #[test]
    fn at_most_twelve_pictures_are_described_for_the_message_and_for_each_answer() {
        let mut recorded = entry();
        recorded.message.files = (0..15)
            .map(|i| attachment(&format!("/w/{i}.png"), &format!("{i}.png"), true))
            .collect();
        recorded.answers[1].images = (0..14).map(|i| format!("/r/{i}.png")).collect();
        let exchange = exchange_with_pictures(&recorded, &Probe::default());
        assert_eq!(exchange.pictures.len(), HISTORY_PICTURES_MAX);
        assert_eq!(exchange.pictures[11].name, "11.png");
        assert_eq!(exchange.attachments, 15, "the count still says all of them");
        assert_eq!(exchange.qs[1].pictures.len(), HISTORY_PICTURES_MAX);
        assert_eq!(exchange.qs[1].images, 14);
        // The ones left out cannot be asked for either.
        assert!(
            find_picture([&recorded], &picture_id("req-1", PictureSlot::Message(12))).is_none()
        );
        assert!(
            find_picture([&recorded], &picture_id("req-1", PictureSlot::Message(11))).is_some()
        );
    }

    #[test]
    fn the_names_travel_but_no_path_does() {
        let probe = Probe::default()
            .with("/Users/me/secret/a.png", 10, Some(b"t"))
            .with("/private/tmp/askhuman/r/q0/pasted.png", 10, None);
        let mut recorded = entry();
        recorded.message.files = vec![attachment("/Users/me/secret/a.png", "a.png", true)];
        recorded.answers[0].images = vec!["/private/tmp/askhuman/r/q0/pasted.png".into()];
        let json = serde_json::to_string(&exchange_with_pictures(&recorded, &probe)).unwrap();
        assert!(json.contains("a.png") && json.contains("pasted.png"));
        for leaked in ["/Users", "secret", "/private", "/tmp", "askhuman"] {
            assert!(!json.contains(leaked), "{leaked} is on the wire");
        }
    }

    #[test]
    fn an_exchange_without_images_gets_no_pictures_and_asks_the_probe_nothing() {
        let probe = Probe::default();
        let mut recorded = entry();
        recorded.message.files = vec![attachment("/w/plan.pdf", "plan.pdf", false)];
        recorded.answers[0].files = vec!["/w/notes.txt".into()];
        let exchange = exchange_with_pictures(&recorded, &probe);
        assert!(exchange.pictures.is_empty());
        assert!(exchange.qs.iter().all(|q| q.pictures.is_empty()));
        assert!(probe.asked.borrow().is_empty());
        assert_eq!(exchange.attachments, 1);
        // Without the probe's help the projection is exactly the plain one.
        assert_eq!(exchange, exchange_of(&recorded));
    }

    #[test]
    fn an_automatic_reply_puts_its_images_on_the_first_question_only() {
        let mut recorded = entry();
        recorded.channel = "away".into();
        recorded.answers = vec![HistoryAnswer {
            images: vec!["/tmp/auto.png".into()],
            ..Default::default()
        }];
        let exchange =
            exchange_with_pictures(&recorded, &Probe::default().with("/tmp/auto.png", 5, None));
        assert_eq!(exchange.qs[0].pictures.len(), 1);
        assert!(exchange.qs[1].pictures.is_empty());
        // And a cancelled entry, without answers, has none.
        recorded.answers.clear();
        assert!(exchange_with_pictures(&recorded, &Probe::default())
            .qs
            .iter()
            .all(|q| q.pictures.is_empty()));
    }

    #[test]
    fn a_picture_is_found_by_the_id_that_was_handed_out_and_by_nothing_else() {
        let first = entry_with_pictures();
        let mut second = entry_with_pictures();
        second.id = "req-2".into();
        second.message.files = vec![attachment("/other/c.png", "c.png", true)];
        second.answers.clear();
        let entries = [&first, &second];

        let found = |id: &str| find_picture(entries, id);
        assert_eq!(
            found(&picture_id("req-1", PictureSlot::Message(2))),
            Some(PictureLocation {
                path: "/work/b.jpg".into(),
                name: "b.jpg".into()
            })
        );
        assert_eq!(
            found(&picture_id(
                "req-1",
                PictureSlot::Reply {
                    question: 0,
                    index: 0
                }
            )),
            Some(PictureLocation {
                path: "/tmp/askhuman/r/q0/one.png".into(),
                name: "one.png".into()
            })
        );
        assert_eq!(
            found(&picture_id("req-2", PictureSlot::Message(0))).map(|p| p.name),
            Some("c.png".into())
        );
        // The PDF is not a picture, so its slot has no id; neither has a slot nobody holds, nor a stranger.
        assert_eq!(found(&picture_id("req-1", PictureSlot::Message(1))), None);
        assert_eq!(found(&picture_id("req-2", PictureSlot::Message(5))), None);
        assert_eq!(found(&picture_id("req-3", PictureSlot::Message(0))), None);
        assert_eq!(found("not-an-id"), None);
        assert_eq!(found(""), None);
        // Every reference the projection hands out is found again.
        let probe = Probe::default();
        let exchange = exchange_with_pictures(&first, &probe);
        for picture in exchange
            .pictures
            .iter()
            .chain(exchange.qs.iter().flat_map(|q| q.pictures.iter()))
        {
            assert!(found(&picture.asset_id).is_some(), "{}", picture.name);
        }
    }
}
