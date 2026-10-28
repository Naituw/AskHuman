# 需求：iOS App 管理待办（选 Mac / iCloud 视角、项目来源）

> 状态：**2026-10-29 起草并开工；全部决策由我（agent）在用户睡觉期间代为做出，待用户次日验收时审阅，可整条否决**（用户当晚留言：「我想给我们的 iOS 客户端添加待办功能的支持，我还没有考虑清楚。比如说是不是可以选择不同设备，然后管理上面的待办，也可以选择 iCloud 同步模式，然后可能还有项目来源的问题。这些你都帮我设计，然后实现验证，全部都能自己搞定。」）。没有经过 AskHuman 问答；所有选择都写在 §3，并给出理由与否决后的代价。
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
| 项目 id | 项目 key（目录路径或 `icloud:<项目身份>`）的不透明哈希；手机看不到、也发不出路径 |
| 行 key | 项目行代表的 key：同一仓库的多个目录合并成一行时，取最近使用的那个目录（与待办窗口侧栏相同） |
| 版本（rev） | `todos.json` 的 mtime + 长度的签名，只用来判断「变了没有」，不是序号 |

## 3. 决策记录（均为 agent 代定，请审阅）

| 编号 | 决策项 | 结论 | 理由 / 否决后的代价 |
|---|---|---|---|
| I1 | 通路 | 沿用 iOS 渠道：手机 ⇄ Mac 网关（MQTT + 端到端加密），新增 `todo.*` 消息；Mac 的 `todos.json` 仍是唯一事实来源，手机只是遥控与展示端 | 不新增服务器、不新增凭据、与「权威状态在 Mac」一致；代价是 Mac 不在线时手机看不到待办（见 I3、§11 的后置项） |
| I2 | 不直连 CloudKit | 本期**不让手机直连 iCloud**（那会是「第三个副本」） | ① 需要给 iOS App ID 开 iCloud 容器权限并重签描述文件——这是 Apple 开发者账号上的改动，需要用户授权；② 要把 Rust 同步核心（HLC 字段级合并、顺序键、墓碑、本地变更检测）在 Swift 里再实现一遍并与 Rust 版逐位对拍，写错会污染用户**所有 Mac 共用的生产容器**，而当前没有安全的测试容器；③ 无法在无人值守时验证。设计上留好位置（§11） |
| I3 | iCloud 模式 | 「iCloud」是一个**来源**，不是另一条数据通路：手机挑一台开启了同步且在线的 Mac 当接入点，读写它的 `todos.json`，由已有的同步核心把改动带到 iCloud 与其他 Mac。接入 Mac 离线时自动换下一台；一台都没有时说明原因（没有 Mac 在线 / 没有 Mac 开启同步） | 用户要的「选 iCloud 同步模式」是「看到所有设备的待办、不用关心是哪台 Mac」，这能做到；比起让用户记住「家里那台开了同步」，自动换机更省心。代价：所有 Mac 都不在线时仍然看不到（与 I2 同源） |
| I4 | 设备模式 | 来源选某台 Mac 时，列表只含**那台 Mac 上有目录的项目**；开着同步的 Mac 上，「仅云端」项目只在 iCloud 模式里出现 | 设备模式回答「这台电脑上我有哪些待办」，iCloud 模式回答「我所有的待办」，两者含义不同才有区分的价值。若合并成一个列表，用户分不清哪些项目其实在这台机器上跑不了 ⚡、也没有目录 |
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

## 4. 用户视角

**入口**：「待办」标签。第一次进来如果没有任何已配对的 Mac，显示与别的页相同的「配对一台 Mac」空状态。

**来源菜单**（导航栏标题处的菜单）：
- 「iCloud」——仅当至少有一台已配对的 Mac 报告过 iCloud 同步已开启时可选；否则灰显，副标题写「没有 Mac 开启 iCloud 待办同步」。选中时标题旁写「通过 MacBook Pro 访问」。
- 每台已配对的 Mac，各带状态：在线 / 离线 / 「需要更新」；已知它开着同步的加一个云图标。
- 默认来源：只有一台 Mac → 它；否则上次的选择；首次且有多台 → 第一台在线的 Mac。

**项目列表**（按来源分段）：
- 设备模式、未开同步：「有待办」「最近项目」（与待办窗口本机模式的侧栏同款分段与排序，活跃 Agent 在前）。
- 开着同步的来源（iCloud 模式；或设备模式但那台 Mac 开着同步）：「iCloud 同步的项目」「无待办的项目（N）」（折叠）、「仅本机（不同步）」。
- 每行：项目名（同名时灰色后缀）、待办条数徽标、云图标（仅云端）、绿点（有 Agent 在这个项目里工作）。
- 底部或顶部一行同步状态（仅 iCloud）：已同步 · 刚刚 / 正在同步… / 离线 · N 项待上传 / 未登录 iCloud / 同步程序未运行 / Apple ID 已更换。出错才变色。

**项目页**：
- 标题：项目名；副标题：仓库地址（若有）或「仅本机」；iCloud 模式下显示「在 N 台 Mac 上有目录」。
- 待办列表：每行正文（最多 4 行）、回形针 + 数量（附件在 Mac 上，手机不查看）、「来自 X」（别的 Mac 创建的）、⚡ 标记。点按 → 编辑页（改正文、开关 ⚡、删除）；向右滑 → 完成；向左滑 → 删除（需二次确认）；右上「编辑」进入拖拽排序。
- 底部输入条：输入框 +（有本机目录时）⚡ 开关 + 发送。发送后立刻出现在末尾，Mac 确认前行尾有转圈。
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

**ProjectRow**：`id`、`name`、`hint?`（同名项目的区分后缀）、`repo?`（规范化后的仓库地址，仅同步项目）、`pending`、`history`、`scope`（`local` 未开同步 / `synced` / `cloudOnly` / `localOnly`）、`section`（`withTodos` / `recent` / `synced` / `empty` / `localOnly`）、`agent?`（`working` / `idle`）、`macs?`（有目录的 Mac 名，仅同步项目）。

**sync**（只在 `mode = icloud` 时有，是**事实**而不是文案，手机据此推导状态行）：`syncing`、`paused?`（`accountChanged` / `zoneMissing` / `notSignedIn` / `quota`）、`helper`（`running` / `notRunning` / `incompatible` / `unsupported`）、`unsynced`、`lastPullMs`、`lastPushMs`、`devices: [{name, lastSyncMs, thisMac}]`。

**Item**：`id`、`text`、`createdAtMs`、`agent?`、`auto?`、`att?`（本机附件数）、`by?`（别的 Mac 创建时的 Mac 名）、`attOn?`（`{n, dev}`：附件在别的 Mac 上）。**Done**：`id`、`text`、`createdAtMs`、`doneAtMs`、`agent?`、`att?`、`doneOn?`（别的 Mac 完成时的 Mac 名）。

**限额**（`ios/wire.rs` 与 Swift `Wire` 必须一致）：`TODO_MAX_PENDING = 300`、`TODO_MAX_HISTORY = 100`、`TODO_MAX_PROJECTS = 200`、`TODO_TEXT_MAX_CHARS = 4000`。`reorder` 的 `order` 最多 `TODO_MAX_PENDING` 个 id。

**错误码（`todo.ack.reason`）**：`empty`、`tooLong`、`unknownProject`、`notFound`、`cloudOnly`（对仅云端项目开 ⚡）、`persist`（写盘失败）、`unsupported`（未知 `op`）。

**幂等**：`todo.op` 的 `clientMsgId` 进 `journal.json`；内容哈希 = 去掉 `clientMsgId` 的其余字段。重放返回第一次的 `status` / `reason` / `todoId`，**`list` 总是现取最新的**（不进日志）。

## 6. Mac 端行为

### 6.1 项目列表（`ios/todos_view.rs` 纯函数 + `daemon/runtime/ios_todos.rs` 取数）

输入全是本地数据，不联网、不 fork `git`：`todos.json`（待办与历史条数）、最近 workspace 索引、活跃 Agent 快照（working / idle）、同步旁路元数据（`todo_sync::service::projects()` / `status()`）。

1. **候选**：与待办窗口、IM `/todo` 同一套（`todo_project_candidates`：workspace ∪ 有待办的 key ∪ 活跃 Agent 的目录；活跃度 → 置顶 → 有 workspace → 最近使用 → 名字排序）。无待办、也无 workspace / 活跃 Agent 的孤儿 key 不列（有历史的除外：历史不空就保留，让人能恢复）。
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

### 6.5 订阅与变更通知

`todo.watch on`：登记手机；`mode = icloud` 时后台 `refresh_if_stale`。网关每个 tick（250 ms）里每秒一次比较 `todos.json` 的签名（没有订阅者时不查）；变了 → 给每个**在线 + 前台 + 已 hello** 的订阅者发 `todo.changed {rev}`。手机离线、退后台、退订或被撤销配对时移出订阅。重启 daemon 丢订阅，手机在重新 hello 后重新订阅。

## 7. 手机端行为

- `PhoneEngine`：`todoProjects(macId:)`、`todoList(macId:projectId:)`（请求 / 应答，超时 10 秒）、`todoApply(macId:projectId:op:)`（等 `todo.ack`，超时 20 秒；离线立即抛 `.macOffline`）、`todoWatch(macId:on:)`；Mac 变为离线时让等待中的请求立刻失败；握手后为已订阅的 Mac 重新订阅。`todo.changed` 以 `EngineEvent.todoChanged(macId:)` 交给 UI。
- `TodosStore`（`@MainActor @Observable`）：来源与接入 Mac 的解析、项目列表与各项目列表的内存缓存、乐观更新与回滚（I9）、同步状态推导、搜索。**iCloud 接入 Mac 的选择**是纯函数（可单测）：候选 = 在线且 `caps.todos` 的 Mac；优先上次用过且已知开着同步的，其次已知开着同步的，最后「模式未知」的（逐台探测，探到 `mode = local` 的跳过）。
- 页面可见 → `todoWatch(on)`；不可见 / 退后台 → `off`。收到 `todo.changed` → 重拉。
- 旧 Mac（`caps.todos == false`）：来源菜单里显示「需要更新」，选中后页面给出说明，不发任何 `todo.*`。

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
6. 项目列表里的活跃 Agent 绿点来自 Mac 的 Agent 快照，只在拉取时更新一次，不实时。

## 11. 后置（本期不做）

1. **手机直连 iCloud（真正的离线 Mac 也能用）**：给 iOS App 加 iCloud / CloudKit 权限（需要用户在开发者账号里为 `com.naituw.humaninloop.ios` 打开 iCloud 并勾上 `iCloud.com.naituw.humaninloop.todosync` 容器，重签描述文件）；在 `AHStore` / `AHCore` 里实现 Swift 版同步核心（记录 `payload` v1 的编解码、HLC 字段级合并、分数索引顺序键、墓碑、`Device` 记录、账号变更暂停），与 Rust 版共用跨语言夹具（`ios/Fixtures/todo-sync-v1/`），并先用 Development 环境的独立容器充分测试再碰 Production。预留：手机端的 `TodoSource` 已是枚举，`TodosStore` 只依赖「项目列表 / 项目列表内容 / 应用操作」三个协议方法，直连后端可作为第三种来源接入而不动界面。
2. 附件：手机端查看（Mac 经 `asset.*` 分块下发）与添加（`upload.*`），以及 iCloud 附件同步后的联动。
3. 「全部待办」聚合页、清空历史（带全局清空的二次确认文案）、从 iCloud 移除项目。
4. 待办的推送（例如被 whats-next 消费时通知手机）；Siri / 快捷指令 / 小组件「加一条待办」。
5. 离线编辑队列（手机或 Mac 离线时先记在手机里，恢复后重放）。

## 12. 验收标准

1. Rust：网关测试台里（虚拟时钟 + 内存待办存储）：项目列表 / 列表 / 七种写操作 / 幂等重放与冲突 / 限额与截断 / 订阅与变更通知（含退订、手机离线、撤销配对）/ `caps.todos` 全部有用例；`todos_view` 的项目合并、分段、同名区分有用例；真实 Host（临时 `ASKHUMAN_HOME`）对 `todos.json` 的读写有用例；跨语言夹具里每个新消息都有样本。
2. Swift：协议夹具通过；引擎对脚本化的 Mac 的请求 / 应答 / 超时 / 离线 / 重订阅用例；`TodosStore` 的来源解析、iCloud 自动换机、乐观更新与回滚、同步状态推导用例。
3. 模拟器 UI 测试（mock 后端，en + zh-Hans）：打开标签、切来源、进项目、新增、完成、恢复、编辑、删除（含确认）、排序、历史折叠、Mac 离线提示、需要更新提示。
4. 真实 daemon 端到端（本地 broker）：手机经真实网关新增 / 完成 / 恢复 / 删除一条待办，`todos.json` 随之变化，Mac 侧直接改文件时手机收到 `todo.changed` 并刷新。
5. 装机后真实 daemon 的 `AskHuman todo list` 与手机看到的一致（只核对条数与 id 前缀，不打印内容）。
6. 不改 `ipc/mod.rs`；`./scripts/install.sh` 通过；`cargo clippy -D warnings`、`cargo fmt --check`、`swift test`、`xcodebuild test` 全绿。
