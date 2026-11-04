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
- **真实验证**不在本计划内自动执行：验证计划写在 [Desktop 计划](cursor-desktop-task-creation.md)里；2026-11-04 用户裁定阶段 1–2 由 Agent 在日常版装好后做、
  阶段 3–4 由用户自己做。

## 验证记录

2026-11-03～04，全部在 worktree 与本机完成：没有运行过 `cursor-agent`，没有驱动 Cursor 运行任何 Agent，没有碰真实的 `~/.askhuman`。
这台机器 8 核 16 GB，期间其他 Agent 的构建与模拟器让平均负载常在 500–1000，下面几处超时都是这个原因。

- **2026-11-04 把 main 合入分支后复测**（合并提交 `chore: merge main into feat/cursor-max-control`，5 处冲突的处理写在提交说明里；
  复测时机器在换页，健康检查多次报 thrashing，负载 25–150）：`cargo fmt --check` 与 `clippy --all-targets -D warnings` 在合并提交的 pre-commit 钩子里通过；
  `cargo test` 2448 通过、0 失败、11 条忽略；`vue-tsc` 通过；vitest 94 个文件 1115 条全部通过（超时放宽到 120 秒）；node 133 条中 114 通过、19 跳过、0 失败
  （下面那两条安装脚本测试这次没有超时）；Swift 包 612 条执行、569 通过、43 条按设计跳过、0 失败；两条 Max 的 iOS UI 测试英文通过（285 秒、72 秒）。
  中文没有重跑 UI 测试（合并没动界面代码），改为核对了字符串目录：540 个键都有 zh-Hans，Max 相关的 8 条齐全。
  之后 main 又进了两个提交（失败卡不再自己撤回，改了 `daemon/runtime/mod.rs`），第二次合入（无冲突）后重跑了 `cargo fmt --check`、
  `clippy --all-targets -D warnings` 与 `cargo test`（2449 通过、0 失败）；前端、Swift、node 与 UI 测试不受这两个提交影响，沿用上面的结果。

- Rust：`cargo test` 2409 通过、0 失败、11 条本来就忽略；`cargo clippy` 与 `cargo fmt --check` 无告警。
  覆盖合成目录夹具（全 Max、混合、无 Max、歧义 id、参数不全、缓存缺失／损坏）、配置文本替换（键序、缩进、权限、无法解析不动、并发互斥）、
  记录哈希新旧兼容、三个入口的拦截／提示／放行、复查的触发与超时。
- 前端：`vue-tsc` 通过；vitest 94 个文件 1111 条，首轮 5 条超时（`PopupInboxView.drop` 4 条、`.later` 1 条，默认 5 秒在负载约 900 时不够，
  与本分支无关），这两个文件放宽超时重跑 98/98 通过。
- Swift 包（AskHumanKit 七个测试包）：569 条通过、0 失败，另 43 条要真实服务的端到端用例按设计跳过；含 `TaskMaxTests`
  （开关默认关、需要 Max 时拦截、放行后可启动、旧 Mac 不显示、换 Agent 或 Mac 时复位）。
- iOS 模拟器 UI 测试（`AskHumanModelUITests` 的两条 Max 用例，iPhone 16 / iOS 18.5，英文与中文各跑）：
  `testCursorOnTheOldPlanStartsWithMaxOffAndBlocksAModelThatNeedsIt`（开关默认关、选需要 Max 的模型被拦下并说明、打开开关解除、换小模型读「不需要 Max」、
  放行后能启动）英文 261 秒、中文 276 秒通过；`testTheMaxSwitchBelongsToCursorAloneAndToAMacThatControlsMax`（只有 Cursor 且 Mac 控制 Max 时才有开关）
  英文 115 秒、中文 148 秒通过（英文之后又重跑过两次，一次等待超时、一次 XCTest 与应用失联，都是负载近 1000 时的基础设施失败，不是断言失败；
  这条的测试体在两次通过之后没有改过）。截图看过：被拦下时开关关、「将使用：… 需要 Max」橙字、红字说明、「开始」灰；打开开关后红字消失。
  过程中修了测试自己的两个问题：第一条的最后一步误用了 Cursor 表单里没有的「权限」行（该 Agent 的权限由它自己决定），改为直接填文字并点开始；
  负载近 1000 时 10 秒的等待不够（改为 30 秒，点开关改为不会因点击迟到而翻回）。
- Node：`cursor-create.node-test.mjs` 19/19；全量 `scripts/*.node-test.mjs` 119 条中 98 通过、19 跳过（平台限定）、2 失败——都是安装脚本测试
  （`install-unix` 的 `TEST_FAIL_SIGN`、`seed-worktree-target` 的「path packages cannot be listed」）在重负载下 `spawnSync bash` 30 秒超时，本分支没改 `scripts/install*`。
- Dev Instance（`scripts/install.sh` 装到 worktree 的 `.askhuman-dev/`）：握手、协议号 5、Desktop 路线按设计不可用。
- 界面：用 Chromium 加假的 Tauri 桥看过 GUI 新建任务表单的 Cursor 行（浅色／深色、中英文）：模型选择器与 Max 标记、「将使用」行、开关、拦截红字；一次性脚本，未提交。
- 本机只读观察（零请求；读 `state.vscdb` 与 `cli-config.json` 的白名单键，不读凭据）：`hasTokenBasedPricing=false`（旧计划）；47 个模型里 27 个可不开 Max 使用、
  20 个按缓存仅限 Max（含 CLI 默认的 Grok 4.7 256K High Fast）；`cli-config.json` 的 `maxMode` 与 `model.maxMode` 都是 `false`。
  对照 Cursor CLI 2026.10.01 的代码（只读）：`resolveMaxModePersistence` 在所选变体需要 Max 时写 `maxMode:true`（并标 `maxModeAutoEnabled`），
  换到不需要 Max 的模型时只在「由自动开启」的前提下才自动关回，所以手动开启过的持久 Max 会一直粘着——这正是启动前强制关闭的原因。
- **未验证**（要真实 Cursor／cursor-agent；日常版装好后，阶段 1–2 由 Agent、阶段 3–4 由用户做）：CLI 在强制关闭之后的实际行为与计费、缓存与服务端目录是否一致、Agents 窗口路线的全部未验证项。
  验证计划见 [Desktop 计划](cursor-desktop-task-creation.md)的「真实验证计划」。
