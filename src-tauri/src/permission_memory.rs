//! Hook-side Codex permission memory analysis (spec `docs/specs/codex-permission-remember.md`).
//!
//! Runs inside the short-lived `__permission-hook` process. Given the raw PermissionRequest
//! stdin, it derives which memory choices the current request provably supports and the
//! matching auto-allow query. Everything here is a fallible enhancement on top of the basic
//! allow-once/deny confirmation (D24): any parse failure, rollout gap or guardian ambiguity
//! yields `Analysis::Basic` and keeps the existing popup unchanged.

use crate::confirm::ActionRole;
use crate::models::ConfirmChoice;
use crate::permission_rules::{
    MemoryQuery, MemorySave, NativeWrite, PermissionMemory, RuleKey, RuleNamespace,
};
use serde_json::Value;
use std::io::BufRead;
use std::path::{Path, PathBuf};

/// Choice action ids added by the memory layer. The daemon validates them against saves.
pub const ACTION_REMEMBER_FILES: &str = "remember_files";
pub const ACTION_REMEMBER_PROJECT: &str = "remember_project";
pub const ACTION_REMEMBER_DISK: &str = "remember_disk";
pub const ACTION_MCP_SESSION: &str = "remember_mcp_session";
pub const ACTION_MCP_ALWAYS: &str = "remember_mcp_always";
pub const ACTION_NETWORK_SESSION: &str = "remember_network_session";
pub const ACTION_NETWORK_ALWAYS: &str = "remember_network_always";
pub const ACTION_SHELL_SESSION: &str = "remember_shell_session";
pub const ACTION_SHELL_PREFIX: &str = "remember_shell_prefix";
pub const ACTION_SHELL_ALWAYS: &str = "remember_shell_always";

/// Outcome of the hook-side analysis.
pub enum Analysis {
    /// Provably guardian-routed (D36): the hook must exit without any decision or popup.
    Suppress,
    /// No memory enhancement for this request; show the basic allow-once/deny popup.
    Basic,
    /// Every shell segment is explicitly allowed by the latest native policy (D42): the
    /// hook answers allow without any surface.
    AutoAllow,
    /// Memory options and auto-allow query for this request.
    Enhanced {
        memory: PermissionMemory,
        /// Choices inserted between allow-once and deny (§4.3 order).
        extra_choices: Vec<ConfirmChoice>,
    },
}

/// Runs the isolated shell analysis (production: `permission_shell::analyze_for_hook`).
/// Injected so unit tests never spawn workers or depend on an installed Codex.
pub type ShellRunner<'a> = &'a dyn Fn(
    &crate::permission_shell::ShellProbe,
) -> Option<crate::permission_shell::ShellWorkerOutput>;

/// Analyze a Codex PermissionRequest for memory support. `input` is the full hook stdin.
pub fn analyze_codex(input: &Value, zh: bool) -> Analysis {
    analyze_codex_with(input, zh, &crate::permission_shell::analyze_for_hook)
}

pub fn analyze_codex_with(input: &Value, zh: bool, shell_runner: ShellRunner<'_>) -> Analysis {
    let object = match input.as_object() {
        Some(object) => object,
        None => return Analysis::Basic,
    };
    let tool_name = object
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_input = object.get("tool_input").cloned().unwrap_or(Value::Null);
    let cwd = object.get("cwd").and_then(Value::as_str).unwrap_or("");

    // Per-type derivation first (cheap), rollout gate second (I/O).
    enum Pending {
        Ready((PermissionMemory, Vec<ConfirmChoice>)),
        /// Network candidate: needs the rollout owner-command cross-check (D39 condition 3).
        Network(NetworkTarget),
        /// Plain shell script: needs the owner FunctionCall fields + worker (D38/D44).
        Shell(String),
    }
    let pending = if tool_name == "apply_patch" {
        // Verifiable native structured file edits (D13/D46).
        patch_paths(&tool_input, cwd)
            .map(|paths| file_edit_enhancement(&paths, cwd, zh))
            .map(Pending::Ready)
    } else if tool_name.starts_with("mcp__") {
        // MCP tool call: tool_name is registry-generated, not model-forgeable (D40).
        mcp_enhancement(tool_name, cwd, zh).map(Pending::Ready)
    } else if tool_name == "Bash" {
        // Network interception candidate (D39 conditions 1+2) first; anything else with a
        // command is a plain shell request (Phase 4).
        match network_target(&tool_input) {
            Some(target) => Some(Pending::Network(target)),
            None => tool_input
                .get("command")
                .and_then(Value::as_str)
                .map(|script| Pending::Shell(script.to_string())),
        }
    } else {
        None
    };
    let Some(pending) = pending else {
        return Analysis::Basic;
    };

    // Guardian / strict_auto_review gate (D36/D43): memory options and auto-allow require a
    // successful rollout read proving the reviewer is the user and no request_permissions
    // FunctionCall exists in the current turn. A proven guardian route suppresses the popup
    // entirely; anything unprovable keeps the basic popup.
    let owner_command = match &pending {
        Pending::Network(target) => Some(target.command.as_str()),
        Pending::Shell(script) => Some(script.as_str()),
        Pending::Ready(_) => None,
    };
    let Some(scan) = rollout_scan(object, owner_command) else {
        return Analysis::Basic;
    };
    match evaluate_gate(&scan) {
        RolloutGate::GuardianProven => return Analysis::Suppress,
        RolloutGate::MemoryAllowed => {}
        RolloutGate::Unproven => return Analysis::Basic,
    }

    let (memory, extra_choices) = match pending {
        Pending::Ready(enhancement) => enhancement,
        Pending::Network(target) => {
            // D39 condition 3: the owner FunctionCall's model-written justification must
            // differ from the Codex-generated description, else this is a forged/plain
            // shell request and gets no network treatment.
            if !network_cross_check(&scan, &target) {
                return Analysis::Basic;
            }
            match network_enhancement(&target, zh) {
                Some(enhancement) => enhancement,
                None => return Analysis::Basic,
            }
        }
        Pending::Shell(script) => match shell_enhancement(&scan, &script, cwd, zh, shell_runner) {
            ShellOutcome::AutoAllow => return Analysis::AutoAllow,
            ShellOutcome::Enhanced(enhancement) => enhancement,
            ShellOutcome::Basic => return Analysis::Basic,
        },
    };
    Analysis::Enhanced {
        memory,
        extra_choices,
    }
}

/// All old/new paths of the patch, lexically normalized against the hook cwd (D46).
/// `None` when the payload is not a fully parseable apply_patch envelope.
fn patch_paths(tool_input: &Value, cwd: &str) -> Option<Vec<String>> {
    let command = tool_input.get("command")?.as_str()?;
    let files = crate::permission_diff::patch::parse_apply_patch(
        command,
        crate::permission_diff::MAX_FILES,
    )
    .ok()?;
    let mut paths: Vec<String> = Vec::new();
    for file in &files {
        let mut push = |raw: &str| -> Option<()> {
            let normalized = crate::permission_rules::normalize_path(raw, cwd)?;
            if !paths.contains(&normalized) {
                paths.push(normalized);
            }
            Some(())
        };
        if let Some(old_path) = file.old_path.as_deref() {
            push(old_path)?;
        }
        push(&file.new_path)?;
    }
    (!paths.is_empty()).then_some(paths)
}

fn file_edit_enhancement(
    paths: &[String],
    cwd: &str,
    zh: bool,
) -> (PermissionMemory, Vec<ConfirmChoice>) {
    let session_note = if zh {
        "本对话（含 Resume 与本对话的子代理）"
    } else {
        "This conversation (including resume and its sub-agents)"
    };
    let mut extra_choices = Vec::new();
    let mut saves = Vec::new();

    let count = paths.len();
    extra_choices.push(ConfirmChoice {
        id: ACTION_REMEMBER_FILES.into(),
        label: if zh {
            format!("本对话不再询问这些文件（{count} 个）")
        } else {
            format!("Don't ask again for these files in this conversation ({count})")
        },
        description: session_note.to_string(),
        role: ActionRole::Default,
    });
    saves.push(MemorySave {
        action_id: ACTION_REMEMBER_FILES.into(),
        namespace: RuleNamespace::Session,
        rules: paths
            .iter()
            .map(|path| RuleKey::FileExact { path: path.clone() })
            .collect(),
        native: None,
    });

    // Aggregated scope (D10/D11): project root from the hook cwd (D12).
    let project_root = crate::permission_rules::normalize_path(".", cwd).map(|normalized_cwd| {
        crate::project::git_root(std::path::Path::new(&normalized_cwd))
            .map(|root| root.to_string_lossy().to_string())
            .unwrap_or(normalized_cwd)
    });
    match project_root {
        Some(root) if paths.iter().all(|path| within_root(path, &root)) => {
            extra_choices.push(ConfirmChoice {
                id: ACTION_REMEMBER_PROJECT.into(),
                label: if zh {
                    "本对话允许所有本项目内的文件修改".into()
                } else {
                    "Allow all file changes inside this project for this conversation".into()
                },
                description: format!("{session_note} · {root}"),
                role: ActionRole::Default,
            });
            saves.push(MemorySave {
                action_id: ACTION_REMEMBER_PROJECT.into(),
                namespace: RuleNamespace::Session,
                rules: vec![RuleKey::FileProject { root }],
                native: None,
            });
        }
        Some(_) => {
            extra_choices.push(ConfirmChoice {
                id: ACTION_REMEMBER_DISK.into(),
                label: if zh {
                    "本对话允许完全磁盘文件修改".into()
                } else {
                    "Allow file changes anywhere on disk for this conversation".into()
                },
                // Danger styling comes from the Destructive role (§4.3).
                description: session_note.to_string(),
                role: ActionRole::Destructive,
            });
            saves.push(MemorySave {
                action_id: ACTION_REMEMBER_DISK.into(),
                namespace: RuleNamespace::Session,
                rules: vec![RuleKey::FileDisk],
                native: None,
            });
        }
        // Unresolvable cwd: exact-file option only (§4.2).
        None => {}
    }

    (
        PermissionMemory {
            query: Some(MemoryQuery::FileEdit {
                paths: paths.to_vec(),
            }),
            saves,
        },
        extra_choices,
    )
}

fn within_root(path: &str, root: &str) -> bool {
    if path == root {
        return true;
    }
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() {
        return path.starts_with('/');
    }
    path.strip_prefix(trimmed)
        .is_some_and(|rest| rest.starts_with('/'))
}

// ===== MCP tools (D40/D41, §6.5.2) =====

/// Where the permanent "always allow" of an MCP tool lands.
enum McpPermanent {
    /// Server defined in a writable native layer: `approval_mode = "approve"` edit.
    Native {
        config_path: PathBuf,
        server: String,
        tool: String,
    },
    /// No native channel we can prove (plugin / codex_apps / ambiguous): D41 global shadow.
    Shadow,
    /// A readable layer explicitly sets prompt/writes for this tool: no memory options.
    RespectExplicitPrompt,
}

fn mcp_enhancement(
    tool_name: &str,
    cwd: &str,
    zh: bool,
) -> Option<(PermissionMemory, Vec<ConfirmChoice>)> {
    mcp_enhancement_at(tool_name, cwd, zh, &default_codex_home()?)
}

pub(crate) fn default_codex_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        if home.starts_with('/') {
            return Some(PathBuf::from(home));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| home.starts_with('/'))
        .map(|home| Path::new(&home).join(".codex"))
}

fn mcp_enhancement_at(
    tool_name: &str,
    cwd: &str,
    zh: bool,
    codex_home: &Path,
) -> Option<(PermissionMemory, Vec<ConfirmChoice>)> {
    // Structural sanity; the full string is also the session rule key (D40).
    if !tool_name.starts_with("mcp__") || tool_name.len() <= 5 || tool_name.len() > 1_024 {
        return None;
    }

    let permanent = mcp_permanent_channel(tool_name, cwd, codex_home);
    if matches!(permanent, McpPermanent::RespectExplicitPrompt) {
        // User explicitly configured prompting for this tool: like native, no memory offer.
        return None;
    }

    let session_note = if zh {
        "本对话（含 Resume 与本对话的子代理）"
    } else {
        "This conversation (including resume and its sub-agents)"
    };
    let mut extra_choices = vec![ConfirmChoice {
        id: ACTION_MCP_SESSION.into(),
        label: if zh {
            "本对话允许此工具".into()
        } else {
            "Allow this tool for this conversation".into()
        },
        description: session_note.to_string(),
        role: ActionRole::Default,
    }];
    let mut saves = vec![MemorySave {
        action_id: ACTION_MCP_SESSION.into(),
        namespace: RuleNamespace::Session,
        rules: vec![RuleKey::McpTool {
            tool: tool_name.to_string(),
        }],
        native: None,
    }];

    match permanent {
        McpPermanent::Native {
            config_path,
            server,
            tool,
        } => {
            extra_choices.push(ConfirmChoice {
                id: ACTION_MCP_ALWAYS.into(),
                label: if zh {
                    "始终允许此工具（写入 Codex 配置）".into()
                } else {
                    "Always allow this tool (saved to Codex config)".into()
                },
                description: config_path.to_string_lossy().to_string(),
                role: ActionRole::Default,
            });
            saves.push(MemorySave {
                action_id: ACTION_MCP_ALWAYS.into(),
                // Session bridge rule: the running Codex does not reload our external
                // config write, so the current conversation is covered by shadow (D40).
                namespace: RuleNamespace::Session,
                rules: vec![RuleKey::McpTool {
                    tool: tool_name.to_string(),
                }],
                native: Some(NativeWrite::McpApprovalMode {
                    config_path: config_path.to_string_lossy().to_string(),
                    server,
                    tool,
                }),
            });
        }
        McpPermanent::Shadow => {
            extra_choices.push(ConfirmChoice {
                id: ACTION_MCP_ALWAYS.into(),
                label: if zh {
                    "始终允许此工具（由 AskHuman 记住）".into()
                } else {
                    "Always allow this tool (remembered by AskHuman)".into()
                },
                description: if zh {
                    "跨会话生效，30 天未使用自动过期".into()
                } else {
                    "Applies across conversations; expires after 30 days unused".into()
                },
                role: ActionRole::Default,
            });
            saves.push(MemorySave {
                action_id: ACTION_MCP_ALWAYS.into(),
                namespace: RuleNamespace::Global,
                rules: vec![RuleKey::McpTool {
                    tool: tool_name.to_string(),
                }],
                native: None,
            });
        }
        McpPermanent::RespectExplicitPrompt => unreachable!(),
    }

    Some((
        PermissionMemory {
            query: Some(MemoryQuery::McpTool {
                tool: tool_name.to_string(),
            }),
            saves,
        },
        extra_choices,
    ))
}

/// One readable Codex config layer, highest precedence first (project nearest → user).
struct ConfigLayer {
    path: PathBuf,
    doc: toml_edit::DocumentMut,
    /// Directory containing `.codex` for project layers, `None` for the user layer.
    project_root: Option<PathBuf>,
}

/// Readable layers: `.codex/config.toml` from the hook cwd up to the git root (nearest
/// first), then `$CODEX_HOME/config.toml`. Unparseable files are skipped (fail toward
/// the shadow fallback, never toward a native write).
fn readable_config_layers(cwd: &str, codex_home: &Path) -> Vec<ConfigLayer> {
    let mut layers = Vec::new();
    if let Some(normalized_cwd) = crate::permission_rules::normalize_path(".", cwd) {
        let cwd_path = PathBuf::from(&normalized_cwd);
        let stop = crate::project::git_root(&cwd_path).unwrap_or_else(|| cwd_path.clone());
        let mut dir = Some(cwd_path.as_path());
        while let Some(current) = dir {
            if let Some(layer) = load_layer(&current.join(".codex/config.toml")) {
                layers.push(ConfigLayer {
                    project_root: Some(current.to_path_buf()),
                    ..layer
                });
            }
            if current == stop {
                break;
            }
            dir = current.parent();
        }
    }
    if let Some(layer) = load_layer(&codex_home.join("config.toml")) {
        layers.push(layer);
    }
    layers
}

fn load_layer(path: &Path) -> Option<ConfigLayer> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    Some(ConfigLayer {
        path: path.to_path_buf(),
        doc,
        project_root: None,
    })
}

/// Server names defined under `[mcp_servers]` of one layer.
fn layer_server_names(layer: &ConfigLayer) -> Vec<String> {
    layer
        .doc
        .as_table()
        .get("mcp_servers")
        .and_then(toml_edit::Item::as_table_like)
        .map(|table| table.iter().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default()
}

/// Explicit approval mode for `server`/`tool` in one layer: tool-level `approval_mode`,
/// falling back to the server's `default_tools_approval_mode`.
fn layer_explicit_mode(layer: &ConfigLayer, server: &str, tool: &str) -> Option<String> {
    let server_table = layer
        .doc
        .as_table()
        .get("mcp_servers")
        .and_then(toml_edit::Item::as_table_like)?
        .get(server)
        .and_then(toml_edit::Item::as_table_like)?;
    let tool_mode = server_table
        .get("tools")
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|tools| tools.get(tool))
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|entry| entry.get("approval_mode"))
        .and_then(toml_edit::Item::as_str);
    tool_mode
        .or_else(|| {
            server_table
                .get("default_tools_approval_mode")
                .and_then(toml_edit::Item::as_str)
        })
        .map(str::to_string)
}

/// Trust check against the user config on disk, for callers outside this module
/// (shell worker rules-layer discovery, D33).
pub(crate) fn codex_project_trusted(codex_home: &Path, root: &Path) -> bool {
    let user_layer = load_layer(&codex_home.join("config.toml"));
    project_is_trusted(user_layer.as_ref(), root)
}

/// D33-lite trust check: `[projects."<root>"] trust_level = "trusted"` in the user
/// config, matching either the raw root string or its canonicalized form. Anything
/// unprovable is untrusted (fail toward shadow).
fn project_is_trusted(user_layer: Option<&ConfigLayer>, root: &Path) -> bool {
    let Some(user_layer) = user_layer else {
        return false;
    };
    let Some(projects) = user_layer
        .doc
        .as_table()
        .get("projects")
        .and_then(toml_edit::Item::as_table_like)
    else {
        return false;
    };
    let mut keys = vec![root.to_string_lossy().to_string()];
    if let Ok(canonical) = std::fs::canonicalize(root) {
        let canonical = canonical.to_string_lossy().to_string();
        if !keys.contains(&canonical) {
            keys.push(canonical);
        }
    }
    keys.iter().any(|key| {
        projects
            .get(key)
            .and_then(toml_edit::Item::as_table_like)
            .and_then(|entry| entry.get("trust_level"))
            .and_then(toml_edit::Item::as_str)
            == Some("trusted")
    })
}

fn mcp_permanent_channel(tool_name: &str, cwd: &str, codex_home: &Path) -> McpPermanent {
    // codex_apps connector tools need a connector_id we cannot derive: D41 shadow.
    if tool_name.starts_with("mcp__codex_apps__") {
        return McpPermanent::Shadow;
    }
    let layers = readable_config_layers(cwd, codex_home);
    let user_layer = layers.iter().find(|layer| layer.project_root.is_none());

    // Candidate (layer index, server, bare tool) whose name splits the hook tool_name.
    let mut candidates: Vec<(usize, String, String)> = Vec::new();
    for (index, layer) in layers.iter().enumerate() {
        for server in layer_server_names(layer) {
            let prefix = format!("mcp__{server}__");
            if let Some(tool) = tool_name.strip_prefix(&prefix) {
                if !tool.is_empty() {
                    candidates.push((index, server, tool.to_string()));
                }
            }
        }
    }

    // Respect explicit prompt/writes config on any candidate reading (D40): a higher layer
    // could override in the effective merge, but hiding the option is the safe direction.
    for (index, server, tool) in &candidates {
        if let Some(mode) = layer_explicit_mode(&layers[*index], server, tool) {
            if mode == "prompt" || mode == "writes" {
                return McpPermanent::RespectExplicitPrompt;
            }
        }
    }

    // An unambiguous server name is required for a native write (`__` split ambiguity).
    let mut names: Vec<&str> = candidates
        .iter()
        .map(|(_, server, _)| server.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.len() != 1 {
        return McpPermanent::Shadow;
    }

    // Codex persists to the defining project layer first, then the user layer.
    for (index, server, tool) in &candidates {
        let layer = &layers[*index];
        match &layer.project_root {
            Some(root) => {
                if project_is_trusted(user_layer, root) {
                    return McpPermanent::Native {
                        config_path: layer.path.clone(),
                        server: server.clone(),
                        tool: tool.clone(),
                    };
                }
                // Untrusted project layer is never a write target; keep looking lower.
            }
            None => {
                return McpPermanent::Native {
                    config_path: layer.path.clone(),
                    server: server.clone(),
                    tool: tool.clone(),
                };
            }
        }
    }
    McpPermanent::Shadow
}

// ===== Network host approvals (D39, §6.5.1) =====

/// A Bash PermissionRequest that structurally looks like a Codex network interception
/// (D39 conditions 1+2). Condition 3 (rollout cross-check) is applied separately.
struct NetworkTarget {
    /// The triggering shell command (or `network-access {target}` when ownerless).
    command: String,
    /// The full hook `description` field, `network-access {proto}://{host}:{port}`.
    description: String,
    host: String,
    protocol: String,
    port: u16,
}

/// Strict machine-format parse of the hook `description`; anything that does not
/// round-trip byte-exactly is treated as a plain shell request.
fn network_target(tool_input: &Value) -> Option<NetworkTarget> {
    let command = tool_input.get("command")?.as_str()?.to_string();
    let description = tool_input.get("description")?.as_str()?.to_string();
    let target = description.strip_prefix("network-access ")?;
    let (protocol, rest) = target.split_once("://")?;
    if !crate::permission_rules::NETWORK_PROTOCOLS.contains(&protocol) {
        return None;
    }
    let (host, port_text) = rest.rsplit_once(':')?;
    if port_text.is_empty() || !port_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port_text.parse().ok()?;
    if !crate::permission_rules::network_host_is_valid(host) {
        return None;
    }
    // Round-trip check pins the exact `format_network_target` shape (no extra text).
    if description != format!("network-access {protocol}://{host}:{port}") {
        return None;
    }
    let host = host.to_string();
    let protocol = protocol.to_string();
    Some(NetworkTarget {
        command,
        description,
        host,
        protocol,
        port,
    })
}

/// D39 condition 3 against the rollout scan: genuine network descriptions are generated by
/// Codex, while a plain shell request's description is the model's own `justification` on
/// the owner FunctionCall. Fail closed toward plain-shell treatment.
fn network_cross_check(scan: &RolloutScan, target: &NetworkTarget) -> bool {
    if scan.owner_calls.is_empty() {
        // No owner FunctionCall: only the native ownerless prompt-command shape qualifies.
        let target_text = format!("{}://{}:{}", target.protocol, target.host, target.port);
        return target.command == format!("network-access {target_text}");
    }
    if scan
        .owner_calls
        .iter()
        .any(|call| call.justification.as_deref() == Some(target.description.as_str()))
    {
        return false;
    }
    // Multiple FunctionCalls for the same command with diverging fields: fail closed (D44).
    scan.owner_calls
        .iter()
        .all(|call| call.justification == scan.owner_calls[0].justification)
}

fn network_enhancement(
    target: &NetworkTarget,
    zh: bool,
) -> Option<(PermissionMemory, Vec<ConfirmChoice>)> {
    let rules_path = default_codex_home()?.join("rules/default.rules");
    let host = &target.host;
    let session_note = if zh {
        "本对话（含 Resume 与本对话的子代理）"
    } else {
        "This conversation (including resume and its sub-agents)"
    };
    let key = RuleKey::NetworkHost {
        host: host.clone(),
        protocol: target.protocol.clone(),
        port: target.port,
    };
    let extra_choices = vec![
        ConfirmChoice {
            id: ACTION_NETWORK_SESSION.into(),
            label: if zh {
                format!("本对话允许访问 {host}")
            } else {
                format!("Allow access to {host} for this conversation")
            },
            description: format!(
                "{session_note} · {}://{host}:{}",
                target.protocol, target.port
            ),
            role: ActionRole::Default,
        },
        ConfirmChoice {
            id: ACTION_NETWORK_ALWAYS.into(),
            label: if zh {
                format!("始终允许访问 {host}（写入 Codex 全局规则）")
            } else {
                format!("Always allow access to {host} (saved to Codex global rules)")
            },
            // The native network_rule is host+protocol wide (any port).
            description: format!("{}://{host}", target.protocol),
            role: ActionRole::Default,
        },
    ];
    let saves = vec![
        MemorySave {
            action_id: ACTION_NETWORK_SESSION.into(),
            namespace: RuleNamespace::Session,
            rules: vec![key.clone()],
            native: None,
        },
        MemorySave {
            action_id: ACTION_NETWORK_ALWAYS.into(),
            // Session bridge: the running Codex never reloads default.rules (D39).
            namespace: RuleNamespace::Session,
            rules: vec![key.clone()],
            native: Some(NativeWrite::NetworkRule {
                rules_path: rules_path.to_string_lossy().to_string(),
                host: host.clone(),
                protocol: target.protocol.clone(),
            }),
        },
    ];
    Some((
        PermissionMemory {
            query: Some(MemoryQuery::NetworkHost {
                host: host.clone(),
                protocol: target.protocol.clone(),
                port: target.port,
            }),
            saves,
        },
        extra_choices,
    ))
}

// ===== Plain shell requests (D28–D35, D38, D42–D45, §6.1) =====

enum ShellOutcome {
    /// Every segment is explicitly allowed by the latest native policy (D42/D28).
    AutoAllow,
    Enhanced((PermissionMemory, Vec<ConfirmChoice>)),
    Basic,
}

/// Collapse the rollout turn context into the two fields the worker's heuristic
/// replication needs. `None` when the schema is not the verified shape (fail closed).
fn shell_turn_fields(context: &Value) -> Option<(String, String)> {
    let approval_policy = match context.get("approval_policy") {
        Some(Value::String(value)) => value.clone(),
        // AskForApproval::Granular is externally tagged: {"granular": {...}}.
        Some(Value::Object(map)) if map.contains_key("granular") => "granular".to_string(),
        _ => return None,
    };
    let sandbox_kind = if let Some(kind) = context
        .get("file_system_sandbox_policy")
        .and_then(|policy| policy.get("kind"))
        .and_then(Value::as_str)
    {
        kind.to_string()
    } else if let Some(mode) = context
        .get("sandbox_policy")
        .and_then(|policy| policy.get("type"))
        .and_then(Value::as_str)
    {
        // Legacy sandbox_policy fallback, mirroring from_legacy_sandbox_policy_for_cwd.
        match mode {
            "danger-full-access" => "unrestricted".to_string(),
            "external-sandbox" => "external-sandbox".to_string(),
            "read-only" | "workspace-write" => "restricted".to_string(),
            _ => return None,
        }
    } else {
        return None;
    };
    Some((approval_policy, sandbox_kind))
}

fn shell_enhancement(
    scan: &RolloutScan,
    script: &str,
    cwd: &str,
    zh: bool,
    shell_runner: ShellRunner<'_>,
) -> ShellOutcome {
    // D44: the owner FunctionCall supplies prefix_rule / sandbox_permissions. No owner
    // call means we cannot prove a plain first-time shell approval; diverging fields
    // abandon derivation. Both fail closed to the basic popup.
    let Some(first) = scan.owner_calls.first() else {
        return ShellOutcome::Basic;
    };
    if scan.owner_calls.iter().any(|call| call != first) {
        return ShellOutcome::Basic;
    }
    let sandbox_override = first
        .sandbox_permissions
        .as_deref()
        .is_some_and(|value| value != "use_default");
    let Some(context) = scan.turn_context.as_ref() else {
        return ShellOutcome::Basic;
    };
    let Some((approval_policy, sandbox_kind)) = shell_turn_fields(context) else {
        return ShellOutcome::Basic;
    };
    let probe = crate::permission_shell::ShellProbe {
        script: script.to_string(),
        cwd: cwd.to_string(),
        approval_policy,
        sandbox_kind,
        sandbox_override,
        prefix_rule: first.prefix_rule.clone(),
    };
    let Some(output) = shell_runner(&probe) else {
        return ShellOutcome::Basic;
    };
    if output.disabled_reason.is_some() || output.segments.is_empty() {
        return ShellOutcome::Basic;
    }
    // D42/D28: every segment explicitly rule-allowed by the freshly re-read native policy
    // is Codex's own bypass_sandbox condition; a reloaded Codex would not prompt at all.
    if output.explicit_allow_all {
        return ShellOutcome::AutoAllow;
    }
    if output.decision == "forbidden" {
        // Our replication says native would forbid without prompting; something is off
        // with this request, so give it no memory surface.
        return ShellOutcome::Basic;
    }

    let session_note = if zh {
        "本对话（含 Resume 与本对话的子代理）"
    } else {
        "This conversation (including resume and its sub-agents)"
    };
    let mut extra_choices: Vec<ConfirmChoice> = Vec::new();
    let mut saves: Vec<MemorySave> = Vec::new();
    let mut query = None;

    // Session tiers and the auto-allow query are gated on the dangerous list (D38).
    if !output.dangerous_any {
        let preview = shell_segments_preview(&output.segments);
        extra_choices.push(ConfirmChoice {
            id: ACTION_SHELL_SESSION.into(),
            label: if zh {
                "本对话不再询问这些确切命令".into()
            } else {
                "Don't ask again for these exact commands this conversation".into()
            },
            description: format!("{session_note} · {preview}"),
            role: ActionRole::Default,
        });
        saves.push(MemorySave {
            action_id: ACTION_SHELL_SESSION.into(),
            namespace: RuleNamespace::Session,
            rules: output
                .segments
                .iter()
                .map(|argv| RuleKey::ShellExact { argv: argv.clone() })
                .collect(),
            native: None,
        });
        query = Some(MemoryQuery::ShellCommands {
            commands: output.segments.clone(),
        });
        // Prefix tier: only the model's own prefix_rule, validated exactly like the
        // permanent amendment (D38).
        if output.amendment_from_prefix_rule {
            if let Some(prefix) = output.amendment.clone() {
                let prefix_text = prefix.join(" ");
                extra_choices.push(ConfirmChoice {
                    id: ACTION_SHELL_PREFIX.into(),
                    label: if zh {
                        format!("本对话允许 {prefix_text} 开头的命令")
                    } else {
                        format!("Allow commands starting with {prefix_text} this conversation")
                    },
                    description: session_note.to_string(),
                    role: ActionRole::Default,
                });
                saves.push(MemorySave {
                    action_id: ACTION_SHELL_PREFIX.into(),
                    namespace: RuleNamespace::Session,
                    rules: vec![RuleKey::ShellPrefix { prefix }],
                    native: None,
                });
            }
        }
    }

    // Permanent tier mirrors the native TUI's conditional "don't ask again" option: shown
    // whenever the native amendment derivation proposes a prefix (D18 baseline). No
    // session bridge: default.rules is the single source of truth and is re-read on the
    // next request (D28).
    if let Some(prefix) = output.amendment.clone() {
        if let Some(home) = default_codex_home() {
            let rules_path = home.join("rules/default.rules");
            let prefix_text = prefix.join(" ");
            extra_choices.push(ConfirmChoice {
                id: ACTION_SHELL_ALWAYS.into(),
                label: if zh {
                    format!("始终允许 {prefix_text} 开头的命令（写入 Codex 全局规则）")
                } else {
                    format!(
                        "Always allow commands starting with {prefix_text} (saved to Codex global rules)"
                    )
                },
                description: prefix_text,
                role: ActionRole::Default,
            });
            saves.push(MemorySave {
                action_id: ACTION_SHELL_ALWAYS.into(),
                namespace: RuleNamespace::Session,
                rules: Vec::new(),
                native: Some(NativeWrite::PrefixRule {
                    rules_path: rules_path.to_string_lossy().to_string(),
                    prefix,
                }),
            });
        }
    }

    if saves.is_empty() {
        return ShellOutcome::Basic;
    }
    ShellOutcome::Enhanced((PermissionMemory { query, saves }, extra_choices))
}

/// Short human preview of split segments for the exact-tier subtext.
fn shell_segments_preview(segments: &[Vec<String>]) -> String {
    const MAX_PREVIEW: usize = 120;
    let mut preview = segments
        .iter()
        .map(|argv| argv.join(" "))
        .collect::<Vec<_>>()
        .join(" && ");
    if preview.chars().count() > MAX_PREVIEW {
        preview = preview.chars().take(MAX_PREVIEW).collect::<String>() + "…";
    }
    preview
}

// ===== Rollout gate (D36/D43) =====

enum RolloutGate {
    /// TurnContextItem proves guardian routing: reviewer=auto_review + policy on-request/granular.
    GuardianProven,
    /// Reviewer is the user and no request_permissions FunctionCall in the current turn.
    MemoryAllowed,
    /// Anything else: unreadable rollout, missing turn context, unknown schema.
    Unproven,
}

/// Upper bound on rollout size we are willing to scan (bail out beyond it: Unproven).
const MAX_ROLLOUT_BYTES: u64 = 512 * 1024 * 1024;
/// Upper bound on a single rollout line we parse.
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Scan the transcript referenced by the hook input. `owner_command` optionally collects
/// the model-side `justification` of every current-turn FunctionCall whose command equals
/// it (D39/D44). `None` means the rollout is unreadable / unprovable.
fn rollout_scan(
    input: &serde_json::Map<String, Value>,
    owner_command: Option<&str>,
) -> Option<RolloutScan> {
    let transcript_path = input.get("transcript_path").and_then(Value::as_str)?;
    let turn_id = input
        .get("turn_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    scan_rollout(
        std::path::Path::new(transcript_path),
        turn_id,
        owner_command,
    )
}

/// Model-side fields of one current-turn FunctionCall matching the probed owner command.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerCall {
    justification: Option<String>,
    prefix_rule: Option<Vec<String>>,
    /// Raw serde value of `sandbox_permissions` (`use_default` when absent).
    sandbox_permissions: Option<String>,
}

struct RolloutScan {
    turn_context: Option<Value>,
    request_permissions_risk: bool,
    /// Current-turn FunctionCalls matching the probed owner command (D39/D44).
    owner_calls: Vec<OwnerCall>,
}

fn scan_rollout(
    path: &std::path::Path,
    turn_id: &str,
    owner_command: Option<&str>,
) -> Option<RolloutScan> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_ROLLOUT_BYTES {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let mut scan = RolloutScan {
        turn_context: None,
        request_permissions_risk: false,
        owner_calls: Vec::new(),
    };
    let mut buffer = Vec::new();
    let mut reader = reader;
    loop {
        buffer.clear();
        match read_capped_line(&mut reader, &mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
        let Ok(line) = std::str::from_utf8(&buffer) else {
            // Compressed or binary rollout: cannot prove anything.
            return None;
        };
        scan_rollout_line(line, turn_id, owner_command, &mut scan);
    }
    Some(scan)
}

/// Reads one line into `buffer` (without the trailing newline). Errors on oversized lines.
fn read_capped_line<R: BufRead>(reader: &mut R, buffer: &mut Vec<u8>) -> std::io::Result<usize> {
    use std::io::Read;
    let read = (&mut *reader)
        .take(MAX_LINE_BYTES as u64 + 1)
        .read_until(b'\n', buffer)?;
    if buffer.len() > MAX_LINE_BYTES {
        return Err(std::io::Error::other("rollout line too long"));
    }
    while buffer
        .last()
        .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
    {
        buffer.pop();
    }
    Ok(read)
}

fn scan_rollout_line(
    line: &str,
    turn_id: &str,
    owner_command: Option<&str>,
    scan: &mut RolloutScan,
) {
    // Cheap substring filters before any JSON parsing. When probing an owner command
    // every function_call line must be parsed (its command may be JSON-escaped in raw
    // text, so a substring test on the command itself would be unreliable).
    let interesting = (line.contains("\"turn_context\"") && line.contains(turn_id))
        || line.contains("request_permissions")
        || (owner_command.is_some() && line.contains("function_call"));
    if !interesting {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        // A line mentioning request_permissions that we cannot parse is treated as risk.
        if line.contains("request_permissions") {
            scan.request_permissions_risk = true;
        }
        return;
    };
    let item_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let payload = value.get("payload").unwrap_or(&Value::Null);
    match item_type {
        "turn_context" => {
            if payload.get("turn_id").and_then(Value::as_str) == Some(turn_id) {
                // Keep the last matching turn context (mid-turn updates override).
                scan.turn_context = Some(payload.clone());
            }
        }
        "response_item" => {
            if payload.get("type").and_then(Value::as_str) != Some("function_call") {
                return;
            }
            let call_turn = payload
                .get("internal_chat_message_metadata_passthrough")
                .and_then(|metadata| metadata.get("turn_id"))
                .and_then(Value::as_str);
            // Missing turn metadata is treated conservatively as current-turn (D43/D44).
            let current_turn = call_turn.is_none() || call_turn == Some(turn_id);
            if payload.get("name").and_then(Value::as_str) == Some("request_permissions") {
                if current_turn {
                    scan.request_permissions_risk = true;
                }
                return;
            }
            let (Some(probe), true) = (owner_command, current_turn) else {
                return;
            };
            // FunctionCall arguments are a JSON string; shell runtimes carry the hook
            // command as `command` (shell) or `cmd` (unified exec).
            let Some(arguments) = payload
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            else {
                return;
            };
            let command = arguments
                .get("command")
                .and_then(Value::as_str)
                .or_else(|| arguments.get("cmd").and_then(Value::as_str));
            if command == Some(probe) {
                let prefix_rule = arguments.get("prefix_rule").and_then(|value| {
                    let tokens = value.as_array()?;
                    tokens
                        .iter()
                        .map(|token| token.as_str().map(str::to_string))
                        .collect::<Option<Vec<String>>>()
                });
                scan.owner_calls.push(OwnerCall {
                    justification: arguments
                        .get("justification")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    prefix_rule,
                    // Present-but-non-string values map to a sentinel so they count as a
                    // sandbox override (conservative) instead of the absent default.
                    sandbox_permissions: arguments.get("sandbox_permissions").map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| "<non-string>".to_string())
                    }),
                });
            }
        }
        _ => {
            // Some other line mentioning request_permissions (e.g. event_msg): only
            // function_call items grant turn-level strict review, ignore the rest.
        }
    }
}

fn evaluate_gate(scan: &RolloutScan) -> RolloutGate {
    let Some(context) = scan.turn_context.as_ref() else {
        return RolloutGate::Unproven;
    };
    let reviewer = context
        .get("approvals_reviewer")
        .and_then(Value::as_str)
        .unwrap_or("user");
    let policy = context.get("approval_policy");
    let policy_guardian_eligible = match policy {
        Some(Value::String(value)) => value == "on-request" || value == "on-failure",
        Some(Value::Object(map)) => map.contains_key("granular"),
        _ => false,
    };
    match reviewer {
        "user" => {
            if scan.request_permissions_risk {
                RolloutGate::Unproven
            } else {
                RolloutGate::MemoryAllowed
            }
        }
        "auto_review" | "guardian_subagent" if policy_guardian_eligible => {
            RolloutGate::GuardianProven
        }
        // Reviewer is not the user but routing is not provable: fail closed to basic.
        _ => RolloutGate::Unproven,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_rollout(lines: &[Value]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        file
    }

    fn turn_context(turn_id: &str, reviewer: Option<&str>, policy: Value) -> Value {
        let mut payload = json!({
            "turn_id": turn_id,
            "approval_policy": policy,
        });
        if let Some(reviewer) = reviewer {
            payload["approvals_reviewer"] = json!(reviewer);
        }
        json!({ "timestamp": "t", "type": "turn_context", "payload": payload })
    }

    fn hook_input(transcript: &std::path::Path, patch: &str) -> Value {
        json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "s1",
            "turn_id": "turn-1",
            "transcript_path": transcript.to_string_lossy(),
            "cwd": "/work/proj",
            "permission_mode": "default",
            "tool_name": "apply_patch",
            "tool_input": { "command": patch },
        })
    }

    const PATCH: &str = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** End Patch\n";

    #[test]
    fn user_reviewer_enables_memory_options() {
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let analysis = analyze_codex(&hook_input(rollout.path(), PATCH), false);
        let Analysis::Enhanced {
            memory,
            extra_choices,
        } = analysis
        else {
            panic!("expected enhancement");
        };
        assert!(matches!(
            memory.query,
            Some(MemoryQuery::FileEdit { ref paths }) if paths == &["/work/proj/src/a.rs"]
        ));
        // No git root exists above the fake cwd, so D12 falls back to cwd as the project
        // root and every path is inside it -> project option.
        let ids: Vec<&str> = extra_choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect();
        assert_eq!(ids, [ACTION_REMEMBER_FILES, ACTION_REMEMBER_PROJECT]);
        assert_eq!(memory.saves.len(), 2);
    }

    #[test]
    fn paths_outside_project_root_offer_full_disk() {
        let patch = "*** Begin Patch\n*** Update File: /etc/hosts\n@@\n-x\n+y\n*** End Patch\n";
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let Analysis::Enhanced {
            memory,
            extra_choices,
        } = analyze_codex(&hook_input(rollout.path(), patch), false)
        else {
            panic!("expected enhancement");
        };
        let ids: Vec<&str> = extra_choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect();
        assert_eq!(ids, [ACTION_REMEMBER_FILES, ACTION_REMEMBER_DISK]);
        let disk_save = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_REMEMBER_DISK)
            .unwrap();
        assert_eq!(disk_save.rules, vec![RuleKey::FileDisk]);
        // Danger styling for the full-disk option (§4.3).
        assert_eq!(extra_choices[1].role, ActionRole::Destructive);
    }

    #[test]
    fn absent_reviewer_defaults_to_user() {
        let rollout = write_rollout(&[turn_context("turn-1", None, json!("untrusted"))]);
        assert!(matches!(
            analyze_codex(&hook_input(rollout.path(), PATCH), true),
            Analysis::Enhanced { .. }
        ));
    }

    #[test]
    fn proven_guardian_suppresses_popup() {
        for policy in [json!("on-request"), json!({"granular": {}})] {
            let rollout = write_rollout(&[turn_context("turn-1", Some("auto_review"), policy)]);
            assert!(matches!(
                analyze_codex(&hook_input(rollout.path(), PATCH), false),
                Analysis::Suppress
            ));
        }
    }

    #[test]
    fn guardian_with_other_policy_is_unproven_basic() {
        let rollout = write_rollout(&[turn_context("turn-1", Some("auto_review"), json!("never"))]);
        assert!(matches!(
            analyze_codex(&hook_input(rollout.path(), PATCH), false),
            Analysis::Basic
        ));
    }

    #[test]
    fn request_permissions_in_turn_disables_memory() {
        let call = json!({
            "timestamp": "t",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "request_permissions",
                "arguments": "{}",
                "call_id": "c1",
                "internal_chat_message_metadata_passthrough": { "turn_id": "turn-1" }
            }
        });
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("user"), json!("on-request")),
            call,
        ]);
        assert!(matches!(
            analyze_codex(&hook_input(rollout.path(), PATCH), false),
            Analysis::Basic
        ));
    }

    #[test]
    fn request_permissions_in_other_turn_is_harmless() {
        let call = json!({
            "timestamp": "t",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "request_permissions",
                "arguments": "{}",
                "call_id": "c1",
                "internal_chat_message_metadata_passthrough": { "turn_id": "turn-0" }
            }
        });
        let rollout = write_rollout(&[
            call,
            turn_context("turn-1", Some("user"), json!("on-request")),
        ]);
        assert!(matches!(
            analyze_codex(&hook_input(rollout.path(), PATCH), false),
            Analysis::Enhanced { .. }
        ));
    }

    #[test]
    fn request_permissions_without_turn_metadata_fails_closed() {
        let call = json!({
            "timestamp": "t",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "request_permissions",
                "arguments": "{}",
                "call_id": "c1"
            }
        });
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("user"), json!("on-request")),
            call,
        ]);
        assert!(matches!(
            analyze_codex(&hook_input(rollout.path(), PATCH), false),
            Analysis::Basic
        ));
    }

    #[test]
    fn missing_rollout_or_turn_context_keeps_basic() {
        let input = json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "s1",
            "turn_id": "turn-1",
            "transcript_path": "/no/such/rollout.jsonl",
            "cwd": "/work/proj",
            "permission_mode": "default",
            "tool_name": "apply_patch",
            "tool_input": { "command": PATCH },
        });
        assert!(matches!(analyze_codex(&input, false), Analysis::Basic));

        // Rollout exists but has a different turn's context only.
        let rollout = write_rollout(&[turn_context("turn-0", Some("user"), json!("on-request"))]);
        assert!(matches!(
            analyze_codex(&hook_input(rollout.path(), PATCH), false),
            Analysis::Basic
        ));

        // No transcript_path at all.
        let mut no_path = hook_input(std::path::Path::new("/tmp/x"), PATCH);
        no_path.as_object_mut().unwrap().remove("transcript_path");
        assert!(matches!(analyze_codex(&no_path, false), Analysis::Basic));
    }

    #[test]
    fn malformed_patch_keeps_basic() {
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let analysis = analyze_codex(
            &hook_input(rollout.path(), "*** Begin Patch\n*** Add File: x\n+x\n"),
            false,
        );
        assert!(matches!(analysis, Analysis::Basic));
    }

    #[test]
    fn unsupported_tools_stay_basic() {
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let mut input = hook_input(rollout.path(), PATCH);
        input["tool_name"] = json!("web_search");
        input["tool_input"] = json!({"query": "weather"});
        assert!(matches!(analyze_codex(&input, false), Analysis::Basic));
    }

    #[test]
    fn project_option_appears_when_all_paths_inside_git_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let mut input = hook_input(rollout.path(), PATCH);
        input["cwd"] = json!(root.join("sub").to_string_lossy());
        let Analysis::Enhanced {
            memory,
            extra_choices,
        } = analyze_codex(&input, true)
        else {
            panic!("expected enhancement");
        };
        let ids: Vec<&str> = extra_choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect();
        assert_eq!(ids, [ACTION_REMEMBER_FILES, ACTION_REMEMBER_PROJECT]);
        let project_save = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_REMEMBER_PROJECT)
            .unwrap();
        assert!(matches!(
            project_save.rules.as_slice(),
            [RuleKey::FileProject { .. }]
        ));
    }

    // ===== MCP (D40/D41) =====

    fn assert_shadow_permanent(saves: &[MemorySave]) {
        let always = saves
            .iter()
            .find(|save| save.action_id == ACTION_MCP_ALWAYS)
            .unwrap();
        assert_eq!(
            always.namespace,
            crate::permission_rules::RuleNamespace::Global
        );
        assert!(always.native.is_none());
    }

    #[test]
    fn mcp_without_native_channel_offers_session_plus_shadow() {
        let home = tempfile::tempdir().unwrap();
        let (memory, choices) = mcp_enhancement_at(
            "mcp__github__create_issue",
            "/work/proj",
            false,
            home.path(),
        )
        .unwrap();
        let ids: Vec<&str> = choices.iter().map(|choice| choice.id.as_str()).collect();
        assert_eq!(ids, [ACTION_MCP_SESSION, ACTION_MCP_ALWAYS]);
        assert!(matches!(
            memory.query,
            Some(MemoryQuery::McpTool { ref tool }) if tool == "mcp__github__create_issue"
        ));
        let session = &memory.saves[0];
        assert_eq!(
            session.namespace,
            crate::permission_rules::RuleNamespace::Session
        );
        assert_eq!(
            session.rules,
            vec![RuleKey::McpTool {
                tool: "mcp__github__create_issue".into()
            }]
        );
        assert_shadow_permanent(&memory.saves);
    }

    #[test]
    fn mcp_user_config_server_gets_native_write() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "[mcp_servers.github]\ncommand = \"gh-mcp\"\n",
        )
        .unwrap();
        let (memory, _) =
            mcp_enhancement_at("mcp__github__create_issue", "/work/proj", true, home.path())
                .unwrap();
        let always = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_MCP_ALWAYS)
            .unwrap();
        assert_eq!(
            always.native,
            Some(NativeWrite::McpApprovalMode {
                config_path: home
                    .path()
                    .join("config.toml")
                    .to_string_lossy()
                    .to_string(),
                server: "github".into(),
                tool: "create_issue".into(),
            })
        );
        // Session bridge rule rides along with the native write (D40).
        assert_eq!(
            always.namespace,
            crate::permission_rules::RuleNamespace::Session
        );
        assert_eq!(
            always.rules,
            vec![RuleKey::McpTool {
                tool: "mcp__github__create_issue".into()
            }]
        );
    }

    #[test]
    fn mcp_explicit_prompt_or_writes_hides_all_options() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "[mcp_servers.github]\ncommand = \"gh-mcp\"\n[mcp_servers.github.tools.create_issue]\napproval_mode = \"prompt\"\n",
        )
        .unwrap();
        assert!(mcp_enhancement_at(
            "mcp__github__create_issue",
            "/work/proj",
            false,
            home.path()
        )
        .is_none());

        std::fs::write(
            home.path().join("config.toml"),
            "[mcp_servers.github]\ncommand = \"gh-mcp\"\ndefault_tools_approval_mode = \"writes\"\n",
        )
        .unwrap();
        assert!(mcp_enhancement_at(
            "mcp__github__create_issue",
            "/work/proj",
            false,
            home.path()
        )
        .is_none());

        // Tool-level explicit auto overrides the server default: options stay.
        std::fs::write(
            home.path().join("config.toml"),
            "[mcp_servers.github]\ncommand = \"gh-mcp\"\ndefault_tools_approval_mode = \"writes\"\n[mcp_servers.github.tools.create_issue]\napproval_mode = \"auto\"\n",
        )
        .unwrap();
        assert!(mcp_enhancement_at(
            "mcp__github__create_issue",
            "/work/proj",
            false,
            home.path()
        )
        .is_some());
    }

    #[test]
    fn mcp_ambiguous_server_split_falls_back_to_shadow() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "[mcp_servers.a]\ncommand = \"a\"\n[mcp_servers.a__b]\ncommand = \"ab\"\n",
        )
        .unwrap();
        let (memory, _) =
            mcp_enhancement_at("mcp__a__b__c", "/work/proj", false, home.path()).unwrap();
        assert_shadow_permanent(&memory.saves);
    }

    #[test]
    fn mcp_codex_apps_always_uses_shadow() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "[mcp_servers.codex_apps]\ncommand = \"x\"\n",
        )
        .unwrap();
        let (memory, _) = mcp_enhancement_at(
            "mcp__codex_apps__calendar__create_event",
            "/work/proj",
            false,
            home.path(),
        )
        .unwrap();
        assert_shadow_permanent(&memory.saves);
    }

    #[test]
    fn mcp_trusted_project_layer_wins_untrusted_falls_through() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join(".codex")).unwrap();
        std::fs::write(
            root.join(".codex/config.toml"),
            "[mcp_servers.github]\ncommand = \"proj\"\n",
        )
        .unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "[mcp_servers.github]\ncommand = \"user\"\n",
        )
        .unwrap();
        let cwd = root.to_string_lossy().to_string();

        // Untrusted project: the project layer is never a write target; user layer wins.
        let (memory, _) =
            mcp_enhancement_at("mcp__github__create_issue", &cwd, false, home.path()).unwrap();
        let always = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_MCP_ALWAYS)
            .unwrap();
        let Some(NativeWrite::McpApprovalMode { config_path, .. }) = &always.native else {
            panic!("expected native write");
        };
        assert!(config_path.ends_with("config.toml"));
        assert!(!config_path.contains(".codex/config.toml"));

        // Mark the project trusted (raw key): project config becomes the target.
        let trusted = format!(
            "[mcp_servers.github]\ncommand = \"user\"\n[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
            root.to_string_lossy()
        );
        std::fs::write(home.path().join("config.toml"), trusted).unwrap();
        let (memory, _) =
            mcp_enhancement_at("mcp__github__create_issue", &cwd, false, home.path()).unwrap();
        let always = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_MCP_ALWAYS)
            .unwrap();
        let Some(NativeWrite::McpApprovalMode { config_path, .. }) = &always.native else {
            panic!("expected native write");
        };
        assert!(config_path.ends_with(".codex/config.toml"));
    }

    #[test]
    fn mcp_requests_go_through_rollout_gate() {
        // Guardian-proven rollouts suppress MCP popups too. The improbable server name keeps
        // the test independent from the developer's real ~/.codex/config.toml.
        let rollout = write_rollout(&[turn_context(
            "turn-1",
            Some("auto_review"),
            json!("on-request"),
        )]);
        let mut input = hook_input(rollout.path(), PATCH);
        input["tool_name"] = json!("mcp__askhuman_test_no_such_server__tool");
        input["tool_input"] = json!({});
        assert!(matches!(analyze_codex(&input, false), Analysis::Suppress));

        // Unprovable rollout keeps the basic popup, no memory options.
        let mut input = input.clone();
        input["transcript_path"] = json!("/no/such/rollout.jsonl");
        assert!(matches!(analyze_codex(&input, false), Analysis::Basic));
    }

    // ===== network (D39) =====

    fn shell_call(turn: &str, command: &str, justification: Option<&str>) -> Value {
        let mut arguments = json!({ "command": command });
        if let Some(justification) = justification {
            arguments["justification"] = json!(justification);
        }
        json!({
            "timestamp": "t",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "shell_command",
                "arguments": arguments.to_string(),
                "call_id": "c1",
                "internal_chat_message_metadata_passthrough": { "turn_id": turn }
            }
        })
    }

    fn network_input(transcript: &std::path::Path, command: &str, description: &str) -> Value {
        json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "s1",
            "turn_id": "turn-1",
            "transcript_path": transcript.to_string_lossy(),
            "cwd": "/work/proj",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": { "command": command, "description": description },
        })
    }

    const NET_DESC: &str = "network-access https://api.github.com:443";

    #[test]
    fn genuine_network_request_offers_host_options() {
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("user"), json!("on-request")),
            shell_call(
                "turn-1",
                "curl https://api.github.com/repos",
                Some("fetch repo data"),
            ),
        ]);
        let input = network_input(
            rollout.path(),
            "curl https://api.github.com/repos",
            NET_DESC,
        );
        let Analysis::Enhanced {
            memory,
            extra_choices,
        } = analyze_codex(&input, false)
        else {
            panic!("expected enhancement");
        };
        assert!(matches!(
            memory.query,
            Some(MemoryQuery::NetworkHost { ref host, ref protocol, port })
                if host == "api.github.com" && protocol == "https" && port == 443
        ));
        let ids: Vec<&str> = extra_choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect();
        assert_eq!(ids, [ACTION_NETWORK_SESSION, ACTION_NETWORK_ALWAYS]);
        let always = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_NETWORK_ALWAYS)
            .unwrap();
        let Some(NativeWrite::NetworkRule {
            rules_path,
            host,
            protocol,
        }) = &always.native
        else {
            panic!("expected network rule write");
        };
        assert!(rules_path.ends_with("/rules/default.rules"));
        assert_eq!(host, "api.github.com");
        assert_eq!(protocol, "https");
        // Bridge session rule keeps the port dimension.
        assert_eq!(
            always.rules,
            vec![RuleKey::NetworkHost {
                host: "api.github.com".into(),
                protocol: "https".into(),
                port: 443,
            }]
        );
    }

    #[test]
    fn forged_network_description_stays_basic() {
        // A plain shell request whose model justification mimics the machine format: the
        // justification equals the hook description, which proves it is model-authored.
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("user"), json!("on-request")),
            shell_call("turn-1", "curl https://evil.com/x", Some(NET_DESC)),
        ]);
        let input = network_input(rollout.path(), "curl https://evil.com/x", NET_DESC);
        assert!(matches!(analyze_codex(&input, false), Analysis::Basic));
    }

    #[test]
    fn ownerless_prompt_command_is_genuine() {
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let input = network_input(
            rollout.path(),
            "network-access https://api.github.com:443",
            NET_DESC,
        );
        assert!(matches!(
            analyze_codex(&input, true),
            Analysis::Enhanced { .. }
        ));
    }

    #[test]
    fn owned_command_missing_from_rollout_stays_basic() {
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let input = network_input(
            rollout.path(),
            "curl https://api.github.com/repos",
            NET_DESC,
        );
        assert!(matches!(analyze_codex(&input, false), Analysis::Basic));
    }

    #[test]
    fn divergent_owner_justifications_fail_closed() {
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("user"), json!("on-request")),
            shell_call("turn-1", "curl https://api.github.com/repos", Some("first")),
            shell_call(
                "turn-1",
                "curl https://api.github.com/repos",
                Some("second"),
            ),
        ]);
        let input = network_input(
            rollout.path(),
            "curl https://api.github.com/repos",
            NET_DESC,
        );
        assert!(matches!(analyze_codex(&input, false), Analysis::Basic));
    }

    #[test]
    fn malformed_network_descriptions_stay_basic() {
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("user"), json!("on-request")),
            shell_call("turn-1", "curl https://x/", Some("fetch")),
        ]);
        for description in [
            "network-access ftp://host:21",          // protocol outside whitelist
            "network-access https://Host.Com:443",   // uppercase host
            "network-access https://host:99999",     // port overflow
            "network-access https://host:443 extra", // trailing text
            "network-access https://host:443\n",     // whitespace
            "please allow network-access https://h:443", // prefix text
            "network-access https://host:",          // empty port
        ] {
            let input = network_input(rollout.path(), "curl https://x/", description);
            assert!(
                matches!(analyze_codex(&input, false), Analysis::Basic),
                "should stay basic: {description:?}"
            );
        }
    }

    #[test]
    fn network_request_in_guardian_turn_is_suppressed() {
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("auto_review"), json!("on-request")),
            shell_call("turn-1", "curl https://api.github.com/repos", Some("fetch")),
        ]);
        let input = network_input(
            rollout.path(),
            "curl https://api.github.com/repos",
            NET_DESC,
        );
        assert!(matches!(analyze_codex(&input, false), Analysis::Suppress));
    }

    // ===== Plain shell (D38/D42/D44) =====

    use crate::permission_shell::{ShellProbe, ShellWorkerOutput};

    fn shell_turn_context(turn_id: &str, policy: Value, sandbox_kind: &str) -> Value {
        json!({
            "timestamp": "t",
            "type": "turn_context",
            "payload": {
                "turn_id": turn_id,
                "approval_policy": policy,
                "approvals_reviewer": "user",
                "file_system_sandbox_policy": { "kind": sandbox_kind },
            }
        })
    }

    fn shell_input(transcript: &std::path::Path, command: &str) -> Value {
        json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "s1",
            "turn_id": "turn-1",
            "transcript_path": transcript.to_string_lossy(),
            "cwd": "/work/proj",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": { "command": command },
        })
    }

    fn shell_call_full(
        turn: &str,
        command: &str,
        prefix_rule: Option<Value>,
        sandbox_permissions: Option<&str>,
    ) -> Value {
        let mut arguments = json!({ "command": command });
        if let Some(prefix_rule) = prefix_rule {
            arguments["prefix_rule"] = prefix_rule;
        }
        if let Some(sandbox_permissions) = sandbox_permissions {
            arguments["sandbox_permissions"] = json!(sandbox_permissions);
        }
        json!({
            "timestamp": "t",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "shell_command",
                "arguments": arguments.to_string(),
                "call_id": "c1",
                "internal_chat_message_metadata_passthrough": { "turn_id": turn }
            }
        })
    }

    fn worker_output(segments: &[&[&str]]) -> ShellWorkerOutput {
        ShellWorkerOutput {
            disabled_reason: None,
            segments: segments
                .iter()
                .map(|argv| argv.iter().map(|token| token.to_string()).collect())
                .collect(),
            decision: "prompt".into(),
            explicit_allow_all: false,
            dangerous_any: false,
            amendment: None,
            amendment_from_prefix_rule: false,
        }
    }

    #[test]
    fn plain_shell_offers_exact_session_tier_and_permanent_amendment() {
        let rollout = write_rollout(&[
            shell_turn_context("turn-1", json!("on-request"), "restricted"),
            shell_call_full("turn-1", "git status && cargo build", None, None),
        ]);
        let input = shell_input(rollout.path(), "git status && cargo build");
        let runner = |_probe: &ShellProbe| {
            let mut output = worker_output(&[&["git", "status"], &["cargo", "build"]]);
            output.amendment = Some(vec!["cargo".into(), "build".into()]);
            Some(output)
        };
        let Analysis::Enhanced {
            memory,
            extra_choices,
        } = analyze_codex_with(&input, false, &runner)
        else {
            panic!("expected enhancement");
        };
        let ids: Vec<&str> = extra_choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect();
        assert_eq!(ids, [ACTION_SHELL_SESSION, ACTION_SHELL_ALWAYS]);
        assert!(matches!(
            memory.query,
            Some(MemoryQuery::ShellCommands { ref commands })
                if commands == &[vec!["git".to_string(), "status".into()],
                                 vec!["cargo".to_string(), "build".into()]]
        ));
        let session = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_SHELL_SESSION)
            .unwrap();
        assert_eq!(session.rules.len(), 2);
        assert!(session.native.is_none());
        let always = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_SHELL_ALWAYS)
            .unwrap();
        // No session bridge for shell permanents: default.rules is re-read next time (D28).
        assert!(always.rules.is_empty());
        assert!(matches!(
            always.native,
            Some(crate::permission_rules::NativeWrite::PrefixRule { ref prefix, ref rules_path })
                if prefix == &["cargo".to_string(), "build".into()]
                    && rules_path.ends_with("/rules/default.rules")
        ));
    }

    #[test]
    fn explicit_allow_all_returns_auto_allow() {
        let rollout = write_rollout(&[
            shell_turn_context("turn-1", json!("untrusted"), "restricted"),
            shell_call_full("turn-1", "cargo build", None, None),
        ]);
        let input = shell_input(rollout.path(), "cargo build");
        let runner = |_probe: &ShellProbe| {
            let mut output = worker_output(&[&["cargo", "build"]]);
            output.decision = "allow".into();
            output.explicit_allow_all = true;
            Some(output)
        };
        assert!(matches!(
            analyze_codex_with(&input, false, &runner),
            Analysis::AutoAllow
        ));
    }

    #[test]
    fn model_prefix_rule_adds_session_prefix_tier() {
        let rollout = write_rollout(&[
            shell_turn_context("turn-1", json!("on-request"), "restricted"),
            shell_call_full(
                "turn-1",
                "cargo build",
                Some(json!(["cargo"])),
                Some("use_default"),
            ),
        ]);
        let input = shell_input(rollout.path(), "cargo build");
        let runner = |probe: &ShellProbe| {
            assert_eq!(
                probe.prefix_rule.as_deref(),
                Some(&["cargo".to_string()][..])
            );
            assert!(!probe.sandbox_override);
            assert_eq!(probe.approval_policy, "on-request");
            assert_eq!(probe.sandbox_kind, "restricted");
            let mut output = worker_output(&[&["cargo", "build"]]);
            output.amendment = Some(vec!["cargo".into()]);
            output.amendment_from_prefix_rule = true;
            Some(output)
        };
        let Analysis::Enhanced {
            memory,
            extra_choices,
        } = analyze_codex_with(&input, true, &runner)
        else {
            panic!("expected enhancement");
        };
        let ids: Vec<&str> = extra_choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                ACTION_SHELL_SESSION,
                ACTION_SHELL_PREFIX,
                ACTION_SHELL_ALWAYS
            ]
        );
        let prefix_save = memory
            .saves
            .iter()
            .find(|save| save.action_id == ACTION_SHELL_PREFIX)
            .unwrap();
        assert_eq!(
            prefix_save.rules,
            vec![RuleKey::ShellPrefix {
                prefix: vec!["cargo".into()]
            }]
        );
        assert!(prefix_save.native.is_none());
    }

    #[test]
    fn dangerous_segments_drop_session_tiers_and_query() {
        let rollout = write_rollout(&[
            shell_turn_context("turn-1", json!("on-request"), "restricted"),
            shell_call_full("turn-1", "rm -rf build", None, None),
        ]);
        let input = shell_input(rollout.path(), "rm -rf build");
        let runner = |_probe: &ShellProbe| {
            let mut output = worker_output(&[&["rm", "-rf", "build"]]);
            output.dangerous_any = true;
            output.amendment = Some(vec!["rm".into(), "-rf".into(), "build".into()]);
            Some(output)
        };
        let Analysis::Enhanced {
            memory,
            extra_choices,
        } = analyze_codex_with(&input, false, &runner)
        else {
            panic!("expected enhancement");
        };
        let ids: Vec<&str> = extra_choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect();
        // Native parity: the permanent amendment option survives, session tiers and the
        // auto-allow query are gated by the dangerous list (D38/D42).
        assert_eq!(ids, [ACTION_SHELL_ALWAYS]);
        assert!(memory.query.is_none());
    }

    #[test]
    fn shell_worker_failure_or_disable_stays_basic() {
        let rollout = write_rollout(&[
            shell_turn_context("turn-1", json!("on-request"), "restricted"),
            shell_call_full("turn-1", "git status", None, None),
        ]);
        let input = shell_input(rollout.path(), "git status");
        let none_runner = |_probe: &ShellProbe| None;
        assert!(matches!(
            analyze_codex_with(&input, false, &none_runner),
            Analysis::Basic
        ));
        let disabled_runner = |_probe: &ShellProbe| {
            Some(ShellWorkerOutput {
                disabled_reason: Some("managed overlay".into()),
                ..worker_output(&[])
            })
        };
        assert!(matches!(
            analyze_codex_with(&input, false, &disabled_runner),
            Analysis::Basic
        ));
        let forbidden_runner = |_probe: &ShellProbe| {
            let mut output = worker_output(&[&["git", "status"]]);
            output.decision = "forbidden".into();
            Some(output)
        };
        assert!(matches!(
            analyze_codex_with(&input, false, &forbidden_runner),
            Analysis::Basic
        ));
    }

    #[test]
    fn ownerless_or_divergent_shell_calls_stay_basic() {
        let panicking_runner =
            |_probe: &ShellProbe| -> Option<ShellWorkerOutput> { panic!("must not run worker") };
        // No owner FunctionCall in the rollout.
        let rollout = write_rollout(&[shell_turn_context(
            "turn-1",
            json!("on-request"),
            "restricted",
        )]);
        let input = shell_input(rollout.path(), "git status");
        assert!(matches!(
            analyze_codex_with(&input, false, &panicking_runner),
            Analysis::Basic
        ));
        // Two owner calls with diverging prefix_rule fields (D44).
        let rollout = write_rollout(&[
            shell_turn_context("turn-1", json!("on-request"), "restricted"),
            shell_call_full("turn-1", "git status", Some(json!(["git"])), None),
            shell_call_full("turn-1", "git status", None, None),
        ]);
        let input = shell_input(rollout.path(), "git status");
        assert!(matches!(
            analyze_codex_with(&input, false, &panicking_runner),
            Analysis::Basic
        ));
    }

    #[test]
    fn sandbox_override_flag_reaches_probe() {
        let rollout = write_rollout(&[
            shell_turn_context("turn-1", json!("on-request"), "restricted"),
            shell_call_full("turn-1", "git push", None, Some("require_escalated")),
        ]);
        let input = shell_input(rollout.path(), "git push");
        let runner = |probe: &ShellProbe| {
            assert!(probe.sandbox_override);
            Some(worker_output(&[&["git", "push"]]))
        };
        let Analysis::Enhanced { memory, .. } = analyze_codex_with(&input, false, &runner) else {
            panic!("expected enhancement");
        };
        // No amendment -> only the exact session tier exists.
        assert_eq!(memory.saves.len(), 1);
        assert_eq!(memory.saves[0].action_id, ACTION_SHELL_SESSION);
    }

    #[test]
    fn missing_sandbox_fields_in_turn_context_stay_basic() {
        // The plain turn_context helper carries no sandbox fields: fail closed.
        let rollout = write_rollout(&[
            turn_context("turn-1", Some("user"), json!("on-request")),
            shell_call_full("turn-1", "git status", None, None),
        ]);
        let input = shell_input(rollout.path(), "git status");
        let panicking_runner =
            |_probe: &ShellProbe| -> Option<ShellWorkerOutput> { panic!("must not run worker") };
        assert!(matches!(
            analyze_codex_with(&input, false, &panicking_runner),
            Analysis::Basic
        ));
    }

    #[test]
    fn legacy_sandbox_policy_maps_to_kind() {
        let context = json!({
            "turn_id": "turn-1",
            "approval_policy": "on-request",
            "sandbox_policy": { "type": "danger-full-access" },
        });
        assert_eq!(
            shell_turn_fields(&context),
            Some(("on-request".to_string(), "unrestricted".to_string()))
        );
        let granular = json!({
            "turn_id": "turn-1",
            "approval_policy": { "granular": { "sandbox_approval": true } },
            "file_system_sandbox_policy": { "kind": "unrestricted" },
        });
        assert_eq!(
            shell_turn_fields(&granular),
            Some(("granular".to_string(), "unrestricted".to_string()))
        );
        assert_eq!(shell_turn_fields(&json!({ "turn_id": "turn-1" })), None);
    }

    #[test]
    fn move_patch_records_both_old_and_new_paths() {
        let patch = "*** Begin Patch\n*** Update File: old.txt\n*** Move to: new.txt\n@@\n-x\n+y\n*** End Patch\n";
        let rollout = write_rollout(&[turn_context("turn-1", Some("user"), json!("on-request"))]);
        let Analysis::Enhanced { memory, .. } =
            analyze_codex(&hook_input(rollout.path(), patch), false)
        else {
            panic!("expected enhancement");
        };
        let Some(MemoryQuery::FileEdit { paths }) = memory.query else {
            panic!("expected file query");
        };
        assert_eq!(paths, ["/work/proj/old.txt", "/work/proj/new.txt"]);
    }
}
