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

待实现后填写。真实 AskHuman 人工 End gate 如不能由本人完成，明确保留，不伪造答案。
