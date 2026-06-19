# 计划：弹窗启动延迟 —— 次轮 daemon/CLI 优化（方案3 + 方案4 + 方案5）

> 状态：**已实现并量化（2026-06）**。同机 compare（mock IM，冷热双跑）相对（首轮后）基线两闸均 OK 无回归：
> COLD 端到端 p90 **-49%**（spawn→painted 1188→600ms）；WARM 端到端 -10%。CLI `detect` COLD -97% / WARM -96%。

需求与方法论见 `docs/specs/popup-launch-performance.md`（§4 方案3/4、§5 方案5、§6.1）。首轮（前端侧）见 `docs/plans/popup-launch-low-risk-optimization.md`。本计划覆盖 daemon/CLI 侧三项：消掉 COLD 的「IM 建连阻塞弹窗」、省掉无 IM 时的钥匙串读取、把 detect 的 `ps` 游走移出 CLI 关键路径。

## 目标与范围

- **做**：方案3（daemon 提前 spawn 弹窗）、方案4（attach 省钥匙串）、方案5(b)（detect 移 daemon 异步 walk）。
- **不做（本轮）**：方案6 预热复用（架构级、远期）、方案8 延后 show/骨架屏、markdown-it 按需懒加载。

## 已确认决策（2026-06）

1. 方案3/4 取**默认实现**（见 spec §4）。
2. 方案5 取 **(b) daemon 侧探测**的完整形态：CLI 上送 `caller_pid` + env 家族/会话；daemon accept 后异步 walk 出家族/pid，经新 IPC 后推弹窗 badge（含 MCP `walk_any` 兜底）。

## 改动详解

### 方案3：daemon 提前 spawn 弹窗（`daemon/mod.rs::handle_submit`）
- 把 `spawn_gui_helper(...)` 提到写 `ServerMsg::Accepted` + `broadcast_tray_state` **之后**、`ensure_inbound_listeners` / `attach_im_channels` **之前**。
- token 在 `registry.create()` 即登记，`handle_gui` 用 token 关联 entry，故不存在「helper 先连上、entry 未注册」竞态。
- 结果：弹窗进程（WebView 初始化）与 daemon 的 IM 建连**并行**。COLD 的 ~464ms IM 串行建连不再挡弹窗。

### 方案4：attach 省钥匙串（`daemon/mod.rs`）
- 新增 `any_im_enabled(&AppConfig)`：只看非密钥的 `channels.*.enabled` 标志。
- `attach_im_channels` 与 `ensure_inbound_listeners` 都先 `any_im_enabled(&AppConfig::load_without_secrets())` 门控：无启用 IM → 直接返回，**完全跳过** `AppConfig::load()`（零钥匙串）；有则才 `load()`。

### 方案5(b)：detect 移 daemon 异步 walk
- **CLI（`cli/mod.rs`）**：`detect_caller_agent()` 改为只读 env（家族 + 会话，零 `ps`），返回 `(Option<kind>, Option<sid>)`；`TaskRequest` 设 `agent_pid: None`、新增 `caller_pid = std::process::id()`。
- **IPC（`ipc/mod.rs`）**：`TaskRequest` 增 `caller_pid: u32`（serde default 0=旧 CLI/跳过）；`ServerMsg` 增 `AgentResolved { kind, pid }`。
- **daemon（`daemon/mod.rs` + `daemon/request.rs`）**：
  - `RequestEntry` 增 `resolved_agent: Arc<Mutex<Option<ResolvedAgent>>>`。
  - `handle_submit` 即时部分仍用 env 家族+会话刷新注册表（pid 此刻 None）；**spawn 弹窗后**调 `spawn_agent_resolve(...)`：独立 task 里 `spawn_blocking` 从 `caller_pid` walk（`walk_agent_pid`；env 判不出家族的 MCP 情形 `walk_any_agent` 兜底），拿到家族/pid 后补刷注册表 + 存入 `entry.resolved_agent` + 经 `entry.gui` 后推 `AgentResolved`。`caller_pid==0` 跳过。
  - `handle_gui` 下发 Show 后，若 `resolved_agent` 已就绪则随握手补发 `AgentResolved`（覆盖「解析早于连接」竞态）。
- **helper（`app/mod.rs` + `commands.rs`）**：收 `AgentResolved` → `set_pushed_agent` 缓存 + emit `agent-resolved`；新增 `popup_agent_resolved` 命令供挂载 pull 初值；`popup_agent_terminal` 改为接收 `pid` 参数（不再读 `AppState.agent_pid`）。
- **前端（`PopupView.vue` + `lib/ipc.ts` + `lib/types.ts`）**：`initAfterPaint` 里 pull `popupAgentResolved()` 初值 + `listen("agent-resolved")`；`applyAgentResolved(kind,pid)` 幂等地补家族 badge 文案、据 pid 调 `popupAgentTerminal(pid)` 升级「可点 ↗」。新增 `PushedAgent` 类型。

## 影响文件
- `src-tauri/src/ipc/mod.rs`（`TaskRequest.caller_pid`、`ServerMsg::AgentResolved`）
- `src-tauri/src/cli/mod.rs`（`detect_caller_agent` 只读 env、`agent_pid: None` + `caller_pid`）
- `src-tauri/src/daemon/mod.rs`（方案3 重排、方案4 `any_im_enabled` 门控、方案5 `spawn_agent_resolve` + handle_gui 补发）
- `src-tauri/src/daemon/request.rs`（`ResolvedAgent` + `RequestEntry.resolved_agent`）
- `src-tauri/src/app/mod.rs`（helper 处理 `AgentResolved` + 注册 `popup_agent_resolved`）
- `src-tauri/src/commands.rs`（`PushedAgent` 缓存/命令、`popup_agent_terminal(pid)`）
- `src/lib/ipc.ts`、`src/lib/types.ts`、`src/views/PopupView.vue`（前端 badge 后到补全）

## 风险与兼容
- 协议：`caller_pid` / `AgentResolved` 均为新增；旧 CLI 不带 `caller_pid`（0→daemon 跳过 walk），旧前端忽略未知事件/字段。
- agent 注册表：MCP 情形的 pid 刷新从「CLI 同步」变「daemon 异步」（延迟 ~数十 ms，best-effort，无害）；env 有会话的常规情形即时刷新不变，仅 pid 后补。
- badge：家族（env 探到时）随 `popup_init` 即显；pid/终端经 `agent-resolved` 略晚到——与原本「终端探测渲染后才补」体验一致。MCP（env 判不出家族）的 badge 整体略晚。
- 并发/竞态：`resolved_agent` 存 entry + handle_gui 握手补发，覆盖「walk 早于 helper 连接」；事件 + pull 初值覆盖「事件早于前端监听」。`std::sync::Mutex` 仅瞬时持有、不跨 await。

## 验证
1. `./scripts/install.sh` 编译安装（`vue-tsc` + cargo release 均通过）。
2. `node scripts/perf-popup.mjs`（隔离 daemon + mock IM + 冷热双跑，对比基线）：两闸 OK；COLD e2e -49%、`daemon recv→spawned` 466→1ms、`detect` -97%；WARM `detect` -96%。
3. 人工 sanity：本仓 `AskHuman` 弹窗顶栏正确显示 `cursor` badge 且解析终端后可点 ↗（端到端验证 caller_pid→walk→AgentResolved→badge 全链路）。
4. 确认有效后 `node scripts/perf-popup.mjs --update-baseline` 把基线刷新为优化后新数（锁定增益，后续以此防回归）。

## 不在本轮（留待后续，见 spec §6）
方案6 预热复用（大头、架构级、远期）、方案8 延后 show/骨架屏、markdown-it 按需懒加载（仅 `isMarkdown` 时）。
