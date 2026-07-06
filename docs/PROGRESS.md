# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 待验收：Agent 插话（Interject）—— M1–M6 全量落地

spec `docs/specs/agent-interject.md`（D1–D9）、计划 `docs/plans/agent-interject.md`（M1–M6），均已
按用户评审定稿实现（详见 overview「Agent 插话」节），456 单测 + vue-tsc 通过、install.sh 已跑：
- **M1 daemon 核心**：`agents/interject.rs` 队列（覆盖/追加/撤回/三态 poll/持久化 interject.json）+
  IPC 扩展 + daemon 三态回帧与 Hold/composer 连接（非保活）+ 会话结束清理 + AgentsState 注入 pendingInterject。
- **M2 hook**：reporter PreToolUse 捎带 `interject_poll` 等一帧裁决（300ms 超时失败放行）+ 各家 deny JSON
  （`[USER INTERJECTION]` 协议文案）+ 安装产物 PreToolUse `timeout: 86400`（旧安装自动迁移 + 手动『更新』兜底）。
- **M3 GUI**：`WindowKind::Interject` 宿主路由 + `InterjectView.vue` composer（预填/覆盖提交/取消）+
  `client/composer.rs` 专属连接（断开=放行 hook）+ AgentsView「发送消息」入口/「插话」徽标/行内撤回。
- **M4 托盘**：`TrayState.agents` 摘要 → 「Agent 状态」子菜单（打开窗口 + 每 agent 发送消息/聚焦终端）。
- **M5 IM**：`/msg <编号> <内容>` 追加、`/msg <编号>` 回显、`/msg-clear` 撤回（别名 `/插话`、`/撤回`；
  grok/ended 拒绝；与 /status 同门控、进 /help）。
- **未做（待你验收）**：未真机端到端——需重启 daemon 用新二进制后：① AgentsView/托盘给某 working agent
  发消息，观察其下一次工具调用收到 `[USER INTERJECTION]`；② 打开 composer 不提交，确认 agent 工具调用挂起
  等待、取消后放行；③ IM 发 `/msg`；④ 已开生命周期的 agent hook 是否被自动迁移出 timeout=86400。

## 待办：Codex 生命周期 hook 信任哈希加固（「hook 新、哈希旧」窗口）

用户曾遇一次 Codex 弹「不信任 AskHuman hook」（时值 M2 迁移逻辑经其它任务的 install.sh 生效、
`migrate_outdated()` 给 PreToolUse 补 timeout=86400 的窗口期）。代码分析确认三个真实窗口
（当前盘上状态已核对一致，哈希与独立复算逐字节相同）：
1. `codex_install` 两步写（hooks.json → config.toml 信任）非原子且第二步失败**不回滚**；
2. config.toml 无锁「读-改-写」，与 Codex CLI 自身写入 / `mcp_config` 并发时后写者覆盖 `[hooks.state]`；
3. 新旧双二进制交错重装（GUI 宿主滞留旧版，见下方既有待办）可致 file/hash 版本错配。
另：信任键含数组下标（外部增删同事件条目即失效）；自愈仅在 daemon 启动时跑。
**候选修法**（用户定案：暂不做）：方案1 第二步失败回滚 hooks.json；方案2（推荐）daemon 周期 tick
顺带核对 Codex trust 一致性、不一致即幂等重装（秒级自愈，兼作竞态事后修复）。

## 待验收：守护进程「保活模式」（实验 Tab）

在「实验」Tab 加**分段控件**（与状态栏图标一致）选 daemon 生命周期：`activity`（默认＝当前行为：按需拉起、5min 空闲退出）/ `keepalive`（保活）。已全量落地：
- **保活 = 装 daemon 登录项（开机自启，`~/Library/LaunchAgents/*.daemon.plist` `RunAtLoad`、无 `KeepAlive`；Linux `daemon start`）+ 空闲循环跳过退出（每轮 `load_without_secrets` 读一次）+ 打开开关即立即启动 daemon（宿主 `apply_config` 换挡即 `ensure_running`，一次触发）**。
- **关闭 = 卸登录项（仅删 plist/.desktop 文件、不 bootout 以免强杀）+ 让 daemon 按原 5min 空闲策略自然退出（策略不改）**。
- 登录项同步（`sync_daemon`）由 daemon 自身在 `serve()` 启动 + `on_config_changed` 幂等执行（宿主只管「立即起」）。
- 提示文案：保活可让 IM 随时收消息，但持续占少量资源 + IM 通道；多设备同时用建议配不同 IM 机器人。
- 触点：`config.rs`(DaemonLifecycleMode + general.daemonLifecycle + test) / `daemon/mod.rs`(idle 循环跳过 + `sync_daemon_login_item` 于 startup/on_config_changed) / `integrations/login_item.rs`(daemon 变体，file-only，+2 test) / `app/gui_host.rs`(HostState.daemon_lifecycle 跟踪 + apply_config 换挡即 ensure_running) / `types.ts` / `SettingsView.vue` / i18n zh+en。Unix only。
- **未做（待你验收）**：未真机端到端——开保活看 daemon 是否立即起且不再空闲退出、`~/Library/LaunchAgents` 是否落 plist、关保活后 plist 消失且 daemon 自然退出。验收前需用新二进制重启 daemon。

## 待验收：弹窗头部显示「提问时间」（相对时间，满一天转绝对）

`install.sh` 通过、vue-tsc 通过。在弹窗头部「Message from …（含胶囊）」之后加一枚灰色小字时间：
- **锚点**：提问创建时刻（epoch ms）。daemon 建请求时 `RequestRegistry::create()` 记录，经
  `ShowPayload.created_at_ms` → `PopupInit.createdAtMs` 透传；冷/单进程弹窗取弹窗构造时刻兜底；
  非弹窗窗口置 0。预热弹窗领用时得到的即为提问真正到达时刻（非热进程 spawn 时刻）。
- **显示**：`<5s 刚刚 / <60s N 秒前 / <60min N 分钟前 / <24h N 小时前`，满 24h 改绝对
  `toLocaleString()`（跟随系统）。前端每秒 tick 走字；hover(title) 给精确绝对时间。
- **窄窗**：`.brand-time` 给远高于标题/胶囊的收缩权重（flex-shrink 100000）+ overflow 裁剪，
  空间不足时**最先**被压没。
- 触点：`ipc.ShowPayload` / `daemon/request.rs` / `app::AppState`（7 处构造）/ `commands.popup_init`
  / `types.ts` / `PopupView.vue` / i18n `popup.time.*`。
- **未做**：未真机看弹窗（需重启 daemon 用上新二进制）——待你验收。

## 待验收：/status 当前活动（一期 transcript 尾部 + 二期 hook 实时工具 + 会话层斜线修复）

一期 `docs/plans/im-status-activity.md`、二期 `docs/plans/im-status-realtime-hook.md` 均已全量落地，
372 单测通过、`install.sh` 通过：

**一期（transcript 尾部）**：
- `registry`：`AgentRecord.seq` 稳定数字编号（daemon 生命周期内单调不复用、`load()` 重排），snapshot 暴露。
- `title.rs`：`transcript_path`/`find_file_recursive` 提为 `pub(super)` 供复用。
- `agents/activity.rs`（新）：尾部读取（256KiB）+ 四家解析 + 「永远给最后一段助手文字、末尾是工具调用再附工具」
  规则 + 工具归一化（仅 读/写/运行命令，其余原名+参数前段）+ 500 字截断 + `Activity.at`（transcript mtime）。
- `autochannel`：`Command::Status(Option<u64>)`；全局行 `[编号] 类型 — 标题（项目）`；`status_detail_text`
  头部 + 空行 +「最近动态（相对时间）」分区标签 + 文字 + `▸` 工具行；i18n。
- `daemon`：`handle_inbound` 按 `sel` 分派全局/详情。

**二期（hook 实时当前工具）**：Pre/PostToolUse hook 经 `report.rs`（解析 stdin 判 pre/post + `classify_tool`
归一化）随 `AgentEvent.tool` 上报；`registry.set/clear_current_tool` 存 `AgentRecord.current_tool`
（`serde(skip)` 不落盘、`snapshot()` 注入 `currentTool`；回合/会话结束清除）；`status_detail_text` 融合
「实时工具 vs transcript 尾部工具」取较新者、相对时间随之取实际展示事件时间——解决 Cursor「工具跑完才落盘」
滞后、覆盖编译等 in-flight 场景。

**会话层斜线修复**：作答期（armed 卡片）收到斜线命令时会话层不再回引导（`answer_inbound_reply` 返回
`Option`），统一交 `handle_inbound`（已注册→命令输出、未注册→help），四家 11 处调用点同步；`helpCmdStatus`
拆两行。

- **未做**：未 live 端到端实测（需真机发 IM `/status`/`/status <编号>` + 触发一次工具看实时工具）——待你验收。
  注意：IM 处理与 hook 上报都跑在 daemon，验收前需**重启 daemon** 用上新二进制。

## 待验收：Grok 集成（仅 MCP）—— 实现已落地并 live 验证

grok skill「不被模型主动加载」已修复并 install 落盘：根因是 skill 懒加载 + 自指悖论（按「需要提问时加载」写，
模型在最需要时意识不到需要它）。处置：把 skill 从「AskHuman 提问技能」重定位为「**无条件必读的交互协议**」——
frontmatter `description` 第一句无条件要求「每 session 先读」+ 前置兜底事实「普通输出人类不可见」；skill 更名
`askhuman` → `interaction-protocol`（`paths.rs` 目录同步）；`grok_skill_body()` 改为复用 `mcp_reference()` +
末尾一段**通用**「联系人类降级阶梯」（MCP 优先→没列出先搜→仍够不到退回其它提问渠道如 CLI，绝不退化成普通输出；
刻意不写死 harness/工具名）。已实测排除两条路线：`when-to-use` 仅拼在 description 后无价值、hook 无法注入
always-on 上下文（三证）。文档 overview.md / 调研 §6.3/§6.4/§7.2 已更新，全量 350 单测通过，install.sh + 落盘
`~/.grok/skills/interaction-protocol/SKILL.md` 已核对。**未做**：未 live 实测两 harness 首触发（避免 grok 子进程
再次卡住 shell）——待你需要时可 `grok inspect` 看 skill 是否加载。

计划 `docs/plans/grok-integration.md`（Q1–Q5 + P1/P2/P3 定案）已按序全量实现：
- P1（MCP 集成）：`AgentTarget::Grok` + `paths` grok 路径族 + `mcp_config`（三超时键
  `startup_timeout_sec=30`/`tool_timeout_sec=86400`/`tool_timeouts={ask=86400}`，且比较容忍整值浮点，
  顺带修好 Codex 因 CLI 归一化 `30→30.0` 造成的「永远需更新」）+ `grok_skill.rs` 指令载体
  （`prompts::grok_skill_body`，P2 措辞：找人一律走 `ask` MCP 工具、其它 shell 不受限）+
  `agent_mode` 两态（None|Mcp，拒 Cli）+ CLI(`agents mode/show/install`)/`doctor`/前端卡片(types/i18n)。
- P2（生命周期）：`AgentKind::Grok` 全链路（`mod`/`detect` 优先判 Grok/`registry`/`title` 解析
  `summary.json` 与解包 `<user_query>`）+ `agent_lifecycle` 原生 hook（`~/.grok/hooks/askhuman-lifecycle.json`，
  7 事件 Nested）+ `report.rs` P1 去重（`running==Grok && intended!=Grok` 跳过 claude/cursor 兼容 hook）。

Live 验证（`grok inspect`）：`askhuman` skill 已加载、MCP Server `askhuman(stdio) config` 已加载、
grok 原生 hook 已加载；同时确认 Grok 确会兼容读取 `~/.claude/CLAUDE.md`（P2 场景，靠 skill 措辞压制）
与触发 `~/.claude` 兼容 hook（P1 场景，靠 reporter 去重）。全部单测通过、`install.sh` 通过。

## 待办：install.sh 换新后 daemon 与 GUI 宿主「换新不同步」→ 旧 GUI 重建旧路径产物

现象（本轮 grok skill 改名 `askhuman`→`interaction-protocol` 时踩到）：`install.sh` 换二进制后 daemon 会自动
drain+重启到新版（`ASKHUMAN_DAEMON_AUTORESTART`），但 **GUI 宿主（`--gui-host` 菜单栏 app）有独立的二进制
换新监视（`gui_host.rs::start_binary_watch`/`maybe_refresh_binary`，每 15s，且仅在「无打开窗口」时才换）**，
可长时间滞留旧二进制（实测滞留 6h+）。分裂期内 **旧 GUI 按旧代码的产物路径反复重建托管产物**：删掉
`~/.grok/skills/askhuman` 后，每逢 daemon 重连/配置事件它又按旧路径补回（内容为旧版 `name: askhuman`），
即便 daemon 已是新版。手动退出并重开 app（GUI 切到新二进制）后复现消失，重启 daemon 回归验证通过。

风险点：任何「产物落点/命名变更」的发布，在用户 GUI 未及时换新前都可能被旧 GUI 以旧路径重建，产生「新旧两份
并存」。待评估修法：install.sh/daemon 换新时主动通知 GUI 宿主换新（而非仅靠其自身 15s+无窗口门控）；或让 GUI
换新不被「有窗口」长期阻塞；或产物 reconcile 统一由单一新二进制来源执行。

## 待验收：Codex app-server 共享 pid 隔离（生命周期追踪）

已实现并 `install.sh` 落盘，376 单测全绿。方案见 `docs/specs/agent-lifecycle-tracking.md`（D25/D26/D27 +
§8 源码+实测坐实 + 风险）与 `docs/plans/agent-lifecycle-tracking.md`（补丁节 P-1..P-4）；overview 生命周期章节已同步。

结论：新版 Codex TUI 经 UDS 连**长寿共享 app-server 守护**（reparent 到 PID 1、多 TUI 共用），hook/工具/MCP
都跑在 app-server 进程树内 → walk **永远拿不到 TUI pid**（源码：hook stdin/env 无 pid、握手 `ClientInfo` 无 pid、
UDS 不读对端凭证、落盘无 pid；实测：会话 rollout 由无 tty 的 app-server 持有、TUI 无 rollout/无 session_id）。
且 Codex **无** interrupt/关窗/SessionEnd hook（`Stop` 仅正常完成触发，Esc 打断走 `TurnAborted` 在 Stop 前返回）。

已落地（＝跟 Claude 无 pid / 被 scrub 时**同一路径**，不改状态机）：
- `agents/detect.rs`：`is_shared_app_server`（判据：命令行里 `codex` 令牌**紧邻下一个**为 `app-server` 子命令，
  覆盖 `node …/codex app-server …` 包装器；提示词里含 app-server 不误伤）；`walk_agent_pid(Codex)` 命中 app-server
  → `None`；`walk_any_agent` 跳过 app-server 节点。+3 单测。
- `agents/registry.rs`：状态机**无改动**——pid=None 天然跳过 D7 轮换与存活轮询，由 TTL + `working_backstop_sweep`
  治理；补模块注释。新增 `refresh_by_session_ids`（在途豁免 session_id 版）+1 单测。
- `daemon/request.rs`：`RequestEntry` 加 `agent_session_id` + `create` 填入 + `in_flight_agent_session_ids()`。
- `daemon/mod.rs`：tick 里 `refresh_by_pids` **并列** `refresh_by_session_ids`（无 pid 的 Codex/Claude-scrub
  等人回答>30min 也不掉空闲，与 Claude-有pid 完全一致）。

**未做（待你验收）**：未真机端到端——需重启 daemon 用上新二进制，对一个 app-server 模式的 Codex 会话触发一次
工具/提问，核对 `~/.askhuman/agents.json` 里该会话 `pid=null`、状态随 `Stop`→空闲与兜底超时正确流转、不再并发轮换误杀。

## 弹窗启动延迟性能优化（埋点 + harness + 基线 + 首轮 + 次轮 + 方案6 已落地；性能已暂停 → 远期余方案8/markdown-it）

文档：`docs/specs/popup-launch-performance.md`（调用链、等待点、优化方案、度量方法论 §7）。
harness 计划：`docs/plans/perf-harness-deterministic-mock-im.md`。
优化计划：`docs/plans/popup-launch-low-risk-optimization.md`（首轮 1/2/7）、`docs/plans/popup-launch-daemon-optimization.md`（次轮 3/4/5）。

**已完成：埋点 + 确定性 harness**（`ASKHUMAN_PERF` 门控默认关；`scripts/perf-popup.mjs` 无脑单命令：隔离 daemon
+ `ASKHUMAN_NO_KEYCHAIN=1` + 全 4 渠道 mock IM（`perf-mock-im.mjs`，建连/发送各注入 ~150ms 探针）+ 冷热双跑
+ 端到端 p90 ±20% 回归闸 + 锁屏/息屏守卫；基线 `docs/perf/baseline.json` 含 cold/warm）。

**已完成：首轮（方案1/2/7 + 支撑 S）** —— 前端侧：main.ts 不阻塞挂载、PopupView.onMounted 先取内容渲染、
Settings/History/Agents 异步组件、popup_init 作弹窗唯一非钥匙串配置源（弹窗路径零 `get_settings()`）；
附带 HistoryView 改用 `history_init.lang`，main.ts 自此零 IPC。

**已完成：次轮（方案3/4/5）** —— daemon/CLI 侧：
- 方案3 daemon 提前 spawn 弹窗（移到 Accepted 后、attach/inbound 前）→ WebView 初始化与 IM 建连并行。
- 方案4 attach/inbound 用 `any_im_enabled`(`load_without_secrets`) 门控，无启用 IM 时跳过 `AppConfig::load()`（零钥匙串）。
- 方案5(b) detect 移 daemon 异步：CLI 只读 env 家族/会话 + 上送 `caller_pid`；daemon spawn 弹窗后独立 task 从
  caller_pid walk 出家族/pid（MCP `walk_any` 兜底），经新 `ServerMsg::AgentResolved` 后推弹窗 badge（缓存 + 事件
  + 握手补发覆盖竞态）。badge 端到端验证通过（本仓 AskHuman 弹窗显 cursor 且可点 ↗）。

**当前基线**（`docs/perf/baseline.json`，次轮后 `--update-baseline` 刷新，屏幕解锁+唤醒+勿遮挡下采）：
- COLD 端到端 p90 ≈ **578ms**（首轮后为 ~1188）：方案3 让 `daemon recv→spawned` 466→1ms，~467ms IM 建连现与弹窗并行、不再进端到端。
- WARM 端到端 p90 ≈ **520ms**（首轮后 ~583）：大头仍是 `GUI total show→painted` ≈496（window visible ~250 + page boot ~435），即 WebView/页面加载固有冷成本。
- CLI `detect` 两路均 ~1ms（方案5：原 COLD ~39 / WARM ~27ms 的 ps 游走已离开 CLI）。

**余下（性能已暂停，远期）**：方案8 延后 show/骨架屏（改观感不减时长，热路径已并入方案6）、markdown-it 仅 `isMarkdown`
时按需懒加载（见 spec §4/§6）。

**已完成：方案6 弹窗预热（进程池）** —— daemon 预热 1 个 `--popup --warm` helper 隐藏待命，`dispatch_popup` 领用喂
`Show` 直接上屏、用后后台重建；默认开可关、非实验；并发第 2+/无显示/未就绪/drain 透明回退冷 spawn；热连接非保活、
idle/换新 `recycle_warm` 重补。关键修正：隐藏窗（ordered-out）rAF 不回调 → 改「领用时 `nextTick` 等正文进 DOM 后直接
后端 `popup_show_window` 上屏」（不依赖 rAF，息屏/锁屏也上屏）。macOS：待命期 helper 设 `Accessory`（不占 Dock/Cmd-Tab），
领用切 `Regular` 并**补设内置图标**（否则 Dock 显通用命令行图标）。三档基线（`docs/perf/baseline.json`）：**hot e2e p90 ≈161ms
vs warm 505（-68%）**、`show→painted` 476→135（-72%），cold/warm 无回归。视觉（无闪现/主题/回退）+ Dock 图标人眼确认 OK。
详见 `docs/specs/popup-prewarm.md`、`docs/plans/popup-prewarm.md`。

**待办**：headless 预热仅 Linux 可验（mac N/A）。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
