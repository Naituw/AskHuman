# 需求：iOS 请求页查看「此会话的历史问答」（下拉加载 / 更多菜单）

> 状态：**2026-11-11 起草并经用户评审通过，尚未实现**（用户在 AskHuman 里逐项确认了 S6、S10、S13 并保留「回到当前提问」胶囊，见 §14）。用户原话：「我希望能更容易看到一个会话的历史记录。在 iOS 的 APP 上，比如说收到个消息，我是不是可以下滑去下拉刷新加载历史的回答记录？我刷新之后，我继续往上滑，看到的就是之前的回答。然后从更多菜单也可以进入。你觉得怎么样？帮我设计一下。」
> 经一轮 AskHuman 问答，用户定了 S1–S4（读 Mac 的完整历史；更多菜单等价于下拉；只做作答页 / 确认页，含已结束页；不做任何提示），并补充：「这个也不是一个刚需功能，就是一个 Power User 功能。一般的用户也可以在更多菜单里找到。」其余决策（S5–S17）是我按默认定的，评审时用户没有否决；标 ⚠ 的 S6、S10、S13 与上一轮口头方案不同，用户已明确点头。
> 关联计划：`docs/plans/ios-session-history.md`。落地后同步修订：`docs/overview-ios.md`（能力位与新消息）、`docs/specs/ios-channel.md`（§6 手机端功能）、`docs/specs/ios-answer-flow.md`（§3.8 E12 的打断源）、`docs/specs/reply-history.md`（一句交叉引用）、`docs/wiki/ios-setup.md`（可选的一句话）。
> 前置资料：`docs/specs/reply-history.md`（Mac 历史记录什么、会话身份 F1–F12）、`docs/specs/ios-todos.md`（同一类「请求 / 应答」读取消息的写法与限额）、`docs/specs/ios-answer-flow.md`（请求页、已结束页、5 秒倒计时）、`docs/overview-ios.md`。
> 平台：Mac 端沿用现有 Daemon（代码跨平台，macOS 验证）；手机端 iPhone（iPad 随既有大屏布局），iOS 17+，中 / 英双语。

## 1. 背景与目标

收到 Agent 的追问时，最常见的需求是「我上次是怎么答的」。手机上现在做不到：

- 「历史」标签只有**这部手机见过**的请求；在 Mac 弹窗 / IM 里答掉的题，手机只知道「已在 X 回答」，**没有答案内容**（`HistoryEntry.answers` 只含这部手机发出的）；
- 它也不按会话分组，请求页上看不到「这个会话之前聊过什么」。

Mac 端的 `history.jsonl` 已经按请求保存了完整的问答（任何渠道作答的真实答案、Agent 会话 id），缺的只是一条「按会话读取」的通路。

**目标**：在作答页与确认页（含已结束页）上，下拉即加载**这个 Agent 会话更早的问答**，显示在当前提问的上方，继续上滑就能读；「⋯」菜单里有一个等价入口。

**定位：Power User 功能**（用户语）。不做任何提示、不加任何设置项、不改变页面默认外观；不用这个功能的人什么都察觉不到。

**非目标**（§11 有后置理由）：

- 手机本地「历史」标签不改（仍是本机的、按条平铺）；
- 「历史」标签的详情页、Agents 详情页不提供这个下拉；
- 不显示 Agent 的原始对话（transcript），只有 AskHuman 的问答；
- 权限审批不在线程里（Mac 历史本来就没记它们）；
- 不搜索、不跨会话；
- 不落盘、不离线（需要 Mac 在线）；手机不能删除 Mac 上的历史；
- MCP 兜底分区（Mac 历史窗口里「近似」的分组）与无会话信息的请求，本期没有入口（S6）。

## 2. 术语

| 术语 | 含义 |
|---|---|
| 会话键 `agentKey` | `{kind}:{sessionId}`，与 `AgentView.key` 是同一个字符串（Mac 的 `ios_agents::agent_key`）；手机本来就能在 Agents 列表里看到它 |
| 往来（exchange） | Mac 历史里的一条记录：一次提问加它的回答。记录的 id 就是请求 id，每个请求至多一条 |
| 线程（thread） | 请求页上方、当前提问之前的往来时间线，旧的在上、新的在下 |
| 锚点 | 线程只含「早于当前请求创建时刻」的往来：当前请求自己的那条（答完的请求在 Mac 历史里也有）和更晚的往来都不在里面 |
| 游标 | `(beforeMs, beforeId)`：只要排序键 `(timestampMs, id)` 小于它的往来；首次的 `beforeMs` 是当前请求的 `createdAt`，之后是线程里最旧一条 |

## 3. 决策记录

| 编号 | 决策项 | 结论 | 理由 / 否决后的代价 |
|---|---|---|---|
| S1 | 数据来源（用户定） | 读 Mac 的完整历史（`history.jsonl`），手机不另存 | 别处作答的真实答案只有 Mac 有；手机本地历史不完整，也没有会话归属。代价：需要 Mac 在线，Mac 与手机都要更新（`caps.sessionHistory`）；老 Mac 上没有入口 |
| S2 | 「更多」菜单（用户定） | 「此会话的历史」等价于下拉：加载最近一页并**滚到最新一条往来**（已加载过则只滚动）；不做单独的全屏页 | 同一套线程、一份实现。代价：不能搜索、不能一次看全（§11） |
| S3 | 支持的页面（用户定） | 作答页、确认页，含它们的已结束态；不含「历史」标签详情页与 Agents 详情页 | 收到消息是主场景；其余后置 |
| S4 | 可发现性（用户定） | **不做任何提示**：没有一次性小提示，没有常驻提示行。只有下拉手势与「更多」菜单项 | 页面保持原样；VoiceOver 与不知道手势的人走菜单项 |
| S5 | 会话键放在请求里 | `RequestView` 新增可选 `agentKey`（§5.1）；不放进 `RequestSummary`（收件箱行用不到） | 手机缓存的请求带着键，Mac 忘了这条请求（已结束、重启后）也能查；沿用 `AgentView.key` 的既有格式，手机本来就看得到，没有新增信息 |
| S6 ⚠（用户 2026-11-11 确认） | 范围：只认真实 Agent 会话 | 本期只对「Mac 知道真实会话 id」的请求提供，即现在能显示会话标题的那一批。MCP 实例兜底分区与无会话信息的请求没有入口（线程区、菜单项、下拉都不出现） | **与上一轮方案相比是收窄**（上一轮写的是「MCP 兜底会话标『近似』」），用户已同意。原因：iOS 请求现在只携带真实会话（`SessionRef`），兜底分区要把 MCP 实例 id 与项目穿过 4 处投递点并新增一种键；而它本来就是「近似」的，会把不同对话混在一起。代价：只有 MCP 兜底的调用方（拿不到真实会话 id 的）用不了这个功能 |
| S7 | 锚点与分页 | 手机用 §2 的游标向更早翻页：每页默认 8 条、最多 20 条；Mac 按字节预算少给也行，`more` 说明后面还有没有 | 用时间加 id 的游标而不是偏移量：新往来加在「新」的一端，偏移会错位；排序键 `(timestampMs, id)` 降序，同毫秒也不重复、不遗漏 |
| S8 | 只含已完成的往来 | 只取 `action = Send` 的记录（与 `show-last` 同一口径）；用户主动取消的不列。离开 / 静音模式的自动回复（`channel` 为 `away` / `mute`）要列出，弱化显示 | 「我答过什么」是核心；自动回复能解释「Agent 为什么自己往下走了」。列出取消的往来可以后置（§11） |
| S9 | 往来的内容 | 只带提问与**已选 / 已写的答案**，不带没选的选项；附件、图片、批注只给个数；不带任何路径 | Mac 历史里的 `selectedOptions` 是文本，没选的选项既对不上号也不是这里要看的；体积小，一页几乎总能装进一条消息；不暴露路径（沿用 ios-todos I6 的姿态） |
| S10 ⚠（用户 2026-11-11 确认） | 展开方式 | 点卡片**就地展开 / 收起**（原生 Markdown 渲染全文），不跳详情页 | **与上一轮「点开进只读详情（复用历史详情页）」不同**，用户已同意。原因：`HistoryDetailScreen` 吃的是手机本地的 `HistoryEntry`（带 `AskView` 与选项下标），与 Mac 的往来对不上；就地展开不新增页面、不新增 WebView，也更符合「往上滑着读」 |
| S11 | 渲染 | 旧卡片用原生 `MarkdownView`（块渲染、有缓存）；**不新增 WebView**，当前提问的消息仍是这一页唯一的 WebView | 一页一个 WebView 的既有约束 |
| S12 | 视口不跳 | 下拉加载后，在上方插入线程，用户正在看的内容（当前提问）不动；继续上滑才看到旧问答。菜单入口则滚到最新一条往来 | 用户原话「刷新之后继续往上滑」。实现上复用 `RequestScrollPosition` 的锚点机制，先做原型（§7.3） |
| S13 ⚠（用户 2026-11-11 确认） | 已结束页与 5 秒倒计时 | 在已结束页上加载历史（下拉或菜单）等于点了「停留」：取消该页的倒计时 | 上一轮的默认，用户已确认。它**增加了一个打断源**：`ios-answer-flow.md` E12 列的打断源不含这一项，「手指碰屏幕不算停留」的决定不变——这里是明确的加载动作，不是碰屏。落地时同步改 E12。否则翻到一半会被自动带走 |
| S14 | 不落盘 | 线程只在页面存活期间的内存里；切到别的请求再回来要重新加载 | 与 ios-todos I13 同一立场；Mac 读取很快 |
| S15 | Mac 离线 / 无回应 | 线程区顶端出一行「连不上 Mac」加「重试」；不回退到手机本地历史 | 两处来源不同，混用会让人以为这就是完整的 |
| S16 | 能力协商与协议号 | `Caps.sessionHistory`（缺省不发，沿用 `annotations` 的写法，夹具里既有能力集的形状不变）；新消息 `history.session.get` / `history.session`；`PROTO`（线协议）与 `PROTOCOL_VERSION`（IPC）都不动 | 加可选字段与新消息类型，旧对端靠能力位与 `unknown_type` 兜底；规则见 `docs/overview.md`「IPC 协议号」与 `docs/overview-ios.md`「不变量」 |
| S17 | 无设置项 | 不加 Mac 设置，也不加手机设置 | Power User 功能 |

## 4. 用户视角

**入口**（两个，等价）：

- 在作答页或确认页，页面停在顶部时**下拉**；
- 「⋯」菜单 →「此会话的历史」。

**下拉**：转圈约 0.3 秒，最近 8 条往来出现在当前提问**上方**，页面仍停在当前提问的顶部（视口不动）。继续上滑，看到更早的往来；再到顶再下拉，加载更早的一页。没有更早的了，线程顶端写「没有更早的记录了」（Mac 只保留最近 N 条，所以不说「第一条」），之后不再请求。

**菜单**：与下拉等价，但加载完会**滚到最新一条往来**，已加载过则只滚动。没有更早的记录时，滚到顶并显示「没有更早的记录了」。

**线程的样子**（示意；当前提问在最下面）：

```
┌ 导航栏：‹ 会话标题              ‹ ›  ⋯
│
│  没有更早的记录了                      ← 仅在确认没有更早的时候
│
│  ┈┈ 昨天 21:40 · Mac 弹窗 ┈┈
│  Agent：我准备把缓存层改成……（最多约 4 行，渐隐）
│  1 缓存要不要改？
│    ✓ 方案 B；先别动 schema
│
│  ┈┈ 今天 09:12 · 离开模式 · 自动回复 ┈┈   ← 整张弱化
│  ……
│
│  ━━━━━━ 当前提问 ━━━━━━
│  [头部] [消息] [问题卡……]               ← 原来的页面，一点不变
│
└ 底栏：已答 0/2                 [发送]
```

**卡片（收起）**：头部一行「相对时间 · 渠道名」；Agent 的消息（原生 Markdown，最多约 4 行，超出渐隐，没有消息时省略）；每题两行：题干（1 行，截断）与「✓ 已选项；写的文字」（至多 2 行），没答的题写「未作答」；多题时最多显示前 2 题，其余写「还有 N 题」。
**卡片（展开）**：点一下展开，全文消息与全部题（题干全文、已选项、文字回答），附件 / 图片 / 批注只写个数（「附件 2 个」）；再点收起。文字可选中复制。展开状态只在内存。

**渠道名**：`popup` Mac 弹窗、`dingding` 钉钉、`feishu` 飞书、`telegram` Telegram、`slack` Slack、`ios` iPhone、`away` 离开模式、`mute` 静音模式（后两者是自动回复）；不认识的 id 原样显示。

**回到当前提问**：向上滚得离当前提问超过一屏时，出现一枚悬浮胶囊「回到当前提问」，点一下滚回去。（这是最先可砍的一项：成本高了就不做，不影响其余；用户 2026-11-11 选择保留。）

**没有入口的情形**：请求没有 `agentKey`（S6）、这台 Mac 太旧（没有 `caps.sessionHistory`）、页面还只有摘要拼出的占位视图（完整视图到了才出现，§8）：没有菜单项，下拉也不出现转圈，页面与现在完全一样。

**错误**：Mac 离线、超时：线程区顶端「连不上 Mac」加「重试」（S15），不影响作答。

## 5. 线协议（`ios/wire.rs` 与 Swift `AHProtocol` 共用 `ios/Fixtures/wire-v1/messages.json`）

全部是加字段 / 新消息类型，JSON 键 camelCase，可选字段缺省不发。

### 5.1 请求上的会话键

`RequestView` 新增可选 `agentKey`：提问的 Agent 会话的键（`{kind}:{sessionId}`，与 `AgentView.key` 逐字相同）。Mac 不知道真实会话（没有 `SessionRef`）时不发。作答页与确认页的请求都带它。

### 5.2 能力位

`hello.ack` / `pair.accept` 的 `caps` 增加 `sessionHistory`（缺省 false、为 false 时不发）。

### 5.3 消息

| 方向 | 类型 | 主要字段 |
|---|---|---|
| 手机 → Mac | `history.session.get` | `reqId`（手机生成，应答原样带回）、`agentKey`、`beforeMs`、`beforeId?`、`limit`（Mac 夹到 `1…SESSION_HISTORY_PAGE_MAX`） |
| Mac → 手机 | `history.session` | `reqId`、`agentKey`、`items: [Exchange]`（新的在前）、`more`（最后一条之前还有没有） |

`beforeMs` / `beforeId` 是 §2 的游标：只要 `(timestampMs, id)` 小于 `(beforeMs, beforeId)` 的往来；没有 `beforeId` 时是 `timestampMs < beforeMs`。`items` 可以比 `limit` 少而 `more` 仍为真（页字节预算用完）。键不合法、会话没有记录、Mac 的历史关着（`general.historyLimit = 0`）时，都是空 `items` 加 `more = false`，不是错误。

**Exchange**：

| 字段 | 含义 |
|---|---|
| `id` | 往来 id（即请求 id） |
| `at` | 记录的时刻（`timestampMs`，回答被记录的那一刻） |
| `channel` | 作答渠道 id（原样，见 §4 的渠道名） |
| `markdown` | 消息与题干是否按 Markdown |
| `message` | Agent 的消息，裁剪到 `HISTORY_MESSAGE_MAX_CHARS` |
| `attachments?` | 消息带的附件个数（只有个数，没有名字和路径） |
| `qs` | 每题一项：`text`（题干，裁剪到 `HISTORY_QUESTION_MAX_CHARS`）、`selected`（已选项文本，每个裁剪到 `HISTORY_OPTION_MAX_CHARS`）、`input?`（写的文字，裁剪到 `HISTORY_INPUT_MAX_CHARS`）、`images?` / `files?`（回复带的图片 / 文件个数） |
| `notes?` | 批注个数 |

第 i 题的答案取 `answers[i]`（自动回复只有一条答案，落在第 0 题）。文字被裁剪时以 `…` 结尾。

**限额**（`ios/wire.rs` 与 Swift `Wire` 必须一致）：`SESSION_HISTORY_PAGE = 8`（手机默认）、`SESSION_HISTORY_PAGE_MAX = 20`、`SESSION_HISTORY_BYTES = 262144`（一页 JSON 的预算）、`HISTORY_MESSAGE_MAX_CHARS = 4000`、`HISTORY_QUESTION_MAX_CHARS = 1000`、`HISTORY_OPTION_MAX_CHARS = 500`、`HISTORY_INPUT_MAX_CHARS = 2000`。整条消息不能超过 `MAX_ENVELOPE_BYTES`（`send` 对超限消息是静默丢弃，必须在构造时避免）。

超预算时的收缩与 `todo.list` 同一套（`shrink`）：先少给最旧的往来（`more` 保持为真，手机下次从新的游标继续）；只剩一条还超，就把它的各段文字减半，直到装下。

## 6. Mac 端行为

### 6.1 请求带键

`IosChannel::start`（`channels/ios.rs`）在写 `session_title` 的同一处写 `agent_key`；确认请求在 `channels/confirm.rs` 写 `session_title` 的同一处写。取值来自 `SessionRef`，格式化函数与 `ios_agents::agent_key` 是**同一个**（挪到两边都能用的地方），保证与 `AgentView.key` 逐字相同。没有 `SessionRef`（无 Agent，或没有真实会话 id）就不写。

### 6.2 读取

`Host` trait（`ios/gateway/mod.rs`）新增 `async fn session_history(&self, agent_key, before, limit) -> wire::SessionHistory`，默认实现返回空页（所以只实现自己要的宿主不受影响，与 `todo_list` 同）。Daemon 的实现在新文件 `daemon/runtime/ios_history.rs`：

1. `ios_agents::split_key` 解析会话键，失败返回空页；
2. 调 `history.rs` 的新函数 `sends_for_session_before(agent_kind, session_id, before, n)`：口径与 `recent_sends_for_session` 相同（`action == Send`，`agent_kind` 与 `agent_session_id` **精确相等**，即 `reply-history.md` F1 的「真实 session」；不碰 MCP 分区），但按 `(timestampMs, id)` 降序，取游标之前的 `n + 1` 条，多出的那一条只用来得出 `more`；
3. 投影成 `Exchange`（裁剪文本、只留个数）；
4. 整个读取在 `spawn_blocking` 里做（要读整份 `history.jsonl`，上限默认 200 条）。

读取是只读的，不进幂等日志，不订阅，不广播。

### 6.3 网关

`ios/gateway/history.rs`（新）的 `on_session_history_get`，与 `on_todo_list_get` 同样用 `spawn_then`：把 `limit` 夹到范围内，调 Host，按 §5.3 收缩后发回，应答回显 `reqId`。老手机不会发这条消息。

### 6.4 能力位

Daemon 的 Host `caps()` 返回 `session_history: true`。

## 7. 手机端行为

### 7.1 入口条件

`item.view?.agentKey != nil` 且这台 Mac 的 `caps.sessionHistory == true`（`macs[macId]?.caps?.sessionHistory`）。不满足时请求页与现在完全一致：没有菜单项，也没有下拉的转圈。`RequestScreen.hasMore` 把这个条件算进去（只有这一项时菜单也要出现）。

### 7.2 模型与通路

- `SessionThread`（`AHCore`，`@MainActor @Observable`，由请求页（`RequestScreen`）持有，**不进 `AppStore` 的缓存**：页面随请求切换重建时一并丢掉，不落盘，即 S14）：状态 `idle / loading / failed / ready`、`reachedStart`；`items`（旧到新）；`loadMore()`（首次与翻页共用一个入口）；同一时刻只有一个在途请求（重复触发忽略）；游标由线程里最旧一条决定，首次用当前请求的 `createdAt`。
- `Backend` 新增 `sessionHistory(macId:agentKey:beforeMs:beforeId:limit:)`（`AHCore/Backend.swift`）。`PhoneEngine+SessionHistory.swift` 用与 `PhoneEngine+Todos.swift` 相同的「信箱 + 超时」（键为 `reqId`，超时 10 秒，沿用 `todoReadTimeout`）；离线、能力位缺失分别抛 `macOffline`、`rejected("unsupported")`。
- `AHMock` 有对应的 `MockBackend+SessionHistory.swift` 与一个带多页历史的场景；`AHCoreTests` 有 `TestMac+SessionHistory.swift`（与 `TestMac+Todos.swift` 同款）。

### 7.3 页面集成与滚动

- `AnswerScreen` 与 `ConfirmScreen` 现在是 `ScrollView { VStack { … }.background(RequestReadingAnchor(name: "request-top")).background(RequestScrollPosition(…)) }`。改成：滚动内容外层再包一层 `VStack`：`[线程区][当前请求区]`，**原来的 VStack 连同 `request-top` 锚点与 `RequestScrollPosition` 整个留在「当前请求区」**，所以阅读位置的保存与还原、「下一题」、问题卡的锚点都不受影响。线程区为空时高度为 0。
- 新文件 `Views/Request/SessionThreadView.swift`：卡片、转圈行、错误行、「没有更早的记录了」行、「回到当前提问」胶囊。
- 「⋯」菜单（`RequestScreen.moreMenu`）加一项「此会话的历史」（`clock.arrow.circlepath`，标识 `request-menu-session-history`）：线程未加载就加载并滚到最新一条往来；加载过就只滚动；失败显示 S15 的错误行并滚到顶。
- **滚动原型先行**（计划阶段第一件事）：验证三件事：(a) 下拉怎么触发（系统 `refreshable`，还是在 `RequestScrollPosition` 里自己判断过度下拉）；(b) 前插之后可见内容不动（`contentOffset` 补偿等于新增内容高度，在同一次布局里完成，不闪）；(c) 与阅读位置还原、键盘的交互式收起、「下一题」的滚动互不干扰。原型不过关，不自行改方案：回来问用户（因为会动 S2 / S12）。

### 7.4 与已结束页的接缝

在已结束页上加载历史（下拉或菜单）时，调 `AppStore.stayOnEnded(_:)`，等同于点了「停留」：取消该页的 5 秒倒计时（S13）。其余打断规则（E12）不变。

### 7.5 其它

- 本地化：英文为源语言，补 zh-Hans，沿用 `Localizable.xcstrings` 的生成脚本。
- 无障碍：每张卡片是一个合并的读屏元素（时间、渠道、消息、各题与答案）；VoiceOver 用户用菜单项（下拉对他们不可用）。
- 大屏：线程在请求页内部，`ColumnsOrStack` 不受影响。
- 深色、最大字号、iPhone SE 宽度下卡片不截断、不挤成几个字宽。

## 8. 兼容

| 组合 | 表现 |
|---|---|
| 新手机 + 旧 Mac | 旧 Mac 不发 `caps.sessionHistory`，也不发 `agentKey`：没有入口，什么都不变 |
| 旧手机 + 新 Mac | 旧手机忽略 `agentKey` 与能力位，不会发新消息：什么都不变 |
| 新 + 新 | 带 `agentKey` 的请求有入口 |

`RequestView` 在每次（重新）投递时重建（恢复快照里只存各手机的投递状态，不存视图），所以 Mac 升级后仍在途的请求也会带上 `agentKey`。`content_hash` 是对整份序列化视图算的（`ios/gateway/requests.rs`），加了字段后新哈希不同于手机缓存的旧视图；手机按既有规则（`PhoneEngine+Requests.swift`：摘要哈希与缓存视图不同就 `request.get`）重拉一次，无害。收件箱摘要里没有 `agentKey`，所以请求页在只有摘要拼出的占位视图时没有入口，完整视图到了才出现。`PROTO`、`PROTOCOL_VERSION` 都不动。

## 9. 隐私与安全

- 已配对的手机本来就是 Mac 的遥控端，能收到所有提问；这里新增的是**能回看 Mac 上保留的旧问答**（包括配对之前的）。传输仍是端到端加密（同 `todo.*`），不进推送、不进通知。
- 手机不写盘（S14），也不会拿到任何路径：往来里没有项目路径、附件路径、图片路径，只有个数（S9）。
- 会话键是手机给的任意字符串：Mac 只做 `split_key` 解析与精确相等匹配，不拼路径、不进 shell。
- 读取是只读的、有上限的（一页 ≤ 20 条，≤ 256 KB），不会被用来拖垮 Mac。

## 10. 已知局限

- **深度受 Mac 历史保留数限制**：默认全局 200 条（`general.historyLimit`，不是每会话），很忙的 Mac 上，较老会话的记录可能已被挤掉；Mac 上删掉的历史（历史窗口、清空）手机也看不到。
- `at` 是回答被记录的时刻，不是提问的时刻；锚点按当前请求的创建时刻切：同一会话里并行提问时，晚于它创建、早于它被看到才答完的往来不会出现在线程里（罕见，接受）。
- `channel` 为 `ios` 时不知道是哪部手机答的。
- 附件、图片、批注只给个数，不能在手机上看。
- 只列「发送」的往来，不列用户主动取消的（S8）。

## 11. 后置（本期不做）

- **MCP 兜底分区**（Mac 历史窗口里「近似」的分组）：要把 MCP 实例 id 与项目一路带到 iOS 的投递点，并在界面上明示「近似」（`reply-history.md` F2）。用户 2026-11-11 同意后置（S6），已在 `docs/PROGRESS.md` 记了一条。
- 「历史」标签详情页、Agents 详情页作为线程的宿主（Agents 要在 Agent 列表里带会话键）。
- 单独的全屏「会话历史」页（可搜索、一次看全）。
- 「历史」标签改读 Mac（按会话分组），把两处来源统一。
- 列出用户主动取消的往来；在手机上看往来的附件与图片。

## 12. 验收标准

1. 有 `agentKey`、Mac 有 `sessionHistory` 能力时，作答页与确认页（含已结束页）在顶部下拉出现转圈；加载后，当前提问的上方出现最近至多 8 条往来（新的在下），**当前提问在屏幕上的位置不变**；继续上滑看到旧往来。
2. 再到顶再下拉加载更早一页，不重复、不遗漏（同毫秒的往来也是）；没有更早的了，线程顶端写「没有更早的记录了」，之后不再发请求。
3. 「⋯」→「此会话的历史」：未加载时加载并滚到最新一条往来，已加载时只滚动，没有记录时滚到顶并显示「没有更早的记录了」。
4. 没有入口的情形（无 `agentKey`、老 Mac、只有占位视图时）：没有菜单项、下拉没有转圈、页面与现在一致。
5. 当前请求自己的往来与比它更晚的往来不出现在线程里；只出现这个会话的往来（`agent_kind` 与 `agent_session_id` 精确相等），不混入别的会话。
6. 卡片收起 / 展开如 §4；离开 / 静音模式的自动回复弱化显示；多题、未作答、很长的文字、没有消息只有题干，都显示正常；不新增 WebView。
7. Mac 离线或超时：线程区顶端「连不上 Mac」加「重试」，作答不受影响；重试成功后线程照常。
8. 在已结束页上加载历史取消 5 秒倒计时（等同「停留」）；不加载时倒计时行为不变。
9. 阅读位置还原、「下一题」、键盘的交互式收起、发送、草稿保存，在加载了线程与未加载时行为一致。
10. 深色、最大字号、iPhone SE 宽度、中英文、iPad 都正常；VoiceOver 能通过菜单项使用。
11. 老手机连新 Mac、新手机连老 Mac 都没有任何变化；`PROTO`、`PROTOCOL_VERSION` 不变；新增的线协议夹具两边往返测试通过，既有夹具原样保留。

## 13. 测试

- **Rust（`history.rs`）**：游标分页（同毫秒、跨页边界、`more`）、只取 `Send`、自动回复列出、精确会话匹配（别的会话与 MCP 分区的记录不混入）、空历史。
- **Rust（网关 / Host）**：`limit` 夹取、非法键、超预算收缩（先少给旧的，再减半文字）、`reqId` 回显、能力位只在 Host 声明时才发；`RequestView.agent_key` 在有 / 无 `SessionRef` 时的取值，并与 `AgentView.key` 逐字相同；投影的裁剪与个数。
- **线协议**：夹具往返（有 `agentKey` 与无 `agentKey` 两种请求、新旧能力集）。
- **Swift 包**：`SessionThread`（首页、前插顺序、耗尽、失败后重试、重复触发只发一次、游标推进）、`PhoneEngine`（应答路由、超时、离线、不支持）、已结束页上加载取消倒计时。
- **UI 测试（Mock 场景）**：下拉与菜单两个入口、前插后当前提问的位置不变（量锚点距离）、收起 / 展开、已结束页与倒计时、没有入口时页面不变、中英文、最大字号、深色。
- 滚动原型（§7.3）的结论写进计划。

## 14. 反馈意见

2026-11-11 用户评审（AskHuman 五问，全部选了推荐项）：

1. **S6 范围收窄**：同意。本期只做真实 Agent 会话，MCP 兜底分区后置（已记入 `docs/PROGRESS.md`）。
2. **S10 展开方式**：就地展开 / 收起，不跳详情页。
3. **S13 倒计时**：加载历史等同点了「停留」，取消倒计时；落地时同步改 `ios-answer-flow.md` E12。
4. **「回到当前提问」胶囊**：保留（仍是最先可砍的一项）。
5. **下一步**：规格通过，先写实现计划（含滚动原型方案），不动代码。

其余决策（S5、S7–S9、S11、S12、S14–S17）评审时没有被否决。
