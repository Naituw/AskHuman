//! Starting Antigravity tasks from the phone, IM and the New Task window.
//!
//! Antigravity has two ways to start a task: `agy` in a terminal, and the desktop App. The App has
//! no public way to open a conversation in a project of our choosing, but it runs *sidecars*
//! (`~/.gemini/config/sidecars/<id>/`): processes the App starts for a project, with a fixed set
//! of granted actions and an `agentapi` that can create conversations in that project. This
//! module owns the route's configuration and status, the sidecars AskHuman manages for it, and the
//! task-creation operation (docs/specs/antigravity-deep-integration.md §12).

mod create;
mod host_config;
mod launch;
mod link;
mod receipt;
mod runtime;
mod sidecar;
mod sidecar_run;

pub use create::Request as CreateRequest;
pub use launch::{launch_status, resolve_launch, LaunchStatus};
pub use link::{shared as manager, CheckIn, Down, Manager, Outbound, Up};
// What the daemon's link tests stand in for; nothing else outside this module sends commands yet.
#[cfg(test)]
pub use link::{Command, Outcome, RunError};
pub use sidecar_run::main as sidecar_main;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The same choice of route as Codex's and Cursor's, so IM and iOS carry one kind of target for
/// every Agent: `Desktop` is the App, `Terminal` is `agy`.
pub use crate::codex_desktop::LaunchTarget;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Which route a new Antigravity task tries first; the other one is the fallback.
    pub launch_preference: LaunchTarget,
    /// Whether AskHuman may create, change and remove the sidecars the App route needs, and the
    /// `sidecars.askhuman-*` entries of Antigravity's `config.json` that enable them. Off until the
    /// user allows it once; turning it off again does not remove what already exists.
    pub manage_sidecars: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            launch_preference: LaunchTarget::Desktop,
            manage_sidecars: false,
        }
    }
}

// MARK: - Whether the daemon accepts a sidecar

/// Decides whether the daemon takes the sidecar that just checked in: it must be one AskHuman
/// made and speak this version of the link. `Err` is the frame that tells it so.
pub fn admit(check_in: &CheckIn) -> Result<(), Down> {
    admit_in(&sidecar::Layout::current(), check_in)
}

fn admit_in(layout: &sidecar::Layout, check_in: &CheckIn) -> Result<(), Down> {
    let refuse = |reason: String, restart: bool| Down::Refused { reason, restart };
    if !sidecar::valid_id(&check_in.id) {
        return Err(refuse("This is not an AskHuman sidecar".into(), false));
    }
    // An older sidecar is told to exit: the App starts it again from the file on disk, which is
    // the build that is installed now.
    if check_in.link_version < link::LINK_VERSION {
        return Err(refuse(
            "This sidecar is an older build than the daemon".into(),
            true,
        ));
    }
    if check_in.link_version > link::LINK_VERSION {
        return Err(refuse(
            "This sidecar is a newer build than the daemon".into(),
            false,
        ));
    }
    match sidecar::entry(layout, &check_in.id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(refuse(
            "AskHuman does not manage this sidecar".into(),
            false,
        )),
        Err(error) => Err(refuse(
            format!("AskHuman could not read its record of its sidecars: {error}"),
            false,
        )),
    }
}

// MARK: - Operations for Settings and the command line

/// What Settings and `AskHuman agents sidecars` ask the daemon to do with the sidecars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Operation {
    /// Every sidecar AskHuman manages, with its state.
    Sidecars,
    /// Remove one sidecar, whether or not managing is switched on.
    Remove { id: String },
    /// Remove every sidecar AskHuman made.
    RemoveAll,
    /// Replace the rules a whitelisted sidecar has of its own.
    SetRules { id: String, rules: Vec<String> },
    /// Start a task in the App, or (`dryRun`) bring the route up and check it.
    Create(CreateRequest),
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// The AskHuman an App-started sidecar runs: the same program the plugin's hooks and MCP entry
/// name.
fn executable() -> Result<String> {
    Ok(crate::development::routing::program_path()?
        .to_string_lossy()
        .into_owned())
}

/// What the daemon answers. A request the daemon refuses on purpose (`status: "refused"` with a
/// `reason` code and a `message`) is an answer; `Err` is for what went wrong.
pub async fn execute(operation: Operation) -> std::result::Result<Value, String> {
    // Creating a task waits for the App and the sidecar, so it is not a file operation.
    if let Operation::Create(request) = operation {
        return create::execute(request).await;
    }
    // The registry and the sidecar directories are files behind a lock.
    tokio::task::spawn_blocking(move || run(&sidecar::Layout::current(), operation))
        .await
        .map_err(|error| error.to_string())?
}

fn refused(reason: &str, message: impl Into<String>) -> Value {
    json!({"status": "refused", "reason": reason, "message": message.into()})
}

/// An error that is a refusal on purpose, as an answer; any other is a failure.
fn answer(error: anyhow::Error) -> std::result::Result<Value, String> {
    if let Some(limit) = error.downcast_ref::<sidecar::LimitReached>() {
        return Ok(refused("limitReached", limit.to_string()));
    }
    if let Some(conflict) = error.downcast_ref::<sidecar::Conflict>() {
        return Ok(refused("conflict", conflict.to_string()));
    }
    if let Some(unknown) = error.downcast_ref::<sidecar::NotManaged>() {
        return Ok(refused("notManaged", unknown.to_string()));
    }
    if let Some(rules) = error.downcast_ref::<sidecar::BadRules>() {
        return Ok(refused("invalidRules", rules.to_string()));
    }
    Err(format!("{error:#}"))
}

fn run(layout: &sidecar::Layout, operation: Operation) -> std::result::Result<Value, String> {
    match operation {
        // `execute` sends it down the asynchronous path before this function is reached.
        Operation::Create(_) => Err("A task is created asynchronously".into()),
        Operation::Sidecars => {
            let managing = crate::config::AppConfig::load_without_secrets()
                .antigravity_desktop
                .manage_sidecars;
            let listings = sidecar::list(layout).map_err(|error| format!("{error:#}"))?;
            let online = manager().online();
            let sidecars: Vec<Value> = listings
                .iter()
                .map(|listing| {
                    let link = online.get(&listing.id);
                    let mut value = serde_json::to_value(listing).unwrap_or(Value::Null);
                    value["online"] = json!(link.is_some());
                    value["link"] = serde_json::to_value(link).unwrap_or(Value::Null);
                    value
                })
                .collect();
            Ok(json!({
                "status": "done",
                "managing": managing,
                "limit": sidecar::LIMIT,
                "sidecars": sidecars,
            }))
        }
        Operation::Remove { id } => match sidecar::remove(layout, &id) {
            Ok(removed) => Ok(json!({
                "status": "done",
                "removed": [{"id": id, "directory": removed.directory}],
            })),
            Err(error) => answer(error),
        },
        Operation::RemoveAll => {
            let results = sidecar::remove_all(layout).map_err(|error| format!("{error:#}"))?;
            let mut removed = Vec::new();
            let mut failed = Vec::new();
            for (id, result) in results {
                match result {
                    Ok(_) => removed.push(json!({"id": id})),
                    Err(error) => failed.push(json!({"id": id, "message": format!("{error:#}")})),
                }
            }
            Ok(json!({"status": "done", "removed": removed, "failed": failed}))
        }
        Operation::SetRules { id, rules } => {
            let config = crate::config::AppConfig::load_without_secrets().antigravity_desktop;
            if !config.manage_sidecars {
                return Ok(refused(
                    "managingOff",
                    "AskHuman is not allowed to manage Antigravity sidecars",
                ));
            }
            let executable = executable().map_err(|error| format!("{error:#}"))?;
            match sidecar::set_rules(
                layout,
                &executable,
                &id,
                &rules,
                &sidecar::attachment_roots(),
                now_ms(),
            ) {
                Ok(change) => Ok(json!({
                    "status": "done",
                    "change": match change {
                        sidecar::Change::Created => "created",
                        sidecar::Change::Updated => "updated",
                        sidecar::Change::Unchanged => "unchanged",
                    },
                })),
                Err(error) => answer(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_in(id: &str, version: u32) -> CheckIn {
        CheckIn {
            id: id.to_string(),
            link_version: version,
            project_id: None,
            version: "1.0.0".into(),
            agentapi: true,
            pid: 1,
        }
    }

    fn layout_in(dir: &std::path::Path) -> sidecar::Layout {
        sidecar::Layout {
            sidecars: dir.join("gemini/sidecars"),
            config_json: dir.join("gemini/config.json"),
            registry: dir.join("askhuman/antigravity-sidecars.json"),
            instance: dir.join("askhuman").to_string_lossy().into_owned(),
        }
    }

    fn make(layout: &sidecar::Layout, directory: &str) -> String {
        let spec = sidecar::Spec {
            kind: sidecar::Kind::Workspace,
            directory: directory.into(),
            rules: Vec::new(),
        };
        sidecar::ensure(
            layout,
            "/Applications/AskHuman.app/Contents/MacOS/AskHuman",
            &spec,
            &[],
            1,
        )
        .unwrap();
        spec.id()
    }

    #[test]
    fn only_a_sidecar_askhuman_made_and_of_this_link_version_is_admitted() {
        let dir = tempfile::tempdir().unwrap();
        let layout = layout_in(dir.path());
        let id = make(&layout, "/Users/me/project");
        assert_eq!(
            admit_in(&layout, &check_in(&id, link::LINK_VERSION)),
            Ok(())
        );

        let refused = |check_in: CheckIn| match admit_in(&layout, &check_in) {
            Err(Down::Refused { reason, restart }) => (reason, restart),
            other => panic!("not refused: {other:?}"),
        };
        // Not one of ours, by name or by record.
        assert!(!refused(check_in("someone-elses", link::LINK_VERSION)).1);
        assert!(!refused(check_in("../../askhuman-ws-x", link::LINK_VERSION)).1);
        let (reason, restart) = refused(check_in("askhuman-ws-00000000", link::LINK_VERSION));
        assert!(reason.contains("does not manage"), "{reason}");
        assert!(!restart);
        // An older build is asked to restart; a newer one just waits.
        let (_, restart) = refused(check_in(&id, link::LINK_VERSION - 1));
        assert!(restart);
        let (_, restart) = refused(check_in(&id, link::LINK_VERSION + 1));
        assert!(!restart);
    }

    #[test]
    fn a_registry_that_cannot_be_read_refuses_instead_of_admitting() {
        let dir = tempfile::tempdir().unwrap();
        let layout = layout_in(dir.path());
        std::fs::create_dir_all(layout.registry.parent().unwrap()).unwrap();
        std::fs::write(&layout.registry, "not json").unwrap();
        assert!(matches!(
            admit_in(
                &layout,
                &check_in("askhuman-ws-abcd1234", link::LINK_VERSION)
            ),
            Err(Down::Refused { restart: false, .. })
        ));
    }

    #[test]
    fn the_list_shows_each_sidecar_with_whether_it_is_connected() {
        let dir = tempfile::tempdir().unwrap();
        let layout = layout_in(dir.path());
        let id = make(&layout, "/Users/me/project");
        let other = make(&layout, "/Users/me/other");
        let (_generation, _receiver) = manager()
            .attach(&check_in(&id, link::LINK_VERSION), 5)
            .unwrap();
        let answer = run(&layout, Operation::Sidecars).unwrap();
        assert_eq!(answer["status"], "done");
        assert_eq!(answer["limit"], 20);
        let sidecars = answer["sidecars"].as_array().unwrap();
        assert_eq!(sidecars.len(), 2);
        let find = |wanted: &str| {
            sidecars
                .iter()
                .find(|entry| entry["id"] == wanted)
                .unwrap()
                .clone()
        };
        assert_eq!(find(&id)["online"], true);
        assert_eq!(find(&id)["link"]["connectedAt"], 5);
        assert_eq!(find(&other)["online"], false);
        assert!(find(&other)["link"].is_null());
        assert_eq!(find(&other)["kind"], "workspace");
        assert_eq!(find(&other)["directory"], "/Users/me/other");
        manager().detach(&id, 1);
    }

    #[test]
    fn removing_one_and_removing_all_report_what_went() {
        let dir = tempfile::tempdir().unwrap();
        let layout = layout_in(dir.path());
        let first = make(&layout, "/Users/me/a");
        let _second = make(&layout, "/Users/me/b");
        let answer = run(&layout, Operation::Remove { id: first.clone() }).unwrap();
        assert_eq!(answer["status"], "done");
        assert_eq!(answer["removed"][0]["id"], first.as_str());
        assert!(!layout.sidecars.join(&first).exists());
        // A name that is not ours is refused, not an error.
        let answer = run(
            &layout,
            Operation::Remove {
                id: "../../etc".into(),
            },
        )
        .unwrap();
        assert_eq!(answer["status"], "refused");
        assert_eq!(answer["reason"], "notManaged");
        let answer = run(&layout, Operation::RemoveAll).unwrap();
        assert_eq!(answer["removed"].as_array().unwrap().len(), 1);
        assert_eq!(answer["failed"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_conflicting_directory_is_a_refusal_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let layout = layout_in(dir.path());
        let id = make(&layout, "/Users/me/a");
        std::fs::write(layout.sidecars.join(&id).join("sidecar.json"), "edited").unwrap();
        let answer = run(&layout, Operation::Remove { id: id.clone() }).unwrap();
        assert_eq!(answer["status"], "refused");
        assert_eq!(answer["reason"], "conflict");
        // Left as it was.
        assert!(layout.sidecars.join(&id).exists());
    }

    #[test]
    fn operations_read_as_they_are_written_by_the_settings_page() {
        let op: Operation = serde_json::from_value(json!({"op": "sidecars"})).unwrap();
        assert_eq!(op, Operation::Sidecars);
        let op: Operation = serde_json::from_value(
            json!({"op": "setRules", "id": "askhuman-ws-1", "rules": ["command(ls)"]}),
        )
        .unwrap();
        assert_eq!(
            op,
            Operation::SetRules {
                id: "askhuman-ws-1".into(),
                rules: vec!["command(ls)".into()]
            }
        );
        assert!(serde_json::from_value::<Operation>(json!({"op": "somethingElse"})).is_err());
    }

    #[test]
    fn creating_a_task_is_an_operation_in_the_same_envelope() {
        let op: Operation = serde_json::from_value(json!({
            "op": "create",
            "cwd": "/work/app",
            "text": "do it",
            "permission": "agent-default",
            "id": "8d4c2c0a-1111-4222-8333-944455556666",
            "model": "gemini-3.5-flash-low",
            "dryRun": true,
        }))
        .unwrap();
        let Operation::Create(request) = op else {
            panic!("expected a creation");
        };
        assert_eq!(request.model.as_deref(), Some("gemini-3.5-flash-low"));
        assert!(request.dry_run);
        let wire = serde_json::to_value(Operation::Create(request)).unwrap();
        assert_eq!(wire["op"], "create");
        assert_eq!(wire["cwd"], "/work/app");
        assert_eq!(wire["dryRun"], true);
    }

    #[test]
    fn nothing_is_managed_until_the_user_allows_it_and_the_app_goes_first() {
        let config = Config::default();
        assert!(!config.manage_sidecars);
        assert_eq!(config.launch_preference, LaunchTarget::Desktop);
    }

    #[test]
    fn a_config_written_before_the_route_existed_reads_as_the_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(!config.manage_sidecars);
        assert_eq!(config.launch_preference, LaunchTarget::Desktop);
        let value = serde_json::to_value(Config {
            launch_preference: LaunchTarget::Terminal,
            manage_sidecars: true,
        })
        .unwrap();
        assert_eq!(value["launchPreference"], "terminal");
        assert_eq!(value["manageSidecars"], true);
    }
}
