//! One sync tick (plan §4.1): local phase, then pull, then push.
//!
//! Everything here is built so that a crash — or a network failure — at any point leaves state that the
//! next tick simply continues from:
//!
//! * `state.json` is saved **before** `todos.json` is written, and its snapshot only after, so the
//!   diff (which is idempotent against the entities, see `diff.rs`) never mistakes the half-finished
//!   materialization for a user edit.
//! * The change token is advanced only after the fetched changes are merged, saved and materialized;
//!   an interrupted pull is simply fetched again (merging is idempotent).
//! * A record is marked pushed only after the server acknowledged it; a lost acknowledgement comes
//!   back as a conflict whose server version equals ours.
//! * `todos.lock` is held only while `todos.json` is read/diffed/materialized/written, never across a
//!   network call.

use super::identity::IdentityResolver;
use super::model::*;
use super::transport::*;
use super::{apply, diff, links, merge, store};
use crate::todos;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Records per `save` call.
pub const SAVE_CHUNK: usize = 200;
/// Conflict-merge-retry rounds per tick.
pub const SAVE_RETRIES: usize = 5;
/// How often this device refreshes its own `Device` record.
pub const DEVICE_PUSH_INTERVAL_MS: u64 = 60 * 60 * 1000;

/// Everything a tick needs to know about the local installation.
#[derive(Debug, Clone)]
pub struct Env {
    pub todos_path: PathBuf,
    pub todos_lock: PathBuf,
    /// The sidecar directory (`state/todo-sync`).
    pub dir: PathBuf,
    /// `todo_history_limit` of this device.
    pub history_limit: usize,
    pub device_name: String,
}

#[derive(Debug)]
pub enum TickError {
    /// `state.json` belongs to a newer build; leave everything alone.
    NewerSchema(u32),
    Todos(todos::ReadError),
    Persist(String),
    Net(NetError),
    /// A failpoint fired (tests only).
    Crash(&'static str),
}

impl fmt::Display for TickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TickError::NewerSchema(v) => {
                write!(f, "sync state is from a newer version (schema {v})")
            }
            TickError::Todos(e) => write!(f, "{e}"),
            TickError::Persist(e) => write!(f, "could not save: {e}"),
            TickError::Net(e) => write!(f, "{e}"),
            TickError::Crash(at) => write!(f, "crashed at {at}"),
        }
    }
}

impl From<NetError> for TickError {
    fn from(e: NetError) -> Self {
        TickError::Net(e)
    }
}

/// Crash injection for tests: the n-th failpoint reached returns [`TickError::Crash`], once.
/// Production code uses [`Failpoints::off`], whose `hit` is a single relaxed load.
pub struct Failpoints {
    countdown: AtomicI64,
    seen: AtomicU64,
}

impl Failpoints {
    pub const fn off() -> Failpoints {
        Failpoints {
            countdown: AtomicI64::new(-1),
            seen: AtomicU64::new(0),
        }
    }

    /// Crash at the `n`-th (0-based) failpoint.
    pub fn crash_at(n: u32) -> Failpoints {
        Failpoints {
            countdown: AtomicI64::new(i64::from(n)),
            seen: AtomicU64::new(0),
        }
    }

    /// How many failpoints were passed (for enumerating crash points).
    pub fn seen(&self) -> u64 {
        self.seen.load(Ordering::Relaxed)
    }

    pub fn hit(&self, name: &'static str) -> Result<(), TickError> {
        self.seen.fetch_add(1, Ordering::Relaxed);
        if self.countdown.load(Ordering::Relaxed) < 0 {
            return Ok(());
        }
        if self.countdown.fetch_sub(1, Ordering::Relaxed) == 0 {
            return Err(TickError::Crash(name));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    /// Sync was never enabled on this device (no `state.json`): nothing was touched.
    pub disabled: bool,
    /// Another tick holds the lock.
    pub busy: bool,
    /// A complete pull finished.
    pub pulled: bool,
    pub fetched: usize,
    /// Entities changed by data from the cloud.
    pub merged: usize,
    /// Records the server acknowledged.
    pub pushed: usize,
    pub conflicts: usize,
    /// Entities still waiting to be uploaded at the end.
    pub unacked: usize,
    /// `todos.json` was rewritten.
    pub file_changed: bool,
    pub paused: Option<PauseReason>,
    pub offline: bool,
    pub retry_after_ms: Option<u64>,
    /// The deadline passed before the tick finished (state is consistent; the next tick continues).
    pub timed_out: bool,
    /// `state.json` was unreadable and has been rebuilt.
    pub rebuilt: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Opts {
    /// Stop starting new network steps after this instant.
    pub deadline: Option<Instant>,
}

/// What the pull and push steps need besides the transport and the state.
struct Ctx<'a> {
    env: &'a Env,
    resolver: &'a dyn IdentityResolver,
    extra_keys: &'a [String],
    now_ms: u64,
    fp: &'a Failpoints,
    opts: &'a Opts,
}

fn io_err(what: &str, e: std::io::Error) -> TickError {
    TickError::Persist(format!("{what}: {e}"))
}

fn persist(env: &Env, state: &State) -> Result<(), TickError> {
    store::save(&env.dir, state).map_err(|e| io_err("state.json", e))
}

// ---- enabling ---------------------------------------------------------------

/// Create this device's sidecar state (idempotent). Nothing is created before this is called.
pub fn enable(env: &Env, now_ms: u64) -> Result<(), TickError> {
    let _lock = store::lock_blocking(&env.dir).map_err(|e| io_err("todo-sync.lock", e))?;
    match store::load(&env.dir).map_err(|e| io_err("state.json", e))? {
        store::Loaded::Newer(v) => Err(TickError::NewerSchema(v)),
        store::Loaded::State(_) => Ok(()),
        store::Loaded::Absent | store::Loaded::Corrupt { .. } => {
            let state = State::new(
                uuid::Uuid::new_v4().to_string(),
                env.device_name.clone(),
                now_ms,
            );
            persist(env, &state)
        }
    }
}

/// The saved state, if sync was ever enabled (read-only; for status displays).
pub fn peek(env: &Env) -> Result<Option<State>, TickError> {
    match store::load(&env.dir).map_err(|e| io_err("state.json", e))? {
        store::Loaded::Absent => Ok(None),
        store::Loaded::Newer(v) => Err(TickError::NewerSchema(v)),
        store::Loaded::State(s) => Ok(Some(*s)),
        store::Loaded::Corrupt { .. } => Ok(None),
    }
}

fn load_for_tick(
    env: &Env,
    now_ms: u64,
    report: &mut TickReport,
) -> Result<Option<State>, TickError> {
    match store::load(&env.dir).map_err(|e| io_err("state.json", e))? {
        store::Loaded::Absent => Ok(None),
        store::Loaded::Newer(v) => Err(TickError::NewerSchema(v)),
        store::Loaded::State(s) => Ok(Some(*s)),
        store::Loaded::Corrupt { .. } => {
            // The sidecar is only a cache of convergence state: rebuild it. Every local entry is seen
            // "for the first time" again, and the cloud's entities are adopted by id, so nothing is
            // duplicated; at worst a deletion made while the state was lost comes back.
            report.rebuilt = true;
            let state = State::new(
                uuid::Uuid::new_v4().to_string(),
                env.device_name.clone(),
                now_ms,
            );
            persist(env, &state)?;
            Ok(Some(state))
        }
    }
}

// ---- local phase ------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    pub file_written: bool,
    pub touched: usize,
}

/// Link directories, diff `todos.json` into entities, materialize entities back, write what changed.
/// Holds `todos.lock` throughout; never touches the network.
pub fn reconcile(
    env: &Env,
    resolver: &dyn IdentityResolver,
    extra_keys: &[String],
    state: &mut State,
    now_ms: u64,
    fp: &Failpoints,
) -> Result<Reconciled, TickError> {
    let session = todos::Session::open(&env.todos_path, &env.todos_lock);
    let mut file = session.read().map_err(TickError::Todos)?;

    let mut keys: BTreeSet<String> = BTreeSet::new();
    keys.extend(file.projects.keys().cloned());
    keys.extend(file.history.keys().cloned());
    keys.extend(extra_keys.iter().cloned());
    keys.extend(state.links.keys().cloned());
    let linked = links::refresh(state, keys.iter().map(String::as_str), resolver, now_ms);

    let before = file.clone().pruned();
    let report = diff::diff(&mut file, state, now_ms);
    let m = apply::materialize(&file, state, env.history_limit);

    let file_changed = m.file != before;
    let snaps_changed = m
        .snaps
        .iter()
        .any(|(k, snap)| state.snapshot.get(k) != Some(snap));

    // Entities (and links) first, with the OLD snapshot: after a crash anywhere below, the diff sees
    // a file that either still matches the old snapshot or already matches the entities.
    if linked.touched || report.touched > 0 {
        fp.hit("reconcile.before-state-save")?;
        persist(env, state)?;
    }
    if file_changed {
        fp.hit("reconcile.before-file-write")?;
        if !session.write(m.file) {
            return Err(TickError::Persist("todos.json".into()));
        }
        fp.hit("reconcile.after-file-write")?;
    }
    drop(session);
    if snaps_changed {
        for (k, snap) in m.snaps {
            state.snapshot.insert(k, snap);
        }
        fp.hit("reconcile.before-snapshot-save")?;
        persist(env, state)?;
    }
    if !m.cleanup.is_empty() {
        crate::todo_attachments::cleanup_attachments(&m.cleanup);
    }
    Ok(Reconciled {
        file_written: file_changed,
        touched: report.touched,
    })
}

/// The local phase on its own (read paths: CLI, popup, Stop card). Never waits for the network and
/// never blocks for long: if a tick is running it is skipped, the running tick materializes anyway.
pub fn local_only(
    env: &Env,
    resolver: &dyn IdentityResolver,
    extra_keys: &[String],
    now_ms: u64,
    fp: &Failpoints,
    wait: Duration,
) -> Result<TickReport, TickError> {
    let mut report = TickReport::default();
    if !store::exists(&env.dir) {
        report.disabled = true;
        return Ok(report);
    }
    let Some(_lock) =
        store::lock_timeout(&env.dir, wait).map_err(|e| io_err("todo-sync.lock", e))?
    else {
        report.busy = true;
        return Ok(report);
    };
    let Some(mut state) = load_for_tick(env, now_ms, &mut report)? else {
        report.disabled = true;
        return Ok(report);
    };
    // Before the first successful pull the local entries must not become entities yet: their order
    // keys are minted against what the cloud already holds (spec §5.6).
    if state.last_pull_ok_ms == 0 {
        return Ok(report);
    }
    let r = reconcile(env, resolver, extra_keys, &mut state, now_ms, fp)?;
    report.file_changed = r.file_written;
    report.paused = state.paused.as_ref().map(|p| p.reason);
    Ok(report)
}

// ---- full tick --------------------------------------------------------------

fn pause(env: &Env, state: &mut State, reason: PauseReason, now_ms: u64) -> Result<(), TickError> {
    if state.paused.as_ref().map(|p| p.reason) != Some(reason) {
        state.paused = Some(Paused {
            reason,
            since_ms: now_ms,
        });
        persist(env, state)?;
    }
    Ok(())
}

fn out_of_time(opts: &Opts) -> bool {
    opts.deadline.is_some_and(|d| Instant::now() >= d)
}

/// Turn a network failure into a report (soft) or an error (hard). Pausing errors pause.
fn net_failure(
    env: &Env,
    state: &mut State,
    report: &mut TickReport,
    e: NetError,
    now_ms: u64,
) -> Result<(), TickError> {
    match e {
        NetError::Offline => {
            report.offline = true;
            Ok(())
        }
        NetError::RateLimited { retry_after_ms } => {
            report.retry_after_ms = Some(retry_after_ms);
            Ok(())
        }
        NetError::NotAuthenticated => {
            pause(env, state, PauseReason::NotSignedIn, now_ms)?;
            report.paused = Some(PauseReason::NotSignedIn);
            Ok(())
        }
        NetError::ZoneNotFound => {
            state.zone_ready = false;
            pause(env, state, PauseReason::ZoneMissing, now_ms)?;
            report.paused = Some(PauseReason::ZoneMissing);
            Ok(())
        }
        NetError::Quota => {
            pause(env, state, PauseReason::Quota, now_ms)?;
            report.paused = Some(PauseReason::Quota);
            Ok(())
        }
        e @ (NetError::TokenExpired | NetError::Other(_)) => Err(TickError::Net(e)),
    }
}

/// Fold one page of cloud changes into the state.
fn absorb(state: &mut State, page: &FetchPage, now_ms: u64, report: &mut TickReport) {
    for c in &page.changes {
        report.fetched += 1;
        match c.rtype.as_str() {
            TYPE_TODO => {
                // A record that is not an entity (garbage, or from an incompatible future) is skipped.
                if let Some(e) = decode_entity(&c.payload) {
                    if merge::merge_remote(state, &c.name, e, &c.tag, now_ms) {
                        report.merged += 1;
                    }
                }
            }
            TYPE_DEVICE => {
                if let Ok(d) = serde_json::from_slice::<DevicePayload>(&c.payload) {
                    state.devices.insert(
                        c.name.clone(),
                        DeviceInfo {
                            name: d.name,
                            last_sync_ms: d.last_sync_ms,
                            projects: d.projects,
                        },
                    );
                }
            }
            _ => {}
        }
    }
    // A record the server no longer has is not a deletion by the user: keep the local copy and
    // upload it again (spec §5.7: the cloud is never allowed to silently delete local todos).
    for d in &page.deletions {
        if d.rtype == TYPE_TODO {
            if let Some(rec) = state.entities.get_mut(&d.name) {
                rec.dirty = true;
                rec.tag = None;
            }
        }
    }
}

/// Fetch every page, merge, save, materialize, and only then advance the token.
fn pull<T: Transport>(
    ctx: &Ctx<'_>,
    transport: &mut T,
    state: &mut State,
    report: &mut TickReport,
) -> Result<bool, TickError> {
    let mut token = state.zone_token.clone();
    let mut restarted = false;
    let last_token: String = loop {
        if out_of_time(ctx.opts) {
            report.timed_out = true;
            persist(ctx.env, state)?;
            return Ok(false);
        }
        let page = match transport.fetch(token.as_deref()) {
            Ok(page) => page,
            Err(NetError::TokenExpired) if !restarted => {
                // Too old to continue from: start over (merging is idempotent).
                restarted = true;
                token = None;
                state.zone_token = None;
                continue;
            }
            Err(e) => {
                // Keep what was merged so far (the token did not move, so nothing is skipped).
                persist(ctx.env, state)?;
                net_failure(ctx.env, state, report, e, ctx.now_ms)?;
                return Ok(false);
            }
        };
        ctx.fp.hit("net.after-fetch")?;
        absorb(state, &page, ctx.now_ms, report);
        if !page.more_coming {
            break page.token;
        }
        token = Some(page.token);
    };
    ctx.fp.hit("net.before-merge-save")?;
    persist(ctx.env, state)?;
    reconcile_into(ctx, state, report)?;
    ctx.fp.hit("net.before-token-advance")?;
    state.zone_token = Some(last_token);
    state.last_pull_ok_ms = ctx.now_ms;
    persist(ctx.env, state)?;
    report.pulled = true;
    Ok(true)
}

fn reconcile_into(
    ctx: &Ctx<'_>,
    state: &mut State,
    report: &mut TickReport,
) -> Result<(), TickError> {
    let r = reconcile(
        ctx.env,
        ctx.resolver,
        ctx.extra_keys,
        state,
        ctx.now_ms,
        ctx.fp,
    )?;
    report.file_changed |= r.file_written;
    Ok(())
}

fn merge_conflict(state: &mut State, name: &str, payload: &[u8], tag: &str, now_ms: u64) {
    match decode_entity(payload) {
        Some(e) => {
            merge::merge_remote(state, name, e, tag, now_ms);
        }
        None => {
            // Unusable server copy: overwrite it with ours on the retry.
            if let Some(rec) = state.entities.get_mut(name) {
                rec.tag = Some(tag.to_string());
                rec.dirty = true;
            }
        }
    }
}

/// Upload dirty entities; on conflict merge the server's version and retry.
fn push<T: Transport>(
    ctx: &Ctx<'_>,
    transport: &mut T,
    state: &mut State,
    report: &mut TickReport,
) -> Result<(), TickError> {
    for _round in 0..SAVE_RETRIES {
        let dirty: Vec<String> = state
            .entities
            .iter()
            .filter(|(_, r)| r.dirty)
            .map(|(id, _)| id.clone())
            .collect();
        if dirty.is_empty() {
            break;
        }
        let mut conflicts = 0usize;
        for chunk in dirty.chunks(SAVE_CHUNK) {
            if out_of_time(ctx.opts) {
                report.timed_out = true;
                return Ok(());
            }
            let sent: HashMap<String, Vec<u8>> = chunk
                .iter()
                .map(|id| (id.clone(), encode_entity(&state.entities[id].e)))
                .collect();
            let records: Vec<OutRecord> = chunk
                .iter()
                .map(|id| OutRecord {
                    name: id.clone(),
                    rtype: TYPE_TODO.to_string(),
                    payload: sent[id].clone(),
                    tag: state.entities[id].tag.clone(),
                })
                .collect();
            ctx.fp.hit("net.before-save")?;
            let results = match transport.save(&records) {
                Ok(r) => r,
                Err(e) => {
                    net_failure(ctx.env, state, report, e, ctx.now_ms)?;
                    return Ok(());
                }
            };
            // The server has applied the batch; we do not know it yet.
            ctx.fp.hit("net.after-save")?;
            for res in results {
                match res {
                    SaveResult::Saved { name, tag } => {
                        if let Some(rec) = state.entities.get_mut(&name) {
                            rec.tag = Some(tag);
                            // Edited again since we encoded it? Then it still has something to send.
                            if sent.get(&name).is_some_and(|s| *s == encode_entity(&rec.e)) {
                                rec.dirty = false;
                            }
                            report.pushed += 1;
                        }
                    }
                    SaveResult::Conflict {
                        name,
                        server_payload,
                        server_tag,
                    } => {
                        conflicts += 1;
                        report.conflicts += 1;
                        merge_conflict(state, &name, &server_payload, &server_tag, ctx.now_ms);
                    }
                    SaveResult::Error { code, .. } => {
                        if code == "quotaExceeded" {
                            pause(ctx.env, state, PauseReason::Quota, ctx.now_ms)?;
                            report.paused = Some(PauseReason::Quota);
                        }
                    }
                }
            }
            ctx.fp.hit("net.before-ack-save")?;
            persist(ctx.env, state)?;
            if state.paused.is_some() {
                return Ok(());
            }
        }
        if conflicts > 0 {
            // The server's versions may have changed what the local file should show.
            reconcile_into(ctx, state, report)?;
        } else {
            break;
        }
    }
    report.unacked = state.entities.values().filter(|r| r.dirty).count();
    if report.unacked == 0 {
        state.last_push_ok_ms = ctx.now_ms;
        persist(ctx.env, state)?;
    }
    Ok(())
}

fn push_device<T: Transport>(
    env: &Env,
    transport: &mut T,
    state: &mut State,
    now_ms: u64,
) -> Result<(), TickError> {
    if state.last_device_push_ms != 0
        && now_ms.saturating_sub(state.last_device_push_ms) < DEVICE_PUSH_INTERVAL_MS
    {
        return Ok(());
    }
    let projects: Vec<String> = state
        .links
        .values()
        .filter_map(|l| l.p.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let payload = DevicePayload {
        v: PAYLOAD_VERSION,
        name: state.device_name.clone(),
        last_sync_ms: now_ms,
        projects: projects.clone(),
    };
    let record = OutRecord {
        name: state.device_id.clone(),
        rtype: TYPE_DEVICE.to_string(),
        payload: serde_json::to_vec(&payload).unwrap_or_default(),
        tag: state.device_rec_tag.clone(),
    };
    // Best effort: a failure here never fails the tick.
    match transport.save(std::slice::from_ref(&record)) {
        Ok(results) => {
            for res in results {
                match res {
                    SaveResult::Saved { tag, .. } => {
                        state.device_rec_tag = Some(tag);
                        state.last_device_push_ms = now_ms;
                    }
                    SaveResult::Conflict { server_tag, .. } => {
                        state.device_rec_tag = Some(server_tag);
                    }
                    SaveResult::Error { .. } => {}
                }
            }
            state.devices.insert(
                state.device_id.clone(),
                DeviceInfo {
                    name: state.device_name.clone(),
                    last_sync_ms: now_ms,
                    projects,
                },
            );
            persist(env, state)
        }
        Err(_) => Ok(()),
    }
}

/// A complete tick: local phase, account checks, pull, push.
pub fn full<T: Transport>(
    env: &Env,
    resolver: &dyn IdentityResolver,
    transport: &mut T,
    extra_keys: &[String],
    now_ms: u64,
    fp: &Failpoints,
    opts: &Opts,
) -> Result<TickReport, TickError> {
    let mut report = TickReport::default();
    // Never create the sidecar (not even its lock file) on a device that did not enable sync.
    if !store::exists(&env.dir) {
        report.disabled = true;
        return Ok(report);
    }
    let ctx = Ctx {
        env,
        resolver,
        extra_keys,
        now_ms,
        fp,
        opts,
    };
    let _lock = store::lock_blocking(&env.dir).map_err(|e| io_err("todo-sync.lock", e))?;
    let Some(mut state) = load_for_tick(env, now_ms, &mut report)? else {
        report.disabled = true;
        return Ok(report);
    };

    // The very first sync pulls before the first local phase (see `local_only`); later ticks put
    // local edits first so they are never delayed by the network.
    if state.last_pull_ok_ms != 0 {
        reconcile_into(&ctx, &mut state, &mut report)?;
    }

    if let Some(p) = &state.paused {
        if p.reason != PauseReason::NotSignedIn {
            report.paused = Some(p.reason);
            return Ok(report);
        }
    }
    if out_of_time(opts) {
        report.timed_out = true;
        return Ok(report);
    }

    // ---- account ----
    let account = match transport.account() {
        Ok(a) => a,
        Err(e) => {
            net_failure(env, &mut state, &mut report, e, now_ms)?;
            return Ok(report);
        }
    };
    match account.status {
        AccountStatus::Available => {}
        AccountStatus::NoAccount | AccountStatus::Restricted => {
            pause(env, &mut state, PauseReason::NotSignedIn, now_ms)?;
            report.paused = Some(PauseReason::NotSignedIn);
            return Ok(report);
        }
        AccountStatus::Unknown => {
            report.offline = true;
            return Ok(report);
        }
    }
    if state.paused.is_some() {
        // Only NotSignedIn gets here: signing in again resumes by itself.
        state.paused = None;
        persist(env, &state)?;
    }
    match (&state.account_hash, &account.hash) {
        (None, hash) => {
            state.account_hash = hash.clone();
            persist(env, &state)?;
        }
        (Some(old), Some(new)) if old != new => {
            pause(env, &mut state, PauseReason::AccountChanged, now_ms)?;
            report.paused = Some(PauseReason::AccountChanged);
            return Ok(report);
        }
        _ => {}
    }

    // ---- zone ----
    if !state.zone_ready {
        if let Err(e) = transport.ensure() {
            net_failure(env, &mut state, &mut report, e, now_ms)?;
            return Ok(report);
        }
        state.zone_ready = true;
        persist(env, &state)?;
    }

    // ---- pull, push ----
    let pulled = pull(&ctx, transport, &mut state, &mut report)?;
    if !pulled || report.paused.is_some() || report.timed_out {
        return Ok(report);
    }
    push(&ctx, transport, &mut state, &mut report)?;
    if report.paused.is_none() && !report.timed_out && !report.offline {
        push_device(env, transport, &mut state, now_ms)?;
    }
    Ok(report)
}

// ---- user decisions after a pause --------------------------------------------

/// "Use this Mac's copy as the cloud's content" (after the zone vanished or the account changed):
/// every entity is uploaded again into a fresh zone.
pub fn restart_from_local(env: &Env) -> Result<(), TickError> {
    if !store::exists(&env.dir) {
        return Ok(());
    }
    let _lock = store::lock_blocking(&env.dir).map_err(|e| io_err("todo-sync.lock", e))?;
    let mut report = TickReport::default();
    let Some(mut state) = load_for_tick(env, 0, &mut report)? else {
        return Ok(());
    };
    state.paused = None;
    state.zone_ready = false;
    state.zone_token = None;
    state.account_hash = None;
    state.last_pull_ok_ms = 0;
    state.device_rec_tag = None;
    state.last_device_push_ms = 0;
    for rec in state.entities.values_mut() {
        rec.dirty = true;
        rec.tag = None;
    }
    persist(env, &state)
}

/// "Try again" after the user made room in iCloud: lifts a [`PauseReason::Quota`] pause (or a stale
/// [`PauseReason::NotSignedIn`] one). The other pauses need a real decision — [`restart_from_local`]
/// or stopping sync. Returns whether a pause was lifted.
pub fn resume(env: &Env) -> Result<bool, TickError> {
    if !store::exists(&env.dir) {
        return Ok(false);
    }
    let _lock = store::lock_blocking(&env.dir).map_err(|e| io_err("todo-sync.lock", e))?;
    let mut report = TickReport::default();
    let Some(mut state) = load_for_tick(env, 0, &mut report)? else {
        return Ok(false);
    };
    match state.paused.as_ref().map(|p| p.reason) {
        Some(PauseReason::Quota | PauseReason::NotSignedIn) => {
            state.paused = None;
            persist(env, &state)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// "Clear iCloud data": delete the zone. This device pauses too; the user then chooses between
/// [`restart_from_local`] and stopping sync.
pub fn clear_cloud<T: Transport>(
    env: &Env,
    transport: &mut T,
    now_ms: u64,
) -> Result<(), TickError> {
    if !store::exists(&env.dir) {
        return Ok(());
    }
    let _lock = store::lock_blocking(&env.dir).map_err(|e| io_err("todo-sync.lock", e))?;
    let mut report = TickReport::default();
    let Some(mut state) = load_for_tick(env, now_ms, &mut report)? else {
        return Ok(());
    };
    transport.delete_zone()?;
    state.zone_ready = false;
    state.zone_token = None;
    state.last_device_push_ms = 0;
    state.device_rec_tag = None;
    state.paused = Some(Paused {
        reason: PauseReason::ZoneMissing,
        since_ms: now_ms,
    });
    persist(env, &state)
}
