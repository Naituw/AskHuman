# 开发计划：版本自更新机制（self-update）

> 关联需求：`docs/specs/self-update.md`
> 计划描述方案与技术 / 规则细节，具体代码以实现为准。参考实现：`/Users/wutian/Developer/humanInLoop-rust`（`updater.rs` / `cliff.toml` / `release.yml` / `useVersionCheck.ts`）。

## 0. 方案总览

```
后台（daemon）：启动 + 每 ~24h
  └─ Updater::check_latest()  → 远端最新正式版
       ├─ latest > local(CARGO_PKG_VERSION) 且 未被忽略 → 标记「有更新」，落 update.json
       └─ 向所有打开的 GUI Helper 广播 UpdateState（有更新 / 待生效）

用户触发更新（弹窗浮层 或 设置「更新」）：
  └─ Updater::apply()
       ├─ DirectUpdater：下载 GitHub 资产 → 解压 → 校验签名 → 备份 .bak → 原子替换 current_exe
       └─ NpmUpdater：spawn `npm i -g askhuman@latest`（失败→回显命令）
  ⇒ 盘上二进制变 → 既有 daemon drain 在「所有在途弹窗答完后」自动换新（不 restart、不打断）

外部更新（终端 npm / 另处安装）：
  └─ daemon 指纹监听感知变化 → 同样广播「待生效」
```

核心：**自更新只负责把新二进制落盘**；「答完再换、不打断」复用 graceful-drain，不新增进程重启。

---

## 1. 新增模块 `src-tauri/src/update/`

- `mod.rs`
  - `UpdateInfo { available, current_version, latest_version, release_notes, ... }`、`InstallKind { Npm, Direct, Unsupported }`、`Updater` trait（`check_latest()`、`apply(progress_cb)`）。
  - `detect_install_kind()`：读 `std::env::current_exe()`，路径含 `node_modules/@humaninloop/` 或 `node_modules/askhuman/` → `Npm`；非 Unix → `Unsupported`（一期仅提示）；否则 `Direct`。
  - `compare_versions(a,b)`：数字段逐段比较（借鉴参考实现）；`current_version()` = `env!("CARGO_PKG_VERSION")`。
  - `select_updater()`：按 `detect_install_kind()` 返回具体实现。
- `direct.rs`（**DirectUpdater**，移植参考 `updater.rs` 并去掉 restart）
  - `check_latest()`：GET `https://api.github.com/repos/Naituw/AskHuman/releases/latest`（`User-Agent` + `Accept: application/vnd.github.v3+json` + 超时）→ 取 `tag_name`（去 `v`）、`body`、`assets`。
  - 平台资产匹配：按 **rust 目标三元组** 匹配资产名 `AskHuman-<triple>-v<ver>.tar.gz`（如 `aarch64-apple-darwin` / `x86_64-apple-darwin` / `x86_64-unknown-linux-gnu`）。
  - `apply()`：下载到 `temp/askhuman_update/` →（进度回调，复用现有事件机制）→ 解压（shell out `tar -xzf` / Linux 同；mac 同）→ 在解压目录找可执行文件 `AskHuman` → **校验**（见 §6 安全）→ 备份当前为 `<exe>.<ver>.bak` → 原子替换 `current_exe`（同目录临时文件 + `set_mode(0o755)` + `rename`）。**不调用 restart**。
- `npm.rs`（**NpmUpdater**）
  - `check_latest()`：`npm view askhuman version --registry https://registry.npmjs.org`（或 registry HTTP JSON）取最新；超时 + 错误静默。
  - `apply()`：spawn `npm i -g askhuman@latest`；macOS/Linux 由当前 `.../<prefix>/lib/node_modules/.../AskHuman` 反推 `<prefix>/bin/npm`，并将 `<prefix>/bin` 前置到子进程 `PATH`，使 GUI/launchd 环境下仍能找到同 prefix 的 npm + node；候选缺失才回退继承 PATH。成功即返回（盘上 node_modules 二进制被 npm 替换 → daemon 指纹变 → drain 换新）。失败 / 找不到 npm → 返回带「手动命令」的错误，前端回显命令。
- `notes.rs`（更新日志展示）
  - `latest_notes()`：单版本，取 `releases/latest` 的 `body`。
  - `aggregated_notes(from,to)`：**懒加载**，GET `releases`（列表，单请求）→ 过滤 tag 在 `(from, to]` → 按版本倒序拼接 body + 各自 compare 链接；结果缓存（内存 / `update.json`）。npm 安装也走 GitHub 取 body（按 tag），缺失则占位。

依赖：`reqwest`（已具）；解压沿用参考做法 **shell out** `tar`/`unzip`（mac/Linux 自带，免新增 crate）。

## 2. 状态存储 `~/.askhuman/update.json`

字段（serde，缺字段走默认）：`latest_version`、`checked_at`、`release_notes`（最新版摘要，可空）、`dismissed_versions: [..]`、`pending: bool`（盘上已是新版、待 drain 生效）。原子写。`paths.rs` 增 `update_state_file()`。

## 3. Daemon 集成 `src-tauri/src/daemon/`

- **后台检查任务**：daemon 启动后 spawn 周期任务（启动即查一次，之后每 ~24h）：`select_updater().check_latest()` → 写 `update.json` → 若 `available` 且未忽略 → 广播。失败静默、下个周期重试。
- **广播给弹窗**：复用既有「给活动 GUI Helper 下发」通道（参照 `ServerMsg::ConfigChanged` 的实现）：新增 `ServerMsg::UpdateState { available, latest_version, pending }`。GUI Helper 收到后向 webview emit 事件（见 §5）。
- **外部更新感知（D6）**：daemon 已在 Hello/指纹路径判定 stale；当检测到「盘上指纹 ≠ 启动指纹」即把 `update.json.pending=true` 并广播 `UpdateState{ pending:true }`（无论变化来自应用内更新还是外部 npm）。注意与 graceful-drain 既有逻辑协同：广播只增「告知前端」，不改 drain 的退出判定。
- **GUI Hello 回包**：GUI Helper 首次 `GuiHello` 时，daemon 在握手响应里带上当前 `UpdateState`，使弹窗一打开即知状态（无需等下一次广播）。

> IPC：`PROTOCOL_VERSION` 保持 1；新增 `ServerMsg::UpdateState{..}`（旧端忽略未知变体的兼容性同 graceful-drain 处理）。CLI 提问/提交路径不涉及，无新增 ClientMsg（更新触发走 commands，见 §4）。

## 4. 前后端命令 `src-tauri/src/commands.rs` ↔ `src/lib/ipc.ts`

新增 `#[tauri::command]`（弹窗与设置进程都可调）：
- `get_app_version() -> String`：`CARGO_PKG_VERSION`。
- `update_check(manual: bool) -> UpdateInfo`：调 `select_updater().check_latest()`；`manual=true` 时清空忽略集合（D9）。
- `update_get_notes(aggregate: bool) -> String`：`notes::latest_notes()` 或 `aggregated_notes()`（懒加载）。
- `update_apply()`：调 `select_updater().apply()`，发进度事件（`update_download_progress` / `update_install_finished` / `update_manual_required`，借鉴参考）。
- `update_dismiss(version: String)`：写 `dismissed_versions`。
- `open_release_page(url)`：系统打开（复用现有 `open_path` / 外开能力）。
- `restart_settings()`：spawn 新 `current_exe --settings` 后退出当前设置窗（仅设置进程用，D13）。

弹窗读取 daemon 广播的状态：经 GUI Helper → webview 事件；设置进程因不经 daemon，直接调 `update_check`（自查）。

## 5. 弹窗前端 `src/views/PopupView.vue`

- 顶部 `.nav-actions` 增「更新」按钮（下载/箭头图标）；`hasUpdate` 时显示圆点。
- 点击 → 小浮层（popover）：`latest_version` + 日志摘要（`update_get_notes`）+ 「更新」按钮 + 文案「更新将在你回答完成后生效」。点「更新」→ `update_apply()`；npm 失败回显命令。
- 「待生效」横幅：监听 GUI Helper 转发的 `update-state` 事件，`pending=true` 时在弹窗顶部显示「新版本将在所有弹窗回复完成后生效，请尽快回复」。
- 状态来源：`popup_init` 返回里带初始 UpdateState（来自 GuiHello 回包）+ 运行期事件更新。

## 6. 安全（D19）

- 下载完成后、替换前（macOS）：`codesign --verify --strict` + `codesign -dvv` 解析 `TeamIdentifier`，要求 `== DMJXDB9H6Q`；不符 / 校验失败 → 放弃替换、回退「请手动下载」。
- **不**主动 `xattr -d com.apple.quarantine`（自有 HTTP 下载通常不带；仅在确实存在该属性时防御性清理，可选）。
- 替换：同目录临时文件 → `chmod 0755` → `rename` 原子替换；先备份当前为 `<exe>.<ver>.bak`（同名冲突追加序号，参照参考实现）。
- Linux：无签名校验，仅校验解压出的可执行文件存在 + 可执行 + 大小合理。

## 7. 更新日志生成（发布流程）

- 新增仓库根 `cliff.toml`（移植参考并按 D15/D16/D20 调整）：
  - `commit_parsers`：先匹配 **breaking**（`^.*!:` 或含 `BREAKING CHANGE`）→ `⚠ Breaking Changes`；再 `^feat`→`✨ Features`、`^fix`→`🐞 Fixes`、`^perf`→`💎 Performance`、`^security`→`🔒 Security`、`^revert`→`⏪ Revert`；`^chore|^docs|^style|^refactor|^test|^ci|^build`→`skip = true`；catch-all 也 `skip`（**选择性**，与参考「归其他」不同）。
  - **单条覆盖（trailer）**：解析 commit footer——含 `Release-Note: skip` 的提交在 parser 里 `skip = true`；含 `Release-Note: <文案>` 的，body 模板优先用该 footer 值作展示文本（git-cliff 模板读 `commit.footers`，token=`Release-Note`），否则回退到去前缀后的 subject。
  - `body` 模板：分组顺序 Breaking → Features → Fixes → Performance → Security → Revert；条目去 `type(scope):` 前缀但**保留 scope 作 `**scope**:` 粗体前缀**、句首大写；末尾 `Full Changelog` 链接。
  - `tag_pattern = "v[0-9].*"`。
- 改 `.github/workflows/release.yml` 的「Create GitHub Release」：
  - 新增 git-cliff 安装步骤（`taiki-e/install-action@git-cliff`）。
  - 生成 body：**若 `docs/release-notes/v<版本>.md` 存在 → 用其内容**（D17）；否则 `git-cliff <prev_tag>..<cur_tag>` 生成 + 追加 Full Changelog。`prev_tag` 取 GitHub `releases/latest`，回退 git tag。
  - `generate_release_notes: false` + `body_path: changelog.md`（替换现有 `generate_release_notes: true`）。
  - 资产命名维持现有 `AskHuman-<triple>-v<版本>.tar.gz/.zip`（与 §1 匹配逻辑一致，无需改 publish 段）。
- 新增目录 `docs/release-notes/`（放说明 + 可选的手写/AI 覆盖文件）。

## 8. i18n `src-tauri/src/i18n.rs` + 前端

新增键（zh/en）：更新按钮 / 浮层标题 / 「答完后生效」说明 / 待生效横幅 / 「检查更新 / 更新 / 已是最新 / 更新失败 / 手动命令」/ 关于区标题 / 当前·最新版本 / 「查看全部发布」/「重启设置页面」/ npm 命令提示等。

## 9. 设置前端 `src/views/SettingsView.vue`

- 「通用」Tab 增「关于」区：当前版本（`get_app_version`）、最新版本与「检查更新」按钮（`update_check(manual)`）、「更新」按钮（`update_apply`）、更新日志渲染（`update_get_notes(aggregate=true)` + markdown，复用 `lib/markdown.ts`）、「查看全部发布」链接。
- 更新完成后：显示「重启设置页面」按钮 → `restart_settings()`；无在途时提示「已更新，下次提问生效」。
- 忽略/手动检查重置逻辑（D9）。

## 10. install 脚本 / 文档 / 提交规范

- `scripts/install.sh`：保持现状（本地源码安装签名沿用本地证书）；自更新走预编译 Release，互不冲突。
- `AGENTS.md`：把「Commit messages」扩写为完整 Conventional Commits 规范（type 归类、scope、subject 写法、breaking、`Release-Note:`/`Release-Note: skip` trailer），并显式说明「会进入用户可见 release notes，须认真撰写」（D20，已落地）。
- `docs/overview.md`：新增「版本自更新」小节（安装方式分流、drain 换新、更新日志 git-cliff + trailer + 覆盖文件、关于区/弹窗入口）。
- `docs/PROGRESS.md`：完成后清理标记。

## 11. 测试

- 单测：
  - `update`：`compare_versions` 边界；`detect_install_kind` 对 npm / 直装 / 非 unix 路径判定；资产名→平台匹配；`aggregated_notes` 区间过滤；`update.json` serde 往返与忽略集合。
  - `ipc`：`ServerMsg::UpdateState` 序列化往返 + 旧端缺字段降级。
  - 发布：`cliff.toml` 用样例提交本地 `git-cliff` 跑一次，核对分组/跳过/格式（手测）。
- 手测（install 后）：
  1. 临时把本地版本号调低（或对一个更高版本的测试 Release）→ 提问 → 弹窗出现更新圆点与浮层；点更新 → 当前作答不受影响、可答完；答完后下次提问为新版。
  2. 两弹窗并存触发更新 → 两窗均出现「待生效」提示条；全答完后换新。
  3. 终端 `npm i -g askhuman@latest`（npm 装环境）→ 已开弹窗出现「待生效」（D6）。
  4. 设置「检查更新 / 更新 / 重启设置页面」全流程；忽略某版本不再弹、手动检查重置。
  5. macOS 校验失败用例（改坏下载）→ 不替换、回退提示。

## 12. 涉及文件清单

- 新增：`src-tauri/src/update/{mod,direct,npm,notes}.rs`；`cliff.toml`；`docs/release-notes/`（含说明）。
- 改：`src-tauri/src/main.rs`（声明 update 模块）、`paths.rs`（`update_state_file`）、`daemon/`（后台检查 + 广播 + 外部感知 + GuiHello 带状态）、`ipc/mod.rs`（`ServerMsg::UpdateState`）、`commands.rs`、`i18n.rs`、`src/views/PopupView.vue`、`src/views/SettingsView.vue`、`src/lib/{ipc.ts,types.ts}`、`.github/workflows/release.yml`、`AGENTS.md`（提交规范）、`docs/overview.md`、`docs/PROGRESS.md`。

## 13. 任务顺序

1. `update/` 模块（detect/compare/Updater + Direct + Npm + notes）+ 单测。
2. `paths.rs` + `update.json` 状态；`commands.rs` 命令；i18n 键。
3. `ipc` 增量 + daemon 后台检查 / 广播 / 外部感知 / GuiHello 带状态。
4. 前端：弹窗入口 + 浮层 + 待生效横幅；设置「关于」区 + 重启设置。
5. `cliff.toml` + `release.yml` 改造 + `docs/release-notes/`。
6. `cargo test` + 文档；待用户同意后 `install.sh` 实测手测脚本。

## 14. 风险与注意

- **首次升级例外**：本特性发布后第一次升级仍由旧代码 daemon 主导（与 graceful-drain D6 同），其后享受完整体验。
- **npm 全局目录权限**：`npm i -g` 可能因权限失败 → 必须可靠回退到「显示命令」。
- **GitHub API 速率**：未鉴权 60 次/小时/IP；周期 24h + 懒加载聚合，远低于上限；403 时静默 + 提示手动。
- **设置进程不经 daemon**：其更新触发是自查自做；与 daemon 后台状态可能短暂不一致，以「盘上二进制」为最终事实（drain 收敛）。
- **校验与签名连续性**：替换必须保持 `com.naituw.humaninloop` 标识 + Developer ID（Release 已签），否则钥匙串信任会断、密钥读取弹框。
- **Windows 一期不替换**：仅提示 + 下载页，避免「覆盖运行中 exe」的复杂度（记为二期：下载到旁路 + 退出时替换 / 借鉴参考的延迟 .bat）。
