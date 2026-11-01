#!/usr/bin/env bash
# 构建并安装 AskHuman 到 ~/.local/bin（macOS / Linux）。
# 若 cwd 在已 `dev enable` 的 worktree 内且未传 --global，则装到该树
# `.askhuman-dev/bin`（见 docs/specs/dev-instance-parallel.md）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

FORCE_GLOBAL=0
BUILD_PROFILE="local-install"
for arg in "$@"; do
  case "$arg" in
    --global) FORCE_GLOBAL=1 ;;
    --release) BUILD_PROFILE="release" ;;
    -h|--help)
      cat <<'EOF'
Usage: ./scripts/install.sh [--global] [--release]

  (default)  If the current directory is under a Dev Instance
             (worktree with .askhuman-dev/workspace.json or legacy enabled), install into
             <root>/.askhuman-dev/bin; otherwise ~/.local/bin. Uses the
             fast local `local-install` Cargo profile.
  --global   Always install to ~/.local/bin (or $INSTALL_DIR if set).
  --release  Build with the production `release` profile instead.

Environment:
  INSTALL_DIR   Explicit install directory (wins over auto-detect;
                with --global, still defaults to ~/.local/bin when unset).
EOF
      exit 0
      ;;
    *)
      echo "错误: 未知参数 ${arg}（可用 --global、--release 或 --help）" >&2
      exit 1
      ;;
  esac
done

find_dev_enabled_root() {
  local dir
  dir="$(pwd)"
  while [ -n "$dir" ] && [ "$dir" != "/" ]; do
    if [ -e "$dir/.askhuman-dev/workspace.json" ] || [ -L "$dir/.askhuman-dev/workspace.json" ] || [ -e "$dir/.askhuman-dev/enabled" ] || [ -L "$dir/.askhuman-dev/enabled" ]; then
      printf '%s\n' "$dir"
      return 0
    fi
    dir="$(dirname "$dir")"
  done
  return 1
}

DEFAULT_INSTALL_DIR="${HOME}/.local/bin"
DEV_ROOT=""
if [ -n "${INSTALL_DIR:-}" ]; then
  : # explicit env wins
elif [ "$FORCE_GLOBAL" -eq 1 ]; then
  INSTALL_DIR="$DEFAULT_INSTALL_DIR"
elif DEV_ROOT="$(find_dev_enabled_root)"; then
  INSTALL_DIR="${DEV_ROOT}/.askhuman-dev/bin"
  mkdir -p "$INSTALL_DIR" "${DEV_ROOT}/.askhuman-dev/home"
  echo "==> Dev Instance 检测到: $DEV_ROOT"
  echo "    安装目标: $INSTALL_DIR"
else
  INSTALL_DIR="$DEFAULT_INSTALL_DIR"
fi

DEV_SOURCE_SNAPSHOT=""
if [ -n "$DEV_ROOT" ]; then
  node "$SCRIPT_DIR/dev-build-record.mjs" assert-target "$DEV_ROOT" "$INSTALL_DIR/AskHuman"
  DEV_SOURCE_SNAPSHOT="$(node "$SCRIPT_DIR/dev-build-record.mjs" snapshot "$REPO_ROOT")"
fi

# Diagnostics must not inherit the calling Agent's client-only context.
run_installed() (
  unset ASKHUMAN_RESOLVED_TARGET_V1 ASKHUMAN_DEV_TEST_PROFILE ASKHUMAN_DEV_VERIFICATION_ID
  unset ASKHUMAN_DEV_TEST_WINDOW ASKHUMAN_DEV_DRIVER_PID ASKHUMAN_HOME
  "$INSTALL_DIR/AskHuman" "$@"
)

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
    printf '/usr/bin/codesign -i %q --force --timestamp=none --sign %q %q > %q 2>&1\n' \
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

# --- target/ cache log and environment guard ---

# Cache-affecting events (budget cleanups, sweeps, toolchain changes, a stray RUSTC_BOOTSTRAP) go
# into a small log so an unexpected cold rebuild can be traced afterwards. It sits at the top level
# of target/: profile cleanups keep it, a full `cargo clean` removes it. Keep the newest 200 lines.
# Logging must never fail an install.
CACHE_LOG="src-tauri/target/askhuman-cache.log"
CACHE_LOG_KEEP=200
_cache_log() {
  local lines
  mkdir -p "$(dirname "$CACHE_LOG")" 2>/dev/null || return 0
  printf '%s %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$*" >> "$CACHE_LOG" 2>/dev/null || return 0
  lines="$(wc -l < "$CACHE_LOG" 2>/dev/null | tr -d ' ' || true)"
  if [ "${lines:-0}" -gt "$CACHE_LOG_KEEP" ]; then
    if tail -n "$CACHE_LOG_KEEP" "$CACHE_LOG" > "$CACHE_LOG.tmp.$$" 2>/dev/null; then
      mv -f "$CACHE_LOG.tmp.$$" "$CACHE_LOG" 2>/dev/null || rm -f "$CACHE_LOG.tmp.$$" 2>/dev/null
    else
      rm -f "$CACHE_LOG.tmp.$$" 2>/dev/null
    fi
  fi
  return 0
}

# A new rustc or SDK invalidates every artifact, a common reason for a sudden full rebuild. Log the
# toolchain identity whenever it differs from the last recorded one.
_toolchain_id() {
  local rustc_v sdk=""
  rustc_v="$(rustc -V 2>/dev/null || true)"
  if [ "$(uname)" = "Darwin" ] && command -v xcrun >/dev/null 2>&1; then
    sdk="$(xcrun --sdk macosx --show-sdk-version 2>/dev/null || true)"
  fi
  printf '%s | macosx-sdk %s' "${rustc_v:-rustc unknown}" "${sdk:-n/a}"
}
_log_toolchain_if_changed() {
  local now last
  now="$(_toolchain_id)"
  # `|| true`: with pipefail a missing log file (first install) must not abort the install.
  last="$(sed -n 's/^[^ ]* toolchain: //p' "$CACHE_LOG" 2>/dev/null | tail -n 1 || true)"
  if [ "$now" != "$last" ]; then
    _cache_log "toolchain: $now"
  fi
  return 0
}

# RUSTC_BOOTSTRAP is tracked by the build scripts of proc-macro2, anyhow and thiserror
# (`rerun-if-env-changed`): one cargo run that sees it marks most of the dependency graph dirty,
# and the next run without it does so again. A stray export in the calling shell must not decide
# whether an install recompiles the whole Tauri stack.
if [ -n "${RUSTC_BOOTSTRAP+x}" ]; then
  echo "警告: 环境里设置了 RUSTC_BOOTSTRAP=${RUSTC_BOOTSTRAP}；它会让依赖树的大半被整体重编（取消设置后还会再来一次）。本次安装已忽略它。" >&2
  _cache_log "env: RUSTC_BOOTSTRAP=${RUSTC_BOOTSTRAP} was set; unset for this build"
  unset RUSTC_BOOTSTRAP
fi
_log_toolchain_if_changed

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

DEV_BUILD_FEATURES="custom-protocol"
export VITE_ASKHUMAN_DEV_WORKBENCH=0
if [ -n "$DEV_ROOT" ]; then
  export VITE_ASKHUMAN_DEV_WORKBENCH=1
  DEV_BUILD_FEATURES="custom-protocol,development-ui"
fi
node scripts/build-frontend-if-needed.mjs

echo "==> 编译 $BUILD_PROFILE 二进制（前端资源在此步骤被嵌入）"
# --features custom-protocol：生产构建必须启用，否则二进制以 dev 模式连 devUrl 导致白屏。
cargo build --profile "$BUILD_PROFILE" --manifest-path src-tauri/Cargo.toml --features "$DEV_BUILD_FEATURES"

BIN_PATH="src-tauri/target/$BUILD_PROFILE/AskHuman"
if [ ! -f "$BIN_PATH" ]; then
  echo "错误: 未找到编译产物 $BIN_PATH" >&2
  exit 1
fi

_file_sha256() {
  local path="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  else
    return 1
  fi
}

# Keep symlink entry points intact (for example ~/.local/bin -> an npm package).
# Stage beside the resolved target so the final rename stays on one filesystem.
resolve_install_target() {
  local target="$1" link hops=0 dir
  while [ -L "$target" ]; do
    if [ "$hops" -ge 40 ]; then
      echo "错误: 安装目标的符号链接层数过多: $1" >&2
      return 1
    fi
    link="$(readlink "$target")"
    case "$link" in
      /*) target="$link" ;;
      *) target="$(dirname "$target")/$link" ;;
    esac
    hops=$((hops + 1))
  done
  if [ -d "$target" ]; then
    echo "错误: 安装目标是目录: $target" >&2
    return 1
  fi
  dir="$(cd "$(dirname "$target")" && pwd -P)" || return 1
  printf '%s/%s\n' "$dir" "$(basename "$target")"
}

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
INSTALLED_BIN="$INSTALL_DIR/AskHuman"
INSTALL_STATE="$INSTALL_DIR/.askhuman-install-state"
SOURCE_HASH="$(_file_sha256 "$BIN_PATH" 2>/dev/null || true)"
SKIP_COPY=0
# Linux checks unchanged installs under the publisher lock so retained images are still pruned.
if [ "$(uname)" != "Linux" ] && [ -n "$SOURCE_HASH" ] && [ -f "$INSTALLED_BIN" ] && [ -f "$INSTALL_STATE" ]; then
  STATE_SOURCE="$(sed -n 's/^source=//p' "$INSTALL_STATE" | head -n1)"
  STATE_INSTALLED="$(sed -n 's/^installed=//p' "$INSTALL_STATE" | head -n1)"
  INSTALLED_HASH="$(_file_sha256 "$INSTALLED_BIN" 2>/dev/null || true)"
  if [ "$STATE_SOURCE" = "$SOURCE_HASH" ] && [ "$STATE_INSTALLED" = "$INSTALLED_HASH" ]; then
    SKIP_COPY=1
    echo "    已安装二进制内容未变化，跳过复制与签名"
  fi
fi

if [ "$SKIP_COPY" -eq 0 ]; then
  INSTALL_TARGET="$(resolve_install_target "$INSTALLED_BIN")"
  STAGED_BIN="$(mktemp "${INSTALL_TARGET}.new.XXXXXX")"
  PUBLISHER_DIR=""
  STATE_TMP=""
  cleanup_install_staging() {
    [ -n "${STAGED_BIN:-}" ] && rm -f "$STAGED_BIN" 2>/dev/null || true
    [ -n "${STATE_TMP:-}" ] && rm -f "$STATE_TMP" 2>/dev/null || true
    if [ -n "${PUBLISHER_DIR:-}" ]; then
      rm -f "$PUBLISHER_DIR/install-linux" 2>/dev/null || true
      rmdir "$PUBLISHER_DIR" 2>/dev/null || true
    fi
  }
  trap cleanup_install_staging EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  cp "$BIN_PATH" "$STAGED_BIN"
  chmod 0755 "$STAGED_BIN"

  if [ "$(uname)" = "Darwin" ]; then
    # Remove quarantine before signing the unpublished file.
    xattr -d com.apple.quarantine "$STAGED_BIN" 2>/dev/null || true
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
      codesign -i com.naituw.humaninloop --force --timestamp=none --sign "$IDENTITY" "$STAGED_BIN"
    else
      echo "==> 签名 (identity: $IDENTITY, identifier: com.naituw.humaninloop)"
      if ! codesign -i com.naituw.humaninloop --force --timestamp=none --sign "$IDENTITY" "$STAGED_BIN"; then
        echo "==> 后台进程无法使用正式签名私钥，改由用户 GUI 会话完成签名"
        sign_via_gui_launchd "$IDENTITY" "$STAGED_BIN" || {
          echo "错误: 正式签名失败，安装已中止" >&2
          exit 1
        }
      fi
    fi
    codesign --verify --strict "$STAGED_BIN"
  fi

  # Hash the unpublished artifact after signing, then let both platforms publish the
  # same staged file. Linux's publisher only performs exchange, retention and pruning.
  INSTALLED_HASH="$(_file_sha256 "$STAGED_BIN" 2>/dev/null || true)"

  if [ "$(uname)" = "Linux" ]; then
    if [ -z "$SOURCE_HASH" ] || [ -z "$INSTALLED_HASH" ]; then
      echo "错误: 无法计算安装二进制的 SHA-256，Linux 原子发布需要哈希状态" >&2
      exit 1
    fi
    # Build a host helper with the existing Rust toolchain; no extra runtime package.
    PUBLISHER_DIR="$(mktemp -d "${TMPDIR:-/tmp}/askhuman-publisher.XXXXXX")"
    rustc --edition=2021 "$SCRIPT_DIR/install-linux.rs" -o "$PUBLISHER_DIR/install-linux"
    "$PUBLISHER_DIR/install-linux" publish "$STAGED_BIN" "$INSTALL_TARGET" "$INSTALL_STATE" "$SOURCE_HASH" "$INSTALLED_HASH"
    rm -f "$STAGED_BIN" 2>/dev/null || true
    STAGED_BIN=""
    rm -f "$PUBLISHER_DIR/install-linux" 2>/dev/null || true
    rmdir "$PUBLISHER_DIR" 2>/dev/null || true
    PUBLISHER_DIR=""
  else
    # Never modify an inode still mapped by MCP/daemon/GUI processes. Publish only
    # after signing and verification succeed, leaving the old install intact on failure.
    mv -f "$STAGED_BIN" "$INSTALL_TARGET"
    STAGED_BIN=""
    if [ -n "$SOURCE_HASH" ] && [ -n "$INSTALLED_HASH" ]; then
      STATE_TMP="$INSTALL_STATE.tmp.$$"
      {
        printf 'source=%s\n' "$SOURCE_HASH"
        printf 'installed=%s\n' "$INSTALLED_HASH"
      } > "$STATE_TMP"
      mv "$STATE_TMP" "$INSTALL_STATE"
      STATE_TMP=""
    fi
  fi
  trap - EXIT INT TERM
fi

if [ -n "$DEV_ROOT" ]; then
  node "$SCRIPT_DIR/dev-build-record.mjs" record "$DEV_ROOT" "$INSTALL_DIR/AskHuman" "$DEV_SOURCE_SNAPSHOT"
  echo "    Dev 验收: AskHuman dev ask --worktree $DEV_ROOT …"

fi

# --- target/ 缓存清理 ---

# cargo-sweep 回收长期未使用的依赖 hash；profile 预算（下方）不依赖它存在。
if command -v cargo-sweep >/dev/null 2>&1; then
  echo "==> 清理 7 天未使用的 target 依赖残留"
  if SWEEP_OUT="$( cd src-tauri && cargo sweep --time 7 2>&1 )"; then
    printf '%s\n' "$SWEEP_OUT"
    # cargo-sweep reports `Cleaned nothing from "<target>"` or `Cleaned <amount> from "<target>"`.
    SWEEP_CLEANED="$(printf '%s\n' "$SWEEP_OUT" | sed -n 's/^\[INFO\] Cleaned \(.*\) from .*/\1/p' | head -n 1)"
    if [ -n "$SWEEP_CLEANED" ] && [ "$SWEEP_CLEANED" != "nothing" ]; then
      _cache_log "sweep: cargo sweep --time 7 cleaned $SWEEP_CLEANED"
    fi
  else
    printf '%s\n' "$SWEEP_OUT" >&2
    echo "警告: cargo-sweep 清理失败，继续执行 profile 预算检查" >&2
    _cache_log "sweep: cargo sweep --time 7 failed"
  fi
fi

# A linked worktree (`git worktree add`) is a parallel checkout with its own target/, usually
# short-lived, so it gets tighter budgets than the main checkout. Anything that is not a git
# checkout (a source tarball) counts as the main checkout.
_is_linked_worktree() {
  local git_dir common_dir
  git_dir="$(git rev-parse --git-dir 2>/dev/null)" || return 1
  common_dir="$(git rev-parse --git-common-dir 2>/dev/null)" || return 1
  git_dir="$(cd "$git_dir" 2>/dev/null && pwd -P)" || return 1
  common_dir="$(cd "$common_dir" 2>/dev/null && pwd -P)" || return 1
  [ "$git_dir" != "$common_dir" ]
}

# Budgets are per profile, in MiB of logical size (`du`). An APFS clone made by
# scripts/seed-worktree-target.sh counts in full although its blocks are shared, so keep these above
# the steady-state size of a seeded profile (about 2.4 GiB local-install, 4.4 GiB debug).
if _is_linked_worktree; then
  CACHE_SCOPE="linked worktree"
  BUDGET_LOCAL_INSTALL_MB=3072
  BUDGET_DEV_MB=5120
  BUDGET_FULL_DEBUG_MB=5120
  BUDGET_RELEASE_MB=3072
else
  CACHE_SCOPE="main checkout"
  BUDGET_LOCAL_INSTALL_MB=4096
  BUDGET_DEV_MB=6144
  BUDGET_FULL_DEBUG_MB=6144
  BUDGET_RELEASE_MB=4096
fi
# The Swift/Clang module cache is shared by all profiles (see build.rs) and holds one ~240 MiB set
# of modules per compiler/SDK combination. Old sets only accumulate after Xcode updates.
SWIFT_CACHE_BUDGET_MB=768

# Cargo 自己执行 package/profile 清理并持有 target lock，避免裸 rm 与并发构建竞争。
# 先只移除本项目产物、保留三方依赖；仍超预算才清整个 profile。
_profile_size_mb() {
  local dir="$1"
  [ -d "$dir" ] || { echo 0; return; }
  du -sk "$dir" 2>/dev/null | awk '{print int(($1 + 1023) / 1024)}'
}

_enforce_profile_budget() {
  local profile="$1" dir="$2" limit_mb="$3" size_mb after_mb action="package clean"
  size_mb="$(_profile_size_mb "$dir")"
  [ "$size_mb" -le "$limit_mb" ] && return 0

  echo "==> $profile 缓存 ${size_mb}MB 超过预算 ${limit_mb}MB（${CACHE_SCOPE}）；清理本项目产物"
  if ! cargo clean --manifest-path src-tauri/Cargo.toml -p humaninloop --profile "$profile"; then
    echo "警告: 无法清理 $profile 本项目缓存" >&2
    _cache_log "budget: $profile ${size_mb}MB > ${limit_mb}MB (${CACHE_SCOPE}); package clean failed"
    return 0
  fi
  after_mb="$(_profile_size_mb "$dir")"
  if [ "$after_mb" -gt "$limit_mb" ]; then
    echo "==> $profile 三方依赖缓存仍有 ${after_mb}MB；执行 profile 级清理"
    if ! cargo clean --manifest-path src-tauri/Cargo.toml --profile "$profile"; then
      echo "警告: 无法完成 $profile profile 级清理" >&2
      _cache_log "budget: $profile ${size_mb}MB > ${limit_mb}MB (${CACHE_SCOPE}); profile clean failed after package clean (${after_mb}MB left)"
      return 0
    fi
    after_mb="$(_profile_size_mb "$dir")"
    action="profile clean"
  fi
  echo "   $profile 缓存: ${size_mb}MB → ${after_mb}MB"
  _cache_log "budget: $profile ${size_mb}MB > ${limit_mb}MB (${CACHE_SCOPE}); $action -> ${after_mb}MB"
}

# Before the module cache moved to target/swift-module-cache, every build-script output directory
# kept its own copy (about 240 MiB each). Nothing reads those any more; drop them.
_remove_legacy_swift_module_caches() {
  local dir removed=0
  for dir in src-tauri/target/*/build/humaninloop-*/out/swift-module-cache \
             src-tauri/target/*/*/build/humaninloop-*/out/swift-module-cache; do
    [ -d "$dir" ] || continue
    rm -rf "$dir" && removed=$((removed + 1))
  done
  if [ "$removed" -gt 0 ]; then
    echo "==> 已移除 $removed 份旧位置的 Swift 模块缓存（现统一在 src-tauri/target/swift-module-cache）"
    _cache_log "migrate: removed $removed per-build Swift module cache(s); shared cache is target/swift-module-cache"
  fi
  return 0
}

_enforce_swift_cache_budget() {
  local dir="src-tauri/target/swift-module-cache" size_mb
  size_mb="$(_profile_size_mb "$dir")"
  [ "$size_mb" -le "$SWIFT_CACHE_BUDGET_MB" ] && return 0

  echo "==> Swift 模块缓存 ${size_mb}MB 超过 ${SWIFT_CACHE_BUDGET_MB}MB（多为旧 Xcode/SDK 留下的条目）；整体删除，下次构建会重建（约 25 秒）"
  if rm -rf "$dir"; then
    _cache_log "budget: swift-module-cache ${size_mb}MB > ${SWIFT_CACHE_BUDGET_MB}MB; removed, rebuilt by the next build"
  fi
  return 0
}

_remove_legacy_swift_module_caches
_enforce_profile_budget "local-install" "src-tauri/target/local-install" "$BUDGET_LOCAL_INSTALL_MB"
_enforce_profile_budget "dev" "src-tauri/target/debug" "$BUDGET_DEV_MB"
_enforce_profile_budget "full-debug" "src-tauri/target/full-debug" "$BUDGET_FULL_DEBUG_MB"
_enforce_profile_budget "release" "src-tauri/target/release" "$BUDGET_RELEASE_MB"
_enforce_swift_cache_budget

echo "==> 完成：$INSTALL_DIR/AskHuman"
if [ -z "$DEV_ROOT" ] && ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo "提示: $INSTALL_DIR 不在 PATH 中，请将其加入 PATH。"
fi

# 多份安装 / PATH 遮蔽检查：内容不同的 AskHuman 并存时会轮流把对方的 daemon 顶掉
# （docs/specs/daemon-graceful-drain.md §6）。只在发现冲突时输出；检查失败不影响安装结果。
# Dev Instance 安装有独立 daemon，检查会自行跳过。
run_installed doctor install --warn-only || true
