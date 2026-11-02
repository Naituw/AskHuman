//! Read-only observation of an existing Codex CLI shared app-server.
//! Queueing is deliberately unavailable: 0.163 cannot bind an idle submission atomically to
//! the failed result, and its ordinary turn/start can steer a different, newly active turn.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::result::{Observation, TurnOutcome, TurnResult};

const MAX_FRAME: u64 = 2 * 1024 * 1024;

struct Proxy {
    _child: Child,
    writer: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

impl Proxy {
    async fn connect(program: &str, home: &Path) -> Result<Self, String> {
        let socket = home.join("app-server-control/app-server-control.sock");
        if !socket.exists() {
            return Err("Codex CLI shared server is unavailable".into());
        }
        let mut command = Command::new(program);
        command
            .args(["app-server", "proxy", "--sock"])
            .arg(socket)
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let mut proxy = Self {
            writer: child.stdin.take().ok_or("missing Codex proxy input")?,
            reader: BufReader::new(child.stdout.take().ok_or("missing Codex proxy output")?),
            _child: child,
            next_id: 0,
        };
        proxy
            .request(
                "initialize",
                json!({"clientInfo":{"name":"askhuman_observer","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),
            )
            .await?;
        proxy
            .writer
            .write_all(b"{\"method\":\"initialized\"}\n")
            .await
            .map_err(|e| e.to_string())?;
        Ok(proxy)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if !matches!(method, "initialize" | "thread/read" | "thread/turns/list") {
            return Err("Codex observer only permits read-only requests".into());
        }
        self.next_id += 1;
        let id = self.next_id;
        let mut frame = serde_json::to_vec(&json!({"id":id,"method":method,"params":params}))
            .map_err(|e| e.to_string())?;
        frame.push(b'\n');
        tokio::time::timeout(Duration::from_secs(4), async {
            self.writer
                .write_all(&frame)
                .await
                .map_err(|e| e.to_string())?;
            loop {
                let mut bytes = Vec::new();
                // A line limit applies before allocation, including unsolicited notifications.
                let read = tokio::io::AsyncReadExt::take(&mut self.reader, MAX_FRAME + 1)
                    .read_until(b'\n', &mut bytes)
                    .await
                    .map_err(|e| e.to_string())?;
                if read == 0 || read as u64 > MAX_FRAME {
                    return Err("Codex proxy closed or exceeded its frame limit".into());
                }
                let response: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if response["id"] != id {
                    continue;
                }
                return response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "Codex observer request was rejected".into());
            }
        })
        .await
        .map_err(|_| "Codex observer timed out".to_string())?
    }
}

#[derive(Default)]
pub struct Observer {
    proxy: Option<(PathBuf, Proxy)>,
}

impl Observer {
    pub fn close(&mut self) {
        self.proxy = None;
    }

    pub async fn failed_turn(
        &mut self,
        program: &str,
        home: &Path,
        session_id: &str,
        expected_turn: &str,
    ) -> Option<Observation> {
        if self
            .proxy
            .as_ref()
            .is_none_or(|(directory, _)| directory != home)
        {
            self.close();
            self.proxy = Some((
                home.to_path_buf(),
                Proxy::connect(program, home).await.ok()?,
            ));
        }
        let proxy = &mut self.proxy.as_mut()?.1;
        let result = async {
            let metadata = proxy.request("thread/read", json!({"threadId":session_id,"includeTurns":false})).await?;
            let turns = proxy.request("thread/turns/list", json!({"threadId":session_id,"limit":1,"sortDirection":"desc","itemsView":"notLoaded"})).await?;
            Ok::<_, String>(failure(&metadata["thread"], &turns["data"], session_id, expected_turn))
        }.await;
        match result {
            Ok(observation) => observation,
            Err(_) => {
                self.close();
                None
            }
        }
    }
}

/// A historical failure is not evidence that the currently active round failed.
fn failure(thread: &Value, turns: &Value, session: &str, expected: &str) -> Option<Observation> {
    if thread["id"] != session
        || !matches!(
            thread["status"]["type"].as_str(),
            Some("idle" | "notLoaded" | "systemError")
        )
    {
        return None;
    }
    let turn = turns.as_array()?.first()?;
    if turn["id"] != expected || turn["status"] != "failed" {
        return None;
    }
    Some(Observation {
        turn_id: Some(expected.into()),
        result: Some(TurnResult {
            id: String::new(),
            outcome: TurnOutcome::Failed,
            source: Some("codexCli".into()),
            turn_id: Some(expected.into()),
            reason: Some("runtime_error".into()),
            summary: turn["error"]["message"]
                .as_str()
                .map(|s| s.chars().filter(|c| !c.is_control()).take(320).collect()),
            at: 0,
        }),
        ..Observation::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn the_native_proxy_only_reads_the_existing_service_and_never_queues_a_prompt() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("app-server-control")).unwrap();
        std::fs::write(
            directory
                .path()
                .join("app-server-control/app-server-control.sock"),
            "",
        )
        .unwrap();
        let executable = directory.path().join("fake-codex");
        std::fs::write(&executable,r#"#!/usr/bin/env python3
import json, os, sys
assert sys.argv[1:4] == ['app-server', 'proxy', '--sock']
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    with open(os.path.join(os.environ['CODEX_HOME'], 'calls'), 'a') as log:
        log.write(method + '\n')
    if method == 'initialized': continue
    assert method in ['initialize', 'thread/read', 'thread/turns/list']
    if method == 'initialize': result = {'userAgent':'fake-codex/0.163.0'}
    elif method == 'thread/read': result = {'thread':{'id':'s','status':{'type':'systemError'}}}
    else: result = {'data':[{'id':'t','status':'failed','error':{'message':'selected model is at capacity'}}]}
    print(json.dumps({'id':request['id'],'result':result}),flush=True)
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut observer = Observer::default();
        let observation = observer
            .failed_turn(executable.to_str().unwrap(), directory.path(), "s", "t")
            .await
            .unwrap();
        assert_eq!(observation.result.unwrap().outcome, TurnOutcome::Failed);
        let proxy = &mut observer.proxy.as_mut().unwrap().1;
        assert!(proxy
            .request("turn/start", json!({"threadId":"s"}))
            .await
            .is_err());
        assert!(proxy
            .request("thread/queue/add", json!({"threadId":"s"}))
            .await
            .is_err());
        observer.close();
        let calls = std::fs::read_to_string(directory.path().join("calls")).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            vec![
                "initialize",
                "initialized",
                "thread/read",
                "thread/turns/list"
            ]
        );
    }

    #[test]
    fn a_final_capacity_failure_keeps_its_native_identity_and_reason() {
        let thread = json!({"id":"session","status":{"type":"idle"}});
        let turns = json!([{"id":"round","status":"failed","error":{"message":"The selected model is at capacity"}}]);
        let observation = failure(&thread, &turns, "session", "round").unwrap();
        assert_eq!(observation.turn_id.as_deref(), Some("round"));
        assert_eq!(
            observation.result.unwrap().summary.as_deref(),
            Some("The selected model is at capacity")
        );
        for status in ["active", "unknown"] {
            let thread = json!({"id":"session","status":{"type":status}});
            assert!(failure(&thread, &turns, "session", "round").is_none());
        }
        assert!(failure(&thread, &turns, "session", "new-round").is_none());
        assert!(failure(&thread, &turns, "other-session", "round").is_none());
        let system_error = json!({"id":"session","status":{"type":"systemError"}});
        assert!(failure(&system_error, &turns, "session", "round").is_some());
    }

    #[test]
    fn retrying_completed_interrupted_and_unidentified_rounds_are_not_failures() {
        let thread = json!({"id":"s","status":{"type":"idle"}});
        for status in ["inProgress", "completed", "interrupted", "future"] {
            assert!(failure(&thread, &json!([{"id":"t","status":status}]), "s", "t").is_none());
        }
        assert!(failure(&thread, &json!([{"status":"failed"}]), "s", "t").is_none());
        assert!(failure(&thread, &json!([]), "s", "t").is_none());
        let system_error = json!({"id":"s","status":{"type":"systemError"}});
        assert!(failure(
            &system_error,
            &json!([{"id":"t","status":"inProgress"}]),
            "s",
            "t"
        )
        .is_none());
    }
}
