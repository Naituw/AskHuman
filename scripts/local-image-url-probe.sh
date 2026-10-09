#!/usr/bin/env bash
# Run the production image loader in a native WebView, independently of daemon and IM state.
set -euo pipefail
TASK_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$TASK_REPO_ROOT"
cargo run --manifest-path src-tauri/Cargo.toml --features custom-protocol --example local_image_url_probe -- "$@"
