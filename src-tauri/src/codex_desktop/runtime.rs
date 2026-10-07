//! Local app discovery, read-only metadata, and short-lived thread creation.
use super::{Config, SessionMeta, UnavailableReason};
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
    let home = if config.codex_home.trim().is_empty() {
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::paths::home().join(".codex"))
    } else {
        PathBuf::from(config.codex_home.trim())
    };
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

pub fn list(home: &Path) -> Result<Vec<SessionMeta>, String> {
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
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(threads)")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let title = if columns.iter().any(|c| c == "name") {
        "COALESCE(name,title,'')"
    } else {
        "COALESCE(title,'')"
    };
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
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_policy: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<ThreadSandboxMode>,
}

fn thread_start_params(cwd: &str, yolo: bool) -> Value {
    serde_json::to_value(ThreadStartParams {
        cwd,
        ephemeral: false,
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
    on_created: impl FnOnce(&str) -> Result<(), String>,
) -> Result<String, CreationError> {
    create_or_finish(
        installation,
        cwd,
        title,
        yolo,
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
) -> Result<String, CreationError> {
    create_or_finish(
        installation,
        cwd,
        title,
        false,
        Some(id),
        |_| Ok(()),
        Duration::from_secs(30),
    )
}

#[allow(clippy::too_many_arguments)]
fn create_or_finish(
    installation: &Installation,
    cwd: &str,
    title: &str,
    yolo: bool,
    existing: Option<&str>,
    on_created: impl FnOnce(&str) -> Result<(), String>,
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
        if existing.is_none() {
            let result = request("thread/start", thread_start_params(cwd, yolo), &session_id)?;
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
            on_created(&id).map_err(|e| {
                let mut error = failure(e, "persist-thread", None, None, &session_id, true);
                error.outcome = CreationOutcome::Unknown;
                error
            })?;
        }
        let id = session_id.clone().expect("created or existing thread ID");
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

    #[test]
    fn thread_start_matches_the_bundled_runtime_schema() {
        let schema: Value =
            serde_json::from_str(include_str!("fixtures/thread-start-contract.json")).unwrap();
        let default = thread_start_params("/project", false);
        assert_eq!(default, json!({"cwd":"/project","ephemeral":false}));
        let yolo = thread_start_params("/project", true);
        for field in yolo.as_object().unwrap().keys() {
            assert!(
                schema["properties"].get(field).is_some(),
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
    rejected = (method == 'initialize' and FAULT == 'initialize-reject') or (method == 'thread/start' and FAULT in ['start-reject', 'start-other']) or (method == 'thread/name/set' and FAULT == 'name-reject') or (method == 'thread/read' and FAULT == 'read-reject')
    if rejected:
        resp = {'id':req['id'], 'error':{'code':-32000 if FAULT == 'start-other' else -32602, 'message':'injected rejection'}}
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
                |id| {
                    persisted = Some(id.to_string());
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(persisted.as_deref(), Some(id.as_str()));
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
                    "thread/start",
                    "thread/name/set",
                    "thread/read"
                ]
            );
            assert_eq!(
                calls[2]["params"],
                thread_start_params(dir.path().to_str().unwrap(), yolo)
            );
            finish_creation(&installation, dir.path().to_str().unwrap(), "title", &id).unwrap();
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
    fn helper_failure_classification_retains_rpc_method_code_and_created_id() {
        for (fault, outcome, method, code, has_id) in [
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
                None,
                |_| Ok(()),
                Duration::from_millis(500),
            )
            .unwrap_err();
            assert_eq!(err.outcome, outcome, "{fault}");
            assert_eq!(err.method.as_deref(), method, "{fault}");
            assert_eq!(err.code, code, "{fault}");
            assert_eq!(err.session_id.is_some(), has_id, "{fault}");
            assert!(!trace(&dir).iter().any(|r| r["method"] == "turn/start"));
        }
        let (dir, mut installation) = fake_runtime("success");
        let err = create(
            &installation,
            dir.path().to_str().unwrap(),
            "title",
            true,
            |_| Err("disk failure".into()),
        )
        .unwrap_err();
        assert_eq!(err.outcome, CreationOutcome::Unknown);
        assert_eq!(err.stage, "persist-thread");
        assert!(err.session_id.is_some());
        assert_eq!(trace(&dir).len(), 3);
        installation.executable = dir.path().join("missing-runtime");
        let err = create(
            &installation,
            dir.path().to_str().unwrap(),
            "title",
            true,
            |_| Ok(()),
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
                .ensure_thread(&installation, dir.path().to_str().unwrap(), "title", true)
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
            let script = std::fs::read_to_string(&installation.executable)
                .unwrap()
                .replace(&format!("FAULT={fault:?}"), "FAULT=\"success\"");
            std::fs::write(&installation.executable, script).unwrap();
            retry
                .ensure_thread(&installation, dir.path().to_str().unwrap(), "title", true)
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
}
