//! A look at this machine's real Agent data, run by hand:
//!
//! ```text
//! cargo test --manifest-path src-tauri/Cargo.toml smoke -- --ignored --nocapture --test-threads=1
//! ```
//!
//! It only reads: the Agents' configuration files, `pi --list-models` and the `--help` of the
//! Agents. It starts no Agent session and sends no prompt. It never touches Cursor, never runs
//! `grok models` and never starts Codex's `app-server` (the last two need the user's real
//! credentials, so they are left to a run on the phone). What it prints is counts and yes / no
//! only, never the name of a model, so the output is safe to paste anywhere.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::parse::{parse_codex_config, parse_pi_list};
use super::source::{load_with, CatalogSource, ConfigFile, Probes, System, LISTING_TIMEOUT};
use crate::agents::AgentKind;

/// How long an Agent may take to print its `--help`.
const HELP_TIMEOUT: Duration = Duration::from_secs(8);

/// [`System`], limited to what the smoke test is allowed to do: a `--help`, `pi --list-models`
/// and reading configuration files other than Cursor's. Anything else fails the test.
struct ReadOnly;

fn is_agent(executable: &str, name: &str) -> bool {
    Path::new(executable)
        .file_name()
        .is_some_and(|file| file == name)
}

fn yes(flag: bool) -> &'static str {
    if flag {
        "yes"
    } else {
        "no"
    }
}

impl Probes for ReadOnly {
    fn listing(&self, executable: &str, args: &[&str], timeout: Duration) -> Option<String> {
        let help = args == ["--help"];
        let pi_list = is_agent(executable, "pi") && args == ["--list-models"];
        assert!(
            help || pi_list,
            "the smoke test may only run `--help` and `pi --list-models`, not `{executable} {args:?}`"
        );
        System.listing(executable, args, timeout)
    }

    fn codex_models(
        &self,
        _executable: &Path,
        _home: &Path,
        _timeout: Duration,
    ) -> Result<Vec<Value>, String> {
        Err("the smoke test never starts Codex's app-server".into())
    }

    fn config_text(&self, file: ConfigFile, codex_home: Option<&Path>) -> Option<String> {
        if file == ConfigFile::CursorCli {
            return None;
        }
        System.config_text(file, codex_home)
    }
}

fn source(kind: AgentKind, executable: &str) -> CatalogSource {
    CatalogSource {
        kind,
        executable: executable.to_string(),
        codex_home: None,
    }
}

#[test]
#[ignore = "reads this machine's real Agent data: run by hand"]
fn this_machines_claude_catalog() {
    let catalog = load_with(&ReadOnly, &source(AgentKind::Claude, "claude"))
        .expect("Claude's catalog is built from fixed aliases and cannot fail");
    let with_efforts = catalog
        .models
        .iter()
        .filter(|m| !m.efforts.is_empty())
        .count();
    println!(
        "claude: {} models ({} with an effort row), fable offered: {}; default row: label {}, {} efforts",
        catalog.models.len(),
        with_efforts,
        yes(catalog.models.iter().any(|m| m.id == "fable")),
        yes(catalog.default.label.is_some()),
        catalog.default.efforts.len(),
    );
    assert!(
        catalog.models.len() >= 3,
        "opus, sonnet and haiku are always offered"
    );
}

#[test]
#[ignore = "reads this machine's real Agent data: run by hand"]
fn this_machines_pi_catalog() {
    let Some(text) = ReadOnly.listing("pi", &["--list-models"], LISTING_TIMEOUT) else {
        println!("pi: no listing (not installed, or it printed nothing)");
        return;
    };
    let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
    let listing = parse_pi_list(&text);
    let thinking = listing
        .models
        .iter()
        .filter(|m| !m.efforts.is_empty())
        .count();
    println!(
        "pi --list-models: {lines} lines, {} models parsed, {thinking} with thinking",
        listing.models.len()
    );
    assert!(
        lines < 2 || !listing.models.is_empty(),
        "pi printed {lines} lines and none parsed as a model: has the table changed?"
    );
    let catalog = load_with(&ReadOnly, &source(AgentKind::Pi, "pi"));
    println!(
        "pi catalog: {}; default row: label {}",
        match &catalog {
            Ok(catalog) => format!("{} models", catalog.models.len()),
            Err(why) => format!("none ({why})"),
        },
        yes(catalog.as_ref().is_ok_and(|c| c.default.label.is_some())),
    );
}

#[test]
#[ignore = "reads this machine's real Agent data: run by hand"]
fn this_machines_codex_default_comes_from_its_config() {
    let Some((_, home)) = crate::codex_desktop::catalog_runtime(
        crate::codex_desktop::LaunchTarget::Terminal,
        Some("codex"),
    ) else {
        println!("codex: no data directory");
        return;
    };
    let text = ReadOnly.config_text(ConfigFile::CodexConfig, Some(&home));
    let config = parse_codex_config(text.as_deref().unwrap_or_default());
    println!(
        "codex config: file read: {}, model set: {}, effort set: {}",
        yes(text.is_some()),
        yes(config.model.is_some()),
        yes(config.effort.is_some()),
    );
}

#[test]
#[ignore = "reads this machine's real Agent data: run by hand"]
fn the_installed_agents_name_the_flags_a_launch_passes() {
    let expected = [
        (AgentKind::Claude, "claude", ["--model", "--effort"]),
        (AgentKind::Codex, "codex", ["--model", "--config"]),
        (AgentKind::Grok, "grok", ["--model", "--reasoning-effort"]),
        (AgentKind::Pi, "pi", ["--model", "--thinking"]),
    ];
    let mut missing = Vec::new();
    for (kind, executable, flags) in expected {
        let Some(help) = ReadOnly.listing(executable, &["--help"], HELP_TIMEOUT) else {
            println!(
                "{}: no --help (not installed, or it printed nothing)",
                kind.as_str()
            );
            continue;
        };
        for flag in flags {
            let found = help.contains(flag);
            println!("{}: {flag} in --help: {}", kind.as_str(), yes(found));
            if !found {
                missing.push(format!("{} {flag}", kind.as_str()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "installed Agents whose --help does not name a flag the launch passes: {missing:?}"
    );
}
