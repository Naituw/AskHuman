# Cursor CLI 后台完成续跑控制

2026-10-15 用户确认并授权实现：macOS、交互式终端、受管命令包装，与 Desktop 共用现有开关。

## 行为

- 全局抑制后台 Shell / 子代理完成通知触发的新模型轮次；后台工作、真实输出、原生 ACK 与完成记录保留。
- 主动提交、当前 Await、Goal 及其他触发源保持原生行为；不维护每聊天的结束状态。
- 唯一偏好仍为 `cursorDesktop.suppressBackgroundCompletionFollowUp`，默认 false。Cursor 集成 None 时恢复入口及运行时。
- 仅接管当前用户安装中核验为 Cursor 的 `~/.local/bin/cursor-agent` / `agent` 符号链接；不调整 PATH，不覆盖 Grok 或自定义脚本。
- 用户继续使用原命令；既有未受管进程需自行重启。直接调用版本目录绕过包装，不受保护。
- print/headless、ACP、子命令原样透传，不宣称受保护。Linux/Windows 与 headless 已记录为后续项目 todo。
- 厂商更新不被禁用；若更新替换入口，仅在新目标再次通过 Cursor 安装核验后重新管理。

## 生命周期与诊断

- Daemon 同步入口和私有策略文件；启动时通过 bundled Node preload 接入原生完成队列，不使用 Inspector。
- 私有目录 0700、文件 0600；原始符号链接目标及包装内容先落盘，再原子替换入口。恢复仅覆盖仍为本实例所拥有的包装。
- Preload 只在精确的 CLI index 路径生效，不修改厂商文件；移除传给后代的自身 NODE_OPTIONS 与 bootstrap 环境。
- 原生 registry 的模块、导出、pull / ACK 契约不符时保持 CLI 可用并报告未保护，不能显示 active。
- 运行中按私有策略文件恢复/重新启用；停用有界等待运行进程确认。Daemon 重启不主动停止用户 CLI。
- Desktop 与 CLI 分别显示状态。入口就绪和实际进程已保护必须区分；状态读取不得启动 daemon。
- 诊断只包含版本、PID、计数、错误，不读取会话、提示词、凭据或模型内容。

## 验收边界

自动回归覆盖完成队列、ACK、Await、错误/取消、重复通知、恢复、包装所有权、启动参数与非目标 Node。
安装后使用真实受管交互式 CLI；所有测试模型仅 Grok 4.7。用户已睡觉，不能冒充人回答真实 AskHuman 结束验收；该人工 gate 如未完成必须单独列出。


## 当前交付状态

实现与自动/原生验证已完成并安装到独立 Dev Instance；完整记录见
[实施计划](../plans/cursor-cli-background-followup.md)。主工作区并行任务仍在使用自己的安装，
生产合并/安装与本人实际 AskHuman End、新状态行观感验收保留为明确的下一步。
