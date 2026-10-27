#!/bin/bash
# Install the CloudKit helper of the cross-Mac todo sync (docs/plans/todo-sync.md P3, self-use stage):
# copy the built app into ~/Applications, register it as a login item and say hello over its socket.
#
#   scripts/todo-sync-install-helper.sh [path/to/AskHumanTodoSync.app]
#
# Without an argument the newest build under todo-sync-helper/build/out/ is used (devid > dev > unsigned).
# On a second Mac: copy the .app over (scp / AirDrop is fine), then run this script there too.
#
# env: TODO_SYNC_APP_DIR   install directory (default ~/Applications)
#      TODO_SYNC_DIR       state directory with helper.sock (default ~/.askhuman/state/todo-sync)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/.." && pwd)"

SRC="${1:-}"
if [ -z "$SRC" ]; then
    for mode in devid dev unsigned; do
        candidate="$repo/todo-sync-helper/build/out/$mode/AskHumanTodoSync.app"
        if [ -d "$candidate" ]; then
            SRC="$candidate"
            break
        fi
    done
fi
[ -n "$SRC" ] && [ -d "$SRC" ] || {
    echo "no helper app found; build one with todo-sync-helper/scripts/build.sh or pass its path" >&2
    exit 2
}

DEST_DIR="${TODO_SYNC_APP_DIR:-$HOME/Applications}"
DEST="$DEST_DIR/AskHumanTodoSync.app"
STATE_DIR="${TODO_SYNC_DIR:-$HOME/.askhuman/state/todo-sync}"
LSREG=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

echo "==> stop a running helper"
if [ -x "$DEST/Contents/MacOS/AskHumanTodoSync" ]; then
    "$DEST/Contents/MacOS/AskHumanTodoSync" --unregister >/dev/null 2>&1 || true
fi
pkill -f "$DEST/Contents/MacOS/AskHumanTodoSync" 2>/dev/null || true
sleep 1

echo "==> install $DEST"
mkdir -p "$DEST_DIR"
rm -rf "$DEST"
ditto "$SRC" "$DEST"
if xattr -p com.apple.quarantine "$DEST" >/dev/null 2>&1; then
    echo "NOTE: the app carries com.apple.quarantine (downloaded / AirDropped)."
    echo "      Gatekeeper may refuse to launch it unless it is notarized; the self-use build is not."
fi
"$LSREG" -f "$DEST"

echo "==> register as login item"
status=0
"$DEST/Contents/MacOS/AskHumanTodoSync" --register || status=$?
if [ "$status" = 3 ]; then
    echo "Approve 'AskHuman Todo Sync' in System Settings > General > Login Items, then run this script again."
    exit 3
elif [ "$status" != 0 ]; then
    echo "registration failed (exit $status)" >&2
    exit "$status"
fi

echo "==> wait for the RPC socket"
sock="$STATE_DIR/helper.sock"
for _ in $(seq 1 30); do
    [ -S "$sock" ] && break
    sleep 0.5
done
[ -S "$sock" ] || {
    echo "helper did not create $sock (see ~/Library/Logs/AskHumanTodoSync/helper.log)" >&2
    exit 1
}

echo "==> hello"
python3 - "$sock" <<'EOF'
import json, socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(10)
s.connect(sys.argv[1])
s.sendall(b'{"id":1,"op":"hello"}\n')
buf = b""
while not buf.endswith(b"\n"):
    chunk = s.recv(65536)
    if not chunk:
        break
    buf += chunk
print(buf.decode().strip())
EOF
