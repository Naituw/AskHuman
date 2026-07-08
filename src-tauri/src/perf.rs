//! Popup-launch performance instrumentation (opt-in via `ASKHUMAN_PERF`).
//!
//! When enabled, each milestone across the four execution environments (CLI / daemon / GUI helper
//! / frontend) appends one TSV line to `~/.askhuman/perf.log`:
//!
//! ```text
//! <epoch_ms>\t<perf_id>\t<stage>\t<pid>
//! ```
//!
//! All lines for one invocation share the CLI-generated `perf_id`, so a harness
//! (`scripts/perf-popup.mjs`) can stitch a single timeline together and compute per-segment deltas.
//!
//! Gating is by **`perf_id` being non-empty**, not by reading `ASKHUMAN_PERF` at the write site —
//! the daemon is long-lived and was started without the env var, so it relies on the per-request
//! `perf_id` propagated through `TaskRequest`. The helper receives the id via `ASKHUMAN_PERF_ID`
//! (set by the daemon when spawning). Disabled by default → zero file IO, zero log noise.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Process-start timestamp (epoch ms), captured as early as possible in `main` so `cli.start`
/// reflects the true process birth rather than the moment we discover perf is enabled.
static START_MS: AtomicU64 = AtomicU64::new(0);

/// Capture the process start time. Cheap (one clock read); called unconditionally from `main`.
pub fn record_start() {
    START_MS.store(now_ms() as u64, Ordering::Relaxed);
}

/// Process start timestamp in epoch ms (0 if `record_start` was never called).
pub fn start_ms() -> u128 {
    START_MS.load(Ordering::Relaxed) as u128
}

/// Whether `ASKHUMAN_PERF` is truthy — used only by the CLI to decide whether to mint a `perf_id`.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| env_truthy("ASKHUMAN_PERF"))
}

/// Runtime perf context for the **warm popup** (方案6): a prewarmed helper is spawned without any
/// perf env, so when it is later adopted for a request it learns the request's `perf_id` +
/// autodismiss from the injected `ShowPayload` and stores them here. `effective_id` / `autodismiss`
/// fall back to this when the env vars are absent. Single-use process → set once.
fn runtime_cell() -> &'static std::sync::Mutex<(String, bool)> {
    static RUNTIME: OnceLock<std::sync::Mutex<(String, bool)>> = OnceLock::new();
    RUNTIME.get_or_init(|| std::sync::Mutex::new((String::new(), false)))
}

/// Set the runtime perf context (warm popup adoption). Idempotent / last-write-wins.
pub fn set_runtime(perf_id: &str, autodismiss: bool) {
    if let Ok(mut g) = runtime_cell().lock() {
        *g = (perf_id.to_string(), autodismiss);
    }
}

/// Whether the harness asked the popup to auto-cancel right after first paint (test-only).
/// Honors the env var (cold helper) or the runtime context (warm helper, injected via `ShowPayload`).
pub fn autodismiss() -> bool {
    if env_truthy("ASKHUMAN_PERF_AUTODISMISS") {
        return true;
    }
    runtime_cell().lock().map(|g| g.1).unwrap_or(false)
}

/// Correlation id carried via `ASKHUMAN_PERF_ID` (daemon sets it on the spawned cold helper).
pub fn env_id() -> String {
    std::env::var("ASKHUMAN_PERF_ID").unwrap_or_default()
}

/// Effective correlation id for marks: the env id (cold helper / daemon-per-request) if present,
/// else the runtime id (warm helper, set on adoption). Empty → perf off → marks are no-ops.
pub fn effective_id() -> String {
    let env = env_id();
    if !env.is_empty() {
        return env;
    }
    runtime_cell()
        .lock()
        .map(|g| g.0.clone())
        .unwrap_or_default()
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0"
        })
        .unwrap_or(false)
}

/// Current wall-clock in epoch milliseconds (consistent across same-machine processes).
pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Record a milestone for `perf_id` at the current time.
pub fn mark(perf_id: &str, stage: &str) {
    mark_at(perf_id, stage, now_ms());
}

/// Record a milestone for the effective correlation id (helper-side marks; honors warm runtime id).
pub fn mark_env(stage: &str) {
    mark(&effective_id(), stage);
}

/// Record the harness-provided spawn timestamp (`ASKHUMAN_PERF_SPAWN_TS`, epoch ms) under
/// `perf_id`, if present. Lets the harness measure a true end-to-end that includes OS process
/// creation + binary load (everything before `main` runs), which `cli.start` cannot see.
pub fn mark_spawn(perf_id: &str) {
    if perf_id.is_empty() {
        return;
    }
    if let Ok(v) = std::env::var("ASKHUMAN_PERF_SPAWN_TS") {
        if let Ok(ts) = v.trim().parse::<u128>() {
            mark_at(perf_id, "spawn", ts);
        }
    }
}

/// Record a milestone with an explicit timestamp. No-op when `perf_id` is empty (perf off).
/// The frontend passes its own `Date.now()` so timings reflect the page, not the IPC round trip.
pub fn mark_at(perf_id: &str, stage: &str, ts_ms: u128) {
    if perf_id.is_empty() {
        return;
    }
    let line = format!("{ts_ms}\t{perf_id}\t{stage}\t{}\n", std::process::id());
    // Best-effort append; O_APPEND keeps small concurrent cross-process writes from interleaving.
    let path = crate::paths::config_dir().join("perf.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}
