//! Exact-session native metadata. Workspace identity is independent from a tool's transient cwd.
use super::{title, AgentKind};
use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const LIMIT: u64 = 1024 * 1024;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub workspace: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub context: Option<ContextUsage>,
    pub reported_at: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub used_tokens: u64,
    pub window_tokens: u64,
    pub remaining_percent: f64,
    pub estimated: bool,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    runtime_report: bool,
    kind: AgentKind,
    session_id: String,
    info: SessionInfo,
}
fn cache_path(kind: AgentKind, sid: &str) -> PathBuf {
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{sid}", kind.as_str()).as_bytes())
    );
    paths::config_dir()
        .join("session-info")
        .join(format!("{key}.json"))
}
fn saved(kind: AgentKind, sid: &str) -> Option<Saved> {
    read_json(&cache_path(kind, sid))
        .and_then(|v| serde_json::from_value::<Saved>(v).ok())
        .filter(|v| v.kind == kind && v.session_id == sid)
}
fn cached(kind: AgentKind, sid: &str) -> SessionInfo {
    saved(kind, sid).map(|s| s.info).unwrap_or_default()
}
fn valid_sid(sid: &str) -> bool {
    !sid.trim().is_empty()
        && sid.len() <= 512
        && !sid.contains(['/', '\\'])
        && !sid.chars().any(char::is_control)
}

fn save(kind: AgentKind, sid: &str, info: SessionInfo, runtime_report: bool) {
    if sid.trim().is_empty() || sid.len() > 512 {
        return;
    }
    let path = cache_path(kind, sid);
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let Ok(bytes) = serde_json::to_vec(&Saved {
        runtime_report,
        kind,
        session_id: sid.into(),
        info,
    }) else {
        return;
    };
    let _ = crate::integrations::hook_edit::atomic_write_private(&path, &bytes);
}
fn directory(raw: &str) -> Option<String> {
    let path = Path::new(raw);
    (path.is_absolute() && path.is_dir()).then(|| {
        fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned()
    })
}
fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() < 8192)
        .map(str::to_string)
}
fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_reader(fs::File::open(path).ok()?.take(LIMIT)).ok()
}
fn records(path: &Path, tail: bool) -> Vec<Value> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let start = if tail {
        file.metadata()
            .map(|m| m.len().saturating_sub(LIMIT))
            .unwrap_or(0)
    } else {
        0
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(LIMIT).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    String::from_utf8_lossy(&bytes)
        .lines()
        .skip(usize::from(start > 0))
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
fn modified(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
fn context(
    used: u64,
    window: u64,
    used_percent: Option<f64>,
    estimated: bool,
) -> Option<ContextUsage> {
    if window == 0 {
        return None;
    }
    let percent = used_percent.unwrap_or(100.0 * used as f64 / window as f64);
    if !percent.is_finite() || !(0.0..=1000.0).contains(&percent) {
        return None;
    }
    Some(ContextUsage {
        used_tokens: used,
        window_tokens: window,
        remaining_percent: (100.0 - percent).clamp(0.0, 100.0),
        estimated,
    })
}

/// Resolve a stable native workspace; never use a mutable registry cwd as an authority.
pub fn workspace(kind: AgentKind, sid: &str) -> Option<String> {
    if !valid_sid(sid) {
        return None;
    }
    let saved = cached(kind, sid);
    if let Some(path) = saved.workspace.as_deref().and_then(directory) {
        return Some(path);
    }
    if kind == AgentKind::Codex {
        let config = crate::config::AppConfig::load_without_secrets().codex_desktop;
        if let Some(workspace) =
            crate::codex_desktop::session_workspace(&config, sid).and_then(|p| directory(&p))
        {
            let info = SessionInfo {
                workspace: Some(workspace.clone()),
                ..Default::default()
            };
            save(kind, sid, info, false);
            return Some(workspace);
        }
    }
    let info = native(kind, sid);
    let workspace = info.workspace.clone();
    if workspace.is_some() {
        save(kind, sid, info, false);
    }
    workspace
}
/// Read current metadata on demand. Runtime reports remain authoritative for Pi estimates.
pub fn resolve(kind: AgentKind, sid: &str) -> SessionInfo {
    if !valid_sid(sid) {
        return SessionInfo::default();
    }
    let source = saved(kind, sid);
    let runtime_report = source.as_ref().is_some_and(|s| s.runtime_report);
    let saved = source.map(|s| s.info).unwrap_or_default();
    let mut info = native(kind, sid);
    info.workspace = info.workspace.or(saved.workspace);
    if kind == AgentKind::Pi
        && runtime_report
        && saved.model.is_some()
        && saved.reported_at >= info.reported_at
    {
        info.model = saved.model;
        info.reasoning = saved.reasoning;
        info.context = saved.context;
        info.reported_at = saved.reported_at;
    }
    info
}

/// How long [`resolve_cached`] reuses an answer, and how many sessions it keeps.
const RESOLVE_TTL: Duration = Duration::from_secs(10);
const RESOLVE_CACHE_MAX: usize = 128;

type ResolveCache = HashMap<(AgentKind, String), (Instant, SessionInfo)>;

/// [`resolve`], except that a session read less than ten seconds ago is not read again: a live
/// view asks every couple of seconds, and the model it shows changes far more slowly than that.
pub fn resolve_cached(kind: AgentKind, sid: &str) -> SessionInfo {
    static CACHE: OnceLock<Mutex<ResolveCache>> = OnceLock::new();
    resolve_cached_in(
        CACHE.get_or_init(Default::default),
        kind,
        sid,
        Instant::now(),
        resolve,
    )
}

fn resolve_cached_in(
    cache: &Mutex<ResolveCache>,
    kind: AgentKind,
    sid: &str,
    now: Instant,
    read: impl FnOnce(AgentKind, &str) -> SessionInfo,
) -> SessionInfo {
    let key = (kind, sid.to_string());
    if let Some((at, value)) = cache.lock().unwrap().get(&key) {
        if now.saturating_duration_since(*at) < RESOLVE_TTL {
            return value.clone();
        }
    }
    // The files are read without the lock: another session's answer must not wait for them.
    let value = read(kind, sid);
    let mut cache = cache.lock().unwrap();
    if cache.len() >= RESOLVE_CACHE_MAX {
        cache.clear();
    }
    cache.insert(key, (now, value.clone()));
    value
}

fn native(kind: AgentKind, sid: &str) -> SessionInfo {
    let mut info = SessionInfo::default();
    if kind == AgentKind::Cursor {
        if let Some(value) = super::cursor_vscdb::session_metadata(sid) {
            info = cursor_info(&value);
        }
        info.workspace = super::cursor_vscdb::session_workspace(sid);
        if info.workspace.is_none() {
            if let Some(path) = title::transcript_path(kind, sid) {
                if let Some(encoded) = path
                    .ancestors()
                    .find(|p| {
                        p.parent()
                            .is_some_and(|parent| parent == paths::cursor_dir().join("projects"))
                    })
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                {
                    let candidates = super::workspaces::recover_cursor_project_key(encoded);
                    if candidates.len() == 1 {
                        info.workspace = directory(&candidates[0].to_string_lossy());
                    }
                }
            }
        }
        if info.model.is_none() {
            if let Ok(entries) = fs::read_dir(paths::cursor_dir().join("chats")) {
                for entry in entries.flatten().take(512) {
                    if let Some(meta) = read_json(&entry.path().join(sid).join("meta.json")) {
                        info.model = text(&meta, "model").or_else(|| text(&meta, "modelId"));
                        info.workspace = info
                            .workspace
                            .or_else(|| text(&meta, "cwd").and_then(|p| directory(&p)));
                        break;
                    }
                }
            }
        }
        return info;
    }
    if kind == AgentKind::Grok {
        if let Some(dir) = title::grok_session_dir(sid) {
            let path = dir.join("summary.json");
            if let Some(value) = read_json(&path) {
                info.workspace = value
                    .pointer("/info/cwd")
                    .and_then(Value::as_str)
                    .and_then(directory);
                info.model = text(&value, "current_model_id");
                info.reported_at = modified(&path);
            }
        }
        return info;
    }
    let transcript = if kind == AgentKind::Codex {
        let config = crate::config::AppConfig::load_without_secrets().codex_desktop;
        let home = if !config.codex_home.trim().is_empty() {
            PathBuf::from(config.codex_home.trim())
        } else {
            std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(paths::codex_dir)
        };
        title::find_file_recursive(&home.join("sessions"), &format!("-{sid}.jsonl"), 4)
    } else {
        title::transcript_path(kind, sid)
    };
    let Some(path) = transcript else {
        if kind == AgentKind::Codex {
            info.workspace = crate::codex_desktop::session_workspace(
                &crate::config::AppConfig::load_without_secrets().codex_desktop,
                sid,
            );
        }
        return info;
    };
    let head = records(&path, false);
    info.workspace = head.iter().find_map(|value| match kind {
        AgentKind::Codex if value["type"] == "session_meta" => value
            .pointer("/payload/cwd")
            .and_then(Value::as_str)
            .and_then(directory),
        AgentKind::Claude if value["sessionId"].as_str().is_none_or(|id| id == sid) => {
            value.get("cwd").and_then(Value::as_str).and_then(directory)
        }
        AgentKind::Pi if value["type"] == "session" && value["id"] == sid => {
            value.get("cwd").and_then(Value::as_str).and_then(directory)
        }
        _ => None,
    });
    let tail = records(&path, true);
    match kind {
        AgentKind::Codex => parse_codex(&tail, &mut info),
        AgentKind::Claude => {
            for value in &tail {
                if value["type"] == "assistant" {
                    if let Some(model) = value
                        .pointer("/message/model")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty() && !s.starts_with('<'))
                    {
                        info.model = Some(model.into());
                    }
                }
            }
        }
        AgentKind::Pi => {
            for value in &tail {
                if value["type"] == "model_change" {
                    info.model = text(value, "modelId");
                }
                if value["type"] == "thinking_level_change" {
                    info.reasoning = text(value, "thinkingLevel");
                }
            }
        }
        _ => {}
    }
    info.reported_at = modified(&path);
    info
}
fn cursor_info(value: &Value) -> SessionInfo {
    let mut info = SessionInfo {
        model: value
            .pointer("/modelConfig/modelName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        ..Default::default()
    };
    if let (Some(used), Some(window)) = (
        value["contextTokensUsed"].as_u64(),
        value["contextTokenLimit"].as_u64(),
    ) {
        info.context = context(used, window, value["contextUsagePercent"].as_f64(), false);
    }
    info.reported_at = value["lastUpdatedAt"].as_u64().map(|n| n / 1000);
    info
}
fn parse_codex(values: &[Value], info: &mut SessionInfo) {
    for value in values {
        let payload = &value["payload"];
        if value["type"] == "turn_context" {
            if let Some(model) = text(payload, "model") {
                if info.model.as_deref() != Some(model.as_str()) {
                    info.context = None;
                }
                info.model = Some(model);
            }
            info.reasoning = text(payload, "effort").or_else(|| text(payload, "reasoning_effort"));
        }
        if value["type"] == "compacted" || payload["type"] == "context_compacted" {
            info.context = None;
        }
        if payload["type"] == "token_count" {
            let usage = &payload["info"];
            if let (Some(used), Some(window)) = (
                usage
                    .pointer("/last_token_usage/total_tokens")
                    .and_then(Value::as_u64),
                usage["model_context_window"].as_u64(),
            ) {
                info.context = context(used, window, None, true);
                // Codex's native user-controllable percentage excludes its 12k baseline.
                if let Some(context) = info.context.as_mut().filter(|_| window > 12_000) {
                    context.remaining_percent = (100.0
                        * window.saturating_sub(used.max(12_000)) as f64
                        / (window - 12_000) as f64)
                        .clamp(0.0, 100.0)
                        .round();
                }
            }
        }
    }
}

/// Capture only authoritative Hook workspace roots or the existing Pi extension's native report.
pub fn capture_hook(
    kind: AgentKind,
    input: &Value,
    env: &std::collections::HashMap<String, String>,
) {
    let sid = super::report::resolve_session_id(kind, env, Some(input));
    if !valid_sid(&sid) {
        return;
    }
    let runtime_report = kind == AgentKind::Pi
        && (input.get("native_context").is_some()
            || saved(kind, &sid).is_some_and(|s| s.runtime_report));
    let mut info = cached(kind, &sid);
    let explicit = input
        .get("workspace_roots")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 1)
        .and_then(|a| a[0].as_str())
        .and_then(directory)
        .or_else(|| match kind {
            AgentKind::Cursor => env.get("CURSOR_PROJECT_DIR").and_then(|s| directory(s)),
            AgentKind::Grok => env.get("GROK_WORKSPACE_ROOT").and_then(|s| directory(s)),
            AgentKind::Claude => env.get("CLAUDE_PROJECT_DIR").and_then(|s| directory(s)),
            AgentKind::Pi => input.get("cwd").and_then(Value::as_str).and_then(directory),
            _ => None,
        });
    info.workspace = explicit.or(info.workspace);
    if kind == AgentKind::Pi {
        if let Some(report) = input.get("native_context") {
            info.model = text(report, "model");
            info.reasoning = text(report, "reasoning");
            info.context = report
                .get("usage")
                .and_then(|v| {
                    Some((
                        v["tokens"].as_u64()?,
                        v["contextWindow"].as_u64()?,
                        v["percent"].as_f64(),
                    ))
                })
                .and_then(|(used, window, percent)| context(used, window, percent, true));
            info.reported_at = Some(crate::history::now_ms().max(0) as u64 / 1000);
        }
    }
    if info.workspace.is_some() || info.model.is_some() {
        save(kind, &sid, info, runtime_report);
    }
}
pub fn report_cli(kind: AgentKind) {
    let env = std::env::vars().collect();
    if super::report::should_skip(kind, &env) {
        return;
    }
    if let Ok(value) = serde_json::from_reader::<_, Value>(std::io::stdin().take(128 * 1024)) {
        capture_hook(kind, &value, &env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn codex_context_uses_latest_window_usage_not_cumulative_session_tokens() {
        let values = vec![
            json!({"type":"turn_context","payload":{"model":"model-a","effort":"high"}}),
            json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":9000000},"last_token_usage":{"total_tokens":106000},"model_context_window":200000}}}),
        ];
        let mut info = SessionInfo::default();
        parse_codex(&values, &mut info);
        assert_eq!(info.model.as_deref(), Some("model-a"));
        assert_eq!(info.reasoning.as_deref(), Some("high"));
        let context = info.context.unwrap();
        assert_eq!(context.used_tokens, 106000);
        assert_eq!(context.remaining_percent, 50.0);
    }
    #[test]
    fn compaction_and_model_switch_do_not_reuse_the_old_context_window() {
        let usage = json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":20000},"model_context_window":200000}}});
        for boundary in [
            json!({"type":"compacted"}),
            json!({"type":"turn_context","payload":{"model":"new-model"}}),
        ] {
            let mut info = SessionInfo {
                model: Some("old-model".into()),
                ..Default::default()
            };
            parse_codex(&[usage.clone(), boundary], &mut info);
            assert!(info.context.is_none());
        }
    }
    #[test]
    fn cursor_reads_native_percent_and_omits_missing_or_invalid_metrics() {
        let value = json!({"modelConfig":{"modelName":"auto"},"contextTokensUsed":25505,"contextTokenLimit":200000,"contextUsagePercent":12.7525});
        let info = cursor_info(&value);
        assert_eq!(info.model.as_deref(), Some("auto"));
        assert!((info.context.unwrap().remaining_percent - 87.2475).abs() < 0.001);
        assert!(cursor_info(&json!({"modelConfig":{"modelName":"model"}}))
            .context
            .is_none());
        assert!(context(42, 0, None, false).is_none());
        assert!(context(42, 100, Some(f64::NAN), false).is_none());
    }
    #[test]
    fn bounded_tail_drops_partial_records_and_keeps_latest_complete_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let data = format!(
            "{}\n{}\n",
            "x".repeat(LIMIT as usize + 20),
            json!({"type":"turn_context","payload":{"model":"latest"}})
        );
        fs::write(&path, data).unwrap();
        let entries = records(&path, true);
        assert_eq!(entries.len(), 1);
        let mut info = SessionInfo::default();
        parse_codex(&entries, &mut info);
        assert_eq!(info.model.as_deref(), Some("latest"));
    }
    #[test]
    fn native_session_ids_cannot_escape_metadata_directories() {
        assert!(!valid_sid("../other"));
        assert!(!valid_sid("C:\\private"));
        assert!(!valid_sid("bad\nidentity"));
        assert!(valid_sid("session-a_1"));
    }

    fn info(model: &str) -> SessionInfo {
        SessionInfo {
            model: Some(model.into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_session_read_a_moment_ago_is_not_read_again() {
        let cache = Mutex::new(ResolveCache::new());
        let start = Instant::now();
        let reads = std::cell::Cell::new(0);
        let read = |_: AgentKind, _: &str| {
            reads.set(reads.get() + 1);
            info(&format!("model-{}", reads.get()))
        };
        let first = resolve_cached_in(&cache, AgentKind::Pi, "s", start, read);
        let again = resolve_cached_in(
            &cache,
            AgentKind::Pi,
            "s",
            start + Duration::from_secs(9),
            read,
        );
        assert_eq!(first.model.as_deref(), Some("model-1"));
        assert_eq!(again.model.as_deref(), Some("model-1"));
        assert_eq!(reads.get(), 1);
        // Past the ten seconds it is read again, and the new answer is the one kept.
        let later = resolve_cached_in(
            &cache,
            AgentKind::Pi,
            "s",
            start + Duration::from_secs(10),
            read,
        );
        assert_eq!(later.model.as_deref(), Some("model-2"));
        let kept = resolve_cached_in(
            &cache,
            AgentKind::Pi,
            "s",
            start + Duration::from_secs(11),
            read,
        );
        assert_eq!(kept.model.as_deref(), Some("model-2"));
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn cached_answers_are_kept_apart_per_family_and_session() {
        let cache = Mutex::new(ResolveCache::new());
        let now = Instant::now();
        let read = |kind: AgentKind, sid: &str| info(&format!("{}-{sid}", kind.as_str()));
        let a = resolve_cached_in(&cache, AgentKind::Pi, "s1", now, read);
        let b = resolve_cached_in(&cache, AgentKind::Pi, "s2", now, read);
        let c = resolve_cached_in(&cache, AgentKind::Grok, "s1", now, read);
        assert_eq!(a.model.as_deref(), Some("pi-s1"));
        assert_eq!(b.model.as_deref(), Some("pi-s2"));
        assert_eq!(c.model.as_deref(), Some("grok-s1"));
    }

    #[test]
    fn the_answer_cache_does_not_grow_without_bound() {
        let cache = Mutex::new(ResolveCache::new());
        let now = Instant::now();
        for i in 0..(RESOLVE_CACHE_MAX * 3) {
            resolve_cached_in(&cache, AgentKind::Pi, &format!("s{i}"), now, |_, _| {
                info("m")
            });
            assert!(cache.lock().unwrap().len() <= RESOLVE_CACHE_MAX);
        }
    }
}
