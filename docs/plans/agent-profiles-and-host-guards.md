# External Host Compatibility & Multi-Profile Agent Integration — Implementation Plan

> **Spec**: [`docs/specs/agent-profiles-and-host-guards.md`](../specs/agent-profiles-and-host-guards.md)
> **Worktree**: an isolated Dev Worktree (see [`docs/agent-worktree-setup.md`](../agent-worktree-setup.md))
> **Constraint**: Do not run live Cursor tests (`D2`). Check `node scripts/machine-health.mjs` before long Rust builds/tests.

---

## Phase 1 — L1 Universal Base Fixes

**Goal**: Make transcript resolution, Antigravity ACP surface classification, `agy acp` PID handling, and host prompt wrapper cleaning work reliably across all non-default paths and hosts.

### Files to Edit
1. `src-tauri/src/agents/session_paths.rs`
   - Generalize from Pi-only to all 6 `AgentKind` variants with `validate_path(kind, session_id, raw_path, cwd)` and `register(kind, session_id, raw_path, cwd)`.
   - Add per-agent structural validation (Antigravity `brain/<uuid>/.system_generated/logs/transcript_full.jsonl` + UID check; Claude `<sid>.jsonl`; Codex `rollout-*-<sid>.jsonl`; Cursor `agent-transcripts/<sid>/<sid>.jsonl`; Grok `<sid>/chat_history.jsonl`; Pi existing v3 JSONL header check).
2. `src-tauri/src/daemon/runtime/mod.rs` & `src-tauri/src/agents/registry.rs` & `src-tauri/src/agents/report.rs`
   - Register validated `transcript_path` for all 6 agents on incoming `AgentEvent` and on `AgentRegistry::restore()`.
   - Forward `resolve_transcript_path` in `report::report_simple_event`.
3. `src-tauri/src/agents/antigravity_hook.rs`
   - Add `Surface::Acp` and component-before-`brain` precedence so `.../providers/antigravity/<hash>/antigravity-acp/brain/...` classifies as `Surface::Acp` (not `Surface::App`).
4. `src-tauri/src/agents/detect.rs`
   - Add `is_shared_agy_acp(&ProcEntry)` and return `None` from `walk_agent_pid` when matched, preventing D7 same-PID eviction across multiplexed `agy acp` sessions.
5. `src-tauri/src/agents/transcript_full.rs` & `src-tauri/src/agents/title.rs`
   - Add pure `strip_host_wrappers()` removing the host's paired instruction/orchestration blocks (`HOST_METADATA_TAGS`) plus `<runtime_info>`, `<ADDITIONAL_METADATA>`, `<system-reminder>` and unwrapping `<USER_REQUEST>` / `<user_query>` / inner `<user_request>`.
   - Wire into `clean_user`, `antigravity_first_request`, `grok_first_query`, and Antigravity `USER_INPUT` transcript parsing.
   - Update `title::transcript_path` (and `antigravity_transcript_in` / `paths::antigravity_brain_dirs_in`) to check `session_paths::get` first and include `antigravity-acp` alongside `antigravity` and `antigravity-cli`.
6. `src-tauri/src/agents/stop.rs` & `src-tauri/src/mcp/ask.rs`
   - Update `antigravity_last_message` to validate `input["transcriptPath"]` via `session_paths::validate_path` before falling back to `title::transcript_path`.
   - Allow `antigravity.google/artifacts_dir` fallback in `mcp/ask.rs` `show_last` / transcript lookup if needed.

### Phase 1 Tests
- Unit tests in `session_paths.rs`, `antigravity_hook.rs`, `detect.rs`, `title.rs`, `transcript_full.rs`, `stop.rs`, `registry.rs`.

---

## Phase 2 — L2 Multi-Profile Backend, Isolated-Home Discovery, CLI & Doctor

**Goal**: Support extra config directories across all 6 agents with shared mode/capabilities, per-profile ownership ledgers, conflict/missing-root isolation, read-only isolated Antigravity home auto-discovery, and full CLI/doctor support.

### Files to Create / Edit
1. `src-tauri/src/paths.rs`
   - Add `agent_profiles_file()` and `antigravity_extra_plugin_ledger_in(config_dir, profile_id)`.
   - Add panic-safe thread-local RAII guard `with_agent_root(kind, root, f)` + `active_agent_root(kind)` subordinate to `development::session::directory_for(kind)`.
   - Ensure `claude_json()` returns `<root>/.claude.json` when `Claude` override is active.
2. `src-tauri/src/integrations/agent_profiles.rs` (new module) & `src-tauri/src/integrations/mod.rs`
   - `AgentProfilesStore` (`~/.askhuman/agent-profiles.json`, version 1): `ProfileEntry`, `DismissedEntry`, `DiscoveredProfile`, `ExtraProfileStatus`, `ProfileSyncOutcome`.
   - Deterministic `profile_id` generation, root validation & Antigravity `/config` normalization (`P1`), duplicate/primary equivalence rejection.
   - Read-only `discover_isolated_homes_in(home, store)` scanner (bounded 64 entries, Unix only, excludes enrolled & dismissed).
   - CRUD + reconcile orchestration: `list`, `add`, `set_enabled`, `remove`, `sync`, `dismiss_discovered`, `reconcile_enabled_profiles_unlocked(target, mode, env)`, `status_for_agent(target, primary_mode)`.
3. `src-tauri/src/integrations/antigravity_plugin.rs`
   - Expose parameterized `Layout::for_antigravity_root(root, ledger)` + `snapshot_for_root`, `install_for_root`, `uninstall_for_root`, `set_lifecycle_for_root`, `set_stop_for_root`, `host_view_for_root`, `detach_ledger_for_root`.
   - Keep primary behavior and `~/.askhuman/antigravity-plugin.json` 100% unchanged.
4. `src-tauri/src/integrations/mcp_config.rs`
   - Extend `CodexIntegrationState` with `extra_profiles: BTreeMap<String, CodexProfileState>` so each Codex extra profile tracks `direct_only_namespace_added_by_askhuman` independently while preserving backward compatibility for primary.
5. `src-tauri/src/integrations/{agent_mode.rs, agent_lifecycle.rs, agent_stop.rs, agent_permission.rs, agent_ask_question.rs}` & `src-tauri/src/commands.rs`
   - Reconcile enabled extra profiles after primary in `set_with_env`, `update`, `update_artifact`, `lifecycle::install/uninstall`, `stop::set_enabled/migrate_outdated`, `permission::set_enabled`, `ask_question::set_enabled/migrate_outdated`, and `save_collaboration_style` / `collaboration_style_apply_integrations`.
   - Implement partial-failure reporting (`P4`) and `rootMissing` refusal (`P2`).
6. `src-tauri/src/agents/title.rs`, `src-tauri/src/agents/stop.rs`, `src-tauri/src/agents/antigravity_workspaces.rs`, `src-tauri/src/agents/context_recovery.rs`
   - Search enabled extra profile roots as fallback in `title::transcript_path` / `resolve_title`, `stop::cursor_last_message`, and `antigravity_workspaces::recent_workspaces` (`P8`).
   - In `context_recovery.rs` (Codex `has_legacy_block`), honor `CODEX_HOME` if it matches an enabled extra profile.
7. `src-tauri/src/cli/agents_cmd.rs` & `src-tauri/src/cli/doctor.rs`
   - Add `agents profile <list|add|enable|disable|remove|sync|dismiss>` subcommands with zh/en help and `--json`.
   - Update `agents show`, `agents cleanup`, and `doctor [--json]` to report/clean extra profiles and surface discovered isolated Antigravity homes.

### Phase 2 Tests
- Comprehensive unit & tempdir integration tests covering all 6 agents' extra profiles, Claude `.claude.json` placement, Codex per-profile `direct_only` ownership + canonical trust keys, Antigravity multi-profile independent ledgers + conflict + detach, missing root refusal, `Mode::None` retention, partial failure isolation, isolated-home scanner & dismiss, and CLI parsing/output.

---

## Phase 3 — L2 Settings UI, Tauri Commands & i18n

**Goal**: Expose discovered isolated Antigravity homes and manual extra config directories cleanly inside Settings → Agents without cluttering unintegrated cards.

### Files to Create / Edit
1. `src-tauri/src/commands.rs` & `src-tauri/src/app/invoke.rs`
   - Add `profiles` and `discovered_profiles` to `AgentModeStatus`.
   - Register Tauri commands: `agent_profile_add`, `agent_profile_set_enabled`, `agent_profile_remove`, `agent_profile_sync`, `agent_profile_dismiss_discovered`, `agent_profile_reveal` (and directory picker helper if not already exposed).
2. `src/lib/types.ts` & `src/lib/ipc.ts`
   - Add TypeScript types (`ExtraProfileStatus`, `DiscoveredProfileInfo`) and IPC wrappers.
3. `src/views/settings/AgentProfilesSection.vue` (new component)
   - Displays discovered isolated homes (`Enable` / `Ignore`), configured extra profiles (enable switch, status badge, path, per-profile host hints, `Sync` / `Reveal` / `Remove` / `Forget without deleting`), and `+ Add directory` form (folder picker + manual path input + optional label).
4. `src/views/settings/IntegrationTab.vue`, `src/views/settings/useIntegration.ts`, `src/views/settings/agentCard.ts`, `src/views/settings/settings.css`
   - Surface Antigravity discovered-profile hint banner cleanly.
   - Reflect extra profile `needsUpdate` / `conflict` / `rootMissing` in `agentCard.ts` (`hasUpdate`, `agentTags`, `needsAction`) and "Update all" overview.
   - Wire profile actions with busy/error feedback and refresh.
5. `src/i18n/zh.ts` & `src/i18n/en.ts`
   - Add all zh/en strings for extra profiles, the discovery hint, conflict/missing-root actions, and tag labels.

### Phase 3 Tests
- Add/update Vitest suites: `agentCard.test.ts`, `useIntegration.test.ts`, `IntegrationTab.test.ts`, plus `AgentProfilesSection.test.ts`.

---

## Phase 4 — L3 Host Conflict Guardrails (Delegated Child Sessions + Pi RPC)

**Goal**: Silence AskHuman cleanly inside host-delegated child sessions across all 6 agents and disable conflicting Pi RPC background control.

### Files to Create / Edit
1. `src-tauri/src/paths.rs`
   - Add `host_threads_cache_dir()` (`state/host-threads`) and `host_delegated_marker_dir()` (`state/host-delegated`).
2. `src-tauri/src/agents/external_host.rs` (new module) & `src-tauri/src/agents/mod.rs`
   - Implement the thread lookup: env check (the host's thread-id variable), thread-id validation, disk cache (24 h TTL / 30 s error TTL + bounded pruning), bounded `<host cli> thread show --json --messages 0 <id>` subprocess (1.5 s timeout, 256 KiB output limit, fail-open), with the CLI query injectable for deterministic tests.
   - Implement session-keyed delegated markers (`classify_and_record_hook`, `marker_silenced`, `env_delegated_reason`) across all 6 `AgentKind` variants.
3. Wire L3 guards across every AskHuman entrypoint:
   - `src-tauri/src/agents/context_recovery.rs`: Codex & Claude `SessionStart`
   - `src-tauri/src/agents/report.rs`: lifecycle hooks & interjection polling (Claude/Codex/Cursor/Grok/Pi)
   - `src-tauri/src/agents/antigravity_hook.rs`: Antigravity `SessionStart`, `PreInvocation`, `Stop`
   - `src-tauri/src/agents/stop.rs`: Stop confirmation suppression for all 6 agents
   - `src-tauri/src/permissions.rs`: PermissionRequest suppression for both Claude and Codex
   - `src-tauri/src/ask_question.rs`: Claude `AskUserQuestion` suppression
   - `src-tauri/src/mcp/ask.rs`: `delegated_child_guard` across `ask`, `whats_next`, `show_last`, `todo_add`, `todo_update` (env + session-keyed marker)
   - `src-tauri/src/cli/mod.rs`: CLI ask / `--whats-next` / `--show-last` / todo-write guard with clear non-retry message + `askhuman_guard` audit
   - `src-tauri/src/daemon/runtime/failure_confirmation.rs`: suppress failure cards for delegated child sessions
4. `src-tauri/resources/integrations/pi/index.ts`
   - Add `isExternallyControlled()` checking the host's thread-id variable and `--mode rpc` / `--mode=rpc`.
   - Disable `startControlPolling()` (and `agent_settled` Stop `sendUserMessage` per P7) when externally controlled.

### Phase 4 Tests
- Unit tests in `external_host.rs`, `context_recovery.rs`, `report.rs`, `antigravity_hook.rs`, `stop.rs`, `permissions.rs`, `ask_question.rs`, `mcp/ask.rs`, `failure_confirmation.rs`.
- Node/Vitest or Rust-driven JS test verifying Pi extension `isExternallyControlled()` detection and control-polling skip under `--mode rpc`, `--mode=rpc`, the host's thread-id variable, and normal interactive invocation.

---

## Phase 5 — Dev Instance Build, End-to-End Verification & Documentation Sync

**Goal**: Verify all code end-to-end inside the isolated Dev Worktree without touching real user configs or running Cursor.

### Steps
1. **Enable Dev Instance in the worktree** (`docs/agent-worktree-setup.md`):
   - Inside the Dev Worktree, run `AskHuman dev enable` (and optionally `./scripts/seed-worktree-target.sh`).
2. **Machine health check (`AGENTS.md`)**:
   - Run `node scripts/machine-health.mjs` before cargo/npm builds & tests.
3. **Automated verification**:
   - `cargo test` (all Rust unit & integration tests)
   - `cargo clippy --all-targets -- -D warnings`
   - `npm test` (Vitest frontend + settings tests) & `npm run build` / `vue-tsc`
4. **Build & install into Dev Instance**:
   - Run `./scripts/install.sh` inside the Dev Worktree.
   - Verify Dev daemon via `node scripts/dev.mjs ask` (ordinary discussion continues on System daemon).
5. **Isolated E2E probes (zero mutation to real `~/.gemini` or the host's data directory; NO Cursor)**:
   - **Antigravity ACP E2E**: Create `/tmp` fake Antigravity home, enroll via Dev CLI `agents profile add antigravity <tmp>`, switch mode to MCP, run isolated `agy acp --gemini-dir <tmp> --app-data-dir antigravity-acp`, verify plugin load, `Surface::Acp`, transcript/title/activity/Stop last-message resolution, then remove profile and confirm clean removal.
   - **Codex / Claude / Grok / Pi isolated profile E2E**: Enroll temp roots for each, verify artifact sync, Codex trust keys + per-profile `direct_only`, Claude `<root>/.claude.json`, Grok `skills/` + `config.toml` + `hooks/`, Pi `extensions/askhuman/index.ts` under `--mode rpc` (confirming zero `__agent-control` polling).
   - **Child-session silence E2E**: Verify the current real main thread is NOT silenced (`parentThreadId: null`); verify a simulated child thread (the CLI-path variable pointing at a mock returning `parentThreadId: "parent-1"`) IS silenced across CLI, MCP, Stop, Permission, AskUserQuestion, and lifecycle hooks with `askhuman_guard` logs.
   - **Settings UI check**: Open Dev Settings (`node scripts/dev.mjs ui`) and verify the Antigravity isolated-home discovery card, manual profile add/sync/remove, conflict banner, and zh/en copy.
6. **Documentation boundary check (`AGENTS.md`)**:
   - Update `docs/PROGRESS.md` if any follow-up (e.g. host control-plane integration) should be tracked.
   - Check `docs/overview.md` and `docs/specs/antigravity-deep-integration.md` — update only if repository-wide invariants or Antigravity plugin/surface descriptions became inaccurate.
7. **Final review & handoff via `AskHuman --caller-agent antigravity`**.
