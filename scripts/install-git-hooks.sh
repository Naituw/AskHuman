#!/usr/bin/env bash
# Install repo-managed git hooks into this clone's .git/hooks (does not replace LFS hooks).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_SRC="$ROOT/scripts/githooks/pre-commit"
HOOKS_DIR="$(git -C "$ROOT" rev-parse --git-path hooks)"
HOOK_DST="$HOOKS_DIR/pre-commit"

if [ ! -f "$HOOK_SRC" ]; then
  echo "error: missing $HOOK_SRC" >&2
  exit 1
fi

mkdir -p "$HOOKS_DIR"
chmod +x "$HOOK_SRC"

# Prefer a relative symlink so the hook tracks the script in the working tree.
rel_src="$(python3 - <<PY
import os
print(os.path.relpath("$HOOK_SRC", "$HOOKS_DIR"))
PY
)"

ln -sfn "$rel_src" "$HOOK_DST"
echo "Installed pre-commit hook -> $HOOK_DST"
echo "  (rustfmt when staged src-tauri/**/*.rs change; bypass with --no-verify)"
