# 计划：iOS 请求页查看「此会话的历史问答」

> 需求：`docs/specs/ios-session-history.md`（决策编号 S1–S17 均指其 §3）。状态：2026-11-11 用户批准需求与本计划，实施中，进度见文末「实施记录」。下文各阶段的文件名是起草时的设想，**实际以「实施记录」为准**。
> 工作方式：**在主工作树开发**（用户 2026-11-11 定，要求一次做完、次日早上查收）；P1 原型不过关就停下来问用户，不自行改方案。主工作树里还有别人未提交的改动，本功能只提交自己的文件，不碰别的。

## 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 需求、本计划、`PROGRESS.md` 的两条（功能本身、后置的 MCP 兜底分区） | 提交 |
| P1 | **滚动原型（决定性关卡）**：只在手机界面里、用假数据，验证「下拉触发」与「前插后视口不动」，选定做法 | 原型的 UI 测试 + 模拟器上手看；结论写进本节 |
| P2 | 线协议：Rust 与 Swift **同一提交**（共用夹具，两边必须同时变绿） | `cargo test ios::fixtures`、`swift test --filter AHProtocolTests` |
| P3 | Mac：会话键、历史读取、投影、网关与 Host | `cargo test`（`history`、`ios::`、`channels::`、`daemon::runtime`）、`cargo clippy -D warnings`、`cargo fmt --check` |
| P4 | 手机引擎：`Backend`、`PhoneEngine`、`SessionThread`、Mock | `cd ios/Packages/AskHumanKit && swift test` |
| P5 | 手机界面：线程视图、两个页面的集成、菜单项、胶囊、本地化、UI 测试 | `xcodebuild build` → `make-catalog.py` → `xcodebuild test`（模拟器，en + zh-Hans，前台跑） |
| P6 | 装机与真实验证、真实 daemon 端到端、文档同步 | 见「验证清单」 |

顺序就是依赖顺序：P1 不依赖线协议，也是整件事成败所在，先做；过了才开始 P2。P2 之后 Mac（P3）与手机（P4–P5）可以各自推进，但只有一个人在做，按序走。

## P1 细节（滚动原型）

S2 与 S12 成不成，只取决于一件事：**在 SwiftUI `ScrollView` 的上方插入内容，用户看到的东西纹丝不动，并且下拉能可靠触发**。先用最小代价验证它，通过再做别的。

做法：

- **假线程**：`SessionThread` 先放在 App 目标里，加载器是注入的闭包（P1 吐假卡片：每次 8 张、延迟 0.3 秒、共 20 张）；P4 再把它搬进 `AHCore` 并换成真加载器。启动参数 `-AHThreadPrototype YES` 只在原型期间存在，让 `demo` 场景的某个作答页与确认页挂上它。不碰线协议。
- **两种触发方式都试**：
  - **A**：SwiftUI `.refreshable { }`（闭包里取数，返回后系统收起转圈），转圈收起**之后**再前插；
  - **B**：把 `UIRefreshControl` 挂到 `RequestScrollPosition` 同样办法找到的 `UIScrollView` 上（沿 `superview` 链找最近的 `UIScrollView`；`valueChanged` 里取数，`endRefreshing()` 之后再前插）。
- **补偿**：新增一个 UIKit 附着视图（形态同 `RequestScrollAttachment`），在它的 `layoutSubviews` 里补偿——与新内容是同一次布局，所以不闪。补偿量 = 前插前后 `request-top` 锚点（`RequestReadingAnchorView`，`convert(.zero, to: scroll).y`）的 y 之差；前插前先记下「该锚点相对视口顶的位置」，布局后把 `contentOffset.y` 调到让它回到同一个相对位置。
- **线程区用普通 `VStack`，不用 `LazyVStack`**：高度是确定的，补偿才准；卡片收起时很矮，几十张可以接受（实测卡顿再议）。
- **要一起验证的互动**：
  1. 短页（内容不足一屏）也能下拉：需要 `alwaysBounceVertical = true`，只在入口条件满足时设；
  2. `RequestScrollPosition.update()` 只在 `restoring != nil` 时才改偏移，用户一拖动就清掉，应不会撤销补偿；但**菜单入口不是手指拖动**，待还原的阅读位置不会自动清掉，会和「滚到最新一条」打架：加载线程前要有个明确的「新的阅读意图」入口（例如给 `RequestScrollPosition` 一个递增的令牌参数，变了就放弃 `restoring`）；
  3. 视口顶在 `request-top` 之上（线程区里）时，`update()` 的兜底分支保存的是**绝对偏移**（`("request-top", 0)` 加整段 `top`）：之后重进页面（线程已丢）会被还原到页面中间。要改成视口在线程区里时保存「`request-top` 加 0」，即回到页面顶部；
  4. 键盘开着时下拉（`.scrollDismissesKeyboard(.interactively)` 会先收键盘）：可以只收键盘、不加载，但不能卡死或跳动；
  5. 「减弱动态效果」开 / 关各一遍；
  6. `ReadingTracker` 与「下一题」在前插后仍正确（它们用的是 ScrollView 可见区的坐标系 `answer-scroll`，理论上不受影响，要实测）；
  7. 菜单入口的「滚到最新一条」：目标是线程里最新一张卡片的顶部，动画结束后仍不被 `restoring` 拉走。

**判据**（全部满足才算过）：

1. 前插前后，`request-title`（请求头里的会话标题，在滚动内容里）在屏幕上的 y 不变（容差 1 pt），肉眼无闪动；
2. 转圈收起后再前插，没有「回弹加偏移」叠加出的跳动；
3. 短页、长页都能下拉触发；不满足入口条件的页面没有转圈、没有多出的回弹；
4. 前插后手指继续把内容往下拖，能顺畅读到新插入的卡片，不卡、不被拉回；
5. 与阅读位置还原（含上面第 2、3 条）、「下一题」、已结束条互不干扰。

**产出**：本节末尾写结论（选 A 或 B，补偿的具体做法与踩到的坑）；原型代码留作 P5 的起点（去掉启动参数）。**两种都不过关**：停下来用 AskHuman 问用户（会动 S2 / S12，不自行改方案）。两种都过，选代码更少的那个，不必问。

## P2 细节（线协议，Rust 与 Swift 同一提交）

- **Rust `ios/wire.rs`**：
  - 常量：`SESSION_HISTORY_PAGE = 8`、`SESSION_HISTORY_PAGE_MAX = 20`、`SESSION_HISTORY_BYTES = 256 * 1024`、`HISTORY_MESSAGE_MAX_CHARS = 4000`、`HISTORY_QUESTION_MAX_CHARS = 1000`、`HISTORY_OPTION_MAX_CHARS = 500`、`HISTORY_INPUT_MAX_CHARS = 2000`；`t::HISTORY_SESSION_GET = "history.session.get"`、`t::HISTORY_SESSION = "history.session"`。
  - 类型：`SessionHistoryGet { req_id, agent_key, before_ms: i64, before_id: Option<String>, limit: u32 }`、`SessionHistory { req_id, agent_key, items: Vec<SessionExchange>, more: bool }`、`SessionExchange { id, at, channel, markdown, message, attachments, qs: Vec<SessionQuestion>, notes }`、`SessionQuestion { text, selected, input, images, files }`（可选字段缺省不发，键 camelCase）。`req_id` 是手机生成的关联号，**与 `request.get` 的 `requestId`（AskHuman 请求的 id）不是一回事**，应答原样带回。
  - `SessionHistory::empty(req_id, agent_key)` 与 `shrink(&mut self) -> bool`（先去掉最旧的往来，只剩一条仍超就把各段文字减半；`more` 在去掉往来时置真），写法与 `TodoList` 的 `shrink` 一致。
  - `Caps.session_history: bool`（`#[serde(default, skip_serializing_if = "is_false")]`）；`RequestView.agent_key: Option<String>`（`#[serde(default, skip_serializing_if = "Option::is_none")]`）。两个结构都有字面构造要补字段：`Caps`（`daemon/runtime/ios.rs` 的 `caps()`、`gateway/testkit.rs`、`wire.rs` 内的一条测试）与 `RequestView`（`views.rs` 两处、`testkit.rs` 两处）；`gateway/requests.rs` 的 `light_view` 是整份 `clone()`、`deliver` 用 `..view`，不用改，但要有测试证明轻量视图保留 `agent_key`。
  - `registry!(...)` 加两项（`ALL_TYPES` 与 `normalize()` 随之更新）。
- **Swift `AHProtocol/Messages.swift`**：`Wire` 的同名常量；`SessionHistoryGet`、`SessionHistory`、`SessionExchange`、`SessionQuestion`；`Caps.sessionHistory`（`@OmitFalse`，初始化器加默认参数 `sessionHistory: Bool = false`）；`RequestView.agentKey: String?`；注册表加两项。
- **夹具 `ios/Fixtures/wire-v1/messages.json`**：只**追加**样本，既有样本原样不动（既有的 `Caps` / `RequestView` 样本保持原样，证明缺省字段不进 JSON）。追加：`history.session.get` 带 / 不带 `beforeId` 各一；`history.session` 带两条往来（含多题、未作答、附件个数、自动回复渠道）且 `more` 为真一条，空页一条；一条带 `agentKey` 的 `request.new` / `request.full` 样本；一条带 `sessionHistory` 能力位的 `hello.ack`。Rust 的 `ios::fixtures` 要求每个注册类型至少一个样本，所以 Rust 与 Swift 必须同一提交。
- **不碰** `ipc/mod.rs`；`PROTO` 与 `PROTOCOL_VERSION` 不动（只加可选字段与新消息类型）。

## P3 细节（Mac）

1. **会话键的格式化函数**：`ios_agents::agent_key` 是 `pub(super)`，`channels/` 用不到。新函数放进 `ios/views.rs`（`channels/ios.rs` 已经在用 `views`）：`agent_key(kind, session_id)` 与 `split_agent_key(key)`；`ios_agents::agent_key` / `split_key` 改成一行转调，保证只有一份实现。`SessionRef::agent_key()` 调它。测试：两处对每种 `AgentKind` 逐字相同；`split` 与 `agent_key` 互逆（含会话 id 里带冒号）。
2. **请求带键**：`channels/ios.rs` 的 `IosChannel::start` 在 `built.view.session_title = title;` 旁写 `built.view.agent_key = session.as_ref().map(SessionRef::agent_key)`；`channels/confirm.rs` 在写 `session_title` 处同样处理。测试：有 / 无 `SessionRef` 时视图里的键；与 `AgentView.key`（由 `agent_view` 生成）逐字相同。
3. **`history.rs`**：新增 `sends_for_session_before(agent_kind, session_id, before: Option<(i64, &str)>, n) -> SendsPage { entries, more }`（及 `_at` 变体供单测，沿用 `recent_sends_at` 的写法）：口径同 `recent_sends_for_session`（`ChannelAction::Send`，`agent_kind` 与 `agent_session_id` **精确相等**，不碰 MCP 分区），排序 `(timestamp_ms, id)` 降序，取游标之前的 `n + 1` 条，多出那条只用来得出 `more`。单测：游标（同毫秒不同 id、跨页边界、`more`）、只取 `Send`、自动回复（`away` / `mute`）列出、精确匹配（别的会话、MCP 分区、无会话的记录不混入）、空历史、历史关着、坏行跳过。
4. **投影**（纯函数，新文件 `ios/history_view.rs`，与 `todos_view.rs` 同一做法）：`exchange_of(&HistoryEntry) -> wire::SessionExchange`：文本按字符数裁剪、超出以 `…` 结尾；`channel` 原样；附件、图片、文件、批注只取个数；第 i 题取 `answers[i]`（越界当没答）；自动回复的单条答案落在第 0 题；`markdown` 取 `is_markdown`。单测覆盖以上，并断言序列化结果里**没有路径**。
5. **Host**：`ios/gateway/mod.rs` 的 `Host` 加 `async fn session_history(&self, agent_key: &str, before: Option<(i64, String)>, limit: usize) -> wire::SessionHistory`，默认实现返回空页；`ios/gateway/testkit.rs` 的测试宿主支持注入页面（与 `todo_list` 的注入同款）；daemon 实现在新文件 `daemon/runtime/ios_history.rs`（`ios_agents::split_key` 解析 → `spawn_blocking` 里调 `history.rs` → `exchange_of`；解析失败返回空页），`daemon/runtime/ios.rs` 的 `impl Host` 接上，并在 `caps()` 里置 `session_history: true`。
6. **网关**：新文件 `ios/gateway/history.rs`：`on_session_history_get`，与 `on_todo_list_get` 同样用 `spawn_then`：把 `limit` 夹到 `1…SESSION_HISTORY_PAGE_MAX`，调 Host，按 `SESSION_HISTORY_BYTES` 与 `MAX_ENVELOPE_BYTES` 反复 `shrink()` 到发得出去，回显 `reqId`；`gateway/mod.rs` 的分发处加 `t::HISTORY_SESSION_GET` 一支。测试（新文件 `tests_history.rs`，参照 `tests_requests.rs` / `tests_todos.rs`）：`limit` 夹取；非法键 / 无记录给空页；超预算先少给旧的、再减半文字；`reqId` 回显；能力位只在 Host 声明时才出现在 `hello.ack` / `pair.accept`；老手机（不发这条消息）不受影响。
7. **不碰** `ipc/mod.rs`、恢复库（`recovery.rs`）；`PROTOCOL_VERSION` 与 `PROTO` 不动。

## P4 细节（手机引擎）

1. `AHCore/Backend.swift`：`RemoteBackend` 加 `sessionHistory(macId:agentKey:beforeMs:beforeId:limit:) async throws -> SessionHistory`；conformer 只有 `PhoneEngine` 与 `MockBackend`。
2. `AHCore/PhoneEngine+SessionHistory.swift`：与 `PhoneEngine+Todos.swift` 同款「信箱 + 超时」，**复用 `todoBoxes`**（`TodoReply` 加 `.history(SessionHistory)` 分支，键 `"h|" + macId + "|" + reqId`）：断线（`PhoneEngine.swift` 的 `failTodos(for: nil, …)`）、某台 Mac 掉线、删除 Mac（`PhoneEngine+Sync.swift`）三处统一失败已经覆盖它，不再另造一套；在 `TodoReply` 与 `todoBoxes` 旁写一句注释，说明它现在装的是「Mac 的只读查询应答」。超时沿用 `todoReadTimeout`（10 秒）；`macs[macId]?.sync == .synced` 否则 `macOffline`，`caps?.sessionHistory == true` 否则 `.rejected("unsupported")`。`PhoneEngine.swift` 的分发里加 `case SessionHistory.type: onSessionHistory(...)`。
3. `AHCore/SessionThread.swift`（P1 时放在 App 目标里的那份搬过来）：`@MainActor @Observable final class SessionThread`：`init(macId:agentKey:anchorMs:load:)`（`load` 默认调 `backend.sessionHistory`）；`items`（旧到新）、`phase`（`idle / loading / failed(reason) / ready`）、`reachedStart`（`more == false` 后为真）；`loadMore()` 一个入口（首次与翻页共用）；**单飞**（在途时再触发被忽略）；游标 = 线程里最旧一条的 `(at, id)`，空时 `(anchorMs, nil)`；失败保留已有 `items`，可重试；`Diagnostics.note` 只记条数与结果，不记内容。**不进 `AppStore` 缓存**（S14：离开页面就丢），由 `RequestScreen` 持有。
4. `AppStore`：`func sessionHistoryAvailable(for item: InboxItem) -> Bool`（`item.view?.agentKey` 非空，且 `macs[item.macId]?.caps?.sessionHistory == true`，且视图不是占位视图）。
5. `AHMock`：`MockBackend.Scenario` 加 `sessionHistory`（`-AHScenario sessionHistory`），该场景的 Mac 声明 `sessionHistory` 能力；`MockBackend+SessionHistory.swift` 按 `agentKey` 与游标分页出假数据，可注入失败与延迟；`MockData` 为此场景造几个带 `agentKey` 的请求（一个会话 20 多条往来，含多题、未作答、自动回复、很长的消息、中文、Markdown；一个会话只有几条；一个没有 `agentKey` 的请求）。**`demo` 场景保持原样**，它就是「没有入口」的回归。
6. 测试：`AHProtocolTests`（P2 已做）；`AHCoreTests/TestMac+SessionHistory.swift`（假 Mac 回页面，记录收到的游标；与 `TestMac+Todos.swift` 同款）；`SessionHistoryEngineTests`（应答路由、超时、离线、能力位缺失、断线时在途请求失败）；`SessionThreadTests`（首页、前插顺序、耗尽后不再请求、失败后重试、单飞、游标推进、同毫秒不重复）；`AppStore` 的可用性判断（无键、老 Mac、占位视图）。

## P5 细节（手机界面）

1. `Views/Request/SessionThreadView.swift`：`SessionThreadView(thread:)`：顶部状态行（转圈 / 「连不上 Mac」加「重试」/「没有更早的记录了」）、卡片列表（`SessionExchangeCard`：收起 / 展开、渠道名映射、自动回复弱化）、「回到当前提问」胶囊。卡片用原生 `MarkdownView`；没有消息时省略消息；`textSelection(.enabled)`；每张卡片一个合并的读屏元素；**线程区不套 `endedDimming`、不受 `disabled` 影响**（已结束页上仍可点开卡片）。胶囊的显隐只在越过阈值（离当前提问超过一屏）时才改状态，不要每帧写 `@State`。
2. **页面集成**（按 P1 选定的做法）：`AnswerScreen` 与 `ConfirmScreen` 的滚动内容改成 `VStack(spacing: 0) { SessionThreadView; 原来的 VStack }`，**原来的 VStack 连同 `.readableWidth()`、`request-top` 锚点与 `RequestScrollPosition` 整个留在「当前请求区」**；线程区用同样的 `.readableWidth()` 与水平内边距，卡片与正文对齐。入口条件不满足时不放线程区、不设 `alwaysBounceVertical`、不挂刷新控件，页面与现在逐像素一致。
3. `RequestScreen`：持有 `@State var thread: SessionThread?`（首次需要时按 `AppStore.sessionHistoryAvailable` 创建，`onChange(of: id)` 时清掉）并传给两个页面；`hasMore` 加上入口条件（只有这一项时菜单也要出现）；`moreMenu` 加一项「此会话的历史」（`clock.arrow.circlepath`，`request-menu-session-history`）：线程未加载就加载并滚到最新一条往来；已加载就只滚动；失败显示错误行并滚到顶。下拉与菜单走同一个入口。
4. `RequestScrollPosition`：按 P1 第 2、3 条的结论加「新的阅读意图」令牌，并修正兜底分支（视口在线程区里时保存「`request-top` 加 0」）。这是对既有文件的小改动，改完要把它的既有用例（`AskHumanRequestNavigationUITests` 里与阅读位置有关的）全跑一遍。
5. **已结束页**：线程的加载入口（下拉、菜单）里先调 `store.stayOnEnded(item.id)`（S13）。
6. **文案与工程**：新增条目只有「此会话的历史」「没有更早的记录了」「连不上 Mac」「重试」「回到当前提问」「还有 N 题」「未作答」「附件 N 个」「离开模式」「静音模式」等（英文为源语言）；`xcodebuild build -derivedDataPath build/DerivedData` → `ios/Tools/make-catalog.py`（缺中文会失败）；新增 Swift 文件后在 `ios/` 下 `xcodegen generate`。
7. **无障碍标识**：`request-menu-session-history`、`session-exchange`（每张卡片）、`session-thread-start`（「没有更早的记录了」）、`session-thread-error`、`session-thread-retry`、`session-return`（胶囊）。
8. **UI 测试** `AskHumanSessionHistoryUITests.swift`（Mock 的 `sessionHistory` 场景，en + zh-Hans，沿用 `AskHumanRequestTodoUITests` 的启动参数写法）：下拉加载（卡片出现，`request-title` 的 y 不变）；到顶再下拉加载更早一页，到头出现「没有更早的记录了」；菜单入口（未加载 / 已加载）；收起 / 展开；确认页同样可用；已结束页加载取消倒计时（沿用 `AskHumanEndedUITests` 的做法）；Mac 离线时的错误行与重试；在线程区里离开再回来，页面回到顶部；`demo` 场景（无键）的页面没有菜单项、下拉没有转圈；最大字号、深色各一条（沿用 `AskHumanEndedUITests` 里「大字号」那条的做法）；iPad 一条（`AskHumanLargeScreenUITests` 的做法）。

## P6 验证清单

1. `node scripts/machine-health.mjs`；退出码 2 时先用 AskHuman 把报告告诉用户并请他腾内存，长任务放后台、短间隔轮询并说明在等什么，不要闷着等。
2. `cargo test`（相关模块）→ 全量一遍；`cargo clippy --all-targets -- -D warnings`；`cargo fmt --check`。
3. `./scripts/install.sh`；之后的提问走新装的 `AskHuman`。
4. **真实 daemon 端到端**（`cd ios/Packages/AskHumanKit && AH_E2E_BIN=<已安装的 AskHuman> AH_E2E_BROKER_DIR=<ios/Tools/local-broker> swift test --filter AHE2ETests`，缺环境变量会跳过）：新增 `SessionHistoryE2ETests`：在一次性 Mac 的 `configDir` 里写入 `history.jsonl`（同一会话 12 条、另一会话与无会话各几条），手机引擎对 `claude:<会话>` 取：第一页 8 条、新的在前，游标翻到第二页 4 条且 `more == false`；不混入别的会话；锚点之后的记录不出现。再造一个带真实会话 id 的在途请求（hook 建会话，同现有端到端），确认手机拿到的请求带 `agentKey` 且等于 Agents 列表里该会话的 `key`。
5. **iOS**：`swift test`；模拟器 `xcodebuild test`：`AskHumanSessionHistoryUITests`，加上与页面滚动、菜单、已结束页有关的既有用例（`AskHumanRequestTodoUITests`、`AskHumanRequestNavigationUITests`、`AskHumanEndedUITests`、`AskHumanBarUITests`、`AskHumanLargeScreenUITests`、`AskHumanInputVisibilityUITests`），en + zh-Hans；确保「没有入口」时既有行为不变。
6. **真实 Mac + 模拟器**：用新装的 `AskHuman` 在同一会话里问两三个问题并答掉，再发第三个；模拟器配对到这台 Mac，在第三个的请求页下拉，看到前两个的真实答案；菜单入口；已结束页（先在 Mac 上答掉）加载历史后不被自动带走。真机与 TestFlight：**不出**，除非用户要（出包按 `ios-testflight` skill）。
7. **文档同步**：`docs/overview-ios.md`（能力位 `sessionHistory`、两条新消息、`ios/history_view.rs` 与 `gateway/history.rs` 进模块地图、「历史只读 Mac、手机不落盘」进不变量）；`docs/specs/ios-channel.md` §6；`docs/specs/ios-answer-flow.md`（§3.8 的 E12 加一个打断源、验收里补一句）；`docs/specs/reply-history.md`（一句交叉引用：iOS 经 `history.session.get` 读取，口径同 F1）；`docs/wiki/ios-setup.md` 与英文版（可选一句话）；本计划的「实施记录」与规格 §13 的「与起草时的差异」（若有）；`docs/PROGRESS.md`：功能那一节改成「待真机验收」或全部验完后删除，MCP 兜底那一节保留。主 `docs/overview.md` 不改（全局地图与不变量没变）。

## 提交（Conventional Commits，英文、祈使句；用户可见的只有第 4 条）

1. `docs(ios): spec and plan for session history on the request page`（P0）
2. `feat(ios): add session history messages to the wire protocol`（P2，Rust + Swift + 夹具，`Release-Note: skip`）
3. `feat(ios): serve a session's earlier exchanges to the phone`（P3，`Release-Note: skip`）
4. `feat(ios): load a session's earlier answers on the request page`（P4–P5，带 `Release-Note:`，一句话写成用户语言）
5. `docs(ios): sync overview and specs with session history`（P6）

P1 的原型不单独提交，结论写进本计划，代码并入第 4 条。

## 风险

1. **前插不跳**（最大风险）：P1 关卡，不过关就停下来问用户。
2. **`UIRefreshControl` 挂到 SwiftUI 的 `UIScrollView` 上**可能被 SwiftUI 的布局重置，或与 `.refreshable` 冲突：P1 两种都试。
3. **`RequestScrollPosition` 与补偿互相打架**：见 P1 第 2、3 条；这个文件是既有页面的阅读位置还原，改动要小、回归要全。
4. **页面过渡快照**（`RequestPageCapture`）会把已加载的线程一并截下来：只是过渡用的内存快照，可接受；UI 测试里走一次「已结束页自动前进」确认没有视觉问题。
5. **短页下拉**需要 `alwaysBounceVertical`：入口条件满足的页面回弹行为会和现在略有不同（规格 §4 只承诺「没有入口时与现在一致」）。
6. **一页的 JSON 体积**：靠字节预算与 `shrink` 保证；单测里造一个极端往来（100 题、各段文字都很长）。
7. **`history.jsonl` 很大时每次下拉要整份读**：默认 200 条、`spawn_blocking`、只在用户动作时触发；实测慢再考虑按文件指纹复用索引（先不做）。
8. **机器很忙**：全量编译与模拟器测试可能要十几分钟；开发中只跑定向测试，阶段结束跑完整的；长任务先 `machine-health`、放后台、短间隔轮询。
9. **主工作树里别人未提交的改动**：见「开工前定下的事」；每个阶段提交前用 `git status` 核对，只 `git add` 自己的文件。

## 开工前定下的事

- **在哪个工作树做**：用户 2026-11-11 选**主工作树**（按 `AGENTS.md` 问过一轮，推荐过独立 Worktree，用户没采纳）。主工作树里别人还有未提交的改动，所以：只提交本功能的文件；凡是别人正在改的文件（开工时是 `daemon/runtime/mod.rs` 等）不碰，必要时换个不碰它们的写法，偏差记在「实施记录」。这个功能的 Dev 验证走生产 daemon 的远程投递，不需要渠道预设的选择。
- **P1 没过关就不进 P2。**（P2–P4 与 P1 的结果无关，P1 卡住时可以先做它们，再带着结果问用户。）
- **TestFlight**：不主动出。

## 实施记录

（开工后填写。）
