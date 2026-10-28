//! What each Agent says about its models, turned into [`AgentCatalog`]s.
//!
//! Every function in here is pure: text or JSON in, entries out. The sources (a process, a
//! config file) live in `source`, so every format is covered by a fixture. A line that does not
//! look like what the Agent is documented to print is skipped, never guessed at: a listing that
//! cannot be understood comes out empty and the phone is left with "Default" only.
//!
//! The config readers are typed on purpose. `settings.json` of Claude Code and
//! `cli-config.json` of Cursor hold credentials next to the one value we want; a struct that
//! names only that value never materialises the rest.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{
    clean_efforts, clean_text, finish_entries, valid_effort, valid_model_id, AgentCatalog,
    DefaultInfo, ModelEntry, MAX_LABEL_CHARS, MAX_NOTE_CHARS,
};

/// A parsed listing: the entries and the id the Agent marked as its default.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Listing {
    pub models: Vec<ModelEntry>,
    pub default_id: Option<String>,
}

impl Listing {
    fn finish(models: Vec<ModelEntry>, default_id: Option<String>) -> Self {
        let models = finish_entries(models);
        let default_id = default_id.filter(|id| models.iter().any(|m| &m.id == id));
        Self { models, default_id }
    }

    #[cfg(test)]
    fn label_of(&self, id: &str) -> Option<String> {
        self.models
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.label.clone())
    }
}

/// A string, or nothing at all when the JSON value is anything else (a number, a table, null).
fn lenient_string<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(de)? {
        Value::String(text) => Some(text),
        _ => None,
    })
}

fn some_text(raw: &Option<String>, max: usize) -> Option<String> {
    raw.as_deref()
        .map(|text| clean_text(text, max))
        .filter(|text| !text.is_empty())
}

// ---------------------------------------------------------------- Claude Code

/// The values of `~/.claude/settings.json` that decide what an alias means. Everything else in
/// the file, credentials included, is dropped while parsing.
#[derive(Debug, Default, Deserialize)]
pub struct ClaudeSettings {
    #[serde(default, deserialize_with = "lenient_string")]
    model: Option<String>,
    #[serde(default, deserialize_with = "lenient_env")]
    env: ClaudeEnv,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeEnv {
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_OPUS_MODEL",
        deserialize_with = "lenient_string"
    )]
    opus: Option<String>,
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
        deserialize_with = "lenient_string"
    )]
    opus_name: Option<String>,
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_SONNET_MODEL",
        deserialize_with = "lenient_string"
    )]
    sonnet: Option<String>,
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
        deserialize_with = "lenient_string"
    )]
    sonnet_name: Option<String>,
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        deserialize_with = "lenient_string"
    )]
    haiku: Option<String>,
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
        deserialize_with = "lenient_string"
    )]
    haiku_name: Option<String>,
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_FABLE_MODEL",
        deserialize_with = "lenient_string"
    )]
    fable: Option<String>,
    #[serde(
        default,
        rename = "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
        deserialize_with = "lenient_string"
    )]
    fable_name: Option<String>,
}

/// `env` that is not an object (or is malformed) counts as absent.
fn lenient_env<'de, D: Deserializer<'de>>(de: D) -> Result<ClaudeEnv, D::Error> {
    let value = Value::deserialize(de)?;
    Ok(ClaudeEnv::deserialize(value).unwrap_or_default())
}

/// Anything that is not a JSON object with the keys above yields empty settings.
pub fn parse_claude_settings(text: &str) -> ClaudeSettings {
    serde_json::from_str(text).unwrap_or_default()
}

/// The reasoning efforts `claude --effort` takes.
const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Claude Code has no list command: the aliases it documents (`opus`, `sonnet`, `haiku`, plus
/// `fable` when `--help` names it), each with what the user's settings map it to.
pub(super) fn claude_catalog(settings: &ClaudeSettings, help: Option<&str>) -> AgentCatalog {
    claude_catalog_with(
        settings,
        help.is_some_and(|text| mentions_word(text, "fable")),
    )
}

/// [`claude_catalog`] when it is already known whether the Agent has the `fable` alias.
pub(super) fn claude_catalog_with(settings: &ClaudeSettings, fable: bool) -> AgentCatalog {
    let env = &settings.env;
    let mut aliases: Vec<(&str, &str, &Option<String>, &Option<String>)> = Vec::new();
    if fable {
        aliases.push(("fable", "Fable", &env.fable, &env.fable_name));
    }
    aliases.push(("opus", "Opus", &env.opus, &env.opus_name));
    aliases.push(("sonnet", "Sonnet", &env.sonnet, &env.sonnet_name));
    aliases.push(("haiku", "Haiku", &env.haiku, &env.haiku_name));
    let models = aliases
        .into_iter()
        .map(|(id, label, mapped, mapped_name)| ModelEntry {
            id: id.to_string(),
            label: label.to_string(),
            note: some_text(mapped_name, MAX_NOTE_CHARS)
                .or_else(|| some_text(mapped, MAX_NOTE_CHARS))
                .map(|target| format!("\u{2192} {target}")),
            efforts: CLAUDE_EFFORTS.iter().map(|e| e.to_string()).collect(),
            default_effort: None,
        })
        .collect();
    AgentCatalog {
        models: finish_entries(models),
        default: DefaultInfo {
            label: some_text(&settings.model, MAX_LABEL_CHARS),
            efforts: CLAUDE_EFFORTS.iter().map(|e| e.to_string()).collect(),
            default_effort: None,
            configured_effort: None,
        },
        listed_default: None,
    }
}

/// Whether `word` occurs in `text` as a whole word, ignoring case.
fn mentions_word(text: &str, word: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
    })
}

// ---------------------------------------------------------------- Codex

/// The two values of `~/.codex/config.toml` that decide what "Default" means.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CodexConfig {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// Top-level `model` and `model_reasoning_effort`; anything else (or a file that does not parse)
/// is ignored.
pub fn parse_codex_config(text: &str) -> CodexConfig {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return CodexConfig::default();
    };
    let string = |key: &str| {
        doc.get(key)
            .and_then(|item| item.as_str())
            .map(str::to_string)
    };
    CodexConfig {
        model: string("model").filter(|m| valid_model_id(m)),
        effort: string("model_reasoning_effort").filter(|e| valid_effort(e)),
    }
}

/// Entries of the `model/list` answers of `codex app-server` (all pages concatenated).
pub(super) fn parse_codex_models(entries: &[Value]) -> Listing {
    let text =
        |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let mut models = Vec::new();
    let mut default_id = None;
    for entry in entries {
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        // `model` is the slug `--model` takes; `id` is the catalog's own key and usually equal.
        let Some(id) = text(entry, "model").or_else(|| text(entry, "id")) else {
            continue;
        };
        let efforts = entry
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("reasoningEffort").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if entry.get("isDefault").and_then(Value::as_bool) == Some(true) && default_id.is_none() {
            default_id = Some(id.clone());
        }
        models.push(ModelEntry {
            label: text(entry, "displayName").unwrap_or_else(|| id.clone()),
            note: text(entry, "description"),
            default_effort: text(entry, "defaultReasoningEffort"),
            efforts,
            id,
        });
    }
    Listing::finish(models, default_id)
}

/// Codex's catalog plus what `config.toml` says is used by default.
pub(super) fn codex_catalog(listing: Listing, config: &CodexConfig) -> AgentCatalog {
    AgentCatalog {
        default: codex_default(&listing.models, listing.default_id.as_deref(), config),
        models: listing.models,
        listed_default: listing.default_id,
    }
}

/// What "Default" stands for on Codex: the model `config.toml` names, else the one the listing
/// marks, with the efforts that model takes and the effort the file pins.
pub(super) fn codex_default(
    models: &[ModelEntry],
    listed_default: Option<&str>,
    config: &CodexConfig,
) -> DefaultInfo {
    let default_id = config.model.as_deref().or(listed_default);
    let default_entry = default_id.and_then(|id| models.iter().find(|m| m.id == id));
    DefaultInfo {
        label: default_id.map(|id| {
            default_entry
                .map(|m| m.label.clone())
                .unwrap_or_else(|| id.to_string())
        }),
        efforts: default_entry.map(|m| m.efforts.clone()).unwrap_or_default(),
        default_effort: config
            .effort
            .clone()
            .or_else(|| default_entry.and_then(|m| m.default_effort.clone())),
        configured_effort: config.effort.clone(),
    }
}

// ---------------------------------------------------------------- Cursor

/// The model `~/.cursor/cli-config.json` selects. Only its display name is read.
#[derive(Debug, Default, Deserialize)]
pub struct CursorConfig {
    #[serde(default, deserialize_with = "lenient_cursor_model")]
    model: Option<String>,
}

fn lenient_cursor_model<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(de)?;
    Ok(value
        .get("displayName")
        .and_then(Value::as_str)
        .map(str::to_string))
}

pub fn parse_cursor_config(text: &str) -> CursorConfig {
    serde_json::from_str(text).unwrap_or_default()
}

/// Output of `cursor-agent --list-models`:
///
/// ```text
/// Available models
///
/// auto - Auto (default)
/// some-model-id - Some Model Name
///
/// Tip: ...
/// ```
///
/// Only lines after the `Available models` header count, so whatever a shell rc printed ahead of
/// the command cannot become a model.
pub(super) fn parse_cursor_list(text: &str) -> Listing {
    let mut lines = text.lines();
    if !lines.any(|line| is_header(line, "available models")) {
        return Listing::default();
    }
    let mut models = Vec::new();
    let mut default_id = None;
    for line in lines {
        let Some((id, rest)) = line.trim().split_once(" - ") else {
            continue;
        };
        let id = id.trim();
        if !valid_model_id(id) {
            continue;
        }
        let (label, is_default) = strip_default_marker(rest);
        if is_default && default_id.is_none() {
            default_id = Some(id.to_string());
        }
        models.push(ModelEntry::plain(id, &label));
    }
    Listing::finish(models, default_id)
}

/// The Agent's default model as the phone shows it: what the config selects, else the listing's
/// own default entry.
pub(super) fn cursor_catalog(listing: Listing, config: &CursorConfig) -> AgentCatalog {
    AgentCatalog {
        default: cursor_default(&listing.models, listing.default_id.as_deref(), config),
        models: listing.models,
        listed_default: listing.default_id,
    }
}

/// [`cursor_catalog`]'s "Default": Cursor folds the effort into the model id, so it never has an
/// effort row, only a name.
pub(super) fn cursor_default(
    models: &[ModelEntry],
    listed_default: Option<&str>,
    config: &CursorConfig,
) -> DefaultInfo {
    let label = some_text(&config.model, MAX_LABEL_CHARS).or_else(|| {
        listed_default
            .and_then(|id| models.iter().find(|m| m.id == id))
            .map(|m| m.label.clone())
    });
    DefaultInfo {
        label,
        ..DefaultInfo::default()
    }
}

/// `"Auto (default)"` → `("Auto", true)`; any other text is returned as it is.
fn strip_default_marker(raw: &str) -> (String, bool) {
    let cleaned = clean_text(raw, 4 * MAX_LABEL_CHARS);
    let lower = cleaned.to_ascii_lowercase();
    match lower.rfind("(default)") {
        Some(at) => {
            let mut label = cleaned[..at].to_string();
            label.push_str(&cleaned[at + "(default)".len()..]);
            (label.trim().to_string(), true)
        }
        None => (cleaned, false),
    }
}

/// `line` is `name`, optionally with a colon, in any case.
fn is_header(line: &str, name: &str) -> bool {
    line.trim()
        .trim_end_matches(':')
        .trim()
        .eq_ignore_ascii_case(name)
}

// ---------------------------------------------------------------- Grok

/// Output of `grok models`:
///
/// ```text
/// Default model: model-a
/// Available models:
///   * model-a (default)
///   - model-b
/// ```
///
/// Grok prints ids only: no display names and no reasoning efforts.
pub(super) fn parse_grok_models(text: &str) -> Listing {
    let mut default_id: Option<String> = None;
    let mut in_list = false;
    let mut models = Vec::new();
    let mut starred = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once(':') {
            if key.trim().eq_ignore_ascii_case("default model") {
                default_id = value.split_whitespace().next().map(str::to_string);
                continue;
            }
        }
        if is_header(trimmed, "available models") {
            in_list = true;
            continue;
        }
        if !in_list {
            continue;
        }
        let Some(rest) = trimmed
            .strip_prefix('*')
            .map(|rest| (rest, true))
            .or_else(|| trimmed.strip_prefix('-').map(|rest| (rest, false)))
        else {
            continue;
        };
        let (rest, bullet_star) = rest;
        let Some(id) = rest.split_whitespace().next() else {
            continue;
        };
        if !valid_model_id(id) {
            continue;
        }
        let marked = bullet_star || rest.to_ascii_lowercase().contains("(default)");
        if marked && starred.is_none() {
            starred = Some(id.to_string());
        }
        models.push(ModelEntry::plain(id, id));
    }
    Listing::finish(models, default_id.or(starred))
}

pub(super) fn grok_catalog(listing: Listing) -> AgentCatalog {
    let label = listing.default_id.clone();
    AgentCatalog {
        models: listing.models,
        default: DefaultInfo {
            label,
            ..DefaultInfo::default()
        },
        listed_default: listing.default_id,
    }
}

// ---------------------------------------------------------------- Pi

/// The default model `~/.pi/agent/settings.json` names.
#[derive(Debug, Default, Deserialize)]
pub struct PiSettings {
    #[serde(
        default,
        rename = "defaultProvider",
        deserialize_with = "lenient_string"
    )]
    provider: Option<String>,
    #[serde(default, rename = "defaultModel", deserialize_with = "lenient_string")]
    model: Option<String>,
}

pub fn parse_pi_settings(text: &str) -> PiSettings {
    serde_json::from_str(text).unwrap_or_default()
}

/// The thinking levels Pi can be asked for. Pi's own scale is longer (`minimal`, `xhigh`, ...);
/// these three exist on every model that thinks at all.
const PI_EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// Output of `pi --list-models`, a whitespace-aligned table:
///
/// ```text
/// provider  model    context  max-out  thinking  images
/// alpha     model-a  200K     64K      yes       yes
/// ```
///
/// A model is addressed as `provider/model`; only rows with `thinking yes` get an effort row.
pub(super) fn parse_pi_list(text: &str) -> Listing {
    let mut lines = text.lines();
    let header = lines.by_ref().find_map(|line| {
        let columns: Vec<String> = line
            .split_whitespace()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let at = |name: &str| columns.iter().position(|c| c == name);
        Some((
            columns.len(),
            at("provider")?,
            at("model")?,
            at("thinking")?,
        ))
    });
    let Some((width, provider_at, model_at, thinking_at)) = header else {
        return Listing::default();
    };
    let mut models = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split_whitespace().collect();
        // A row that does not line up with the header is not a model row.
        if cells.len() != width {
            continue;
        }
        let id = format!("{}/{}", cells[provider_at], cells[model_at]);
        let thinks = cells[thinking_at].eq_ignore_ascii_case("yes");
        models.push(ModelEntry {
            efforts: if thinks {
                clean_efforts(PI_EFFORTS)
            } else {
                Vec::new()
            },
            ..ModelEntry::plain(&id, &id)
        });
    }
    Listing::finish(models, None)
}

pub(super) fn pi_catalog(listing: Listing, settings: &PiSettings) -> AgentCatalog {
    AgentCatalog {
        default: pi_default(&listing.models, settings),
        models: listing.models,
        listed_default: None,
    }
}

/// Pi's "Default": the `provider/model` its settings name, with the efforts that model thinks at.
pub(super) fn pi_default(models: &[ModelEntry], settings: &PiSettings) -> DefaultInfo {
    let default_id = match (&settings.provider, &settings.model) {
        (Some(provider), Some(model)) => Some(format!("{provider}/{model}")),
        _ => None,
    };
    let default_entry = default_id
        .as_deref()
        .and_then(|id| models.iter().find(|m| m.id == id));
    DefaultInfo {
        label: default_id
            .as_deref()
            .map(|id| clean_text(id, MAX_LABEL_CHARS)),
        efforts: default_entry.map(|m| m.efforts.clone()).unwrap_or_default(),
        ..DefaultInfo::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURSOR_LIST: &str = include_str!("fixtures/cursor-list.txt");
    const PI_LIST: &str = include_str!("fixtures/pi-list.txt");
    const GROK_MODELS: &str = include_str!("fixtures/grok-models.txt");
    const CODEX_PAGE_1: &str = include_str!("fixtures/codex-model-list-1.json");
    const CODEX_PAGE_2: &str = include_str!("fixtures/codex-model-list-2.json");
    const CLAUDE_SETTINGS: &str = include_str!("fixtures/claude-settings.json");
    const CLAUDE_HELP: &str = include_str!("fixtures/claude-help.txt");
    const CODEX_CONFIG: &str = include_str!("fixtures/codex-config.toml");
    const CURSOR_CONFIG: &str = include_str!("fixtures/cursor-cli-config.json");
    const PI_SETTINGS: &str = include_str!("fixtures/pi-settings.json");

    /// The fixtures hold `{ZWSP}` where a real listing holds an invisible character, so an editor
    /// cannot strip what the test is about.
    fn with_invisibles(text: &str) -> String {
        text.replace("{ZWSP}", "\u{200b}")
    }

    fn ids(listing: &Listing) -> Vec<&str> {
        listing.models.iter().map(|m| m.id.as_str()).collect()
    }

    fn codex_entries() -> Vec<Value> {
        [CODEX_PAGE_1, CODEX_PAGE_2]
            .iter()
            .flat_map(|page| {
                let value: Value = serde_json::from_str(page).unwrap();
                value["data"].as_array().unwrap().clone()
            })
            .collect()
    }

    // ------------------------------------------------------------ Cursor

    #[test]
    fn the_cursor_list_keeps_valid_ids_and_the_default() {
        let listing = parse_cursor_list(&with_invisibles(CURSOR_LIST));
        assert_eq!(
            ids(&listing),
            [
                "auto",
                "model-a-low",
                "model-a-low-fast",
                "model-a",
                "model-a-high",
                "model-b-thinking-high",
                "model-c"
            ]
        );
        assert_eq!(listing.default_id.as_deref(), Some("auto"));
        let label = |id: &str| listing.label_of(id).unwrap();
        // The marker is not part of the name; the other suffixes are.
        assert_eq!(label("auto"), "Auto");
        assert_eq!(label("model-a-low-fast"), "Model A Low Fast");
        assert_eq!(
            label("model-b-thinking-high"),
            "Model B 1M Thinking (NO ZDR)"
        );
        assert_eq!(label("model-c"), "Model C");
        assert!(listing.models.iter().all(|m| m.efforts.is_empty()));
    }

    #[test]
    fn what_a_shell_prints_before_the_cursor_header_is_not_a_model() {
        let listing = parse_cursor_list(&with_invisibles(CURSOR_LIST));
        assert!(!ids(&listing).contains(&"nvm"));
        assert!(!ids(&listing).contains(&"Last"));
    }

    #[test]
    fn a_cursor_list_survives_windows_line_endings() {
        let crlf = with_invisibles(CURSOR_LIST).replace('\n', "\r\n");
        let listing = parse_cursor_list(&crlf);
        assert_eq!(listing.models.len(), 7);
        assert_eq!(listing.label_of("auto").as_deref(), Some("Auto"));
    }

    #[test]
    fn the_cursor_header_is_matched_loosely_but_required() {
        for header in [
            "Available models",
            "available models:",
            "  AVAILABLE MODELS  ",
        ] {
            let text = format!("{header}\n\nauto - Auto (default)\n");
            assert_eq!(parse_cursor_list(&text).models.len(), 1, "{header:?}");
        }
        // Without the header nothing is trusted, however much it looks like a list.
        let headerless = "auto - Auto (default)\nmodel-a - Model A\n";
        assert_eq!(parse_cursor_list(headerless), Listing::default());
        assert_eq!(parse_cursor_list(""), Listing::default());
    }

    #[test]
    fn a_garbled_or_truncated_cursor_list_never_panics() {
        for text in [
            "Available models\n\n",
            "Available models\n\nauto - ",
            "Available models\n\n - \n - -\n--\n",
            "Available models\n\n\u{0}\u{1b}[31mauto - Auto\u{1b}[0m\n",
            "Available models\n\nauto - Auto (default) (default)\n",
            "Available models\n\n\u{ff}\u{fe} - x\n",
        ] {
            let _ = parse_cursor_list(text);
        }
        let listing = parse_cursor_list("Available models\n\nauto - Auto (default) (default)\n");
        assert_eq!(listing.models.len(), 1);
        assert_eq!(listing.default_id.as_deref(), Some("auto"));
    }

    #[test]
    fn a_huge_cursor_list_is_cut_at_the_cap() {
        let mut text = String::from("Available models\n\n");
        for i in 0..2000 {
            text.push_str(&format!("model-{i} - Model {i}\n"));
        }
        assert_eq!(
            parse_cursor_list(&text).models.len(),
            crate::agents::model_catalog::MAX_MODELS_PER_AGENT
        );
    }

    #[test]
    fn the_cursor_default_is_what_the_config_selects() {
        let config = parse_cursor_config(CURSOR_CONFIG);
        let catalog = cursor_catalog(parse_cursor_list(&with_invisibles(CURSOR_LIST)), &config);
        assert_eq!(catalog.default.label.as_deref(), Some("Model A"));
        assert!(catalog.default.efforts.is_empty());
        // No config: the listing's own default entry stands in.
        let catalog = cursor_catalog(
            parse_cursor_list(&with_invisibles(CURSOR_LIST)),
            &CursorConfig::default(),
        );
        assert_eq!(catalog.default.label.as_deref(), Some("Auto"));
    }

    #[test]
    fn the_cursor_config_never_exposes_its_credentials() {
        let config = parse_cursor_config(CURSOR_CONFIG);
        let shown = format!("{config:?}");
        assert!(!shown.contains("synthetic-token-do-not-leak"), "{shown}");
        assert!(!shown.contains("example.invalid"), "{shown}");
        // Garbage and wrong shapes read as "no model selected".
        for text in [
            "",
            "[]",
            "{",
            r#"{"model": 3}"#,
            r#"{"model": {"displayName": 3}}"#,
        ] {
            assert!(parse_cursor_config(text).model.is_none(), "{text:?}");
        }
    }

    // ------------------------------------------------------------ Pi

    #[test]
    fn the_pi_table_gives_provider_slash_model_and_effort_rows_for_thinking_models() {
        let listing = parse_pi_list(PI_LIST);
        assert_eq!(
            ids(&listing),
            ["alpha/model-a", "alpha/model-b-mini", "beta/model-d"]
        );
        let efforts = |id: &str| {
            listing
                .models
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .efforts
                .clone()
        };
        assert_eq!(efforts("alpha/model-a"), ["low", "medium", "high"]);
        assert!(efforts("alpha/model-b-mini").is_empty());
        assert_eq!(efforts("beta/model-d"), ["low", "medium", "high"]);
        assert_eq!(listing.default_id, None);
    }

    #[test]
    fn pi_rows_that_do_not_line_up_or_cannot_be_ids_are_skipped() {
        let listing = parse_pi_list(PI_LIST);
        // A row of the wrong width, and a model whose slug holds a colon.
        assert!(!ids(&listing).iter().any(|id| id.contains("broken")));
        assert!(!ids(&listing).iter().any(|id| id.contains(':')));
    }

    #[test]
    fn a_pi_listing_needs_its_table_header() {
        assert_eq!(parse_pi_list(""), Listing::default());
        assert_eq!(
            parse_pi_list("alpha model-a 200K 64K yes yes\n"),
            Listing::default()
        );
        // Columns may move: they are found by name.
        let text = "model thinking provider\nmodel-a yes alpha\n";
        let listing = parse_pi_list(text);
        assert_eq!(ids(&listing), ["alpha/model-a"]);
        assert_eq!(listing.models[0].efforts, ["low", "medium", "high"]);
    }

    #[test]
    fn the_pi_default_is_the_model_its_settings_name() {
        let settings = parse_pi_settings(PI_SETTINGS);
        let catalog = pi_catalog(parse_pi_list(PI_LIST), &settings);
        assert_eq!(catalog.default.label.as_deref(), Some("alpha/model-a"));
        assert_eq!(catalog.default.efforts, ["low", "medium", "high"]);
        // A default that is not in the listing still has a name, but no effort row.
        let unknown = parse_pi_settings(r#"{"defaultProvider":"gamma","defaultModel":"x"}"#);
        let catalog = pi_catalog(parse_pi_list(PI_LIST), &unknown);
        assert_eq!(catalog.default.label.as_deref(), Some("gamma/x"));
        assert!(catalog.default.efforts.is_empty());
        // Half a default is no default.
        let half = parse_pi_settings(r#"{"defaultModel":"x"}"#);
        assert_eq!(
            pi_catalog(parse_pi_list(PI_LIST), &half).default.label,
            None
        );
        for text in ["", "[]", "{", r#"{"defaultModel": 7}"#] {
            let settings = parse_pi_settings(text);
            assert!(settings.model.is_none() || settings.provider.is_none());
        }
    }

    // ------------------------------------------------------------ Grok

    #[test]
    fn the_grok_list_has_ids_a_default_and_no_efforts() {
        let listing = parse_grok_models(GROK_MODELS);
        assert_eq!(ids(&listing), ["model-a", "model-b", "model-c"]);
        assert_eq!(listing.default_id.as_deref(), Some("model-a"));
        assert!(listing.models.iter().all(|m| m.efforts.is_empty()));
        assert_eq!(listing.label_of("model-b").as_deref(), Some("model-b"));
        let catalog = grok_catalog(listing);
        assert_eq!(catalog.default.label.as_deref(), Some("model-a"));
        assert!(catalog.default.efforts.is_empty());
    }

    #[test]
    fn grok_marks_its_default_by_bullet_or_marker_when_no_default_line_is_printed() {
        let text = "Available models:\n  - model-a\n  * model-b (default)\n";
        let listing = parse_grok_models(text);
        assert_eq!(listing.default_id.as_deref(), Some("model-b"));
        let text = "Available models:\n  - model-a\n  - model-b (default)\n";
        assert_eq!(
            parse_grok_models(text).default_id.as_deref(),
            Some("model-b")
        );
        // The `Default model:` line wins over a bullet.
        let text =
            "Default model: model-a\nAvailable models:\n  * model-b (default)\n  - model-a\n";
        assert_eq!(
            parse_grok_models(text).default_id.as_deref(),
            Some("model-a")
        );
    }

    #[test]
    fn grok_lines_before_the_header_and_garbage_are_not_models() {
        assert_eq!(parse_grok_models(""), Listing::default());
        assert_eq!(
            parse_grok_models("- looks-like-a-model\n* another\n"),
            Listing::default()
        );
        let listing = parse_grok_models("Available models:\n  - --yolo\n  -\n  *\n  - ok-1\n");
        assert_eq!(ids(&listing), ["ok-1"]);
        // A default that is not listed is not kept.
        let listing = parse_grok_models("Default model: ghost\nAvailable models:\n  - model-a\n");
        assert_eq!(listing.default_id, None);
    }

    // ------------------------------------------------------------ Codex

    #[test]
    fn codex_pages_give_models_efforts_and_the_default() {
        let listing = parse_codex_models(&codex_entries());
        assert_eq!(ids(&listing), ["model-a", "model-b", "model-c"]);
        assert_eq!(listing.default_id.as_deref(), Some("model-a"));
        let a = &listing.models[0];
        assert_eq!(a.label, "Model A");
        assert_eq!(a.note.as_deref(), Some("The flagship model."));
        assert_eq!(a.efforts, ["low", "medium", "high", "xhigh"]);
        assert_eq!(a.default_effort.as_deref(), Some("medium"));
        // `model/list` gives a slug under `model`; `id` is only the fallback.
        let b = &listing.models[1];
        assert_eq!(b.id, "model-b");
        assert_eq!(b.label, "model-b");
        assert!(b.efforts.is_empty());
    }

    #[test]
    fn hidden_codex_models_are_not_offered() {
        let listing = parse_codex_models(&codex_entries());
        assert!(!ids(&listing).contains(&"model-hidden"));
    }

    #[test]
    fn codex_entries_that_are_not_models_are_skipped() {
        let entries: Vec<Value> = vec![
            serde_json::json!(null),
            serde_json::json!(7),
            serde_json::json!({}),
            serde_json::json!({"model": "--yolo"}),
            serde_json::json!({"model": 3}),
            serde_json::json!({"id": "fallback-id"}),
            serde_json::json!({"model": "model-a", "supportedReasoningEfforts": "high"}),
            serde_json::json!({"model": "model-b", "supportedReasoningEfforts": [{"x": 1}, {"reasoningEffort": "Bad"}, {"reasoningEffort": "low"}]}),
        ];
        let listing = parse_codex_models(&entries);
        assert_eq!(ids(&listing), ["fallback-id", "model-a", "model-b"]);
        assert_eq!(listing.models[2].efforts, ["low"]);
        assert!(listing.models[1].efforts.is_empty());
        assert_eq!(parse_codex_models(&[]), Listing::default());
    }

    #[test]
    fn codex_default_comes_from_the_config_before_the_catalog() {
        let config = parse_codex_config(CODEX_CONFIG);
        assert_eq!(config.model.as_deref(), Some("model-c"));
        assert_eq!(config.effort.as_deref(), Some("xhigh"));
        let catalog = codex_catalog(parse_codex_models(&codex_entries()), &config);
        assert_eq!(catalog.default.label.as_deref(), Some("Model C"));
        assert_eq!(catalog.default.default_effort.as_deref(), Some("xhigh"));
        assert_eq!(catalog.default.configured_effort.as_deref(), Some("xhigh"));
        assert_eq!(catalog.default.efforts, ["low", "high"]);
        // No config: the catalog's own default and its own effort.
        let catalog = codex_catalog(
            parse_codex_models(&codex_entries()),
            &CodexConfig::default(),
        );
        assert_eq!(catalog.default.label.as_deref(), Some("Model A"));
        assert_eq!(catalog.default.default_effort.as_deref(), Some("medium"));
        assert_eq!(catalog.default.configured_effort, None);
    }

    #[test]
    fn a_codex_default_model_missing_from_the_catalog_still_has_a_name() {
        let config = CodexConfig {
            model: Some("model-ghost".into()),
            effort: None,
        };
        let catalog = codex_catalog(parse_codex_models(&codex_entries()), &config);
        assert_eq!(catalog.default.label.as_deref(), Some("model-ghost"));
        assert!(catalog.default.efforts.is_empty());
    }

    #[test]
    fn the_codex_config_reads_two_keys_and_ignores_the_rest() {
        assert_eq!(parse_codex_config(""), CodexConfig::default());
        assert_eq!(parse_codex_config("= = ="), CodexConfig::default());
        let config = parse_codex_config(
            "model = 5\nmodel_reasoning_effort = \"Not Valid\"\n[projects.\"/tmp/x\"]\nmodel = \"nested\"\n",
        );
        assert_eq!(config, CodexConfig::default());
        let config = parse_codex_config("model = \"--yolo\"\n");
        assert_eq!(config.model, None);
    }

    // ------------------------------------------------------------ Claude

    #[test]
    fn claude_offers_its_aliases_with_what_the_settings_map_them_to() {
        let settings = parse_claude_settings(CLAUDE_SETTINGS);
        let catalog = claude_catalog(&settings, Some(CLAUDE_HELP));
        let ids: Vec<&str> = catalog.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["fable", "opus", "sonnet", "haiku"]);
        let note = |id: &str| catalog.model(id).unwrap().note.clone();
        // A display name wins over the raw mapped id.
        assert_eq!(note("opus").as_deref(), Some("\u{2192} Gateway Large"));
        assert_eq!(note("sonnet").as_deref(), Some("\u{2192} gateway-medium"));
        assert_eq!(note("haiku").as_deref(), Some("\u{2192} gateway-small"));
        assert_eq!(note("fable").as_deref(), Some("\u{2192} gateway-next"));
        for model in &catalog.models {
            assert_eq!(model.efforts, ["low", "medium", "high", "xhigh", "max"]);
        }
        assert_eq!(catalog.default.label.as_deref(), Some("opus"));
        assert_eq!(catalog.default.efforts.len(), 5);
    }

    #[test]
    fn fable_appears_only_when_claude_help_names_it() {
        let settings = ClaudeSettings::default();
        let without = claude_catalog(&settings, Some("--model <model>  e.g. 'opus' or 'sonnet'"));
        let ids: Vec<&str> = without.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["opus", "sonnet", "haiku"]);
        assert!(claude_catalog(&settings, None).model("fable").is_none());
        // A word that merely contains it does not count.
        assert!(claude_catalog(&settings, Some("an affable tool"))
            .model("fable")
            .is_none());
        assert!(claude_catalog(&settings, Some("e.g. 'Fable', 'opus'"))
            .model("fable")
            .is_some());
    }

    #[test]
    fn unmapped_claude_aliases_have_no_note() {
        let catalog = claude_catalog(&ClaudeSettings::default(), None);
        assert!(catalog.models.iter().all(|m| m.note.is_none()));
        assert_eq!(catalog.default.label, None);
    }

    #[test]
    fn claude_settings_never_expose_their_credentials() {
        let settings = parse_claude_settings(CLAUDE_SETTINGS);
        let shown = format!("{settings:?}");
        assert!(
            !shown.contains("sk-synthetic-secret-do-not-leak"),
            "{shown}"
        );
        assert!(!shown.contains("gateway.example.invalid"), "{shown}");
        let catalog = claude_catalog(&settings, Some(CLAUDE_HELP));
        let shown = format!("{catalog:?}");
        assert!(
            !shown.contains("sk-synthetic-secret-do-not-leak"),
            "{shown}"
        );
        assert!(!shown.contains("gateway.example.invalid"), "{shown}");
    }

    #[test]
    fn claude_settings_of_the_wrong_shape_read_as_empty() {
        for text in [
            "",
            "[]",
            "{",
            r#"{"model": 3, "env": "x"}"#,
            r#"{"env": {"ANTHROPIC_DEFAULT_OPUS_MODEL": 5}}"#,
            r#"{"env": [1,2]}"#,
        ] {
            let catalog = claude_catalog(&parse_claude_settings(text), None);
            assert!(catalog.models.iter().all(|m| m.note.is_none()), "{text:?}");
            assert_eq!(catalog.default.label, None, "{text:?}");
        }
    }

    #[test]
    fn a_hostile_claude_setting_cannot_reach_the_phone_as_markup_or_a_flag() {
        let text = r#"{"model": "x\u0000\u200b  y", "env": {"ANTHROPIC_DEFAULT_OPUS_MODEL": "a\u202eb\nc"}}"#;
        let catalog = claude_catalog(&parse_claude_settings(text), None);
        assert_eq!(catalog.default.label.as_deref(), Some("x y"));
        assert_eq!(
            catalog.model("opus").unwrap().note.as_deref(),
            Some("\u{2192} ab c")
        );
    }
}
