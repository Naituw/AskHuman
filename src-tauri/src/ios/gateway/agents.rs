//! The Agents stream, Follow and its notifications (plan §3.4, §4.6).
//!
//! Every phone has its own `agents` stream (revision and last-sent state per phone) because
//! `followed` differs between phones. The agent list itself comes from the host about once a
//! second, only while somebody could look at it or follows an Agent.
//!
//! A followed Agent is one the phone wants the first result of (`done` or `failed`) from. A phone
//! that is not looking gets that result as a push only once the Mac has been idle for the
//! configured time, like a question (`docs/specs/ios-follow-narrowing.md`); once the result was
//! dealt with the Agent leaves the follow set.

use std::collections::{HashMap, HashSet};

use crate::agents::result::{TurnOutcome, TurnResult};
use serde_json::Value;

use super::hostapi::{AgentEventInfo, AgentEventKind};
use super::tasks::TaskWait;
use super::Actor;
use crate::ios::wire::{
    self, AgentView, FollowEvent, FollowSet, FollowState, PushPlain, MAX_FOLLOW,
    PUSH_PREVIEW_MAX_CHARS,
};

/// The agent list is re-read at most this often.
const AGENTS_POLL_MS: i64 = 1_000;
/// A finished turn is announced after this quiet period (a new turn or activity cancels it).
const DONE_DEBOUNCE_MS: i64 = 3_000;
/// Sessions the phones can see at most: the active ones first, then the latest ended ones.
const MAX_AGENTS: usize = 60;
const MAX_ENDED_AGENTS: usize = 10;

/// A result (`done` or `failed`) of a followed Agent for a phone that was not looking when it
/// happened: a push that waits until the Mac has been idle for the configured time. Memory only.
pub(crate) struct FollowNotice {
    pub phone_id: String,
    pub agent_key: String,
    pub loc_key: &'static str,
    /// What the notification says, fixed when the result happened.
    pub plain: PushPlain,
    /// When the result happened.
    pub at: i64,
}

/// What the gateway remembers about Agents between ticks.
#[derive(Default)]
pub(crate) struct Live {
    /// Latest list from the host (per-phone fields left at their defaults).
    pub agents: Vec<AgentView>,
    pub agents_at: i64,
    /// Sessions whose last turn failed (cleared by the next turn).
    pub failed: HashSet<String>,
    pub results: HashMap<String, TurnResult>,
    /// Session → when "done" is announced.
    pub done_at: HashMap<String, i64>,
    /// Results waiting for the Mac to be idle long enough; at most one per phone and Agent.
    pub follow_notices: Vec<FollowNotice>,
    /// New tasks waiting for their Agent session to show up.
    pub task_waits: Vec<TaskWait>,
    /// `task.create` messages whose terminal is still being started.
    pub task_inflight: HashSet<String>,
    /// Messages being handed to a native app or hook; duplicates await the first result.
    pub interject_inflight: HashSet<String>,
    /// `task.create` message → Agent session that already showed up while its terminal was still
    /// being started (the session can beat the launcher's return).
    pub task_early: HashMap<String, String>,
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Signature of what the phone shows. The elapsed time is left out: it ticks every second and
/// the phone extrapolates it from the last value.
fn agent_sig(view: &AgentView) -> String {
    let mut view = view.clone();
    view.active_elapsed_secs = 0;
    serde_json::to_string(&view).unwrap_or_default()
}

impl Actor {
    // ------------------------------------------------------------ the list

    pub(super) fn refresh_agents(&mut self, now: i64) {
        let mut agents = self.host.agents();
        // Active sessions first (latest activity first), then the most recent ended ones.
        agents.sort_by(|a, b| {
            (a.state == "ended")
                .cmp(&(b.state == "ended"))
                .then(b.last_activity_at.cmp(&a.last_activity_at))
                .then(a.key.cmp(&b.key))
        });
        let mut ended = 0;
        agents.retain(|a| {
            if a.state == "ended" {
                ended += 1;
                ended <= MAX_ENDED_AGENTS
            } else {
                true
            }
        });
        agents.truncate(MAX_AGENTS);
        self.live.agents = agents;
        self.live.agents_at = now;
    }

    fn refresh_agents_if_stale(&mut self) {
        let now = self.now();
        if self.live.agents_at == 0 || now - self.live.agents_at >= AGENTS_POLL_MS {
            self.refresh_agents(now);
        }
    }

    /// The Agent behind `key` (reads the host again when the cache is stale).
    pub(super) fn known_agent(&mut self, key: &str) -> Option<AgentView> {
        self.refresh_agents_if_stale();
        self.live.agents.iter().find(|a| a.key == key).cloned()
    }

    /// The list as one phone sees it.
    fn agents_for(&self, phone_id: &str) -> Vec<AgentView> {
        let follow = self
            .store
            .state
            .phone(phone_id)
            .map(|p| p.follow_set.clone())
            .unwrap_or_default();
        self.live
            .agents
            .iter()
            .map(|agent| {
                let mut agent = agent.clone();
                agent.followed = follow.contains(&agent.key);
                agent.last_turn_failed = agent
                    .last_turn_result
                    .as_ref()
                    .map(|result| result.outcome == TurnOutcome::Failed)
                    .unwrap_or_else(|| self.live.failed.contains(&agent.key));
                agent
            })
            .collect()
    }

    pub(super) fn send_agents_snapshot(&mut self, phone_id: &str) {
        self.refresh_agents_if_stale();
        let views = self.agents_for(phone_id);
        let rt = self.rt.entry(phone_id.to_string()).or_default();
        rt.agents_sent = views.iter().map(|a| (a.key.clone(), a.clone())).collect();
        let rev = rt.agents_rev;
        let snapshot = wire::AgentsSnapshot {
            epoch: self.epoch.clone(),
            rev,
            agents: views,
        };
        self.send(phone_id, &snapshot);
    }

    pub(super) fn send_agents_delta(&mut self, phone_id: &str) {
        let views = self.agents_for(phone_id);
        let Some(rt) = self.rt.get_mut(phone_id) else {
            return;
        };
        let mut upsert = Vec::new();
        let mut next = HashMap::with_capacity(views.len());
        for view in views {
            let changed = rt
                .agents_sent
                .get(&view.key)
                .is_none_or(|old| agent_sig(old) != agent_sig(&view));
            if changed {
                upsert.push(view.clone());
            }
            next.insert(view.key.clone(), view);
        }
        let remove: Vec<String> = rt
            .agents_sent
            .keys()
            .filter(|key| !next.contains_key(*key))
            .cloned()
            .collect();
        if upsert.is_empty() && remove.is_empty() {
            return;
        }
        rt.agents_rev += 1;
        rt.agents_sent = next;
        let delta = wire::AgentsDelta {
            rev: rt.agents_rev,
            upsert,
            remove,
        };
        self.send(phone_id, &delta);
    }

    /// Re-reads the list when somebody could use it and sends what changed to the phones that
    /// look at it.
    pub(super) fn tick_agents(&mut self, now: i64) {
        let live = self.live_phones();
        let following = self
            .store
            .state
            .phones
            .iter()
            .any(|p| !p.follow_set.is_empty());
        if (live.is_empty() && !following) || now - self.live.agents_at < AGENTS_POLL_MS {
            return;
        }
        self.refresh_agents(now);
        self.prune_follows();
        for phone_id in live {
            self.send_agents_delta(&phone_id);
        }
    }

    // ------------------------------------------------------------ follow

    fn send_follow_state(&mut self, phone_id: &str, error: Option<&str>) {
        let keys = self
            .store
            .state
            .phone(phone_id)
            .map(|p| p.follow_set.clone())
            .unwrap_or_default();
        self.send(
            phone_id,
            &FollowState {
                rev: self.revs.follow,
                keys,
                error: error.map(str::to_string),
            },
        );
    }

    pub(super) fn on_follow_set(&mut self, phone_id: &str, set: FollowSet) {
        let known = self.known_agent(&set.agent_key);
        let mut error = None;
        if let Some(phone) = self.store.state.phone_mut(phone_id) {
            let present = phone.follow_set.contains(&set.agent_key);
            if !set.on {
                phone.follow_set.retain(|key| key != &set.agent_key);
                phone.follow_turns.remove(&set.agent_key);
            } else if present {
                // Already following: nothing to change.
            } else if known.is_none_or(|a| a.state == "ended") {
                error = Some("notFound");
            } else if phone.follow_set.len() >= MAX_FOLLOW {
                error = Some("limit");
            } else {
                phone.follow_set.push(set.agent_key.clone());
            }
        }
        self.store.save_state();
        self.revs.follow += 1;
        self.send_follow_state(phone_id, error);
        self.send_agents_delta(phone_id);
    }

    /// Adds an Agent to the follow set when there is room (tasks created from the phone).
    pub(super) fn activate_control_follow(
        &mut self,
        agent_key: &str,
        control: &crate::agents::control::StartedControl,
    ) -> bool {
        if !self
            .store
            .state
            .active_phones()
            .any(|phone| phone.phone_id == control.phone_id)
        {
            return false;
        }
        let Some(phone) = self.store.state.phone_mut(&control.phone_id) else {
            return false;
        };
        if phone.activated_controls.contains(&control.operation_id) {
            return false;
        }
        phone.activated_controls.push(control.operation_id.clone());
        if phone.activated_controls.len() > 128 {
            phone.activated_controls.remove(0);
        }
        let room = phone.follow_set.iter().any(|key| key == agent_key)
            || phone.follow_set.len() < MAX_FOLLOW;
        if room {
            if !phone.follow_set.iter().any(|key| key == agent_key) {
                phone.follow_set.push(agent_key.into());
            }
            phone
                .follow_turns
                .insert(agent_key.into(), control.turn_id.clone());
        }
        self.store.save_state();
        self.revs.follow += 1;
        self.send_follow_state(&control.phone_id, (!room).then_some("limit"));
        room
    }

    /// Adds an Agent to the follow set when there is room (tasks created from the phone).
    pub(super) fn follow_if_room(&mut self, phone_id: &str, agent_key: &str) -> bool {
        let Some(phone) = self.store.state.phone_mut(phone_id) else {
            return false;
        };
        if phone.follow_set.iter().any(|key| key == agent_key) {
            return true;
        }
        if phone.follow_set.len() >= MAX_FOLLOW {
            return false;
        }
        phone.follow_set.push(agent_key.to_string());
        self.store.save_state();
        self.revs.follow += 1;
        self.send_follow_state(phone_id, None);
        true
    }

    /// Ended sessions leave every follow set silently.
    fn prune_follows(&mut self) {
        let alive: HashSet<&str> = self
            .live
            .agents
            .iter()
            .filter(|a| a.state != "ended")
            .map(|a| a.key.as_str())
            .collect();
        let mut changed = Vec::new();
        for phone in self.store.state.phones.iter_mut() {
            let before = phone.follow_set.len();
            phone.follow_set.retain(|key| alive.contains(key.as_str()));
            phone
                .follow_turns
                .retain(|key, _| phone.follow_set.contains(key));
            if phone.follow_set.len() != before {
                changed.push(phone.phone_id.clone());
            }
        }
        if changed.is_empty() {
            return;
        }
        self.store.save_state();
        self.revs.follow += 1;
        for phone_id in changed {
            self.send_follow_state(&phone_id, None);
        }
    }

    fn followers(&self, agent_key: &str) -> Vec<String> {
        self.store
            .state
            .active_phones()
            .filter(|p| {
                p.follow_set.iter().any(|key| key == agent_key)
                    && p.follow_turns.get(agent_key).is_none_or(|turn| {
                        self.live
                            .results
                            .get(agent_key)
                            .and_then(|result| result.turn_id.as_ref())
                            == Some(turn)
                    })
            })
            .map(|p| p.phone_id.clone())
            .collect()
    }

    // ------------------------------------------------------------ events

    pub(super) fn on_agent_event(&mut self, info: AgentEventInfo) {
        let now = self.now();
        let key = info.agent_key;
        if let Some(control) = info.started_control {
            if self.activate_control_follow(&key, &control) {
                self.live.results.remove(&key);
            }
        }
        match info.event {
            // The Agent works again: the result that was about to be announced is no result.
            AgentEventKind::TurnStart => {
                self.live.done_at.remove(&key);
                self.live.failed.remove(&key);
                self.live.results.remove(&key);
                self.drop_notices(&key);
            }
            AgentEventKind::Activity => {
                self.live.done_at.remove(&key);
                self.drop_notices(&key);
            }
            AgentEventKind::SessionEnd => {
                self.live.done_at.remove(&key);
                self.drop_notices(&key);
            }
            AgentEventKind::TurnEnd => {
                let outcome =
                    info.result
                        .as_ref()
                        .map(|result| result.outcome)
                        .unwrap_or(if info.failed {
                            TurnOutcome::Failed
                        } else {
                            TurnOutcome::Unknown
                        });
                if let Some(result) = info.result {
                    if self.live.results.get(&key).is_some_and(|old| {
                        old.turn_id == result.turn_id
                            && old.outcome == result.outcome
                            && old.reason == result.reason
                    }) {
                        return;
                    }
                    self.live.results.insert(key.clone(), result);
                }
                self.live.done_at.remove(&key);
                self.drop_notices(&key);
                if outcome == TurnOutcome::Failed {
                    self.live.failed.insert(key.clone());
                    self.announce(&key, "failed", now);
                } else if outcome == TurnOutcome::Incomplete {
                    self.live.failed.remove(&key);
                    self.announce(&key, "incomplete", now);
                } else if outcome == TurnOutcome::Completed && !self.followers(&key).is_empty() {
                    self.live.failed.remove(&key);
                    self.live.done_at.insert(key, now + DONE_DEBOUNCE_MS);
                } else if outcome == TurnOutcome::Interrupted {
                    self.live.failed.remove(&key);
                    let turn = self
                        .live
                        .results
                        .get(&key)
                        .and_then(|result| result.turn_id.clone())
                        .unwrap_or_default();
                    for phone in self
                        .store
                        .state
                        .phones
                        .iter_mut()
                        .filter(|phone| phone.follow_set.contains(&key))
                    {
                        // An interrupted phone round must not subscribe to unrelated later Mac turns.
                        phone
                            .follow_turns
                            .entry(key.clone())
                            .or_insert_with(|| turn.clone());
                    }
                    self.store.save_state();
                }
            }
        }
    }

    pub(super) fn tick_follow(&mut self, now: i64) {
        if self.live.done_at.is_empty() {
            return;
        }
        let due: Vec<String> = self
            .live
            .done_at
            .iter()
            .filter(|(_, at)| now >= **at)
            .map(|(key, _)| key.clone())
            .collect();
        for key in due {
            self.live.done_at.remove(&key);
            self.announce(&key, "done", now);
        }
    }

    /// Forgets the results of an Agent that works again or is gone, before they were pushed.
    fn drop_notices(&mut self, agent_key: &str) {
        self.live
            .follow_notices
            .retain(|notice| notice.agent_key != agent_key);
    }

    /// Tells every phone that follows `agent_key` about a result (`done` or `failed`): over the
    /// live link when the app is open. Otherwise the push is queued until the Mac has been idle
    /// for the configured time (`flush_follow_notices`). Whatever reaches the phone, or no longer
    /// needs to, ends the follow: it was for this one result.
    fn announce(&mut self, agent_key: &str, kind: &str, now: i64) {
        let followers = self.followers(agent_key);
        if followers.is_empty() {
            return;
        }
        let agent = self
            .live
            .agents
            .iter()
            .find(|a| a.key == agent_key)
            .cloned();
        let title = agent
            .as_ref()
            .map(|a| a.title.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| agent.as_ref().map(|a| a.kind_label.clone()))
            .unwrap_or_default();
        let project = agent
            .as_ref()
            .map(|a| a.project.name.clone())
            .unwrap_or_default();
        let label = agent
            .as_ref()
            .map(|a| a.kind_label.clone())
            .unwrap_or_default();
        let preview = self
            .live
            .results
            .get(agent_key)
            .filter(|result| result.outcome.needs_attention())
            .and_then(|result| result.summary.clone().or_else(|| result.reason.clone()))
            .or_else(|| {
                self.host.watch_frame(agent_key).and_then(|data| {
                    data.frame
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            })
            .map(|text| clip(&text, PUSH_PREVIEW_MAX_CHARS))
            .unwrap_or_default();
        let loc_key = match kind {
            "done" => "PUSH_FOLLOW_DONE",
            "incomplete" => "PUSH_FOLLOW_INCOMPLETE",
            _ => "PUSH_FOLLOW_FAILED",
        };
        let result_at = self
            .live
            .results
            .get(agent_key)
            .map(|result| result.at as i64)
            .filter(|at| *at > 0 && *at <= now)
            .unwrap_or(now);
        let plain = PushPlain {
            k: format!("follow.{kind}"),
            rid: Some(agent_key.to_string()),
            mac: self.mac_name(),
            agent: label,
            project: project.clone(),
            session: None,
            text: if preview.is_empty() {
                clip(&title, PUSH_PREVIEW_MAX_CHARS)
            } else {
                preview.clone()
            },
            ts: result_at,
        };
        for phone_id in followers {
            if kind == "incomplete"
                && !self
                    .store
                    .state
                    .phone(&phone_id)
                    .is_some_and(|phone| phone.lifecycle_results)
            {
                continue;
            }
            if self.is_looking(&phone_id) {
                self.send(
                    &phone_id,
                    &FollowEvent {
                        agent_key: agent_key.to_string(),
                        kind: kind.to_string(),
                        title: title.clone(),
                        project: project.clone(),
                        preview: preview.clone(),
                    },
                );
                self.deliver_follow(&phone_id, agent_key);
            } else if self.can_push(&phone_id, now) {
                // A newer result replaces an older one that was not pushed yet.
                self.live.follow_notices.retain(|notice| {
                    !(notice.phone_id == phone_id && notice.agent_key == agent_key)
                });
                self.live.follow_notices.push(FollowNotice {
                    phone_id,
                    agent_key: agent_key.to_string(),
                    loc_key,
                    plain: plain.clone(),
                    at: result_at,
                });
            }
            // A phone that cannot be pushed to stays a follower: nothing could reach it.
        }
        // With the Mac already idle for long enough (or no waiting configured) the push goes
        // out right here.
        self.flush_follow_notices(now);
    }

    /// The result of `agent_key` reached the phone, or does not need to any more: the follow was
    /// for that one result, so the Agent leaves the phone's follow set.
    fn deliver_follow(&mut self, phone_id: &str, agent_key: &str) {
        let Some(phone) = self.store.state.phone_mut(phone_id) else {
            return;
        };
        let before = phone.follow_set.len();
        phone.follow_set.retain(|key| key != agent_key);
        phone.follow_turns.remove(agent_key);
        if phone.follow_set.len() == before {
            return;
        }
        self.store.save_state();
        self.revs.follow += 1;
        self.send_follow_state(phone_id, None);
        if self.is_live(phone_id) {
            // The `followed` flag of the Agent changed.
            self.send_agents_delta(phone_id);
        }
    }

    /// Pushes the results a phone has not looked at once the Mac has been idle for the configured
    /// time (`docs/specs/ios-follow-narrowing.md`), the way questions are
    /// (`flush_due_notifications`). A result is dropped, and its follow ended, when the phone
    /// looks at the app first or when the Mac saw input after the result: its owner is back and
    /// has seen it. An unknown idle time counts as long enough. Cheap while nothing waits: the
    /// system clock is read only for a waiting result.
    pub(super) fn flush_follow_notices(&mut self, now: i64) {
        if self.live.follow_notices.is_empty() {
            return;
        }
        let after = self.host.alert_after_secs();
        let idle = if after > 0 {
            self.host.mac_idle_secs()
        } else {
            None
        };
        let mut waiting = Vec::new();
        let mut dealt = Vec::new();
        for notice in std::mem::take(&mut self.live.follow_notices) {
            let following = self
                .store
                .state
                .phone(&notice.phone_id)
                .is_some_and(|phone| phone.follow_set.contains(&notice.agent_key));
            if !following {
                // The phone is gone, or the session ended and left the follow set.
                continue;
            }
            let target = (notice.phone_id.clone(), notice.agent_key.clone());
            if self.is_looking(&notice.phone_id) {
                // It can see the Agent in the list.
                dealt.push(target);
                continue;
            }
            if let Some(idle) = idle {
                // The idle time is whole seconds rounded down: leave a second of margin before
                // concluding that there was input after the result.
                if idle.saturating_mul(1000).saturating_add(1000) <= (now - notice.at).max(0) as u64
                {
                    dealt.push(target);
                    continue;
                }
                if idle < after {
                    waiting.push(notice);
                    continue;
                }
            }
            if !self.can_push(&notice.phone_id, now) {
                // The push registration vanished while the result waited: nothing can reach the
                // phone, so it stays a follower.
                continue;
            }
            let mut plain = notice.plain.clone();
            plain.ts = now;
            self.spawn_push(
                &notice.phone_id,
                "alert",
                notice.loc_key,
                &format!("fw-{}", notice.agent_key),
                &plain,
                now,
            );
            dealt.push(target);
        }
        self.live.follow_notices.extend(waiting);
        for (phone_id, agent_key) in dealt {
            self.deliver_follow(&phone_id, &agent_key);
        }
    }
}
