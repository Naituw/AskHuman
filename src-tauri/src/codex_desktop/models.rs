//! What `codex app-server` says about the models it offers (`model/list`).
//!
//! Read-only: the server is started, initialised, asked for its model pages and closed. No thread
//! is created and no prompt is sent. The catalog only shapes what an iOS phone may pick for a new
//! task (docs/specs/ios-task-model.md).

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Most pages read: a catalog that never ends is cut.
const MAX_PAGES: usize = 20;
/// Most raw entries read.
const MAX_ENTRIES: usize = 2000;

/// The visible models of the Codex at `executable`, run with `home` as its data directory, all
/// pages in order. `timeout` bounds the whole exchange, start-up included.
pub fn list_models(
    executable: &Path,
    home: &Path,
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let deadline = Instant::now() + timeout;
    let mut server = AppServer::start(executable, home)?;
    server.request(
        "initialize",
        json!({
            "clientInfo": {"name": "askhuman", "title": "AskHuman", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true},
        }),
        deadline,
    )?;
    server.send(&json!({"method": "initialized"}))?;
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    for _ in 0..MAX_PAGES {
        let page = server.request(
            "model/list",
            json!({"includeHidden": false, "limit": 100, "cursor": cursor}),
            deadline,
        )?;
        let data = page["data"]
            .as_array()
            .ok_or("Codex did not return a model catalog")?;
        entries.extend(data.iter().cloned());
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        // The last page, a catalog that repeats itself, or more than anyone will scroll.
        let more = cursor.as_ref().is_some_and(|c| seen.insert(c.clone()));
        if !more || entries.len() >= MAX_ENTRIES {
            break;
        }
    }
    Ok(entries)
}

/// One `codex app-server` over stdio, closed when dropped.
struct AppServer {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: mpsc::Receiver<Value>,
    next_id: u64,
}

impl AppServer {
    fn start(executable: &Path, home: &Path) -> Result<Self, String> {
        let executable = executable
            .to_str()
            .ok_or("The Codex executable path is not valid text")?;
        let mut child = crate::integrations::agent_launch::login_shell_command(
            executable,
            &["app-server", "--listen", "stdio://"],
        )
        .env("CODEX_HOME", home)
        // Nothing here belongs to a project: do not pick up one's `.codex/` by accident.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start codex app-server: {error}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("codex app-server has no stdio".into());
        };
        let (sender, messages) = mpsc::channel();
        // Not joined: it ends when the server's stdout closes, and a stray process that keeps it
        // open must not hold anything up.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin: Some(stdin),
            messages,
            next_id: 0,
        })
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("codex app-server is closed")?;
        writeln!(stdin, "{message}")
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("Could not write to codex app-server: {error}"))
    }

    /// The `result` of the answer to `method`. Notifications and requests the server makes on its
    /// own are skipped: only a message without a `method` can be an answer.
    fn request(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"id": id, "method": method, "params": params}))?;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self
                .messages
                .recv_timeout(left)
                .map_err(|_| format!("codex app-server did not answer {method} in time"))?;
            if message.get("method").is_some() || message["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let reason = error["message"].as_str().unwrap_or("no reason given");
                return Err(format!("codex app-server rejected {method}: {reason}"));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| format!("codex app-server gave no result for {method}"));
        }
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        // The end of its input is the polite way out; a server that stays is killed.
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A `codex` that answers like `app-server` does, according to `fault`, and records what it
    /// was asked and run with in `trace.jsonl`.
    fn fake_codex(fault: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("codex");
        let trace = dir.path().join("trace.jsonl");
        let header = format!(
            "#!/usr/bin/env python3\nFAULT = {fault:?}\nTRACE = {:?}\n",
            trace.to_string_lossy()
        );
        std::fs::write(
            &executable,
            header
                + r#"
import json, os, sys
if len(sys.argv) > 1 and sys.argv[1] == '--prime':
    sys.exit(0)
with open(TRACE, 'a') as f:
    f.write(json.dumps({'argv': sys.argv[1:], 'codex_home': os.environ.get('CODEX_HOME'), 'cwd': os.getcwd()}) + '\n')
if FAULT == 'exit':
    sys.exit(3)
def entry(name, hidden=False):
    return {'id': name, 'model': name, 'displayName': name.upper(), 'hidden': hidden, 'isDefault': name == 'model-a',
            'defaultReasoningEffort': 'medium',
            'supportedReasoningEfforts': [{'reasoningEffort': 'low', 'description': ''}, {'reasoningEffort': 'medium', 'description': ''}]}
for line in sys.stdin:
    req = json.loads(line)
    with open(TRACE, 'a') as f:
        f.write(json.dumps(req) + '\n')
    method = req.get('method')
    if method == 'initialized':
        continue
    if FAULT == 'chatty':
        # A notification and a request of the server's own, whose id collides with ours.
        print(json.dumps({'method': 'remoteControl/status/changed', 'params': {}}), flush=True)
        print(json.dumps({'id': req['id'], 'method': 'item/tool/requestUserInput', 'params': {}}), flush=True)
    if method == 'model/list' and FAULT == 'silent':
        continue
    if method == 'model/list' and FAULT == 'reject':
        print(json.dumps({'id': req['id'], 'error': {'code': -32000, 'message': 'catalog unavailable'}}), flush=True)
        continue
    if method == 'model/list' and FAULT == 'no-result':
        print(json.dumps({'id': req['id']}), flush=True)
        continue
    if method == 'model/list' and FAULT == 'garbage':
        print(json.dumps({'id': req['id'], 'result': {'data': 'not a list'}}), flush=True)
        continue
    if method == 'model/list':
        cursor = req['params'].get('cursor')
        if FAULT == 'cycle':
            result = {'data': [entry('model-a')], 'nextCursor': 'same'}
        elif FAULT == 'endless':
            result = {'data': [entry('model-%d' % i) for i in range(100)], 'nextCursor': 'c%d' % (hash(cursor) % 100000)}
        elif cursor is None:
            result = {'data': [entry('model-a'), entry('model-hidden', True)], 'nextCursor': 'page-2'}
        else:
            result = {'data': [entry('model-b')], 'nextCursor': None}
        print(json.dumps({'id': req['id'], 'result': result}), flush=True)
        continue
    print(json.dumps({'id': req['id'], 'result': {}}), flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A fork in another test can briefly hold the write handle and fail the first exec with
        // ETXTBSY; get past that here.
        for _ in 0..50 {
            let primed = std::process::Command::new(&executable)
                .arg("--prime")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if primed.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        (dir, executable)
    }

    fn trace(dir: &tempfile::TempDir) -> Vec<Value> {
        std::fs::read_to_string(dir.path().join("trace.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn names(entries: &[Value]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| entry["model"].as_str().unwrap())
            .collect()
    }

    const TIMEOUT: Duration = Duration::from_secs(20);

    #[test]
    fn every_page_is_read_in_order_and_hidden_models_are_not_asked_for() {
        let (dir, codex) = fake_codex("pages");
        let home = tempfile::tempdir().unwrap();
        let entries = list_models(&codex, home.path(), TIMEOUT).unwrap();
        // The fake server hides one model itself when asked for hidden ones; ours asks not to.
        assert_eq!(names(&entries), ["model-a", "model-hidden", "model-b"]);
        let calls = trace(&dir);
        let methods: Vec<&str> = calls
            .iter()
            .skip(1)
            .map(|call| call["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            ["initialize", "initialized", "model/list", "model/list"]
        );
        assert_eq!(
            calls[3]["params"],
            json!({"includeHidden": false, "limit": 100, "cursor": null})
        );
        assert_eq!(calls[4]["params"]["cursor"], "page-2");
        assert_eq!(calls[1]["params"]["capabilities"]["experimentalApi"], true);
    }

    #[test]
    fn the_server_runs_as_app_server_over_stdio_with_the_given_data_directory() {
        let (dir, codex) = fake_codex("pages");
        let home = tempfile::tempdir().unwrap();
        list_models(&codex, home.path(), TIMEOUT).unwrap();
        let started = &trace(&dir)[0];
        assert_eq!(
            started["argv"],
            json!(["app-server", "--listen", "stdio://"])
        );
        assert_eq!(started["codex_home"], home.path().to_str().unwrap());
        // Nowhere near a project.
        let cwd = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(
            std::fs::canonicalize(started["cwd"].as_str().unwrap()).unwrap(),
            cwd
        );
    }

    #[test]
    fn only_listing_requests_are_ever_sent() {
        let (dir, codex) = fake_codex("pages");
        let home = tempfile::tempdir().unwrap();
        list_models(&codex, home.path(), TIMEOUT).unwrap();
        for call in trace(&dir).iter().skip(1) {
            let method = call["method"].as_str().unwrap();
            assert!(
                ["initialize", "initialized", "model/list"].contains(&method),
                "{method}"
            );
        }
    }

    #[test]
    fn messages_the_server_makes_up_on_its_own_are_not_taken_for_answers() {
        let (_dir, codex) = fake_codex("chatty");
        let home = tempfile::tempdir().unwrap();
        let entries = list_models(&codex, home.path(), TIMEOUT).unwrap();
        assert_eq!(names(&entries), ["model-a", "model-hidden", "model-b"]);
    }

    #[test]
    fn a_catalog_that_repeats_its_cursor_stops() {
        let (dir, codex) = fake_codex("cycle");
        let home = tempfile::tempdir().unwrap();
        let entries = list_models(&codex, home.path(), TIMEOUT).unwrap();
        assert_eq!(names(&entries), ["model-a", "model-a"]);
        let pages = trace(&dir)
            .iter()
            .filter(|call| call["method"] == "model/list")
            .count();
        assert_eq!(pages, 2);
    }

    #[test]
    fn a_catalog_that_never_ends_is_cut() {
        let (_dir, codex) = fake_codex("endless");
        let home = tempfile::tempdir().unwrap();
        let entries = list_models(&codex, home.path(), TIMEOUT).unwrap();
        assert!(entries.len() <= MAX_ENTRIES + 100, "{}", entries.len());
        assert!(entries.len() >= 100);
    }

    #[test]
    fn a_rejected_or_malformed_answer_is_an_error() {
        for (fault, expect) in [
            ("reject", "catalog unavailable"),
            ("no-result", "no result"),
            ("garbage", "model catalog"),
        ] {
            let (_dir, codex) = fake_codex(fault);
            let home = tempfile::tempdir().unwrap();
            let error = list_models(&codex, home.path(), TIMEOUT).unwrap_err();
            assert!(error.contains(expect), "{fault}: {error}");
        }
    }

    #[test]
    fn a_silent_server_is_given_up_on_at_the_deadline_and_closed() {
        let (_dir, codex) = fake_codex("silent");
        let home = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let error = list_models(&codex, home.path(), Duration::from_millis(800)).unwrap_err();
        assert!(error.contains("in time"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_server_that_exits_at_once_is_an_error() {
        let (_dir, codex) = fake_codex("exit");
        let home = tempfile::tempdir().unwrap();
        assert!(list_models(&codex, home.path(), TIMEOUT).is_err());
    }

    #[test]
    fn a_missing_executable_is_an_error_not_a_hang() {
        let home = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let result = list_models(
            Path::new("/nonexistent/askhuman-no-such-codex"),
            home.path(),
            Duration::from_secs(3),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
