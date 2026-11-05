# 计划：特殊回复（手机选待办、离开模式、本会话不再使用、快捷回复）

> 需求：`docs/specs/special-replies.md`（决策编号 D1–D40、默认做法 N1–N10 均指其 §3）。状态：2026-11-05 用户批准设计并授权「按顺序全部实现」；实现在分支 `feat/special-replies`（worktree `HumanInLoop-special-replies`，Dev Instance）上进行，进度与差异见文末「实施记录」。下文的文件名与步骤是起草时对代码现状的判断，**实施时以代码为准，并在文末「实施记录」补记差异**。
> 分支 / 工作方式：见「开工前定下的事」。每一期单独验收、单独提交（D1），期与期之间互不阻塞。

## 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 需求、计划、界面稿与浏览器原型入库（本次已做，只改文档与 `src/prototype/`） | `vue-tsc --noEmit` |
| P1 | iOS 待办：手机在提问里选项目待办、What's next 看完整队列 | `cargo test`（`ios::`）、`swift test`、模拟器 UI 测试、真实 daemon 端到端 |
| P2 | 离开模式 / 本会话不再使用：Daemon 模式引擎、`--escalate`、引导语、弹窗待发条、Mac 承载面、iOS 承载面（P2a–P2d，按顺序提交） | 见各子阶段与 §验证清单 |
| P3 | 快捷回复：预设数据、设置卡、弹窗与手机菜单里的预设 | `vitest`、`cargo test`、模拟器 UI 测试 |
| P4（可选） | CLI `AskHuman away on / off / status` 与 IM `/away`、`/back` | `cargo test`、IM 命令测试 |
| P5 | 文档同步（需求 §8），`PROGRESS.md` 改写或删除对应条目 | 提交 |

## P1 细节（iOS 待办）

Mac 端本来就有现成的东西：`QuestionAnswer.todo_ids` / `todo_selections`（`models.rs`）、Coordinator 终态汇聚点的 `todos::ids_to_dequeue` 与 `apply_todo_deliveries`（所有渠道共用，附件快照随之送达）、`todo.list.get`（`ios/wire.rs`，最多 300 条）。缺的是：弹窗里「把待办文字并进回答」是前端拼的（`usePopupCore.ts::collectAnswers`），手机没有这一步。所以由 iOS 网关在 Mac 端按同一规则拼。

1. **线协议**（`ios/wire.rs`、Swift `AHProtocol`、`ios/Fixtures/wire-v1/messages.json`，两边测试必须同时通过）：`AnswerSubmit.todo_ids: Vec<String>`（`#[serde(default, skip_serializing_if = "Vec::is_empty")]`）；`Caps` 加一个能力位（暂名 `answer_todos`，`#[serde(default)]`）。老 Mac 没有这个位，手机就不显示「待办」卡，免得选了的待办被老 Mac 当成多余字段丢掉。都是可选字段，不升 iOS `PROTO`。`requests.rs::answer_hash` 把 `todo_ids` 算进去，同一条回答重发仍幂等。
2. **Mac 网关**（`ios/gateway/requests.rs::build_answers`）：
   - 项目取该请求的项目（`pending.view.project.key`）；每个 id 在 `todos::list(project)`（先 `todo_sync::service::before_read`，同 Stop hook）里反查；找不到就整条回答以 `invalid`（`todo gone`）拒收，手机重新取一次列表再发（与未知选项下标同一处理）。
   - 普通提问：文字并进**最后一题**——手写文字在前、每条待办一行「另外看一下这个待办任务：…」，空行分隔（与弹窗一致）；同一题的 `todo_ids` 与 `todo_selections`（附件快照）按弹窗的形状填好。仅选择模式的请求带 `todo_ids` 一律拒收（D28：该模式不显示待办卡）。
   - What's next：选中的待办一律按 id 上报，网关按弹窗对「刷新出来的待办」的做法处理：回答文字 = 待办原文（其后空行 + 手写补充），不放进 `selected_options`，`todo_ids` + `todo_selections` 照填。这样前 10 个名额之内与之外的待办在 Agent 看到的结果、出队与附件送达上完全一样，手机也不必把待办行对应到选项下标。`todo: true` 的选项在手机上被完整队列区替代，不再单独显示。
   - 合并句要有 Rust 侧的 i18n（对应前端 `popup.todos.mergeLine`，中英各一条，跟随 Mac 界面语言）。
   - 测试（`tests_requests.rs`）：合并顺序、仅选待办、附件快照、终态出队、未知 id、仅选择模式拒收、What's next 名额之外的待办、`answer_hash` 幂等。
3. **手机引擎**（`ios/Packages/AskHumanKit`，`AHCore` / `AHApp`）：`RequestTodoPicker`（`@MainActor @Observable`）：按请求的项目 id（`TodoProjectId.of(key:)`）取 `todo.list.get`、`selected: Set<String>`、`unavailable`（Mac 离线 / Mac 没有 `answer_todos` / 项目没有待办）、What's next 的 `expanded`；回答合成时带 `todoIds`。只选待办也能发送（`canSend` 的判断要把它算进去）。
4. **手机界面**（`ios/AskHuman`）：`AnswerScreen` 最后一题下面的折叠「待办 N」卡（`TodoPickCard`：与选项同构的多选行、附件数角标、右侧「已选 N」；仅选择模式与无待办时不出现）；What's next 页的待办区（前 5 条 + 「显示全部 N 条待办」原地展开，「结束本轮」始终在最后）；文案进本地化目录（先 `xcodebuild build`，再跑 `ios/Tools/make-catalog.py`，缺中文会失败；新增 Swift 文件后在 `ios/` 下 `xcodegen generate`）。
5. **测试**：`swift test`（选择逻辑、回答合成）；Mock 后端的 UI 测试（en + zh-Hans）：折叠 / 展开、多选、仅选待办发送、What's next 展开、选择模式不显示、Mac 太旧不显示；真实 daemon 端到端（`AHE2ETests/TodoE2ETests` 加一条）：一次性 Mac 上有待办的项目挂一条提问，手机引擎发 `answer.submit` 带 `todoIds`，期望输出里有合并句、`todos.json` 里这条已出队、附件已随回答送达。

## P2 细节（离开模式 / 不再使用）

### P2a 引擎与 Agent 文字（Daemon、CLI、MCP、Hook）

- **`daemon/away.rs`（新）**：状态结构、`verdict()`、计数与防呆、持久化到 `~/.askhuman/state/` 下的状态文件（路径函数加进 `paths.rs`；临时文件 + 原子替换；Daemon 独占写入）；会话键用 `ask_dedup::session_key`；「仅此会话」的设置在会话结束时清除（订阅 `state.agents` 的会话结束事件，或在每次读到时按注册表惰性清理——开工时选更简单的那个）。单测：优先级矩阵（N1）、全局 / 会话覆盖、升级放行、防呆窗口（第 4 次放行、真实回答或过期清零）、无会话键、持久化往返与重启恢复、会话结束清除、`Stop` 确认 / 出错卡 / 权限审批不受影响。
- **插入点**：`daemon/runtime/mod.rs::handle_submit`，Agent 会话活动刷新之后、`registry.try_attach` 与重放缓存之前；自动回复走 `ServerMsg::Accepted` + `ServerMsg::Final`（退出码 0）；写历史与计数；不触发 `ask-received`；不进重放缓存（N4）。输出沿用 `[status]` 区块 / JSON `status` 字段，再加一个可选的自动回复种类字段。
- **`--escalate`**：`TaskRequest.escalate`（可选字段）、`cli/args.rs`、`cli/help.rs`（`--agent-help` 只写一行）、`mcp/ask.rs` 的 `ask` / `whats_next` 入参 `escalate`；升级放行的请求在 `ShowPayload` 与 iOS 摘要里带标记（可选字段，防呆放行用另一个值）。`cli/mod.rs::try_whats_next_auto` 不动（D7）。
- **引导语**：三段加解除句，中英各一份，放 `prompts.rs` / `i18n.rs`；有备注时接在后面（D11）。文本逐字采用需求 §4.1。
- **控制通路**：`ipc/mod.rs` 新增 `ClientMsg::Away(AwayOp)` / `ServerMsg::Away(AwayResult)`（与 `Ios`、`CursorDesktop`、`AgentMessage` 同构：一个变体承载所有操作，以后加操作不再抬协议号），操作含：读状态、开 / 关全局离开（范围、备注）、设 / 清会话模式。新增变体按 `docs/overview.md`「IPC 协议号」要 +1：5 若还没随发布版发出，可并入 5（那一节的历史写明了这个先例），已发出就改成 6；+1 时补历史一行并在 `daemon-graceful-drain.md` 记过渡期影响。`TaskRequest.escalate` 与弹窗回答里的 `directive` 是可选字段，不单独引起 +1。
- **Hook**：`agents/stop.rs::confirmation_suppression_reason` 加「会话已静音」的原因（读状态文件，命中时 `log_guard_audit` 写 `suppressed` 并按没接入 AskHuman 放行）；识别 Stop 确认用 `task.stop_confirmation`，**不用** `popup_kind()`（它把 `!record_history` 的内部卡片、出错卡也算 Stop）。`ask_question.rs`：`run_inner` 开头读状态文件，「不再使用」就什么都不写；离开时 `decide()` 认出自动回复标记，返回 `deny`，引导语放进 `permissionDecisionReason`。
- **解除「不再使用」**：`agents/interject.rs::InterjectQueue::append(会话, 「可以继续使用 AskHuman 了」, [], None)`；Grok 不通知（D15）。绝不走 Cursor Desktop 之类会开新一轮的「发送消息」通道。
- **历史**：条目加可选字段标明自动回复种类（`history.rs`，旧条目缺字段当作人答的）；历史窗口加「离开自动回复」筛选（做法同 `docs/plans/reply-history-session-filter.md`）；结束离开时的摘要入口打开它。
- **验证**：`cargo test`（`daemon::away`、`ipc`、`cli`、`mcp`、`agents::stop`、`ask_question`）、`cargo clippy -D warnings`、`cargo fmt --check`。

### P2b 弹窗（Vue）

- 页脚「快捷回复」按钮与菜单（本期菜单里只有两条指令，P3 才加预设与「管理快捷回复…」）：`QuickReplyMenu.vue`、待发条 `ArmedDirectiveBar.vue`（离开模式：范围下拉 + 备注；不再使用：范围固定）、升级提示条 `EscalatedNotice.vue`；`usePopupCore.ts` 管状态与键盘（⌘K / Ctrl+K、↑↓ 回车、Esc；菜单打开期间对键盘模态，做法同 `useTodoPopover.ts`）；`src/lib/shortcut.ts::shortcutConflict()` 加 ⌘K；`popup.css`；i18n（zh / en）。待发时页脚收成「取消 / 稍后回答 / 快捷回复 / 发送并…」，多题的上一个 / 下一个临时收起。What's next 弹窗菜单只有两条指令；升级弹窗菜单项是「结束离开模式」。Stop 确认卡、权限审批、出错卡里没有这个按钮（N10）。
- 提交载荷：`QuestionAnswer` 之外加可选的 `directive { kind, scope, note }`（`src/lib/types.ts` 与 `models.rs` 对应），`submit_popup` 在记录回答后调用引擎并追加引导语（N3）。`popup_init` 带上升级标记。
- 测试：`PopupView.quickReply.test.ts`（参照 `PopupView.todoPopover.test.ts` 的 Tauri mock）：菜单键盘与模态、待发条与主按钮文案、撤销、多题待发、What's next 菜单、升级条与「结束离开模式」、Stop / 确认卡里没有按钮、窄窗口；原型 `popup-reply-menu.html` 同步。

### P2c Mac 承载面（托盘、控制台、设置、桌面胶囊）

- **托盘**（三平台，`app/gui_host.rs`）：`scripts/gen_tray_icons.py` 加月亮徽标的图标状态（「待答」的「?」优先；Windows 另有紧凑图标）；菜单状态区下方的「开启离开模式」（一键，所有 Agent、无备注）/ 状态行 +「结束离开模式」；每个 Agent 子菜单底部两项勾选（`TrayAgentInfo` 加可选的 `mode`）。托盘里的「要求提问」那一次视为升级提问（N2）。
- **控制台**：全局离开的提示条、侧栏只给单独设置过的会话加小图标、详情页「AskHuman ⌄」下拉（Agent 快照加每会话的可选 `mode`）。
- **设置**：「通用」页「回复历史」下面加「离开模式」卡（桌面胶囊、回到电脑时提醒、推送到手机的等待时间）；配置字段 `general.away { capsule, welcomeBack, pushAfterSecs }`，默认 `true / true / 60`（`config.rs`，老配置缺字段按默认；`docs/overview-configuration.md` 同步）。「快捷回复」卡在 P3。
- **桌面胶囊与欢迎回来**（仅 macOS）：GUI Host 里新增一个浮窗：顶部居中（刘海正下方）、盖过普通窗口、所有桌面可见、不抢键盘焦点、可拖动、位置存 `state/`、点击或悬停展开成卡；首选 Tauri 窗口属性（置顶、全空间、不可聚焦、无边框透明），不够再退到 NSPanel。**先做一个小 spike 再铺开**：确认「不抢焦点、全空间、全屏应用上方是否可见」三件事，结果记进实施记录。「欢迎回来」由空闲采样触发：离开期间用 `idle::secs_since_last_input()` 低频采样，空闲满 5 分钟之后的第一次操作展开同一张卡（N7；门槛做成常量）。胶囊数据（开始时间、各 Agent 自动回复次数、等着的升级提问）来自引擎的状态推送。
- 原型 `mac-away.html`、`mac-away-board.html`、`console-away.html`、`settings-away.html` 与实现对照，不一致处以决策为准、同步改原型。
- **推送节奏**：离开期间 `ios-alert-timing` 的「Mac 空闲 ≥ D」里 D 取 `general.away.pushAfterSecs`（默认 60 秒）；读不到空闲时间的平台沿用「未知 → 立即推」（D14）。

### P2d iOS 承载面

- **线协议**（可选字段与新消息，不升 iOS `PROTO`）：`away.get` / `away.state` / `away.set` / `away.session.set`；`Caps` 加 `away` 能力位（老 Mac 没有它，手机隐藏相关入口或提示需要更新）；`RequestView` / `RequestSummary` 的可选 `escalated`；Agent 行的可选 `mode`；夹具与两边测试同步。
- **Mac 网关**：`ios/gateway` 收到 `away.*` 与回答里的 `directive` 后调同一个引擎接口；多台 Mac 各自独立，手机按在线的 Mac 逐台下发（D34、N9）。
- **手机**：`AwayStore`（每台 Mac 的状态汇总、持续时间 / 自动回复次数的显示）；常驻条 `AwayStrip` 由 `RootView` 用 `safeAreaInset(edge: .bottom)` 画在标签栏正上方（所有标签页；iOS 26+ `glassEffect(.regular.tint(橙色), in: .capsule)`，更早是材质 + 橙色，位置一致；「结束」一键）；Agents 页工具栏的月亮按钮与半屏开启页（说明、生效的 Mac、备注、「开启离开模式」；≥2 台 Mac 才显示 Mac 卡，不在线的置灰）；`AgentDetailScreen` 的「此会话的 AskHuman」行；`RequestScreen` 顶部的升级提示卡（带「结束离开模式」）；列表行「离开期间升级」「不再使用 AskHuman」标签；`AnswerScreen` 底栏的玻璃按钮 + 系统菜单（本期只有两条指令）与待发面板。
- 测试：Swift 包测试（状态汇总、范围计算）；UI 测试（iOS 26+ 一遍，降级样式用启动参数再跑一遍；en + zh-Hans）；`hello` 快照后常驻条在重连后仍正确。

## P3 细节（快捷回复）

- **数据**：`general.quickReplies: Option<Vec<String>>`（缺省 = 出厂 5 条，跟随界面语言；一旦编辑就存成用户的列表；最多 10 条，每条有长度上限）；出厂文字放 `prompts.rs` / `i18n.rs`。
- **设置**：`SettingsView.vue` 的「快捷回复」卡：可编辑列表、拖动排序、增删、恢复默认；「离开 / 不再使用」固定在菜单底部的说明。
- **弹窗**：菜单上半加预设、数字键直选（1–9，第 10 条是 0）、底部「管理快捷回复…」（打开设置 › 通用）。点预设直接发送：已作答的题保持原答案、未作答的题都填这一句；全部已作答时预设置灰（D18）；What's next 弹窗没有预设（D20）。
- **手机**：预设经现有 Mac 状态快照的可选字段下发（请求所属那台 Mac 的，N8）；底栏菜单里加预设，点一下就发，合成规则同弹窗；手机不编辑。
- 测试：`vitest`（菜单、合成、置灰、数字键）、`cargo test`（配置往返、上限）、iOS 包测试与 UI 测试（预设发送、What's next 页没有预设）；原型 `popup-reply-menu.html` 同步。

## P4 细节（可选：CLI 与 IM 命令）

- `AskHuman away on [备注] [--session]` / `off` / `status`（`cli/args.rs`、`cli/help.rs`、走 `ClientMsg::Away`）。在 Agent 进程里被调用时拒绝 `off`（用 `agents/detect.rs` 的最近 Agent 祖先判定，N6）。
- IM 命令 `/away [备注]`、`/back`（`docs/overview-im-commands.md`），回复一行当前状态。
- 测试：命令解析、拒绝逻辑、IM 命令分发。

## 验证清单

1. 每次大构建前 `node scripts/machine-health.mjs`；退出码 2 时先通过 AskHuman 告诉用户、等腾出内存，后台跑并短间隔轮询。
2. 每期结束：`./scripts/install.sh`，之后的提问都走新装的 `AskHuman`。
3. **真实 Agent 端到端**（P2）：Claude Code、Codex、Cursor 各一轮：开离开 → 提问与 whats-next 每次都被自动回复 → 带 `--escalate` 的提问像没离开一样送达 → 手动结束；「不再使用」→ 即时提示、Stop 确认静音、权限与出错卡仍在 → 解除后收到一句插话；离开时 Stop / 权限 / 出错卡照常；防呆第 4 次放行。
4. **真实弹窗里手动按一遍**：⌘K 与菜单键盘、待发条、预设、升级条、`Enter` / `Esc` 在 Stop 与权限卡里不受影响。
5. **平台**：macOS 全套（含胶囊与欢迎回来）；Windows / Linux 验弹窗、托盘月亮、控制台（桌面胶囊与欢迎回来不在范围）。
6. **iOS**：`swift test`；模拟器 `xcodebuild test`（en + zh-Hans；iOS 26+ 与降级样式）；真机一轮（iPhone）；iOS 17–18 上核一遍常驻条的降级样式。**TestFlight 不出**，除非用户要。
7. `cargo test`（相关模块）、`cargo clippy -D warnings`、`cargo fmt --check`、`npx vitest run`、`npx vue-tsc --noEmit`。

## 提交（Conventional Commits，英文、祈使句）

下面是预计的拆分，每期至少一条；`docs` / `test` / `refactor` 不进发布说明。

1. `docs(prototype): add away-mode and quick-reply UI prototypes`（P0，本次）
2. `docs(special-replies): add design spec and plan for iOS todo picking, away mode and quick replies`（P0，本次）
3. `feat(ios): pick project todos when answering from the phone`（P1，Mac 网关与手机引擎 / 界面可拆成两条，Mac 一条带 `Release-Note: skip`）
4. `feat(daemon): add away and mute modes with escalate-to-reach`（P2a）
5. `feat(popup): add a quick-reply menu with away and mute directives`（P2b）
6. `feat(tray,console,settings): show and control away mode on the desktop`（P2c）
7. `feat(ios): add away mode strip, start sheet and per-session control`（P2d）
8. `feat(popup,ios,settings): add editable quick replies`（P3）
9. `feat(cli): add away on / off / status and IM /away /back`（P4，可选）
10. `docs(special-replies): sync overview, specs and wiki with the shipped behavior`（P5）

## 风险

1. **把出错卡当 Stop 静音掉**：`popup_kind()` 把内部卡片也归为 `Stop`，识别 Stop 确认必须用 `task.stop_confirmation`；P2a 的单测钉住，装机后在真实出错卡上再验一次。
2. **Hook 读状态文件的并发**：写入用临时文件 + 原子替换，读到的永远是完整版本；Hook 读不到文件或解析失败一律当作正常模式（宁可多问一次，不静默吞掉）。
3. **桌面胶囊的窗口行为**：不抢焦点、全空间、全屏应用上方是否可见、多显示器位置记忆；P2c 先做 spike，不行就退到 NSPanel，再不行回来找用户（需求已定「默认顶部居中」，窗口类型是实现细节，但可见性不达标要说）。
4. **协议号**：新增 `ClientMsg::Away` / `ServerMsg::Away` 变体要 +1（或并入尚未发布的 5）；升级过渡期里新旧 Daemon 互相让位，按 `daemon-graceful-drain.md` 记影响。
5. **老 Mac / 老手机**：靠 `Caps` 能力位兜底（`answer_todos`、`away`）；P1 / P2d 各有一条「老 Mac 不显示入口」的测试。
6. **iOS 常驻条与系统标签栏**：iOS 26 的浮动标签栏、键盘、动态字体、横屏 / iPad 下 `safeAreaInset` 的位置；降级样式在 iOS 17–18 上要真的看一眼（模拟器里的降级图是套样式渲染的）。
7. **欢迎回来误触发**：键鼠抖动器会把空闲清零（`ios-alert-timing.md` 已知局限同源）；它只是提醒，不自动结束，影响有限。
8. **多台 Mac 部分失败**：手机逐台下发，某台失败要在开启页明确显示哪一台没生效（不静默）。
9. **Agent 不遵守引导语**（不认识 `--escalate`、不理会「不再使用」）：防呆（D9）兜底一部分；引导语的措辞已按用户意见精简，不再加细则（D16）。
10. **机器很忙**：全量编译与模拟器测试可能要十几分钟；开发中只跑定向测试，每期结束跑一遍完整的。

## 开工前定下的事

用户在开工时说「你自己按顺序全部实现完成，我要去上班了，不能回答你的问题了」，所以下面几条由实施者按默认值定下（没有问用户），用户回来后可整条否决：

- **工作方式**：worktree + Dev Instance——`/Users/wutian/Developer/HumanInLoop-special-replies`，分支 `feat/special-replies`（自本地 `main` 的 `2adccc03` 起）；`AskHuman dev enable` 不挂渠道预设（没人能答）。主检出里别的 agent 的未提交改动（iOS 界面一批、`Localizable.xcstrings`、`make-catalog.py`）不碰；日后合回 `main` 时 `Localizable.xcstrings` / `make-catalog.py` 可能冲突，那是预期的。
- **P2a 并入未发布的协议号 5**：最近的发布版 v0.16.0 的 `PROTOCOL_VERSION` 是 4，5 还没发出，所以 `ClientMsg::Away` / `ServerMsg::Away` 并入 5、不再 +1；`overview.md` 的历史段落在 P5 补一句。
- **iOS 模拟器**：为本任务新克隆一台专用设备，用完删除；不碰 `AskHuman iOS Dev` 与 `iPhone Duo`。
- **TestFlight**：不出；**不推送**远端；不做全局安装（`./scripts/install.sh` 在 worktree 里只装进 `.askhuman-dev/bin/`）。
- **验证时的投递**：`node scripts/dev.mjs ask --delivery local`，避免走远程中继把验证用的提问推到用户手机上。
- 需求 §3.10 的默认做法 N1–N10 按原文采用。

## 实施记录

### P1 iOS 待办（已完成）

与上文设计一致，差异与补充：

- **线协议**：`AnswerSubmit.todo_ids`、`Caps.answer_todos` 如设计；`answer_hash` 只在 `todoIds` 非空时才把它算进去，所以不带待办的回答哈希与旧版逐字相同（不会让旧版发件箱里的重发被判成冲突）。网关用 `invalid` + 原因文字拒收：`this question accepts only the listed options`（仅选择模式）、`too many todos`、`duplicate todo`、`choose one next task`（What's next 一次只收一个待办）、`todo gone`（常量 `wire::REJECT_TODO_GONE`）。手机把 `invalid` 回执显示为 `RequestState.failed(reason:)`，其中 `todo gone` 有单独的提示与「重新选择」按钮（重新取列表，草稿保留）。
- **Mac 网关**：`Host` trait 加 `todo_picks(project_key, ids)`（先 `todo_sync::service::before_read` 再 `todos::list`）与 `todo_merge_line(text)`（i18n 键 `todo.mergeLine`）；合并规则在纯函数 `merge_todo_picks`：普通提问把「手写文字 + 每条待办一句合并句」并进最后一题（空行分隔），What's next 把待办原文放在回答最前、手写文字在后，都填 `todo_ids` / `todo_selections`（附件快照）。
- **手机引擎**：`RequestTodoPicker`（按请求缓存在 `AppStore.todoPickers`，请求结束时清理）；`AnswerComposer` 持有 `todoIds`，「已作答」与「有内容」都把待办算进去（只选待办也能发送），What's next 的待办与选项互斥且最多选一个，草稿 `Draft.todoIds` 随发件箱落盘、重启后保留。取列表失败时 What's next 回落成问题自带的 `todo: true` 选项；Mac 离线期间队列保留在屏幕上（草稿不丢），重连后重新读；刷新保持 `.loaded` 不闪烁；折叠时已选但排在第 5 条之后的行仍然可见。
- **老 Mac**：`caps.answerTodos == false` 时不显示待办卡，What's next 保持原样，`AppStore.send` 一律发 `todoIds: []`。Mock 后端的默认 `demo` 场景保持 `answerTodos: false`，已有 UI 测试不受影响；新场景 `todoPick`（`-AHScenario todoPick`）带七条待办（第 4 条带一个附件）。
- **界面**：`TodoPickCard`（折叠的「待办 N」卡，`checklist` 图标、数量、「已选 N」、脚注）与 `TodoQueueRows`（What's next 的单选行 + 「显示全部 N 条待办」）在新文件 `Views/Request/TodoPickViews.swift`；`OptionRow` 改成包内可见以便复用，附件数用回形针小标签。
- **测试**：Rust `ios::` 282 条（含 8 条新网关用例与夹具往返）；Swift 包 `AHAppTests` / `AHCoreTests` / `AHProtocolTests` / `AHStoreTests` 全绿（新增 11 条 `AnswerComposer`、25 条 `RequestTodoPicker`、3 条 `SessionTests`）；`AHE2ETests/TodoE2ETests` 加四条（合并 + 出队 + 附件、What's next 名额之外按 id、已不存在的待办拒收且请求保留、仅选择模式拒收）；UI 测试 `AskHumanTodoPickUITests`（en + zh-Hans）。具体结果见下面各条验证记录。

### P2a 引擎与 Agent 文字（已完成）

与上文设计一致，差异与补充：

- **模块**：`daemon/away.rs`（纯判定 `judge` + `AwayEngine`：状态、持久化到 `state/away.json`（临时文件 + 原子替换）、计数；hook 用 `session_muted` 直接读文件，读不到或解析失败一律当作正常模式）；`daemon/runtime/away_ops.rs`（自动回复、`AwayOp` 的应用、答案里的 directive、变化后的广播）。
- **谁被拦截**：`is_agent_ask` ——Ask 类弹窗、不是开发中继、没有 `native_request_id`。Stop 确认、出错 / 复查卡、Codex 原生问题、开发中继既不被拦截，也不显示快捷回复（`AskRequest.quick_reply = false`，N10；该字段只在 false 时序列化，所以不认识它的旧发送方仍有菜单）。
- **会话键**：复用 `ask_dedup::session_key`（`sid:<Agent 会话 id>`，没有就 `mcp:<实例 id>`；两者都没有的调用方只受全局设置影响）。优先级：会话覆盖 > 全局；会话里 mute > away > normal；`normal` 是「全局离开时，这个会话照常问我」（N1）。
- **防呆**：同一会话 10 分钟内连续 3 次不带 `--escalate` 的提问被自动回复，第 4 次放行（`escalated: repeated`），放行后计数清零。托盘「要求提问」（P2c 接 UI）发的是固定文案的插话（`QUICK_ASK_TEXT`），daemon 在追加插话时认出它，给该会话开 30 分钟的一次性放行（`escalated: requested`，N2）。
- **自动回复的形态**：`Accepted` + `Final`（退出码 0）；文本模式是 `[status]` 区块，JSON 模式是 `{"action":"auto_reply","channel":"away|mute","status":…}`；提问 / whats-next / 不再使用各有一段中英文案（`away.reply.ask|whatsNext|mute`），离开备注接在后面（`away.note`）。历史 channel 为 `away` / `mute`；不进重放缓存，不触发 `ask-received`。
- **`--escalate`**：`TaskRequest.escalate`（可选）、CLI 旗标、MCP `ask` / `whats_next` 的 `escalate` 入参；`AskRequest.escalated` 带原因（`asked` / `repeated` / `requested`）。**端到端时才发现**：`cli/mod.rs` 入口有一份「第一个参数允许的旗标」白名单，`--escalate` 不在里面，`AskHuman --escalate …` 会被当成未知选项；已改成常量 `ASK_LEADING_FLAGS`，并加了「白名单里每一项都被 `parse_ask` 接受」的测试。
- **弹窗回答里的 directive**：`ClientMsg::Answer.directive`（可选字段）暂存在 `RequestEntry` 上，回答被接受之后才应用（被拒收的回答不会改状态），引导语追加在回答后面；弹窗一侧是 `PopupSubmission.directive`，Tauri 命令 `away_end` 经 `client::away_op` 发 `AwayOp::End`。
- **解除与清理**：「不再使用」的解除只经 interject 队列（D15）；会话结束（SessionEnd）与 15 秒一次的清扫（`sweep`）清掉会话覆盖与计数。
- **Hook**：Stop 确认对静音会话放行（`suppression_reason_with_mute`，识别用 `task.stop_confirmation`）；Claude `AskUserQuestion` 接管（`ask_question.rs`）：「不再使用」时什么都不写；离开时 `decide()` 返回 `deny`，自动回复文字放进理由；带 directive 的回答把答案放进 `deny` 理由（`askQuestion.answered`）。
- **协议**：`ClientMsg::Away` / `ServerMsg::Away` 并入未发布的 5，没有再 +1；`Answer.directive`、`TaskRequest.escalate`、`AskRequest.escalated` / `quick_reply` 都是可选字段。
- **验证**：全量 `cargo test`（bin）2534 条通过（其中 `daemon::away` 约 35 条、`away_ops`、`ipc`、`cli`、`models`、`daemon::request` 的新用例）；`cargo fmt`、`cargo clippy --all-targets -D warnings` 通过（提交钩子也会跑）。**真实 daemon 端到端**（`./scripts/install.sh` 装的二进制复制到 `/tmp`，独立 `ASKHUMAN_HOME`，用 `CLAUDECODE` + `CLAUDE_CODE_SESSION_ID` 冒充 Claude 会话）：全局离开时的提问 / `--output json` / whats-next 都得到自动回复且计数落盘；会话静音（含 whats-next）；`normal` 覆盖照常送达；第 4 次放行；`--escalate` 送达，之后的普通提问仍被自动回复（升级不解除离开）；在真实弹窗里选「不再使用」后发送，会话变成静音，回答带静音引导语；历史里有 `away` / `mute` 条目。**没做**：真实 Claude / Codex / Cursor 各一轮（要先有真实 Agent 调用的授权）；Stop hook 与 `AskUserQuestion` 接管目前只有单测。

### P2b 弹窗（已完成）

与上文设计一致，差异与补充：

- **组件**：`QuickReplyMenu.vue`、`ArmedDirectiveBar.vue`、`EscalatedNotice.vue`，状态与键盘在新的组合式函数 `useQuickReply.ts`（接进 `usePopupCore.ts`）；`PopupFooter.vue` 的多题与单题页脚放了触发按钮（权限 / 确认卡的页脚没有；Stop 确认与出错卡走单题页脚，靠 `quick_reply = false` 隐藏）；样式是 `popup.css` 里的 `.qr-*`。
- **键盘**：⌘K / Ctrl+K 开关菜单，↑↓ 与回车选择，Esc 关闭并还焦点；菜单打开期间对键盘模态（回答区自己的 Enter / Esc、数字键、⌘T 都不穿透；先于待办浮层处理，菜单开着时待办浮层不响应）；`shortcut.ts` 把 ⌘K 列为保留键（自定义快捷键冲突检测）。
- **待发**：选了指令后进入 `armedDirective`；`canSubmit` 在待发时恒为真（纯选择题也能不选就发）；主按钮变成「发送并…」，多题页脚的上一个 / 下一个临时收起；× 撤销。离开模式的范围（所有 Agent / 仅此会话）与备注随 `submit_popup` 的 `directive` 发给 daemon，备注为空时省略。
- **升级提示条**：来自 `AskRequest.escalated`；菜单里「离开模式…」变成「结束离开模式」（Tauri 命令 `away_end`）。
- **英文按钮文案**：设计稿只有中文（「发送并开启离开模式」「发送并停用 AskHuman」）；英文取 “Send and go away” / “Send and turn off”——更长的版本在 560pt 宽的默认窗口里会把别的按钮挤成两行。
- **真窗口目视**（隔离 daemon 的真实弹窗，截图）发现并修了三处：提示条标题后多了一段空隙（CSS 的 `margin-right` 把冒号隔开了）、待发时页脚按钮折行（`.footer .btn { white-space: nowrap }`）、待发条的说明被一行截断（现在最多两行）。
- **验证**：`PopupView.quickReply.test.ts` 36 条（菜单键盘与模态、待发条、撤销、多题、严格选择、What's next、非 Agent 卡片没有按钮、升级条与 `away_end`、与查找 / 语音 / 待办浮层并存、粘贴隔离、合并窗口）；全量 `npx vitest run` 98 个文件 1208 条通过；`npx vue-tsc --noEmit` 通过。原型 `popup-reply-menu.html` 没有改：它是视觉规格，产品里的类名是 `qr-*`。

### P2c Mac 承载面（已完成）

与上文设计一致，差异与补充：

- **托盘**：`scripts/gen_tray_away_icon.py` 生成月亮徽标图标（`tray-idle-away.png`）；图标优先级是「有待答请求 > 离开 > 集成提醒的「?」」——有待答请求时仍是「活动」图标，离开状态压过低等级的集成提醒（提醒仍留在菜单里）。菜单状态区下面是离开分组（未开：一键「开启离开模式」；已开：状态行（精确到分钟）+「结束离开模式」），每个 Agent 子菜单底部两项勾选；托盘「要求提问」发的是固定文案的插话，Daemon 认出后给该会话开 30 分钟的一次性放行（P2a 的 `QUICK_ASK_TEXT`）。托盘设为「关闭」（`menuBarIcon = off`）时没有菜单栏图标，托盘上的入口自然不存在；没有为此另造入口，胶囊、控制台、CLI、IM 与手机仍可用。
- **控制台**：`AwayBanner.vue`（全局离开的提示条）、`AwayMenu.vue`（会话页头的「AskHuman ⌄」下拉）、`Sidebar.vue` 只给单独设置过的会话加小图标；格式化放进 `console/away.ts`（有单测）。
- **设置**：「离开模式」卡（`AwayCard.vue`）三项：桌面胶囊、回到电脑时提醒、推送到手机的等待时间；`general.away.pushAfterSecs` 取它与 iOS 渠道 `alertAfterSecs` 的较小值（只会让推送更早，见 `AwayConfig::alert_after_secs`）。
- **桌面胶囊（macOS）**：spike 的结论是**不需要 NSPanel**——Tauri 窗口属性（透明、无边框、置顶、全空间、不可聚焦、不进任务栏）加两条 NSWindow 设置（`collectionBehavior = canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary`，`level = NSStatusWindowLevel (25)`）就够。真机检查（隔离 daemon，`CGWindowListCopyWindowInfo`）：窗口层级 25、在屏幕上、前台应用不变（不抢焦点）；3008×1692 点的主屏上落在 X=1401（= 屏宽/2 − 胶囊宽/2）、Y=38（= 可用区域顶 30 + 8）。位置记忆按屏幕布局存；**窗口大小由页面上报**（`capsule_resize`），宿主按胶囊中心重新摆放。
- **踩到的坑**：窗口创建参数是逻辑点，而 tao 报的显示器几何是物理像素；胶囊最初两者混用，在 2× 屏上落点偏了一倍（`b4c5bec5` 修复，改为先把显示器换算成点）。
- **欢迎回来**：离开期间每 3 秒读一次 `idle::secs_since_last_input()`，空闲满 `WELCOME_IDLE_SECS`（300 秒，N7）之后的第一次输入把同一张卡展开成「欢迎回来」；120 秒无人回应收回成胶囊，不会自动结束离开模式（D23）。判定是纯函数（有单测，含「采样来得晚」的情形）。
- **浏览器里目视真实组件**（`mockIPC` 把真实的 `AwayCapsuleView` 摆在假桌面上，浅色 / 深色、中 / 英）：胶囊（月亮、时长、等待数徽标）、展开卡（备注、各 Agent 次数、等待条与「查看」）、欢迎回来卡都与原型一致。
- **没做 / 没验证**：胶囊的悬停与点击展开、拖动并记忆位置、欢迎回来的真实触发、全屏应用与多个桌面上方是否可见——都只有单测或浏览器里的页面，没有在真实桌面上手动操作（这台机器上截不到 GUI Host 的窗口：`screencapture` 失败，computer-use 也不列出这类附属应用）；Windows / Linux 没有胶囊与欢迎回来（按计划）。

### P2d iOS 承载面（已完成）

与上文设计一致，差异与补充：

- **线协议**：`away.state`（`rev`、`global`、`sessions[]`、`escalatedWaiting`）、`away.set`、`away.session.set`；`Caps.away`；`AnswerSubmit.directive`；`RequestView.escalated`。**与设计的差异**：没有单独的 `away.get`——状态放在 `hello` 快照里，之后每次变化、重新同步时主动下发；`away.set` 没有单独回执（紧接着到的 `away.state` 就是确认）；会话行里没有每行的 `mode`（手机用 `away.state.sessions` 推出），Agent 快照因此没动。夹具与两边测试同步（`ios/Fixtures/wire-v1/messages.json`）。
- **Mac 网关**：`Host` trait 加 `away()` / `away_apply()`，Daemon 侧调同一个引擎接口（`daemon/runtime/ios.rs`）；回答里的 `directive` 只在**这条回答赢得请求**时才应用——选择落在 `Coordinator` 的一个 directive 槽里，由收尾代码取走，所以无论哪个渠道答，应用的位置只有一处。多台 Mac 各自独立，手机逐台下发。
- **手机**：`AwayStore`（`AppStore+Away.swift`、`AwayModels.swift`、`PhoneEngine+Away.swift`）；`AwayStrip`（标签栏根页面上方的橙色条，带「结束」；推出去的页面没有标签栏也就没有条）、Agents 工具栏的月亮按钮与半屏开启页（≥2 台 Mac 才显示 Mac 卡）、Agent 详情里的「此会话的 AskHuman」行与 Agent 行上的小标签、升级提问上的提示卡、底栏旁的菜单与待发面板（`Views/Away/`）。
- **界面细节**：待发面板在请求结束时清空；Agent 行上的次数用紧凑的小标签；条里的文字允许缩小（`minimumScaleFactor`）；一个 SwiftUI 的坑：空的待发面板仍然占一份布局间距，已处理（面板为空时不参与布局）。
- **老 Mac**：没有 `caps.away` 就不显示任何离开相关入口（有测试）。
- **验证**：Rust `ios::gateway` 的 `tests_away`；Swift 包测试（引擎、`AwayStore`、夹具往返）；模拟器 UI 测试 `AskHumanAwayUITests`（en + zh-Hans，模拟器 iOS 27）。iOS 18.5 的经典条样式在后面的「iOS 18.5 经典条补验」里补验过；iOS 17 没看过。

### P3 快捷回复（已完成）

与上文设计一致，差异与补充：

- **预设由 Daemon 解好随请求带着**：`quick_replies_for`（`daemon/request.rs`）在建请求时按配置（没编辑过就取出厂 5 条，随界面语言）算出列表，放进 `AskRequest.quick_replies`，手机从 `AskView.quickReplies` 读同一份（可选字段，没有升协议号）。What's next、严格选择和没有菜单的卡片里是空的，所以每台 Mac 各用各的配置（N8），手机不编辑。
- **配置与清洗**：`general.quickReplies`（`null` = 没编辑过）；`sanitize_quick_replies`（去空白、丢空行、每条最多 200 个字符、最多 10 条）在读取和保存时都跑，前端 `settings/quickReplies.ts` 有同样的常量与函数。
- **弹窗**：数字键直选是 **1–9，第 10 条是 0**（计划里写的 1–5 是起草时的想法）；全部已作答时预设**置灰而不是隐藏**（仍列出、箭头键跳过，头部提示改成「每个问题都已作答」），这样菜单的形状不随答题进度变化；多出一行「管理快捷回复…」，打开设置并高亮那张卡。`submit()` 保持无参（模板里直接绑 `@click`），预设走 `submitWith(presetText)`。点预设时已选 / 已写 / 已挑待办的题保持原答案，其余题的文字被填成这一句；待发指令照常随预设一起发出。
- **手机**：底栏菜单的第一段是「快捷回复」（SwiftUI `Menu` 向上展开，系统会把顺序翻过来，所以第一个预设离拇指最近——用截图确认过），第二段是两条指令；点预设就发。`AnswerComposer.items(uploads:fillingWith:)` 只用**这条请求自带的**预设填充（传别的文字一律忽略）；`quickReplyTargets == 0`（全部已答）时预设置灰。Mock 后端把预设挂在「支持离开模式的 Mac」上（`away` / `awayOn` 场景），默认 `demo` 场景不带。UI 测试按**标签**找菜单项（系统菜单项没有 `accessibilityIdentifier`）。
- **真窗口 / 浏览器里目视**（`mockIPC` 里的真实 `PopupView` 与 `SettingsView`）发现并修了四处：多题页脚里菜单从按钮右缘往左伸 336pt，会越过窗口左缘被截掉（现在打开后量一次、平移回窗口内）；菜单头部的标题被挤成两行、提示也折成两行（标题不换行，提示缩短：「Click to send · fills 3 unanswered」）；英文出厂预设「Use your own judgement, no need to wait for me」排不进一行、留下孤字（缩短成「…, don't wait for me」，Rust、手机 mock 与 UI 测试同步）；十条预设的菜单比小窗口还高（菜单加 `max-height` 与滚动，方向键移动时把高亮行滚进视野）。预设行最多显示两行，再长就省略（完整文字在悬停提示里）。
- **验证**：`PopupView.quickReply.test.ts` 51 条；`QuickRepliesCard.test.ts` 14 条加清洗函数用例；`cargo test` 的 `config` / `daemon::request` / `ios` 新用例；Swift 包 `AnswerComposerTests` / `AwayStoreTests`；模拟器 UI 测试四条（菜单顺序、预设一点即发并在历史里可见、全部已答时置灰、严格选择与 What's next 没有预设）。

### P4 CLI 与 IM 命令（已完成）

与上文设计一致，差异与补充：

- `AskHuman away on [备注] [--session]` / `off` / `status`（`cli/away_cmd.rs`）：`status` 直接读 `state/away.json`、不拉起 Daemon；`--session` 只作用于调用它的 Agent 会话；**Agent 进程里只允许开启、不允许 `off`**（用户决定什么时候回来，N6）。
- IM：`/away [备注]`、`/back`（别名 `/离开`、`/回来`；Slack 用 `!`），各答一行并写明回来的办法；只有带前缀的形式才是命令（「away」一词出现在普通消息里不会触发）。
- 测试：命令解析、拒绝逻辑、IM 命令分发（`autochannel`、`daemon/runtime/inbound.rs`）。

### 合并 main 与最终验证（2026-11-05）

P4 之后把当时的 main（`b0e83385`：守护进程交接、失败恢复、iOS 顶栏按钮）合进了分支（`30fe8973`），所以之后对 main 可以直接 fast-forward。三处冲突都是两边各自追加：`docs/overview-ios.md`（两条 iOS 要点都留）、`daemon/runtime/agent_control.rs` 的测试夹具（沿用本分支共享的 `test_state`，补上 main 给 `ServerState` 新加的 `exit_intent`、`closing`）、`ios/gateway/tests_requests.rs`（两组新测试都留）。协议号两边都是 5，没有变。

合并后的验证（都在隔离环境里，没碰日常 Daemon，没有任何东西发到用户手机）：

- **Rust**：`cargo fmt --check` 与 `clippy --all-targets -D warnings` 干净（只有 `vendor/tao` 里本来就有的警告），`cargo test --bin AskHuman` 2592 通过 / 0 失败 / 11 忽略。
- **前端**：`vue-tsc` 通过；vitest 102 个文件、1276 条全过。
- **Swift 包**：6 个测试包共 672 条、0 失败（要真实服务的端到端按设计跳过）。
- **真实 Daemon 端到端**（`AHE2ETests`，本地 broker，用合并后装出来的二进制）：41 通过、12 按设计跳过、**3 失败，而且在 main 上同样失败**：`AgentE2ETests.testAMessageToAnAgentThatIsNotWorkingIsRefused` 与 `testSessionsOfTheMacAreListedWithTheirState`（期望 `notWorking`，main 的 `agent_control.rs::can_interject` 现在对空闲会话返回 `idleUnsupported`），以及 `testAFollowedSessionReportsWhenItsTurnEnds`（等不到 `done` 事件）。用 main 的日常版二进制（`~/.local/bin/AskHuman`，装机早于 `b0e83385`）单独跑这三条，失败一模一样，所以不是本功能或这次合并带来的，多半是 main 上的 Agent 生命周期改动之后这些用例没有跟着改，留给那边处理。`TodoE2ETests` 里本功能加的四条通过。
- **隔离 Daemon 里用合并后的二进制冒充 Claude 会话**：`away on` → 提问被自动回复（带备注）→ `away status` 显示自动回复次数 → `away off`，一切如设计。
- **iOS 模拟器 UI 测试**（`AskHuman SR UI`，iPhone，iOS 27.0）：`AskHumanAwayUITests` 16/16（英文、简体中文各一遍）、`AskHumanTodoPickUITests` 6/6（简体中文）、主类 `AskHumanUITests` 63/63（英文）、`AskHumanRequestNavigationUITests` 8、`AskHumanRequestTodoUITests` 8、`AskHumanTodosUITests` 10、`AskHumanRequestConnectionUITests` 3、`AskHumanArrivalUITests` 7 全过；`AskHumanInputVisibilityUITests` 9 条里 7 过、1 条要专用相册夹具而跳过、1 条 `testEachQuestionRevealsItsOwnAttachmentEntryPoint` 偶发失败（`second.typeText` 前的点按没拿到键盘焦点，约两次里一次）。**在不含本分支改动的 main 上同样复现**（同一行、同一报错，第 2 轮失败），是已有的不稳定用例，不是本功能引入的。
- **没有重跑**：`AskHumanDeveloperUITests`、`AskHumanModelUITests`、`AskHumanWebUITests`、`AskHumanLargeScreenUITests`（要 iPad / Duo 模拟器）、`AskHumanLiveUITests`（要真实服务）和视频附件那一条。它们覆盖开发者模式、新建任务表单、Markdown 页与大屏，本分支没有改这些页面（新建任务表单的顶栏按钮是 main 带来的改动，那边自己验过）。

### iOS 18.5 经典条补验与再次合并 main（2026-11-05）

在 iOS 18.5 的模拟器（`AskHuman SR 18`，iPhone 16）上补看了经典样式（iOS 17–25 没有液态玻璃的底栏）：常驻条、开启页、升级提示卡、底栏菜单与待发面板逐屏目视，再跑 `AskHumanAwayUITests`。

- **目视发现并修了一处**（`02ca1be7`）：待发「发送并离开」时，经典底栏里计数块分到的宽度比它需要的多，按钮标签被截成「Send and go…」。现在待发期间按钮先定尺寸、只有计数块让位（一行，最小缩到 80%）；没有待发指令时两个修饰符取的都是默认值，布局与之前完全一样。其余几屏（常驻条、开启页、升级提示、底栏菜单的顺序、待发面板）与 iOS 27 一致，没有别的问题。
- **两处测试脆弱点，不是应用问题**（`734a86a2`）：① iOS 18.5 上点一行不会先把它从底栏下面滚出来，`isHittable` 还报真——迁移请求第一题的选项落在屏幕下沿，点按没有选中，「全部答完后预设置灰」那条用例因此失败；测试里加了 `scrollIntoReach`，先把这一行滑到上下栏之间再点，并等计数变成 3/3。② 发送之后页面有时没有自己退回列表：Mock 的回答是即时的，「已结束」事件可能先于「发送中」状态到达，而 `AppStore.advance(from:)` 只在状态是「发送中」时才换页（真实 Mac 有网络往返，不会这样；这个设计在 main 上就是如此）；测试里加了 `leaveTheSentRequest()`，没回到列表就点返回。
- **结果**：iOS 18.5 上 `AskHumanAwayUITests` 16/16；iOS 27.0 上同一个类英文 16/16、简体中文 16/16；`AskHumanInputVisibilityUITests` 8 通过、1 条要专用相册夹具而跳过（上一轮偶发失败的那条这次也过了）；`AskHumanRequestNavigationUITests` 7/8，失败的是 `testManualReturnRestoresTheReadingPositionOfALongForm`（返回后长表单的阅读位置 15 秒内没回到原处，`AskHumanRequestNavigationUITests.swift:101`）。**这条在 main（`b03606fc`，不含本分支任何改动，临时 worktree 里单独构建）上同样失败、同样 7/8**，暂存掉本次的 `AnswerScreen` 改动也一样；它之前通过过（合并后的第一轮验证里这个类 8/8，重启模拟器后单跑也过过一次），所以是 main 上已有的不稳定用例，不是本功能引入的，已记进 PROGRESS。
- **再次合并 main**（`86aa3f7d`）：main 又走了两个提交（弹窗里拦截原生导航离开答题页、Codex 的 stdio MCP 配置修复），无冲突，协议号仍是 5，分支继续可对 main 快进。合并后 `cargo fmt --check`、`clippy --all-targets -D warnings` 干净，`cargo test --bin AskHuman` 2596 通过 / 0 失败 / 11 忽略。
- **没看的**：iOS 17、iPad 与 Duo 大屏上的常驻条，真机。

### 总体没有验证的部分

- **真实 Agent 端到端**：Claude Code / Codex / Cursor 各一轮（开离开 → 提问与 whats-next 被自动回复 → `--escalate` 送达 → 结束；「不再使用」→ 即时提示、Stop 确认静音、权限与出错卡仍在）。需要用户授权真实 Agent 调用；只有隔离 Daemon 里用 `CLAUDECODE` + 会话 id 冒充 Claude 会话的端到端，以及 Stop hook / `AskUserQuestion` 接管的单测。
- **桌面胶囊在真实桌面上的交互**与 Windows / Linux 的托盘月亮、控制台（见 P2c）。
- **iOS 17 的降级条样式**、iPad / Duo 大屏上的常驻条与真机一轮（iOS 18.5 的经典条已补验，见上）；**TestFlight 没有出**。
- 需求 §3.10 的默认做法 N1–N10 没有再次征求用户意见（用户不在），按原文采用。

