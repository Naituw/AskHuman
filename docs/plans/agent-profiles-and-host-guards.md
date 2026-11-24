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

---

## 发布记录（2026-11-25：Mac v0.18.2）

用户说「然后发新的patch版本和 TestFlight」，版本号定为 **v0.18.2**。发布说明手写（英文，四条功能加两条修复），随版本提交为 `docs/release-notes/v0.18.2.md`，Release 页的正文与该文件逐字一致。
按 2026-11-24 的决定，「外部宿主」那几条用通用说法、不出现产品名（`731630b1` 的标题含产品名，历史不改，所以才用文件整份取代自动生成的日志，见 [self-update.md](../specs/self-update.md) D17）。
受管文件检查（`git diff v0.18.1..HEAD --stat -- src-tauri/src/integrations src-tauri/resources/integrations`）：写进用户目录的内容只有 Pi 扩展 `index.ts` 变了（`isExternallyControlled()`），所以说明里写了「升级后到 设置 → Agents 给 Pi 点一次 Update」；
Antigravity 插件的改动只是给额外目录加了入口，`agent_rules.rs` 只改了可见性，`codex_protocol.rs` 只是把规则刷新带到额外目录，其余是对账代码。
两条只涉及 iPhone App 的修复（`577bdedc`、`bfe94f8c`）没列入，沿用 v0.17.x 的写法加了一句「iPhone App 通过 TestFlight 另行发布」。

- **先修 CI 的红灯**：main 的 CI 从多 profile 那次提交（`731630b1`）起一直是红的：Windows 的 `cargo test` 里只有 1 项失败，`normalize_antigravity_config_subdir_to_gemini_root`
  （`Antigravity integration is only supported on Unix platforms`）。根因是该用例走的是 Antigravity 在非 Unix 上被拒绝的路径（产品本意），而用例没分平台。
  `d3205604`：用例只在 Unix 跑，另加一个非 Unix 用例断言「被拒绝」。本机 `cargo test --bin AskHuman integrations::agent_profiles` 9 项通过；Windows 只有 CI 为证。
- **流程**：主工作树（干净）里 `bump-version` 到 0.18.2；`cargo update --workspace --offline` 后 `--locked --offline` 没有可改的（`Cargo.lock` 只变 `humaninloop` 一行）；
  提交 `faee4924 chore(release): prepare v0.18.2`（连同发布说明）与 `d3205604` 一起推送到 `main`。该提交的 [CI 45462154650](https://github.com/Naituw/AskHuman/actions/runs/45462154650) 8 项最终全绿：
  `broker-probe` 第一次失败（`ios/Tools/broker-probe` 的 keepalive 用例，四个同时报 `no PINGRESP after 1 s idle`，该目录自 11-22 后没改过），只重跑那一项就过了；原因没有查，见 `PROGRESS.md`。
  整轮约 28 分钟：macOS 任务排队久（`aarch64-apple-darwin` 最后一个结束）。更早的三次运行（`339c41b5`、`b4fc73b0`、`e8314cca`）里，排了队的 macOS 任务都在排满 15 分钟（15 分 02 秒）时被取消，
  所以我取消了已被取代的 `2da7f975` 那次运行，让发版提交的运行尽快拿到机器；推发版提交前别叠着别的运行。
- **用户点头这一步**：CI 全绿后我按流程通过 AskHuman 问了「打 tag 并发布吗」，附了发布说明，但那时用户已开启离开模式，自动回复让我按原目标继续、不做例行确认。
  我把「发新的patch版本」的明确请求加离开模式的指示当作授权，往下做了；这一点写在这里，让用户看得到。
- **打 tag**：注解 tag `v0.18.2` 指向 `faee4924`，只推了这一个 tag（没有 `--tags`，没有 `scripts/publish.sh`）。推送前确认过远端没有 `v0.18.2`、npm 上最新是 0.18.1、签名与 npm 所需的 Secrets 都在（只看名字）、
  `.github/workflows` 与打包脚本自 v0.18.1 起没有改动。推完发现远端有一个更早就在的 `ios-testflight-21` 标签（与本次推送无关，没有动它；这类标签本应只留本地）。
- **发布结果**：[Release 45467981895](https://github.com/Naituw/AskHuman/actions/runs/45467981895) 全部成功，从推 tag（19:13:45 UTC）到结束（19:38:15）约 24 分半，其中 `aarch64-apple-darwin` 构建排了 10 分钟的队；
  2026-11-25 03:38（北京时间）发布；Release 为非草稿、非预发布，四个二进制归档和 `SHA256SUMS` 都在。辅助组件公证（1 分 48 秒）、四个平台的构建、两个 macOS 的 Developer ID 签名一次通过。
- **核验**：四个归档都通过 `SHA256SUMS`。Mac ARM64：严格签名验证通过（Developer ID Application，TeamID `DMJXDB9H6Q`，hardened runtime，带时间戳），`--version` 为 v0.18.2，
  `todo-sync resource-info --require-bundled` 给出内嵌组件版本 0.18.2、协议 1，`todo-sync resource-verify` 通过（都在隔离的 `HOME` 与 `ASKHUMAN_HOME` 里运行，没有留下任何状态）。
  Mac Intel：签名验证通过、TeamID 相同、hardened runtime，没有运行。Linux 与 Windows：只用 `file` 认出是 ELF / PE，没有运行。ARM64 二进制 24.3 MB（v0.18.1 是 24.2 MB）。
- **npm**：发布步骤的日志里五个包都打印了 `+ …@0.18.2`（四个平台包在前，主包 19:37:50 最后）。发布后约 1 到 2 分钟 `npm view` 还是 404，约 5 分钟后 `latest` 指向 0.18.2（与上次一样是注册表的延迟，没有重发）。
  隔离目录、全新缓存里 `npm install askhuman@0.18.2` 装上了主包与 `darwin-arm64`，`AskHuman --version` 为 v0.18.2，其 native binary 的 SHA-256 与 GitHub 下载包一致。
- **没验证**：Windows、Linux、Intel Mac 的发布二进制没有在本机运行（CI 只覆盖编译与测试）；没有实际走一遍应用内自更新到 0.18.2；
  新功能在真实外部宿主里的表现没有为这次发版重新验证（验证范围见上面各阶段与规格 §6）；Pi 扩展「点 Update 才生效」这一步没有在真实 Pi 上走一遍。
