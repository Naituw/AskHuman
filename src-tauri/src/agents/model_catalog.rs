//! The model and reasoning effort a new Agent task may be started with
//! (docs/specs/ios-task-model.md).
//!
//! This is the shared vocabulary of the launch pipeline: `TaskSpec` carries a [`ModelChoice`],
//! the one-time `LaunchRecord` stores it, the helper turns it into command-line flags and the
//! Codex App path into thread settings. On top of that sits the catalog an iOS phone picks from:
//! what each Agent says about its own models ([`AgentCatalog`], read by [`source`], cached by
//! [`cache`]) and the check that a choice names something the catalog really offers
//! ([`resolve_choice`]). Nothing here starts an Agent session.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::AgentKind;

pub mod cache;
mod parse;
#[cfg(test)]
mod smoke;
pub mod source;

/// Longest model id accepted from outside the Agent itself.
pub const MAX_MODEL_ID_CHARS: usize = 128;
/// Longest reasoning effort accepted from outside the Agent itself.
pub const MAX_EFFORT_CHARS: usize = 16;
/// Most models kept per Agent: a longer listing is cut, never trusted to be small.
pub const MAX_MODELS_PER_AGENT: usize = 400;
/// Longest display name of a model on the phone.
pub const MAX_LABEL_CHARS: usize = 80;
/// Longest secondary line of a model on the phone.
pub const MAX_NOTE_CHARS: usize = 120;
/// Most reasoning efforts kept per model.
pub const MAX_EFFORTS_PER_MODEL: usize = 8;

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

// ---------------------------------------------------------------- the catalog

/// One model an Agent offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    /// What goes after `--model`; passes [`valid_model_id`].
    pub id: String,
    /// Display name for the phone; never empty (falls back to the id).
    pub label: String,
    /// Secondary line (a description, the gateway a Claude alias maps to).
    pub note: Option<String>,
    /// Reasoning efforts this model can be run with, weakest first as the Agent lists them. Empty
    /// = no effort row for this model.
    pub efforts: Vec<String>,
    /// The effort this model runs with when none is named; one of `efforts` when set.
    pub default_effort: Option<String>,
}

impl ModelEntry {
    pub fn plain(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            note: None,
            efforts: Vec::new(),
            default_effort: None,
        }
    }
}

/// What "Default" stands for on the phone: the model the Agent uses when no flag is passed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefaultInfo {
    /// Display name of that model, when it is known.
    pub label: Option<String>,
    /// Efforts the default model can be run with (the phone shows an effort row for them).
    pub efforts: Vec<String>,
    /// The effort the default model runs with when none is named.
    pub default_effort: Option<String>,
    /// Codex: the effort `config.toml` pins. A model picked without an effort that cannot take
    /// it would be run with an effort it does not know, so its own default is passed instead
    /// (spec M10).
    pub configured_effort: Option<String>,
}

/// Everything the phone may pick for one Agent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentCatalog {
    pub models: Vec<ModelEntry>,
    pub default: DefaultInfo,
    /// The id the Agent's own listing marks as its default (not the one the user configured).
    /// The listing is cached for a long time and the configuration is not, so "Default" is
    /// worked out again from this and the configuration as it is now (see `source`).
    pub listed_default: Option<String>,
}

impl AgentCatalog {
    pub fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|entry| entry.id == id)
    }
}

/// Why a choice was refused; [`ChoiceError::reason`] is the wire string of the failed task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceError {
    /// The model is not in the Agent's catalog (any more).
    ModelUnavailable,
    /// The effort is not one the model (or the default model) offers.
    EffortUnavailable,
}

impl ChoiceError {
    pub fn reason(self) -> &'static str {
        match self {
            ChoiceError::ModelUnavailable => "modelUnavailable",
            ChoiceError::EffortUnavailable => "effortUnavailable",
        }
    }
}

/// Matches what the phone asked for against what the Agent offers right now and returns what the
/// launch should carry. A default choice always passes; anything else must name a catalog entry
/// and one of its efforts. A model picked without an effort keeps "no effort flag", except when
/// Codex's configured effort does not exist for that model: then the model's own default is
/// passed (spec M10).
pub fn resolve_choice(
    kind: AgentKind,
    catalog: &AgentCatalog,
    wanted: &ModelChoice,
) -> Result<ModelChoice, ChoiceError> {
    if wanted.is_default() {
        return Ok(ModelChoice::default());
    }
    let Some(model) = &wanted.model else {
        // A bare effort belongs to whatever the Agent runs by default.
        let effort = wanted.effort.as_deref().unwrap_or_default();
        return if catalog.default.efforts.iter().any(|e| e == effort) {
            Ok(wanted.clone())
        } else {
            Err(ChoiceError::EffortUnavailable)
        };
    };
    let entry = catalog.model(model).ok_or(ChoiceError::ModelUnavailable)?;
    match &wanted.effort {
        Some(effort) => {
            if entry.efforts.iter().any(|e| e == effort) {
                Ok(wanted.clone())
            } else {
                Err(ChoiceError::EffortUnavailable)
            }
        }
        None => {
            let pinned_is_unknown = kind == AgentKind::Codex
                && !entry.efforts.is_empty()
                && catalog
                    .default
                    .configured_effort
                    .as_ref()
                    .is_some_and(|pinned| !entry.efforts.contains(pinned));
            Ok(ModelChoice {
                model: Some(entry.id.clone()),
                effort: pinned_is_unknown
                    .then(|| entry.default_effort.clone())
                    .flatten(),
            })
        }
    }
}

// ---------------------------------------------------------------- cleaning what an Agent prints

/// Text that is safe to put on a phone: invisible and bidirectional-control characters dropped,
/// runs of whitespace folded to one space, and cut to `max` characters (with an ellipsis).
pub fn clean_text(raw: &str, max: usize) -> String {
    let kept: String = raw.chars().filter(|c| !is_invisible(*c)).collect();
    let folded = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if folded.chars().count() <= max {
        return folded;
    }
    let mut cut: String = folded.chars().take(max.saturating_sub(1)).collect();
    while cut.ends_with(' ') {
        cut.pop();
    }
    cut.push('\u{2026}');
    cut
}

fn is_invisible(c: char) -> bool {
    (c.is_control() && !c.is_whitespace())
        || matches!(
            c,
            '\u{00ad}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

/// The effort names worth keeping: valid, unique, in the order given, at most
/// [`MAX_EFFORTS_PER_MODEL`] of them.
pub(crate) fn clean_efforts<I, S>(raw: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    for effort in raw {
        let effort = effort.as_ref().trim();
        if valid_effort(effort) && !out.iter().any(|e| e == effort) {
            out.push(effort.to_string());
        }
        if out.len() >= MAX_EFFORTS_PER_MODEL {
            break;
        }
    }
    out
}

/// Applies every rule about what may reach the phone to entries a parser produced: ids that could
/// pass for a flag are dropped, duplicates keep their first place, texts are cleaned and the
/// whole list is capped.
pub(crate) fn finish_entries(entries: Vec<ModelEntry>) -> Vec<ModelEntry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for mut entry in entries {
        if !valid_model_id(&entry.id) || !seen.insert(entry.id.clone()) {
            continue;
        }
        entry.label = clean_text(&entry.label, MAX_LABEL_CHARS);
        if entry.label.is_empty() {
            entry.label = entry.id.clone();
        }
        entry.note = entry
            .note
            .map(|note| clean_text(&note, MAX_NOTE_CHARS))
            .filter(|note| !note.is_empty());
        entry.efforts = clean_efforts(&entry.efforts);
        entry.default_effort = entry
            .default_effort
            .filter(|effort| valid_effort(effort) && entry.efforts.contains(effort));
        out.push(entry);
        if out.len() >= MAX_MODELS_PER_AGENT {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(model: Option<&str>, effort: Option<&str>) -> ModelChoice {
        ModelChoice::new(model.map(str::to_string), effort.map(str::to_string))
    }

    fn entry(id: &str, efforts: &[&str], default_effort: Option<&str>) -> ModelEntry {
        ModelEntry {
            efforts: efforts.iter().map(|e| e.to_string()).collect(),
            default_effort: default_effort.map(str::to_string),
            ..ModelEntry::plain(id, id)
        }
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

    fn sample_catalog() -> AgentCatalog {
        AgentCatalog {
            models: vec![
                entry("model-a", &["low", "medium", "high"], Some("medium")),
                entry("model-b", &["medium", "high"], Some("high")),
                entry("model-c", &[], None),
            ],
            default: DefaultInfo {
                label: Some("Model A".into()),
                efforts: vec!["low".into(), "medium".into(), "high".into()],
                default_effort: Some("medium".into()),
                configured_effort: None,
            },
            ..AgentCatalog::default()
        }
    }

    #[test]
    fn a_default_choice_always_resolves_and_stays_default() {
        for kind in AgentKind::ALL {
            assert_eq!(
                resolve_choice(kind, &AgentCatalog::default(), &ModelChoice::default()),
                Ok(ModelChoice::default())
            );
        }
    }

    #[test]
    fn a_picked_model_and_effort_must_both_be_in_the_catalog() {
        let catalog = sample_catalog();
        let kind = AgentKind::Claude;
        assert_eq!(
            resolve_choice(kind, &catalog, &choice(Some("model-a"), Some("low"))),
            Ok(choice(Some("model-a"), Some("low")))
        );
        assert_eq!(
            resolve_choice(kind, &catalog, &choice(Some("model-a"), None)),
            Ok(choice(Some("model-a"), None))
        );
        assert_eq!(
            resolve_choice(kind, &catalog, &choice(Some("nope"), None)),
            Err(ChoiceError::ModelUnavailable)
        );
        assert_eq!(
            resolve_choice(kind, &catalog, &choice(Some("nope"), Some("low"))),
            Err(ChoiceError::ModelUnavailable)
        );
        // `low` exists for model-a but not for model-b.
        assert_eq!(
            resolve_choice(kind, &catalog, &choice(Some("model-b"), Some("low"))),
            Err(ChoiceError::EffortUnavailable)
        );
        // A model without an effort row takes none.
        assert_eq!(
            resolve_choice(kind, &catalog, &choice(Some("model-c"), Some("high"))),
            Err(ChoiceError::EffortUnavailable)
        );
        assert_eq!(
            resolve_choice(kind, &catalog, &choice(Some("model-c"), None)),
            Ok(choice(Some("model-c"), None))
        );
    }

    #[test]
    fn a_bare_effort_must_be_one_the_default_model_offers() {
        let catalog = sample_catalog();
        assert_eq!(
            resolve_choice(AgentKind::Claude, &catalog, &choice(None, Some("high"))),
            Ok(choice(None, Some("high")))
        );
        assert_eq!(
            resolve_choice(AgentKind::Claude, &catalog, &choice(None, Some("max"))),
            Err(ChoiceError::EffortUnavailable)
        );
        // Nothing is known about the default model: nothing can be asked of it.
        assert_eq!(
            resolve_choice(
                AgentKind::Claude,
                &AgentCatalog::default(),
                &choice(None, Some("high"))
            ),
            Err(ChoiceError::EffortUnavailable)
        );
    }

    #[test]
    fn an_empty_catalog_refuses_every_named_model() {
        assert_eq!(
            resolve_choice(
                AgentKind::Pi,
                &AgentCatalog::default(),
                &choice(Some("model-a"), None)
            ),
            Err(ChoiceError::ModelUnavailable)
        );
    }

    #[test]
    fn a_pinned_effort_the_picked_model_lacks_is_replaced_by_its_default() {
        let mut catalog = sample_catalog();
        catalog.default.configured_effort = Some("low".into());
        // model-b has no `low`: Codex would be run with an effort it does not know.
        assert_eq!(
            resolve_choice(AgentKind::Codex, &catalog, &choice(Some("model-b"), None)),
            Ok(choice(Some("model-b"), Some("high")))
        );
        // model-a supports it: nothing to add.
        assert_eq!(
            resolve_choice(AgentKind::Codex, &catalog, &choice(Some("model-a"), None)),
            Ok(choice(Some("model-a"), None))
        );
        // An effort the user picked is never overridden.
        assert_eq!(
            resolve_choice(
                AgentKind::Codex,
                &catalog,
                &choice(Some("model-b"), Some("medium"))
            ),
            Ok(choice(Some("model-b"), Some("medium")))
        );
        // A model with no effort row cannot be given one.
        assert_eq!(
            resolve_choice(AgentKind::Codex, &catalog, &choice(Some("model-c"), None)),
            Ok(choice(Some("model-c"), None))
        );
        // Only Codex pins an effort in a file we read; no other Agent is touched.
        assert_eq!(
            resolve_choice(AgentKind::Claude, &catalog, &choice(Some("model-b"), None)),
            Ok(choice(Some("model-b"), None))
        );
    }

    #[test]
    fn choice_errors_have_stable_wire_reasons() {
        assert_eq!(ChoiceError::ModelUnavailable.reason(), "modelUnavailable");
        assert_eq!(ChoiceError::EffortUnavailable.reason(), "effortUnavailable");
    }

    #[test]
    fn clean_text_drops_invisible_characters_and_folds_whitespace() {
        assert_eq!(
            clean_text("  Model\u{200b}  \t A \u{202e}(High)\u{feff}\n", 80),
            "Model A (High)"
        );
        assert_eq!(clean_text("bell\u{7}ring\u{1b}[0m", 80), "bellring[0m");
        assert_eq!(clean_text("", 80), "");
        assert_eq!(clean_text("\u{200b}\u{200c}", 80), "");
    }

    #[test]
    fn clean_text_cuts_to_the_limit_with_an_ellipsis() {
        let text = "word ".repeat(40);
        let cut = clean_text(&text, 20);
        assert!(cut.chars().count() <= 20, "{cut:?}");
        assert!(cut.ends_with('\u{2026}'));
        assert!(!cut.contains("  "));
        assert_eq!(clean_text("short", 20), "short");
        // Counted in characters, not bytes.
        let wide = "\u{6a21}\u{578b}".repeat(30);
        assert_eq!(clean_text(&wide, 10).chars().count(), 10);
    }

    #[test]
    fn finished_entries_hold_only_clean_unique_valid_models() {
        let mut messy = entry(
            "model-a",
            &["low", "Bad", "low", "high", "-x"],
            Some("nope"),
        );
        messy.label = "  Model\u{200b} A  ".into();
        messy.note = Some("\u{200b}  ".into());
        let kept = finish_entries(vec![
            messy,
            ModelEntry::plain("--yolo", "flag"),
            ModelEntry::plain("model-a", "duplicate"),
            ModelEntry::plain("has space", "x"),
            ModelEntry::plain("model-b", "\u{200b}"),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].id, "model-a");
        assert_eq!(kept[0].label, "Model A");
        assert_eq!(kept[0].note, None);
        assert_eq!(kept[0].efforts, vec!["low", "high"]);
        assert_eq!(kept[0].default_effort, None);
        // An empty label falls back to the id.
        assert_eq!(kept[1].label, "model-b");
    }

    #[test]
    fn a_default_effort_outside_the_efforts_is_dropped() {
        let kept = finish_entries(vec![
            entry("a", &["low", "high"], Some("high")),
            entry("b", &["low", "high"], Some("medium")),
            entry("c", &[], Some("high")),
        ]);
        assert_eq!(kept[0].default_effort.as_deref(), Some("high"));
        assert_eq!(kept[1].default_effort, None);
        assert_eq!(kept[2].default_effort, None);
    }

    #[test]
    fn the_list_and_the_efforts_are_capped() {
        let many: Vec<ModelEntry> = (0..MAX_MODELS_PER_AGENT + 50)
            .map(|i| ModelEntry::plain(&format!("model-{i}"), "m"))
            .collect();
        assert_eq!(finish_entries(many).len(), MAX_MODELS_PER_AGENT);
        let efforts: Vec<String> = (0..20).map(|i| format!("e{i}")).collect();
        assert_eq!(clean_efforts(&efforts).len(), MAX_EFFORTS_PER_MODEL);
    }
}
