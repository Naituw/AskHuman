# 需求：Daemon 优雅排空换新（graceful drain）

> 状态：已实现并用于 Daemon 换新、stop 与 restart。2026-10-22 补充「多份二进制互相顶替」的刹车与可观测性：D10–D12、§6（`daemon/restart_guard.rs`、`cli/install_check.rs`）。
> 关联计划：`docs/plans/daemon-graceful-drain.md`
> 影响面：Daemon 生命周期（`daemon/mod.rs`）、IPC 协议消息（`ipc/mod.rs`，仅增量字段/枚举值，PROTOCOL_VERSION 不变）、CLI 客户端（`client/mod.rs`）、`daemon stop/restart/status` 子命令、`scripts/install.sh`（及 `install-windows.ps1`）。**不改**正常提问流程的任何行为、stdout 契约、退出码语义。

## 1. 背景

多个 Agent 并行开发本项目时，任一 Agent 跑 `install.sh` 更新 `~/.local/bin/AskHuman` 后，下一个连进来的客户端握手（Hello）会让 Daemon 发现「盘上二进制指纹 ≠ 启动时指纹」→ 判定过时 → **立即** `shutdown.notify_one()` 退出。后果：

- 其他 Agent 正在等待人回答的 `AskHuman` 调用被掐断，CLI 报 `daemon connection lost`（退出码 3）；
- 对应的弹窗成僵尸窗、IM 卡片成无主卡片（虽有 2 秒兜底窗口把卡片收尾为 Cancelled，但提问本身已失败）；
- `daemon stop` / `daemon restart` 同样是立即终止，会打断在途请求。

同时受 IM 平台限制（同一应用同时只能有一条长连接），**不能**用多 Daemon 并存来解决。

## 2. 目标

一句话：**换新不打断在途，新提问等待换新完成。**

- Agent A 的提问挂起中（等人作答），Agent B 安装新二进制并再次提问：A 完全无感；B 的 CLI 打印等待提示并阻塞，待 A 的请求完结、旧 Daemon 退出后，自动拉起新 Daemon 并正常弹出 B 的提问。
- 人始终拥有「立即换新」的强制手段（明知会打断在途请求）。

## 3. 已确认决策

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 总体方案 | **方案 A：优雅排空（drain）**。检测到过时（二进制指纹 / 协议版本变化）且有在途请求时，Daemon 进入 draining 状态：在途请求继续服务直至全部完结；排空期**拒绝新 Submit**；排空完成后退出，由等待中的 CLI 拉起新 Daemon。否决：B（排空期由旧 Daemon 继续接新题——会用旧代码服务新语义，且新字段反序列化可能失败，如 `-o!` 的 `OptionItem`）；C（在途请求状态移交新 Daemon——复杂度过高，记为远期方向）；D（`ASKHUMAN_DAEMON_AUTORESTART=0` 手工管理——仅保留为逃生口，不作为默认方案） |
| D2 | 新提问的等待行为 | **无限等待**（不超时放弃），CLI 在等待期间**周期性输出 stderr 提示**：剩余在途请求数 + 「回答现有弹窗后本问题会自动弹出」+ 强制换新命令提示（`AskHuman daemon restart --force`）。等待不消耗既有的「提交重试次数」预算 |
| D7 | 等待的结束条件（2026-09-16 修正） | 等待按**实例**判定，而非按「连不上」：CLI 记住排空中 daemon 的 pid，每 500ms 查 `Status`；`Status` 无应答、**pid 变化**或 **不再 draining** 三者任一即结束等待、转向新 daemon 提交。剩余在途数只取自 pid 匹配的旧 daemon。原因：旧 daemon 退出后 ≤1s 内新 daemon 就会被任一 `ensure_running()` 调用方（agent hook、GUI Host、其他等待中的 CLI）拉起，只测连通性的 CLI 会把健康的新 daemon 当成仍在排空而永远等下去（实测有 CLI 卡了 3 天）。`daemon stop/restart` 的 graceful 等待与 `wait_until_down` 同样按 pid 判定 |
| D8 | 拉起串行化 | `ensure_running()` 的「拉起 + 等就绪」段跨进程持 `~/.askhuman/spawn.lock`，拿锁后先重新握手；macOS 上 launchd 仍报任务进程存活时不 `bootout`（bootout = SIGTERM，会杀掉兄弟进程刚拉起、可能已受理请求的实例），仅在等就绪超时后才以替换模式重拉，保留对失活实例的自愈 |
| D9 | SIGTERM 语义 | Daemon 收到 SIGTERM（launchd bootout / 登出 / `kill <pid>`）等同 graceful `daemon stop`：有在途请求则进入 drain，否则立即退出；两种都写 daemon.log。launchd 的 exit timeout（5s）仍会在其后 SIGKILL，故 bootout 触发的 drain 有界 |
| D3 | stop / restart 语义 | `daemon stop` / `daemon restart` **默认也走 graceful drain**（有在途请求时等待其完结，期间拒新提问）；新增 `--force` 标志 = 立即终止（即旧版行为，在途请求按现状收尾为 Cancelled） |
| D4 | 可观测性配套 | `daemon status` 显示 draining 状态与在途请求数；`install.sh`（及 Windows 版）在开始时检测 Daemon 在途请求数，>0 则输出提示（换新将在这些请求完结后自动发生 / 可 `--force` 立即），**不强杀** |
| D5 | 协议演进 | （当时的状态；现行 `PROTOCOL_VERSION` 为 3，何时 +1 的规则见 `docs/overview.md`「IPC 协议号」与本文 D10。）`PROTOCOL_VERSION` 保持 1（全部为增量演进，新旧消息互相可解）：`HelloStatus` 增 `Draining`；`ServerMsg` 增 `Draining { active }`（Submit 被拒时回复）；`ClientMsg::Stop` 增 `force: bool`（serde default，旧 Daemon 解析时忽略额外字段）；`StatusInfo` 增 `draining: bool`（serde default）。过渡期边界：**旧二进制 CLI** 收到新枚举值会解码失败 → 按现状报 `daemon connection lost` 退 3，可接受（正常流程下旧 CLI 不会再发起新握手：盘上已是新二进制） |
| D6 | 首次升级例外 | 本特性发布后的**第一次**升级仍由旧代码的 Daemon 主导（立即退出、打断在途），无法避免；其后所有升级享受 drain |
| D10 | 互相顶替的刹车（hold，2026-10-22） | 同一对二进制（按内容指纹）10 分钟内已互相顶替 ≥2 次后，第 3 次起 Daemon **不再**因「客户端二进制与自己内容不同」而让位：回 `HelloStatus::Ok`，照常服务该客户端（同一协议号、内容不同的客户端共用当前 Daemon）。只拦这一种让位：自身盘上二进制被换（那是升级）、客户端协议号不同、`ASKHUMAN_DAEMON_AUTORESTART=0` 的行为都不变。hold 的状态：内存里一份「粘性」状态（任何内容不同的客户端 Hello 都续期，30 分钟没有这种 Hello 即结束，Daemon 退出即清），外加持久化的换新历史 `~/.askhuman/state/daemon-restarts.json`（≤50 条 / ≤24 小时），让刚重启的新 Daemon 立刻重新推出 hold、不参与新一轮互顶。`daemon stop` / `daemon restart` 会清掉历史（人为重置）。连装新构建每次都是新内容，凑不成「同一对」，不受影响。分析见 §6 |
| D11 | 协议号不同的客户端永不 hold | 旧协议号客户端收到 `Restarting` 后会等待并最终以替换策略（`SpawnPolicy::Replace`）杀掉仍存活的 Daemon，也解码不了新版消息，让它们与新 Daemon 共存没有安全的办法，所以协议号不同永远让位。由此也要求**同一协议号内必须保持互通**（hold 之后，同号不同内容的二进制会真的互相通信）：新增枚举变体、改字段含义等会让旧端出错的改动都要 +1，见 `docs/overview.md`「IPC 协议号」 |
| D12 | 可观测性（2026-10-22） | 每次让位都在 `daemon.log` 写明原因（`client binary differs` / `own binary changed on disk` / `protocol version differs`）、客户端版本 / 协议号 / pid / 路径，以及 Daemon 自己的 exe 与指纹；`started` 行带 `exe=` 与 `fp=`；hold 开始写一行详细说明，之后至多每 5 分钟一行。`daemon status` 增 `binary` 行与 hold 警告；`StatusInfo` 增 `exePath` / `fingerprint` / `binaryConflict`（都带 `serde(default)`，新旧互相忽略）。`AskHuman doctor` 增 Install 小节，`doctor install [--warn-only] [--json]` 可单独运行：列出 PATH 上的各份二进制（按内容指纹区分，npm shim 解析到真实平台包），指出排第一的是哪份，并提示 ① 当前二进制被 PATH 里更靠前的另一份遮蔽 ② PATH 上有内容不同的多份 ③ Daemon 来自另一个文件 ④ hold 正在生效；并列出近 1 小时的换新记录。指向同一个文件的多个入口（例如 `~/.local/bin/AskHuman` 是链到 npm 包内二进制的符号链接，装一次所有名字同时更新）算同一份、不报警：单文件布局本身没有不一致，`install.sh` 照常写到链接指向的文件。`install.sh` 装完后调用 `doctor install --warn-only`（无冲突时静默；Dev Instance 只跳过 PATH 比对）。不做托盘 / 菜单栏提示；Windows 脚本与「hooks / MCP 配置里写死路径」的扫描之后再做（见 `docs/PROGRESS.md`） |

## 4. 约束与既有规则（不可破坏）

- **在途请求体验零变化**：排空期内，其 CLI 连接、GUI Helper 连接（GuiHello 凭 token）、IM 卡片交互全部照常；多渠道抢答、CLI 断开取消等语义不变。
- **单 Daemon 不变**：排空期旧 Daemon 持有 IM 长连接直至退出；新 Daemon 在旧的完全退出（释放 flock 与 socket）后才启动，不存在双连接窗口。
- **无在途请求时行为同现状**：stale 握手 → 立即退出换新，不引入额外延迟。
- **空闲退出、配置热加载等既有机制不变**。
- 排空期 `Detect`（设置窗口自动识别 userId）不接：客户端握手即得 `Draining` 而回退进程内识别（与现状「Daemon 不可达即回退」一致）。

## 5. 验收标准

1. 终端 1 发起 `AskHuman` 提问（不作答）；终端 2 跑 `install.sh` 后再发起提问：终端 2 打印等待提示并阻塞、不弹窗；终端 1 的弹窗 / IM 卡片仍可正常作答；作答后旧 Daemon 自动退出，终端 2 的提问在新 Daemon 上正常弹出并可完成。
2. 等待期间 stderr 周期性（约 30s）出现提示，含剩余在途请求数、「回答后自动弹出」说明与 `daemon restart --force` 提示。
9. 旧 daemon 退出后新 daemon 被其他进程抢先拉起（如 agent hook）：等待中的 CLI 在 ≤500ms 内转向新 daemon 提交并弹窗，不再无限等待；多个等待中的 CLI 同时拉起时不会互相 bootout 杀掉对方的 daemon，daemon.log 不出现「只有 started 没有 stopped」的换 pid。
3. 排空期 `daemon status`：显示 draining 状态与在途请求数；`im conns` 照常。
4. `daemon stop`（有在途）：打印等待进度，drain 完结后退出；期间新提问被拒并等待。`daemon stop --force`：立即退出，在途请求 IM 卡片收尾为 Cancelled（现状行为）。
5. `daemon restart` 默认 drain 后换新；`daemon restart --force` 立即换新。
6. `install.sh` 在 Daemon 有在途请求时输出提示文案，安装流程照常完成，不强杀。
7. 无在途请求时：升级后下一次提问立即换新成功（与现状等同）。
8. 回归：单 Agent 正常 ask（弹窗 / IM / 文本回退）、`daemon status/logs`、空闲退出、`Detect` 全部不变。
10. 两份内容不同、协议号相同的二进制 A、B 轮流当客户端：A→B、B→A 两次让位照常发生；之后再来一个 B 时 Daemon 不让位，`daemon.log` 出现 `restart hold` 行，`daemon status` 显示 `binary` 行与 hold 警告，B 的请求照常受理、不触发排空。
11. hold 期间，没有 hold 逻辑的旧版客户端（同协议号）握手与 `daemon status` 照常工作（新字段对旧端被忽略）。
12. Daemon 自身盘上二进制被换：照常排空 / 换新，`daemon.log` 写 `binary on disk changed; draining for restart (exe=… fp x -> y)`，历史记 `trigger: disk`，不受 hold 影响。
13. Daemon 自行退出后，10 分钟窗口内新起的 Daemon 对第一个内容不同的客户端直接 hold；`daemon restart` 清掉历史后，同一对二进制重新只发生一次让位。
14. 连续安装多个新构建（每次内容都是新的）永远不触发 hold。
15. `doctor install` 在真实环境里指出 PATH 遮蔽；多个入口指向同一个文件（如 `~/.local/bin/AskHuman` 链到 npm 包内二进制）时不报警；`--warn-only` 无冲突时无输出；Dev Instance 里跳过 PATH 比对但仍报告 hold 与换新记录；`install.sh` 末尾调用它。

## 6. 多份二进制互相顶替（2026-10-22 补充）

**现象**：同一台机器上装了两份内容不同的 AskHuman（常见：npm 全局安装与 `install.sh` 本地构建，或别的工具捆绑的旧 CLI）。握手只比内容指纹（`daemon/lifecycle.rs`），两份二进制的客户端轮流连接时，Daemon 每次都判定「过时」而让位，客户端再用自己的二进制拉起新 Daemon，另一份的客户端稍后又来一遍：IM 长连接反复重连，新提问被排空挡住，一方的提问要等另一方作答。`daemon.log` 里实测出现过同一协议号下十几次来回换新，期间 IPC 类型并没有变化。

**为什么不能只靠提示**：升级（盘上二进制变了）与「两份二进制并存」在握手上是同一件事——客户端指纹 ≠ 自己指纹。区别只能靠历史判断：升级是一次性的，每次都是新内容；并存则是同一对内容来回出现。

**判定（D10）**：Hello 发现过时时先分类——自身盘上二进制变了 → 升级，让位；协议号不同 → 不可共存（D11），让位；否则是「内容不同的客户端」：在换新历史里数这一对指纹（任一方向）10 分钟内的互换次数，已有 2 次就 hold。让位前把原因、客户端和 Daemon 的信息写进 `daemon.log` 与历史文件（D12）。

**hold 之后**：同号客户端共用当前 Daemon，各自的二进制不再触发换新。代价是客户端与 Daemon 的实现版本可能不同，所以 `PROTOCOL_VERSION` 的纪律成为硬约束（D11）。hold 在 30 分钟内没有内容不同的 Hello 后自行结束；Daemon 自己退出时内存状态消失，但 10 分钟窗口内新起的 Daemon 会按历史立刻重新 hold；`daemon stop` / `restart` 是人为重置，会清掉历史。

**不在范围内**：hooks / MCP 配置里写死的 AskHuman 路径与 Daemon 实际二进制的比对；托盘 / 菜单栏警告；Windows 安装脚本。
