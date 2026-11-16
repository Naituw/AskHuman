use super::*;
use base64::Engine;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};

#[tokio::test]
async fn an_idle_legacy_host_receives_no_recovery_envelopes() {
    let directory = tempfile::tempdir().unwrap();
    let state = test_state(directory.path());
    let lease = state.popup_inbox.lock().unwrap().start().unwrap();
    let (sender, mut messages) = tokio::sync::mpsc::unbounded_channel();
    state
        .popup_inbox
        .lock()
        .unwrap()
        .connect(&lease.token, sender)
        .unwrap();
    assert!(matches!(
        messages.try_recv().unwrap(),
        ServerMsg::PopupHostAccepted { .. }
    ));
    let message = ServerMsg::Recovery {
        version: crate::daemon::recovery::VERSION,
        event: serde_json::json!({"event": "updating", "handoffId": "test"}),
    };
    assert_eq!(
        super::super::live_update::announce_popup_update(&state, message.clone()),
        0
    );
    assert!(matches!(
        messages.try_recv().unwrap(),
        ServerMsg::PopupHostShutdown
    ));
    broadcast_popup_state(&state, message);
    assert!(messages.try_recv().is_err());
}

#[tokio::test]
async fn update_handoff_waits_for_the_popup_notice_to_reach_its_connection() {
    let announcement = Arc::new(super::super::live_update::PopupAnnouncement::default());
    let id = announcement.begin();
    let message = ServerMsg::Recovery {
        version: crate::daemon::recovery::VERSION,
        event: serde_json::json!({"event": "updating", "handoffId": id}),
    };
    let (mut writer, reader) = tokio::io::duplex(1);
    let pending = announcement.clone();
    let notice = message.clone();
    let writing = tokio::spawn(async move {
        write_popup_message(&mut writer, &notice, &pending)
            .await
            .unwrap();
    });
    tokio::task::yield_now().await;
    assert!(!announcement.delivered(&id, 1));
    let mut reader = tokio::io::BufReader::new(reader);
    let delivered = ipc::read_msg::<_, ServerMsg>(&mut reader)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(delivered, ServerMsg::Recovery { event, .. }
        if matches!(serde_json::from_value::<crate::daemon::recovery::Event>(event.clone()), Ok(crate::daemon::recovery::Event::Updating))));
    writing.await.unwrap();
    assert!(announcement.delivered(&id, 1));
    let next = announcement.begin();
    announcement.written(&message);
    assert!(
        !announcement.delivered(&next, 1),
        "a delayed earlier notice cannot acknowledge a later handoff"
    );
}

#[tokio::test]
async fn popup_stderr_capture_drains_beyond_the_retained_limit() {
    let input = vec![b'x'; POPUP_HOST_STDERR_LIMIT * 32];
    let mut stream = std::io::Cursor::new(&input);
    let mut captured = Vec::new();
    capture_popup_stderr(&mut stream, &mut captured).await;
    assert_eq!(captured.len(), POPUP_HOST_STDERR_LIMIT);
    assert_eq!(stream.position(), input.len() as u64);
}

#[tokio::test]
async fn popup_stderr_capture_retries_interrupted_reads() {
    struct Interrupted {
        first: bool,
        rest: std::io::Cursor<Vec<u8>>,
    }
    impl tokio::io::AsyncRead for Interrupted {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.first {
                self.first = false;
                return Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::Interrupted)));
            }
            Pin::new(&mut self.rest).poll_read(cx, buffer)
        }
    }
    let stream = Interrupted {
        first: true,
        rest: std::io::Cursor::new(b"Gdk-Message: fatal protocol error\n".to_vec()),
    };
    let mut captured = Vec::new();
    capture_popup_stderr(stream, &mut captured).await;
    assert_eq!(captured, b"Gdk-Message: fatal protocol error\n");
}

#[cfg(unix)]
#[tokio::test]
async fn popup_reaper_logs_failure_and_reaps_before_descendant_stderr_closes() {
    let directory = tempfile::tempdir().unwrap();
    let descendant_file = directory.path().join("descendant.pid");
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 2 & printf '%s' \"$!\" > \"$1\"; printf 'failure marker\\nend\\n' >&2; exit 7")
        .arg("popup-host-fixture")
        .arg(&descendant_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut logs = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(1),
        reap_popup_host(child, pid, Default::default(), |message| logs.push(message)),
    )
    .await
    .expect("direct child cleanup must not wait for descendant-held stderr");
    let descendant: libc::pid_t = std::fs::read_to_string(descendant_file)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(descendant, 0) }, 0);
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    assert_eq!(logs.len(), 1);
    assert!(logs[0].contains(&format!("pid={pid} exited with")));
    assert!(logs[0].contains("failure marker\\n"));
    assert!(logs[0].contains("exit status: 7"));
    assert!(!logs[0].contains('\n'));
    // The fixture descendant exits by itself; leave no long-lived test process behind.
    tokio::time::sleep(Duration::from_secs(2)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn popup_reaper_drains_large_stderr_and_keeps_successful_exit_silent() {
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("head -c 262144 /dev/zero >&2")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut logs = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(3),
        reap_popup_host(child, pid, Default::default(), |message| logs.push(message)),
    )
    .await
    .expect("over-limit stderr must still be drained");
    assert!(
        logs.is_empty(),
        "successful hosts must not emit captured noise"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn popup_reaper_ends_a_host_the_daemon_gave_up_and_reports_it_once() {
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("printf 'could not connect\\n' >&2; exec /bin/sleep 600")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let abandon = Arc::new(tokio::sync::Notify::new());
    let logs = std::sync::Mutex::new(Vec::new());
    let reaper = reap_popup_host(child, pid, abandon.clone(), |message| {
        logs.lock().unwrap().push(message)
    });
    tokio::pin!(reaper);

    // Nobody gave up on it: a host that is merely slow keeps running.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut reaper)
            .await
            .is_err()
    );
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, 0);

    abandon.notify_one();
    tokio::time::timeout(Duration::from_secs(3), &mut reaper)
        .await
        .expect("a host the daemon gave up must be ended, not left running");
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) },
        -1,
        "the process was already reaped"
    );
    let logs = logs.lock().unwrap();
    assert_eq!(
        logs.len(),
        1,
        "being ended is reported once, not as a crash"
    );
    assert!(logs[0].contains(&format!("pid={pid} was given up before it connected")));
    assert!(logs[0].contains("could not connect"));
    assert!(!logs[0].contains("exited with"));
}

#[cfg(windows)]
#[tokio::test]
async fn popup_reaper_logs_windows_failure_with_pid_and_escaped_stderr() {
    let child = tokio::process::Command::new("cmd")
        .args([
            "/C",
            "echo failure marker 1>&2 & echo second line 1>&2 & exit /b 7",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut logs = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        reap_popup_host(child, pid, Default::default(), |message| logs.push(message)),
    )
    .await
    .expect("a failed host must be reaped");
    assert_eq!(logs.len(), 1);
    assert!(logs[0].contains(&format!("popup host pid={pid} exited with")));
    assert!(logs[0].contains("exit code: 7"));
    assert!(logs[0].contains("failure marker"));
    assert!(logs[0].contains("second line"));
    assert!(
        !logs[0].contains('\n'),
        "control characters in stderr must stay escaped"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn popup_reaper_drains_windows_stderr_and_keeps_successful_exit_silent() {
    let child = tokio::process::Command::new("cmd")
        .args(["/C", "for /L %i in (1,1,32768) do @echo xxxxxxxxxx 1>&2"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut logs = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        reap_popup_host(child, pid, Default::default(), |message| logs.push(message)),
    )
    .await
    .expect("over-limit stderr must still be drained");
    assert!(
        logs.is_empty(),
        "successful hosts must not emit captured noise"
    );
}

// Run fixtures in a separate test process so native stderr handles and exit codes are real.
const STDERR_FIXTURE_TEST: &str = "daemon::server_impl::inbox::tests::popup_stderr_process_fixture";

fn stderr_fixture_command(mode: &str, ready: &std::path::Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", STDERR_FIXTURE_TEST, "--ignored", "--nocapture"])
        .env("ASKHUMAN_STDERR_FIXTURE", mode)
        .env("ASKHUMAN_STDERR_READY", ready)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    command
}

#[test]
#[ignore = "subprocess fixture, invoked by stderr regression tests"]
fn popup_stderr_process_fixture() {
    use std::io::{Read, Write};
    let mode = std::env::var("ASKHUMAN_STDERR_FIXTURE").unwrap();
    let ready = std::env::var_os("ASKHUMAN_STDERR_READY").unwrap();
    match mode.as_str() {
        "hold" => {
            std::fs::write(&ready, "ready").unwrap();
            // The descendant is bounded even if its parent test fails.
            std::thread::sleep(Duration::from_secs(3));
            let result = std::io::stderr()
                .write_all(&vec![b'x'; 256 * 1024])
                .and_then(|()| std::io::stderr().flush());
            std::fs::write(
                std::path::Path::new(&ready).with_extension("written"),
                if result.is_ok() { "ok" } else { "error" },
            )
            .unwrap();
        }
        "descendant" => {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", STDERR_FIXTURE_TEST, "--ignored", "--nocapture"])
                .env("ASKHUMAN_STDERR_FIXTURE", "hold")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit());
            let mut child = command.spawn().unwrap();
            // Wait until the descendant owns the inherited pipe before exiting the direct child.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !std::path::Path::new(&ready).exists() {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("descendant did not become ready");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            std::io::stderr().write_all(b"descendant marker\n").unwrap();
            std::process::exit(7);
        }
        "overflow" => {
            let mut stderr = std::io::stderr().lock();
            stderr.write_all(b"overflow marker\n").unwrap();
            stderr.write_all(&vec![b'x'; 256 * 1024]).unwrap();
            stderr.flush().unwrap();
            std::process::exit(7);
        }
        "closed" => {
            #[cfg(unix)]
            assert_eq!(unsafe { libc::close(libc::STDERR_FILENO) }, 0);
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::Foundation::CloseHandle;
                use windows_sys::Win32::System::Console::{
                    GetStdHandle, SetStdHandle, STD_ERROR_HANDLE,
                };
                assert_ne!(CloseHandle(GetStdHandle(STD_ERROR_HANDLE)), 0);
                assert_ne!(SetStdHandle(STD_ERROR_HANDLE, std::ptr::null_mut()), 0);
            }
            std::fs::write(ready, "ready").unwrap();
            let mut byte = [0];
            std::io::stdin().read_exact(&mut byte).unwrap();
        }
        _ => panic!("unknown stderr fixture"),
    }
}

#[tokio::test]
async fn popup_reaper_bounds_failed_stderr_after_overflow() {
    let directory = tempfile::tempdir().unwrap();
    let child = stderr_fixture_command("overflow", &directory.path().join("ready"))
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut logs = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        reap_popup_host(child, pid, Default::default(), |message| logs.push(message)),
    )
    .await
    .expect("failed over-limit stderr must be drained");
    assert_eq!(logs.len(), 1);
    assert!(logs[0].contains("overflow marker\\n"));
    assert!(logs[0].contains(&format!("pid={pid} exited with")));
    assert!(!logs[0].contains('\n'));
    assert!(logs[0].len() < POPUP_HOST_STDERR_LIMIT + 256);
    assert!(logs[0].matches('x').count() > POPUP_HOST_STDERR_LIMIT / 2);
}

#[tokio::test]
async fn popup_reaper_does_not_wait_for_inherited_stderr_on_any_platform() {
    let directory = tempfile::tempdir().unwrap();
    let child = stderr_fixture_command("descendant", &directory.path().join("ready"))
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut logs = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(2),
        reap_popup_host(child, pid, Default::default(), |message| logs.push(message)),
    )
    .await
    .expect("the direct child must finish before its three-second descendant");
    assert_eq!(logs.len(), 1);
    assert!(logs[0].contains("descendant marker"));
    // Late output exceeds the pipe capacity and must still be drained after the reaper returns.
    let written = directory.path().join("ready").with_extension("written");
    tokio::time::timeout(Duration::from_secs(10), async {
        // `fs::write` creates the file before it fills it: wait for the content, not the name.
        while !std::fs::read_to_string(&written).is_ok_and(|text| !text.is_empty()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the descendant must finish writing after the host exits");
    assert_eq!(std::fs::read_to_string(written).unwrap(), "ok");
}

#[tokio::test]
async fn popup_reaper_waits_for_exit_after_stderr_closes() {
    let directory = tempfile::tempdir().unwrap();
    let ready = directory.path().join("ready");
    let mut child = stderr_fixture_command("closed", &ready).spawn().unwrap();
    let pid = child.id().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture must close stderr and signal readiness");
    let mut logs = Vec::new();
    {
        let reaper = reap_popup_host(child, pid, Default::default(), |message| logs.push(message));
        tokio::pin!(reaper);
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut reaper)
                .await
                .is_err()
        );
        stdin.write_all(b"x").await.unwrap();
        drop(stdin);
        tokio::time::timeout(Duration::from_secs(5), &mut reaper)
            .await
            .expect("reaper must finish once the child exits");
    }
    assert!(logs.is_empty());
}

fn state() -> Arc<ServerState> {
    state_inheriting(Inherited::adopting([]))
}

fn state_inheriting(inherited: Arc<Inherited>) -> Arc<ServerState> {
    let mut config = AppConfig::default();
    config.general.popup_prewarm = false;
    Arc::new(ServerState {
        failure_confirmations: failure_confirmation::Cards::default(),
        startup_fp: lifecycle::Fingerprint { size: 0, hash: 0 },
        exe_path: String::new(),
        restart_hold: Default::default(),
        started_at: 0,
        active: AtomicUsize::new(1),
        last_active: Mutex::new(Instant::now()),
        shutdown: Default::default(),
        drain_watch_started: AtomicBool::new(false),
        force_stop: AtomicBool::new(false),
        draining: AtomicBool::new(true),
        exit_intent: Mutex::new(Default::default()),
        closing: AtomicBool::new(false),
        registry: RequestRegistry::new(),
        recovery: None,
        popup_announcement: Default::default(),
        popup_focus: Mutex::new(PopupFocusArbiter::new()),
        popup_inbox: Mutex::new(Default::default()),
        inherited,
        dd_router: Default::default(),
        fs_router: Default::default(),
        tg_router: Default::default(),
        sl_router: Default::default(),
        config: Mutex::new(config),
        update: Mutex::new(Default::default()),
        agents: Arc::new(AgentRegistry::new()),
        agent_controls: crate::agents::control::ControlStore::load(),
        antigravity_trees: Default::default(),
        interject: crate::agents::interject::InterjectStore::new(),
        away: crate::daemon::away::AwayEngine::in_memory(),
        grok_bindings: Default::default(),
        agent_subs: Default::default(),
        tray_subs: Default::default(),
        active_channel: Mutex::new(None),
        inbound_listeners: Default::default(),
        warm_pool: Mutex::new(None),
        warm_spawning: AtomicBool::new(false),
        watch: Default::default(),
        select: Default::default(),
        pending_launches: Default::default(),
        gui_focus: Default::default(),
        last_waiting: Default::default(),
        replay: Mutex::new(crate::daemon::ask_dedup::ReplayCache::new()),
        ios: Default::default(),
    })
}

fn request(
    state: &ServerState,
) -> (
    Arc<RequestEntry>,
    tokio::sync::mpsc::UnboundedReceiver<crate::app::RenderOutcome>,
) {
    let task = serde_json::from_value(serde_json::json!({
        "message": { "text": "isolated IPC regression" },
        "questions": [{ "message": "answer", "predefinedOptions": ["option"] }],
        "isMarkdown": false, "source": "test", "lang": "en",
        "recordHistory": false, "outputFormat": "json"
    }))
    .unwrap();
    let (entry, rx) = state.registry.create(task, None);
    state.popup_inbox.lock().unwrap().enqueue(
        entry.request_id.clone(),
        entry.seq,
        InteractionEntry::Ask(entry.clone()),
    );
    (entry, rx)
}

#[tokio::test]
async fn each_popup_refreshes_renamed_sessions_even_with_a_cached_title() {
    let state = state();
    let home = tempfile::tempdir().unwrap();
    state.config.lock().unwrap().codex_desktop.codex_home =
        home.path().to_string_lossy().into_owned();
    let db = rusqlite::Connection::open(home.path().join("state_5.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE threads(id TEXT PRIMARY KEY,name TEXT,title TEXT); INSERT INTO threads VALUES ('session','Native name','Original prompt');").unwrap();
    state
        .agents
        .update_desktop("session", "Stale cached title", "/tmp", false);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut last_entry = None;
    for expected in ["Native name", "Renamed task", "Stale cached title"] {
        let task = serde_json::from_value(serde_json::json!({
            "message": {"text": "popup title regression"}, "questions": [],
            "isMarkdown": false, "source": "test", "lang": "en",
            "recordHistory": false, "outputFormat": "json",
            "agentKind": "codex", "agentSessionId": "session"
        }))
        .unwrap();
        let (entry, _result) = state.registry.create(task, None);
        let interaction = InteractionEntry::Ask(entry.clone());
        show_entry(&interaction, &tx, &state);
        let Some(ServerMsg::Show(show)) = rx.recv().await else {
            panic!("expected the question before asynchronous title resolution");
        };
        assert_eq!(show.request_id, entry.request_id);
        assert_eq!(
            show.agent_session_title.as_deref(),
            Some("Stale cached title")
        );
        let message = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap();
        let Some(ServerMsg::PopupSessionTitle { request_id, title }) = message else {
            panic!("expected the refreshed native title");
        };
        assert_eq!(request_id, entry.request_id);
        assert_eq!(title, expected);
        if expected == "Native name" {
            db.execute("UPDATE threads SET name='Renamed task'", [])
                .unwrap();
        } else if expected == "Renamed task" {
            db.execute_batch("DROP TABLE threads;").unwrap();
        }
        // Editing the native name does not publish another title into an already-open popup.
        assert!(tokio::time::timeout(Duration::from_millis(25), rx.recv())
            .await
            .is_err());
        last_entry = Some(interaction);
    }
    let interaction = last_entry.unwrap();
    if let InteractionEntry::Ask(entry) = &interaction {
        entry.coordinator.submit(ChannelResult::cancel("popup"));
    }
    show_entry(&interaction, &tx, &state);
    assert!(rx.try_recv().is_err());
}

struct Observed<R> {
    reader: R,
    consumed: Option<tokio::sync::oneshot::Sender<()>>,
}

#[tokio::test(start_paused = true)]
async fn a_host_that_stays_silent_is_given_up_after_the_startup_timeout() {
    use futures_util::FutureExt;
    let state = state();
    let (generation, abandon) = {
        let mut host = state.popup_inbox.lock().unwrap();
        let lease = host.start().unwrap();
        (
            lease.generation,
            host.abandon_signal(lease.generation).unwrap(),
        )
    };
    let timer = tokio::spawn(expire_unauthenticated(state.clone(), generation));

    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        state.popup_inbox.lock().unwrap().generation(),
        Some(generation)
    );
    assert!(
        abandon.notified().now_or_never().is_none(),
        "still within the timeout"
    );

    tokio::time::sleep(Duration::from_secs(2)).await;
    timer.await.unwrap();
    // Nobody waits for this host, so recovery has nothing to restart it for; the point is that
    // the lease is gone and the process that held it was told to end.
    assert!(state.popup_inbox.lock().unwrap().generation().is_none());
    assert!(
        abandon.notified().now_or_never().is_some(),
        "the host's process is ended"
    );
}

#[tokio::test(start_paused = true)]
async fn a_host_that_authenticated_in_time_is_left_alone_by_the_startup_timer() {
    use futures_util::FutureExt;
    let state = state();
    let (generation, abandon, token) = {
        let mut host = state.popup_inbox.lock().unwrap();
        let lease = host.start().unwrap();
        (
            lease.generation,
            host.abandon_signal(lease.generation).unwrap(),
            lease.token,
        )
    };
    let timer = tokio::spawn(expire_unauthenticated(state.clone(), generation));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    state
        .popup_inbox
        .lock()
        .unwrap()
        .connect(&token, tx)
        .unwrap();

    tokio::time::sleep(Duration::from_secs(6)).await;
    timer.await.unwrap();
    let host = state.popup_inbox.lock().unwrap();
    assert_eq!(host.generation(), Some(generation));
    assert!(host.sender().is_some(), "still connected");
    assert!(abandon.notified().now_or_never().is_none(), "never ended");
}

#[tokio::test(start_paused = true)]
async fn the_startup_window_starts_over_while_the_daemon_is_restoring() {
    let restoring = std::sync::atomic::AtomicBool::new(true);
    let closing = std::sync::atomic::AtomicBool::new(false);
    let window = startup_window(
        || closing.load(Ordering::SeqCst),
        || restoring.load(Ordering::SeqCst),
    );
    tokio::pin!(window);

    // Restoring: a host cannot authenticate yet, so its silence says nothing.
    assert!(tokio::time::timeout(Duration::from_secs(12), &mut window)
        .await
        .is_err());
    restoring.store(false, Ordering::SeqCst);
    assert!(tokio::time::timeout(Duration::from_secs(6), &mut window)
        .await
        .expect("the window ends once the restore is over"));
}

#[tokio::test(start_paused = true)]
async fn the_startup_window_is_dropped_when_the_daemon_closes() {
    let closing = std::sync::atomic::AtomicBool::new(false);
    let window = startup_window(|| closing.load(Ordering::SeqCst), || false);
    tokio::pin!(window);
    closing.store(true, Ordering::SeqCst);
    assert!(!tokio::time::timeout(Duration::from_secs(6), &mut window)
        .await
        .expect("a closing daemon ends the wait"));
}

#[tokio::test]
async fn a_popup_is_refused_while_hosts_keep_failing_to_start() {
    let state = state();
    {
        let mut host = state.popup_inbox.lock().unwrap();
        let mut lease = host.start().unwrap();
        for _ in 0..2 {
            lease = match host.disconnected(lease.generation, Instant::now(), true) {
                Recovery::Restart(lease) => lease,
                _ => panic!("expected a restart within the budget"),
            };
        }
        // The third host in a row never came up either: recovery gives up.
        assert!(matches!(
            host.disconnected(lease.generation, Instant::now(), true),
            Recovery::Failed(_)
        ));
    }
    let task = serde_json::from_value(serde_json::json!({
        "message": { "text": "refused popup" }, "questions": [],
        "isMarkdown": false, "source": "test", "lang": "en",
        "recordHistory": false, "outputFormat": "json"
    }))
    .unwrap();
    let (entry, _result) = state.registry.create(task, None);

    assert!(popup_launch_blocked(&state));
    // The popup is reported as undeliverable. Queueing it here would leave the request waiting
    // for a host that is not going to be started, and starting one anyway is how a broken host
    // used to be respawned every few seconds without end.
    assert!(!dispatch_inbox_popup(InteractionEntry::Ask(entry), &state));
    let host = state.popup_inbox.lock().unwrap();
    assert!(host.requests().is_empty());
    assert!(host.generation().is_none(), "no host was started");
}

#[tokio::test]
async fn a_committed_stop_keeps_host_disconnect_out_of_recovery() {
    let state = state();
    let (_entry, mut result) = request(&state);
    let generation = {
        let mut host = state.popup_inbox.lock().unwrap();
        let mut lease = host.start().unwrap();
        // Exhaust the restart budget so a regression cannot launch a GUI child.
        for _ in 0..2 {
            lease = match host.disconnected(lease.generation, Instant::now(), true) {
                Recovery::Restart(lease) => lease,
                _ => panic!("expected recovery lease"),
            };
        }
        lease.generation
    };
    state.closing.store(true, Ordering::SeqCst);
    recover_inbox(&state, generation);
    let host = state.popup_inbox.lock().unwrap();
    assert_eq!(host.generation(), Some(generation));
    assert_eq!(host.requests().len(), 1);
    assert!(result.try_recv().is_err());
}
impl<R: AsyncRead + Unpin> AsyncRead for Observed<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.reader).poll_read(cx, buf);
        if buf.filled().len() > before {
            if let Some(consumed) = self.consumed.take() {
                let _ = consumed.send(());
            }
        }
        result
    }
}

async fn next<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut ipc::MessageReader<R>) -> ServerMsg {
    tokio::time::timeout(Duration::from_secs(2), reader.read())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
async fn ack<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut ipc::MessageReader<R>,
    id: &str,
) -> String {
    loop {
        if let ServerMsg::PopupSubmissionAck {
            request_id,
            winner,
            error,
        } = next(reader).await
        {
            assert_eq!(request_id, id);
            assert!(error.is_none());
            return winner.unwrap();
        }
    }
}

async fn fragmented_answer(external_terminal: bool) {
    let state = state();
    let (first, mut first_result) = request(&state);
    let (second, mut second_result) = request(&state);
    let (other, _other_result) = request(&state);
    let lease = {
        let mut host = state.popup_inbox.lock().unwrap();
        let mut lease = host.start().unwrap();
        // A failing regression must not launch a child process as part of recovery.
        for _ in 0..2 {
            lease = match host.disconnected(lease.generation, Instant::now(), true) {
                Recovery::Restart(lease) => lease,
                _ => panic!("expected reserved recovery lease"),
            };
        }
        lease
    };
    let generation = lease.generation;
    let (server, client) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(server);
    let (consumed_tx, consumed_rx) = tokio::sync::oneshot::channel();
    let reader = BufReader::new(Observed {
        reader: read,
        consumed: Some(consumed_tx),
    });
    let run_state = state.clone();
    let handler =
        tokio::spawn(
            async move { handle_popup_host(lease.token, reader, write, &run_state).await },
        );
    let (read, mut write) = tokio::io::split(client);
    let mut reader = ipc::MessageReader::new(BufReader::new(read));
    assert!(
        matches!(next(&mut reader).await, ServerMsg::PopupHostAccepted { generation: g, .. } if g == generation)
    );
    while !matches!(next(&mut reader).await, ServerMsg::UpdateState { .. }) {}

    let mut image = include_bytes!("../../../../icons/32x32.png").to_vec();
    image.resize(328_082, 0);
    let answer = ClientMsg::Answer {
        request_id: first.request_id.clone(),
        action: ChannelAction::Send,
        answers: vec![crate::models::QuestionAnswer {
            selected_options: vec!["option".into()],
            user_input: Some("typed 中文 text".into()),
            images: vec![crate::models::ImageAttachment {
                data: base64::engine::general_purpose::STANDARD.encode(&image),
                media_type: "image/png".into(),
                filename: Some("screenshot.png".into()),
            }],
            ..Default::default()
        }],
        directive: None,
        annotations: Vec::new(),
    };
    let mut bytes = serde_json::to_vec(&answer).unwrap();
    bytes.push(b'\n');
    write.write_all(&bytes[..1024]).await.unwrap();
    consumed_rx.await.unwrap();
    if external_terminal {
        assert!(other
            .coordinator
            .try_submit(ChannelResult::cancel("feishu")));
    }
    tokio::time::advance(Duration::from_millis(350)).await;
    tokio::task::yield_now().await;
    if external_terminal {
        loop {
            let ServerMsg::Cancel { request_id, winner } = next(&mut reader).await else {
                panic!("expected the external terminal to cancel its popup");
            };
            assert_eq!(request_id, other.request_id);
            if winner == "feishu" {
                break;
            }
        }
        assert!(state
            .popup_inbox
            .lock()
            .unwrap()
            .get(&other.request_id)
            .is_none());
    }
    write.write_all(&bytes[1024..]).await.unwrap();
    assert_eq!(ack(&mut reader, &first.request_id).await, "popup");
    let result = first_result.recv().await.unwrap();
    assert_eq!(result.exit_code, 0);
    let json: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(json["answers"][0]["user_input"], "typed 中文 text");
    assert_eq!(json["answers"][0]["selected_options"][0], "option");
    let path = json["answers"][0]["files"][0].as_str().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), image);
    assert_eq!(
        state.popup_inbox.lock().unwrap().generation(),
        Some(generation)
    );

    // A duplicate cannot win again, and the next request still uses the same connection.
    ipc::write_msg(&mut write, &answer).await.unwrap();
    assert_eq!(ack(&mut reader, &first.request_id).await, "system");
    assert!(first_result.try_recv().is_err());
    ipc::write_msg(
        &mut write,
        &ClientMsg::Answer {
            request_id: second.request_id.clone(),
            action: ChannelAction::Send,
            answers: vec![crate::models::QuestionAnswer {
                user_input: Some("second".into()),
                ..Default::default()
            }],
            directive: None,
            annotations: Vec::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(ack(&mut reader, &second.request_id).await, "popup");
    assert!(second_result
        .recv()
        .await
        .unwrap()
        .stdout
        .contains("second"));
    if !external_terminal {
        other.coordinator.submit(ChannelResult::cancel("timeout"));
    }
    ipc::write_msg(&mut write, &ClientMsg::PopupHostIdle { generation })
        .await
        .unwrap();
    loop {
        if let ServerMsg::PopupHostIdleAck {
            generation: g,
            keep_warm,
        } = next(&mut reader).await
        {
            assert_eq!(g, generation);
            assert!(!keep_warm);
            break;
        }
    }
    handler.await.unwrap();
    assert!(state.popup_inbox.lock().unwrap().generation().is_none());
    let _ = std::fs::remove_dir_all(crate::paths::request_temp_dir(&first.request_id));
}

#[tokio::test(start_paused = true)]
async fn fragmented_image_answer_survives_unchanged_pruning_and_next_submission() {
    fragmented_answer(false).await;
}

#[tokio::test(start_paused = true)]
async fn external_terminal_pruning_preserves_another_partial_answer() {
    fragmented_answer(true).await;
}

#[tokio::test]
async fn eof_and_invalid_json_keep_the_idle_disconnect_contract() {
    for bytes in [&b""[..], &b"{invalid json}\n"[..]] {
        let state = state();
        let lease = state.popup_inbox.lock().unwrap().start().unwrap();
        let (mut writer, reader) = tokio::io::duplex(128);
        writer.write_all(bytes).await.unwrap();
        writer.shutdown().await.unwrap();
        handle_popup_host(
            lease.token,
            BufReader::new(reader),
            tokio::io::sink(),
            &state,
        )
        .await;
        assert!(state.popup_inbox.lock().unwrap().generation().is_none());
        assert_eq!(state.active.load(Ordering::SeqCst), 1);
    }
}

struct BrokenWriter;
impl tokio::io::AsyncWrite for BrokenWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "isolated failure",
        )))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn writer_failure_ends_a_pending_read_without_hanging() {
    let state = state();
    let lease = state.popup_inbox.lock().unwrap().start().unwrap();
    let (_peer, reader) = tokio::io::duplex(128);
    tokio::time::timeout(
        Duration::from_secs(2),
        handle_popup_host(lease.token, BufReader::new(reader), BrokenWriter, &state),
    )
    .await
    .unwrap();
    assert!(state.popup_inbox.lock().unwrap().generation().is_none());
}

#[tokio::test]
async fn prewarmed_idle_host_retires_after_drain_without_reconnecting() {
    let state = state();
    state.draining.store(false, Ordering::SeqCst);
    state.config.lock().unwrap().general.popup_prewarm = true;
    let lease = state.popup_inbox.lock().unwrap().start().unwrap();
    let generation = lease.generation;
    let (server, client) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(server);
    let run_state = state.clone();
    let handler = tokio::spawn(async move {
        handle_popup_host(lease.token, BufReader::new(read), write, &run_state).await;
    });
    let (read, mut write) = tokio::io::split(client);
    let mut reader = ipc::MessageReader::new(BufReader::new(read));
    assert!(matches!(
        next(&mut reader).await,
        ServerMsg::PopupHostAccepted { .. }
    ));
    assert!(matches!(
        next(&mut reader).await,
        ServerMsg::UpdateState { .. }
    ));
    for keep_warm in [true, false] {
        state.draining.store(!keep_warm, Ordering::SeqCst);
        ipc::write_msg(&mut write, &ClientMsg::PopupHostIdle { generation })
            .await
            .unwrap();
        assert!(
            matches!(next(&mut reader).await, ServerMsg::PopupHostIdleAck { generation: g, keep_warm: w } if g == generation && w == keep_warm)
        );
    }
    handler.await.unwrap();
    assert!(state.popup_inbox.lock().unwrap().generation().is_none());
}

#[test]
fn an_old_popup_connection_cannot_detach_the_reconnected_window() {
    let state = state();
    let (entry, _) = request(&state);
    let interaction = InteractionEntry::Ask(entry.clone());
    let (old, _) = tokio::sync::mpsc::unbounded_channel();
    let (current, _) = tokio::sync::mpsc::unbounded_channel();
    connect_entry(&interaction, &old);
    connect_entry(&interaction, &current);
    assert!(!detach_entry(&interaction, &old));
    assert!(entry.gui_connected.load(Ordering::SeqCst));
    assert!(entry
        .gui
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .same_channel(&current));
    assert!(detach_entry(&interaction, &current));
    assert!(!entry.gui_connected.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_restored_ready_form_does_not_take_focus_again() {
    let state = state();
    let (entry, _result) = request(&state);
    let lease = state.popup_inbox.lock().unwrap().start().unwrap();
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (read, mut write) = tokio::io::split(client);
    let (server_read, server_write) = tokio::io::split(server);
    let task_state = state.clone();
    let known = vec![entry.request_id.clone()];
    let task = tokio::spawn(async move {
        handle_popup_host_supported(
            lease.token,
            Some(std::process::id()),
            known,
            tokio::io::BufReader::new(server_read),
            server_write,
            &task_state,
        )
        .await;
    });
    let mut reader = ipc::MessageReader::new(tokio::io::BufReader::new(read));
    loop {
        if matches!(next(&mut reader).await, ServerMsg::UpdateState { .. }) {
            break;
        }
    }
    ipc::write_msg(
        &mut write,
        &ClientMsg::PopupReady {
            request_id: entry.request_id.clone(),
            window_number: None,
        },
    )
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), reader.read::<ServerMsg>())
            .await
            .is_err()
    );
    assert!(entry.gui_ready.load(Ordering::SeqCst));
    task.abort();
}

#[tokio::test]
async fn independent_popup_reconnect_clears_the_previous_update_banner() {
    let state = state();
    let (entry, _result) = request(&state);
    let token = entry.token.clone();
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (read, _write) = tokio::io::split(client);
    let (server_read, server_write) = tokio::io::split(server);
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        handle_gui(
            token,
            Box::new(tokio::io::BufReader::new(server_read)),
            Box::new(server_write),
            &task_state,
        )
        .await;
    });
    let mut reader = ipc::MessageReader::new(tokio::io::BufReader::new(read));
    assert!(matches!(next(&mut reader).await, ServerMsg::Show(_)));
    assert!(matches!(
        next(&mut reader).await,
        ServerMsg::UpdateState {
            pending: false,
            available: false,
            ..
        }
    ));
    task.abort();
}

/// Hosts kept across a live update that are this daemon's own children: real processes, nobody
/// holding a waiter for them, as after an in-place update.
#[cfg(unix)]
mod adopted_hosts {
    use super::*;
    use crate::daemon::server_impl::inherited::testing::{
        exists, orphaned, wait_until_zombie, Stranger,
    };
    use crate::daemon::server_impl::inherited::{Gone, Inherited};
    use std::os::unix::process::ExitStatusExt;

    /// Publishes the saved host the way the end of a restore does, and returns its generation.
    fn adopt(state: &Arc<ServerState>, pid: u32, birth: Option<&str>) -> u64 {
        let saved = SavedHost {
            lease: Lease {
                generation: 7,
                token: "saved".into(),
            },
            pid,
            birth: birth.map(str::to_owned),
        };
        *state.popup_inbox.lock().unwrap() = adopt_host(state, saved);
        7
    }

    /// The start time the daemon records for a running process, as a host's record carries it.
    fn born(pid: u32) -> String {
        crate::daemon::recovery::process_birth(pid).expect("a running child has a start time")
    }

    fn lease_given_up(state: &ServerState) -> bool {
        state.popup_inbox.lock().unwrap().generation().is_none()
    }

    async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn reaped(inherited: &Inherited, pid: u32) -> Gone {
        let mut watch = inherited.watch(pid).expect("an inherited child");
        tokio::time::timeout(Duration::from_secs(10), watch.exited())
            .await
            .expect("the child is reaped")
    }

    #[tokio::test]
    async fn a_kept_host_that_exits_before_it_reconnects_is_recovered_at_once() {
        let (pid, held) = orphaned("read line; exit 0");
        let inherited = Inherited::adopting([pid]);
        inherited.start();
        let state = state_inheriting(inherited.clone());
        adopt(&state, pid, Some(&born(pid)));
        recover_adopted_host_when_gone(&state);

        // No startup timer runs here: only the host's own exit can end the wait.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!lease_given_up(&state), "still waiting while the host runs");

        drop(held);
        eventually("the lease of the exited host is recovered", || {
            lease_given_up(&state)
        })
        .await;
        reaped(&inherited, pid).await;
    }

    #[tokio::test]
    async fn a_kept_host_that_had_exited_before_the_watch_began_is_recovered_at_once() {
        let (pid, held) = orphaned("read line; exit 0");
        // Recorded while it ran, as the previous image did.
        let recorded = born(pid);
        drop(held);
        wait_until_zombie(pid);
        let inherited = Inherited::adopting([pid]);
        let state = state_inheriting(inherited);
        adopt(&state, pid, Some(&recorded));
        recover_adopted_host_when_gone(&state);
        eventually("the lease of the exited host is recovered", || {
            lease_given_up(&state)
        })
        .await;
    }

    #[tokio::test]
    async fn a_kept_host_that_reconnected_is_not_recovered_when_it_exits() {
        let (pid, held) = orphaned("read line; exit 0");
        let inherited = Inherited::adopting([pid]);
        inherited.start();
        let state = state_inheriting(inherited.clone());
        let generation = adopt(&state, pid, Some(&born(pid)));
        recover_adopted_host_when_gone(&state);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        state
            .popup_inbox
            .lock()
            .unwrap()
            .connect("saved", tx)
            .unwrap();

        drop(held);
        reaped(&inherited, pid).await;
        // The watcher has had its notice; a connected host's end is its connection's to report.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let host = state.popup_inbox.lock().unwrap();
        assert_eq!(host.generation(), Some(generation));
        assert!(host.sender().is_some());
    }

    #[tokio::test]
    async fn giving_up_a_kept_host_ends_its_process() {
        let (pid, _held) = orphaned("exec sleep 600");
        let inherited = Inherited::adopting([pid]);
        inherited.start();
        let state = state_inheriting(inherited.clone());
        adopt(&state, pid, Some(&born(pid)));

        // Any way of giving the lease up does it; the startup timer is one of them.
        assert!(state.popup_inbox.lock().unwrap().stop_idle().is_none());

        let Gone::Reaped(status) = reaped(&inherited, pid).await else {
            panic!("a child of ours is reaped");
        };
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        assert!(!exists(pid));
    }

    #[tokio::test]
    async fn a_kept_host_with_no_start_time_on_record_is_not_ended() {
        let (pid, _held) = orphaned("exec sleep 600");
        let inherited = Inherited::adopting([pid]);
        inherited.start();
        let state = state_inheriting(inherited.clone());
        adopt(&state, pid, None);

        assert!(state.popup_inbox.lock().unwrap().stop_idle().is_none());

        // A pid alone does not say that this is the host that was saved.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(exists(pid), "left running");
        inherited.end(pid);
        reaped(&inherited, pid).await;
    }

    #[tokio::test]
    async fn a_kept_host_whose_start_time_is_not_the_one_on_record_is_not_ended() {
        // A pid that another child was given after the host: `process_alive` would not accept
        // this, but the adoption must not depend on it for the one thing it cannot undo.
        let (pid, _held) = orphaned("exec sleep 600");
        let inherited = Inherited::adopting([pid]);
        inherited.start();
        let state = state_inheriting(inherited.clone());
        adopt(&state, pid, Some("an earlier process"));

        assert!(state.popup_inbox.lock().unwrap().stop_idle().is_none());

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(exists(pid), "left running");
        inherited.end(pid);
        reaped(&inherited, pid).await;
    }

    #[test]
    fn only_a_start_time_that_matches_the_one_on_record_identifies_a_host() {
        let saved = |birth: Option<&str>| SavedHost {
            lease: Lease {
                generation: 1,
                token: "t".into(),
            },
            pid: 1,
            birth: birth.map(str::to_owned),
        };
        assert!(is_the_saved_process(&saved(Some("a")), Some("a")));
        assert!(!is_the_saved_process(&saved(Some("a")), Some("b")));
        // `process_alive` lets these through; nothing here says it is the same process.
        assert!(!is_the_saved_process(&saved(Some("a")), None));
        assert!(!is_the_saved_process(&saved(None), Some("a")));
        assert!(!is_the_saved_process(&saved(None), None));
    }

    #[tokio::test]
    async fn a_kept_host_that_is_not_our_child_is_neither_watched_nor_ended() {
        let stranger = Stranger::start();
        let inherited = Inherited::adopting([stranger.0]);
        let state = state_inheriting(inherited);
        adopt(&state, stranger.0, Some(&born(stranger.0)));
        recover_adopted_host_when_gone(&state);

        // Its exit would not be noticed, so it is not waited for...
        unsafe { libc::kill(stranger.0 as libc::pid_t, libc::SIGSTOP) };
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!lease_given_up(&state));
        // ...and giving the lease up does not touch it.
        assert!(state.popup_inbox.lock().unwrap().stop_idle().is_none());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(exists(stranger.0));
        unsafe { libc::kill(stranger.0 as libc::pid_t, libc::SIGCONT) };
    }

    #[tokio::test]
    async fn the_exit_watch_does_not_keep_the_daemon_state_alive() {
        let (pid, held) = orphaned("read line; exit 0");
        let inherited = Inherited::adopting([pid]);
        inherited.start();
        let state = state_inheriting(inherited.clone());
        adopt(&state, pid, Some(&born(pid)));
        recover_adopted_host_when_gone(&state);

        // A daemon that resumes after a failed update builds another state; the old one is not
        // kept for as long as the host lives.
        let weak = Arc::downgrade(&state);
        drop(state);
        assert!(weak.upgrade().is_none());

        drop(held);
        reaped(&inherited, pid).await;
    }
}
