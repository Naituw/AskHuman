# AGENTS.md

## Before a complex task

Read [`docs/overview.md`](docs/overview.md) first to understand the architecture and project layout. When the task is complete, update `docs/overview.md` so it stays accurate.

Also read [`docs/PROGRESS.md`](docs/PROGRESS.md). It tracks ToDos and current progress, organized by concrete work task or requirement. For a large task, update `docs/PROGRESS.md` from time to time while developing. When a task or requirement is finished, delete its corresponding section.

## Verifying your changes

After making any change to this project's functionality or logic, verify the result by running the install script to compile the new code directly into your environment, then use the newly installed `AskHuman` for subsequent prompts:

```bash
# macOS / Linux
./scripts/install.sh

# Windows
./scripts/install-windows.ps1
```

## Commit messages

Write git commit messages in English.
