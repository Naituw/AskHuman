//! Exact recovery of the latest completed AskHuman exchange after Agent context compaction.

use crate::history::{HistoryAnswer, HistoryEntry};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const MESSAGE_FILE_THRESHOLD_BYTES: usize = 8 * 1024;
pub const MESSAGE_STDOUT_PREFIX_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone)]
pub enum Scope {
    AgentSession {
        agent_kind: String,
        session_id: String,
    },
    McpInstance {
        mcp_instance_id: String,
        project: String,
    },
    Project(String),
}

impl Scope {
    fn storage_key(&self) -> String {
        match self {
            Scope::AgentSession {
                agent_kind,
                session_id,
            } => format!("session:{agent_kind}:{session_id}"),
            Scope::McpInstance {
                mcp_instance_id,
                project,
            } => format!("mcp:{mcp_instance_id}:{project}"),
            Scope::Project(project) => format!("project:{project}"),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    HistoryDisabled,
    NotFound,
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::HistoryDisabled => write!(f, "AskHuman history is disabled"),
            Error::NotFound => write!(f, "No completed AskHuman exchange was found for this scope"),
            Error::Io(error) => write!(
                f,
                "Failed to prepare the recovered AskHuman exchange: {error}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Query and format the latest completed exchange. Exact session scopes never cascade to weaker
/// partitions; callers must select the one authoritative scope they possess.
pub fn recover(scope: &Scope) -> Result<String, Error> {
    let history_limit = crate::config::AppConfig::load_without_secrets()
        .general
        .history_limit;
    recover_with(
        scope,
        history_limit,
        |scope| match scope {
            Scope::AgentSession {
                agent_kind,
                session_id,
            } => crate::history::latest_send_for_session(agent_kind, session_id),
            Scope::McpInstance {
                mcp_instance_id,
                project,
            } => crate::history::latest_send_for_mcp_instance(mcp_instance_id, project),
            Scope::Project(project) => crate::history::latest_send_for_project(project),
        },
        &crate::paths::show_last_dir(),
    )
}

fn recover_with(
    scope: &Scope,
    history_limit: u32,
    lookup: impl FnOnce(&Scope) -> Option<HistoryEntry>,
    storage_dir: &std::path::Path,
) -> Result<String, Error> {
    if history_limit == 0 {
        return Err(Error::HistoryDisabled);
    }
    let entry = lookup(scope).ok_or(Error::NotFound)?;
    render_at(&entry, &scope.storage_key(), storage_dir).map_err(Error::Io)
}

fn render(entry: &HistoryEntry, storage_key: &str) -> std::io::Result<String> {
    render_at(entry, storage_key, &crate::paths::show_last_dir())
}

fn render_at(
    entry: &HistoryEntry,
    storage_key: &str,
    storage_dir: &std::path::Path,
) -> std::io::Result<String> {
    let mut blocks = Vec::new();
    let mut message_sections = Vec::new();

    if entry.message.text.len() > MESSAGE_FILE_THRESHOLD_BYTES {
        let path = write_full_message_at(storage_dir, storage_key, &entry.message.text)?;
        message_sections.push(format!(
            "[message_truncated]\n{}",
            utf8_prefix(&entry.message.text, MESSAGE_STDOUT_PREFIX_BYTES)
        ));
        message_sections.push(format!("[message_full_file]\n{}", path.display()));
    } else if !entry.message.text.is_empty() {
        message_sections.push(format!("[message]\n{}", entry.message.text));
    }

    if !entry.message.files.is_empty() {
        message_sections.push(format!(
            "[message_files]\n{}",
            entry
                .message
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !message_sections.is_empty() {
        blocks.push(message_sections.join("\n\n"));
    }

    for (index, question) in entry.questions.iter().enumerate() {
        let mut question_sections = vec![format!("[question]\n{}", question.message)];
        question_sections.extend(render_answer(entry.answers.get(index)));
        blocks.push(question_sections.join("\n\n"));
    }

    Ok(blocks.join("\n\n---\n\n"))
}

fn render_answer(answer: Option<&HistoryAnswer>) -> Vec<String> {
    let Some(answer) = answer else {
        return vec!["[answer_status]\nunanswered".into()];
    };
    let mut sections = Vec::new();

    if !answer.selected_options.is_empty() {
        sections.push(format!(
            "[answer_selected_options]\n{}",
            answer.selected_options.join(", ")
        ));
    }

    if let Some(input) = answer
        .user_input
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!("[answer_user_input]\n{input}"));
    }

    if !answer.images.is_empty() || !answer.files.is_empty() {
        sections.push(format!(
            "[answer_files]\n{}",
            answer
                .images
                .iter()
                .chain(&answer.files)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if sections.is_empty() {
        sections.push("[answer_status]\nunanswered".into());
    }
    sections
}

fn write_full_message_at(
    storage_dir: &std::path::Path,
    storage_key: &str,
    message: &str,
) -> std::io::Result<PathBuf> {
    let digest = Sha256::digest(storage_key.as_bytes());
    let filename = format!("{:x}.md", digest);
    let path = storage_dir.join(filename);
    crate::integrations::hook_edit::atomic_write_private(&path, message.as_bytes())
        .map_err(std::io::Error::other)?;
    Ok(path)
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChannelAction, FileAttachment, MessagePrompt, OptionItem, Question};

    fn sample(message: &str) -> HistoryEntry {
        HistoryEntry {
            id: "id".into(),
            timestamp_ms: 1,
            project: "/p".into(),
            source: "Codex".into(),
            agent_kind: Some("codex".into()),
            agent_session_id: Some("s".into()),
            mcp_instance_id: Some("m".into()),
            channel: "popup".into(),
            action: ChannelAction::Send,
            is_markdown: true,
            message: MessagePrompt::new(message.into(), Vec::new()),
            questions: vec![Question::new(
                "Full question".into(),
                vec![OptionItem::new("Yes", true), OptionItem::new("No", false)],
            )],
            answers: vec![HistoryAnswer {
                selected_options: vec!["Yes".into()],
                user_input: Some("details".into()),
                images: vec!["/tmp/image.png".into()],
                files: vec!["/tmp/file.txt".into()],
            }],
        }
    }

    #[test]
    fn renders_only_non_empty_context_question_and_actual_answer_fields() {
        let output = render(&sample("Context"), "scope").unwrap();
        assert_eq!(
            output,
            "[message]\nContext\n\n---\n\n[question]\nFull question\n\n\
             [answer_selected_options]\nYes\n\n[answer_user_input]\ndetails\n\n\
             [answer_files]\n/tmp/image.png\n/tmp/file.txt"
        );
        assert!(!output.contains("No"));
        assert!(!output.contains("recommended"));
        assert!(!output.contains("askhuman_last_exchange"));
    }

    #[test]
    fn renders_empty_message_files_multiple_questions_and_unanswered_states() {
        let mut entry = sample("");
        entry.message.files = vec![FileAttachment {
            path: "/tmp/context.pdf".into(),
            name: "context.pdf".into(),
            size: 42,
            is_image: false,
        }];
        entry.questions.push(Question::new(
            "Second question".into(),
            vec![OptionItem::new("A", false), OptionItem::new("B", true)],
        ));
        entry.answers[0].selected_options = vec!["Yes".into(), "No".into()];
        entry.answers[0].user_input = Some("  details  ".into());
        // Missing answer entries and present-but-empty answers both render explicitly unanswered.
        let output = render_at(&entry, "scope", tempfile::tempdir().unwrap().path()).unwrap();
        assert!(output.starts_with("[message_files]\n/tmp/context.pdf\n\n---\n\n"));
        assert!(output.contains("[answer_selected_options]\nYes, No"));
        assert!(output.contains("[answer_user_input]\ndetails"));
        assert!(output.contains("\n\n---\n\n[question]\nSecond question"));
        assert!(output.ends_with("[answer_status]\nunanswered"));
        assert!(!output.contains("[message]\n"));
        assert!(!output.contains("recommended"));

        entry.answers.push(HistoryAnswer {
            selected_options: Vec::new(),
            user_input: Some("  ".into()),
            images: Vec::new(),
            files: Vec::new(),
        });
        let output = render_at(&entry, "scope", tempfile::tempdir().unwrap().path()).unwrap();
        assert!(output.ends_with("[answer_status]\nunanswered"));
    }

    #[test]
    fn recovery_errors_are_explicit_and_history_disabled_skips_lookup() {
        let scope = Scope::Project("/p".into());
        let looked_up = std::cell::Cell::new(false);
        let dir = tempfile::tempdir().unwrap();
        let disabled = recover_with(
            &scope,
            0,
            |_| {
                looked_up.set(true);
                Some(sample("ignored"))
            },
            dir.path(),
        );
        assert!(matches!(disabled, Err(Error::HistoryDisabled)));
        assert!(!looked_up.get());
        assert!(matches!(
            recover_with(&scope, 200, |_| None, dir.path()),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn scope_storage_keys_are_partitioned_and_stable() {
        let session = Scope::AgentSession {
            agent_kind: "codex".into(),
            session_id: "same".into(),
        };
        let mcp = Scope::McpInstance {
            mcp_instance_id: "same".into(),
            project: "/p".into(),
        };
        let project = Scope::Project("/p".into());
        assert_eq!(session.storage_key(), "session:codex:same");
        assert_eq!(mcp.storage_key(), "mcp:same:/p");
        assert_eq!(project.storage_key(), "project:/p");
        assert_ne!(session.storage_key(), mcp.storage_key());
        assert_ne!(mcp.storage_key(), project.storage_key());
    }

    #[test]
    fn utf8_prefix_never_splits_a_character() {
        assert_eq!(utf8_prefix("a你b", 2), "a");
        assert_eq!(utf8_prefix("a你b", 4), "a你");
    }

    #[test]
    fn long_message_uses_private_overwrite_file_and_two_kib_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let message = "x".repeat(MESSAGE_FILE_THRESHOLD_BYTES + 1);
        let output = render_at(&sample(&message), "same-scope", dir.path()).unwrap();
        let path = write_full_message_at(dir.path(), "same-scope", &message).unwrap();

        assert!(output.contains("[message_truncated]"));
        assert!(output.contains("[message_full_file]"));
        assert!(output.contains(&path.display().to_string()));
        let displayed = output
            .split("[message_truncated]\n")
            .nth(1)
            .unwrap()
            .split("\n\n[message_full_file]")
            .next()
            .unwrap();
        assert_eq!(displayed.len(), MESSAGE_STDOUT_PREFIX_BYTES);
        assert!(!output.contains("[message]\n"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), message);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn message_at_exact_threshold_stays_inline() {
        let dir = tempfile::tempdir().unwrap();
        let message = "x".repeat(MESSAGE_FILE_THRESHOLD_BYTES);
        let output = render_at(&sample(&message), "threshold", dir.path()).unwrap();
        assert!(!output.contains("[message_truncated]"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
