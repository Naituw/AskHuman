//! Pairing state machine of the gateway (plan §3.3).
//!
//! `pair_start` → the user shows the code to the phone → `pair.request` arrives on the ticket
//! topic → the Mac shows the six-digit check code and waits for the user's decision → on allow
//! the Mac sends `pair.accept` on the phone's own topic → the phone confirms with `pair.done`.

use super::{sanitize_name, Actor, PairingView};
use crate::ios::crypto::{self, KeyPair, PairContext, PairKeys};
use crate::ios::envelope::{self, Plain, Scope};
use crate::ios::store::{PhoneKeys, PhoneRecord, PhoneState, PushReg};
use crate::ios::wire::{
    self, t, Msg, PairCode, CLOCK_SKEW_MS, KID_TICKET, PAIR_DONE_TIMEOUT_SECS, PAIR_TICKET_SECS,
    PROTO,
};

/// Result of starting a pairing, shown on the settings page (code text and QR).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairInfo {
    pub ticket_id: String,
    pub code: String,
    pub expires_at_ms: i64,
}

enum Stage {
    /// Waiting for the phone to knock.
    Waiting,
    /// The phone knocked; the user has to compare the check code and decide.
    Pending {
        phone_id: String,
        name: String,
        sas: String,
        keys: PairKeys,
    },
    /// Accepted; waiting for the phone's `pair.done`.
    Accepted { phone_id: String, deadline_ms: i64 },
}

pub(crate) struct Pairing {
    pub ticket_id: String,
    ticket_secret: [u8; 32],
    key_pair: KeyPair,
    k_t: [u8; 32],
    code: String,
    expires_ms: i64,
    stage: Stage,
}

impl Actor {
    pub(super) async fn pair_start(&mut self) -> Result<PairInfo, String> {
        if self.cfg.host.trim().is_empty() || self.cfg.username.trim().is_empty() {
            return Err(
                "iOS channel is not configured (broker host and username are required)".into(),
            );
        }
        self.pair_cancel().await;

        let ticket_id = crypto::new_id();
        let ticket_secret = crypto::random_bytes::<32>();
        let k_t = crypto::ticket_key(&ticket_secret, &ticket_id);
        let key_pair = KeyPair::generate();
        let now = self.now();
        let expires_ms = now + PAIR_TICKET_SECS * 1000;
        let code = PairCode {
            v: PROTO,
            h: self.cfg.host.clone(),
            p: self.cfg.port,
            w: self.cfg.ws_port,
            u: self.cfg.username.clone(),
            pw: self.cfg.password.clone(),
            r: self.cfg.relay_url.clone(),
            m: self.mac_id.clone(),
            mn: self.mac_name(),
            t: ticket_id.clone(),
            ts: crypto::b64u(&ticket_secret),
            ek: crypto::b64u(&key_pair.public),
            x: expires_ms / 1000,
        }
        .encode();

        if self.connected {
            let topic = wire::topic_pair(&self.user, &ticket_id);
            self.transport
                .subscribe(&topic)
                .await
                .map_err(|e| format!("cannot listen for the phone: {e}"))?;
        }
        self.pairing = Some(Pairing {
            ticket_id: ticket_id.clone(),
            ticket_secret,
            key_pair,
            k_t,
            code: code.clone(),
            expires_ms,
            stage: Stage::Waiting,
        });
        Ok(PairInfo {
            ticket_id,
            code,
            expires_at_ms: expires_ms,
        })
    }

    pub(super) async fn pair_cancel(&mut self) {
        let Some(pairing) = self.pairing.take() else {
            return;
        };
        match pairing.stage {
            Stage::Accepted { phone_id, .. } => {
                // The phone never completed: forget the half-made pairing.
                if self
                    .store
                    .state
                    .phone(&phone_id)
                    .is_some_and(|p| p.state == PhoneState::PendingConfirm)
                {
                    self.remove_phone(&phone_id);
                }
            }
            Stage::Pending { .. } => {
                self.send_on_pair_topic(
                    &pairing,
                    &wire::PairReject {
                        reason: "cancelled".into(),
                    },
                );
            }
            Stage::Waiting => {}
        }
        let topic = wire::topic_pair(&self.user, &pairing.ticket_id);
        let _ = self.transport.unsubscribe(&topic).await;
    }

    pub(super) fn pairing_view(&self) -> Option<PairingView> {
        let pairing = self.pairing.as_ref()?;
        let (stage, phone_name, sas) = match &pairing.stage {
            Stage::Waiting => ("waiting", None, None),
            Stage::Pending { name, sas, .. } => ("pending", Some(name.clone()), Some(sas.clone())),
            Stage::Accepted { phone_id, .. } => (
                "accepted",
                self.store.state.phone(phone_id).map(|p| p.name.clone()),
                None,
            ),
        };
        Some(PairingView {
            ticket_id: pairing.ticket_id.clone(),
            code: pairing.code.clone(),
            expires_at_ms: pairing.expires_ms,
            stage: stage.into(),
            phone_name,
            sas,
        })
    }

    /// A message on a ticket topic. Only a well-formed, authentic `pair.request` is acted on;
    /// everything else is dropped without a reply (a stranger learns nothing).
    pub(super) async fn on_pair_topic(&mut self, topic: &str, ticket_id: &str, payload: &[u8]) {
        let Some(pairing) = self.pairing.as_ref() else {
            return;
        };
        if pairing.ticket_id != ticket_id || !matches!(pairing.stage, Stage::Waiting) {
            return;
        }
        let Ok(env) = envelope::parse(payload) else {
            return;
        };
        if env.f == self.mac_id || env.k != KID_TICKET {
            return;
        }
        let Ok(plain) = envelope::open_plain(&pairing.k_t, Scope::Topic(topic), &env) else {
            return;
        };
        if plain.t != t::PAIR_REQUEST || (plain.ts - self.now()).abs() > CLOCK_SKEW_MS {
            return;
        }
        let Ok(request) = serde_json::from_value::<wire::PairRequest>(plain.b.clone()) else {
            return;
        };
        if request.phone_id != env.f || !crypto::is_valid_id(&request.phone_id) {
            return;
        }
        self.on_pair_request(request).await;
    }

    async fn on_pair_request(&mut self, request: wire::PairRequest) {
        let Some(pairing) = self.pairing.as_ref() else {
            return;
        };
        let reject = |this: &mut Self, reason: &str| {
            if let Some(pairing) = this.pairing.as_ref() {
                this.send_on_pair_topic(
                    pairing,
                    &wire::PairReject {
                        reason: reason.into(),
                    },
                );
            }
        };
        if request.proto != PROTO {
            reject(self, "unsupported");
            return;
        }
        let Some(phone_public) =
            crypto::b64u_decode(&request.ek).and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        else {
            reject(self, "invalid");
            return;
        };
        let Some(shared) = pairing.key_pair.diffie_hellman(&phone_public) else {
            reject(self, "invalid");
            return;
        };
        let keys = crypto::derive_pair_keys(
            &shared,
            &PairContext {
                mac_id: &self.mac_id,
                phone_id: &request.phone_id,
                ticket_id: &pairing.ticket_id,
                ticket_secret: &pairing.ticket_secret,
                mac_public: &pairing.key_pair.public,
                phone_public: &phone_public,
            },
        );
        let sas = crypto::sas_code(&keys.sas);
        let name = sanitize_name(&request.name);
        let mac_id = self.mac_id.clone();
        if let Some(pairing) = self.pairing.as_mut() {
            pairing.stage = Stage::Pending {
                phone_id: request.phone_id,
                name,
                sas,
                keys,
            };
        }
        if let Some(pairing) = self.pairing.as_ref() {
            self.send_on_pair_topic(pairing, &wire::PairPending { mac_id });
        }
    }

    /// The user compared the check codes and chose.
    pub(super) async fn pair_decision(&mut self, ticket_id: &str, allow: bool) {
        let Some(pairing) = self.pairing.as_mut() else {
            return;
        };
        if pairing.ticket_id != ticket_id {
            return;
        }
        let Stage::Pending {
            phone_id,
            name,
            keys,
            ..
        } = std::mem::replace(&mut pairing.stage, Stage::Waiting)
        else {
            return;
        };
        if !allow {
            if let Some(pairing) = self.pairing.as_ref() {
                self.send_on_pair_topic(
                    pairing,
                    &wire::PairReject {
                        reason: "rejected".into(),
                    },
                );
            }
            self.pairing = None;
            let topic = wire::topic_pair(&self.user, ticket_id);
            let _ = self.transport.unsubscribe(&topic).await;
            return;
        }

        let now = self.now();
        // Re-pairing a known phone starts from scratch: old keys, follow set and push handle go.
        self.store.remove_phone(&phone_id);
        self.store
            .put_keys(&phone_id, PhoneKeys::from_pair_keys(&keys));
        self.store.upsert_phone(PhoneRecord {
            phone_id: phone_id.clone(),
            name,
            alias_name: None,
            created_at: now,
            last_seen_at: now,
            state: PhoneState::PendingConfirm,
            follow_set: Vec::new(),
            push: None,
        });
        self.rt.insert(phone_id.clone(), Default::default());
        if let Some(pairing) = self.pairing.as_mut() {
            pairing.stage = Stage::Accepted {
                phone_id: phone_id.clone(),
                deadline_ms: now + PAIR_DONE_TIMEOUT_SECS * 1000,
            };
        }
        let accept = wire::PairAccept {
            mac_id: self.mac_id.clone(),
            mac_name: self.mac_name(),
            caps: self.host.caps(),
        };
        self.send(&phone_id, &accept);
    }

    /// `pair.done` from a phone whose pairing the user accepted.
    pub(super) async fn on_pair_done(&mut self, phone_id: &str, done: wire::PairDone) {
        let waiting_for_this_phone = matches!(
            self.pairing.as_ref().map(|p| &p.stage),
            Some(Stage::Accepted { phone_id: id, .. }) if id == phone_id
        );
        if !waiting_for_this_phone {
            return;
        }
        let name = sanitize_name(&done.name);
        if let Some(phone) = self.store.state.phone_mut(phone_id) {
            phone.state = PhoneState::Active;
            phone.name = name;
            phone.push = match (done.handle, done.env, done.expires_at) {
                (Some(handle), Some(env), Some(expires_at)) if !handle.is_empty() => {
                    Some(PushReg {
                        handle,
                        env,
                        expires_at,
                        status: "ok".into(),
                    })
                }
                _ => None,
            };
        }
        self.store.save_state();
        if let Some(pairing) = self.pairing.take() {
            let topic = wire::topic_pair(&self.user, &pairing.ticket_id);
            let _ = self.transport.unsubscribe(&topic).await;
        }
        self.refresh_status();
    }

    pub(super) async fn tick_pairing(&mut self, now: i64) {
        let Some(pairing) = self.pairing.as_ref() else {
            return;
        };
        let expired = match &pairing.stage {
            Stage::Waiting | Stage::Pending { .. } => now > pairing.expires_ms,
            Stage::Accepted { deadline_ms, .. } => now > *deadline_ms,
        };
        if expired {
            self.pair_cancel().await;
        }
    }

    fn send_on_pair_topic<M: Msg>(&self, pairing: &Pairing, body: &M) {
        let topic = wire::topic_pair(&self.user, &pairing.ticket_id);
        let plain = Plain::new(M::TYPE, body.to_value(), self.now());
        let bytes = envelope::seal_plain(
            &pairing.k_t,
            Scope::Topic(&topic),
            &self.mac_id,
            KID_TICKET,
            &plain,
        );
        self.outbox.send(topic, bytes, false);
    }
}
