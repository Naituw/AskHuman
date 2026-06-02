#!/usr/bin/env bash
# 发布驱动脚本：
#   1) 校验各处版本号一致（由 scripts/bump-version.mjs 保证）
#   2) 若当前版本已发布到 npm，则报错并提示用 bump-version 更新版本号
#   3) 否则打 tag 并推送，触发 .github/workflows/release.yml 完成真实发布
#      （编译 4 平台二进制 → npm publish 主包+子包 → 创建 GitHub Release）
#
# 用法:
#   ./publish.sh          # 交互确认后发布
#   ./publish.sh -y       # 跳过确认直接发布
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

REGISTRY="https://registry.npmjs.org"
PKG="packaging/npm/humaninloop/package.json"

for cmd in node npm git; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "错误: 需要 $cmd" >&2; exit 1; }
done

NAME=$(node -p "require('./$PKG').name")
VERSION=$(node -p "require('./$PKG').version")
TAG="v$VERSION"
echo "==> 包: $NAME   版本: $VERSION"

# 1) 版本一致性
fail_inconsistent() {
  echo "错误: $1 版本 ($2) 与主包 ($VERSION) 不一致。" >&2
  echo "      请统一版本号: node scripts/bump-version.mjs $VERSION" >&2
  exit 1
}
[ "$(node -p "require('./src-tauri/tauri.conf.json').version")" = "$VERSION" ] \
  || fail_inconsistent "tauri.conf.json" "$(node -p "require('./src-tauri/tauri.conf.json').version")"
for sub in darwin-arm64 darwin-x64 win32-x64 linux-x64; do
  v=$(node -p "require('./packaging/npm/platforms/$sub/package.json').version")
  [ "$v" = "$VERSION" ] || fail_inconsistent "humaninloop-$sub" "$v"
done
CARGO_VER=$(grep -m1 '^version' src-tauri/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
[ "$CARGO_VER" = "$VERSION" ] || fail_inconsistent "Cargo.toml" "$CARGO_VER"

# 2) 该版本是否已发布到 npm
PUBLISHED=$(npm view "$NAME@$VERSION" version --registry "$REGISTRY" 2>/dev/null || true)
if [ -n "$PUBLISHED" ]; then
  echo "错误: $NAME@$VERSION 已发布到 npm，无法重复发布。" >&2
  echo "      请先更新版本号再发布: node scripts/bump-version.mjs <新版本>" >&2
  echo "      （如 patch: node scripts/bump-version.mjs ${VERSION%.*}.$(( ${VERSION##*.} + 1 )) ）" >&2
  exit 1
fi

# 3) 发布前置检查
if [ -n "$(git status --porcelain)" ]; then
  echo "错误: 工作区存在未提交改动，请先提交后再发布。" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "错误: 本地已存在 tag $TAG。如需重发请先删除: git tag -d $TAG && git push origin :refs/tags/$TAG" >&2
  exit 1
fi

# 4) 确认
if [ "${1:-}" != "-y" ] && [ "${1:-}" != "--yes" ]; then
  printf "确认发布 %s ？将打 tag 并推送，触发 CI 真实发布到 npm/GitHub Release [y/N] " "$TAG"
  read -r ans
  case "$ans" in
    [yY] | [yY][eE][sS]) ;;
    *) echo "已取消"; exit 0 ;;
  esac
fi

# 5) 打 tag 并推送，触发 CI 发布
echo "==> 创建并推送 tag $TAG"
git tag -a "$TAG" -m "release: $TAG"
git push
git push origin "$TAG"

echo "==> 已推送 $TAG。"
echo "    在 GitHub Actions 的 release 工作流查看发布进度；"
echo "    成功后可用 'npm view $NAME@$VERSION' 验证。"
