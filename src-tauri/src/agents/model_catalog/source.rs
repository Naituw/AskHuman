//! Where each Agent's catalog comes from: a CLI listing or the Agent's own configuration files.
//!
//! Everything that touches the system goes through [`Probes`], so the logic of what to ask whom
//! is tested against a script, and the real implementation ([`System`]) is a thin shell. Nothing
//! here starts an Agent session or sends a prompt: the commands are listings (`--list-models`,
//! `models`, `--help`) and Codex's `model/list`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::parse::{
    claude_catalog, claude_catalog_with, codex_catalog, codex_default, cursor_catalog,
    cursor_default, grok_catalog, parse_claude_settings, parse_codex_config, parse_codex_models,
    parse_cursor_config, parse_cursor_list, parse_grok_models, parse_pi_list, parse_pi_settings,
    pi_catalog, pi_default,
};
use super::AgentCatalog;
use crate::agents::AgentKind;

/// How long a listing command may run.
pub const LISTING_TIMEOUT: Duration = Duration::from_secs(8);
/// How long `claude --help` may run (it only tells whether `fable` exists).
const HELP_TIMEOUT: Duration = Duration::from_secs(4);
/// The largest config file read; a bigger one is not read at all.
const CONFIG_MAX_BYTES: u64 = 1024 * 1024;

/// Where one Agent's catalog is read from right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSource {
    pub kind: AgentKind,
    /// The Agent's executable, as the login shell resolves it.
    pub executable: String,
    /// Codex only: the data directory (`CODEX_HOME`) `app-server` runs with.
    pub codex_home: Option<PathBuf>,
}

impl CatalogSource {
    /// Two sources with the same key give the same catalog, so they share a cache slot.
    pub fn cache_key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.kind.as_str(),
            self.executable,
            self.codex_home
                .as_deref()
                .map(|home| home.to_string_lossy())
                .unwrap_or_default()
        )
    }
}

/// The configuration files that say what "Default" is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFile {
    ClaudeSettings,
    CodexConfig,
    CursorCli,
    PiSettings,
}

/// Everything [`load_with`] needs from the machine.
pub trait Probes {
    /// What `executable args…` printed (stdout, then stderr); `None` when it could not be run or
    /// printed nothing.
    fn listing(&self, executable: &str, args: &[&str], timeout: Duration) -> Option<String>;
    /// The entries of Codex's `model/list`, all pages.
    fn codex_models(
        &self,
        executable: &Path,
        home: &Path,
        timeout: Duration,
    ) -> Result<Vec<Value>, String>;
    /// The text of a configuration file; `None` when it is missing or unreadable.
    fn config_text(&self, file: ConfigFile, codex_home: Option<&Path>) -> Option<String>;
}

/// The catalog of `source`. `Err` when the Agent could not be asked or said nothing usable, so
/// the caller keeps whatever it knew before.
pub fn load_with(probes: &dyn Probes, source: &CatalogSource) -> Result<AgentCatalog, String> {
    let exe = source.executable.as_str();
    let codex_home = source.codex_home.as_deref();
    let config = |file| probes.config_text(file, codex_home).unwrap_or_default();
    let listing = |args: &[&str]| {
        probes.listing(exe, args, LISTING_TIMEOUT).ok_or_else(|| {
            format!(
                "`{} {}` gave no answer",
                source.kind.as_str(),
                args.join(" ")
            )
        })
    };
    let non_empty = |catalog: AgentCatalog| {
        if catalog.models.is_empty() {
            Err(format!("{} offered no usable models", source.kind.label()))
        } else {
            Ok(catalog)
        }
    };
    match source.kind {
        AgentKind::Claude => {
            // Claude Code has no list command: the aliases are fixed, `--help` only tells whether
            // the newest one exists in this version.
            let help = probes.listing(exe, &["--help"], HELP_TIMEOUT);
            let settings = parse_claude_settings(&config(ConfigFile::ClaudeSettings));
            Ok(claude_catalog(&settings, help.as_deref()))
        }
        AgentKind::Codex => {
            let home = codex_home.ok_or("Codex has no data directory")?;
            let entries = probes.codex_models(Path::new(exe), home, LISTING_TIMEOUT)?;
            let models = parse_codex_models(&entries);
            let catalog = codex_catalog(
                models,
                &parse_codex_config(&config(ConfigFile::CodexConfig)),
            );
            non_empty(catalog)
        }
        AgentKind::Cursor => {
            let models = parse_cursor_list(&listing(&["--list-models"])?);
            let catalog =
                cursor_catalog(models, &parse_cursor_config(&config(ConfigFile::CursorCli)));
            non_empty(catalog)
        }
        AgentKind::Grok => non_empty(grok_catalog(parse_grok_models(&listing(&["models"])?))),
        AgentKind::Pi => {
            let models = parse_pi_list(&listing(&["--list-models"])?);
            let catalog = pi_catalog(models, &parse_pi_settings(&config(ConfigFile::PiSettings)));
            non_empty(catalog)
        }
    }
}

/// [`load_with`] on this machine.
pub fn load(source: &CatalogSource) -> Result<AgentCatalog, String> {
    load_with(&System, source)
}

/// `catalog` with what "Default" stands for read again from the Agent's configuration files.
///
/// The listing is the slow part and is trusted for half an hour; the settings are a few KB that
/// the user (or the Agent) may change at any time, so they are read on every ask and a model
/// switched to a minute ago is what the phone sees (spec `ios-task-model` M12). Nothing is
/// listed again: the models stay as they were cached, except Claude's, whose aliases and notes
/// are made of the settings themselves.
pub fn refresh_default_with(
    probes: &dyn Probes,
    source: &CatalogSource,
    catalog: &AgentCatalog,
) -> AgentCatalog {
    // A catalog that could not be listed has nothing for "Default" to point at.
    if catalog.models.is_empty() {
        return catalog.clone();
    }
    let codex_home = source.codex_home.as_deref();
    let config = |file| probes.config_text(file, codex_home).unwrap_or_default();
    let listed = catalog.listed_default.as_deref();
    let default = match source.kind {
        AgentKind::Claude => {
            let settings = parse_claude_settings(&config(ConfigFile::ClaudeSettings));
            return claude_catalog_with(&settings, catalog.model("fable").is_some());
        }
        AgentKind::Codex => codex_default(
            &catalog.models,
            listed,
            &parse_codex_config(&config(ConfigFile::CodexConfig)),
        ),
        AgentKind::Cursor => cursor_default(
            &catalog.models,
            listed,
            &parse_cursor_config(&config(ConfigFile::CursorCli)),
        ),
        AgentKind::Pi => pi_default(
            &catalog.models,
            &parse_pi_settings(&config(ConfigFile::PiSettings)),
        ),
        // Its default comes from the listing alone.
        AgentKind::Grok => return catalog.clone(),
    };
    AgentCatalog {
        models: catalog.models.clone(),
        default,
        listed_default: catalog.listed_default.clone(),
    }
}

/// [`refresh_default_with`] on this machine.
pub fn refresh_default(source: &CatalogSource, catalog: &AgentCatalog) -> AgentCatalog {
    refresh_default_with(&System, source, catalog)
}

/// The real machine.
pub struct System;

impl Probes for System {
    fn listing(&self, executable: &str, args: &[&str], timeout: Duration) -> Option<String> {
        crate::integrations::agent_launch::run_listing(executable, args, timeout)
    }

    fn codex_models(
        &self,
        executable: &Path,
        home: &Path,
        timeout: Duration,
    ) -> Result<Vec<Value>, String> {
        crate::codex_desktop::list_models(executable, home, timeout)
    }

    fn config_text(&self, file: ConfigFile, codex_home: Option<&Path>) -> Option<String> {
        let path = config_path(file, codex_home, &crate::paths::home())?;
        read_small_file(&path)
    }
}

fn config_path(file: ConfigFile, codex_home: Option<&Path>, home: &Path) -> Option<PathBuf> {
    Some(match file {
        ConfigFile::ClaudeSettings => std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(".claude"))
            .join("settings.json"),
        ConfigFile::CodexConfig => codex_home?.join("config.toml"),
        ConfigFile::CursorCli => home.join(".cursor").join("cli-config.json"),
        ConfigFile::PiSettings => home.join(".pi").join("agent").join("settings.json"),
    })
}

/// A regular file of at most [`CONFIG_MAX_BYTES`], as text.
fn read_small_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > CONFIG_MAX_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

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

    /// A machine that answers from a script and remembers what it was asked.
    #[derive(Default)]
    struct Scripted {
        listing: Option<String>,
        help: Option<String>,
        codex: Option<Result<Vec<Value>, String>>,
        configs: Vec<(ConfigFile, &'static str)>,
        asked: RefCell<Vec<String>>,
    }

    impl Probes for Scripted {
        fn listing(&self, executable: &str, args: &[&str], _timeout: Duration) -> Option<String> {
            self.asked
                .borrow_mut()
                .push(format!("{executable} {}", args.join(" ")));
            if args == ["--help"] {
                self.help.clone()
            } else {
                self.listing.clone()
            }
        }

        fn codex_models(
            &self,
            executable: &Path,
            home: &Path,
            _timeout: Duration,
        ) -> Result<Vec<Value>, String> {
            self.asked.borrow_mut().push(format!(
                "app-server {} {}",
                executable.display(),
                home.display()
            ));
            self.codex
                .clone()
                .unwrap_or_else(|| Err("not scripted".into()))
        }

        fn config_text(&self, file: ConfigFile, _codex_home: Option<&Path>) -> Option<String> {
            self.configs
                .iter()
                .find(|(candidate, _)| *candidate == file)
                .map(|(_, text)| text.to_string())
        }
    }

    fn source(kind: AgentKind) -> CatalogSource {
        CatalogSource {
            kind,
            executable: format!("/opt/bin/{}", kind.as_str()),
            codex_home: (kind == AgentKind::Codex).then(|| PathBuf::from("/data/codex")),
        }
    }

    fn ids(catalog: &AgentCatalog) -> Vec<&str> {
        catalog.models.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn cursor_runs_its_list_command_and_reads_its_default_from_the_config() {
        let probes = Scripted {
            listing: Some(CURSOR_LIST.replace("{ZWSP}", "")),
            configs: vec![(ConfigFile::CursorCli, CURSOR_CONFIG)],
            ..Scripted::default()
        };
        let catalog = load_with(&probes, &source(AgentKind::Cursor)).unwrap();
        assert_eq!(catalog.models.len(), 7);
        assert_eq!(catalog.default.label.as_deref(), Some("Model A"));
        assert_eq!(*probes.asked.borrow(), ["/opt/bin/cursor --list-models"]);
    }

    #[test]
    fn grok_runs_models_and_pi_runs_list_models() {
        let probes = Scripted {
            listing: Some(GROK_MODELS.to_string()),
            ..Scripted::default()
        };
        let catalog = load_with(&probes, &source(AgentKind::Grok)).unwrap();
        assert_eq!(ids(&catalog), ["model-a", "model-b", "model-c"]);
        assert_eq!(*probes.asked.borrow(), ["/opt/bin/grok models"]);

        let probes = Scripted {
            listing: Some(PI_LIST.to_string()),
            configs: vec![(ConfigFile::PiSettings, PI_SETTINGS)],
            ..Scripted::default()
        };
        let catalog = load_with(&probes, &source(AgentKind::Pi)).unwrap();
        assert_eq!(ids(&catalog).len(), 3);
        assert_eq!(catalog.default.label.as_deref(), Some("alpha/model-a"));
        assert_eq!(*probes.asked.borrow(), ["/opt/bin/pi --list-models"]);
    }

    #[test]
    fn codex_asks_app_server_for_its_models_with_the_data_directory() {
        let page: Value = serde_json::from_str(CODEX_PAGE_1).unwrap();
        let probes = Scripted {
            codex: Some(Ok(page["data"].as_array().unwrap().clone())),
            configs: vec![(ConfigFile::CodexConfig, CODEX_CONFIG)],
            ..Scripted::default()
        };
        let catalog = load_with(&probes, &source(AgentKind::Codex)).unwrap();
        // Hidden models are not offered.
        assert_eq!(ids(&catalog), ["model-a"]);
        assert_eq!(catalog.default.configured_effort.as_deref(), Some("xhigh"));
        assert_eq!(
            *probes.asked.borrow(),
            ["app-server /opt/bin/codex /data/codex"]
        );
    }

    #[test]
    fn claude_needs_no_list_command_and_asks_help_only_for_the_newest_alias() {
        let probes = Scripted {
            help: Some(CLAUDE_HELP.to_string()),
            configs: vec![(ConfigFile::ClaudeSettings, CLAUDE_SETTINGS)],
            ..Scripted::default()
        };
        let catalog = load_with(&probes, &source(AgentKind::Claude)).unwrap();
        assert_eq!(ids(&catalog), ["fable", "opus", "sonnet", "haiku"]);
        assert_eq!(*probes.asked.borrow(), ["/opt/bin/claude --help"]);
        // `--help` that does not answer still leaves the stable aliases.
        let probes = Scripted::default();
        let catalog = load_with(&probes, &source(AgentKind::Claude)).unwrap();
        assert_eq!(ids(&catalog), ["opus", "sonnet", "haiku"]);
    }

    #[test]
    fn a_listing_that_gives_no_answer_or_nothing_usable_is_an_error() {
        for kind in [AgentKind::Cursor, AgentKind::Grok, AgentKind::Pi] {
            let silent = Scripted::default();
            assert!(load_with(&silent, &source(kind)).is_err(), "{kind:?}");
            let noise = Scripted {
                listing: Some("error: not logged in\n".into()),
                ..Scripted::default()
            };
            assert!(load_with(&noise, &source(kind)).is_err(), "{kind:?}");
        }
        let failing = Scripted {
            codex: Some(Err("app-server did not start".into())),
            ..Scripted::default()
        };
        assert_eq!(
            load_with(&failing, &source(AgentKind::Codex)).unwrap_err(),
            "app-server did not start"
        );
        let empty = Scripted {
            codex: Some(Ok(Vec::new())),
            ..Scripted::default()
        };
        assert!(load_with(&empty, &source(AgentKind::Codex)).is_err());
    }

    #[test]
    fn codex_without_a_data_directory_is_an_error_and_never_asked() {
        let probes = Scripted::default();
        let mut src = source(AgentKind::Codex);
        src.codex_home = None;
        assert!(load_with(&probes, &src).is_err());
        assert!(probes.asked.borrow().is_empty());
    }

    #[test]
    fn a_missing_config_only_costs_the_default_label() {
        let probes = Scripted {
            listing: Some(CURSOR_LIST.replace("{ZWSP}", "")),
            ..Scripted::default()
        };
        let catalog = load_with(&probes, &source(AgentKind::Cursor)).unwrap();
        assert_eq!(catalog.default.label.as_deref(), Some("Auto"));
    }

    #[test]
    fn cache_keys_tell_sources_apart() {
        let a = source(AgentKind::Codex);
        let mut b = a.clone();
        b.codex_home = Some(PathBuf::from("/other"));
        let mut c = a.clone();
        c.executable = "/elsewhere/codex".into();
        let keys = [a.cache_key(), b.cache_key(), c.cache_key()];
        assert_eq!(
            keys.iter().collect::<std::collections::HashSet<_>>().len(),
            3
        );
        assert_eq!(a.cache_key(), a.clone().cache_key());
        assert_ne!(
            source(AgentKind::Grok).cache_key(),
            source(AgentKind::Pi).cache_key()
        );
    }

    #[test]
    fn config_paths_follow_each_agents_own_layout() {
        let home = Path::new("/home/u");
        assert_eq!(
            config_path(ConfigFile::CursorCli, None, home).unwrap(),
            Path::new("/home/u/.cursor/cli-config.json")
        );
        assert_eq!(
            config_path(ConfigFile::PiSettings, None, home).unwrap(),
            Path::new("/home/u/.pi/agent/settings.json")
        );
        assert_eq!(
            config_path(
                ConfigFile::CodexConfig,
                Some(Path::new("/data/codex")),
                home
            )
            .unwrap(),
            Path::new("/data/codex/config.toml")
        );
        assert!(config_path(ConfigFile::CodexConfig, None, home).is_none());
    }

    // ------------------------------------------------------------ "Default" follows the config

    /// A machine whose only answers are these configuration files; anything it is asked to run
    /// shows up in `asked`.
    fn config_only(file: ConfigFile, text: &'static str) -> Scripted {
        Scripted {
            configs: vec![(file, text)],
            ..Scripted::default()
        }
    }

    /// Codex's catalog as cached while `config.toml` named model-c and pinned `xhigh`.
    fn cached_codex() -> AgentCatalog {
        let entries: Vec<Value> = [CODEX_PAGE_1, CODEX_PAGE_2]
            .iter()
            .flat_map(|page| {
                let page: Value = serde_json::from_str(page).unwrap();
                page["data"].as_array().unwrap().clone()
            })
            .collect();
        let probes = Scripted {
            codex: Some(Ok(entries)),
            configs: vec![(ConfigFile::CodexConfig, CODEX_CONFIG)],
            ..Scripted::default()
        };
        load_with(&probes, &source(AgentKind::Codex)).unwrap()
    }

    #[test]
    fn a_changed_codex_config_moves_the_default_without_asking_app_server_again() {
        let cached = cached_codex();
        assert_eq!(ids(&cached), ["model-a", "model-b", "model-c"]);
        assert_eq!(cached.default.configured_effort.as_deref(), Some("xhigh"));
        let model_a = cached.model("model-a").unwrap();

        // The user switched to model-a and pinned `low`.
        let probes = config_only(
            ConfigFile::CodexConfig,
            "model = \"model-a\"\nmodel_reasoning_effort = \"low\"\n",
        );
        let now = refresh_default_with(&probes, &source(AgentKind::Codex), &cached);
        assert_eq!(now.models, cached.models);
        assert_eq!(now.default.label.as_deref(), Some(model_a.label.as_str()));
        assert_eq!(now.default.efforts, model_a.efforts);
        assert_eq!(now.default.configured_effort.as_deref(), Some("low"));
        assert_eq!(now.default.default_effort.as_deref(), Some("low"));
        assert!(probes.asked.borrow().is_empty(), "nothing is listed again");

        // The file is gone: the model the listing marks is the default again, run with its own
        // effort and with nothing pinned.
        let none = Scripted::default();
        let now = refresh_default_with(&none, &source(AgentKind::Codex), &cached);
        assert_eq!(now.default.label.as_deref(), Some(model_a.label.as_str()));
        assert_eq!(now.default.configured_effort, None);
        assert_eq!(
            now.default.default_effort.as_deref(),
            model_a.default_effort.as_deref()
        );
        assert!(none.asked.borrow().is_empty());
    }

    #[test]
    fn a_changed_cursor_config_renames_the_default_and_leaves_the_models_alone() {
        let listing = Scripted {
            listing: Some(CURSOR_LIST.replace("{ZWSP}", "")),
            configs: vec![(ConfigFile::CursorCli, CURSOR_CONFIG)],
            ..Scripted::default()
        };
        let cached = load_with(&listing, &source(AgentKind::Cursor)).unwrap();
        assert_eq!(cached.default.label.as_deref(), Some("Model A"));

        let probes = config_only(
            ConfigFile::CursorCli,
            r#"{"model":{"displayName":"Model C"}}"#,
        );
        let now = refresh_default_with(&probes, &source(AgentKind::Cursor), &cached);
        assert_eq!(now.default.label.as_deref(), Some("Model C"));
        assert_eq!(now.models, cached.models);
        assert!(probes.asked.borrow().is_empty());

        // No config: the model the listing marks.
        let now = refresh_default_with(&Scripted::default(), &source(AgentKind::Cursor), &cached);
        assert_eq!(now.default.label.as_deref(), Some("Auto"));
        // Cursor never has an effort row.
        assert!(now.default.efforts.is_empty());
    }

    #[test]
    fn a_changed_pi_setting_moves_the_default_and_its_efforts() {
        let listing = Scripted {
            listing: Some(PI_LIST.to_string()),
            configs: vec![(ConfigFile::PiSettings, PI_SETTINGS)],
            ..Scripted::default()
        };
        let cached = load_with(&listing, &source(AgentKind::Pi)).unwrap();
        assert_eq!(cached.default.label.as_deref(), Some("alpha/model-a"));
        assert_eq!(cached.default.efforts, ["low", "medium", "high"]);

        // A model that does not think gets no effort row.
        let probes = config_only(
            ConfigFile::PiSettings,
            r#"{"defaultProvider":"alpha","defaultModel":"model-b-mini"}"#,
        );
        let now = refresh_default_with(&probes, &source(AgentKind::Pi), &cached);
        assert_eq!(now.default.label.as_deref(), Some("alpha/model-b-mini"));
        assert!(now.default.efforts.is_empty());
        assert_eq!(now.models, cached.models);
        assert!(probes.asked.borrow().is_empty());
    }

    #[test]
    fn claudes_aliases_follow_the_settings_but_fable_only_stays_when_it_was_there() {
        let listing = Scripted {
            help: Some(CLAUDE_HELP.to_string()),
            configs: vec![(ConfigFile::ClaudeSettings, CLAUDE_SETTINGS)],
            ..Scripted::default()
        };
        let cached = load_with(&listing, &source(AgentKind::Claude)).unwrap();
        assert_eq!(ids(&cached), ["fable", "opus", "sonnet", "haiku"]);
        assert_eq!(cached.default.label.as_deref(), Some("opus"));

        let settings =
            r#"{"model":"sonnet","env":{"ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"Mid Tier"}}"#;
        let probes = config_only(ConfigFile::ClaudeSettings, settings);
        let now = refresh_default_with(&probes, &source(AgentKind::Claude), &cached);
        assert_eq!(now.default.label.as_deref(), Some("sonnet"));
        assert_eq!(ids(&now), ["fable", "opus", "sonnet", "haiku"]);
        assert_eq!(
            now.model("sonnet").unwrap().note.as_deref(),
            Some("\u{2192} Mid Tier")
        );
        assert!(probes.asked.borrow().is_empty(), "--help is not run again");

        // A Claude whose `--help` did not name `fable` does not gain it from the settings.
        let plain = load_with(&Scripted::default(), &source(AgentKind::Claude)).unwrap();
        let probes = config_only(ConfigFile::ClaudeSettings, CLAUDE_SETTINGS);
        let now = refresh_default_with(&probes, &source(AgentKind::Claude), &plain);
        assert_eq!(ids(&now), ["opus", "sonnet", "haiku"]);
    }

    #[test]
    fn grok_and_a_catalog_that_could_not_be_listed_keep_what_they_have() {
        let listing = Scripted {
            listing: Some(GROK_MODELS.to_string()),
            ..Scripted::default()
        };
        let cached = load_with(&listing, &source(AgentKind::Grok)).unwrap();
        let now = refresh_default_with(&Scripted::default(), &source(AgentKind::Grok), &cached);
        assert_eq!(now, cached);

        // Nothing was listed, so there is nothing for a configured default to point at.
        for kind in AgentKind::ALL {
            let probes = config_only(ConfigFile::ClaudeSettings, CLAUDE_SETTINGS);
            let now = refresh_default_with(&probes, &source(kind), &AgentCatalog::default());
            assert_eq!(now, AgentCatalog::default(), "{kind:?}");
        }
    }

    #[test]
    fn a_config_file_that_is_not_a_small_regular_file_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok.json");
        std::fs::write(&ok, "{}").unwrap();
        assert_eq!(read_small_file(&ok).as_deref(), Some("{}"));
        assert!(read_small_file(&dir.path().join("missing.json")).is_none());
        assert!(read_small_file(dir.path()).is_none());
        let big = dir.path().join("big.json");
        std::fs::write(&big, vec![b' '; CONFIG_MAX_BYTES as usize + 1]).unwrap();
        assert!(read_small_file(&big).is_none());
        let binary = dir.path().join("binary.json");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert!(read_small_file(&binary).is_none());
    }
}
