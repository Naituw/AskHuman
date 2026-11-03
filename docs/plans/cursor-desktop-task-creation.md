# Cursor Desktop（Agents 窗口）新建任务实施与验证

需求与决策见 [规格](../specs/cursor-desktop-task-creation.md)；Max 逻辑见 [Max 计划](cursor-max-control.md)。
同一 worktree、同一分支实现，排在 Max 控制之后。

**硬约束**：开发与测试不启动真实 Cursor、不发送提示词；适配器只用合成对象图。Dev Instance 按设计不启用该能力，
所以真实验证只能在日常安装上做，并且只在验证计划获批后。

## 实施

1. **适配器**：消息桥的渲染器适配器（`message_renderer.js`）增加 `create(operation)`——Agents 窗口与项目选择、结构预检、
   `createAgent` 加提交前闸门（读回核对通过才允许提交）、`dryRun`、`pending` 与按操作 ID 去重；主进程分发
   （`message_dispatch.js`）只在唯一的 active Agents 窗口执行。背景完成适配器及其修订不动。
2. **Rust 运行时**：`cursor_desktop::Operation::Create`、回执（`cursor-desktop-actions/<id>.json`）、状态与不可用原因、
   桥调用与结果映射；`launchPreference` 配置与目标选择（与 Codex 同构）；整体可用性只做静态检测。
3. **IPC**：新增 `ClientMsg::CursorDesktop` / `ServerMsg::CursorDesktop`，`PROTOCOL_VERSION` 4→5；先读 overview 的
   「IPC 协议号」一节，更新历史与 `daemon-graceful-drain.md`。
4. **入口**：`start_task` 与 GUI `new_task_launch` 的 Desktop 分支泛化为按 Agent 种类；iOS 回执里的 Agent 种类不再写死
   Codex；表单加 Cursor 的目标状态行；操作 ID 复用以便重试。
5. **Max 与权限**：`modelConfig` 由 Max 核心模块构造；`yolo`→`agentApprovalModeOverride`；提交后读 `composerData`
   复查 `maxMode`，异常发告警卡。
6. 文档：规格与计划、overview 的模块地图与新任务一段、`overview-configuration.md` 的新字段、用户向 wiki
   （`settings.md` / `settings.en.md`）；协议号历史与 `daemon-graceful-drain.md` §8 记 4→5 的过渡。

## 验证

- JS：假服务与假草稿的对象图脚本测试——成功、结构预检失败、非 Agents、各核对项不符、提交抛错、重复调用、`dryRun`
  清理。Rust：桩桥与临时目录覆盖回执阶段、崩溃恢复、选择规则、错误映射、协议新旧互通。
- Dev Instance 内只验证非 Cursor 部分（界面、状态行、协议）。
- **验证计划**（实现后提交审批，批准后执行）：先 `dryRun`（不计费），通过后一次最小真实创建；失败即停，不重试。

## 验证记录

（实现后补。）
