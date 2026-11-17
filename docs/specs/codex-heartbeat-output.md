# Codex heartbeat 输出适配

> 2026-11-17；用户批准精简方案并指定主工作树实施，已完成程序实现与安装验证。
> 已完成真实模型的两种结果验证与 iOS 通知卡真人作答；验证范围和测试脚本事故见 §7。
> 关联：[结束确认](agent-stop-confirmation.md)、[Codex 桌面接入](codex-desktop-session-integration.md)、
> [生命周期结果](agent-lifecycle-results-and-recovery.md)、[手机结果通知](ios-follow-narrowing.md)。

## 1. 问题与依据

用户在 AskHuman 中看到 Codex 的原始结果：

```xml
<heartbeat>
<automation_id>gerrit-186127</automation_id>
<decision>DONT_NOTIFY</decision>
<message>重跑已进入 Unit 测试阶段，目前无需你处理，继续静默跟踪。</message>
</heartbeat>
```

这是客户端要求模型生成的自动检查结果协议。`DONT_NOTIFY` 表示本轮无需提醒，
不表示业务任务完成，也不负责启停调度。改动前 AskHuman 把它当普通助手正文，
Stop 可以创建结束确认，Watch／手机详情可以直接显示 XML。

已核对本机 ChatGPT.app `26.1111.21308 (27194)`、内置 Codex `0.169.0-alpha.11.1`：

- 程序包 `webview/assets/async/907538.0572f09d40.js` 含这套提示模板与展示解析。
  模板要求每个 heartbeat 以非空结果结束；DONT_NOTIFY 时只有 XML，NOTIFY 时可以有前置正文。
- `.vite/build/early-bootstrap-226-2f6bd181558008cf.js` 的结果解析直接消费助手输出，
  不以核验 `automationKind`、`turnTrigger` 或自动任务 ID 为前提。
- 改动前 `agents/stop.rs` 只清理本项目的用户确认结束标记；
  `codex_desktop/mod.rs` 与 Codex 转录读取路径没有解释 heartbeat。
- 手机完成通知已有资格、回合关联与去重逻辑；没有证据证明用户这次还触发了独立的完成推送。
  本方案仅让已有通知链路消费同一结果决策。

原始发生记录不在本机 AskHuman 历史中，尚未区分当时具体是结束卡还是 Watch。
上述输出处理缺口由代码确认，不把推测写成复现结论。

## 2. 行为约定

| 输入／场景 | 结束确认 | 主动结果通知 | Watch、详情、会话转录展示 |
| --- | --- | --- | --- |
| DONT_NOTIFY 结果 | 不创建卡，正常结束本轮生命周期 | 有原通知资格也不发本轮完成提醒 | 显示 message |
| NOTIFY 结果 | 沿用现有开关与投放规则 | 沿用现有资格与时机，不新建订阅 | 有正常正文则显示正文，否则显示 message |
| 普通回复、解释中的 XML 示例 | 原行为 | 原行为 | 原文 |
| 无法解析的内容 | 原行为 | 原行为 | 原文 |
| 真实提问／权限审批 | 正常处理 | 沿用现有规则 | 正常处理 |
| 运行失败／未完成／用户中断 | 沿用现有结果处理 | 沿用现有结果处理 | 保留真实结果 |

静默只影响自动结束交接与完成提醒，不关闭会话、不更改 Stop 偏好，
也不生成 `[user_confirmed_end_turn]`。普通对话的下一轮行为不变。

NOTIFY 是允许现有提醒，不是要求向所有渠道新增通知。这里不把 heartbeat 的重要更新
扩展为新的手机订阅功能，也不把一次检查完成解释成被监控的测试已经完成。

## 3. 解析规则

新增一个 Codex 专用的纯函数，输入完整助手文字，返回可选的
`HeartbeatOutput { decision, display_text }`。所有消费者共用；不改通用 Markdown 渲染器。

1. 只处理 Codex 助手输出，不处理用户消息、工具输出或思考内容。
2. 接受一个完整的 heartbeat 结果块，以及客户端支持的整块 XML 代码围栏；
   message 支持 CDATA。只接受 NOTIFY／DONT_NOTIFY，不搜索散落在正文里的关键词。
3. DONT_NOTIFY 按宿主模板要求独占整段输出；前后还有解释文字时按普通回复处理。
   NOTIFY 可以是整段输出，也可以是正常正文加末尾结果块。嵌在解释中间、引用中的例子不提取。
4. `automation_id` 是协议内容，不查询调度器、不做 ID 对账，不作为新增身份机制。
5. 有正常正文时保留正文；否则取 message。没有可显示文字或结果格式不完整，返回未识别。
6. 先解析再使用各入口已有的截断规则。不增加用于修复不完整输出的等待或重试。

这里有意保留一个简单边界：如果助手整段只输出完整的 heartbeat 样例，文本上就等同协议结果，
按协议处理；不承诺仅凭文本判断它是不是用户要求抄写的样例。为区分这种情况引入跨进程来源核验，
不符合本次已确认的精简范围。

本期不扩展到裸 `DONT_NOTIFY`、`skip_notification`、`SKIP_COMPLETION` 等其他通知指令。
它们有不同的原生上下文约束，不为了这条 XML 输出一并建立通用指令框架。

## 4. 实现落点

### 4.1 Stop 与生命周期结果

- `agents/stop.rs` 在构建卡片前解释 Codex 最后回复。DONT_NOTIFY 跳过卡片，但继续执行
  既有 `report_turn_end`；NOTIFY 用解析后的文字构建卡片。
- `agents/result.rs` 从同一份 Hook 原文取得通知决策；
  `daemon/runtime/agent_control.rs::desktop_observation` 从本轮助手最终结果取得决策。
  不使用可能属于别的回合的全会话最新文字。
- `TurnResult` 增加可选 `notify: Option<bool>`：缺省沿用原行为，false 对应 DONT_NOTIFY，
  true 对应 NOTIFY。使用默认值与缺省不序列化；不改变 `outcome` 的意义。
- 两个结果来源在发出完成事件时就带上决策，不依赖稍后补一次查询。
  复用 registry 现有回合关联；如现有合并逻辑忽略同一结果的可选信息补齐，局部补齐即可。
  同轮已知决策不被缺字段覆盖，不新建缓存、计时器或回合状态机。

### 4.2 可读展示

`codex_desktop/mod.rs`、`agents/activity.rs`、`agents/transcript_full.rs` 的 Codex 助手文字入口
调用同一解析函数，再沿用现有 `text`／`latest_text`／`AssistantText` 数据流。

完整结果显示正文或 message；没有完整结果则保持既有行为。本次不新增流式片段缓冲器。
转录文件与 AskHuman 已落盘的历史记录不批量改写；本次统一的是 Codex 会话转录的展示投影与新生成的卡片。

Mac 控制台、IM Watch、手机详情复用后端文字，因此不需要各前端再实现解析器。

### 4.3 手机完成通知

`ios/gateway/agents.rs::on_agent_event` 继续记录本轮结果；在准备完成通知时消费 `notify`：

- 结果 completed 且 notify=false：不进入完成通知队列，不标记已通知，不消耗一次性通知资格。
- notify=true 或缺省：沿用原通知资格、Mac 空闲门槛与时机。
- failed／incomplete／interrupted：按原有分支处理，不让完成通知的静默位改变真实运行结果。

不自动将原回合的通知资格转给下一个 heartbeat。已经绑定具体回合的资格仍保持原绑定；
对未绑定回合的既有资格，也不建立新的长期自动检查订阅。

通知判断在 Mac 网关完成，手机仍接收现有文本与通知消息，不新增 iOS wire 消息或 UI。
该字段属于已有 AgentEvent 载荷的兼容增量，不提高 IPC 协议号；不改恢复数据库 schema 或 TaskRequest 哈希。

### 4.4 提示协议

在 `prompts.rs` 的 Codex CLI／MCP 文案中加入明确的作用域例外，含义为：

> A host-scheduled Codex heartbeat check does not require an AskHuman completion report,
> a whats-next handoff, or user approval to end that check. Complete the host's heartbeat
> response directly. AskHuman remains available for a genuine question or required approval.
> Do not emit the user-confirmed-end marker without actual user approval.

这是对自动结束交接的例外，不是允许绕过用户设定的权限或停止整个任务。
同步调整 `whats_next` 工具描述与 CLI 帮助中的绝对表述，使其尊重适用的交互协议例外。
其它 Agent 的交互协议维持原约定，不增加 MCP／CLI 运行时拦截。

Codex 的协议哈希由正文自动改变，沿用 SessionStart 的现有更新路径。
已在运行的聊天不会因此被主动改写上下文；新协议在正常新建、恢复或压缩等注入点生效。
Stop／展示／网关的程序修复随安装生效，不以模型先收到新文案为条件。

## 5. 实施顺序与验收

实施顺序：解析函数与局部回归 → Stop／结果传播 → 展示入口与手机网关 → 提示文案 → 安装验收。
这是一次修复内部的顺序，不拆成要求用户逐阶段重新授权的任务。

| 验收 | 必须观察到的结果 |
| --- | --- |
| 用户给的 DONT_NOTIFY 样本 | 没有结束卡；生命周期仍结束；显示中文 message |
| NOTIFY：仅协议／正文加协议 | 正常文案；遵守现有 Stop 开关，不扩张通知资格 |
| 有原通知资格的 DONT_NOTIFY | 无完成通知，也不产生“已通知”的消费结果 |
| 普通正文、用户引用、正文中的示例、未闭合 XML | 原行为，不因命中一个关键词而静默 |
| 已有真实失败／提问／审批路径 | 保持原处理；静默后下一轮普通对话照常 |
| Hook 与原生完成事件 | 两个来源均在结果上携带决策，重复事件沿用已有去重 |
| 旧 TurnResult 反序列化与重新序列化 | 缺省行为和旧 JSON 形态保持；无意外字段或哈希变化 |
| Codex 提示词与工具帮助 | 无强制 heartbeat 交接的矛盾；不改变其它 Agent 的规则 |

不启动真实模型即可先用隔离 Hook 输入、原生状态样本与网关测试验证上述程序行为。
按项目规则，在长测试／安装前运行机器健康检查，再运行 `./scripts/install.sh` 并使用新安装的
AskHuman 验收；独立 Dev Worktree 使用 `dev_ask`／`dev.mjs ask`。
真实模型是否遵守新文案不能由单元测试替代，需要真实回合时另行说明用量并取得授权。

实现完成后更新上述相关规格；仓库级地图与不变量未改变时不改主 overview。
验收结果见下节；程序验证与真实模型行为验证分别记录。

## 6. 实施与验证记录（2026-11-17）

- 已按本文实现共用解析、Stop 静默、三种转录格式与原生状态的可读展示、结果通知决策传播、
  手机完成提醒过滤，以及 Codex 提示文案例外。没有新增来源核验、运行时工具拦截、轮询或状态机。
- 13 项新增 heartbeat 回归通过。相关模块回归共 780 项通过、4 项既有忽略；
  覆盖 agents、codex_desktop、agent_control、手机网关、prompts、MCP 与 CLI 帮助。
- `cargo clippy --all-targets --features custom-protocol -- -D warnings`、格式与差异检查通过；
  vendor/tao 与 block 的既有依赖警告未改。
- `./scripts/install.sh` 已将主工作树安装到日常路径；正式 daemon 已运行安装产物，指纹 `a86faeb8`。
- 用已安装程序与隔离的模拟 IPC 验证真实 Stop 入口：DONT_NOTIFY 提交 0 张卡、上报 completed 与 notify=false；
  NOTIFY 提交 1 张仅含可读文字的卡、上报 completed 与 notify=true。没有调用模型或向真实渠道发测试消息。
  第一版测试替身串行接收连接，验证 NOTIFY 时因并发连接超时失败；改为并发处理替身后两项通过，
  产品代码未因此修改。该次替身失败拉起的私有 daemon 已停止，临时目录已清理。
- 日志：`/tmp/askhuman-heartbeat-tests.log`、`/tmp/askhuman-heartbeat-regression.log`、
  `/tmp/askhuman-heartbeat-clippy.log`、`/tmp/askhuman-heartbeat-install.log`、
  `/tmp/askhuman-heartbeat-installed-smoke.log`。
- 本阶段未调用模型；随后经用户要求并批准，完成下述真实验证。没有修改调度任务。

## 7. 真实模型与日常渠道验证（2026-11-17）

用户要求“你实测一下”，并明确批准创建临时 Codex 测试聊天、结束后归档。
使用本机内置 Codex app-server 与当前默认 `gpt-6.1-sol` / `xhigh`，总计 3 次短模型调用。
测试聊天为 `01a1e939-bd7e-7ad1-90e8-2b02db14becc`，标题
“AskHuman heartbeat 实测（2026-11-17）”；验证后已归档、两个测试 app-server 均已退出。

测试通过 `turn/start` 的 `additionalContext.heartbeat_instructions` 传入从本机 App
提取的原始 heartbeat 模板，并使用 `automation_heartbeat_…` 的 turnTrigger。
聊天为持久化、originator 为 `askhuman`，没有采用 `codex exec` 或 ephemeral 路径，
因此不会被委派任务静默规则跳过。真实 SessionStart hook 已注入新 heartbeat 例外，
Stop hook 使用日常安装的 AskHuman 与生产 daemon；没有替换 MCP 工具或伪造人类回复。

| 场景 | 真实证据 |
| --- | --- |
| 无变化，Unit 测试仍在进行 | 模型仅输出 DONT_NOTIFY XML，0 次工具调用；Stop hook 36 ms 内完成；该聊天此时有 0 条 AskHuman 历史记录；生产 registry 为 idle、completed、notify=false；Watch 实际帧仅含中文状态 |
| Unit 完成，首次通知轮 | 模型仅输出 NOTIFY XML，0 次工具调用；创建了 1 张正文已清理的 Stop 卡，但测试脚本等待超时撤回，未取得真人作答，不计为渠道验收通过 |
| Integration 完成，补测通知轮 | 模型仅输出 NOTIFY XML，0 次工具调用；Stop 卡路由记录为 ios；用户确认看到纯正文并点击“结束对话”；真实历史记录保存 ios 作答，随后 Codex completed、registry idle、notify=true，Watch 仍为纯正文 |

第一张通知卡在 19:18 创建；测试脚本错误地给等人阶段设置了 15 分钟上限，
19:33 退出导致 caller 断开、请求取消。用户 20:13 返回“没有看到这张测试卡”时，
卡已撤回。保留该取消记录，不把它解释为产品投递故障。补测取消了脚本的等人超时，
20:15 用户完成 iOS 作答，通知回合随即正常结束。产品代码未因这个测试脚本问题修改。

本次确认了真实模型遵守新提示文案、真实 Stop 静默与通知分支、生产 Watch 输出、
以及 iOS 的真实展示和作答。检查状态由测试宿主提供，没有实际运行 CI；
没有等待原生定时调度器触发，也没有新增或更改任何真实自动任务。
没有逐一验收每个 IM 渠道或独立完成推送/APNs；其资格与通知过滤仍以 §6 的回归为证。

证据保存在 `/tmp/askhuman-heartbeat-live/` 与 `/tmp/askhuman-heartbeat-live-retry/`：
真实请求、事件流、模型输出、Watch 帧、registry 结果和请求历史；原始模型转录保留在已归档聊天中。
