//! Local app discovery, read-only metadata, and short-lived thread creation.
use super::{Config, SessionMeta, UnavailableReason};
use crate::agents::model_catalog::ModelChoice;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub app: PathBuf,
    pub executable: PathBuf,
    pub home: PathBuf,
    pub socket: PathBuf,
}

pub fn detect(config: &Config) -> Result<Installation, String> {
    inspect(config).map_err(|reason| reason.text(crate::i18n::Lang::En).to_string())
}

pub fn inspect(config: &Config) -> Result<Installation, UnavailableReason> {
    if !cfg!(target_os = "macos") {
        return Err(UnavailableReason::UnsupportedPlatform);
    }
    let apps = if config.app_path.trim().is_empty() {
        vec![
            PathBuf::from("/Applications/Codex.app"),
            PathBuf::from("/Applications/ChatGPT.app"),
            crate::paths::home().join("Applications/Codex.app"),
            crate::paths::home().join("Applications/ChatGPT.app"),
        ]
    } else {
        vec![PathBuf::from(config.app_path.trim())]
    };
    let app_detected = apps.iter().any(|app| app.is_dir());
    let (app, executable) = apps
        .into_iter()
        .find_map(|app| {
            [
                "Contents/Resources/codex-cli/bin/codex",
                "Contents/Resources/codex",
                "Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
            ]
            .iter()
            .map(|s| app.join(s))
            .find(|p| p.is_file())
            .map(|exe| (app, exe))
        })
        .ok_or(if app_detected {
            UnavailableReason::AppRuntimeMissing
        } else {
            UnavailableReason::AppNotDetected
        })?;
    let home = data_home(config);
    if !home.is_absolute() || !home.is_dir() {
        return Err(UnavailableReason::AppDataUnavailable);
    }
    Ok(Installation {
        socket: home.join("ipc/ipc.sock"),
        app,
        executable,
        home,
    })
}

pub(super) fn data_home(config: &Config) -> PathBuf {
    if config.codex_home.trim().is_empty() {
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::paths::home().join(".codex"))
    } else {
        PathBuf::from(config.codex_home.trim())
    }
}

fn thread_database(home: &Path) -> Result<rusqlite::Connection, String> {
    let mut databases: Vec<_> = std::fs::read_dir(home)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                n.to_string_lossy().starts_with("state_")
                    && n.to_string_lossy().ends_with(".sqlite")
            })
        })
        .collect();
    databases.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
    let database = databases
        .last()
        .ok_or("No Codex thread database was found")?;
    let conn = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(300))
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn title_expression(conn: &rusqlite::Connection) -> Result<&'static str, String> {
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(threads)")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok(if columns.iter().any(|c| c == "name") {
        "COALESCE(NULLIF(TRIM(name),''),title,'')"
    } else {
        "COALESCE(title,'')"
    })
}

/// Read the persisted name for an exact session without discovery limits or a live app connection.
pub(super) fn session_title(config: &Config, session_id: &str) -> Option<String> {
    let home = data_home(config);
    if !home.is_absolute() || session_id.trim().is_empty() {
        return None;
    }
    let conn = thread_database(&home).ok()?;
    let title = title_expression(&conn).ok()?;
    let sql = format!("SELECT {title} FROM threads WHERE id=?1");
    let title: String = conn.query_row(&sql, [session_id], |row| row.get(0)).ok()?;
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

pub(super) fn session_workspace(config: &Config, session_id: &str) -> Option<String> {
    let home = data_home(config);
    if !home.is_absolute() || session_id.trim().is_empty() {
        return None;
    }
    let conn = thread_database(&home).ok()?;
    let cwd: String = conn
        .query_row("SELECT cwd FROM threads WHERE id=?1", [session_id], |row| {
            row.get(0)
        })
        .ok()?;
    (!cwd.trim().is_empty()).then_some(cwd)
}

pub fn list(home: &Path) -> Result<Vec<SessionMeta>, String> {
    let conn = thread_database(home)?;
    let title = title_expression(&conn)?;
    let sql = format!("SELECT id, {title}, cwd FROM threads WHERE archived=0 AND (originator IN ('Codex Desktop','codex_work_desktop','codex_mobile_bridge','askhuman') OR (originator IS NULL AND source='vscode')) AND COALESCE(source,'') NOT LIKE '%subagent%' ORDER BY updated_at DESC LIMIT 24");
    let mut statement = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                title: r.get(1)?,
                cwd: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn open(installation: &Installation, id: Option<&str>) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/open");
    command.arg("-a").arg(&installation.app);
    if let Some(id) = id {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid session ID")?;
        command.arg(format!("codex://threads/{id}"));
    }
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not open Codex App".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CreationOutcome {
    Rejected,
    Created,
    Unknown,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationError {
    pub message: String,
    pub stage: String,
    pub method: Option<String>,
    pub code: Option<i64>,
    pub session_id: Option<String>,
    pub outcome: CreationOutcome,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub model: String,
    pub reasoning_effort: String,
}

/// The model and effort a new thread starts with: what the user picked, else what Codex's own
/// configuration says, else what its model catalog calls the default. Pages through `model/list`
/// whenever something has to be looked up.
///
/// With a default `choice` this is exactly what Codex would do on its own. A picked model must
/// be in the catalog; a picked effort must be one that model offers; and an effort left to the
/// configuration is only kept for a picked model that offers it, otherwise that model's own
/// default is used (the configured one would make the thread fail to start).
fn resolve_model_settings<E>(
    config: &Value,
    choice: &ModelChoice,
    mut list_models: impl FnMut(Option<String>) -> Result<Value, E>,
    invalid: impl Fn(String) -> E,
) -> Result<ModelSettings, E> {
    let config = config["config"]
        .as_object()
        .ok_or_else(|| invalid("Codex did not return an effective configuration".into()))?;
    let configured = |key: &str| -> Result<Option<String>, E> {
        match config.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
            _ => Err(invalid(format!("Invalid Codex configuration field: {key}"))),
        }
    };
    let configured_model = configured("model")?;
    let configured_effort = configured("model_reasoning_effort")?;
    if choice.is_default() {
        if let (Some(model), Some(reasoning_effort)) = (&configured_model, &configured_effort) {
            return Ok(ModelSettings {
                model: model.clone(),
                reasoning_effort: reasoning_effort.clone(),
            });
        }
    }
    let wanted = choice.model.clone().or(configured_model);
    let selected = find_catalog_entry(&mut list_models, &invalid, |entry| match &wanted {
        Some(model) => entry["model"] == *model || entry["id"] == *model,
        None => entry["isDefault"] == true,
    })?
    .ok_or_else(|| {
        invalid(if choice.model.is_some() {
            "Codex does not offer the selected model".into()
        } else {
            "Codex's model catalog could not resolve the launch defaults".into()
        })
    })?;
    let model = wanted
        .or_else(|| selected["model"].as_str().map(str::to_owned))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid("Codex did not identify its default model".into()))?;
    let offered: Vec<&str> = selected["supportedReasoningEfforts"]
        .as_array()
        .map(|options| {
            options
                .iter()
                .filter_map(|option| option["reasoningEffort"].as_str())
                .collect()
        })
        .unwrap_or_default();
    let reasoning_effort = match (choice.effort.clone(), configured_effort) {
        (Some(effort), _) => {
            // A catalog entry that lists no efforts says nothing either way.
            if !offered.is_empty() && !offered.contains(&effort.as_str()) {
                return Err(invalid(
                    "The selected reasoning effort is not offered by the model".into(),
                ));
            }
            effort
        }
        (None, Some(effort))
            if choice.model.is_none()
                || offered.is_empty()
                || offered.contains(&effort.as_str()) =>
        {
            effort
        }
        (None, _) => selected["defaultReasoningEffort"]
            .as_str()
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| invalid("Codex did not identify the model's default effort".into()))?,
    };
    Ok(ModelSettings {
        model,
        reasoning_effort,
    })
}

/// The first catalog entry `matches` accepts, paging through `model/list`; `None` when the
/// catalog ends (or starts repeating itself) without one.
fn find_catalog_entry<E>(
    list_models: &mut impl FnMut(Option<String>) -> Result<Value, E>,
    invalid: &impl Fn(String) -> E,
    matches: impl Fn(&Value) -> bool,
) -> Result<Option<Value>, E> {
    let mut cursor = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = list_models(cursor)?;
        let models = page["data"]
            .as_array()
            .ok_or_else(|| invalid("Codex did not return a model catalog".into()))?;
        if let Some(selected) = models.iter().find(|entry| matches(entry)) {
            return Ok(Some(selected.clone()));
        }
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor
            .as_ref()
            .is_none_or(|value| !seen.insert(value.clone()))
        {
            return Ok(None);
        }
    }
}

// ThreadStartParams uses SandboxMode (kebab-case), not turn/start's SandboxPolicy.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum ThreadSandboxMode {
    DangerFullAccess,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadStartParams<'a> {
    cwd: &'a str,
    ephemeral: bool,
    model: &'a str,
    config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_policy: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<ThreadSandboxMode>,
}

fn thread_start_params(cwd: &str, yolo: bool, settings: &ModelSettings) -> Value {
    serde_json::to_value(ThreadStartParams {
        cwd,
        ephemeral: false,
        model: &settings.model,
        config: json!({"model_reasoning_effort":settings.reasoning_effort}),
        approval_policy: yolo.then_some("never"),
        sandbox: yolo.then_some(ThreadSandboxMode::DangerFullAccess),
    })
    .expect("thread start parameters contain only serializable values")
}

pub fn create(
    installation: &Installation,
    cwd: &str,
    title: &str,
    yolo: bool,
    choice: &ModelChoice,
    on_created: impl FnOnce(&str, &ModelSettings) -> Result<(), String>,
) -> Result<String, CreationError> {
    create_or_finish(
        installation,
        cwd,
        title,
        yolo,
        choice,
        None,
        None,
        on_created,
        Duration::from_secs(30),
    )
}

pub fn finish_creation(
    installation: &Installation,
    cwd: &str,
    title: &str,
    id: &str,
    choice: &ModelChoice,
    model_settings: Option<&ModelSettings>,
    on_created: impl FnOnce(&str, &ModelSettings) -> Result<(), String>,
) -> Result<String, CreationError> {
    create_or_finish(
        installation,
        cwd,
        title,
        false,
        choice,
        Some(id),
        model_settings,
        on_created,
        Duration::from_secs(30),
    )
}

#[allow(clippy::too_many_arguments)]
fn create_or_finish(
    installation: &Installation,
    cwd: &str,
    title: &str,
    yolo: bool,
    choice: &ModelChoice,
    existing: Option<&str>,
    model_settings: Option<&ModelSettings>,
    on_created: impl FnOnce(&str, &ModelSettings) -> Result<(), String>,
    response_timeout: Duration,
) -> Result<String, CreationError> {
    let mut session_id = existing.map(str::to_owned);
    let mut start_attempted = false;
    let failure = |message: String,
                   stage: &str,
                   method: Option<&str>,
                   code: Option<i64>,
                   id: &Option<String>,
                   attempted: bool| CreationError {
        message,
        stage: stage.into(),
        method: method.map(str::to_owned),
        code,
        session_id: id.clone(),
        outcome: if id.is_some() {
            CreationOutcome::Created
        } else if !attempted
            || (method == Some("thread/start") && matches!(code, Some(-32600 | -32602)))
        {
            CreationOutcome::Rejected
        } else {
            CreationOutcome::Unknown
        },
    };
    let mut child = Command::new(&installation.executable)
        .args(["app-server", "--listen", "stdio://"])
        .env("CODEX_HOME", &installation.home)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| failure(e.to_string(), "spawn", None, None, &session_id, false))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if tx.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let mut counter = 0;
    let result = (|| {
        let mut request =
            |method: &str, params: Value, id: &Option<String>| -> Result<Value, CreationError> {
                counter += 1;
                if method == "thread/start" {
                    start_attempted = true;
                }
                writeln!(
                    stdin,
                    "{}",
                    json!({"id":counter,"method":method,"params":params})
                )
                .and_then(|_| stdin.flush())
                .map_err(|e| {
                    failure(
                        e.to_string(),
                        "write",
                        Some(method),
                        None,
                        id,
                        start_attempted,
                    )
                })?;
                let deadline = std::time::Instant::now() + response_timeout;
                loop {
                    let response = rx
                        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                        .map_err(|_| {
                            failure(
                                "Thread creation response unavailable".into(),
                                "response",
                                Some(method),
                                None,
                                id,
                                start_attempted,
                            )
                        })?;
                    if response["id"].as_u64() != Some(counter) {
                        continue;
                    }
                    if let Some(error) = response.get("error") {
                        return Err(failure(
                            error["message"]
                                .as_str()
                                .unwrap_or("Thread creation rejected")
                                .into(),
                            "response",
                            Some(method),
                            error["code"].as_i64(),
                            id,
                            start_attempted,
                        ));
                    }
                    if response.get("result").is_none() {
                        return Err(failure(
                            "Invalid creation response".into(),
                            "response",
                            Some(method),
                            None,
                            id,
                            start_attempted,
                        ));
                    }
                    if method == "initialize" {
                        writeln!(stdin, "{{\"method\":\"initialized\"}}")
                            .and_then(|_| stdin.flush())
                            .map_err(|e| {
                                failure(
                                    e.to_string(),
                                    "write",
                                    Some("initialized"),
                                    None,
                                    id,
                                    start_attempted,
                                )
                            })?;
                    }
                    return Ok(response["result"].clone());
                }
            };
        request(
            "initialize",
            json!({"clientInfo":{"name":"askhuman","title":"AskHuman","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),
            &session_id,
        )?;
        let settings = match model_settings {
            Some(settings) => settings.clone(),
            None => {
                let config = request(
                    "config/read",
                    json!({"cwd":cwd,"includeLayers":false}),
                    &session_id,
                )?;
                resolve_model_settings(
                    &config,
                    choice,
                    |cursor| {
                        request(
                            "model/list",
                            json!({"includeHidden":true,"limit":100,"cursor":cursor}),
                            &session_id,
                        )
                    },
                    |message| failure(message, "model-settings", None, None, &session_id, false),
                )?
            }
        };
        if existing.is_none() {
            let result = request(
                "thread/start",
                thread_start_params(cwd, yolo, &settings),
                &session_id,
            )?;
            let id = result["thread"]["id"]
                .as_str()
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                .ok_or_else(|| {
                    failure(
                        "No valid thread ID returned".into(),
                        "thread-id",
                        Some("thread/start"),
                        None,
                        &session_id,
                        true,
                    )
                })?
                .to_string();
            session_id = Some(id.clone());
        }
        let id = session_id.clone().expect("created or existing thread ID");
        if existing.is_none() || model_settings.is_none() {
            // Persist the model snapshot together with the ID before App handoff.
            on_created(&id, &settings).map_err(|e| {
                let mut error = failure(e, "persist-thread", None, None, &session_id, true);
                error.outcome = CreationOutcome::Unknown;
                error
            })?;
        }
        request(
            "thread/name/set",
            json!({"threadId":id,"name":title}),
            &session_id,
        )?;
        request(
            "thread/read",
            json!({"threadId":id,"includeTurns":true}),
            &session_id,
        )?;
        Ok(id)
    })();
    drop(stdin);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let exited = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let _ = reader.join();
    if result.is_ok() && !exited {
        return Err(failure(
            "Creation runtime did not exit cleanly".into(),
            "runtime-exit",
            None,
            None,
            &session_id,
            start_attempted,
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ModelSettings {
        ModelSettings {
            model: "configured-model".into(),
            reasoning_effort: "max".into(),
        }
    }

    #[test]
    fn thread_start_matches_the_bundled_runtime_schema() {
        let schema: Value =
            serde_json::from_str(include_str!("fixtures/thread-start-contract.json")).unwrap();
        let model_schema: Value =
            serde_json::from_str(include_str!("fixtures/model-settings-contract.json")).unwrap();
        let default = thread_start_params("/project", false, &settings());
        assert_eq!(
            default,
            json!({"cwd":"/project","ephemeral":false,"model":"configured-model","config":{"model_reasoning_effort":"max"}})
        );
        let yolo = thread_start_params("/project", true, &settings());
        for field in yolo.as_object().unwrap().keys() {
            assert!(
                schema["properties"].get(field).is_some()
                    || model_schema["threadStartProperties"].get(field).is_some(),
                "unknown field: {field}"
            );
        }
        assert!(schema["definitions"]["SandboxMode"]["enum"]
            .as_array()
            .unwrap()
            .contains(&yolo["sandbox"]));
        assert!(schema["definitions"]["AskForApproval"]["oneOf"][0]["enum"]
            .as_array()
            .unwrap()
            .contains(&yolo["approvalPolicy"]));
    }

    #[test]
    fn configured_model_and_effort_take_priority_over_model_catalog_defaults() {
        let resolved = resolve_model_settings(
            &json!({"config":{"model":"configured-model","model_reasoning_effort":"max"}}),
            &ModelChoice::default(),
            |_| -> Result<Value, String> {
                panic!("explicit defaults do not need the model catalog")
            },
            |message| message,
        )
        .unwrap();
        assert_eq!(resolved, settings());
    }

    #[test]
    fn unset_defaults_resolve_the_selected_model_across_catalog_pages() {
        for (model, effort, expected) in [
            (Some("configured-model"), None, "medium"),
            (None, Some("high"), "high"),
            (None, None, "medium"),
        ] {
            let mut cursors = Vec::new();
            let resolved = resolve_model_settings(
                &json!({"config":{"model":model,"model_reasoning_effort":effort}}),
                &ModelChoice::default(),
                |cursor| -> Result<Value, String> {
                    cursors.push(cursor.clone());
                    Ok(if cursor.is_none() {
                        json!({"data":[{"model":"other-model","isDefault":false,"defaultReasoningEffort":"low"}],"nextCursor":"page-2"})
                    } else {
                        json!({"data":[{"model":"configured-model","isDefault":true,"defaultReasoningEffort":"medium"}],"nextCursor":null})
                    })
                },
                |message| message,
            ).unwrap();
            assert_eq!(resolved.model, "configured-model");
            assert_eq!(resolved.reasoning_effort, expected);
            assert_eq!(cursors, [None, Some("page-2".into())]);
        }
    }

    #[test]
    fn missing_or_malformed_launch_defaults_fail_without_silently_changing_models() {
        for config in [
            json!({}),
            json!({"config":{"model":12}}),
            json!({"config":{"model":"missing-model"}}),
        ] {
            assert!(resolve_model_settings(
                &config,
                &ModelChoice::default(),
                |_| Ok(json!({"data":[],"nextCursor":null})),
                |message| message
            )
            .is_err());
        }
        let mut calls = 0;
        assert!(resolve_model_settings(
            &json!({"config":{}}),
            &ModelChoice::default(),
            |_| {
                calls += 1;
                Ok(json!({"data":[],"nextCursor":"same-page"}))
            },
            |message| message
        )
        .is_err());
        assert_eq!(calls, 2);
    }

    /// `model/list` entries shaped like the app-server schema; `efforts` are the offered ones.
    fn entry(model: &str, is_default: bool, efforts: &[&str], default_effort: &str) -> Value {
        json!({
            "id": format!("id-of-{model}"),
            "model": model,
            "isDefault": is_default,
            "defaultReasoningEffort": default_effort,
            "supportedReasoningEfforts": efforts
                .iter()
                .map(|effort| json!({"reasoningEffort": effort, "description": ""}))
                .collect::<Vec<_>>(),
        })
    }

    fn pick(model: Option<&str>, effort: Option<&str>) -> ModelChoice {
        ModelChoice::new(model.map(str::to_string), effort.map(str::to_string))
    }

    /// Resolves against a two-page catalog and returns the outcome with the cursors asked for.
    fn resolve_in_catalog(
        config: Value,
        choice: &ModelChoice,
    ) -> (Result<ModelSettings, String>, Vec<Option<String>>) {
        let mut cursors = Vec::new();
        let resolved = resolve_model_settings(
            &config,
            choice,
            |cursor| -> Result<Value, String> {
                cursors.push(cursor.clone());
                Ok(if cursor.is_none() {
                    json!({"data":[
                        entry("model-a", true, &["low", "medium", "high"], "medium"),
                    ],"nextCursor":"page-2"})
                } else {
                    json!({"data":[
                        entry("model-b", false, &["medium", "high", "xhigh"], "high"),
                        entry("model-c", false, &[], "low"),
                    ],"nextCursor":null})
                })
            },
            |message| message,
        );
        (resolved, cursors)
    }

    fn settings_of(model: &str, effort: &str) -> ModelSettings {
        ModelSettings {
            model: model.into(),
            reasoning_effort: effort.into(),
        }
    }

    #[test]
    fn a_picked_model_is_looked_up_and_takes_the_configured_effort_only_when_it_offers_it() {
        let configured = |effort: Option<&str>| json!({"config":{"model":"model-a","model_reasoning_effort":effort}});

        // The model on page 2 offers the configured effort: keep it.
        let (resolved, cursors) =
            resolve_in_catalog(configured(Some("high")), &pick(Some("model-b"), None));
        assert_eq!(resolved.unwrap(), settings_of("model-b", "high"));
        assert_eq!(cursors, [None, Some("page-2".into())]);

        // It does not offer it: that model's own default instead of a thread that cannot start.
        let (resolved, _) =
            resolve_in_catalog(configured(Some("low")), &pick(Some("model-b"), None));
        assert_eq!(resolved.unwrap(), settings_of("model-b", "high"));

        // Nothing configured: the model's default.
        let (resolved, _) = resolve_in_catalog(configured(None), &pick(Some("model-a"), None));
        assert_eq!(resolved.unwrap(), settings_of("model-a", "medium"));

        // A model that lists no efforts leaves the configured one alone.
        let (resolved, _) =
            resolve_in_catalog(configured(Some("max")), &pick(Some("model-c"), None));
        assert_eq!(resolved.unwrap(), settings_of("model-c", "max"));

        // Matched by `id` as well as by `model`; the thread gets the model name.
        let (resolved, _) =
            resolve_in_catalog(configured(Some("high")), &pick(Some("id-of-model-b"), None));
        assert_eq!(resolved.unwrap(), settings_of("id-of-model-b", "high"));

        // No model configured at all: the picked one still resolves.
        let (resolved, _) = resolve_in_catalog(json!({"config":{}}), &pick(Some("model-b"), None));
        assert_eq!(resolved.unwrap(), settings_of("model-b", "high"));
    }

    #[test]
    fn a_picked_effort_must_be_offered_by_the_model_it_goes_with() {
        let config = || json!({"config":{"model":"model-a","model_reasoning_effort":"low"}});

        // Both picked.
        let (resolved, _) = resolve_in_catalog(config(), &pick(Some("model-b"), Some("xhigh")));
        assert_eq!(resolved.unwrap(), settings_of("model-b", "xhigh"));
        let (resolved, _) = resolve_in_catalog(config(), &pick(Some("model-b"), Some("low")));
        assert!(resolved.unwrap_err().contains("not offered"));

        // Only the effort: it goes with the configured model, even when both are configured.
        let (resolved, cursors) = resolve_in_catalog(config(), &pick(None, Some("high")));
        assert_eq!(resolved.unwrap(), settings_of("model-a", "high"));
        assert_eq!(cursors, [None]);
        let (resolved, _) = resolve_in_catalog(config(), &pick(None, Some("xhigh")));
        assert!(resolved.unwrap_err().contains("not offered"));

        // Only the effort, with the model left to Codex's default.
        let (resolved, _) = resolve_in_catalog(json!({"config":{}}), &pick(None, Some("high")));
        assert_eq!(resolved.unwrap(), settings_of("model-a", "high"));

        // A model without declared efforts accepts the pick as it is.
        let (resolved, _) = resolve_in_catalog(config(), &pick(Some("model-c"), Some("anything")));
        assert_eq!(resolved.unwrap(), settings_of("model-c", "anything"));
    }

    #[test]
    fn a_model_the_catalog_does_not_offer_is_an_error_not_a_silent_fallback() {
        let (resolved, cursors) = resolve_in_catalog(
            json!({"config":{"model":"model-a","model_reasoning_effort":"low"}}),
            &pick(Some("gone"), Some("low")),
        );
        assert!(resolved.unwrap_err().contains("selected model"));
        assert_eq!(cursors, [None, Some("page-2".into())]);

        // A catalog that repeats a cursor ends the search instead of looping.
        let mut calls = 0;
        assert!(resolve_model_settings(
            &json!({"config":{}}),
            &pick(Some("gone"), None),
            |_| {
                calls += 1;
                Ok::<_, String>(json!({"data":[],"nextCursor":"same-page"}))
            },
            |message| message
        )
        .is_err());
        assert_eq!(calls, 2);
    }

    #[cfg(unix)]
    fn fake_runtime(fault: &str) -> (tempfile::TempDir, Installation) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("app-server");
        let trace = dir.path().join("trace.jsonl");
        let script = format!(
            "#!/usr/bin/env python3\nFAULT={fault:?}\nTRACE={:?}\n",
            trace.to_string_lossy()
        );
        std::fs::write(&executable, script + r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    with open(TRACE, 'a') as f:
        f.write(json.dumps(req) + '\n')
    method = req['method']
    if method == 'initialized':
        continue
    if method == 'thread/start' and FAULT == 'start-drop':
        sys.exit(0)
    if method == 'thread/start' and FAULT == 'start-timeout':
        continue
    rejected = (method == 'initialize' and FAULT == 'initialize-reject') or (method == 'config/read' and FAULT == 'config-reject') or (method == 'thread/start' and FAULT in ['start-reject', 'start-other']) or (method == 'thread/name/set' and FAULT == 'name-reject') or (method == 'thread/read' and FAULT == 'read-reject')
    if rejected:
        resp = {'id':req['id'], 'error':{'code':-32000 if FAULT == 'start-other' else -32602, 'message':'injected rejection'}}
    elif method == 'config/read':
        resp = {'id': req['id'], 'result': {'config': {'model': 'configured-model', 'model_reasoning_effort': 'max'}}}
    elif method == 'model/list':
        resp = {'id': req['id'], 'result': {'data': [{'id': 'model-x', 'model': 'model-x', 'isDefault': False, 'defaultReasoningEffort': 'medium', 'supportedReasoningEfforts': [{'reasoningEffort': 'low', 'description': ''}, {'reasoningEffort': 'medium', 'description': ''}]}], 'nextCursor': None}}
    else:
        resp = {'id':req['id'], 'result':{'thread':{'id':'10000000-0000-4000-8000-000000000001'}}}
    print(json.dumps(resp), flush=True)
sys.exit(1 if FAULT == 'exit' else 0)
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let installation = Installation {
            app: dir.path().into(),
            executable,
            home: dir.path().into(),
            socket: dir.path().join("unused"),
        };
        (dir, installation)
    }

    #[cfg(unix)]
    fn trace(dir: &tempfile::TempDir) -> Vec<Value> {
        std::fs::read_to_string(dir.path().join("trace.jsonl"))
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }

    #[test]
    #[cfg(unix)]
    fn creation_helper_emits_only_empty_thread_setup_and_resume_does_not_start_again() {
        for yolo in [false, true] {
            let (dir, installation) = fake_runtime("success");
            let mut persisted = None;
            let id = create(
                &installation,
                dir.path().to_str().unwrap(),
                "title",
                yolo,
                &ModelChoice::default(),
                |id, settings| {
                    persisted = Some((id.to_string(), settings.clone()));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(persisted, Some((id.clone(), settings())));
            let calls = trace(&dir);
            let methods: Vec<_> = calls
                .iter()
                .map(|v| v["method"].as_str().unwrap())
                .collect();
            assert_eq!(
                methods,
                [
                    "initialize",
                    "initialized",
                    "config/read",
                    "thread/start",
                    "thread/name/set",
                    "thread/read"
                ]
            );
            assert_eq!(
                calls[2]["params"],
                json!({"cwd":dir.path().to_str().unwrap(),"includeLayers":false})
            );
            assert_eq!(
                calls[3]["params"],
                thread_start_params(dir.path().to_str().unwrap(), yolo, &settings())
            );
            finish_creation(
                &installation,
                dir.path().to_str().unwrap(),
                "title",
                &id,
                &ModelChoice::default(),
                Some(&settings()),
                |_, _| panic!("saved settings must be reused"),
            )
            .unwrap();
            assert_eq!(
                trace(&dir)
                    .iter()
                    .filter(|r| r["method"] == "thread/start")
                    .count(),
                1
            );
            assert!(!trace(&dir).iter().any(|r| r["method"] == "turn/start"));
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_picked_model_and_effort_reach_thread_start_and_an_unknown_model_starts_nothing() {
        let (dir, installation) = fake_runtime("success");
        let cwd = dir.path().to_str().unwrap();
        let mut persisted = None;
        let id = create(
            &installation,
            cwd,
            "title",
            false,
            &pick(Some("model-x"), Some("low")),
            |id, settings| {
                persisted = Some((id.to_string(), settings.clone()));
                Ok(())
            },
        )
        .unwrap();
        // The thread and the saved snapshot (which the first turn reuses) carry the same pair.
        let picked = settings_of("model-x", "low");
        assert_eq!(persisted, Some((id, picked.clone())));
        let calls = trace(&dir);
        let methods: Vec<_> = calls
            .iter()
            .map(|v| v["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "config/read",
                "model/list",
                "thread/start",
                "thread/name/set",
                "thread/read"
            ]
        );
        assert_eq!(
            calls[3]["params"],
            json!({"includeHidden":true,"limit":100,"cursor":null})
        );
        assert_eq!(calls[4]["params"], thread_start_params(cwd, false, &picked));

        // A model Codex does not list: rejected before `thread/start`, nothing created.
        let (dir, installation) = fake_runtime("success");
        let err = create(
            &installation,
            dir.path().to_str().unwrap(),
            "title",
            false,
            &pick(Some("gone"), None),
            |_, _| panic!("no thread may be recorded"),
        )
        .unwrap_err();
        assert_eq!(err.outcome, CreationOutcome::Rejected);
        assert_eq!(err.stage, "model-settings");
        assert!(err.session_id.is_none());
        assert!(!trace(&dir).iter().any(|r| r["method"] == "thread/start"));
    }

    #[test]
    #[cfg(unix)]
    fn helper_failure_classification_retains_rpc_method_code_and_created_id() {
        for (fault, outcome, method, code, has_id) in [
            (
                "config-reject",
                CreationOutcome::Rejected,
                Some("config/read"),
                Some(-32602),
                false,
            ),
            (
                "initialize-reject",
                CreationOutcome::Rejected,
                Some("initialize"),
                Some(-32602),
                false,
            ),
            (
                "start-reject",
                CreationOutcome::Rejected,
                Some("thread/start"),
                Some(-32602),
                false,
            ),
            (
                "start-other",
                CreationOutcome::Unknown,
                Some("thread/start"),
                Some(-32000),
                false,
            ),
            (
                "start-drop",
                CreationOutcome::Unknown,
                Some("thread/start"),
                None,
                false,
            ),
            (
                "start-timeout",
                CreationOutcome::Unknown,
                Some("thread/start"),
                None,
                false,
            ),
            (
                "name-reject",
                CreationOutcome::Created,
                Some("thread/name/set"),
                Some(-32602),
                true,
            ),
            (
                "read-reject",
                CreationOutcome::Created,
                Some("thread/read"),
                Some(-32602),
                true,
            ),
            ("exit", CreationOutcome::Created, None, None, true),
        ] {
            let (dir, installation) = fake_runtime(fault);
            let err = create_or_finish(
                &installation,
                dir.path().to_str().unwrap(),
                "title",
                true,
                &ModelChoice::default(),
                None,
                None,
                |_, _| Ok(()),
                // Process startup competes with the full parallel suite. Allow margin
                // while still exercising a bounded wait for the injected timeout.
                Duration::from_secs(5),
            )
            .unwrap_err();
            assert_eq!(err.outcome, outcome, "{fault}: {err:?}");
            assert_eq!(err.method.as_deref(), method, "{fault}: {err:?}");
            assert_eq!(err.code, code, "{fault}: {err:?}");
            assert_eq!(err.session_id.is_some(), has_id, "{fault}: {err:?}");
            assert!(!trace(&dir).iter().any(|r| r["method"] == "turn/start"));
        }
        let (dir, mut installation) = fake_runtime("success");
        let err = create(
            &installation,
            dir.path().to_str().unwrap(),
            "title",
            true,
            &ModelChoice::default(),
            |_, _| Err("disk failure".into()),
        )
        .unwrap_err();
        assert_eq!(err.outcome, CreationOutcome::Unknown);
        assert_eq!(err.stage, "persist-thread");
        assert!(err.session_id.is_some());
        assert_eq!(trace(&dir).len(), 4);
        installation.executable = dir.path().join("missing-runtime");
        let err = create(
            &installation,
            dir.path().to_str().unwrap(),
            "title",
            true,
            &ModelChoice::default(),
            |_, _| Ok(()),
        )
        .unwrap_err();
        assert_eq!(err.outcome, CreationOutcome::Rejected);
        assert_eq!(err.stage, "spawn");
    }

    #[test]
    #[cfg(unix)]
    fn process_faults_drive_durable_receipts_and_resume_without_duplicate_creation() {
        use super::super::actions::Action;
        for fault in [
            "config-reject",
            "start-reject",
            "start-drop",
            "name-reject",
            "read-reject",
            "exit",
        ] {
            let (dir, installation) = fake_runtime(fault);
            let path = dir.path().join("receipt.json");
            let input = json!({"op":"create","id":"operation","text":"task"});
            let mut action = Action::load(path.clone(), input.clone()).unwrap();
            action
                .ensure_thread(
                    &installation,
                    dir.path().to_str().unwrap(),
                    "title",
                    true,
                    &ModelChoice::default(),
                )
                .unwrap_err();
            let mut retry = Action::load(path, input).unwrap();
            if fault == "start-drop" {
                assert!(retry.receipt().is_err());
                assert_eq!(
                    trace(&dir)
                        .iter()
                        .filter(|r| r["method"] == "thread/start")
                        .count(),
                    1
                );
                continue;
            }
            assert!(retry.receipt().unwrap().is_none());
            if retry.session_id().is_some() {
                assert_eq!(retry.model_settings().unwrap(), Some(settings()));
            }
            let script = std::fs::read_to_string(&installation.executable)
                .unwrap()
                .replace(&format!("FAULT={fault:?}"), "FAULT=\"success\"")
                .replace(
                    "'model_reasoning_effort': 'max'",
                    "'model_reasoning_effort': 'high'",
                );
            std::fs::write(&installation.executable, script).unwrap();
            retry
                .ensure_thread(
                    &installation,
                    dir.path().to_str().unwrap(),
                    "title",
                    true,
                    &ModelChoice::default(),
                )
                .unwrap();
            let starts = trace(&dir)
                .iter()
                .filter(|r| r["method"] == "thread/start")
                .count();
            assert_eq!(
                starts,
                if fault == "start-reject" { 2 } else { 1 },
                "{fault}"
            );
            let expected_effort = if matches!(fault, "config-reject" | "start-reject") {
                "high"
            } else {
                "max"
            };
            assert_eq!(
                retry.model_settings().unwrap().unwrap().reasoning_effort,
                expected_effort,
                "{fault}"
            );
        }
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn installation_check_distinguishes_missing_app_runtime_and_data_without_a_socket() {
        let root =
            std::env::temp_dir().join(format!("askhuman-app-check-{}", uuid::Uuid::new_v4()));
        let app = root.join("Codex.app");
        let home = root.join("data");
        let config = Config {
            app_path: app.to_string_lossy().into(),
            codex_home: home.to_string_lossy().into(),
            ..Config::default()
        };
        assert_eq!(
            inspect(&config).err(),
            Some(UnavailableReason::AppNotDetected)
        );
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        assert_eq!(
            inspect(&config).err(),
            Some(UnavailableReason::AppRuntimeMissing)
        );
        std::fs::write(app.join("Contents/Resources/codex"), "test runtime").unwrap();
        assert_eq!(
            inspect(&config).err(),
            Some(UnavailableReason::AppDataUnavailable)
        );
        std::fs::create_dir_all(&home).unwrap();
        let installation = inspect(&config).unwrap();
        assert!(!installation.socket.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn discovery_excludes_terminal_archived_and_subagent_rows() {
        let home =
            std::env::temp_dir().join(format!("askhuman-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT,title TEXT,cwd TEXT,originator TEXT,source TEXT,archived INTEGER,updated_at INTEGER); INSERT INTO threads VALUES ('app','native','/tmp','Codex Desktop','vscode',0,5),('cli','cli','/tmp','codex_cli_rs','cli',0,6),('hidden','archived','/tmp','Codex Desktop','vscode',1,7),('child','child','/tmp','Codex Desktop','subagent',0,8),('created','created','/tmp','askhuman','cli',0,9);").unwrap();
        let ids = list(&home)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["created", "app"]);
        drop(db);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn popup_titles_read_committed_renames_from_wal_for_exact_sessions() {
        let home = tempfile::tempdir().unwrap();
        let config = Config {
            codex_home: home.path().to_string_lossy().into_owned(),
            ..Config::default()
        };
        let db = rusqlite::Connection::open(home.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE threads(id TEXT PRIMARY KEY,name TEXT,title TEXT,cwd TEXT,originator TEXT,source TEXT,archived INTEGER,updated_at INTEGER); INSERT INTO threads VALUES ('older','Old name','Original prompt','/tmp','Codex Desktop','vscode',0,0);").unwrap();
        for i in 1..=25 {
            db.execute("INSERT INTO threads VALUES (?1,NULL,'Recent','/tmp','Codex Desktop','vscode',0,?2)", rusqlite::params![format!("newer-{i}"), i]).unwrap();
        }
        assert!(!list(home.path()).unwrap().iter().any(|r| r.id == "older"));
        assert_eq!(session_title(&config, "older").as_deref(), Some("Old name"));
        db.execute("UPDATE threads SET name='Renamed' WHERE id='older'", [])
            .unwrap();
        assert_eq!(session_title(&config, "older").as_deref(), Some("Renamed"));
        assert_eq!(session_title(&config, "older' OR 1=1 --"), None);
        db.execute("UPDATE threads SET name='  ' WHERE id='older'", [])
            .unwrap();
        assert_eq!(
            session_title(&config, "older").as_deref(),
            Some("Original prompt")
        );
    }

    #[test]
    fn popup_titles_support_old_schemas_and_unavailable_metadata() {
        let home = tempfile::tempdir().unwrap();
        let config = Config {
            codex_home: home.path().to_string_lossy().into_owned(),
            ..Config::default()
        };
        assert_eq!(session_title(&config, "session"), None);
        assert!(!home.path().join("state_5.sqlite").exists());
        let db = rusqlite::Connection::open(home.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY,title TEXT); INSERT INTO threads VALUES ('session','Legacy name');").unwrap();
        assert_eq!(
            session_title(&config, "session").as_deref(),
            Some("Legacy name")
        );
        db.execute("UPDATE threads SET title=NULL", []).unwrap();
        assert_eq!(session_title(&config, "session"), None);
        db.execute_batch("DROP TABLE threads;").unwrap();
        assert_eq!(session_title(&config, "session"), None);
    }
}
