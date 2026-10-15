# Cursor CLI 后台完成续跑控制实施计划

## 已确认范围

见 [规格](../specs/cursor-cli-background-followup.md)；调查和原型见
[实测记录](../investigations/cursor-cli-background-followup.md)。2026-10-15 用户授权独立编写计划、仔细审核、实现与验证，无需等待常规设计问题答复。

## 实施顺序

1. 在独立 worktree / popup-only Dev Instance 开发，避免另一任务的主工作区及生产 daemon。
2. 实现 startup preload：精确目标识别、registry 契约、原生 pull → ACK → 空通知批次、私有策略热恢复与无内容运行状态。
3. 实现 macOS 命令入口管理：核验、先写恢复记录、原子替换、原 argv exec、更新重接管、停用/None 恢复所有权。
4. 接入 daemon、既有单开关、独立 CLI status、doctor 与前端显示；保持 Desktop API 兼容。
5. 审核生命周期、异常恢复、第三方覆盖、NODE_OPTIONS 后代污染、headless 误拦截、子代理 durable 记录、Await 竞争。
6. 自动测试、fmt/clippy、安装；用安装产物进行最少量 Grok 4.7 CLI 实测与恢复检查。
7. 记录证据和真实人工验收限制，按小批次提交；不提交其他任务改动。

## 设计审核（实现前）

- 仅禁用完成通知消费端的新模型触发，不取消工作，不改通用 AgentClient.run；避免影响 Goal 和主动续聊。
- 当前原型证明原生 pull 之后 ACK 会清理 inflight，但 Await 依赖 CLI 原生 suppress；必须用真实 Await 竞争与子代理补验，不能以 Shell 一项推论全部结果类型。
- 文件策略比任意脚本 IPC 更窄：同用户私有原子策略 + 每进程无内容心跳，无 Inspector/执行端点；daemon 重启仍可恢复。
- Dev Instance 只管理其自己配置的测试入口，不应抢生产用户入口。生产入口管理限规范用户 bin；测试使用镜像 HOME，不能写全局模型/权限配置。
- 更新仅接受核验后的新符号链接，不恢复用户删除的入口，不覆盖第三方文件。恢复记录保留至实际清理完成。
- 原始参数使用 exec argv，无用户输入 shell 插值。原始 NODE_OPTIONS 保留，自己的预加载与 bootstrap 必须从后代环境剥离。
- 停用恢复有界失败必须保留诊断和记录；不能仅删状态谎报完成。原生 ABI 不匹配必须报告未保护。

## 验证记录

2026-10-15 已完成实现、自审、安装和自动验证，分支 `codex/cursor-cli-followup`。

| 检查 | 结果 |
|---|---|
| Rust 全量 | 1324 passed / 4 ignored |
| 前端 | 55 文件、385 Vitest passed；25 Node passed |
| fmt / Clippy / production frontend build | 通过；提交钩子亦通过 |
| 安装 | `./scripts/install.sh`，签名并安装至独立 Dev Instance |
| 最终安装 startup smoke | CLI `2026.10.01-e373342`、Node v26.8.1、adapter `1630f242f58d8892`；启动已保护，无 Inspector/调试横幅 |
| 真实模型 | 本轮 3 次父会话 + 1 次子代理调用，均由调用前校验限制为 Grok 4.7；另一次观察器字段错误在进入原生调用前拦截，没有计为成功请求 |
| 自动完成续跑 | 真实后台 Shell / 子代理在父会话结束后完成，原生 ACK；观察到 0 次 `backgroundTaskCompletionAction` 模型调用 |
| Await 竞争 | Shell 764846：工具开始后释放 gate，原生 complete / regexMatch / exitCode 0；子代理 Shell 244283 的阻塞 Await 亦正常取得结果 |
| Ctrl+C | Shell 764848 在停止后真实成功、ACK，无自动模型；随后手动回复 `AH_MANUAL_OK` |
| 原生队列无模型矩阵 | Shell / 子代理 × success/error/aborted；含重复入队的 12 条通知均原生 ACK，pending / inflight 清空 |
| 生命周期 | daemon 重启保留同一 CLI PID 与保护；关闭 / 重开恢复 pull 和 startup loader 的精确函数引用，再接入成功 |
| 入口所有权 | 已验证入口替换后重接管并保存相对原目标；第三方脚本不覆盖；用户删除不重建；关闭恢复原符号链接 |
| 非目标模式 | 受管入口 `--print --help` 原样返回 0，不进入交互模型；完整 headless 模型链按已确认范围延期 |
| 清理 | 所有本轮 CLI 测试 PID 退出、入口恢复并移除测试镜像，Inspector 9229 无监听；原 Cursor 主进程 92826 保留，生产偏好仍关闭 |

真实会话 `a7a2441f-b01f-470b-a5c0-2d65fe690007`，子代理
`b6c1cfd8-ea65-4386-9725-55f38c78bee7`。原生输出文件 764846 / 764847 /
244283 / 764848 全部包含预期标记且 exit_code 为 0；子代理持久化 transcript 的可见
assistant 文本为 `AH_MANAGED_CHILD_OK`。测试观察器只保存请求元数据与工具结果，不读取 thinking。

新版 CLI 原生 Await 会拒绝等待子代理并要求结束当前轮次；不能把该原生限制报告成拦截错误，也不能宣称父会话的子代理 Await 验收成功。子代理真实结果和通知 ACK 已分别验证。

证据目录为本机 `/var/folders/sm/h90d_zys04110r1_b2c3b9n80000gn/T/ah-cli-managed-jcd0w8v6`；
关键摘要文件：`final-requests.json`、`after-stop.json`、`native-matrix-final.json`、
`installed-final-smoke.json`、`installed-final-restoration.json`、`durable-output-evidence.json`、
`replacement-adopted.json`、`foreign-entry-preserved.json`、`deleted-entry-preserved.json`、`cleanup-final.json`。
本文保留关键事实，不要求临时目录永久存在。

## 最终自审与交付边界

- 修复独立状态请求、部分覆盖误报、停用后错误重试、可选 resume/worktree 参数识别。
- 停用恢复自身 registry / chunk factory / Node loader；第三方后续包装保留。
- 策略瞬时读取失败保留上一次明确策略并报告错误，避免意外恢复自动续跑；缺失模块/未知契约报告未保护。
- 厂商 launcher 改变时恢复自有入口并原样启动，不能因适配失败使原命令不可用。
- 当前主工作区另有功能仍在修改并已安装自己的二进制，因此本分支只安装到 Dev Instance，未覆盖生产安装或混入其未提交代码。待该任务收口后合并并安装生产版本。
- 用户已睡觉，未冒充人回答 CLI 的真实 AskHuman End；人工 End 与新 CLI 状态行的观感仍待验收。测试中的 Stop hooks 仅在拥有的测试进程内临时过滤，未改全局 hook 文件，不能把它当作真人 End 证据。
- Linux/Windows、print/headless 是已确认的后续 todo #7 / #6。


## 升级与恢复复审（2026-10-15）

用户要求继续检查兼容/恢复风险，并解释原命令如何进入包装。本轮为只读代码审核和隔离的无模型 ABI 探针，未修改功能代码。

发现生产合并前需处理的边界：

1. **入口依赖 AskHuman 二进制**：包装脚本直接 exec 绝对路径的 AskHuman。直接删除该二进制、或降级到不支持 `--cursor-cli-run` 的版本时，包装没有独立原命令回退，Cursor 命令可能不可用。正常关闭/None 的恢复验证不能覆盖该路径。
2. **ABI 检查不足以约束升级语义**：目前只检查模块 ID、导出与 pull/ACK 的源码特征，没有已验证版本/构建约束。隔离探针把 ACK 换为异步实现，同时保留 `forgetInFlight` 特征：当前适配器仍报告 active，并在 ACK 完成前返回空通知批次。该探针不代表当前 Cursor 的 ACK 已异步化，证明的是未来兼容检查缺口。
3. **入口归属检查与 rename 存在并发窗口**：安装/恢复均先检查文件，再原子 rename。Cursor 安装器或其他程序若恰在两者之间替换入口，仍可能覆盖其修改。已通过的第三方覆盖/删除测试是顺序事件，不能宣称消除了该竞态。

这些边界尚未修补。保持当前分支与生产偏好关闭，待用户确定后补强并复验，再合并生产。
