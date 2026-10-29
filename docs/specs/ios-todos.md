# 需求：iOS App 管理待办（选 Mac / iCloud 视角、项目来源）

> 状态：**2026-10-29 起草并实现；全部决策由我（agent）在用户睡觉期间代为做出，待用户次日验收时审阅，可整条否决**（用户当晚留言：「我想给我们的 iOS 客户端添加待办功能的支持，我还没有考虑清楚。比如说是不是可以选择不同设备，然后管理上面的待办，也可以选择 iCloud 同步模式，然后可能还有项目来源的问题。这些你都帮我设计，然后实现验证，全部都能自己搞定。」）。没有经过 AskHuman 问答；所有选择都写在 §3，并给出理由与否决后的代价。实现过程中补充的细节（与起草时的差异）集中写在各节里，并在 §13 汇总。
> 追加（2026-10-29 晚）：用户反馈「待办里的状态比较混乱，如果我的 Mac 都开了 iCloud 存储，手机默认还是会选中一台 Mac，另一台 Mac 上的 Agent 就不会标绿点了」，随后经 AskHuman 两轮问答定案并实现了 I17–I19（来源合并、Agent 点、默认来源）；这三条由用户确认，不在上面「全部由 agent 代定」之列。见 §13 第 11 条。
> 关联计划：`docs/plans/ios-todos.md`。落地后同步修订：`docs/overview-ios.md`、`docs/specs/todo-sync.md`（§10 「iOS 网关的待办视图」、§12 「iOS App 读写待办」）、`docs/specs/ios-channel.md`（§6 手机端功能）、`docs/wiki/ios-setup.md`（用户说明）。
> 前置资料：`docs/specs/ios-channel.md`（Gateway、线协议、端到端加密）、`docs/specs/todo-sync.md`（项目身份、副本、仅云端项目、虚拟键 `icloud:<项目身份>`、§9 侧栏模型）、`docs/specs/todo-whats-next.md`（待办的本机语义）。
> 平台：Mac 端沿用现有 Daemon（macOS 验证；`todos.json` 的读写本身跨平台）；手机端 iPhone，iOS 17+，中 / 英双语。

## 1. 背景与目标

待办现在只能在 Mac 上用：待办窗口、CLI、IM、whats-next 的选项。手机已经能回答提问、看 Agent、新建任务（新建任务时能挑一条项目待办），但**不能看、不能加、不能整理待办**——出门在外想起一件事，只能等回到电脑前。

**目标**：iOS App 新增「待办」标签页，能对 Mac 上的待办做用户在待办窗口里做的那些日常操作：浏览项目、查看待办与执行历史、新增、编辑、勾选完成、恢复、删除、拖拽排序、切换 ⚡ 自动执行。

三个用户提出的设计问题（§3 逐条定案）：
1. **不同设备**：手机配对了多台 Mac，待办在哪台上管？
2. **iCloud 同步模式**：iCloud 待办同步（`docs/specs/todo-sync.md`）开着时，手机怎么用？
3. **项目来源**：手机上的项目列表从哪来？没有目录的项目怎么办？能不能在手机上新建项目？

**非目标**（§11 有后置理由）：附件的查看与添加；清空历史（iCloud 模式下是全局清空，手机上误触代价太大）；从 iCloud 移除项目；「全部待办」聚合页；手机直连 CloudKit 的 iOS 副本；离线编辑（手机离线或 Mac 离线时不排队，直接提示）；待办相关的推送通知。

## 2. 术语

| 术语 | 含义 |
|---|---|
| 来源（source） | 手机「待办」页现在看的是谁的待办：某一台 Mac，或「iCloud」 |
| 设备模式 | 来源是一台 Mac：只列**这台 Mac 上有目录的项目**（含本机独立的、无 remote 的项目），不列「仅云端」项目 |
| iCloud 模式 | 来源是「iCloud」：列出账号下**所有同步的项目**（含这台 Mac 没有目录的仅云端项目），由一台**开启了 iCloud 同步且在线**的 Mac 代为读写，那台 Mac 掉线时自动换另一台 |
| 接入 Mac | iCloud 模式下实际收发消息的那台 Mac（界面上写「通过 X 访问」） |
| 成员 Mac | 开着 iCloud 同步的 Mac。配对了多台 Mac 时，它们是「iCloud」的成员：不再单独出现在来源菜单里，菜单里「iCloud」的副标题列出它们（I17） |
| 项目 id | 项目 key（目录路径或 `icloud:<项目身份>`）的不透明哈希；手机看不到、也发不出路径 |
| 行 key | 项目行代表的 key：同一仓库的多个目录合并成一行时，取最近使用的那个目录（与待办窗口侧栏相同） |
| 版本（rev） | `todos.json` 的 mtime + 长度的签名，只用来判断「变了没有」，不是序号 |

## 3. 决策记录（均为 agent 代定，请审阅）

| 编号 | 决策项 | 结论 | 理由 / 否决后的代价 |
|---|---|---|---|
| I1 | 通路 | 沿用 iOS 渠道：手机 ⇄ Mac 网关（MQTT + 端到端加密），新增 `todo.*` 消息；Mac 的 `todos.json` 仍是唯一事实来源，手机只是遥控与展示端 | 不新增服务器、不新增凭据、与「权威状态在 Mac」一致；代价是 Mac 不在线时手机看不到待办（见 I3、§11 的后置项） |
| I2 | 不直连 CloudKit | 本期**不让手机直连 iCloud**（那会是「第三个副本」） | ① 需要给 iOS App ID 开 iCloud 容器权限并重签描述文件——这是 Apple 开发者账号上的改动，需要用户授权；② 要把 Rust 同步核心（HLC 字段级合并、顺序键、墓碑、本地变更检测）在 Swift 里再实现一遍并与 Rust 版逐位对拍，写错会污染用户**所有 Mac 共用的生产容器**，而当前没有安全的测试容器；③ 无法在无人值守时验证。设计上留好位置（§11） |
| I3 | iCloud 模式 | 「iCloud」是一个**来源**，不是另一条数据通路：手机挑一台开启了同步且在线的 Mac 当接入点，读写它的 `todos.json`，由已有的同步核心把改动带到 iCloud 与其他 Mac。接入 Mac 离线时自动换下一台；一台都没有时，手机上已经加载过的行**留着、只读**，页面顶部说明那台 Mac 暂时连不上（与选某台 Mac 做来源时一致）；没有行可留，或者在线的 Mac 都没开同步时，说明原因（没有 Mac 在线 / 没有 Mac 开启同步） | 用户要的「选 iCloud 同步模式」是「看到所有设备的待办、不用关心是哪台 Mac」，这能做到；比起让用户记住「家里那台开了同步」，自动换机更省心。代价：所有 Mac 都不在线时，手机只能看到这次启动里已经加载过的内容（I13 不落盘），重新启动后什么也看不到（与 I2 同源）。配对了多台 Mac 且不止一台开着同步时，iCloud 视图还会向其余同步 Mac 要一份各自「仅本机」的项目并进来（I17） |
| I4 | 设备模式 | 来源选某台 Mac 时，列表只含**那台 Mac 上有目录的项目**；开着同步的 Mac 上，「仅云端」项目只在 iCloud 模式里出现 | 设备模式回答「这台电脑上我有哪些待办」，iCloud 模式回答「我所有的待办」，两者含义不同才有区分的价值。若合并成一个列表，用户分不清哪些项目其实在这台机器上跑不了 ⚡、也没有目录。**配对了多台 Mac 时，开着同步的 Mac 没有单独的设备模式视图**（它的内容属于 iCloud，I17）；没开同步的 Mac 与「只配对了一台 Mac」时不变 |
| I5 | 项目来源 | 项目列表由 **Mac 计算**，与待办窗口侧栏同源：有待办的项目 ∪ 最近 workspace ∪ 活跃 Agent 的目录；iCloud 模式再并入同步项目（按仓库合并、仅云端、无待办的折叠段）。手机**不能新建项目**，也不显示路径，只显示名字与区分同名项目的后缀 | 项目 key 是目录，手机没有目录可选；让手机凭空编一个路径会写出没人用得到的垃圾 key。给「还没有待办」的项目加第一条待办，从「最近项目 / 无待办的项目」里点进去即可——只要那个仓库在 Mac 上被 Agent 用过，它就在里面 |
| I6 | 不暴露路径 | 线上项目用不透明 id（key 的 SHA-256 前 8 字节十六进制），与新任务的 `workspaceId` 同一做法；Mac 按 id 在当前已知 key 里反查 | 沿用 ios-channel 的既有安全姿态：手机丢了、中继或 broker 被攻破时，路径不会成为额外泄露面。待办正文本身仍是端到端加密的 |
| I7 | 能力协商 | `hello.ack` / `pair.accept` 的 `caps` 增加 `todos`（缺省 false）。旧 Mac 没有它：手机上该 Mac 显示「请先更新这台 Mac 上的 AskHuman」 | 加字段不改协议号（`PROTO` 仍为 1）；旧手机忽略未知字段 |
| I8 | 写入语义 | 所有写操作按待办 **id** 定位，带 `clientMsgId` 进幂等日志（重放返回第一次的结果，同 id 不同内容被拒绝）；结果里附上该项目的**最新完整列表**，手机直接替换。并发冲突（别处已删 / 已做）返回 `notFound` + 最新列表，手机显示一句提示后刷新 | 与回答、插话、新任务同一套机制；按 id 定位使「手机和 Mac 同时改」不会改错条目 |
| I9 | 乐观更新 | 手机先在本地改列表（勾掉、删掉、排好序），Mac 的结果到了再以它为准；失败则回滚并提示。同一项目有未完成的写入时，忽略期间到达的整表（避免界面抖回旧值），写入全部完成后采用最后一份 | 往返要 0.3–1.5 秒，不乐观更新会觉得「点了没反应」 |
| I10 | 实时性 | 「待办」页在前台可见时向 Mac **订阅**（`todo.watch`）；Mac 每秒比较 `todos.json` 的签名，变了就发 `todo.changed`（每部手机至多每秒一条），手机重拉项目列表与当前打开的项目。离开页面 / 退到后台即退订 | 在 Mac 上勾选、在 Agent 里被消费、iCloud 同步写入，手机上几秒内跟上；不订阅时 Mac 一个字节都不发、不查文件 |
| I11 | 新鲜度 | 订阅（或首次拉取）时，若开着 iCloud 同步，Mac 在后台做一次「过期才拉」（与待办窗口打开时相同的 `refresh_if_stale`）；拉到新内容写入 `todos.json`，由订阅推给手机。读取一个项目前先做本地阶段（`before_read`，只读本地、不联网），保证新 worktree 里不会出现「云端有、本地空」 | 复用 todo-sync §6 的既有保证，手机不另造一套 |
| I12 | 限额 | 单项目最多回 300 条待办 + 100 条历史（最新在前），附总数；项目列表最多 200 行；待办正文最长 4000 字符（超出被拒 `tooLong`，不静默截断）；整条消息不能超过 `MAX_ENVELOPE_BYTES`（Mac 先收缩列表再发） | 保证任何数据都能装进一条 MQTT 消息；`send` 对超限消息是静默丢弃，必须在构造时避免 |
| I13 | 不落盘 | 手机**不把待办内容写入磁盘**（只在内存里缓存，供同一次启动里切回「待办」页时立刻显示）；也不进推送、不进通知 | 待办可能含项目内幕；手机没有离线编辑，落盘缓存的收益小于泄露面。「来源」的选择（只是个标识）才存 `UserDefaults` |
| I14 | 界面 | 底部标签栏新增第 5 项「待办 / Todos」（`checklist`，放在 Agents 之后）；根页是项目列表（来源菜单在导航栏、按段分组、可搜索），点进项目是待办列表（滑动完成 / 恢复，点按编辑，拖拽排序，底部输入条新增，历史折叠区）。⚡ 开关只在有本机目录的项目里出现 | 与手机端既有的列表 / 详情结构一致；5 个标签在 iPhone 上仍可用 |
| I15 | 本地化 | 英文为源语言，补 zh-Hans；沿用 `Localizable.xcstrings` 的生成脚本 | 既有流程 |
| I16 | 不改 IPC | 不改 `ipc/mod.rs`，`PROTOCOL_VERSION` 不动；`PROTO`（线协议）不动 | 新增的是 iOS 线协议里的新消息类型，旧对端靠 `caps.todos` 与 `unknown_type` 错误兜底，规则见 `docs/overview.md`「IPC 协议号」与 `docs/overview-ios.md`「不变量」 |
| I17 | 来源合并（用户 2026-10-29 定） | 配对了**多台** Mac 时，开着 iCloud 同步的 Mac 不再作为单独的来源出现：来源菜单只留「iCloud」（副标题列出它的成员 Mac，离线的注明）和没开同步的 Mac。「iCloud」视图 = 接入 Mac 回答的同步项目（每台同步 Mac 都把所有同步项目——含仅云端的——列在自己的回答里）+ **其余同步 Mac 各自「仅本机」的项目**（没有仓库地址、不同步的那些）：有待办的按 Mac 分段写「仅在 X」，没有待办的并进「无待办的项目」、行下写「仅在 X」；点进去到**那台** Mac 上读写（项目页的写入、订阅、离线提示都跟着那台 Mac）。只配对一台 Mac 时不变（它既是自己，也是 iCloud）。一台同步 Mac 离线时，它上次说过的「仅本机」项目仍留在列表里（与 I3 一致：留着、只读，点进去说那台 Mac 暂时连不上），取消配对才清掉 | 用户：「我觉得某个 mac 是 icloud 存储，它就不应该单独出现，应该标记在 icloud 下。如果有 icloud 应该优先显示 icloud。」代价：打开页面（以及每次用户要求的加载）时多问其余同步 Mac 一次项目列表；它们的「仅本机」项目只在这时更新，不实时（接入 Mac 与已打开项目所在的 Mac 才订阅 `todo.changed`） |
| I18 | 项目行上的点（用户 2026-10-29 定） | **复用 Agents 页已有的状态，不另造**：等待你 = 橙点、工作中 = 绿点、空闲 = 灰点，已结束不标；同一项目有多个 Agent 时取最紧急的（等待 > 工作 > 空闲）。Agent 属于哪一行：Mac 在 Agent 的项目信息（`agents.snapshot` / `agents.delta` 里的 `project`）里多带一个可选的 `repo`（规范化后的仓库地址，与待办项目行的 `repo` 同一个值；只在那台 Mac 开着 iCloud 同步时填），手机按**仓库地址**把任何 Mac、任何 worktree 里的 Agent 对到项目行；没有仓库地址的项目（仅本机）按「同一台 Mac 上同一个目录」对（目录经 I6 的同一个哈希算出）。点跟着 Agents 页的实时流变，不另外取数、不轮询。`todo.projects` 行里 Mac 自己算的 `agent` 仍然发（旧手机用），新手机不再读 | 用户：「iCloud 应该显示所有正在工作的 agent」「我们 agents 页面不是有 agent 状态吗？……是不是应该复用这些状态，不要随便新增」。根因：行上的 `agent` 由接入 Mac 按**它自己的** Agent 登记算，另一台 Mac 上的 Agent 它看不见。代价：另一台 Mac 也要装带 `repo` 的新版，否则它那边的 Agent 没点（和以前一样）；`repo` 是可选字段，旧 Mac / 旧手机互相忽略，`PROTO` 不动 |
| I19 | 默认来源（用户 2026-10-29 定） | 配对了多台 Mac、没手动选过来源时：先用第一台在线 Mac 起步；它回答说自己开着同步，就换成「iCloud」并记住各 Mac 说过的模式（`ah.todos.modes`），以后每次启动直接是 iCloud。**手动选过的来源永远不动**：选过一台没开同步的 Mac 就一直是它；选过的 Mac 后来发现开着同步，则并入 iCloud（它没有单独的来源了）。换来源发生在回答到手的那一刻，直接用手里这份回答展示，不会闪一下 Mac 视图再换。下拉刷新时，其余 Mac 也重新说一遍模式（某台 Mac 刚开 / 刚关同步时随之更新） | 用户：「没手动选过来源时先用第一台在线 Mac 起步，它回答说自己开着同步就自动换成 iCloud，并记住（以后启动直接是 iCloud）；你手动选过的来源永远不动」。已存的模式只是起步的线索，每次回答都会更新；如果之后所有 Mac 都不再同步，页面照常说明「iCloud 同步没开」，菜单里这些 Mac 重新出现，不自动改来源 |

## 4. 用户视角

**入口**：「待办」标签。第一次进来如果没有任何已配对的 Mac，显示与别的页相同的「配对一台 Mac」空状态。

**来源菜单**（导航栏标题处的菜单）：
- 「iCloud」——仅当至少有一台已配对的 Mac 报告过 iCloud 同步已开启时可选；否则灰显，副标题写「没有 Mac 开启 iCloud 待办同步」。配对了多台 Mac 时，副标题列出它的成员 Mac（「MacBook Pro · Mac Studio（离线）」，I17）。选中时页面上写「通过 MacBook Pro 访问」。
- 每台已配对的 Mac，各带状态：在线 / 离线 / 「需要更新」；已知它开着同步的加一个云图标。**配对了多台 Mac 时，开着同步的 Mac 不在这里**（它们是 iCloud 的成员，I17）；只配对一台 Mac 时它仍在，与「iCloud」并列。
- 默认来源（I19）：只有一台 Mac → 它；否则上次的选择（选过的 Mac 后来发现开着同步的，视为 iCloud）；没选过但已知有 Mac 开着同步 → iCloud；都不知道 → 第一台在线的 Mac，它回答说自己开着同步就换成 iCloud。

**项目列表**（按来源分段）：
- 设备模式、未开同步：「有待办」「最近项目」（与待办窗口本机模式的侧栏同款分段与排序，活跃 Agent 在前）。
- 开着同步的来源（iCloud 模式；或只配对了一台 Mac 时、设备模式看那台开着同步的 Mac）：「iCloud 同步的项目」「无待办的项目（N）」（折叠）、「仅在 X」（X 是那台 Mac 的名字；iCloud 视图里每台有这类项目的 Mac 各一段，I17）。
- 每行：项目名（同名时灰色后缀）、待办条数徽标、云图标（仅云端）、Agent 点（I18：橙 = 有 Agent 在等你回复，绿 = 在工作，灰 = 空闲；同一项目有多个 Agent 时取最紧急的；没有 Agent 或都已结束则没有点）。点是 Agents 页那些状态的叠加，不论 Agent 在哪台 Mac 上。
- 底部或顶部一行同步状态（仅 iCloud）：已同步 · 刚刚 / 正在同步… / 离线 · N 项待上传 / 未登录 iCloud / 同步程序未运行 / Apple ID 已更换。出错才变色。

**项目页**：
- 标题：项目名；顶部的信息卡：仓库地址（同步项目）或「仅在 X，未同步」（配对了多台 Mac 时点出那台 Mac 的名字，只有一台时写「仅在这台 Mac，未同步」）「仅在 iCloud：没有 Mac 上有对应的文件夹」，以及「位于 Home MacBook Pro, Office iMac」（哪几台 Mac 有这个项目的目录，只列名字，不列路径）。
- 待办列表：每行正文（最多 4 行）、回形针 + 数量（附件在 Mac 上，手机不查看）、「来自 X」（别的 Mac 创建的）、⚡ 标记。点按 → 编辑页（改正文、开关 ⚡、删除）；向右滑 → 完成；向左滑 → 露出「删除」，点一下就删，不再另弹确认（滑开再点已经是两步；一路滑到底不会删）；右上「编辑」进入拖拽排序。
- 底部输入条：输入框 +（有本机目录时）⚡ 开关 + 发送。发送后立刻出现在末尾，Mac 确认前行尾有转圈；输入框随即清空（输入法里还在组字的也一并清掉），键盘保持弹出，可以接着写下一条。
- 「执行历史」折叠区（最近 100 条）：每行正文、完成时间、「在 X 上完成」；向右滑 → 恢复到队尾。没有「清空」。
- 下拉刷新；Mac 离线时顶部提示条，输入条禁用并说明原因（不排队）。

**错误与提示**：Mac 离线、被拒绝（太长 / 空 / 仅云端不能 ⚡）、别处已改（「这条待办已在别处处理」，列表自动刷新）。

## 5. 线协议（`ios/wire.rs` 与 Swift `AHProtocol` 共用 `ios/Fixtures/wire-v1/messages.json`）

全部是新的消息类型，JSON 键 camelCase，可选字段缺省不发。

| 方向 | 类型 | 主要字段 |
|---|---|---|
| 手机 → Mac | `todo.projects.get` | （空） |
| Mac → 手机 | `todo.projects` | `rev`、`mode`（`local` / `icloud`）、`sync?`（见下）、`projects: [ProjectRow]`、`truncated?` |
| 手机 → Mac | `todo.list.get` | `id`（项目 id） |
| Mac → 手机 | `todo.list` | `id`、`found`、`rev`、`name`、`canAuto`、`pending: [Item]`、`pendingTotal`、`history: [Done]`、`historyTotal` |
| 手机 → Mac | `todo.op` | `clientMsgId`、`id`、`op`（`add` / `edit` / `complete` / `restore` / `remove` / `reorder` / `auto`）、`todoId?`、`text?`、`auto?`、`order?` |
| Mac → 手机 | `todo.ack` | `clientMsgId`、`status`（`ok` / `notFound` / `rejected` / `failed`）、`reason?`、`todoId?`（`add` 返回新条目 id）、`list?`（该项目最新的 `todo.list` 内容） |
| 手机 → Mac | `todo.watch` | `on` |
| Mac → 手机 | `todo.changed` | `rev` |

**ProjectRow**：`id`、`name`、`hint?`（同名项目的区分后缀）、`repo?`（规范化后的仓库地址，仅同步项目）、`pending`、`history`、`scope`（`local` 未开同步 / `synced` / `cloudOnly` / `localOnly`）、`section`（`withTodos` / `recent` / `synced` / `empty` / `localOnly`）、`agent?`（`working` / `idle`；Mac 按自己的 Agent 登记算的，**新手机不再读它**，行上的点来自 Agents 页的状态，I18；仍然发给旧手机）、`macs?`（有目录的 Mac 名，仅同步项目）。

**Agent 的项目信息**（不是 `todo.*` 消息，而是 `agents.snapshot` / `agents.delta` 里 `AgentView.project` 的一个可选字段，I18）：`repo?`——规范化后的仓库地址，与 ProjectRow 的 `repo` 是同一个值。只在那台 Mac 开着 iCloud 同步（有链接表可查）且这个目录有仓库地址时才有，缺省不发。可选字段、旧对端互相忽略，`PROTO` 不动；夹具里带字段的样本是 `ios/Fixtures/wire-v1/messages.json` 中名为 `repo` 的 `agents.snapshot`；不带字段的老样本原样保留，两边的往返测试因此同时钉住「有就带上、没有就不发这个键」。

**sync**（只在 `mode = icloud` 时有，是**事实**而不是文案，手机据此推导状态行）：`syncing`、`paused?`（`accountChanged` / `zoneMissing` / `notSignedIn` / `quota`）、`helper`（`running` / `notRunning` / `incompatible` / `unsupported`）、`unsynced`、`lastPullMs`、`lastPushMs`、`devices: [{name, lastSyncMs, thisMac}]`。

**Item**：`id`、`text`、`createdAtMs`、`agent?`、`auto?`、`att?`（本机附件数）、`by?`（别的 Mac 创建时的 Mac 名）、`attOn?`（`{n, dev}`：附件在别的 Mac 上）。**Done**：`id`、`text`、`createdAtMs`、`doneAtMs`、`agent?`、`att?`、`doneOn?`（别的 Mac 完成时的 Mac 名）。

**限额**（`ios/wire.rs` 与 Swift `Wire` 必须一致）：`TODO_MAX_PENDING = 300`、`TODO_MAX_HISTORY = 100`、`TODO_MAX_PROJECTS = 200`、`TODO_TEXT_MAX_CHARS = 4000`。`reorder` 的 `order` 最多 `TODO_MAX_PENDING` 个 id。

**状态与错误码（`todo.ack`）**：

| `status` | `reason` | 含义 |
|---|---|---|
| `ok` | — | 已写入；`todoId` 是 `add` 新建的条目 |
| `notFound` | `unknownProject` | 项目 id 反查不到（被移除，或不属于这台 Mac） |
| `notFound` | `notFound` | 项目在，但那条待办已不在（别处已删 / 已做）——`list` 里是最新的 |
| `rejected` | `empty` / `tooLong` | 正文为空 / 超过 4000 字符 |
| `rejected` | `cloudOnly` | 对仅云端项目开 ⚡（它没有目录，没有 Agent 能在那里跑） |
| `rejected` | `invalid` | 缺字段或字段互相矛盾（如 `add` 没有正文、`reorder` 没有 `order`） |
| `rejected` | `unsupported` | 未知的 `op` |
| `rejected` | `conflict` | 同一个 `clientMsgId` 带着不同的内容又来了 |
| `failed` | `persist` | 写盘失败 |
| `failed` | `interrupted` | 日志里有这个 `clientMsgId`，但上次写入没有留下结果（daemon 在写的过程中重启）；手机提示「Mac 没能保存」并刷新列表，由用户看过后决定是否重做，**不会**自动重发同一个 id |

**`todo.list` 的 `found`**：`false` 表示项目 id 反查不到，其余字段为空；手机的项目页显示「无法载入待办」并保留返回路径（不自动关闭页面）。

**订阅的第一个回包**：`todo.watch {on: true}` 之后 Mac **立刻**回一条 `todo.changed {rev: 当前}`，手机以此作为「已知版本」的基线（其后与基线相同的 `todo.changed` 是自己写入引起的回声，不重拉）。

**幂等**：`todo.op` 的 `clientMsgId` 进 `journal.json`；内容哈希 = 去掉 `clientMsgId` 的其余字段。重放返回第一次的 `status` / `reason` / `todoId`，**`list` 总是现取最新的**（不进日志）。

## 6. Mac 端行为

### 6.1 项目列表（`ios/todos_view.rs` 纯函数 + `daemon/runtime/ios_todos.rs` 取数）

输入全是本地数据，不联网、不 fork `git`：`todos.json`（待办与历史条数）、最近 workspace 索引、活跃 Agent 快照（working / idle）、同步旁路元数据（`todo_sync::service::projects()` / `status()`）。

1. **候选**：与待办窗口、IM `/todo` 同一套（`todo_project_candidates`，排序抽成共用的 `commands.rs::rank_todo_projects`：workspace ∪ 有待办的 key ∪ 活跃 Agent 的目录；活跃度 → 置顶 → 有 workspace → 最近使用 → 名字排序）。无待办、也无 workspace / 活跃 Agent 的孤儿 key 不列——**哪怕它有历史**（与待办窗口一致；那个目录再有 workspace 记录时历史条数会跟着回来）。例外在同步模式：同步项目的目录键 / 虚拟键不在候选里（比如待办都做完了）、但 `todos.json` 里还有它的历史时，补一行（`pending = 0`，历史条数取自那里），行上仍写着做完了多少条。
2. **未开同步**（`mode = local`）：每个候选一行；`section` = 有待办 → `withTodos`，否则 `recent`；`scope = local`。
3. **开着同步**（`mode = icloud`，且同步元数据里有项目）：与前端 `mergeProjects` 同一算法——按链接表把同一仓库的多个目录折成一行（行 key = 候选里排最前的目录，计数取最大值）；同步项目里候选没提到的补一行（没有待办但有历史的项目）；没有链接的目录是 `localOnly`；没有本机目录的是 `cloudOnly`（行 key = `icloud:<项目身份>`）。分段：有待办的同步项目 → `synced`，条数为 0 → `empty`，有待办的无 remote 项目 → `localOnly`。同步元数据为空时退回未开同步的算法（与前端一致），但 `mode` 仍是 `icloud`。
4. **同名区分**（`hint`）：与前端 `labelProjects` 相同——同名的项目取最少的几级上层目录名让标签两两不同；同步项目按仓库地址的 owner 区分，无 remote 的按目录。**不含完整路径**。
5. 排序后截到 200 行。

### 6.2 项目 id 的反查

`id = sha256(key)[..8]` 十六进制。Mac 在「已知 key 集合」里找哈希相等的那个：`todos.json` 的所有项目与历史 key、同步元数据的目录与虚拟键、候选里的 key。找不到 → `found = false` / `unknownProject`。（不缓存，集合很小。）

### 6.3 读取

`todo.list.get` / `todo.ack.list`：先 `todo_sync::service::before_read(key)`（本地阶段），再读队列与历史；开着同步时用 `project_details(key)` 填 `by` / `attOn` / `doneOn`。`canAuto = !是虚拟键`。超出限额时截断并给出总数；整条消息超过 `MAX_ENVELOPE_BYTES` 时先裁历史再裁队尾。

### 6.3.1 同步状态

`sync` 取自 `todo_sync::service::status()`（只读旁路元数据与 helper 套接字探测，几毫秒），**只在** `todo.projects` 里带。

### 6.4 写入

直接调用 `todos.rs` 的现有函数（与 GUI / CLI 同一把锁、同一原子写）：

| op | 调用 |
|---|---|
| `add` | `add` / `add_auto`（正文先 trim；空 → `empty`；> 4000 字符 → `tooLong`；虚拟键忽略 `auto`） |
| `edit` | `set_text`（保留 id、创建时间、⚡、附件；空 → `empty`） |
| `complete` | `take`（与 whats-next、GUI 勾选同一路径，记入历史，受 `todo_history_limit` 约束） |
| `restore` | `restore`（排到队尾） |
| `remove` | `remove_checked` |
| `reorder` | `reorder`（未知 id 忽略，漏掉的保持相对顺序跟在后面） |
| `auto` | `set_auto`（虚拟键 → `cloudOnly`） |

找不到条目 → `notFound`。写盘失败 → `persist`（`failed`）。写入之后同步靠 helper 对 `todos.json` 的文件监听触发 tick，网关不另行处理。

**串行**：网关是单个 actor，写操作在 actor 里**就地等待**（写盘很快，且天然保证同一部手机的两次写按到达顺序生效）；只读的 `todo.projects.get` / `todo.list.get` 走 `spawn_then`，不阻塞 tick。

### 6.5 订阅与变更通知

`todo.watch on`：登记手机，**立刻**回一条 `todo.changed {rev: 当前}`（手机的基线）；`mode = icloud` 时后台 `refresh_if_stale`。网关每个 tick（250 ms）里每秒一次比较 `todos.json` 的签名（没有订阅者时不查）；变了 → 给每个**在线 + 前台 + 已 hello** 的订阅者发 `todo.changed {rev}`。手机离线、退后台、退订或被撤销配对时移出订阅。重启 daemon 丢订阅，手机在重新 hello 后重新订阅。

### 6.6 Agent 的仓库地址（`daemon/runtime/ios_agents.rs`，I18）

手机的 Agents 列表里每个 Agent 带一个 `project`（`ProjectRef {key, name, repo?}`）。Mac 解析它时，**在本机开着 iCloud 同步**（`todo_sync::service::is_enabled()`）的前提下，用与同步链接表同一个解析器（`FsResolver`，读 `.git/config`、跟随 worktree 的 `gitdir` / `commondir`）算出这个目录的仓库地址，填进 `repo`；没开同步、没有 remote、不是 git 目录时不填。解析要读磁盘，所以按目录缓存 60 秒（`ProjectCache`，开关同步最多滞后一分钟）。`key` 与 `name` 的算法没有变（每个 worktree 仍是各自的项目 key，`todo.*` 与 Agent 页用的是同一个 `detect_from`）。

## 7. 手机端行为

- `PhoneEngine`（`PhoneEngine+Todos.swift`）：`todoProjects(macId:)`、`todoList(macId:projectId:)`（请求 / 应答，超时 10 秒）、`todoApply(macId:operation:)`（等 `todo.ack`，超时 20 秒；离线立即抛 `.macOffline`）、`todoWatch(macId:on:)`；Mac 变为离线时让等待中的请求立刻失败；握手后为已订阅的 Mac 重新订阅（`reopenTodoWatch`）。`todo.changed` 以 `EngineEvent.todoChanged(macId:rev:)` 交给 UI。引擎对同一个等待键的并发请求回 `.unavailable("busy")`，所以 `TodosStore` 每种请求同时只发一个。
- `TodosStore`（`@MainActor @Observable`，`AHCore`；见类注释里的两条规则）：
  - **屏幕上的列表 = Mac 最后一次说的（`confirmed`）+ 用户还没被应答的写入（`overlay`，用 `TodoMutation.apply` 叠上去）**。被拒绝或丢失的写入自己消失，期间到达的整表不会让界面抖回旧值（I9 的实现方式；比「回滚」更简单也更不容易出错）。
  - **写入串行**：所有写入进一个队列（`queue` / `drainer`），一条应答之后才发下一条；超时时用**同一个 `clientMsgId` 重试一次**（幂等日志保证不会写两次）；再超时则提示「Mac 没有确认」并刷新；发现 Mac 离线则丢弃队列里这台 Mac 的后续写入并提示。队列空了之后再拉一次整表。
  - **一种请求同时只一个**：`kick(projects:list:)` + 脏标记，后来的需求只是让它再跑一轮。
  - 订阅只在「待办页可见 ∧ 前台」时开，订阅的是接入 Mac 与已打开项目所在的那台 Mac（iCloud 视图里点开另一台 Mac 的「仅本机」项目时是两台，`syncWatch`）；`knownRevs`（按 Mac 记）吃掉自己写入引起的 `todo.changed` 回声。
  - **同步状态行会自己复查**：回答里的 `sync.syncing` 只是 Mac 回答那一刻的快照，而打开页面时的订阅会让 Mac 立刻做一次「过期才拉」的同步（约 0.5–1.3 秒，见 §6.5），第一份回答常常正好撞上它；同步结束后待办文件不一定有变化，也就不会有 `todo.changed`，状态行会一直停在「正在同步…」。所以手机看到 `syncing` 为真时，隔 1.5 / 2 / 3 / 4 / 5 / 5 / 5 / 5 / 5 秒（共约 35 秒）静默重拉一次项目列表（`kick(projects:quiet:)`，`armSyncRecheck`），直到它不再为真。重拉没有得到回答时屏幕保持原样、继续下一次；不会为它去换 iCloud 接入 Mac，只有那台 Mac 说自己不再同步了才走一次平常的加载。收起页面、退后台、换来源时清零；下拉刷新给一份新的次数。次数用完仍在同步（比如网络很慢）就停下，状态行保持 Mac 最后说的。
  - **iCloud 接入 Mac 的选择**是纯函数（`TodoSources`，可单测）：候选 = 在线、`caps.todos`、没有被判定为 `mode = local`、没有失败过的 Mac；顺序：上次用过且已知开着同步的 → 已知开着同步的 → 上次用过的 → 模式未知的（逐台探测，探到 `mode = local` 的跳过）。接入 Mac 变化时清掉已打开的项目（它的 id 在另一台上不一定存在），只在那台 Mac 上有的项目除外（I17：它仍由那台 Mac 服务）。
  - **iCloud 视图的合并**（I17）：`others[macId]` 存其余同步 Mac 上次的回答；`sections` 把其中 `localOnly` / `local` 范围的行并进来。`TodoListedProject` = 一行 + 说出它的 Mac；行 id 是目录路径的哈希，两台 Mac 可能给出同一个 id，所以列表按 `macId|id` 区分。有待办的行进「仅在 X」段（每台 Mac 一段，段的 id 带着 macId），没有待办的并进「无待办的项目」。其余 Mac 只在一次**非静默**的项目列表加载（页面出现、回到前台、换来源、下拉刷新、写入排空后、`todo.changed` 触发的重拉；只有同步状态行的复查是静默的）的主请求都做完之后才逐台再问，和主请求串行（引擎对同一个等待键只允许一个在途请求），有新的加载排队时立刻让位；一台 Mac 在线却不回答时，这一轮最多多等一个读超时（10 秒）。接入 Mac 换人时，上一台的回答变成「其余 Mac」之一；Mac 说自己不再同步、或被取消配对时清掉它的那份。`open(_ project:)` 按行带的 `macId` 打开项目，项目页的读写与订阅就在那台 Mac 上。
  - **Agent 点**（I18）：`AppStore` 每次状态变化把 Agents 列表交给 `TodosStore.update(agents:)`，它建一个 `TodoActivityIndex`（`AHCore/TodoActivity.swift`）：Agent 的 `repo` → 最紧急的状态，`macId|sha256(project.key)[..8]` → 最紧急的状态（后者与 Mac 的 `todos_view::project_id` 是同一个算法，两边各钉同一组样本）。一行的点取两个索引里更紧急的。状态映射是 `TodoActivity(agentState:)`：`waiting` → 橙、`working` → 绿、其余未结束的 → 灰，`ended` 没有点。
  - **模式的更新**：每一次回答（接入 Mac、其余 Mac、选中的 Mac）都更新 `modes`；下拉刷新还会把没开同步的 Mac 也重新问一遍，所以某台 Mac 刚开同步也能被发现；平时一台已知没开同步的 Mac 不会被再问，直到用户在菜单里选它（它的回答说自己开着同步时，就并入 iCloud）。
  - 来源（`ah.todos.source`）、上次的 iCloud 接入 Mac（`ah.todos.icloudMac`）与各 Mac 上次说的模式（`ah.todos.modes`，`{macId: icloud|local}`，取消配对时清掉）存 `UserDefaults`；待办内容不落盘（I13）。`TodoSources.initial()`（I19）：已存的来源是 iCloud 时保持（除非所有 Mac 都已知是 `local`）；已存的是某台 Mac 时保持，但配对了多台、而它已知开着同步 → iCloud；没存过：只有一台 Mac → 它，已知有 Mac 开着同步 → iCloud，否则第一台在线的 Mac。起步在某台 Mac 上、它回答说自己开着同步时，`adopt` 当场改成 iCloud 并用手里这份回答展示，不另外再取一次。选中来源时，已并入 iCloud 的 Mac 会被换成 iCloud（`select`）。
  - 选中的 Mac 离线时，保留上次的行并标成「失败（离线）」，页面顶部给一行提示，输入条禁用（不排队）。
  - **iCloud 来源的一次加载没得到回答**（超时、请求失败，而 Mac 的在线状态没变）时，和上一条一样：保留上次的行、打开着的项目和同步状态行，页面顶部给同一句提示；没有行可留（第一次加载就失败）时，页面说真实的原因（「Mac 没有响应」等），而不是「iCloud 同步没开」。**接入的 Mac 离线、又没有别的 Mac 能代答**（在线状态里没有在线、开着同步的 Mac）时也一样（I3）：行、打开着的项目和同步状态行留着，只读（`canWrite` 为假，输入条禁用），页面顶部说那台 Mac 暂时连不上；行只在这次启动的内存里（I13），重新启动后没有行可留，页面说明原因。**只有这些行已经不能当作 iCloud 的视图时才清空列表并说明原因**：那台 Mac 已被取消配对，或者在线的 Mac 都报告没开同步。
- 页面可见 → `todoWatch(on)`；不可见 / 退后台 → `off`。收到 `todo.changed`（版本与已知的不同）→ 重拉。
- 旧 Mac（`caps.todos == false`）：来源菜单里显示「需要更新」，选中后页面给出说明，不发任何 `todo.*`。
- 界面（`ios/AskHuman/Views/Todos/`、`Design/TodoTexts.swift`）：`TodosScreen`（来源菜单 + 项目列表 + 同步状态行）、`TodoProjectScreen`（待办队列、历史、底部输入条）、`TodoEditSheet`。同步状态行的文字、`helper` 的三种问题与桌面端设置页用同一套措辞；线协议里的 `helper` 只有 `running / notRunning / incompatible / unsupported`，没有桌面端区分的「未安装」（`installed`），所以「没装」与「没在跑」在手机上都显示「同步程序未运行」；`incompatible` 显示「同步程序无法使用 iCloud」，`unsupported`（这个平台没有同步程序，实际上同步行不会出现）显示「未安装同步程序」。

## 8. 兼容

- 新手机 + 旧 Mac：靠 `caps.todos`（缺省 false）不发 `todo.*`；即使发了，旧 Mac 回 `error unknown_type`，手机把它当「不支持」。
- 旧手机 + 新 Mac：多出的 `caps.todos` 字段被忽略；Mac 只在收到 `todo.*` 时才动作。
- 线协议版本与 IPC 协议号都不变（I16）。

## 9. 隐私与安全

- 待办正文只在端到端加密的信道里走（与提问同一套密钥）；不进推送、不进通知扩展、不进手机磁盘（I13）。
- 手机看不到、也发不出项目路径（I6）；项目 id 只是哈希，Mac 不接受任何路径。
- 写操作只作用于 Mac 已知的项目（反查失败即拒绝），手机不能借此往任意路径写 `todos.json` 的 key。
- 一部已被撤销配对的手机收不到也发不了任何 `todo.*`（与其他消息同一道门）。
- 文本长度、条数、消息大小都有上限（I12），一部行为异常的手机不能撑爆 Mac 的 `todos.json` 或 broker。

## 10. 已知局限

1. **Mac 都不在线就看不到**（I1、I2）。这是本期最大的局限，也是 §11 第 1 项后置的理由。
2. 第一版没有附件、没有「全部待办」聚合页、没有清空历史、没有从 iCloud 移除项目。
3. 手机与 Mac 同时改**同一条的正文**：后到的覆盖先到的（与 Mac 上两个窗口同时改相同）；删除 / 完成冲突按 `notFound` 处理。
4. iCloud 的传播延迟照旧（`docs/specs/todo-sync.md` 附录 A 与计划 §10 的实测：推送中位约 16 秒，轮询 10 分钟兜底）：手机写入 → Mac `todos.json` 是 1 秒量级，但「手机写入 → 另一台 Mac 看到」仍是同步的延迟。
5. 「订阅」靠 Mac 每秒比较文件签名；同一秒内的两次改动合并成一条 `todo.changed`（手机重拉后看到最终状态）。
6. 行上的 Agent 点（I18）用的是 Agents 页的实时状态，Agent 一变点就跟着变；但**跨 Mac 的点要靠那台 Mac 也装了带 `project.repo` 的新版**：没装的 Mac 上的 Agent 报不出仓库地址，只能按「同一台 Mac 上的目录」对到行，所以在另一台 Mac 的项目行上没有点（与改动前一样）。点按仓库地址对，同一仓库的各个 worktree 共用一个点；没有 remote 的目录只在自己那台 Mac 上有点。
7. iCloud 视图合并其余 Mac 的「仅本机」项目要多问那些 Mac 一次（§7），它们的这部分只在非静默的加载时更新：订阅（`todo.changed`）只覆盖接入 Mac 与打开着的项目。另一台 Mac 刚新增一个「仅本机」项目，要等下一次页面出现 / 回到前台 / 下拉刷新才看到。
8. 默认来源记着「iCloud」（I19）之后，如果所有 Mac 都关掉了同步，页面会说「iCloud 同步没开」，菜单里 Mac 重新出现，需要手动选一台；没有自动退回到某台 Mac。
9. **单条超长正文**：手机自己写入的正文限 4000 字符，但桌面与 CLI 的待办没有长度限制。Mac 在整表装不进一条消息时先裁历史、再裁队尾（每次减半），所以几百条各 12 KB 的待办没问题；但**一条**正文超过约 600 KB 时，那个项目的 `todo.list` 发不出去（`send` 对超限消息是静默丢弃），手机上表现为这个项目载入超时（页面显示「Mac 没有响应」，下拉重试也一样），其他项目不受影响。真实使用里不会出现（最大的一份真实数据里单个项目的整表也只有 17 KB），不为它加截断（截断后手机编辑再写回会丢数据）。

## 11. 后置（本期不做）

1. **手机直连 iCloud（真正的离线 Mac 也能用）**：给 iOS App 加 iCloud / CloudKit 权限（需要用户在开发者账号里为 `com.naituw.humaninloop.ios` 打开 iCloud 并勾上 `iCloud.com.naituw.humaninloop.todosync` 容器，重签描述文件）；在 `AHStore` / `AHCore` 里实现 Swift 版同步核心（记录 `payload` v1 的编解码、HLC 字段级合并、分数索引顺序键、墓碑、`Device` 记录、账号变更暂停），与 Rust 版共用跨语言夹具（`ios/Fixtures/todo-sync-v1/`），并先用 Development 环境的独立容器充分测试再碰 Production。预留：手机端的 `TodoSource` 已是枚举，`TodosStore` 只依赖「项目列表 / 项目列表内容 / 应用操作」三个协议方法，直连后端可作为第三种来源接入而不动界面。
2. 附件：手机端查看（Mac 经 `asset.*` 分块下发）与添加（`upload.*`），以及 iCloud 附件同步后的联动。
3. 「全部待办」聚合页、清空历史（带全局清空的二次确认文案）、从 iCloud 移除项目。
4. 待办的推送（例如被 whats-next 消费时通知手机）；Siri / 快捷指令 / 小组件「加一条待办」。
5. 离线编辑队列（手机或 Mac 离线时先记在手机里，恢复后重放）。

## 12. 验收标准

> 状态（2026-10-29）：7 项都已自测通过；数字取自当天最后一次完整运行，机器上的运行记录不入库。第 7 项是晚上加的（多台 Mac 开着同步，I17–I19），前 6 项的数字是它之前的运行结果。

1. ✅ **Rust**：网关测试台（虚拟时钟 + 内存待办存储）、`todos_view`、真实 Host（临时 `ASKHUMAN_HOME`）与跨语言夹具的用例见 `src-tauri/src/ios/gateway/tests_todos.rs`、`ios/todos_view.rs`、`daemon/runtime/ios_todos.rs` 的 `#[cfg(test)]`：项目列表 / 列表 / 七种写操作 / 幂等重放与冲突 / 限额与裁剪 / 订阅与变更通知（含退订、手机离线、撤销配对）/ `caps.todos`；项目合并、分段、同名区分逐条镜像前端用例。结果：`cargo test ios::` 252 条、`ios_todos`（daemon 的真实 Host）25 条全绿。
2. ✅ **Swift 包**（`swift test`）：协议夹具、引擎（请求 / 应答 / 超时 / 离线 / 重订阅，`TodoEngineTests`）、纯函数（`TodoModelsTests`：来源、接入 Mac 选择、同步状态行、变更叠加）、`TodosStore`（`TodosStoreTests`：来源解析、iCloud 自动换机、写入叠加与撤销、串行与重试、离线不排队、订阅随页面可见性开关、旧 Mac 的说明、「正在同步…」的复查与收手、iCloud 来源没得到回答或接入的 Mac 离线时保留行）。结果：整包 414 条无失败（协议 27、存储 21、传输 14、Markdown 8、核心 144、应用 159、端到端 41，其中 4 条要真实服务、已跳过）；待办相关的 `TodoEngineTests` 12、`TodoModelsTests` 17、`TodosStoreTests` 24。之后又加了状态行复查的 5 条与 iCloud 来源保留行的 6 条（没得到回答 2 条、接入的 Mac 离线 2 条，另 2 条钉住仍然清空的边界：同步被关掉、那台 Mac 被取消配对；`TodosStoreTests` 共 35；不带端到端环境变量的整包运行：协议 27、存储 21、传输 14、Markdown 10、核心 144、应用 185 全绿，端到端 41 条按缺环境变量跳过）。
3. ✅ **模拟器 UI 测试**（mock 后端，en 与 zh-Hans 各跑一遍，截图自查）：打开标签与同步状态行、切来源（iCloud ⇄ Mac，含同名项目的区分）、进项目、新增、完成、恢复、编辑、删除（滑开点「删除」即删）、**拖拽排序**、历史折叠、Mac 离线提示、**旧 Mac 的「需要更新」说明与灰显的菜单项**。结果：`AskHumanTodosUITests` 9 条（含发送后输入框清空的 2 条，见 §13 第 9 条），英文与中文各跑一遍全部通过。
4. ✅ **真实 daemon 端到端**（本地 broker，`TodoE2ETests`）：手机经真实网关新增 / 编辑 / ⚡ / 排序 / 完成 / 恢复 / 删除，`AskHuman todo list`（CLI）与 `todos.json` 随之变化；同一 `clientMsgId` 重放只生效一次，拒绝与冲突都有应答；Mac 侧改文件时手机收到 `todo.changed`，退订后不再收到；项目列表里没有任何绝对路径。5 条用例，约 30 秒。
5. ✅ **真实数据形状自查**：工作树的 Dev Instance 里没有用户的真实待办，所以拿真实 `todos.json` 的**一份拷贝**放进一次性 Mac（同步关闭），让手机引擎对账，只打印条数与字节数：15 个项目键（含 2 个 `icloud:` 虚拟键）、10 个历史键；手机看到 15 行，每一行的待办数、历史数都与文件逐项一致，每个项目的 `todo.list` 总数与行一致，最大一份整表 17 KB（上限 900 KB），最多 14 条待办、55 条历史。拷贝与临时测试跑完即删。这次没有覆盖「开着同步」的 iCloud 视图（那需要真实的 CloudKit 与 helper），它由 §12.1 的用例按桌面端算法覆盖。
6. ✅ 没有改 `ipc/mod.rs`；`./scripts/install.sh` 通过（工作树里装到 `.askhuman-dev/bin`）；`cargo clippy --all-targets -D warnings`、`cargo fmt --check` 无告警。
7. ✅ **多台 Mac 开着同步（I17–I19，晚上加的）**：
   - **Rust**：`cargo test ios` 319 条全绿，其中新增 `daemon/runtime/ios_agents.rs` 里 `repo` 的 4 条（开着同步才带、worktree 带主检出的仓库地址、没有 remote 不带、没有时线上不发这个键且旧消息照样能读）与 `todos_view::project_ids_match_the_ones_the_phone_computes`（与手机同一组 id 样本）；60 秒缓存（`ProjectCache`）没有单独的用例；`cargo fmt --check` 无差异，新代码无 clippy 告警（`cargo clippy -D warnings` 整个 crate 有一条与这次无关的 `nonminimal_bool`，在 `integrations/antigravity_rule.rs`）；没有改 `ipc/mod.rs`，`PROTO` 不动，夹具里加了带 `repo` 的 `agents.snapshot` 样本，两边往返。
   - **Swift 包**（`swift test`）：整包 441 条无失败（协议 27、存储 21、传输 14、Markdown 10、核心 169、应用 200；端到端 43 条在没有环境变量时按设计跳过）；待办相关 `TodosStoreTests` 47、`TodoModelsTests` 22、`TodoActivityTests` 8，覆盖来源菜单（成员 Mac、没开同步的 Mac 仍单独出现、只有一台时不变）、默认来源与迁移（起步 → 换成 iCloud、记住、手动选过的不动、选过的同步 Mac 并入 iCloud）、其余 Mac 的「仅本机」项目分段与在那台 Mac 上打开 / 写入、接入 Mac 掉线由另一台接手、关掉同步的 Mac 回到菜单、下拉刷新重新问各 Mac、点的合并与状态变化。
   - **端到端**（本地 broker + 刚装的 `AskHuman`，`AH_E2E_BIN` / `AH_E2E_BROKER_DIR`）：43 条中 39 条通过、4 条要真实服务而跳过、无失败；`TodoE2ETests` 5 条与 `AgentE2ETests`（Agents 快照经真实 daemon 到手机引擎）都在其中。这次没有对真实 broker 与真实 iCloud 跑。
   - **模拟器 UI 测试**（iOS 27.0，mock 后端）：`AskHumanTodosUITests` 10 条，中文整类全过；英文整类 9 条一次过，`testDraggingATodoUpChangesTheOrder`（拖动排序，靠真实手势的时长）在机器负载很高时失败了两次（一次整类里、一次单独跑），第三次单独重跑通过，没有对照到是不是负载引起，它不经过这次改的代码。新增的 `testMacsThatBothSyncAreOneICloudSourceWithTheirOwnProjectsAndTheAgentsDots`（场景 `bothSync`）两种语言都过：菜单只有 iCloud 且副标题写出两台 Mac、橙点与绿点在对的行上、「仅在 Office iMac」分段、点进去的项目页写着「仅在 Office iMac，未同步」。UI 测试要用 iOS 27.0 的模拟器：18.5 上 `testAddingFinishingEditingAndRestoringATodo` 会卡住（见 `docs/overview-ios.md` 的 XCUITest 经验）。
   - **没有验证的**：真机；另一台 Mac 装上新版后的跨 Mac 点（`docs/PROGRESS.md`）；对真实 iCloud 的合并视图。

## 13. 与起草时的差异（实现中的变更）

起草（`d29699a`）之后实现与验证中改过的地方，都已回写进上面各节，这里汇总，方便审阅：

1. **乐观更新的做法**（I9）：起草写的是「先改本地、失败回滚、写入期间忽略整表」。实现为「屏幕上的列表 = Mac 最后一次的答复 + 还没被应答的写入」（§7）：被拒绝或丢失的写入自己消失，不需要显式回滚，也就不会出现回滚与新整表互相覆盖。写入进一个串行队列，超时用同一个 `clientMsgId` 重试一次。
2. **订阅的基线**（I10）：`todo.watch on` 之后 Mac 立刻回一条 `todo.changed {rev}`，手机据此知道「已知版本」，之后与它相同的 `todo.changed` 当作自己写入的回声（§5、§6.5）。起草时没有写这一步。
3. **应答的状态码**（§5）与 `todo.list` 的 `found: false`（「无法载入待办」，页面不自动关闭）是实现时定的。
4. **旧 Mac 在来源菜单里是灰显的、不可选**（起草写的是「选中后页面给出说明」）。只有它恰好是默认来源（比如只配对了一台 Mac）时，页面才显示「这台 Mac 需要更新」。验证时发现这条路径之前会一直转圈（见下）。
5. **同步状态行**的措辞与桌面端设置页一致；线协议里的 `helper` 没有桌面端区分的「未安装」，所以「没装」与「没在跑」在手机上都显示「同步程序未运行」（§7）。
6. **项目页的信息卡**显示仓库地址与「位于 <Mac 名…>」，而不是「在 N 台 Mac 上有目录」（§4）。
7. **验证中发现并修复的缺陷**：「待办」页在 Mac 列表到达之前就已经在屏幕上（例如启动后直接落在这一页）时，来源是 Mac 列表到达之后才选中的；而只有「某台 Mac 变得可用」才会触发加载，于是一台**不认识待办的旧 Mac**（永远不会「变得可用」）会让页面停在骨架屏上，看不到任何说明。现在「刚选中的来源」本身就会触发一次加载，旧 Mac 由此得到「需要更新」的说明；用 `TodosStoreTests.testAnOutdatedMacIsExplainedEvenWhenTheTabIsOnScreenBeforeTheMacsAreKnown` 和 UI 测试 `testAMacThatPredatesTodosAsksForAnUpdate` 钉住。
8. 已知局限补了一条（§10 第 9 条）：单条超长正文。「搜索项目」输入框在「Mac 连不上 / 需要更新」这类页面上不显示（那里没有东西可搜），有项目可显示时才出现。
9. **用户反馈的缺陷：发送后输入框偶尔没清空**（待办已加进列表，输入框里的字原样还在）。原因：SwiftUI 只在绑定的值与它上次看到的值不同时，才把值推给 UIKit 输入框；发送与最后一次按键落在同一轮更新里时（主线程正忙，例如刚加完上一条、列表还在滚动），`text = ""` 恰好等于它缓存的旧值，什么也没推，于是输入框留着字、页面状态却是空的。试过两种办法都不行：延后清空（30 次里仍有 6 次没清）、换一个新的输入框（键盘焦点丢了）。现在发送时直接清空拿着键盘的那个输入框自己的文字（`FocusedTextInput`：先结束组字、再替换全文），焦点不丢，输入法里组到一半的字也不会在下一次按键时冒回来。UI 测试：`testTheAddFieldIsEmptyAgainAfterSending`（打字、发送、输入框为空）与 `testSendingRightAfterTypingAlwaysEmptiesTheField`（调试构建的启动参数 `-AHTodoAddStress YES`：在同一轮里打字并发送 30 次，数输入框还留着字的次数，要求为 0；不修时跑过 4 次，每次都有留着字的，做过变异验证）。拼音组字（真实按键）后发送、再接着打字，用一次性的 UI 测试在模拟器上确认过，没有入库（它依赖模拟器装了拼音键盘）。真机尚未验证。
10. **用户反馈：删除待办太繁琐**（向左滑、点「删除」之后还要在对话框里再点一次确认）。用户认为滑开再点「删除」已经是两步确认，所以去掉了对话框（`testDeletingATodoTakesTheSwipeAndTheTapOnly`）；一路滑到底仍不会删（`allowsFullSwipe: false`）。编辑页里的「删除待办」按钮没动，仍先确认——那条路径只有点一下。要知道的代价：删除不可恢复（执行历史里只有完成的），Mac 上这条待办的附件也随之删掉；没有撤销。
11. **用户反馈：待办里的状态比较混乱**（几台 Mac 都开着 iCloud 存储时，手机默认还是选中某一台 Mac，另一台 Mac 上的 Agent 就不会标绿点）。原因有两层：来源菜单把每台开着同步的 Mac 都当成独立来源，默认选中了其中一台；行上的绿点由「回答的那台 Mac」按它自己的 Agent 登记算（`ProjectRow.agent`），另一台 Mac 上的 Agent 它看不到。做法是用户在两轮提问里定的（I17–I19，§4、§6.6、§7）：开着同步的 Mac 并进 iCloud，各 Mac 的「仅本机」项目在 iCloud 视图里分段列出；点复用 Agents 页的状态、按仓库地址（`project.repo`）或同机目录对到行；默认来源按各 Mac 说过的模式（`ah.todos.modes`）起步在 iCloud。`ProjectRow.agent` 保留给旧手机，新手机不读。用 UI 测试 `testMacsThatBothSyncAreOneICloudSourceWithTheirOwnProjectsAndTheAgentsDots` 与 `TodosStoreTests`、`TodoActivityTests`、`TodoModelsTests` 里的新用例钉住。真机、以及另一台 Mac 装上新版后的实际效果尚未验证。
