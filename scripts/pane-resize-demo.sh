#!/usr/bin/env bash
# Verify the responsive/frozen canvas handoff without daemon or product configuration.
set -euo pipefail
RESIZE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RESIZE_ROOT"
RESIZE_PORT="${ASKHUMAN_RESIZE_PORT:-5201}"
mkdir -p .askhuman-dev
export ASKHUMAN_RESIZE_LOG="$RESIZE_ROOT/.askhuman-dev/pane-resize.jsonl"
if [ "${1:-}" = "--auto" ]; then
  export ASKHUMAN_RESIZE_AUTO=1
  : > "$ASKHUMAN_RESIZE_LOG"
elif [ -n "${1:-}" ]; then
  if [ "$1" = "--polish" ]; then
    export ASKHUMAN_RESIZE_POLISH=1
    export ASKHUMAN_RESIZE_PDF="$RESIZE_ROOT/.askhuman-dev/native-fixture.pdf"
  else
    echo "usage: scripts/pane-resize-demo.sh [--auto|--polish]" >&2
    exit 1
  fi
fi
export TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:$RESIZE_PORT\"}}"
pnpm exec vite --port "$RESIZE_PORT" --strictPort > .askhuman-dev/pane-resize-vite.log 2>&1 &
RESIZE_VITE_PID=$!
trap 'kill "$RESIZE_VITE_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 80); do
  if curl -fsS "http://localhost:$RESIZE_PORT/prototype/pane-resize.html" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$RESIZE_VITE_PID" 2>/dev/null; then cat .askhuman-dev/pane-resize-vite.log >&2; exit 1; fi
  sleep .1
done
cargo build --manifest-path src-tauri/Cargo.toml --example pane_resize_demo
RESIZE_EXE="$RESIZE_ROOT/src-tauri/target/debug/examples/pane_resize_demo"
if [ "$(uname -s)" = "Darwin" ]; then
  RESIZE_APP="$RESIZE_ROOT/.askhuman-dev/PaneResizeDemo.app/Contents"
  mkdir -p "$RESIZE_APP/MacOS"
  cp "$RESIZE_EXE" "$RESIZE_APP/MacOS/PaneResizeDemo"
  cat > "$RESIZE_APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>
<key>CFBundleName</key><string>Pane Resize Demo</string>
<key>CFBundleIdentifier</key><string>com.naituw.askhuman.pane-resize-demo</string>
<key>CFBundleExecutable</key><string>PaneResizeDemo</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
  codesign --force --sign - "$RESIZE_ROOT/.askhuman-dev/PaneResizeDemo.app" >/dev/null 2>&1
  RESIZE_EXE="$RESIZE_APP/MacOS/PaneResizeDemo"
fi
"$RESIZE_EXE"
if [ "${ASKHUMAN_RESIZE_AUTO:-}" = "1" ]; then
  pnpm exec node --input-type=module - "$ASKHUMAN_RESIZE_LOG" <<'JS'
import { readFileSync } from 'node:fs';
const rows=readFileSync(process.argv[2],'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
const report=rows.at(-1);
if (!report?.pass) { console.error('Responsive handoff failed:', JSON.stringify(report));process.exit(1); }
console.log('Responsive handoff checks:', report.results.length);
JS
fi
