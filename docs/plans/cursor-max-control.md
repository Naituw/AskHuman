# Cursor Max 模式控制实施与验证

需求与决策见 [规格](../specs/cursor-max-control.md)；Desktop 创建见 [另一份计划](cursor-desktop-task-creation.md)。
2026-11-03 起在独立 worktree `feat/cursor-max-control` 实现，用 Dev Instance 验证非计费部分，最后合并。

**硬约束**：全程不运行 `cursor-agent` 任务、不驱动 Cursor 运行 Agent；测试只用桩可执行文件、临时配置目录与合成目录 JSON。

## 实施

1. **核心模块 `agents/cursor_max`**（纯函数优先，夹具驱动）：只读目录读取（IDE 缓存 `ItemTable`）、变体解析与
   `requires_max`、计划检测与设置覆盖、`cli-config.json` 读取（只取白名单键）与最小文本替换（跨进程锁、原子写入）、
   统一入口 `assess()`（计划／策略／允许 Max／模型 → 放行、带提示放行、拦截）。
2. **启动链路**：`ModelChoice` 增加 `allow_max` 并沿 `TaskSpec`、`LaunchRecord`、`payload_sha256` 贯穿（缺省不改哈希）；
   `create_record_internal` 统一判定与拦截；`run_helper` 在 `exec` 前再判一次并强制关闭配置。
3. **启动后复查**：启动方进程里一个最长 45 秒的有界任务，发现 `maxMode:true` 且不允许时写日志并经 `internal_ask`
   发「知道了」卡；GUI Host、IM、iOS 三条路径共用。
4. **入口**：GUI（`new_task_init` 返回 Cursor Max 状态；新增 Cursor 选项命令返回模型清单与 Max 标记；`new_task_launch`
   接收模型与允许 Max；表单增加选择器、「将使用」行与开关）；IM 按默认策略并给出拦截文案；iOS 线协议增加
   `maxControl`／`maxRequired`／`allowMax`（可选字段，协议号不变），手机增加开关与标记。
5. **设置**：`agentTasks.cursorMax.{policy, plan}`，设置页 Cursor 小节与检测结果只读显示，中英文。
6. **Desktop 会话 Max 只读显示**：`session_info` 读取 `modelConfig.maxMode`，会话信息里标红「Max」。
7. 文档：局部实现地图与 `docs/PROGRESS.md`（只记未完成项）；不改主 overview 的全局架构，除非实现使其不再准确。

## 验证

- Rust：合成目录夹具覆盖全 Max、混合、无 Max、歧义 id、参数不全、缓存缺失／损坏；配置文本替换的键序／缩进／权限／
  无法解析不动／并发互斥；记录哈希新旧兼容；三个入口的拦截、提示与放行；复查的触发与超时。
- 前端：表单行、选择器、开关、新计划与「沿用」策略下隐藏；设置页。Swift：新字段、开关、标记、旧 Mac 兼容、共用夹具；
  中英文模拟器界面测试。
- `scripts/install.sh`（Dev Instance）后在本树用 `dev_ask` 验收界面与告警卡；不触发任何 Agent。
- **真实验证**不在本计划内自动执行：实现完成后单独提交「验证计划」，经用户批准才进行。

## 验证记录

（实现后补。）
