# Agent 最终失败确认

> 状态：2026-11-02 已按用户批准实现；独立 Dev 安装、隔离链路与 iOS 模拟失败卡验收通过，同日合并进 `main` 并装为日常版；
> 真实容量错误与失败后的原生接续尚未实测。
> 2026-11-03：手机 Agents 页不再提供「要求继续」，错误卡成为唯一处理入口；Cursor 重试用尽后不发 `stop` 的缺口由
> 结构化日志检测补上，`incomplete` 同样出卡，未处理的卡跨 daemon 换新保留（F9、F10），见
> [失败处理统一到收件箱](agent-failure-inbox.md)。
> 实施计划：[agent-failure-confirmation](../plans/agent-failure-confirmation.md)。
> 正常 Stop 与原生控制基础分别见 [Stop 确认](agent-stop-confirmation.md) 和
> [生命周期结果与接续](agent-lifecycle-results-and-recovery.md)。

## 1. 目标

Agent 因模型容量、API、连接等错误最终停止一轮时，用户能通过现有 AskHuman
作答窗口、iOS 收件箱和启用的远程渠道看到错误，并在具有可靠控制能力时要求原会话继续。
这张卡不要求任务由手机或 AskHuman 创建，也不消耗一次模型调用来让 Agent 自己报告错误。

界面复用普通 Ask 的单选、自由输入、首答胜出与历史；错误后的继续由各家适配器执行，
不能假设正常 Stop 的 `block/reason` 或 `followup_message` 在错误路径仍会被消费。

## 2. 已确认决策

| 编号 | 决策 |
| --- | --- |
| F1 | 仅整轮最终失败触发；Agent 正在自动重试、单个工具失败和用户主动取消均不发卡 |
| F2 | 与每家已有「结束时确认」共用一个偏好与开关，不新增独立错误开关 |
| F3 | 设置入口改为「结束／出错时确认」；保留已有偏好和默认值，不因升级替用户开启 |
| F4 | 所有已集成的普通主会话都纳入，与从哪里创建任务无关；子代理／委派运行沿用现有静默规则 |
| F5 | 支持原会话接续时，操作为「要求继续」＋可选补充指令，以及「知道了」 |
| F6 | Agent 暂无接续能力时，提供「知道了（该 Agent 暂不支持接续）」；不能显示一个无效的继续操作 |
| F7 | 错误卡立即进入 iOS 收件箱，系统推送沿用现有 Mac 空闲门槛，不为错误另设立即推送或延迟 |
| F8 | Grok 也提供同一个设置项，说明「仅支持出错提醒」；首版错误卡按 F6 处理 |
| F9 | 凡能检测到的意外结束都出卡：`failed` 与 `incomplete`（如 Grok 达到运行上限／没有进展）一视同仁；无法接续的按 F6 当作通知 |
| F10 | 未处理的错误卡跨 daemon 换新保留（`~/.askhuman/state/failure-cards.json`），且不再拦截换新排空 |

已有默认值为 Claude/Codex/Cursor 关闭、Pi 开启；新增 Grok 无既有偏好，按关闭初始化。
集成 mode=None 时不激活该能力，保留偏好；重新集成后恢复偏好。
共享开关不等于共享每一家的原生事件契约，也不改变正常完成时现有 Stop 的行为。

## 3. 用户看到的行为

### 3.1 错误卡

显示 Agent、会话／项目、失败时间与有界错误原因，明确表达「本轮因错误停止」。
错误原文可以包含 `selected model is at capacity`，不需要按这句英文做字符串特判。
错误原因是显示数据，不作为 shell 命令或新指令执行。

支持接续时：

- 「要求继续」使用既有原会话接续语义：检查失败原因及已完成步骤，从当前进度继续，避免重复成功的操作。
- 可附加最多 1,000 字符的补充指令；只有补充文字、未选选项时也表示要求继续。
- 「知道了」关闭卡片，不启动 Agent；即使同时带有输入文字也不发送。

不支持接续时：

- 只有「知道了（该 Agent 暂不支持接续）」；正文说明需回原 Agent 处理。
- 不提供会被丢弃的补充输入，也不将纯文字回答解释为发送成功。
- 原本支持但连接暂不可用、会话已结束等情况，说明具体原因，不能把暂时不可用说成永久不支持。

关闭、取消或沿用的 24 小时请求到期均只关闭处理卡，不重试、不停止原会话。
关闭卡片不把真实失败改成完成，Agents 页仍保留真实的上轮结果。

### 3.2 从手机操作

错误卡与普通 Ask 一样进入收件箱，是失败的唯一处理入口。手机 Agents 页只显示结果与原因，没有「需要处理」分组，
也没有「要求继续」按钮（2026-11-03 去掉）；可接收消息的空闲会话仍可在详情页发自由消息，它与错误卡共用同一份提交保护。

结果推送的旧限制（只有手机发起／接续的那一轮才有一次性通知资格）不限制新错误卡。
本功能开关关闭时，已有手机一次性结果通知保持现有行为。
同一失败已形成错误卡时，不能再为同一手机重复发送 `follow.failed` 结果提醒；若卡片未被接纳，
不得仅因尝试建卡就吞掉原有结果通知。

iOS 系统推送完全复用普通提问当前的提醒策略与配置，不暗中换用结果提醒的另一套撤销语义。
Mac 在使用时收件箱仍能看到卡片；推送等待空闲门槛。IM、Popup 按普通 Ask 的启用、活跃槽与
watch 路由投放，首个合法回答生效。

### 3.3 已在电脑继续或重复失败

真正的新回合开始后撤销旧失败卡；打开原会话、收到心跳或用户点击发送不算已经开始。
晚到的旧失败回调不能覆盖新回合，也不能重新发卡。
用户要求继续后再次发生最终失败，为新一轮形成新卡；不自动循环重试或换模型。
回合和结果身份去重，不以错误文字去重，因此相同 capacity 错误的两轮仍是不同结果。

## 4. 能力调查与覆盖边界

| 来源 | 最终失败来源 | 原会话接续 | 本期边界 |
| --- | --- | --- | --- |
| Codex Desktop / macOS | 现有原生 canonical turn 的 `failed` 与 `error.message` | 已有原生发送与持久回执 | 可复用；仍需失败卡到原生操作的完整验证 |
| Codex CLI / 共享 app-server | ≥0.163.0：只读原服务的 thread metadata 与最新 turn，严格核对本轮 ID 与 `failed` | 暂不开放；现有队列不能原子绑定预期失败结果 | 可检测已知共享服务的最终失败，卡片仅提供「知道了」；无服务／其它运行形态不猜测失败 |
| Claude Code | `StopFailure` | 当前没有接入空闲原会话控制 | 可报错，先提供 F6 的确认卡 |
| Cursor Desktop | `stop.status=error`；**重试用尽的最终出错不发 `stop`**（2026-11-03 在本机三次真实失败上确认），由只读尾随 Cursor 结构化日志的 `agent.turn.outcome` 补上，见 [agent-failure-inbox §4](agent-failure-inbox.md#4-cursor-出错检测) | 已有原生消息桥 | 静态契约与隔离替身验证；不得擅自启动或探测真实 Cursor；真实出错与续跑待自然发生后核实 |
| Cursor CLI | `stop.status=error` | 当前没有接入空闲原会话控制 | 可报错，先提供 F6 的确认卡 |
| Pi | 扩展 settled 后本轮 `stopReason=error` | 已有扩展控制与 `pi.sendUserMessage` | 复用同会话／代际控制 |
| Grok | `StopFailure` | 首版不接入接续 | 仅错误卡与 F6，正常结束确认仍不支持 |
| Antigravity | 当前无可靠生命周期／最终失败适配器 | 当前无控制适配器 | 不宣称可以检测或发卡；无事件来源与无接续能力是两个不同限制 |

这里的「可报错」仍以用户开启共享开关、受支持版本及事件确实送达为前提。
原生最终结果不可得时不能凭长时间无输出或 working backstop 合成失败。
Codex `--no-daemon`、自定义远端／独立 app-server 等不能仅凭安装了 0.163.0 就判为可控制。

### 4.1 Codex CLI 0.163.0 的具体证据

本机 CLI 帮助与该版本官方源码均具有：

- `codex queue --thread <id> --message <text>`，通过 `thread/queue/add` 投递至共享服务。
- `codex app-server proxy --sock <path>`，可接入已有共享服务，而非另起持有同一历史的模型运行进程。
- `error` 通知包含 `threadId`、`turnId`、`willRetry`；中间流错误为 `true`，影响本轮状态的最终错误为 `false`。
- 终态由回合错误决定 `failed`／`completed`；按最终终态建卡，不对每次网络重试建卡。
- 原生队列有持久身份及空闲启动；`thread/queue/start` 在服务端使用 `start_turn_if_idle`，繁忙时拒绝。

不能直接把现成 CLI 命令当成已完成的可靠接续：

- `codex queue` 每次调用新建 client message ID，不能用重复运行命令解决结果不明。
- 队列可自动派发；预检之后电脑启动新回合时，旧继续指令不能滞留到下一次空闲才被执行。
- 普通 `turn/start` 使用 `start_or_steer_turn`，不能据此保证绝不插入用户刚在电脑开始的另一轮。
- 接受、排队、实际开始是三种状态，必须有实际开始证明和可恢复的操作回执。
- 必须准确定位原服务与会话，保持模型、权限与上下文；不通过另开 `codex resume` 替代原 TUI。

2026-11-02 只读探测发现本机 CLI 共享服务 socket 拒绝连接，未为此启动服务或真实模型会话。
已检查 CLI 帮助、生成的本机协议 schema 和对应官方 tag 源码；未验证真实失败后接续、竞态或推送。

## 5. 实现约束

- Daemon 持有错误处理卡；被动报错 Hook 快速上报，不在 Hook 里阻塞 24 小时等待并返回无效裁决。
- 与正常 Stop handler 和 registry 的统一结果合流，避免同一回合两个来源重复建卡。
- 开关不依赖用户额外开启完整生命周期追踪；需要的错误事件独立安装，不悄悄把全量追踪偏好改为开启。
- 已集成 mode 内 lifecycle × shared-confirm 各组合、更新与卸载都需保持事件产物一致。
- 接续沿用现有持久操作 ID、预期结果 ID、代际／会话检查及实际开始回执；错误卡与空闲自由消息不能各发一次。
- 明确拒绝要反馈原因并保留失败事实；受理但结果不明时禁止自动重发，不能仅把 Ask 回答提交成功当成 Agent 开始。
- 新回合、结束会话与卡片过期须撤销旧操作资格；卡片不能干扰正在等待的权限／普通 Ask。
- 使用当前普通请求契约；若确需新增 IPC 载荷，按 overview 的兼容规则决定是否提升协议，不能预先假定无需升级。
- 重启沿用现有请求恢复边界；不因读到历史 failed 就批量补弹旧错误，也不把本功能扩展成通用在途请求恢复工程。
  唯一的例外是本功能自己建过且尚未处理的卡（F10）：只恢复它们，且卡不计入排空判定。
- 发生能力漂移、失联或基础设施错误时，说明不可接续；不自动提权、改模型、盲打终端按键或另起会话。

## 6. 验收要点

1. 最终失败恰一张卡；中间重试、工具错误、用户取消、未知空闲和关闭开关不发卡。
2. 电脑创建与手机创建行为一致；委派／子代理遵守原规则；Grok 仅错误、Antigravity 不误报支持。
3. 支持接续与仅知道了两种卡片；纯文字、继续附言、知道了带文字、关闭和到期的映射正确。
4. 手机收件箱、空闲门槛、Popup／IM 首答胜出，以及与结果通知不重复。
5. 旧回合迟到、电脑抢先继续、错误卡与空闲自由消息竞争、双击、回执丢失和 daemon 重启不重复发送。
6. 开关偏好保留、生命周期关闭仍能按能力报错、None 清理、Codex Hook 信任与版本能力更新。
7. Codex CLI 每种运行形态独立声明能力；共享服务接续需先通过 P0，真实启动／发送另经用户批准。

## 7. 依据

- [Codex Hooks](https://developers.openai.com/codex/hooks)：正常 Stop 的 continuation 契约。
- [Codex App Server](https://learn.chatgpt.com/docs/app-server)：原生回合状态、事件与请求。
- [Codex 0.163.0 队列命令](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/tui/src/session_queue_commands.rs)。
- [Codex 0.163.0 队列服务](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/ext/queue/src/service.rs)。
- [Codex 0.163.0 错误与终态映射](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/app-server/src/bespoke_event_handling.rs)。
- [Claude StopFailure](https://code.claude.com/docs/en/hooks#stopfailure)：可通知，裁决输出不被消费。
- [Cursor Hooks](https://cursor.com/docs/hooks#stop)：错误状态与 follow-up 字段；错误后不消费的实现边界沿用已有 Stop 调查，不将文档的通用字段描述当作错误路径实测。
- Grok 本机 `~/.grok/docs/user-guide/10-hooks.md`：StopFailure 为被动观察；正常 Stop gate 的新语义不等于错误可 continuation。
- 项目当前 `agents/{stop,result,report}.rs`、`daemon/runtime/agent_control.rs`、Pi 受管扩展和现有 Stop／结果接续规格。

## 8. 实现地图与证据

- `agents/failure.rs`：普通请求正文、稳定 native request ID、支持／不支持接续两种选项和答案映射。
- `daemon/runtime/failure_confirmation.rs`：daemon 持有／去重／取消错误卡，待答请求优先，转交已有原生控制与实际开始回执。
- `agents/codex_cli.rs`：已有共享服务的只读 proxy；不加载／恢复原会话，不接受任何发送或排队 RPC。
  只接受匹配 registry 当前 turn ID 的明确 failed；`active`、重试中、未知与旧回合不发卡。
- `integrations/agent_stop.rs`：共享偏好及 Grok 扩展，缺少全量 lifecycle 时安装必要事件；None 清理，保留原有默认值。
- `ios/gateway`：同结果错误卡与 Follow 提醒去重；失败投递恢复既有结果通知；手机接续仍按实际开始证明登记下一轮通知。
- 项目 Dev 的错误卡只从受管测试 profile 取得原生身份、集成来源与验证授权，不读取日常集成作为该 profile 的权限。
- 手机仍使用普通提问／答案协议，本次没有改 Swift、推送中继或 IPC 枚举，协议号不变；不需要新的 iOS App 才能回答错误卡。

自动验证已通过：Rust 2,336 项（10 忽略，2 个真实 Cursor 用例明确排除）、前端 1,079 项、
Pi 隔离脚本 6 项、`vue-tsc`、Clippy 1.99 `--all-targets -- -D warnings`。
包含容量错误原文、重试／用户取消／旧回合隔离、共享开关产物、错误卡答案与撤销、提醒去重／失败恢复等；
Codex proxy 用隔离进程替身验证仅发送只读请求，不代表已对真实 capacity 错误完成端到端验收。


2026-11-02 手机验收：独立 Dev daemon 为受管 Codex 测试 profile 的模拟最终错误建卡，经正式远程出口送到
用户的 iOS；用户提交预选「知道了（该 Agent 暂不支持接续）」，答案成功回传。
记录 `b899a626-0f9e-48de-a950-501221342aec` 为 completed／exitCode=0，source fingerprint
`3110877e152ba199506889e98730cb12a2b6f0e7f51e93db98e4af714841a2b0`、binary fingerprint
`aa9313849a90365f1d0c544d9d7abfda6691dfa0d7d7052d8270848a17a92b67`。
随后补齐临时断线与控制拒绝的提示并重新安装／验证。该证据不涉及真实模型请求、真实 capacity 失败或原生继续，
也不单独证明锁屏 APNs；手机确认和内部构建记录分别验证送达体验与构建／回传身份。
