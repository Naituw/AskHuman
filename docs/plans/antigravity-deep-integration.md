# 计划：Antigravity 深度集成（插件、生命周期、Stop 确认、插话、Watch、手机 / IM 新建任务与模型）

> 状态：2026-11-16 起草，**同日经用户批准**（执行位置：主工作区；P2–P3 过渡规则：固定 YOLO 时「不可启动」并说明原因）。
> 真实验证的授权口径随后放宽（用户在门 G1 验证计划的回答里，见规格 §1.2）：只用 Antigravity 的真实测试不再逐项授权，仍受各门预算、隔离与停止条件约束；**严禁使用 Cursor**。
> 门 G1 已完成（规格附录 A.2）；它暴露的子会话设计缺口 **D17 待用户决定**（用户离开期间未答复）：P1 不依赖它（按「每个会话独立」写，是 A / B / C 共同的底座），P3 的 Stop 卡与完成判定依赖它。
> 实施进度：见文末「实施记录」。
> 需求与决策：[规格](../specs/antigravity-deep-integration.md)（D1–D16、P1–P15、V1–V23、验收 §18；D17 待定）。
> 地图：`docs/overview.md`（文档边界、IPC 协议号）；落地时要回写的文档见规格 §19。
> 本计划照既有先例做，不另起炉灶：
> - 托管产物：`integrations/pi_extension.rs`（`status()`、`set_*_enabled`、渲染后比对判断 outdated）。
> - 新建任务的 App / CLI 双路线：`cursor_desktop/`（`Config`、`LaunchStatus`、`select_target`、`resolve_launch`、`Operation::Create`、回执、IPC 往返）与 [Cursor Desktop 新建任务计划](cursor-desktop-task-creation.md)。
> - 模型清单与启动映射：[iOS 模型与强度计划](ios-task-model.md)；终端桥与 `LaunchRecord`：[IM 创建任务计划](im-agent-task-launch.md)。

**硬约束**

1. 自动化测试与夹具**不启动真实 Antigravity 模型**：用假 `agentapi`、假 sidecar、假 `agy`（只用来通过「存在且可执行」的检查，从不被执行）和合成夹具；命令行靠断言组装出的 `Command`。
2. 真实验证只用 Antigravity（**严禁使用 Cursor**）：Flash 低价档、一句话任务、临时目录。授权口径（规格 §1.2）：原先沿用 [D27](../specs/im-agent-task-launch.md) 逐项经 AskHuman 批准，
   2026-11-16 起用户放宽为「只用 Antigravity 不再逐项授权」；但仍给验证计划（预计任务数与请求数）、遵守各门预算与停止条件，结束后如实报告实际用量
   （调研阶段约 61 次，G1 约 28 次且 T2 超出估计，见规格附录 A 与 A.2）。退出用户的 App、改会影响用户真实会话的全局插件目录、对真实目录开宽授权（YOLO）这类超出预算与隔离的动作仍先问。
3. 不碰用户正在使用的 Antigravity 会话；对 `~/.gemini/config/config.json` **只做读-改-写**（规格 P11），绝不用备份整文件还原——其中有用户自己的授权。
4. 不做轮询 / 延时重试式补丁（AGENTS.md「No workaround patches」）；遇到只能绕过的宿主缺陷，先经 AskHuman 问用户。
5. 默认不改变现有行为（规格 §18）：插件迁移只在用户点「更新 / 切换模式」或执行 `AskHuman agents` 命令时发生；sidecar 能力默认关；Dev Instance 默认不安装插件、不创建 sidecar（规格 §14，沿用 `DesktopFacts.dev_instance` 的写法）。

## 开工前

1. **执行位置（已定：主工作区）**：用户在批准本计划的同一轮里选了主工作区（我推荐的是独立 Dev Worktree，理由见规格 §20）。主工作区里另有一个会话在改 Cursor Desktop 的未提交文件，因此：
   - 只 `git add <明确路径>` 自己的文件，从不 `git add -A` / `git commit -a`；不还原、不搁置（`git stash`）、不格式化别人的文件；
   - `cargo test` / `clippy` 会把别人的半成品一起编进去：失败先判断是否来自它们，不是我的就报告，不修；
   - 主工作区里有别人的未提交改动时**不装日常版**（`./scripts/install.sh` 会把半成品编进用户的日常环境），先经 AskHuman 问用户怎么办；
   - 每次提交前 `git status` / `git diff --cached --stat` 核对只含自己的文件。
2. **基线**：`git status` / `git log -1`；先跑 `node scripts/machine-health.mjs`，再跑 `cargo test` 与 `cargo clippy` 基线（`scripts/codex-native-title.node-test.mjs` 是已知的无关失败，不算本任务）。
3. **先读**：overview「IPC 协议号」与「文档边界」；P2 动 IPC 前读 [即时换新](../specs/daemon-live-update.md) 与 `daemon-graceful-drain.md` §8；本计划预期**不动** `recovery.rs`（回执是文件），若发现要动，按 AGENTS.md 递增 `SCHEMA_VERSION` 并加迁移与夹具。
4. **每期开头**：全仓 `rg -n -i antigravity`（Rust、前端、iOS）对一遍下面的「特判清单」，防止漏项。

## 做法：把 Antigravity 并入现有机制，三处各照一个先例

- **产物管理仿 Pi**：新 `integrations/antigravity_plugin.rs` 提供 `status()`（`installed / outdated / mcp / lifecycle / stop / disabledInHost`）与 `set_lifecycle_enabled` / `set_stop_enabled`；规则与 MCP 由 `agent_mode.rs` 的 Antigravity 分支经它写。
  生命周期与 Stop 的偏好沿用各自的偏好文件，**加 `antigravity` 字段**（`#[serde(default)]`，向后兼容，不改既有键）。与 Pi 的区别只是产物是一个目录而不是一个文件。
- **新建任务仿 Cursor Desktop**：新 `antigravity_desktop/` 模块，复用 `LaunchTarget`（`Desktop` = App 路径，`Terminal` = `agy`），同样的 `Config`、`LaunchStatus`、`select_target`、`resolve_launch`、`Operation::Create`、回执与 `ClientMsg` / `ServerMsg` 往返。
  区别：创建动作由 daemon 经 sidecar 执行，而不是经应用内的消息桥。
- **身份仿 Codex**：`mcp/ask.rs` 的 `resolve_direct_binding` 读 `_meta`，与 Codex 读 `threadId` 同构。

## 阶段总览

| 阶段 | 内容 | 验证 | 协议 / 存储 |
| --- | --- | --- | --- |
| P1 基础 | 插件与迁移、`__agent-hook antigravity`、可信身份、生命周期与结果状态 | 夹具单测、临时 HOME 迁移测试、日常版最小真实验证（门 G1） | 不改 IPC，不改 SQLite |
| P2 手机 / IM 新建任务 + 模型 + Watch | `antigravity_desktop/`、sidecar 管理器与运行时、App 路径 `/new`、模型清单与 `--model`、Watch 解析、设置页与新建任务窗口 | 假 `agentapi` / 假 sidecar 测试台、`config.json` 并发写测试、模型夹具、日常版最小真实验证（门 G2） | **IPC +1（5→6）**；回执是文件 |
| P3 Stop 确认 + 插话 / 空闲消息 | Stop 处理器、`PreInvocation` 注入、经 sidecar 的 `send-message` | 假载荷与假 sidecar 测试、日常版最小真实验证（门 G3） | 不改 IPC |
| P4 CLI 路径 + YOLO + 冷启动种子 | `agy` 启动适配器（含 `--model`）、YOLO sidecar、最近目录种子 | 存根 `agy` 测试、日常版最小真实验证（门 G4） | 不改 IPC |

- 顺序固定为 P1 → P2 → P3 → P4（D12）。每期**单独提交**、单独装机验证；期与期之间向用户报告并取得下一期门的批准。提交遵循 Conventional Commits，只在本地，**不推送**，除非用户要求。
- 新模块在非 macOS 平台编译为「不支持」（`cfg` 隔离 `open -g` 等系统调用），Linux / Windows 构建不能被破坏；Linux 放开另行实测（规格 P13）。

## 现有 Antigravity 特判清单（逐个翻转）

下面每一项都读过代码、位置是当前所见；行号会漂移，开工时按符号找。

| 位置 | 现状 | 改成 | 期 |
| --- | --- | --- | --- |
| `agents/report.rs` `run` | `intended == Antigravity` 提前返回 | 分发到新 `agents/antigravity_hook.rs` | P1 |
| `agents/report.rs` `resolve_session_id` | Antigravity 返回空 | 去掉特判（stdin 的 `conversationId` 已在候选键里） | P1 |
| `agents/detect.rs` `session_id_from_env_map` | Antigravity 返回 `None` | **保持**（纯函数，验证不了）；环境 ID 只作候选，由 daemon 校验（见 P1 步骤 4） | P1 |
| `agents/detect.rs` `matches_agent` / `walk_agent_pid` | 只认 `agy` 进程 | **不改**：App 的共享 `language_server` 不匹配 → `walk_agent_pid` 返回 `None` → 走「无 pid」路径；补测试 | P1 |
| `cli/mod.rs` `caller_context_from_env` | MCP 来源的 Antigravity 丢弃会话 ID | 保留 `MCP_AGENT_SESSION_ID_ENV`；直接 CLI 把环境 ID 作候选 | P1 |
| `mcp/ask.rs` `resolve_direct_binding`、`configure_child` | 只认 Codex `threadId` 与令牌；子进程环境清单 | 加 Antigravity `_meta`（UUID，取根会话）；清单加 Antigravity | P1 |
| `agents/result.rs` `observation` | 无 Antigravity 分支 | `terminationReason` → `TurnOutcome`（`NO_TOOL_CALL`=完成、`ERROR`=失败、`MAX_*`=未完成、`USER_CANCELED`=中断，其余未知） | P1 |
| `agents/title.rs`（三处按家族取原生标题） | Antigravity 返回 `None` | **保持**（原生名在私有库，本版不读）；标题回退取 `lastUserInput` 首行，开工时核对回退链 | P1 |
| `integrations/agent_lifecycle.rs`：`LifecyclePreferences::get/set`、`events`、`disk_status`、`set_enabled`、`install`、`uninstall_unlocked`、`reconcile_unlocked`、`json` 状态检查 | 一律「不支持」/ 空事件集 | 偏好加 `antigravity`；产物状态取 `antigravity_plugin::status().lifecycle`（同 Pi）；安装 / 卸载走 `set_lifecycle_enabled` | P1 |
| `integrations/agent_mode.rs` Antigravity 分支、`antigravity_rule.rs`、`agent_rules.rs`、`mcp_config.rs` | 单文件规则 + 全局 MCP 条目 | 插件目录事务与迁移 | P1 |
| `commands.rs` `collaboration_style_apply_integrations` | 先读 `antigravity_status()` | 改写插件里的规则（内容变才写，V14） | P1 |
| `integrations/agent_stop.rs`：`supported`、`enabled`、`set_enabled`、偏好、`reconcile_current_mode_unlocked`、占位路径 `unsupported-stop-hooks.json` | `supported = false` | 偏好加 `antigravity`；产物取 `antigravity_plugin::status().stop` | P3 |
| `agents/stop.rs` `turn_failed` | Antigravity 恒为 `false` | `terminationReason == ERROR`（生命周期上报的 `failed` 标记要用） | P1 |
| `agents/stop.rs` `is_natural_stop`、`last_assistant_message`、`continuation_output` | Antigravity 分别落在 `false`、无文本、`{}` | `NO_TOOL_CALL` 且 `fullyIdle`；正文取自 transcript 而不是载荷；输出 `{"decision":"continue","reason":…}` | P3 |
| `daemon/runtime/ios_agents.rs` `can_interject` | `grok、antigravity` 返回 `unsupported` | 对 Antigravity 移除（工作中可插话）；全库再搜 `interject` 的按家族排除点（控制台、IM、帮助文案） | P3 |
| `agents/transcript_full.rs`、`agents/activity.rs` `push_events` | Antigravity 为空 | 共用一个 Antigravity 行解析器 | P2 |
| `agents/model_catalog/source.rs` `load_with` | 返回「不支持模型发现」 | `agy models` / 三个别名 | P2 |
| `integrations/agent_launch.rs` `launch_supported` | `false` | `true`（新建任务）；**Fork 仍不支持**：`fork_readiness_uncached` 保持，补测试 | P2 |
| `integrations/agent_start.rs`（`route`、`startable`、`startable_now`、`judge` 与测试） | Antigravity 永远不可启动 | 第三个 `LaunchStatus`（`antigravity_desktop`），同 Codex / Cursor | P2 |
| `codex_desktop::resolve_launch` | Codex、Cursor 两个分支，其余固定 `Terminal` | 加 Antigravity 分支 | P2 |
| `daemon/runtime/task_launch.rs` `start_task` 的 Desktop 分支 | 只有 Codex、Cursor | 加 `start_antigravity_desktop`（同 `start_cursor_desktop`） | P2 |
| `daemon/runtime/inbound.rs`：`/new` 探测元组、Agent 列表的 `secondary`、Pi 的权限卡特判、`Started::Desktop` 回复文案 | 无 Antigravity | 加 Antigravity（回复文案「Antigravity 已在 App 创建任务」，中英文） | P2 |
| `daemon/runtime/ios_tasks.rs`：`probe_agents`（含「Antigravity 不上手机」的注释）、`AgentProbes`、`is_ready`、`catalog_source`、`permission_selectable`、`effective_permission`；`agents/task_options.rs` `catalog_source` | 无 Antigravity | 第三个状态、清单来源（无 `agy` 时不返回 `None`）、过渡权限规则（见 P2 步骤 4、8） | P2 |
| `commands.rs` `new_task_launch`（桌面分支的「只有 Codex 和 Cursor」检查）、`cursor_launch_status` 的对应命令 | 无 Antigravity | 加 Antigravity 与 `antigravity_launch_status` | P2 |
| `onboarding/text.rs`（用 `launch_supported` 过滤引导里的 Agent） | 排除 Antigravity | 随 `launch_supported` 自然纳入；改相应测试与文案 | P2 |
| 前端：`views/newtask/NewTaskForm.vue`（`agentReady`、备注、pending-create 键、`pi` 权限特判）、`lib/ipc.ts`、`views/setup/TasksPage.vue`（「Antigravity 不能建任务」）、`views/settings/readiness.ts` 注释、`useAgentTasks.ts` | 无 Antigravity | 加 Antigravity | P2 |
| 前端：`views/settings/useIntegration.ts`（`recommended: "cli"`、`lifecycle.supported: false`）、`IntegrationTab.vue`、i18n（`antigravityScope`、`antigravityLifecycleUnsupported`、`console.antigravityHint`） | 说明「未支持」 | 推荐 MCP、生命周期可开、文案更新 | P1 |
| 前端：`views/console/InteractSlot.vue`（Antigravity 不显示发送框并给提示） | 不支持消息 | 随 P3 空闲消息放开；P1 先把提示改成「插话与空闲消息待开放」 | P1 / P3 |
| `daemon/runtime/agent_control.rs`：`can_send_with`、`send_with`、`mark_messageable`、`CursorChat` | 无 Antigravity | 经 sidecar 的空闲消息（见 P3 步骤 4） | P3 |
| `integrations/agent_launch.rs`：`create_record_with_options`、`agent_args`、`model_flags`、`yolo_flag`、`command_name`（已是 `agy`） | `create_record_with_options` 一律拒绝；`agent_args` 空 | `LaunchMode::New` 放行（`Fork` 仍拒）；`-i`、`--model`、`--dangerously-skip-permissions` | P4 |
| iOS | 仅 `AgentBadge.swift` 有图标 | 预期无新代码，P2 核对 `TaskComposer` 对「有模型、无强度」的处理并补一条用例 | P2 |

## 横切约定

- **IPC（只在 P2）**：新增两组变体，合并成一次 `PROTOCOL_VERSION` 5→6。
  1. `ClientMsg::AntigravityDesktop { operation }` / `ServerMsg::AntigravityDesktop { result }`：GUI / 命令行 → daemon，照 `CursorDesktop`，配 `client::antigravity_desktop`、daemon 里与 `CursorDesktop` 并列的处理分支、隐藏子命令 `__antigravity-desktop`（stdin JSON，同 `__cursor-desktop`，`dryRun` 只确保 sidecar 在线并核对授权，不创建会话）。
     `Operation` 含 `Create`、`Sidecars`（列表，带在线状态）、`RemoveSidecar`、`RemoveAllSidecars`、`SetRules`；daemon 是 sidecar 与 `config.json` 的**唯一写者**，所以设置页的操作也走它。
  2. sidecar ↔ daemon 的长连接：`ClientMsg::AntigravitySidecar`（报到、命令结果）与 `ServerMsg::AntigravitySidecar`（`new_conversation`、`send_message` 命令），以接管连接的方式处理（同 `Control::Recover` 的做法，开工时核对 `Control` 的写法）。
  同步 overview「IPC 协议号」历史与 `daemon-graceful-drain.md` §8，并审计全部发送点与广播。P1、P3、P4 复用既有 `ClientMsg::AgentEvent`（`transcript_path`、`hint_pid`、`launch_id`、`interject_poll`、`failed`、`observation`），**不动枚举**；新增字段一律 `#[serde(default)]`。
- **hook 契约**：`__agent-hook antigravity <event>` 永远退出码 0；除非要应答（Stop 继续、`PreInvocation` 注入），stdout 为空；daemon 不可达 / 超时 / 解析失败一律 fail-open；热路径不拉起 daemon（规格 §8）。
- **登记表**：`~/.askhuman/antigravity-sidecars.json`（id、目录、种类、用户追加的授权规则、创建与最近使用时间）是授权规则的**唯一来源**，`sidecar.json` 由它确定性生成；每个 sidecar 目录里放 `.askhuman-managed`。登记表丢失时，从带标记的目录重建只读视图，不据此改写任何东西。
- **所有权与账本**：沿用 [antigravity.md](../specs/antigravity.md) §5（路径、摘要、uid、实例账本）；用户改过的内容一律视为冲突，不覆盖、不删除、报告。`antigravity_rule.rs` 里能复用的所有权 / 账本 / 原子写 / JSONC 编辑抽成共享函数，不复制。
- **配置**：`AppConfig` 新增 `antigravity_desktop: { launchPreference（desktop 默认 / terminal）, manageSidecars（默认 false） }`，键名按现有 `cursor_desktop` 的风格（规格里的 `antigravity.launchPreference` 是拟定名，以此为准并回写规格）；登记到 `overview-configuration.md`；`AskHuman config set` 按 `AppConfig` 的路径通用处理，新键自动可设，无需改键表（是否要像 `cursor_desktop::configured` 那样在设置后确保 daemon 在跑，开工时核对，预期不需要：sidecar 按需创建）。
- **i18n**：所有用户可见文案同批进 `src/i18n/{zh,en}.ts` 与 Rust 侧既有 i18n；提示词类文案只放 `prompts.rs`。

## 验证门与预算

「门」是每期开工前的最小真实验证；全部在临时目录、Flash 低价档、一句话任务下做；每个门开始前向用户报预计任务数与请求数（授权口径见硬约束 2：不再逐项批准，但上限照旧），结果写回规格 §17 并更新 §4。
**「调用数」按任务计、另报模型请求数**：G1 的教训是一个任务可能产生几十次模型请求，所以下表的上限同时给任务数，并在任务提示里写明确切的工具调用参数，减少模型摸索。

| 门 | 验证项（规格 §17） | 做法 | 预计调用上限 |
| --- | --- | --- | --- |
| G1（P1 前）**已完成** | V3-a、V5、V6、V12、V14 | 2026-11-16 执行：4 个任务、约 31 次请求（T1a `agy` 未信任目录 2；T2 App 带子 Agent 24；T3 App 90 秒 MCP 调用 2；T1c `agy` 受信任目录补测 3；T1b 免费探针 0）；结果在规格附录 A.2，V3-a / V5 / V6 / V14 已验，V12 部分已验，并暴露 §8 L4（D17） | 5（任务数）/ 实际 4 |
| G2（P2 前） | V18、V19、V20（App，只测 Flash 三档）、V8、V9、V7 | 白名单预授权的 MCP 与全局授权是否对 sidecar 会话生效（1–2）；`agentapi --model=` 对 `-low` / `-medium` / `-high` 的接受（3，Pro 级与 Claude 级只在你另行批准后各 1）；重建同名全局 sidecar 是否认领孤儿项目（0）；宽授权命令是否在沙箱外、能否连 daemon socket（1）；sidecar 会话在 App 里的位置（你在场看，0） | 6 |
| G3（P3 前） | V2、V10、V11 | Stop `continue` 的次数上限：**最多连续 8 次**，到上限即停，8 次内没碰到就记「上限不影响人工确认」（≤8，人在回路里每次都要点一下）；`send-message` 对工作中会话 / 用户自建会话（2–3，只做一次、记录后停） | 12 |
| G4（P4 前） | V3（其余）、V15、V20（`agy`） | `agy` 上 Stop `continue`、`PreInvocation` 注入、`send-message` 唤醒（3）；`agy -i` 对 `-` 开头文本（1）；`agy --model <id>` 与 `=` 写法（1–2） | 8 |
| 另 | V1、V16 | 要退出用户的 App / 关闭窗口：只在你同意并在场时做，0 调用 | 0 |

合计上限约 31 次（调研阶段为 61 次）；实际用量逐门报告。任一门与预期不符就**停下、记录、报告**，不重试、不绕过；规格里对应的设计要先改并经你确认。

## P1 细节（基础：插件、hooks、身份、生命周期）

提交切分：① 夹具 + hook 上报器 + 结果状态 ② 身份（MCP / CLI）③ 插件、所有权与迁移 ④ 生命周期接线、设置页与文案。

1. **夹具先于代码**：把实测证据脱敏成夹具，放到相应模块的 `fixtures/`：六类 hook 载荷（`SessionStart`；`PreInvocation` 的 `invocationNum` 为 0 与大于 0，含后台任务自醒、`lastUserInput` 过期的一回合；
   `Stop` 的 `NO_TOOL_CALL` × `fullyIdle` 真 / 假，`ERROR` 按文档合成）、MCP `_meta`（App 实测，含子会话的 `parent_conversation_id`；`agy` 的字段按文档合成并标注）、`run_command` 环境、`config.json` 样本（含 `plugins`、`sidecars`、`userSettings.globalPermissionGrants`）。
   G1 新增的夹具来源（`~/agy-hook-lab/g1/{logs,evidence}`，仓库外）：**子会话**的 `SessionStart` / `PreInvocation` / `Stop`（`lastUserInput` 为空）；**T2 的完整时序**（根 `Stop(false)` → 子 `send_message` 唤醒根 → 根 `Stop(false)` → 子 `Stop(true)`，对应规格 §4.1 与 L4）；
   父 transcript 里 `invoke_subagent` 调用与其「Created the following subagent」结果步骤；子会话第 0 步 `SYSTEM_MESSAGE`；一个根会话 `Stop(true)` 时 transcript 已含最终回复（V6）；`agy` 的 `hooks_manager` 载入日志行（未信任 / 信任各一条）。
   脱敏：会话 ID、路径、用户名、令牌全部替换；**不含**真实 CSRF、邮箱、API key，也不含 hook 环境里的 `AGY_BROWSER_WS_URL`（含会话级秘密）。
2. **hook 上报器**：新 `agents/antigravity_hook.rs`——解析载荷，按规格 §8 映射成 `LifecycleEvent`（`PreInvocation` 且 `invocationNum==0` → `turn-start`，其余 → `activity`；`Stop` 且 `fullyIdle` → `turn-end`，否则 `activity`），
   取会话 ID（`conversationId`）、项目（`workspacePaths[0]`，缺省 `cwd`）、模型（`modelName`，经 `session_info::capture_hook`）、`transcriptPath`、`launch_id`、`prompt_sha256`（来自 `lastUserInput`）、`failed`（`terminationReason==ERROR`），发既有 `ClientMsg::AgentEvent`；
   `report.rs::run` 去掉提前返回并分发过来，复用其读 stdin、`detect::parent_pid`、`client::report_agent_event`。`result::observation` 加 Antigravity 分支（见清单）。**不新增轮询**。
3. **pid 与注册表**：`agy` 的 hook 父进程是 `agy` 自己（`matches_agent` 已认 `agy`）→ 作会话 pid；App 的 hook 父进程是共享 `language_server`，不匹配 → `None`，走 1 小时 TTL 与 30 分钟「工作中」兜底（[D12 / D25](../specs/agent-lifecycle-tracking.md)）。
   表面（App / `agy` / IDE）由 `transcriptPath` 目录判定（`antigravity/`、`antigravity-cli/`、`antigravity-ide/`，后者不处理）。只补测试，不改 `detect.rs` 的判定。
4. **可信身份**：
   - MCP：`resolve_direct_binding` 读 `_meta["antigravity.google/conversation_id"]` / `root_conversation_id`（须是 UUID，取根）→ `AgentBinding{antigravity, <根会话 ID>}`；`configure_child` 的清环境清单加入 Antigravity；`caller_context_from_env` 不再丢弃 MCP 来源的会话 ID；`show_last` 按会话作用域（规格 §7）。
   - CLI 模式：环境里的 `ANTIGRAVITY_CONVERSATION_ID` 只作为**候选**随提问带给 daemon（`CallerContext.from_mcp == false`），daemon 在接收提问登记的位置（`daemon/runtime/mod.rs` 处理 `TaskRequest` 处）对照注册表里 hook 已登记、未结束的会话，一致才升级为会话绑定，否则按「只认家族」处理；
     `detect::session_id_from_env_map` 本身仍返回 `None`（纯函数，没有注册表可查）。落点开工时先读码确认，原则不变。
5. **插件生成器、所有权与迁移**（规格 §6）：新 `integrations/antigravity_plugin.rs`（API 仿 `pi_extension.rs`，产物是 `plugins/askhuman/` 目录）；`agent_mode.rs` 的 Antigravity 分支（现在「先备好配置再发布」）改为插件目录事务。
   - **目录事务设计**（规格 §6.3 留给本计划的问题）：首次安装在同一文件系统的私有临时目录里**整体写好**，再用一次 `rename` 发布，宿主永远看不到半成品插件；更新已有插件时**逐文件**原子替换（临时文件 + `rename`、0600、不跟随符号链接）、**只重写内容变了的文件**（避免 V14），
     顺序固定为 `rules/AGENTS.md` → `mcp_config.json` → `hooks.json` → `.askhuman-managed`（账本最后写）；中途崩溃时账本与文件不符，下一次对账幂等修复，错误信息写明保留状态与显式重试命令。
     G1 的 V14 结论：重写文件不会重启 MCP server、不会打断在途 `ask`（工作区级插件；全局目录留 G2 验证，V22）；**但 `mcp_config.json` 的改动要等 server 重启才作用于已运行的进程**，所以「更新」的回执里要写明这一点（例如改 `timeoutSeconds` 后需重启 Antigravity）。
   - `hooks.json` 里三个事件在 P1 就按最终形态写（`PreInvocation` 与 `Stop` 的 `timeout` 都是 86400）：P1 的 `PreInvocation` 处理器从不阻塞，但先写好，P3 启用插话与 Stop 确认时就不必再改文件、也就不会触发插件重载。
   - **迁移顺序**（规格 §6.5）：先发布插件，再清理旧共享规则 `rules/askhuman-antigravity.md` 与全局 `mcp_config.json` 里的托管条目；任一步证明不了所有权就保留、报告冲突、停止清理。
   - 「插件已被用户停用」只读检测 `config.json` 的 `plugins.askhuman.enabled`，**绝不写**；V3 若显示 `agy` 不触发插件 hooks，则 hooks 改装全局 `hooks.json`（同样的所有权规则；该分支只在 G1 结论要求时实现）。
   - 协作风格变更（`collaboration_style_apply_integrations`）改写插件里的 `rules/AGENTS.md`，内容变才写。
6. **生命周期接线**：见清单；偏好文件加 `antigravity` 字段；`agent_start.rs` 与 `agent_launch.rs` 的「不可启动」在 P1 **保持不变**。
7. **设置与命令行**：Agents 页 Antigravity 卡——集成模式（None / CLI / MCP，新装默认 MCP，P1）、生命周期开关、「插件已在 Antigravity 中停用」提示、首次「始终允许」的说明（可只读看 `userSettings.globalPermissionGrants` 是否已含 `mcp(askhuman_mcp/*)` 或 `mcp(*)`，仅作提示，不依赖）；
   `AskHuman agents` 与 `doctor` 同步；`src/views/settings/{IntegrationTab.vue,useIntegration.ts}`、i18n、`vitest`。
8. **测试**：载荷解析与事件映射（含自醒回合、取消无事件、`fullyIdle:false`）；hook 输出（空 stdout、退出码 0、daemon 缺席 fail-open）；注册表（App `pid=None`、`agy` 有 pid）；MCP 绑定（根会话 ID、环境 ID 与登记不一致）；
   插件生成 / 所有权 / 迁移 / 冲突 / 幂等 / 用户改过 / 临时 HOME；`config.json` 只读检测；偏好与 lifecycle 安装卸载往返；Dev Instance 守卫。
   **子会话（L3 / L4，D17 待定）**：P1 的登记与生命周期按「每个会话独立」写；hook 上报器里**不**做根 / 子判断，也不用 `lastUserInput` 为空去猜；D17 定案后再接线，并补夹具驱动的测试（T2 时序必须成为一条回归测试）。
9. **P1 完成判定**：规格 §18 中的迁移、hooks、身份、生命周期四条在测试里全部成立；`./scripts/install.sh` 装机（主工作区里有别人的未提交改动时先问用户，见「开工前」1）；在**日常版**上做最小真实验证（只用 Antigravity，一个临时目录、一个 Flash 低价档的一句话任务）：
   会话出现在 Agent 控制台并正确转换「工作中 / 空闲」，`show-last` 按会话工作，插件可在 Antigravity 里停用并被检测到。

## P2 细节（新建任务、模型、Watch）

提交切分（每步可独立合入）：① 模型清单与 `--model` 映射 ② Watch 解析器 ③ `antigravity_desktop/` 状态与配置 ④ sidecar 文件与 `config.json` 编辑器 ⑤ IPC 与 sidecar 运行时 ⑥ 创建操作、回执与 App 启动 ⑦ 入口接线、前端与文案。

1. **门 G2**。结果决定 §12.4 的默认授权集是否要改（V18 / V19），以及别名回退里能放哪些别名（V20）。
2. **模型清单与 `--model`**（纯 Rust，不依赖 sidecar，先做）：
   - `model_catalog/parse.rs` 新增 `parse_antigravity_models`（每行「id、制表符、显示名」，忽略 stderr 与不含制表符的行，复用 `valid_model_id`、去重、截断上限）；用 `agy models` 的真实输出做夹具（模型 ID 与显示名是公开信息），另加脏行与不规则条目（`gemini-pro-orchestrator-g1-pro`）。
   - `source.rs::load_with` 的 Antigravity 分支（现在直接 `Err`）：`executable` 非空 → `agy models`（既有 `LISTING_TIMEOUT`、30 分钟缓存、失败降级只剩「默认」）；`executable` 为空（这台机器没有 `agy`）→ 三个别名 `flash_lite` / `flash` / `pro`（副标题「Antigravity 别名」，仅 App 路径有效），不执行任何进程。
     `CatalogSource.executable` 是 `String`，空串表示「无 `agy`」，`cache_key` 已含它，所以别名清单有自己的缓存位；两处 `catalog_source`（`task_options.rs`、`ios_tasks.rs`）对 Antigravity 不再在没有可执行文件时返回 `None`。无强度行、无「默认所指」。
   - `ModelChoice::check_for(Antigravity)` 拒绝 `effort` 与 `speed`（同 Cursor）；`task_options::validate` 与其它 Agent 同一入口，id 须与当前清单某一项完全相等，否则 `modelUnavailable`、什么都不启动。
   - 启动映射：App 路径 sidecar 命令 `agentapi new-conversation --model=<id> …`；CLI 路径 `model_flags` 的 `--model <id>`（P4 接通，P2 只写纯函数与测试）。「默认」时命令行与不接入时**逐字节相同**（有测试）。
3. **Watch 解析器**（纯 Rust）：`transcript_full.rs` 与 `activity.rs` 共用一个 Antigravity 行解析器——`PLANNER_RESPONSE.content` 作最新正文（`status==DONE` 才定稿）、`tool_calls[]` 与其后的 `GENERIC` 结果作工具、`USER_INPUT` 去掉 `<USER_REQUEST>`；`SYSTEM_MESSAGE` 与注入消息不当正文。
   路径来自 hook 的 `transcriptPath`（登记到注册表，沿用 `session_paths` 的登记模式），规范化后须落在 `~/.gemini/antigravity{,-cli}/brain/<conv>/.system_generated/logs/` 下；夹具含 `content` 与 `tool_calls` 并存、半行、超长行；沿用既有的有界尾读与预算。
4. **`antigravity_desktop/` 状态与配置**（照 `cursor_desktop/launch.rs`）：`Config`、`LaunchStatus{preference, target, desktop_available, terminal_available, integrated, desktop_reason, terminal_reason}`、`select_target`、`UnavailableReason`（中英文 `text(lang)`）、`LaunchStatus::description(lang)`、`resolve_launch`（有回执则钉住 `Desktop`）；
   「App 路径整体可用」按规格 §12.1 逐条判断（macOS、App 已装、插件已装未停用、集成模式为 MCP、`manageSidecars` 已开、目录已有 sidecar 或总数未达上限）；Dev Instance 与非 macOS 在这里就挡掉。测试照 `cursor_desktop` 的那组（偏好只定优先级、另一条可用就改道、都不可用时列出两边原因）。
   **过渡规则**：P2–P3 期间 Antigravity 还没有 YOLO（P4 才有），所以 `agentTasks.permissionPrompt == yolo` 时两条路线都标 `YoloUnsupported`（P4 删除）。`launch_status` 本来就读配置，规则放这里，手机、IM、GUI、设置、doctor 自动一致，**不静默降级成白名单**。
5. **sidecar 文件与 `config.json` 编辑器**（纯函数 + 临时目录）：单写者、每次写前重读、同目录临时文件原子 `rename`、0600、保留未知键、只动 `sidecars.askhuman-*`；冲突重读重试至多 3 次，仍失败报错不强写。
   用「假 App 写入者」线程并发改写来测（含写入 `userSettings.globalPermissionGrants` 与 `projectId` / `resolvedPermissions`）。`sidecar.json` 的字段与白名单规则校验：每条须是合法 `action(target)`，白名单里禁止全局通配；总数上限 20。附件目录取 `todo_attachments` 的投递目录（开工时核对），以只读授权写进白名单。
6. **IPC 与 sidecar 运行时**：按「横切约定」加变体、`client::antigravity_desktop`、daemon 处理分支、`__antigravity-desktop`，`PROTOCOL_VERSION` 5→6 并同步文档。
   `__antigravity-sidecar --id <id>`（`cli/mod.rs` 分发到 `agents/antigravity_sidecar.rs`）：无状态；**不拉起 daemon**；连接断开后有上限的退避重连；对 SIGINT / SIGTERM 干净退出；只执行 daemon 下发的 `agentapi` 调用，任务文本作单个 argv（无 shell），不从 stdin 或网络接受别的指令；日志里不记任务正文。
   命令行：`AskHuman agents` 家族下补 sidecar 的 `list` / `remove <id>` / `remove --all`，headless 机器也能管理。
7. **创建操作、回执与 App 启动**（`antigravity_desktop/create.rs`）：`Operation::Create{cwd,text,permission,id,model,title,dry_run}`；回执 `~/.askhuman/antigravity-tasks/<operationId>.json`，阶段 `prepared → sent → created / failed / unknown`；只有 `prepared` 与 `failed` 可用同一操作 ID 重来；崩溃恢复按回执阶段判定。
   答复里带 `sessionId`（= `conversationId`），手机与 IM 据此**立即配对**（`ios_tasks` 的 `task_matched`、IM 的 `find_agent_by_session`），并在注册表**预登记**会话（来源渠道、标题），hooks 到达后转「工作中」；60 秒内没有 hook 则来源 IM 告警（沿用 [D22](../specs/im-agent-task-launch.md)）。
   App 未运行：`open -g` 启动，等 sidecar 报到（事件驱动，最多约 30 秒，与创建 sidecar 共用预算），超时回退 CLI（P4 才可用，P2 里只报错并说明原因）。
8. **入口接线**：清单里 P2 的各项。要点：`agent_start` 加第三个状态；`launch_supported(Antigravity)=true` 而 Fork 不变；`start_task` 的 Desktop 分支；`inbound.rs` 的 Agent 列表、回复文案；`ios_tasks.rs` 的 `AgentProbes`、清单来源。
   **没有 YOLO 期间的权限步骤**：手机 `permission_selectable=false`；IM 走与 Pi 相同的「跳过权限卡」分支；GUI 的 `selectedKind === 'pi'` 两处特判加上 Antigravity；P4 删除这些特判并恢复完整权限步骤。附件与警告拼进任务文本（`task_with_attachments`），有警告则拒绝（同现有 Cursor 桌面分支）。
9. **前端与文案**：`NewTaskForm.vue` 的 `agentReady`、备注、pending-create 键（`antigravity-desktop-pending-create`，保证重试用同一操作 ID）、`pi` 特判；`lib/ipc.ts` 的 `antigravityLaunchStatus` 与 sidecar 操作；`lib/antigravityLaunch.ts`（描述文案，仿 `cursorLaunch.ts`）；
   设置页 Antigravity 卡新增：「允许 AskHuman 管理 Antigravity sidecar」总开关、新建任务偏好（App / CLI）、已管理 sidecar 列表（目录、类型、在线状态、最近使用，目录不存在时标出）与「移除」「移除全部」、按目录的授权规则编辑；`TasksPage.vue`、`readiness.ts`；
   中英文 i18n、`vitest`；用户文档（wiki 中英文）。
10. **测试**：假 `agentapi`（脚本，按预设返回会话 ID / 错误 / 超时）+ 假 sidecar（直接说 IPC）驱动状态机；回执阶段与崩溃恢复；目标选择（App 优先、对称回退、不静默改道）；readiness 单一判断（手机 / GUI / IM / 设置一致）；过渡规则（固定 YOLO 时不可启动）；
    模型夹具与命令行断言；Watch 夹具；`config.json` 并发；旧新协议互通（审计所有发送点）；Fork 仍不支持。
11. **P2 完成判定**：规格 §18 的新建任务（App）、模型、Watch、配置写入四条成立；装机后在日常版做最小真实验证（经批准）：在临时目录里用手机与 IM 各建一个任务（模型分别选默认与一个 Flash 档），看到回执、默认 watch、白名单拒绝未授权动作、sidecar 在设置页可见可移除。

## P3 细节（Stop 确认、插话、空闲消息）

提交切分：① Stop 处理器与偏好 ② 工作中插话 ③ 空闲消息。

1. **门 G3**。V2 的结果决定 Stop 续跑有没有需要降级的上限；V6 决定卡片正文与标记去重的读取时机；V10 / V11 决定空闲消息的范围（规格 §10.2）。
2. **Stop 处理器**：`agent_stop.rs` 的偏好与产物，`agents/stop.rs` 的 `is_natural_stop`、`last_assistant_message`、`continuation_output`（见清单）——按规格 §9 的条件出卡（`NO_TOOL_CALL` 且 `fullyIdle` 且根会话）；`ERROR` 走最终失败卡，`MAX_*` 走「未完成」，其余静默；
   卡片正文读 transcript 最后一个带 `content` 的 `PLANNER_RESPONSE`（路径校验同 P2 Watch）；继续 = 输出 `{"decision":"continue","reason":"<续跑提示词>"}`，提示词用既有的结构化包裹（`prompts.rs` 已有 Antigravity 分支 `stop_continue_wrapped_instruction`，开工时核对），文案只放 `prompts.rs`；
   `[user_confirmed_end_turn]` 去重；失败 fail-open（空 stdout、退出码 0）。与生命周期共用同一个 `Stop` 处理器（[D16](../specs/agent-lifecycle-tracking.md)）；`hooks.json` 已在 P1 写好，启用不改文件。
3. **工作中插话**：`antigravity_hook.rs` 的 `PreInvocation` 走既有 `interject_poll`（同一连接读回一帧裁决，热路径零文件 IO，首帧约 300 ms 超时 fail-open）；`Deny{text,attachments}` 映射成 `{"injectSteps":[{"userMessage":"<包装文本>"}]}`；
   包装文案在 `prompts.rs` 单一来源，保留 `[USER INTERJECTION]` 前缀，**去掉**「工具调用被拦下」那句；composer 打开中阻塞等待提交 / 取消。`ios_agents::can_interject` 与 Mac 控制台 `InteractSlot.vue` 放开工作中状态。
4. **空闲 / 已结束消息**（`agent_control.rs`）：`can_send_with` 的 `available` 分支加 Antigravity（有在线 sidecar 且会话属于范围）；`send_with` 在 Cursor 分支旁加 Antigravity 分支，经 sidecar 的 `send_message` 命令执行 `agentapi send-message`，
   并把 `CursorChat` 这个测试替身 trait 泛化成「原生聊天通道」（Cursor 与 Antigravity 各一个实现，Cursor 现有测试保持通过）；`mark_messageable` 的 `has_chat` 按家族分派；回执沿用 `agent_controls` 日志（`dispatched / accepted / rejected / unknown`，[结果与恢复 §5.4](../specs/agent-lifecycle-results-and-recovery.md)），
   `accepted` 的判定（`send-message` 命令成功返回，还是要等到观察到新回合 `PreInvocation(0)`）由门 G3 的 V10 / V11 定；**绝不**对工作中会话调用。
5. **测试**：Stop 条件矩阵（含 `fullyIdle:false`、子会话——按 D17 的结论、`ERROR`、`MAX_*`）；继续 / 结束 / 关窗 / 超时输出；标记去重；注入输出与「只送达一次」；composer 阻塞与取消；空闲消息的能力判定与回执；取消后的滞后在兜底内收敛。
6. **P3 完成判定**：规格 §18 的 Stop 确认与插话两条成立；装机后日常版最小真实验证（经批准）：一个临时任务结束时出确认卡并能「继续」一次；工作中从手机插话被送达；空闲会话被消息叫醒。

## P4 细节（CLI 路径、YOLO、种子）

提交切分：① `agy` 启动适配器与目标选择 ② YOLO sidecar 与设置页标注 ③ 种子（可选）。

1. **门 G4**。
2. **`agy` 启动适配器**（`agent_launch.rs`）：readiness（login shell 里解析 `agy`、版本达标）；`create_record_with_options` 对 `LaunchMode::New` 放行（`Fork` 仍拒）；`agent_args` 的 New 分支加 `-i`（任务文本单个 argv；V15 的结论决定要不要在前面加 `--`）与 `model_flags`（`--model <id>`），
   权限映射：Agent 默认 → 不加；YOLO → `yolo_flag`（`--dangerously-skip-permissions`，补进 [IM 创建任务 §6.3](../specs/im-agent-task-launch.md) 的 YOLO 表）；`LaunchRecord` 与 `__agent-launch` 沿用，hook 带回 launch id 与 `LaunchRecord` 配对；首次引导 / 信任提示不代答（V12）。
   `antigravity_desktop::launch_status` 的 `terminal_reason` 按 `cli_reason` 的写法接上；目标选择对称，回退只发生在选择阶段。
3. **YOLO sidecar**：`askhuman-yolo-<h8>`，授权 `command(*)`、`read_file(*)`、`write_file(*)`、`read_url(*)`、`mcp(*)`；沿用 `/new` 权限卡与全局 `permissionPrompt`，**不加二次确认**（D14）；显示名带「YOLO」、`description` 写明宽授权，设置页标成「宽授权」并可一键移除；
   删除 P2 的过渡规则（`YoloUnsupported`）与「无权限步骤」的特判，恢复手机 / IM / GUI 的完整权限步骤。
4. **最近目录冷启动种子（可选）**：只读各产品 `conversation_summaries.db` 的 `workspace_uris`（私有格式、读取失败静默、不依赖）；只给首次候选，不改「手动添加始终可用」。
5. **测试与完成判定**：存根 `agy` 断言命令行与 `LaunchRecord` 哈希；目标选择矩阵；YOLO 授权集与移除；种子读取失败静默；过渡规则已删除且各入口权限步骤恢复。装机后日常版最小真实验证（经批准）：IM 在 App 不可用时回退到 `agy`，YOLO 任务在临时目录联网命令无人值守跑通。

## 文档回写

- 每期落地后：规格 §20 记反馈与实测结论、§17 更新 V 状态、§4 补事实；本计划更新阶段状态与实施记录。
- P1：`docs/overview.md`（模块地图里 Antigravity 的集成产物，生命周期与插话 / Stop 能力表里「Antigravity 不支持」的几处）、[antigravity.md](../specs/antigravity.md)（§4、§5–§7 被取代的标注）、[MCP 模式](../specs/mcp.md)（`_meta` 会话绑定）、wiki 中英文。
- P2：overview「IPC 协议号」历史与 `daemon-graceful-drain.md` §8；`overview-configuration.md`；[iOS 模型](../specs/ios-task-model.md)（来源表与启动表加 Antigravity）、[GUI 新建任务](../specs/gui-agent-task-launch.md)（G5 不再排除）、[IM 创建任务](../specs/im-agent-task-launch.md)。
- P3：[Stop 确认](../specs/agent-stop-confirmation.md)、[插话](../specs/agent-interject.md)、[结果与恢复](../specs/agent-lifecycle-results-and-recovery.md)、[观察最新正文](../specs/agent-observation-latest-text.md)。
- 开工后在 `docs/PROGRESS.md` 登记未完成项（V1、V13、V16 等长期未验证项与 Linux），做完一项删一项（规格 P15）。

## 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| Stop 续跑上限很小（V2） | 门 G3 先测；若小，另行设计降级并经你确认，**不预设** |
| 全局 sidecar 热注册依赖 App 行为 | 实测约 10 秒；等待是事件驱动的上限而非轮询；超时回退 CLI 或报错；版本不在已验证范围时状态标「未验证版本」 |
| `config.json` 与 App 并发写 | 读-改-写 + 原子 `rename` + 只动自己的键 + 重试至多 3 次；假 App 写入者测试；任何失败都不强写 |
| 更新插件文件打断在途 `ask`（V14） | 门 G1 实测（工作区级插件）：不重启 server、不打断在途调用；仍只重写内容变了的文件；全局插件目录留门 G2 验证（V22）；`mcp_config.json` 的改动要等 server 重启才生效，回执里写明 |
| 带子 Agent 的会话：根的最后一个 `Stop` 可能是 `fullyIdle:false`（L4）、子会话认不出来（L3） | **D17 待用户决定**：P1 先按「每个会话独立」写、不接线根 / 子判断；定案后补归并、完成判定与回归测试（T2 时序）；P3 的 Stop 卡依赖它 |
| 用户取消不产生事件 | 沿用 30 分钟「工作中」兜底，不新增轮询（规格 L1） |
| YOLO sidecar 是长期存在的宽授权项目 | 显示名带「YOLO」、设置页标注、一键移除；不加二次确认是用户的决定（D14） |
| P2–P3 期间没有 YOLO | 固定 YOLO 时不可启动并说明原因，不静默降级；P4 删除过渡规则 |
| AskHuman 被手动删除后 sidecar 残留 | 设置页与命令行「移除全部」；`doctor` 列出命令路径已不存在的 sidecar |
| 回退到旧版 AskHuman | 旧版不认识插件目录（看到模式 None），Antigravity 在旧版下暂无协议规则，需在旧版里重新选模式；插件目录被忽略，不冲突 |
| 宿主文档化契约变化（hooks 字段、`agentapi` 参数） | 解析一律宽松取需要的字段、其余忽略；夹具测试发现；版本探测标「未验证版本」 |
| 实测计费 | 每个门先报预算并批准；只用 Flash 低价档与一句话任务；上限见「验证门与预算」 |
| 并发改同一个 spec 文件（编辑中曾出现来源不明的段落） | 编辑前先重读、用精确匹配替换；发现异常即报告；见规格 §20 |

## 不做

F 权限接管与 Fork（规格 §13）；H 上下文恢复 Hook 与原生 `ask_question` 接管；Antigravity IDE / 扩展、Linux、Windows；IM `/new` 的模型步骤（D16，以后另开一项）；读取私有 SQLite 的其它用途（原生标题等；P4 的目录种子除外）；任何私有 LS RPC。

## 实施记录

- 2026-11-16：计划获批（主工作区；过渡规则：固定 YOLO 时不可启动）。
- 2026-11-16 基线（HEAD `7e5887cc`，主工作区含另一个会话的未提交 Cursor Desktop 改动）：`machine-health` ok；`cargo fmt --check` 通过；`cargo clippy --all-targets -D warnings` 通过（只有 vendored `tao` 的警告）；`cargo test` 3103 通过、0 失败、12 忽略。
- 2026-11-16 门 G1 验证计划经用户批准（回答：只用 Antigravity、不需要逐项授权、严禁使用 Cursor；批准后马上做）。
- 2026-11-16 门 G1 完成：4 个真实任务、约 31 次 Flash 低档请求（T2 24 次，超出估计，原因见规格附录 A.2）；V3-a、V5、V6、V14 已验，V12 部分已验；临时 sidecar / 项目 / 会话 / 插件 / MCP 进程已清理（`config.json` 除该 sidecar 的键外与实验前相同）。
  **发现设计缺口**：根会话的最后一个 `Stop` 可能是 `fullyIdle:false`、之后没有 `fullyIdle:true`（规格 §4.1、§8 L4），子会话认不出来（L3）。**D17 待用户决定**（报告时用户已开启离开模式，未答复）。
  **意外**：T1c 的 `agy` 会话遵守用户的全局 AskHuman 规则，真的调用了 `AskHuman --whats-next`，在用户的 AskHuman 历史里留下 1 条 `away` 记录（规格附录 A.2）；今后 `agy` 实验的提示必须写明「不要运行 AskHuman」。
- 2026-11-16 规格与计划文档提交为 `864aacad`（只含这两个文件，本地、未推送）。主工作区此时干净（另一个会话的改动已由它自己提交为 `bcf77897`）。**尚未写代码；下一步开工 P1。**
  已更新：规格 §1.2、§4.1–§4.3、§4.6、§6.3、§7–§9、§17（V3 / V5 / V6 / V12 / V14 更新，新增 V22 / V23）、§18、附录 A.2、§20；本计划的硬约束 2、验证门、P1 步骤 1 / 5 / 8–9、风险表。
