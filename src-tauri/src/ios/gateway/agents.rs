//! The Agents stream, Follow and its notifications (plan §3.4, §4.6).
//!
//! Every phone has its own `agents` stream (revision and last-sent state per phone) because
//! `followed` differs between phones. The agent list itself comes from the host about once a
//! second, only while somebody could look at it or follows an Agent.

use std::collections::{HashMap, HashSet};

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
/// A question nobody on iOS can answer is announced once it has been open this long.
const WAITING_DELAY_MS: i64 = 2_000;
/// Sessions the phones can see at most: the active ones first, then the latest ended ones.
const MAX_AGENTS: usize = 60;
const MAX_ENDED_AGENTS: usize = 10;

/// What the gateway remembers about Agents between ticks.
#[derive(Default)]
pub(crate) struct Live {
    /// Latest list from the host (per-phone fields left at their defaults).
    pub agents: Vec<AgentView>,
    pub agents_at: i64,
    /// Sessions whose last turn failed (cleared by the next turn).
    pub failed: HashSet<String>,
    /// Session → when "done" is announced.
    pub done_at: HashMap<String, i64>,
    /// Session → (request id, first seen) of a question that is not on iOS.
    pub waiting_since: HashMap<String, (String, i64)>,
    /// Session → request id that was announced already.
    pub waiting_notified: HashMap<String, String>,
    /// New tasks waiting for their Agent session to show up.
    pub task_waits: Vec<TaskWait>,
    /// `task.create` messages whose terminal is still being started.
    pub task_inflight: HashSet<String>,
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
                agent.last_turn_failed = self.live.failed.contains(&agent.key);
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
        self.tick_waiting(now);
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
            .filter(|p| p.follow_set.iter().any(|key| key == agent_key))
            .map(|p| p.phone_id.clone())
            .collect()
    }

    // ------------------------------------------------------------ events

    pub(super) fn on_agent_event(&mut self, info: AgentEventInfo) {
        let now = self.now();
        let key = info.agent_key;
        match info.event {
            AgentEventKind::TurnStart => {
                self.live.done_at.remove(&key);
                self.live.failed.remove(&key);
            }
            AgentEventKind::Activity => {
                self.live.done_at.remove(&key);
            }
            AgentEventKind::SessionEnd => {
                self.live.done_at.remove(&key);
                self.live.failed.remove(&key);
                self.live.waiting_since.remove(&key);
                self.live.waiting_notified.remove(&key);
            }
            AgentEventKind::TurnEnd => {
                self.live.done_at.remove(&key);
                if info.failed {
                    self.live.failed.insert(key.clone());
                    self.announce(&key, "failed", now);
                } else if !self.followers(&key).is_empty() {
                    self.live.done_at.insert(key, now + DONE_DEBOUNCE_MS);
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

    /// A followed Agent waits for an answer that was not offered to the phones as a question.
    fn tick_waiting(&mut self, now: i64) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut fresh: Vec<(String, String)> = Vec::new();
        let mut fire: Vec<(String, String)> = Vec::new();
        for agent in &self.live.agents {
            let Some(rid) = agent.waiting_request_id.clone() else {
                continue;
            };
            if agent.state == "ended"
                || self.requests.find(&rid).is_some()
                || self.followers(&agent.key).is_empty()
            {
                continue;
            }
            seen.insert(agent.key.clone());
            match self.live.waiting_since.get(&agent.key) {
                Some((known, first)) if *known == rid => {
                    let announced = self.live.waiting_notified.get(&agent.key) == Some(&rid);
                    if !announced && now - first >= WAITING_DELAY_MS {
                        fire.push((agent.key.clone(), rid));
                    }
                }
                _ => fresh.push((agent.key.clone(), rid)),
            }
        }
        self.live.waiting_since.retain(|key, _| seen.contains(key));
        self.live
            .waiting_notified
            .retain(|key, _| seen.contains(key));
        for (key, rid) in fresh {
            self.live.waiting_since.insert(key, (rid, now));
        }
        for (key, rid) in fire {
            self.live.waiting_notified.insert(key.clone(), rid);
            self.announce(&key, "waiting", now);
        }
    }

    /// Tells every phone that follows `agent_key`: over the live link when the app is open,
    /// otherwise as a push.
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
        let preview = if kind == "waiting" {
            agent.as_ref().and_then(|a| a.waiting_preview.clone())
        } else {
            None
        }
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
            "waiting" => "PUSH_FOLLOW_WAITING",
            _ => "PUSH_FOLLOW_FAILED",
        };
        for phone_id in followers {
            if self.is_live(&phone_id) {
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
            } else {
                let plain = PushPlain {
                    k: format!("follow.{kind}"),
                    rid: Some(agent_key.to_string()),
                    mac: self.mac_name(),
                    agent: label.clone(),
                    project: project.clone(),
                    text: if preview.is_empty() {
                        clip(&title, PUSH_PREVIEW_MAX_CHARS)
                    } else {
                        preview.clone()
                    },
                    ts: now,
                };
                self.spawn_push(
                    &phone_id,
                    "alert",
                    loc_key,
                    &format!("fw-{agent_key}"),
                    &plain,
                    now,
                );
            }
        }
    }
}
