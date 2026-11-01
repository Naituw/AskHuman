# AGENTS.md

## Before a complex task

Read [`docs/overview.md`](docs/overview.md) first to understand the architecture and project layout.
Its [“文档边界”](docs/overview.md#文档边界) section defines what belongs in the main overview versus supplementary overview, spec, and plan documents.
When the task is complete, check whether the change made any overview or referenced spec inaccurate and update only the appropriate document; do not edit the main overview when the repository-wide map and invariants are unchanged.

## Git worktree / parallel development

If the task needs a **new or existing git worktree** for parallel feature work on this repo (so install/AskHuman do not share the main daemon or overwrite another agent’s binary), read [`docs/agent-worktree-setup.md`](docs/agent-worktree-setup.md) first and follow it—including asking the human via AskHuman whether to attach a channel preset—before coding. After `dev enable`, run `./scripts/install.sh` and verify through the project `askhuman-dev` MCP’s `dev_ask` (or `node scripts/dev.mjs ask` for CLI). Ordinary discussion, approvals and handoff continue through the normal AskHuman connection.

## Tracking unfinished work in docs/PROGRESS.md

[`docs/PROGRESS.md`](docs/PROGRESS.md) tracks durable unfinished or deferred work and concrete next steps across sessions. Read it before starting work. When a tracked item is completed, delete its section — history stays in git.

## Verifying your changes

After making any change to this project's functionality or logic, verify the result by running the install script to compile the new code directly into your environment, then use the newly installed `AskHuman` for subsequent prompts:

For an enabled Dev Instance, use the explicit verification flow above; ordinary discussion
continues through the normal AskHuman connection.

```bash
# macOS / Linux
./scripts/install.sh

# Windows
./scripts/install-windows.cmd
```

## Installing or updating the iOS app

For remote iPhone installation or a new internal TestFlight build, use the project skill
[`ios-testflight`](.agents/skills/ios-testflight/SKILL.md). It documents the existing app,
Keychain-backed signing helper, build/upload commands, and phone-side verification. Reuse that setup
for routine updates; credentials stay outside the repository.

## Changing IPC messages

Before editing the message types in [`src-tauri/src/ipc/mod.rs`](src-tauri/src/ipc/mod.rs) (`ClientMsg`, `ServerMsg` and their payloads), read the [“IPC 协议号”](docs/overview.md#ipc-协议号protocol_version) section of `docs/overview.md`. It says which changes must bump `PROTOCOL_VERSION` and which must not, and what to update when you bump. In short: an additive field with `#[serde(default)]` never bumps; a new enum variant an older peer can receive, or any change an older peer would misread, always does. A daemon may serve clients of the same protocol number but a different build, so “same number” has to keep meaning “can talk”.

## Code comments

Write code comments in English.

## Commit messages

Follow **Conventional Commits**. Release notes shown to end users are generated
automatically from these messages (git-cliff, see `docs/specs/self-update.md`), so
**write them carefully** — a sloppy `feat`/`fix` subject becomes a sloppy user-facing
changelog line.

**Format**: `<type>(<scope>): <subject>` (scope optional).

**Subject**: English, imperative mood, lowercase after the colon, no trailing period,
ideally ≤72 chars. E.g. `feat(update): add in-app self-update via daemon drain`.

**Types** — only these reach the user-visible release notes:

- `feat` → ✨ Features
- `fix` → 🐞 Fixes
- `perf` → 💎 Performance
- `security` → 🔒 Security
- `revert` → ⏪ Revert

These are **excluded** from release notes (use them for non-user-facing work):
`docs`, `style`, `refactor`, `test`, `ci`, `build`, `chore`.

**Scope**: optional but encouraged; lowercase area name
(`channels`, `daemon`, `popup`, `cli`, `settings`, `slack`, `feishu`, `dingtalk`,
`telegram`, `hooks`, `config`, `i18n`, `update`, …); multiple joined by comma
(e.g. `popup,cli`). The scope is rendered as a **bold prefix** in release notes, so
a clear scope pays off.

**Breaking changes**: mark with `type!:` (e.g. `feat!: …`) or a `BREAKING CHANGE: <desc>`
footer. These are listed first under a dedicated **⚠ Breaking Changes** group.

**Body** (optional): motivation / context / trade-offs, separated by a blank line.
The body is NOT included in release notes (only the subject is).

**Per-commit override of release-note text** (footer trailers):

- `Release-Note: <text>` — use `<text>` instead of the subject in the release notes.
- `Release-Note: skip` — exclude this commit from the release notes even if it is a
  `feat`/`fix`/etc.

What reaches the release notes is decided by the `type` (plus the trailers above). For a
fully custom changelog of a given version, provide `docs/release-notes/v<version>.md`
(it overrides git-cliff for that release).
