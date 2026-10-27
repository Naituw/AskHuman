//! The model and reasoning effort a new Agent task may be started with
//! (docs/specs/ios-task-model.md).
//!
//! This is the shared vocabulary of the launch pipeline: `TaskSpec` carries a [`ModelChoice`],
//! the one-time `LaunchRecord` stores it, the helper turns it into command-line flags and the
//! Codex App path into thread settings. Nothing in here talks to an Agent.

use serde::{Deserialize, Serialize};

use super::AgentKind;

/// Longest model id accepted from outside the Agent itself.
pub const MAX_MODEL_ID_CHARS: usize = 128;
/// Longest reasoning effort accepted from outside the Agent itself.
pub const MAX_EFFORT_CHARS: usize = 16;

/// What the user picked for a new task. `None` leaves that part to the Agent: no flag is passed
/// and the Agent's own configuration decides, exactly as before this existed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl ModelChoice {
    pub fn new(model: Option<String>, effort: Option<String>) -> Self {
        Self { model, effort }
    }

    /// Nothing was picked: the Agent is started without any model or effort flag.
    pub fn is_default(&self) -> bool {
        self.model.is_none() && self.effort.is_none()
    }

    /// Whether `kind` can honour this choice and nothing in it could be mistaken for a flag.
    /// The values are also matched against the Agent's catalog before a task is accepted; this is
    /// the last line of defence between a value and an argv.
    pub fn check_for(&self, kind: AgentKind) -> Result<(), String> {
        if let Some(model) = &self.model {
            if !valid_model_id(model) {
                return Err("invalid model id".into());
            }
        }
        if let Some(effort) = &self.effort {
            if !valid_effort(effort) {
                return Err("invalid reasoning effort".into());
            }
            // Cursor folds the effort into the model id; there is no separate switch to set.
            if kind == AgentKind::Cursor {
                return Err("Cursor has no separate reasoning effort".into());
            }
        }
        Ok(())
    }
}

/// `[A-Za-z0-9][A-Za-z0-9._/-]*`, at most [`MAX_MODEL_ID_CHARS`] characters: never starts with a
/// dash, never holds whitespace, control characters or `:` (Pi reads `:` as a thinking-level
/// shorthand).
pub fn valid_model_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.chars().count() <= MAX_MODEL_ID_CHARS
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

/// `[a-z][a-z0-9_-]*`, at most [`MAX_EFFORT_CHARS`] characters.
pub fn valid_effort(effort: &str) -> bool {
    let mut chars = effort.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && effort.chars().count() <= MAX_EFFORT_CHARS
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(model: Option<&str>, effort: Option<&str>) -> ModelChoice {
        ModelChoice::new(model.map(str::to_string), effort.map(str::to_string))
    }

    #[test]
    fn model_ids_are_plain_words_that_cannot_pass_for_a_flag() {
        for ok in [
            "opus",
            "gpt-5.3-codex-low",
            "claude-fable-5",
            "provider/model-1",
            "a",
            "model_a.b",
        ] {
            assert!(valid_model_id(ok), "{ok}");
        }
        let long = "a".repeat(MAX_MODEL_ID_CHARS + 1);
        for bad in [
            "",
            "-m",
            "--yolo",
            "/abs",
            ".hidden",
            "a b",
            "a\tb",
            "a\nb",
            "a\0b",
            "a:high",
            "a=b",
            "a,b",
            "a\u{200b}b",
            "模型",
            long.as_str(),
        ] {
            assert!(!valid_model_id(bad), "{bad:?}");
        }
        assert!(valid_model_id(&"a".repeat(MAX_MODEL_ID_CHARS)));
    }

    #[test]
    fn efforts_are_short_lowercase_words() {
        for ok in ["low", "medium", "high", "xhigh", "max", "a", "x_1-2"] {
            assert!(valid_effort(ok), "{ok}");
        }
        let long = "a".repeat(MAX_EFFORT_CHARS + 1);
        for bad in [
            "",
            "High",
            "-low",
            "1high",
            "a b",
            "a\n",
            "a=b",
            long.as_str(),
        ] {
            assert!(!valid_effort(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_default_choice_is_empty_and_serialises_to_nothing() {
        let none = ModelChoice::default();
        assert!(none.is_default());
        assert_eq!(serde_json::to_string(&none).unwrap(), "{}");
        assert!(!choice(Some("m"), None).is_default());
        assert!(!choice(None, Some("high")).is_default());
        let back: ModelChoice = serde_json::from_str("{}").unwrap();
        assert!(back.is_default());
        let both: ModelChoice =
            serde_json::from_str(r#"{"model":"m","effort":"high","future":1}"#).unwrap();
        assert_eq!(both, choice(Some("m"), Some("high")));
    }

    #[test]
    fn check_for_rejects_what_an_agent_cannot_take() {
        assert!(ModelChoice::default().check_for(AgentKind::Cursor).is_ok());
        assert!(choice(Some("auto"), None)
            .check_for(AgentKind::Cursor)
            .is_ok());
        assert!(choice(None, Some("high"))
            .check_for(AgentKind::Cursor)
            .is_err());
        assert!(choice(Some("m"), Some("high"))
            .check_for(AgentKind::Cursor)
            .is_err());
        for kind in [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Grok,
            AgentKind::Pi,
        ] {
            assert!(choice(Some("m"), Some("high")).check_for(kind).is_ok());
            assert!(choice(Some("--yolo"), None).check_for(kind).is_err());
            assert!(choice(None, Some("-x")).check_for(kind).is_err());
        }
    }
}
