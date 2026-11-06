# 桌面新建任务设置对齐 iOS

2026-11-06，用户确认在主工作树实施，完整覆盖模型、思考强度、Codex Fast、Cursor Max 和本机偏好。
行为规则见 [GUI 规格 §8](../specs/gui-agent-task-launch.md#8-桌面新建任务设置与-ios-对齐2026-11-06)。

## 实现

- `agents/task_options.rs` 为桌面提供当前启动路径的目录和能力，并在副作用之前验证选择；
  复用现有源、缓存、选择解析与 Codex 工作区速度校验。
- `new_task_options` 替代 Cursor 专用选项命令；`new_task_launch` 向终端和 App 传递完整 `ModelChoice`。
- `useTaskSettings.ts` 管理桌面偏好、失效回退、Fast / Max 复位；三个入口共用同一表单与搜索。
- 失败重试保留草稿与操作身份，改变模型 / 强度 / 速度 / Max 生成新身份。
- 原主工作树有其他并行改动；本次只修改任务设置相关代码和对应规格，不处理其他功能。

## 验证记录

- `pnpm test`：Vitest 全量 1293 项通过；Node 139 项通过、19 项按平台跳过。
- 最终任务表单专项 `pnpm exec vitest run src/views/newtask`：36 项通过，含升级前回执与跨 Agent 保留回执的补充回归。
- `cargo test --manifest-path src-tauri/Cargo.toml --bin AskHuman`：2614 项通过、11 项忽略；含新增的 4 项目录 / Max / 速度投影验证。
- `vue-tsc --noEmit` 与最终 `./scripts/install.sh` 成功；生产前端、Rust local-install 构建和稳定身份签名完成，安装到 `~/.local/bin/AskHuman`。
- 安装后已使用正式 AskHuman 发起表单验收提问；用户开启离开模式，尚未取得真人界面反馈。原 daemon 已自动换新，未强制打断其他在途请求。
- 原生窗口控制工具不能定位这个未注册为 App bundle 的 GUI 宿主，因此没有把 jsdom 组件测试称为真实桌面点击验收。待用户查看三个桌面入口的模型、强度和 Fast / Max 显示；真实模型启动仍未执行。
- 全程未提交真实模型任务；未用自动化测试代替真实计费启动验收。


2026-11-06 用户已通过 AskHuman 接受当前实现与自动化验证作为本次交付，选择保留真人界面未确认的记录、日常使用时观察。
后续界面验收已登记项目待办 #9（`90e4972e-cf41-4c59-9a0b-a216989915fb`）；不触发真实模型测试。
