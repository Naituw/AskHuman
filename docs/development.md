# Development

Engineering notes for contributors. User-facing docs live in [`docs/wiki/`](./wiki); design specs and plans live in [`docs/specs/`](./specs) and [`docs/plans/`](./plans). A code-level architecture overview is in [`overview.md`](./overview.md).

## Prerequisites

- [Rust toolchain](https://rustup.rs) 1.88 or newer (required by the patched ZIP dependency and the locked image/serde_with/time versions)
- [pnpm](https://pnpm.io) (Node 20+)
- macOS only: an Xcode SDK is required because some macOS-native pieces are built from Swift via `build.rs`.

## Layout

- `src/` — Vue 3 + Vite + TypeScript frontend. The Vite entry `index.html` lives here, and Vite's `root` is set to `src` (build output goes to the repo-root `dist/`, which Tauri embeds).
- `src-tauri/` — Rust backend (Tauri 2). Produces the single `AskHuman` binary.
- `scripts/` — build/install/release helpers (`install.sh`, `install-windows.cmd`, `publish.sh`, `bump-version.mjs`).
- `packaging/npm/` — npm main package (`askhuman`) and scoped per-platform binary subpackages.

## Develop, build, test

```bash
pnpm install
pnpm tauri dev                                              # Vite + Tauri debug window
pnpm build && cargo build --release \
  --manifest-path src-tauri/Cargo.toml --features custom-protocol   # release (frontend embedded at cargo build time)
cargo test --manifest-path src-tauri/Cargo.toml            # Rust unit tests
```

### UI prototyping (browser-only, no Tauri)

`src/prototype/` hosts standalone HTML entries that run in a plain browser with mock data and
hot reload — no Tauri, no daemon. They share the app's design tokens (`src/styles/*`) and
Markdown renderer, and are **excluded from release builds** (Vite only bundles `src/index.html`).

```bash
pnpm dev
# open http://localhost:5180/prototype/agent-console.html
```

Current prototypes:

- `agent-console.html` — the two-pane Agent console (spec `docs/specs/gui-agent-console.md`).
  This is the visual/interaction baseline for `views/AgentsView.vue` + `views/console/*`.
- `popup-inbox.html` — the mock request inbox (spec `docs/specs/popup-request-inbox.md`).
  Run `./scripts/popup-inbox-demo.sh` for the Vue/Tauri native demo, or add `--auto` for the
  native geometry/state checks. It starts its own Vite server on port 5198 and does not connect
  to the product daemon. Logs are written under `.askhuman-dev/`. The Rust example and prototype
  frontend are excluded from the normal AskHuman binary and release frontend entry.
- `popup-collapse.html` — the collapsible inbox window (spec `docs/specs/popup-inbox-collapse.md`).
  Run `./scripts/popup-collapse-demo.sh` for the native demo (controller window + answer window;
  `--effect blur|glass|solid` picks the window material), or add `--auto` for the native
  geometry/state checks and per-transition frame statistics (`--auto-controls` instead has the
  controller window drive the answer window through its real event channel). It uses its own Vite port 5199 and
  app identity, never connects to the daemon. The animation driver (`ah_collapse_animate`) lives in
  `src-tauri/swift/PopupCollapse.swift` and is part of the product build. Everything that only exists to verify it
  lives in `src-tauri/swift/prototype/PopupCollapseVerify.swift` and is compiled **only** with the `collapse-prototype`
  Cargo feature (`scripts/popup-collapse-demo.sh` passes it; build the example by hand with
  `cargo build --manifest-path src-tauri/Cargo.toml --example popup_collapse_demo --features collapse-prototype`),
  so the product binary carries none of it. Those helpers can screenshot the app's own windows
  (CoreGraphics via `dlsym`, no screen-recording permission needed) and diff regions of two captures; the
  `--auto` run uses that for pixel checks and writes review PNGs to `.askhuman-dev/popup-collapse-shots/`.
  `--video` measures what the display really shows during collapse / expand: a plain backdrop window sits behind the
  answer window while `scripts/popup-collapse-frames.swift` (ScreenCaptureKit, needs the terminal's Screen Recording
  permission) logs every delivered frame's display time and window bounding box into
  `.askhuman-dev/popup-collapse-frames.tsv`; the demo logs per-tick driver timings into the jsonl log. Pick variants and
  interleave them with `ASKHUMAN_DEMO_QUERY="cycles=2&rounds=5&variants=base,ctl-driver,hidden"` (`ctl-*` are plain-window
  reference animations). Frame pacing depends heavily on the machine's momentary WindowServer load, so only compare
  variants that ran interleaved (`rounds`), never one after another. `--burst` captures the app's own window
  (it disturbs the main thread; good for tearing, not for judging smoothness).
  `--auto` and `--auto-controls` run **silently**: the process replaces `-[NSApplication activateIgnoringOtherApps:]`
  with a no-op (`ah_collapse_forbid_activation`) and shows its windows without focusing them, so a scripted run never takes
  the keyboard focus from whoever is working (the user objected to exactly that). Add `--focus` only for the one check
  that needs a real app switch (S15 hands the activation to Finder); `--burst` / `--video` / the interactive mode keep
  the normal active window. Anything that takes the focus must be agreed with the user first. To prove a run is silent,
  poll `lsappinfo front` while it runs: the front app must stay the one you started from.
- `popup-session-title.html` — where the Agent session title shows inside the popup (spec
  `docs/specs/popup-session-info.md`). Every cell is an iframe running the **real** `PopupView` against `mockIPC`
  (`sessionTitleMock.ts`). The chosen placement, the title strip (`strip`), now ships in `PopupNavbar`, so that cell is
  the product itself; the rejected placements (`caption|reply|inline`, `SessionTitleFrame.vue`, teleported into the
  real DOM) are the only prototype code, and a cell without `strip` hides the product's strip for comparison. Controls
  cover theme, title length, window width and late arrival; `popup-session-title-frame.html?variant=strip|caption|reply|inline&…`
  renders one window. Pure browser, no Tauri, so the Windows / Linux native title is only drawn as an illustration.

**Workflow:** iterate UI ideas on the prototype first (cheap, instant feedback, user can review
in a browser), get the design confirmed, then port to the real views. Keep the prototype in
sync when a design decision changes — it stays the reference for the next iteration round.

### Optional local git hooks (fmt + clippy)

Linux CI fails the job on `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`. To catch that before push:

```bash
./scripts/install-git-hooks.sh
```

This only installs a `pre-commit` hook (LFS hooks are left alone). It runs when staged `src-tauri/**/*.rs` files change. Typical warm cost is a few seconds; a cold clippy can take about a minute. Bypass with `git commit --no-verify`. Local toolchain may still differ slightly from CI’s stable channel.

> `--features custom-protocol` is mandatory for production builds; without it the binary runs in dev mode against the Vite dev URL and shows a blank window.

Build and install locally:

```bash
# macOS / Linux  → installs to ~/.local/bin/AskHuman
# (or <worktree>/.askhuman-dev/bin when Dev Instance is enabled — see below)
./scripts/install.sh

# Build/install the exact production release profile when needed:
./scripts/install.sh --release

# Force production install path even inside an enabled worktree:
./scripts/install.sh --global

# Windows        → installs to %LOCALAPPDATA%\Programs\AskHuman
.\scripts\install-windows.cmd

# Windows exact production profile:
.\scripts\install-windows.cmd -Release
```

The Windows command wrapper works under the default restrictive PowerShell execution policy. The
installer idempotently adds its install directory to the current user's `PATH` and installs a
managed `AskHuman.cmd` launcher in the standard per-user `WindowsApps` command directory. This makes
`AskHuman --version` available immediately even when an Agent installs from a different Windows
session. The uninstaller removes only its managed launcher and install-directory entry.

> Running the GUI popup on Linux needs system WebKitGTK (e.g. `libwebkit2gtk-4.1`). If it's missing and a session-based channel (Telegram / DingTalk / Feishu) is configured, AskHuman automatically uses that channel; if none is available it exits with code 3 to signal graceful degradation.

`install.sh` uses the dedicated `local-install` Cargo profile (`opt-level=0`, 64 codegen units) so
small local edits compile quickly; publishing and CI continue to use the production `release`
profile. The script fingerprints frontend inputs and reuses `dist/` when they have not changed,
preventing an unchanged Vite rebuild from invalidating Tauri's embedded resources. It also skips
copy/sign when the built and installed binary state is unchanged (local macOS signing does not
request a network timestamp). After a successful copy it enforces per-profile target budgets with
Cargo-coordinated cleanup: local package artifacts are removed first, while dependency caches are
retained whenever they fit.

The budgets are 4 / 6 / 6 / 4 GiB for `local-install` / `dev` / `full-debug` / `release` in the main
checkout and 3 / 5 / 5 / 3 GiB in a linked git worktree, where several copies of `target/` add up.
The Swift speech bridge's Clang/Swift module cache is shared by every profile in
`src-tauri/target/swift-module-cache` (about 250 MiB; it is deleted and rebuilt in about 25 seconds
when it outgrows 768 MiB). Budget cleanups, sweeps, a changed compiler/SDK pair and a stray
`RUSTC_BOOTSTRAP` are appended to `src-tauri/target/askhuman-cache.log` (newest 200 lines); read it
first when a build was unexpectedly cold.

Do not `export` `RUSTC_BOOTSTRAP` or `RUSTFLAGS` in a shell that builds this project. Several build
scripts record `RUSTC_BOOTSTRAP` in their fingerprint, so setting it recompiles most of the
dependency tree (about 120 units) and unsetting it does so again; `RUSTFLAGS` is part of every
unit's fingerprint. `install.sh` unsets `RUSTC_BOOTSTRAP` for its own build and says so. Put such a
variable in front of the one command that needs it instead, for example
`RUSTC_BOOTSTRAP=1 cargo build -Z unstable-options --unit-graph`.

Mind Cargo feature unification when adding macOS dependencies. The `objc2-*` framework crates have
one feature per header and enable all of them by default (about 300 bindings for AppKit), and a
single dependency that leaves the defaults on re-enables them for the whole graph.
`src-tauri/Cargo.toml` therefore turns the defaults off and lists the bindings it uses; if a build
fails with an unresolved `objc2_*` import or a missing method, add that type's feature (named after
its header). `drag` is vendored in `src-tauri/vendor/drag` only so that its own objc2 dependencies
can do the same: its published manifest omits `default-features = false`. Drop the
`[patch.crates-io]` entry and the vendored copy once upstream releases a fixed manifest; the
comments in `Cargo.toml` and in the vendored manifest say exactly what differs.

On macOS/Linux, installation stages the binary next to the destination, sets permissions, and
finishes macOS signing and verification before atomically replacing the destination. Running MCP,
daemon, and GUI processes keep their original executable inode; overwriting or re-signing that
inode can terminate them with a macOS code-signature error. A signing/verification failure leaves
the installed binary and its saved hash state intact. Symlink entry points are preserved by
replacing their resolved target. MCP caches its executable path at startup so subsequent
`ask`/`whats_next` calls launch the newly installed CLI, including on Linux where resolving a
replaced process image again may return a path ending in ` (deleted)`.

This supports continued questions in the same Agent session across local installs. Existing
questions finish under the old daemon before graceful drain switches to the new build. MCP tool
schemas and code running inside the MCP process itself stay at that process's original version
until the client restarts its MCP connection. Windows installation still follows its separate
executable-lock handling; it is not covered by the Unix atomic replacement behavior.

The script ends by running `AskHuman doctor install --warn-only` with the freshly installed binary.
It prints nothing unless AskHuman copies would fight over the daemon: a different build earlier on
`PATH` than the one just installed, several builds on `PATH`, or a daemon hold already in effect.
Several names for one file count as one copy and are not reported: with `~/.local/bin/AskHuman`
symlinked to the binary inside the npm package, an install writes through the link and every name
runs the same build. See `docs/specs/daemon-graceful-drain.md` D12. A Dev Instance install skips the
`PATH` comparison.

Default dev/test builds retain line tables for file/line backtraces without full debugger local
variable metadata. When full source-level debugger information is needed, use:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --profile full-debug
cargo test --manifest-path src-tauri/Cargo.toml --profile full-debug
```

### Parallel development (Dev Instance / git worktrees)

Multiple agents can develop in separate git worktrees without sharing one daemon or overwriting one binary. Design: [`docs/specs/dev-instance-parallel.md`](./specs/dev-instance-parallel.md). **Agent checklist:** [`docs/agent-worktree-setup.md`](./agent-worktree-setup.md).

```bash
cd /path/to/worktree
AskHuman dev enable                    # or: dev enable --preset <name>
./scripts/install.sh                   # → .askhuman-dev/bin
AskHuman "…"                           # auto re-exec into this instance
```

Default instance config is popup-only and never reads the main keychain. Optional machine-level channel presets live in `~/.askhuman/dev-presets/` with exclusive leases.

A new worktree starts with an empty `src-tauri/target/` and compiles every dependency (several
minutes; its `target/` then grows to several GiB). On macOS/APFS, `./scripts/seed-worktree-target.sh`
seeds it from the main checkout with copy-on-write clones so that the first install compiles only
this project. It is optional and never run by `dev enable`; see the seeding step and its caveats in
[`docs/agent-worktree-setup.md`](./agent-worktree-setup.md).

## Checklist: adding a new IM channel

There is no single `Channel` trait yet — a new channel touches many match arms. Grep an
existing channel (`slack` is the newest and most complete) and mirror every hit. Roughly:

**Rust backend**

- [ ] `src-tauri/src/<channel>/` — client crate-module: API client (`client.rs`; wrap the
  unified request exits with `track()` reporting to `channels::health`), long-connection
  router (`router.rs` / `ws.rs`), watch cards (`watch.rs`), select cards (`select.rs`),
  confirm adapter (`confirm.rs`), markdown conversion if the platform needs it.
- [ ] `src-tauri/src/channels/<channel>.rs` — ask/answer channel adapter (message + file
  delivery, card building, conversation binding), declared in `channels/mod.rs`.
- [ ] `src-tauri/src/config.rs` — `channels.<channel>` config struct + defaults.
- [ ] `src-tauri/src/secrets.rs` — keychain migration/storage for the channel's secrets
  (and update the module doc comment listing managed secrets).
- [ ] `src-tauri/src/autochannel.rs` — channel id/label, auto-activation participation.
- [ ] `src-tauri/src/daemon/runtime/` — `mod.rs` `ensure_<channel>_router` (report/clear
  channel health on connect), plus per-channel arms in `detect.rs`, `watch.rs`,
  `select.rs`, `inbound.rs`.
- [ ] `src-tauri/src/confirm/` — `transport.rs` / `choice_cards.rs` arms.
- [ ] `src-tauri/src/i18n.rs` — channel label + user-visible strings (en/zh).
- [ ] `src-tauri/src/cli/` — `channel_cmd.rs`, `config_cmd.rs`, `cfgio.rs`, `output.rs`,
  `help.rs` mentions; `dev_presets.rs` if dev-instance presets should cover it.

**Frontend**

- [ ] `src/lib/types.ts` — config type mirror; `src/lib/ipc.ts` if a test command exists.
- [ ] `src/views/SettingsView.vue` — channel card (enable switch, credential fields, test
  button, R7 issue banner via `channelIssueText`, "Setup guide" link via
  `CHANNEL_SETUP_DOCS`) **and** entries in the settings search index (`searchIndex`).
- [ ] `src/i18n/zh.ts` + `src/i18n/en.ts` — settings strings.

**Docs**

- [ ] `docs/wiki/<channel>-setup.md` + `.en.md`, linked from both READMEs.
- [ ] `docs/overview-configuration.md` field map; `docs/overview.md` if the repo-wide map
  changes; a spec under `docs/specs/` for non-trivial platform behaviors.

## Checklist: adding a new agent family

`AgentKind` is a closed enum matched in many places. Grep an existing kind (`grok` is the
newest) and mirror every hit. Roughly:

- [ ] `src-tauri/src/agents/mod.rs` — `AgentKind` variant + `as_str`/`label`/`parse`.
- [ ] `src-tauri/src/agents/` — per-kind logic in `detect.rs` (process-chain detection),
  `title.rs`, `transcript_full.rs`, `activity.rs`, `registry.rs`, `report.rs`, `stop.rs`.
- [ ] `src-tauri/src/agents/interject.rs` — decide whether the family supports interjection
  (grok is excluded: no reliable relay channel).
- [ ] `src-tauri/src/integrations/` — `agent_lifecycle.rs` (hook install paths),
  `agent_stop.rs`, `agent_launch.rs` (IM `/new` agent tasks), rules/skills installers.
- [ ] `src-tauri/src/prompts.rs` — interaction-protocol artifact for the family.
- [ ] `src-tauri/src/cli/` — `doctor.rs` readiness checks, `agents_cmd.rs`.
- [ ] `src-tauri/src/app/gui_host.rs` — nothing usually (labels go through
  `AgentKind::label`), but verify tray agent submenu renders the new kind.
- [ ] Frontend: `src/views/SettingsView.vue` integration tab (install docs URL in
  `AGENT_INSTALL_DOCS`, hook/rule cards), `src/i18n/*`, `src/lib/types.ts` if the kind
  appears in typed payloads.
- [ ] Docs: `docs/overview.md` agent integration section, wiki page if setup differs.

## Release

Versions across the repo are kept in sync by `scripts/bump-version.mjs` (writes `Cargo.toml`, `tauri.conf.json`, root `package.json`, the npm main package, and the platform subpackages, including the main package's lock on subpackage versions):

```bash
# 1. Bump version everywhere
node scripts/bump-version.mjs 0.2.0
git commit -am "release: v0.2.0"

# 2. Publish: verifies version consistency / not-already-published → tags → pushes (triggers CI)
./scripts/publish.sh           # add -y to skip the confirmation prompt
```

`scripts/publish.sh` checks that all versions match and that the version isn't already on npm (errors and asks you to bump otherwise), then tags and pushes. `.github/workflows/release.yml` then compiles the four platform binaries → publishes to npm (main package + platform subpackages) → creates a GitHub Release.

> Prerequisite: set `NPM_TOKEN` (an npmjs automation token) under the repo's Settings → Secrets. Pre-release versions (e.g. `0.2.0-rc.1`) are published under the npm dist-tag `next` and marked as a GitHub pre-release.

The release architecture and channel-degradation design are documented in [`docs/plans/release-and-channel-degradation.md`](./plans/release-and-channel-degradation.md).

仅用于统一窗口几何验收：在 popup-only Dev Instance 的空队列重启 Daemon 后，以
`ASKHUMAN_INBOX_LAYOUT_REVIEW=1 AskHuman '测试内容' -q '测试题目'` 启动。本轮窗口顶部
提供反复显示/隐藏 Sidebar 的按钮，走正式几何事务；结束测试后停止测试 Daemon，再正常
启动即可恢复普通流程。此环境变量只在 Dev Instance 生效，不改变正常 Sidebar 生命周期。

原生诊断位于该实例的 `state/popup-canvas-review.jsonl`（窗口、呈现层、WebKit/PDF 坐标）
与 `state/popup-pulse-review.jsonl`（提醒前 WindowServer transform）。它们只查询自身窗口。
使用临时测试 app bundle 时，从 `src-tauri/Info.plist` 合并麦克风/语音用途声明，并给每类
测试窗口独立 bundle ID；不能把生产安装 bin 改为测试签名或符号链接。

三栏几何的独立基础实验使用 `./scripts/pane-foundation-demo.sh`，原生 Vue/Tauri 窗口使用
专用端口 5199；`--auto` 跑坐标、viewport 与输入状态验证。它不连接 Daemon 或读取产品
配置；固定窗口对照和原生扩窗的验证边界见 `docs/investigations/popup-pane-stability.md`。

普通缩放/固定画布交接：`./scripts/pane-resize-demo.sh --auto` 在 5201 端口验证同一 Swift
画布桥的自适应布局、固定分区、原生 PDFView 和输入状态。`--polish` 打开隔离交互 Demo，
包含 mock 请求、附件、未查看标记；不连接 Daemon，不修改产品配置。Demo 样式候选并非
正式实现，最终正式范围见 `docs/specs/popup-request-inbox.md` §16。
