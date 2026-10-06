#!/usr/bin/env bash
# Run the isolated fixed-viewport experiment, without product configuration or daemon access.
set -euo pipefail
FOUNDATION_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$FOUNDATION_ROOT"
FOUNDATION_PORT="${ASKHUMAN_FOUNDATION_PORT:-5199}"
mkdir -p .askhuman-dev
export ASKHUMAN_FOUNDATION_LOG="$FOUNDATION_ROOT/.askhuman-dev/pane-foundation.jsonl"
if [ "${1:-}" = "--auto" ]; then
  export ASKHUMAN_FOUNDATION_AUTO=1
  : > "$ASKHUMAN_FOUNDATION_LOG"
elif [ -n "${1:-}" ]; then
  echo "usage: scripts/pane-foundation-demo.sh [--auto]" >&2
  exit 1
fi
export TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:$FOUNDATION_PORT\"}}"
pnpm exec vite --port "$FOUNDATION_PORT" --strictPort > .askhuman-dev/pane-foundation-vite.log 2>&1 &
FOUNDATION_VITE_PID=$!
trap 'kill "$FOUNDATION_VITE_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 80); do
  if curl -fsS "http://localhost:$FOUNDATION_PORT/prototype/pane-foundation.html" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$FOUNDATION_VITE_PID" 2>/dev/null; then cat .askhuman-dev/pane-foundation-vite.log >&2; exit 1; fi
  sleep .1
done
cargo build --manifest-path src-tauri/Cargo.toml --example pane_foundation_demo
FOUNDATION_EXE="$FOUNDATION_ROOT/src-tauri/target/debug/examples/pane_foundation_demo"
if [ "$(uname -s)" = "Darwin" ]; then
  FOUNDATION_APP="$FOUNDATION_ROOT/.askhuman-dev/PaneFoundationDemo.app/Contents"
  mkdir -p "$FOUNDATION_APP/MacOS"
  ln -sfn "$FOUNDATION_EXE" "$FOUNDATION_APP/MacOS/PaneFoundationDemo"
  cat > "$FOUNDATION_APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Pane Foundation Demo</string>
<key>CFBundleDisplayName</key><string>Pane Foundation Demo</string>
<key>CFBundleIdentifier</key><string>com.naituw.askhuman.pane-foundation-demo</string>
<key>CFBundleExecutable</key><string>PaneFoundationDemo</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
  FOUNDATION_EXE="$FOUNDATION_APP/MacOS/PaneFoundationDemo"
fi
"$FOUNDATION_EXE"
if [ "${ASKHUMAN_FOUNDATION_AUTO:-}" = "1" ]; then
  pnpm exec node --input-type=module - "$ASKHUMAN_FOUNDATION_LOG" <<'JS'
import { readFileSync } from 'node:fs';
const rows = readFileSync(process.argv[2], 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
const report = rows[rows.length - 1];
if (!report?.pass) { console.error('Fixed-viewport native invariants failed:', JSON.stringify(report)); process.exit(1); }
console.log('Fixed-viewport native runs:', report.results.length);
JS
fi
