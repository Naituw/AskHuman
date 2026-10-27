use std::sync::atomic::Ordering;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::testkit::*;
use super::*;
use crate::ios::wire::{self, t, TodoOption};

const SECOND: Duration = Duration::from_secs(1);

/// Lets the hello burst and the one the Mac repeats a second after it connected pass.
async fn settle_hello(phone: &mut TestPhone) {
    phone.recv(t::SLOT_STATE).await;
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    phone.clear_queue();
}

/// A live phone and a working Agent `claude:s1`.
async fn with_agent() -> (Fixture, TestPhone) {
    let fx = Fixture::new().await;
    fx.host.set_agents(vec![
        sample_agent("claude:s1", "working"),
        sample_agent("claude:s2", "working"),
    ]);
    let mut phone = fx.live_phone().await;
    settle_hello(&mut phone).await;
    (fx, phone)
}

fn interject(id: &str, agent_key: &str, text: &str) -> wire::InterjectSend {
    wire::InterjectSend {
        client_msg_id: id.into(),
        agent_key: agent_key.into(),
        text: text.into(),
        files: Vec::new(),
    }
}

async fn ack(phone: &mut TestPhone) -> wire::InterjectAck {
    phone.recv_body(t::INTERJECT_ACK).await
}

// ---------------------------------------------------------------- interjections

#[tokio::test(start_paused = true)]
async fn an_interjection_reaches_a_working_agent() {
    let (fx, mut phone) = with_agent().await;
    phone
        .send(&interject("i1", "claude:s1", "  also run the linter \n"))
        .await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.client_msg_id, "i1");
    assert_eq!(reply.agent_key, "claude:s1");
    assert_eq!(reply.status, "queued");
    assert_eq!(reply.pending, 1);
    assert!(reply.reason.is_none());
    assert_eq!(
        *fx.host.interjects.lock().unwrap(),
        [(
            phone.phone_id.clone(),
            "claude:s1".to_string(),
            "also run the linter".to_string(),
            0
        )]
    );
}

#[tokio::test(start_paused = true)]
async fn interjections_are_refused_with_a_reason() {
    let (fx, mut phone) = with_agent().await;
    fx.host.update_agent("claude:s2", |a| {
        a.state = "idle".into();
        a.can_interject.ok = false;
        a.can_interject.reason = Some("notWorking".into());
    });
    fx.host.agents.lock().unwrap().push({
        let mut other = sample_agent("codex:s3", "working");
        other.can_interject.ok = false;
        other.can_interject.reason = Some("unsupported".into());
        other
    });
    let cases = [
        ("i1", "claude:s2", "go on", "notWorking"),
        ("i2", "claude:nope", "go on", "notFound"),
        ("i3", "claude:s1", "   ", "empty"),
        ("i4", "codex:s3", "go on", "unsupported"),
    ];
    for (id, key, text, reason) in cases {
        phone.send(&interject(id, key, text)).await;
        let reply = ack(&mut phone).await;
        assert_eq!(reply.status, "rejected", "{id}");
        assert_eq!(reply.reason.as_deref(), Some(reason), "{id}");
    }
    phone
        .send(&interject("i5", "claude:s1", &"x".repeat(20_001)))
        .await;
    assert_eq!(ack(&mut phone).await.reason.as_deref(), Some("tooLong"));
    assert!(
        fx.host.interjects.lock().unwrap().is_empty(),
        "nothing reached the Agent"
    );
}

#[tokio::test(start_paused = true)]
async fn what_the_host_says_is_passed_on() {
    let (fx, mut phone) = with_agent().await;
    *fx.host.interject_result.lock().unwrap() = Some(InterjectResult {
        status: "delivered",
        reason: None,
        pending: 0,
    });
    phone.send(&interject("i1", "claude:s1", "now")).await;
    assert_eq!(ack(&mut phone).await.status, "delivered");

    *fx.host.interject_result.lock().unwrap() = Some(InterjectResult::rejected("hooksOff"));
    phone.send(&interject("i2", "claude:s1", "again")).await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.status, "rejected");
    assert_eq!(reply.reason.as_deref(), Some("hooksOff"));
}

#[tokio::test(start_paused = true)]
async fn repeating_an_interjection_does_not_queue_it_twice() {
    let (fx, mut phone) = with_agent().await;
    let send = interject("i1", "claude:s1", "stop using unwrap");
    phone.send(&send).await;
    let first = ack(&mut phone).await;
    phone.send(&send).await;
    let again = ack(&mut phone).await;
    assert_eq!(first, again);
    assert_eq!(fx.host.interjects.lock().unwrap().len(), 1);

    phone
        .send(&interject("i1", "claude:s1", "something else"))
        .await;
    let conflict = ack(&mut phone).await;
    assert_eq!(conflict.status, "rejected");
    assert_eq!(conflict.reason.as_deref(), Some("conflict"));
    assert_eq!(fx.host.interjects.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn queued_interjections_can_be_read_and_withdrawn() {
    let (fx, mut phone) = with_agent().await;
    *fx.host.interject_pending.lock().unwrap() = (2, "first\n\nsecond".into());
    phone
        .send(&wire::InterjectQuery {
            agent_key: "claude:s1".into(),
        })
        .await;
    let state: wire::InterjectState = phone.recv_body(t::INTERJECT_STATE).await;
    assert_eq!(state.pending, 2);
    assert_eq!(state.text.as_deref(), Some("first\n\nsecond"));

    phone
        .send(&wire::InterjectClear {
            client_msg_id: "c1".into(),
            agent_key: "claude:s1".into(),
        })
        .await;
    let state: wire::InterjectState = phone.recv_body(t::INTERJECT_STATE).await;
    assert_eq!(state.pending, 0);
    assert!(state.text.is_none());
    assert_eq!(*fx.host.cleared.lock().unwrap(), ["claude:s1"]);
}

#[tokio::test(start_paused = true)]
async fn the_phone_hears_when_an_interjection_was_read_by_the_agent() {
    let (fx, mut phone) = with_agent().await;
    fx.gateway.interject_consumed(&phone.phone_id, "claude:s1");
    let consumed: wire::InterjectConsumed = phone.recv_body(t::INTERJECT_CONSUMED).await;
    assert_eq!(consumed.agent_key, "claude:s1");

    phone.set_presence(true, false).await;
    fx.gateway.interject_consumed(&phone.phone_id, "claude:s1");
    assert!(!phone.gets(t::INTERJECT_CONSUMED, 2 * SECOND).await);
}

// ---------------------------------------------------------------- interjection files

fn sha_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Uploads `data` for an Agent in one chunk and waits until the Mac has it; returns the upload id.
async fn upload_for_agent(
    phone: &mut TestPhone,
    name: &str,
    mime: &str,
    data: &[u8],
    key: &str,
) -> String {
    let id = crypto::new_id();
    phone
        .send(&wire::UploadBegin {
            upload_id: id.clone(),
            name: name.into(),
            mime: mime.into(),
            size: data.len() as u64,
            chunks: 1,
            sha256: sha_hex(data),
            scope: wire::UploadScope {
                request_id: None,
                agent_key: Some(key.into()),
            },
        })
        .await;
    let begun: wire::UploadAck = phone.recv_body(t::UPLOAD_ACK).await;
    assert_eq!(begun.status, "ok", "{:?}", begun.reason);
    phone
        .send(&wire::UploadChunk {
            upload_id: id.clone(),
            index: 0,
            data: crypto::b64u(data),
        })
        .await;
    loop {
        let reply: wire::UploadAck = phone.recv_body(t::UPLOAD_ACK).await;
        if reply.status != "ok" {
            assert_eq!(reply.status, "complete", "{:?}", reply.reason);
            return id;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn files_are_uploaded_for_an_agent_and_handed_to_the_host() {
    let (fx, mut phone) = with_agent().await;
    let data = b"fn main() {}\n";
    let id = upload_for_agent(&mut phone, "main.rs", "text/x-rust", data, "claude:s1").await;
    let mut send = interject("i1", "claude:s1", "see attached");
    send.files = vec![id];
    phone.send(&send).await;
    assert_eq!(ack(&mut phone).await.status, "queued");

    let staged = fx.host.staged.lock().unwrap().clone();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].name, "main.rs");
    assert_eq!(staged[0].size, data.len() as u64);
    assert!(!staged[0].is_image);
    assert_eq!(std::fs::read(&staged[0].path).unwrap(), data);
}

#[tokio::test(start_paused = true)]
async fn an_upload_only_belongs_to_the_agent_it_was_made_for() {
    let (fx, mut phone) = with_agent().await;
    let id = upload_for_agent(&mut phone, "a.png", "image/png", b"png", "claude:s1").await;
    let mut send = interject("i1", "claude:s2", "wrong agent");
    send.files = vec![id.clone()];
    phone.send(&send).await;
    let reply = ack(&mut phone).await;
    assert_eq!(reply.status, "rejected");
    assert_eq!(reply.reason.as_deref(), Some("attachment"));
    assert!(fx.host.interjects.lock().unwrap().is_empty());

    // The upload survives the failed attempt: the right agent can still use it.
    let mut send = interject("i2", "claude:s1", "right agent");
    send.files = vec![id];
    phone.send(&send).await;
    assert_eq!(ack(&mut phone).await.status, "queued");
    assert!(fx.host.staged.lock().unwrap()[0].is_image);
}

#[tokio::test(start_paused = true)]
async fn uploads_for_unknown_agents_are_refused() {
    let (_fx, mut phone) = with_agent().await;
    phone
        .send(&wire::UploadBegin {
            upload_id: crypto::new_id(),
            name: "a.txt".into(),
            mime: "text/plain".into(),
            size: 1,
            chunks: 1,
            sha256: sha_hex(b"a"),
            scope: wire::UploadScope {
                request_id: None,
                agent_key: Some("claude:nope".into()),
            },
        })
        .await;
    let reply: wire::UploadAck = phone.recv_body(t::UPLOAD_ACK).await;
    assert_eq!(reply.status, "error");
    assert_eq!(reply.reason.as_deref(), Some("gone"));
}

// ---------------------------------------------------------------- new tasks

fn create(id: &str, text: &str) -> wire::TaskCreate {
    wire::TaskCreate {
        client_msg_id: id.into(),
        workspace_id: "ws1".into(),
        agent_kind: "claude".into(),
        permission: "default".into(),
        todo_id: None,
        text: text.into(),
        model: None,
        effort: None,
    }
}

async fn status(phone: &mut TestPhone) -> wire::TaskStatus {
    phone.recv_body(t::TASK_STATUS).await
}

/// Asks for the options the way the app does before creating anything.
async fn load_options(phone: &mut TestPhone) -> wire::TaskOptions {
    phone.send(&wire::TaskOptionsGet {}).await;
    phone.recv_body(t::TASK_OPTIONS).await
}

#[tokio::test(start_paused = true)]
async fn options_come_from_the_host() {
    let (fx, mut phone) = with_agent().await;
    let options = load_options(&mut phone).await;
    assert!(options.enabled);
    assert_eq!(options.workspaces.len(), 1);
    assert_eq!(options.workspaces[0].id, "ws1");
    assert_eq!(options.agents[0].kind, "claude");

    fx.host.choices.lock().unwrap().options.enabled = false;
    fx.host.choices.lock().unwrap().options.blocked = Some("disabled".into());
    let options = load_options(&mut phone).await;
    assert!(!options.enabled);
    assert_eq!(options.blocked.as_deref(), Some("disabled"));
}

#[tokio::test(start_paused = true)]
async fn todos_are_only_offered_for_workspaces_from_the_options() {
    let (fx, mut phone) = with_agent().await;
    fx.host.todos.lock().unwrap().todos = vec![TodoOption {
        id: "t1".into(),
        text: "Fix the flaky test".into(),
        auto: false,
        attachments: 1,
    }];
    fx.host.todos.lock().unwrap().overflow = 3;
    let ask = wire::TaskTodosGet {
        workspace_id: "ws1".into(),
    };
    phone.send(&ask).await;
    let todos: wire::TaskTodos = phone.recv_body(t::TASK_TODOS).await;
    assert!(todos.todos.is_empty(), "no options loaded yet");

    load_options(&mut phone).await;
    phone.send(&ask).await;
    let todos: wire::TaskTodos = phone.recv_body(t::TASK_TODOS).await;
    assert_eq!(todos.workspace_id, "ws1");
    assert_eq!(todos.todos[0].text, "Fix the flaky test");
    assert_eq!(todos.overflow, 3);

    phone
        .send(&wire::TaskTodosGet {
            workspace_id: "elsewhere".into(),
        })
        .await;
    let todos: wire::TaskTodos = phone.recv_body(t::TASK_TODOS).await;
    assert!(todos.todos.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_task_is_launched_once_and_its_agent_followed() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    let task = create("n1", "  add dark mode  ");
    phone.send(&task).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    let launched = status(&mut phone).await;
    assert_eq!(launched.stage, "launched");
    assert!(launched.agent_key.is_none());
    {
        let launches = fx.host.launches.lock().unwrap();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].workspace, "/tmp/proj");
        assert_eq!(launches[0].agent_kind, "claude");
        assert_eq!(launches[0].permission, "default");
        assert_eq!(launches[0].text, "add dark mode");
        assert_eq!(launches[0].phone_id, phone.phone_id);
    }

    fx.gateway.task_matched(&phone.phone_id, "n1", "claude:s9");
    let watching = status(&mut phone).await;
    assert_eq!(watching.stage, "watching");
    assert_eq!(watching.agent_key.as_deref(), Some("claude:s9"));
    let follow: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert_eq!(follow.keys, ["claude:s9"]);

    // The same message again changes nothing: the journal answers.
    phone.send(&task).await;
    let replay = status(&mut phone).await;
    assert_eq!(replay.stage, "launched");
    assert_eq!(fx.host.launches.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_task_that_is_still_starting_is_not_started_twice() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    fx.host.task_delay_ms.store(500, Ordering::SeqCst);
    let task = create("n1", "build it");
    phone.send(&task).await;
    phone.send(&task).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    assert_eq!(fx.host.launches.lock().unwrap().len(), 1);
    assert!(!phone.gets(t::TASK_STATUS, 2 * SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn a_session_that_beats_the_launcher_is_still_matched() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    fx.host.task_delay_ms.store(500, Ordering::SeqCst);
    phone.send(&create("n1", "build it")).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    // The Agent reports its first turn while the terminal is still opening.
    tokio::time::sleep(Duration::from_millis(100)).await;
    fx.gateway.task_matched(&phone.phone_id, "n1", "claude:s9");
    assert_eq!(status(&mut phone).await.stage, "launched");
    let watching = status(&mut phone).await;
    assert_eq!(watching.stage, "watching");
    assert_eq!(watching.agent_key.as_deref(), Some("claude:s9"));
    // Nothing is left to time out later.
    tokio::time::sleep(Duration::from_secs(70)).await;
    assert!(!phone.gets(t::TASK_STATUS, SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn a_failed_launch_is_reported_and_replayed() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    *fx.host.task_result.lock().unwrap() = Err("terminalFailed".into());
    let task = create("n1", "build it");
    phone.send(&task).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    let failed = status(&mut phone).await;
    assert_eq!(failed.stage, "failed");
    assert_eq!(failed.reason.as_deref(), Some("terminalFailed"));

    phone.send(&task).await;
    let replay = status(&mut phone).await;
    assert_eq!(
        (replay.stage.as_str(), replay.reason),
        ("failed", failed.reason)
    );
    assert_eq!(fx.host.launches.lock().unwrap().len(), 1);

    // Another attempt is a new message and may succeed.
    *fx.host.task_result.lock().unwrap() = Ok(());
    phone.send(&create("n2", "build it")).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
}

#[tokio::test(start_paused = true)]
async fn invalid_tasks_are_refused_before_anything_starts() {
    let (fx, mut phone) = with_agent().await;
    phone.send(&create("n0", "no options loaded")).await;
    let failed = status(&mut phone).await;
    assert_eq!(failed.reason.as_deref(), Some("workspaceExpired"));

    load_options(&mut phone).await;
    let mut unknown = create("n1", "text");
    unknown.workspace_id = "not-offered".into();
    let mut bad_permission = create("n2", "text");
    bad_permission.permission = "root".into();
    let mut no_agent = create("n3", "text");
    no_agent.agent_kind = " ".into();
    let cases = [
        (unknown, "workspaceExpired"),
        (bad_permission, "invalid"),
        (no_agent, "invalid"),
        (create("n4", "   "), "empty"),
        (create("n5", "a\0b"), "invalid"),
        (create("n6", &"x".repeat(3_001)), "invalid"),
    ];
    for (task, reason) in cases {
        let id = task.client_msg_id.clone();
        phone.send(&task).await;
        let failed = status(&mut phone).await;
        assert_eq!(failed.stage, "failed", "{id}");
        assert_eq!(failed.reason.as_deref(), Some(reason), "{id}");
    }
    assert!(fx.host.launches.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_todo_can_stand_in_for_the_text() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    let mut task = create("n1", "");
    task.todo_id = Some("t1".into());
    phone.send(&task).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    let launches = fx.host.launches.lock().unwrap();
    assert_eq!(launches[0].todo_id.as_deref(), Some("t1"));
    assert!(launches[0].text.is_empty());
}

// ---------------------------------------------------------------- model and effort

fn with_model(mut task: wire::TaskCreate, model: &str, effort: Option<&str>) -> wire::TaskCreate {
    task.model = Some(model.into());
    task.effort = effort.map(str::to_string);
    task
}

#[tokio::test(start_paused = true)]
async fn the_models_an_agent_offers_reach_the_phone_with_the_options() {
    let (fx, mut phone) = with_agent().await;
    let models = vec![
        wire::ModelOption {
            id: "model-a".into(),
            label: "Model A".into(),
            note: Some("flagship".into()),
            efforts: Some(vec!["low".into(), "high".into()]),
            default_effort: Some("high".into()),
        },
        wire::ModelOption {
            id: "model-b".into(),
            label: "Model B".into(),
            note: None,
            efforts: None,
            default_effort: None,
        },
    ];
    let default_model = wire::DefaultModel {
        label: Some("Model A".into()),
        efforts: Some(vec!["low".into(), "high".into()]),
        default_effort: Some("high".into()),
    };
    {
        let mut choices = fx.host.choices.lock().unwrap();
        choices.options.agents[0].models = Some(models.clone());
        choices.options.agents[0].default_model = Some(default_model.clone());
    }
    let options = load_options(&mut phone).await;
    assert_eq!(options.agents[0].models, Some(models));
    assert_eq!(options.agents[0].default_model, Some(default_model));
}

#[tokio::test(start_paused = true)]
async fn a_picked_model_and_effort_reach_the_host_and_none_means_none() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    phone
        .send(&with_model(
            create("n1", "build it"),
            "model-a",
            Some("high"),
        ))
        .await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    phone.send(&create("n2", "build it too")).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    phone
        .send(&with_model(create("n3", "model only"), "model-b", None))
        .await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    let launches = fx.host.launches.lock().unwrap();
    assert_eq!(launches.len(), 3);
    assert_eq!(launches[0].model.as_deref(), Some("model-a"));
    assert_eq!(launches[0].effort.as_deref(), Some("high"));
    assert_eq!(
        (launches[1].model.as_deref(), launches[1].effort.as_deref()),
        (None, None)
    );
    assert_eq!(launches[2].model.as_deref(), Some("model-b"));
    assert_eq!(launches[2].effort, None);
}

#[tokio::test(start_paused = true)]
async fn a_task_that_names_a_model_is_not_the_same_task_as_one_that_does_not() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    let plain = create("n1", "build it");
    phone.send(&plain).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    // Sent again as it was: answered from the journal, nothing starts.
    phone.send(&plain).await;
    assert_eq!(status(&mut phone).await.stage, "launched");
    // The same id with a model is a different message.
    phone
        .send(&with_model(plain.clone(), "model-a", None))
        .await;
    let conflict = status(&mut phone).await;
    assert_eq!(conflict.stage, "failed");
    assert_eq!(conflict.reason.as_deref(), Some("conflict"));
    assert_eq!(fx.host.launches.lock().unwrap().len(), 1);

    // And the other way round: a task that carries a model replays as itself, and another
    // model or effort under the same id is a conflict.
    let picked = with_model(create("n2", "build it"), "model-a", Some("high"));
    phone.send(&picked).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    phone.send(&picked).await;
    assert_eq!(status(&mut phone).await.stage, "launched");
    for other in [
        with_model(create("n2", "build it"), "model-b", Some("high")),
        with_model(create("n2", "build it"), "model-a", Some("low")),
        with_model(create("n2", "build it"), "model-a", None),
    ] {
        phone.send(&other).await;
        let conflict = status(&mut phone).await;
        assert_eq!(conflict.reason.as_deref(), Some("conflict"), "{other:?}");
    }
    assert_eq!(fx.host.launches.lock().unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn values_that_cannot_be_a_model_or_an_effort_are_refused_before_anything_starts() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    let long = "a".repeat(129);
    let bad_models = [
        "--yolo",
        "-m",
        "a b",
        "a:high",
        "",
        " ",
        "a\nb",
        long.as_str(),
    ];
    for (i, model) in bad_models.iter().enumerate() {
        phone
            .send(&with_model(create(&format!("m{i}"), "text"), model, None))
            .await;
        let failed = status(&mut phone).await;
        assert_eq!(failed.reason.as_deref(), Some("invalid"), "{model:?}");
    }
    for (i, effort) in ["High", "-x", "", "a b", "1x"].iter().enumerate() {
        phone
            .send(&with_model(
                create(&format!("e{i}"), "text"),
                "model-a",
                Some(effort),
            ))
            .await;
        let failed = status(&mut phone).await;
        assert_eq!(failed.reason.as_deref(), Some("invalid"), "{effort:?}");
    }
    // An effort alone (for "Default") is a fine shape; only a bad one is refused.
    let mut effort_only = create("e-only", "text");
    effort_only.effort = Some("--bad".into());
    phone.send(&effort_only).await;
    assert_eq!(status(&mut phone).await.reason.as_deref(), Some("invalid"));
    assert!(fx.host.launches.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_task_that_never_shows_up_is_reported_as_unmatched() {
    let (fx, mut phone) = with_agent().await;
    load_options(&mut phone).await;
    phone.send(&create("n1", "build it")).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    tokio::time::sleep(Duration::from_secs(55)).await;
    assert!(!phone.gets(t::TASK_STATUS, SECOND).await);
    let unmatched = status(&mut phone).await;
    assert_eq!(unmatched.stage, "unmatched");
    assert_eq!(unmatched.client_msg_id, "n1");

    // A late match is ignored: the phone already moved on.
    fx.gateway.task_matched(&phone.phone_id, "n1", "claude:s9");
    assert!(!phone.gets(t::TASK_STATUS, 2 * SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn a_full_follow_list_leaves_the_new_agent_unfollowed() {
    let (fx, mut phone) = with_agent().await;
    let many: Vec<_> = (0..5)
        .map(|i| sample_agent(&format!("claude:f{i}"), "working"))
        .collect();
    fx.host.agents.lock().unwrap().extend(many);
    // The phone follows what its list shows: give the gateway time to read the new sessions.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    phone.clear_queue();
    for i in 0..5 {
        phone
            .send(&wire::FollowSet {
                client_msg_id: format!("f{i}"),
                agent_key: format!("claude:f{i}"),
                on: true,
            })
            .await;
        let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
        assert_eq!(state.keys.len(), i + 1);
    }
    load_options(&mut phone).await;
    phone.send(&create("n1", "build it")).await;
    assert_eq!(status(&mut phone).await.stage, "accepted");
    assert_eq!(status(&mut phone).await.stage, "launched");
    fx.gateway.task_matched(&phone.phone_id, "n1", "claude:s9");
    let matched = status(&mut phone).await;
    assert_eq!(matched.stage, "launched");
    assert_eq!(matched.reason.as_deref(), Some("followLimit"));
    assert_eq!(matched.agent_key.as_deref(), Some("claude:s9"));
}

// ---------------------------------------------------------------- the slot

#[tokio::test(start_paused = true)]
async fn taking_over_questions_goes_through_the_host() {
    let (fx, mut phone) = with_agent().await;
    fx.host.slot.lock().unwrap().active = false;
    phone
        .send(&wire::SlotTake {
            client_msg_id: "s1".into(),
        })
        .await;
    let state: wire::SlotState = phone.recv_body(t::SLOT_STATE).await;
    assert!(state.active);
    assert_eq!(fx.host.slot_taken.load(Ordering::SeqCst), 1);
}
