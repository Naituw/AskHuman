# 离开模式：完成标准与带理由升级

> 状态：2026-11-06 用户已通过 AskHuman 确认第二版完整文案与规则定稿；已在主工作树实施、验证，并通过 `./scripts/install.sh` 安装日常版。验收见 §8。
> 关联：[特殊回复](special-replies.md)、[协作风格](collaboration-style.md)。本稿是特殊回复 D4 / D9 / D16 / §4.1 的已批准后续修订，替换首版相应行为；静音引导不涉及。

## 1. 已确认的目标与边界

- 修正过早停下：围绕用户原目标继续推进，调研、方案、局部实现不能自动算整个任务完成。
- 真正完成、确实无法继续时仍可通知。默认走现有升级投递；Cursor 用户可选的待阅回执见 §9。
- 保留明确范围限制。仅要求分析或设计时，以相应交付为完成目标；明确要求先审核、暂不实施时，不因离开而自行实施。
- 需要真人验收时，先做完其他不依赖该验收的工作，再升级说明；不能把未验收写成已验证。
- 升级参数应携带理由。要求 Agent 仔细检查当前情况，确认没有其他可自主推进的工作、确实需要联系用户后再填写。
- 理由放在报告最前面，所有渠道及历史都能看到；明确标为 Agent 提供，不冒充系统核验结论。
- 旧布尔值 true、空理由不直接升级；返回指导。保留第 4 次重复提问的防卡死例外，明确标注系统放行、Agent 未提供理由。
- 参数描述应简短，完整使用条件与传参方法放在离开自动回复里，使 CLI/MCP 共用。既有 ask / What's next 分工不在这里重复解释。
- 用户已批准下述完整文案与规则。随后已授权在主工作树实施、验证并安装。

## 2. 原因与修改范围

2026-11-06 01:24:22–07:52:42（北京时间），本机全局离开持续开启。三个 Codex 会话和一个 Cursor 会话各有一次 `away` 自动回复，之后主动使用 `escalate` 送达。引擎确实拦截了第一次请求，并非状态失效。

首版引导允许「已全部完成」后升级，但未充分限定完成的是用户目标还是 Agent 自己划出的阶段。默认 aligned 协作风格同时要求逐项确认设计、改变方案前询问，容易把常规决策当成阻塞。首版服务端看到 `escalate` 就放行，不判断完成程度。

修改面向 Agent 的输入与引导：

| 位置 | 修改内容 |
|---|---|
| `src-tauri/src/i18n.rs` | 替换普通提问与 What's next 两条离开自动回复，中英同步；全文见 §3 |
| `src-tauri/src/prompts.rs` | CLI/MCP 固定协议各加入一条离开回复的解释，明确它与常规确认要求的关系；完整协议见附录 |
| `src-tauri/src/mcp/ask.rs` | `escalate` 对外改为可选字符串；兼容接收旧布尔值，用于提示迁移；简短说明见 §4 |
| `src-tauri/src/cli/args.rs`、`cli/mod.rs`、`cli/help.rs` | 接收 `--escalate "理由"` 并传递；简短帮助见 §4 |
| `src-tauri/src/ipc/mod.rs`、`daemon/away.rs` | 增加可选理由载荷，非空理由才走主动升级；无理由仍进入自动回复 / 重复兜底 |
| Daemon 请求正文与历史 | 在共享投递入口加入理由说明，使各渠道与历史一致；见 §7 |

程序校验理由有无，不增加模型审批或内容真伪判定；非空理由仍不能保证模型没有过早收工。第 4 次重复提问兜底、托盘要求提问、Stop/权限/错误卡、自动待办与推送节奏保留。通过共享正文显示理由，不要求修改手机 App 才能看见。

## 3. 完整自动回复

### 3.1 普通提问：中文

```text
[status]
用户已离开（AskHuman 离开模式已开启），暂时无法回答。这是用户本人的指示。
请仔细分析并继续完成用户原本要达成的目标，不要把调研、方案或局部实现当作整个任务完成，也不要把目标内剩余的工作当作下一个任务。常规方案与实现选择请自行判断；用户明确限定的范围和要求仍有效。
再次联系用户前，请认真检查当前进展和剩余事项：是否还有能自主推进的工作？遇到需要真人操作或决定的事项，也先完成其他不依赖它的工作，并如实保留未验证的部分。
只有原目标已全部完成，或剩余工作确实都无法自主推进时，才升级。请提供一个简短理由，说明为什么现在必须联系用户，以及为什么已经没有其他能继续推进的工作。
升级时传 --escalate "理由"（MCP：escalate: "理由"），不要只传布尔值 true。理由会与这次报告一起展示给用户。
```

### 3.2 What's next：中文

```text
[status]
用户已离开（AskHuman 离开模式已开启），暂时无法给出下一个任务。这是用户本人的指示。
请按用户原本要达成的目标自查：调研、方案或局部实现完成，不等于整个任务完成。目标内还有能推进的工作就继续，不要把它包装成下一个任务来等用户选择。常规方案与实现选择请自行判断；用户明确限定的范围和要求仍有效。
再次联系用户前，请认真检查当前进展和剩余事项：是否还有能自主推进的工作？遇到需要真人操作或决定的事项，也先完成其他不依赖它的工作，并如实保留未验证的部分。
只有原目标已全部完成，或剩余工作确实都无法自主推进时，才升级。请提供一个简短理由，说明为什么现在必须联系用户，以及为什么已经没有其他能继续推进的工作。
升级时传 --escalate "理由"（MCP：escalate: "理由"），不要只传布尔值 true。理由会与这次报告一起展示给用户。
```

### 3.3 普通提问：英文

```text
[status]
The user is away (AskHuman away mode is on) and cannot answer right now. This is the user's own instruction.
Analyze carefully and keep working toward the user's original objective. Research, a plan, or a partial implementation does not complete the whole task; do not reclassify remaining work within that objective as a separate next task. Make ordinary design and implementation decisions yourself; explicit task limits and requirements still apply.
Before contacting the user again, carefully assess current progress and remaining work: is there anything else you can do independently? If something requires the user's action or decision, first finish other work that does not depend on it, and accurately identify anything that remains unverified.
Escalate only when the original objective is fully complete or none of the remaining work can genuinely proceed independently. Provide a brief reason explaining why contacting the user is necessary now and why there is no other work you can continue.
Pass --escalate "reason" (MCP: escalate: "reason"), not just the boolean true. The reason will be shown to the user with this report.
```

### 3.4 What's next：英文

```text
[status]
The user is away (AskHuman away mode is on) and cannot give the next task right now. This is the user's own instruction.
Check completion against the user's original objective: finishing research, a plan, or a partial implementation does not complete the whole task. Continue any work you can still do within that objective; do not package it as a separate next task and wait for the user to choose it. Make ordinary design and implementation decisions yourself; explicit task limits and requirements still apply.
Before contacting the user again, carefully assess current progress and remaining work: is there anything else you can do independently? If something requires the user's action or decision, first finish other work that does not depend on it, and accurately identify anything that remains unverified.
Escalate only when the original objective is fully complete or none of the remaining work can genuinely proceed independently. Provide a brief reason explaining why contacting the user is necessary now and why there is no other work you can continue.
Pass --escalate "reason" (MCP: escalate: "reason"), not just the boolean true. The reason will be shown to the user with this report.
```

有用户备注时，仍使用现有 `away.note` 格式在上述正文之后追加。`[status]` 仍由输出层添加，不重复写进 i18n 值。

### 3.5 旧格式或空理由的完整补充说明

离开期间收到旧布尔值 true、没有值的 `--escalate`，或去空白后为空的字符串时，返回对应的 §3.1/§3.2（英文对应 §3.3/§3.4）全文，并在开头追加下面一句，不直接投递。是否触发重复兜底按 §7 处理。

```text
这次升级没有提供理由，尚未送达用户。请按下面的指引自查；仍需升级时，以文字填写理由。
```

```text
This escalation did not include a reason and has not been delivered to the user. Follow the guidance below to reassess; if escalation is still necessary, provide a reason as text.
```

## 4. 完整帮助文案

### 4.1 MCP：两个工具的 escalate 参数说明

```text
Why this request must reach the user now. Use only as instructed by an AskHuman away reply.
```

### 4.2 CLI：简短 --help 条目

```text
  --escalate <理由>        带理由升级提问；仅按离开自动回复的指引使用
```

```text
  --escalate <reason>      Escalate with a reason; use only as instructed by an away reply
```

### 4.3 CLI：--agent-help 的离开段

```text
离开模式:
  AskHuman 会在自动回复中说明如何继续，以及何时可以升级。
  按该指引升级时传 --escalate "理由"；MCP 对应 escalate: "理由"。
```

```text
Away mode:
  AskHuman's automatic reply explains how to continue and when to escalate.
  When instructed to escalate, pass --escalate "reason"; for MCP, use escalate: "reason".
```

## 5. 交互协议新增条款

加入 CLI/MCP 的固定协议正文，放在恢复最近问答规则之后。它只解释 AskHuman 返回的离开自动回复；不把全局协作风格改成 autonomous，不改用户自定义正文。

完整英文条款（CLI/MCP 使用同一条，不重复原有工具分工）：

```text
- When AskHuman returns away-mode guidance, follow it and make ordinary decisions needed to continue toward my original objective without routine confirmation under the collaboration style below. Explicit task limits still apply. Provide an escalation reason only after the self-check described in that reply.
```

该条款限定的是本工具明确返回的离开指令，不能用「工具超时」「无人回答」自行推定离开或获得授权，也不能覆盖用户明确限制或宿主要求。之后用户的新指令照常生效。

## 6. 场景验收与交付边界

| 场景 | 期望行为 |
|---|---|
| 用户要求修复问题，已找到原因和可行方案 | 继续原目标内的实现与验证，不把实现作为下一任务 |
| 实现中需要普通技术调整 | 自主分析、调整、继续，最终说明重要取舍 |
| 用户只要求分析或设计 | 完成对应交付后可以升级；不能自动扩大成实施 |
| 用户明确要求审核后再实施 | 先完成其他允许的准备；需要其决定时说明边界，不能自行跨过 |
| 需要真机、真人操作，但还有独立工作 | 先完成独立工作，再以未完成任务的阻塞提问升级 |
| 必要验收是唯一剩余项 | 可以 ask 升级，明确未验证；不冒称全部完成 |
| 原目标及其要求的验证都已完成 | 可以 whats_next 升级交付 |
| Agent 建议的下一步实际上属于当前目标 | 继续做，不用 whats_next 把它改成新任务 |
| 工具返回离开自动回复 | 不把它当作人的选项回答、下一个任务或结束批准 |

实施后的验证应覆盖中英文、CLI/MCP 生成、字符串参数、旧格式与空理由处理、理由展示/历史/去重，以及重复兜底，保持其余现有机制回归通过。确定性测试只能证明正确文字和路由被交付，不能证明模型的判断一定正确。真实 Agent 的后续自然使用需观察；本设计不授权另外启动付费 Agent 测试。

正在运行的会话不能被许诺立即加载新协议：自动回复在后续被拦截的调用中送达；已升级并正在等人的调用不因此自动恢复工作。Codex 协议按现有 SessionStart/恢复/压缩入口刷新，Cursor 托管规则按现有更新路径刷新，实际加载要核对。设计不包含向现有会话主动发送消息或取消请求。

上述源码、特殊回复与协作风格的对应说明，以及用户文档已同步更新；主 overview 仅同步离开升级不变量，其他架构描述不变。实现与安装验证见 §8，真实 Agent 自然使用的行为仍需后续观察。

## 7. 输入、兼容、展示与兜底

### 7.1 对外调用

MCP 对模型展示 `escalate?: string`；未提供表示普通提问。字符串为理由，前后去空白；不要求固定格式，不用关键词猜测理由真假，不另加分类或证据字段。

```json
{
  "escalate": "实现和自动验证已完成；剩余验收必须由你在真机上操作，其他不依赖真机的工作也已完成。",
  "message": "本次成果，以及需要执行的真机步骤……"
}
```

CLI 接收 `--escalate "理由"`，也支持 `--escalate="理由"`。MCP 到 CLI 使用独立 argv 元素传值，不拼 shell 字符串。正常提问不传理由；现有问题、消息、附件、whats-next 参数保持各自用途。

旧格式 true / 无值 flag 兼容解析，但不补造理由；交给新 Daemon 返回 §3.5 的说明。旧 false / 缺省仍为普通提问。空字符串也不算有效理由。理由不加最低字数；上限取 2000 个字符，超限返回参数错误并要求简写，不静默截断。

CLI 新旧形式有不可消除的歧义：旧命令若写成 `--escalate "问题正文"`，新语法会把这个字符串读作理由；若因此没有问题或交接，必须明确报错说明新语法，不能把理由再当问题静默发送。旧命令 `"问题" --escalate` 或 `--escalate -q "问题"` 可识别为缺理由。明确 `-q` 或 `--stdin` 的旧组合仍按新语法解释，不能宣称能完美识别所有旧命令意图。

### 7.2 IPC 与旧会话

保留现有线上 `TaskRequest.escalate: bool` 供旧端表达升级请求，增加带 `#[serde(default)]` 的可选理由字段（`escalationReason: Option<String>`）。新客户端以理由字段表示带理由升级，不把字符串塞进旧布尔字段。新 Daemon 判定时区分有理由、旧请求缺理由、普通请求。

依照现有 IPC 规则，这个方案是添加可选字段，不改旧字段类型、不加枚举变体，不升协议号。新客户端不要仅为照顾旧 Daemon 再设置旧布尔值 true：旧 Daemon 忽略理由后会按普通请求走其既有离开流程，不能假装它已执行新校验。新约束以新 Daemon 为准；若进入旧二进制 hold，必须明确版本边界。

运行中的旧 MCP 进程或宿主缓存可能仍只接受布尔值，不能保证看到回复后就能成功传字符串；要刷新 MCP 工具目录/重连才能完整采用新 schema。不得为了迁移自行重启其他正在等待回答的会话。工具参数用于会话绑定的规范化指纹必须同步更新，保证字符串理由在调用与 hook 一侧一致，旧格式也有确定的一致表示。

### 7.3 服务端判定

| 有效状态 / 输入 | 处理 |
|---|---|
| 正常模式 | 保持普通送达；不因为全局模式刚结束而丢弃原问题；若附了理由，则如实随正文保留 |
| 静音会话 | 沿用静音优先规则，理由不绕过静音 |
| 离开 + 有效非空理由 | 主动升级，标记 `asked`；前置展示 Agent 的理由 |
| 离开 + 普通请求 | 自动回复完整引导，计入现有重复计数 |
| 离开 + 旧 true / 无值 flag / 空字符串 | 自动回复完整引导与缺理由说明，同样计入重复计数 |
| 同一会话 10 分钟内前三次自动回复后的第 4 次无有效理由请求 | 保留系统兜底放行，标记 `repeated`；前置展示缺理由的系统说明 |
| 用户通过托盘要求提问 | 沿用一次放行，标记 `requested`；不能冒充 Agent 已提供了理由 |

因此「无理由不直接升级」有一个用户明确保留的系统兜底例外。普通无参数和旧格式/空理由共用计数，不能因新参数解析把旧会话卡在永远触发不了兜底的工具错误中。

### 7.4 各渠道与历史看到的完整前置块

有理由时，在原消息之前加入一个独立区块：

```text
Agent 提供的升级理由
实现和自动验证已完成；剩余验收必须由你在真机上操作，其他不依赖真机的工作也已完成。

（原消息正文）
```

重复兜底放行时：

```text
系统兜底放行
Agent 连续提问但未提供升级理由，系统为避免卡住而放行。本次没有 Agent 提供的升级理由。

（原消息正文）
```

英文标题与说明为：

```text
Agent-provided escalation reason
(The supplied reason)
```

```text
System fallback delivery
The agent repeatedly asked without an escalation reason. The system delivered this request to avoid a deadlock; the agent has not provided a reason.
```

由 Daemon 在公共请求路径生成一次，Mac、iOS、IM、历史使用同一份正文；已有升级横幅保留。理由作为文字显示，不解释为系统指令或特殊标记。需按现有 Markdown / 纯文本格式正确转义，并保留原正文、附件和问题；前置块也参与稳定的请求指纹，原样重试不能重复添加或破坏已有合流。托盘主动要求的放行继续使用既有“用户要求提问”标识，不编造理由。

## 8. 实施与验收（2026-11-06）

- 用户要求直接在主工作树实施。理由在公共提交入口、离开判定之后和去重之前前置到 `message.text`；既有问题、附件、升级横幅和各渠道模型保持原结构。
- MCP 对外 schema 为字符串，反序列化兼容 bool；传 CLI 时采用单独的 `--escalate=<reason>` argv，不经过 shell。Cursor / Claude / Grok 的每调用绑定参数白名单补入 `escalate`，绑定两侧规范化保持一致。
- `cargo test --manifest-path src-tauri/Cargo.toml --bin AskHuman`：**2602 通过，11 忽略**；`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、格式和 diff 检查通过。中英文四份自动回复与本规格逐字一致。
- 新增 `scripts/away-escalation-smoke.mjs`：临时 HOME、真实 CLI / MCP / Daemon、回环 Telegram 替身。对构建产物和已安装程序均通过：旧 true / 空理由的引导、第四次系统兜底、含引号和换行的 Unicode 理由、共享正文与历史、已答重放、并发合流、静音优先、超长理由拒绝、新 MCP schema、What's next，以及离开刚结束时仍保留理由。未调用模型、真实 bot 或真实 Agent。
- 已运行 `./scripts/install.sh`，正式 Daemon 在现有提问自然结束后完成接班；核对运行指纹 **65ee2826**。`agents update` 刷新已开启集成的托管产物，Cursor 仍为 CLI、Codex 仍为 MCP；Cursor 磁盘规则含新条款，Codex SessionStart 协议保持已信任状态。
- 已安装程序的问答用于交付确认。运行中的旧 MCP 进程 / 宿主工具缓存需要正常重连才能获得字符串 schema；没有强制重启其他会话、取消其提问或主动发送消息。真实模型是否能持续正确判断原目标尚未实测，后续自然使用时观察；本次没有 iOS 出包或真机视觉验收（理由复用已有正文显示）。

## 9. Cursor 离开升级转待阅

2026-11-09 用户确认在 Cursor 集成里增加「离开时汇报后结束」，默认关闭，配置为
`cursorDesktop.endEscalationsWhileAway`。判定复用离开引擎，仅有效离开状态、可信 Cursor
来源、带非空有效理由且判定为 Agent 主动升级时生效。普通 ask 与 whats_next 都可触发。
静音、要求提问的一次放行、重复防呆与旧布尔升级继续走各自既有判定。

有 ⚡ 自动待办时优先完成队列。whats_next 沿用提交前原子领取入口；普通 ask 升级时，
工具指导 Cursor 调用 whats_next 领取，队列用尽后再带理由汇报。普通 ask 的判定先复用
whats_next 的有界同步刷新，保持 iCloud 待办读取时机一致。

队列为空时，Daemon 先把独立请求持久写入现有 owned-call metadata，再向原调用返回自动
status 回执 `[cursor_away_report_saved]` 与明确结束指令。该回执承载用户对此情形的预先授权，
CLI/MCP 协议允许 Cursor 立即结束并附结束标记，包含从普通 ask 结束。
用户次日点已阅时，原调用已取回回执，无需继续等待或自动重试。

独立卡片在当前请求列表显示「待阅汇报」，保留正文、升级理由、原问题、原选项和附件路径引用。
原问题与原选项成为只读正文，卡片仅提供「已阅」，严格单选。此选项也涵盖真正阻塞时的升级说明，
内容由 Agent 负责，系统只识别来源、有效离开判定与参数，不核验其完成声明。

待阅请求使用现有 Ask、原生请求身份及 daemon-owned Call，保留请求和渠道恢复能力。
原调用退出、会话结束、设置关闭、结束离开及服务换新都不撤走已保存的待阅汇报。
显式关停取消当前传输，owned 源仍保留，下次正常启动续建；已阅或用户明确取消后标记完成。
投递失败保留未读源并重试。保存失败时沿用人工升级，不能发出已保存回执。
相同原恢复调用重试读取同一回执，保存后、回复前崩溃也能续接。
附件沿用既有路径引用生命周期。schema 与 Task 序列化形态复用现有字段。

验证包含设置默认值与保存回滚、CLI 的普通升级与 whats_next、自动待办、静音与来源隔离，
以及私有渠道中的调用结束、服务重启、已阅和重试恢复。模型收到回执后的自然结束行为仍需实际使用观察。

### 9.1 实施与验收

已在主树实施并通过安装脚本发布到日常环境。日常 daemon 指纹为 `4598b14a`，四条 IM
连接正常，开关保持默认关闭。实现由 Cursor 设置布尔值、离开判定后的回执分支、
`runtime/cursor_reports.rs` 的 owned-call 管理及 `Store.retry_owned` 的原子重投组成。

- Rust 全量 2925 passed / 11 ignored，包含保存与原回执重放、关停续建、失败重投和已阅后保持完成。
- 前端 127 个文件、1799 项测试通过，含设置默认关闭、保存与失败回滚。Node 209 passed / 21 skipped。
- 严格 clippy、前端类型检查与安装通过。
- `scripts/cursor-away-report-smoke.mjs /Users/wutian/.local/bin/AskHuman` 使用已安装程序、
  私有 System fixture v2 与 loopback Telegram，覆盖 CLI 普通 ask、whats_next、可信 MCP 的两个入口、
  提前回执、单一已阅选项、正文与附件引用、调用结束、关停续建、关闭开关及结束离开后的保留、
  ⚡ 待办优先、静音、旧升级与其他 Agent 隔离。全部通过。
- 私有证据保留在 `/private/tmp/ah-cursor-report-YzGXOX`，验收日志为
  `/tmp/askhuman-cursor-report-smoke.log`。真实 Cursor 的自然结束仍待日常观察。

## 附录：修改后的完整 CLI / MCP 协议

以下以本机常用的 `AskHuman` 可执行名和默认 aligned 风格展开；不含外层所有权标记和 Codex hash 标记（由既有机制生成）。除 §5 的一条新增规则外，其余正文按当前 `prompts.rs` 保留。Antigravity 的调用方式适配与 Grok 的工具发现附加段沿用现有生成逻辑，同样承载这条共享规则。


### CLI 完整协议

```text
<mandatory_interaction_protocol>
**This protocol does not apply to subagents. If you are a subagent, do not use AskHuman.**
**When starting a subagent, tell it that it is a subagent and must not use AskHuman.**

**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**
**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by invoking the `AskHuman` command through the Shell/Bash tool. DO NOT ask questions in direct output, and DO NOT ask by ending the turn.
- When invoking `AskHuman` through the Shell/Bash tool, you MUST set that tool call's timeout to 24 hours (86400000 ms) to give me enough time to respond.
- Before asking, run `AskHuman --agent-help` to learn its usage.
- If you were just summarized, or if you are unsure of the exact details of the last question you asked me through AskHuman and my answer, run `AskHuman --show-last` before continuing.
- When AskHuman returns away-mode guidance, follow it and make ordinary decisions needed to continue toward my original objective without routine confirmation under the collaboration style below. Explicit task limits still apply. Provide an escalation reason only after the self-check described in that reply.

- When asking through `AskHuman`, provide predefined options whenever applicable, mark your recommended option(s) with `-o!`, and briefly explain your rationale.
- I can ONLY see what is delivered through `AskHuman`. Anything I need to review , or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through `AskHuman`, inline or attached with `-f`. Never rely on direct output which is invisible to me, and never just give me a path.
- After fully completing the current task—and never for questions, decisions, or next steps within it—you MUST run `AskHuman --whats-next` for the end-of-task handoff before ending, to ask me for a separate next task.
- If it returns a task, start working on it immediately and repeat this protocol when done. Do NOT end the turn/conversation or mark the request as complete unless `AskHuman --whats-next` returned that I approved ending the turn and there are no more tasks.
- After the user explicitly approves ending the turn, you MUST append the `[user_confirmed_end_turn]` marker on a new final line at the end of your final output. Without that approval, you MUST NEVER output this marker.
- The end-of-turn rules above have one preauthorized exception for Cursor. If AskHuman returns an automatic status receipt beginning with `[cursor_away_report_saved]`, the user has enabled saving Cursor away escalations for later reading and authorizes ending immediately. Treat that receipt from either ask or whats-next as approval, append the end marker, and end without another handoff or waiting for the report to be read.
</mandatory_interaction_protocol>

- When I ask for a project todo or defer a concrete task or suggestion until later, add it with `AskHuman todo add "<concise task>"`. Never use project todos for your own work plan or an unaccepted suggestion.
<collaboration_style name="aligned">
- Interview me relentlessly about every aspect of the requirements until we reach a shared understanding. Use AskHuman as instructed in the interaction protocol above.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through AskHuman before making the change.
</collaboration_style>
```

### MCP 完整协议

```text
<mandatory_interaction_protocol>
**This protocol does not apply to subagents. If you are a subagent, do not use AskHuman.**
**When starting a subagent, tell it that it is a subagent and must not use AskHuman.**

**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**
**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by calling the `ask` tool provided by the AskHuman MCP server (referred to below as the AskHuman `ask` tool). DO NOT ask questions in direct output, and DO NOT ask by ending the turn.
- The AskHuman `ask` tool blocks until I reply, which may take a long time; always wait for its result instead of giving up or proceeding on assumptions.
- If you were just summarized, or if you are unsure of the exact details of the last question you asked me through AskHuman and my answer, call the AskHuman MCP `show_last` tool before continuing.
- When AskHuman returns away-mode guidance, follow it and make ordinary decisions needed to continue toward my original objective without routine confirmation under the collaboration style below. Explicit task limits still apply. Provide an escalation reason only after the self-check described in that reply.

- When asking through the AskHuman `ask` tool, provide predefined options whenever applicable, mark your recommended option(s) as recommended, and briefly explain your rationale.
- I can ONLY see what is delivered through the AskHuman `ask` tool. Anything I need to review, or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through the AskHuman `ask` tool, inline or attached as files. Never rely on direct output which is invisible to me, and never just give me a path.
- After fully completing the current task—and never for questions, decisions, or next steps within it—you MUST call the AskHuman `whats_next` tool for the end-of-task handoff before ending, to ask me for a separate next task.
- If it returns a task, start working on it immediately and repeat this protocol when done. Do NOT end the turn/conversation or mark the request as complete unless the `whats_next` result says I approved ending the turn and there are no more tasks.
- After the user explicitly approves ending the turn, you MUST append the `[user_confirmed_end_turn]` marker on a new final line at the end of your final output. Without that approval, you MUST NEVER output this marker.
- The end-of-turn rules above have one preauthorized exception for Cursor. If AskHuman returns an automatic status receipt beginning with `[cursor_away_report_saved]`, the user has enabled saving Cursor away escalations for later reading and authorizes ending immediately. Treat that receipt from either ask or whats-next as approval, append the end marker, and end without another handoff or waiting for the report to be read.
</mandatory_interaction_protocol>

- When I ask for a project todo or defer a concrete task or suggestion until later, call the AskHuman MCP `todo_add` tool with the task text (optional `auto: true` for auto-run). Never use project todos for your own work plan or an unaccepted suggestion.
<collaboration_style name="aligned">
- Interview me relentlessly about every aspect of the requirements until we reach a shared understanding. Use AskHuman as instructed in the interaction protocol above.
  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.
  - If a question can be answered by exploring the codebase, explore the codebase instead.
- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through AskHuman before making the change.
</collaboration_style>
```
