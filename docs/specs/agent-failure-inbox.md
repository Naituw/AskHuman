# Agent 失败处理统一到收件箱，并检测 Cursor 的出错停止

> 状态：2026-11-03 草案。§2 已实现（iOS Agents 页只报告结果，已过自动化，未出 TestFlight 包）；
> §3 起是待用户确认的方案，确认前不写代码。
> 关联：[最终失败确认](agent-failure-confirmation.md)（错误卡本体）、
> [生命周期结果与原会话继续](agent-lifecycle-results-and-recovery.md)、
> [Cursor 等待期保活](../investigations/cursor-idle-wait-keepalive.md)（预防；本文是兜底，两者独立）。
> 约束：Cursor 一律不实测（会计费）。本文关于 Cursor 的结论只来自本机只读的日志与文件。

## 1. 背景与决定

两个现象指向同一件事——「需要我处理」的动作分散在两处，其中一处几乎没有覆盖：

1. 手机 Agents 页「需要处理」分组里，两个 Codex 会话（11-02 22:09 因用量上限出错）一直不消失。它们早被
   「无进程 1 小时」的 TTL 判为已结束，分组却仍收纳已结束会话；「要求继续」对已结束会话是灰的，也没有任何
   忽略方式。这两次出错发生在错误卡上线之前，而错误卡规格明确不为历史失败补弹，所以与 Mac／iOS 版本无关。
2. 11-03 Cursor 连续三次出错（05:47、08:59、12:53，都发生在等待 AskHuman 回答期间）没有出现任何提醒，
   会话随后一路变成「已结束」。原因见 §4.1：Cursor 在这类出错时既不发 `stop` 也不发 `sessionEnd`。

用户决定（2026-11-03）：错误卡已经是弹窗，**所有需要处理的动作都放进收件箱**；Agents 页只看状态，不再提供
「要求继续」之类的操作。旧手机版本不必兼容（手机端尚未发布，只有用户本人在用）。

## 2. 已实现：Agents 页只报告

- 分组为「等待回答 → 运行中 → 空闲 → 已结束」，没有「需要处理」。失败（红「上轮失败」）／未完成（橙「未完成」）
  与原因只在会话既不在运行也不在等待回答时显示；已结束的会话照样显示它最后的结果。
- 列表行与详情页没有任何继续按钮。可接收消息的空闲会话仍有详情页的自由消息栏——它是普通消息，不是失败的处理入口；
  发送即开始新一轮，旧结果随之清除。
- 手机线协议 `InterjectSend` 去掉 `expectedResultId`，Mac 不再接收该字段。错误卡内部的原会话接续仍用
  `expected_result_id` 校验（`agent_control::send`），不受影响。
- 那两张 Codex 卡随新版 App 消失，不需要手动处理。

## 3. 只用收件箱时还缺什么

错误卡（F1–F8）已覆盖「hook 报告了最终失败」的所有 Agent。拿掉 Agents 页的按钮后，下列情形原先靠 Agents 页兜底，
现在要么补上，要么明确放弃：

| 情形 | 只看收件箱时 | 方案 |
| --- | --- | --- |
| Cursor 重试用尽后放弃 | **没有卡**：Cursor 不发 `stop`／`sessionEnd`（§4.1） | §4 从 Cursor 日志检测 |
| 卡未处理时 daemon 换新或重启 | 卡只在内存里，丢了再没有入口；未处理的卡还是一个真实在途请求，会让换新一直排空等它（`begin_drain` 等 `active_count()==0`），期间所有新提问排队 | §5.1 卡片持久化，并不再拦换新 |
| 点了「知道了」又想继续 | 卡已关 | 不另设入口：可接收消息的会话用 Agents 详情页自由消息，或回原 Agent（§3.1「关闭不改失败事实」不变） |
| 暂时不可接续（桌面断开等） | 卡上写明原因，只给「知道了（暂时无法接续）」 | 不变 |
| 未完成（`incomplete`） | 无卡 | 不做（I8）：只有 Grok 的 `max_turns`／`no_progress` 产生，用户的 Grok 开关是关的 |
| 已结束的会话 | 无卡 | 不做：已结束就不能继续，不该要求处理 |
| 不支持接续的 Agent | 「知道了（该 Agent 暂不支持接续）」 | 不变（F6） |

## 4. Cursor 出错检测

### 4.1 证据（本机只读）

- 11-03 的三次最终失败（会话 `c53277a5…`，Cursor 3.27.22；回合 `7ed62884`、`1b670aca`、`9b81d7cb`）在 hook 日志里
  最后一个事件都是挂着的 `AskHuman` 工具调用的 `preToolUse`；之后既没有 `stop` 也没有 `sessionEnd`，下一个事件是
  用户手动续跑的 `beforeSubmitPrompt`（新的 generation id，07:23:26／09:42:33／17:58:14）。现有 `stop(status=error)`
  路径对这类失败完全失明；会话状态只靠兜底变化：工作中 →（30 分钟）空闲 →（1 小时无进程 TTL）已结束，卡从未创建。
- Cursor 在回合真正结束时会写一行 `agent.turn.outcome`，位置
  `~/Library/Application Support/Cursor/logs/<启动>/window*/exthost/anysphere.cursor-always-local/Cursor Structured Logs*.log`，
  格式 `YYYY-MM-DD HH:MM:SS.mmm [info] {json}`。`metadata` 里有：`outcome`（`success`／`error`／`cancelled`）、
  `conversation_id`（= 会话 ID）、`request_id`（= hook 的 `generation_id` = registry 的回合 ID）、`error_text`、
  `error_type`、`error_code`、`retries_attempted`、`turn_type`（`new`／`resume`）、`turn_number`、`cancel_source`。
  `retries_attempted`、`turn_number`、`duration_ms` 是**字符串**。
- 本机保留的 7 次启动、22 个文件里（10-19 起）共 35 条：21 条 error（全部 `error_type=retriable`；文本为「Agent turn
  stopped after repeated resume attempts made no progress」13 条、「Connection failed repeatedly」7 条、
  「Connection stalled repeatedly」1 条）、9 条 success、5 条 cancelled（`user_stopped_generation`／`cancellation_token`）。
  11-03 三条 error 的 `request_id` 与上面三个回合一一对应，`retries_attempted` 为 6／4／0。重试发生在同一个
  `request_id` 内，error 行是重试用尽后的最终结果，所以不违反 F1（不为自动重试发卡）。
- 出错时挂着的提问，用户事后的回答不会被 Agent 直接读到（回合已终止）。11-03 三次续跑后的 Agent 要么先运行
  `AskHuman --show-last` 取回（07:23:36），要么重新执行同一个 AskHuman 命令（09:42、17:58）。
- Cursor `state.vscdb` 里的会话状态只有 none／completed／aborted，没有出错；诊断行 `[AGENT_ERROR_DIAGNOSTICS]` 要靠
  请求 ID 间接关联。结构化日志是唯一一处一行带齐「会话、回合、最终出错」的来源。

### 4.2 检测方式

- **只读尾随**。仅 macOS 的 Cursor Desktop；daemon 在有 Cursor 会话处于「工作中」时每 5 秒查一次，没有就不读盘。
  起点是文件末尾，不回放历史；新出现的文件（新窗口、新启动）从头读；文件变小视为轮转，从头读。只处理以换行结尾的
  完整行，单次读取有上限，超长行直接跳过。
- 只认 `message=="agent.turn.outcome"` 且 `metadata.outcome=="error"`，并且 `conversation_id`、`request_id` 非空；
  `error_type` 不筛选（以后出现非 retriable 的最终失败同样要报）。其它一律忽略。无法解析的行不报错，计数后每小时
  至多在 `daemon.log` 记一行，格式漂移因此可见。
- 命中后向 registry 提交一次合成的 `TurnEnd`：outcome=Failed，来源 `cursor-log`，回合 ID = `request_id`，
  原因摘要 = `error_text` 的有界文本（≤500 字符），并带上重试次数；不保存该行的其它字段（工作区、trace 等）。
- registry 的回合保护照常生效，**不另写去重**：`request_id` 必须是该会话当前回合才被接受。用户已续跑（新回合已开始）、
  同一失败已由 `stop(status=error)` 报告、会话已结束、未知会话，一律被拒，不会出第二张卡。
- 与「结束／出错时确认」共用同一个 Cursor 开关（F2），不新增开关；开关关闭时不读日志、不改状态。
- 不做：Windows／Linux（日志路径未核实）、Cursor CLI（`stop` hook 照旧）、daemon 停机期间的补扫。

### 4.3 时序（以 11-03 的真实形态为准：出错时有一个提问挂着）

| 时刻 | 发生什么 | 各处看到 |
| --- | --- | --- |
| T0 | Cursor 放弃并写入 error 行；`AskHuman` 命令进程没有被杀，仍在等人 | 收件箱：那个提问仍在；Agents：等待回答 |
| T0 + ≤5 s | 尾随读到，合成 `TurnEnd(Failed)`；registry 工作中 → 空闲，上轮结果为失败 | 无变化（在途提问优先于旧结果） |
| 同上 | `failure_confirmation::observe` 登记卡 `failure:cursor:{sid}:{result.id}`；会话有在途提问，**沿用「在途提问优先」**，卡先不出 | 无变化 |
| T1 用户回答那个提问 | 答案进历史（Agent 读不到）；会话不再有在途提问，卡立刻入收件箱／弹窗，推送沿用 Mac 空闲门槛 | 收件箱：失败卡；Agents：空闲，红色「上轮失败」 |
| T1 之后 点「要求继续」 | `agent_control::send` 核对会话空闲、结果未变、桥可用，向原会话发送续跑指令（§5.2） | Cursor 里开始新一轮 |
| 新一轮开始 | `beforeSubmitPrompt` → `TurnStart`：撤销卡、清除旧结果 | Agents：运行中 |
| 出错时**没有**提问挂着 | T0 + ≤5 s 卡直接出现，其余同上 | |
| 用户直接在 Cursor 里手动续跑 | `TurnStart` 撤销卡 | |

卡在会话还「活着」时才有效：卡本身是带 `agent_session_id` 的在途请求，daemon 每 15 秒会刷新这类会话的活动时间，
所以它不会因 1 小时 TTL 被判为已结束而被撤销。「知道了」之后失败结果仍留在 Agents，会话照常走 TTL 到「已结束」。

### 4.4 卡片内容

沿用 `agents/failure.rs` 的错误卡：标题「Agent 本轮出错了，接下来怎么处理？」，正文为「Cursor · 时间」加原因。
Cursor 的原因是 Cursor 自己的英文原文（如「Agent turn stopped after repeated resume attempts made no progress」），
追加一行「已重试 N 次」；不翻译、不当作指令。选项由现有能力判定（`agent_control::can_send`）：桥可用给「要求继续」
＋「知道了」，不可用给 F6 的「知道了（…）」。Cursor 桥只做过静态与替身验证，首次真实使用就是第一次自然发生的失败（I9）。

### 4.5 挂着的提问（需要你选）

`agent_control::send` 在会话仍有在途提问时直接拒绝（`stateChanged`），所以出错时挂着的提问必须先了结，才能从卡上续跑：

- **A. 沿用现有规则（推荐）**：卡排在提问后面，你回答（或取消）那个提问之后立刻出现，再一键「要求继续」。零新机制、
  不替你取消任何东西；代价是出错通知要等到你回答那个提问才出现。按 `history.jsonl`，11-03 的三次你分别在出错后
  42 分钟、15 分钟、约 2 小时回答了挂着的提问；A 把「回答后发现 Agent 没反应、再去 Cursor 里看」变成回答后直接出卡。
- **B. 出错即处置**：确认出错的同时取消该会话挂着的提问（它属于已终止的回合，没有 Agent 会读），把提问正文放进
  卡片，让「补充指令」框兼作作答。出错通知最早，但要新增「按会话取消在途请求」，会让你正看着的提问窗口消失，
  还要处理提问里的附件与选项；比 A 大得多。

## 5. 两处配套完善

### 5.1 卡片跨换新保留，并且不再拦换新

未处理的错误卡最长挂 24 小时，而它是真实在途请求：用户这时 `install.sh`，下一次有人提问就会进入「排空等待」，
一直卡到有人回答这张卡，或手动 `daemon restart --force`。收件箱成为唯一入口后这种卡只会更多，所以：

- 建卡时把「卡 ID（`failure:{kind}:{sid}:{result.id}`）＋创建时间」写入 `~/.askhuman/failure-cards.json`（原子写），
  卡被回答、撤销（新回合、会话结束、开关关闭）或到期时移除。
- 排空判定与 `daemon status` 的在途数**不计**这类卡；daemon 退出时它们照常被取消。新 daemon 启动后，对仍满足
  「会话未结束、上轮结果仍是同一个失败、开关仍开、未满 24 小时」的条目重新建卡并刷新会话活动时间（否则恢复出来的
  会话会被 TTL 立刻判为已结束）。只恢复**这个功能建过且还没处理的卡**，不为从未建过卡的历史失败补弹（§5 的约束不变）。
- 另外，`observe()` 在排空期间直接返回，排空期间发生的最终失败今天会永远没有卡；有了上面的文件后改为登记并在新
  daemon 上出卡。

### 5.2 续跑指令提示读回答

错误卡的「要求继续」目前发送的是固定的 `CONTINUE_PROMPT`。只给错误卡的续跑追加一句：
「如果出错时你正在等待 AskHuman 的回答，先运行 `AskHuman --show-last` 读取它。」普通的自由消息不加。
续跑后的 Agent 不必猜测、也不必重新提问就能取回你在出错期间给的回答。

## 6. 决定

| 编号 | 决定 | 状态 |
| --- | --- | --- |
| I1 | 处理动作只在收件箱的错误卡；Agents 页只报告结果 | 已确认并实现 |
| I2 | 不兼容旧手机，手机线协议去掉 `expectedResultId` | 已确认并实现 |
| I3 | 已结束的会话不再作为「需要处理」的对象 | 已确认并实现 |
| I4 | Cursor 出错检测：只读尾随结构化日志，仅 macOS，共用 Cursor 的确认开关 | 待确认（推荐做） |
| I5 | 挂着的提问：方案 A（沿用在途提问优先） | 待确认（推荐 A） |
| I6 | 错误卡续跑指令追加「先 `AskHuman --show-last`」 | 待确认（推荐做） |
| I7 | 卡片跨换新保留，并且不再拦换新 | 待确认（推荐做） |
| I8 | `incomplete` 不出卡 | 待确认（推荐不做） |
| I9 | Cursor 卡按现有能力判定是否给「要求继续」，不为 Cursor 单独关掉 | 待确认（推荐） |

## 7. 验证与边界

- 能自动验证的：解析夹具（把 21 条真实 error 行脱敏后入库，另加 success／cancelled／缺字段／字符串数值／超长行）、
  尾随（从末尾开始、追加、半行、轮转、新文件、权限错误、读取上限）、registry 回合保护（已续跑／已有 stop(error)／
  未知会话／已结束／重复行）、卡片时序（提问挂着时延后、回答后出现、续跑后撤销、24 小时到期、开关关闭）、持久化
  （写入、恢复、已处理不恢复、到期不恢复、恢复后不被 TTL 判结束、排空计数排除）。测试里用环境变量把日志目录指向
  临时目录，不碰真实的 Cursor 目录。
- 不能验证的：真实的 Cursor 出错（只能等自然发生）、Cursor 桥的「要求继续」（永不实测）、Cursor 升级后日志格式漂移
  （靠 `daemon.log` 的漂移行发现）。上线后第一次自然发生的 Cursor 出错要对照核实，结果写进 `docs/PROGRESS.md`。
- `daemon.log` 新增两类行：检测到失败（会话、`request_id`、重试次数，不含原文）与格式漂移。
- 与 [Cursor 等待期保活](../investigations/cursor-idle-wait-keepalive.md) 的关系：保活是降低出错频率，本文是出错后
  仍有入口；11-03 的三次失败都发生在等待 AskHuman 回答期间，两条线互不依赖，也不互相替代。

## 8. 实现落点

- `agents/cursor_log.rs`（新）：行解析与尾随状态，纯函数加临时目录测试。
- `daemon/runtime/agent_control.rs`：新增 `observe_cursor_failure`，走现有 `apply(...)`（与 Codex Desktop 观察、
  Codex CLI 观察同一接缝），由 registry 决定是否接受，再进 `failure_confirmation::observe` 与 iOS 网关的 `agent_event`。
- `daemon/runtime/failure_confirmation.rs`：卡持久化与恢复、排空期登记；`daemon/runtime/mod.rs` 的 `begin_drain` 与
  `status` 的在途数排除这类卡；`CONTINUE_PROMPT` 的错误卡专用后缀。
- iOS：无改动（错误卡是普通提问）。`PROTOCOL_VERSION` 不变（没有新增 IPC 消息；`failure-cards.json` 只是本地文件）。
