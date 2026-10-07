# Codex App 会话接入

> 2026-10-04 定案：首版本机 macOS；在当前 worktree 实施。
> Windows、Linux、SSH、手机 Web、账号/API 管理不在本次范围。

## 配置与入口

Codex 卡沿用整体提问集成（CLI / MCP / 未集成）、权限与生命周期选项；macOS 在原有选项
最后增加一行「默认运行方式」：**优先 Desktop App（默认） / 优先 CLI**。下方显示检测结果
「新任务将使用 Desktop App / CLI」。实际目标与偏好不一致时，括号说明具体原因（缺少 App/CLI、
追踪未开启、配置需更新等），右侧显示「刷新」按钮；两边不可用时说明双方原因，也可刷新。
刷新期间按钮禁用并显示「检测中…」，重新检测成功后更新结果；恢复首选目标时原因和刷新按钮
一起消失。切换任一运行偏好时，结果行立即显示「检测中…」，只在本次检测结束后显示结果；
不沿用上一次偏好的结果冒充当前检测。检测失败会结束等待并显示错误。检测不改变偏好、不启动 App。没有独立连接开关、连接状态面板、聊天列表或路径表单。

CLI/MCP 表示调用 AskHuman 的方式；运行偏好表示新任务的执行位置。两类会话共存：优先项
不可用时选择另一项，两者不可用才阻止启动。App 可用性依赖本地安装及已启用 Codex 集成，
不依赖 PATH 中的 codex 或终端生命周期 Hook，也不以 App 是否正在运行、socket 是否连接作为判断。
关闭整体 Codex 集成同时停止桌面接入；切换运行偏好不关闭已有连接或迁移已有会话。

设置页、新建表单、IM `/new` 与实际启动共用后端判定。配置事件、页面重新聚焦和集成模式变化
会重新读取偏好与结果；旧检测响应不能覆盖后续保存。新建流程只选 Codex，显示实际启动位置，
不再增加 App/CLI 选择步骤。全局偏好提供长期回退；不另设单次目标菜单。App 未运行时仅在用户
启动任务或打开原会话时唤起，后台检测不会打开 App。

控制台复用既有输入、附件和快捷键，空闲桌面会话也可继续；「打开原会话」使用标题栏动作，
停止只在运行时出现。错误在对应操作旁显示，保留输入和附件。GUI 新建、待办启动、IM `/new`
共用 daemon 创建编排；IM `/msg` 对桌面会话直接提交。现有终端 Fork 不能套用于桌面会话。
一旦创建产生操作台账，后续重试仍指向原执行器，即使偏好改变也不把结果未知的操作转到终端重跑。

## 实现与不变量

- `src-tauri/src/codex_desktop/`：`runtime` 检测安装、只读 SQLite 会话发现、短期创建进程；
  `launch` 共用运行偏好、可用性与回退原因判定；`protocol` 处理版本化 IPC、帧和状态增量；
  `requests` 适配实时原生问题和审批；模块根负责
  owner、连接、控制操作与结果台账。原生连接只由 daemon 持有，GUI/CLI 均走 AskHuman IPC。
- 本机端点是 Codex 数据目录下的 `ipc/ipc.sock`。协议为 4 字节小端长度 + JSON；广播
  `thread-stream-state-changed` 当前适配版本 11。只有 host、会话、owner 和 revision 匹配才
  可操作；patch 失配废弃控制资格并重新订阅，不猜测增量或自动切换执行器。
- SQLite 只发现最近 24 条未归档桌面会话，排除 CLI 和子 agent；历史文件不产生待审批。
  Hook 与桌面状态按真实 session ID 合并；Watch 文本可取原生最新回复，回复出现不等于完成。
- 原生 `requestUserInput` 和异步问题进入现有 AskHuman 提问协调器；原生请求 ID 参与去重，
  避免相同题面重放上一轮答案。回答前重新验证 owner 和请求内容，请求消失即撤销等待。
- 命令、文件和权限申请支持单次批准 / 拒绝；权限申请限定本 turn。沿用 Codex 权限开关；
  已安装 AskHuman 权限 Hook 时由 Hook 独占审批，以保持记忆规则并避免双卡。
  Secret 问题、MCP elicitation、计划执行确认及其它未适配请求仍在 App 中处理。
- 发送 / 停止 / 新建均有 UUID，在 `state/codex-desktop-actions/` 持久化私有台账。
  相同 ID 不接受不同内容；已接受操作重试只回执，结果未知不自动重发。GUI 保留失败草稿，
  新建待提交 ID 保存在本地；控制台需明确检查原聊天后才能编辑新的发送。
  新建的明确拒绝记录为 `rejected`，用户再次提交同一 UUID 时可重试；立即落盘线程 ID 和
  `created` 后，命名、读取、辅助进程退出及 App 接管失败均复用原线程续接。首次任务提交前
  写 `unknown`，收到接受回执后才写 `accepted`。解析错误仅 -32600/-32602 证明未执行
  thread/start；其它错误保守处理。台账保留失败阶段、RPC method/code 和已知 ID；ID 落盘
  失败仍为 unknown，已知 ID 仅作诊断。历史 unknown 不自动降级或清理。

## 新建链路与权限

```text
验证目录、任务、权限、附件及操作 UUID
  → App 内置 codex app-server --listen stdio://
  → initialize / initialized → thread/start(ephemeral=false)
  → 立即记录 thread ID / created → thread/name/set → thread/read(includeTurns=true)
  → 关闭 stdin，等待辅助进程退出
  → codex://threads/<id> 打开 App → 等待 owner 接管
  → 原生 start-turn 提交首条任务 → 记录 accepted → 待办出队
```

辅助进程不执行 turn/start。目录可以不是 App 已保存项目；不创建 worktree。默认权限继承目标
目录配置，省略 approvalPolicy 和 sandbox；YOLO 明确设置 approvalPolicy=never、
sandbox=danger-full-access。App 接管空线程可能按默认权限恢复，因此 YOLO 创建的首轮
私有 start-turn 还须显式携带 approvalPolicy=never、sandboxPolicy.type=dangerFullAccess；
此覆盖作用于本轮及后续轮次。默认创建和后续普通发送继承 App 当前线程设置。
若 YOLO 创建在提交首条任务前发现线程已有活动轮次，保留已创建 ID 并提示检查，不将任务
作为 steer 发送进未知权限的轮次。thread/start 使用 SandboxMode；turn/start.sandboxPolicy
的 SandboxPolicy 类型使用 camelCase，不能全局替换拼写。
App 打开失败保留已创建 ID；传输结果未知保留任务和附件，提示检查原聊天，不回退终端。
深链接可能切换桌面当前聊天。发送中附件以绝对路径引用，图片同时使用 localImage 输入。

## 验证与参考

实测机器：ChatGPT.app `26.930.31730`，内置 Codex `0.160.0`；运行时路径为
`Contents/Resources/codex-cli/bin/codex`。已验证私有 IPC v11、canonical turnHistory、
独立目录的空聊天创建、持久化、owner 接管、首条回复及原生中断。
运行偏好改版已通过 Rust 1196 项（2 项忽略）、Vitest 192 项、Node 5 项及 Clippy；覆盖选择矩阵、原因分类、无 socket 的安装检测、配置往返、焦点同步、异步竞态、刷新恢复和 App-only 表单。
适配器测试覆盖帧、owner / revision / 版本失配、canonical 顺序、请求答案映射、权限范围、
只读发现和台账；安装版已验证原生新建、继续回复、运行中补充进入同一 turn、停止、相同 UUID 去重和未知结果
禁止重发。测试聊天停止后已归档。运行中补充确认的是 App
接收记录，不将它表述为模型已执行。原生提问 / 审批通过协议与映射测试，尚未逐类做真人 IM
实机验收；原生 GUI 自动化工具无法绑定独立 AskHuman 二进制；设置卡已用真实 Vue 组件和原有样式在浏览器中核对布局、切换显示，浏览器后端为测试桩，不冒充实机 IPC 验证。

参考 [Codex Mobile Bridge](https://github.com/try2love/codex-mobile-bridge/tree/9059cbaa077f55ab4b5ea30333b7b70dee5f509e)
固定提交 `9059cbaa` 的 `bridge/ipc.py`、`service.py`、`create.py`、`model.py`。
该协议是 App 私有实现，不能视为稳定公开 API；不兼容时只显示原因，不发送降级命令。
第三方署名与 MIT 许可保存在 `src-tauri/src/codex_desktop/NOTICE.md`。

设置交互已获用户验收：卡片末尾的偏好与结果、回退原因及右侧刷新按钮、双向切换的即时检测反馈。

2026-10-07 创建契约修复已安装并经用户真实 IM 验收，安全重试通过故障注入和安装版 accepted
回执复验。随后证实新 App 接管空线程时重新应用 :workspace，导致仅靠
`inheritThreadSettings=true` 的首轮没有保持 YOLO。已将首轮改为显式传递 YOLO，安装后
独立测试线程的 turn_context 为 never / danger-full-access，permission_profile=disabled；
默认路径仍受限，同 UUID 返回原回执且未重复发送。证据见
`docs/plans/codex-task-launch-contract-fix.md` §7。
