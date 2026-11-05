# Agent 失败处理统一到收件箱，并检测 Cursor 的出错停止

> 状态：2026-11-03 定稿（I1–I9 均经用户确认），2026-11-04 实现并验证：全量 `cargo test`（2376 个通过）、CI 同参数
> `clippy -D warnings`、隔离冒烟 `scripts/agent-failure-cursor-smoke.mjs`（§7，八个场景）对 `install.sh` 装出的二进制通过。
> 装机当天第一次自然出错（两次）暴露了卡会在约 2 秒内自己撤销的问题，已修复（§5.3）；修复后又自然出错三次（11-04），
> 卡都留到用户处理、「要求继续」经 Cursor 桥都让原会话开始了新一轮（核实记录见 `docs/PROGRESS.md`）。
> Cursor 的真实出错与「要求继续」从未、也不会主动实测（会计费），只用自然发生的失败来核实。
> 未出 TestFlight 包（iOS 只涉及 Agents 页，等用户说出包）。
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
凡是能检测到的意外结束都弹卡；即使无法接续，也当作通知弹出（I8）。

## 2. Agents 页只报告

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

| 情形 | 只看收件箱时 | 处理 |
| --- | --- | --- |
| Cursor 重试用尽后放弃 | Cursor 不发 `stop`／`sessionEnd`（§4.1），原本没有卡 | §4 从 Cursor 日志检测（已实现） |
| 卡未处理时 daemon 换新或重启 | 卡只在内存里，丢了再没有入口；未处理的卡还是真实在途请求，会让换新一直排空等它（`begin_drain` 等 `active_count()==0`），期间所有新提问排队 | §5.1 卡片持久化，并不再拦换新（已实现） |
| 点了「知道了」又想继续 | 卡已关 | 不另设入口：可接收消息的会话用 Agents 详情页自由消息，或回原 Agent（失败事实不因关闭而改变） |
| 暂时不可接续（桌面断开等） | 已支持原生接续的卡保留「要求继续」及原期限；点击时说明原因，明确未发送时可再次点击 | 2026-11-05 修订，见 §9 |
| 未完成（`incomplete`） | 原本无卡 | **出卡**（I8）：Grok 因 `max_turns`／`no_progress` 停下，与出错一样需要你知道 |
| 已结束的会话 | 已登记而未处理的失败卡仍保留至检测后 24 小时；原生支持接续时可要求继续 | 不为从未登记的历史失败补弹；见 §9 |
| 不支持接续的 Agent | 「知道了（该 Agent 暂不支持接续）」 | 不变（F6），当作通知 |
| 进程在回合中途消失（崩溃、被杀、终端关闭） | 无卡：没有任何来源报告「失败」，只能由 TTL 判为已结束 | 未纳入本次；不猜测失败（见 §7） |

## 4. Cursor 出错检测

### 4.1 证据（本机只读）

- 11-03 的三次最终失败（会话 `c53277a5…`，Cursor 3.27.22；回合 `7ed62884`、`1b670aca`、`9b81d7cb`）在 hook 日志里
  最后一个事件都是挂着的 `AskHuman` 工具调用的 `preToolUse`；之后既没有 `stop` 也没有 `sessionEnd`，下一个事件是
  用户手动续跑的 `beforeSubmitPrompt`（新的 generation id，07:23:26／09:42:33／17:58:14）。现有 `stop(status=error)`
  路径对这类失败完全失明；会话状态只靠兜底变化：工作中 →（30 分钟）空闲 →（1 小时无进程 TTL）已结束，卡从未创建。
- Cursor 在回合真正结束时会写一行 `agent.turn.outcome`，位置
  `~/Library/Application Support/Cursor/logs/<启动>/window*/exthost/anysphere.cursor-always-local/Cursor Structured Logs*.log`，
  格式 `YYYY-MM-DD HH:MM:SS.mmm [info] {json}`（前缀之后是一个 JSON 对象）。JSON 顶层有 `level`、`key`、`message`、
  `metadata`；`message` 为 `agent.turn.outcome`。`metadata` 里有：`outcome`（`success`／`error`／`cancelled`）、
  `conversation_id`（= 会话 ID）、`request_id`（= hook 的 `generation_id` = registry 的回合 ID）、`error_text`、
  `error_type`、`error_code`、`retries_attempted`、`turn_type`（`new`／`resume`）、`turn_number`、`cancel_source`，
  另有大量耗时字段。`metadata` 的值**全是字符串**（含 `retries_attempted`、`turn_number`、`duration_ms`）。
- 本机保留的 7 次启动、22 个文件里（10-19 起）共 35 条：21 条 error（全部 `error_type=retriable`；文本为「Agent turn
  stopped after repeated resume attempts made no progress」13 条、「Connection failed repeatedly」7 条、
  「Connection stalled repeatedly」1 条）、9 条 success、5 条 cancelled（`user_stopped_generation`／`cancellation_token`）。
  11-03 三条 error 的 `request_id` 与上面三个回合一一对应，`retries_attempted` 为 6／4／0。重试发生在同一个
  `request_id` 内，error 行是重试用尽后的最终结果，所以不违反 F1（不为自动重试发卡）。
- 日志按约 5 MB 轮转：当前文件改名为 `….N.log`，同名新文件重新开始；59k 行里绝大多数与出错无关。
- 出错时挂着的提问，用户事后的回答不会被 Agent 直接读到（回合已终止）。11-03 三次续跑后的 Agent 要么先运行
  `AskHuman --show-last` 取回（07:23:36），要么重新执行同一个 AskHuman 命令（09:42、17:58）。
- Cursor `state.vscdb` 里的会话状态只有 none／completed／aborted，没有出错；诊断行 `[AGENT_ERROR_DIAGNOSTICS]` 要靠
  请求 ID 间接关联。结构化日志是唯一一处一行带齐「会话、回合、最终出错」的来源。

### 4.2 检测方式（`agents/cursor_log.rs` 与 `agent_control::watch_cursor_log`）

- **只读尾随**。仅 macOS 的 Cursor Desktop，且不是项目 Dev 实例；Cursor 的「结束／出错时确认」开关开着时 daemon
  每 5 秒读一次，关着就不读盘（开关重新打开后从文件末尾重新开始，期间发生的不算新闻）。只打开文件读、从不写，
  不碰 Cursor 的数据库，也不与 Cursor 通信。
- 只看最新两次启动目录下的 `Cursor Structured Logs*.log`（最多 32 个，按修改时间新的优先）；每 10 秒重新枚举一次。
  **所有文件都从末尾开始**，包括之后才出现的文件：不回放历史；代价是新窗口刚创建的头 10 秒内的行会漏读，而出错行
  不可能出现得那么早。已跟踪文件被替换（inode 变化）或变小视为轮转，从头读。只处理以换行结尾的完整行；半行等下次；
  单次最多读 1 MiB（积压在后续几拍追上）；超过 64 KiB 的行直接丢弃（模型清单之类，从不是 outcome）；不可读的文件
  跳过并保留位置。
- 只认 `message=="agent.turn.outcome"` 且 `metadata.outcome=="error"`，并且 `conversation_id`、`request_id` 是非空、
  ≤128 字符的标识符（字母数字、`-`、`_`）；`error_type` 不筛选（以后出现非 retriable 的最终失败同样要报）。
  其它一律忽略。看起来是 outcome 行却读不出所需字段的计为「格式漂移」，每小时至多在 `daemon.log` 记一行。
- 命中后，**仅当会话已被 hook 登记过**（registry 里有活动或最近结束的记录）才向 registry 提交一次合成的
  `TurnEnd`：outcome=Failed、原因码 `runtime_error`、来源 `cursorLog`、回合 ID = `request_id`、摘要 = `error_text` 的
  有界文本（≤320 字符、去控制字符）加一行「Cursor 已重试 N 次。」。这条日志列出 Cursor 跑过的所有会话（子代理、
  其它窗口、daemon 启动前的会话），没有 hook 登记的会话没有回合可结束，也不应被这个来源凭空创造出来。
  其它字段（工作区、trace 等）一概不保存。
- registry 的回合保护照常生效，**不另写去重**：`request_id` 必须是该会话当前回合才被接受。用户已续跑（新回合已开始）、
  同一失败已由 `stop(status=error)` 报告、已有同回合的失败结果，一律被拒，不会出第二张卡。
- 与「结束／出错时确认」共用同一个 Cursor 开关（F2），不新增开关；集成模式为 None 时不动作。
- `daemon.log` 新增两类行：`cursor log: failed turn session=<前 8 位> retries=N`（不含原文）与格式漂移行。
- 不做：Windows／Linux（日志路径未核实）、Cursor CLI（`stop` hook 照旧）、daemon 停机或换新瞬间的补扫。
- 已知边界：会话已超出 registry「最近结束」列表（保留 10 条）时没有记录，对应的失败不出卡。11-03 的三次都发生在
  等待提问期间，会话活动每 15 秒被刷新，不会走到这一步；长时间完全无动静后才失败的会话才可能撞上。

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

2026-11-05 修订：会话活动与错误卡寿命分开。卡不刷新会话活动；会话可以按一小时无活动规则进入 ended，
但未处理卡及它所引用的失败记录保留至首次检测失败后 24 小时，不受普通 ended 历史 10 条容量淘汰。
「知道了」、新回合、明确 sessionEnd、关闭确认开关或到期撤销卡。见 §9。

### 4.4 卡片内容

沿用 `agents/failure.rs` 的错误卡：标题「Agent 本轮出错了，接下来怎么处理？」，正文为「Cursor · 时间」加原因。
Cursor 的原因是 Cursor 自己的英文原文（如「Agent turn stopped after repeated resume attempts made no progress」），
追加一行「Cursor 已重试 N 次。」；不翻译、不当作指令。2026-11-05 起，选项区分原生接续能力和即时就绪：已支持原会话接续的卡保留
「要求继续」＋「知道了」，临时断线在点击时报告且保留重试；本来不支持原会话接续的 Agent 仍只给「知道了」。Cursor 桥只做过静态与替身验证，
首次真实使用就是第一次自然发生的失败。

### 4.5 挂着的提问（I5：沿用「在途提问优先」）

`agent_control::send` 在会话仍有在途提问时直接拒绝（`stateChanged`），所以出错时挂着的提问必须先了结，才能从卡上续跑。
决定沿用现有规则：卡排在提问后面，你回答（或取消）那个提问之后立刻出现，再一键「要求继续」。零新机制、不替你取消
任何东西；代价是出错通知要等到你回答那个提问才出现。按 `history.jsonl`，11-03 的三次你分别在出错后 42 分钟、
15 分钟、约 2 小时回答了挂着的提问。

未采纳的备选：出错即处置——确认出错的同时取消该会话挂着的提问并把正文放进卡片。通知最早，但要新增「按会话取消在途
请求」，会让正看着的提问窗口消失，还要处理提问里的附件与选项。

## 5. 两处配套完善

### 5.1 卡片跨换新保留，并且不再拦换新

未处理的错误卡最长挂 24 小时，而它是真实在途请求：用户这时 `install.sh`，下一次有人提问就会进入「排空等待」，
一直卡到有人回答这张卡，或手动 `daemon restart --force`。收件箱成为唯一入口后这种卡只会更多，所以：

- 建卡时把「Agent 种类、会话 ID、结果 ID、创建时间」写入 `~/.askhuman/state/failure-cards.json`（原子写；每个会话
  至多一条）；卡被回答、撤销（新回合、会话结束、开关关闭、被更新的结果取代）或满 24 小时时移除，没有卡时文件不存在。
- **这类卡不再拦换新**：`RequestEntry::held` 由 `native_request_id` 的 `failure:`／`failure-control:` 前缀决定，
  `RequestRegistry::active_count()`（排空判定、`daemon status`、SIGTERM／`daemon stop` 是否排空）不计它们；
  `held_count()` 只写进 `drain complete` 日志。该前缀只由 daemon 内部提问设置，CLI 无法设置。
  菜单栏图标与托盘「待答」列表用另一个计数 `in_flight_count()`，仍包含这些卡：它们照样在等你回答，列表本身
  （`pending_infos`）也列出它们，数字和列表要对得上。
- daemon 关停时（accept 循环退出后、取消所有请求之前）先 `freeze()`：此后卡被取消只是进程结束，不是你处理了它，记录里
  的条目保留；冻结之后新出现的卡仍会写入，不会漏。SIGKILL／崩溃同样留下记录。
- 新 daemon 启动 2 秒后 `restore()`：对每个条目先刷新会话活动时间（否则恢复出来的会话会被 TTL 立刻判为已结束），再核对
  「会话仍有同一个上轮结果、该结果仍需要处理、会话空闲、开关仍开、未满 24 小时」，满足就重新建卡（剩余寿命 = 24 小时
  减去已过时间，创建时间不变），不满足就移除条目。加载时已过期、种类未知或字段缺失的条目直接丢弃。只恢复**这个功能
  建过且还没处理的卡**，不为从未建过卡的历史失败补弹。
- 排空期间发生的最终失败：旧行为是 `observe()` 直接返回，永远没有卡；现在登记进记录，由接任的 daemon 出卡。
- 换新时旧 daemon 会把仍在途的卡取消（IM 卡定格为「已取消」，弹窗关闭），新 daemon 再发一张同内容的卡。

### 5.2 续跑指令带一个中性提示

错误卡的「要求继续」发送固定的 `CONTINUE_PROMPT`；它在末尾追加中性的一句：
「If needed, you can run `AskHuman --show-last` to see the latest AskHuman question and the answer given to it.」
不假设 Agent 当时在做什么（不说「如果你在等 AskHuman 的回答……」）。因为 `CONTINUE_PROMPT` 同时用于手机的空闲消息接续，
这句话对所有「要求继续」一视同仁；普通的自由消息不带它。

### 5.3 卡自己的提交不算 Agent 又在工作

2026-11-04 的两次自然出错（07:08:59 与 07:56:18，会话 `539e12dc`、`05275d1a`）检测与登记都对，卡却在建立后约 2 秒就被
撤销，没等到人看：开着「按需发送」（日常配置 `autoActivation: true`）时，daemon 受理每个提问都会把提问所属的 Agent
登记为工作中；错误卡本身也是带会话 ID 的提问，于是刚失败的会话被拉回「工作中」，而卡每 2 秒核对一次「会话仍空闲、结果
未变」，一读到「工作中」就自己撤销。`539e12dc` 那次出错时挂着一个提问，卡在提问被回答后才提交，同样 2 秒内被撤销。
证据：`history.jsonl` 里各有一条 caller 取消的卡记录；失败的会话停在「工作中」，失败结果还在，`activeSince` 正是卡提交
的那一秒。

修复：`marks_session_working(auto, from_mcp, native_request_id)` 决定一次提交是否登记 Agent 为工作中，daemon 自持的卡
与续跑反馈卡（`is_card_id`）一律不登记，只刷新活动时间。卡是同一条提交路径，按代码判断，由 hook 上报最终失败的
Codex、Claude 等的卡在开着该开关时也会这样（未单独观察）。更早的 `scripts/agent-failure-smoke.mjs` 与本功能最初的冒烟
都把 `autoActivation` 关着，所以没有发现；冒烟现在多一个场景（§7 的 8）在开着它时核对卡能一直等到回答，修复前该场景
对装出的二进制失败。那两张被撤销的卡不补弹（I7：不为历史失败补弹）。

## 6. 决定

| 编号 | 决定 | 状态 |
| --- | --- | --- |
| I1 | 处理动作只在收件箱的错误卡；Agents 页只报告结果 | 已确认并实现 |
| I2 | 不兼容旧手机，手机线协议去掉 `expectedResultId` | 已确认并实现 |
| I3 | 已结束的会话不再作为「需要处理」的对象 | 已确认并实现 |
| I4 | Cursor 出错检测：只读尾随结构化日志，仅 macOS，共用 Cursor 的确认开关 | 已确认并实现 |
| I5 | 挂着的提问：沿用「在途提问优先」，卡排在提问后面 | 已确认（零新机制） |
| I6 | 续跑指令追加中性提示（需要时可运行 `AskHuman --show-last`），不做假设性说明 | 已确认并实现 |
| I7 | 卡片跨换新保留，并且不再拦换新 | 已确认并实现 |
| I8 | 能检测到的意外结束都弹：`incomplete` 也出卡；不能接续的当作通知 | 已确认并实现 |
| I9 | Cursor 卡按现有能力判定是否给「要求继续」，桥不可用时只给「知道了」 | 已确认并实现 |

## 7. 验证与边界

自动化（`cargo test`、`clippy --all-targets -D warnings`）：

- 解析：真实形态的 error 行（时间前缀、字符串值）、success／cancelled／其它 message、缺字段／坏 JSON／标识符不合法
  的「格式漂移」行、数值型重试、控制字符与超长文本的清洗、摘要措辞。
- 尾随：历史不回放、追加只报一次、半行等换行、文件在行中间被发现、轮转（改名换新文件／截断／inode 变化）、新窗口、
  只跟最新两次启动与结构化日志、根目录或文件消失、超长行、大积压多拍追上、漂移每小时至多一报、不可读文件。
  测试里日志根目录是临时目录，从不触碰真实的 Cursor 目录。
- registry：`knows_session` 不创建记录；回合保护沿用已有用例（旧回合、已续跑、重复结果）。
- 卡：持久化往返与保留创建时间、答案／取代／撤销移除、关停冻结不抹掉记录、排空期登记、过期／未知种类／坏文件不恢复、
  表容量上限；`active_count` 不计 held 卡且其它 native id 照常计，`in_flight_count` 与 `pending_infos` 的条数一致；
  `Incomplete` 卡的标题与原因措辞；iOS 网关对 `Incomplete` 的去重与「卡没送到则回退为提醒」。

隔离冒烟 `node scripts/agent-failure-cursor-smoke.mjs [AskHuman 路径]`（默认 `src-tauri/target/local-install/AskHuman`；
约 80 秒）：私有 HOME 与独立 daemon、回环 Telegram 替身、伪造的 Cursor 日志行（`ASKHUMAN_CURSOR_LOG_ROOT` 指向临时
目录），不启动 Cursor，不产生任何 Cursor 请求。八个场景：

1. 伪造的 error 行 → 一张卡与一条 `source=cursorLog`、回合 ID 对得上的失败结果；陌生会话被忽略（不凭空创建会话）；
   重复的行不出第二张卡。
2. 卡写入 `failure-cards.json`，`status` 的 `activeRequests` 为 0（不拦换新）。
3. SIGTERM 后进程直接退出（不排空）、记录保留；重启后卡恢复，创建时间不变。
4. 在卡上作答后记录移除，失败结果仍留在 Agents。
5. 新回合撤销未答的卡。
6. 关闭开关即停止跟随，重新打开后不回放。
7. 出错时挂着一个提问：提问保持在途，卡排队，提问回答后才出现。
8. 打开「按需发送」（日常配置，见 §5.3）后，卡一直等到回答：会话保持空闲、记录仍在，回答后失败仍留在 Agents。
   场景 1–7 都在该开关关着时跑，看不到 §5.3 的问题。

对没有这一功能的旧二进制必然失败（已用旧版做过反证）；场景 8 在 §5.3 的修复前对装出的二进制失败（会话被卡自己拉回
「工作中」）。机器负载高到数百时个别命令可能超时（`agents monitor --json` 30 秒），重跑一次即可。

不能验证的：

- **真实的 Cursor 出错**（只能等自然发生）、**Cursor 桥的「要求继续」**（永不实测）、Cursor 升级后日志格式漂移
  （靠 `daemon.log` 的漂移行发现）。第一次自然发生的 Cursor 出错要对照核实，结果写进 `docs/PROGRESS.md`。
  11-04 先自然发生两次：检测、登记、卡的内容都对，但卡被自己撤销（§5.3，已修复）；修复后又自然发生三次（09:39、09:58、
  10:58）：卡留到用户在手机上点「要求继续」，Cursor 桥让原会话开始新一轮（控制日志 `started`），10:58 那次的原因文本与
  重试次数也逐项对过 Cursor 自己的结构化日志。尚未单独核实的只有「回答挂着的提问之后」才出现这条路径。
- 隔离验证只用伪造的日志文件和 hook 子命令，不启动 Cursor、不产生任何 Cursor 请求。

用户可见的注意点：

- Grok 的「结束／出错时确认」开关默认关闭；开关关着时 Grok 的 `incomplete` 不出卡（与它的 `failed` 一致）。
- 与 [Cursor 等待期保活](../investigations/cursor-idle-wait-keepalive.md) 的关系：保活是降低出错频率，本文是出错后
  仍有入口；11-03 的三次失败都发生在等待 AskHuman 回答期间，两条线互不依赖，也不互相替代。

## 8. 实现落点

- `agents/cursor_log.rs`（新）：行解析 `parse_line`、`Failure::summary` 与尾随 `Tailer`，纯函数加临时目录测试。
- `daemon/runtime/agent_control.rs`：`watch_cursor_log`（与 `watch_codex_cli` 同处启动）与 `apply_cursor_failure`，走现有
  `apply(...)`，由 registry 决定是否接受，再进 `failure_confirmation::observe` 与 iOS 网关的 `agent_event`；
  `CONTINUE_PROMPT` 的中性提示。
- `agents/registry.rs`：只读的 `knows_session`。
- `daemon/runtime/failure_confirmation.rs`：`Cards::persistent／remember／freeze／pending`、`restore`、`start_card`；
  `observe` 对 `failed` 与 `incomplete` 一视同仁。`daemon/runtime/mod.rs`：启动时 `persistent(paths::failure_cards_file())`
  与延迟 `restore`，accept 循环退出后 `freeze`。
- `daemon/request.rs`：`RequestEntry::held`、`active_count` 排除 held、`held_count`、`in_flight_count`（含 held）。
  `daemon/runtime/subs.rs`：`build_tray_state` 用 `in_flight_count`（菜单栏图标与「待答」列表）。`agents/failure.rs`：
  卡片 ID 与 `is_card_id`、`Incomplete` 的标题与原因措辞。`ios/gateway/agents.rs`：`Incomplete` 与卡的去重。`paths.rs`：
  `failure_cards_file()`。
- `daemon/runtime/mod.rs`：`marks_session_working`（§5.3；`handle_submit` 与 `spawn_agent_resolve` 都用它决定是否把提问的
  Agent 登记为工作中）。
- `scripts/agent-failure-cursor-smoke.mjs`（新）：§7 的隔离冒烟。
- iOS：本文 §2 之外无改动（错误卡是普通提问）。`PROTOCOL_VERSION` 不变（没有新增 IPC 消息；`failure-cards.json` 只是
  本地文件）。

## 9. 排空数小时后的恢复与 24 小时接续（2026-11-05）

事故、用户决定、实现方案及验证矩阵见 [Daemon 更新接班与错误卡 24 小时接续](daemon-update-and-failure-recovery.md)。
本节修订 I3/I9 和 §5.1 中“仍空闲才恢复”的限制：已登记的未处理卡不因单纯 TTL ended 消失；不为未知历史错误补卡。

- 首次检测时就写入未处理清单并给对应 registry 失败记录有限期的保留引用；普通 ended 保留 10 条外，
  额外保留最多 256 条未处理错误所需的记录，状态仍是 ended。清单为权威，启动时对齐引用，过期/已处理释放。
- 卡及其反馈不参与 Agent 活动刷新；普通在途提问仍每 15 秒刷新其会话。
- `failure-cards.json` 保留初始 createdMs，新增可选 attempt（手机身份和每次明确点击的消息 ID）；
  重启、补弹和重试不延长 24 小时。发送前持久化尝试，落盘失败则不发送。
- 明确未发送时重新提供原错误的作答卡；每次明确重试有新操作 ID，同一次点击重传复用旧 ID。
  已提交但无法确认时保留待核实卡与回执，不重新发送。原生已经开始的持久回执可结束旧卡。
- Cursor 不自动打开程序或聊天；窗口未加载原聊天时明确提示打开后重试。开始新回合仍走原会话、原子空闲与结果 ID 校验。
- 卡的身份是失败结果，单次投放使用独立 native request ID，避免一次“要求继续”的答题结果被重复提问缓存自动重放。
- 全程使用伪造日志、私有 home 和控制替身验证，禁止真实 Cursor Agent 请求。
