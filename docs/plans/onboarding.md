# 安装后设置向导（`AskHuman setup`）—— 开发计划

> spec: `docs/specs/onboarding.md`（已通过审阅；下文「D 编号」「验收 N」均引自该 spec）

## 概述

新增统一的「设置向导」，让新用户装完二进制后用约 2 分钟完成必要配置，老用户可随时重走同一流程补缺：

- **GUI**：GUI Host 承载的全局唯一窗口（label `setup`，3 页：接入 Agent → 远程回答 → 完成）；
- **文字版**：`AskHuman setup [--text]`，无图形会话时自动使用；
- **后端**：新模块 `src-tauri/src/onboarding/`（三域「已决定」派生、Agent 探测 / 预览 / 应用、环境体检、文字流程、可选验证）；
- **入口**：`setup` 子命令、终端无参欢迎语、设置页横幅与「通用」卡片、托盘「设置向导…」、弹窗页脚提示条（取代旧 IM 小条）；
- **配套**：GUI Host 登录项写入改为受「已做过选择」门控，macOS `restart_host` 补守卫；可选验证需要 daemon 两处增量（均为可选字段，不升协议号）。

分四期，每期可独立验证、独立提交（Conventional Commits，`feat(setup): …`；仅在用户要求时提交）：

| 期 | 内容 | 可独立验证的产物 |
|---|---|---|
| P1 | 状态与后端基础：`ui-state.json` 新字段、三域派生、登录项门控 + `restart_host` 守卫、Agent 表 / 探测 / 预览 / 应用、环境体检 | Rust 单测（无界面变化；登录项门控已生效） |
| P2 | CLI：`setup` 子命令、文字流程、终端欢迎语、帮助 | pty 脚本在临时 HOME 里跑完整文字流程 |
| P3 | GUI：向导窗口三页、Tauri 命令层、全部提示入口、弹窗提示条取代旧 IM 小条 | vitest + 安装后手动 / 截图 |
| P4 | 可选验证：本机自测、渠道往返测试、在 Agent 里试一次（含 daemon 增量） | Dev 实例 `dev_ask`；渠道往返需用户配合 |

## 0. 总体约定

### 0.1 技术决策

| # | 决策 | 理由 |
|---|---|---|
| T1 | 统一用词 `setup`：子命令 `setup`、`WindowKind::Setup`（serde `"setup"`）、窗口 label `setup`、`?view=setup`、`Group::Setup`、Tauri 命令前缀 `setup_`；Rust 模块 `onboarding/`，CLI 文件 `cli/setup_cmd.rs` | 与 `newtask` / `forktask` 的命名与路由先例一致；`setup_` 前缀避开 `app/invoke.rs` 里 `_test` / `_detect_` 的渠道分组匹配（测试类命令不叫 `*_test`） |
| T2 | 目标页复用 `HostMsg::OpenWindow.project` 槽位，取值 `agents` / `remote` / `done`，其它值按 `None`；不新增线上字段 | 与设置窗口「槽位 = 初始 tab」的做法一致 |
| T3 | 三域状态 = 纯函数 `derive(&Evidence)` + 取证函数 `gather()`；只持久化 `skipped` 与 `background_decided` | spec §4.1：状态尽量派生，只存「决定」 |
| T4 | **配置写入归属**：GUI 内所有 `config.json` 写入（协作风格、渠道、`menuBarIcon`、`daemonLifecycle`）走前端同一份 `core.config` 的 `persist()`；`setup_*` 后端命令只写 `ui-state.json`、Agent 集成文件、登录项，绝不写 `config.json`；文字版直接 `AppConfig::load()` / `save()` | 避免后端写盘后被前端旧副本整份覆盖 |
| T5 | 变化通知用全局事件 `setup-status-changed`（无 payload）。发出点：`refresh_integration_updates`（Agent 集成变化的既有汇合点）、`setup_skip` / `setup_finish` / `setup_login_item_enable` / `popup_setup_hint_dismiss`、`apply_config` 的模式切换分支；托盘 `setup_pending` 在同一处重算 | 复用既有汇合点，不另建轮询 |
| T6 | Agent 表（id / 标题 / 支持模式 / 推荐模式 / 平台）的唯一来源是 Rust `onboarding/agents.rs`；向导行数据全部由 `setup_agents_scan` 下发。一个 Rust 单测用 `include_str!` 读 `src/views/settings/useIntegration.ts` 的 `AGENTS` 行逐项对拍，防止与设置页漂移 | 文字版没有前端可读；保持单一来源 |
| T7 | 向导发起的测试提问用 `native_request_id` 的保留前缀 `setup-probe:<target>:<uuid>` 标记（`target` = `popup` 或渠道 id `telegram` / `dingding` / `feishu` / `slack`），不新增 `TaskRequest` 字段；daemon 另在 `AskRequest` 上带 `setup_probe: bool`，供弹窗隐藏提示条 | 与 failure card 用 id 前缀识别（`failure::is_card_id`）同一做法；避免改约 8 处 `TaskRequest` 字面量 |
| T8 | Dev 实例：向导 Agent 区只读（沿用 `everydayIntegrationsReadOnly`）；登录项操作沿用函数内 `is_dev_instance_context()` 的 no-op；`ui-state.json` 随 `ASKHUMAN_HOME`；后台域忽略全局登录项证据（`instance_has_login_item()` 在 Dev 恒为 false）；环境体检在 Dev 返回空报告；Agent 域证据不做 Dev 特判 | spec D23 |

### 0.2 协议号判定（`docs/overview.md`「IPC 协议号」）

| 改动 | 性质 | 升 `PROTOCOL_VERSION`？ |
|---|---|---|
| 探测请求：`native_request_id` 新保留前缀 | 线上无结构变化 | 否。旧 daemon 把它当普通原生请求：探测退化为一次普通提问（弹窗 + 全部渠道、进历史），仅影响可选验证的纯度 |
| `AskRequest.setup_probe: bool`（`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`） | 新增可选字段 | 否 |
| `PendingRequestInfo.agent_kind: Option<String>`、`agent_ask: bool`（均 `#[serde(default, skip_serializing_if …)]`） | 新增可选字段；旧 Host 忽略 | 否 |
| `WindowKind::Setup`（`HostMsg::OpenWindow.kind`） | GUI Host 私有 IPC，不在 daemon 协议内；旧 Host 对未知 kind 静默丢弃（同 NewTask / ForkTask 先例） | 否 |
| 其余（CLI 子命令、Tauri 命令、`ui-state.json` 字段） | 非线上 | 否 |

无新增 `ClientMsg` / `ServerMsg` 变体。实现时同步修改 `ipc/mod.rs` 里构造 `PendingRequestInfo` 的往返测试（约 1389 行）与 `ios/gateway/testkit.rs` 里 `AskRequest` 的字面量（约 521 行）。

### 0.3 对既有行为的影响（评审需知情）

1. **登录项门控（D13）**：全新用户启动 GUI Host 不再静默写登录项。附带变化：曾经有登录项、后来手动删掉、且从未做过选择的用户，不再被下次启动静默补回（只有设置里显式切换 `menuBarIcon` 或向导「完成」才会写）。已有登录项的用户行为不变。
2. **`restart_host` 守卫**：macOS 上仅当登录项已安装才走「`exit(0)` 交 launchd」路径；否则走已有的 spawn 路径。副作用：Dev 实例（从不写登录项）的宿主换新现在也走 spawn 路径，不再指望不存在的 launchd 项。
3. **弹窗页脚旧 IM 小条**：命令 `popup_im_tip_visible` / `popup_im_tip_dismiss`、前端 `imTip*`、样式 `.im-tip*`、文案 `popup.imTip.*` 全部移除，无兼容层；条件逻辑不变（无渠道启用且没关闭过），旧 `im_tip_dismissed = true` 读取时等同「远程回答已跳过」。
4. **保留字 `setup`**：`AskHuman setup` 不再被当成提问文本（与 `todo` / `doctor` 同级）；问题恰好是 `setup` 时用 `-q setup`。`agents/transcript_full.rs` 的 `NON_ASK_SUBCOMMANDS` 同步加入 `"setup"`。
5. 既有遗留（**不在本计划内修复**，向用户报告）：`is_hosted_label` 漏了 `"fork-task"`；`detect_integration_updates()` 漏了 Antigravity。

## 1. P1：状态模型、登录项门控与 Agent 后端

### 1.1 `uistate.rs`

- `UiState` 新增 `onboarding: OnboardingDecisions`（`#[serde(default)]`，camelCase）：

```rust
pub struct OnboardingDecisions { pub skipped: Vec<String>, pub background_decided: bool }
```

- 读取规范化：`skipped` 只保留 `"agents"` / `"remote"`，去重；字段缺失 = 空集 / `false`。
- 旧字段 `im_tip_dismissed` 保留：只读兼容，`update` 回写时原值保留，新代码不再写它。
- 新增 `pub fn update(f: impl FnOnce(&mut UiState))`：进程内静态 `Mutex` 串行化 load、mutate、save（保存沿用现有 best-effort）。跨进程（CLI / GUI Host / 弹窗进程）不加文件锁：写入全部由用户手动触发、低频、单字段，与现有 `im_tip_dismissed` 同一口径。

### 1.2 `onboarding/state.rs`（三域判定）

```rust
pub struct Evidence {
    pub skipped: Vec<String>, pub legacy_im_tip_dismissed: bool,
    pub any_agent_integrated: bool, pub history_has_agent_origin: bool,
    pub any_channel_enabled: bool,
    pub login_item_present: bool, pub background_decided: bool,
}
pub struct Decided { pub agents: bool, pub remote: bool, pub background: bool }
pub fn derive(e: &Evidence) -> Decided;          // 纯函数
pub fn gather() -> Evidence;                      // 读真实机器
pub fn decided() -> Decided;                      // derive(&gather())
pub fn agents_decided() -> bool;                  // 短路：skipped → 集成 → 历史
pub fn remote_decided() -> bool;
pub fn mark_skipped(domain: &str);                // 只接受 "agents" | "remote"；幂等
pub fn mark_background_decided();
pub fn login_item_gate_open() -> bool;            // instance_has_login_item() || background_decided
```

判定（spec §4.1，逐字实现）：

- `agents` = `any_agent_integrated` ∨ `skipped ∋ "agents"` ∨ `history_has_agent_origin`；
- `remote` = `any_channel_enabled` ∨ `skipped ∋ "remote"` ∨ `legacy_im_tip_dismissed`；
- `background` = `login_item_present` ∨ `background_decided`。

取证细则：

- `any_agent_integrated`：遍历 `agents::available()`，任一家 `agents::current_mode(def)` 为 Cli 或 Mcp 即真；`Err`（Antigravity 严格读取失败）视为未集成，绝不阻塞。
- `history_has_agent_origin`：新增 `history::has_agent_origin()` = `load(None, true)` 中存在 `agent_kind` 非空白的记录（终端里手动 `AskHuman "hi"` 没有 Agent 来源，不算）。
- `any_channel_enabled`：`AppConfig::load_without_secrets().channels` 的 `telegram` / `dingding` / `feishu` / `slack` 任一 `enabled`（**不含** popup 与 iOS，与旧 IM 小条条件完全一致）。
- `login_item_present` = `login_item::instance_has_login_item()`。

### 1.3 登录项门控与 `restart_host` 守卫

- `integrations/login_item.rs`：新增 `pub fn instance_has_login_item() -> bool { !is_dev_instance_context() && is_installed() }`（放在「共用」区，`is_installed` 已有 unix / windows 两个实现）。
- `app/gui_host.rs` `setup()`（约 367 行）：`if mode == Always { ensure_installed() }` 改为 `if mode == Always && onboarding::state::login_item_gate_open() { … }`。
- `apply_config`（约 2316 行起）：`new_mode != old_mode` 分支处理完（`Off` / `Active` / `Always` 三个子分支各自逻辑不变，`Always` 分支的 `ensure_installed()` 不受门控）后调用 `mark_background_decided()` 并 emit `setup-status-changed`。
- `cli/config_cmd.rs`：`set` / `unset` 成功写入 key `general.menuBarIcon` 后调用 `mark_background_decided()`（CLI 里的显式选择同样算做过选择）。
- `restart_host`（约 2404 行）macOS 分支：抽纯函数 `fn restart_via_launchd(mode: MenuBarIconMode, has_login_item: bool) -> bool { mode == Always && has_login_item }`；为真才 `app.exit(0)`，否则落入既有 spawn 路径。
- 新增命令 `setup_login_item_enable`（P3 §3.1）：`ensure_installed()` + `mark_background_decided()` + emit，供设置页「开机自启尚未启用」的「启用」按钮。

### 1.4 Agent 表与探测

`onboarding/agents.rs`（顺序即展示顺序；`available()` 在 `!cfg!(unix)` 时排除 Antigravity，与 `collaboration_style_apply_integrations` 的平台口径一致）：

| id | 标题 | `AgentTarget` | CLI | MCP | 推荐 |
|---|---|---|---|---|---|
| `cursor` | Cursor | `Cursor` | ✓ | ✓ | CLI |
| `claude` | Claude Code | `ClaudeCode` | ✓ | ✓ | CLI |
| `codex` | Codex | `Codex` | ✓ | ✓ | MCP |
| `grok` | Grok | `Grok` | ✗ | ✓ | MCP |
| `antigravity` | Antigravity | `Antigravity` | ✓ | ✓ | CLI |
| `pi` | Pi | `Pi` | ✓ | ✗ | CLI |

`id` 与 `AgentTarget::parse` 接受的字符串一致；`AgentKind` 由既有的 `agent_mode::kind_for_target(target)` 得到，不在表里重复。

```rust
pub struct AgentDef { pub id: &'static str, pub title: &'static str, pub target: AgentTarget,
                      pub cli: bool, pub mcp: bool, pub recommended: Mode }
pub fn all() -> &'static [AgentDef];  pub fn available() -> Vec<&'static AgentDef>;
pub fn find(id: &str) -> Option<&'static AgentDef>;
pub fn current_mode(def: &AgentDef) -> Result<Mode, String>;
pub fn needs_update(def: &AgentDef) -> Result<bool, String>;
```

`current_mode` / `needs_update` 是两条读取路径的统一入口：非 Antigravity 直接用 `agent_mode::current(target)` / `agent_mode::needs_update(target)`（二者不会失败）；Antigravity 用 `agent_mode::antigravity_status()`（严格读取，返回 `(Mode, ArtifactUpdates, bool)`，失败即 `Err`），`needs_update` = 其 `ArtifactUpdates` 的 `rule` / `hook` / `mcp` 任一为真。`Err` 只影响该行（该行显示「无法读取状态」且不可操作），不阻塞向导，也不影响三域判定。

`onboarding/detect.rs`：「已安装」满足任一信号（D6）。纯函数 `classify(inputs) -> Signals { cli, config_dir, app }`，真实取证函数 `scan()`：

| 信号 | 取法 |
|---|---|
| `cli` | `agent_launch::all_readiness()`（登录 shell 探测，自带缓存与并行）里该家 `executable.is_some()`；不新增探测函数 |
| `config_dir` | `paths::{cursor_dir, claude_dir, codex_dir, grok_dir, pi_agent_dir}()` 存在；Antigravity 无独立目录信号 |
| `app`（仅 macOS） | `/Applications` 与 `~/Applications` 下存在 `Cursor.app` / `Codex.app` / `Antigravity.app`；其余家无此信号 |

行数据（`AgentScanRow`，camelCase）：`id` / `title` / `signals` / `detected`（`signals` 任一）/ `mode`（`none|cli|mcp`）/ `needsUpdate` / `statusError`（Antigravity 严格读取失败时的原因，行显示「无法读取状态」且不可操作）/ `supportsCli` / `supportsMcp` / `recommended` / `delivery`（仅 Codex：`codex_protocol::delivery().as_str()`，即 `notInstalled|trusted|untrusted|needsUpdate|unsupportedVersion`）/ `planned: { cli: PlannedFile[], mcp: PlannedFile[] }`。

`needsUpdate`：非 Antigravity = `agent_mode::needs_update(target)`；Antigravity = `antigravity_status()` 的 `ArtifactUpdates` 任一为真。

### 1.5 应用、回读与「将改动的文件」预览

`onboarding/apply.rs`：

```rust
pub struct Selection { pub agent: String, pub mode: Mode }       // mode 只会是 Cli | Mcp：向导从不卸载
pub struct ApplyResult { pub agent: String, pub requested: String, pub ok: bool,
                         pub error: Option<String>, pub mode_after: String, pub delivery: Option<String> }
pub fn apply(selections: &[Selection]) -> Vec<ApplyResult>;
```

逐家顺序执行，单家失败不影响其他家：

1. 校验 `agents::find` 与该家支持该模式（Grok 无 CLI、Pi 无 MCP）；不满足 → 该行 `ok=false`，不调用底层。
2. `current = current_mode(def)`；读取返回 `Err` 时该行 `ok=false`、`error` 为原因，不调用底层。`current == mode` 且 `needs_update(def)` 为假：无事可做，`ok=true`；`current == mode` 且 `needs_update(def)` 为真：调用 `agent_mode::update(target)`；其余情况调用 `agent_mode::set(target, mode)`。
3. 回读 `mode_after = current_mode(def)`；`ok = 无 error && mode_after == mode`，不一致时 `error = "read-back mismatch: expected X got Y"`。
4. Codex：`delivery = codex_protocol::delivery().as_str()`；刚装完为 `untrusted` 是**预期状态**（D9：「待信任」提示），不算失败。

Dev 实例的拒绝由 `agent_mode::set` 内的 `IntegrationMutationLock::acquire()` 完成（与既有 `agent_mode_set` 命令同一行为），结果以各行 `error` 返回。

`onboarding/plan.rs`：`planned_files(def, mode) -> Vec<PlannedFile { path: PathBuf, exists: bool }>`，由 `paths::*` 组合（因此随 `CODEX_HOME` 等覆盖保持一致）。初稿如下，**最终以守卫测试的实测集合为准（测试失败时改表，不改守卫）**：

| (Agent, 模式) | 文件 |
|---|---|
| Cursor · CLI | `cursor_rule_file`、`cursor_hooks_json`、`cursor_hook_script` |
| Cursor · MCP | `cursor_rule_file`、`cursor_hooks_json`、`cursor_mcp_json` |
| Claude · CLI | `claude_md`、`claude_settings_json`、`claude_hook_script` |
| Claude · MCP | `claude_md`、`claude_settings_json`、`claude_json` |
| Codex · CLI / MCP | `codex_hooks_json`、`codex_config_toml`（两种模式都写：协议 hook 与启用 hooks 的开关；MCP 另在同一文件里加服务条目） |
| Grok · MCP | `grok_skill_md`、`grok_config_toml`、`grok_hooks_json` |
| Pi · CLI | `pi_agents_md`、`pi_extension_file` |
| Antigravity · CLI / MCP | `antigravity_rule_file`（MCP 另加 `antigravity_mcp_config_json`） |

守卫测试（沿用 `commands.rs` 里「子进程 + 临时 HOME + `ASKHUMAN_HOME`」模式，约 5343–5380 行）：在一个子进程里，对每个 (Agent, 模式) 先清空 HOME、再执行 `agent_mode::set`，比较「HOME 之下被创建或修改的文件」与 `planned_files`；`ASKHUMAN_HOME` 下的集成状态 / 锁文件不列入预览。`set` 对 Codex 会探测其版本（登录 shell + `codex --version`），二者都会在 HOME 里留下自己的文件（`.zsh_history`、`.codex/tmp/…`），所以测试里用 `agent_launch::with_no_agent_installed` 让探测立即返回「未安装」——测试既不跑登录 shell，也不启动任何真实 Agent 二进制。

### 1.6 环境体检 `onboarding/env.rs`

`EnvReport`（camelCase）：

```text
askhumanOnPath: "found" | "missing" | "unknown"
pathFix:   { command, dir, needsPathEdit } | null     // missing 且二进制不在临时位置时给出
transient: { path, kind: "temp" | "downloads" | "npx" } | null
installs:  { message } | null
```

- **PATH**：`agent_launch.rs` 新增 `pub(crate) fn lookup_with_login_path(name: &'static str) -> (EnvLookup, String)`（`EnvLookup = Found(path) | Missing | Unknown(detail)`）。Unix 复用现有私有的 `run_login_shell` / `interpret_lookup` / `marked_line`（不改它们的行为）取 `command -v AskHuman` 与登录 shell 的 `PATH`；Windows 用 `resolve_windows_executable`，`PATH` 取当前进程环境。`Unknown` 不出卡。
- **稳定位置**：纯函数 `is_transient_location(path, home, temp_dir) -> Option<kind>`——命中 `std::env::temp_dir()`、`/tmp`、`/private/tmp`、`/var/folders`（含 `/private` 前缀）、`~/Downloads`、路径含 `_npx` 段（npm npx 缓存）；Windows 同理（`%TEMP%`、`%USERPROFILE%\Downloads`、`\_npx\`）。对 `current_exe()` 判定。
- **修复命令**（纯函数 `fix_command(exe, login_path_dirs, home, is_writable, os)`）：
  - Unix：在登录 `PATH` 里依次找「位于 `$HOME` 下且可写」的目录，其次「任意可写」目录；命中则 `ln -sf '<exe>' '<dir>/AskHuman'`。都没有 → `mkdir -p ~/.local/bin && ln -sf '<exe>' ~/.local/bin/AskHuman`，且 `needsPathEdit=true`（卡片附一行把 `export PATH="$HOME/.local/bin:$PATH"` 加入 shell 配置的说明）。
  - Windows：PowerShell 一行，把二进制所在目录追加到用户级 `Path`（`[Environment]::SetEnvironmentVariable('Path', … + ';<dir>', 'User')`），提示新开终端生效。
  - 二进制在临时位置时不给 `pathFix`（只出「稳定位置」卡，避免把临时路径链进 PATH）。
- **多份**：`cli/install_check.rs` 新增 `collect_with_path(status, path_dirs: Option<Vec<PathBuf>>)`（`collect(status)` = `collect_with_path(status, None)`），体检传登录 `PATH` 目录；`Finding::message` 改 `pub(crate)`。文案 = 第 1 条 finding 的 `message(lang, now_ms)` + 「详见 `AskHuman doctor install`」，多于 1 条再追加「另有 N 项」。daemon 状态用不启动 daemon 的查询（无 daemon = `None`）。
- Dev 实例（`is_dev_instance_context()`）：整份报告为空。
- 卡片可见性由前端按行状态决定：PATH 卡 = `askhumanOnPath == "missing"` 且存在「已集成或本次勾选」的 CLI 模式 Agent；稳定位置卡、多份卡有值即显示。

### 1.7 P1 测试

Rust 单测（均不碰真实 HOME / 钥匙串）：

- `uistate`：旧 JSON（只有 `imTipDismissed`）可读；`update` 往返保留旧字段；`skipped` 的未知值被丢弃、去重。
- `state::derive`：三域真值表（每个证据单独成立 / 全不成立；旧 `im_tip_dismissed` ⇒ 渠道域已决定；`agent_kind` 空串不算历史证据；渠道域不受 Agent 状态影响）。
- `gather`：一个临时 HOME + `ASKHUMAN_HOME` 的集成测试（环境变量用现有 `ENV_LOCK` 模式）。
- 门控：`login_item_gate_open` 的组合真值；`restart_via_launchd` 的 (模式 × 是否有登录项) 矩阵。
- `detect::classify`、`agents` 表对拍（T6）、`apply`（假底层，覆盖：支持校验、`set` / `update` / 无事可做三条路径、回读不一致）、`plan::planned_files` 守卫、`env`（`is_transient_location`、`fix_command` 的目录选择与引号、`Found/Missing/Unknown × 稳定位置 × finding × Dev` 的报告组合）。
- `config_cmd`：设置 / 取消 `general.menuBarIcon` 会标记 `background_decided`，其它 key 不会。

## 2. P2：CLI `setup` 与文字流程

### 2.1 分发（`cli/mod.rs`、`cli/setup_cmd.rs`、`dev_instance.rs`）

- `cli/mod.rs` 分发表加 `"setup" => setup_cmd::dispatch(&argv[2..], lang)`（与 `channel` / `config` 同位置）；`setup_cmd::dispatch` 自己 `exit`。
- 参数：无 / `--text` / `help` `-h` `--help`；其它参数 → 错误 + 用法，退出 1。
- 选路：`--text` 或 `!daemon::runtime::has_display()`（改 `pub(crate)`）→ 文字版，要求 stdin 为 TTY（`cfgio::stdin_is_tty()`），否则报错退出 1（验收 2）；否则 `gui_host::host_open(WindowKind::Setup, false, None, None, None)`，成功立即 `exit(0)`、什么都不打印。`host_open` 失败时：stdin 为 TTY 则降级走文字版并提示，否则报错退出 1（不做「进程内建窗」兜底）。
- **分期**：`WindowKind::Setup` 要到 P3 才存在，所以 P2 的 `setup` 只走文字版（要求 TTY；`--text` 已被接受，P3 之前与不加没有区别）。`has_display` 改 `pub(crate)` 与上面的 GUI 分支一起在 P3 加入，P3 同时把选路抽成纯函数并补选路测试。
- `dev_instance::classify_command`：`"setup"` 归入 `CommandClass::Config`（可在无实例二进制时运行并写实例 home）。
- `agents/transcript_full.rs` 的 `NON_ASK_SUBCOMMANDS` 加 `"setup"`。
- `cli/help.rs`：EN + ZH 各加一条 `setup [--text]`（安装后的引导设置；无图形会话自动用文字版），放在「管理」块第一行。

### 2.2 终端无参欢迎语（D2 / D17，验收 3）

`cli/mod.rs` 无参分支：在打印完帮助文本之后、`exit(1)` 之前，当 `stdin` 与 `stderr` 都是 TTY 且 `!onboarding::state::agents_decided()` 时，向 **stderr** 追加一行欢迎语（`i18n::tr(lang, "cli.setupWelcome")`）：

- zh：``第一次使用？运行 `AskHuman setup`，约 2 分钟接入 Agent（以及可选的远程回答）。``
- en：``New here? Run `AskHuman setup` to connect your Agents and, optionally, remote answering (about 2 minutes).``

命令名用 `{prog}` 占位（与 `cli.seeAgentHelp` 同款），所以 npm 包里以别的程序名运行时也对。退出码保持 1；非 TTY（Agent 内调用）不出现；`agents_decided()` 只在 stdin 与 stderr 都是 TTY 时才现算（`setup_cmd::welcome_due` 接收一个闭包，Agent 的管道调用不会读盘）。

### 2.3 文字流程 `onboarding/text.rs`（D20，验收 1 / 2 / 6）

入口 `run(lang, io: &mut dyn TextIo, deps: &mut dyn TextDeps) -> i32`：`TextIo { say, note, ask }`（stdout / stderr / 读一行，EOF 视为错误）；`TextDeps` 封装全部副作用（扫描、环境体检、读写 `AppConfig`、应用、渠道状态、渠道向导、登录项、`ui-state` 标记）。真实实现 `StdIo` + `RealDeps`，测试用脚本化 `FakeIo` + `FakeDeps`。文案走 `cfgio::t(lang, en, zh)`。

**步骤 0**：打印标题；环境体检——「稳定位置」「多份」命中时立即打印，「PATH」在 Agent 选择确定后、应用前，对「已集成或本次选择」的 CLI 模式 Agent 判定后打印（含修复命令）。无命中不打印任何东西。

**[1/3] 接入 Agent**

- 编号列表：已检测到或已集成的在前，其余标「（未检测到）」排后。预选规则（D19，与 GUI 同）：已集成 → 选中并保持当前模式；检测到未集成 → 选中并取推荐模式；未检测到 → 不选。有 `needsUpdate` 的已集成项标「有更新」且计入待应用；Codex `untrusted` 标「待信任」。
- 有待应用项：列出每家「将改动的文件」（标新建 / 修改），提示 `应用以上设置？[Y 应用 / n 逐家调整 / 编号 只调整一家 / s 跳过此步]`：回车或 `y` = 应用；`n` = 按序逐家询问 `模式 [c=CLI / m=MCP / x=不改动]`（回车取该家当前预选），问完重新展示并再次确认；编号 = 只调整该家；`s` = 跳过。无待应用项：提示「Agent 已是最新」，回车继续、编号可调整。（spec §3.3 的示例提示为 `[Y/n/编号调整]`，这里多一个 `s` 是为了与 GUI 的「跳过此步」对应。）
- `s` 跳过：仅在 Agent 域未决时 `mark_skipped("agents")`，直接进入第 2 步。
- 确认之后、执行之前询问协作风格，编号选择 `1) 对齐  2) 自主`，当前值标「（当前）」并作为回车默认；当前为 `custom` 时增加 `3) 自定义（当前，在设置页编辑）` 并默认选中。无待应用项时同样询问（回车保持当前）。选择与当前不同 → 写 `general.collaborationStyle`（`AppConfig::load()` / `save()`），随后调用 `commands::collaboration_style_apply_integrations()`，再执行 `apply`（先风格后集成，D7）。
- 执行后逐家打印 ✓ / ✗ + 原因；末尾提醒「已打开的 Agent 会话需要重启才会加载」；Codex 待信任、Cursor 行（项目需在用户主目录下）附既有说明。

**[2/3] 远程回答（可跳过）**

- 列表：`1) Telegram  2) Slack  3) 钉钉  4) 飞书  s) 跳过  [s]`，每项后附状态（未配置 / 已配置 / 已启用）。顺序与 GUI 相同（Telegram、Slack、钉钉、飞书），由 `onboarding` 内的一个常量给出，不沿用 `channel_cmd::CHANNELS` 的顺序；其内部 id 为 `telegram` / `slack` / `dingding` / `feishu`。回车 = `s`。
- 选中渠道 → `channel_cmd::wizard_for_setup(id, lang)`（新增 `pub(crate)`：与现有 `wizard` 同流程，只是「启用该渠道？」默认 Yes）；完成后：钉钉 / 飞书 / Slack 且用户 ID 仍为空 → 询问 `自动识别你的用户 ID？[Y/n]` → `channel_cmd::detect`（改 `pub(crate)`；其内部自带「是否保存」提示）；再问 `测试连接？[y/N]` → `channel_cmd::test`（改 `pub(crate)`）。回到列表可继续配下一个。
- 退出本步时若**没有任何已启用渠道** → `mark_skipped("remote")`；有则不写。

**[3/3] 后台与启动**

- A（仅 `gui_host::tray_supported()` 为真时问，改 `pub(crate)`）：`菜单栏常驻并开机自启？[Y/n]`，默认 = 当前是否 `always`。
- B（仅有 IM 渠道启用时问）：`保持后台常驻（让手机 IM 随时能发起命令，会常驻少量资源）？[y/N]`，默认 = 当前是否 `keepalive`。
- 写入：A 开 → `menuBarIcon = always` 且 `login_item::ensure_installed()`；A 关且当前为 `always` → `active` 且 `login_item::uninstall()`；A 关且当前为 `active` / `off` → 不变。B → `daemonLifecycle = keepalive | activity`。`mark_background_decided()`；Agent 域仍未决（既没集成也没跳过）→ `mark_skipped("agents")`。
- 打印摘要（已集成 Agent / 风格 / 已启用渠道 / 后台）与「下次在 Agent 里直接使用；随时 `AskHuman setup` 重跑」。

中途 EOF / Ctrl-C：退出码 1，不保存步骤进度，已应用的改动保留。不提供 `--yes`。**文字版不含三项可选验证**（那三项只在 GUI 完成页提供）。

### 2.4 i18n（Rust `i18n.rs`）

新增 `cli.setupWelcome`（EN + ZH）；文字流程自己的文案不进 `i18n.rs`，沿用 `channel_cmd` / `config_cmd` 的做法，在代码里用 `cfgio::t(lang, en, zh)` 成对给出（§2.3）。`title.setup`、`tray.openSetup` 在 P3 加。

### 2.5 P2 测试

- 脚本化流程（`FakeIo` + `FakeDeps`）：新用户全回车 → 以推荐模式应用两家、渠道步回车 ⇒ 记 `skipped ∋ remote`、A 默认 → 写 `always` + `background_decided`；重跑且无待应用项 → 不出应用确认、不改已有集成；`n` 逐家调整；编号只调整一家；`s` 跳过 ⇒ `skipped ∋ agents`；风格改动先于应用且自定义可保持；A 仅在托盘受支持时出现、B 仅在有 IM 渠道时出现；A 关的三种当前值映射；环境警告只在命中时打印。
- `setup_cmd` 选路：`--text` / 无显示 / stdin 非 TTY 的退出码与消息；参数校验。
- 终端欢迎语：TTY × 域已决定 / 未决的四象限，且退出码仍为 1。
- 手动：pty 脚本（`script` / python `pty`）在「临时 HOME + `ASKHUMAN_HOME`」里跑一遍新用户与重跑。

## 3. P3：GUI 向导与全部入口

### 3.1 窗口路由与 Tauri 命令层

**`gui_host/mod.rs`**：`WindowKind` 新增 `Setup`（serde lowercase → `"setup"`）。

**`app/gui_host.rs`**：

- `is_hosted_label` 加 `"setup"`；`open_window` 的 `match kind` 加 `WindowKind::Setup => create_setup_window(app, &cfg, param.as_deref(), pin_above_popup)`，建后聚焦的 label 映射加 `"setup"`；`param` 的文档注释补一句「安装向导窗口 = 目标页」。新增枚举变体后，编译器会指出其余穷尽匹配点（`open_window` 的两处 `match`、`commands.rs::route_open_window`、`gui_host/mod.rs` 的序列化往返测试），逐一补上。
- `HostState` 新增 `setup_pending: AtomicBool`（`setup()` 里初始值 `!agents_decided()`）；`refresh_integration_updates` 末尾重算 `setup_pending`，变化则 `refresh_on_main` 并 emit `setup-status-changed`（Agent 集成变化与该汇合点一致，T5）。
- 托盘：`build_specs` 与 `menu_signature` 各加 `setup_pending: bool` 参数（签名串加一项；`refresh_tray` 从 `HostState.setup_pending` 读取；机械更新 `gui_host.rs` 测试里的 7 处 `build_specs` 与 3 处 `menu_signature` 调用点）。`setup_pending` 为真时在操作区 `open_settings` **之前**插入条目 `open_setup`（文案 `tray.openSetup`：「设置向导…」/ "Setup Guide…"）；点击 → `open_window(app, WindowKind::Setup, false, Some("agents".into()), None, None, None)`。图标与 tooltip 不变。`tray_supported()` 改 `pub(crate)`。

**`app/mod.rs`**：新增 `create_setup_window(manager, config, page: Option<&str>, pin_above_popup)`（整体结构照抄 `create_settings_window`，Linux 页面缩放分支照抄 `create_new_task_window`）：已开窗 → `set_focus`，`page` 非空则 emit `setup-goto-page`（payload 为页名字符串）；新建 → URL `index.html?view=setup[&page=…]`（再经 `append_window_effect_query` 追加窗口材质参数），标题 `development::window_title(i18n::tr(lang, "title.setup"))`，尺寸 640×720、最小 560×600、居中，`always_on_top(pin_above_popup)`，主题 / 材质经 `window_theme` / `apply_surface` / macOS `set_runtime_window_effect_with_bg` 与设置窗口同款处理。

**`commands.rs` `route_open_window`**：兜底分支加 `WindowKind::Setup => create_setup_window(&fallback, &cfg, project.as_deref(), pin)`。`capabilities/default.json` 的 `windows: ["*"]` 已覆盖，无需改。

**`app/invoke.rs`**：新增 `Group::Setup`（`setup(invoke)`，`#[inline(never)]`，`generate_handler!` 只含下表命令）；`group_for` 里 `command.starts_with("setup_") || matches!(command, "open_setup" | "popup_setup_hint" | "popup_setup_hint_dismiss")` 放在 **Channel 分支之前**；从 `core` 的 `generate_handler!` 里删除 `popup_im_tip_visible` / `popup_im_tip_dismiss`；补 `group_for` 单测。

**`onboarding/commands.rs`**（Tauri 层，先例 `todo_sync::commands`；耗时命令一律 `spawn_blocking`）：

| 命令 | 参数 → 返回 | 说明 |
|---|---|---|
| `setup_status` | → `SetupStatus` | `{ agentsDecided, remoteDecided, backgroundDecided, loginItemInstalled, loginItemAvailable, trayAvailable, everydayIntegrationsReadOnly }`；`trayAvailable = tray_supported()`；`loginItemAvailable = trayAvailable && !is_dev_instance_context()`；`loginItemInstalled = instance_has_login_item()` |
| `setup_agents_scan` | → `AgentScanRow[]` | §1.4 |
| `setup_env_check` | → `EnvReport` | §1.6 |
| `setup_apply_agents` | `selections` → `ApplyResult[]` | §1.5；结束后 `refresh_integration_updates(&app)` |
| `setup_channel_status` | → `{ id, enabled, configured, connected: bool \| null }[]` | `enabled` / `configured` 来自 `channel_cmd::is_enabled` / `is_configured`；`connected` 来自不启动 daemon 的状态查询里的 `im_connections`（daemon 未运行 = `null`） |
| `setup_skip` | `domain: "agents" \| "remote"` | `mark_skipped` + emit |
| `setup_finish` | — | `mark_background_decided()`；读磁盘配置，`menuBarIcon == always` 则 `ensure_installed()`；Agent 域仍未决 → `mark_skipped("agents")`；刷新托盘 + emit。不改 `config.json`（T4） |
| `setup_login_item_enable` | — | §1.3 |
| `open_setup` | `page: Option<String>` | `route_open_window(Setup, …)`；弹窗进程也经此命令打开 |
| `popup_setup_hint` | → `{ id, page } \| null` | `hint::next_hint(&decided())`：v1 只有一项，`!decided.remote` 时返回 `{ id: "remote", page: "remote" }`；表驱动（待办项 → 文案 id + 目标页），以后新增只加一项 |
| `popup_setup_hint_dismiss` | `id` | 只接受已知 id；`mark_skipped(id)` + emit |

### 3.2 前端骨架与共享抽取

- `App.vue`：`view === "setup"` → 异步组件 `SetupView`。
- `views/settings/context.ts`：`createCore()` 加 `export`（设置页与向导共用同一个 `persist()`、密钥「已保存」占位与清除语义）。
- `ChannelsTab.vue`：四张渠道卡加元素 id `channel-<id>`，并接入既有的 `settingsTargetHighlight` 高亮，使 `channel#channel-<id>` 深链可定位；iOS 行的深链指向 `experimental` tab。
- `lib/ipc.ts` + `lib/types.ts`：上表命令的封装与类型（`SetupStatus`、`AgentScanRow`、`PlannedFile`、`ApplyResult`、`EnvReport`、`ChannelStatusRow`、`SetupHint`）；`AskRequest` 加 `setupProbe?: boolean`（P4）。
- 新文件：
  - `views/SetupView.vue`：窗口壳（可点击的步骤条 + 页面容器 + 底部导航）；挂载时 `Promise.all([getSettings(), setupStatus()])`，把 `config` / `secretsPresent` / `everydayIntegrationsReadOnly` 灌入 `core`（同 `SettingsView`）；应用主题 / 语言并监听 `settings-updated`；初始页读 URL `?page=`，监听 `setup-goto-page` 与 `setup-status-changed`。
  - `views/setup/useSetup.ts`：状态机与动作（页、`status`、`rows`、`style`、`applyResults`、`env`、`channelStatus`、后台两个开关）。
  - `views/setup/AgentsPage.vue`（Agent 磁贴 `AgentTile.vue`）、`RemotePage.vue`、`DonePage.vue`、`SetupChannelForm.vue`、`EnvCards.vue`；共用 `SetupHeader.vue`（页头）与 `BrandBadge.vue`（徽标，图案注册表 `brandBadges.ts`）。版式见 spec §8 第 7 轮，图标素材（Agent 含 Antigravity 都用 iOS 的 SVG 源，渠道的在 `views/setup/logos/`，来源与许可见该目录 `NOTICE.md`）见第 8 轮。
  - `views/settings/useSetupStatus.ts`：共享的 `status` + `refresh()`（挂载、`setup-status-changed`、窗口获得焦点时刷新），供设置页横幅 / 卡片 / 菜单栏提示使用。
- 中途关闭窗口不保存步骤进度；已应用的改动保留（D4）。

### 3.3 第 1 页：接入 Agent（D5–D9、D19、D23）

- 挂载即并行发起 `setupAgentsScan()` 与 `setupEnvCheck()`；扫描期间显示占位，不阻塞步骤导航。
- 分组：**主区** = `detected || mode != none` 的行；**「未检测到的 Agent（N）」折叠区** = 其余（默认不勾选，仍可手动接入）。
- 预填：`mode`（所选模式）= `currentMode != none ? currentMode : recommended`；`checked` = `currentMode != none ? true : detected`。已集成行取消勾选 = 「本次不动它」，向导永不卸载。
- 每个磁贴（`AgentTile.vue`，整块可点勾选）：徽标 + 标题 + 状态标签 + 勾选标记 + 接入方式分段控件（只列该家支持的模式，推荐项带「推荐」；只支持一种时显示固定标签；点选方式同时勾选该家）。「将创建或修改的文件」不放在磁贴里，而是磁贴下方的折叠合并清单（按 Agent 分组，来自 `planned[mode]`，标新建 / 修改）。状态标签：已集成 →「已集成 · CLI/MCP」；`needsUpdate` →「有更新」（默认纳入应用）；Codex `delivery == untrusted` →「待信任」，并在磁贴下方的提示卡片里写「首次启动 Codex 时按提示信任 Hook」；`unsupportedVersion` → 磁贴内错误；`statusError` →「无法读取状态」（磁贴禁用）。勾选 Cursor 时，同一提示卡片附既有 `settings.integration.cursorRulesHint` 文案。「已检测到」标签不在磁贴上重复（分区标题已写明）。
- `pending(row)` = `checked && (mode != currentMode || needsUpdate)`。主按钮：存在 `pending` 或风格有改动 →「应用并继续」，否则「下一步」。「跳过此步」仅在 Agent 域未决时显示，点击 `setupSkip("agents")` 后前进。
- 协作风格 2 选 1（对齐 / 自主，各一句说明），预选当前值；当前为 `custom` 时多一项已选中的「自定义（在设置页编辑）」。
- 应用顺序（D7）：风格有改动 → `core.config.general.collaborationStyle = …` → `await core.persist()` → `await collaborationStyleApplyIntegrations()`；随后 `setupApplyAgents(pending 行)`；刷新扫描。结果就地显示每行 ✓ / ✗ + 原因（失败行可重试）；全部处理完显示「已打开的 Agent 会话需要重启才会加载」；主按钮变「继续」，不自动跳转。
- Dev（`everydayIntegrationsReadOnly`）：整个 Agent 区只读（行、风格均禁用，隐藏应用按钮），顶部显示只读说明；「下一步」仍可用。
- 页头：存在已集成 Agent 视为重跑，显示精简页头与「已保留你现有的配置，只会改动你勾选的项」；否则显示欢迎页头（应用图标 + 欢迎标题 + 首次文案）。
- 环境红卡（`EnvCards.vue`，位于页面顶部，无命中不占版面）：PATH 卡（含可复制的修复命令与 `needsPathEdit` 说明）、稳定位置卡、多份卡（摘要 + `AskHuman doctor install`）；可见性规则见 §1.6。

### 3.4 第 2 页：远程回答（D10、D18、D19、D22）

- 渠道由注册表 `views/setup/channelDefs.ts` 驱动：`feishu`（带「推荐」标）/ `telegram` / `dingding` / `slack` 四项，顺序同设置页的渠道页；当「实验功能」开启（`core.config.experimental.enabled`）时再追加一行「iOS（实验）」，点击 `openSettings("experimental")`，不内嵌表单。后续把 iOS 接成同构模块只需新增一项。
- 列表态：每行显示状态（未配置 / 已配置 / 已配置 · 已连接 / 异常——异常取 `useChannels.channelIssueText(id)`）+「配置」或「编辑」。页面可见期间 `core.activeTab.value = "channel"`（让 `useChannels` 的健康轮询工作），离开时还原 `"general"`；进入页面与保存后刷新 `setupChannelStatus()`。
- 详情态（`SetupChannelForm.vue`，一次一个渠道，返回回到列表）：3–5 行平台侧要点（从 `docs/wiki/<slug>.md` 摘取，i18n）+「查看教程」（`useChannels.openChannelGuide(id)`）+ 必填字段 + 自动识别 +「保存并启用」+「在设置页打开更多选项」（`openSettings("channel#channel-<id>")`，多账号 / 超时 / 卡片模板等高级项都走这里）。

| 渠道（`core.config.channels.<id>`） | 必填字段 | 自动识别 |
|---|---|---|
| Telegram（`telegram`） | `botToken`（密钥）、`chatId` | 无（先给机器人发一条消息，再按教程取 `chatId`） |
| 钉钉（`dingding`） | `clientId`、`clientSecret`（密钥）、`userId` | `runDingtalkDetect` |
| 飞书 | `appId`、`appSecret`（密钥）、`openId` | `runFeishuDetect` |
| Slack | `botToken`、`appToken`（均密钥）、`userId` | `runSlackDetect` |

- 「保存并启用」：必填项齐全（密钥字段以 `secretsPresent` 的「已保存」占位视为已填）→ `enabled = true` → `await core.persist()`；保存后出现「测试往返」按钮（P4 才有实际行为，P3 先不渲染）。连接测试沿用 `useChannels` 的 `run*Test` / `run*Detect`，与设置页同源同结果（验收 11）。
- 页脚按钮：没有任何已启用渠道时只显示「跳过」（`setupSkip("remote")` 后前进）；至少一个已启用渠道时显示「继续」（不写 `skipped`）。

### 3.5 第 3 页：完成（D8、D12、D19）

- 摘要：已集成的 Agent（含模式）、协作风格、已启用渠道；「已启用的能力」一行——对每个已集成 Agent 复用 `agentModeStatus(id)`，汇总生命周期追踪 / 权限接管 / 停止确认 / AskUserQuestion 接管的已启用项（字段以 `AgentModeStatus` 为准），带「去设置页调整」链接（`openSettings("integration")`）。
- 后台与启动紧凑卡：
  - 开关 A「菜单栏常驻并开机自启」：仅 `status.trayAvailable` 时显示；初始 = `menuBarIcon == always`；旁注提示 macOS 通常会弹出「后台项目已添加」通知。
  - 开关 B「保持后台常驻」：仅存在已启用 IM 渠道时显示；初始 = `daemonLifecycle == keepalive`；说明「让手机上的 IM 随时能发起命令，会常驻少量资源」。
- 「完成」：A 开 → `menuBarIcon = "always"`；A 关且当前为 `always` → `"active"`；A 关且当前为 `active` / `off` → 不变；B → `daemonLifecycle`；`await core.persist()`；再 `await setupFinish()`（登录项写入、`background_decided`、必要的 `skipped ∋ agents`）；最后关闭窗口。
- 「下次怎么用」文案；P4 的「本机自测」「在 Agent 里试一次」按钮放在摘要下方（P3 不渲染）。

### 3.6 设置页入口（D2 ③⑥、D13、验收 3 / 5 / 8）

- `SetupBanner.vue`：`SettingsView.vue` 的 `tabbar` 与 `settings-body` 之间；条件 `!status.agentsDecided`；文案 + 「打开向导」（`openSetup("agents")`）+ ✕（`setupSkip("agents")`，等同跳过 Agent 步骤）。
- `SetupCard.vue`：`GeneralTab.vue` 顶部常驻卡「设置向导」+ 说明 + 「打开向导」，不受任何状态影响。
- 「通用 → 菜单栏图标」卡：`menuBarIcon == always && status.loginItemAvailable && !status.loginItemInstalled` 时多一行「开机自启尚未启用」+「启用」按钮（`setupLoginItemEnable()` 后刷新状态）；Dev 实例不显示（`loginItemAvailable` 为 false）。`useSetupStatus` 在窗口获得焦点与 `setup-status-changed` 时刷新，使显式切换模式后提示自动消失。

### 3.7 弹窗提示条取代旧 IM 小条（D18，验收 4）

- `lib/ipc.ts`：`popupImTipVisible` / `popupImTipDismiss` 换成 `popupSetupHint()` / `popupSetupHintDismiss(id)` / `openSetup(page?)`。
- `usePopupCore.ts`（约 29 / 1184–1194 / 2701 / 2990–2992 行）：`imTipVisible` / `imTipConfigure` / `imTipDismiss` 换成 `setupHint`（`ref<{id, page} | null>`）/ `setupHintAct()`（`openSetup(hint.page)`，同时本地隐藏并 `popupSetupHintDismiss(hint.id)`）/ `setupHintDismiss()`（隐藏并 `popupSetupHintDismiss(hint.id)`）；原 `popupImTipVisible()` 调用处改为：当前提问 `setupProbe === true` 时跳过，否则 `setupHint.value = await popupSetupHint()`。
- `PopupFooter.vue`（约 16–18 / 46–55 行）与 `popup.css`（`.im-tip*` → `.setup-hint*`）：改用新字段；文案 `popup.setupHint.<id>.text` / `.action` / `.dismiss`（`remote`：「配置 IM 渠道后，离开电脑也能在手机上收到并回复提问。」/「去配置」/「不再提示」）。
- 点「去配置」与 ✕ 都记为该项已决定（与旧小条一致，不反复提醒）。
- 同步更新：`PopupFooter.later.test.ts`（字段重命名）、`src/prototype/replyMenuMock.ts` 与 `sessionTitleMock.ts` 里的 `popup_im_tip_visible` 分支（改为 `popup_setup_hint` 返回 `null`）。`src/prototype/annotationsMock.ts` 属于另一项进行中的工作（message-annotations），其中同名分支不由本计划修改，该工作合入时随之改名。

### 3.8 i18n（`src/i18n/zh.ts` / `en.ts`；Rust `i18n.rs`）

- 前端新增键组：`setup.*`（窗口标题、步骤名、各页文案、状态标签、按钮、环境卡片、各渠道要点、完成页、重跑欢迎）、`popup.setupHint.*`（替换 `popup.imTip.*`）、`settings.setup.*`（横幅 / 卡片 / 开机自启提示）。
- Rust：`title.setup`、`tray.openSetup`。

### 3.9 P3 测试

vitest：

- 向导壳：`?page=` 初始页、`setup-goto-page` 事件跳页、步骤条跳转。
- 第 1 页：预填表（已集成保持勾选 / 检测到未集成默认勾选 / 未检测到折叠不勾选 / `needsUpdate` 默认纳入 / Codex 待信任 / `statusError` 禁用）、主按钮文案、风格含「自定义」、应用顺序（风格 → 集成）、Dev 只读、环境卡可见性。
- 第 2 页：状态标签、详情保存即启用（调用 `persist`）、页脚「跳过 / 继续」规则、iOS 行仅在实验功能开启时出现。
- 第 3 页：A / B 的显示条件与三种映射、「完成」先 `persist` 再 `setupFinish`。
- `SetupBanner`（显示 / 关闭）、`SetupCard`、菜单栏图标提示（含 Dev 隐藏）、弹窗提示条（显示 / 隐藏 / 探测提问隐藏 / 两个按钮的调用）。

Rust：`group_for` 单测；`hint::next_hint`；`build_specs` / `menu_signature` 含 `setup_pending` 的用例（更新既有测试调用点）；`setup_status` 字段在 Dev 与非 Dev 下的取值。

手动（临时 HOME + `ASKHUMAN_HOME`）：全新环境走完 3 页；重跑；渠道详情保存与深链定位；设置页横幅与卡片；托盘项；弹窗提示条。

## 4. P4：可选验证（D11，验收 10）

验证都是「想测就测」的按钮：任一失败 / 超时都不阻止「完成」，也不改变「完成」判定。

### 4.1 daemon 侧：探测请求的识别与处理

- 识别与策略（`onboarding/probe.rs`，纯函数，便于单测）：

  ```rust
  pub enum ProbeTarget { Popup, Channel(&'static str) }       // 渠道 id 仅限 telegram | dingding | feishu | slack
  pub enum ImScope { All, Nothing, Only(&'static str) }
  pub struct ProbePolicy { pub popup: bool, pub im: ImScope, pub ask_received_hook: bool, pub follow_active_channel: bool }
  pub fn parse_probe_id(native_request_id: Option<&str>) -> Option<ProbeTarget>;  // 前缀 setup-probe:；其它值 → None
  pub fn policy(target: Option<ProbeTarget>) -> ProbePolicy;
  ```

  `policy(None)`（普通提问）= `{ popup: true, im: All, ask_received_hook: true, follow_active_channel: true }`，即今天的行为；`policy(Some(Popup))` = `{ true, Nothing, false, false }`；`policy(Some(Channel(id)))` = `{ false, Only(id), false, false }`。
- `daemon/runtime/mod.rs` `handle_submit` 不另开处理函数：结果等待、调用方断开取消、收尾都与普通提问共用同一份代码，只在排空闸门之后取一次 `let policy = probe::policy(probe::parse_probe_id(task.native_request_id.as_deref()));`，再让四处按策略取值：
  1. **弹窗**：`popup_enabled = policy.popup && development_popup_enabled(&config, development, has_display())`；弹窗不可用时沿用现有的 `surface_lost("popup", …)`。
  2. **IM**：`ImScope::Nothing` 跳过整个 IM 挂接（`im_attached = false`）；`Only(id)` 调用 `attach_im_channels(…, only = Some(id))`（§4.2）；`All` 传 `None`，保持原逻辑。
  3. **`ask-received` 用户 hook**：仅 `policy.ask_received_hook` 为真时触发。
  4. **活跃渠道**：两处 `finish_request_bookkeeping(state, &entry, &request_id, auto)` 改传 `auto && policy.follow_active_channel`，在 IM 里答探测不能改写用户的「活跃渠道」。
- 其余环节对探测天然无效，不需要守卫：登记表刷新与重复提问收敛都以 Agent 会话键为前提（探测没有 `agent_kind` / `agent_session_id`）；away 裁决 `judge_task` 对带 `native_request_id` 的任务恒为 `Deliver`（`is_agent_ask` 要求其为空），离开 / 静音模式不会拦截探测；`spawn_agent_resolve` 在 `caller_pid == 0` 时直接返回。
- `daemon/request.rs` `create_escalated`：先取 `let probe = parse_probe_id(task.native_request_id.as_deref());`（在 `task` 被逐字段 move 之前）；探测请求**不进回复历史**——传给 `Coordinator::new_ipc` 的 `record_history` 改为 `task.record_history && probe.is_none()`（**不**把 `task.record_history` 设为 false：那会让 `popup_kind()` 变成 `Stop`）；`request.setup_probe = probe.is_some()`。`quick_reply` 无需处理：`is_agent_ask` 要求 `native_request_id` 为空，探测天然为 `false`（弹窗与手机端都不出快捷回复菜单）。
- `models.rs`：`AskRequest` 新增 `setup_probe: bool`（serde default + 为假不序列化）；`AskRequest::new` 默认 `false`；补序列化往返单测（含「旧发送方缺字段」）。

### 4.2 渠道范围投递

`attach_im_channels` 新增参数 `only: Option<&str>`：`Some(id)` 时 `candidates` = 纯函数 `scoped_candidates(Some(id), &available)` 的结果（即 `available_im_channels(&config)` 中等于 `id` 的那一项；绕过活跃槽 / 自动激活 / watch 选择，且不经 `with_ios`）；`None` 保持原逻辑（常规提问调用点传 `None`）。渠道未启用或配置不全 → 无面可用 → 协调器按既有口径以 `EXIT_NO_CHANNEL`（3）收尾。daemon 配置快照由 config watcher 更新；按钮只在保存后出现，人工点击间隔远大于去抖，不另做同步。

### 4.3 「首次 Agent 提问」观察

- `ipc/mod.rs` `PendingRequestInfo` 新增 `agent_kind: Option<String>`、`agent_ask: bool`（serde default / 缺省不序列化）；`daemon/request.rs` 两处构造（约 939 / 949 行；确认条目恒 `None` / `false`）：提问条目 `agent_kind` = `runtime::entry_agent_kind(entry, None)` 的家族名（该函数先取 `resolved_agent` 里异步解析出的家族，再取 `show.agent_kind`；改 `pub(crate)` 后复用），`agent_ask` = `entry.request().quick_reply`（该字段即「Agent 自己的提问」，见 `models.rs` 注释 spec N10；Stop 确认、失败卡片、原生 Codex 提问、开发中继都为 `false`，探测同样为 `false`）。
- `spawn_agent_resolve` 把解析结果存入 `entry.resolved_agent` 之后补一次 `broadcast_tray_state`，让 MCP 路径（env 判不出家族）的订阅者也能拿到家族。
- 向导侧 `onboarding/probe.rs` 的「Agent 观察」：自带一条 `TraySubscribe` 连接循环（沿用 `start_status_subscription` 的 connect / 写 `ClientMsg::TraySubscribe` / 读 `ServerMsg::TrayState` 写法；daemon 未运行或断连时每 2 秒重连一次；该订阅不保活 daemon；**不**依赖 Host 自身的托盘订阅，因为 `menuBarIcon = off` 时它不订阅）。以首帧为基线，之后出现**新的**待答请求且 `agent_ask && agent_kind.is_some()` 即成功，结果带来源 Agent；5 分钟无响应 → 超时并给排查清单（重启会话 / CLI 模式需在 PATH / Codex 信任 Hook / Cursor 项目需在主目录下 / MCP 需在 Agent 内启用）。

### 4.4 向导侧命令与按钮

- 命令：`setup_probe_start(kind: "popup" | "channel" | "agent", channel?: string) -> probeId`（`spawn` 后台任务）、`setup_probe_cancel(probeId)`；结果经事件 `setup-probe-result { probeId, kind, channel?, outcome: "ok" | "timeout" | "cancelled" | "failed", detail?, agentKind? }` 推回。同类探测同时最多一个；窗口卸载时前端调用取消（尽力而为，兜底靠自然超时）。
- 提问构造：`popup` 与 `channel` 探测经 `client::internal_ask(task, &accepted)`（`accepted` 是接收 `Accepted` 里请求 id 的 `Mutex<Option<String>>` 槽位；daemon 未运行时它会先拉起 daemon；丢弃其 future 即取消请求），`TaskRequest` 字面量参照 `agents::stop::build_task`，任务为单题单选项「收到」的 `select_only` + `single` 提问，`record_history` 保持 `true`，`source` = 「AskHuman 设置向导」，`caller_pid = 0`，`native_request_id = setup-probe:<target>:<uuid>`，文案走 i18n；`tokio::time::timeout(120s)`，超时丢弃 future 即取消请求（IM 卡随之收尾为「调用方取消」）。判定：退出码 0 → `ok`；`EXIT_NO_CHANNEL` → `failed`（弹窗被关闭 / 无图形环境 / 渠道未就绪）；其它非零 → `cancelled`。
- 按钮：完成页「本机自测」（`popup`）、「在 Agent 里试一次」（`agent`）；渠道详情「测试往返」（保存后出现，`channel`，超时给该渠道的排查提示：凭据 / 可用范围 / 机器人是否在会话里）。三者的状态机均为 空闲 → 进行中（含取消）→ 成功 / 失败 / 超时，互不影响。

### 4.5 旧 daemon 降级

旧 daemon 不识别探测前缀 → 探测退化为一次普通提问（见 §0.2）；向导不做版本探测，也不阻止使用。daemon 在二进制换新后会自动重启为新版，降级只是过渡态。

### 4.6 P4 测试

Rust：`parse_probe_id`（合法 / 非法渠道 / 非探测前缀）；`policy` 对普通提问、`Popup`、`Channel` 三种输入的取值（普通提问必须与今天的行为逐项相同）；`create_escalated` 对探测不进历史、`popup_kind` 仍为 `Ask`、`quick_reply` 为假、`setup_probe` 为真，对普通提问这三项不变；`scoped_candidates(only, &available)`（`Some(id)` 命中时只返回该渠道、`id` 不在可用列表时为空、不含 iOS）；`PendingRequestInfo` 新字段往返与旧负载兼容；`agent_ask` 对探测 / 普通提问 / Stop / failure card 的取值。
vitest：三个按钮的状态机（成功 / 失败 / 超时 / 取消）与「失败不影响完成」。
手动：Dev 实例 `dev_ask` 验证弹窗链路；渠道往返与「在 Agent 里试一次」需用户用真实凭据 / 真实 Agent 配合。

## 5. 文档

- `README.md` / `README.zh-CN.md` / `packaging/npm/humaninloop/README.md`：Usage 把「运行 `AskHuman setup`」写成安装后的第一步，原第 2、3 节（手动配置 Agent / 渠道）降为补充说明（D24）。
- `docs/overview.md`：`App.vue` 路由列表加 `setup`；前端 views 列表加 `SetupView.vue` / `views/setup/*`；Rust 目录树加 `onboarding/`（说明 `ui-state.json` 的 `onboarding` 字段）与 `cli/setup_cmd.rs`；CLI 小节加 `setup`；GUI Host 窗口类型列表 / 路由加 `setup`；第 478 行登录项描述补「always 的登录项在用户做过选择后才写入」。主概览的全局不变量不变，不另改。
- `docs/specs/menu-bar-tray.md`：D4 / D12 处加一句指向 `onboarding.md` D13（登录项门控）的说明。
- `docs/specs/cli-config.md`：命令总览加一节 `AskHuman setup [--text]`，指向本 spec。
- `docs/specs/onboarding.md`：评审后把反馈追加到 §8（标注「反馈意见」）。
- `docs/PROGRESS.md`：无待记录项；若评审后决定推迟某一期，则把该期作为未完成项记入。

## 6. 验证

**工作位置（已定，评审第 5 轮）**：在主 worktree 里做，不创建独立 Dev Worktree。因此每期末的安装验证直接 `./scripts/install.sh` 装进日常环境，弹窗链路用已安装的 `AskHuman` 直接验证；向导的写盘行为（Agent 集成、登录项、`ui-state.json`）一律只在「临时 HOME + `ASKHUMAN_HOME`」里验证，不碰真实 HOME 里的 Agent 集成。主 worktree 里还有进行中的 iOS 与 message-annotations 改动，只改本计划列出的文件，提交（仅在用户要求时）只选本功能的文件。

**每期**：长构建 / 测试前先 `node scripts/machine-health.mjs`（退出码 2 → 经 AskHuman 告知用户并后台分片轮询）；`cargo test`、`cargo clippy`、`npm test`、`npm run build`；P1 起每期末 `./scripts/install.sh` 安装验证。

**隔离**：Dev 实例拒绝改日常 Agent 集成，所以 Agent 步骤只在「临时 HOME + `ASKHUMAN_HOME`」里验证；弹窗链路用 Dev 实例的 `dev_ask`；GUI 三页截图核对；文字版用 pty 脚本驱动；不拉起真实 Agent。

## 7. 实施顺序与风险

1. **P1**：`uistate` → `onboarding/state` → 登录项门控 + `restart_host` 守卫 + `config_cmd` 钩子 → `agents` / `detect` / `apply` / `plan` / `env`（含 `agent_launch` 登录 PATH 包装、`install_check::collect_with_path`、`history::has_agent_origin`）→ 单测。
2. **P2**：`channel_cmd` 的 `wizard_for_setup` / `detect` / `test` 开放 → `text.rs` → `setup_cmd` + 分发 + `dev_instance` / `NON_ASK_SUBCOMMANDS` / `help` → 欢迎语 → pty 手动验证。
3. **P3**：窗口路由与 `WindowKind::Setup` → `onboarding/commands` + `invoke` 分组 → 前端骨架（`createCore` 导出、`ChannelsTab` 深链 id、`useSetupStatus`）→ 三页 → 设置页入口 → 托盘项 → 弹窗提示条替换（含 mock / 测试同步）→ i18n → 截图验证。
4. **P4**：`AskRequest.setup_probe` + `PendingRequestInfo` 字段 → `handle_submit` 的探测守卫与 `create_escalated` 不进历史 / `attach_im_channels(only)` / agent 解析后广播 → `probe.rs` 与向导命令 → 三个按钮。
5. **文档**（§5）与最终复核：`docs/overview.md` 与相关 spec 是否因本次改动失准。

风险：popup 相关文件（`usePopupCore.ts`、`PopupFooter.vue`、`popup.css`）与进行中的 message-annotations 可能有并发改动，P3 开工前先 `git status` / `git log` 复查，避免覆盖；向导与设置窗口各持一份 `config` 并整份保存（与既有「设置窗口 + CLI `config set`」同样的最后写入者胜出语义），两者同时打开且都改配置时可能互相覆盖——向导只在「应用风格 / 保存渠道 / 完成」三个时点 `persist()`，窗口获得焦点时若无未保存编辑则重新 `getSettings()` 刷新。

## 8. 验收对照

| 验收 | 落点 |
|---|---|
| 1 全新安装走完 3 页、逐家回读 | §1.5、§3.3–3.5 |
| 2 文字版与非 TTY | §2.1、§2.3 |
| 3 / 5 Agent 域提示（横幅 / 托盘 / 欢迎语）与老用户 | §1.2、§2.2、§3.1、§3.6 |
| 4 渠道域提示条 | §1.2、§3.1、§3.7 |
| 6 重跑预填 | §3.3–3.5、§2.3 |
| 7 协作风格 / 待信任 / `needs_update` | §1.4、§1.5、§3.3 |
| 8 登录项门控 | §1.3、§3.5、§3.6 |
| 9 环境红卡 | §1.6、§3.3 |
| 10 可选验证 | §4 |
| 11 渠道页同源 | §3.4 |
| 12 Dev 实例 | T8、§1.3、§1.6、§3.3、§3.6 |
| 13 i18n | §2.4、§3.8 |
| 14 既有行为不变 | §0.2、§0.3 |
| 15 README / overview | §5 |
