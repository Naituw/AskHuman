#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "==> 编译 GUI (release)"
swift build -c release --product SpeechGUI

BIN=".build/release/SpeechGUI"
APP=".build/SpeechDemo.app"

echo "==> 打包 .app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/SpeechGUI"
cp gui/Info.plist "$APP/Contents/Info.plist"

echo "==> 签名 (ad-hoc, 带麦克风权限)"
codesign --force --sign - \
    --entitlements gui/SpeechDemo.entitlements \
    "$APP"

echo "==> 关闭旧实例"
killall SpeechGUI 2>/dev/null || true
sleep 0.5

echo "==> 启动 $APP"
open -n "$APP"
echo "若没弹权限，去 系统设置→隐私与安全性→麦克风/语音识别 给 SpeechDemo 打勾后重开。"
