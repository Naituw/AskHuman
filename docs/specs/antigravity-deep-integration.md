# Antigravity 深度集成：生命周期、Stop 确认、插话、Watch 与手机 / IM 新建任务

> 状态：2026-11-16 起草，经用户两轮审阅后**规格已定**：§3.2 的默认决策整体确认（P1–P11、P13、P15），其余由 D11–D16 确认或取代（§3.1、§20）。
> 实施计划（`docs/plans/antigravity-deep-integration.md`）已批准；门 G1 的真实验证已完成（附录 A.2），暴露的子会话设计缺口 **D17 待用户决定**（§8 L4）。
> 本文只含规格与调研结论，不含代码，也不改变现有行为。
> 范围：Antigravity App（macOS）与 Antigravity CLI（`agy`）。
> 落地后取代 [antigravity.md](antigravity.md) 的 §4（无可信会话身份）、§5–§6（共享规则所有权与模式切换，改为插件所有权）与
> §7（支持边界）；该文其余约束（三模式互斥、MCP `timeoutSeconds`、配置文件的链接与所有者策略）继续有效。
> 参照：[生命周期追踪](agent-lifecycle-tracking.md)、[Stop 结束确认](agent-stop-confirmation.md)、
> [插话](agent-interject.md)、[观察最新正文](agent-observation-latest-text.md)、[IM 创建任务](im-agent-task-launch.md)、
> [Cursor Desktop 新建任务](cursor-desktop-task-creation.md)（App / CLI 双目标与对称回退的先例）、
> [Grok CLI 调研](grok-cli-integration-research.md)（调研型规格的格式先例）。

## 1. 目标与边界

现状（[antigravity.md](antigravity.md) §7）：Antigravity 只有提问（CLI 或 MCP）和共享规则，没有生命周期、Stop 确认、插话、Watch、新建任务。
本规格把它补到与其他已适配 Agent 同一水平，范围是用户 2026-11-15 经 AskHuman 选定的 A–E 与 G：

| 编号 | 能力 | 结果 |
| --- | --- | --- |
| A | 可信会话身份 | `show-last`、历史、去重按真实会话工作，不再「按项目保守拒绝」 |
| B | 生命周期追踪 | Agent 控制台、状态与结果状态 |
| C | Stop 结束确认 | Agent 一轮真正结束时在弹窗 / IM 里确认，可让它继续 |
| D | 插话 | 工作中追加消息；空闲时发消息把它叫醒 |
| E | Watch / IM 实时画面 | 解析会话 transcript，给出最新正文与工具 |
| G | 新建任务 | 手机 / IM `/new` 在 App 或 CLI 里创建并自动开始；手机与 Mac 新建任务窗口可选模型（D11） |

用户对 G 的原话（2026-11-15）：「我希望至少能实现我在 IM 或者是手机端能创建任务，这个是很重要的。」
**结论：只靠公开契约就能做到**——App 路径用 sidecar + `agentapi new-conversation`（官方文档化，§4.3），CLI 路径用 `agy -i`（§4.4），
不需要私有接口。**更正**：此前我说 Antigravity「没有公开的新建任务契约」，进一步调研后该说法作废。

### 1.1 非目标

- **F 权限接管与 Fork 只记调研结论**（§13）。用户 2026-11-15 不批准私有接口，因此不建私有 LS RPC 适配层。
- H 上下文恢复 Hook、原生 `ask_question` 接管：未被选中，不做。
- Antigravity IDE / 扩展、Linux、Windows：本版不声明支持（§14）。
- 不做轮询或延时重试式补丁（AGENTS.md「No workaround patches」）：例如用户取消不产生 Stop 导致的状态滞后，沿用既有「工作中兜底」，
  不新增轮询 Antigravity 内部状态的机制。

### 1.2 研发与验证约束

启动真实 Antigravity Agent、发送 prompt 或其它会计费的测试，原先必须先经 AskHuman 明确批准（沿用 [IM 创建任务](im-agent-task-launch.md) D27）。
**2026-11-16 起放宽（用户在门 G1 验证计划的回答里）**：只用 Antigravity 的真实测试不再逐项授权，但仍受各门的任务数预算、隔离与停止条件约束；
退出用户的 App、改动会影响用户真实会话的全局插件目录、对真实目录开宽授权（YOLO）这类超出预算与隔离的动作仍先经 AskHuman 问。
**严禁**在本任务的验证里使用 Cursor（不启动真实 Cursor Agent，不调用 Cursor Desktop 的冒烟脚本）。
实测用低价档 `gemini-3.8-flash-low`；调研与 G1 的用量与还原情况见附录 A。

## 2. 证据口径与实测环境

标注含义：**〔实测〕** 本机观察；**〔文档〕** Antigravity 内置或公开文档；**〔私有〕** 私有接口 / 文件格式，只记录、不依赖；**〔未验证〕** 没验证过（编号 V*，见 §17）。

环境：macOS（darwin 27.2.0）、Antigravity App 2.32.1（`ANTIGRAVITY_LS_VERSION=hub-2.32.1`）、`agy` 1.5.9、已登录账号。
实验在临时目录与临时 sidecar / 插件 / hooks 里做，每个实验后把 `~/.gemini` 还原；不读取用户自己会话的内容。

## 3. 决策

### 3.1 已确认（用户经 AskHuman 定案，2026-11-15）

| 编号 | 决策项 | 结论 |
| --- | --- | --- |
| D1 | 能力范围 | A 可信身份、B 生命周期、C Stop 确认、D 插话（含空闲消息）、E Watch、G 新建任务；F 与 Fork 只记调研结论；H 不做 |
| D2 | 产品范围 | App 与 CLI（`agy`）都用；二者共用同一份全局 hooks / 插件 / MCP 配置〔实测〕 |
| D3 | 打包 | 一个 **AskHuman 插件**（规则 + MCP + hooks 一个目录装齐，可在 Antigravity 里整体启停）；旧共享规则文件与全局 MCP 条目升级时迁移掉（§6） |
| D4 | 私有接口 | **不批准**。F 与 Fork 不实现；手机新建任务用 sidecar 预授权白名单，未授权动作自动拒绝（§12.4） |
| D5 | 目录粒度 | v1 支持多个常用目录，每个目录一个 sidecar，动态注册。实测修正前提：**全局 sidecar 热注册，不需要重载插件**（§12.3） |
| D6 | 默认目标 | App 优先；App 整体不可用而 CLI 可用时自动改用 CLI，反之亦然；偏好只决定优先级；任务一旦开始不静默改道（§12.1） |
| D7 | 权限 | 白名单 + 可选 YOLO：每个目录默认只预授权 AskHuman 自己的 MCP，用户在设置页按目录追加规则；YOLO 用另一组宽授权 sidecar，沿用 `/new` 的权限卡与全局设置（§12.4）。第三轮选项里的「且二次确认」是我写的措辞、不是用户的话，已由 D14 取消 |
| D8 | sidecar 创建 | 按需自动：`/new` 选到某个最近目录时才创建；先在设置里一次性打开「允许 AskHuman 管理 Antigravity sidecar」；设置页可查看与移除（§12.3、§12.10） |
| D9 | App 未运行 | 自动启动 App，等 sidecar 连上（最多约 30 秒），失败再回退 CLI 或报错（§12.5） |
| D10 | 实测 | 可用本机 App 与 `agy` 实测，但控制用量，优先 Flash 低价档 |
| D11 | 模型（2026-11-16） | v1 就接入「新建任务模型与强度」管线（[ios-task-model](ios-task-model.md)），**手机端可选 Antigravity 模型**；Mac 新建任务窗口因共用同一管线一并获得（§12.9）。用户原话选项：「v1 就接入「新建任务模型与强度」档位，手机端可选」 |
| D12 | 分期顺序（2026-11-16） | P1 基础 → P2 手机 / IM 新建任务 + Watch → P3 Stop 确认 + 插话 → P4 CLI 路径 + YOLO（P14 的默认顺序，用户选定） |
| D13 | `agy` 去留（2026-11-16） | 保留 `~/.local/bin/agy`（V3、V12、V15 与 CLI 路径验收还要用） |
| D14 | YOLO 确认（2026-11-16） | **不加二次确认**：只用 `/new` 现有的权限卡（YOLO 项自带危险样式与说明）；YOLO sidecar 的显示名带「YOLO」，设置页标注宽授权并可一键移除（§12.4） |
| D15 | 模型清单形态（2026-11-16） | 平铺列表（同 Cursor）：`agy models` 的全部条目，超过 12 项手机端自带搜索，不出强度行；不从 id 后缀猜「模型 + 档位」分组（§12.9） |
| D16 | IM 的模型步骤（2026-11-16） | 沿用现状：IM `/new` 不选模型（所有 Agent 一致），IM 创建的 Antigravity 任务用默认档；给所有 Agent 的 IM 加模型步骤以后另开一项（§12.9） |

### 3.2 默认决策（我据调研提出）

2026-11-16 用户整体确认：**P1–P11、P13、P15 按默认**；P12、P14、P16、P17 已分别转为 D15、D12、D14、D16。

| 编号 | 默认 | 理由 / 位置 |
| --- | --- | --- |
| P1 | 插件默认装 **MCP 变体**（CLI 变体仍可选） | App 默认沙箱挡住 CLI 模式连 daemon socket（§4.5），MCP 在 App 与 `agy` 都能用；代价是首次要在 App 里点一次「始终允许」（§6.4） |
| P2 | 最低版本 App ≥ 2.32.1、`agy` ≥ 1.5.9 | 均为实测版本；更低版本不承诺，状态显示「未验证版本」（§14） |
| P3 | hooks 只装 `SessionStart`、`PreInvocation`、`Stop` | 工具级事件不增加信息：每个工具结果之后必有一次 `PreInvocation`（§8） |
| P4 | 回合开始 = `PreInvocation` 且 `invocationNum == 0`；回合结束 = `Stop` 且 `fullyIdle == true` | `fullyIdle:false` 表示后台任务还在跑，Agent 会自行醒来（§4.1、§8） |
| P5 | 会话结束没有事件：`agy` 用 hook 父进程做存活判断，App 记 `pid=None` 走 1 小时 TTL 与 30 分钟「工作中」兜底 | §8；用户取消不产生 Stop，状态最多滞后到兜底 |
| P6 | Stop 确认卡只在 `terminationReason == NO_TOOL_CALL`、`fullyIdle == true`、根会话时出；`ERROR` 走最终失败卡，`MAX_*` 走「未完成」，其余静默 | §9 |
| P7 | 工作中插话走 `PreInvocation` 的 `injectSteps.userMessage`；空闲 / 已结束走 sidecar 的 `agentapi send-message`；**绝不**对工作中会话调 `send-message` | §10 |
| P8 | Watch 读 `transcript_full.jsonl`（只读、有界尾读），不读私有 SQLite；没有公开思考，只显示正文与工具 | §11 |
| P9 | sidecar 命名 `askhuman-ws-<目录哈希>` / `askhuman-yolo-<目录哈希>` / `askhuman-ctl`；总数上限 20 | §12.3 |
| P10 | 移除 sidecar 只删 AskHuman 自己写的 sidecar 目录与 `config.json` 里的同名键；App 生成的 `projects/<id>.json` 与 `sidecar_data/<id>` 不删 | 保留任务历史归属（§12.3） |
| P11 | `config.json` 只动 `sidecars.askhuman-*`，读-改-写 + 同目录临时文件原子 rename，保持 0600 与未知键；不碰 `plugins.*`、`userSettings`、权限授予 | Antigravity 自带的 plugin skill 明确警告手改会覆盖这些；非原子写入实测让 LS 记了解析失败（§4.3） |
| P12 | Antigravity 的模型清单 = `agy models` 的**平铺列表**，档位折在 id 里、不出强度行（与 Cursor 同口径 M8）；不从 id 后缀猜「模型 + 档位」分组；没装 `agy` 的机器退回 `agentapi` 文档公开的三个别名 | 已确认（D15）；§12.9 |
| P13 | 本版只在 macOS 验收；Linux 待实测后放开；Windows 不支持（沿用） | §14 |
| P14 | 分期 P1 基础 → P2 手机 / IM 新建任务 + Watch → P3 Stop 确认 + 插话 / 空闲消息 → P4 CLI 路径 + YOLO + 冷启动种子 | §16；按「新建任务最重要」排序；已确认（D12） |
| P15 | 本规格确认前不改 `docs/overview.md`、其他 spec 与 `docs/PROGRESS.md`；确认后随实施计划登记 | §19 |
| P16 | YOLO 不加二次确认，只用 `/new` 权限卡；sidecar 显示名带「YOLO」，设置页标注宽授权并可一键移除 | 已确认（D14）。别的 Agent 的 YOLO 是一次性启动参数，而这里会在 App 里留下长期存在的宽授权项目，所以用显示名与设置页标注来缓解（§12.4） |
| P17 | IM `/new` 沿用现状：没有模型步骤（所有 Agent 一致），IM 创建的 Antigravity 任务用 Antigravity 的默认档 | 现状见 [ios-task-model 已知局限 7](ios-task-model.md)；App 路径的默认实测是 Flash 族的一档，可能弱于你在 App 里常用的模型；已确认（D16，§12.9） |

## 4. 宿主契约（调研事实）

### 4.1 Hooks

配置与运行：

- `hooks.json` 顶层键是 hook 名（AskHuman 用 `askhuman`），值按事件分组；`SessionStart` / `PreInvocation` / `Stop` 是扁平处理器列表，
  `PreToolUse` / `PostToolUse` 需要 `matcher` 分组。处理器 `command` 经 `sh -c` 运行，`timeout` 单位秒、缺省 30。〔文档〕
- 同一事件的多个来源（全局、插件、工作区）合并后**顺序同步**执行，会阻塞 Agent 循环。〔文档〕
- 全局 `~/.gemini/config/hooks.json` 在 App 与 `agy` 上都触发；插件根的 `hooks.json` 在 App 上触发且环境带 `PLUGIN_ROOT`。〔实测〕
  hook 进程的环境另有 `ANTIGRAVITY_CONVERSATION_ID`（与载荷的 `conversationId` 一致）和 `AGY_BROWSER_*`（浏览器调试地址，含会话级秘密，AskHuman 不读不记）；
  父进程是 App 共享的 `language_server`。〔App 实测，G1〕
- `agy` 上，**工作区级**插件的 hooks 只在工作区被信任时才载入：不信任时 CLI 日志是 `hooks_manager.go:54] loaded 0 named hooks from 0 hooks.json file(s)`，
  用 `--settings '{"trustedWorkspaces":[…]}'` 信任后变成 `loaded 1 … from 1 …`，**并且会话里真的触发**（T1c：`SessionStart`、`PreInvocation` ×3、`Stop(NO_TOOL_CALL, fullyIdle:true)`）；
  这行日志是免费的载入诊断。〔`agy` 实测，G1〕
  信任列表按**符号链接解析后的路径**比较：macOS 上写 `/tmp/x` 不生效（工作目录被解析成 `/private/tmp/x`，日志仍是 `loaded 0 …`），要写 `/private/tmp/x`。〔`agy` 实测，P1〕
  `agy` 的 hook 进程：父进程是该会话自己的 `agy`；环境带 `ANTIGRAVITY_CONVERSATION_ID`、`ANTIGRAVITY_LS_ADDRESS`、`PLUGIN_ROOT`；`transcriptPath` 在 `~/.gemini/antigravity-cli/brain/<会话>/.system_generated/logs/`；
  `Stop` 触发时 transcript 已含最终回复。〔`agy` 实测，G1 T1c〕
  **全局**插件：隔离 HOME 里 `agy plugin list` 能识别 AskHuman 生成的全局插件（`askhuman (enabled)`，`1 rule, 1 MCP server, 1 hook`）；它的 hooks 在**会话**里是否触发没验——
  P1 的会话实测用的是同内容的工作区级副本（V3、V22；附录 A.3）〔未验证〕。
  `agy plugin disable askhuman` / `enable askhuman` 写 `config.json` 的 `plugins.askhuman.enabled`（`false` / `true`），AskHuman 的只读检测读的正是这个键。〔`agy` 实测，P1〕
- `timeout: 86400` 被接受，阻塞 40 秒被如实等待。〔实测〕整 24 小时〔未验证〕。

载荷（stdin，camelCase）：公共字段 `conversationId`、`workspacePaths`、`cwd`（Agent 的工作目录，不是 hook 进程的）、`transcriptPath`、
`artifactDirectoryPath`、`modelName`、`lastUserInput`、`hookEventName`；`PreInvocation` / `PostInvocation` 另有 `invocationNum`、
`initialNumSteps`；工具类事件另有 `stepIdx`、`toolCall{id,name,args}`；`Stop` 另有 `terminationReason`、`fullyIdle`、`executionNum`、
`continuationActive`、`error`。〔实测，与文档一致〕`transcriptPath` 的目录区分产品：`antigravity/`（App）、`antigravity-cli/`（CLI）、
`antigravity-ide/`（IDE）。〔文档〕

事件语义（均〔实测〕）：

- **没有 SessionEnd。**
- `invocationNum == 0` 是一轮的第一次模型调用。后台任务完成后 Agent 自己醒来的那一轮同样从 0 开始，但 `lastUserInput` 沿用上一次用户输入（已过期）；同一轮内递增。
- 长 `run_command` 约 10 秒后自动转后台。模型回合可能先以 `Stop(NO_TOOL_CALL, fullyIdle:false)` 结束，任务完成后 Agent 自行醒来
  （`PostToolUse` → `PreInvocation(0)` → `PostInvocation` → `Stop(fullyIdle:true)`）。所以**真正完成 = `Stop` 且 `fullyIdle == true`**——
  对没有子 Agent 的会话成立；有子 Agent 时不一定成立，见后两条。
- **子 Agent（`invoke_subagent`）是独立会话，同样触发 hooks。**子会话用自己的 `conversationId` 走 `SessionStart` / `PreInvocation` / `Stop`，
  载荷里**没有**父会话字段，`lastUserInput` 是空串（它从未收到用户输入，第 0 步是父会话发来的 `SYSTEM_MESSAGE`）。〔App 实测，G1 T2〕
- **根会话的最后一个 `Stop` 可能是 `fullyIdle:false`，之后再没有 `fullyIdle:true` 的根 `Stop`。**T2 的时序：根 `Stop(false)` 02:07:46.751（子会话在跑）→
  子会话 `send_message` 给根，根被唤醒 `PreInvocation(0)` 02:07:53.766 → 根 `Stop(false)` 02:07:56.246 → 子会话 `Stop(true)` 02:07:56.551（晚 0.3 秒）。
  最后一条 `fullyIdle:true` 来自**子会话**，根会话没有。子 Agent 通常把 `send_message` 当作最后一个工具调用，随后还要一次模型调用才结束，
  与父会话被唤醒后的那次模型调用几乎同时结束，所以这是竞态，不是个例。〔App 实测一次，时序依赖，G1 T2〕
- 用户取消（对会话调用 `CancelCascadeInvocation`，两次）**不产生 Stop，也不产生 PostInvocation**，会话直接变为空闲。文档列出的 `USER_CANCELED` 从未观察到。
- `terminationReason` 只观察到 `NO_TOOL_CALL`；其余取值来自文档：`MAX_INVOCATIONS`、`MAX_FORCED_INVOCATIONS`、`MAX_TOKEN_BUDGET_EXCEEDED`、
  `ERROR`、`USER_CANCELED`、`TERMINAL_STEP_TYPE`、`TERMINAL_CUSTOM_HOOK`、`INJECTED_RESPONSE`。〔文档〕

输出语义（均〔实测〕）：

| 事件 | 空 stdout + 退出码 0 | 其他输出 |
| --- | --- | --- |
| 全部六个事件 | 无操作 | — |
| `PreToolUse` | 无操作 | `{}` 被记为 **deny**；`{"decision":"allow"}` 才放行；非零退出让该次工具调用失败 |
| `PreInvocation` | 无操作 | `{"injectSteps":[{"userMessage":"…"}]}` 注入一条用户消息（transcript 里是 `USER_INPUT`，来源 `SYSTEM_SDK`）；非 JSON 杂项输出被忽略 |
| `Stop` | 无操作（会话正常结束） | `{"decision":"continue","reason":"…"}` 让会话继续，`reason` 以系统消息注入（transcript 里是 `SYSTEM_MESSAGE`），`executionNum` 递增；退出码 1 被忽略 |

`allow`、`permissionOverrides`、`overwrite` 都不会预先批准原生权限提示（§13）。
续跑次数有上限：执行器配置里有 `max_stop_hook_continuations`（来自 LS 二进制字符串，**未见文档**，默认值未知，〔私有〕，V2）。

### 4.2 MCP、会话元数据与进程环境

- MCP `tools/call` 的 `params._meta` 带 `antigravity.google/conversation_id`、`…/root_conversation_id`、`…/artifacts_dir`、`progressToken`。〔App 实测〕
  子会话的调用另带 `…/parent_conversation_id`：`conversation_id` 是子会话自己，`parent_conversation_id` 与 `root_conversation_id` 指向父 / 根（T2 里两者相同）。〔App 实测，G1〕
  `agy` 的同类字段〔文档：CLI 1.4.4 起带 root id；未实测，V5〕。
- 插件里的 MCP server **以工作区为单位启动，不随会话结束**：T2 的会话一开始就启动，T3 的新会话复用同一个进程（父进程是共享的 `language_server`）。
  `initialize` 里宿主自称 `antigravity-client` v1.0.0、协议 `2025-11-25`，并声明 `elicitation`（form / url）能力。server 启动时宿主写
  `~/.gemini/antigravity/mcp/<插件>_<server>/<tool>.json`（工具发现文件），server 退出后文件仍留着。〔App 实测，G1〕
- 宿主用通用工具 `call_mcp_tool{ServerName, ToolName, Arguments}` 调 MCP；工具靠 `~/.gemini/antigravity/mcp/<server>/<tool>.json` 被发现；
  插件里的 MCP server 以 `<插件名>_<server>` 命名。〔实测〕
- `mcp_config.json` 里每个 server 的 `timeoutSeconds`：400 秒的阻塞调用成功。〔实测〕缺省超时与整 24 小时〔未验证，V4〕。
- App 里 MCP 调用默认走「询问」，除非有 `mcp(server/tool)` 授权；原生提示是会话里的 WAITING 步骤，hooks 看不到。〔实测〕
- `run_command` 子进程环境：`ANTIGRAVITY_AGENT=1`、`ANTIGRAVITY_CONVERSATION_ID`、`ANTIGRAVITY_TRAJECTORY_ID`、`ANTIGRAVITY_SOURCE_METADATA`
  （JSON，含该命令所属会话 ID、步骤序号与工具调用）、`ANTIGRAVITY_LS_VERSION`、`ANTIGRAVITY_AGENTAPI_EXE`、`ANTIGRAVITY_APP_DATA_DIR`。〔App 实测〕
  另有 LS 地址与 CSRF 令牌：AskHuman 不使用、不记录。
- `agentapi get-conversation-metadata <id>` 返回 `parentConversationId`、`rootConversationId`、`nestingDepth`、`projectId`、`workspaceUris`、
  `sourceMetadata.sidecar.sidecarId`、`createdAt`、`tags`、`subagentSpec`；**没有运行状态字段**。〔实测〕

### 4.3 Sidecar 与 agentapi（App，公开契约）

- 清单 `sidecar.json`：`command`（或 `builtin`）、`args`、`env`、`restart_policy`、`display_name`、`description`、
  `agent_permissions{workspace_uris, access_grants, allows_user_interactions}`。位置：全局 `~/.gemini/config/sidecars/<id>/`，或插件内 `plugins/<插件>/sidecars/<id>/`。〔文档〕
- 启用写在 `~/.gemini/config/config.json` 的 `sidecars.<id>.enabled`。〔文档〕**全局 sidecar 在写入后约 10 秒内热注册，不需要重载或重启 App。**
  改 `sidecar.json` 会重启该 sidecar 并更新授权；删除目录或停用会向它发 SIGINT（约 5 秒后停止；LS 日志 `Sidecar <id> removed from directory, stopping...`，
  G1 里进程约 10 秒内退出）。遗留物：`config.json` 里的条目、`projects/<id>.json`、`sidecar_data/<id>/{logs,events,data}`，
  以及该项目下会话的文件与私有摘要库里的条目。〔实测〕插件 sidecar 另需 `plugins.<插件>.enabled`〔实测，本规格不用〕。
  清理时先删清单目录让它停下，再删 `config.json` 里的键，最后才删项目文件：反过来 LS 会在日志里记一次 `missing project file`（无害，一次性）。
- 每个 sidecar 对应 App 里一个「项目」，名字取 `display_name`；App 启用后会在 `config.json` 条目里补 `projectId`、`resolvedPermissions`。
  重建同名**插件** sidecar 时 App 会「认领孤儿项目」（LS 日志）〔实测〕；全局 sidecar 是否同样认领〔未验证，V8〕。
- sidecar 进程不在沙箱里：环境带 `ANTIGRAVITY_PROJECT_ID`、`ANTIGRAVITY_AGENTAPI_EXE`、`ANTIGRAVITY_LS_ADDRESS`，PATH 里有 `agentapi`，
  能连 `~/.askhuman/daemon.sock`。〔实测〕
- `agentapi` 只有三个命令：`get-conversation-metadata <id>`、`new-conversation [--model=<flash_lite|flash|pro|model_id>] [--title=] [--profile=] <prompt>`、
  `send-message [--title=] <recipient_id> <content>`。`new-conversation` **立即**返回 `{"response":{"newConversation":{"conversationId":"…"}}}`；
  会话的工作区就是该 sidecar 的 `workspace_uris`；不传 `--model` 时实测用的模型是占位 ID M322，与显式指定的 `gemini-3.8-flash-low`（M320）同属 Flash 族
  （LS 二进制里二者映射到同一个基础模型〔私有〕），据此推断是 Gemini 3.8 Flash 的另一个档位，而不是 `agy models` 清单里排在前面的 Gemini 4 / Claude 系列。〔实测 + 推断〕
  调用 `agentapi` 本身不需要任何 `access_grants`。〔实测〕
  `agentapi` 离开 sidecar 的环境就不能用：直接在终端运行 `agentapi get-conversation-metadata <id>` 得到 `Unauthenticated: missing CSRF token`
  （sidecar 环境里带着宿主发的受限令牌，AskHuman 不读不记）。`ANTIGRAVITY_AGENTAPI_EXE` 指向的是 `language_server` 本体，要用 PATH 里的 `agentapi`。
  sidecar 事件文件 `sidecar_data/<id>/events/*.json` 里的 `commandInvocationTimestampMs` 是 sidecar **进程的启动时间**（同一进程的各事件值相同），不是每条命令的时间。〔实测，G1〕
- 任意 sidecar 的 `send-message` 能唤醒**另一个 sidecar 创建的**空闲会话（跨项目）。〔实测〕用户自建会话与 `agy` 会话〔未验证，V11〕；对工作中会话的语义〔未验证，V10〕。
- `allows_user_interactions:false` 时，未被授权的动作自动拒绝。〔实测〕宽授权（`command(*)`、`read_file(*)`、`write_file(*)`、`read_url(*)`、`mcp(*)`）的 sidecar
  无人值守跑通了一个联网命令。〔实测〕
- `config.json` 同时被 App 写入。用**非原子**方式（截断再写）改它，LS 日志出现过两次瞬时的 `failed to parse user config`。〔实测〕
  Antigravity 自带的 plugin skill 也警告：手改 `config.json` 会覆盖 `userSettings` 与权限授予；它对插件启停建议走 UI。〔文档〕
  这个文件里确实有用户的授权：用户在 App 里对 `command(AskHuman)`、`command(ls)` 点「始终允许」后，`config.json` 多出
  `userSettings.globalPermissionGrants.allow` 数组（私有格式）。〔实测，来自用户自己的操作〕所以任何写入都必须读-改-写，绝不能用旧备份整文件覆盖。

### 4.4 CLI（`agy` 1.5.9）

- 官方安装脚本把 `agy` 放到 `~/.local/bin/agy`（校验 SHA512），并往 `~/.zshrc` 追加 PATH 行。登录凭据与 App 共用（keychain）。〔实测〕
- 读取同一份全局 `~/.gemini/config/{hooks.json, mcp_config.json, plugins/}`；数据在 `~/.gemini/antigravity-cli/`（含同格式的 `transcript_full.jsonl`）；
  LS 地址与 App 是同一个 hub。〔实测〕
- `agy -i "<任务>"` 以交互 TUI 启动并执行初始任务；`--dangerously-skip-permissions` 即 YOLO；另有 `--mode`、`--settings <json>`（仅本次会话）、`--model`。〔文档 + 实测〕
  首次交互运行先显示引导（配色、数据使用同意），`-i` 的任务仍会执行。AskHuman 不替用户完成同意。
- hook 的父进程就是该会话自己的 `agy` 进程（可做存活判断）；hooks 继承启动它的环境，launch-id 环境变量能传到 hook。〔实测〕
- CLI 的终端沙箱默认关闭，`run_command` 能连 daemon socket 并运行 AskHuman。〔实测，`--dangerously-skip-permissions` 下〕
- `agy acp`（ACP 服务）与交互里的 `/fork` 存在。〔文档〕取消 / Ctrl-C 的 hook 行为〔未验证，V3〕。
- **`agy models`** 只列清单、不起会话：stdout 每行「id、制表符、显示名」（本机 18 行，如 `gemini-4-argon-high` ↔ `Gemini 4 Argon (High)`、`gemini-3.8-flash-low` ↔ `Gemini 3.8 Flash (Low)`、
  `claude-sonnet-5-5-medium` ↔ `Claude Sonnet 5.5 (Medium)`、`gpt-oss-120b-medium` ↔ `GPT-OSS 120B (Medium)`），stderr 一行 `Fetching available models...`，没有「默认」标记，约 4 秒，会联网取清单。〔实测〕未登录时的表现〔未验证，V21〕。
  强度档位（Low / Medium / High / XHigh）已折在 id 与显示名里，没有独立的强度开关；个别条目不符合这个规律（`gemini-pro-orchestrator-g1-pro`、只有 Medium 的 `gpt-oss-120b-medium`）。〔实测〕
  清单是否随账号与套餐而变〔推断，未验证〕。运行任何 `agy` 命令（包括 `agy models`）都会建 `~/.gemini/antigravity-cli/`（缓存、日志、内置技能副本）与 `~/.gemini/config/projects/default-cli-project.json`。〔实测〕
  hook 载荷的 `modelName` 与这里的 id 是同一命名空间（实测值 `gemini-3.8-flash-low`），`agentapi --model=gemini-3.8-flash-low` 被接受。〔实测〕`agy --model <id>` 的写法与效果〔未验证，V20〕。

### 4.5 沙箱与权限

- **App 的默认终端沙箱挡住 `ps`，也挡住 `run_command` 连 `~/.askhuman/daemon.sock`**：CLI 模式的 AskHuman 在 App 里要么失败，要么请求 `BypassSandbox`（每次需本机批准）。
  MCP server、hooks、sidecar 都是宿主起的非沙箱进程，不受影响。〔实测〕
- 授权语法 `action(target)`：`command`、`read_file`、`write_file`、`read_url`、`execute_url`、`download_url`、`mcp`；`mcp(server/tool)`、`mcp(server/*)`、`mcp(*)`；
  `(*)` 通配整个 action；`write_file` 蕴含 `read_file`；`read_url` 的域名会编进沙箱出站白名单，`read_file` / `write_file` 路径进沙箱文件系统白名单。〔文档〕
- App 预设 Request Review / Default / Auto / Turbo；sidecar 项目继承全局预设。〔文档〕

### 4.6 transcript 与本地数据

- 路径：App `~/.gemini/antigravity/brain/<conv>/.system_generated/logs/transcript_full.jsonl`；`agy` 在 `~/.gemini/antigravity-cli/brain/...`。
  另有精简版 `transcript.jsonl`（参数全是字符串）。〔实测〕
- 一行一个步骤：`step_index`、`source`（`USER_EXPLICIT` / `MODEL` / `SYSTEM` / `SYSTEM_SDK`）、`type`（`USER_INPUT`、`PLANNER_RESPONSE`、
  `GENERIC`（工具结果）、`SYSTEM_MESSAGE`）、`status`、`created_at`、`content` 或 `tool_calls[{name,args}]`、token 计数。
  模型的正文是 `PLANNER_RESPONSE.content`（可与 `tool_calls` 并存）；用户输入包在 `<USER_REQUEST>` 里；PreInvocation 注入的用户消息来源是 `SYSTEM_SDK`
  （`send-message` 注入的消息来源未核对）。〔实测〕
- 没有观察到公开的思考文本（样本模型是 Flash 低档）。〔实测；高思考模型〔未验证，V17〕〕
- 每个产品各有私有的 `conversation_summaries.db`（标题、预览、步数、状态、workspace…）。〔私有〕本规格最多只读用于「最近目录冷启动」（§12.6），不依赖它。
  G1 里只读看了表结构：`conversation_summaries` 有 `parent_conversation_id`、`nesting_depth`、`status`（如 `CASCADE_RUN_STATUS_IDLE`）、`not_fully_idle`、`project_id` 等列，
  即 App 自己有「整棵会话树是否空闲」的记录，但它是私有库，D4 不批准依赖；仅记录，供以后若宿主把它公开时参考。

## 5. 总体架构

```text
Antigravity App / agy
   │ hooks: SessionStart / PreInvocation / Stop ─→ AskHuman __agent-hook antigravity <event> ─┐
   │ MCP tools/call（_meta 带会话 ID）────────────→ AskHuman mcp ──────────────────────────────┤→ daemon
   │                                                                                           │  （注册表 / Stop / 插话 / Watch / 任务）
   ▲ agentapi new-conversation / send-message                                                  │
 sidecar（askhuman-ws-* / askhuman-yolo-* / askhuman-ctl）←── 私有通道（sidecar 主动连 daemon）─┘
```

- **插件**（§6）：规则、MCP、hooks。App、`agy` 共用，用户可在 Antigravity 里整体启停。
- **hooks**：只上报与应答，永远 fail-open；是 A、B、C、D（工作中）的来源。
- **sidecar**（§12.3）：全局、按目录；是 G（App 路径）与 D（空闲消息）的执行者。它们**不在插件里**，所以增删 sidecar 从不触发插件重载，也就不会打断正在等待的 `ask`。
- **daemon**：沿用既有注册表、Stop 服务、插话队列、Watch、任务启动；新增 Antigravity 适配（§16.1）。

## 6. 插件、模式与迁移

### 6.1 布局

```text
~/.gemini/config/plugins/askhuman/
  plugin.json        # {"name":"askhuman", …}；是插件的标记文件
  rules/AGENTS.md    # 交互协议（CLI 或 MCP 变体，沿用现有生成器）
  mcp_config.json    # 仅 MCP 变体：mcpServers.mcp = {command, args:["mcp"], timeoutSeconds:86400}
  hooks.json         # 生命周期 hooks（§6.2）
  .askhuman-managed  # AskHuman 的所有权标记
```

宿主会把插件里的 MCP server 命名为 `askhuman_mcp`（`<插件名>_<server>`，实施时以实测为准），授权写作 `mcp(askhuman_mcp/*)`。
手动放进 `plugins/` 的目录无需登记就会被 App 热加载〔实测〕，`agy plugin list` 能列出〔实测〕。
P1 实测：`rules/AGENTS.md` 不带 frontmatter 也被识别——AskHuman 生成的全局插件在 `agy plugin list` 里是 `askhuman (enabled)`、`1 rule, 1 MCP server, 1 hook`；
同一道题（「你的规则里是否要求调用 `ask` MCP 工具」），插件没载入时模型答「否」，载入后答「是」（附录 A.3）。

### 6.2 模式与 hooks 内容

- 三种模式沿用 [antigravity.md](antigravity.md) §1 的互斥语义：**None** 不留插件；**CLI** 装规则（CLI 变体）+ hooks，不装 `mcp_config.json`；
  **MCP** 装规则（MCP 变体）+ `mcp_config.json` + hooks。
- `hooks.json` 按已开启的能力生成：生命周期（含插话）开 → 下面三个事件；只开 Stop 确认 → 只写 `Stop`；都关 → 不写。
  Stop 确认与生命周期共用同一个 `Stop` 处理器（[生命周期 D16](agent-lifecycle-tracking.md)）。
- 若 V3 显示 `agy` 不触发插件里的 hooks，则 hooks 改装到全局 `~/.gemini/config/hooks.json`（已验证对 App 与 `agy` 都生效），按同样的所有权与幂等规则托管；规则与 MCP 仍在插件里。

```json
{
  "askhuman": {
    "SessionStart": [{"type": "command", "command": "\"/abs/AskHuman\" __agent-hook antigravity session-start"}],
    "PreInvocation": [{"type": "command", "command": "\"/abs/AskHuman\" __agent-hook antigravity pre-invocation", "timeout": 86400}],
    "Stop": [{"type": "command", "command": "\"/abs/AskHuman\" __stop-hook antigravity track", "timeout": 86400}]
  }
}
```

（P1 已按此形态落地。`command` 经 `sh -c`；可执行文件路径含引号、`$`、反引号、反斜杠、换行或不是绝对路径时拒绝生成。
`Stop` 用既有的共享处理器 `__stop-hook`（`track` 表示只追踪、不弹确认卡），而不是 `__agent-hook … stop`，这样 Stop 确认与生命周期共用一个处理器（[生命周期 D16](agent-lifecycle-tracking.md)）；
P3 开启确认时这条命令多一个 `confirm` 参数，`hooks.json` 随之重写（内容变了才写；重写不打断在途 `ask`，V14）。`PreInvocation` 要 86400 秒，是因为插话的「composer 打开」会阻塞等待提交（§10.1）；P1 的处理器从不阻塞，插话启用只改 daemon 的裁决、不改这个文件。）

### 6.3 所有权与写入

插件目录是 AskHuman 托管内容：沿用 [antigravity.md](antigravity.md) §5 的所有权判定（路径、摘要、uid 与实例账本；用户改过内容即冲突，不覆盖不删除）和写入纪律
（私有临时文件、原子发布、0600、不跟随符号链接）。多文件的目录级事务需要在实施计划里设计。
**只在内容真的变化时重写**（幂等，也减少与 App 并发写入的机会）。

**V14 实测（工作区级插件，G1 T3）**：一次 90 秒的 MCP 调用进行期间，依次原子重写 `rules/AGENTS.md`、`hooks.json`（给 `Stop` 加 `timeout`）、
`mcp_config.json`（改 `timeoutSeconds`）、`plugin.json`（改描述），每次改后 3 秒与 10 秒各查一次：MCP server 进程没有重启（pid 不变、没有新的启动记录），
那次调用正常返回，会话以 `Stop(fullyIdle:true)` 结束。结论：**重写插件文件不会打断在途 `ask`**；代价是 **`mcp_config.json` 的改动不会作用到已在运行的 server**，要等它重启（App 重启）才生效。
删除插件目录同样不会停止其 MCP server（等 12 秒仍在，要 App 退出或手动结束）。
全局插件目录（`~/.gemini/config/plugins/`）的行为未单独验证：在用户真实会话运行时改它会影响他们，所以留到 G2 用真正的 AskHuman 插件、在没有在途 `ask` 时验证（V22）。

### 6.4 App 里的首次授权

App 里 MCP 工具默认「询问」（§4.2）。第一次调用 AskHuman 时会弹原生提示，用户点「始终允许」之后持久；也可以在 App 的权限设置里手加 `mcp(askhuman_mcp/*)`。
点过之后的授权存在 `config.json` 的 `userSettings.globalPermissionGrants.allow`（私有格式，§4.3）。AskHuman 只能**只读**地看它是否已含 `mcp(askhuman_mcp/*)` 或 `mcp(*)`，
仅用于设置页的提示，不依赖、不写入；安装回执里写明这一步。手机新建的任务不受此限：sidecar 已预授权（§12.4）。

### 6.5 迁移（升级与首次安装）

1. 先发布插件，再清理旧产物——任何时刻都不出现「没有交互协议」的窗口，短暂的新旧并存只是重复规则。
2. 旧共享规则 `~/.gemini/config/rules/askhuman-antigravity.md`：能证明由当前实例托管才删（沿用既有所有权判定）。
3. 旧全局 `mcp_config.json` 里的托管条目 `mcpServers.askhuman`：字段完全匹配才删，保留其它条目与注释（沿用既有 JSONC 编辑器）。
4. 任何一步无法证明所有权（用户改过）→ 保留、报告冲突、不继续清理；插件已发布则保持，并在状态里说明「旧条目仍在，可能重复」。失败语义沿用 [antigravity.md](antigravity.md) §6：不假定原子，错误写明保留状态与显式重试命令。

### 6.6 停用与卸载

用户在 Antigravity 里停用插件会在 `config.json` 写 `plugins.askhuman.enabled=false`。AskHuman **只读**检测它并显示「已在 Antigravity 中停用」：生命周期与 Stop 确认停止，
手机 / IM 的 Antigravity 任务不可用（readiness 不过）。None 模式卸载只删 AskHuman 托管的插件目录，从不改 `plugins.*`。

## 7. A：可信会话身份

会话身份 = **根会话 ID**（UUID）。

| 来源 | 用在 | 信任 |
| --- | --- | --- |
| hook 载荷 `conversationId` | 生命周期、Stop、插话、Watch | 宿主生成 |
| MCP `_meta` 的 `conversation_id` / `root_conversation_id` | MCP 提问、`show_last`、历史 | 宿主注入，模型不能改 |
| `run_command` 环境的 `ANTIGRAVITY_CONVERSATION_ID`（及 `ANTIGRAVITY_SOURCE_METADATA`） | CLI 模式提问 | 宿主设置；仍要求与 hook 已登记的活会话一致 |
| 都没有 | 回到现状：只认家族，`--show-last` 拒绝项目级回退 | — |

- 家族检测不变：`--caller-agent antigravity` 与 MCP 家族绑定仍用来判定家族；拿到可信 ID 后升级为会话绑定。
- 环境 ID 必须与 daemon 注册表里 hook 已登记（或 sidecar 任务已预登记）且未结束的会话一致；不一致按「无可信 ID」处理，防止被无关上下文继承。
- MCP 的 `show_last` 与历史按 `_meta` 会话 ID 作用域，不再按「MCP server 实例 + 项目」；重复提问合并也按会话。
- 子会话：MCP `_meta` 里 `root_conversation_id` 与 `conversation_id` 不同即子会话，身份取根（`parent_conversation_id` 给出直接父会话）。
  **G1 已实测（V5）：子会话会触发 hooks，载荷里没有父会话字段**（§4.1），所以只看 hooks 认不出它是子会话。可用的父子关系来源，按可靠性：
  1. 父会话的 transcript：`invoke_subagent` 的工具结果步骤里有 `Created the following subagent: {"conversationId": "…"}`
     （和读最后回复同一份 transcript 契约，P1 夹具覆盖；解析失败退化为「没有子会话」，即现状）；
  2. sidecar 的 `agentapi get-conversation-metadata <id>`：`parentConversationId` / `rootConversationId` / `nestingDepth`（文档化命令，只有 sidecar 在线时可用）；
  3. 子会话的 MCP `_meta`（只有它调用 MCP 工具时才有）；
  4. 弱信号：子会话 `SessionStart` / `PreInvocation(0)` 的 `lastUserInput` 为空串（从未收到用户输入；首步是父会话的 `SYSTEM_MESSAGE`，带 `sender=<父会话 ID>`，私有文本格式，不依赖）。只作辅助，不单独裁决。
  怎么用这些来归并子会话、去噪、判断「整棵树空闲」，见 §8 L3 / L4，**待用户决定（D17）**；在此之前 P1 的登记按「每个会话独立」写，不接线根 / 子判断。

## 8. B：生命周期追踪

| 来源 | 动作 |
| --- | --- |
| `SessionStart` | 登记会话：项目 = `workspacePaths[0]`（缺省 `cwd`），模型 = `modelName`，表面（App / CLI / IDE）由 `transcriptPath` 目录判断 |
| `PreInvocation`，`invocationNum == 0` | `turn-start`，含后台任务完成后 Agent 自己唤醒的回合；幂等登记（[D6](agent-lifecycle-tracking.md)：缺 SessionStart 也能追踪）；会话还没有标题时取 `lastUserInput` |
| `PreInvocation`，`invocationNum > 0` | 只刷新活动时间；顺带插话轮询（§10.1） |
| `Stop`，`fullyIdle == true` | `turn-end` → 空闲；再按 §9 出确认卡或失败卡 |
| `Stop`，`fullyIdle == false` | 只刷新活动时间，仍是「工作中」：后台任务完成后 Agent 会自行醒来 |
| 无事件 | 会话结束靠 pid / TTL；用户取消靠兜底 |

- **pid**：`agy` 的 hook 父进程是该会话自己的进程 → 作会话 pid，沿用 [D7](agent-lifecycle-tracking.md)（同一 pid 出现新会话 ID 则旧会话结束）与存活轮询。
  App 的 hook 父进程是 App 唯一的共享 `language_server`，**不是会话 pid** → `pid=None`，走 [D12](agent-lifecycle-tracking.md) 的 1 小时 TTL 与
  30 分钟「工作中」兜底（`WORKING_BACKSTOP_SECS`），与 [D25](agent-lifecycle-tracking.md) 的 Codex app-server 同一条路径。
- **标题**：`lastUserInput` 首行，80 字符截断（[D10](agent-lifecycle-tracking.md) 的回退口径）；sidecar 任务用 `--title`；本版不读私有 `conversation_summaries.db` 里的原生名。
- **不装 `PostToolUse`**：每个工具结果之后必有一次 `PreInvocation`，工具级事件不增加信息，却多一次进程启动。
- **hook 契约**：`__agent-hook antigravity …` **永远退出码 0；除非要应答（§9、§10），stdout 为空**（[D23](agent-lifecycle-tracking.md)）。空输出是各事件统一的无操作；
  不用 `{}` 表示无操作，因为它在 `PreToolUse` 上是 deny（§4.1；本集成不装工具类 hook，但保持同一约定，避免日后误装）。daemon 不可达、超时、解析失败一律 fail-open。

已知限制：

- **L1** 用户取消不产生 Stop，状态最多滞后到 30 分钟兜底；其间手机发来的插话会排队而不被送达，直到状态降为空闲后改走空闲消息。这与 Claude 用户中断同类（[Stop 确认 §2](agent-stop-confirmation.md)），本版接受。
- **L2** App 会话没有结束事件，只靠 1 小时 TTL 变「已结束」。
- **L3** 子会话会触发 hooks（G1 已证实，V5），而 hooks 里认不出它：在做父子归并之前，它会多出一行，其 `Stop(fullyIdle:true)` 会出一张 Stop 卡（§7、L4、D17）。
- **L4** 带子 Agent 的会话，根会话的最后一个 `Stop` 可能是 `fullyIdle:false`，之后没有 `fullyIdle:true` 的根 `Stop`（§4.1 的时序）。按上表「`fullyIdle:false` 不置空闲」，
  这样的会话会一直显示「工作中」直到 30 分钟兜底，也不出 Stop 确认卡。解法需要把子会话归并成树（登记结构、「整棵树空闲」的判定、Stop 卡由谁在哪个 hook 里出），**待用户决定（D17）**。

## 9. C：Stop 结束确认与最终失败

出普通 Stop 确认卡须同时满足：「结束时确认」开关开启；根会话；`terminationReason == NO_TOOL_CALL`；`fullyIdle == true`。
（带子 Agent 的会话见 §8 L3 / L4：根会话可能永远等不到 `fullyIdle:true`，子会话又认不出来，**待用户决定（D17）**。）

| `terminationReason` | 处理 |
| --- | --- |
| `NO_TOOL_CALL` 且 `fullyIdle == true` | Stop 确认卡 |
| `NO_TOOL_CALL` 且 `fullyIdle == false` | 非终态，不出卡 |
| `ERROR` | 最终失败卡（[失败确认](agent-failure-confirmation.md)）；有空闲消息通道时可「要求继续」，否则只有「知道了」 |
| `MAX_INVOCATIONS` / `MAX_FORCED_INVOCATIONS` / `MAX_TOKEN_BUDGET_EXCEEDED` | 「未完成」（与 Grok 的 `incomplete` 同口径） |
| `USER_CANCELED`、`TERMINAL_STEP_TYPE`、`TERMINAL_CUSTOM_HOOK`、`INJECTED_RESPONSE` | 静默 |

- **卡片正文**：Stop 载荷没有最后回复，从 `transcriptPath` 读最后一个带 `content` 的 `PLANNER_RESPONSE`（路径须规范化并落在 `~/.gemini/antigravity{,-cli}/brain/<conv>/.system_generated/logs/` 下，同 Cursor transcript 的约束）。
  读取失败显示降级占位，不影响裁决。Stop 触发时 transcript 是否已含最终回复〔未验证，V6〕。
- **继续**：输出 `{"decision":"continue","reason":"<续跑提示词>"}`。因为 `reason` 以**系统消息**注入（不是用户消息），提示词采用 Claude 那一类的结构化包裹（有后续指令：`[USER CONTINUATION]` + `<user_message>`；无指令：三家共用的 meta 提示）。文案仍只放 `prompts.rs`。
- **结束 / 关窗 / 超时 / 基础设施失败**：fail-open，空 stdout 退出码 0；Stop 的 hook `timeout` 与客户端等待都是 86400 秒（沿用 [Stop 确认 §3.4](agent-stop-confirmation.md)）。
- **去重**：沿用 `[user_confirmed_end_turn]` 标记：在 transcript 最后回复里任意位置出现即静默放行并剥离；读不到 transcript 时退化为再发一张卡（安全方向）。
- **上限**：续跑次数受执行器 `max_stop_hook_continuations` 约束，默认值未知（V2）。实现第一步先实测；若上限很小，另行设计降级（不在本规格预设）。
- 每次用户选择继续后，下一次 `fullyIdle:true` 的 Stop 仍再发确认卡；`continuationActive` 只作信息，不用来抑制确认（这里是人在回路里，不是自动循环）。

## 10. D：插话与空闲消息

### 10.1 工作中：`PreInvocation` 注入

沿用 [插话 D2–D4](agent-interject.md) 的消息队列、IPC 与性能约束（热路径零文件 IO、复用 hook 到 daemon 的连接、首帧超时约 300 ms 一律 fail-open），触发点换成 `PreInvocation`：

1. 队列有已提交消息 → 输出 `{"injectSteps":[{"userMessage":"<包装文本>"}]}`，随后标记已送达；
2. 该会话的 composer 打开中 → hook 阻塞等待提交或取消（取消 → 空 stdout）；
3. 都没有 → 空 stdout，毫秒级。

- 包装文案仍在 `prompts.rs` 单一来源：保留 `[USER INTERJECTION]` 前缀、`<user_message>`、`<attachments>`，**去掉**「这次工具调用被拦下只是为了送信」那句——这里没有拦截。
- 送达粒度是下一次模型调用之前；一个长工具调用期间不会送达。同一会话的并发 hook 只交付一条（daemon 原子出队）。
- 注册表仍认为「工作中」但实际已被取消的会话见 L1。

### 10.2 空闲 / 已结束：sidecar `send-message`

- 通道：daemon 经一个在线 sidecar（目录 sidecar 或 `askhuman-ctl`）执行 `agentapi send-message <conversationId> <内容>`。**没有可用 sidecar 时该能力不可用**，显示原因；这是新增「允许管理 sidecar」之后才有的能力。
- 前置：注册表状态为空闲 / 已结束（工作中走 §10.1）；目标是 App 会话。**绝不**对工作中会话调用 `send-message`——它对运行中会话的语义〔未验证，V10〕，可能排队也可能打断，
  且 `agentapi` 没有原子「仅当空闲」检查。竞态（注册表落后于真实状态）是已知残余风险。
- 范围：只验证过 sidecar 创建的会话（§4.3）。P3 先实测 V11：用户自建的 App 会话能被唤醒则对所有 App 会话开放，否则只对 sidecar 创建的会话开放；
  `agy` 会话验证前一律 `agent_control::can_send` 返回「通道不可用」并显示原因。
- 回执沿用 [结果与恢复 §5.4](agent-lifecycle-results-and-recovery.md)：`accepted`（观察到新回合开始，即 `PreInvocation(0)`）、`rejected`、`unknown`（可能已发出，不自动重发）。
- 手机、Mac 控制台、IM `/msg` 共用同一能力判定。

## 11. E：Watch / IM 实时画面

- 数据源：hook 载荷里的 `transcriptPath`，校验同 §9。只读、有界尾读；沿用 [观察最新正文 §5](agent-observation-latest-text.md) 的预算、事件唤醒与工作中约 2 秒兜底读取（既有观察机制，不是新轮询）。
- 映射：
  - **最新正文** = 最后一个带 `content` 的 `PLANNER_RESPONSE`（同一步骤带 `tool_calls` 时 `content` 仍是候选）；`status == DONE` 才算定稿。
  - **工具** = `tool_calls[]`：`run_command` 取 `CommandLine`，`call_mcp_tool` 取 `ServerName/ToolName`，其余取 `name`；完成状态取其后的 `GENERIC` 结果步骤。
  - 用户输入 = `USER_INPUT`（剥掉 `<USER_REQUEST>`）；`SYSTEM_MESSAGE` 与注入的消息不当正文。
  - **没有思考**（§4.6）。
- 降级：读取失败保留最后内容并单独表达读取状态；`transcriptPath` 越界或缺失则该会话不可 Watch，并显示原因。
- 可用性：会话已被 hook 登记且 transcript 可读。App 与 `agy` 同格式，共用解析器。

## 12. G：手机 / IM 新建任务

### 12.1 目标选择与「能不能启动」

- 设置 `antigravity.launchPreference`：`app`（默认）/ `cli`，只决定**优先**走哪条路径（D6，与 [Cursor Desktop](cursor-desktop-task-creation.md) §3 同构）：
  优先路径整体可用 → 用它；整体不可用而另一条整体可用 → 自动改用另一条，并说明原因；两条都不可用 → 如实列出各自的障碍。
  **任务一旦开始，不静默改走另一条路径**：回退只发生在选择阶段。
- **App 路径整体可用**（廉价静态检测）：macOS；App 已安装；插件已装且未被停用；集成模式为 **MCP**（App 沙箱挡 CLI 模式，§4.5）；「允许管理 sidecar」已开；
  目标目录已有 sidecar，或 sidecar 总数未达上限。
- **CLI 路径整体可用**：`agy` 能在用户 login shell 里解析、版本达标；插件已装；集成模式为 CLI 或 MCP；终端就绪（沿用 [IM 创建任务 §4.2](im-agent-task-launch.md)）。
- 「能不能启动」只有一个判断 `integrations::agent_start`：`/new` 的 Agent 列表、手机、GUI、设置共用它。Antigravity 从「永远不可启动」改成「按上述两条路径判断」。

### 12.2 App 路径的创建流程

1. `/new` 走既有流程：readiness → 工作目录 → Agent（Antigravity）→ 权限卡（Agent 默认 / YOLO）→ 任务输入 → 提交。
2. 选定目标目录（canonical，不提升到 git root，[D13](im-agent-task-launch.md)）与变体：权限为 YOLO → `askhuman-yolo-<哈希>`，否则 `askhuman-ws-<哈希>`。
3. 保证该 sidecar 存在且在线（§12.3、§12.5）。
4. 写操作回执 `~/.askhuman/antigravity-tasks/<operationId>.json`：`prepared`（发出前）→ `sent`（命令已写给 sidecar）→ `created{conversationId}` / `failed{原因}` / `unknown`。
   只有 `prepared` 与 `failed` 可用同一操作 ID 重来；`sent` 与 `unknown` 表示可能已创建，不自动重试，提示用户去 App 里查看（同 Cursor Desktop 的回执语义）。
5. daemon 经私有通道让 sidecar 执行 `agentapi new-conversation [--title=<首行≤60字>] [--model=…] -- <任务文本>`（无 shell，任务文本是单个 argv），解析 `conversationId` 回给 daemon。
6. daemon 在注册表**预登记**该会话（来源 = 手机 / IM，带来源渠道）；hooks 随后到达（`SessionStart` / `PreInvocation(0)`）转入「工作中」。
7. IM / 手机回执：「已在 Antigravity App 创建任务」，并按 [D20–D22](im-agent-task-launch.md) 把来源渠道设为活跃槽、默认 watch（§12.8）。

### 12.3 sidecar 的创建、更新与清理

- **种类与命名**：`askhuman-ws-<h8>`（白名单）、`askhuman-yolo-<h8>`（YOLO）、`askhuman-ctl`（控制：没有目录 sidecar 时发空闲消息；工作区是 AskHuman 私有空目录，不授予任何动作）；`<h8>` 是 canonical 目录 SHA-256 的前 8 位十六进制。
  总数上限 20，超限时 `/new` 拒绝创建并指向设置页。`display_name` 形如「AskHuman · <目录名>」（YOLO 变体为「AskHuman · <目录名> · YOLO」），`description` 说明由 AskHuman 创建与管理、在 AskHuman 设置里移除；YOLO 变体另写明宽授权。
- **清单**：`command` = AskHuman 绝对路径，`args` = `["__antigravity-sidecar","--id","<id>"]`（子命令名为拟定），`restart_policy` = `always`，`agent_permissions` 见 §12.4。
- **创建顺序**：写 sidecar 目录（原子）→ 对 `config.json` 做读-改-写加 `sidecars.<id>.enabled=true` → 等 sidecar 进程上线（事件驱动，见下）。
- **config.json 写入纪律**（P11）：daemon 内单写者；每次写前重读、写后核对 App 补上的字段；同目录临时文件 + 原子 rename，保持 0600；只动 `sidecars.askhuman-*`，保留未知键，
  不碰 `plugins.*`、`userSettings`、权限授予。与 App 的写入冲突时重读重试至多 3 次，仍失败则报错，不强写。
- **更新**：授权规则变化 → 重写 `sidecar.json`，App 自动重启该 sidecar 并更新授权〔实测〕。内容没变不重写。
- **移除**：删除 sidecar 目录（App 向它发 SIGINT，约 5 秒后停止）并从 `config.json` 删同名键。**不删** App 生成的 `projects/<id>.json` 与 `sidecar_data/<id>`（App 自己的数据，保留任务历史归属；
  重建同名 sidecar 时 App 是否认领见 V8）。
- **sidecar 进程约束**：无状态；**不拉起 daemon**（App 一启动就会起所有 sidecar，不能借此让 daemon 常驻）；与 daemon 的连接断开后用有上限的退避重连；对 SIGINT / SIGTERM 干净退出；
  只执行 daemon 下发的 `agentapi` 调用，不从 stdin 或网络接受别的指令。
- **私有通道**：sidecar 主动连 daemon 并报到（sidecar id、`ANTIGRAVITY_PROJECT_ID`、版本、`agentapi` 路径），daemon 用「报到」事件判定就绪，不轮询 sidecar。具体消息在实施计划里定；
  按 overview 规则，新增 `ClientMsg` / `ServerMsg` 枚举变体要递增 `PROTOCOL_VERSION`，仅加 `#[serde(default)]` 字段不递增。
- 「允许 AskHuman 管理 Antigravity sidecar」是一次性总开关：它授权 AskHuman 创建 / 修改 / 删除 `~/.gemini/config/sidecars/askhuman-*/` 与 `config.json` 里的 `sidecars.askhuman-*` 键，别无其它。
  关闭开关不会自动删除已有 sidecar，设置页提供「移除全部」。

### 12.4 权限：白名单与可选 YOLO

- **白名单（`ws`，D7 默认）**：`allows_user_interactions:false`，`access_grants` 只含 AskHuman 自己的 MCP `mcp(askhuman_mcp/*)` 与任务附件目录的只读授权（路径实施时定），再加用户在设置页给该目录追加的规则。
  未授权的动作自动拒绝（§4.3）；是否免询问放行由全局预设决定。因此「Agent 默认行为」在 Antigravity App 路径上的含义是**白名单 sidecar**，不是你在 Mac 上逐项点过的授权的原样延续
  （你的全局「始终允许」授权是否对 sidecar 项目生效〔未验证，V19〕）。
- **规则编辑**：设置页按目录编辑，每条须是合法 `action(target)`；全局通配 `(*)` 不允许出现在白名单里（要放开就用 YOLO）。
- **YOLO（`yolo`）**：另一组 sidecar，`access_grants` 为 `command(*)`、`read_file(*)`、`write_file(*)`、`read_url(*)`、`mcp(*)`（`read_url` 用于放开沙箱出站网络，实测联网命令需要它）。
  沿用 `/new` 的权限卡与全局 `permissionPrompt` 设置，风险说明写明「放开命令、文件读写与联网，不再逐项询问」。
  **不再追加二次确认**（D14）：权限卡就是每次选 YOLO 时的显式确认。区别于别的 Agent 的一次性启动参数：这里的 YOLO sidecar 会在 App 里留下一个长期存在的宽授权项目，
  之后在 App 里手动打开它开聊也不会再询问，所以做两点缓解：sidecar 的显示名带「YOLO」、`description` 写明宽授权；设置页把它标成「宽授权」并提供一键移除。
- 白名单需要预授权的 MCP 在 sidecar 会话里真正生效〔未验证，V18〕——P2 第一步用一次 Flash 调用核对；若不成立，默认授权集要重新设计。
- 宽授权是否真的让命令跑在沙箱之外、以及此时能否连 daemon socket〔未验证，V9〕。

### 12.5 App 未运行

- 路径整体可用但 App 没在运行：后台启动 App（`open -g`，不抢前台），等该 sidecar 报到，**最多约 30 秒**（与创建新 sidecar 共用同一预算）。超时或失败 → 回退 CLI（若整体可用）或报错说明。
- App 冷启动时 sidecar 是否随之启动、登录失效时 `new-conversation` 的失败形态〔未验证，V1〕。需要退出用户的 App 才能测，须先征得同意。

### 12.6 最近目录

沿用 [IM 创建任务 §5](im-agent-task-launch.md) 的 `~/.askhuman/agent-workspaces.json`：hooks 登记的会话增量更新（`source: lifecycle`），手动添加始终可用。
**冷启动种子（P4，可选）**：只读各产品 `conversation_summaries.db` 的 `workspace_uris` 作为首次候选；私有格式、读取失败静默、不依赖。

### 12.7 CLI 路径

- 经既有终端启动桥（[IM 创建任务 §7–§8](im-agent-task-launch.md)）：新增 `agy` 适配器，固定 argv `agy [--dangerously-skip-permissions] -i <任务文本>`（任务文本是单个 argv；以 `-` 开头的文本是否安全〔未验证，V15〕），
  一次性 `LaunchRecord`、helper `__agent-launch <token>`、launch-id 环境变量沿用；hook 带回 launch id 即可与 `LaunchRecord` 配对（已验证环境继承）。
- 权限映射：Agent 默认 → 不加 flag；YOLO → `--dangerously-skip-permissions`（补进 [IM 创建任务 §6.3](im-agent-task-launch.md) 的 YOLO 表）。
- `agy` 首次引导与新目录的信任提示会出现在 Terminal 里，AskHuman 不代答〔V12〕；IM 回执提示「若终端停在引导页，请在电脑上完成」。
- pid 用 hook 父进程（§8），可聚焦终端窗口的能力沿用既有终端桥。

### 12.8 配对、Watch 与回执

- App 路径按 `conversationId` **精确配对**（不用 60 秒匹配窗口）；CLI 路径按 launch id。
- 60 秒内没有收到该会话的任何 hook → 任务仍算创建成功，来源 IM 明确告警「未能自动 watch（可能插件被停用或 hooks 未生效）」（沿用 [D22](im-agent-task-launch.md)）。
- 来源渠道活跃槽、默认 watch 与 `/watch` 的行为不变。

### 12.9 模型、标题与附件

- **模型（D11）**：Antigravity 接入既有的「新建任务模型与强度」管线（[ios-task-model](ios-task-model.md)）。手机（`task.options` / `task.create`）与 Mac 新建任务窗口
  （[GUI 规则](gui-agent-task-launch.md)：所有可启动 Agent 都能选模型）都能选；IM `/new` 沿用现状，没有模型步骤，所有 Agent 一致（D16）。
  - **清单来源**：`agy models`（§4.4；只列清单，不起会话，不产生模型调用）。走既有 `CatalogSource` 探测通道：已解析的可执行文件 + 固定参数、8 秒超时、30 分钟缓存、
    失败降级成只有「默认」（[M12](ios-task-model.md)）；清单不写死，因为它是 Antigravity 当时给的。**只在手机 / 窗口打开新任务表单、Antigravity 已就绪且 `agy` 可解析时才运行它**。
  - **形态（D15）**：平铺列表，档位折在 id 里（`gemini-4-argon-high`、`gemini-3.8-flash-low` …），**不出强度行**、`efforts` 缺省，与 Cursor 同口径（[M8](ios-task-model.md)）。
    不从 id 后缀或显示名里猜「模型 + 档位」的分组：`gemini-pro-orchestrator-g1-pro`、只有 Medium 的 `gpt-oss-120b-medium` 不符合规律，猜错会把一个不存在的 id 传下去，
    且「某模型不选档位时用哪一档」没有 Antigravity 自己声明的默认值可依。当前 18 项，超过 12 项时手机端自带搜索。
  - **没装 `agy` 的机器**：App 路径仍能建任务，清单退回 `agentapi` 文档公开的三个别名 `flash_lite` / `flash` / `pro`（显示名 Flash Lite / Flash / Pro，副标题注明「Antigravity 别名」）。
    别名只在 App 路径有效，各自对应的具体模型〔未验证，V20〕；CLI 路径本来就需要 `agy`，不受影响。
  - **「默认」**：不传 `--model`，由 Antigravity 决定（[M7](ios-task-model.md)：「默认」= 不传任何开关，argv 与今天逐字节相同）。「默认所指」无从得知——`agy models` 没有标记，
    `config.json` 里也没有默认模型——所以手机显示「Antigravity 自己的设置」。**App 路径不传时，实测用的是 Flash 族的一个档位**（§4.3），`agy` 路径的默认另算，二者可能不同；
    IM 与手机上留「默认」的任务就是这个结果（D16）。
  - **启动映射**：只对新任务。App 路径 `agentapi new-conversation --model=<id> …`（等号写法，值不会被当成开关）；CLI 路径 `agy --model <id> -i <任务>`〔写法与效果未验证，V20〕。
    `id` 须符合既有净化规则（`valid_model_id`）并与**当前清单**某一项完全相等，否则 `modelUnavailable`、什么都不启动（[M9](ios-task-model.md)）；`effort` 一律拒绝
    （同 Cursor：没有独立的强度开关）。
  - **显示实际模型（[M4](ios-task-model.md)）**：详情页取 hook 载荷的 `modelName`（`SessionStart` 与每次 `PreInvocation` 都带，中途换模型会更新）；进度页显示所选。
  - **不改线协议**：`models[]`、`defaultModel`、`modelUnavailable` 都已存在，Antigravity 只是新增一个清单来源与一条启动映射；iOS 线版本与 Mac IPC 的 `PROTOCOL_VERSION` 都不变。
    iOS App 预期不需要新代码（`AgentBadge` 已有 Antigravity 图标，未知 Agent 名有大写首字母的回退，表单对「有模型、无强度」的 Agent 与 Cursor 同形），实施计划里核对一遍。
  - **成本**：清单里有 Pro 级与 Claude 级模型，选它们按所选模型计费；手机上不显示价格（沿用 [已知局限 4](ios-task-model.md)）。验证只用 Flash 低价档（V20）。
- **标题**：`--title` = 任务首行截断。
- **附件**：与终端路径一致，把附件路径与警告拼进任务文本；白名单 sidecar 预授权附件目录的只读访问（§12.4）。

### 12.10 设置页

Agents 页的 Antigravity 卡新增：集成模式（None / CLI / MCP）、生命周期与 Stop 确认开关、新建任务偏好（App / CLI）、「允许 AskHuman 管理 Antigravity sidecar」、
已管理 sidecar 列表（目录、类型、在线状态、最近使用，目录不存在时标出）与「移除」/「移除全部」、按目录的授权规则编辑、YOLO 说明。中英文随既有 i18n 体系。

## 13. F 与 Fork：调研结论（本版不做）

### 13.1 权限接管（F）

结论：**不做**，`permission` 能力保持「不支持」。依据：

- 原生权限提示是会话里的一个 WAITING 步骤（`requestedInteraction.permission`），hooks 看不到它，也回答不了它。〔实测〕
- `PreToolUse` 返回 `allow`、`permissionOverrides` 或 `overwrite` 都不会预先批准原生提示（App 2.32.1）；文档把 `permissionOverrides` 描述为临时授权，但未观察到该效果。hook 只能 `deny`，或把决定留给原生提示。〔实测〕
- 能回答原生提示的只有私有 LS RPC（`HandleCascadeUserInteraction`，需要 `cascadeId`）。〔私有〕用户 2026-11-15 不批准私有接口。
- 公开但改变形态的替代：ACP（`agy acp`）与 SDK 让**客户端**持有权限请求，但前提是 AskHuman 自己托管会话、取代 TUI / App，这不是增强，不在本版。〔文档〕
- sidecar 的 `allows_user_interactions:false` 是预授权白名单，不是接管（§12.4）。

### 13.2 Fork

没有公开的 headless Fork 入口：`agentapi` 只有 `new-conversation` / `send-message` / `get-conversation-metadata`；`agy` 的 `/fork` 只在交互里；私有 LS RPC 不批准。
手机 / IM 的 `/fork` 对 Antigravity 保持不可用并显示原因。

## 14. 版本、平台与实例范围

- **版本**：实测 App 2.32.1、`agy` 1.5.9。更新日志里的下限〔文档〕：`lastUserInput` 为 CLI 1.4.7 / App 2.28，`hookEventName` / `cwd` / `toolCall.id` 为 CLI 1.5.4，`continuationActive` 为 CLI 1.5.6，
  MCP `_meta.root_conversation_id` 为 CLI 1.4.4。安装与状态检测读 App 的 `Info.plist` 版本、`agy --version`，运行时可读 `ANTIGRAVITY_LS_VERSION`。低于实测版本不阻止安装，只标「未验证版本」。
- **平台**：只在 macOS 验收。现有 Antigravity 集成仅 Unix，Windows 明确不支持，沿用；Linux 要先实测（App 与 `agy` 在 Linux 上的 hooks / sidecar / 沙箱行为都未验证）。IDE 与扩展不声明支持（`transcriptPath` 为 `antigravity-ide/`，行为未测）。
- **Dev Instance**：默认不安装插件、不创建 sidecar、不动真实 `~/.gemini`（与 Cursor Desktop 路径同口径）；需要验证时按 [Dev Instance](dev-instance-parallel.md) 与 `docs/agent-worktree-setup.md` 显式启用，只能操作自己账本里的产物。
  多份安装互顶由所有权账本拦住（沿用 [antigravity.md](antigravity.md) §5）。

## 15. 失败与降级

| 情形 | 行为 |
| --- | --- |
| daemon 不可达 / draining / hook 超时 / 解析失败 | hooks fail-open（空 stdout、退出码 0），Agent 正常继续；插话留队列不丢 |
| 插件被用户停用 | 只读检测并显示；生命周期 / Stop 停止；手机 / IM 的 Antigravity 任务不可用 |
| 用户取消 | 无事件；状态滞后到 30 分钟兜底（L1） |
| Stop 触发时 transcript 未就绪 / 读取失败 | 卡片显示降级占位；标记检测退化为再发一张卡 |
| 续跑被执行器上限拒绝 | 见 V2；本规格不预设降级 |
| sidecar 未在 ≈30 秒内报到 | 回退 CLI（若可用）或报错并说明原因；已写入的 sidecar 保留在设置页 |
| `config.json` 写入冲突 / 失败 | 重读重试至多 3 次，仍失败则报错，不强写，不改其它键 |
| sidecar 总数达到上限 | `/new` 拒绝创建，指向设置页移除 |
| `new-conversation` 失败 / 登录失效 | 回执 `failed` 并带原因；不改走另一条路径 |
| 回执 `sent` / `unknown` | 不自动重试，提示用户到 App 里查看 |
| 白名单拒绝了动作 | Agent 自己在会话里看到拒绝；用户可在设置页追加规则或改用 YOLO |
| 旧 App / `agy` 版本过低 | 状态标「未验证版本」，不阻止安装 |
| 旧共享规则 / 全局 MCP 条目无法证明所有权 | 保留、报告冲突，可能与插件重复（§6.5） |
| 旧 iOS App 不认识 `antigravity` 这个 Agent 种类 | 实施计划里核对 `agents[]` 对未知 kind 的处理（[ios-task-model](ios-task-model.md)） |

## 16. 实施分期与触点

### 16.1 分期（D12，已确认）

| 期 | 内容 | 依赖 |
| --- | --- | --- |
| P1 基础 | 插件与迁移（§6）、`__agent-hook antigravity`（§8）、A 可信身份（§7）、B 生命周期 | 无 sidecar |
| P2 手机 / IM 新建任务 + Watch | sidecar 管理器与设置页开关（§12.3、§12.10）、App 目标的 `/new`（§12.1–§12.2、§12.5）、白名单权限（§12.4）、模型清单与 `--model` 映射（§12.9，App 路径部分）、E Watch（§11） | P1；先核对 V18、V20 的 Flash 部分 |
| P3 Stop 确认 + 插话 | C（§9）、D 工作中注入与空闲消息（§10） | P1；空闲消息依赖 P2 的 sidecar；先实测 V2、V6 |
| P4 CLI 路径 + YOLO + 种子 | `agy` 终端适配器（§12.7，含 `--model`）、YOLO sidecar、最近目录冷启动种子（§12.6） | P1–P2；先核对 V3、V12、V15、V20 的 `agy` 部分 |

每期落地前，先写 `docs/plans/antigravity-deep-integration.md` 实施计划并经你确认。

### 16.2 代码触点（已读到的位置）

- `integrations/agent_lifecycle.rs`、`agent_stop.rs`、`agent_launch.rs`、`agent_start.rs`：多处把 `AgentKind::Antigravity` 直接判为「不支持」或空事件集，需改为按本规格判断。
- `integrations/antigravity_rule.rs` 与 `agent_rules.rs` / `mcp_config.rs`：单文件规则与全局 MCP 写入，迁移为插件目录事务（§6）。
- `agents/detect.rs`：`ANTIGRAVITY_CONVERSATION_ID` 由「家族标记」升级为经校验的会话身份（§7）。
- `agents/{activity,observation,transcript_full,title,workspaces,session_paths}.rs`：各家 transcript、标题与最近目录的解析，新增 Antigravity 分支（§8、§11、§12.6）。
- `daemon/runtime/ios_agents.rs`、`ios_tasks.rs`：iOS 列表与任务选项当前把 Antigravity 当不可启动 / 不可插话。
- 新增：`__agent-hook antigravity`、`__antigravity-sidecar`（拟定名）、Antigravity transcript 解析器、sidecar 管理器、`agy` 适配器。
- 持久化：`agents.json` 的 kind 枚举若新增取值，要核对降级读取；任务回执是文件（`antigravity-tasks/`），不动 SQLite 恢复库，若实施发现需要改 `recovery.rs` 则按 AGENTS.md 递增 `SCHEMA_VERSION` 并加迁移与夹具。
- 模型：`agents/model_catalog/{source,parse}.rs`（`AgentKind::Antigravity` 现在直接返回「不支持模型发现」，新增 `agy models` 解析与别名回退）、`model_catalog.rs` 的 `ModelChoice::check_for`（拒绝 effort）、
  `integrations/agent_launch.rs` 的 `agent_args`（CLI 路径加 `--model`）与 sidecar 任务命令（`--model=`）、`daemon/runtime/ios_tasks.rs` 的 `catalog_source`；
  iOS 与 Mac GUI 不需要新协议，需核对 `TaskComposer` 对「有模型、无强度」的处理（Cursor 同形）与 GUI 的 Agent 列表不再排除 Antigravity（[GUI G5](gui-agent-task-launch.md)）。
- 其它：Mac 控制台 Agent 分组与图标、Agents 设置页卡片、`/new` 的 Agent 列表、wiki 中英文档。

## 17. 未验证项（如实记录）

| 编号 | 未验证 | 影响 | 怎么验 |
| --- | --- | --- | --- |
| V1 | App 冷启动时 sidecar 是否随之启动、≈30 秒是否够；登录失效时 `new-conversation` 的失败形态 | §12.5 | 需退出用户的 App，先征得同意；或在用户自然重启时观察 |
| V2 | Stop 续跑上限（`max_stop_hook_continuations`）默认值与达到上限后的行为 | §9 | P3 第一步实测；若很小则另行设计降级 |
| V3 | `agy` 上：**全局**插件 hooks 是否载入并在会话里触发（不触发则 hooks 改装全局 `hooks.json`，§6.2）、Stop `continue`、`PreInvocation` 注入、取消 / Ctrl-C、`send-message` 能否唤醒 `agy` 会话。**部分已验（G1，含 T1c）**：工作区级插件的 hooks 在受信任的工作区会被载入**并在会话里触发**，不信任则不载入（§4.1）。**P1 又验了**：AskHuman 自己生成的插件（工作区级副本）在 `agy` 会话里触发 `SessionStart` / `PreInvocation` / `Stop(fullyIdle:true)`，由 AskHuman 的处理器接住，会话进注册表（附录 A.3）；
全局插件在会话里的触发、`continue`、注入、取消、`send-message` 没验 | P1、P3、P4 | 全局插件的会话触发并入 V22（G2）；`continue` / 注入等随 P3 逐项最小实测（`agy` + Flash 低档）。**`agy` 实验的提示必须写明「不要运行 AskHuman」**（见附录 A.2） |
| V4 | MCP 缺省超时（无 `timeoutSeconds`）与整 24 小时等待 | `ask` 长等待 | 无法真等 24 小时；沿用既有口径 |
| V5 | **已验（App，G1）**：子会话触发 hooks，载荷无父字段；MCP `_meta` 带 `parent_conversation_id`（§4.1、§4.2、§7）。**`agy` 的 MCP `_meta` 字段仍未验** | §7 | `agy` 那半留到 G4 |
| V6 | **已验（G1）**：`Stop` 触发时 `transcript_full.jsonl` 已含最终回复（App 三个会话：根 `done`、子 `ok`、T3 `done`；`agy` 的 T1c：`ok`；均为最后一个 `PLANNER_RESPONSE`、`DONE`） | §9 卡片正文与标记去重 | — |
| V7 | sidecar 创建的会话在 App 里的可见位置、是否隐藏于会话列表 | 用户在电脑上接手任务 | 看 App 界面与 `hideFromConversationList` 归属 |
| V8 | 重建同名**全局** sidecar 时 App 是否认领孤儿项目（插件 sidecar 已见到） | §12.3 历史归属 | 删除再重建同名 sidecar 后看项目与会话归属 |
| V9 | 宽授权是否真让命令跑在沙箱之外，以及此时能否连 daemon socket | YOLO 在 CLI 模式下的可用性 | 对照实验一次 |
| V10 | `send-message` 对工作中会话是排队还是打断 | §10.2 目前一律禁止 | 若要放开才测 |
| V11 | 用户自建 App 会话、`agy` 会话能否被 sidecar `send-message` 唤醒 | §10.2 范围 | 最小实测 |
| V12 | `agy` 首次引导与新目录信任提示在 IM 启动时的表现。**部分已验（G1）**：首次运行先显示引导（配色、数据使用同意），`-i` 任务仍被发出并执行（起了 2 次模型请求）；未信任的目录里工作区级插件 hooks 不载入。**没看到**新目录的信任提示本身（实验用 `--settings` 的 `trustedWorkspaces` 绕开了它） | §12.7 | G4 在一个新目录里用 `agy -i` 看一次 |
| V13 | Linux / Windows | §14 | 各自平台实测 |
| V14 | **已验（工作区级插件，G1 T3）**：改写插件文件不重启 MCP server、不打断在途调用，新 `mcp_config.json` 也不作用于已运行的 server（§6.3）。全局插件目录另验，见 V22 | §6.3 | — |
| V15 | `agy -i` 对以 `-` 开头的任务文本在固定 argv 下是否安全 | §12.7 | 一次最小实测 |
| V16 | App 窗口没开、只有 `agy` 在跑时 sidecar 是否运行 | §12.1 的「App 路径可用」口径 | 一次最小实测 |
| V17 | 高思考模型的 transcript 是否含思考文本 | §11 | 换模型看一次 |
| V18 | 预授权 `mcp(askhuman_mcp/*)` 在 sidecar 会话里调用插件 MCP 时是否生效 | §12.4 白名单默认授权集 | P2 第一步，一次 Flash 调用 |
| V19 | 用户的全局「始终允许」授权（`userSettings.globalPermissionGrants.allow`）是否对 sidecar 项目生效（只知道预设会继承） | §12.4 白名单的实际范围 | 与 V18 同一次实验里加一个对照动作 |
| V20 | `agentapi --model=<id>` 对清单里其它 id 与别名 `flash_lite` / `flash` / `pro` 的接受情况及各自对应的模型；`agy --model <id>`（含 `=` 写法）是否生效 | §12.9 启动映射与别名回退 | 只用 Flash 低价档各试一次（`-low` / `-medium` / `-high`）；Pro 级与 Claude 级只在你批准后各试一次，且用一句话任务 |
| V21 | `agy models` 未登录、未完成首次引导、网络失败时的输出与退出码；清单是否随账号 / 套餐变化 | §12.9 清单降级 | 一次只读实测（不产生模型调用） |
| V22 | 在用户真实会话运行时改写 / 新增**全局**插件目录：MCP server 是否重启、hooks 是否立刻对在跑的会话生效；全局插件的 hooks 在 `agy` / App **会话**里是否触发。P1 没有发布到用户真实的全局目录，只在隔离 HOME 里验证了 `agy plugin list` 识别（附录 A.3） | §6.3、§6.5 迁移 | G2：用真正的 AskHuman 插件，在没有在途 `ask` 的时刻验证 |
| V23 | D17 方案的可行性：父 transcript 里 `invoke_subagent` 结果的格式是否稳定；根会话的 Stop hook 阻塞等待子孙空闲时，子会话的 `send_message` 是排队还是丢失；根被阻塞期间唤醒是否重复出卡 | §8 L4、§9 | D17 定案后：P1 夹具 + G3 里带子 Agent 的一轮 Flash 实测 |

## 18. 验收

- 默认不改变现有行为：未升级插件、未打开「管理 sidecar」时，Antigravity 与现在完全一致。
- 迁移：旧共享规则与旧全局 MCP 条目在所有权可证明时被清理；用户改过则保留并报告冲突；任何时刻都有交互协议；失败写明保留状态与重试命令。
- hooks：daemon 缺席、超时、解析失败均 fail-open；除应答 Stop / 插话外 stdout 恒为空、退出码恒为 0；本集成不安装工具类 hook。
- 身份：MCP 与 CLI 模式都能按会话 `show-last`；环境 ID 与登记不符时按「无可信 ID」处理；子会话归并到根。
- 生命周期：回合开始 / 结束映射符合 P4（含后台任务自醒、`fullyIdle:false` 不置空闲）；App 会话 `pid=None`、`agy` 会话有 pid 存活判断；取消的滞后在兜底内收敛。
- Stop 确认：只在 §9 条件下出卡；继续 / 结束 / 关窗 / 超时行为与既有 Agent 一致；标记去重有效。
- 插话：工作中经 `PreInvocation` 送达且只送达一次；空闲 / 已结束经 sidecar 送达并有回执；绝不对工作中会话调用 `send-message`。
- Watch：App 与 `agy` 会话都能显示最新正文与工具；读取失败保留旧内容。
- 新建任务（App）：手机与 IM 都能在选定目录创建任务；sidecar 按需创建、热注册、可在设置页看到并移除；白名单拒绝未授权动作；YOLO 由权限卡显式选择，sidecar 名带「YOLO」且设置页标注并可移除；App 未运行时按 §12.5 处理；同一操作 ID 重试不重复创建；`sent` / `unknown` 不自动重试。
- 新建任务（CLI）与回退：目标选择对称、不静默改道；`agent_start` 是唯一判断。
- 模型：手机与 Mac 窗口出现 Antigravity 的「模型」行、没有「强度」行；选「默认」时 argv / sidecar 命令与不接入模型管线时逐字节相同；选定模型时 argv 精确含 `--model`（CLI）或 `--model=`（App），断言组装出的命令而不真的启动；伪造或已不在清单里的 id → `modelUnavailable` 且不启动任何东西；`agy` 缺失时退回三个别名；清单命令失败 / 超时 → 只剩「默认」；解析器用合成夹具（含脏行与不规律条目）验证。
- 配置写入：`config.json` 只出现 `sidecars.askhuman-*` 的变化，保持 0600 与未知键；并发写入不破坏文件（用假 App 写入者测试）。
- 测试：解析器、hooks 输出、注册表映射、sidecar 清单与 `config.json` 编辑器、回执状态机用夹具与假 `agentapi` / 假 sidecar 覆盖；**测试不启动真实 Antigravity 模型**。真实验证只用 Antigravity、优先 Flash 低档，受各门的任务数预算、隔离与停止条件约束（§1.2：不再逐项授权），**不使用 Cursor**。
- 子会话（待 D17 定案后补）：登记归并、去噪与「整棵树空闲」判定的验收条目。

## 19. 对既有文档的影响（随各期落地时更新）

P1 已更新：`antigravity.md`（按插件重写）、`mcp.md`、`docs/overview.md`（Antigravity 的插件、会话身份与标题）、两份 wiki（`antigravity-setup`）、两份 README、`docs/PROGRESS.md`；下列其余各项随对应阶段落地时更新。

- [antigravity.md](antigravity.md)：§4、§5–§6、§7 由本规格取代；§2–§3 的 CLI / MCP 语义保留。[MCP 模式规格](mcp.md)：增加 Antigravity 的 `_meta` 会话绑定。
- `docs/overview.md`：生命周期 / 插话 / Stop / 启动能力描述里「Antigravity 不支持」的几处，以及 Antigravity 集成的规则路径与 MCP 路径；若全局地图与不变量变化再改，局部细节放 `overview-<topic>.md`。
- [iOS 新建任务模型与强度](ios-task-model.md)（「清单从哪来」表加 Antigravity 行、「怎么启动」表加 `--model`）、[Mac GUI 新建任务](gui-agent-task-launch.md)（G5 不再排除 Antigravity）。
- [IM 创建任务](im-agent-task-launch.md)（Agent 列表、readiness、YOLO 表）、[结果与恢复 §5.3](agent-lifecycle-results-and-recovery.md)（空闲消息能力表）、
  [Stop 确认 §2](agent-stop-confirmation.md)（能力表）、[插话 D1](agent-interject.md)（范围）、[观察最新正文](agent-observation-latest-text.md)（新增来源）。
- `docs/PROGRESS.md`：本规格确认并开工后登记（P15）。

## 附录 A：实测记录

| 主题 | 结果 |
| --- | --- |
| 六个 hook 事件的载荷、环境、祖先进程（App） | 字段与文档一致；App 的 hook 父进程是共享 `language_server`；`agy` 的是会话自己的 `agy` |
| 六个事件的输出语义 | 空 stdout + 0 恒为无操作；`PreToolUse` 的 `{}` 是 deny；非零退出让工具调用失败；Stop 退出码 1 与 PreInvocation 杂项输出被忽略 |
| Stop `continue` | `reason` 以系统消息注入，`executionNum` 递增；`timeout: 86400` 被接受，阻塞 40 秒被等待 |
| PreInvocation 注入 | `injectSteps.userMessage` 生效，transcript 里为 `USER_INPUT` / `SYSTEM_SDK` |
| 用户取消（2 次，直接调用取消 RPC） | 无 Stop、无 PostInvocation；会话变空闲 |
| 长命令自动转后台 | `Stop(fullyIdle:false)` 后 Agent 自醒，最后 `Stop(fullyIdle:true)` |
| MCP 元数据与长调用 | `_meta` 带会话 ID；400 秒调用在 `timeoutSeconds` 下成功 |
| 插件载入 | 规则 / MCP / hooks 无需登记即热加载；MCP 命名 `<插件>_<server>`；插件 hooks 环境带 `PLUGIN_ROOT` |
| 全局 sidecar 生命周期 | 约 10 秒热注册；改清单重启；删除目录停止；遗留物如 §4.3 |
| `agentapi` | `new-conversation` 立即返回会话 ID；`send-message` 跨项目唤醒；`get-conversation-metadata` 无运行状态 |
| 授权 | `allows_user_interactions:false` 自动拒绝；宽授权 sidecar 无人值守跑联网命令 |
| sidecar 连 daemon socket | 可以 |
| App 沙箱 | 挡 `ps` 与 daemon socket；`run_command` 环境带会话 ID |
| `agy` 1.5.9 | 安装、凭据共享、读全局配置、hook 父进程、launch-id 继承、`-i` 启动；`--dangerously-skip-permissions` 下可连 daemon socket |
| `agy models` | 18 项平铺清单，每行 id 与显示名（制表符分隔），无默认标记，约 4 秒，不起会话；档位折在 id 里；会建 `~/.gemini/antigravity-cli/` 与 `config/projects/default-cli-project.json`（均已清理） |
| 不传 `--model` | sidecar 任务用占位模型 M322（与显式 `gemini-3.8-flash-low` 的 M320 同族，推断为 Flash 的另一档） |
| `config.json` 非原子写入 | LS 日志两次瞬时 `failed to parse user config` |

没做成：`agy` 的取消 / Ctrl-C（界面停在登录引导，结论不明）；MCP 缺省超时的对照实验（会话跑偏，约 14 次调用浪费）。

用量与还原：合计约 61 次 Flash 低档调用（含 `agy` 约 5 次，超出你批准的 2–3 次）。`agy` 安装脚本追加到 `~/.zshrc` 的 PATH 行已按字节还原，`~/.local/bin/agy` 保留；
`~/.gemini` 在每个实验后还原到实验前的状态；此后多出的内容都来自用户自己在 App 里的操作（`config.json` 里的「始终允许」授权、其 HumanInLoop 项目与规则文件），
我没有再用备份覆盖过它们。`agy` 的数据目录已清理，其首次引导与数据使用同意**没有**被替用户完成。

### 附录 A.2：门 G1 实测（2026-11-16，用户批准后执行；只用 Antigravity，未使用 Cursor）

| 主题 | 结果 |
| --- | --- |
| 工作区级插件（App） | 放在 sidecar 项目的工作区 `.agents/plugins/<名>/` 里即热加载：规则、MCP（`<插件>_<server>`）、hooks 都生效；hook 进程环境带 `ANTIGRAVITY_CONVERSATION_ID`、`PLUGIN_ROOT` |
| 子 Agent | 子会话触发 `SessionStart` / `PreInvocation` / `Stop`，载荷无父字段、`lastUserInput` 为空；MCP `_meta` 带 `parent_conversation_id`；子→父 `send_message` 唤醒空闲的父会话（§4.1） |
| 根会话的 `fullyIdle` | 根最后一次 `Stop(fullyIdle:false)` 之后没有根 `Stop`；`fullyIdle:true` 只来自子会话，晚 0.3 秒（§4.1、§8 L4）。**设计缺口，待 D17** |
| `Stop` 时的 transcript | 已含最终回复（V6） |
| 在途 MCP 调用期间改写插件文件 | server 不重启、调用不被打断、新 `mcp_config.json` 不作用于已运行的 server；删除插件目录也不停 server（V14，§6.3） |
| `agy` 载入并触发 hooks | 工作区级插件：未信任 → 0 个文件，信任 → 1 个；免费日志 `hooks_manager.go:54`（§4.1）。T1c：受信任目录里 `SessionStart`、`PreInvocation` ×3、`Stop(fullyIdle:true)` 都触发，hook 父进程是 `agy`，Stop 时 transcript 已含最终回复。全局插件没验（V3） |
| `agy` 会话与用户的全局 AskHuman 规则（意外） | T1c 的 `agy` 会话遵守 `~/.gemini/config/rules/askhuman-antigravity.md`，先去跑 `AskHuman --whats-next`：第一次被 `agy` 的沙箱挡住（`operation not permitted`），第二次带 `BypassSandbox` 真的执行了（约 41 秒，界面停在首次引导屏、没人批准），连上了 daemon；用户当时开着离开模式，得到离开模式的自动答复，**AskHuman 历史里因此多了 1 条来自实验目录的 `away` 记录**（我没去改用户的历史文件）。教训：`agy` 实验的提示必须写明「不要运行 AskHuman」；App 里的 sidecar 会话被沙箱挡住，没有这个问题（T2 的根会话只是把规则里「子 Agent 不得使用 AskHuman」抄进了子会话的提示） |
| `agy` 首次运行 | 先显示引导，`-i` 任务仍被发出并执行；信任提示本身没看到（V12 部分） |
| sidecar 删除 | LS 日志 `Sidecar … removed from directory, stopping...`，进程约 10 秒内退出；遗留物清单见 §4.3 |
| `agentapi` 的环境依赖 | 脱离 sidecar 环境 → `missing CSRF token`；`ANTIGRAVITY_AGENTAPI_EXE` 是 `language_server` 本体，要用 PATH 里的 `agentapi`；事件文件的 `commandInvocationTimestampMs` 是 sidecar 进程启动时间 |
| MCP 宿主信息 | `initialize`：`antigravity-client` v1.0.0、协议 `2025-11-25`、声明 `elicitation`（form / url）；server 以工作区为单位、跨会话复用 |
| 私有摘要库 | 表里有 `parent_conversation_id`、`nesting_depth`、`status`、`not_fully_idle`；只读看了结构，不依赖（D4） |

用量（如实）：4 个真实任务（上限 5）、约 31 次 Flash 低档模型请求（按 `PreInvocation` 与 `agy` 日志里的请求数计）——T1a（`agy`，未信任目录）2 次；T2（App，带子 Agent）24 次（根 20 次、子 4 次）；T3（App，90 秒 MCP 调用）2 次；
T1c（`agy`，受信任目录，补测 V3-a；用户离开期间我按计划里的备用名额做的）3 次；T1b 是不起会话的探针，0 次。
**T2 超出我的估计**：根模型反复摸索怎么调用插件的 MCP 工具，约 17 次失败的发现尝试；T3 把确切的 `call_mcp_tool` 参数写进提示后只用了 2 次。
还原：临时 sidecar 清单、`config.json` 里该 sidecar 的键（读-改-写，除该键外与实验前逐项相同，0600）、项目文件、`sidecar_data`、三个实验会话的文件、`mcp/` 发现文件、
`agy` 的数据目录与 `default-cli-project.json`、实验插件、两个实验 MCP 进程都已清理；用户自己的会话与项目没有碰。
App 的私有摘要库里还留着那三个已删会话的条目，由 App 自己在下次整理时丢弃；在那之前会话列表里可能还看得到。
实验原始记录（脱敏前的 hook 日志、transcript、sidecar 事件）保存在本机 `~/agy-hook-lab/g1/`（仓库外），P1 夹具从它脱敏生成；规格与计划被接受后删除。

### 附录 A.3：P1 实测（2026-11-16；只用 Antigravity，未使用 Cursor）

安装的是 P1 的日常版（`./scripts/install.sh`）；实验用 `ASKHUMAN_HOME` 指向实验目录的 AskHuman 实例（daemon、配置、账本与用户的完全分开；实验实例的离开模式打开，防止任何意外的提问弹窗或阻塞）。

| 主题 | 结果 |
| --- | --- |
| 免费探针（隔离 HOME，0 次模型请求） | `HOME`、`ASKHUMAN_HOME` 都指向实验目录时，`AskHuman agents mode antigravity mcp` 生成整个插件；`agy plugin list` → `askhuman (enabled)`、`Components: 1 rule, 1 MCP server, 1 hook`；`agy mcp list` 只列用户配置的 server，不列插件里的。`agy plugin disable askhuman` 在 `config.json` 写入 `plugins.askhuman.enabled=false`，`agents mode antigravity` 与 `doctor` 随即给出「插件在 Antigravity 中已停用」；`enable` 后提示消失 |
| 隔离 HOME 跑不了会话 | 隔离 HOME 里 `agy models` 报 `Please sign in`：登录态不随 HOME 走，认证又不能复制，所以会话实测只能用真实 HOME（`agy` 自带的登录）+ 实验目录 + 上面的实验实例 |
| 信任路径 | 第一次 `trustedWorkspaces` 写了 `/tmp/…`，被当作未信任（`loaded 0 …`），白跑了一个任务（§4.1）。改成 `/private/tmp/…` 后，免费探针（不带 `-i`）确认 `loaded 1 named hooks from 1 hooks.json file(s)` |
| 会话实测（1 个有效任务） | 工作区级的插件副本，hooks 命令前加了 `ASKHUMAN_HOME=…`（只为让实验 daemon 接收；App 的 hook 进程环境极简——G1 记录里只有 `HOME`、`LANG`、`PWD`、`SHELL`、`USER` 与 `AGY_*`、`ANTIGRAVITY_CONVERSATION_ID`；`agy` 是否继承自己的环境没有单独验）。注册表：启动后 3.6 秒出现 `antigravity` 会话、`pid` 是该 `agy` 进程、`working`；8.5 秒 `idle`，`lastTurnResult` = `completed`（来源 hook）；标题是第一条请求；终止 `agy` 约 20 秒后标 `ended`。模型答「是」：规则已载入（§6.1） |
| 手动喂载荷 | 真实形状的 `SessionStart` / `PreInvocation` / `Stop` 载荷经安装版 `__agent-hook`、`__stop-hook` 喂给实验 daemon：`Stop(fullyIdle:false)` 仍是 `working`，`Stop(fullyIdle:true)` 变 `idle`；载荷里的原文提示没有出现在注册表任何字段。标题由 daemon 按会话 ID 去读 transcript：daemon 与会话的 `HOME` 不同时标题为空（生产里二者相同） |
| 身份端到端（0 次模型请求） | 一个合成的 Antigravity 宿主（脚本）经 stdio 驱动安装版 `AskHuman mcp`，带 App 实测的 `_meta` 键，对着实验 daemon（离开模式，`ask` 即时返回）：根会话的 `ask` 记到 `antigravity` + 根会话 ID；子会话（`conversation_id` ≠ `root_conversation_id`）的 `ask` 也记到根；没有键的调用只带 MCP 实例分区。`show_last`：根会话的 `_meta` 拿到根与子的两条，不相干的会话得到「没有已完成的提问」，没有键的只看到没有键的那条（范围隔离）。CLI：干净的 Antigravity 环境里带**已登记**会话 ID 的提问绑定到该会话，带**未登记** ID 的只认家族；`AskHuman --show-last` 拒绝项目级回退。**`agy` 的 MCP `_meta` 键名仍没验**（合成宿主用的是 App 的键名，G4）。注意：在 Cursor 的 shell 里测 CLI 必须清环境——`CURSOR_AGENT` 的家族检测排在 `ANTIGRAVITY_CONVERSATION_ID` 之前 |
| 迁移演练（0 次请求） | 把用户真实的 `config/` 子树（旧共享规则 `rules/askhuman-antigravity.md`、`config.json`、空的 `plugins/` 与 `sidecars/`）和旧规则的台账复制到临时 HOME / 临时 AskHuman 实例（台账里的 `path` 改成副本路径，`sha256` 与 `owner` 不变），安装版跑 `agents mode antigravity mcp`：「CLI（需更新）」→ 插件发布、旧共享规则退役、台账里的 `antigravity.rule` 移除，`config.json` 逐字节不变；再跑一次 `mcp`（幂等，文件 mtime 不动）→ `cli`（无 MCP 配置）→ `none`（插件目录删除，`plugins/` 留空，`config.json` 仍不变）。旧规则退役后留下一个空的 `rules/` 目录（AskHuman 不能证明它是自己建的，所以不删）。真实目录没有被碰（演练前后逐项核对） |
| 网络 | 有一次首个模型请求在启动后约 1 分钟才发出（日志里一串 `TLS handshake timeout`）；`agy` 从启动到首个 hook 事件在网络差时可达分钟级。P4 的启动观察窗口要留够 |
| 首次引导 | 引导屏（配色 → 数据使用同意）会吞掉输入，期间发 `/exit` 不退出；会话任务本身照常执行（V12） |
| 对用户真实 `~/.gemini` 的影响 | 会话实测在真实 HOME 里创建了 `~/.gemini/antigravity-cli/`（含 3 个实验会话）和 `config/projects/default-cli-project.json`，已全部删除；目录清单与实验前逐项相同，`config.json`、`mcp_config.json` 的哈希不变；没有写真实的全局插件目录，没有碰用户的 App 会话 |

用量（如实）：3 次 `agy` 启动、约 5 次 Flash 低档模型请求（第 1 次 2 次，信任路径写错、白跑；第 2 次 1 次，网络慢、被我的 60 秒超时终止；第 3 次 2 次，有效）；探针与手动喂载荷 0 次。
还原：实验 daemon 与 GUI 宿主进程已停止，实验 launchd 条目已移除。原始记录在本机 `/tmp/agy-p1/evidence/`（仓库外），与 G1 的实验目录一起，规格与计划被接受后删除。

## 20. 反馈记录

### 2026-11-16 首轮审阅（用户经 AskHuman 回答）

| 问题 | 回答 | 处理 |
| --- | --- | --- |
| 手机 / IM 新建任务用哪个模型（P12） | 选「v1 就接入「新建任务模型与强度」档位，手机端可选」 | 记为 D11；清单与启动映射见 §12.9。我在选项里写的「要改 iOS 与协议」高估了范围：线协议已通用，实际只加 Mac 端的清单来源与启动映射 |
| YOLO 的「二次确认」（P16） | 「什么叫二次确认？我好像没有说过二次确认。」 | 确认：该措辞是我写在第三轮选项里的，用户选的是整个选项，不是原话。已在 D7 标注，澄清结果见第二轮（D14） |
| 分期顺序（P14） | 选默认顺序 | 记为 D12 |
| 其余默认（P1–P11、P13、P15）整体确认？ | 整体确认，进入实施计划 | 记入 §3.2 |
| `agy` 去留 | 保留 | 记为 D13 |

### 2026-11-16 第二轮澄清（用户经 AskHuman 回答，三题都选了我推荐的项）

| 问题 | 回答 | 处理 |
| --- | --- | --- |
| YOLO 的「二次确认」怎么处理 | 不加：只用 `/new` 现有的权限卡；sidecar 显示名带「YOLO」，设置页标注宽授权、可一键移除 | 记为 D14；§12.3、§12.4 已改 |
| 手机上的 Antigravity 模型怎么呈现 | 平铺列表（同 Cursor），不出强度行 | 记为 D15；§12.9 |
| IM 的 `/new` 要不要也能选模型 | 沿用现状：IM 不选模型，Antigravity 在 IM 里用默认档；以后另开一项再做 | 记为 D16；§12.9 |

### 2026-11-16 计划审阅（用户经 AskHuman 回答）

| 问题 | 回答 | 处理 |
| --- | --- | --- |
| 开工的执行位置（AGENTS.md） | 选「在主工作区执行」。我推荐的是独立 Dev Worktree（主工作区里有别的会话的未提交改动），用户另有取舍，按用户的决定 | 在主工作区开工：只 `git add` 自己的文件，不提交、不还原、不搁置别人的改动；主工作区里还有别人的未提交改动时，不装日常版，先经 AskHuman 问用户 |
| P2–P3 期间全局权限固定为 YOLO 时 Antigravity 怎么办 | 「不可启动」并说明原因，不静默降级；P4 做完 YOLO 后删掉这条过渡规则 | 记入计划 P2 步骤 4、8 与风险表 |
| 计划是否批准 | 批准 | 下一步：先把门 G1 的验证计划发给用户，批准后才做；然后开工 P1 |

写计划期间（01:42 起）主工作区里出现了不属于本任务的未提交改动，来自另一个在做 Cursor Desktop 的会话：`src-tauri/src/cursor_desktop/{bridge.js,runtime.rs,stall_attach.js,stall_discovery.js,stall_renderer.js}`、
`scripts/cursor-stall-discovery.node-test.mjs`、`docs/specs/cursor-desktop-background-followup.md`。本任务不碰它们。

### 2026-11-16 门 G1 验证计划（用户经 AskHuman 回答）

| 问题 | 回答 | 处理 |
| --- | --- | --- |
| G1 只用 Antigravity 可以吗（授权） | 「只用 Antigravity 对吧？可以，不需要授权。但严禁使用 Cursor」 | 我的理解（请纠正）：只用 Antigravity 的真实测试不再逐项授权，但仍受任务数预算、隔离与停止条件约束，超出（退出 App、改全局插件目录、真实目录开 YOLO）仍先问；本任务验证**严禁**使用 Cursor。已写入 §1.2 |
| 是否批准 G1 的计划 | 「批准后马上做（T1 不碰 App；T2/T3 只在 App 里多一个临时项目，约 10–15 分钟）」 | 已执行，结果见 §4、§6.3、§17 与附录 A.2；暴露的设计缺口见 §8 L4（待 D17） |

### 2026-11-16 G1 报告（用户已开启离开模式，三个问题未答复）

| 问题 | 状态 | 我的处理（离开模式：常规选择自行判断，设计变更不自行决定） |
| --- | --- | --- |
| D17：带子 Agent 的会话怎么处理（A 归并成树 / B 本版不处理 / C 只去噪） | **未答复，仍待定** | 不自行选择。P1 按「每个会话独立」写，这对 A / B / C 都是共同的底座（A 之后在其上加父子链接与树完成判定）；P3 的 Stop 卡与完成判定依赖它，做到那里仍没答复就升级 |
| 补测 V3-a（`agy` 会话里工作区级插件 hooks 是否触发） | 未答复 | 用计划里的备用名额补做了 T1c（1 个任务、3 次请求，在已信任的实验目录；用户明确说过只用 Antigravity 不需要授权），结果：会触发（附录 A.2）。同时意外让该 `agy` 会话真的调用了 AskHuman（见附录 A.2） |
| 提交规格与计划文档 | 未答复 | 按计划里「合适时提交」，只 `git add` 这两个文件，提交 `864aacad`（本地、未推送） |

P1 的实现不依赖 D17；D17 影响的是 P1 之后接线的部分（根 / 子判断、完成判定）与 P3。
