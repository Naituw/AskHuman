# T3 Code Compatibility & Multi-Profile Agent Integration Specification

> **Status**: Approved (2026-11-24, all D1–D6 and P1–P9 confirmed).
> **Scope**: L1 (universal base fixes) + L2 (multi-profile / extra config directory deployment) + L3 (host conflict guardrails).
> **Out of scope**: `t3 thread` control-plane integration (sending idle messages or creating new threads via T3 CLI), live testing against Cursor.

---

## 1. Background & Problem Statement

T3 Code (`t3`, desktop & web orchestrator) hosts all six agents AskHuman integrates with: **Antigravity** (`agy acp`), **Codex** (`codex app-server`), **Claude Code**, **Cursor**, **Grok** (`grok agent stdio`), and **Pi** (`pi --mode rpc`).

Running these agents inside T3 Code — or under any isolated/custom configuration directory (`GEMINI_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `GROK_HOME`, `PI_CODING_AGENT_DIR`, `CURSOR_CONFIG_DIR`) — exposes three classes of defects in AskHuman:

1. **Universal base assumptions break under non-default paths and wrappers (L1)**:
   - `paths::antigravity_brain_dirs_in` and `title::antigravity_transcript_in` only look inside `~/.gemini/{antigravity,antigravity-cli}/brain/`, and `daemon/runtime/mod.rs` only persists hook-supplied `transcript_path` for `AgentKind::Pi`. Any Antigravity session under `--gemini-dir <custom> --app-data-dir antigravity-acp` (such as T3 Code) or any other agent running under a custom config root loses titles, Watch/activity, `/transcript`, Stop last-message display, and Antigravity subagent tree folding.
   - `antigravity_hook::surface()` matches the substring `"antigravity"` anywhere in the path, misclassifying `~/.t3/userdata/providers/antigravity/<hash>/antigravity-acp/...` as `Surface::App`.
   - `detect.rs` treats `agy` as a single-session process (`walk_agent_pid` returns its PID). Because one `agy acp` process can multiplex multiple `session/new` sessions, D7 same-PID eviction can prematurely end live sessions (the same issue Codex hit with `codex app-server`).
   - T3 Code injects wrapper blocks (`<t3_code_instructions>`, `<t3_code_orchestration>`, `<runtime_info>`, `<ADDITIONAL_METADATA>`, and nested `<user_request>`) into user turns across agents, polluting session titles and `/transcript` output (and causing `is_injected_block` to drop the entire first turn in Cursor transcripts).

2. **Single-root integration misses isolated provider homes (L2)**:
   - T3 Code launches Antigravity with `--gemini-dir ~/.t3/userdata/providers/antigravity/<sha256(instanceId)> --app-data-dir antigravity-acp` and `GEMINI_HOME=<that dir>`. Its `<hash>/config/` does not inherit `~/.gemini/config/plugins/askhuman/`, so Antigravity inside T3 Code receives zero AskHuman rules, MCP server declarations, or lifecycle/Stop hooks.
   - Users running multiple profiles or isolated config roots for Codex (`CODEX_HOME`), Claude (`CLAUDE_CONFIG_DIR`), Grok (`GROK_HOME`), Pi (`PI_CODING_AGENT_DIR`), Cursor (`CURSOR_CONFIG_DIR`), or Antigravity (`GEMINI_HOME`) currently have no way to sync AskHuman integrations to extra config directories without manually copying files.

3. **Host orchestration conflicts when T3 Code owns delegation and RPC control (L3)**:
   - T3 Code spawns delegated child threads (`delegate_task` → child thread with `parentThreadId != null`) using the same provider config directory as interactive parent threads. Static rule files, MCP tools, Stop confirmation, PermissionRequest, Claude `AskUserQuestion` takeover, failure cards, and lifecycle notifications would otherwise fire on unattended child threads.
   - Pi's managed extension (`resources/integrations/pi/index.ts`) runs `startControlPolling()` unconditionally whenever `lifecycle || stop` is enabled, calling `pi.sendUserMessage()` while idle. In `pi --mode rpc` (which T3 Code drives over stdio JSON-RPC), `ctx.hasUI` remains `true`, so background `sendUserMessage()` calls race with T3 Code's RPC session state.

---

## 2. Empirical Evidence (2026-11-24)

All probes below were executed in isolated `/tmp` directories (cleaned up afterward) with zero mutations to user config files and zero Cursor executions.

| ID | Agent / Host | Verified Fact | Confidence |
| --- | --- | --- | --- |
| E1 | T3 Code → Antigravity | T3 launches `agy acp --gemini-dir ~/.t3/userdata/providers/antigravity/<hash> --app-data-dir antigravity-acp` with `GEMINI_HOME=<hash>`, `AGY_ACP_FORCE_FILE_STORAGE=1`, `T3CODE_THREAD_ID`, `T3CODE_CLI_PATH`. No `--sandbox` flag; CLI `AskHuman` reaches `~/.askhuman/daemon.sock`. | Verified |
| E2 | T3 Code Antigravity dir | `<hash>/config/` contains `config.json`, `mcp_config.json` (empty), `projects/`, and symlink `skills -> ~/.gemini/config/skills`. No `plugins/`, `rules/`, or `hooks.json`. | Verified |
| E3 | `agy acp` plugin loading | Real directory `<gemini_root>/config/plugins/askhuman/` loads `rules/AGENTS.md`, `mcp_config.json` (`askhuman_mcp`), and `hooks.json` without `trustedWorkspaces`. | Verified (isolated `/tmp` `agy acp`, Flash Low) |
| E4 | `agy acp` MCP merge | ACP `session/new` `mcpServers` (e.g. `t3-code`) **merges** with plugin `mcp_config.json`; neither overwrites the other. | Verified |
| E5 | `agy acp` MCP `_meta` | `tools/call` `params._meta` includes `antigravity.google/conversation_id`, `antigravity.google/root_conversation_id`, and `antigravity.google/artifacts_dir` (`<gemini_root>/antigravity-acp/brain/<uuid>`). | Verified |
| E6 | `agy acp` hooks | `SessionStart`, `PreInvocation` (`injectSteps` injection observed), and `Stop` (`decision: "continue"` observed) all fire without `trustedWorkspaces`, providing `conversationId`, `cwd`, `workspacePaths`, `modelName`, and full `transcriptPath`. | Verified |
| E7 | `agy acp` multiplexing | One `agy acp` process can create and host multiple `session/new` sessions under the same PID. | Verified |
| E8 | `grok agent stdio` | Speaks ACP JSON-RPC; runs explicit `phase: "mcp_merge"` merging `$GROK_HOME/config.toml` MCP servers with session `mcpServers`. `GROK_HOME` redirects `skills/`, `config.toml`, and `hooks/`. | Verified (zero-LLM) |
| E9 | `codex app-server` | `CODEX_HOME=<dir> codex app-server --listen stdio://` redirects `hooks.json` (trust keys use canonical `<CODEX_HOME>/hooks.json:...`) and `config.toml` MCP. | Verified (zero-LLM) |
| E10 | `pi --mode rpc` | `PI_CODING_AGENT_DIR=<dir> pi --mode rpc --session-dir <dir>` loads `<dir>/extensions/`. **`ctx.hasUI` is STILL `true` in `--mode rpc`**, so RPC detection must inspect `process.argv` / `process.env.T3CODE_THREAD_ID`, never `ctx.hasUI`. | Verified (zero-LLM) |
| E11 | T3 Code thread metadata | `T3CODE_THREAD_ID` and `T3CODE_CLI_PATH` (`/Users/wutian/.t3/bin/t3`) are present in child env. `t3 thread show --json --messages 0 <id>` completes in ~263 ms and returns `thread.parentThreadId`, `thread.relationshipToParent`, `thread.createdBy`, `thread.providerInstanceId`, `thread.title`. | Verified |
| E12 | T3 prompt wrappers | Observed in real Antigravity transcript: `<USER_REQUEST>\n<t3_code_instructions>...</t3_code_instructions>\n\n<user_request>...</user_request><runtime_info>...</runtime_info>\n</USER_REQUEST>`. Observed in real Cursor SDK transcript: `<timestamp>...</timestamp>\n<user_query>\n<t3_code_orchestration>...`. | Verified (read-only transcript inspection) |
| E13 | Unverified items | (a) Whether T3 Code preserves `<hash>/config/plugins/askhuman/` across T3 app restarts/updates (expected yes because `<hash>/config/` holds persistent user config, to be confirmed in Phase 5); (b) live UI behavior of Claude Code / Pi inside T3 Code desktop UI; (c) Cursor live runs (explicitly excluded by D2). | Explicitly unverified |

---

## 3. Decisions

### 3.1 Confirmed Owner Decisions (D1–D6)

| ID | Decision |
| --- | --- |
| **D1** | **Scope**: Implement L1 (universal base fixes) + L2 (multi-profile / extra config directory deployment) + L3 (host conflict guardrails). Exclude `t3 thread` control plane (idle messaging / `/new` creating T3 threads) from this phase. |
| **D2** | **No live Cursor tests**: Never launch Cursor or run live Cursor model probes. Cursor receives only generic L1 prompt cleaning and L2 extra config directory plumbing covered by unit tests. |
| **D3** | **Discovery + explicit opt-in**: Auto-discover T3 Code Antigravity isolated directories (`~/.t3/userdata/providers/antigravity/<hash>`) and show them in Settings / CLI, requiring explicit user opt-in before writing anything. Newly discovered directories on future logins only surface a hint — **never silently write into `~/.t3/`**. All 6 agents support manually adding extra config directories. |
| **D4** | **Shared mode & capability model**: An agent's primary directory and all enabled extra profiles share one integration mode (`None / CLI / MCP`) and one set of capability toggles (`lifecycle`, `stop`, `permission`, `askQuestion`, `collaborationStyle`). Each profile maintains independent ownership ledgers and conflict/status reporting. Antigravity extra profiles always publish real managed plugin directories (never symlinks) under strict ownership rules. |
| **D5** | **Host guardrails**: Silence T3 delegated child threads (`parentThreadId != null`) across all 6 agents, and disable Pi RPC `startControlPolling()` / background `pi.sendUserMessage()` when externally controlled (`--mode rpc` / `T3CODE_THREAD_ID`). Main T3 threads retain AskHuman rules, Claude `AskUserQuestion` takeover, Claude/Codex `PermissionRequest`, and Stop confirmation. |
| **D6** | **Workflow & execution location**: Author spec + phased implementation plan first and submit via `AskHuman` for approval before coding. Perform all development in the isolated Dev Worktree `/Users/wutian/Developer/HumanInLoop-t3-multi-profile` (`feat/t3-multi-profile`). |

### 3.2 Proposed Default Decisions for Approval (P1–P9)

| ID | Topic | Proposed Default | Rationale / Alternative |
| --- | --- | --- | --- |
| **P1** | Antigravity extra profile root | Store the `GEMINI_HOME` root (e.g. `~/.t3/userdata/providers/antigravity/<hash>`), not the inner `/config` directory. If a user manually pastes `<root>/config` where `<root>` is an Antigravity home (under `~/.t3/userdata/providers/antigravity/<hash>/config` or sibling `antigravity*` dir exists), normalize to `<root>` and display the normalized root. | One root cleanly derives `config/plugins/askhuman`, `config/config.json` (`host_view`), `<surface>/brain/<uuid>/...`, and `<surface>/conversation_summaries.db`. |
| **P2** | Missing extra profile root on disk | Refuse to `mkdir -p` a missing extra profile root. Report `rootMissing`, skip writing it without blocking other profiles, and offer one-click Remove in Settings / `agents profile remove` in CLI. Inside an *existing* root, creating `config/plugins/askhuman` (or `hooks/`, `rules/`, `extensions/`) is allowed as normal. | Prevents creating zombie directories inside `~/.t3/` after T3 deletes a provider instance. |
| **P3** | Disable / Remove / `Mode::None` cleanup | Disabling or removing an extra profile uninstalls AskHuman-managed artifacts in that profile via its own ledger (preserving user content). On strict ownership conflict, refuse deletion and offer explicit detach (`--detach` / "Forget without deleting"). Switching the agent to `Mode::None` uninstalls artifacts from primary + all enabled extra profiles while **keeping** the saved profile list in `agent-profiles.json`. `agents cleanup` cleans primary + all enabled extra profiles. | Safe by default (never destroys user edits), convenient when toggling `Mode::None` → `Mode::Mcp` without re-adding profiles. |
| **P4** | Partial failure & conflict isolation | Reconcile primary first, then each enabled extra profile independently. One failing/missing/conflicting extra profile never blocks primary or other healthy extra profiles. Preference toggles (`stop`, `permission`, `askQuestion`, `lifecycle`) roll back only if primary fails; if primary succeeds and an extra profile fails, retain the preference and report per-profile errors. | A stale or hand-edited T3 directory cannot brick the user's primary Agent setup. |
| **P5** | T3 Antigravity discovery & dismiss | Read-only scan of `~/.t3/userdata/providers/antigravity/*` (bounded to 64 entries; requires directory containing `config/` or `antigravity-acp/`). Un-enrolled entries appear in Antigravity Settings card + `agents profile list antigravity` + `doctor` with `Enable` and `Ignore` (`dismissed` list in `agent-profiles.json`). Ignored roots stop hinting unless manually re-added or un-dismissed. | Clear visibility without nagging users who intentionally keep an isolated test login unmanaged. |
| **P6** | L3 T3 child-thread silence scope & Codex delegated toggle | Silence **all** AskHuman surfaces in a T3 delegated child thread (`parentThreadId != null || relationshipToParent != null`): protocol/recovery injection, CLI ask/`--whats-next`/`--show-last`/todo-write, MCP ask/`whats_next`/`show_last`/`todo_add`/`todo_update`, Stop confirmation, PermissionRequest, Claude `AskUserQuestion`, failure cards, and lifecycle/interjection reporting. Always active even if `agents delegated codex on` is enabled (that toggle specifically governs CLI `codex exec` / plugin runs). Fail-open on T3 CLI error/timeout (1.5 s cap, 24 h cache, 30 s transient error cache). | Matches D5 + `codex-delegated-run-silence` D1 ("behave as if not integrated"), prevents child-thread cards/IM spam, and never delays or breaks interactive threads when `t3` is unavailable. |
| **P7** | Pi `--mode rpc` Stop continuation | Always disable `startControlPolling()` when `--mode rpc` / `--mode=rpc` or `T3CODE_THREAD_ID` is present. For `agent_settled` Stop confirmation under `--mode rpc`: **also disable** `pi.sendUserMessage()` continuation (still report `turn-end` cleanly) unless Phase 5 verifies that T3 Code's Pi RPC adapter cleanly handles post-`agent_end` extension-initiated user messages. | Avoids unverified RPC state-desync risk from extension-injected turns after T3 already received `agent_end` (`AGENTS.md`: no unverified workarounds). |
| **P8** | Workspace seeding from extra Antigravity profiles | Extend `agents/antigravity_workspaces.rs` to also inspect `<root>/{antigravity,antigravity-cli,antigravity-acp}/conversation_summaries.db` (read-only immutable SQLite, best-effort) for enabled extra Antigravity profiles. | Lets T3 Antigravity workspaces appear alongside App/`agy` recent folders with zero write risk. |
| **P9** | Protocol & storage compatibility | Keep IPC `PROTOCOL_VERSION` unchanged (6) and SQLite recovery `SCHEMA_VERSION` unchanged. Persist extra profiles in `~/.askhuman/agent-profiles.json`, extra Antigravity ledgers in `~/.askhuman/antigravity-plugin-profiles/<id>.json`, Codex extra `direct_only` ownership inside existing `integration-state.json`, and T3 caches/markers in `~/.askhuman/state/{t3-threads,t3-delegated}/`. | Zero breaking protocol/schema migrations; fully backward compatible with existing `config.json` and daemon recovery DBs. |

---

## 4. Detailed Design

### 4.1 L1 — Universal Base Fixes

#### 4.1.1 Generalized Transcript Path Registration & Resolution

Currently, `src-tauri/src/agents/session_paths.rs` only supports `AgentKind::Pi`, `daemon/runtime/mod.rs:2330` only registers `transcript_path` for `Pi`, `registry.rs:335` only reloads `Pi` paths on restart, and `report.rs:184` (`report_simple_event`) only forwards `PI_SESSION_FILE`.

**Changes**:
1. **Generalize `agents/session_paths.rs`**:
   - Keep `register_pi(session_id, raw_path, cwd)` as the Pi validator and add `register(kind, session_id, raw_path, cwd) -> Option<String>` + pure validator `validate_path(kind, session_id, raw_path, cwd) -> Option<PathBuf>`.
   - Strict structural validation per agent (so untrusted hook payloads can never redirect AskHuman to arbitrary files):
     - **Antigravity**: `session_id` parses as canonical UUID; `raw_path` canonicalizes to a regular file owned by current UID (Unix); path suffix is `brain/<canonical-uuid>/.system_generated/logs/transcript_full.jsonl` (case-insensitive UUID match normalized to hyphenated lowercase); parent of `brain` is a valid surface dir (not `antigravity-ide`).
     - **Claude**: canonical regular file ending with `<session_id>.jsonl` (validating `session_id` has no path separators / `..`).
     - **Codex**: canonical regular file whose filename ends with `-<session_id>.jsonl` and starts with `rollout-` (and optional first-line `session_meta` check when present).
     - **Cursor**: canonical regular file whose filename is `<session_id>.jsonl` under an `agent-transcripts/<session_id>/` parent (or canonical under primary `~/.cursor` or an enabled extra Cursor profile root).
     - **Grok**: canonical regular file ending with `<session_id>/chat_history.jsonl`.
     - **Pi**: existing v3 JSONL header validation (`type == "session"`, `version == 3`, `id == session_id`, equivalent `cwd`).
   - All entries share the bounded LRU cache (`MAX_CACHED_PATHS = 1024`, keyed by `(AgentKind, session_id)`).
2. **Daemon ingest & restart restore**:
   - In `daemon/runtime/mod.rs` (around line 2330): register validated `transcript_path` via `session_paths::register(kind, &session_id, path, event_cwd.as_deref())` for **all** `AgentKind` variants, persisting via `state.agents.set_transcript_path(kind, &session_id, path)`.
   - In `agents/registry.rs` (`restore`, around line 334): re-register persisted `record.transcript_path` into `session_paths` for **all** `AgentKind` variants.
   - In `agents/report.rs` (`report_simple_event`, around line 184): pass `resolve_transcript_path(&detect::env_snapshot(), Some(input))` instead of `PI_SESSION_FILE` only.
3. **Lookup order in `agents/title.rs` (`transcript_path` & `resolve_title`)**:
   - Order:
     1. `development::session::transcript_for(kind, session_id)` (unchanged — Dev test profile isolation always wins).
     2. `session_paths::get(kind, session_id)` (validated runtime path from hook or restart restore).
     3. Primary agent root (`~/.gemini/{antigravity,antigravity-cli,antigravity-acp}/brain/...`, `~/.claude/projects/...`, `~/.codex/sessions/...`, `~/.cursor/projects/...`, `~/.grok/sessions/...`, `~/.pi/agent/sessions/...`).
     4. Enabled extra profile roots for `kind` (from `agent-profiles.json`, read-only).
   - This automatically fixes:
     - `title.rs` (`resolve_title`, `antigravity_title`, `claude_title`, `codex_title`, `grok_title`, `cursor_title`, `pi_title`)
     - `activity.rs` (Watch / live activity)
     - `transcript_full.rs` (`/transcript`)
     - `daemon/runtime/antigravity_tree.rs` (`Trees::transcript`)
4. **Short-lived Stop hook process (`agents/stop.rs`)**:
   - `__stop-hook` runs outside the daemon, so it does not share the daemon's in-memory `session_paths` cache.
   - `antigravity_last_message(input)` first checks `input["transcriptPath"]` via `session_paths::validate_path(AgentKind::Antigravity, id, raw_path, None)`, then falls back to `title::transcript_path(AgentKind::Antigravity, id)` (which also searches primary + enabled extra Antigravity roots).
   - `cursor_last_message(input)` accepts `canonical_under(path, root)` for primary `~/.cursor` OR any enabled extra Cursor profile root.
5. **Antigravity MCP `_meta` artifacts_dir hint (optional fallback)**:
   - `mcp/ask.rs` already reads `antigravity.google/conversation_id` and `antigravity.google/root_conversation_id`. When `antigravity.google/artifacts_dir` is present in `_meta`, `show_last` / transcript resolution can derive `<artifacts_dir>/.system_generated/logs/transcript_full.jsonl` and validate it via `session_paths::validate_path` as a fallback when lifecycle hooks are disabled.

#### 4.1.2 Antigravity `Surface` Classification

In `src-tauri/src/agents/antigravity_hook.rs`:
- Extend `Surface`:
  ```rust
  pub enum Surface {
      App,
      Cli,
      Acp,
      Ide,
      Unknown,
  }
  ```
- Inspect path components cleanly: prefer the component immediately preceding `"brain"` when `"brain"` is present (e.g. `.../antigravity-acp/brain/...` → `Surface::Acp`, `.../antigravity-cli/brain/...` → `Surface::Cli`, `.../antigravity-ide/brain/...` → `Surface::Ide`, `.../antigravity/brain/...` → `Surface::App`), falling back to prioritized scan (`antigravity-ide` → `antigravity-acp` → `antigravity-cli` → `antigravity`) when `"brain"` is absent.
- `report_for` continues to ignore `Surface::Ide` only; `Surface::Acp` is reported like `App` and `Cli`.
- `antigravity_desktop/message.rs` remains strictly App-only (`~/.gemini/antigravity/brain/`), never touching `antigravity-acp` or extra profiles.

#### 4.1.3 Shared `agy acp` Process PID Exclusion

In `src-tauri/src/agents/detect.rs`:
- Add `is_shared_agy_acp(entry: &ProcEntry) -> bool`, analogous to `is_shared_app_server` (line 181):
  - After a token whose basename is `agy` or `agy.exe`, skip leading flags (handling `=` and value-taking global flags if any), and check whether the first positional subcommand token is `"acp"`.
- In `walk_agent_pid`, hitting `is_shared_agy_acp` for `AgentKind::Antigravity` returns `None` immediately (no session PID attached), preventing D7 same-PID eviction across concurrent `session/new` conversations hosted by one `agy acp` process.
- Standalone `agy` (interactive CLI) still resolves its own PID as before.

#### 4.1.4 T3 Code Prompt Wrapper Cleaning

In `src-tauri/src/agents/transcript_full.rs` (shared with `title.rs`):
- Add a pure helper `strip_host_wrappers(text: &str) -> String`:
  1. Unwrap outer `<USER_REQUEST>...</USER_REQUEST>` (Antigravity) and `<user_query>...</user_query>` (Cursor/Grok) when present.
  2. Strip known paired host-injected XML-like metadata blocks (non-greedy, repeated):
     - `<t3_code_instructions>...</t3_code_instructions>`
     - `<t3_code_orchestration>...</t3_code_orchestration>`
     - `<runtime_info>...</runtime_info>`
     - `<ADDITIONAL_METADATA>...</ADDITIONAL_METADATA>`
     - `<system-reminder>...</system-reminder>`
  3. Unwrap inner `<user_request>...</user_request>` if present after stripping wrapper blocks.
  4. Trim whitespace; if empty, return empty so injected-only blocks are still skipped.
- Apply in:
  - `transcript_full::clean_user` (automatically covers `title::first_user_message` for Cursor, Claude, Codex, Pi, and `/transcript` rendering).
  - `title::antigravity_first_request` and `transcript_full`'s Antigravity `USER_INPUT` parser.
  - `title::grok_first_query`.
- Preserve safety: only strip exact paired tags from the explicit allowlist above — never strip arbitrary user-written `<tag>` text like HTML/JSX snippets.

---

### 4.2 L2 — Multi-Profile / Extra Config Directory Deployment

#### 4.2.1 Storage Schema (`~/.askhuman/agent-profiles.json`)

New module `src-tauri/src/integrations/agent_profiles.rs`:
- File path: `paths::agent_profiles_file()` = `paths::config_dir().join("agent-profiles.json")` (isolated automatically per Dev Instance via `ASKHUMAN_HOME`).
- Schema (versioned, additive, preserves unknown fields):
  ```json
  {
    "version": 1,
    "profiles": [
      {
        "id": "ag-7f8a9b1c2d3e4f50",
        "agent": "antigravity",
        "root": "/Users/wutian/.t3/userdata/providers/antigravity/ac0a3dfd...",
        "label": "T3 Code (ac0a3dfd)",
        "source": "t3_discovered",
        "enabled": true,
        "createdAt": 1795444600
      }
    ],
    "dismissed": [
      {
        "agent": "antigravity",
        "root": "/Users/wutian/.t3/userdata/providers/antigravity/deadbeef...",
        "dismissedAt": 1795444600
      }
    ]
  }
  ```
- **Stable profile ID**: `id` is derived deterministically from `(agent, canonical_or_normalized_root)` via `format!("{}-{}", agent.as_str(), &sha256(canonical_root)[..16])` (safe ASCII filename segment for per-profile ledgers and CLI targeting; unique per `(agent, root)`).
- **Root validation on add**:
  - Must be an absolute path, no NUL bytes, no `..` traversal components after normalization.
  - Must exist as a directory on disk at add time.
  - Must not equal the agent's primary root (canonicalized comparison via `path_identity::equivalent`), nor duplicate an existing profile for the same agent.
  - For Antigravity (P1): if the supplied path ends with `/config` and its parent is an Antigravity root (under `~/.t3/userdata/providers/antigravity/*` or has sibling `antigravity*` dir), normalize to the parent directory and return the normalized root.
- **Dev Test Profile Isolation**:
  - Whenever `development::session::directory_for(kind)` is active (a Dev Test Profile), `agent_profiles` returns **no extra profiles** — Dev tests stay 100% hermetic in their single test directory.

#### 4.2.2 T3 Code Antigravity Auto-Discovery (Read-Only)

In `agent_profiles.rs`:
- `discover_t3_antigravity_in(home: &Path) -> Vec<DiscoveredProfile>`:
  - Inspects `<home>/.t3/userdata/providers/antigravity` (Unix only, matching Antigravity's Unix-only integration).
  - Read-only `read_dir`, bounded to at most 64 entries.
  - Accepts entries whose symlink-free metadata is a real directory owned by the current user AND containing either `config/` or `antigravity-acp/` subdirectory.
  - Canonicalizes each candidate root via `physical_root`; skips any root already present in `profiles` (enabled or disabled) or in `dismissed`.
  - Derives a human-friendly default label: `"T3 Code (<first 8 chars of dir name>)"`, plus optional secondary hint if `<root>/antigravity-acp/brain` has recent mtime.
  - **Never creates or modifies anything inside `~/.t3/`**.
- Discovery runs on read (`agent_mode_status`, `agents profile list`, `agents show`, `doctor`) — strictly fast local `readdir` + `stat`, zero subprocesses.

#### 4.2.3 Scoped Root Dispatch (`paths::with_agent_root`)

To avoid duplicating 6 agents' worth of rule/MCP/hook/extension writers, `src-tauri/src/paths.rs` provides a thread-local RAII scope guard:

```rust
pub(crate) fn with_agent_root<T>(
    kind: AgentKind,
    root: &Path,
    f: impl FnOnce() -> T,
) -> T
```

- **Priority order** inside `cursor_dir()`, `claude_dir()`, `claude_json()`, `codex_dir()`, `grok_dir()`, `pi_agent_dir()`:
  1. `crate::development::session::directory_for(kind)` (highest priority — Dev test isolation can never be overridden).
  2. Thread-local active override for `kind` (if set by `with_agent_root`).
  3. Default primary path under `integration_home()`.
- **Claude `.claude.json` rule**: When `with_agent_root(AgentKind::Claude, root, ...)` is active, `claude_dir()` returns `root` AND `claude_json()` returns `root.join(".claude.json")` (matching Claude Code's `CLAUDE_CONFIG_DIR` contract and `paths.rs:327-329`).
- **Panic safety**: RAII `Drop` restores the previous thread-local slot even on panic, supporting clean nesting if ever needed.
- **Canonicalization**: `with_agent_root` stores `agent_rules::physical_root(root.to_path_buf())` so Codex hook trust keys (`<canonical_root>/hooks.json:...`) always match what `codex` computes on macOS (`/var` vs `/private/var`, symlinks).
- **Codex `direct_only` ownership**: `mcp_config.rs` detects whether a thread-local Codex extra root is active (via `paths::active_agent_root(AgentKind::Codex)`). Primary reads/writes `state.codex.direct_only_namespace_added_by_askhuman`; extra profiles read/write `state.codex.extra_profiles[profile_id].direct_only_namespace_added_by_askhuman` inside `integration-state.json`.
- **Antigravity plugin layout**: `antigravity_plugin.rs` adds `Layout::for_root(gemini_root: &Path, ledger: PathBuf)` where:
  - `config = physical_root(gemini_root).join("config")`
  - `dir = config.join("plugins/askhuman")`
  - `ledger = paths::antigravity_extra_plugin_ledger_in(&askhuman_config, profile_id)` (`~/.askhuman/antigravity-plugin-profiles/<profile_id>.json`)
  - Reuses the exact same strict staging, atomic rename, SHA-256 ledger, conflict detection, and `host_view_at(&layout.config.join("config.json"))`.
  - Never touches `earlier_antigravity_left()` / standalone legacy rule retirement on extra profiles (only primary migrates legacy standalone rules).

#### 4.2.4 Shared Mode Reconciliation & Lifecycle Across All Entry Points

All mutation entry points hold `IntegrationMutationLock` and reconcile **primary first, then each enabled extra profile** (for that agent):

1. `agent_mode::set_with_env(target, mode, env)`
2. `agent_mode::update(target)`
3. `agent_mode::update_artifact(target, artifact)`
4. `agent_lifecycle::install(kind)` / `agent_lifecycle::uninstall(kind)`
5. `agent_stop::set_enabled(kind, enabled)` / `agent_stop::migrate_outdated()`
6. `agent_permission::set_enabled(target, enabled)`
7. `agent_ask_question::set_enabled(kind, enabled)` / `agent_ask_question::migrate_outdated()`
8. `commands::save_collaboration_style` / `commands::collaboration_style_apply_integrations` (re-renders rules/plugins for primary + enabled extra profiles)
9. `agents_cmd::cleanup` (uninstalls primary + all enabled extra profiles)

**Reconciling a single extra profile**:
- Pre-check: if `!profile.root.is_dir()` → return `ProfileError::RootMissing` (do NOT create the root, P2).
- When `mode == Mode::None`: uninstall managed artifacts in that profile (leaving user files intact).
- When `mode == Mode::Cli | Mode::Mcp`: reconcile the exact same mode + capability preferences (`lifecycle`, `stop`, `permission`, `askQuestion`) into that profile.
- **Per-profile status (`ExtraProfileStatus`)**:
  ```rust
  pub struct ExtraProfileStatus {
      pub id: String,
      pub agent: String,
      pub root: String,
      pub label: String,
      pub source: String,              // "t3_discovered" | "manual"
      pub enabled: bool,
      pub state: String,               // "synced" | "outdated" | "notInstalled" | "conflict" | "rootMissing" | "disabled"
      pub mode: String,                // observed on-disk mode in this profile ("none"|"cli"|"mcp")
      pub needs_update: bool,
      pub error: Option<String>,       // conflict reason or IO error
      pub host_plugin_disabled: bool,  // Antigravity config.json plugins.askhuman.enabled == false
      pub mcp_grant: Option<bool>,     // Antigravity config.json mcp allow grant (MCP mode)
  }
  ```
- **Enabling / disabling / removing a profile**:
  - `profile_add(agent, root, label, source, enable)` → validates root, saves entry, removes from `dismissed` if present, and if `enable && primary_mode != Mode::None`, reconciles this profile immediately.
  - `profile_set_enabled(agent, id, enabled, detach_on_conflict)`:
    - `enabled = true`: marks enabled; if `primary_mode != Mode::None`, reconciles immediately (if root missing or conflict, returns error and keeps `enabled` state accurate or rolls back on add — on toggle from disabled→enabled, if root is missing or conflicts, keep disabled and return actionable error so user isn't stuck with a broken enabled toggle).
    - `enabled = false`: if root exists, uninstalls managed artifacts in that profile first. On conflict, refuses unless `detach = true`; when `detach = true`, removes AskHuman's ledger for that profile (if Antigravity) without modifying disk files and marks disabled.
  - `profile_remove(agent, id, detach)`: same safe uninstall semantics as disabling, then deletes the entry from `agent-profiles.json` (and cleans up its Antigravity ledger file if removed cleanly or detached).
  - `profile_sync(agent, id)`: explicitly reconciles one enabled extra profile to the agent's current primary mode and preferences.
  - `profile_dismiss_discovered(agent, root)`: records `(agent, canonical_root)` in `dismissed`.

#### 4.2.5 CLI Surface (`AskHuman agents profile ...` & `doctor` & `agents show`)

Add `agents profile` subcommand group in `src-tauri/src/cli/agents_cmd.rs`:
- `AskHuman agents profile list [<agent>] [--json]` — lists configured extra profiles (with status) and newly discovered T3 Antigravity directories.
- `AskHuman agents profile add <agent> <path> [--label <name>] [--disabled]` — validates & enrolls a custom config directory (immediately syncs if agent mode != None and not `--disabled`).
- `AskHuman agents profile enable <agent> <id|path>` — enables a saved or discovered profile and syncs it to the agent's current mode.
- `AskHuman agents profile disable <agent> <id|path> [--detach]` — cleanly uninstalls managed artifacts from that profile and disables it.
- `AskHuman agents profile remove <agent> <id|path> [--detach]` — uninstalls (or detaches on `--detach`) and deletes the profile entry.
- `AskHuman agents profile sync <agent> [<id|path>]` — reconciles one or all enabled extra profiles for `<agent>`.
- `AskHuman agents profile dismiss <agent> <path>` — ignores a discovered directory.
- `AskHuman agents show [<agent>]` and `AskHuman doctor [--json]`:
  - Include extra profiles summary (count, per-profile sync/conflict/missing status, and count of un-enrolled discovered T3 Antigravity directories).
  - In `doctor`, an enabled extra profile in `conflict` or `rootMissing` reports a clear diagnostic warning/error; an un-enrolled discovered T3 directory reports an informational note (never exits non-zero on its own).

#### 4.2.6 Settings UI & Tauri Commands

- **Tauri commands** (`src-tauri/src/commands.rs` + `src-tauri/src/app/invoke.rs`):
  - Extend `AgentModeStatus` with two additive fields:
    - `profiles: Vec<ExtraProfileStatus>`
    - `discovered_profiles: Vec<DiscoveredProfileInfo>`
  - New commands:
    - `agent_profile_add(app, agent, root, label, enabled)`
    - `agent_profile_set_enabled(app, agent, id, enabled, detach)`
    - `agent_profile_remove(app, agent, id, detach)`
    - `agent_profile_sync(app, agent, id)`
    - `agent_profile_dismiss_discovered(app, agent, root)`
    - `agent_profile_reveal(agent, id)` (reveals the profile's config/plugin folder in Finder/Explorer)
    - `pick_directory()` (reuse existing folder dialog command if present, or add a minimal directory picker)
- **Settings → Agents UI (`src/views/settings/IntegrationTab.vue`, `useIntegration.ts`, `agentCard.ts`, `AgentProfilesSection.vue`)**:
  - Keep card headers clean (`docs/specs/settings-agents-channels-cards.md`):
    - If any enabled extra profile has `needsUpdate`, `hasUpdate` becomes `true` (so the card's `needsUpdate` tag and top "Update all" overview naturally include it!).
    - If any enabled extra profile has `conflict` or `rootMissing`, surface a `profileIssue` warning tag on the card header and auto-expand details via `needsAction`.
    - Keep unintegrated (`mode === "none"`) and folded cards free of extra hint text outside `Details` (`docs/specs/settings-agents-channels-cards.md` D13); discovered T3 directories live strictly inside `AgentProfilesSection.vue` under `Details`, reverse-mapping T3 `providerInstances` names (e.g. `T3 Code (antigravity)`) and collapsing `$HOME` to `~` in display paths (with full path on hover).
  - Inside each agent's **Details** section, add a dedicated `AgentProfilesSection.vue` subsection ("Extra config directories"):
    - Lists discovered (un-enrolled) T3 directories with `Enable` and `Ignore` buttons.
    - Lists configured extra profiles with enable/disable toggle, status badge (`Synced` / `Needs update` / `Conflict` / `Folder missing` / `Disabled`), path, Antigravity `hostPluginDisabled` / `mcpGrant` per-profile notes, quick actions (`Sync` / `Reveal` / `Remove`, plus `Forget without deleting` if conflict).
    - `+ Add directory` action (folder picker + manual path input + optional label) available for **all 6 agents**.
  - Full `zh.ts` + `en.ts` i18n coverage.

---

### 4.3 L3 — Host Conflict Guardrails

#### 4.3.1 T3 Code Delegated Child Thread Detection (`src-tauri/src/agents/t3_host.rs`)

New focused module `src-tauri/src/agents/t3_host.rs`:

1. **Trigger**: Check `T3CODE_THREAD_ID` in the environment. If unset or empty → immediately return `T3ThreadState::NotT3` (zero I/O, zero subprocesses for non-T3 users!).
2. **Thread ID validation**: Reject IDs longer than 128 chars or containing anything outside `[A-Za-z0-9_-]` (fail-open as `Unknown`).
3. **Disk cache (`~/.askhuman/state/t3-threads/<sha256(thread_id)>.json`)**:
   - Stores `{ "threadId", "delegated": bool, "parentThreadId": Option<String>, "relationshipToParent": Option<String>, "checkedAtMs": u64, "transientError": bool }`.
   - Valid TTL: **24 hours** for conclusive answers (`transientError == false`); **30 seconds** for transient errors (`transientError == true`, preventing repeated 1.5 s timeouts on every hook invocation if `t3` daemon is momentarily unresponsive).
   - Opportunistic pruning (same pattern as `delegated.rs`: scan at most 200 entries on write, prune files older than 7 days).
4. **CLI query on cache miss**:
   - Locate `t3` binary: `T3CODE_CLI_PATH` env if set, absolute, and an existing regular file; otherwise fallback to `<system_account_home>/.t3/bin/t3`.
   - Spawn `<t3> thread show --json --messages 0 <thread_id>` with `stdin(null)`, captured stdout/stderr, bounded output (256 KiB), and hard timeout **1,500 ms** (kill child on timeout).
   - Parse JSON `thread`:
     ```rust
     let delegated = thread.get("parentThreadId")
         .and_then(Value::as_str)
         .is_some_and(|s| !s.trim().is_empty())
         || thread.get("relationshipToParent").is_some_and(|v| !v.is_null());
     ```
   - Deliberately does **NOT** treat top-level threads with `createdBy == "agent"` and `parentThreadId == null` as delegated.
   - Any failure (missing binary, non-zero exit, invalid JSON, timeout) records a 30 s transient-error cache entry and **fails open** (`delegated = false`).
5. **Session-keyed T3 delegated marker (`~/.askhuman/state/t3-delegated/<sha256(kind:session_id)>.json`)**:
   - Why needed in addition to env lookup:
     - The daemon (`failure_confirmation.rs`) never inherits a child thread's `T3CODE_THREAD_ID`.
     - Shared MCP server processes (e.g. a shared `agy acp` or `codex app-server` MCP server) may not inherit a child thread's `T3CODE_THREAD_ID`, whereas hooks (`SessionStart`, `PreInvocation`, `Stop`) DO run in the child's context and carry `(kind, session_id)`.
   - API (injectable `Probes` trait for deterministic unit tests, mirroring `agents/delegated.rs`):
     - `record_session(kind, session_id)` on hook entry when `T3CODE_THREAD_ID` is present:
       - If `delegated == true` → write marker `{ kind, session_id, thread_id, parent_thread_id, ts_ms }` (14-day TTL pruning).
       - If `delegated == false` (confirmed main interactive T3 thread) → clear any stale marker for `(kind, session_id)`.
     - `silence_reason(kind, session_id: Option<&str>) -> Option<String>`:
       1. If `T3CODE_THREAD_ID` is present in current process env:
          - If `delegated == true` → return `Some("t3_delegated_child_thread")`.
          - If `delegated == false` → do **not** trust env alone if shared process? Check marker for `(kind, session_id)` too! (Wait: for a confirmed non-delegated session, `record_session` cleared its marker; for a child session sharing a parent's process env, the child's own marker is present and catches it!)
       2. Otherwise, if `session_id` is non-empty and a valid marker exists for `(kind, session_id)` → return `Some("t3_delegated_marker")`.
       3. Otherwise `None` (interactive).

#### 4.3.2 Guard Coverage Across All AskHuman Surfaces (All 6 Agents)

| Surface | File | Behavior when T3 delegated child thread |
| --- | --- | --- |
| Codex / Claude `SessionStart` protocol & compact recovery | `agents/context_recovery.rs` | Records marker, emits no output, logs `askhuman_guard` (`component: "protocol_hook"`). |
| Lifecycle & Interjection hooks (Claude/Codex/Cursor/Grok/Pi) | `agents/report.rs` | Records marker on `SessionStart` (or first event); logs `askhuman_guard` on `SessionStart`; skips reporting event to daemon and skips `interject_poll`. |
| Antigravity hooks (`SessionStart`, `PreInvocation`, `Stop` track) | `agents/antigravity_hook.rs` | Records marker; skips reporting to daemon and skips `PreInvocation` interjection; emits empty stdout. |
| Stop confirmation (all 6 agents) | `agents/stop.rs` | Checked in `confirmation_suppression_reason` for **all** `AgentKind` (not just Codex). Logs `askhuman_guard` (`component: "stop_confirmation"`), emits empty/`{}` without raising card or reporting turn-end. |
| PermissionRequest (Claude & Codex) | `permissions.rs` | Checked for **both** Claude and Codex (generalizing `delegated_silence`). Logs `askhuman_guard` (`component: "permission_hook"`), emits nothing so host/T3 handles permission natively. |
| Claude `AskUserQuestion` takeover | `ask_question.rs` | Checked before building task; logs `askhuman_guard` (`component: "ask_question_hook"`), emits nothing so Claude/T3 handles natively. |
| MCP server (`ask`, `whats_next`, `show_last`, `todo_add`, `todo_update`) | `mcp/ask.rs` | New `t3_delegated_guard(binding, tool)` runs for **all** agents (not just Codex) after resolving binding (or before via env check). Returns `CallToolResult::error` with clear non-retry guidance text and logs `askhuman_guard` (`component: "mcp_tool"`). `todo_list` remains read-only allowed. |
| CLI `AskHuman` (ask / `--whats-next` / `--show-last` / todo-write) | `cli/mod.rs` | Before connecting to daemon for user-interactive prompts, checks `t3_host::env_delegated_reason()`. If delegated, logs `askhuman_guard` (`component: "cli_ask"`), prints non-retry guidance message to stdout/stderr, exits with code 2. Management subcommands (`agents`, `doctor`, `daemon`, `dev`, `config`, `channel`, `away`, `__*`) are never blocked. |
| Failure confirmation cards | `daemon/runtime/failure_confirmation.rs` | Generalizes line 689 to also suppress when `t3_host::marker_silenced(kind, sid).is_some()`. |

#### 4.3.3 Pi RPC Extension Guardrail (`src-tauri/resources/integrations/pi/index.ts`)

Add an explicit helper:
```ts
function isExternallyControlled(): boolean {
  if (process.env.T3CODE_THREAD_ID) return true;
  const argv = process.argv;
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--mode=rpc") return true;
    if (arg === "--mode" && argv[i + 1] === "rpc") return true;
  }
  return false;
}
```
- In `startControlPolling(pi, ctx)`: return immediately when `isExternallyControlled()` is `true` (never poll `__agent-control`, never call `pi.sendUserMessage()` asynchronously while idle).
- In `pi.on("agent_settled", ...)`: under P7, when `isExternallyControlled()` is `true`, skip spawning `__stop-hook pi confirm` / calling `pi.sendUserMessage()` and report `turn-end` directly (unless Phase 5 confirms RPC `sendUserMessage` safety with T3 Code and the user prefers keeping Stop confirmation on main T3 Pi threads).
- Because `pi/index.ts` changes, `pi_extension::status().outdated` will naturally flag installed Pi extensions for update via the standard "Update" button / `AskHuman agents update pi`.

---

## 5. Security, Ownership & Invariants

1. **No silent writes into third-party directories**: AskHuman NEVER writes into `~/.t3/` unless the user explicitly enables a discovered profile or manually adds a path.
2. **Strict ownership preserved per profile**: Every Antigravity extra profile has its own SHA-256 ledger (`~/.askhuman/antigravity-plugin-profiles/<id>.json`) and atomic staging/rename flow. Hand-edited files trigger `Conflict` and are never overwritten or deleted.
3. **No symlink tricks for Antigravity plugins**: Every Antigravity profile publishes a real directory tree.
4. **No path traversal in transcript hints**: `session_paths::validate_path` canonicalizes and checks agent-specific structural suffixes (and UID ownership for Antigravity) before caching or reading.
5. **Bounded latency & fail-open**: T3 CLI queries are strictly bounded (1.5 s timeout, 256 KiB output cap, 24 h cache, 30 s error cache) and fail open on any error. Non-T3 users incur zero overhead (single `env::var_os("T3CODE_THREAD_ID")` check).
6. **Dev Instance & Dev Test Profile isolation**: `development::session::directory_for` always takes precedence over `paths::with_agent_root`, and `agent-profiles.json` lives inside `paths::config_dir()` (`$ASKHUMAN_HOME`), so Dev Instances never mutate the user's production profile list or System daemon.

---

## 6. Verification & Acceptance Criteria

1. **Unit & Integration Tests (Rust + Vitest)**:
   - L1: `session_paths` validation per agent (valid & malicious/traversal paths), `Surface::Acp` vs `App`/`Cli`/`Ide`, `is_shared_agy_acp` process matching, T3 wrapper stripping across Antigravity/Cursor/Claude/Codex/Grok/Pi transcripts, `antigravity_last_message` with `transcriptPath`.
   - L2: `agent_profiles` CRUD, normalization, duplicate/primary rejection, T3 Antigravity discovery & dismiss, `paths::with_agent_root` thread-local scoping & panic safety, Claude custom root `.claude.json` placement, Codex per-profile `direct_only` ownership, Antigravity multi-profile independent ledgers & conflict isolation, missing root refusal, `Mode::None` cleanup + profile retention, partial failure isolation, `agents profile` CLI, `doctor --json`, Settings Vue component & `agentCard.ts` tests.
   - L3: `t3_host` JSON parsing (`parentThreadId`, `relationshipToParent`, top-level `createdBy == "agent"` NOT delegated), cache TTL & transient error cache, fail-open on timeout/missing binary, session-keyed marker write/clear/read, guard suppression + `askhuman_guard` audit across `context_recovery`, `report`, `antigravity_hook`, `stop`, `permissions`, `ask_question`, `mcp/ask`, `cli`, `failure_confirmation`; Pi extension unit test for `isExternallyControlled()` (`--mode rpc`, `--mode=rpc`, `T3CODE_THREAD_ID`, normal interactive mode).
2. **Isolated E2E Verification in Dev Worktree (NO Cursor)**:
   - Enable Dev Instance in `/Users/wutian/Developer/HumanInLoop-t3-multi-profile`, build & install via `./scripts/install.sh` (checking `node scripts/machine-health.mjs` first).
   - Run isolated `/tmp` `agy acp --gemini-dir <tmp> --app-data-dir antigravity-acp` with an enrolled extra Antigravity profile: verify plugin install, `rules/AGENTS.md` + MCP + hooks, `Surface::Acp`, transcript/title/activity/`/transcript`/Stop last-message resolution, and clean uninstall.
   - Run zero-LLM isolated profile verification for Codex (`CODEX_HOME`), Claude (`CLAUDE_CONFIG_DIR`), Grok (`GROK_HOME`), and Pi (`PI_CODING_AGENT_DIR` + `--mode rpc`).
   - Run T3 child-thread silence verification using a mock `T3CODE_CLI_PATH` script + real `t3 thread show` on the current main thread (confirming main thread is NOT silenced while a simulated child thread IS silenced across CLI, MCP, Stop, Permission, AskUserQuestion, and lifecycle hooks).
   - Verify Settings UI in Dev Instance (`node scripts/dev.mjs ui`) visually / interactively.
