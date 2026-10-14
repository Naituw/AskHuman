# Cursor 后台完成自动续跑调查

调查日期：2026-10-13。行为需求见
[`../specs/cursor-desktop-background-followup.md`](../specs/cursor-desktop-background-followup.md)。
本文区分本机证据、静态分析与尚未验收事项；不是产品已实现能力说明。

## 1. Desktop 问题原因

Cursor 的 Stop 停止当前模型轮次，后台 Shell / 子代理可继续运行。完成通知由独立队列调度，
会启动新的 `backgroundTaskCompletionAction`。AskHuman 返回结束答案或模型输出结束标记，
也不会改变 Cursor 原生后台调度策略。

本机原问题记录中，同一 Shell 任务 `177438` 只执行一次：00:23:52 主轮次结束，
00:24:08 命令完成；第一次完成续跑被用户在 00:24:16 Stop，通知随后重新排队。
00:25:13 创建空白聊天改变加载列表后，旧通知再次被调度。新聊天创建与第二次调度相差约 17ms。
因此用户看到的“过一会又跑起来”还包含取消后的重新排队，不只是后台命令仍在运行。

本机 Cursor 3.24.12 的 Agents bundle：
`/Applications/Cursor.app/Contents/Resources/app/out/vs/workbench/workbench.glass.main.js`。
关键契约包括 `_runBackgroundTaskCompletionAction`、`_runWakeupAction`、后台工作服务的
`pull/ack/nack`，以及取消后带 `requeue` 的分支。源码位置和压缩类名不能作为稳定接口。

官方支持也解释了 Stop 与后台完成唤醒的关系，以及终端等待超时后转入后台的行为：

- [Stop 后会话自动恢复](https://forum.cursor.com/t/agent-session-resumes-automatically-if-stopped/172081)
- [前台 Shell 完成后触发额外请求](https://forum.cursor.com/t/foreground-shell-commands-trigger-an-automatic-extra-agent-request-after-the-main-turn-finishes/172103)

## 2. 两种方案的复杂度

逐会话方案要把 Stop、AskHuman 结束与原生会话精确关联，并持续维护“已结束/允许恢复”。
还要区分主动输入、Hook continuation 和模拟消息，解决持久化、重启及竞态；不能只识别一个
结束标记或 `beforeSubmitPrompt`。

全局方案直接阻止后台完成动作进入新一轮模型，保留原生结果与确认流程，不维护上述会话状态。
它仍需承担进程/窗口发现、正常启动接入、重载、版本兼容和恢复成本。

将聊天归档会原生静音，但用户不接受归档/隐藏；修改提示词不能硬性阻止原生调度。
普通扩展、现有公开 Hook 与应用控制入口未找到等价的全局控制能力。

## 3. 隔离接入与队列验证

已验证：无调试启动参数的 macOS Cursor，可在启动后通过 SIGUSR1 临时开启主进程 Inspector；
实际 LaunchServices 普通启动也能接入。Cursor 当前 Electron fuse 允许此方式。
建立认证的用户私有控制桥后，可以关闭 Inspector 而继续管理原生完成队列。

隔离队列/控制器测试涵盖：静音前后入队、Shell/子代理的成功/失败/中断、在途 nack、
清理后不能重排、解除后新通知可用、原生取消，以及手动提交继续可达。
控制器部分使用虚拟运行时，不等同于真实模型验收。

普通窗口首次加载后的服务并非立即就绪。一次性 `did-finish-load` 回调不足以可靠接入；
等待服务并有界重试后，两次重载恢复约为 2.699s / 2.790s。重启会丢失窗口内存状态。
9229 被占用时 Cursor 仍能运行，但附加不能成功，必须检查端点所有者并报告失败。
用户已要求按实用优先处理正常启动，不为这段接入过程额外接管启动。

技术参考：[Electron fuses](https://www.electronjs.org/docs/latest/tutorial/fuses)、
[Electron Debugger API](https://www.electronjs.org/docs/latest/api/debugger)、
[Node Inspector close](https://nodejs.org/api/inspector.html#inspectorclose)。

## 4. 真实 Grok 4.7 验收

测试环境：本机 Cursor 3.24.12，独立空临时工作区；模型 Grok 4.7，Low、Fast 关闭。
未修改 AskHuman 产品、Cursor 安装包或全局模型偏好。原型只作用于临时工作区，避免影响其它任务；
其规则对范围内的完成动作统一处理，没有逐会话结束判断。

| 时间（北京时间） | 证据 |
|---|---|
| 08:48:52 | 真实用户测试消息开始，日志确认 `runtime=legacy` |
| 08:49:11 | Shell 任务 `172244` 开始：sleep 后输出固定标记 |
| 08:49:28 | 模型主轮次结束，后台命令仍运行 |
| 08:49:44 | Shell 成功、退出码 0；捕获一次真实完成动作，跳过进入模型 |
| 08:49:44—08:51:12 | 约 88 秒内无自动模型请求 |
| 08:51:12—08:51:32 | 极短手动消息正常返回 `AH_MANUAL_OK`，日志确认仍为 Grok 4.7 |

原生已确认 Shell 记录包含 `172244`，pending 和 inFlight 均为 0，自动唤醒入口记录为 0。
两条有效消息的 request ID 分别为 `101801a7-a8a2-4a80-bc2b-4ddb2389be49` 和
`aad85a63-d68c-443e-8d56-bcc9a37dddd3`。任务结果/输出与原生确认没有伪造。

还有一条普通 IDE / Agent Host 前置消息没有执行 Shell，只返回文本，已判为无效样本。
不能据此声称 Agent Host 的后台控制已经通过。

三条消息的客户端用量字段累计 inputTokens 52,706、outputTokens 251、cacheReadTokens 18,944。
缓存字段的计费关系未核算，不据此推算金额。两处临时包装已还原，临时 IDE 窗口与 Inspector
已关闭；原 Cursor 主进程继续运行，测试聊天未归档或删除。

临时原始证据曾保存在 `/tmp/ah-global-hhzm0c6l/real-test-evidence.json` 和
`real-test-log-evidence.json`。本节保留关键事实，避免后续设计依赖 /tmp 文件仍然存在。
以上是原型阶段的历史边界。后续实现已补齐真实子代理/Await、普通 IDE Agent Host 和产品
安装链路，完整证据见 [`../plans/cursor-desktop-background-followup.md`](../plans/cursor-desktop-background-followup.md)。

## 5. Cursor CLI 早期静态分析（历史边界）

本节写于用户要求 CLI 不实测的 Desktop 阶段；2026-10-15 后续授权和新版实测见 [CLI 调查](cursor-cli-background-followup.md)。当时本节只读取本机已安装版本 **2026.07.23-e383d2b** 的文件，
没有启动 CLI 会话、发送模型请求或升级版本；结论不自动覆盖较新的 CLI。

安装入口是 `~/.local/bin/cursor-agent`，指向
`~/.local/share/cursor-agent/versions/2026.07.23-e383d2b/cursor-agent`。
主要代码证据在该目录的 `4329.index.js` 和 `index.js`：

- 交互式代码通过 `setOnCompletionEnqueued` 触发空闲完成处理，构造
  `backgroundTaskCompletionAction` 并进入模型；成功 ack，失败可 nack/requeue。
- 用户中断会设置 `user_aborted`；完成处理先检查这个状态，将通知以 `requeue:false` 丢弃。
  因此交互式 CLI 对显式中断有专门保护，不能直接断言 Desktop 的 Stop 缺陷同样存在。
- 自然完成没有上述用户中断标记；代码存在完成后继续处理后台事件的路径。
  因而 AskHuman 选择结束、随后模型自然停止时，存在同类自动续跑可能；尚未实测复现。
- headless 路径在首轮之后等待后台工作并发送完成动作。隐藏参数 `--single-turn` 默认关闭，
  开启后跳过后台等待与后续轮次；参数校验明确限定为 `--print/headless`，不能用于普通交互模式。
- CLI 已完全退出后的进程不再承担这些调度；不能将“结束一轮”与“退出 CLI 进程”混为一谈。

当前[官方 CLI 配置说明](https://cursor.com/docs/cli/reference/configuration)没有列出全局禁用
后台完成续跑的字段；`notifications` 控制终端通知，`suggestNextPrompt` 控制后续提示建议，
都不能据名称认定为关闭自动模型调用。隐藏参数来自本机代码，不是公开稳定配置承诺。
[CLI 使用说明](https://cursor.com/docs/cli/using)区分交互使用、恢复会话与 headless 模式。

终端 CLI 后续已独立调查版本与接入契约，见上述 CLI 调查与 `../specs/cursor-cli-background-followup.md`。
