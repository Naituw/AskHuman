//! The todo messages (docs/specs/ios-todos.md §5–§6) against a scripted host.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::testkit::*;
use super::*;
use crate::ios::wire::{self, t, TODO_MAX_PENDING, TODO_TEXT_MAX_CHARS};

const SECOND: Duration = Duration::from_secs(1);

/// Lets the hello burst and the one the Mac repeats a second after it connected pass.
async fn settle_hello(phone: &mut TestPhone) {
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    phone.clear_queue();
}

async fn live() -> (Fixture, TestPhone) {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    settle_hello(&mut phone).await;
    (fx, phone)
}

fn item(id: &str, text: &str) -> wire::TodoItem {
    wire::TodoItem {
        id: id.into(),
        text: text.into(),
        created_at_ms: BASE_MS as u64,
        agent: None,
        auto: false,
        att: 0,
        by: None,
        att_on: None,
    }
}

fn done(id: &str, text: &str) -> wire::TodoDone {
    wire::TodoDone {
        id: id.into(),
        text: text.into(),
        created_at_ms: BASE_MS as u64 - 1_000,
        done_at_ms: BASE_MS as u64,
        agent: None,
        att: 0,
        done_on: None,
    }
}

fn list_of(id: &str, pending: &[(&str, &str)]) -> wire::TodoList {
    wire::TodoList {
        id: id.into(),
        found: true,
        rev: 1,
        name: "app".into(),
        can_auto: true,
        pending: pending.iter().map(|(i, text)| item(i, text)).collect(),
        pending_total: pending.len() as u64,
        history: vec![done("d1", "shipped")],
        history_total: 1,
    }
}

fn op(id: &str, project: &str, kind: &str) -> wire::TodoOp {
    wire::TodoOp {
        client_msg_id: id.into(),
        id: project.into(),
        op: kind.into(),
        todo_id: None,
        text: None,
        auto: None,
        order: None,
    }
}

fn add(id: &str, project: &str, text: &str) -> wire::TodoOp {
    wire::TodoOp {
        text: Some(text.into()),
        ..op(id, project, "add")
    }
}

async fn ack(phone: &mut TestPhone) -> wire::TodoAck {
    phone.recv_body(t::TODO_ACK).await
}

fn row(id: &str, name: &str, pending: u64) -> wire::TodoProjectRow {
    wire::TodoProjectRow {
        id: id.into(),
        name: name.into(),
        hint: None,
        repo: None,
        pending,
        history: 0,
        scope: "local".into(),
        section: "withTodos".into(),
        agent: None,
        macs: Vec::new(),
    }
}

// ---------------------------------------------------------------- reading

#[tokio::test(start_paused = true)]
async fn the_mac_announces_the_todo_capability() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    let hello = phone.hello().await;
    assert!(hello.caps.todos);
}

#[tokio::test(start_paused = true)]
async fn the_project_list_comes_from_the_host() {
    let (fx, mut phone) = live().await;
    *fx.host.todo_projects_reply.lock().unwrap() = wire::TodoProjects {
        rev: 7,
        mode: "icloud".into(),
        sync: None,
        projects: vec![row("p1", "app", 2), row("p2", "blog", 0)],
        truncated: false,
    };
    phone.send(&wire::TodoProjectsGet {}).await;
    let projects: wire::TodoProjects = phone.recv_body(t::TODO_PROJECTS).await;
    assert_eq!(projects.rev, 7);
    assert_eq!(projects.mode, "icloud");
    assert_eq!(
        projects
            .projects
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["app", "blog"]
    );
    assert_eq!(
        fx.host.todo_refreshes.load(Ordering::SeqCst),
        1,
        "looking at the list asks the host to catch up with other devices"
    );
}

#[tokio::test(start_paused = true)]
async fn a_list_is_read_by_project_id_and_an_unknown_id_is_not_found() {
    let (fx, mut phone) = live().await;
    fx.host.todo_lists.lock().unwrap().insert(
        "p1".into(),
        list_of("p1", &[("t1", "first"), ("t2", "second")]),
    );
    phone.send(&wire::TodoListGet { id: "p1".into() }).await;
    let list: wire::TodoList = phone.recv_body(t::TODO_LIST).await;
    assert!(list.found);
    assert_eq!(list.pending.len(), 2);
    assert_eq!(list.pending[1].text, "second");
    assert_eq!(list.history_total, 1);

    phone.send(&wire::TodoListGet { id: "nope".into() }).await;
    let missing: wire::TodoList = phone.recv_body(t::TODO_LIST).await;
    assert!(!missing.found);
    assert_eq!(missing.id, "nope");
    assert!(missing.pending.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_list_too_big_for_the_broker_is_cut_down_but_keeps_its_totals() {
    let (fx, mut phone) = live().await;
    let text = "x".repeat(3_500);
    let mut list = list_of("p1", &[]);
    list.pending = (0..TODO_MAX_PENDING)
        .map(|i| item(&format!("t{i}"), &text))
        .collect();
    list.pending_total = TODO_MAX_PENDING as u64 + 40;
    list.history = (0..wire::TODO_MAX_HISTORY)
        .map(|i| done(&format!("d{i}"), &text))
        .collect();
    list.history_total = 500;
    fx.host.todo_lists.lock().unwrap().insert("p1".into(), list);
    phone.send(&wire::TodoListGet { id: "p1".into() }).await;
    let got: wire::TodoList = phone.recv_body(t::TODO_LIST).await;
    assert!(got.found);
    assert!(
        got.pending.len() + got.history.len() < TODO_MAX_PENDING + wire::TODO_MAX_HISTORY,
        "something had to go"
    );
    assert!(!got.pending.is_empty(), "the head of the queue stays");
    assert!(
        got.history.len() < wire::TODO_MAX_HISTORY,
        "the history goes first"
    );
    assert_eq!(got.pending_total, TODO_MAX_PENDING as u64 + 40);
    assert_eq!(got.history_total, 500);
    assert_eq!(got.pending[0].id, "t0");
}

// ---------------------------------------------------------------- writing

#[tokio::test(start_paused = true)]
async fn a_write_reaches_the_host_trimmed_and_is_answered_with_the_fresh_list() {
    let (fx, mut phone) = live().await;
    fx.host
        .todo_lists
        .lock()
        .unwrap()
        .insert("p1".into(), list_of("p1", &[("t1", "first")]));
    *fx.host.todo_apply_result.lock().unwrap() = Some(TodoApplied::ok(Some("t9".into()), None));
    phone.send(&add("o1", "p1", "  buy milk \n")).await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.client_msg_id, "o1");
    assert_eq!(reply.status, "ok");
    assert_eq!(reply.todo_id.as_deref(), Some("t9"));
    assert_eq!(reply.list.as_ref().map(|l| l.pending.len()), Some(1));
    let ops = fx.host.todo_ops.lock().unwrap().clone();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].text.as_deref(), Some("buy milk"));
    assert_eq!(ops[0].id, "p1");
}

#[tokio::test(start_paused = true)]
async fn what_the_host_refuses_is_passed_on() {
    let (fx, mut phone) = live().await;
    *fx.host.todo_apply_result.lock().unwrap() =
        Some(TodoApplied::refused("notFound", "unknownProject", None));
    phone.send(&add("o1", "gone", "hello")).await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.status, "notFound");
    assert_eq!(reply.reason.as_deref(), Some("unknownProject"));
    assert!(reply.list.is_none());

    *fx.host.todo_apply_result.lock().unwrap() =
        Some(TodoApplied::refused("failed", "persist", None));
    phone.send(&add("o2", "gone", "hello")).await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.status, "failed");
    assert_eq!(reply.reason.as_deref(), Some("persist"));
}

#[tokio::test(start_paused = true)]
async fn malformed_writes_are_refused_before_the_host_sees_them() {
    let (fx, mut phone) = live().await;
    fx.host
        .todo_lists
        .lock()
        .unwrap()
        .insert("p1".into(), list_of("p1", &[("t1", "first")]));
    let with_id = |kind: &str| wire::TodoOp {
        todo_id: Some("t1".into()),
        ..op("x", "p1", kind)
    };
    let cases: Vec<(&str, wire::TodoOp, &str)> = vec![
        ("blank add", add("c1", "p1", "  \n "), "empty"),
        (
            "too long",
            add("c2", "p1", &"y".repeat(TODO_TEXT_MAX_CHARS + 1)),
            "tooLong",
        ),
        ("nul", add("c3", "p1", "a\0b"), "invalid"),
        (
            "edit without id",
            wire::TodoOp {
                text: Some("x".into()),
                ..op("c4", "p1", "edit")
            },
            "invalid",
        ),
        (
            "edit blank",
            wire::TodoOp {
                text: Some(" ".into()),
                ..with_id("edit")
            },
            "empty",
        ),
        ("complete without id", op("c6", "p1", "complete"), "invalid"),
        (
            "auto without flag",
            wire::TodoOp {
                auto: None,
                ..with_id("auto")
            },
            "invalid",
        ),
        ("unknown op", op("c8", "p1", "explode"), "unsupported"),
        (
            "reorder too long",
            wire::TodoOp {
                order: Some((0..=TODO_MAX_PENDING).map(|i| format!("t{i}")).collect()),
                ..op("c9", "p1", "reorder")
            },
            "invalid",
        ),
        (
            "reorder without order",
            op("c10", "p1", "reorder"),
            "invalid",
        ),
    ];
    for (name, mut op, reason) in cases {
        let id = format!("case-{name}");
        op.client_msg_id = id.clone();
        phone.send(&op).await;
        let reply = ack(&mut phone).await;
        assert_eq!(reply.client_msg_id, id, "{name}");
        assert_eq!(reply.status, "rejected", "{name}");
        assert_eq!(reply.reason.as_deref(), Some(reason), "{name}");
        assert_eq!(
            reply.list.as_ref().map(|l| l.pending.len()),
            Some(1),
            "{name}: a refusal still shows the phone where things stand"
        );
    }
    assert!(
        fx.host.todo_ops.lock().unwrap().is_empty(),
        "nothing reached the host"
    );
}

#[tokio::test(start_paused = true)]
async fn every_operation_reaches_the_host_with_its_arguments() {
    let (fx, mut phone) = live().await;
    let sent = vec![
        wire::TodoOp {
            todo_id: Some("t1".into()),
            text: Some(" new text ".into()),
            ..op("e1", "p1", "edit")
        },
        wire::TodoOp {
            todo_id: Some("t1".into()),
            ..op("e2", "p1", "complete")
        },
        wire::TodoOp {
            todo_id: Some("d1".into()),
            ..op("e3", "p1", "restore")
        },
        wire::TodoOp {
            todo_id: Some("t2".into()),
            ..op("e4", "p1", "remove")
        },
        wire::TodoOp {
            todo_id: Some("t2".into()),
            auto: Some(true),
            ..op("e5", "p1", "auto")
        },
        wire::TodoOp {
            order: Some(vec!["t2".into(), "t1".into()]),
            ..op("e6", "p1", "reorder")
        },
    ];
    for message in &sent {
        phone.send(message).await;
        assert_eq!(ack(&mut phone).await.status, "ok", "{}", message.op);
    }
    let seen = fx.host.todo_ops.lock().unwrap().clone();
    assert_eq!(seen.len(), sent.len());
    assert_eq!(seen[0].text.as_deref(), Some("new text"));
    assert_eq!(seen[4].auto, Some(true));
    assert_eq!(
        seen[5].order.as_deref(),
        Some(&["t2".to_string(), "t1".to_string()][..])
    );
}

#[tokio::test(start_paused = true)]
async fn a_repeated_write_is_replayed_with_a_fresh_list_and_not_applied_twice() {
    let (fx, mut phone) = live().await;
    fx.host
        .todo_lists
        .lock()
        .unwrap()
        .insert("p1".into(), list_of("p1", &[("t1", "first")]));
    *fx.host.todo_apply_result.lock().unwrap() = Some(TodoApplied::ok(Some("t9".into()), None));
    phone.send(&add("o1", "p1", "buy milk")).await;
    let first = ack(&mut phone).await;
    assert_eq!(first.list.as_ref().map(|l| l.pending.len()), Some(1));

    // Someone else adds a todo before the phone, which never saw the ack, retries.
    fx.host.todo_lists.lock().unwrap().insert(
        "p1".into(),
        list_of(
            "p1",
            &[("t1", "first"), ("t9", "buy milk"), ("t10", "other")],
        ),
    );
    phone.send(&add("o1", "p1", "buy milk")).await;
    let again = ack(&mut phone).await;
    assert_eq!(again.status, "ok");
    assert_eq!(
        again.todo_id.as_deref(),
        Some("t9"),
        "the replay names the todo the first run made"
    );
    assert_eq!(
        again.list.as_ref().map(|l| l.pending.len()),
        Some(3),
        "the list is current"
    );
    assert_eq!(fx.host.todo_ops.lock().unwrap().len(), 1, "applied once");
}

#[tokio::test(start_paused = true)]
async fn the_same_id_for_a_different_write_is_a_conflict() {
    let (fx, mut phone) = live().await;
    phone.send(&add("o1", "p1", "one")).await;
    assert_eq!(ack(&mut phone).await.status, "ok");
    phone.send(&add("o1", "p1", "another")).await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.status, "rejected");
    assert_eq!(reply.reason.as_deref(), Some("conflict"));
    assert_eq!(fx.host.todo_ops.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_refusal_is_replayed_too() {
    let (fx, mut phone) = live().await;
    phone.send(&add("o1", "p1", "  ")).await;
    assert_eq!(ack(&mut phone).await.reason.as_deref(), Some("empty"));
    phone.send(&add("o1", "p1", "  ")).await;
    let again = ack(&mut phone).await;
    assert_eq!(again.status, "rejected");
    assert_eq!(again.reason.as_deref(), Some("empty"));
    assert!(fx.host.todo_ops.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn writes_survive_a_restart_as_replays() {
    let (fx, mut phone) = live().await;
    *fx.host.todo_apply_result.lock().unwrap() = Some(TodoApplied::ok(Some("t9".into()), None));
    phone.send(&add("o1", "p1", "buy milk")).await;
    assert_eq!(ack(&mut phone).await.status, "ok");

    let fx = fx.restart().await;
    phone.set_presence(true, true).await;
    phone.hello().await;
    phone.send(&add("o1", "p1", "buy milk")).await;
    let again = ack(&mut phone).await;
    assert_eq!(again.todo_id.as_deref(), Some("t9"));
    assert_eq!(
        fx.host.todo_ops.lock().unwrap().len(),
        1,
        "not applied again"
    );
}

// ---------------------------------------------------------------- watching

#[tokio::test(start_paused = true)]
async fn a_watcher_is_told_the_current_signature_and_then_every_change() {
    let (fx, mut phone) = live().await;
    phone.send(&wire::TodoWatch { on: true }).await;
    let first: wire::TodoChanged = phone.recv_body(t::TODO_CHANGED).await;
    assert_eq!(first.rev, 1);
    assert_eq!(fx.host.todo_refreshes.load(Ordering::SeqCst), 1);

    assert!(
        !phone.gets(t::TODO_CHANGED, Duration::from_secs(3)).await,
        "nothing changed, nothing sent"
    );
    fx.host.todo_store_rev.store(5, Ordering::SeqCst);
    let changed: wire::TodoChanged = phone.recv_body(t::TODO_CHANGED).await;
    assert_eq!(changed.rev, 5);
    assert!(!phone.gets(t::TODO_CHANGED, Duration::from_secs(3)).await);
}

#[tokio::test(start_paused = true)]
async fn the_store_is_polled_about_once_a_second() {
    let (fx, mut phone) = live().await;
    phone.send(&wire::TodoWatch { on: true }).await;
    phone.recv(t::TODO_CHANGED).await;
    fx.host.todo_store_rev.store(2, Ordering::SeqCst);
    let started = tokio::time::Instant::now();
    phone.recv(t::TODO_CHANGED).await;
    assert!(
        started.elapsed() <= SECOND * 2,
        "heard within about a second"
    );
}

#[tokio::test(start_paused = true)]
async fn leaving_the_screen_stops_the_nudges() {
    let (fx, mut phone) = live().await;
    phone.send(&wire::TodoWatch { on: true }).await;
    phone.recv(t::TODO_CHANGED).await;
    phone.send(&wire::TodoWatch { on: false }).await;
    fx.host.todo_store_rev.store(2, Ordering::SeqCst);
    assert!(!phone.gets(t::TODO_CHANGED, Duration::from_secs(3)).await);
}

#[tokio::test(start_paused = true)]
async fn a_phone_in_the_background_is_dropped_from_the_watchers() {
    let (fx, mut phone) = live().await;
    phone.send(&wire::TodoWatch { on: true }).await;
    phone.recv(t::TODO_CHANGED).await;
    phone.set_presence(true, false).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    fx.host.todo_store_rev.store(2, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(3)).await;
    phone.clear_queue();

    // Back in the foreground the phone says hello again and has to ask again.
    phone.set_presence(true, true).await;
    phone.hello().await;
    phone.clear_queue();
    fx.host.todo_store_rev.store(3, Ordering::SeqCst);
    assert!(
        !phone.gets(t::TODO_CHANGED, Duration::from_secs(3)).await,
        "it was dropped, so it hears nothing until it subscribes again"
    );
    phone.send(&wire::TodoWatch { on: true }).await;
    let again: wire::TodoChanged = phone.recv_body(t::TODO_CHANGED).await;
    assert_eq!(again.rev, 3);
}

#[tokio::test(start_paused = true)]
async fn a_revoked_phone_is_dropped_from_the_watchers() {
    let (fx, mut phone) = live().await;
    phone.send(&wire::TodoWatch { on: true }).await;
    phone.recv(t::TODO_CHANGED).await;
    assert!(fx.gateway.revoke(&phone.phone_id).await);
    fx.host.todo_store_rev.store(2, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!phone.gets(t::TODO_CHANGED, Duration::from_secs(1)).await);
}

#[tokio::test(start_paused = true)]
async fn every_watching_phone_hears_the_change() {
    let (fx, mut a) = live().await;
    let mut b = fx.live_phone().await;
    settle_hello(&mut b).await;
    a.send(&wire::TodoWatch { on: true }).await;
    a.recv(t::TODO_CHANGED).await;
    b.send(&wire::TodoWatch { on: true }).await;
    b.recv(t::TODO_CHANGED).await;
    fx.host.todo_store_rev.store(9, Ordering::SeqCst);
    let got_a: wire::TodoChanged = a.recv_body(t::TODO_CHANGED).await;
    let got_b: wire::TodoChanged = b.recv_body(t::TODO_CHANGED).await;
    assert_eq!((got_a.rev, got_b.rev), (9, 9));
}
