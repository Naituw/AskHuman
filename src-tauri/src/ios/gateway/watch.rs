//! Live Agent views for phones that have the app open (plan §3.4, §4.7).
//!
//! The daemon computes a frame for a session; the gateway asks for it every two seconds per open
//! view and only sends it when its signature changed. Views end when the phone leaves the
//! foreground (see `Actor::on_presence`), when the session ends, or when the phone closes them.

use serde_json::Value;

use super::Actor;
use crate::ios::wire::{
    WatchClosed, WatchFrame, MAX_LIVE_WATCH_PER_PHONE, WATCH_FRAME_MAX_BYTES, WATCH_MIN_INTERVAL_MS,
};

/// One open view of one Agent on one phone.
pub(crate) struct WatchRt {
    pub agent_key: String,
    /// Number of the last frame sent.
    pub seq: u64,
    pub last_sig: String,
    /// When the host was asked last (frames are polled, not pushed).
    pub last_polled_ms: i64,
}

/// Shrinks a frame until it fits the size budget: the assistant text first, then the oldest
/// steps, then the todo list.
pub(super) fn fit_frame(mut frame: Value) -> Value {
    for _ in 0..24 {
        let size = serde_json::to_vec(&frame).map(|b| b.len()).unwrap_or(0);
        if size <= WATCH_FRAME_MAX_BYTES {
            break;
        }
        let Some(object) = frame.as_object_mut() else {
            break;
        };
        let text_len = object
            .get("text")
            .and_then(Value::as_str)
            .map_or(0, |t| t.chars().count());
        if text_len > 400 {
            let cut = text_len * 3 / 4;
            let text = object
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Keep the end of the text: the latest words are the interesting ones.
            let kept: String = text.chars().skip(text_len - cut).collect();
            object.insert("text".into(), Value::String(format!("…{kept}")));
            continue;
        }
        let mut shrunk = false;
        for field in ["steps", "todos"] {
            if let Some(items) = object.get_mut(field).and_then(Value::as_array_mut) {
                if !items.is_empty() {
                    let drop = (items.len() / 2).max(1);
                    items.drain(..drop);
                    shrunk = true;
                    break;
                }
            }
        }
        if !shrunk {
            object.insert("text".into(), Value::String(String::new()));
        }
    }
    frame
}

impl Actor {
    /// Views that currently stream to a phone in the foreground (keeps an idle daemon alive).
    pub(super) fn live_watch_total(&self) -> usize {
        self.rt
            .values()
            .filter(|rt| rt.online && rt.foreground)
            .map(|rt| rt.watches.len())
            .sum()
    }

    pub(super) fn on_watch_open(&mut self, phone_id: &str, agent_key: &str) {
        if self.known_agent(agent_key).is_none() {
            self.send(
                phone_id,
                &WatchClosed {
                    agent_key: agent_key.to_string(),
                    reason: "notFound".into(),
                },
            );
            return;
        }
        let Some(rt) = self.rt.get_mut(phone_id) else {
            return;
        };
        if let Some(existing) = rt.watches.iter_mut().find(|w| w.agent_key == agent_key) {
            // Opened again (the phone lost its state): start over with a fresh frame.
            existing.last_sig.clear();
            existing.last_polled_ms = 0;
        } else if rt.watches.len() >= MAX_LIVE_WATCH_PER_PHONE {
            self.send(
                phone_id,
                &WatchClosed {
                    agent_key: agent_key.to_string(),
                    reason: "limit".into(),
                },
            );
            return;
        } else {
            rt.watches.push(WatchRt {
                agent_key: agent_key.to_string(),
                seq: 0,
                last_sig: String::new(),
                last_polled_ms: 0,
            });
        }
        self.refresh_status();
    }

    pub(super) fn on_watch_close(&mut self, phone_id: &str, agent_key: &str) {
        if let Some(rt) = self.rt.get_mut(phone_id) {
            rt.watches.retain(|w| w.agent_key != agent_key);
        }
        self.refresh_status();
    }

    pub(super) fn tick_watch(&mut self, now: i64) {
        if self.live_watch_total() == 0 {
            return;
        }
        for phone_id in self.live_phones() {
            let due: Vec<String> = self
                .rt
                .get(&phone_id)
                .map(|rt| {
                    rt.watches
                        .iter()
                        .filter(|w| now - w.last_polled_ms >= WATCH_MIN_INTERVAL_MS as i64)
                        .map(|w| w.agent_key.clone())
                        .collect()
                })
                .unwrap_or_default();
            for key in due {
                self.watch_step(&phone_id, &key, now);
            }
        }
    }

    fn watch_step(&mut self, phone_id: &str, key: &str, now: i64) {
        let data = self.host.watch_frame(key);
        let failed = self.live.failed.contains(key);
        let Some(rt) = self.rt.get_mut(phone_id) else {
            return;
        };
        let Some(watch) = rt.watches.iter_mut().find(|w| w.agent_key == key) else {
            return;
        };
        watch.last_polled_ms = now;
        let Some(data) = data else {
            rt.watches.retain(|w| w.agent_key != key);
            self.send(
                phone_id,
                &WatchClosed {
                    agent_key: key.to_string(),
                    reason: "ended".into(),
                },
            );
            self.refresh_status();
            return;
        };
        let mut frame_to_send = None;
        if data.sig != watch.last_sig {
            watch.seq += 1;
            watch.last_sig = data.sig.clone();
            frame_to_send = Some((watch.seq, data.frame.clone()));
        }
        if data.ended {
            rt.watches.retain(|w| w.agent_key != key);
        }
        if let Some((seq, mut frame)) = frame_to_send {
            if let Some(object) = frame.as_object_mut() {
                object.insert("failed".into(), Value::Bool(failed));
            }
            self.send(
                phone_id,
                &WatchFrame {
                    agent_key: key.to_string(),
                    seq,
                    frame: fit_frame(frame),
                },
            );
        }
        if data.ended {
            self.send(
                phone_id,
                &WatchClosed {
                    agent_key: key.to_string(),
                    reason: "ended".into(),
                },
            );
            self.refresh_status();
        }
    }
}
