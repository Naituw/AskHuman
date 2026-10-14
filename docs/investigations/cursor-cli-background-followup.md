# 终端 Cursor CLI 后台完成续跑调查

2026-10-15。Desktop 功能已完成并提交为 `a00a0cd`。用户随后要求制定 CLI 适配方案，并明确
新授权最少量真实 CLI 测试，模型仍只限 Grok 4.7；这取代此前仅允许静态分析的 CLI 限制。
本轮没有实施 CLI 产品功能、修改厂商文件或升级 CLI。

## 1. 本机版本与公开接口

当前入口 `~/.local/bin/cursor-agent` 指向版本 `2026.10.01-e373342`；此前调查的
`2026.07.23-e383d2b` 只作为历史证据，不能代替当前版本核对。

实际 `--help` 和 `--list-models` 确认支持显式 `--model`，可用 `grok-4.7-low`。
测试以该参数启动，原生 AgentClient 的模型字段实际为 `grok-4.7`。

[官方配置说明](https://cursor.com/docs/cli/reference/configuration)列出模型、终端通知和后续提示
等配置，未列出禁止后台完成自动启动模型的设置；支持 `CURSOR_CONFIG_DIR` 自定义配置目录。
[Hooks 说明](https://cursor.com/docs/hooks)区分提交前钩子和结束后 follow-up。本机代码进一步
确认，不能把这两个钩子当成后台完成入口的通用拦截器。

## 2. 当前源码处理链

证据来自已安装版本目录内的原生 bundle；未依赖压缩类名或固定源码行号。

- `9969.index.js` 的交互式完成调度先检查正在运行、排队用户消息和 `user_aborted`。
  `user_aborted` 时 pull 后以 `requeue:false` nack；自然完成没有这个退出条件。
- 可分发完成批次先排除当前 Await 的任务 ID，然后构造 `backgroundTaskCompletionAction`，
  调用下一轮模型；成功后 ACK，失败可 requeue。
- 自动入队回调和 post-stream 路径都会尝试分发。因此只替换一次通知回调或只在 Stop 后设置标记
  不能可靠实现全局策略。
- `beforeSubmitPrompt` 位于主动用户提交函数中；后台完成通过另一条调度路径直接进入模型。
  `stop` Hook 的 `followup_message` 可添加轮次，缺少该字段不等于取消后台调度。
- `2320.index.js` 的稳定模块 ID `./src/shared/cli-background-work-registry.ts` 导出 CLI 专用
  registry。它在原生队列上增加入队回调、队列监听和子代理在途记录；pull/ack/nack 均保留
  原生 wakeup 台账。任务运行和结果文件独立于完成通知。
- `--print`/headless 的循环在同一 registry 上等待后台工作并分发通知，但具有独立的 JSON/
  stream-json 输出与退出语义。当前隐藏 `--single-turn` 仅限 print/headless，说明明确为
  “完成初始轮次和委派子代理，跳过后台 Shell 等待和其它后续轮次”；不是交互式全局开关，
  也不是全部子代理完成通知的抑制方案。
- ACP 还有独立完成分发代码（`3351.index.js`），不能把共享队列当作其已验收的依据。

## 3. 不改厂商文件的普通运行进程接入可行性

本轮创建一个正常启动、没有调试启动参数的隔离交互式 CLI。验证 PID、启动 argv、当前用户、
版本路径和端口所有者后，通过 Node SIGUSR1 临时附加本机 Inspector。

从已知 stdin 输入监听器出发，有界遍历函数的 Inspector scope 和输入流监听链，获得实际
`__webpack_require__`，再按上述稳定模块 ID 定位已经加载的 registry。没有全堆扫描，也没有
读取环境凭据或其它用户会话内容；不需要对厂商文件作字节替换。

临时 Inspector 在每次验证后关闭。产品接入应只 bootstrap 一次有限的私有控制桥，稳态通过
带认证的 status/restore 管理，避免反复开端口。Node 附加会向 CLI 的 stderr 输出调试提示；
这是与 Desktop 的可见差异，方案需要明确这一取舍，不能宣称完全没有终端显示影响。

该接入证明只来自本机 macOS 的当前交互式 CLI。headless、Windows/Linux、persist/ACP/worker
等入口及不同打包结构仍需要单独核对；尚不据此发布支持承诺。

## 4. 真实 Grok 4.7 与原型证据

隔离目录 `/tmp/ah-cli-plan-rwi1incd`，独立 `CURSOR_CONFIG_DIR` / `CURSOR_DATA_DIR`，空工作区。
CLI 以 `--model grok-4.7-low --disable-auto-update` 启动；模型入口增加测试用核验，非 Grok 4.7
不允许进入实际调用。共记录 4 次顶层 AgentClient 调用：3 条主动消息、1 条自动后台完成请求。

聊天 `53484306-ec85-4a57-b832-42ddd103e78a`：

| 调用 | Request ID | 原生 action | 实际模型 |
|---|---|---|---|
| 基线主动消息 | `08b31d64-2582-46bd-8d4b-9e3e1a433748` | userMessageAction | grok-4.7 |
| 原生自动续跑 | `d1f88562-6479-430b-b2ae-21dee48f08da` | backgroundTaskCompletionAction | grok-4.7 |
| 原型主动消息 | `250fd3a0-7449-4dce-a3a9-e7e79e548921` | userMessageAction | grok-4.7 |
| 手动续聊与 Await | `e8bbdb5b-4b4f-4642-9893-a9098cd4f36e` | userMessageAction | grok-4.7 |

### 基线

首个模型请求于 00:17:29 停止生成，返回 `AH_CLI_IDLE`；Shell `476843` 仍被外部释放文件阻挡。
00:17:59 释放后，Shell 成功并入队，随即出现第二条真实模型请求，返回 `AH_CLI_AUTO_ONLY`。
原始输出文件保留 `AH_CLI_BG_DONE`，退出码 0。

本机 CLI 即使使用独立配置目录，仍加载用户级 Hooks 和 MCP。两条 Stop Hook 此时仍等待
AskHuman 回答；后台调度并不等待它们，模型已可继续。这说明“停止生成”和“完成整个结束
确认链”不能混为一谈，也证明 Stop Hook 不能代替全局后台完成控制。

为了结束基线并隔离剩余原型测试，本轮取消了**仅属于该测试 CLI 子进程**的两个 AskHuman
Hook 客户端，并只在该进程内临时停用 Stop Hook，未伪造人类结束回答，也未修改全局 Hook 文件。
基线的在途通知随后正常 ACK；不能把这次取消当作真实 AskHuman 结束验收。

### 队列原型

临时类原型包装保持原生 pull，开启策略时对拉出的完成批次调用原生 ACK，返回空通知批次，
从而不进入自动模型分发。新服务实例共用该类原型；尚未将这段验证代码作为最终产品实现。

- Shell `476844`：先调用显式 Await 等待，超时后正常返回 running；外部随后释放，实际成功。
  完成通知在当前模型轮次结束后被 ACK，没有产生自动请求。
- Shell `476845`：在模型请求 00:26:50 结束后才释放；00:27:00 原生完成通知被 pull/ACK，
  不产生第四条自动请求。约 75 秒后，第四条调用来自明确的手动续聊。
- 两条完成均有真实 success、输出文件与 exit 0；原生工作列表为空，pending / inFlight 为 0。
- 手动消息正常执行 AwaitShell `476844`，返回 complete、exit 0、outputFilePath 与 outputLength。
  该 CLI 的 Await 返回文件元数据而非内联输出；模型按“禁止读文件”的测试指令正确报告无法
  内联核对标记，没有输出预期的 `AH_MANUAL_OK`。主代理读取真实终端文件，确认
  `AH_CLI_AWAIT_DONE` 存在。不能将这种测试预期差异写成 Await 丢结果，也不能伪造通过标记。

这证明当前交互式 CLI 的通知抑制、真实 Shell 结果、已完成任务的 Await 和主动续聊可行。
**没有覆盖** Await 正在阻塞时恰好成功的完整竞争矩阵、真实 CLI 子代理、错误/中断结果、
真实 AskHuman 明确结束、headless 输出/退出、跨进程生命周期和所有平台；这些仍是实施验收门槛。

## 5. 清理与后续设计边界

- 原生方法、测试进程内 Stop Hook 和临时引用已还原；隔离 CLI 与其 MCP 子进程退出。
- Inspector 9229 确认无监听；没有改动或停止已有用户 CLI 会话。
- 原 CLI 配置文件 SHA-256 与测试前一致；厂商安装文件未改，版本未升级。
- 原始证据的关键文件为 `final-native-evidence.json`、`artifact-evidence.json`、
  `visible-assistant-evidence.json`、`baseline-complete.json`、`cleanup.json`；本节保留关键事实，
  不要求 /tmp 文件永久存在。

用户已确认 CLI 沿用“所有后台完成通知都不能自动启动模型”的全局策略，包含未明确结束的
会话。用户已进一步确认首版仅交互式 CLI；print/headless 后续单独处理，已记录为项目待办 #6
（`9e794f6e-bfde-4741-975e-793db183076c`）。最终规格仍需确认平台、接入方式和开关归属，
再把当前原型补齐为可恢复、
有契约检查的产品链路。不能仅凭本原型宣称终端 CLI 已受现有 Desktop 开关保护。
