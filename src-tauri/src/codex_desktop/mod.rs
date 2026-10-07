//! Optional Codex desktop adapter. Only the daemon owns the live connection.
//! Protocol attribution and license: see NOTICE.md in this directory.
mod actions;
mod launch;
pub use launch::{
    available, integration_enabled, launch_status, resolve_launch, LaunchStatus, UnavailableReason,
};
pub mod protocol;
pub mod requests;
mod runtime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Arc, Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub app_path: String,
    pub codex_home: String,
    pub launch_preference: LaunchTarget,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchTarget {
    Terminal,
    #[default]
    Desktop,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Operation {
    Status,
    Open {
        session_id: Option<String>,
    },
    Send {
        session_id: String,
        text: String,
        files: Vec<String>,
        id: String,
    },
    Stop {
        session_id: String,
        id: String,
    },
    Create {
        cwd: String,
        text: String,
        files: Vec<String>,
        permission: String,
        id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub cwd: String,
}
#[derive(Clone, Default)]
pub struct Session {
    pub meta: Option<SessionMeta>,
    pub owner: Option<String>,
    pub state: Arc<Value>,
    pub revision: Option<u64>,
    pub connected: bool,
    pub error: Option<String>,
}
impl Session {
    pub fn active(&self) -> bool {
        self.connected && self.state["threadRuntimeStatus"]["type"] == "active"
    }
    pub fn latest_text(&self) -> Option<String> {
        for turn in protocol::turns(&self.state).into_iter().rev() {
            for item in protocol::entities(&turn["items"]).into_iter().rev() {
                if matches!(
                    item["type"].as_str(),
                    Some("agentMessage" | "assistantMessage")
                ) {
                    if let Some(text) = item["text"].as_str().filter(|s| !s.is_empty()) {
                        return Some(
                            text.chars()
                                .rev()
                                .take(12000)
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect(),
                        );
                    }
                }
            }
        }
        None
    }
}

pub struct Bridge {
    connection: tokio::sync::Mutex<Option<Arc<protocol::Connection>>>,
    pub sessions: Mutex<HashMap<String, Session>>,
    pub config: Mutex<Config>,
    error: Mutex<Option<String>>,
    actions: tokio::sync::Mutex<()>,
    last_scan: Mutex<Option<std::time::Instant>>,
    refresh_gate: tokio::sync::Mutex<()>,
    generation: std::sync::atomic::AtomicU64,
}

pub fn shared() -> Arc<Bridge> {
    static BRIDGE: OnceLock<Arc<Bridge>> = OnceLock::new();
    BRIDGE
        .get_or_init(|| {
            Arc::new(Bridge {
                connection: tokio::sync::Mutex::new(None),
                sessions: Mutex::new(HashMap::new()),
                config: Mutex::new(Config::default()),
                error: Mutex::new(None),
                actions: tokio::sync::Mutex::new(()),
                last_scan: Mutex::new(None),
                refresh_gate: tokio::sync::Mutex::new(()),
                generation: std::sync::atomic::AtomicU64::new(0),
            })
        })
        .clone()
}

pub fn is_desktop_session(record: &Value) -> bool {
    record["terminal"] == "codex-app" || record["desktop"].is_object()
}

impl Bridge {
    pub async fn refresh(self: &Arc<Self>, config: Config) {
        let _refresh = self.refresh_gate.lock().await;
        let previous = self.config.lock().unwrap().clone();
        let changed =
            previous.app_path != config.app_path || previous.codex_home != config.codex_home;
        let enabled = integration_enabled();
        *self.config.lock().unwrap() = config.clone();
        if changed || !enabled {
            self.generation.fetch_add(1, Ordering::SeqCst);
            if let Some(c) = self.connection.lock().await.take() {
                c.close();
            }
            self.sessions.lock().unwrap().clear();
            *self.last_scan.lock().unwrap() = None;
        }
        if !enabled {
            *self.error.lock().unwrap() = None;
            return;
        }
        if let Err(error) = self.connect_and_scan(&config).await {
            *self.error.lock().unwrap() = Some(error);
        }
    }

    async fn connect_and_scan(self: &Arc<Self>, config: &Config) -> Result<(), String> {
        let installation = runtime::detect(config)?;
        let mut slot = self.connection.lock().await;
        if slot
            .as_ref()
            .is_none_or(|c| !c.alive.load(Ordering::SeqCst))
        {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                while let Some(message) = rx.recv().await {
                    let Some(bridge) = weak.upgrade() else { break };
                    if bridge.generation.load(Ordering::SeqCst) == generation {
                        bridge.event(message);
                    }
                }
            });
            *slot = Some(
                protocol::Connection::connect(&installation.socket.to_string_lossy(), tx).await?,
            );
            for session in self.sessions.lock().unwrap().values_mut() {
                session.connected = false;
                session.owner = None;
                session.revision = None;
            }
        }
        let conn = slot.as_ref().unwrap().clone();
        drop(slot);
        let scan = self
            .last_scan
            .lock()
            .unwrap()
            .is_none_or(|at| at.elapsed() > Duration::from_secs(15));
        if scan {
            let home = installation.home;
            let meta = tokio::task::spawn_blocking(move || runtime::list(&home))
                .await
                .map_err(|e| e.to_string())??;
            let mut sessions = self.sessions.lock().unwrap();
            for row in meta {
                let id = row.id.clone();
                sessions.entry(id).or_default().meta = Some(row);
            }
            *self.last_scan.lock().unwrap() = Some(std::time::Instant::now());
        }
        let ids: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| !s.connected)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            conn.follow(&id, true).await?;
        }
        *self.error.lock().unwrap() = None;
        Ok(())
    }

    fn event(&self, message: Value) {
        let mut sessions = self.sessions.lock().unwrap();
        if message["type"] == "disconnected" {
            for s in sessions.values_mut() {
                s.connected = false;
                s.owner = None;
                s.revision = None;
                s.error = Some("Codex App disconnected".into());
            }
            return;
        }
        if message["method"] == "client-status-changed"
            && message["params"]["status"] == "disconnected"
        {
            for s in sessions
                .values_mut()
                .filter(|s| s.owner.as_deref() == message["params"]["clientId"].as_str())
            {
                s.connected = false;
                s.owner = None;
                s.revision = None;
            }
            return;
        }
        if message["method"] != "thread-stream-state-changed"
            || message["params"]["hostId"] != "local"
        {
            return;
        }
        let Some(s) = message["params"]["conversationId"]
            .as_str()
            .and_then(|id| sessions.get_mut(id))
        else {
            return;
        };
        if message["version"] != 11 {
            s.connected = false;
            s.error = Some("Incompatible Codex App protocol".into());
            *self.error.lock().unwrap() = s.error.clone();
            return;
        }
        let change = &message["params"]["change"];
        let owner = message["sourceClientId"].as_str();
        if owner.is_none() {
            return;
        }
        if s.owner.is_some() && s.owner.as_deref() != owner {
            return;
        }
        let valid = if change["type"] == "snapshot"
            && change["conversationState"]["id"] == message["params"]["conversationId"]
        {
            s.state = Arc::new(change["conversationState"].clone());
            s.owner = owner.map(str::to_owned);
            true
        } else if change["type"] == "patches"
            && s.revision.is_some()
            && change["baseRevision"].as_u64() == s.revision
        {
            protocol::patches(Arc::make_mut(&mut s.state), &change["patches"]).is_ok()
        } else {
            false
        };
        let valid = valid && change["revision"].as_u64().is_some();
        s.connected = valid;
        s.revision = if valid {
            change["revision"].as_u64()
        } else {
            None
        };
        s.error = if valid {
            None
        } else {
            Some("Refreshing desktop snapshot".into())
        };
        if !valid {
            s.owner = None;
        }
    }

    pub fn session(&self, id: &str) -> Option<Session> {
        self.sessions.lock().unwrap().get(id).cloned()
    }
    pub fn status(&self) -> Value {
        let config = self.config.lock().unwrap().clone();
        let installation = runtime::detect(&config);
        let sessions = self.sessions.lock().unwrap();
        let connected = self
            .connection
            .try_lock()
            .ok()
            .is_some_and(|c| c.as_ref().is_some_and(|c| c.alive.load(Ordering::SeqCst)));
        json!({"supported":cfg!(target_os="macos"),"enabled":integration_enabled(),"installed":installation.is_ok(),"connected":connected,"installation":installation.as_ref().ok(),"launch":launch_status(false),"error":self.error.lock().unwrap().clone().or_else(||installation.err()),"sessions":sessions.iter().filter_map(|(id,s)|s.meta.as_ref().map(|m|json!({"id":id,"title":m.title,"cwd":m.cwd,"connected":s.connected,"active":s.active(),"error":s.error}))).collect::<Vec<_>>()})
    }
    async fn connection(&self) -> Result<Arc<protocol::Connection>, String> {
        self.connection
            .lock()
            .await
            .clone()
            .filter(|c| c.alive.load(Ordering::SeqCst))
            .ok_or_else(|| "Codex App is not connected".into())
    }
    pub async fn call(&self, id: &str, method: &str, mut params: Value) -> Result<Value, String> {
        let session = self
            .session(id)
            .filter(|s| s.connected)
            .ok_or("Desktop session is not connected")?;
        params["conversationId"] = json!(id);
        self.connection()
            .await?
            .request(method, params, session.owner.as_deref())
            .await
    }
    pub async fn activate(self: &Arc<Self>, id: &str) -> Result<(), String> {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid session ID")?;
        if self.session(id).is_none() {
            return Err("Unknown desktop session".into());
        }
        if self.session(id).is_some_and(|s| s.connected) {
            return Ok(());
        }
        let installation = runtime::detect(&self.config.lock().unwrap())?;
        let thread = id.to_string();
        tokio::task::spawn_blocking(move || runtime::open(&installation, Some(&thread)))
            .await
            .map_err(|e| e.to_string())??;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            let config = self.config.lock().unwrap().clone();
            if !integration_enabled() {
                return Err("Codex integration was disabled".into());
            }
            if let Ok(conn) = self.connection().await {
                conn.follow(id, true).await?;
            } else {
                let _ = self.connect_and_scan(&config).await;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self.session(id).is_some_and(|s| s.connected) {
                return Ok(());
            }
        }
        Err(
            "Desktop has not loaded this chat. It may have switched pages; no message was sent."
                .into(),
        )
    }

    pub async fn execute(self: &Arc<Self>, op: Operation) -> Result<Value, String> {
        if matches!(op, Operation::Status) {
            self.refresh(crate::config::AppConfig::load_without_secrets().codex_desktop)
                .await;
            return Ok(self.status());
        }
        if let Operation::Open { session_id } = op {
            let installation =
                runtime::detect(&crate::config::AppConfig::load_without_secrets().codex_desktop)?;
            tokio::task::spawn_blocking(move || {
                runtime::open(&installation, session_id.as_deref())
            })
            .await
            .map_err(|e| e.to_string())??;
            return Ok(json!({"status":"opened"}));
        }
        let _guard = self.actions.lock().await;
        let (id, text, files) = match &op {
            Operation::Send {
                id, text, files, ..
            }
            | Operation::Create {
                id, text, files, ..
            } => (id, text.as_str(), files.as_slice()),
            Operation::Stop { id, .. } => (id, "", &[][..]),
            _ => unreachable!(),
        };
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid operation ID")?;
        let folder = crate::paths::state_dir().join("codex-desktop-actions");
        let path = folder.join(format!("{id}.json"));
        let input = serde_json::to_value(&op).map_err(|e| e.to_string())?;
        let mut action = actions::Action::load(path, input)?;
        // A durable receipt takes precedence over changed files, readiness or preferences.
        if let Some(receipt) = action.receipt()? {
            return Ok(receipt);
        }
        let validation = (|| {
            if !integration_enabled() {
                return Err("Enable Codex integration in Settings first".to_string());
            }
            if text.contains('\0') || text.chars().count() > 100000 || files.len() > 20 {
                return Err("Invalid task content".into());
            }
            if !matches!(op, Operation::Stop { .. }) && text.trim().is_empty() && files.is_empty() {
                return Err("Enter a task first".into());
            }
            for file in files {
                if !std::path::Path::new(file).is_absolute()
                    || !std::path::Path::new(file).is_file()
                {
                    return Err(format!("Attachment is unavailable: {file}"));
                }
            }
            if let Operation::Create {
                cwd, permission, ..
            } = &op
            {
                if !std::path::Path::new(cwd).is_absolute() || !std::path::Path::new(cwd).is_dir() {
                    return Err("Select an existing absolute project directory".into());
                }
                if !matches!(permission.as_str(), "agent-default" | "yolo") {
                    return Err("Choose task permissions".into());
                }
            }
            runtime::detect(&crate::config::AppConfig::load_without_secrets().codex_desktop)
        })();
        let installation = match validation {
            Ok(installation) => installation,
            Err(error) => return Err(action.fail_before_submit("validation", &error)),
        };
        self.refresh(crate::config::AppConfig::load_without_secrets().codex_desktop)
            .await;
        let session_id = match &op {
            Operation::Create {
                cwd, permission, ..
            } => {
                let cwd = cwd.clone();
                let title: String = text.chars().take(80).collect();
                let yolo = permission == "yolo";
                let (updated, result) = tokio::task::spawn_blocking(move || {
                    let result = action.ensure_thread(&installation, &cwd, &title, yolo);
                    (action, result)
                })
                .await
                .map_err(|e| {
                    format!(
                        "Creation helper failed: {e}. Check the operation receipt before retrying."
                    )
                })?;
                action = updated;
                result?
            }
            Operation::Send { session_id, .. } | Operation::Stop { session_id, .. } => {
                session_id.clone()
            }
            _ => unreachable!(),
        };
        if let Operation::Create { cwd, .. } = &op {
            self.sessions
                .lock()
                .unwrap()
                .entry(session_id.clone())
                .or_default()
                .meta = Some(SessionMeta {
                id: session_id.clone(),
                cwd: cwd.clone(),
                title: text.chars().take(80).collect(),
            });
        }
        if let Err(error) = self.activate(&session_id).await {
            return Err(action.fail_before_submit("activate", &error));
        }
        let s = match self.session(&session_id) {
            Some(session) => session,
            None => return Err(action.fail_before_submit("activate", "Session disappeared")),
        };
        let intent = match &op {
            Operation::Stop { .. } => ControlIntent::Stop,
            Operation::Create { permission, .. } if permission == "yolo" => {
                ControlIntent::YoloCreate
            }
            _ => ControlIntent::Inherit,
        };
        let (method, params) = control_request(&s, &session_id, id, text, files, intent)
            .map_err(|error| action.fail_before_submit("prepare-submit", &error))?;
        action
            .submit(&session_id, method, || {
                self.call(&session_id, method, params)
            })
            .await
    }
}

#[derive(Clone, Copy)]
enum ControlIntent {
    Inherit,
    YoloCreate,
    Stop,
}

fn control_request(
    session: &Session,
    session_id: &str,
    id: &str,
    text: &str,
    files: &[String],
    intent: ControlIntent,
) -> Result<(&'static str, Value), String> {
    Ok(if matches!(intent, ControlIntent::Stop) {
        let turn = protocol::turns(&session.state)
            .into_iter()
            .rev()
            .find(|t| t["status"] == "inProgress")
            .ok_or("No active turn to stop")?;
        (
            "thread-follower-interrupt-turn",
            json!({"mode":"user-stop","expectedTurnId":turn["turnId"]}),
        )
    } else {
        if matches!(intent, ControlIntent::YoloCreate) && session.active() {
            return Err("Created thread already has an active turn. Inspect it before retrying the YOLO task.".into());
        }
        let mut input_text = text.to_string();
        if !files.is_empty() {
            input_text.push_str("\n\nAttached files:\n");
            input_text.push_str(&serde_json::to_string(files).map_err(|e| e.to_string())?);
        }
        let mut input = json!([{"type":"text","text":input_text,"text_elements":[]}]);
        for file in files {
            if std::path::Path::new(file)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "webp"
                    )
                })
            {
                input
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"type":"localImage","path":file}));
            }
        }
        let mut request = json!({"threadId":session_id,"input":input,"clientUserMessageId":id});
        if matches!(intent, ControlIntent::YoloCreate) {
            // The App can resume an empty thread with its default permissions. Carry
            // the launch choice into turn/start, where SandboxPolicy is camelCase.
            request["approvalPolicy"] = json!("never");
            request["sandboxPolicy"] = json!({"type":"dangerFullAccess"});
        }
        let context = json!({"inheritThreadSettings":true,"attachments":[],"commentAttachments":[],"fileAttachments":files.iter().map(|p|json!({"path":p,"label":std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy()})).collect::<Vec<_>>()});
        if session.active() {
            (
                "thread-follower-steer-turn",
                json!({"input":input,"clientUserMessageId":id,"restoreMessage":{"request":request,"context":context},"attachments":[]}),
            )
        } else {
            (
                "thread-follower-start-turn",
                json!({"turnStart":{"request":request,"context":context}}),
            )
        }
    })
}

fn save(path: &std::path::Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Invalid operation path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::launch::select_target;
    use super::*;
    fn bridge() -> Bridge {
        Bridge {
            connection: tokio::sync::Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            config: Mutex::new(Config::default()),
            error: Mutex::new(None),
            actions: tokio::sync::Mutex::new(()),
            last_scan: Mutex::new(None),
            refresh_gate: tokio::sync::Mutex::new(()),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }
    fn snapshot(owner: &str, version: u64, revision: Value) -> Value {
        json!({"method":"thread-stream-state-changed","version":version,"sourceClientId":owner,"params":{"hostId":"local","conversationId":"s","change":{"type":"snapshot","revision":revision,"conversationState":{"id":"s","threadRuntimeStatus":{"type":"idle"}}}}})
    }
    #[test]
    fn control_requests_preserve_target_message_id_attachments_and_expected_turn() {
        let mut session = Session {
            connected: true,
            state: Arc::new(json!({"threadRuntimeStatus":{"type":"idle"}})),
            ..Session::default()
        };
        let files = vec!["/tmp/document.md".into(), "/tmp/image.PNG".into()];
        let (method, params) = control_request(
            &session,
            "thread",
            "operation",
            "task",
            &files,
            ControlIntent::Inherit,
        )
        .unwrap();
        assert_eq!(method, "thread-follower-start-turn");
        let request = &params["turnStart"]["request"];
        assert_eq!(request["threadId"], "thread");
        assert_eq!(request["clientUserMessageId"], "operation");
        assert_eq!(
            request["input"][1],
            json!({"type":"localImage","path":"/tmp/image.PNG"})
        );
        assert!(request["input"][0]["text"]
            .as_str()
            .unwrap()
            .contains("/tmp/document.md"));
        assert_eq!(
            params["turnStart"]["context"]["inheritThreadSettings"],
            true
        );
        session.state = Arc::new(
            json!({"threadRuntimeStatus":{"type":"active"},"turnHistory":{"kind":"canonical","history":[{"turnId":"current","status":"inProgress","items":[]}]}}),
        );
        let (method, steer) = control_request(
            &session,
            "thread",
            "operation",
            "task",
            &files,
            ControlIntent::Inherit,
        )
        .unwrap();
        assert_eq!(method, "thread-follower-steer-turn");
        assert_eq!(steer["restoreMessage"]["request"], *request);
        assert_eq!(steer["clientUserMessageId"], "operation");
        let (method, stop) =
            control_request(&session, "thread", "stop", "", &[], ControlIntent::Stop).unwrap();
        assert_eq!(method, "thread-follower-interrupt-turn");
        assert_eq!(stop["expectedTurnId"], "current");
        assert!(control_request(
            &Session::default(),
            "thread",
            "stop",
            "",
            &[],
            ControlIntent::Stop
        )
        .is_err());
    }

    #[test]
    fn yolo_first_turn_overrides_app_resumed_workspace_permissions() {
        let mut session = Session {
            connected: true,
            state: Arc::new(json!({
                "threadRuntimeStatus":{"type":"idle"},
                "currentPermissions":{
                    "approvalPolicy":"on-request",
                    "activePermissionProfile":{"id":":workspace"},
                    "sandboxPolicy":{"type":"workspaceWrite","networkAccess":false}
                }
            })),
            ..Session::default()
        };
        let (method, params) = control_request(
            &session,
            "created",
            "operation",
            "task",
            &[],
            ControlIntent::YoloCreate,
        )
        .unwrap();
        assert_eq!(method, "thread-follower-start-turn");
        let request = &params["turnStart"]["request"];
        let schema: Value =
            serde_json::from_str(include_str!("fixtures/turn-start-contract.json")).unwrap();
        assert!(schema["definitions"]["AskForApproval"]["oneOf"][0]["enum"]
            .as_array()
            .unwrap()
            .contains(&request["approvalPolicy"]));
        assert!(schema["definitions"]["SandboxPolicy"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variant| {
                variant["properties"]["type"]["enum"]
                    .as_array()
                    .is_some_and(|values| values.contains(&request["sandboxPolicy"]["type"]))
            }));
        assert_eq!(request["approvalPolicy"], "never");
        assert_eq!(request["sandboxPolicy"], json!({"type":"dangerFullAccess"}));
        assert!(request.get("permissions").is_none());
        assert_eq!(request["threadId"], "created");
        assert_eq!(request["clientUserMessageId"], "operation");
        // Ordinary sends and default launches must inherit even after a YOLO launch.
        for policy in ["workspaceWrite", "dangerFullAccess"] {
            session.state = Arc::new(json!({"threadRuntimeStatus":{"type":"idle"},
                "currentPermissions":{"sandboxPolicy":{"type":policy}}}));
            let (_, inherited) = control_request(
                &session,
                "created",
                "send",
                "next",
                &[],
                ControlIntent::Inherit,
            )
            .unwrap();
            let inherited = &inherited["turnStart"]["request"];
            assert!(inherited.get("approvalPolicy").is_none());
            assert!(inherited.get("sandboxPolicy").is_none());
            assert!(inherited.get("permissions").is_none());
        }
        session.state = Arc::new(json!({"threadRuntimeStatus":{"type":"active"}}));
        assert!(control_request(
            &session,
            "created",
            "operation",
            "task",
            &[],
            ControlIntent::YoloCreate,
        )
        .is_err());
    }

    #[test]
    fn desktop_origin_remains_identifiable_when_disconnected() {
        assert!(is_desktop_session(
            &json!({"terminal":"codex-app","desktop":{"connected":false}})
        ));
        assert!(is_desktop_session(&json!({"terminal":"codex-app"})));
        assert!(!is_desktop_session(&json!({"terminal":"Terminal.app"})));
    }
    #[test]
    fn preference_selects_available_runtime_without_a_connection_requirement() {
        use LaunchTarget::{Desktop, Terminal};
        for (desktop, terminal, prefer_app, prefer_cli) in [
            (true, true, Some(Desktop), Some(Terminal)),
            (true, false, Some(Desktop), Some(Desktop)),
            (false, true, Some(Terminal), Some(Terminal)),
            (false, false, None, None),
        ] {
            assert_eq!(select_target(Desktop, desktop, terminal), prefer_app);
            assert_eq!(select_target(Terminal, desktop, terminal), prefer_cli);
        }
    }
    #[test]
    fn preference_defaults_to_desktop_and_survives_config_roundtrip() {
        assert_eq!(Config::default().launch_preference, LaunchTarget::Desktop);
        let legacy: Config =
            serde_json::from_value(json!({"enabled":false,"defaultLaunch":"terminal"})).unwrap();
        assert_eq!(legacy.launch_preference, LaunchTarget::Desktop);
        let value: Config = serde_json::from_value(json!({"launchPreference":"terminal"})).unwrap();
        assert_eq!(value.launch_preference, LaunchTarget::Terminal);
        assert_eq!(
            serde_json::from_value::<Config>(serde_json::to_value(&value).unwrap()).unwrap(),
            value
        );
    }
    #[test]
    fn owner_revision_and_protocol_must_match_before_control() {
        let b = bridge();
        b.sessions
            .lock()
            .unwrap()
            .insert("s".into(), Session::default());
        b.event(snapshot("owner", 11, json!(1)));
        assert!(b.session("s").unwrap().connected);
        b.event(snapshot("impostor", 11, json!(2)));
        assert_eq!(b.session("s").unwrap().revision, Some(1));
        b.event(json!({"method":"thread-stream-state-changed","version":11,"sourceClientId":"owner","params":{"hostId":"local","conversationId":"s","change":{"type":"patches","baseRevision":0,"revision":2,"patches":[]}}}));
        assert!(!b.session("s").unwrap().connected);
        assert!(b.session("s").unwrap().owner.is_none());
        b.event(snapshot("new-owner", 11, json!(5)));
        assert!(b.session("s").unwrap().connected);
        b.event(snapshot("new-owner", 12, json!(6)));
        assert!(!b.session("s").unwrap().connected);
        b.event(snapshot("new-owner", 11, Value::Null));
        assert!(!b.session("s").unwrap().connected);
    }
    #[test]
    fn ledger_preserves_unknown_outcomes_and_private_files() {
        let dir = std::env::temp_dir().join(format!("askhuman-ledger-{}", uuid::Uuid::new_v4()));
        let path = dir.join("operation.json");
        let unknown = json!({"status":"unknown","sessionId":"existing","input":{"text":"task"}});
        save(&path, &unknown).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
            unknown
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
