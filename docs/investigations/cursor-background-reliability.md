# Cursor 后台完成控制：可靠性复审与撤回

2026-10-15。用户要求先检查 Desktop 旧版兼容，再找 CLI 可靠替代；宁可不提供功能，也不接受脆弱运行时补丁。

## 结论

撤回实验性的后台完成续跑拦截。Desktop 方案也依赖私有 JS 服务、调试器和内部返回协议；旧版存在实质差异，单构建验证不能推导稳定升级兼容。CLI 的入口替换/JS 注入实验不合并发布。没有找到同时保留后台任务、当前 Await 和主动续聊，又选择性禁止后台完成模型轮次的公开控制面。

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

Rust 1327 passed / 3 ignored；Vitest 382 passed（55 文件）、Node 7 passed；fmt、Clippy 通过。Dev Instance 安装成功，安装后的 MCP show_last 正常，doctor 不再包含 cursorDesktop/cursorCli。生产整合与安装结果见实施计划。

原始证据位于 /tmp/ah-cursor-desktop-compat，包含包内版本、官方来源/CRC、提取的方法与 final-evidence.json；本文保存结论，不要求临时文件永久存在。
