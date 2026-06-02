#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

echo "==> 编译 release 版本"
cd "$SCRIPT_DIR"
swift build -c release

BIN_PATH="$(swift build -c release --show-bin-path)/AskHuman"
if [ ! -f "$BIN_PATH" ]; then
  echo "错误: 未找到编译产物 $BIN_PATH" >&2
  exit 1
fi

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
cp "$BIN_PATH" "$INSTALL_DIR/AskHuman"
chmod 0755 "$INSTALL_DIR/AskHuman"

echo "==> 完成：$INSTALL_DIR/AskHuman"
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo "提示: $INSTALL_DIR 不在 PATH 中，请将其加入 PATH。"
fi
