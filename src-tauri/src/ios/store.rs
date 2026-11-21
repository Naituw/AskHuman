//! Persistent state of the iOS gateway under `~/.askhuman/ios/` (plan §4.1, §3.5).
//!
//! - `state.json`: Mac id, paired phones, follow sets, push handles, traffic counters (0600).
//! - `journal.json`: idempotency journal of state-changing phone messages (0600).
//! - Pairing keys live in the OS keychain (`ios.pairing.<phoneId>`); when the keychain is
//!   unavailable they fall back to `keys.json` (0600, plaintext) and the settings page says so.
//!
//! Nothing in here logs or prints secrets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::crypto::{self, PairKeys};
use super::wire::{JOURNAL_MAX_AGE_DAYS, JOURNAL_MAX_ENTRIES, KID_PAIR};

// ---------------------------------------------------------------- atomic private files

/// Writes `bytes` to `path` atomically with owner-only permissions (0600 file, 0700 directory).
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

// ---------------------------------------------------------------- state.json

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PhoneState {
    PendingConfirm,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushReg {
    pub handle: String,
    pub env: String,
    pub expires_at: i64,
    /// `ok`, `gone` or `error` (relay verdict of the last push).
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneRecord {
    pub phone_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_name: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub last_seen_at: i64,
    pub state: PhoneState,
    #[serde(default)]
    pub follow_set: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lifecycle_results: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub follow_turns: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activated_controls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<PushReg>,
}

impl PhoneRecord {
    pub fn display_name(&self) -> &str {
        self.alias_name
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Traffic {
    /// `YYYY-MM` the counters belong to.
    pub month: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub msgs_in: u64,
    pub msgs_out: u64,
}

/// What one kind of message added up to: how many, and how many bytes of envelope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindCount {
    pub msgs: u64,
    pub bytes: u64,
}

/// The gateway's own account of where its traffic goes this month, by direction and message type
/// (docs/specs/ios-traffic-reduction.md T6): `out:agents.delta`, `in:hello`, `in:presence`, … A diagnostic for the
/// person running this Mac. It stays on this Mac, is read with `channel ios status --json`, and never goes on the wire.
///
/// It counts the envelopes the gateway sealed and queued, and the messages it received, so it is close to, not
/// exactly, the broker ledger in [`Traffic`] (which counts what the broker confirmed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrafficBreakdown {
    /// `YYYY-MM` the counters belong to.
    pub month: String,
    pub kinds: BTreeMap<String, KindCount>,
    /// The attachment traffic told apart by what it was for (docs/specs/ios-attachment-traffic.md §6.6), under the keys of
    /// [`AssetPart`]. It is a second view of messages `kinds` already counts, not more traffic: the `chunk.*` keys add up
    /// to `kinds["out:asset.chunk"]`, every byte in exactly one of them, and the `get.*` keys count the `asset.get`
    /// messages the gateway acted on. A state file written before this was counted loads with it empty.
    pub assets: BTreeMap<String, KindCount>,
}

/// What a chunk of an attachment or a request for one was for, as `TrafficBreakdown::assets` tells them apart. The set is
/// fixed: nothing a phone sends becomes a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetPart {
    /// A chunk of a preview, the first time this phone is sent that content's preview.
    PreviewChunk,
    /// A chunk of a preview that this phone was already sent in full: the same content, fetched again.
    PreviewChunkRepeat,
    OriginalChunk,
    OriginalChunkRepeat,
    /// An `asset.get` for a file of a pending request.
    GetRequest,
    /// An `asset.get` for a picture of the history.
    GetHistory,
    /// An `asset.get` for an id that is neither (it is gone, or never was).
    GetUnresolved,
}

impl AssetPart {
    /// The key the part is counted under.
    pub fn key(self) -> &'static str {
        match self {
            Self::PreviewChunk => "chunk.preview",
            Self::PreviewChunkRepeat => "chunk.preview.repeat",
            Self::OriginalChunk => "chunk.original",
            Self::OriginalChunkRepeat => "chunk.original.repeat",
            Self::GetRequest => "get.request",
            Self::GetHistory => "get.history",
            Self::GetUnresolved => "get.unresolved",
        }
    }
}

impl TrafficBreakdown {
    /// The kind of an inbound or outbound message as it is counted: the registered type, or `other`. A phone chooses
    /// the type string it sends, so an unknown one must not become a key of its own.
    pub fn kind_of(msg_type: &str) -> &str {
        match msg_type {
            "presence" | "pair" | "rejected" => msg_type,
            known if super::wire::ALL_TYPES.contains(&known) => known,
            _ => "other",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IosState {
    pub version: u32,
    pub mac_id: String,
    pub phones: Vec<PhoneRecord>,
    pub traffic: Traffic,
    /// Where this month's traffic went, by message type; local only (see [`TrafficBreakdown`]).
    pub traffic_breakdown: TrafficBreakdown,
    pub traffic_broker: Option<String>,
    pub traffic_records: Vec<super::traffic::TrafficRecord>,
    pub traffic_synced_at: BTreeMap<String, i64>,
    pub traffic_observed_since: BTreeMap<String, i64>,
    /// `tls` or `wss`: the transport that connected last.
    pub last_transport: Option<String>,
}

impl IosState {
    pub fn phone(&self, phone_id: &str) -> Option<&PhoneRecord> {
        self.phones.iter().find(|p| p.phone_id == phone_id)
    }

    pub fn phone_mut(&mut self, phone_id: &str) -> Option<&mut PhoneRecord> {
        self.phones.iter_mut().find(|p| p.phone_id == phone_id)
    }

    pub fn active_phones(&self) -> impl Iterator<Item = &PhoneRecord> {
        self.phones.iter().filter(|p| p.state == PhoneState::Active)
    }

    pub fn has_active_phone(&self) -> bool {
        self.active_phones().next().is_some()
    }

    /// Adds `bytes` to the monthly counters, rolling over when the month changed.
    pub fn count_traffic(&mut self, month: &str, inbound: bool, bytes: usize) {
        if self.traffic.month != month {
            self.traffic = Traffic {
                month: month.to_string(),
                ..Traffic::default()
            };
        }
        if inbound {
            self.traffic.bytes_in = self.traffic.bytes_in.saturating_add(bytes as u64);
            self.traffic.msgs_in = self.traffic.msgs_in.saturating_add(1);
        } else {
            self.traffic.bytes_out = self.traffic.bytes_out.saturating_add(bytes as u64);
            self.traffic.msgs_out = self.traffic.msgs_out.saturating_add(1);
        }
    }

    /// The breakdown of `month`: this month's, or a new one when the month changed.
    fn breakdown_of(&mut self, month: &str) -> &mut TrafficBreakdown {
        if self.traffic_breakdown.month != month {
            self.traffic_breakdown = TrafficBreakdown {
                month: month.to_string(),
                ..TrafficBreakdown::default()
            };
        }
        &mut self.traffic_breakdown
    }

    /// Adds one message to the breakdown by type, rolling over when the month changed.
    pub fn count_message(&mut self, month: &str, inbound: bool, msg_type: &str, bytes: usize) {
        let key = format!(
            "{}:{}",
            if inbound { "in" } else { "out" },
            TrafficBreakdown::kind_of(msg_type)
        );
        let kind = self.breakdown_of(month).kinds.entry(key).or_default();
        kind.msgs = kind.msgs.saturating_add(1);
        kind.bytes = kind.bytes.saturating_add(bytes as u64);
    }

    /// Adds one message to the part of the attachment traffic it was for. The message is counted by type as well (with
    /// [`Self::count_message`]); this tells the same bytes apart.
    pub fn count_asset_part(&mut self, month: &str, part: AssetPart, bytes: usize) {
        let kind = self
            .breakdown_of(month)
            .assets
            .entry(part.key().to_string())
            .or_default();
        kind.msgs = kind.msgs.saturating_add(1);
        kind.bytes = kind.bytes.saturating_add(bytes as u64);
    }
}

// ---------------------------------------------------------------- pairing keys

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredKeys {
    kid: u32,
    m2p: String,
    p2m: String,
}

/// Direction keys of one pairing, ready for the envelope layer.
#[derive(Clone, PartialEq, Eq)]
pub struct PhoneKeys {
    pub kid: u32,
    pub m2p: [u8; 32],
    pub p2m: [u8; 32],
}

impl std::fmt::Debug for PhoneKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PhoneKeys(..)")
    }
}

impl PhoneKeys {
    pub fn from_pair_keys(keys: &PairKeys) -> Self {
        Self {
            kid: KID_PAIR,
            m2p: keys.m2p,
            p2m: keys.p2m,
        }
    }

    fn to_stored(&self) -> StoredKeys {
        StoredKeys {
            kid: self.kid,
            m2p: crypto::b64u(&self.m2p),
            p2m: crypto::b64u(&self.p2m),
        }
    }

    fn from_stored(stored: &StoredKeys) -> Option<Self> {
        Some(Self {
            kid: stored.kid,
            m2p: crypto::b64u_decode(&stored.m2p)?.try_into().ok()?,
            p2m: crypto::b64u_decode(&stored.p2m)?.try_into().ok()?,
        })
    }
}

/// Where the pairing keys of a phone are kept (reported on the settings page).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyBackend {
    Keychain,
    File,
}

// ---------------------------------------------------------------- journal.json

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub phone_id: String,
    pub msg_type: String,
    /// SHA-256 (hex) of the canonical request body.
    pub hash: String,
    /// `started` (side effect in flight) or `done`.
    pub status: String,
    /// Result message (type + body) to replay on a duplicate.
    pub result: Option<JournalResult>,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalResult {
    pub msg_type: String,
    pub body: Value,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct JournalFile {
    #[serde(default)]
    entries: BTreeMap<String, JournalEntry>,
}

/// Outcome of looking a `clientMsgId` up in the journal.
#[derive(Debug, PartialEq, Eq)]
pub enum JournalLookup {
    /// Never seen: execute and record.
    New,
    /// Seen with identical content: replay the first result (if it has one).
    Replay(Option<JournalResult>),
    /// Seen with different content from the same phone.
    Conflict,
    /// Seen but the side effect never finished (daemon restarted mid-flight).
    Interrupted,
}

// ---------------------------------------------------------------- the store

/// Disk-backed state. Owned by the gateway task; not shared across threads.
pub struct Store {
    dir: PathBuf,
    pub state: IosState,
    journal: JournalFile,
    /// Keys kept in memory so the hot path never touches the keychain.
    keys: BTreeMap<String, PhoneKeys>,
    use_keychain: bool,
}

impl Store {
    pub fn open(dir: PathBuf) -> Self {
        Self::open_with(dir, true)
    }

    /// `use_keychain = false` keeps keys in `keys.json` only (tests, Dev Instance, headless Linux).
    pub fn open_with(dir: PathBuf, use_keychain: bool) -> Self {
        let mut state: IosState = read_json(&dir.join("state.json")).unwrap_or_default();
        state.version = 1;
        let journal = read_json(&dir.join("journal.json")).unwrap_or_default();
        let mut store = Self {
            dir,
            state,
            journal,
            keys: BTreeMap::new(),
            use_keychain,
        };
        if store.state.mac_id.is_empty() {
            store.state.mac_id = crypto::new_id();
            store.save_state();
        }
        let ids: Vec<String> = store
            .state
            .phones
            .iter()
            .map(|p| p.phone_id.clone())
            .collect();
        for id in ids {
            if let Some(keys) = store.load_keys(&id) {
                store.keys.insert(id, keys);
            }
        }
        store
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save_state(&self) {
        if let Err(error) = self.save_state_checked() {
            eprintln!("askhuman: could not save iOS state: {error}");
        }
    }

    pub fn save_state_checked(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(|error| error.to_string())?;
        write_private(&self.dir.join("state.json"), &bytes).map_err(|error| error.to_string())
    }

    fn save_journal(&self) {
        if let Ok(bytes) = serde_json::to_vec(&self.journal) {
            let _ = write_private(&self.dir.join("journal.json"), &bytes);
        }
    }

    // ------------------------------------------------------------ keys

    fn keys_file(&self) -> PathBuf {
        self.dir.join("keys.json")
    }

    fn read_key_file(&self) -> BTreeMap<String, StoredKeys> {
        read_json(&self.keys_file()).unwrap_or_default()
    }

    fn load_keys(&self, phone_id: &str) -> Option<PhoneKeys> {
        if self.use_keychain {
            if let Ok(Some(text)) =
                crate::secrets::get(&crate::secrets::ios_pairing_account(phone_id))
            {
                if let Some(keys) = serde_json::from_str::<StoredKeys>(&text)
                    .ok()
                    .and_then(|s| PhoneKeys::from_stored(&s))
                {
                    return Some(keys);
                }
            }
        }
        PhoneKeys::from_stored(self.read_key_file().get(phone_id)?)
    }

    /// Stores the keys of one pairing; returns where they ended up.
    pub fn put_keys(&mut self, phone_id: &str, keys: PhoneKeys) -> KeyBackend {
        let stored = keys.to_stored();
        let json = serde_json::to_string(&stored).expect("keys serialize");
        self.keys.insert(phone_id.to_string(), keys);
        if self.use_keychain
            && crate::secrets::set(&crate::secrets::ios_pairing_account(phone_id), &json).is_ok()
        {
            // A stale plaintext copy (from an earlier fallback) must not outlive the keychain item.
            self.remove_key_file_entry(phone_id);
            return KeyBackend::Keychain;
        }
        let mut file = self.read_key_file();
        file.insert(phone_id.to_string(), stored);
        if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
            let _ = write_private(&self.keys_file(), &bytes);
        }
        KeyBackend::File
    }

    fn remove_key_file_entry(&self, phone_id: &str) {
        let mut file = self.read_key_file();
        if file.remove(phone_id).is_some() {
            if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
                let _ = write_private(&self.keys_file(), &bytes);
            }
        }
    }

    pub fn keys(&self, phone_id: &str) -> Option<&PhoneKeys> {
        self.keys.get(phone_id)
    }

    /// Where the keys of `phone_id` live right now (`None` when it has no keys).
    pub fn key_backend(&self, phone_id: &str) -> Option<KeyBackend> {
        self.keys.get(phone_id)?;
        if self.use_keychain
            && matches!(
                crate::secrets::get(&crate::secrets::ios_pairing_account(phone_id)),
                Ok(Some(_))
            )
        {
            Some(KeyBackend::Keychain)
        } else {
            Some(KeyBackend::File)
        }
    }

    pub fn delete_keys(&mut self, phone_id: &str) {
        self.keys.remove(phone_id);
        if self.use_keychain {
            let _ = crate::secrets::delete(&crate::secrets::ios_pairing_account(phone_id));
        }
        self.remove_key_file_entry(phone_id);
    }

    // ------------------------------------------------------------ phones

    pub fn upsert_phone(&mut self, record: PhoneRecord) {
        match self.state.phone_mut(&record.phone_id) {
            Some(existing) => *existing = record,
            None => self.state.phones.push(record),
        }
        self.save_state();
    }

    pub fn remove_phone(&mut self, phone_id: &str) -> bool {
        let before = self.state.phones.len();
        self.state.phones.retain(|p| p.phone_id != phone_id);
        let removed = self.state.phones.len() != before;
        self.delete_keys(phone_id);
        if removed {
            self.save_state();
        }
        removed
    }

    // ------------------------------------------------------------ journal

    pub fn journal_lookup(&self, client_msg_id: &str, phone_id: &str, hash: &str) -> JournalLookup {
        match self.journal.entries.get(client_msg_id) {
            None => JournalLookup::New,
            Some(e) if e.phone_id != phone_id || e.hash != hash => JournalLookup::Conflict,
            Some(e) if e.status == "started" && e.result.is_none() => JournalLookup::Interrupted,
            Some(e) => JournalLookup::Replay(e.result.clone()),
        }
    }

    /// Records a new entry (before its side effect for `started`, after for `done`).
    pub fn journal_put(&mut self, client_msg_id: &str, entry: JournalEntry) {
        self.journal
            .entries
            .insert(client_msg_id.to_string(), entry);
        self.save_journal();
    }

    pub fn journal_finish(&mut self, client_msg_id: &str, result: JournalResult, now_ms: i64) {
        if let Some(entry) = self.journal.entries.get_mut(client_msg_id) {
            entry.status = "done".into();
            entry.result = Some(result);
            entry.ts = now_ms;
            self.save_journal();
        }
    }

    /// Drops entries older than the retention window and caps the total count.
    pub fn journal_prune(&mut self, now_ms: i64) {
        let cutoff = now_ms - JOURNAL_MAX_AGE_DAYS * 24 * 3600 * 1000;
        let before = self.journal.entries.len();
        self.journal.entries.retain(|_, e| e.ts >= cutoff);
        if self.journal.entries.len() > JOURNAL_MAX_ENTRIES {
            let mut by_age: Vec<(String, i64)> = self
                .journal
                .entries
                .iter()
                .map(|(k, e)| (k.clone(), e.ts))
                .collect();
            by_age.sort_by_key(|(_, ts)| *ts);
            let excess = self.journal.entries.len() - JOURNAL_MAX_ENTRIES;
            for (key, _) in by_age.into_iter().take(excess) {
                self.journal.entries.remove(&key);
            }
        }
        if self.journal.entries.len() != before {
            self.save_journal();
        }
    }

    #[cfg(test)]
    pub fn journal_len(&self) -> usize {
        self.journal.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, state: PhoneState) -> PhoneRecord {
        PhoneRecord {
            phone_id: id.into(),
            name: "iPhone".into(),
            alias_name: None,
            created_at: 1,
            last_seen_at: 0,
            state,
            follow_set: vec![],
            lifecycle_results: false,
            follow_turns: BTreeMap::new(),
            activated_controls: vec![],
            push: None,
        }
    }

    fn keys() -> PhoneKeys {
        PhoneKeys {
            kid: KID_PAIR,
            m2p: [1; 32],
            p2m: [2; 32],
        }
    }

    #[test]
    fn mac_id_is_created_once_and_state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_with(dir.path().to_path_buf(), false);
        let mac_id = store.state.mac_id.clone();
        assert!(crypto::is_valid_id(&mac_id));
        store.upsert_phone(record("phone1", PhoneState::Active));
        store.state.last_transport = Some("wss".into());
        store.save_state();
        drop(store);
        let reopened = Store::open_with(dir.path().to_path_buf(), false);
        assert_eq!(reopened.state.mac_id, mac_id);
        assert_eq!(reopened.state.phones.len(), 1);
        assert_eq!(reopened.state.last_transport.as_deref(), Some("wss"));
        assert!(reopened.state.has_active_phone());
    }

    #[test]
    fn keys_fall_back_to_a_private_file_and_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_with(dir.path().to_path_buf(), false);
        store.upsert_phone(record("phone1", PhoneState::Active));
        assert_eq!(store.put_keys("phone1", keys()), KeyBackend::File);
        assert_eq!(store.keys("phone1"), Some(&keys()));
        assert_eq!(store.key_backend("phone1"), Some(KeyBackend::File));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("keys.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(store);
        let reopened = Store::open_with(dir.path().to_path_buf(), false);
        assert_eq!(reopened.keys("phone1"), Some(&keys()));
        let mut reopened = reopened;
        assert!(reopened.remove_phone("phone1"));
        assert!(reopened.keys("phone1").is_none());
        let again = Store::open_with(dir.path().to_path_buf(), false);
        assert!(again.keys("phone1").is_none());
        assert!(again.state.phones.is_empty());
    }

    #[test]
    fn debug_output_never_contains_key_material() {
        assert_eq!(format!("{:?}", keys()), "PhoneKeys(..)");
    }

    #[test]
    fn journal_distinguishes_new_replay_conflict_and_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_with(dir.path().to_path_buf(), false);
        assert_eq!(store.journal_lookup("c1", "p1", "h1"), JournalLookup::New);
        store.journal_put(
            "c1",
            JournalEntry {
                phone_id: "p1".into(),
                msg_type: "task.create".into(),
                hash: "h1".into(),
                status: "started".into(),
                result: None,
                ts: 100,
            },
        );
        assert_eq!(
            store.journal_lookup("c1", "p1", "h1"),
            JournalLookup::Interrupted
        );
        let result = JournalResult {
            msg_type: "task.status".into(),
            body: serde_json::json!({"stage": "launched"}),
        };
        store.journal_finish("c1", result.clone(), 200);
        assert_eq!(
            store.journal_lookup("c1", "p1", "h1"),
            JournalLookup::Replay(Some(result))
        );
        assert_eq!(
            store.journal_lookup("c1", "p1", "other"),
            JournalLookup::Conflict
        );
        assert_eq!(
            store.journal_lookup("c1", "p2", "h1"),
            JournalLookup::Conflict
        );
        // Survives a restart.
        drop(store);
        let reopened = Store::open_with(dir.path().to_path_buf(), false);
        assert!(matches!(
            reopened.journal_lookup("c1", "p1", "h1"),
            JournalLookup::Replay(Some(_))
        ));
    }

    #[test]
    fn journal_prunes_by_age_and_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_with(dir.path().to_path_buf(), false);
        let day = 24 * 3600 * 1000;
        let now = 100 * day;
        let entry = |ts| JournalEntry {
            phone_id: "p".into(),
            msg_type: "t".into(),
            hash: "h".into(),
            status: "done".into(),
            result: None,
            ts,
        };
        store.journal_put("old", entry(now - 31 * day));
        store.journal_put("fresh", entry(now - day));
        store.journal_prune(now);
        assert_eq!(store.journal_len(), 1);
        for i in 0..(JOURNAL_MAX_ENTRIES + 10) {
            store
                .journal
                .entries
                .insert(format!("k{i}"), entry(now - 10_000 + i as i64));
        }
        store.journal_prune(now);
        assert_eq!(store.journal_len(), JOURNAL_MAX_ENTRIES);
        // The oldest were evicted first.
        assert!(!store.journal.entries.contains_key("k0"));
        assert!(store
            .journal
            .entries
            .contains_key(&format!("k{}", JOURNAL_MAX_ENTRIES + 9)));
    }

    #[test]
    fn traffic_rolls_over_with_the_month() {
        let mut state = IosState::default();
        state.count_traffic("2026-10", true, 100);
        state.count_traffic("2026-10", false, 40);
        assert_eq!((state.traffic.bytes_in, state.traffic.bytes_out), (100, 40));
        assert_eq!((state.traffic.msgs_in, state.traffic.msgs_out), (1, 1));
        state.count_traffic("2026-11", true, 5);
        assert_eq!(state.traffic.month, "2026-11");
        assert_eq!((state.traffic.bytes_in, state.traffic.bytes_out), (5, 0));
    }

    #[test]
    fn the_breakdown_counts_by_direction_and_type_and_rolls_over_with_the_month() {
        let mut state = IosState::default();
        state.count_message("2026-10", false, "agents.delta", 300);
        state.count_message("2026-10", false, "agents.delta", 200);
        state.count_message("2026-10", true, "hello", 150);
        let kinds = &state.traffic_breakdown.kinds;
        assert_eq!(state.traffic_breakdown.month, "2026-10");
        assert_eq!(
            kinds["out:agents.delta"],
            KindCount {
                msgs: 2,
                bytes: 500
            }
        );
        assert_eq!(
            kinds["in:hello"],
            KindCount {
                msgs: 1,
                bytes: 150
            }
        );
        assert_eq!(kinds.len(), 2, "one key per direction and type");

        state.count_message("2026-11", false, "agents.delta", 7);
        assert_eq!(state.traffic_breakdown.month, "2026-11");
        assert_eq!(
            state.traffic_breakdown.kinds["out:agents.delta"],
            KindCount { msgs: 1, bytes: 7 }
        );
        assert_eq!(
            state.traffic_breakdown.kinds.len(),
            1,
            "last month's counts are gone"
        );
    }

    #[test]
    fn a_type_a_phone_makes_up_never_becomes_a_key_of_its_own() {
        let mut state = IosState::default();
        for index in 0..50 {
            state.count_message("2026-11", true, &format!("made.up.{index}"), 10);
        }
        state.count_message("2026-11", true, "presence", 40);
        state.count_message("2026-11", true, "pair", 40);
        state.count_message("2026-11", false, "rejected", 40);
        state.count_message("2026-11", true, "watch.open", 10);
        let kinds = &state.traffic_breakdown.kinds;
        assert_eq!(
            kinds["in:other"],
            KindCount {
                msgs: 50,
                bytes: 500
            }
        );
        assert!(kinds.contains_key("in:presence"));
        assert!(kinds.contains_key("in:pair"));
        assert!(kinds.contains_key("out:rejected"));
        assert!(kinds.contains_key("in:watch.open"));
        assert_eq!(
            kinds.len(),
            5,
            "fifty invented types made {} keys",
            kinds.len()
        );
    }

    #[test]
    fn a_state_file_from_before_the_breakdown_loads_with_an_empty_one() {
        let state: IosState = serde_json::from_str(r#"{"version":1,"macId":"m"}"#).unwrap();
        assert_eq!(state.traffic_breakdown, TrafficBreakdown::default());
        let again: IosState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(again, state);
    }

    #[test]
    fn the_asset_parts_are_counted_apart_from_the_types_and_roll_over_with_the_month() {
        let mut state = IosState::default();
        state.count_message("2026-10", false, "asset.chunk", 700);
        state.count_asset_part("2026-10", AssetPart::PreviewChunk, 300);
        state.count_asset_part("2026-10", AssetPart::OriginalChunk, 400);
        state.count_asset_part("2026-10", AssetPart::OriginalChunk, 100);
        let breakdown = &state.traffic_breakdown;
        assert_eq!(
            breakdown.kinds["out:asset.chunk"],
            KindCount {
                msgs: 1,
                bytes: 700
            },
            "the type is counted as it always was"
        );
        assert_eq!(
            breakdown.assets["chunk.original"],
            KindCount {
                msgs: 2,
                bytes: 500
            }
        );
        assert_eq!(
            breakdown.assets["chunk.preview"],
            KindCount {
                msgs: 1,
                bytes: 300
            }
        );

        state.count_asset_part("2026-11", AssetPart::GetHistory, 90);
        assert_eq!(state.traffic_breakdown.month, "2026-11");
        assert_eq!(state.traffic_breakdown.assets.len(), 1);
        assert!(
            state.traffic_breakdown.kinds.is_empty(),
            "a new month starts both views over"
        );
    }

    #[test]
    fn every_asset_part_has_a_key_of_its_own() {
        let parts = [
            AssetPart::PreviewChunk,
            AssetPart::PreviewChunkRepeat,
            AssetPart::OriginalChunk,
            AssetPart::OriginalChunkRepeat,
            AssetPart::GetRequest,
            AssetPart::GetHistory,
            AssetPart::GetUnresolved,
        ];
        let keys: std::collections::BTreeSet<_> = parts.iter().map(|part| part.key()).collect();
        assert_eq!(keys.len(), parts.len());
    }

    #[test]
    fn a_state_file_from_before_the_asset_parts_loads_with_none() {
        let state: IosState = serde_json::from_str(
            r#"{"version":1,"macId":"m","trafficBreakdown":{"month":"2026-11","kinds":{"out:asset.chunk":{"msgs":3,"bytes":9}}}}"#,
        )
        .unwrap();
        assert!(state.traffic_breakdown.assets.is_empty());
        assert_eq!(state.traffic_breakdown.kinds["out:asset.chunk"].bytes, 9);
    }
}
