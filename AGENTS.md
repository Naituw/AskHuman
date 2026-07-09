# AGENTS.md

## Before a complex task

Read [`docs/overview.md`](docs/overview.md) first to understand the architecture and project layout. When the task is complete, update `docs/overview.md` so it stays accurate.

## Git worktree / parallel development

If the task needs a **new or existing git worktree** for parallel feature work on this repo (so install/AskHuman do not share the main daemon or overwrite another agent’s binary), read [`docs/agent-worktree-setup.md`](docs/agent-worktree-setup.md) first and follow it—including asking the human via AskHuman whether to attach a channel preset—before coding. After `dev enable`, verification is still `./scripts/install.sh` then `AskHuman` (auto-routed into that worktree’s Dev Instance).

## Tracking progress in docs/PROGRESS.md

[`docs/PROGRESS.md`](docs/PROGRESS.md) has two jobs: track unfinished work and what's next, and mark the one task currently in progress. Read it first.

Before starting ANY work, mark what you're now doing there (reuse an existing entry or add one; a simple task just needs the marker, not detailed steps). Keep it current as you go. When done, clear the marker so nothing shows as in progress, and delete finished sections — history stays in git.

## Verifying your changes

After making any change to this project's functionality or logic, verify the result by running the install script to compile the new code directly into your environment, then use the newly installed `AskHuman` for subsequent prompts:

```bash
# macOS / Linux
./scripts/install.sh

# Windows
./scripts/install-windows.ps1
```

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
