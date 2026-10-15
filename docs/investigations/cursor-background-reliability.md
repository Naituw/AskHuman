# Cursor 后台完成控制：可靠性复审与撤回

2026-10-15。用户要求先检查 Desktop 旧版兼容，再找 CLI 可靠替代；宁可不提供功能，也不接受脆弱运行时补丁。

后续按用户选择完成了[Desktop 恢复前实机验证](cursor-desktop-restoration-validation.md)：
3.14 / 3.18 新增真实 Grok 4.7 证据，三个构建的六个窗口通过原生队列及生命周期检查。
以下旧版静态调查与比较记录的是当时证据；最新覆盖范围以该报告为准。2026-10-16 用户随后授权
恢复默认关闭的 Desktop 实验功能并加入原生预检，实施状态见[恢复计划](../plans/cursor-desktop-restoration.md)；CLI 继续延期。

## 结论

撤回实验性的后台完成续跑拦截。Desktop 方案也依赖私有 JS 服务、调试器和内部返回协议；旧版存在实质差异，单构建验证不能推导稳定升级兼容。CLI 的入口替换/JS 注入实验不合并发布。没有找到同时保留后台任务、当前 Await 和主动续聊，又选择性禁止后台完成模型轮次的公开控制面。

原问题的关键是两段生命周期：前台轮次结束后，仍运行的后台任务完成事件可以再启动模型轮次；AskHuman 的结束答复也没有取消 Cursor 内部任务。此次撤回恢复普通集成，不再提供阻止 Cursor 原生后台完成唤醒的保证。

## Desktop 旧版证据

通过官方 [下载页面](https://cursor.com/download) 的 golden 端点取得发行包，读取同一官方 CDN 的 ZIP。使用 HTTP Range 提取主 Info.plist、package/product 与 workbench JS；验证 ZIP CRC 和包内版本。没有安装或启动旧 Desktop，也没有调用模型。第三方索引不作为兼容性证据。

| 官方构建 | 当前适配器的静态契约 | 主要缺口 |
|---|---|---|
| 2.5.26 | 不匹配 | provider / services / methodsParsed / runWakeup / completionBranch / nativeAck / awaitExclusion / pairedInboxCommit |
| 3.0.16 | 不匹配 | provider / runWakeup / completionBranch / nativeAck / pairedInboxCommit |
| 3.9.16 | 不匹配 | runWakeup / completionBranch / nativeAck / pairedInboxCommit |
| 3.14.27 | 不匹配 | pairedInboxCommit |
| 3.18.25 | 匹配，尚非实机验收 | — |
| 3.20.21 | 匹配，尚非实机验收 | — |
| 3.22.12 | 匹配，尚非实机验收 | — |
| 3.23.23 | 匹配，尚非实机验收 | — |
| 3.24.12（本机） | 此前两条路径真实 Grok 4.7 与生命周期验收通过 | 只能证明当时的单构建行为 |

最旧的静态匹配样本是 3.18.25；这不构成真正的版本下限，也不能宣布 3.18 至今所有版本受支持。3.14.27 已有 completed/ACK，但缺少当前 guard 要求的 paired inbox 提交路径；3.9.16 / 3.0.16 的执行及确认协议有实质差异，2.5.26 缺少接入点。旧版实际启动、调试器和服务发现未验收，不冒充完整兼容。

检查包括 provider、DI 服务、后台处理器调用 _runWakeupAction、completed 分支、原生 ACK、Await 排除、paired inbox 提交。方法源码经过独立语法解析；比较没有执行厂商代码。

## CLI 与公开替代方案

| 路径 | 证据与限制 |
|---|---|
| 官方配置 | [CLI 配置](https://cursor.com/docs/cli/reference/configuration) 的 notifications 控制终端提示，没有对应后台完成轮次策略 |
| 官方 Hook | [Hook 文档](https://cursor.com/docs/hooks) 的 beforeSubmitPrompt 覆盖用户提交，stop 的 followup_message 控制自身追加；本机完成调度直接构造 backgroundTaskCompletionAction，不能据此阻止 |
| ACP | [ACP 协议](https://cursor.com/docs/cli/acp) 提供 prompt/cancel；安装源码的 ACP 等待器内部仍调用 runFollowup(backgroundTaskCompletionAction)，过滤客户端通知不能阻止内部请求 |
| SDK | [官方 SDK](https://cursor.com/docs/sdk/typescript) 明确后台子代理结果成为同一 run 的 follow-up；cancel 中止 stream 和在途工具，不保留所需后台工作语义 |
| --single-turn | 当前 CLI 限 print/headless，仍等待委派子代理，不能作为交互式全局策略 |
| 禁止工具、终止/冻结进程、归档聊天 | 改变后台执行、结果、交互或可见性，不满足既定语义 |
| 提示词 | 模型已被唤起后才起作用，无法保证不增加模型请求 |

这不是证明 Cursor 永远不能提供接口；这是对当前官方支持面和安装实现的结论。不再用更多私有字段或版本补丁绕过缺口。

## 撤回与清理

- 移除设置开关、搜索/i18n、状态/doctor、配置字段、daemon 接入/保活、Inspector/renderer/private bridge 及专用测试。
- 先用旧安装关闭偏好，确认 active=false、windows=[]、无自有 bridge manifest；不强杀 Cursor 或中断其他待答问答。
- 旧 cursorDesktop 配置按未知字段忽略，回归验证其他设置保留。
- CLI 实验未发布，生产入口保持厂商链接；取消实验派生的 headless 和其他平台待办。
- 普通 AskHuman 集成、生命周期、原有 Stop 确认与会话信息功能保留。

## 验证

Rust 1327 passed / 3 ignored；Vitest 382 passed（55 文件）、Node 7 passed；fmt、Clippy 通过。Dev Instance 与主环境均已安装并签名成功；新安装的 MCP show_last 正常，doctor 不再包含 cursorDesktop/cursorCli，生效 config 无退休字段。主环境安装后旧 daemon 正等待另一条问答排空再换新，后台控制已提前关闭。提交与安装记录见 [实施计划](../plans/cursor-background-reliability.md)。

官方来源、版本、ZIP 大小/CRC、workbench SHA-256 与逐项检查结果保存于 [兼容性证据](cursor-desktop-compatibility-evidence.json)。厂商源码及提取的方法仅保留于临时目录 /tmp/ah-cursor-desktop-compat，不要求临时文件永久存在。

## 追加审查：旧版内部接口是否消失

2026-10-15，按用户追问对比旧包与当前 3.24.12。这里的接口均指 workbench 内部方法，不是公开 SDK API。以完整标识符匹配并解析定义，避免把“不再由此路径调用”误判成“方法已删除”；同时检查当前 Agents 与 IDE 两个 bundle。

| 旧接口 | 旧版证据 | 当前 3.24.12 |
|---|---|---|
| `_mirrorDirectSubagentCompletionToParentHistory` | 3.0.16、3.9.16 有定义；把子代理完成信息写成父会话的模拟用户消息 | 同名方法/引用均消失；3.14.27 样本已不存在 |
| `_appendCompletionToParentHistory` | 3.0.16、3.9.16 有定义，子代理完成入队器会调用 | 同名方法/引用均消失；新入队器改用结构化 wakeup 队列，没有原来的 history append 分支 |
| `_backgroundRunKey`、`_markUnprocessedBackgroundRun`、`_hasUnprocessedBackgroundRun`、`_clearUnprocessedBackgroundRun` | 3.9.16 用 run key 集合与计数器跟踪/抑制后台完成 | 同名方法/引用均消失；3.14.27 已改为运行记录的 `disposition="suppress"` 等状态 |
| `_markParentTaskBubbleCompleted` | 3.9.16、3.14.27 有定义 | 旧名消失；3.18.25 起抽查到 `markParentTaskBubbleCompleted`，成功/取消/错误状态写入仍在，父 bubble 查找也重构了 |

同时，有几项不能当作删除证据：

- `_runBackgroundTaskCompletionAction` 与 `_maybeDispatchBackgroundCompletions` 从 3.0.16（4 月 9 日构建）到 3.24.12（10 月 9 日构建）都保留，跨度约六个月。
- `drainCompletions`、`enqueueCompletion`、`unmarkAwaitedCompletion` 在当前版仍有真实方法定义。虽然新版分发器主要用 pull/ack/nack，这些旧方法仍以兼容包装存在。
- 名称保留不等于调用协议不变：完成入口由 3 参数变成 4 参数，旧版直接 runAgentLoop；较新版委托 _runWakeupAction，分发器按返回的 completed 决定 ACK 或重排。

因此，历史上既有删除/重构，也有持续保留的核心入口。不能把部分辅助方法的消失泛化成所有接入点随时改名。此前“约 1.3 个月”仅指完整旧适配器契约在已抽查样本中的静态匹配跨度（8 月 31 日至 10 月 9 日），不是核心方法的存续时间，也不是未来稳定期预测。

逐构建日期、方法定义/引用状态、源码与方法哈希、参数及调用观察保存于 [内部接口演进证据](cursor-private-api-evolution.json)。本次只做静态研究，没有执行厂商代码或调用模型。

## Desktop 与 CLI 的持久性比较

2026-10-15 用户要求从历史接口演进角度重新评估。核心机制的存续证据比初步判断更积极；没有证据说明两条方案必然几周失效。仍需区分核心算法、实际接入链和当前实现缺口。

| 维度 | Desktop | 交互式 CLI |
|---|---|---|
| 核心接入点历史 | 两个核心分发/执行方法名在 4 月至 10 月的样本中保留约六个月 | registry 模块 ID 与导出 t 从 7 月 20 日至 10 月 14 日的四个样本中保留，约 86 天 |
| 实际语义证据 | completed → 原生 ACK 的架构从 3.14.27 至 3.24.12 样本可见，约 66 天；完整旧 guard 的静态匹配从 3.18.25 起约 39 天 | 7 月两个构建与 10 月 1 日构建的原生队列探针均通过；pull → 原生 ACK → 空通知批次这一算法跨越约 73 天，旧版仍需继承方法适配 |
| 关键变化 | 同名入口的参数、返回协议变化；parent history 辅助方法删除、父任务方法改名 | pull/ACK 从继承变为派生类自有；10 月 14 日又把子代理专用 inflight 统计改成通用按 kind 统计，但 pull/ACK/forgetInFlight 仍在 |
| 接入链维护 | 主进程附加、Electron 调试能力、provider 闭包、私有 DI 服务、多窗口重载与重新注入 | 原始命令包装、bundled Node preload、CJS chunk loader、私有模块/导出、队列原型；不需要窗口调试器 |
| 实际验收范围 | 3.24.12 的 Agents / IDE 两条路径及生命周期 | 10 月 1 日构建真实模型/生命周期；7 月原生队列探针；10 月 14 日仅静态候选，不是运行验收 |

补查当前 [官方安装脚本](https://cursor.com/install) 指向的 `2026.10.14-941492e` 发行包：launcher、模块 ID/导出、当前适配器所检查的 pull/super.pull 与 ACK/forgetInFlight 特征仍在。包内 `inFlightSubagentIds` 与 `hasInFlightSubagentCompletions` 已消失，改为 `inFlight` Map 与 `hasInFlightCompletionsOfKind(kind)`；它们不是旧 preload 的直接挂接依赖。只下载并静态解析，没有执行安装脚本、更新本机 CLI 或运行该发行包。来源和哈希见 [CLI 持久性证据](cursor-cli-durability-evidence.json)。

按历史证据，两者都值得作为需要持续维护的内部适配技术评估。按接入复杂度推断，CLI 的集中队列方案更便于维护；这不是对其未来可用月数的保证。Desktop 核心名称的观察窗口更长，但窗口发现/调试接入仍是另一层依赖，不能只看方法名。

旧适配器拒绝某版，也不一定意味着原理不可用：Desktop 的 paired inbox 检查、CLI 的自有 pull/新版 inflight 检查部分来自我们选择的严格契约。放宽或新增适配器需真实验证，不能直接去掉检查宣布支持。

当前 CLI 实现还存在三个具体未修缺口：AskHuman 被删除/降级时受管命令缺少独立回退、ACK 异步语义变化仍可能误报 active、入口所有权检查与 rename 间有竞态。这些是交付阻断，不能由核心 API 较长寿命抵消。Desktop 的真实验收更完整，也仍不构成数月免维护承诺。

最终评估：核心机制有跨月可维护依据；在持续跟进升级的前提下，CLI 更适合优先投入。安装后长期不跟进版本、却承诺数月稳定，两者都缺少证据。当前撤回状态保持；本节记录技术评估，不代表功能重新发布。
