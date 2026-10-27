//! The production [`Transport`]: a client of the CloudKit helper app's RPC (plan §3.3).
//!
//! The helper is a thin, stateless `.app` that owns the iCloud entitlement. It listens on a unix
//! socket (`<sidecar dir>/helper.sock`); every request and every reply is one JSON object on one
//! line. This module is the only place that knows the wire format, which is pinned by the fixtures in
//! `todo-sync-helper/Fixtures/rpc-v1` (the Swift side tests against the same files).
//!
//! Failure policy: anything that means "the helper cannot be used right now" is a *soft*
//! [`NetError::Unavailable`] (the tick still does all of its local work); a call that outlives its
//! time budget is [`NetError::TimedOut`], and the connection is dropped because the stream is then
//! out of step with the helper.

use super::transport::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// RPC protocol number; a helper that answers differently is not talked to (plan §2).
pub const PROTOCOL: u64 = 1;

/// Where the helper listens, given the sidecar directory.
pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("helper.sock")
}

/// The `hello` reply: who the helper is and what it can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloInfo {
    pub protocol: u64,
    pub helper_version: String,
    /// CloudKit environment of the build: `production` or `development`.
    pub environment: String,
    pub container: String,
    pub account: AccountInfo,
    /// Why this build cannot reach CloudKit at all (for example `missingEntitlement`).
    pub problem: Option<String>,
}

/// What a quick `hello` found out about the helper (status displays; never blocks for long).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// Nothing listens on the socket: the app is not installed, not started, or crashed.
    NotRunning,
    /// The helper answers but speaks another protocol (it, or this app, needs updating).
    Incompatible {
        helper_protocol: u64,
    },
    Running(HelloInfo),
}

fn parse_account_status(raw: &str) -> AccountStatus {
    match raw {
        "available" => AccountStatus::Available,
        "noAccount" => AccountStatus::NoAccount,
        "restricted" => AccountStatus::Restricted,
        _ => AccountStatus::Unknown,
    }
}

fn parse_hello(v: &Value) -> Result<HelloInfo, NetError> {
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let Some(protocol) = v.get("protocol").and_then(Value::as_u64) else {
        return Err(malformed("hello has no protocol"));
    };
    Ok(HelloInfo {
        protocol,
        helper_version: text("helperVersion").unwrap_or_default(),
        environment: text("environment").unwrap_or_default(),
        container: text("container").unwrap_or_default(),
        account: AccountInfo {
            status: parse_account_status(&text("accountStatus").unwrap_or_default()),
            hash: text("accountHash"),
        },
        problem: text("problem"),
    })
}

fn malformed(what: &str) -> NetError {
    NetError::Other(format!("malformed reply from the sync helper: {what}"))
}

/// The `error` object of a failed reply, as the core understands it.
fn parse_error(err: &Value) -> NetError {
    let code = err.get("code").and_then(Value::as_str).unwrap_or("other");
    let message = err.get("message").and_then(Value::as_str).unwrap_or("");
    match code {
        "offline" => NetError::Offline,
        "rateLimited" => NetError::RateLimited {
            retry_after_ms: err
                .get("retryAfterMs")
                .and_then(Value::as_u64)
                .unwrap_or(30_000),
        },
        "notAuthenticated" => NetError::NotAuthenticated,
        "zoneNotFound" => NetError::ZoneNotFound,
        "tokenExpired" => NetError::TokenExpired,
        "quota" => NetError::Quota,
        _ => NetError::Other(format!("{code}: {message}")),
    }
}

fn describe_problem(problem: &str) -> String {
    match problem {
        "missingEntitlement" => {
            "this helper build has no iCloud entitlement (it must be signed with the CloudKit \
             profile)"
                .to_string()
        }
        other => format!("the helper cannot reach CloudKit ({other})"),
    }
}

fn b64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn b64_decode(text: &str, what: &str) -> Result<Vec<u8>, NetError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| malformed(what))
}

fn parse_fetch(v: &Value) -> Result<FetchPage, NetError> {
    let str_of = |item: &Value, key: &str| -> Result<String, NetError> {
        item.get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| malformed(&format!("fetch record without {key}")))
    };
    let mut changes = Vec::new();
    for item in v
        .get("changes")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        changes.push(Change {
            name: str_of(item, "recordName")?,
            rtype: str_of(item, "recordType")?,
            payload: b64_decode(&str_of(item, "payload")?, "payload is not base64")?,
            tag: str_of(item, "changeTag")?,
        });
    }
    let mut deletions = Vec::new();
    for item in v
        .get("deletions")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        deletions.push(Deletion {
            name: str_of(item, "recordName")?,
            rtype: str_of(item, "recordType")?,
        });
    }
    Ok(FetchPage {
        changes,
        deletions,
        token: str_of(v, "token")?,
        more_coming: v
            .get("moreComing")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn save_request(records: &[OutRecord]) -> Value {
    let items: Vec<Value> = records
        .iter()
        .map(|r| {
            let mut item = json!({
                "recordName": r.name,
                "recordType": r.rtype,
                "payload": b64_encode(&r.payload),
            });
            if let Some(tag) = &r.tag {
                item["changeTag"] = Value::String(tag.clone());
            }
            item
        })
        .collect();
    json!({ "records": items })
}

fn parse_save(v: &Value) -> Result<Vec<SaveResult>, NetError> {
    let mut out = Vec::new();
    for item in v
        .get("results")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let name = item
            .get("recordName")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("save result without recordName"))?
            .to_string();
        let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
        out.push(match item.get("status").and_then(Value::as_str) {
            Some("saved") => SaveResult::Saved {
                name,
                tag: text("changeTag").ok_or_else(|| malformed("saved without changeTag"))?,
            },
            Some("conflict") => SaveResult::Conflict {
                name,
                server_payload: b64_decode(
                    &text("serverPayload").ok_or_else(|| malformed("conflict without payload"))?,
                    "serverPayload is not base64",
                )?,
                server_tag: text("serverChangeTag")
                    .ok_or_else(|| malformed("conflict without serverChangeTag"))?,
            },
            Some("error") => SaveResult::Error {
                name,
                code: text("code").unwrap_or_else(|| "other".to_string()),
            },
            _ => return Err(malformed("unknown save status")),
        });
    }
    Ok(out)
}

/// Per-call limits (the tick's own deadline can only make them shorter).
const HELLO_LIMIT: Duration = Duration::from_secs(10);
const ENSURE_LIMIT: Duration = Duration::from_secs(45);
const FETCH_LIMIT: Duration = Duration::from_secs(60);
const SAVE_LIMIT: Duration = Duration::from_secs(60);

#[cfg(unix)]
pub use unix::HelperClient;

#[cfg(not(unix))]
pub use fallback::HelperClient;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    /// Replies are one line; a runaway one must not eat the memory of a CLI process.
    const MAX_REPLY_BYTES: u64 = 64 * 1024 * 1024;

    struct Conn {
        reader: BufReader<UnixStream>,
    }

    pub struct HelperClient {
        socket: PathBuf,
        deadline: Option<Instant>,
        next_id: u64,
        conn: Option<Conn>,
        /// The last good `hello` of this connection (also what `account` reports from).
        handshake: Option<HelloInfo>,
    }

    impl HelperClient {
        pub fn new(socket: PathBuf) -> HelperClient {
            HelperClient {
                socket,
                deadline: None,
                next_id: 1,
                conn: None,
                handshake: None,
            }
        }

        /// No call is allowed to run past this instant (whats-next's pull budget).
        pub fn with_deadline(mut self, deadline: Option<Instant>) -> HelperClient {
            self.deadline = deadline;
            self
        }

        pub fn set_deadline(&mut self, deadline: Option<Instant>) {
            self.deadline = deadline;
        }

        /// The handshake of this connection, if one succeeded.
        pub fn hello_info(&self) -> Option<&HelloInfo> {
            self.handshake.as_ref()
        }

        #[cfg(test)]
        pub fn connected(&self) -> bool {
            self.conn.is_some()
        }

        fn budget(&self, limit: Duration) -> Result<Duration, NetError> {
            let Some(deadline) = self.deadline else {
                return Ok(limit);
            };
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(NetError::TimedOut);
            }
            Ok(limit.min(left))
        }

        fn connect(&mut self, budget: Duration) -> Result<(), NetError> {
            if self.conn.is_some() {
                return Ok(());
            }
            // `UnixStream::connect` does not block for long: the kernel answers at once whether
            // anyone is listening.
            let stream = UnixStream::connect(&self.socket).map_err(|e| match e.kind() {
                ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
                    NetError::Unavailable("the sync helper is not running".to_string())
                }
                _ => NetError::Unavailable(format!("cannot reach the sync helper: {e}")),
            })?;
            let _ = stream.set_write_timeout(Some(budget));
            self.conn = Some(Conn {
                reader: BufReader::new(stream),
            });
            Ok(())
        }

        /// One request/reply round trip. Returns the reply object on `ok: true`.
        fn call(&mut self, op: &str, fields: Value, limit: Duration) -> Result<Value, NetError> {
            let budget = self.budget(limit)?;
            self.connect(budget)?;
            let id = self.next_id;
            self.next_id += 1;
            let mut request = json!({ "id": id, "op": op });
            if let (Some(target), Value::Object(extra)) = (request.as_object_mut(), fields) {
                target.extend(extra);
            }
            let mut line = request.to_string();
            line.push('\n');

            let started = Instant::now();
            let result = self.exchange(id, line.as_bytes(), budget, started);
            if result.is_err() {
                // Out of step with the helper (or gone): the next call starts over.
                self.conn = None;
            }
            let reply = result?;
            if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                Ok(reply)
            } else {
                Err(reply
                    .get("error")
                    .map_or_else(|| malformed("failure without error"), parse_error))
            }
        }

        fn exchange(
            &mut self,
            id: u64,
            request: &[u8],
            budget: Duration,
            started: Instant,
        ) -> Result<Value, NetError> {
            let conn = self.conn.as_mut().expect("connected above");
            let stream = conn.reader.get_mut();
            let _ = stream.set_write_timeout(Some(budget));
            stream.write_all(request).map_err(|e| io_failure(&e))?;
            // Replies to calls we gave up on never reach a fresh connection, but skip strays anyway.
            for _ in 0..8 {
                let left = budget.saturating_sub(started.elapsed());
                if left.is_zero() {
                    return Err(NetError::TimedOut);
                }
                let _ = conn.reader.get_ref().set_read_timeout(Some(left));
                let mut buf = Vec::new();
                let read = (&mut conn.reader)
                    .take(MAX_REPLY_BYTES)
                    .read_until(b'\n', &mut buf)
                    .map_err(|e| io_failure(&e))?;
                if read == 0 {
                    return Err(NetError::Unavailable(
                        "the sync helper closed the connection".to_string(),
                    ));
                }
                if buf.last() != Some(&b'\n') {
                    return Err(malformed("reply is too large or cut off"));
                }
                let reply: Value = serde_json::from_slice(&buf)
                    .map_err(|_| malformed("reply is not valid JSON"))?;
                if reply.get("id").and_then(Value::as_u64) == Some(id) {
                    return Ok(reply);
                }
            }
            Err(malformed("no reply to this request"))
        }

        /// Handshake once per connection: refuses helpers of another protocol or without CloudKit.
        fn handshake(&mut self) -> Result<HelloInfo, NetError> {
            if let Some(info) = &self.handshake {
                return Ok(info.clone());
            }
            let reply = self.call("hello", json!({}), HELLO_LIMIT)?;
            let info = parse_hello(&reply)?;
            if info.protocol != PROTOCOL {
                return Err(NetError::Unavailable(format!(
                    "the sync helper speaks protocol {} but this app speaks {PROTOCOL}; update \
                     the helper",
                    info.protocol
                )));
            }
            if let Some(problem) = &info.problem {
                return Err(NetError::Unavailable(describe_problem(problem)));
            }
            self.handshake = Some(info.clone());
            Ok(info)
        }

        /// The helper's own bookkeeping (push registration, last tick, ...), for status displays.
        pub fn status(&mut self) -> Result<Value, NetError> {
            self.handshake()?;
            self.call("status", json!({}), HELLO_LIMIT)
        }
    }

    fn io_failure(e: &std::io::Error) -> NetError {
        match e.kind() {
            ErrorKind::WouldBlock | ErrorKind::TimedOut => NetError::TimedOut,
            _ => NetError::Unavailable(format!("lost the sync helper: {e}")),
        }
    }

    impl Transport for HelperClient {
        fn account(&mut self) -> Result<AccountInfo, NetError> {
            Ok(self.handshake()?.account)
        }

        fn ensure(&mut self) -> Result<(), NetError> {
            self.handshake()?;
            self.call("ensure", json!({}), ENSURE_LIMIT).map(|_| ())
        }

        fn fetch(&mut self, token: Option<&str>) -> Result<FetchPage, NetError> {
            self.handshake()?;
            let fields = match token {
                Some(t) => json!({ "token": t }),
                None => json!({}),
            };
            parse_fetch(&self.call("fetch", fields, FETCH_LIMIT)?)
        }

        fn save(&mut self, records: &[OutRecord]) -> Result<Vec<SaveResult>, NetError> {
            self.handshake()?;
            parse_save(&self.call("save", save_request(records), SAVE_LIMIT)?)
        }

        fn delete_zone(&mut self) -> Result<(), NetError> {
            self.handshake()?;
            self.call("deleteZone", json!({}), SAVE_LIMIT).map(|_| ())
        }
    }

    /// One quick `hello` (1 second at most): is the helper there, and does it fit this app?
    pub fn probe(socket: &Path) -> Reach {
        let mut client = HelperClient::new(socket.to_path_buf());
        let hello = client.call("hello", json!({}), Duration::from_secs(1));
        match hello.and_then(|reply| parse_hello(&reply)) {
            Ok(info) if info.protocol == PROTOCOL => Reach::Running(info),
            Ok(info) => Reach::Incompatible {
                helper_protocol: info.protocol,
            },
            Err(_) => Reach::NotRunning,
        }
    }
}

#[cfg(unix)]
pub use unix::probe;

#[cfg(not(unix))]
mod fallback {
    use super::*;

    /// The helper is a macOS app; elsewhere there is nothing to talk to.
    pub struct HelperClient;

    impl HelperClient {
        pub fn new(_socket: PathBuf) -> HelperClient {
            HelperClient
        }

        pub fn with_deadline(self, _deadline: Option<std::time::Instant>) -> HelperClient {
            self
        }

        pub fn hello_info(&self) -> Option<&HelloInfo> {
            None
        }

        pub fn status(&mut self) -> Result<Value, NetError> {
            Err(unsupported())
        }
    }

    fn unsupported() -> NetError {
        NetError::Unavailable("iCloud sync is only available on macOS".to_string())
    }

    impl Transport for HelperClient {
        fn account(&mut self) -> Result<AccountInfo, NetError> {
            Err(unsupported())
        }
        fn ensure(&mut self) -> Result<(), NetError> {
            Err(unsupported())
        }
        fn fetch(&mut self, _token: Option<&str>) -> Result<FetchPage, NetError> {
            Err(unsupported())
        }
        fn save(&mut self, _records: &[OutRecord]) -> Result<Vec<SaveResult>, NetError> {
            Err(unsupported())
        }
        fn delete_zone(&mut self) -> Result<(), NetError> {
            Err(unsupported())
        }
    }
}

#[cfg(not(unix))]
pub fn probe(_socket: &Path) -> Reach {
    Reach::NotRunning
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    // ---- fixtures (shared with the Swift side) ----

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../todo-sync-helper/Fixtures/rpc-v1")
    }

    #[derive(Clone)]
    struct Fixture {
        name: String,
        request: Value,
        response: Value,
    }

    fn fixture(file: &str) -> Fixture {
        let text = std::fs::read_to_string(fixtures_dir().join(file))
            .unwrap_or_else(|e| panic!("fixture {file}: {e}"));
        let v: Value = serde_json::from_str(&text).unwrap();
        Fixture {
            name: v["name"].as_str().unwrap().to_string(),
            request: v["request"].clone(),
            response: v["response"].clone(),
        }
    }

    fn without_id(v: &Value) -> Value {
        let mut v = v.clone();
        v.as_object_mut().unwrap().remove("id");
        v
    }

    // ---- a fake helper ----

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// Sockets live under /tmp: the path of a unix socket is limited to ~104 bytes.
    fn temp_socket() -> PathBuf {
        let dir = PathBuf::from(format!(
            "/tmp/ahp-hc-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("helper.sock")
    }

    /// What the fake does with each request line.
    type Script = Box<dyn FnMut(&Value) -> Option<Value> + Send>;

    struct FakeHelper {
        socket: PathBuf,
        seen: Arc<Mutex<Vec<Value>>>,
    }

    impl FakeHelper {
        /// `script` maps a request to a reply (`None` = stay silent). The reply's `id` is set to the
        /// request's, like the real helper does.
        fn start(mut script: Script) -> FakeHelper {
            let socket = temp_socket();
            let listener = UnixListener::bind(&socket).unwrap();
            let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { return };
                    let mut writer = stream.try_clone().unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 {
                        let request: Value = serde_json::from_str(&line).unwrap();
                        line.clear();
                        log.lock().unwrap().push(request.clone());
                        if let Some(mut reply) = script(&request) {
                            reply["id"] = request["id"].clone();
                            let mut text = reply.to_string();
                            text.push('\n');
                            if writer.write_all(text.as_bytes()).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
            FakeHelper { socket, seen }
        }

        /// Replies with the recorded fixtures, after checking each request equals the recorded one.
        fn from_fixtures(files: &[&str]) -> FakeHelper {
            let fixtures: Vec<Fixture> = files.iter().map(|f| fixture(f)).collect();
            let mut index = 0usize;
            FakeHelper::start(Box::new(move |request| {
                let f = &fixtures[index];
                index += 1;
                assert_eq!(
                    without_id(request),
                    without_id(&f.request),
                    "request differs from fixture `{}`",
                    f.name
                );
                Some(f.response.clone())
            }))
        }

        fn client(&self) -> HelperClient {
            HelperClient::new(self.socket.clone())
        }
    }

    impl Drop for FakeHelper {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.socket.parent().unwrap());
        }
    }

    // ---- protocol ----

    #[test]
    fn hello_reports_the_account() {
        let helper = FakeHelper::from_fixtures(&["hello.json"]);
        let account = helper.client().account().unwrap();
        assert_eq!(account.status, AccountStatus::Available);
        assert_eq!(account.hash.as_deref(), Some("0123456789abcdef"));
    }

    #[test]
    fn no_icloud_account_is_reported_as_such() {
        let helper = FakeHelper::from_fixtures(&["hello-no-account.json"]);
        let account = helper.client().account().unwrap();
        assert_eq!(account.status, AccountStatus::NoAccount);
        assert_eq!(account.hash, None);
    }

    #[test]
    fn a_helper_that_cannot_reach_cloudkit_is_unavailable_not_offline() {
        let helper = FakeHelper::from_fixtures(&["hello-problem.json"]);
        let err = helper.client().account().unwrap_err();
        assert!(
            matches!(&err, NetError::Unavailable(m) if m.contains("entitlement")),
            "{err:?}"
        );
    }

    #[test]
    fn a_helper_with_another_protocol_is_refused() {
        let helper = FakeHelper::start(Box::new(|_| {
            let mut hello = fixture("hello.json").response;
            hello["protocol"] = json!(PROTOCOL + 1);
            Some(hello)
        }));
        let mut client = helper.client();
        let err = client.account().unwrap_err();
        assert!(
            matches!(&err, NetError::Unavailable(m) if m.contains("protocol")),
            "{err:?}"
        );
        // No further call goes through after a refused handshake.
        assert!(client.fetch(None).is_err());
        assert!(helper
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|r| r["op"] == "hello"));
    }

    #[test]
    fn handshake_happens_once_per_connection() {
        let helper = FakeHelper::from_fixtures(&["hello.json", "ensure.json"]);
        let mut client = helper.client();
        client.account().unwrap();
        client.ensure().unwrap();
        let ops: Vec<String> = helper
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["op"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ops, ["hello", "ensure"]);
    }

    #[test]
    fn operations_other_than_account_handshake_first() {
        let helper = FakeHelper::from_fixtures(&["hello.json", "delete-zone.json"]);
        helper.client().delete_zone().unwrap();
    }

    #[test]
    fn fetch_parses_pages_and_deletions() {
        let helper =
            FakeHelper::from_fixtures(&["hello.json", "fetch-first.json", "fetch-next.json"]);
        let mut client = helper.client();
        let first = client.fetch(None).unwrap();
        assert!(first.more_coming);
        assert_eq!(first.token, "em9uZS10b2tlbi0x");
        assert_eq!(first.changes.len(), 2);
        assert_eq!(first.changes[0].name, "todo-1");
        assert_eq!(first.changes[0].rtype, TYPE_TODO_NAME);
        assert_eq!(first.changes[0].tag, "tag-a1");
        assert!(first.changes[0].payload.starts_with(b"{\"v\":1"));
        assert_eq!(first.changes[1].rtype, "Device");

        let next = client.fetch(Some(&first.token)).unwrap();
        assert!(!next.more_coming);
        assert!(next.changes.is_empty());
        assert_eq!(
            next.deletions,
            vec![Deletion {
                name: "todo-9".into(),
                rtype: "Todo".into()
            }]
        );
        assert_eq!(next.token, "em9uZS10b2tlbi0y");
    }

    const TYPE_TODO_NAME: &str = "Todo";

    #[test]
    fn save_sends_the_fixture_request_and_parses_every_status() {
        let helper = FakeHelper::from_fixtures(&["hello.json", "save-mixed.json"]);
        let request = fixture("save-mixed.json").request;
        let records: Vec<OutRecord> = request["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| OutRecord {
                name: r["recordName"].as_str().unwrap().into(),
                rtype: r["recordType"].as_str().unwrap().into(),
                payload: b64_decode(r["payload"].as_str().unwrap(), "payload").unwrap(),
                tag: r
                    .get("changeTag")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect();
        let results = helper.client().save(&records).unwrap();
        assert_eq!(
            results,
            vec![
                SaveResult::Saved {
                    name: "todo-2".into(),
                    tag: "tag-b1".into()
                },
                SaveResult::Saved {
                    name: "todo-1".into(),
                    tag: "tag-a2".into()
                },
                SaveResult::Conflict {
                    name: "todo-3".into(),
                    server_payload: b64_decode(
                        fixture("save-mixed.json").response["results"][2]["serverPayload"]
                            .as_str()
                            .unwrap(),
                        "serverPayload"
                    )
                    .unwrap(),
                    server_tag: "tag-c9".into()
                },
                SaveResult::Error {
                    name: "device-1".into(),
                    code: "quotaExceeded".into()
                },
            ]
        );
    }

    #[test]
    fn whole_call_failures_map_to_the_cores_error_kinds() {
        let cases: [(&str, NetError); 7] = [
            ("error-offline.json", NetError::Offline),
            (
                "error-rate-limited.json",
                NetError::RateLimited {
                    retry_after_ms: 30_000,
                },
            ),
            ("error-not-authenticated.json", NetError::NotAuthenticated),
            ("error-zone-not-found.json", NetError::ZoneNotFound),
            ("error-token-expired.json", NetError::TokenExpired),
            ("error-quota.json", NetError::Quota),
            (
                "error-other.json",
                NetError::Other("other: serverRejectedRequest".into()),
            ),
        ];
        for (file, expected) in cases {
            let f = fixture(file);
            let op = f.request["op"].as_str().unwrap().to_string();
            let helper = FakeHelper::start(Box::new({
                let f = f.clone();
                move |request| {
                    Some(if request["op"] == "hello" {
                        fixture("hello.json").response
                    } else {
                        f.response.clone()
                    })
                }
            }));
            let mut client = helper.client();
            let err = match op.as_str() {
                "fetch" => client.fetch(f.request["token"].as_str()).unwrap_err(),
                "ensure" => client.ensure().unwrap_err(),
                "save" => client.save(&[]).unwrap_err(),
                other => panic!("no case for {other}"),
            };
            assert_eq!(err, expected, "{file}");
        }
    }

    #[test]
    fn a_bad_request_reply_is_a_hard_error_with_its_message() {
        let helper = FakeHelper::start(Box::new(|request| {
            Some(if request["op"] == "hello" {
                fixture("hello.json").response
            } else {
                fixture("error-bad-request.json").response
            })
        }));
        let mut client = helper.client();
        client.account().unwrap();
        let err = client.ensure().unwrap_err();
        assert!(
            matches!(&err, NetError::Other(m) if m.contains("badRequest")),
            "{err:?}"
        );
    }

    #[test]
    fn status_passes_the_helpers_bookkeeping_through() {
        let helper = FakeHelper::from_fixtures(&["hello.json", "status.json"]);
        let status = helper.client().status().unwrap();
        assert_eq!(status["apns"], "registered");
        assert_eq!(status["ticksRun"], 12);
    }

    // ---- failure policy ----

    #[test]
    fn nothing_listening_is_unavailable() {
        let socket = temp_socket();
        let err = HelperClient::new(socket.clone()).account().unwrap_err();
        assert!(
            matches!(&err, NetError::Unavailable(m) if m.contains("not running")),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(socket.parent().unwrap());
    }

    #[test]
    fn a_stale_socket_file_is_unavailable() {
        // A crashed helper leaves the file behind; connecting is refused.
        let socket = temp_socket();
        drop(UnixListener::bind(&socket).unwrap());
        let err = HelperClient::new(socket.clone()).account().unwrap_err();
        assert!(matches!(err, NetError::Unavailable(_)), "{err:?}");
        let _ = std::fs::remove_dir_all(socket.parent().unwrap());
    }

    #[test]
    fn a_silent_helper_times_out_within_the_deadline() {
        let helper = FakeHelper::start(Box::new(|_| None));
        let mut client = helper
            .client()
            .with_deadline(Some(Instant::now() + Duration::from_millis(150)));
        let started = Instant::now();
        let err = client.account().unwrap_err();
        assert_eq!(err, NetError::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn an_expired_deadline_makes_no_call_at_all() {
        let helper = FakeHelper::from_fixtures(&["hello.json"]);
        let mut client = helper.client().with_deadline(Some(Instant::now()));
        assert_eq!(client.account().unwrap_err(), NetError::TimedOut);
        assert!(helper.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_helper_that_hangs_up_is_unavailable() {
        let socket = temp_socket();
        let listener = UnixListener::bind(&socket).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                drop(stream);
            }
        });
        let err = HelperClient::new(socket.clone()).account().unwrap_err();
        assert!(matches!(err, NetError::Unavailable(_)), "{err:?}");
        let _ = std::fs::remove_dir_all(socket.parent().unwrap());
    }

    #[test]
    fn garbage_replies_are_malformed_not_a_panic() {
        for garbage in ["not json at all\n", "{\"id\":1,\"ok\":true}\n"] {
            let socket = temp_socket();
            let listener = UnixListener::bind(&socket).unwrap();
            let reply = garbage.to_string();
            std::thread::spawn(move || {
                if let Ok((stream, _)) = listener.accept() {
                    let mut writer = stream.try_clone().unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    let _ = reader.read_line(&mut line);
                    let _ = writer.write_all(reply.as_bytes());
                }
            });
            let err = HelperClient::new(socket.clone()).account().unwrap_err();
            assert!(
                matches!(&err, NetError::Other(m) if m.contains("malformed")),
                "{err:?}"
            );
            let _ = std::fs::remove_dir_all(socket.parent().unwrap());
        }
    }

    #[test]
    fn a_timed_out_connection_is_not_reused() {
        // The reply to the first (abandoned) request must never be taken for the second one's.
        let calls = Arc::new(AtomicU64::new(0));
        let counter = calls.clone();
        let helper = FakeHelper::start(Box::new(move |request| {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            if request["op"] == "hello" {
                return Some(fixture("hello.json").response);
            }
            // The first fetch is never answered.
            (n > 1).then(|| fixture("fetch-next.json").response)
        }));
        let mut client = helper.client();
        assert!(client.account().is_ok());
        client.set_deadline(Some(Instant::now() + Duration::from_millis(150)));
        assert_eq!(client.fetch(None).unwrap_err(), NetError::TimedOut);
        assert!(!client.connected());
    }

    #[test]
    fn probe_distinguishes_missing_incompatible_and_running() {
        let socket = temp_socket();
        assert_eq!(probe(&socket), Reach::NotRunning);

        let helper = FakeHelper::from_fixtures(&["hello.json"]);
        assert!(matches!(probe(&helper.socket), Reach::Running(i) if i.protocol == PROTOCOL));

        let other = FakeHelper::start(Box::new(|_| {
            let mut hello = fixture("hello.json").response;
            hello["protocol"] = json!(7);
            Some(hello)
        }));
        assert_eq!(
            probe(&other.socket),
            Reach::Incompatible { helper_protocol: 7 }
        );
        let _ = std::fs::remove_dir_all(socket.parent().unwrap());
    }

    /// Manual check against a running helper app (plan P3): start it with
    /// `ASKHUMAN_TODO_SYNC_DIR=<dir>`, then run
    /// `ASKHUMAN_TEST_HELPER_SOCK=<dir>/helper.sock cargo test against_a_real_helper -- --ignored`.
    /// An unsigned build says `problem = missingEntitlement` and every operation fails softly
    /// (`Unavailable`); a signed one reaches iCloud (its own container, so run it against a
    /// Development build).
    #[test]
    #[ignore]
    fn against_a_real_helper() {
        let Some(socket) = std::env::var_os("ASKHUMAN_TEST_HELPER_SOCK") else {
            return;
        };
        let socket = PathBuf::from(socket);
        let Reach::Running(info) = probe(&socket) else {
            panic!("no compatible helper on {socket:?}");
        };
        eprintln!("helper: {info:?}");
        let mut client = HelperClient::new(socket);
        match &info.problem {
            Some(_) => {
                // Soft: the tick keeps doing its local work and the UI says why.
                let err = client.ensure().unwrap_err();
                assert!(matches!(err, NetError::Unavailable(_)), "{err:?}");
            }
            None => {
                client.ensure().unwrap();
                let page = client.fetch(None).unwrap();
                eprintln!(
                    "{} changes, {} deletions",
                    page.changes.len(),
                    page.deletions.len()
                );
            }
        }
    }
}
