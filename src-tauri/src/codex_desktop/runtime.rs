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

pub fn create(
    installation: &Installation,
    cwd: &str,
    title: &str,
    yolo: bool,
    on_created: impl FnOnce(&str) -> Result<(), String>,
) -> Result<String, String> {
    let mut child = Command::new(&installation.executable)
        .args(["app-server", "--listen", "stdio://"])
        .env("CODEX_HOME", &installation.home)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
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
        let mut request = |method: &str, params: Value| -> Result<Value, String> {
            counter += 1;
            writeln!(
                stdin,
                "{}",
                json!({"id":counter,"method":method,"params":params})
            )
            .and_then(|_| stdin.flush())
            .map_err(|e| e.to_string())?;
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                let response = rx
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .map_err(|_| {
                        "Thread creation response unavailable; check the App before creating again"
                    })?;
                if response["id"].as_u64() != Some(counter) {
                    continue;
                }
                if let Some(error) = response.get("error") {
                    return Err(error["message"]
                        .as_str()
                        .unwrap_or("Thread creation rejected")
                        .to_string());
                }
                if method == "initialize" {
                    writeln!(stdin, "{{\"method\":\"initialized\"}}")
                        .and_then(|_| stdin.flush())
                        .map_err(|e| e.to_string())?;
                }
                return Ok(response["result"].clone());
            }
        };
        request(
            "initialize",
            json!({"clientInfo":{"name":"askhuman","title":"AskHuman","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),
        )?;
        let mut params = json!({"cwd":cwd,"ephemeral":false});
        if yolo {
            params["approvalPolicy"] = json!("never");
            params["sandbox"] = json!("dangerFullAccess");
        }
        let result = request("thread/start", params)?;
        let id = result["thread"]["id"]
            .as_str()
            .ok_or("No thread ID returned")?
            .to_string();
        uuid::Uuid::parse_str(&id).map_err(|_| "Invalid created thread ID")?;
        on_created(&id)?;
        request("thread/name/set", json!({"threadId":id,"name":title}))?;
        request("thread/read", json!({"threadId":id,"includeTurns":true}))?;
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
    if !exited {
        return Err(
            "Creation runtime did not exit cleanly; inspect the desktop chat list before retrying"
                .into(),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
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
