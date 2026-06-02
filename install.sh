#!/usr/bin/env bash
# 构建并安装 AskHuman 到 ~/.local/bin（macOS / Linux）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
cd "$SCRIPT_DIR"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "错误: 需要 pnpm（npm i -g pnpm）" >&2
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "错误: 需要 Rust 工具链（https://rustup.rs）" >&2
  exit 1
fi

echo "==> 安装前端依赖"
pnpm install

echo "==> 构建前端 (dist/)"
pnpm build

echo "==> 编译 release 二进制（前端资源在此步骤被嵌入）"
cargo build --release --manifest-path src-tauri/Cargo.toml

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
  # 清除 quarantine 并 ad-hoc 重签名，降低拷贝后被 Gatekeeper 拦截的概率
  xattr -d com.apple.quarantine "$INSTALL_DIR/AskHuman" 2>/dev/null || true
  echo "==> 重新签名 (ad-hoc)"
  codesign --force --sign - "$INSTALL_DIR/AskHuman" 2>/dev/null || true
fi

echo "==> 完成：$INSTALL_DIR/AskHuman"
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo "提示: $INSTALL_DIR 不在 PATH 中，请将其加入 PATH。"
fi
