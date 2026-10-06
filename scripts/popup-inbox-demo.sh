#!/usr/bin/env bash
# Run the mock-only Vue/Tauri prototype on its own development port.
set -euo pipefail
DEMO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DEMO_ROOT"
DEMO_PORT="${ASKHUMAN_DEMO_PORT:-5198}"
DEMO_LOG="$DEMO_ROOT/.askhuman-dev/popup-inbox-demo.jsonl"
mkdir -p "$DEMO_ROOT/.askhuman-dev"
if [ "${1:-}" = "--auto" ]; then
  export ASKHUMAN_DEMO_AUTOTEST=1
  : > "$DEMO_LOG"
elif [ -n "${1:-}" ]; then
  echo "usage: scripts/popup-inbox-demo.sh [--auto]" >&2
  exit 1
fi
export ASKHUMAN_DEMO_LOG="$DEMO_LOG"
export TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:$DEMO_PORT\"}}"
pnpm exec vite --port "$DEMO_PORT" --strictPort > "$DEMO_ROOT/.askhuman-dev/popup-inbox-vite.log" 2>&1 &
DEMO_VITE_PID=$!
trap 'kill "$DEMO_VITE_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 80); do
  if curl -fsS "http://localhost:$DEMO_PORT/prototype/popup-inbox.html" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$DEMO_VITE_PID" 2>/dev/null; then
    cat "$DEMO_ROOT/.askhuman-dev/popup-inbox-vite.log" >&2
    exit 1
  fi
  sleep 0.1
done
cargo build --manifest-path src-tauri/Cargo.toml --example popup_inbox_demo
DEMO_EXE="$DEMO_ROOT/src-tauri/target/debug/examples/popup_inbox_demo"
if [ "$(uname -s)" = "Darwin" ]; then
  # Give the prototype a separate app identity so native UI inspection can select it reliably.
  DEMO_APP="$DEMO_ROOT/.askhuman-dev/PopupInboxDemo.app/Contents"
  mkdir -p "$DEMO_APP/MacOS"
  ln -sfn "$DEMO_EXE" "$DEMO_APP/MacOS/PopupInboxDemo"
  cat > "$DEMO_APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Popup Inbox Demo</string>
<key>CFBundleDisplayName</key><string>Popup Inbox Demo</string>
<key>CFBundleIdentifier</key><string>com.naituw.askhuman.popup-inbox-demo</string>
<key>CFBundleExecutable</key><string>PopupInboxDemo</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
  DEMO_EXE="$DEMO_APP/MacOS/PopupInboxDemo"
fi
"$DEMO_EXE"
if [ "${ASKHUMAN_DEMO_AUTOTEST:-}" = "1" ]; then
  # macOS may terminate the application through AppKit with a zero process status.
  # Check the actual native test report before accepting the run as successful.
  pnpm exec node --input-type=module - "$DEMO_LOG" <<'JS'
import { readFileSync } from "node:fs";
const entries = readFileSync(process.argv[2], "utf8").trim().split("\n").filter(Boolean).map(JSON.parse);
const report = entries.reverse().find(entry => Array.isArray(entry.results));
if (report?.pass !== true) {
  console.error("Native prototype verification failed:", JSON.stringify(report ?? "missing report"));
  process.exit(1);
}
JS
fi
