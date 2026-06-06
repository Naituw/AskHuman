# Development

Engineering notes for contributors. User-facing docs live in [`docs/wiki/`](./wiki); design specs and plans live in [`docs/specs/`](./specs) and [`docs/plans/`](./plans). A code-level architecture overview is in [`overview.md`](./overview.md).

## Prerequisites

- [Rust toolchain](https://rustup.rs)
- [pnpm](https://pnpm.io) (Node 20+)
- macOS only: an Xcode SDK is required because some macOS-native pieces are built from Swift via `build.rs`.

## Layout

- `src/` — Vue 3 + Vite + TypeScript frontend. The Vite entry `index.html` lives here, and Vite's `root` is set to `src` (build output goes to the repo-root `dist/`, which Tauri embeds).
- `src-tauri/` — Rust backend (Tauri 2). Produces the single `AskHuman` binary.
- `scripts/` — build/install/release helpers (`install.sh`, `install-windows.ps1`, `publish.sh`, `bump-version.mjs`).
- `packaging/npm/` — npm main package (`askhuman`) and scoped per-platform binary subpackages.

## Develop, build, test

```bash
pnpm install
pnpm tauri dev                                              # Vite + Tauri debug window
pnpm build && cargo build --release \
  --manifest-path src-tauri/Cargo.toml --features custom-protocol   # release (frontend embedded at cargo build time)
cargo test --manifest-path src-tauri/Cargo.toml            # Rust unit tests
```

> `--features custom-protocol` is mandatory for production builds; without it the binary runs in dev mode against the Vite dev URL and shows a blank window.

Build and install locally:

```bash
# macOS / Linux  → installs to ~/.local/bin/AskHuman
./scripts/install.sh

# Windows        → installs to %LOCALAPPDATA%\Programs\AskHuman
./scripts/install-windows.ps1
```

> Running the GUI popup on Linux needs system WebKitGTK (e.g. `libwebkit2gtk-4.1`). If it's missing and a session-based channel (Telegram / DingTalk / Feishu) is configured, AskHuman automatically uses that channel; if none is available it exits with code 3 to signal graceful degradation.

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
