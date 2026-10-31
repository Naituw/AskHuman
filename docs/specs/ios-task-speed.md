# iOS 新建 Codex 任务的 Fast 模式

2026-10-31 用户确认并要求实施。入口仅限 iOS 新建任务，同时覆盖 Codex 桌面 App 和终端 CLI。
关联：[模型与强度](ios-task-model.md)、[实施与验证](../plans/ios-task-speed.md)。

## 交互

模型与强度下方增加 Fast 模式开关，每次新建默认关闭，关闭明确表示标准速度。速度不保存为手机偏好，
也不改写 Mac 全局配置；模型与强度仍按原规则记忆。说明为「Fast 更快，但消耗更多额度」，无需额外确认。

同一表单切换到仍支持 Fast 的模型时保留选择；新模型不支持或模型消失时关闭并提示。
切换 Mac 或 Agent 时关闭，切回 Codex 后也需重新开启。同一任务因网络问题重发时保留原值和原消息 ID。

仅模型目录明确声明支持时允许开启。模型选「默认」时，现有清单不能证明目标工作区实际默认模型的速度能力，
因此提示先选具体模型，不根据全局配置的显示标签猜测。Mac 未提供速度能力时隐藏开关并说明需要更新，保持旧行为；
此时不能保证覆盖 Mac 已有的 Fast 配置。

## 发现与校验

清单取当前启动路径对应的 Codex 可执行文件。优先解析逐模型 `serviceTiers` 的 `fast`／`priority`；
新字段不存在时兼容旧 `additionalSpeedTiers`，新字段为空时不回退旧字段。Ultrafast 和未知档位不作为 Fast。
条目没有任何速度元数据时不声明速度控制能力。

Mac 在启动前对当前可执行文件、工作区读取 `config/read` 并重新获取目录；明确关闭 `features.fast_mode`
或所选模型不再支持 Fast 时返回 `speedUnavailable`，不创建任务、不静默改用标准。桌面创建时再校验模型与速度。
清单与配置读取不发送提示词，不创建线程。

## 数据与执行

| 位置 | 新增可选字段 | 语义 |
|---|---|---|
| `task.options.agents[]` | `speedControl: true` | Mac 可显式控制此启动路径的速度 |
| `models[]` | `fastSupported: Bool` | 此模型是否明确支持 Fast |
| `defaultModel` | `fastSupported: Bool?` | 预留可靠的默认模型能力；当前不推断、不发送 |
| `task.create` | `speed: "standard" \| "fast"` | 显式标准／Fast；缺省沿用旧行为 |
| `watch.frame.frame` | `speed: "standard" \| "fast"` | 仅来自会话已报告的速度配置 |

支持速度控制的新版手机即使开关关闭，也提交 `standard`。共享 `ModelChoice.speed` 贯穿 `TaskSpec`、
`LaunchRecord`、桌面 `Operation::Create` 与创建快照。只接受固定枚举，非 Codex 不接受独立速度。

CLI 在提示词前传固定 argv：标准为 `-c 'service_tier="default"'`，Fast 为 `-c 'service_tier="fast"'`。
桌面 `thread/start.serviceTier` 与接管首轮 `turn/start.serviceTier` 使用同一创建快照，分别为 `default`／`fast`。
重试复用快照，不能重新继承改变后的 Mac 配置。速度不改变模型或思考强度。

速度参与任务幂等身份、一次性记录的完整性哈希与桌面操作身份。缺少速度字段时维持原哈希及序列化；
不带速度的旧 App、IM、Mac GUI、续聊与 Fork 保持原有继承行为。仅增加可选字段，不新增消息变体，
iOS 线协议版本及本机 IPC 协议号不变。

## 展示

进度页显示「所选速度：标准／Fast」。详情页只有会话最新 turn 明确报告速度配置时显示「速度配置」。
Codex 原生 `turn_context.payload.service_tier` 或桌面最新 turn 的 `params.serviceTierForTurn`／`serviceTier`
可以提供证据；缺省、未知档位或后续 turn 缺失字段时不沿用旧值。创建回执、所选值与全局配置均不能替代证据。
此显示不承诺服务端实际处理档位或吞吐速度。

## 验收

- 默认关闭；关闭传显式标准，即使 Mac 已全局开启 Fast；只选速度时也必须生效。
- Fast 不持久化；模型／强度仍记忆；切换模型、Agent、Mac 与能力刷新按交互规则复位。
- 不支持模型、未知档位及非 Codex 请求被拒绝；过期 Fast 不静默降级。
- 线程创建与 App 首轮接管携带一致速度；断线重发保持参数，改变速度但复用消息 ID 必须冲突。
- 旧消息、旧记录、旧哈希及没有新能力的 Mac 保持兼容；Rust／Swift 共用夹具覆盖新字段。
- Rust 启动参数、校验、网关与桌面测试，Swift 状态／偏好／重试测试，中英文模拟器界面测试。
- 修改后安装 Mac 构建；真实模型调用与 TestFlight 分发按用户授权另行执行。

## 资料

[Codex 速度说明](https://learn.chatgpt.com/docs/agent-configuration/speed)解释 Fast 与额外用量；
[配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)说明 `service_tier` 及 `fast_mode`。
本机离线协议与空凭据 `config/read` 已确认 CLI 0.163.0、桌面内置 0.164.0-alpha.11.1 接受 `default`／`fast`。
