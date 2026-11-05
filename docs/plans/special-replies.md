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
- **弹窗**：菜单上半加预设、数字键 1–5 直选、底部「管理快捷回复…」（打开设置 › 通用）。点预设直接发送：已作答的题保持原答案、未作答的题都填这一句；全部已作答时预设置灰（D18）；What's next 弹窗没有预设（D20）。
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
