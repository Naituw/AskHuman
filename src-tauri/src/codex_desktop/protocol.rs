//! Versioned desktop IPC framing and state reduction. This is not app-server JSON-RPC.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

const MAX_FRAME: usize = 64 * 1024 * 1024;
trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}
type Writer = tokio::io::WriteHalf<Box<dyn Stream>>;

pub struct Connection {
    writer: tokio::sync::Mutex<Writer>,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    client: Mutex<String>,
    pub alive: AtomicBool,
    stop: tokio::sync::Notify,
}

impl Connection {
    #[cfg(target_os = "macos")]
    pub async fn connect(
        path: &str,
        events: tokio::sync::mpsc::UnboundedSender<Value>,
    ) -> Result<Arc<Self>, String> {
        #[cfg(target_os = "macos")]
        let stream: Box<dyn Stream> = Box::new(
            tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::UnixStream::connect(path),
            )
            .await
            .map_err(|_| "Codex App connection timed out")?
            .map_err(|e| format!("Open Codex App and retry: {e}"))?,
        );
        let (mut reader, writer) = tokio::io::split(stream);
        let conn = Arc::new(Self {
            writer: tokio::sync::Mutex::new(writer),
            pending: Mutex::new(HashMap::new()),
            client: Mutex::new(String::new()),
            alive: AtomicBool::new(true),
            stop: tokio::sync::Notify::new(),
        });
        let c = conn.clone();
        tokio::spawn(async move {
            loop {
                let message = tokio::select! { _ = c.stop.notified() => break, value = read_frame(&mut reader) => match value { Ok(v) => v, Err(_) => break } };
                match message["type"].as_str() {
                    Some("response") => {
                        let sender = c
                            .pending
                            .lock()
                            .unwrap()
                            .remove(message["requestId"].as_str().unwrap_or_default());
                        if let Some(sender) = sender {
                            let _ = sender.send(message);
                        }
                    }
                    Some("client-discovery-request") => {
                        let _ = c.send(json!({"type":"client-discovery-response","requestId":message["requestId"],"response":{"canHandle":false}})).await;
                    }
                    Some("broadcast") => {
                        let client = c.client.lock().unwrap().clone();
                        if message["targetClientIds"]
                            .as_array()
                            .is_none_or(|ids| ids.iter().any(|id| id.as_str() == Some(&client)))
                        {
                            let _ = events.send(message);
                        }
                    }
                    _ => {}
                }
            }
            c.alive.store(false, Ordering::SeqCst);
            c.pending.lock().unwrap().clear();
            let _ = events.send(json!({"type":"disconnected"}));
        });
        let result = conn
            .request("initialize", json!({"clientType":"askhuman"}), None)
            .await;
        match result {
            Ok(v) => {
                let Some(id) = v["clientId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                else {
                    conn.close();
                    return Err("Desktop initialization returned no client identity".into());
                };
                *conn.client.lock().unwrap() = id;
            }
            Err(e) => {
                conn.close();
                return Err(e);
            }
        }
        Ok(conn)
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn connect(
        _path: &str,
        _events: tokio::sync::mpsc::UnboundedSender<Value>,
    ) -> Result<Arc<Self>, String> {
        Err("Desktop integration currently supports macOS only".into())
    }

    async fn send(&self, message: Value) -> Result<(), String> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err("Codex App disconnected".into());
        }
        let data = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        if data.len() > MAX_FRAME {
            return Err("Desktop request is too large".into());
        }
        let mut writer = self.writer.lock().await;
        tokio::time::timeout(Duration::from_secs(5), async {
            writer.write_all(&(data.len() as u32).to_le_bytes()).await?;
            writer.write_all(&data).await
        })
        .await
        .map_err(|_| "Desktop write timed out; outcome may be unknown".to_string())?
        .map_err(|e| e.to_string())
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        target: Option<&str>,
    ) -> Result<Value, String> {
        let version = match method {
            "initialize" => 0,
            "thread-follower-start-turn" => 2,
            "thread-follower-steer-turn" => 1,
            "thread-follower-interrupt-turn" => 4,
            "thread-follower-submit-user-input"
            | "thread-follower-command-approval-decision"
            | "thread-follower-file-approval-decision"
            | "thread-follower-permissions-request-approval-response" => 1,
            _ => return Err("Unsupported desktop operation".into()),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let client = self.client.lock().unwrap().clone();
        let mut message = json!({"type":"request","requestId":id,"sourceClientId":if client.is_empty(){Value::Null}else{json!(client)},"method":method,"version":version,"params":params,"timeoutMs":30000});
        if let Some(target) = target {
            message["targetClientId"] = json!(target);
        }
        let sent = self.send(message).await;
        let result = match sent {
            Err(e) => Err(e),
            Ok(()) => match tokio::time::timeout(Duration::from_secs(32), rx).await {
                Ok(Ok(v)) if v["resultType"] == "success" => Ok(v["result"].clone()),
                Ok(Ok(v)) => Err(v["error"].as_str().unwrap_or("Desktop rejected the operation").to_string()),
                _ => Err("Desktop response unavailable; operation outcome may be unknown. Check the original chat before retrying.".into()),
            },
        };
        self.pending.lock().unwrap().remove(&id);
        result
    }

    pub async fn follow(&self, id: &str, enabled: bool) -> Result<(), String> {
        let client = self.client.lock().unwrap().clone();
        self.send(json!({"type":"broadcast","sourceClientId":client,"method":"thread-stream-following-changed","version":1,"params":{"hostId":"local","conversationId":id,"following":enabled}})).await
    }

    pub fn close(&self) {
        self.alive.store(false, Ordering::SeqCst);
        self.stop.notify_one();
    }
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Value, String> {
    let size = reader.read_u32_le().await.map_err(|e| e.to_string())? as usize;
    if size == 0 || size > MAX_FRAME {
        return Err("Invalid desktop frame size".into());
    }
    let mut bytes = vec![0; size];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub fn patches(state: &mut Value, patches: &Value) -> Result<(), String> {
    for patch in patches.as_array().ok_or("Invalid patches")? {
        let keys: Vec<String> = match &patch["path"] {
            Value::String(s) if s.is_empty() || s.starts_with('/') => s
                .split('/')
                .skip(1)
                .map(|s| s.replace("~1", "/").replace("~0", "~"))
                .collect(),
            Value::Array(a) => a
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                        .ok_or("Invalid path component")
                })
                .collect::<Result<_, _>>()?,
            _ => return Err("Invalid patch path".into()),
        };
        let op = patch["op"].as_str().ok_or("Invalid operation")?;
        if !matches!(op, "remove" | "replace" | "add")
            || (op != "remove" && patch.get("value").is_none())
        {
            return Err("Invalid patch".into());
        }
        if keys.is_empty() {
            if op != "replace" {
                return Err("Invalid root patch".into());
            }
            *state = patch["value"].clone();
            continue;
        }
        let mut target = &mut *state;
        for key in &keys[..keys.len() - 1] {
            target = if target.is_array() {
                target.get_mut(key.parse::<usize>().map_err(|_| "Invalid index")?)
            } else {
                target.get_mut(key)
            }
            .ok_or("Missing patch parent")?;
        }
        let key = keys.last().unwrap();
        match target {
            Value::Array(a) => {
                let index = if key == "-" && op == "add" {
                    a.len()
                } else {
                    key.parse::<usize>().map_err(|_| "Invalid array index")?
                };
                if index > a.len() || (op != "add" && index == a.len()) {
                    return Err("Array index out of bounds".into());
                }
                match op {
                    "add" => a.insert(index, patch["value"].clone()),
                    "replace" => a[index] = patch["value"].clone(),
                    _ => {
                        a.remove(index);
                    }
                }
            }
            Value::Object(o) => {
                if op != "add" && !o.contains_key(key) {
                    return Err("Missing patch member".into());
                }
                if op == "remove" {
                    o.remove(key);
                } else {
                    o.insert(key.clone(), patch["value"].clone());
                }
            }
            _ => return Err("Invalid patch target".into()),
        }
    }
    Ok(())
}

pub fn turns(state: &Value) -> Vec<&Value> {
    if state["turnHistory"]["kind"] == "canonical" {
        let h = &state["turnHistory"]["history"];
        return entities(h);
    }
    state["turns"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

pub fn entities(value: &Value) -> Vec<&Value> {
    if let Some(a) = value.as_array() {
        return a.iter().collect();
    }
    let mut result = Vec::new();
    if let Some(islands) = value["islands"].as_array() {
        let mut seen = std::collections::HashSet::new();
        for island in islands {
            if let Some(entries) = island["entries"].as_array() {
                for entry in entries {
                    if let Some(key) = entry["value"].as_str() {
                        if seen.insert(key) {
                            if let Some(v) = value["entitiesByKey"].get(key) {
                                result.push(v);
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_paths_arrays_and_invalid_revisions_do_not_guess() {
        let mut state = json!({"a/b":[1,3]});
        patches(&mut state, &json!([{"op":"add","path":"/a~1b/1","value":2},{"op":"replace","path":["a/b",0],"value":0}])).unwrap();
        assert_eq!(state, json!({"a/b":[0,2,3]}));
        assert!(patches(&mut state, &json!([{"op":"remove","path":["a/b",3]}])).is_err());
        assert!(patches(&mut state, &json!([{"op":"move","path":[]}])).is_err());
    }
    #[test]
    fn canonical_history_preserves_order_without_duplicates() {
        let state = json!({"turnHistory":{"kind":"canonical","history":{"entitiesByKey":{"b":{"turnId":"b"},"a":{"turnId":"a"}},"islands":[{"entries":[{"value":"b"},{"value":"a"},{"value":"b"}]}]}},"turns":[{"turnId":"wrong"}]});
        assert_eq!(
            turns(&state)
                .iter()
                .map(|t| t["turnId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }
    #[tokio::test]
    async fn frame_rejects_oversize_before_allocation() {
        let data = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert!(read_frame(&mut &data[..]).await.is_err());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod transport_tests {
    use super::*;
    #[tokio::test]
    async fn handshake_and_owner_bound_versioned_request() {
        let path = std::env::temp_dir().join(format!("ah-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let init = read_frame(&mut stream).await.unwrap();
            assert_eq!(init["method"], "initialize");
            assert_eq!(init["version"], 0);
            assert!(init["sourceClientId"].is_null());
            let data=serde_json::to_vec(&json!({"type":"response","requestId":init["requestId"],"resultType":"success","result":{"clientId":"follower"}})).unwrap();
            stream.write_u32_le(data.len() as u32).await.unwrap();
            stream.write_all(&data).await.unwrap();
            let request = read_frame(&mut stream).await.unwrap();
            assert_eq!(request["targetClientId"], "owner");
            assert_eq!(request["sourceClientId"], "follower");
            assert_eq!(request["version"], 4);
            let data=serde_json::to_vec(&json!({"type":"response","requestId":request["requestId"],"resultType":"success","result":{}})).unwrap();
            stream.write_u32_le(data.len() as u32).await.unwrap();
            stream.write_all(&data).await.unwrap();
        });
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = Connection::connect(path.to_str().unwrap(), tx)
            .await
            .unwrap();
        connection
            .request(
                "thread-follower-interrupt-turn",
                json!({"conversationId":"s","expectedTurnId":"t","mode":"user-stop"}),
                Some("owner"),
            )
            .await
            .unwrap();
        connection.close();
        server.await.unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
