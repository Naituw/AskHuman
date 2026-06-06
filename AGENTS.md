# AGENTS.md

Guidance for AI agents working in this repository.

## Before a complex task

Read [`docs/overview.md`](docs/overview.md) first to understand the architecture and project layout. When the task is complete, update `docs/overview.md` so it stays accurate.

## Verifying changes to the `askhuman` command

After iterating on the `askhuman` command, verify the result by running the install script to compile the new code directly into your environment, then use the newly installed version for subsequent prompts:

```bash
# macOS / Linux
./scripts/install.sh

# Windows
./scripts/install-windows.ps1
```

## Commit messages

When performing a git commit, write the commit message in English.
