# 正式实例的账户主目录路由

2026-11-09，用户选择在主工作树实施，并批准同一候选／发布二进制提供显式内部 System fixture。
本文补充 [项目级开发验收](dev-verification-routing.md) 和 [Dev Instance 并行开发](dev-instance-parallel.md)。

## 1 问题与调查证据

Cursor 为其他工具设置临时 `HOME` 后，普通 AskHuman 找不到该目录下的 socket，另起 daemon，问答和历史进入临时目录。
用户交接记录了两套实例使用相同的正式二进制。调查基线为 main `00c8d704`、已安装 0.17.1、IPC 5。
对同一个 `/Users/wutian/.local/bin/AskHuman`，只读查询得到以下结果。

| 环境 | 普通及显式 System 的 `dev target` | `daemon status` |
| --- | --- | --- |
| 正常 HOME | `/Users/wutian/.askhuman` | 日常 daemon 正常运行 |
| 临时 HOME | 临时目录下的 `.askhuman` | 找不到 daemon |

旧 `paths::config_dir()` 与 `development::state::system_home()` 间接依赖环境 HOME。
旧 `routing::apply()` 删除 System 的 `ASKHUMAN_HOME` 后，路径又从 HOME 重算。
macOS 原有账户查询只用于 launchd 标签，Node 的 `os.homedir()` 也受临时 HOME 影响。

## 2 已定不变量

- 新的普通 System 与显式 `--instance system` 使用操作系统账户根。
- `Target.home` 统一决定配置、状态、历史、待办、恢复存储、socket 与锁的数据根。
- 保留 System、Dev、Isolated 三种 TargetKind 及现有入口、普通 MCP 六工具、项目 `dev_ask`、profile 和远程转送归属。
- fixture 固定测试账户上下文，fixture System 覆盖 System relay 和同步路径。
- fixture 全链路禁用钥匙串，使用私有启动身份，阻止共享登录项、Hooks 和同步 helper 注册等自动写入。
- 旧 v1 System 的非账户根通过继承目标原地续接，配置与共享凭据只读。

## 3 账户、集成和调用方路径

Unix 使用 `getpwuid_r(getuid())`，ERANGE 时扩容，并处理无记录、空／相对目录和不可序列化路径。
Windows 使用账户 Profile Known Folder。Rust 和 Node 在业务配置初始化前明确报告账户查询错误。
Node 使用 `os.userInfo().homedir`。显式 Isolated 的基本读写、状态和启动可独立于账户查询，
访问正式预设、正式转送等账户资源时再要求解析成功。

| 用途 | 目录依据 |
| --- | --- |
| 新 System | 真实 account home / `.askhuman` |
| fixture System | 验证后的 fixture account home / `.askhuman` |
| Dev | 工作树 `.askhuman-dev/home` |
| 显式 Isolated | 既有 ASKHUMAN_HOME 规则，保留相对路径解析 |
| System Agent 产物与旧 `.humaninloop` 回退 | 同一 System account anchor |
| 正式预设与 relay state | 已选正式账户根 |
| Dev 私有 profile | `session::directory_for()` 优先 |
| Isolated 显式 Agent 集成 | caller HOME/profile |
| 前台输入 `~`、附件与输入文件 | caller HOME |
| Node transcript monitor | 清理前的 CODEX_HOME，否则 caller HOME |

`integration_home()` 负责 Cursor、Claude、Codex、Grok、Pi、Antigravity 集成目录 fallback、路径展示及 onboarding 文件预览。
Node guard 读取 System anchor 下的偏好和标记，monitor 跟踪原会话。
System daemon、GUI Host、Popup、同步 tick、relay 与更新角色把后台 HOME 固定为选定账户 anchor，
覆盖直接后台入口、launchd、detach、Windows 子进程和更新接班。

前台 CLI、权限 hook、Cursor CLI 包装器与原生 profile 保留调用方 HOME。
用户路径展开、原生授权规则目录、保护路径算法、Cursor 显式目录优先级及数据库候选顺序沿用既有规则。

## 4 fixture v2 的入口与继承

### 4.1 首次 Rust 入口

精确能力查询 `AskHuman --development-capabilities` 增加 `systemFixture=2`，保留 `developmentVerification=2`。
查询在配置、钥匙串和业务状态初始化前返回。
首次 fixture 调用使用以下完整形式。

```text
AskHuman --development-capabilities --internal-system-fixture-v2 /absolute/test-account -- <现有命令及参数>
```

新程序在能力查询的提前返回前识别并验证完整组合，再按现有规则选路。
缺目录、缺 `--`、空命令或畸形组合立即失败。
旧程序遇已知的第一个 capability flag 就返回能力，避免未知 flag 触发 Lang 的配置读取和权限收紧。

目标验证后、业务分发或 reexec 前，首次受保护调用向 stderr 输出一行回执。

```text
ASKHUMAN_SYSTEM_FIXTURE_V2_ACTIVE {"version":2,"accountHome":"/absolute/test-account","kind":"system","home":"/absolute/test-account/.askhuman"}
```

kind/home 描述此时实际选中的目标。`dev` 管理入口先验证并 apply bootstrap 目标，随后由管理命令绑定本树数据。
继承子进程省略回执。harness 必须解析回执，核对账户、目标及退出状态，长期角色还需核对私有 endpoint 就绪。

### 4.2 target 封套

继续使用旧端认识的环境键 `ASKHUMAN_RESOLVED_TARGET_V1`，fixture 值使用 version 2。

```json
{
  "version": 2,
  "kind": "system",
  "binary": "/absolute/AskHuman",
  "home": "/absolute/test-account/.askhuman",
  "worktree": null,
  "reason": "explicit system fixture",
  "noKeychain": true,
  "fixture": { "version": 2, "accountHome": "/absolute/test-account" }
}
```

旧端拒绝 version 2。新端拒绝 v1 携带任何 fixture 字段、v2 缺 fixture、未知版本、相对路径和显式／继承上下文冲突。
fixture 内的 Dev、Isolated、profile 与管理目标始终携带同一 fixture，普通目标继续使用 v1。

### 4.3 路径与共享写入

account home 必须已存在、为绝对目录、属于当前用户且其他用户不可写。
先按物理路径规范化，允许 `/tmp` 这类父路径别名。
fixture 账户与数据根须避开真实账户及生产数据，`.askhuman` 必须为私有目录，拒绝 symlink／junction／reparse 重定向。
外部 Isolated／管理目标还检查最近已存在祖先，防止缺失子目录通过链接落入生产根。
Swift 使用 POSIX `realpath` 与最近已存在祖先解析，保证 `/tmp`、`/private/tmp` 和尚未创建的文件有同一身份。
受管 Agent 写入也检查物理祖先归属。

fixture 使用私有 launchd 标签、socket、锁和 GUI Host 实例。
共享登录项、同步 helper 安装注册及自动 Agent 集成写入由既有实例守卫保护。
显式私有集成写入限制在 fixture 账户或有效测试 profile 内。

### 4.4 执行与更新边界

fixture 跨二进制执行前要求 `systemFixture=2`，覆盖 routing reexec、旧 enabled 选路、Dev worker、
relay 预检／发送／取消、profile、daemon、GUI Host、Popup、更新 worker 和同步 tick。
v2 封套继续防止探测后替换成旧程序而丢失上下文。能力失败时保持 fixture 并返回错误，禁止清除身份后降级重试。

fixture daemon 遇不支持的更新候选时继续服务。
检查覆盖 disk/Hello 更新、live-update、最终 shutdown/handoff 和 GUI Host 释放锁之前。
按内容指纹缓存失败，候选变化后重试，显式 stop 保留优先级。

## 5 Node controller、worker 和 enable bootstrap

```text
node scripts/dev.mjs --internal-system-fixture-v2 /absolute/test-account <现有操作及参数>
```

fixture workspace manifest 使用 version 2 与 fixture 字段，普通 manifest 使用 version 1。
新 controller 要求显式前缀、继承上下文和 manifest 匹配，继承 fixture 而缺前缀时在清理环境前拒绝。
精确能力查询提前返回。旧 controller 拒绝未知前缀或 v2 manifest。
fixture 项目 MCP 配置携带前缀，生成前先对 controller 做限时能力探测。

`runWorker()` 验证 manifest 与私有 binary/home，再构造本树 Dev target。
`workerEnvironment()` 清除日常身份和 provider 环境后，只重新加入验证后的 target、no-keychain 与必要 driver 信息。
取消的 `__dev-interrupt` 复用本次 Dev target。

首次 `dev enable` 使用现有正式／候选程序执行受保护入口，bootstrap 先固定并验证数据目标。`dev target` 保留诊断选路，其余管理命令使用 bootstrap 目标。
`ensure_layout()` 创建本树布局，`pin_management()` 选择本树 Isolated 管理目标，binary 保留当前执行程序。

## 6 旧 v1 System 的原地续接

旧 daemon 执行 `execve(..., ["daemon", "run"])` 时，新映像继承原 v1 target。
仅接受旧生成形状，包括 System、无 fixture、绝对 binary/home、worktree=null、home 末级为 `.askhuman`。
与真实账户根不同的目标进入内部 `legacy-system-home` 兼容上下文，TargetKind 保持 System。
账户查询失败时也保守启用只读保护。

- 原 Target.home、socket、锁、恢复数据库、检查点、iOS 状态与 GUI endpoint 保持原地。
- anchor 取原 home 路径的 parent，后台 HOME 和后续 v1 继承保留原上下文。
- 既有 CLI/MCP/窗口可继续重连，新裸调用选择真实账户根。
- 旧 recovery probe 早于 handoff record 写入，续接因此只依赖可兼容的继承目标形状。
- 保留 noKeychain。既有钥匙串读取可以恢复，迁移、set/delete 和共享注册均受阻止。
- `AppConfig::load()` 跳过 secret migration 和权限收紧，save 在持久化凭据前明确拒绝。
- config/channel/settings、新配对、显式 revoke/rename 及导入密钥在副作用前拒绝，避免失败后回退明文文件。
- 历史、回答、journal、回执、恢复与业务运行状态继续保存。
- 运行期失效配对可清理旧根本地状态，钥匙串删除仍受保护。

诊断显示 legacy 标记与原根。完成原调用后可用单次 `ASKHUMAN_HOME=/old/temp/.askhuman AskHuman daemon status|stop` 管理，
也可显式读取旧历史。旧数据的处置由用户决定。

## 7 同步 helper 的 fixture 契约

### 7.1 启动、路径与注册

fixture System 保留同步核心能力。Rust 共享 installer prepare/register/stop/repair 明确拒绝，maintain 跳过共享安装维护。
显式私有 app 路径须通过能力探测，fixture enable 再验证 socket 来源及同账户身份。

Swift Kit 提供纯参数解析，App 在 NSApplication、日志、CloudKit 和 LoginItem 初始化前验证。
精确 `--version` 只返回 version 与 systemFixture，fixture 唯一运行形式如下。

```text
AskHumanTodoSync --version --internal-system-fixture-v2 /absolute/test-account run
```

旧 helper 只处理已知 `--version` 后退出。新 helper 拒绝缺／多参数、尾项 register/unregister 和上下文冲突。
fixture 管理命令在访问 SMAppService 前返回 unmanaged，register/unregister 返回失败。
state 固定为 `<accountHome>/.askhuman/state/todo-sync`，日志为该目录的 `helper.log`。
非空 ASKHUMAN_TODO_SYNC_DIR 必须一致，启动器核对私有 socket 的实际身份。

### 7.2 自动 tick 与旧 helper

TickProcessRunner 检查 config 的 stateDir/todosPath 归属及绝对可执行路径。
每次 tick 前清除 target 和 Dev/profile 上下文做能力探测，要求 systemFixture=2。
探测限时、限制输出并清理私有输出临时文件，实际 tick 注入 System v2、固定 home、binary 和 noKeychain。

已经运行的旧 HOME-only helper 会从稳定路径启动新版核心，因此 Rust bootstrap 在配置读取、apply、reexec 与 HOME 归一前检查以下条件。

- helper-origin 的实际 todo-sync 调用，其非空 sidecar 必须为绝对路径且等于所选 Target.home/state/todo-sync。
- 缺少有效继承 target 和显式 fixture 时，原始 HOME 如存在必须指向真实账户，否则明确提示停止旧私有 helper 并以 guarded 入口重启。
- 精确能力查询保持提前返回，允许 probe 清除 target 后继续携带 fixture HOME。

fixture 沿用现有 CloudKit bundle id、entitlements 和容器。真实 iCloud、删除 zone 和双 Mac 操作沿用各自授权约定。

### 7.3 hello 与显式传输桥

hello 增加可选 `systemFixture`，普通响应省略，RPC 保持 1。
fixture core→helper 握手验证私有 v2 身份，并要求物理 socket 等于声明账户下的 helper.sock。
连接生产根 socket 会在发送 hello 前拒绝，重新连接时重新握手。

显式 B/helper.sock→A/helper.sock 桥可用于手动 tick，调用方数据仍来自自己的 Target.home。
enable 另要求 helper 与调用方同账户，保证自动调度服务正确实例。
Swift 自动 tick 固定自己的 context/config，helper 自身的 state/socket 拒绝符号链接。

单机同步 E2E 改为 A、B 两个私有 helper，各自 enable，B 随后通过 helper.json 关闭自动调度并等待已开始的 tick 完成。
B 核心由手动 tick 驱动，rebuild 恢复该条件，offline 停止／恢复两者，kill 场景只操作 B。
两个脚本在覆盖任何核心或 helper 二进制前，检查已有 peer 身份与进程／持锁者。
旧 peer、身份不符或仍活跃却无法 hello 时拒绝迁移，报告需停止的私有 PID／目录。
日志收集与清理限定在私有根。

## 8 peer probe 与 manifest 修复

fixture 出站 daemon 客户端在 Hello／Recover／业务消息前只读 Status，检查可选 systemFixture 的版本和账户。
relay 继续核对协议、实际指纹和渠道能力。GUI Host 保留 unit Ping 消息，响应增加可选身份，客户端先 probe 再发 OpenWindow/Shutdown。
普通 OpenWindow ack 与既有 raw IPC 消息保持兼容。默认可选字段保持 IPC 5，任务序列化哈希与 recovery schema 5 保持兼容。

manifest 读取拆分为结构／物理安全与当前 context binding 校验。
只有显式 dev enable 可在项目锁内修复旧 v1 中单独漂移的 systemHome。
修复前验证 worktree、版本、manifest 链接边界、私有 bin/home，并拒绝活动 Agent session 或 verification executor。
保留有效 systemBinary，缺失或非法时提示显式 --system-binary。
备份原 manifest 后原子写入新根，按完整 manifest 比较，保留私有数据与 profile，再次 enable 幂等。
fixture v2 与普通 v1 的绑定必须显式匹配。

## 9 实现地图

| 模块 | 职责 |
| --- | --- |
| paths/account.rs、paths.rs | OS 账户查询、fixture 物理路径验证、integration home |
| development/routing.rs、state.rs、workspace.rs、project.rs | 固定 target、guarded bootstrap、legacy、能力门槛、manifest 修复 |
| scripts/development/context.mjs、runtime.mjs、caller.mjs、mcp.mjs | Node 账户与 fixture、Dev target 注入、guard/monitor 分离 |
| client、daemon、gui_host、update、executable、mcp | 出站 peer probe、后台环境、更新前门槛 |
| config、secrets、integrations、onboarding、ios pairing | legacy 配置／凭据只读与私有集成写入边界 |
| todo_sync、Swift Kit/App、同步脚本 | helper 入口、身份、日志、tick 与双私有进程测试 |

## 10 验证范围

本机验证使用临时目录、loopback 渠道和替身，无真实模型调用。
`system-home-routing-smoke.mjs` 覆盖临时／空／缺失 HOME、旧 helper、fixture 冷启动、生产别名拒绝、
真实旧版 guarded 入口及 v2 拒绝、旧临时 System 在途 A 原地 exec 后答复与新题 B，以及拒绝旧候选后接受兼容换新。
`dev-routing-smoke.mjs` 覆盖两个 Dev、普通→Dev→普通、旧 marker、profile 和 MCP 归属。
`dev-relay-smoke.mjs` 覆盖项目 MCP、转送问答、取消、权限／Stop／待办归属及失败收尾。
已有显式 ASKHUMAN_HOME 的隔离与故障脚本继续使用原入口。

## 11 当前状态

2026-11-09 在主工作树完成实现与 Astra xhigh 复审，复审无剩余 P1／P2。
最终候选通过 `./scripts/install.sh` 编译、签名并安装到 `/Users/wutian/.local/bin/AskHuman`。
日常 daemon 在原 PID 与账户 socket 处完成 exec 接班，四条 IM 连接正常。

| 验证 | 结果 |
| --- | --- |
| Rust 全量，local-install、custom-protocol、2 test threads | 2920 passed，0 failed，11 ignored |
| clippy，all-targets、local-install、custom-protocol、`-D warnings` | 通过，vendor/tao 保留既有依赖警告 |
| Node 全量，`scripts/*.node-test.mjs` | 208 passed，0 failed，21 skipped |
| Swift Kit，`swift test --jobs 1` | 69 passed，0 failed |
| helper App，unsigned Debug xcodebuild | BUILD SUCCEEDED |
| System 真实进程 smoke | 通过，含真实旧 0.17.1 在途接班与旧候选拒绝 |
| Dev 路由与 relay smoke | 通过，含两个 Dev、旧 marker、项目 MCP 和动作归属 |
| 显式 ASKHUMAN_HOME 接班 smoke | 通过，含在途问答、权限确认、停止优先及失败候选恢复 |
| 本地 helper 实进程 | 通过 guarded 启动、物理身份、缺失 todos 时自动 tick、私有日志和 unmanaged 注册命令 |

本地 helper 使用无 CloudKit entitlement 的构建和 tick 替身，安装后的 Rust 核心可读取其运行状态。
本机检查使用 Homebrew Rust/Cargo 1.94.1、Node 24.2.0，构建按机器健康检查结果以单 job 后台执行。
Windows／Linux 原生路径尚待验证，具体步骤保留在 PROGRESS。
真实 iCloud、双 Mac 同步脚本和 GUI Host 实窗冷启动未在本轮执行。
同步脚本已完成语法检查与 fixture 入口迁移，本机 helper 已验证启动与自动 tick 边界。
