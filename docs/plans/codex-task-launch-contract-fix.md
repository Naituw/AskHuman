# Codex 任务启动协议修复与路径验收

> 2026-10-07：创建报错修复已实现、安装并经用户 IM 验收；随后发现的 App 接管覆盖 YOLO 也已修复、安装并核对真实执行权限，见 §7。
> 依据：`docs/specs/codex-desktop-session-integration.md`、实际操作台账、App 内置
> `0.162.0-alpha.2` schema，以及 Codex 上游源码和提交历史。

## 1. 已确认的问题

### Desktop YOLO 的请求枚举错误

`codex_desktop/runtime.rs` 给 `thread/start.sandbox` 发送 `dangerFullAccess`。
该字段的类型为 `SandboxMode`，当前合法值是 `danger-full-access`。
`turn/start.sandboxPolicy.type` 的 `SandboxPolicy` 才使用 `dangerFullAccess`。

上游曾在 2025-12-11（北京时间）通过提交
[`bfb4d571` / #7658](https://github.com/openai/codex/commit/bfb4d5710b883df074ff3af8da3766dca83cfd2f)
把 V2 `SandboxMode` 从 camelCase 改为 kebab-case。
`0.160.0`、`0.162.0-alpha.2` 和调查时主分支 `b17c74cf` 的这个契约一致。

实际失败操作 `eb2c465e-7257-48a9-b0f1-01f1599301aa` 使用 YOLO，台账无 session ID。
结合参数解析拒绝及只读线程数据库，确认没有创建线程或提交任务。

### 明确失败被保留为结果未知

Bridge 在开始创建前写 `unknown`；runtime 只返回错误字符串，丢失 JSON-RPC code 和失败阶段。
所以明确的参数拒绝与创建响应丢失会产生相同台账和提示。
后续相同 UUID 一律拒绝重试，GUI 的稳定 UUID 会使这种确定失败持续卡住。

### CLI 新建任务缺少参数边界

`integrations/agent_launch.rs` 的 New 分支把任务文本直接放在 argv 最后，没有 `--`。
默认和 YOLO 都受影响：以 `--...` 开头的文本会被 Codex 当成命令行选项。
用当前独立 CLI `0.148.0` 的无副作用解析探针验证，两种模式均退出 2，并提示加 `--`。
Fork 分支已有该分隔符。

## 2. 建议修复范围

### 2.1 明确请求类型与权限行为

- 将 `thread/start` 参数组装收敛为单一函数和小型可序列化类型。
- 默认权限省略 `approvalPolicy` 和 `sandbox`，继承目标目录的有效配置；不固定为 workspace-write。
- YOLO 明确发送 `approvalPolicy=never`、`sandbox=danger-full-access`。
- App 接管后，YOLO 创建的首轮显式发送 `approvalPolicy=never` 和
  `sandboxPolicy.type=dangerFullAccess`，保证覆盖恢复阶段的默认权限（后续批准的修正见 §7）。
- 保留 `cwd` 和 `ephemeral=false`，辅助进程只创建空线程，不执行 `turn/start`。
- 权限领域仍只有 AgentDefault / Yolo；不增加新 UI 选项。
- 独立表示线程的 SandboxMode，避免对其它原生 IPC 字段做全局字符串替换。

### 2.2 按阶段记录结果

runtime 内部保留 JSON-RPC code、method、失败阶段和已获得的 session ID；Bridge 负责落台账。
外部入口复用统一结果文案，不根据英文错误字符串猜测状态。

| 状态 | 判定依据 | 用户显式重试时的行为 |
|---|---|---|
| 未创建 / rejected | 本地校验或 spawn 失败；创建请求尚未发送；thread/start 明确返回参数解析拒绝 | 可用相同 UUID、相同内容再试，仍使用原执行器 |
| 已创建 / created | session ID 已成功持久化，首条任务尚未尝试提交 | 复用原 session ID，继续剩余步骤，不再次 thread/start |
| unknown | thread/start 写入/响应有歧义；首条消息已尝试提交但没有可靠回执 | 提示检查原聊天，不自动重发或回退 CLI |
| accepted | 首条任务已获得接受回执 | 相同 UUID 只返回原回执 |

约束：

- 仅明确的解析拒绝（例如 -32600/-32602）可证明 thread/start 未执行；其它服务端错误保守处理。
- 拿到 ID 后 name/read、进程退出或 App 激活失败，应保留 ID 和准确失败阶段。
- session ID 落盘失败无法建立可靠恢复依据，保守保留 unknown，并提供已知诊断信息。
- 本次先不扩大私有 start/steer/stop 错误的安全重试条件；它们的传输歧义继续按 unknown 处理。
- 任何重试都是用户再次提交触发，失败处理本身不自动重试。
- 相同 UUID 禁止不同内容；改变偏好也不迁移已落台账的操作。
- 历史 unknown 台账不因“缺少 sessionId”而自动降级为 rejected，不删除或重放本次旧操作。
- 默认/YOLO、任一入口失败时都保留草稿、待办及必要交付文件；Desktop 仅 accepted 后待办出队。

### 2.3 修复 CLI 参数边界

Codex New 的 argv 固定为：

```text
默认：codex -- <完整任务文本>
YOLO：codex --dangerously-bypass-approvals-and-sandbox -- <完整任务文本>
```

任务、换行和附件描述始终是一个 argv；不改变其他 Agent 的适配器。
Fork 继续使用现有固定 argv，并分别验证默认 / YOLO。

### 2.4 文档和既有边界

- 修正 Desktop spec 的 sandbox 拼写，补充明确拒绝、已创建、未知结果的恢复契约。
- CLI 参数说明放在任务启动的适用 spec / plan；仓库模块地图没有变化时不改主 overview。
- Desktop 来源仍不允许套用终端 Fork。验证选择器及提交时的来源复检；若后端可绕过，按既有
  spec 补齐拒绝，不新增 Desktop Fork 能力。

## 3. 按行为差异组织验证

共享核心测试覆盖协议与台账；每个入口另测自身的输入保留和成功收尾，避免把同一核心重复
跑完后误认为所有入口都已验收。

| 路径 | 必须覆盖的差异 | 通过条件 |
|---|---|---|
| Desktop 默认创建 | cwd 配置为 read-only / workspace-write / full-access，权限配置不同 | 请求省略权限覆盖；有效设置继承正确 |
| Desktop YOLO 创建 | cwd 原配置不同，手选 YOLO / 固定 YOLO 设置 | 始终 never + danger-full-access，创建成功 |
| IM / GUI / 待办 | 固定默认 / 固定 YOLO / ask 选择；取消选择；直接文本 / 待办+补充 | 权限同源；取消无创建；失败保留内容；仅 accepted 出队 |
| 附件 | 无附件 / 普通文件 / 图片 / 文件消失 / 待办快照过期 | 图片为 localImage，文件引用完整；失效不误删待办；重试内容稳定 |
| App 激活与路由 | 已启动 / 未启动；有无已保存项目；App-only / CLI-only / 两者可用 / 两者不可用 | 创建前选择正确目标；创建后无自动迁移；App 只在实际操作时唤起 |
| 创建失败 | spawn、initialize、thread/start 明确拒绝 / 断线 / 超时、name/read、退出、ID 落盘、owner 接管 | 状态符合 §2.2；有已知 ID 不创建第二条；未知不重发 |
| 幂等与恢复 | accepted 重复、rejected 重试、created 续接、unknown 重试、偏好变化、UUID 内容冲突 | 线程数和首条消息数可计数；每种状态无重复副作用 |
| CLI 新建与 Fork | 默认 / YOLO；普通文本、以 - 或 -- 开头、换行和 shell 特殊字符 | 正确 argv，任务作为数据；新建不吞文本；Fork 保持来源身份 |
| 空闲 / 工作中发送 | start-turn / steer-turn；附件；未知回执；重复 UUID | 发往同一目标；工作中不新开 turn；重复 accepted 不再次发送 |
| Stop | 有运行 turn / 已空闲、目标 turn 改变、连接失效 | 只停止期望 turn；无法验证时明确失败，不误停其它会话 |
| 原生提问 / 审批 | 原生与异步问题、命令/文件/权限审批、Hook 已拥有审批、Secret/未适配请求 | ID/owner 保持一致；只批准本次；不扩大权限或产生双卡 |
| 版本与私有 IPC | 上游 0.160.0 / 当前内置 schema；owner/revision/帧版本失配 | 公共请求符合各自 schema；私有 IPC 不猜测降级 |

默认配置组合和响应故障使用临时 Codex home / 目录及可控 app-server，不污染生产配置。
真实 App 验证另外记录线程 ID、最终权限和首条消息计数，不能以 mock 或 schema 校验代替。

## 4. 验证方式与验收证据

1. 外部契约测试：用实际二进制生成的 schema 核对请求；固定上游版本的定义作为回归依据，
   不只断言实现自己写出的字符串。
2. 创建辅助进程测试：可控 app-server 捕获 initialize/initialized/start/name/read 的完整序列，
   注入各阶段拒绝、断线和超时，断言台账及重试计数；断言辅助进程没有 turn/start。
3. Bridge 和入口测试：覆盖发送/停止的真实消息组装、操作 UUID、路由锁定、草稿及待办收尾。
4. 针对 CLI 真实解析器验证参数边界；既有 Fork/argv 测试覆盖相应回归。
5. 跑相关 Rust/Vitest、必要的全项目检查；安装后使用新安装的 AskHuman，再做真实 App 与
   真实 IM 的代表路径验收。没有真实证据的项标为待验，不报告“所有 case 已通过”。

当前分析阶段已经通过 Desktop 模块 Rust 15 项、启动适配器 Rust 14 项、相关交付 Rust 9 项及
GUI Desktop 表单 Vitest 2 项；启动适配器的真实 Fork help 测试默认 ignored。这些测试没有覆盖完整的创建请求
及新设计的失败恢复，也未完成本轮真实 IM/GUI 创建矩阵。

本机 Desktop 安装版只读检查显示连接正常；独立 CLI 为 0.148.0，当前启动就绪判定为
`cliTrackingNeedsUpdate`。这项阻止实际终端启动，需在隔离验证环境满足就绪条件后进行真实验收。

## 5. 实施环境

按用户后续要求，在当前 checkout 开发，不创建 worktree，并保留其他 Agent 的修改。
协议与故障注入使用临时目录 / Codex home；在当前目录跑 `./scripts/install.sh`，再通过新安装
的 AskHuman 和真实 App / IM 验证。

实施顺序：请求/错误类型 → 台账状态与恢复 → 入口文案 → Codex CLI 参数 → 回归与实机验收 →
更新适用文档。将每项验证结果补到本计划；未完成的持久外部验收记录在 PROGRESS，用户明确
延期时才记录为项目 todo。

## 6. 2026-10-07 实施与验证记录

已实现有类型的 ThreadStartParams / SandboxMode、带阶段和 RPC code 的创建错误、
独立操作台账及 rejected / created / unknown / accepted 恢复。拿到 ID 后立即保存 created；
命名/读取/退出失败的显式重试只补原线程的剩余设置，激活失败不重复运行已完成的辅助进程。
首条消息提交前保存 unknown，accepted 重复只回原回执。历史 unknown 保持阻止重发。
IM 不再为所有错误统一追加“检查后再创建”；GUI 保留旧版 pending UUID，并将待办附件快照
纳入新操作内容键。Codex New 的默认/YOLO argv 已加 `--`。Desktop Fork 的选择器、
GUI 初始化/提交及 IM 指定来源/最终提交均补齐来源复检。

自动验证证据：

- 全量 Rust：1268 passed / 3 ignored；之后新增进程与台账组合测试，Desktop 相关 24 项通过。
- 全量 Vitest：266 passed，Node 5 passed；随后补旧版 pending UUID 兼容回归，相关 GUI 6 项通过。
- vue-tsc / Vite production build、Clippy all-targets、git diff --check 均通过。
- 固定测试契约来自实际内置 0.162.0-alpha.2 生成的 ThreadStartParams schema，保留来源与所用
  字段定义。可控辅助进程覆盖 initialize、thread/start 明确拒绝/其它错误/断线/超时、name/read、
  退出、ID 保存失败和 spawn 失败；线程设置序列不含 turn/start；续接的 thread/start 次数可计数。
- 在独立临时 Codex home 中用真实内置运行时验证 read-only / workspace-write / full-access ×
  默认 / YOLO 六组有效权限，全部符合预期；新辅助进程能够给原 ID 命名/读取，不创建第二条。
  临时证据：`/var/folders/sm/h90d_zys04110r1_b2c3b9n80000gn/T/askhuman-codex-permissions-h7bm6jlc/results.json`。
- 独立 0.148.0 CLI 与内置 0.162.0-alpha.2 的默认/YOLO 四组真实解析探针均接受以 `--` 开头的
  多行任务，进入 TUI 后因无 TTY 退出 1，没有选项解析错误；这不是实际终端启动验收。
- `./scripts/install.sh` 已完成当前 checkout 的 local-install 编译、正式签名与安装。

安装后旧 daemon 按 graceful drain 自然换新，新 daemon PID 6318。用户从原 IM 入口确认创建
成功，并要求提交。操作 `b8f52363-227a-4924-8cc9-b2e562ca39d2` 为 YOLO / accepted，
线程 `01a116b5-1c53-7881-a1e8-5f7ee559241e` 已由 App 接管。安装版同 UUID、同内容
调用返回原 accepted 回执、原 session ID，台账时间未改变。
原故障操作 `eb2c465e-7257-48a9-b0f1-01f1599301aa` 未修改或重放。

尚不以自动测试冒充真实 IM 全矩阵、未启动 App、终端就绪修复后的 TTY 启动及逐类原生审批
实机验收；本次真实代表路径的结果在完成后继续补记。

### 实机验收新发现：App 接管覆盖 YOLO

提交前核对真实 rollout 时发现：IM 台账选择 YOLO，但 App 接管后的两次
`thread_settings_applied` 和首轮 `turn_context` 均为 on-request / :workspace（网络受限）。
因此本轮已经解决创建参数拒绝和安全恢复，不能声称实际首轮执行仍保持 YOLO。
隔离辅助进程中的 never / danger-full-access 六组结果仍成立；差异发生在真实 App 接管阶段。
当前私有 start-turn 的 `inheritThreadSettings=true` 没有保持预期权限，需要继续核查新版本
permission profile 和 Desktop 接管契约，并确认修复方向。没有对这个真实线程再次发消息、
停止或修改权限。此问题已写入 PROGRESS，按用户要求先提交已验收的创建修复。

## 7. App 接管后首轮 YOLO 修复

2026-10-07 用户确认同一任务的输入框显示 Full Access，且没有手动修改权限；要求先分析，
随后通过 AskHuman 确认首轮显式传递权限的方案，并要求修复、安装及验收最终执行权限。

当前安装 App 26.1002.52244 / 内置 0.162.0-alpha.2 的实际日志：

- 14:12:12.940Z `maybe_resume_success`：hasCurrentPermissions、hasExplicitPermissions、
  hasLatestThreadSettings、hasLatestTurnParams 均为 false，turnCount=0；恢复请求没有
  permission override；响应是 on-request / :workspace。
- 14:12:13.453Z `Reasoning summary turn-start config resolved`：首轮实际请求是
  approvalPolicy=on-request、permissions=:workspace，useAppServerPermissionDefault=false。
- 对应 rollout 的 thread_settings_applied 和首轮 turn_context 与日志一致。

安装包 bootstrap 的恢复逻辑没有空线程缓存设置时按 App/server 默认恢复；start-turn 的
inheritThreadSettings=true 继承恢复后的状态。输入框权限 hook 同时读取线程状态和本机
agent-mode-by-host-id 偏好，不能作为过去轮次的权限审计。本机保存 local=full-access，
解释了用户看到的选项与 IM 首轮的实际权限不同。

实施只改 YOLO Create 的首轮请求：显式 approvalPolicy=never、
sandboxPolicy={type:dangerFullAccess}；不携带冲突的 permissions 字段。当前 App bootstrap
的请求组装确认显式字段优先；生成的 TurnStartParams schema 明确这些覆盖作用于本轮及
后续轮次。默认 Create 与 Send 仍继承，不在后来用户改过权限时再次强制 YOLO。
若恢复期间出现其它活动 turn，不能把 YOLO 新建任务作为 steer 混入该轮；错误保留 created
台账和原 ID。已有恢复/去重状态机保持不变，结果未知不自动重发。

回归验证：Desktop 相关 Rust 25 项通过，包括恢复为 :workspace 的首轮覆盖、默认/普通
发送不覆盖、活动轮次保护及此前的进程故障/台账恢复/UUID 去重。首轮字段与实际生成的
TurnStartParams 固定 schema fixture 对照。Clippy all-targets、安装脚本中的 vue-tsc / Vite
生产构建、local-install 编译与正式签名、git diff --check 均通过。新 daemon PID 70422。

安装版经真实 App owner 提交的独立测试：

| 路径 | operation / thread | 最终执行上下文 |
|---|---|---|
| YOLO Create | 86085a15-13b6-416f-a7e9-1b76af272a7c / 01a116dc-fe5a-71c1-8874-19c2c000827f | approval_policy=never；sandbox_policy.type=danger-full-access；permission_profile.type=disabled |
| 默认 Create | 0d973851-0a49-4619-839e-429c0427ea8a / 01a116dd-d756-7b93-8536-39937867ffd8 | approval_policy=on-request；sandbox_policy.type=read-only；managed restricted filesystem/network |

App 日志的 YOLO 首轮 requestApprovalPolicy=never、requestSandboxPolicyType=dangerFullAccess、
requestPermissionProfile=null；对应数据库为 never / disabled。rollout 的 SandboxPolicy 使用
kebab-case，与 turn/start 的 camelCase 请求编码不同，不应据此替换请求拼写。
两条线程都实际执行 date +%s，exitCode=0；同 YOLO UUID 重复调用返回原 session ID / accepted，
台账 mtime 不变，canonical UserMessage 和 turn_context 各一条。恢复重试由上述故障注入
测试覆盖；本轮没有人为制造真实 App 的断线或未知结果。

测试线程完成命令后按全局 AskHuman 协议进入 whats_next 等待；验收读取了已执行命令及
权限上下文，随后只停止这两条测试 turn，并确认 idle / interrupted 后归档。未把中断状态
表述为模型完整结束。原用户工作线程和最初失败操作没有被重放或修改。
临时操作、回执及上下文摘要保存在
`/var/folders/sm/h90d_zys04110r1_b2c3b9n80000gn/T/askhuman-yolo-final-z40rzs69/`。
