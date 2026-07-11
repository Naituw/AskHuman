# Agent 权限请求经本地弹窗 / IM 审批

> 状态：计划已确认（2026-07-11）  
> 范围：macOS / Linux；Claude Code + Codex 原生 `PermissionRequest` 闭环  
> 实现前提：不改变 Agent 自身权限规则，只代答原本即将出现的单次权限弹窗
> 前置任务：`docs/plans/codex-mcp-blocking.md` 已独立完成；本计划不包含通用 MCP 阻塞行为

## 1. 目标

当 Claude Code 或 Codex 准备弹出权限确认时，由用户级 `PermissionRequest` Hook 把请求交给
AskHuman。AskHuman 同时投放到本地弹窗和当前应接收提问的 IM 渠道，沿用现有首答胜出、落败端收尾、
IM 按需发送与在途补推机制。用户只做两个决定：

- **批准一次**：Hook 返回 `allow`，只批准当前权限请求；
- **拒绝**：Hook 返回 `deny`，拒绝当前权限请求。

用户明确关闭本地确认窗等同“拒绝”。Daemon、弹窗、网络或渠道等基础设施故障，以及等待 24 小时
仍无人处理，均不替用户作决定：Hook 不输出裁决，让 Agent 回到自己的原生权限弹窗。

本需求还要沉淀一套可复用的“双动作确认”交互，而不是在权限功能里复制 `/stage` 或普通 Ask 的逻辑。
首期只接权限审批，未来脚本确认、危险操作确认等可复用同一请求模型和抢答通道。

## 2. 已确认的产品决策

| 决策 | 定案 |
|---|---|
| Agent 范围 | 只做有原生闭环的 Claude Code、Codex；Cursor、Grok 不做替代权限系统 |
| 动作 | 仅“批准一次 / 拒绝”；不做“始终允许”、权限规则写入或输入改写 |
| 抢答范围 | 本地弹窗 + 现有 Ask 会投放的 IM；首个有效动作胜出，其余端定格收尾 |
| IM 按需发送 | 完全沿用 `auto_activation`、当前活跃槽、watch 关联渠道和在途补推 |
| 关闭语义 | 用户明确关闭本地弹窗 = 拒绝 |
| 故障语义 | 基础设施故障 = 不裁决，回 Agent 原生权限弹窗 |
| 等待上限 | 24 小时；到期定格为“已过期”，随后回原生权限弹窗 |
| 历史 | 不写 AskHuman 回复历史，避免命令、路径、MCP 参数长期落入 `history.jsonl` |
| 详情 | 优先提取人能读懂的工具摘要；内容超长时明确标记截断，但仍允许用户远程批准 |
| 设置入口 | 复用每家 Agent 自动集成卡的 Hook 产物状态；不新增权限开关 |
| 生效关系 | Claude/Codex 选择 CLI 或 MCP 时一并安装；选择“未集成”时卸载；与生命周期追踪无关 |
| 旧安装升级 | 已处于 CLI/MCP 的旧用户显示现有“需更新 / 更新”按钮，点击后补齐 PermissionRequest Hook；不静默迁移 |
| 平台 | 首期 macOS/Linux；Windows 等 named-pipe Daemon 完成后再接入相同语义 |

## 3. 四家能力结论

调研只判断“Hook 能否在原生权限弹窗出现前阻塞，并把人的决定同步返回 Agent”，不把
`PreToolUse deny`、自动运行或修改 Agent 权限策略当作同等能力。

| Agent | 原生事件 | Hook 可返回批准 | Hook 可返回拒绝 | 首期 |
|---|---:|---:|---:|---:|
| Claude Code | `PermissionRequest` | 是，`decision.behavior=allow` | 是，`decision.behavior=deny` | 支持 |
| Codex | `PermissionRequest` | 是，`decision.behavior=allow` | 是，`decision.behavior=deny` | 支持 |
| Cursor | 无可代答原生权限弹窗的事件 | 否；`preToolUse allow` 后仍会进入 Cursor 权限服务 | 只能预先 deny 工具 | 不支持 |
| Grok | 无用户级 `PermissionRequest` | 否；`PreToolUse allow` 不等于批准 | 可 deny，但失败/超时 fail-open | 不支持 |

证据口径：

- Claude Code：官方 [Hooks reference](https://code.claude.com/docs/en/hooks)；`PermissionRequest`
  在权限对话框即将显示时触发，可返回 allow/deny，command Hook 默认 600 秒、可配置超时。
- Codex：官方 Hook 文档与本机 Codex Rust `0.144.1` 源码
  `codex-rs/hooks/src/events/permission_request.rs`、`schema.rs`；输入含 `tool_name`、`tool_input`、
  `cwd`，输出支持 allow/deny，当前不接受 `updatedInput`、`updatedPermissions` 或 `interrupt`。
- Cursor：本机 Cursor `3.7.36` 的 `cursor-agent-exec` bundle；Claude 兼容映射中
  `PermissionRequest` 为 `null`，`preToolUse` 的 `ask` 未实现，allow 仍进入本地权限服务。
- Grok：本机 Grok `0.2.93` 的 `10-hooks.md`、`22-permissions-and-safety.md`；用户 Hook 列表没有
  `PermissionRequest`，PreToolUse 只能 deny，Hook 超时/失败 fail-open。

## 4. 非目标

- 不为 Cursor/Grok 打开 Auto-run、Always Allow 或 `failClosed`，再用 AskHuman 自建替代权限系统；
- 不批准一类命令、不写“始终允许”规则、不消费 Claude 的 `permission_suggestions`；
- 不修改工具输入，不支持 Codex 当前保留的 `updatedInput` / `updatedPermissions`；
- 不绕过 Agent 的 deny、ask、managed policy 或 sandbox 规则；其它规则仍可在 Hook allow 后拒绝/再询问；
- 不把权限详情写入回复历史，不在首期新增持久化审计日志；
- 不提供“只关闭权限确认、保留其它 AskHuman 集成”的独立开关；三态模式是唯一启停入口；
- 不把通用确认开放为新的公开 CLI 参数；先稳定内部请求契约，公开脚本接口另立需求；
- 不在首期提供 Windows 行为不一致的“全 IM 群发”简化版。

## 5. 总体设计

### 5.1 双动作确认是独立交互类型

不要把“批准一次 / 拒绝”伪装成普通选项文本后再按本地化字符串反解结果。新增稳定、机器可读的确认模型：

```rust
ConfirmRequest {
    id,
    title,
    body_md,
    details,
    confirm: ConfirmAction { id, label, role },
    cancel:  ConfirmAction { id, label, role },
    dismiss_action_id,
    record_history,
    expires_at_ms,
}

ConfirmResult {
    action_id,
    source_channel_id,
}
```

权限请求固定映射：

- `confirm.id = "approve_once"`，视觉角色为 primary；
- `cancel.id = "deny"`，视觉角色为 destructive/negative；
- `dismiss_action_id = "deny"`；
- `record_history = false`；
- `expires_at_ms = created_at + 24h`。

普通 `AskRequest` / `ChannelResult` 的公开 CLI 输出和历史格式保持不变。Daemon 内部把两类请求包装为
`InteractionRequest::{Ask, Confirm}`、结果包装为 `InteractionResult::{Ask, Confirm}`，共享请求登记、首答协调、
弹窗关联、渠道挂接、取消和收尾；只有渲染与终态回传分支不同。

### 5.2 IPC 使用结构化确认终态

新增专用 IPC 负载，而不是让权限 Hook 解析普通 AskHuman stdout：

```text
ClientMsg::SubmitConfirm(ConfirmTask)
ServerMsg::ConfirmAccepted { request_id }
ServerMsg::ConfirmFinal { action_id, source_channel_id }
ServerMsg::ConfirmFallback { reason }
```

- `ConfirmFinal` 只代表人明确作出的批准或拒绝；
- `ConfirmFallback` 代表到期、无可用渠道、GUI 异常退出、Daemon 排空/换新等无法取得人的决定；
- Hook 客户端遇到连接失败、协议错误或 `ConfirmFallback` 都必须安静退出 0 且 stdout 为空；
- Hook 被 Agent 杀死或 stdin 非法时同样不输出裁决；
- IPC 增量保持 serde 向后兼容；若实际改动使同一连接上的旧新端无法安全互通，再提升
  `PROTOCOL_VERSION`，不能依靠“通常同版本”掩盖不兼容。

### 5.3 协调器区分“人拒绝”和“系统失败”

现有 GUI Helper EOF 会被当作 popup cancel。确认请求不能照搬：

- 前端收到用户关窗事件时，显式发送 `deny`；
- 前端点击“拒绝”同样发送 `deny`；
- Helper 崩溃、连接断开或窗口未成功建立属于通道失败，不得合成 `deny`；
- 如果仍有 IM 渠道，继续等其它端；如果所有渠道均失败，返回 `ConfirmFallback`；
- 24 小时定时器与用户动作进入同一原子终态闸门，谁先提交谁生效；过期后的迟到点击只做幂等收尾。

普通 Ask 的“用户关窗 = cancel”语义不变。

### 5.4 复用 Ask 的投放与按需补推

`RequestRegistry` 的在途条目改为可携带 Ask 或 Confirm，以下机制按交互类型无差别工作：

1. 提交时立即分配 request id，优先领用预热本地弹窗；
2. `attach_im_channels` 根据现有配置投放：
   - `auto_activation=false`：所有已启用且可用 IM；
   - `auto_activation=true`：当前活跃槽，加上正在 watch 本次 agent session 的渠道；
3. 用户在另一 IM 执行 `/here` 或其它激活动作时，`backfill_inflight` 也补推尚未答复的确认卡；
4. 首答胜出后更新活跃槽，与普通 Ask 一致；
5. 其它弹窗/卡片定格为“已通过 X 批准”或“已通过 X 拒绝”，不能继续点击。

确认卡出现同样算非 watch 扰动；收尾后沿用现有 watch 跟底恢复逻辑。

### 5.5 通用确认渲染

扩展现有 `confirm.rs` / 四渠道 confirm builder，使其接收通用 `ConfirmView`，但不复用 `/stage` 的业务台账。
`/stage` 仍负责 git 指纹校验和执行；它与权限确认只共享展示模型、双按钮卡片构建和终态样式。

- 本地弹窗：`PopupView` 按 `InteractionRequest` 分流，确认视图隐藏普通题目导航、自由输入、附件、语音和
  “提交”按钮，直接显示详情和“拒绝 / 批准一次”；窗口关闭显式发送 `deny`。
- 飞书：复用交互卡双按钮与回卡终态，回调按 request/message id 路由。
- 钉钉：复用 confirm 卡模板的 `confirm_ok` / `confirm_cancel`，会话层映射为稳定 action id。
- Telegram：复用 inline keyboard；callback_data 带确认请求身份或由 message id 路由，避免并发串卡。
- Slack：复用 Block Kit actions；按 message ts + action id 路由并即时 ack。

所有渠道必须显示：Agent、workspace、工具名、权限模式、可读摘要、创建时间。终态卡不保留可点击按钮。

### 5.6 工具详情归一化与截断

Hook 输入不直接原样拼成卡片。先把 `tool_name + tool_input` 归一化为结构化详情：

- Bash / shell：完整命令优先，其次 description；
- Edit / Write / apply_patch：目标路径、操作类型，以及渠道容量允许的内容摘要；
- MCP：server、tool、参数 JSON；
- 其它已知工具：提取路径、URL、查询或目标等稳定字段；
- 未知工具：pretty JSON 兜底。

安全与容量规则：

- stdin 和单字段都设硬上限，先拒绝异常大载荷，避免 Hook/Daemon 内存放大；异常时回原生弹窗；
- 本地弹窗在总上限内展示完整归一化详情；
- IM 按各平台卡片限制截断，保留开头和结尾并明确显示“内容已截断”；
- 按用户定案，即使 IM 内容被截断仍保留“批准一次”按钮；
- JSON/Markdown/卡片字段必须按平台转义，Hook 输入永远不能注入 callback id、action id 或 Hook 输出结构；
- 不读取 `transcript_path` 内容，不上传 transcript；该字段只用于诊断，不进入卡片。

### 5.7 Hook 适配器

新增隐藏入口：

```text
AskHuman __permission-hook claude
AskHuman __permission-hook codex
```

入口从 stdin 读取一次原生 Hook JSON，验证 `hook_event_name == "PermissionRequest"`，构造通用确认任务，
阻塞等结构化终态，再输出对应 Agent 的 JSON。stdout 只能出现最终 JSON，所有诊断走 stderr；基础设施失败
应默认静默，避免 Agent 把诊断误当裁决。

批准一次：

```json
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}
```

拒绝：

```json
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"The user denied this permission request via AskHuman."}}}
```

两家当前格式同构，但代码仍按 adapter 分开，避免未来协议差异污染通用交互。Claude 的 allow 不能覆盖 deny/ask
规则；Codex 多 Hook 中任一 deny 仍优先。AskHuman 不声称“allow 一定执行”，只表示“用户批准本次请求”。

### 5.8 Hook 安装与共存

权限确认使用独立命令标记（例如 `__permission-hook`），但**不是独立安装项**；它属于现有
`agent_mode` 的 Hook 产物包，只触碰自己的配置条目：

- Claude：`~/.claude/settings.json` 的 `hooks.PermissionRequest`，nested shape，timeout 86400；
- Codex：`~/.codex/hooks.json` 的 `hooks.PermissionRequest`，nested shape，timeout 86400，并写
  `~/.codex/config.toml [hooks.state]` 信任哈希；
- Cursor/Grok：不写配置；status 返回 `supported=false` 和稳定原因码；
- Windows：四家 status 均对本功能返回暂不支持，避免生成行为不一致的 Hook。

模式与 Hook 产物包的关系：

| Agent / mode | Hook 产物包 |
|---|---|
| Claude CLI | 既有 CLI 超时 Hook + PermissionRequest Hook |
| Claude MCP | PermissionRequest Hook；不安装 CLI 超时 Hook |
| Codex CLI / MCP | PermissionRequest Hook |
| Cursor CLI | 既有 CLI 超时 Hook；无 PermissionRequest 能力 |
| Cursor MCP / Grok MCP | 无 Hook 产物 |
| 任一家 None | 卸载该自动集成模式拥有的 Hook；生命周期 Hook 仍按自己的开关管理 |

`Artifact::Hook` 从“CLI 超时 Hook”提升为“当前模式的 Hook 产物包”。`artifact_updates()` 在 Claude/Codex
的 CLI、MCP 两种模式下都检查 PermissionRequest 条目；因此升级前已集成的用户会自然得到现有
`hookNeedsUpdate=true`，设置页出现“更新”，`update_artifact(..., Hook)` 和“全部更新”补齐该条目。
`Mode::None` 仍返回全 false，不能因为发现旧权限条目而把未集成误判成 CLI。

权限条目必须与 lifecycle 的 `__agent-hook`、CLI timeout hook 和用户自有 Hook 共存。为此先从
`agent_lifecycle.rs` 抽出共享的 JSONC/TOML Hook 编辑器，并修正 Codex 当前“卸载 lifecycle 时按整个
hooks.json 路径删除全部信任项”的粒度：

1. 变更前从旧 hooks.json 找出 AskHuman 自有 marker 对应的 trust keys；
2. 原子写入新 hooks.json；
3. 在 config.toml 中只移除旧 AskHuman keys，再写入新文件中仍存在的 AskHuman handlers 的 hashes；
4. 保留用户/其它产品的 trust entries；
5. config.toml 更新失败时回滚 hooks.json，不能留下“Hook 新、信任旧”的半安装态；
6. lifecycle 或 agent_mode Hook 产物包任一安装、卸载、迁移后都走同一 reconcile；
7. 状态检查同时验证命令路径、事件、timeout 和 Codex trust hash；已安装但过期时显示更新。

这部分是权限功能安全共存的前置工作，不顺带改变 lifecycle 的产品行为。

### 5.9 设置页与现有更新机制

不新增 switch、Tauri command 或 config 布尔值。继续由现有 `agent_mode_status`、`artifact_updates`、
`agent_mode_update_artifact` 和 `agent_mode_update` 驱动：

- Claude CLI 的现有“Hook”行改为 Hook 产物包，说明同时包含“CLI 超时 + 权限确认”；
- Claude MCP、Codex CLI/MCP 新显示“Hook”产物行，说明其用途是权限确认；
- 已集成但缺 PermissionRequest 的旧安装，沿用现有橙色“更新”按钮；
- 自动集成待更新总览继续统计 `Hook ×N`，Agents Tab 提示点和“全部更新”无需另造状态机；
- 新切到 CLI/MCP 时，`agent_mode::set` 当场安装完整 Hook 包；切到“未集成”时卸载权限条目；
- Cursor CLI 仍显示超时 Hook，并补充“当前无原生权限确认 Hook”；Cursor MCP 与 Grok MCP 用说明文字
  标明权限确认不支持，不伪造 Hook 产物；
- Windows 上 Claude/Codex 的 Hook 产物判 unsupported，并说明“等待 Windows Daemon 支持”；
- 权限状态信息可以在 Agent 卡片中展示，但不能出现第二个启停入口，也不能把生命周期追踪开关当依赖。

为避免前端继续用 `timeoutHook*` 表达已经扩大的概念，Rust/TS 状态字段改为通用 `hookSupported`、
`hookInstalled`、`hookNeedsUpdate`；必要时用 capabilities/hint 区分 timeout 与 permission。旧字段仅是
Tauri 本进程调用契约，不持久化；同版本前后端一起升级。

## 6. 实施里程碑

### M0：通用确认模型与协议

1. `models.rs`：增加 `ConfirmRequest`、`ConfirmAction`、`ConfirmResult`、action role、终态/过期原因；
2. `ipc/mod.rs`：增加 Confirm task/client/server 消息和 GUI show/answer 负载；
3. `daemon/request.rs`：登记项支持 Ask/Confirm 两类 interaction；pending 摘要、agent session 关联保持通用；
4. `app/coordinator.rs`：抽出共享首答/收尾核心；Ask 分支保持现有 stdout/历史，Confirm 分支返回结构化结果且
   永不写 history；
5. 单测：首答唯一性、action id 校验、普通 Ask 输出/历史回归、Confirm 不落历史。

### M1：本地确认弹窗

1. `ShowPayload` / `popup_init` 支持 interaction enum，预热 helper 可领用任一类型；
2. `PopupView.vue` 增确认视图：元信息、详情、截断提示、两动作按钮；
3. 新增显式 `submit_confirm_action`；用户关窗走 `deny`，helper EOF 走通道失败；
4. 落败/过期/调用方取消时关闭窗口；不改变普通 Ask 的取消确认与关闭语义；
5. 前端测试或可测试纯函数覆盖类型分流、按钮映射、转义和截断标记。

### M2：Daemon 抢答、IM 卡与按需发送

1. `attach_im_channels`、`backfill_inflight`、watch 扰动/恢复改为接受通用 interaction；
2. `confirm.rs` 提升为通用 `ConfirmView`，四渠道 builder 接稳定动作映射；
3. 四个 channel adapter 实现 Confirm 的发送、回调、首答投递、落败定格和过期定格；
4. 24 小时 timer 与 action 争抢同一终态；全通道失败返回 fallback；
5. 成功动作后沿用 winner 更新 active channel；新激活 IM 能补推在途确认卡；
6. `/stage` 继续使用自己的业务台账，但迁到同一通用 view/builder，确保既有行为不回归；
7. 测试 popup/四 IM 的竞态、迟到点击、重复 callback、active/watch 投放和 `/here` 补推。

### M3：权限输入归一化与 Hook 运行器

1. 新建权限模块解析 Claude/Codex stdin，限制总大小，提取 session/cwd/tool/tool_input；
2. 实现 Bash、文件工具、MCP、unknown JSON 的摘要器和各渠道预算下的截断；
3. `client/` 增 `run_confirm`，基础设施失败/timeout/fallback 均返回“无裁决”；
4. `cli/mod.rs` 注册隐藏 `__permission-hook`，严格保持 stdout 洁净；
5. 两家 adapter 输出 allow/deny JSON；恶意引号、换行、超长 Unicode 必须由 serde 正确转义；
6. fake daemon 集成测试：approve、deny、user-close、helper-crash、daemon-loss、24h 虚拟时钟 timeout。

### M4：agent_mode Hook 产物包与 Codex trust 共存

1. 从 `agent_lifecycle.rs` 抽共享 JSONC Hook 编辑、marker 定位、atomic write；
2. 实现内部 `agent_permission` status/install/uninstall/update，但只由 `agent_mode` 的 Hook 产物包调用，
   不暴露独立开关或独立产品状态；
3. 扩展 `agent_mode::{set, update, update_artifact, artifact_updates, uninstall_all}`：
   - Claude/Codex 的 CLI 与 MCP 都管理 PermissionRequest；
   - Claude MCP 只卸 CLI timeout 条目、保留/安装 permission 条目；
   - None 卸载 permission；
   - Cursor/Grok 保持现有能力；
4. 重构 Codex trust reconcile，做到 feature 级增删、失败回滚、保留用户 Hook；
5. 组合测试至少覆盖：
   - CLI ↔ MCP ↔ None 的权限条目安装/保留/卸载；
   - lifecycle → agent_mode update → 卸 lifecycle；
   - agent_mode update → lifecycle → 切 None；
   - lifecycle 自动迁移与 agent_mode 手动更新交错；
   - 用户同事件多 group、多 handler、JSONC 注释与自定义 timeout；
   - config.toml 已有其它 `[hooks.state]`；
   - trust 写失败时 hooks.json 回滚；
6. 明确禁止 daemon 自动补装 PermissionRequest：旧版已集成用户必须先看到现有“需更新”，再通过
   单项更新或“全部更新”安装；lifecycle 自己的既有自动迁移行为不变。

### M5：设置 UI 与现有更新可观测性

1. 把 Rust/TS 的 `timeoutHook*` 展示状态泛化为当前模式 Hook 包状态，不新增权限专用 command；
2. 四张 Agent 卡按 §5.9 展示 Hook 包、包含能力和不支持原因，不出现独立 switch；
3. 验证旧 Claude/Codex CLI/MCP 安装会产生现有 `hookNeedsUpdate`，单项“更新”、总览 `Hook ×N`、
   Agents Tab 提示点和“全部更新”全链路复用；
4. `agents mode` / `agents update --hook` / `doctor` 复用新的 Hook 包口径；CLI help 把“超时 Hook”改成
   “当前模式 Hook”，并按 Agent/mode 输出实际包含能力；
5. i18n 中英文齐全，Cursor/Grok/Windows 的“不支持”原因不能只靠灰色状态表达。

### M6：验证、文档与发布收尾

1. Rust 单元/集成测试、`cargo fmt --check`、`cargo test`；
2. `npm` 前端 typecheck/build；
3. 按项目规则运行 `./scripts/install.sh`，后续人工确认全部使用新安装的 AskHuman；
4. 无真实危险操作的端到端验收：Claude/Codex 各触发 harmless permission request，覆盖本地批准、IM 批准、
   本地拒绝、IM 拒绝、关窗拒绝、Daemon/网络故障回原生、超时回原生；
5. 四 IM 真机验收并发收尾、卡片不可重复点击、按需投放与 `/here` 补推；
6. 更新 `docs/overview.md`、用户 wiki、`docs/PROGRESS.md`；功能提交使用清晰的 Conventional Commit，
   例如 `feat(hooks): route agent permission requests to AskHuman`。

## 7. 测试矩阵与验收标准

| 场景 | 预期 |
|---|---|
| 自动集成为“未集成” | Agent 原生行为完全不变，不 spawn AskHuman 权限 Hook |
| 新切换 Claude/Codex 到 CLI 或 MCP | 随现有集成产物一并安装 PermissionRequest Hook |
| 升级前已是 Claude/Codex CLI/MCP | 现有 Hook 产物显示“需更新”；点击单项/全部更新后才补装 |
| Claude/Codex 请求本来无需权限 | `PermissionRequest` 不触发，AskHuman 不出现 |
| 本地“批准一次”先答 | Hook 输出 allow；所有 IM 定格“已批准”，不写回复历史 |
| IM“批准一次”先答 | Hook 输出 allow；本地窗关闭，其它 IM 定格赢家 |
| 任一端“拒绝”先答 | Hook 输出 deny；其它端全部定格“已拒绝” |
| 用户关闭本地确认窗 | 明确 deny，不回原生弹窗 |
| GUI Helper 崩溃但 IM 可用 | 不 deny；继续等 IM |
| 所有渠道不可用 / Daemon 丢失 | stdout 为空；Agent 显示原生权限弹窗 |
| 24 小时无人处理 | 卡片定格过期；stdout 为空；回原生弹窗 |
| IM 详情被截断 | 清楚显示截断；仍可按用户定案批准一次 |
| `/here` 切换到另一 IM | 未答权限确认补推一次；旧端按活跃槽规则收尾/保持可追踪状态 |
| lifecycle 与 agent_mode 交错安装/卸载 | 只增删各自 marker；Codex 两类 trust 均保持正确 |
| 其它 Hook 返回 deny | AskHuman allow 不绕过 deny；最终仍由 Agent 权限系统裁定 |
| Cursor/Grok | 设置页说明不支持；磁盘不写权限 Hook |
| Windows | 设置页说明待 Daemon 支持；无简化版行为 |

完成标准：上述自动化测试通过，Claude/Codex 与四 IM 的人工验收均有结果；普通 Ask、`/stage`、生命周期
追踪、插话、watch 和自动集成三态无回归。

## 8. 风险与控制

1. **远程放大高风险操作**：权限卡必须醒目标明 Agent、workspace、工具和截断；只支持单次批准。用户已明确
   接受在截断详情下仍可批准，实施时不得暗中改成“始终允许”。
2. **Hook 卡住 Agent**：24 小时是产品定案；Hook/Daemon 必须有可取消连接和过期 timer，基础设施失败
   fail-open 到原生权限弹窗，不能 fail-allow 工具本身。
3. **误把崩溃当拒绝**：确认协议必须要求显式 action；EOF 只是 channel failure。
4. **Codex trust 相互破坏**：权限功能上线前先完成 feature 级 reconcile 和交错组合测试。
5. **卡片并发串答**：路由键至少包含渠道消息 id 与 request id，action id 只能从服务端台账取，不能信任客户端
   回传的任意字符串。
6. **敏感参数外发**：新安装或用户点击现有更新后，权限确认随 Claude/Codex 的 AskHuman 集成生效；
   UI/更新说明需明确权限详情会发送到当前启用的 IM。未集成时关闭、不落历史，但 IM 平台自身的消息留存
   不由 AskHuman 控制。
7. **旧新 Daemon 混用**：利用现有二进制指纹/drain；协议无法兼容时提升版本并验证在途请求收尾。

## 9. 建议提交拆分

1. `refactor(confirm): add reusable confirmation interaction core`
2. `feat(confirm): deliver confirmations through popup and IM channels`
3. `refactor(hooks): share hook config and reconcile codex trust`
4. `feat(hooks): route agent permission requests to AskHuman`
5. `feat(settings): show permission hooks in agent integration status`
6. `docs(hooks): document remote agent permission approval`

每个提交都必须保持可编译；对外可见的 `feat` subject 会进入 release notes，最终可在合并前按实际用户价值
调整拆分，避免把内部重构写成用户可见功能。
