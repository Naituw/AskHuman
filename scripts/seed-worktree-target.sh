#!/usr/bin/env bash
# Seed this checkout's src-tauri/target from another checkout using APFS clones (macOS only).
#
# A fresh worktree starts with an empty target/ and compiles every third-party crate (about
# 3.5 minutes at the local-install profile). Registry dependencies produce the same artifacts in
# every checkout, so a cloned target/ lets the first build compile only this project (about
# 1.5 minutes) and shares the dependency blocks copy-on-write instead of duplicating them.
# Measurements and caveats: docs/agent-worktree-setup.md.
#
# What it does, per profile directory (local-install, debug):
#   1. clones <source>/src-tauri/target/<dir> into this checkout (skipped if it already has output);
#   2. removes Swift module caches inside the clone: they embed their absolute path and make swiftc
#      fail from another location (the shared target/swift-module-cache is never cloned);
#   3. runs `cargo clean -p <every path package>` for that profile: path crates are judged fresh by
#      file mtime, so a clone could otherwise be reused although it was built from other sources.
# Any failure rolls the profile directory back, so a half-seeded target/ is never left behind.
set -euo pipefail

usage() {
  cat <<'EOF'
用法: ./scripts/seed-worktree-target.sh [--from <检出目录>] [--dry-run]

用 APFS clone 把另一个检出的 src-tauri/target（local-install 与 debug 两个 profile）播种到
当前检出，让新 worktree 的首次构建不必从头编译全部第三方依赖。仅支持 macOS/APFS，且来源与
当前检出必须在同一个卷上；其他环境会提示后直接跳过。

选项:
  --from <目录>  来源检出（默认：本仓库的主检出）
  --dry-run      只打印将要做的事，不改动任何文件
  -h, --help     显示本帮助
EOF
}

SOURCE_ROOT=""
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from)
      if [ $# -lt 2 ]; then
        echo "错误: --from 需要一个目录参数" >&2
        exit 1
      fi
      SOURCE_ROOT="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "错误: 未知参数 ${1}（可用 --from <目录>、--dry-run 或 --help）" >&2
      exit 1
      ;;
  esac
done

if [ "$(uname)" != "Darwin" ]; then
  echo "跳过: APFS clone 仅适用于 macOS；首次构建会完整编译全部依赖。"
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
cd "$REPO_ROOT"

# "<directory under target/>:<cargo profile that owns it>"
PROFILES=("local-install:local-install" "debug:dev")

DEST_TARGET="$REPO_ROOT/src-tauri/target"

# --- Resolve the source checkout (default: the main checkout of this repository) ---
if [ -z "$SOURCE_ROOT" ]; then
  if ! common_dir="$(git rev-parse --git-common-dir 2>/dev/null)"; then
    echo "错误: 当前目录不在 git 检出里，请用 --from <目录> 指定来源。" >&2
    exit 1
  fi
  common_dir="$(cd "$common_dir" && pwd -P)"
  if [ "$(basename "$common_dir")" != ".git" ]; then
    echo "错误: 无法从 ${common_dir} 推出主检出目录，请用 --from <目录> 指定来源。" >&2
    exit 1
  fi
  SOURCE_ROOT="$(dirname "$common_dir")"
fi
if ! SOURCE_ROOT="$(cd "$SOURCE_ROOT" 2>/dev/null && pwd -P)"; then
  echo "错误: 来源目录不存在: ${SOURCE_ROOT}" >&2
  exit 1
fi
if [ "$SOURCE_ROOT" = "$REPO_ROOT" ]; then
  echo "错误: 来源就是当前检出本身。请在新 worktree 里运行本脚本，或用 --from 指向另一个检出。" >&2
  exit 1
fi
SRC_TARGET="$SOURCE_ROOT/src-tauri/target"
if [ ! -d "$SRC_TARGET" ]; then
  echo "错误: 来源没有 ${SRC_TARGET}；请先在来源检出里构建一次。" >&2
  exit 1
fi

# --- Cloning only works on one APFS volume; cp -c silently degrades to a full copy elsewhere ---
_clone_supported() {
  local a="$1" b="$2" dev_a dev_b device
  dev_a="$(stat -f '%d' "$a")" || return 1
  dev_b="$(stat -f '%d' "$b")" || return 1
  [ "$dev_a" = "$dev_b" ] || return 1
  device="$(df -P "$a" | awk 'NR == 2 { print $1 }')"
  [ -n "$device" ] || return 1
  # awk decides by itself: `grep -q` would close the pipe early and trip pipefail via SIGPIPE.
  mount | awk -v dev="$device" '$1 == dev && /\(apfs,/ { found = 1 } END { exit !found }'
}
dest_probe="$REPO_ROOT/src-tauri"
[ -d "$dest_probe" ] || dest_probe="$REPO_ROOT"
if ! _clone_supported "$SRC_TARGET" "$dest_probe"; then
  echo "错误: 来源与当前检出不在同一个 APFS 卷上；cp -c 会退化成真拷贝（占满磁盘），已中止。" >&2
  exit 1
fi

# --- Packages whose sources live on this machine (the workspace crate and path patches) ---
_path_packages() {
  cargo metadata --format-version 1 --manifest-path src-tauri/Cargo.toml 2>/dev/null |
    node -e 'const d = JSON.parse(require("fs").readFileSync(0, "utf8")); for (const p of d.packages) if (p.source === null) console.log(p.name);' 2>/dev/null ||
    true
}
PACKAGE_ARGS=()
PACKAGE_LABEL=""
for name in $(_path_packages); do
  PACKAGE_ARGS+=(-p "$name")
  PACKAGE_LABEL="${PACKAGE_LABEL:+${PACKAGE_LABEL}, }${name}"
done
if [ "${#PACKAGE_ARGS[@]}" -eq 0 ]; then
  # Guessing would risk a stale path crate (for example the vendored drag) being reused: stop instead.
  echo "错误: 读不到 cargo metadata（需要 cargo 与 node），无法确定要清理哪些 path 包；已中止，未改动任何文件。" >&2
  exit 1
fi

SEEDED=""
FAILED=0
if [ "$DRY_RUN" = "1" ]; then
  echo "（--dry-run：以下只是计划，不会改动任何文件）"
fi
echo "==> 来源: ${SOURCE_ROOT}"
for entry in "${PROFILES[@]}"; do
  dir="${entry%%:*}"
  profile="${entry##*:}"
  src="$SRC_TARGET/$dir"
  dest="$DEST_TARGET/$dir"

  if [ ! -d "$src" ]; then
    echo "   ${dir}: 来源没有构建产物，跳过"
    continue
  fi
  if [ -d "$dest" ] && [ -n "$(ls -A "$dest" 2>/dev/null)" ]; then
    echo "   ${dir}: 这里已有构建产物，不覆盖，跳过"
    continue
  fi
  if [ "$DRY_RUN" = "1" ]; then
    echo "   ${dir}: 将 clone，并执行 cargo clean --profile ${profile}（${PACKAGE_LABEL}）"
    continue
  fi

  echo "==> ${dir}: APFS clone（几乎不占物理空间）"
  mkdir -p "$DEST_TARGET"
  rm -rf "$dest"
  if ! cp -c -R "$src" "$dest"; then
    echo "警告: ${dir} 克隆失败，已回滚。" >&2
    rm -rf "$dest"
    FAILED=1
    continue
  fi
  # Module caches embed their absolute path; a clone of one fails to load from this location.
  find "$dest" -name swift-module-cache -type d -prune -exec rm -rf {} + 2>/dev/null || true

  echo "==> ${dir}: 清理本检出的 path 包产物（${PACKAGE_LABEL}）"
  if ! cargo clean --manifest-path src-tauri/Cargo.toml --profile "$profile" "${PACKAGE_ARGS[@]}"; then
    echo "警告: ${dir} 的 cargo clean 失败，已回滚（否则可能复用别的检出编出来的产物）。" >&2
    rm -rf "$dest"
    FAILED=1
    continue
  fi
  SEEDED="${SEEDED:+${SEEDED} }${dir}"
done

if [ "$DRY_RUN" = "1" ]; then
  exit 0
fi

if [ -n "$SEEDED" ]; then
  log_file="$DEST_TARGET/askhuman-cache.log"
  printf '%s seed: cloned %s from %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$SEEDED" "$SOURCE_ROOT" \
    >>"$log_file" 2>/dev/null || true
  echo "==> 完成（${SEEDED}）。接着运行 ./scripts/install.sh：首次构建只编译本项目；Swift 模块缓存会冷启动重建一次。"
else
  echo "==> 没有播种任何 profile。"
fi
exit "$FAILED"
