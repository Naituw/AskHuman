# AGENTS.md

## Before a complex task

Read [`docs/overview.md`](docs/overview.md) first to understand the architecture and project layout. When the task is complete, update `docs/overview.md` so it stays accurate.

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

Write git commit messages in English.
