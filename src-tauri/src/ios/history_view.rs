//! What the phone sees of one recorded exchange (docs/specs/ios-session-history.md §5.3).
//!
//! A pure projection of a [`HistoryEntry`]: it keeps the Agent's message, the questions and what was
//! chosen or written to them, cuts every text to the limits of the wire, and turns everything that
//! lives on disk (attachments, saved images, reply files, notes) into a count. No path ever goes on
//! the wire (the same stance as the todos, docs/specs/ios-todos.md I6).

use super::wire::{
    SessionExchange, SessionQuestion, HISTORY_INPUT_MAX_CHARS, HISTORY_MESSAGE_MAX_CHARS,
    HISTORY_OPTION_MAX_CHARS, HISTORY_QUESTION_MAX_CHARS,
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
        qs: entry
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| question_of(&question.message, entry.answers.get(index)))
            .collect(),
        notes: entry.annotations.len() as u64,
    }
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
}
