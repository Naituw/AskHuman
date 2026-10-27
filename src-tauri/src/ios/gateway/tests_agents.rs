use std::time::Duration;

use super::testkit::*;
use super::watch::fit_frame;
use super::*;
use crate::ios::envelope::{self, Scope};
use crate::ios::wire::{self, t, PushPlain};

const SECOND: Duration = Duration::from_secs(1);

fn follow(agent_key: &str, on: bool) -> wire::FollowSet {
    wire::FollowSet {
        client_msg_id: crypto::new_msg_id(),
        agent_key: agent_key.into(),
        on,
    }
}

fn keys(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("claude:s{i}")).collect()
}

fn event(agent_key: &str, event: AgentEventKind, failed: bool) -> AgentEventInfo {
    AgentEventInfo {
        agent_key: agent_key.into(),
        event,
        failed,
    }
}

/// Reads the agents snapshot of a hello and drops everything else that came with it: the rest
/// of the burst (inbox, follow, slot) and the one the Mac repeats a second after it connected.
async fn snapshot(phone: &mut TestPhone) -> wire::AgentsSnapshot {
    phone.recv(t::SLOT_STATE).await;
    let snap = phone.recv_body(t::AGENTS_SNAPSHOT).await;
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    phone.clear_queue();
    snap
}

/// A fixture with one working agent and a live phone that already read the first snapshot.
async fn with_agent() -> (Fixture, TestPhone) {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    let mut phone = fx.live_phone().await;
    snapshot(&mut phone).await;
    (fx, phone)
}

// ---------------------------------------------------------------- the list

#[tokio::test(start_paused = true)]
async fn hello_brings_the_agent_list_with_the_followed_flags() {
    let fx = Fixture::new().await;
    fx.host.set_agents(vec![
        sample_agent("claude:s1", "working"),
        sample_agent("codex:s2", "idle"),
    ]);
    let mut phone = fx.live_phone().await;
    snapshot(&mut phone).await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;

    phone.hello().await;
    let snap = snapshot(&mut phone).await;
    assert_eq!(snap.epoch.len(), 16);
    assert_eq!(snap.agents.len(), 2);
    let by_key = |key: &str| snap.agents.iter().find(|a| a.key == key).unwrap();
    assert!(by_key("claude:s1").followed);
    assert!(!by_key("codex:s2").followed);
    assert_eq!(by_key("claude:s1").project.name, "proj");
}

#[tokio::test(start_paused = true)]
async fn the_list_puts_active_sessions_first_and_keeps_few_ended_ones() {
    let fx = Fixture::new().await;
    let mut agents = vec![sample_agent("claude:old", "idle")];
    agents[0].last_activity_at = BASE_MS - 5_000;
    agents.push(sample_agent("claude:new", "working"));
    for i in 0..14 {
        let mut ended = sample_agent(&format!("claude:e{i}"), "ended");
        ended.last_activity_at = BASE_MS + i;
        agents.push(ended);
    }
    fx.host.set_agents(agents);
    let mut phone = fx.live_phone().await;
    let snap = snapshot(&mut phone).await;
    let keys: Vec<&str> = snap.agents.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(&keys[..2], ["claude:new", "claude:old"]);
    let ended: Vec<&&str> = keys.iter().filter(|k| k.starts_with("claude:e")).collect();
    assert_eq!(ended.len(), 10, "only the latest ended sessions");
    assert!(keys.contains(&"claude:e13") && !keys.contains(&"claude:e0"));
}

#[tokio::test(start_paused = true)]
async fn changes_arrive_as_numbered_deltas_and_removals_too() {
    let (fx, mut phone) = with_agent().await;
    fx.host.update_agent("claude:s1", |a| {
        a.state = "idle".into();
        a.turn_steps = 7;
    });
    fx.host
        .agents
        .lock()
        .unwrap()
        .push(sample_agent("codex:s2", "working"));
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(delta.rev, 1);
    assert_eq!(delta.upsert.len(), 2);
    assert!(delta.remove.is_empty());

    fx.host
        .set_agents(vec![sample_agent("codex:s2", "working")]);
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(delta.rev, 2);
    assert!(delta.upsert.is_empty());
    assert_eq!(delta.remove, ["claude:s1"]);
}

#[tokio::test(start_paused = true)]
async fn a_ticking_clock_alone_sends_nothing() {
    let (fx, mut phone) = with_agent().await;
    // Only the elapsed time moves, as it does every second for a working agent.
    for secs in 31..36 {
        fx.host
            .update_agent("claude:s1", |a| a.active_elapsed_secs = secs);
        tokio::time::sleep(SECOND).await;
    }
    assert!(!phone.gets(t::AGENTS_DELTA, SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn an_agent_change_between_hello_and_the_foreground_presence_is_caught_up() {
    // The app says "foreground" only after its first sync: until then it gets no deltas.
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, false).await;
    phone.hello().await;
    snapshot(&mut phone).await;

    fx.host
        .update_agent("claude:s1", |a| a.state = "idle".into());
    assert!(!phone.gets(t::AGENTS_DELTA, 3 * SECOND).await);

    phone.set_presence(true, true).await;
    let snap: wire::AgentsSnapshot = phone.recv_body(t::AGENTS_SNAPSHOT).await;
    assert_eq!(snap.agents[0].state, "idle");

    // The stream continues right after the snapshot.
    fx.host
        .update_agent("claude:s1", |a| a.state = "working".into());
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(delta.rev, snap.rev + 1);
}

#[tokio::test(start_paused = true)]
async fn a_background_phone_gets_no_deltas_but_a_fresh_snapshot_on_hello() {
    let (fx, mut phone) = with_agent().await;
    phone.set_presence(true, false).await;
    fx.host
        .update_agent("claude:s1", |a| a.state = "idle".into());
    assert!(!phone.gets(t::AGENTS_DELTA, 3 * SECOND).await);

    phone.set_presence(true, true).await;
    phone.hello().await;
    let snap = snapshot(&mut phone).await;
    assert_eq!(snap.agents[0].state, "idle");

    // The stream continues right after the snapshot.
    fx.host
        .update_agent("claude:s1", |a| a.state = "working".into());
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert_eq!(delta.rev, snap.rev + 1);
}

// ---------------------------------------------------------------- follow

#[tokio::test(start_paused = true)]
async fn following_is_limited_and_unknown_agents_are_refused() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(keys(7).iter().map(|k| sample_agent(k, "working")).collect());
    let mut phone = fx.live_phone().await;
    snapshot(&mut phone).await;
    for (i, key) in keys(5).iter().enumerate() {
        phone.send(&follow(key, true)).await;
        let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
        assert_eq!(state.keys.len(), i + 1);
        assert!(state.error.is_none());
    }

    phone.send(&follow("claude:s5", true)).await;
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert_eq!(state.keys.len(), 5);
    assert_eq!(state.error.as_deref(), Some("limit"));

    phone.send(&follow("claude:nope", true)).await;
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert_eq!(state.error.as_deref(), Some("notFound"));

    phone.send(&follow("claude:s0", false)).await;
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert_eq!(state.keys.len(), 4);
    assert!(state.error.is_none());

    // Following something already followed changes nothing and is no error.
    phone.send(&follow("claude:s1", true)).await;
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert_eq!(state.keys.len(), 4);
    assert!(state.error.is_none());
}

#[tokio::test(start_paused = true)]
async fn the_follow_set_is_still_there_after_a_restart() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    let mut phone = fx.live_phone().await;
    snapshot(&mut phone).await;
    phone.send(&follow("claude:s1", true)).await;
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert_eq!(state.keys, ["claude:s1"]);

    let fx = fx.restart().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    phone.set_presence(true, true).await;
    phone.clear_queue();
    phone.hello().await;
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert_eq!(state.keys, ["claude:s1"]);
    let snap = snapshot(&mut phone).await;
    assert!(snap.agents[0].followed);
}

#[tokio::test(start_paused = true)]
async fn a_followed_flag_changes_with_the_next_delta() {
    let (_fx, mut phone) = with_agent().await;
    phone.send(&follow("claude:s1", true)).await;
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert!(delta.upsert[0].followed);
}

#[tokio::test(start_paused = true)]
async fn ended_sessions_leave_the_follow_set_silently() {
    let (fx, mut phone) = with_agent().await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;
    fx.host
        .update_agent("claude:s1", |a| a.state = "ended".into());
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert!(state.keys.is_empty());
    assert!(state.error.is_none());
}

// ---------------------------------------------------------------- notifications

#[tokio::test(start_paused = true)]
async fn done_is_announced_after_a_quiet_period_and_cancelled_by_new_work() {
    let (fx, mut phone) = with_agent().await;
    fx.host
        .set_frame("claude:s1", "a", "All tests pass now.", false);
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;

    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, false));
    assert!(!phone.gets(t::FOLLOW_EVENT, SECOND).await, "debounced");
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnStart, false));
    assert!(
        !phone.gets(t::FOLLOW_EVENT, 4 * SECOND).await,
        "a new turn cancels the announcement"
    );

    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, false));
    let seen: wire::FollowEvent = phone.recv_body(t::FOLLOW_EVENT).await;
    assert_eq!(seen.kind, "done");
    assert_eq!(seen.agent_key, "claude:s1");
    assert_eq!(seen.project, "proj");
    assert_eq!(seen.title, "Task s1");
    assert_eq!(seen.preview, "All tests pass now.");
}

#[tokio::test(start_paused = true)]
async fn activity_during_the_quiet_period_cancels_done() {
    let (fx, mut phone) = with_agent().await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, false));
    tokio::time::sleep(SECOND).await;
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::Activity, false));
    assert!(!phone.gets(t::FOLLOW_EVENT, 5 * SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn a_failed_turn_is_announced_at_once_and_marks_the_agent() {
    let (fx, mut phone) = with_agent().await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;
    phone.clear_queue();

    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, true));
    let seen: wire::FollowEvent = phone.recv_body(t::FOLLOW_EVENT).await;
    assert_eq!(seen.kind, "failed");
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert!(delta.upsert[0].last_turn_failed);

    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnStart, false));
    let delta: wire::AgentsDelta = phone.recv_body(t::AGENTS_DELTA).await;
    assert!(!delta.upsert[0].last_turn_failed);
}

#[tokio::test(start_paused = true)]
async fn nothing_is_announced_for_agents_nobody_follows() {
    let (fx, mut phone) = with_agent().await;
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, true));
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, false));
    assert!(!phone.gets(t::FOLLOW_EVENT, 5 * SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn an_agent_that_waits_for_an_answer_is_not_announced() {
    // The question is what notifies (the inbox and its push); the Agent waiting for it adds nothing.
    let (fx, mut phone) = with_agent().await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;
    fx.host.update_agent("claude:s1", |a| {
        a.waiting_request_id = Some("req-popup".into());
        a.waiting_preview = Some("Delete the build folder?".into());
        a.state = "waiting".into();
    });
    assert!(!phone.gets(t::FOLLOW_EVENT, 6 * SECOND).await);
    phone.set_presence(true, false).await;
    tokio::time::sleep(6 * SECOND).await;
    assert_eq!(fx.pusher.count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_background_phone_gets_a_push_instead_of_a_live_event() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    let mut phone = fx.live_phone().await;
    snapshot(&mut phone).await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;
    phone.set_presence(true, false).await;
    fx.host.set_frame(
        "claude:s1",
        "a",
        "Refactor finished, 12 files changed.",
        false,
    );

    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, false));
    tokio::time::sleep(4 * SECOND).await;
    assert!(!phone.gets(t::FOLLOW_EVENT, SECOND).await);
    let push = fx.pusher.last().expect("a push went out");
    assert_eq!(push.loc_key, "PUSH_FOLLOW_DONE");
    assert_eq!(push.collapse_id, "fw-claude:s1");

    let keys = phone.keys.clone().unwrap();
    let plain = envelope::open(&keys.m2p, Scope::Push, &push.ah).expect("push opens");
    let plain: PushPlain = serde_json::from_slice(&plain).unwrap();
    assert_eq!(plain.k, "follow.done");
    assert_eq!(plain.rid.as_deref(), Some("claude:s1"));
    assert_eq!(plain.project, "proj");
    assert_eq!(plain.text, "Refactor finished, 12 files changed.");
}

// A result (done / failed) of a followed Agent reaches a phone that is not looking only after
// the Mac has been idle for `alert_after_secs`, the way a question does, and ends the follow:
// the follow was for that one result (docs/specs/ios-follow-narrowing.md).

const MINUTE: Duration = Duration::from_secs(60);

/// A phone that follows `claude:s1` and then leaves the app, so its results are pushed once due.
async fn away_follower() -> (Fixture, TestPhone) {
    let (fx, mut phone) = with_agent().await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;
    phone.set_presence(true, false).await;
    phone.clear_queue();
    (fx, phone)
}

fn turn_end(failed: bool) -> AgentEventInfo {
    event("claude:s1", AgentEventKind::TurnEnd, failed)
}

/// Whether the follow set the Mac reports on hello is empty.
async fn follows_nothing(phone: &mut TestPhone) -> bool {
    phone.clear_queue();
    phone.hello().await;
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    state.keys.is_empty()
}

#[tokio::test(start_paused = true)]
async fn a_result_is_pushed_once_the_mac_has_been_idle_long_enough() {
    let (fx, mut phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(false));

    tokio::time::sleep(10 * MINUTE).await;
    assert_eq!(
        fx.pusher.count(),
        0,
        "the Mac has been idle for ten minutes only"
    );
    tokio::time::sleep(6 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1, "idle for sixteen minutes by now");
    let push = fx.pusher.last().unwrap();
    assert_eq!(
        (
            push.kind.as_str(),
            push.loc_key.as_str(),
            push.collapse_id.as_str()
        ),
        ("alert", "PUSH_FOLLOW_DONE", "fw-claude:s1")
    );

    // That was the result the phone asked for: the Agent left the follow set.
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert!(state.keys.is_empty());

    // Later turns do not notify again, however long the Mac idles.
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnStart, false));
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(60 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1, "once");
}

#[tokio::test(start_paused = true)]
async fn a_result_is_pushed_at_once_when_the_mac_is_already_idle() {
    let (fx, _phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.idle_for(20 * 60);
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(5 * SECOND).await;
    assert_eq!(fx.pusher.count(), 1, "the 3 s quiet period and a tick");
}

#[tokio::test(start_paused = true)]
async fn a_zero_wait_pushes_a_result_at_once() {
    let (fx, _phone) = away_follower().await;
    fx.host.set_alert_after(0);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(true));
    tokio::time::sleep(SECOND).await;
    assert_eq!(
        fx.pusher.count(),
        1,
        "a failure is not delayed by the quiet period"
    );
}

#[tokio::test(start_paused = true)]
async fn an_unknown_idle_time_pushes_a_result_at_once() {
    let (fx, _phone) = away_follower().await;
    fx.host.set_alert_after(900); // the test host knows no idle time until told
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(5 * SECOND).await;
    assert_eq!(fx.pusher.count(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failure_waits_like_a_finished_turn() {
    let (fx, phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(true));

    tokio::time::sleep(5 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0, "the Mac is in use");
    tokio::time::sleep(11 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1);
    let push = fx.pusher.last().unwrap();
    assert_eq!(push.loc_key, "PUSH_FOLLOW_FAILED");
    let keys = phone.keys.clone().unwrap();
    let plain = envelope::open(&keys.m2p, Scope::Push, &push.ah).expect("push opens");
    let plain: PushPlain = serde_json::from_slice(&plain).unwrap();
    assert_eq!(plain.k, "follow.failed");
    assert_eq!(plain.rid.as_deref(), Some("claude:s1"));
}

#[tokio::test(start_paused = true)]
async fn input_on_the_mac_after_a_result_voids_it() {
    let (fx, mut phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(5 * MINUTE).await;

    fx.host.touch_input(); // back at the keyboard: the result has been seen there
    tokio::time::sleep(40 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0, "no push for what you have seen");
    assert!(
        follows_nothing(&mut phone).await,
        "and nothing left to notify"
    );
}

#[tokio::test(start_paused = true)]
async fn input_just_before_a_result_does_not_void_it() {
    let (fx, _phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input(); // the last thing the user did before the Agent finished
    tokio::time::sleep(2 * SECOND).await;
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(5 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0);
    tokio::time::sleep(11 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_agent_that_works_again_voids_the_result_but_stays_followed() {
    let (fx, mut phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(5 * MINUTE).await;

    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnStart, false));
    tokio::time::sleep(40 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0, "it was no result after all");
    assert!(!follows_nothing(&mut phone).await, "it is still followed");

    // The real result finds the Mac idle already.
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(5 * SECOND).await;
    assert_eq!(fx.pusher.count(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_session_that_ends_voids_the_result() {
    let (fx, _phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(10 * SECOND).await;
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::SessionEnd, false));
    tokio::time::sleep(40 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_looks_before_the_wait_is_over_gets_no_push() {
    let (fx, mut phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(false));
    tokio::time::sleep(5 * MINUTE).await;

    phone.set_presence(true, true).await;
    phone.hello().await;
    tokio::time::sleep(40 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0, "it can see the Agent in the list");
    assert!(follows_nothing(&mut phone).await);
}

#[tokio::test(start_paused = true)]
async fn a_live_phone_gets_the_banner_and_the_follow_ends() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;

    fx.gateway.agent_event(turn_end(true));
    let seen: wire::FollowEvent = phone.recv_body(t::FOLLOW_EVENT).await;
    assert_eq!(seen.kind, "failed");
    let state: wire::FollowState = phone.recv_body(t::FOLLOW_STATE).await;
    assert!(state.keys.is_empty());
    tokio::time::sleep(30 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 0);
}

#[tokio::test(start_paused = true)]
async fn each_follower_is_judged_on_its_own() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    fx.host.set_alert_after(900);
    fx.host.idle_for(20 * 60);
    let mut looking = fx.live_phone().await;
    snapshot(&mut looking).await;
    let mut away = fx.pair_phone().await;
    for phone in [&mut looking, &mut away] {
        phone.send(&follow("claude:s1", true)).await;
        phone.recv(t::FOLLOW_STATE).await;
    }
    away.set_presence(true, false).await;

    fx.gateway.agent_event(turn_end(false));
    let seen: wire::FollowEvent = looking.recv_body(t::FOLLOW_EVENT).await;
    assert_eq!(seen.kind, "done");
    tokio::time::sleep(5 * SECOND).await;
    assert_eq!(fx.pusher.count(), 1, "only the phone that is away");
    let push = fx.pusher.last().unwrap();
    let keys = away.keys.clone().unwrap();
    assert!(envelope::open(&keys.m2p, Scope::Push, &push.ah).is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_newer_result_replaces_one_that_was_not_pushed_yet() {
    let (fx, _phone) = away_follower().await;
    fx.host.set_alert_after(900);
    fx.host.touch_input();
    fx.gateway.agent_event(turn_end(true));
    tokio::time::sleep(SECOND).await;
    fx.gateway.agent_event(turn_end(false));

    tokio::time::sleep(20 * MINUTE).await;
    assert_eq!(fx.pusher.count(), 1, "one notification, not two");
    assert_eq!(fx.pusher.last().unwrap().loc_key, "PUSH_FOLLOW_DONE");
}

#[tokio::test(start_paused = true)]
async fn a_phone_that_cannot_be_pushed_to_keeps_following() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(vec![sample_agent("claude:s1", "working")]);
    fx.host.set_alert_after(0);
    let mut phone = fx.pair_phone_without_push().await;
    phone.set_presence(true, true).await;
    phone.hello().await;
    snapshot(&mut phone).await;
    phone.send(&follow("claude:s1", true)).await;
    phone.recv(t::FOLLOW_STATE).await;
    phone.set_presence(true, false).await;

    fx.gateway.agent_event(turn_end(true));
    tokio::time::sleep(30 * SECOND).await;
    assert_eq!(fx.pusher.count(), 0, "there is nowhere to push to");
    assert!(
        !follows_nothing(&mut phone).await,
        "nothing reached the phone, so the result is still owed"
    );
}

// ---------------------------------------------------------------- watch

fn watch(agent_key: &str) -> wire::WatchOpen {
    wire::WatchOpen {
        agent_key: agent_key.into(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_watch_streams_changed_frames_only() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_frame("claude:s1", "a", "Reading files", false);
    phone.send(&watch("claude:s1")).await;
    let first: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!((first.seq, first.agent_key.as_str()), (1, "claude:s1"));
    assert_eq!(first.frame["text"], "Reading files");
    assert_eq!(first.frame["failed"], false);

    assert!(
        !phone.gets(t::WATCH_FRAME, 5 * SECOND).await,
        "unchanged frames are not sent again"
    );
    fx.host
        .set_frame("claude:s1", "b", "Editing main.rs", false);
    let second: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(second.seq, 2);
    assert_eq!(second.frame["text"], "Editing main.rs");
}

#[tokio::test(start_paused = true)]
async fn frames_are_at_least_two_seconds_apart() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_frame("claude:s1", "a", "one", false);
    phone.send(&watch("claude:s1")).await;
    phone.recv(t::WATCH_FRAME).await;
    fx.host.set_frame("claude:s1", "b", "two", false);
    assert!(
        !phone
            .gets(t::WATCH_FRAME, Duration::from_millis(1200))
            .await
    );
    assert!(phone.gets(t::WATCH_FRAME, 2 * SECOND).await);
}

#[tokio::test(start_paused = true)]
async fn a_watch_ends_with_the_session() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_frame("claude:s1", "a", "Working", false);
    phone.send(&watch("claude:s1")).await;
    phone.recv(t::WATCH_FRAME).await;
    fx.host.set_frame("claude:s1", "z", "Done.", true);
    let last: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(last.frame["phase"], "ended");
    let closed: wire::WatchClosed = phone.recv_body(t::WATCH_CLOSED).await;
    assert_eq!(closed.reason, "ended");
    assert_eq!(fx.gateway.live_watch_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_vanished_session_closes_the_watch() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_frame("claude:s1", "a", "Working", false);
    phone.send(&watch("claude:s1")).await;
    phone.recv(t::WATCH_FRAME).await;
    fx.host.frames.lock().unwrap().clear();
    let closed: wire::WatchClosed = phone.recv_body(t::WATCH_CLOSED).await;
    assert_eq!(closed.reason, "ended");
}

#[tokio::test(start_paused = true)]
async fn two_watches_at_most_and_unknown_agents_are_refused() {
    let fx = Fixture::new().await;
    fx.host
        .set_agents(keys(3).iter().map(|k| sample_agent(k, "working")).collect());
    let mut phone = fx.live_phone().await;
    snapshot(&mut phone).await;
    phone.send(&watch("claude:s0")).await;
    phone.send(&watch("claude:s1")).await;
    assert_eq!(fx.gateway.live_watch_count(), 2);
    phone.send(&watch("claude:s2")).await;
    let closed: wire::WatchClosed = phone.recv_body(t::WATCH_CLOSED).await;
    assert_eq!(
        (closed.agent_key.as_str(), closed.reason.as_str()),
        ("claude:s2", "limit")
    );
    phone.send(&watch("claude:ghost")).await;
    let closed: wire::WatchClosed = phone.recv_body(t::WATCH_CLOSED).await;
    assert_eq!(closed.reason, "notFound");

    phone
        .send(&wire::WatchClose {
            agent_key: "claude:s0".into(),
        })
        .await;
    assert_eq!(fx.gateway.live_watch_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn leaving_the_foreground_stops_every_watch() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_frame("claude:s1", "a", "Working", false);
    phone.send(&watch("claude:s1")).await;
    phone.recv(t::WATCH_FRAME).await;
    assert_eq!(fx.gateway.live_watch_count(), 1);

    phone.set_presence(true, false).await;
    assert_eq!(fx.gateway.live_watch_count(), 0);
    fx.host.set_frame("claude:s1", "b", "More work", false);
    assert!(!phone.gets(t::WATCH_FRAME, 6 * SECOND).await);

    // Back in the foreground the phone opens its view again and gets a fresh frame.
    phone.set_presence(true, true).await;
    phone.hello().await;
    phone.send(&watch("claude:s1")).await;
    let frame: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(frame.seq, 1);
    assert_eq!(frame.frame["text"], "More work");
}

#[tokio::test(start_paused = true)]
async fn opening_a_watch_again_resends_the_frame() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_frame("claude:s1", "a", "Same", false);
    phone.send(&watch("claude:s1")).await;
    phone.recv(t::WATCH_FRAME).await;
    phone.send(&watch("claude:s1")).await;
    let again: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(again.seq, 2);
}

#[tokio::test(start_paused = true)]
async fn failed_turns_show_in_the_frame() {
    let (fx, mut phone) = with_agent().await;
    fx.host.set_frame("claude:s1", "a", "Boom", false);
    fx.gateway
        .agent_event(event("claude:s1", AgentEventKind::TurnEnd, true));
    phone.send(&watch("claude:s1")).await;
    let frame: wire::WatchFrame = phone.recv_body(t::WATCH_FRAME).await;
    assert_eq!(frame.frame["failed"], true);
}

#[test]
fn oversized_frames_are_shrunk_to_the_budget() {
    let big = "word ".repeat(6_000);
    let steps: Vec<_> = (0..200)
        .map(|i| serde_json::json!({"kind": "run", "object": format!("cmd {i}"), "state": "done"}))
        .collect();
    let frame = fit_frame(serde_json::json!({
        "phase": "working",
        "text": big,
        "steps": steps,
        "todos": [],
    }));
    let size = serde_json::to_vec(&frame).unwrap().len();
    assert!(size <= wire::WATCH_FRAME_MAX_BYTES, "{size} bytes");
    assert_eq!(frame["phase"], "working");
    let text = frame["text"].as_str().unwrap();
    assert!(text.starts_with('…') && text.ends_with("word "));

    let small = serde_json::json!({"phase": "idle", "text": "hi", "steps": [], "todos": []});
    assert_eq!(fit_frame(small.clone()), small);
}
