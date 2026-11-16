//! The sidecar process: `AskHuman __antigravity-sidecar --id <id>`, which the App starts for each
//! sidecar directory AskHuman made (docs/specs/antigravity-deep-integration.md §12.3).
//!
//! What it does, and nothing else: connect to the daemon, say who it is, and run the few
//! `agentapi` calls the daemon sends (`link::Command`), answering with how each ended.
//!
//! - It **never starts the daemon.** The App starts every sidecar when it opens; if that could
//!   start the daemon, opening the App would keep it running for good. Without a daemon it
//!   reconnects with a bounded backoff, and the daemon is there when something needs it.
//! - It executes no shell command and reads no instruction from anywhere but the daemon's
//!   connection; the arguments of `agentapi` are built here from typed commands.
//! - SIGINT and SIGTERM end it cleanly (the App sends SIGINT when the sidecar is removed).

use super::link::{CheckIn, Command, Down, Outcome, Up, LINK_VERSION};
use crate::ipc::{self, ClientMsg, MessageReader, ServerMsg};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::mpsc;

/// The exit code that asks the App to start the sidecar again (`restart_policy: always`) from
/// the file on disk: used when this process is an older build than the daemon.
const EXIT_RESTART: i32 = 75;

/// How long one `agentapi` call may take before it is stopped. The daemon waits a little longer
/// than this, so its patience never ends first.
const AGENTAPI_TIMEOUT: Duration = Duration::from_secs(40);
const MAX_STDOUT: usize = 64 * 1024;
const MAX_STDERR: usize = 16 * 1024;
/// How many `agentapi` calls run at once.
const PARALLEL: usize = 4;

// MARK: - Reconnecting

/// How long to wait before the next try: 1, 2, 4, 8 seconds and then 10, until a connection is
/// accepted.
#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub fn next_delay(&mut self) -> Duration {
        let seconds = (1u64 << self.failures.min(4)).min(10);
        self.failures = self.failures.saturating_add(1);
        Duration::from_secs(seconds)
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

// MARK: - agentapi

/// `agentapi` as the App put it on this process's PATH, if it did.
pub fn find_agentapi() -> Option<PathBuf> {
    find_on_path("agentapi", std::env::var_os("PATH")?.as_os_str())
}

fn find_on_path(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Runs `program` with the arguments of `command`; the answer is what it printed and how it
/// ended, or why it did not end.
pub async fn execute(program: &Path, command: &Command, timeout: Duration) -> Outcome {
    let argv = match command.argv() {
        Ok(argv) => argv,
        Err(error) => return Outcome::failed_to_run(error),
    };
    let mut process = tokio::process::Command::new(program);
    process
        .args(&argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => return Outcome::failed_to_run(format!("could not start agentapi: {error}")),
    };
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Outcome::failed_to_run("could not read what agentapi prints");
    };
    let finished = async {
        let (out, err, status) = tokio::join!(
            read_limited(&mut stdout, MAX_STDOUT),
            read_limited(&mut stderr, MAX_STDERR),
            child.wait()
        );
        (out, err, status)
    };
    match tokio::time::timeout(timeout, finished).await {
        Ok((out, err, status)) => {
            let (status, error) = match status {
                Ok(status) => match status.code() {
                    Some(code) => (Some(code), None),
                    None => (None, Some("agentapi was stopped by a signal".to_string())),
                },
                Err(error) => (None, Some(format!("could not wait for agentapi: {error}"))),
            };
            Outcome {
                status,
                stdout: out,
                stderr: err,
                error,
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            Outcome::failed_to_run(format!(
                "agentapi did not finish within {} seconds",
                timeout.as_secs()
            ))
        }
    }
}

/// Reads to the end of the stream, keeps the first `limit` bytes and discards the rest (so the
/// process never blocks on a full pipe).
async fn read_limited<R: AsyncRead + Unpin>(stream: &mut R, limit: usize) -> String {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..count.min(room)]);
            }
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

// MARK: - One connection

/// How a connection to the daemon ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// The daemon closed it or it broke after it was accepted.
    Closed,
    /// The connection ended before the daemon accepted this sidecar.
    NotAccepted,
    /// The daemon does not want this sidecar; `restart` when it is an older build.
    Refused { reason: String, restart: bool },
}

/// Speaks to the daemon over one connection until it ends. `accepted` is called once when the
/// daemon welcomes the sidecar.
pub async fn session<R, W>(
    reader: R,
    mut writer: W,
    id: &str,
    program: Option<&Path>,
    mut accepted: impl FnMut(),
) -> Ended
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = MessageReader::new(reader);
    let check_in = CheckIn {
        id: id.to_string(),
        link_version: LINK_VERSION,
        project_id: std::env::var("ANTIGRAVITY_PROJECT_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        version: env!("CARGO_PKG_VERSION").to_string(),
        agentapi: program.is_some(),
        pid: std::process::id(),
    };
    if ipc::write_msg(
        &mut writer,
        &ClientMsg::AntigravitySidecar {
            frame: Up::CheckIn(check_in),
        },
    )
    .await
    .is_err()
    {
        return Ended::NotAccepted;
    }
    let (finished_tx, mut finished_rx) = mpsc::unbounded_channel::<(String, Outcome)>();
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(PARALLEL));
    let mut welcomed = false;
    loop {
        tokio::select! {
            frame = reader.read::<ServerMsg>() => match frame {
                Ok(Some(ServerMsg::AntigravitySidecar { frame })) => match frame {
                    Down::Welcome => {
                        if !welcomed {
                            welcomed = true;
                            accepted();
                        }
                    }
                    Down::Refused { reason, restart } => {
                        return Ended::Refused { reason, restart };
                    }
                    Down::Run { command_id, command } => {
                        let program = program.map(Path::to_path_buf);
                        let finished = finished_tx.clone();
                        let limit = limit.clone();
                        // The call runs to its end even if the connection goes away meanwhile:
                        // stopping `agentapi` halfway is worse than a result nobody reads.
                        tokio::spawn(async move {
                            let _permit = limit.acquire_owned().await;
                            let outcome = match program {
                                Some(program) => execute(&program, &command, AGENTAPI_TIMEOUT).await,
                                None => Outcome::failed_to_run(
                                    "agentapi is not on this sidecar's PATH",
                                ),
                            };
                            let _ = finished.send((command_id, outcome));
                        });
                    }
                    Down::Unknown => {}
                },
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    return if welcomed { Ended::Closed } else { Ended::NotAccepted };
                }
            },
            Some((command_id, outcome)) = finished_rx.recv() => {
                let sent = ipc::write_msg(
                    &mut writer,
                    &ClientMsg::AntigravitySidecar {
                        frame: Up::Done { command_id, outcome },
                    },
                )
                .await;
                if sent.is_err() {
                    return Ended::Closed;
                }
            }
        }
    }
}

// MARK: - The process

/// Connects, serves and reconnects until the daemon refuses an older build (the one reason to
/// return). `connect` makes a connection to the daemon *without starting it*.
pub async fn serve<C, Fut, R, W>(id: &str, program: Option<PathBuf>, mut connect: C) -> i32
where
    C: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::io::Result<(R, W)>>,
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut backoff = Backoff::default();
    loop {
        if let Ok((reader, writer)) = connect().await {
            let ended = session(reader, writer, id, program.as_deref(), || backoff.reset()).await;
            if let Ended::Refused {
                reason,
                restart: true,
            } = &ended
            {
                eprintln!("askhuman sidecar: {reason}; exiting so the App starts it again");
                return EXIT_RESTART;
            }
            if let Ended::Refused { reason, .. } = &ended {
                eprintln!("askhuman sidecar: {reason}");
            }
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// The entry of `AskHuman __antigravity-sidecar --id <id>`.
pub fn main(args: &[String]) -> i32 {
    let id = match args {
        [flag, id] if flag == "--id" => id.clone(),
        _ => {
            eprintln!("usage: AskHuman __antigravity-sidecar --id <id>");
            return 2;
        }
    };
    if !super::sidecar::valid_id(&id) {
        eprintln!("askhuman sidecar: {id:?} is not a sidecar id");
        return 2;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("askhuman sidecar: could not start: {error}");
            return 1;
        }
    };
    runtime.block_on(async {
        let program = find_agentapi();
        if program.is_none() {
            eprintln!("askhuman sidecar: agentapi was not found on PATH; tasks cannot be created");
        }
        tokio::select! {
            code = serve(&id, program, connect_daemon) => code,
            _ = shutdown() => 0,
        }
    })
}

/// A connection to the daemon, which is not started if it is not running.
async fn connect_daemon() -> std::io::Result<(
    tokio::io::BufReader<ipc::transport::OwnedReadHalf>,
    ipc::transport::OwnedWriteHalf,
)> {
    let (reader, writer) = crate::client::connect_stream().await?.into_split();
    Ok((tokio::io::BufReader::new(reader), writer))
}

/// Resolves when the process is asked to stop.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let terminate = signal(SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::BufReader;

    /// A stand-in for `agentapi`: a script that records its arguments, one per line.
    struct Fake {
        dir: tempfile::TempDir,
    }

    impl Fake {
        fn new(body: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("agentapi");
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}/args\"\n{body}\n",
                    dir.path().display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self { dir }
        }

        fn program(&self) -> PathBuf {
            self.dir.path().join("agentapi")
        }

        fn args(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("args"))
                .unwrap()
                .lines()
                .map(String::from)
                .collect()
        }
    }

    fn new_conversation(prompt: &str) -> Command {
        Command::NewConversation {
            prompt: prompt.to_string(),
            title: Some("A title".into()),
            model: Some("gemini-3.8-flash-low".into()),
        }
    }

    // MARK: agentapi

    #[tokio::test]
    async fn the_prompt_reaches_agentapi_as_one_argument_whatever_it_holds() {
        let fake = Fake::new("echo '{\"ok\":true}'");
        let prompt = "- fix \"this\" $HOME `ls`; rm -rf /\nsecond line with 'quotes' and *";
        let outcome = execute(
            &fake.program(),
            &new_conversation(prompt),
            Duration::from_secs(10),
        )
        .await;
        assert!(outcome.succeeded(), "{outcome:?}");
        assert_eq!(outcome.stdout.trim(), "{\"ok\":true}");
        let args = fake.args();
        // The prompt has two lines, so it spans two lines of the record; what matters is that it
        // came after `--` and nothing was interpreted.
        assert_eq!(
            &args[..4],
            [
                "new-conversation",
                "--model=gemini-3.8-flash-low",
                "--title=A title",
                "--"
            ]
        );
        assert_eq!(args[4..].join("\n"), prompt);
    }

    #[tokio::test]
    async fn what_agentapi_says_when_it_fails_is_kept() {
        let fake = Fake::new("echo 'about to fail' >&2; echo partial; exit 3");
        let outcome = execute(&fake.program(), &Command::Probe, Duration::from_secs(10)).await;
        assert_eq!(outcome.status, Some(3));
        assert_eq!(outcome.stdout.trim(), "partial");
        assert_eq!(outcome.stderr.trim(), "about to fail");
        assert_eq!(outcome.error, None);
        assert!(!outcome.succeeded());
        assert_eq!(
            outcome.failure_text(),
            "agentapi exited with status 3: about to fail"
        );
    }

    #[tokio::test]
    async fn a_call_that_does_not_finish_is_stopped() {
        let fake = Fake::new(&format!(
            "echo $$ > \"{}/pid\"\nexec sleep 30",
            "$(dirname \"$0\")"
        ));
        let started = std::time::Instant::now();
        let outcome = execute(&fake.program(), &Command::Probe, Duration::from_millis(300)).await;
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(outcome.status, None);
        assert!(outcome.error.as_deref().unwrap().contains("did not finish"));
        // The process is gone, not left running.
        let pid: i32 = std::fs::read_to_string(fake.dir.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut gone = false;
        for _ in 0..50 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(gone, "the process {pid} is still running");
    }

    #[tokio::test]
    async fn a_program_that_cannot_start_and_text_that_cannot_be_sent_fail_without_running() {
        let outcome = execute(
            Path::new("/nonexistent/agentapi"),
            &Command::Probe,
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(outcome.status, None);
        assert!(outcome
            .error
            .unwrap()
            .starts_with("could not start agentapi"));
        let fake = Fake::new("exit 0");
        let outcome = execute(
            &fake.program(),
            &new_conversation(""),
            Duration::from_secs(5),
        )
        .await;
        assert!(outcome.error.is_some());
        // It was never started: the fake would have recorded its arguments.
        assert!(!fake.dir.path().join("args").exists());
    }

    #[tokio::test]
    async fn a_lot_of_output_is_cut_and_does_not_stop_the_process() {
        let fake = Fake::new(
            "head -c 300000 /dev/zero | tr '\\0' 'x'; head -c 100000 /dev/zero | tr '\\0' 'e' >&2",
        );
        let outcome = execute(&fake.program(), &Command::Probe, Duration::from_secs(20)).await;
        assert_eq!(outcome.status, Some(0));
        assert_eq!(outcome.stdout.len(), MAX_STDOUT);
        assert_eq!(outcome.stderr.len(), MAX_STDERR);
    }

    #[test]
    fn agentapi_is_found_on_the_path_only_when_it_can_run() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = dir.path().join("agentapi");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        // Not executable yet.
        let path = std::env::join_paths([other.path(), dir.path()]).unwrap();
        assert_eq!(find_on_path("agentapi", &path), None);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_on_path("agentapi", &path), Some(file.clone()));
        // A relative entry of the path is not searched.
        let relative = std::env::join_paths(["bin"]).unwrap();
        assert_eq!(find_on_path("agentapi", &relative), None);
        // A directory of that name is not a program.
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("agentapi")).unwrap();
        assert_eq!(find_on_path("agentapi", folder.path().as_os_str()), None);
    }

    // MARK: backoff

    #[test]
    fn the_wait_between_tries_grows_to_a_cap_and_starts_over_after_a_connection() {
        let mut backoff = Backoff::default();
        let waits: Vec<u64> = (0..8).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 10, 10, 10, 10]);
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
        // Never overflows however long the daemon stays away.
        let mut backoff = Backoff { failures: u32::MAX };
        assert_eq!(backoff.next_delay(), Duration::from_secs(10));
    }

    // MARK: a connection

    /// The daemon's end of a connection, in memory.
    struct Daemon {
        reader: MessageReader<BufReader<tokio::io::DuplexStream>>,
        writer: tokio::io::DuplexStream,
    }

    fn pair() -> (
        (BufReader<tokio::io::DuplexStream>, tokio::io::DuplexStream),
        Daemon,
    ) {
        let (sidecar_out, daemon_in) = tokio::io::duplex(1 << 20);
        let (daemon_out, sidecar_in) = tokio::io::duplex(1 << 20);
        (
            (BufReader::new(sidecar_in), sidecar_out),
            Daemon {
                reader: MessageReader::new(BufReader::new(daemon_in)),
                writer: daemon_out,
            },
        )
    }

    impl Daemon {
        async fn up(&mut self) -> Up {
            match self.reader.read::<ClientMsg>().await.unwrap().unwrap() {
                ClientMsg::AntigravitySidecar { frame } => frame,
                other => panic!("not a sidecar frame: {other:?}"),
            }
        }

        async fn send(&mut self, frame: Down) {
            ipc::write_msg(&mut self.writer, &ServerMsg::AntigravitySidecar { frame })
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn a_sidecar_checks_in_then_runs_what_the_daemon_sends_and_reports_each_result() {
        let fake = Fake::new("echo \"{\\\"n\\\":$#}\"");
        let ((reader, writer), mut daemon) = pair();
        let program = fake.program();
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = accepted.clone();
        let sidecar = tokio::spawn(async move {
            session(
                reader,
                writer,
                "askhuman-ws-abcd1234",
                Some(&program),
                move || {
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                },
            )
            .await
        });

        let Up::CheckIn(check_in) = daemon.up().await else {
            panic!("the first frame is not a check-in")
        };
        assert_eq!(check_in.id, "askhuman-ws-abcd1234");
        assert_eq!(check_in.link_version, LINK_VERSION);
        assert!(check_in.agentapi);
        assert_eq!(check_in.pid, std::process::id());

        daemon.send(Down::Welcome).await;
        daemon.send(Down::Welcome).await;
        daemon
            .send(Down::Run {
                command_id: "c1".into(),
                command: Command::Probe,
            })
            .await;
        let Up::Done {
            command_id,
            outcome,
        } = daemon.up().await
        else {
            panic!("not a result")
        };
        assert_eq!(command_id, "c1");
        assert!(outcome.succeeded());
        assert_eq!(outcome.stdout.trim(), "{\"n\":2}");
        // A frame from a later version is passed over.
        daemon.send(Down::Unknown).await;
        daemon
            .send(Down::Run {
                command_id: "c2".into(),
                command: new_conversation("hi"),
            })
            .await;
        let Up::Done {
            command_id,
            outcome,
        } = daemon.up().await
        else {
            panic!("not a result")
        };
        assert_eq!(command_id, "c2");
        assert_eq!(outcome.stdout.trim(), "{\"n\":5}");
        assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);

        // The daemon goes away: the connection ended after it was accepted.
        drop(daemon);
        assert_eq!(sidecar.await.unwrap(), Ended::Closed);
    }

    #[tokio::test]
    async fn without_agentapi_every_command_fails_to_run_and_says_why() {
        let ((reader, writer), mut daemon) = pair();
        let sidecar = tokio::spawn(async move {
            session(reader, writer, "askhuman-ws-abcd1234", None, || {}).await
        });
        let Up::CheckIn(check_in) = daemon.up().await else {
            panic!("not a check-in")
        };
        assert!(!check_in.agentapi);
        daemon.send(Down::Welcome).await;
        daemon
            .send(Down::Run {
                command_id: "c1".into(),
                command: Command::Probe,
            })
            .await;
        let Up::Done { outcome, .. } = daemon.up().await else {
            panic!("not a result")
        };
        assert_eq!(outcome.status, None);
        assert!(outcome
            .error
            .unwrap()
            .contains("not on this sidecar's PATH"));
        drop(daemon);
        let _ = sidecar.await;
    }

    #[tokio::test]
    async fn a_refusal_ends_the_connection_and_says_whether_to_restart() {
        for restart in [false, true] {
            let ((reader, writer), mut daemon) = pair();
            let sidecar = tokio::spawn(async move {
                session(reader, writer, "askhuman-ws-abcd1234", None, || {}).await
            });
            let _ = daemon.up().await;
            daemon
                .send(Down::Refused {
                    reason: "older".into(),
                    restart,
                })
                .await;
            assert_eq!(
                sidecar.await.unwrap(),
                Ended::Refused {
                    reason: "older".into(),
                    restart
                }
            );
        }
    }

    #[tokio::test]
    async fn a_connection_that_ends_before_the_welcome_was_not_accepted() {
        let ((reader, writer), mut daemon) = pair();
        let sidecar = tokio::spawn(async move {
            session(reader, writer, "askhuman-ws-abcd1234", None, || {}).await
        });
        let _ = daemon.up().await;
        drop(daemon);
        assert_eq!(sidecar.await.unwrap(), Ended::NotAccepted);
    }

    #[tokio::test]
    async fn a_command_runs_to_its_end_even_when_the_connection_is_gone() {
        let fake = Fake::new("sleep 1; echo done > \"$(dirname \"$0\")/finished\"");
        let ((reader, writer), mut daemon) = pair();
        let program = fake.program();
        let sidecar = tokio::spawn(async move {
            session(
                reader,
                writer,
                "askhuman-ws-abcd1234",
                Some(&program),
                || {},
            )
            .await
        });
        let _ = daemon.up().await;
        daemon.send(Down::Welcome).await;
        daemon
            .send(Down::Run {
                command_id: "c1".into(),
                command: Command::Probe,
            })
            .await;
        // Wait until the script has started, then drop the connection.
        for _ in 0..100 {
            if fake.dir.path().join("args").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(daemon);
        let _ = sidecar.await;
        for _ in 0..200 {
            if fake.dir.path().join("finished").exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("agentapi was stopped before it finished");
    }

    // MARK: the process

    #[tokio::test(start_paused = true)]
    async fn the_sidecar_keeps_trying_with_growing_waits_and_serves_once_the_daemon_is_there() {
        let started = tokio::time::Instant::now();
        let attempts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = attempts.clone();
        let mut daemons = Vec::new();
        let ((reader, writer), daemon) = pair();
        daemons.push(((reader, writer), daemon));
        let slot = std::sync::Arc::new(std::sync::Mutex::new(daemons));
        let served = slot.clone();
        let connect = move || {
            let log = log.clone();
            let served = served.clone();
            async move {
                log.lock().unwrap().push(started.elapsed().as_secs());
                if log.lock().unwrap().len() <= 4 {
                    return Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
                }
                let Some(((reader, writer), _daemon)) = served.lock().unwrap().pop() else {
                    return Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
                };
                Ok((reader, writer))
            }
        };
        // The fifth try connects; the daemon end is dropped at once, so the session ends
        // unaccepted and the sidecar goes on trying (the test ends it from outside).
        let result = tokio::time::timeout(
            Duration::from_secs(40),
            serve("askhuman-ws-abcd1234", None, connect),
        )
        .await;
        assert!(result.is_err(), "serve returned: {result:?}");
        let attempts = attempts.lock().unwrap().clone();
        // Waits of 1, 2, 4 and 8 seconds before the first five tries, then 10 seconds each.
        assert_eq!(&attempts[..5], [0, 1, 3, 7, 15]);
    }

    #[tokio::test(start_paused = true)]
    async fn an_older_sidecar_exits_when_the_daemon_says_so_and_a_newer_one_waits() {
        for (restart, exits) in [(true, true), (false, false)] {
            let ((reader, writer), daemon) = pair();
            let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(((reader, writer), daemon))));
            let connect = move || {
                let slot = slot.clone();
                async move {
                    match slot.lock().unwrap().take() {
                        Some(((reader, writer), mut daemon)) => {
                            tokio::spawn(async move {
                                let _ = daemon.up().await;
                                daemon
                                    .send(Down::Refused {
                                        reason: "version".into(),
                                        restart,
                                    })
                                    .await;
                                // Keep the connection's far end alive until the test is over.
                                tokio::time::sleep(Duration::from_secs(3600)).await;
                                drop(daemon);
                            });
                            Ok((reader, writer))
                        }
                        None => Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)),
                    }
                }
            };
            let result = tokio::time::timeout(
                Duration::from_secs(60),
                serve("askhuman-ws-abcd1234", None, connect),
            )
            .await;
            match (exits, result) {
                (true, Ok(code)) => assert_eq!(code, EXIT_RESTART),
                (false, Err(_)) => {}
                (exits, result) => panic!("exits={exits}: {result:?}"),
            }
        }
    }

    #[test]
    fn the_command_line_needs_a_sidecar_id() {
        for args in [
            vec![],
            vec!["--id".to_string()],
            vec!["--id".to_string(), "../../etc".to_string()],
            vec!["--id".to_string(), "other-name".to_string()],
            vec!["--name".to_string(), "askhuman-ws-abcd1234".to_string()],
        ] {
            assert_eq!(main(&args), 2, "{args:?}");
        }
    }
}
