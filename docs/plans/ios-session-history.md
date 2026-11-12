# 计划：iOS 请求页查看「此会话的历史问答」

> 需求：`docs/specs/ios-session-history.md`（决策编号 S1–S19 均指其 §3）。状态：2026-11-11 用户批准需求与本计划，2026-11-12 实施完成（待真机验收），提交、偏差与验证见文末「实施记录」。下文各阶段的文件名是起草时的设想，**实际以「实施记录」为准**。**2026-11-12 追加「下拉的重新设计」（R0–R4，见文末），用户评审中、代码还没动**：它取代下面 P1 结论里「触发」一项（`UIRefreshControl`）与 P5 里与之相关的描述，前插补偿等其余结论仍然有效。
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

### P1 结论（2026-11-12，iPhone 16 / iOS 18.5 模拟器）

> 2026-11-12 追记：下面「触发」用的 `UIRefreshControl` 已被文末「追加：下拉的重新设计」取代（用户觉得系统转圈像在刷新消息）；「补偿」「踩到的坑」里与前插有关的部分仍是后续做法的基础。

**选 B**（`UIRefreshControl` 挂到 SwiftUI 的 `UIScrollView`）。判据 1–5 全部满足，没有动 S2 / S12。

- **触发**：`ThreadScrollHook`（`UIViewRepresentable`，形态同 `RequestScrollAttachment`）沿 `superview` 找到最近的 `UIScrollView`，**只在页面有线程时**装一个 `UIRefreshControl` 并设 `alwaysBounceVertical = true`（短页也能下拉）；`valueChanged` 里 `loadMore`。没有线程的页面不装、不设，与现在一致。
- **时序**：拿到一页 → `endRefreshing()` → 轮询 `contentOffset` 回到顶部（从手指抬起算，上限 1.5 秒；见「实施记录」）→ 记下 `request-top` 锚点相对视口顶的位置 → 才前插。转圈与前插不叠加，没有「回弹加偏移」的跳动。
- **补偿**：`reconcile()` 把 `contentOffset.y` 设成「锚点在内容坐标里的 y 减去记下的视口相对位置」，窗口 0.5 秒，**必须在同一次布局里**调用：附着视图的 `layoutSubviews`（`RequestScrollAttachment.changed`）、`contentSize` 的 KVO、以及锚点**各级父视图**（一直到滚动视图）的 `layer.position` 与 `bounds` 的 KVO。线程区是普通 `VStack`（不用 `LazyVStack`），高度确定。
- **验证**：运行循环 observer 挂在 CA 提交之后，逐帧采样请求锚点相对视口的偏移：**每一帧都是 0.0**（`max=0.0 last=0.0`，修正 3 次），即没有任何一帧被提交成错位——不只是最终位置对。原型 UI 测试 9 条（长页 / 短页 / 确认页下拉、读到线程顶再下拉加载更早一页、菜单入口、带「待还原阅读位置」的菜单入口、读线程时离开再回来落在请求顶部、线程在上时「下一题」与作答、键盘弹起时下拉），**减弱动态效果开 / 关各跑一遍，全过**。
- **踩到的坑**：
  1. 只观察锚点自己的 `layer.position` **不行**：SwiftUI 移动的是 `UIViewRepresentable` 外面的宿主视图，锚点在宿主里的 frame 不变，KVO 一次都不触发，请求被整个线程高度（实测 1430 pt）推走且回不来。改为观察各级父视图，并在附着视图的 `layoutSubviews` 里补偿（它在内容摆好之后才布局，正是修正的时机）。
  2. 菜单入口的「滚到最新一条」要先结束补偿窗口，否则动画滚动期间的布局事件会把页面拉回去。
  3. `RequestScrollPosition` 的两处改动已在原型里验证：「新的阅读意图」令牌（`intent`，变了就放弃待还原位置；菜单入口不是拖动，没有它会与「滚到最新一条」打架），与兜底分支（视口在请求之上时保存「`request-top` 加 0」，重进页面回到请求顶部，而不是被还原到页面中间）。
  4. XCUITest：键盘弹起时 `app.scrollViews.firstMatch` 可能是键盘建议栏的滚动视图，拖动要用 app 窗口坐标，否则会把建议词打进输入框；增量构建曾跑到旧的测试代码，拿不准时在测试里加一行标记输出确认。
  5. （P5 才暴露，原型里没有踩到）SwiftUI 会在自己的更新里把 `scrollView.refreshControl` 摘掉（页面没有 `.refreshable`），还会按内容高度把 `alwaysBounceVertical` 改回 `false`——内容不足一屏的页面（多数请求）因此拉不动。原型的页面碰巧每次都重装，产品里不行：两个属性都用 KVO 盯着，被改掉的那一刻放回去，并在每次附着时再确认一遍。
  6. （P5 才暴露）当前提问不足一屏时，它下面没有内容可滚，补偿要设的偏移超出内容末端。UIKit 不夹住程序设置的越界偏移：页面可能停在越界处（第一次触摸时跳一下），也可能在之后的某次布局被夹回，结果取决于时序——同一个测试在机器卡顿时请求落到页面底部（y 差 212 pt），空闲时又纹丝不动。办法是让线程出现后当前提问的区域至少一屏高（`requestRoom`：`ZStack` 里一块 `containerRelativeFrame(.vertical)` 的透明垫子，没有线程时高度为 0、结构不变），所有要滚到的位置（原位、最新一条、回到当前提问）就都在有效范围内。UI 测试：短页、确认页、长页（`req-history-tall`，长消息）都断言请求原位不动，短页再拖一下确认页面没有越界回弹。
- **A（`.refreshable`）试过，不选**：条件修饰符在线程晚于首帧出现时改变 `ScrollView` 的结构身份（滚动视图被重建，补偿失效，线程把请求推走）；线程从首帧就在时，同一个下拉手势没有触发。B 已全部过关，没有再深究 A。
- **原型代码**（`SessionThread.swift` 等）留在工作树里作 P5 的起点，不单独提交。

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

**在主工作树做完，2026-11-11 夜到 2026-11-12 凌晨，一次做完等用户查收。** 机器全程很卡（负载 100–370，交换区 15 GiB 以上，另有别的项目的模拟器在跑），编译与模拟器测试普遍是文档基线的几倍时间；没有因此砍掉任何一项验证。

提交（都只含本功能的文件，工作树里别人的改动没碰）：

| 阶段 | 提交 | 内容 |
|---|---|---|
| P0 | `c3e0e6b2` | 规格与计划 |
| P2 | `6e2957c3` | 线协议：`history.session.get` / `history.session`、`Caps.sessionHistory`、`RequestView.agentKey`，Rust + Swift + 夹具 |
| P3 | `1cd65337` | Mac：会话键、`history.jsonl` 分页读取、投影、Host、网关限额与预算、能力位 |
| P4 | `7b06045d` | 手机引擎：`PhoneEngine+SessionHistory`、`SessionThread`、Mock 场景 |
| P1 + P5 | `01995fa6` | 请求页界面：线程区、下拉与前插补偿、菜单入口、卡片、胶囊、本地化、已结束页「停留」、UI 测试（P1 原型并入这一条，没有单独提交） |
| P6 | `22df80b4` | 真实 daemon 端到端测试 `SessionHistoryE2ETests` |
| P6 | `f66bf8ce` | 总览、规格、本计划、`PROGRESS.md` 与 iOS 使用说明的同步 |
| P6 | `f9eb132c` | 修正：等页面回弹停稳再前插（见偏差 6），UI 测试认得 iPad 的分栏 |
| P6 | `81736e5c` | 记下回归与 iPad 的结果 |
| P6 | `ca61fa04` | UI 测试：键盘弹起时的下拉拖得更远些（见「验证」里的 iOS 26.2） |
| P6 | `c5109174` | 记下 iOS 26.2 与 27.2 的结果 |
| P6 | `35a722f8` | UI 测试：深色那一条改用设备外观（见「验证」里的深色） |
| P6 | 本文所在的 `docs(ios)` 提交 | 记下深色的更正与最大字号下的一处观感 |

**与计划的偏差**（都是实现细节，没动规格的决定 S1–S17）：

1. **下拉控件要靠两处 KVO 才立得住**（P1 原型里没有踩到，P5 才暴露）：SwiftUI 会在自己的更新里摘掉 `scrollView.refreshControl`，还会按内容高度把 `alwaysBounceVertical` 改回 `false`，短页因此拉不动。两个属性都用 KVO 盯着，被改掉的那一刻放回去（`ThreadScrollCoordinator.installRefresh`）。
2. **短页要给当前提问留余地**（`requestRoom`）：见上文 P1 结论第 6 条。首次全量跑 UI 测试时同一个断言在机器卡顿时落到页面底部（差 212 pt），空闲时又纹丝不动，才看出是时序相关的越界偏移。
3. **`AppStore.stayOnEnded` 也取消「还没开始」的倒计时**：条还没显示、倒计时还没开始时就读历史，原先的守卫会放过它，条一出现页面就被带走。新增 `EndedCountdownTests.testStayingBeforeTheCountdownBeganKeepsItFromBeginning`。
4. **Mock 场景多了两样**：长消息的请求 `req-history-tall`（页面比一屏高，覆盖 `requestRoom` 不起作用的那一半），与启动参数 `-AHHistoryFail N`（让 Mock 的 Mac 前 N 次读历史不应答，测错误行与重试）。
5. UI 测试里取「页面的滚动视图」要取最大的那个，不能取 `scrollViews.firstMatch`：确认页的代码块是横向滚动视图，`firstMatch` 有时是它，下拉就拉在了代码块上（zh-Hans 那一遍暴露）。
6. **等页面回弹，从手指抬起算**（iPad 上第一次跑才暴露；iPhone 上同样有，只是差得不到 1 pt，被测试的 1 pt 容差放过了）：原来的「至多 1.5 秒」从下拉生效那一刻算起，而下拉在手指还按着时就生效，慢拉还能再拉一秒多；回弹弹簧的尾巴很长，1.5 秒到点时页面还差几个点没回到顶（iPad 差 4 pt，iPhone 差 0.67 pt），这时前插并补偿，请求就停在弹簧当时的位置，比拉之前低几个点。临时插桩逐帧记下偏移量才看清；现在手指按着时不计时（最多 10 秒），抬起之后等弹簧停住，至多 1.5 秒。插桩已拿掉，不在提交里。

**验证**（2026-11-12）：

- Rust：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 干净；`cargo test` 2963 过、0 败、11 忽略。
- Swift：`swift test`（AskHumanKit）533 条全过（`SessionHistoryStoreTests` 10 条、`EndedCountdownTests` 29 条）。
- 真实 daemon 端到端（一次性 Mac + 本地 broker，装机后的日常版二进制）：`SessionHistoryE2ETests` 5 条全过（12 条往来按 8 + 4 翻页且互不混、同一毫秒的往来不丢不重、不带别的会话与别的 Agent、没有记录或读不了的键给空页、真实在途请求带的 `agentKey` 与 Agents 列表那一行的 `key` 逐字相同、不带会话的请求没有键）；`AHE2ETests` 全量 63 条：52 过、8 跳过（真实服务 / Codex 付费项）、3 败，三条败的就是上文「`AgentE2ETests` 在 main 上就失败」记下的那三条，与本功能无关。
- 模拟器 UI（iPhone 16 / iOS 18.5）：`AskHumanSessionHistoryUITests` 共 26 条（1 条 iPad 专用在手机上跳过）：英文 25 过；zh-Hans 24 过，另 1 条因测试取错滚动视图失败，改后复跑通过。
- `./scripts/install.sh` 成功，日常版 v0.17.1 已换上（也补完了 3 秒倒计时那次被挡住的安装）。
- **既有 UI 类回归**（iPhone 16 / iOS 18.5，英文；`requestRoom` 把每一页请求都包了一层，所以全跑了）：`AskHumanUITests`（64）、`Annotation`（9）、`Bar`（11）、`Ended`（17）、`RequestNavigation`（8）、`Away`（16）、`TodoPick`（6）、`RequestConnection`（3）、`DirectAsk`（9）、`Web`（22，另 1 条宽屏专用而跳过）全过；`Arrival` 8 条里 2 条在最忙的那一阵失败（动画的诊断元素在测试看到之前已经消失），空闲时复跑全过；`AttachmentDrag`（4）与 `VideoAttachment`（1）要专用夹具，跳过。失败的还有 5 条，**在功能开发之前的提交（`6b04266a`，临时 worktree 里单独构建，同一台模拟器）上同样失败**，与本功能无关：`InputVisibility.testShortAnswerAndAttachmentFitWithAccessibilityText`（键盘焦点事件，`PROGRESS.md` 早有记录）、`RequestTodo` 里读按钮 `isEnabled` 的 3 条（`docs/plans/ios-channel.md` 记过：18.5 的无障碍树把工具栏按钮报成两个同 id 节点）、`Focus.testSwitchingAnswerFieldsDoesNotTypeIntoAnEarlierQuestion`（第二题输入框拿不到键盘焦点，在基线上有两种失败方式）。
- **iPad**（iPad Pro 11-inch (M4) / iOS 18.5）：`AskHumanLargeScreenUITests` 27 条，19 过、8 条因侧栏只在更窄的分栏里才浮出而跳过；会话历史类 26 条整类跑一遍：23 过 3 败，三条败的都是测试对手机的假设，不是页面的问题——两条用手机的返回键回收件箱（iPad 的列表一直在旁边，改成「打开另一条再打开回来」，此时线程照规格被丢掉），一条是键盘弹起时的下拉（iPad 的下拉控件要拉很长，键盘上方的空间不够一次拉到触发；没有键盘时同样长度的下拉也不触发，是系统的行为，菜单入口不受影响，所以这条在 iPad 上跳过）。改完测试后这三条复跑：2 过 1 跳过。
- 修正之后在 iPhone 16 / iOS 18.5 上把会话历史类再跑一遍：英文 25 过 1 跳过（iPad 专用），简体中文 25 过 1 跳过。
- **iOS 26.2**（临时建的 iPhone 17 模拟器，跑完已删）：会话历史类第一遍 24 过 1 跳过 1 败，败的是 `testPullingWithTheKeyboardUpDoesNotJump`。逐帧记下偏移量才看清：下拉控件要把页面拉过顶部约 173 pt 才触发（18.5 上约 171 pt），而键盘弹起时那一次拖动最多拉到约 172.7 pt，差零点几个点，是测试的拖动距离卡在边上；没有键盘的普通下拉在 26.2 上照常触发（拉到 173 pt 处），页面没问题。拖动从屏幕高度的 0.15–0.5 改成 0.14–0.56（终点仍在键盘上方）后，26.2 与 18.5 上会话历史类各整类跑一遍：都是 26 条 25 过 1 跳过（iPad 专用）、0 败。
- **iOS 27.2**（iOS 27 的运行时只带 iPhone Duo 一种设备，没有普通 iPhone；临时建了一台 Duo，只用折起来的外屏，466 × 678 pt，跑完已删）：会话历史类 26 条，21 过 1 跳过 4 败。**所有下拉的用例都过**（含短页、长页、读到线程顶再下拉、键盘弹起时的下拉）；败的 4 条是测试对外屏版式的假设，不是页面的问题：外屏上顶栏的按钮被系统挪到右侧的竖条里，`app.navigationBars` 找不到返回键（`testLeavingWhileReadingTheThread…`、`testTheMenuIsNotPulledBackByAReadingPositionWaitingToBeRestored` 两条的 `backToInboxAndReopen`；既有的 `RequestNavigation` 等类也是这个写法，没有在外屏上跑过）；外屏没有顶栏，顶部内边距只有约 24 pt，菜单入口把最新一条放在内边距之下 8 pt（`scrollToNewest`，截图里卡片就在那里，没有被遮），而 `testTheMenuLoadsTheHistoryAndScrollsToItsNewestExchange`、`testTheMenuOnALoadedThreadOnlyScrolls` 的断言写的是 iPhone 顶栏之下（`minY > 40`，实测 32）。这两处测试没有为一台外屏专门改写。
- **深色（更正）**：`testDarkMode` 原先靠启动参数 `-AppleInterfaceStyle Dark`，这并不会让 App 变深，那条测试一直在看浅色的页面，先前记的「深色一条通过」并没有测到深色；拿截图时才发现。改成设备外观（`XCUIDevice.shared.appearance`，与 `AskHumanWebUITests` 的做法相同，结束后复原）后在 iPhone 16 / iOS 18.5 上复跑通过，截图里深色下卡片、分隔线与文字都清楚。
- **最大字号**（`AccessibilityXXXL`，同一次截图里看的）：卡片铺满宽度、文字清楚，断言通过；但「回到当前提问」胶囊也跟着放大，折成三行，盖住卡片的一块。没有给它限定字号，要不要限定是个小的设计选择，留给用户看过截图再定。

**没做 / 留给真机**：iOS 27 上普通 iPhone 的手感（只在 Duo 外屏的模拟器上验过）、真机一轮、真实 Mac 加手机界面的一轮（需要真实服务与签名，只有引擎一层有真实 daemon 的端到端）、TestFlight 出包（用户没要）。页面过渡的快照（`RequestPageCapture`）会带上已加载的线程，只是过渡动画里的内存快照，没有专门的视觉断言。

## 追加：下拉的重新设计（R0–R4，2026-11-12）

> 需求：规格的 **S18、S19**，以及改写过的 S2、S4、S12 与 §4、§7.3、§12–§14。原因：用户用过第一版后说系统的下拉控件「让人以为是刷新消息」，要求重新设计并与他讨论；讨论结果（自绘、安静款、松手留一行空地、文字 shimmer、请求不动）已写进规格，图稿在 `docs/assets/ios-session-history/`。
> 状态：规格与本节在评审中，**代码还没动**。只改手机界面：不碰线协议、Mac 端、`SessionThread` 的取数逻辑；`PROTO` 与 `PROTOCOL_VERSION` 都不动。

### 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| R0 | 规格与本节（含两张图稿） | 用户评审通过后提交 |
| R1 | **原型关卡（决定性）**：只在手机界面里、用 Mock 场景，验证「自绘下拉头 + 松手持住一行空地 + 历史到达时请求不动」，选定做法 | 原型的 UI 测试（含逐帧采样）+ 模拟器上手看；结论写进下面的「R1 结论」 |
| R2 | 实现：下拉状态机（纯类型）、下拉头与持住、顶端状态行、`ShimmerText`（含菜单胶囊）、去掉 `UIRefreshControl`、本地化 | `swift test`；`xcodebuild build` → `make-catalog.py` |
| R3 | UI 测试改写与验证：会话历史类在 iPhone iOS 18.5（en + zh-Hans）、iOS 26.2、iPad、Duo 外屏；相关既有类回归；给用户看的真实截图序列 | `xcodebuild test`（模拟器，前台跑） |
| R4 | 文档同步：总览、使用说明（中英）、`PROGRESS.md`、本节的实施记录 | 提交 |

R1 不过关不进 R2。

### R1 细节（原型）

要验证的只有一件事：**在 SwiftUI 的 `ScrollView` 上自绘下拉、持住一行空地，历史到达时请求纹丝不动。** 方案先试 A；A 不过再试 B；都不过关就停下来问用户。

- **A：用 `contentInset.top` 持住**（自绘下拉刷新的经典做法，回弹交给系统的弹簧，不用自己做动画）。
  - **手势**：给滚动视图自己的 `panGestureRecognizer` 加一个 target（开始 / 变化 / 结束），再加已有的 `contentOffset` KVO；越界量 = `-(contentOffset.y + adjustedContentInset.top)`，只在 `isTracking` 时计。
  - **下拉头**：UIKit 子视图（图标 + 标签），位置在内容原点上方 `[-H, 0]`，随内容一起动，不比内容晚一帧；静止时透明度为 0（否则会从半透明的导航栏后面透出来）；颜色用动态色（`secondaryLabel` / `label`）。
  - **松手已过阈值**：在手势结束的回调里用 `UIView.animate` 把 `contentInset.top` 加上 H，页面弹到「空地」处停住；下拉头换成「正在加载历史…」（shimmer 用 `CAGradientLayer` 做遮罩动画，或承载 SwiftUI 的 `ShimmerText`，原型里选省事的）。
  - **到达**：沿用现在的等待（页面停住，手指抬起后至多 1.5 秒，手指不放最多 10 秒）→ `prepareInsertion()` → 去掉下拉头、`contentInset.top` 还原 → 线程插入；补偿沿用 `reconcile()`（锚点是 `request-top`）。顶端状态行的最小高度 = H，所以失败 / 读到头时偏移刚好落在静止位置，不需要动画。
  - **要最先验证**：SwiftUI 会不会把 `contentInset` 改回去（会，就和 `alwaysBounceVertical` 一样用 KVO，被改掉的那一刻放回去）。
- **B：松手即把空地做成页面内容**：松手已过阈值的同一帧里，顶端状态行以 H 高度进入内容，`contentOffset` 同时补偿 H（屏幕上不跳）；用 `setContentOffset(_, animated: false)` 停住系统的回弹，再自己用弹簧把页面送到静止位置。比 A 更「SwiftUI」，但要接管回弹的手感，排第二。

**判据**（全部满足才算过；逐帧采样沿用 P1 的做法，运行循环 observer 挂在 CA 提交之后，采样请求锚点相对视口的位置，用完拿掉）：

1. 松手已过阈值：页面停在「留出一行」的位置，`request-title` 比拉之前低 H（±1 pt），停住前后没有闪动或回跳；停住之后手指拖动页面，空地与请求一起动，滚走再滚回来都正常，页面不被锁住；
2. 到达（页面静止在顶部）：采样的每一帧里请求锚点相对视口的位置都不变，最新一条往来的下沿与「当前提问」分隔线出现在空地里；
3. 到达时页面不在顶部（用户已经往下滚了）：屏幕上看到的东西一点没变；到达时手指按着或页面还在滑动：等停下再换，之后同 2；
4. 失败与读到头：那一行与空地等高、请求不动；点「重试」，同一行回到加载状态；
5. 没过阈值松手：不加载、没有空地、页面照常弹回；下拉头只在手指按着时出现，惯性冲出去的越界与程序滚动都没有；
6. 短页、长页、键盘弹起、iPad、折叠屏外屏都能拉起并持住；SwiftUI 的更新（`@Observable` 变化、键盘出入、「下一题」）不会把 `contentInset` 或持住的位置冲掉；
7. 与阅读位置还原（`RequestScrollPosition`，含 `intent`）、「下一题」、已结束页的「停留」（S13）、「回到当前提问」胶囊互不干扰；
8. 「减弱动态效果」开 / 关各一遍。

**产出**：在本节末尾写「R1 结论」（选 A 还是 B、踩到的坑、定下来的数字）；原型代码留作 R2 的起点（去掉临时插桩）。**A、B 都不过关**：停下来用 AskHuman 问用户（会动 S19，不自行改方案）。已知的备选是「松手后页面完全回弹、空地再以动画展开」：文字会先消失再出现，与用户要的「文字留在那」不一致，所以只能由用户来定。

### R2 细节（实现）

1. `AHCore/HistoryPull.swift`（新，纯值类型，不依赖 UIKit）：输入 `update(overscroll:tracking:enabled:)` 与 `released()`；输出阶段（`idle / pulling(progress) / armed / holding`）、下拉头的透明度与图标旋转量、「刚从下往上越过阈值」事件（轻震）、松手时是否触发。阈值、死区、淡入距离是常量，原型里调定后写回规格 S18。单元测试 `AHCoreTests/HistoryPullTests.swift`。
2. `Views/Request/PullHeader.swift`（新）：下拉头的 UIKit 视图（图标 + 标签，动态字体、动态色），对 VoiceOver 隐藏。
3. `Views/Request/ShimmerText.swift`（新，SwiftUI）：灰字加一道主文字色高光循环扫过；「减弱动态效果」时是静态的灰字。`SessionLoadingPill`（菜单入口的胶囊）的字改用它。
4. `SessionThreadView` 的顶端：`SessionStateRow`（加载中 / 错误 / 没有更早的记录，三态同一个最小高度，`@ScaledMetric`）取代现在的 `topRow`；点「重试」时同一行回到加载中。（读代码看，第一版里重试一开始错误行就收起、下面的内容会跟着动，没有实测；三态同高就不存在这个问题。）
5. `ThreadScrollCoordinator`：去掉 `UIRefreshControl` 与它的 `refreshControl` KVO（`alwaysBounceVertical` 的 KVO 保留）；接上手势、状态机、轻震（`UIImpactFeedbackGenerator(style: .light)`）、持住与放开；`RequestHistory.pull()` 改成松手触发，`loadFinished()` 改为放开持住。`farAbove`、菜单入口、`retry()`、`returnToCurrent()` 不变。**不动 `RequestScrollPosition`**。
6. 无障碍与测试挂钩：下拉头对 VoiceOver 隐藏；加载开始时 `UIAccessibility.post(.announcement, …)` 读出「正在加载历史」；留一个只在状态变化时更新的无障碍元素（`session-pull-state`，值是 `idle / pulling / armed / loading`），UI 测试靠它读状态，不依赖 UIKit 子视图是否出现在 SwiftUI 的无障碍树里。
7. 文案（英文为源语言，补 zh-Hans）：「Pull down to see this session's history」「Release to see this session's history」「Pull down to load earlier records」「Release to load earlier records」；既有的「Loading history…」「No earlier records」「Can't reach the Mac」「Try again」不变。`ios/Tools/make-catalog.py` 缺中文会失败。新增 Swift 文件后在 `ios/` 下 `xcodegen generate`。

### R3 细节（验证）

- **UI 测试**（`AskHumanSessionHistoryUITests`）：`pull(from:to:inWindow:)` 留着；断言改成：松手后请求比拉之前低一行空地（±2 pt），历史到达前后请求位置不变（±1 pt）；没拉到阈值松手不加载、不留空地；空地里是「正在加载历史…」（`session-pull-state` 为 `loading`）。Mock 的延迟用新增的启动参数 `-AHHistoryDelay <秒>` 拉长（只给会话历史用），好让「持住」的那段能被看到。键盘弹起的那条在 iPad 上不再跳过（阈值是我们自己的）；iPad、最大字号、深色各一条照旧。
- **验证矩阵**：iPhone 16 / iOS 18.5（en + zh-Hans）、iPhone 17 / iOS 26.2（临时建，跑完删）、iPad Pro 11 / iOS 18.5、Duo 外屏 / iOS 27.2（只看下拉相关的几条，其余是已知的外屏假设）。回归只跑与线程和页面滚动相关的类：`AskHumanSessionHistoryUITests`、`AskHumanEndedUITests`、`AskHumanRequestNavigationUITests`、`AskHumanLargeScreenUITests`（这次不动共享的 `RequestScrollPosition`，范围比第一版小）。`swift test` 全跑。
- **给用户看的东西**：UI 测试里在三个时刻截图（拉过阈值、持住加载中、到达），拼成一条序列用 AskHuman 发给用户；轻震和真实手感只能在真机上验，模拟器没有触觉。
- 构建与测试前先 `node scripts/machine-health.mjs`；用独立的 `-derivedDataPath`（`build/DerivedDataPull`），不与同一棵树里别的会话互相覆盖。

### R4 细节（文档同步）

`docs/overview-ios.md`「请求页的会话历史」一条（现在写着 `UIRefreshControl` 与它的两处 KVO）、`docs/wiki/ios-setup.md` 与英文版（下拉的样子）、`docs/PROGRESS.md`（真机那一条里与系统控件触发距离、转圈时长有关的话改成新的验收点：阈值与空地的手感、轻震、shimmer）、本节的实施记录。

### 提交（Conventional Commits）

1. `docs(ios): redesign the session history pull in spec and plan`（R0，含图稿）
2. `feat(ios): draw the session history pull ourselves instead of the refresh spinner`（R1–R3，代码与测试；`Release-Note: skip`——第一版没发布过，用户看不到转圈版，更新说明里已有「加载会话更早的回答」那一条）
3. `docs(ios): sync overview, wiki and progress with the history pull redesign`（R4）

### 风险

1. **SwiftUI 与 `contentInset`**（R1 第一件事）：SwiftUI 管理滚动视图的内边距，持住用的 `contentInset.top` 可能被改回去；用 KVO 放回，不行就走方案 B，再不行问用户。
2. **自己跟踪手势**：iOS 18.5 / 26.2 / 27 上 `panGestureRecognizer` 的状态与 `isTracking` 的时序可能略有差异；状态机是纯类型、单测覆盖，手势这层的接线靠各个运行时上的 UI 测试。
3. **UIKit 下拉头不一定出现在 SwiftUI 的无障碍树里**：UI 测试不依赖它，读 `session-pull-state`（R2 第 6 条）。
4. **下拉头从导航栏后面透出来 / 静止时露出一条边**：静止时透明度为 0，随拉出的距离淡入，R1 的判据 5 守着。
5. **轻震与「手感」只能在真机上验**：阈值（约 72 pt）、空地（约 64 pt）都是初值；真机一轮由用户来看，不满意就改常量，不动结构。
6. **主工作树里还有别的会话在改 iOS 的「流量统计」**（`Messages.swift`、`Localizable.xcstrings`、`PROGRESS.md` 等已有未提交的改动，仍在变）：只提交本功能的文件；与它共用的文件（字符串目录、`PROGRESS.md`）用 `git apply --cached` 只暂存自己的那几处，不夹带对方的改动；编译前看有没有别的 `xcodebuild` 在跑，没有再 `xcodegen generate`，也不与对方的模拟器抢同一台设备。

### 开工前定下的事

- **执行位置**：沿用用户 2026-11-11 选的主工作树（见上文「开工前定下的事」），不再问。
- **TestFlight**：不主动出（用户要时按 `ios-testflight` skill，默认同时发 Internal 与 Friends）。
- **R1 没过关就不进 R2**；A、B 都不过关就问用户。
