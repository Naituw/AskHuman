#!/usr/bin/env bash
# 构建并安装 AskHuman 到 ~/.local/bin（macOS / Linux）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
cd "$REPO_ROOT"

sign_via_gui_launchd() {
  local identity="$1"
  local target="$2"
  local sign_dir status_file log_file runner_file plist_file label service gui_rc

  sign_dir="$(mktemp -d "${TMPDIR:-/tmp}/askhuman-sign.XXXXXX")"
  status_file="$sign_dir/status"
  log_file="$sign_dir/codesign.log"
  runner_file="$sign_dir/sign.sh"
  plist_file="$sign_dir/sign.plist"
  label="com.naituw.askhuman-sign.$$"
  service="gui/$(id -u)/$label"

  {
    echo '#!/usr/bin/env bash'
    printf '/usr/bin/codesign -i %q --force --sign %q %q > %q 2>&1\n' \
      "com.naituw.humaninloop" "$identity" "$target" "$log_file"
    printf 'rc=$?\nprintf "%%s\\n" "$rc" > %q\nexit "$rc"\n' "$status_file"
  } > "$runner_file"
  chmod 0700 "$runner_file"

  cat > "$plist_file" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$label</string>
  <key>ProgramArguments</key>
  <array>
    <string>$runner_file</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
PLIST

  if ! launchctl bootstrap "gui/$(id -u)" "$plist_file"; then
    rm -rf "$sign_dir"
    return 1
  fi

  # A one-shot agent in the user's GUI launchd domain retains keychain access without opening
  # Terminal, even when the installer itself runs under a background Codex app-server.
  for _ in $(seq 1 300); do
    [ -f "$status_file" ] && break
    sleep 0.1
  done
  if [ ! -f "$status_file" ]; then
    echo "错误: 等待 GUI 会话正式签名超时" >&2
    launchctl bootout "$service" 2>/dev/null || true
    rm -rf "$sign_dir"
    return 1
  fi

  gui_rc="$(cat "$status_file")"
  cat "$log_file"
  launchctl bootout "$service" 2>/dev/null || true
  rm -rf "$sign_dir"
  [ "$gui_rc" = "0" ]
}

if ! command -v pnpm >/dev/null 2>&1; then
  echo "错误: 需要 pnpm（npm i -g pnpm）" >&2
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "错误: 需要 Rust 工具链（https://rustup.rs）" >&2
  exit 1
fi

# 在途请求提示：daemon 正服务中的提问不会被安装打断——换新会在它们完结后自动发生（graceful drain），
# 期间新提问会等待。此处只提示，不强杀。
if command -v AskHuman >/dev/null 2>&1; then
  ACTIVE="$(AskHuman daemon status 2>/dev/null | sed -n 's/.*requests[[:space:]]*\([0-9][0-9]*\) active.*/\1/p' | head -n1 || true)"
  if [ -n "${ACTIVE:-}" ] && [ "$ACTIVE" -gt 0 ] 2>/dev/null; then
    echo "提示: daemon 当前有 $ACTIVE 个在途请求；安装后将在它们完结后自动换新（期间新提问会等待）。"
    echo "      立即换新: AskHuman daemon restart --force（会打断在途请求）"
  fi
fi

echo "==> 安装前端依赖"
pnpm install

echo "==> 构建前端 (dist/)"
pnpm build

echo "==> 编译 release 二进制（前端资源在此步骤被嵌入）"
# --features custom-protocol：生产构建必须启用，否则二进制以 dev 模式连 devUrl 导致白屏。
cargo build --release --manifest-path src-tauri/Cargo.toml --features custom-protocol

BIN_PATH="src-tauri/target/release/AskHuman"
if [ ! -f "$BIN_PATH" ]; then
  echo "错误: 未找到编译产物 $BIN_PATH" >&2
  exit 1
fi

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
cp "$BIN_PATH" "$INSTALL_DIR/AskHuman"
chmod 0755 "$INSTALL_DIR/AskHuman"

if [ "$(uname)" = "Darwin" ]; then
  # 清除 quarantine，降低拷贝后被 Gatekeeper 拦截的概率
  xattr -d com.apple.quarantine "$INSTALL_DIR/AskHuman" 2>/dev/null || true
  # Sign with a stable identity + fixed identifier so the OS keychain trusts the binary across
  # rebuilds (its designated requirement is cdhash-independent) → secret reads stay prompt-free.
  # Identity: $CODESIGN_IDENTITY if set, else prefer a "Developer ID Application" cert, else the
  # first available codesigning cert, else ad-hoc (per-build keychain prompts).
  #
  # Prefer Developer ID *deterministically*: `find-identity` order is not stable, and when both a
  # "Developer ID Application" and an "Apple Development" cert exist, picking whichever lands first
  # flips the binary's designated requirement between installs → the keychain ACL stops trusting it
  # → silent secret reads break (esp. for the background daemon). Developer ID's DR is also cdhash-
  # independent and non-expiring, so pinning it keeps the ACL valid across rebuilds.
  IDENTITY="${CODESIGN_IDENTITY:-}"
  if [ -z "$IDENTITY" ]; then
    IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | awk '/Developer ID Application/{print $2; exit}')"
  fi
  if [ -z "$IDENTITY" ]; then
    IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | awk '/^[[:space:]]*[0-9]+\)/{print $2; exit}')"
  fi
  [ -z "$IDENTITY" ] && IDENTITY="-"
  if [ "$IDENTITY" = "-" ]; then
    echo "==> 签名 (ad-hoc; 设置 CODESIGN_IDENTITY 可避免每次重装的钥匙串弹框)"
    codesign -i com.naituw.humaninloop --force --sign "$IDENTITY" "$INSTALL_DIR/AskHuman"
  else
    echo "==> 签名 (identity: $IDENTITY, identifier: com.naituw.humaninloop)"
    if ! codesign -i com.naituw.humaninloop --force --sign "$IDENTITY" "$INSTALL_DIR/AskHuman"; then
      echo "==> 后台进程无法使用正式签名私钥，改由用户 GUI 会话完成签名"
      sign_via_gui_launchd "$IDENTITY" "$INSTALL_DIR/AskHuman" || {
        echo "错误: 正式签名失败，安装已中止" >&2
        exit 1
      }
    fi
  fi
  codesign --verify --strict "$INSTALL_DIR/AskHuman"
fi

# 顺手回收 target/ 历史编译残留：cargo 不会自动 GC 旧哈希产物（同一库会堆出多份），
# 装了 cargo-sweep 就清掉 14 天未使用的（保留近期产物，下次仍可增量编译）。未装则跳过（best-effort）。
if command -v cargo-sweep >/dev/null 2>&1; then
  echo "==> 清理 target 历史残留 (cargo sweep --time 14)"
  ( cd src-tauri && cargo sweep --time 14 ) >/dev/null 2>&1 || true
fi

echo "==> 完成：$INSTALL_DIR/AskHuman"
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo "提示: $INSTALL_DIR 不在 PATH 中，请将其加入 PATH。"
fi
