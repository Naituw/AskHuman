#!/usr/bin/env bash
# Run the mock-only collapsible-inbox prototype (docs/specs/popup-inbox-collapse.md) on its own
# development port. It never connects to the product daemon, IM channels or user configuration.
#
#   scripts/popup-collapse-demo.sh                 interactive (controller window + answer window)
#   scripts/popup-collapse-demo.sh --effect glass  window material: blur (default) | glass | solid
#   scripts/popup-collapse-demo.sh --auto          native geometry/state checks, no controller window
#   scripts/popup-collapse-demo.sh --auto-controls the controller window drives the answer window
#                                                  through its real event channel and verifies it
#   (both run silently: the process refuses to activate itself, so the keyboard focus stays with whoever is working;
#    add --focus to let it take the focus, which the one check that needs a real app switch requires)
#   scripts/popup-collapse-demo.sh --burst         record the composited frames of collapse / expand cycles as
#                                                  PNG sequences under .askhuman-dev/popup-collapse-shots/
#   scripts/popup-collapse-demo.sh --video         run collapse / expand cycles on a plain backdrop while a
#                                                  ScreenCaptureKit probe (scripts/popup-collapse-frames.swift,
#                                                  needs the screen recording permission of the calling terminal)
#                                                  logs every displayed frame to .askhuman-dev/popup-collapse-frames.tsv
set -euo pipefail
DEMO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DEMO_ROOT"
DEMO_PORT="${ASKHUMAN_COLLAPSE_PORT:-5199}"
DEMO_DIR="$DEMO_ROOT/.askhuman-dev"
DEMO_LOG="$DEMO_DIR/popup-collapse-demo.jsonl"
mkdir -p "$DEMO_DIR"

usage() {
  echo "usage: scripts/popup-collapse-demo.sh [--auto | --auto-controls | --burst | --video] [--focus] [--effect blur|glass|solid]" >&2
  exit 1
}
while [ $# -gt 0 ]; do
  case "$1" in
    --auto)
      export ASKHUMAN_DEMO_AUTOTEST=1
      : > "$DEMO_LOG"
      shift
      ;;
    --auto-controls)
      export ASKHUMAN_DEMO_AUTOTEST=controls
      : > "$DEMO_LOG"
      shift
      ;;
    --burst)
      export ASKHUMAN_DEMO_AUTOTEST=burst
      : > "$DEMO_LOG"
      shift
      ;;
    --video)
      export ASKHUMAN_DEMO_AUTOTEST=video
      : > "$DEMO_LOG"
      shift
      ;;
    --focus)
      export ASKHUMAN_DEMO_FOCUS=1
      shift
      ;;
    --effect)
      [ $# -ge 2 ] || usage
      case "$2" in blur | glass | solid) export ASKHUMAN_DEMO_EFFECT="$2" ;; *) usage ;; esac
      shift 2
      ;;
    *) usage ;;
  esac
done

export ASKHUMAN_DEMO_LOG="$DEMO_LOG"
export TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:$DEMO_PORT\"}}"
pnpm exec vite --port "$DEMO_PORT" --strictPort > "$DEMO_DIR/popup-collapse-vite.log" 2>&1 &
DEMO_VITE_PID=$!
trap 'kill "$DEMO_VITE_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 100); do
  if curl -fsS "http://localhost:$DEMO_PORT/prototype/popup-collapse.html" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$DEMO_VITE_PID" 2>/dev/null; then
    cat "$DEMO_DIR/popup-collapse-vite.log" >&2
    exit 1
  fi
  sleep 0.1
done

# The verification-only Swift helpers (swift/prototype/) are compiled only with this feature.
cargo build --manifest-path src-tauri/Cargo.toml --example popup_collapse_demo --features collapse-prototype
DEMO_EXE="$DEMO_ROOT/src-tauri/target/debug/examples/popup_collapse_demo"
if [ "$(uname -s)" = "Darwin" ]; then
  # A separate app identity keeps the prototype apart from the product (Dock, window lists, UI inspection).
  DEMO_APP="$DEMO_DIR/PopupCollapseDemo.app/Contents"
  mkdir -p "$DEMO_APP/MacOS"
  ln -sfn "$DEMO_EXE" "$DEMO_APP/MacOS/PopupCollapseDemo"
  cat > "$DEMO_APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Popup Collapse Demo</string>
<key>CFBundleDisplayName</key><string>Popup Collapse Demo</string>
<key>CFBundleIdentifier</key><string>com.naituw.askhuman.popup-collapse-demo</string>
<key>CFBundleExecutable</key><string>PopupCollapseDemo</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
  DEMO_EXE="$DEMO_APP/MacOS/PopupCollapseDemo"
fi
DEMO_FRAMES="$DEMO_DIR/popup-collapse-frames.tsv"
if [ "${ASKHUMAN_DEMO_AUTOTEST:-}" = "video" ]; then
  DEMO_FRAMES_TOOL="$DEMO_DIR/popup-collapse-frames"
  if [ ! -x "$DEMO_FRAMES_TOOL" ] || [ "$DEMO_ROOT/scripts/popup-collapse-frames.swift" -nt "$DEMO_FRAMES_TOOL" ]; then
    swiftc -O -o "$DEMO_FRAMES_TOOL" "$DEMO_ROOT/scripts/popup-collapse-frames.swift"
  fi
  rm -f "$DEMO_FRAMES"
  (
    # Start probing once the page has put the backdrop up (the scenario then waits a few seconds).
    until grep -q '"label":"video-ready"' "$DEMO_LOG" 2>/dev/null; do sleep 0.1; done
    exec "$DEMO_FRAMES_TOOL" 400 60 1000 760 "$DEMO_FRAMES" > /dev/null
  ) &
  DEMO_RECORDER_PID=$!
fi
"$DEMO_EXE" || DEMO_STATUS=$?

if [ -n "${DEMO_RECORDER_PID:-}" ]; then
  # SIGINT lets the probe stop its stream and flush the log.
  kill -INT "$DEMO_RECORDER_PID" 2>/dev/null || true
  wait "$DEMO_RECORDER_PID" 2>/dev/null || true
  echo "frame log: $DEMO_FRAMES ($(wc -l < "$DEMO_FRAMES" | tr -d ' ') frames)"
  exit "${DEMO_STATUS:-0}"
fi

if [ "${ASKHUMAN_DEMO_AUTOTEST:-}" = "burst" ]; then
  pnpm exec node --input-type=module - "$DEMO_LOG" <<'JS'
import { readFileSync } from "node:fs";
const entries = readFileSync(process.argv[2], "utf8").trim().split("\n").filter(Boolean).map(JSON.parse);
const result = [...entries].reverse().find(entry => entry.label === "burst-result");
if (!result) { console.error("burst run produced no result"); process.exit(1); }
for (const line of result.summary ?? []) console.log(line);
if (result.error) console.error(result.error);
JS
elif [ -n "${ASKHUMAN_DEMO_AUTOTEST:-}" ]; then
  # macOS may end the process through AppKit with a zero status; trust the recorded report instead.
  pnpm exec node --input-type=module - "$DEMO_LOG" <<'JS'
import { readFileSync } from "node:fs";
const entries = readFileSync(process.argv[2], "utf8").trim().split("\n").filter(Boolean).map(JSON.parse);
const transitions = entries.filter(entry => entry.label === "transition");
const report = [...entries].reverse().find(entry => Array.isArray(entry.results));
if (!report) {
  console.error("Native prototype verification failed: missing report");
  process.exit(1);
}
for (const check of report.results) {
  console.log(`${check.pass ? "PASS" : "FAIL"}  ${check.name}${check.detail ? `  (${check.detail})` : ""}`);
}
console.log(`\n${transitions.length} native transitions recorded:`);
for (const t of transitions) {
  const cost = t.report ? `  start ${t.report.startDelayMs.toFixed(0)}ms  frame cost mean ${t.report.meanCostMs.toFixed(1)} / max ${t.report.maxCostMs.toFixed(1)} ms` : "";
  const page = t.page ? `  page ${t.page.frames} frames, max gap ${t.page.maxGapMs.toFixed(1)} ms, slow ${t.page.slowFrames}` : "";
  console.log(`  ${t.kind.padEnd(8)} ${String(t.frames).padStart(3)} frames  gap mean ${t.meanGapMs.toFixed(1)} / p95 ${t.p95GapMs.toFixed(1)} / max ${t.maxGapMs.toFixed(1)} ms  dropped ${t.dropped}  err ${t.maxErrorPx.toFixed(2)}px  ${Math.round(t.refreshHz)}Hz${cost}${page}`);
}
if (report.pass !== true) {
  console.error("\nNative prototype verification failed.");
  process.exit(1);
}
console.log("\nNative prototype verification passed.");
JS
fi
