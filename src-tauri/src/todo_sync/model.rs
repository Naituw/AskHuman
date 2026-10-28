//! Data model: the synced entity, the cloud payload and the sidecar `state.json` (plan §3.1–§3.2).
//!
//! One synced todo = one [`Entity`]: a project identity plus a handful of last-writer-wins registers
//! (`text`, `ord`, `st`, `att`), each stamped with its own HLC. The same JSON is the cloud record's
//! encrypted `payload`; unknown fields are kept and written back, so a newer build's extra fields
//! survive a round trip through an older one.

use super::hlc::ClockState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// `state.json` schema version. A file from a newer schema is never touched.
pub const SCHEMA: u32 = 1;
/// Cloud payload version written into every record.
pub const PAYLOAD_VERSION: u32 = 1;
/// Record types.
pub const TYPE_TODO: &str = "Todo";
pub const TYPE_DEVICE: &str = "Device";
/// A todo whose text exceeds this is not synced (CloudKit's record limit is 1 MiB; spec §5.3).
pub const TEXT_MAX: usize = 256 * 1024;

pub type Hlc = String;

/// A last-writer-wins register: the value and the HLC of the write that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reg<T> {
    pub v: T,
    pub h: Hlc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum St {
    Pending,
    Done,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Created {
    pub ms: u64,
    pub dev: String,
    /// Agent family that added it (`cursor`, `claude`, …), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ak: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DoneInfo {
    pub ms: u64,
    pub dev: String,
}

/// Attachment hint: attachments themselves never sync (T6), only "N files on device X".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Att {
    pub n: u32,
    #[serde(default)]
    pub names: Vec<String>,
    pub dev: String,
    pub h: Hlc,
}

fn payload_version() -> u32 {
    PAYLOAD_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    #[serde(default = "payload_version")]
    pub v: u32,
    /// Project identity (normalized remote); fixed for the entity's lifetime.
    pub p: String,
    pub text: Reg<String>,
    /// Order key (`order_key`); empty for entities that have never been queued.
    pub ord: Reg<String>,
    pub st: Reg<St>,
    pub c: Created,
    /// Execution info; travels with `st`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dn: Option<DoneInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub att: Option<Att>,
    /// Fields written by a newer build: preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// An entity plus its local-only bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rec {
    pub e: Entity,
    /// Has local changes the cloud has not acknowledged.
    #[serde(default)]
    pub dirty: bool,
    /// Server change tag of the version we last saw (opaque).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// What a local directory's `todos.json` slice looked like when the sync core last wrote or read it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snap {
    /// Pending ids in queue order.
    pub pending: Vec<String>,
    /// History ids, oldest first.
    pub history: Vec<String>,
    /// Text hash per pending id (change detection without keeping the text twice).
    #[serde(default)]
    pub hash: BTreeMap<String, String>,
}

/// Local project key → project identity (`None` = local only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub p: Option<String>,
    #[serde(default)]
    pub cfg_mtime_ms: u64,
    #[serde(default)]
    pub checked_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    /// The iCloud account changed (or its hash differs from the one this state was built on).
    AccountChanged,
    /// The cloud zone is gone (cleared from another Mac); we never silently re-upload.
    ZoneMissing,
    NotSignedIn,
    Quota,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paused {
    pub reason: PauseReason,
    pub since_ms: u64,
}

/// What this device knows about a device (itself included) from `Device` records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    #[serde(default)]
    pub last_sync_ms: u64,
    #[serde(default)]
    pub projects: Vec<String>,
}

/// The `Device` record payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePayload {
    #[serde(default = "payload_version")]
    pub v: u32,
    pub name: String,
    #[serde(default)]
    pub last_sync_ms: u64,
    #[serde(default)]
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub schema: u32,
    pub device_id: String,
    pub device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_hash: Option<String>,
    #[serde(default)]
    pub enabled_at_ms: u64,
    /// Opaque change token from the last fully-applied fetch page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_token: Option<String>,
    /// The zone and subscription exist (an `ensure` succeeded).
    #[serde(default)]
    pub zone_ready: bool,
    #[serde(default)]
    pub last_pull_ok_ms: u64,
    #[serde(default)]
    pub last_push_ok_ms: u64,
    /// When this device last wrote its own `Device` record.
    #[serde(default)]
    pub last_device_push_ms: u64,
    /// Server change tag of this device's own `Device` record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_rec_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<Paused>,
    #[serde(default)]
    pub clock: ClockState,
    #[serde(default)]
    pub entities: BTreeMap<String, Rec>,
    #[serde(default)]
    pub snapshot: BTreeMap<String, Snap>,
    #[serde(default)]
    pub links: BTreeMap<String, Link>,
    #[serde(default)]
    pub devices: BTreeMap<String, DeviceInfo>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl State {
    /// A fresh state for a device that has never synced.
    pub fn new(device_id: String, device_name: String, now_ms: u64) -> State {
        State {
            schema: SCHEMA,
            device_id,
            device_name,
            account_hash: None,
            enabled_at_ms: now_ms,
            zone_token: None,
            zone_ready: false,
            last_pull_ok_ms: 0,
            last_push_ok_ms: 0,
            last_device_push_ms: 0,
            device_rec_tag: None,
            paused: None,
            clock: ClockState::default(),
            entities: BTreeMap::new(),
            snapshot: BTreeMap::new(),
            links: BTreeMap::new(),
            devices: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }

    /// The tag embedded in this device's timestamps.
    pub fn tag(&self) -> String {
        super::hlc::device_tag(&self.device_id)
    }

    /// Issue a timestamp for a new local event.
    pub fn stamp(&mut self, now_ms: u64) -> Hlc {
        let tag = self.tag();
        self.clock.issue(now_ms, &tag)
    }

    /// Project identity linked to a local key (`None`: unlinked or local-only).
    pub fn project_of(&self, key: &str) -> Option<&str> {
        self.links.get(key).and_then(|l| l.p.as_deref())
    }

    /// Entity ids per project identity, in id order.
    pub fn by_project(&self) -> BTreeMap<&str, Vec<&str>> {
        let mut out: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (id, rec) in &self.entities {
            out.entry(rec.e.p.as_str()).or_default().push(id.as_str());
        }
        out
    }
}

/// Encode an entity as the cloud `payload` bytes.
pub fn encode_entity(e: &Entity) -> Vec<u8> {
    serde_json::to_vec(e).unwrap_or_default()
}

/// Decode a cloud `payload`; `None` when it is not a valid entity (such a record is skipped).
pub fn decode_entity(bytes: &[u8]) -> Option<Entity> {
    serde_json::from_slice(bytes).ok()
}

/// Cheap, stable fingerprint of a todo's text (first 8 bytes of SHA-256, hex).
pub fn text_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity() -> Entity {
        Entity {
            v: 1,
            p: "github.com/a/b".into(),
            text: Reg {
                v: "hello".into(),
                h: "0000000000001-0000-aaaaaaaa".into(),
            },
            ord: Reg {
                v: "V".into(),
                h: "0000000000001-0000-aaaaaaaa".into(),
            },
            st: Reg {
                v: St::Pending,
                h: "0000000000001-0000-aaaaaaaa".into(),
            },
            c: Created {
                ms: 5,
                dev: "dev-a".into(),
                ak: Some("cursor".into()),
            },
            dn: None,
            att: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn payload_roundtrip_keeps_unknown_fields() {
        let json = serde_json::json!({
            "v": 1, "p": "github.com/a/b",
            "text": {"v": "hi", "h": "0000000000002-0000-bbbbbbbb"},
            "ord": {"v": "V", "h": "0000000000002-0000-bbbbbbbb"},
            "st": {"v": "done", "h": "0000000000003-0000-bbbbbbbb"},
            "c": {"ms": 1, "dev": "dev-b"},
            "dn": {"ms": 9, "dev": "dev-b"},
            "labels": ["future", "field"],
            "color": {"v": "red", "h": "0000000000004-0000-bbbbbbbb"}
        });
        let e = decode_entity(json.to_string().as_bytes()).unwrap();
        assert_eq!(e.st.v, St::Done);
        assert_eq!(e.extra.len(), 2);
        let back: Value = serde_json::from_slice(&encode_entity(&e)).unwrap();
        assert_eq!(
            back, json,
            "an older build must write a newer build's fields back unchanged"
        );
        assert!(decode_entity(b"{\"not\":\"an entity\"}").is_none());
        assert!(decode_entity(b"\xff\xfe").is_none());
    }

    #[test]
    fn state_roundtrip_and_defaults() {
        let mut s = State::new("device-1".into(), "Studio".into(), 10);
        s.entities.insert(
            "t1".into(),
            Rec {
                e: entity(),
                dirty: true,
                tag: Some("tag1".into()),
            },
        );
        s.links.insert(
            "/repo".into(),
            Link {
                p: Some("github.com/a/b".into()),
                cfg_mtime_ms: 3,
                checked_ms: 4,
            },
        );
        s.paused = Some(Paused {
            reason: PauseReason::ZoneMissing,
            since_ms: 7,
        });
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains("\"deviceId\""),
            "top level is camelCase: {json}"
        );
        let back: State = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        // A minimal file from an older writer still loads.
        let minimal: State = serde_json::from_str(
            r#"{"schema":1,"deviceId":"d","deviceName":"n","futureKey":{"x":1}}"#,
        )
        .unwrap();
        assert!(minimal.entities.is_empty());
        assert_eq!(minimal.extra["futureKey"]["x"], 1);
    }

    #[test]
    fn text_hash_is_stable_and_discriminating() {
        assert_eq!(text_hash("a"), text_hash("a"));
        assert_ne!(text_hash("a"), text_hash("b"));
        assert_eq!(text_hash("").len(), 16);
    }
}
