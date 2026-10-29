# Codex 委派运行静默（Delegated Run Silence）

> 状态：**已实现并验证**（2026-10-25 实现，2026-10-26 真实环境验证）：D1–D13 全部定案并落地，源码调研（§5）已完成；V1 单元测试、V2 隔离集成验证
> （真实 Codex 0.163.0 + 本地 mock Responses 端点，不计费）、V3 真实 `~/.codex` 迁移与 V4 真实 Agent 验证（用户批准，计费）均已完成，结果与偏差见 §9。
> 关联计划：`docs/plans/codex-delegated-run-silence.md`。
> 调研方式：本机一次真实事故的 rollout / `daemon.log` / 历史记录取证、`codex debug prompt-input` 渲染（只渲染，不调模型）、
> Codex CLI 0.163.0 帮助与配置命令、官方 hooks 文档、`openai/codex-plugin-cc` 默认分支源码，以及对准本机版本的
> `openai/codex` tag `rust-v0.163.0`（commit `83086188ae06`）稀疏检出后的全文源码阅读（F12–F24）。
> 调研与实现期间未启动任何会计费的真实 Agent；V2 的真实 Codex 只连本地 mock 端点，使用隔离的 `HOME` / `CODEX_HOME` / `ASKHUMAN_HOME`（F25–F31）。
> V3 / V4 经用户批准后于 2026-10-26 用真实 `~/.codex` 与真实账号进行（§9.2）。

## 1. 背景与事故取证

「委派运行」指：其他 Agent / 脚本 / 插件以非交互方式拉起的 Codex 运行（`codex exec`、官方插件
`codex-plugin-cc` 经 app-server 启动的线程等），该 Codex 会话前没有人在交互。它启动的是一个和用户日常使用
**完全相同配置**的 Codex，因此会加载 `~/.codex` 里 AskHuman 的全部集成：全局规则、MCP 服务器、hooks。

2026-10-25 在本机真实发生了一次（外部 Agent 以 `codex exec` 委派另一个本地项目的评审任务，20:47 启动、
21:14 结束）：

| 项 | 观察 |
| --- | --- |
| 线程特征 | rollout 首行 `originator=codex_exec`、`source=exec`、`thread_source=user`（Codex 0.163.0） |
| 委派提示词 | 5131 字符，没有任何 AskHuman / subagent / “不要提问”的声明（只做关键词检索，未读内容） |
| MCP `ask` | 21:03:54 发起，21:05:09 经**飞书**渠道被回答（历史记录 `channel=feishu`，经 MCP） |
| 守卫审计 | `daemon.log`：`askhuman_guard · mcp_tool · passed · codex_thread_source_passed · threadSource=user` |
| Stop 确认 | 21:14:04 弹窗被回答（历史记录 `channel=popup`、无 MCP 实例，判断为 Stop hook），6 秒后会话结束 |
| 会话登记 | agents 注册表里是一条普通 Codex 会话（pid / cwd / 起止时间齐全）→ hooks 在 `exec` 下照常运行 |

MCP 提问、Stop 确认、会话登记三条路径全部触发。危害不只是打扰：

- 外层 Agent 的工具调用在等子 Codex 结束；子 Codex 卡在 `ask` 上时（本机 `tool_timeout_sec = 86400`），外层最长被挂 24 小时。
- 手机 / 弹窗里收到的是子 Codex 的问题，用户没有外层上下文，很难答对。
- 在 Stop 确认里给了新指令，子 Codex 会被继续驱动，外层拿到的结果被污染。

### 现有防线为什么没拦住

| 现有防线 | 触发条件 | 对委派运行 |
| --- | --- | --- |
| 规则里的 subagent 例外（`~/.codex/AGENTS.md`） | 模型自认是 subagent | 子 Codex 看到了该规则，但提示词没告诉它是被委派的 → 自认主 Agent，无效 |
| 规则要求「启动 subagent 要告知其别用 AskHuman」 | 外层 Agent 自觉执行 | 那次提示词里没有；靠 LLM 自觉不可靠 |
| `SubagentStart` guard hook | Codex 自己 spawn 的 subagent | 外部拉起的是顶层线程，不触发 |
| MCP `thread_source` 守卫 | `system` / `ambient_suggestions` | `user` 或缺失 → 放行（日志已证） |
| Stop 确认静默 | 同上，或 `transcript_path == null` | 普通 `codex exec` 有 transcript → 照弹 |

## 2. 已验证的 Codex 事实

| # | 事实 | 验证方式 |
| --- | --- | --- |
| F1 | `~/.codex/AGENTS.md` 作为 **user 角色**消息（`# AGENTS.md instructions for …`）注入每个线程；AskHuman 规则就在里面。Codex 没有“按运行模式有条件加载”的机制 | `codex debug prompt-input` 渲染；真实 exec rollout 中同样出现 |
| F2 | AskHuman MCP 的 server `instructions` **不进模型上下文**（真实 exec rollout 中出现 0 次）；影响模型的只有 AGENTS.md 规则与工具 schema | rollout 检索 |
| F3 | Codex 发给 MCP 服务器的 `initialize` 客户端信息固定为 `codex-mcp-client` / `Codex`，不含 originator、线程或运行模式 | 源码 `codex-mcp/src/rmcp_client.rs` |
| F4 | `thread/start.serviceName` 只变成 OTel 指标标签；仅 5 个 OpenAI 内部白名单值会改写 originator，插件的 `claude_code_codex_plugin` 不在其中。它不在 MCP `_meta` 的 turn metadata、hook 公共输入字段、rollout `session_meta`、MCP `initialize` 里 → **AskHuman 无法读取** | 源码 `thread_manager.rs` / `responses_metadata.rs` / `session.rs`，本机 `session_meta` key 列表 |
| F5 | `originator` 等于 app-server `initialize.clientInfo.name`（本机：AskHuman 自己初始化时 name 为 `askhuman`，其 rollout 的 originator 即 `askhuman`）。官方插件的 client 名为 `Claude Code`。originator **只写进持久化 rollout**，ephemeral 线程没有 rollout | 本机 rollout 统计（28 个：`Codex Desktop` 25、`askhuman` 2、`codex_exec` 1）；插件源码 |
| F6 | 官方插件启动线程的默认参数：`approvalPolicy:"never"`、`sandbox:"read-only"`、`ephemeral:true`（rescue 类仅 `persistThread` 时持久化），不带任何 config / MCP / hook 覆盖；`thread_source` 不由插件设置 | 插件源码 `codex.mjs` |
| F7 | hooks：非托管 hook 须按 hash 审阅并信任，新增 / 改动的 hook 被跳过直到信任；用户可在 `/hooks` 里单独禁用；企业 `allow_managed_hooks_only=true` 会跳过用户 hook；多个同事件 hook 并发执行 | 官方 hooks 文档 |
| F8 | SessionStart 的 `source` ∈ `startup / resume / clear / compact / fork`；`additionalContext` 作为 developer 上下文加入；默认阈值 2500 token，超出存盘并换成预览（需 `additionalContextLimit`）；子 Agent 走 `SubagentStart`，不触发 SessionStart；`transcript_path` 可为 `null`（无 rollout 的 ephemeral 线程） | 官方 hooks 文档；`stop.rs` 既有处理 |
| F9 | Codex MCP 配置支持 `enabled`、`enabled_tools`、`disabled_tools`、`env_vars` 字段；`-c mcp_servers.<name>.enabled=false` 在配置层生效（`codex mcp list` 显示 `disabled`）。这些是**调用方**才能设置的静态配置 | 本机 `codex mcp get --json` / `list` |
| F10 | 实测开销（macOS，1134 个进程）：`ps` 查单个进程约 5 ms，全量快照约 62 ms，`AskHuman __context-recovery-hook` 进程本身约 14 ms；hook 里整链遍历进程树曾实测 253–297 ms（`docs/plans/hook-perf-walk-optimization.md`），已被移出 hook。**实现后实测**（release 二进制，CPU 时间口径，机器负载很高）：不做探测的基线约 20.8 ms；委派快路径（`transcript_path==null` / 标记命中）约 23.3 ms；交互会话 `startup` / `resume` / `compact`（要做有界祖先探测）约 42–46 ms，p95 约 52 ms，额外开销来自逐跳 `ps`。仍在 < 60 ms 的目标内但接近上限；可选后续优化：免进程的 ppid 查询（macOS `sysctl KERN_PROC_PID`、Linux `/proc/<pid>/stat`），把逐跳 `ps` 换掉 | 本机基准；既有计划；实现后用 V2 夹具实测 |
| F11 | `agents/detect.rs` 已有 `codex_subcommand()`：跳过前导全局选项取出 `codex` 之后的子命令位（`exec` / `app-server` / 无），已有单测，可识别 `exec` | 代码阅读 |

下面 F12–F24 来自对准 `rust-v0.163.0`（commit `83086188ae06`）的源码阅读，文件路径均相对 `codex-rs/`。

| # | 事实 | 出处 |
| --- | --- | --- |
| F12 | `codex exec` 是**进程内 app-server 的客户端**：`session_source = Exec`，`thread_source` 默认 `user`（`--thread-source` 可改），`--ephemeral` 决定是否落 rollout；hooks / MCP / AGENTS.md 与交互线程走同一套加载，没有 exec 专属的关闭或改写逻辑 | `exec/src/lib.rs`（约 L744、L772、L1441–1463） |
| F13 | 全局规则由 `CodexHomeUserInstructionsProvider` 读取 `$CODEX_HOME/AGENTS.override.md`（优先）或 `AGENTS.md`，在 app-server 里**无条件构造**，每个请求边界刷新。`--ignore-user-config` 只把**用户层 `config.toml`** 置空（MCP 服务器配置与 `[hooks.state]` 信任哈希一并消失），**不影响 AGENTS.md** | `codex-home/src/instructions/mod.rs`、`app-server/src/message_processor.rs`、`config/src/loader/mod.rs`、`core/src/agents_md_manager.rs` |
| F14 | 0.163.0 的**所有** hook 输入都没有 `thread_source` / `originator` / `ephemeral`。SessionStart：`session_id`、`transcript_path`（可空）、`cwd`、`hook_event_name`、`model`、`permission_mode`、`source`；Stop 另有 `turn_id`、`stop_hook_active`、`last_assistant_message`；PermissionRequest / PreToolUse / UserPromptSubmit 另有可选 `agent_id` / `agent_type`（仅子 Agent 触发时出现）。`StopCommandInput` 字段在 0.144.1 / 0.145.0 / 0.155.0 / 0.163.0 完全一致 → AskHuman `agents/stop.rs` 读取的 Stop 输入 `thread_source` 在 Codex 里不存在，该分支从不触发，Stop 静默实际只靠 `transcript_path == null` | `hooks/schema/generated/*.json`、`hooks/src/schema.rs`；各 tag 的 `StopCommandInput` |
| F15 | `SessionSource`：`cli` / `vscode`（app-server 默认）/ `exec` / `mcp` / `custom(…)` / `internal_*` / `subagent_*`；`ThreadSource`：`user` / `subagent` / `guardian_review` / `memory_consolidation` / 任意自定义字符串。两者都落在 rollout `session_meta`；MCP `_meta` 只带 `thread_source`，hook 输入两者都不带 | `protocol/src/protocol.rs` |
| F16 | SessionStart 是**惰性**触发的：线程创建时只记录“待触发的 source”，在 turn 开始、首次采样前才运行（压缩后同一入口再次触发）；顺序为 初始上下文（AGENTS.md 等）→ SessionStart 注入 → UserPromptSubmit → 用户首条输入。MCP 服务器在会话创建时就启动，**早于** SessionStart。hook 的 `transcript_path` 来自 `hook_transcript_path()`，它会**先把 rollout 落盘**（首行 `session_meta` 已写入），ephemeral / 非本地存储返回 `null`。子 Agent 会话不触发 SessionStart（`ThreadSpawn` 的 startup / fork 触发 `SubagentStart`，其余子 Agent 无 start hook） | `core/src/hook_runtime.rs`、`core/src/session/mod.rs`（L5333）、`core/src/session/turn.rs`（L294–L370） |
| F17 | `additionalContext` 变成一条 **developer 角色**、无包装标记的对话历史条目，写入历史与 rollout。SessionStart 退出码 0 + 空输出 = 无操作；纯文本 stdout 也会被当作上下文；非 0 退出 / 非法 JSON = 该 hook 标记失败（UI 可见）、上下文丢失、turn 照常继续。TUI **不展示**模型侧 hook 上下文，成功且无用户可见输出的 hook 不留痕迹。超出 `additionalContextLimit`（默认 2500 token，hook 级可配）会截成预览并把全文存盘 | `core/src/context/hook_additional_context.rs`、`hooks/src/events/session_start.rs`、`hooks/src/output_spill.rs`、`tui/src/history_cell/hook_cell.rs` |
| F18 | hook 信任：用户 / 项目层的 hook 仅当 `enabled` 且（信任哈希匹配，或 `--bypass-hook-trust`）才注册；`Untrusted`（无哈希）与 `Modified`（哈希变化）都是**静默跳过**。`hooks` 特性为 Stable、默认开，可被 `features.hooks=false` 关闭；`allow_managed_hooks_only` 下用户 hook 整体不生效。哈希含事件、matcher、命令与超时 → 改 matcher 即换身份。`[hooks.state]` 只从 User / SessionFlags 层读取，所以 `--ignore-user-config` 下所有用户 hook 都变成未信任而被跳过 | `hooks/src/engine/discovery.rs`、`hooks/src/config_rules.rs`、`features/src/lib.rs` |
| F19 | MCP：每个会话自有 `McpRuntime`；主会话 `Eager` 启动，`LazyWhenCached` 只用于子 Agent 会话；工具目录缓存是**进程内** LRU（32 项 / 30 分钟），不落盘，且不缓存空目录；服务器可在 `initialize` 能力里声明 `experimental["codex/tool-catalog-cache"].cacheable=false` 退出缓存；`notifications/tools/list_changed` **只记日志不刷新**，目录在连接建立后固定；`tools/list` 为空只是空工具集，主路径没有报错 / 重试（`startup_readiness` 默认 `connection`，AskHuman 未配置 `catalog`） | `core/src/session/mcp_runtime.rs`、`codex-mcp/src/tool_catalog_cache.rs`、`codex-mcp/src/rmcp_client.rs`、`rmcp-client/src/logging_client_handler.rs` |
| F20 | MCP 子进程环境只有白名单（`HOME`、`LOGNAME`、`PATH`、`SHELL`、`USER`、`LANG`、`LC_ALL`、`TERM`、`TMPDIR`、`TZ` 等）加配置里的 `env_vars`，**没有任何 `CODEX_*`**；Shell 工具子进程另带 `CODEX_THREAD_ID` / `CODEX_SESSION_ID` | `rmcp-client/src/utils.rs`、`protocol/src/shell_environment.rs` |
| F21 | MCP `tools/call._meta`：`callId`、`threadId` / `sessionId` / `windowId` / `itemId`、`x-codex-turn-metadata`（含 `thread_source`、`turn_trigger`、`sandbox*`、`parent_thread_id`、`subagent_kind`、`workspaces` 与 app-server 客户端附带的 extra 等）。**没有** `session_source` / `originator` / `ephemeral`；`turn_trigger` 由 app-server 客户端设置，exec 不设置 | `core/src/mcp_tool_call.rs`、`core/src/responses_metadata.rs` |
| F22 | `originator` = app-server `initialize.clientInfo.name`（进程级首个“发起方”客户端；内部环境变量 `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` 可整体覆盖）；exec 为 `codex_exec`；线程级 originator 随 rollout `session_meta` 持久化 | `app-server/src/request_processors/initialize_processor.rs`、`login/src/auth/default_client.rs` |
| F23 | **压缩不保留 hook 注入的 developer 上下文**，三条压缩路径一致：本地压缩 = 当前规范初始上下文（AGENTS.md 随之重建）+ 有预算的真实 user 消息 + 摘要；远程 V2 只保留真实 user 消息、`HookPrompt`，以及带 `client_authored` 标记的 developer 消息（app-server 客户端经 `inject_items` / 输入队列注入，且需特性 `RetainClientDeveloperMessages`），hook 的 `additionalContext` 不带该标记；token-budget 新窗口同理。所有路径都经 `replace_compacted_history()`，并排队 `SessionStart(source=compact)`；mid-turn 压缩在继续采样前立即触发，pre-turn / 独立压缩在下一个 turn 开始时触发。触发的 `additionalContext` 作为 developer 条目写在 `compacted` 记录之后。**本机实证**（Codex Desktop `0.163.0-alpha.10.2` / `0.164.0-alpha.11.1`，17 个会话共 38 次压缩）：0 次压缩把旧的 AskHuman 恢复提示留在 `replacement_history` 里，38 次压缩后都紧跟一条重新注入的 hook 条目（含同一会话内连续 6 次压缩）。注：本条更正了此前草稿中“远程压缩 V2 可能保留注入内容”的错误推断（它来自既有计划对旧版本的概括，0.163.0 只保留 client-authored 的 developer 消息） | 源码 `core/src/compact.rs`、`core/src/compact_remote_v2.rs`（`is_retained_for_remote_compaction_v2`）、`core/src/session/mod.rs`（`replace_compacted_history`、`start_new_context_window`）、`core/src/session/turn.rs`（L655–L700）；本机 `~/.codex/sessions` rollout 统计 |
| F24 | 回滚（rollback / 回退到更早的用户消息）只会裁掉紧贴被回滚 turn 边界之上的**已知上下文前缀**条目（`<permissions instructions>` 等）；会话首个真实 turn 之前的前缀条目不会被回滚越过。hook 注入的 developer 条目不属于已知前缀，且位于首条用户输入之前 → 回滚不会把它删掉 | `core/src/context_manager/history.rs`（`trim_pre_turn_context_updates`）、`core/src/event_mapping.rs`（`CONTEXTUAL_DEVELOPER_PREFIXES`） |

下面 F25–F32 是 2026-10-25 在隔离夹具里对**真实 Codex 0.163.0**（`HOME` / `CODEX_HOME` / `ASKHUMAN_HOME` 全部隔离，模型端点指向本地 mock Responses，不计费）的实测，
多数是对上面源码结论的二次确认。

| # | 事实 | 验证方式 |
| --- | --- | --- |
| F25 | MCP 子进程环境白名单**实锤**：`ASKHUMAN_HOME` 不会传给 `AskHuman mcp`。隔离 / 开发实例若要让 MCP 进程读到同一份配置目录，必须在 `[mcp_servers.askhuman]` 里写 `env = { ASKHUMAN_HOME = "…" }`（`agents update codex` 会保留用户自加的 `env` 行）。夹具最初没写，表现为 `tools/list` 为空且无 `instructions`，与“委派运行”的外观一模一样 | MCP 流量垫片记录 `initialize` / `tools/list` |
| F26 | `tools/call._meta` 实测带 `threadId` / `sessionId` 与 `x-codex-turn-metadata`（`session_id`、`thread_id`、`turn_id` 等）；`exec` 与 app-server 客户端的调用里**都没有观察到** `thread_source`、`originator`、`ephemeral`。与 F21 一致：MCP 侧只能靠父进程命令行（exec）或 SessionStart 写下的标记（其余） | 夹具里让 mock 端点下发函数调用，记录 MCP 入参 |
| F27 | 模型请求里是否出现 AskHuman 工具（命名空间 `mcp__askhuman`），取决于 MCP 服务器是否在**首个模型请求之前**就绪（`tool_search_always_defer_mcp_tools=true` 为默认）；机器负载很高（load avg 约 400）时存在竞态，首个请求可能没有 MCP 工具。这是 Codex 既有行为，与 AskHuman 是否隐藏工具无关；做 A/B 对比时要先等 MCP 就绪 | 多轮重复运行；`RUST_LOG=info` 下 `tool_count=6` 稳定 |
| F28 | `codex review` 会起**两个** `AskHuman mcp` 进程（两个线程各一个）；两个都落在 exec 判定下，各自 `tools/list` 为空 | 夹具观察（MCP 垫片日志） |
| F29 | `--ignore-user-config`：不加载任何 MCP 服务器，用户 hook 因信任哈希随用户层 `config.toml` 一起消失而不运行（印证 F13 / F18）；迁移后的夹具里请求体不含任何 AskHuman 文本 | 请求体检查 |
| F30 | **压缩复现**（真实二进制 + mock 摘要端点，`thread/compact/start`）：压缩丢弃 hook 注入的 developer 条目；随后 `SessionStart(source=compact)` 重新注入“协议 + 恢复句”（3274 字符，起始为 3130 字符协议），下一次请求里恰好一条协议条目、没有重复。app-server 不发 `thread/compacted` 通知，客户端要用 `turn/completed` 判定结束 | 请求体逐条检查；app-server 驱动脚本 |
| F31 | `additionalContextLimit: 0` 实测生效：3130 字符的协议没有被截成预览（默认阈值 2500 token 会触发）；AskHuman 计算的信任哈希与 `hooks/list` 的 `currentHash` 逐字节一致 | `hooks/list` 与 `config.toml` 对照；请求体长度 |
| F32 | Codex 把 hooks 文件路径**规范化**后再写进信任 key（`/tmp` → `/private/tmp`）；AskHuman 用未规范化路径生成 key。真实 `$HOME` 是规范路径时无影响，只会在 `$HOME` 含符号链接时让信任对不上——既有限制，本任务未改，夹具因此必须用规范路径（`/private/tmp/…`） | 夹具信任失败的排查 |

## 3. 目标与非目标

目标：

1. 委派运行里，AskHuman 的**交互面**表现与「未集成 AskHuman」一致：不弹窗、不发 IM、不写项目待办、不弹结束确认、不弹权限审批。
2. 普通交互会话（TUI、桌面端、AskHuman 自己的 IM 任务）零回归；任何识别失败都按交互处理（fail-open）。
3. 给用户一个明确的开关来恢复委派运行里的 AskHuman。

非目标：

- 不修改 Codex，不要求调用方配合（`--ignore-user-config`、`-c …enabled=false` 之类只能写成文档配方，不是产品方案）。
- 不拦截委派运行自己调用的 `AskHuman` CLI（没集成时二进制也在 PATH 上，同样能用）。
- 不改会话登记：委派会话仍作为普通 Codex 会话出现。
- 不为委派线程构造“假的人类回答”或“假的结束批准”。

## 4. 已确认决策（用户，2026-10-25）

### D1：原则 —— 与未集成一致

对被判定为委派的运行，AskHuman 的交互面应与未集成时一致。hook 层的实现是**无输出、让 Codex 走原生流程**，
不是替 Codex 拒绝或批准任何请求（用户原话：“希望效果就是和没有集成 AskHuman 是一样的”）。

### D2：默认静默，集成页给一个默认关闭的开关

- 默认：委派运行静默。
- 在设置的 **Codex 集成页面**（`src/views/settings/IntegrationTab.vue` 的 Codex 行）增加一个开关，**默认关**。
  语义：「允许被委派的 Codex 运行使用 AskHuman」。关 = 委派运行静默；开 = 委派运行与普通线程一致。
- 纯偏好，不安装 hook；覆盖 D6 的两类委派。只对**新启动**的运行生效（已启动的 MCP 进程缓存的判定保持到进程结束）。
- 沿用现有配置机制，同时可经命令行配置：`AskHuman agents delegated codex [on|off]`（`on` = 允许），偏好文件 `delegated-preferences.json`，与 Stop / Permission 开关并列（D13）。
- 本期不提供环境变量放行。

### D3：静默范围

- MCP 五个交互 / 副作用工具：`ask`、`whats_next`、`show_last`、`todo_add`、`todo_update`；只读 `todo_list` 不受影响。
- Stop 确认。
- PermissionRequest 权限审批：hook 无输出，回落 Codex 原生处理。
- **不拦 CLI**：委派运行若自己调用 `AskHuman` CLI，照常响应；靠“不提示它去调”降低概率（用户原话：“我们要做的是不要在 Prompt 或者规则里提示它来调 CLI”）。
- **会话登记不变**（Agent 控制台 / IM status / watch 里仍显示，只是不再提问）。

### D4：exec 运行里 MCP 工具不出现

被判定为 `codex exec` 的运行里，AskHuman MCP 的 `tools/list` 返回空（等同未集成），而不是“工具保留、调用时报错”。
判定在 MCP 进程启动时做一次：沿父进程链找到最近的 `codex`，其子命令位为 `exec`（复用 `codex_subcommand()`），
**不进入每次工具调用的 hook 热路径**。（Codex 对空工具列表的处理、MCP 进程的生命周期与可用环境见 §5 O2。）

### D5：识别原则

- fail-open：任何识别失败 / 信号冲突都按交互处理，宁可漏拦也不误伤交互会话。
- 不在每次工具调用的 hook 里遍历进程树；探测是**有界祖先探测**（沿父链找到第一个 `codex` 即停），每个 MCP 进程 / 会话只算一次并缓存。
- 复用现有 `askhuman_guard` 审计行，为新信号增加 `reason`；不记录提问正文或任意未识别的 metadata。

### D6：app-server 系纳入，尽力识别

官方插件等经 app-server 启动的委派线程纳入本期，尽力识别（用户已知晓误伤交互线程的风险）：

- **显式 ephemeral 信号**：hook 输入的 `transcript_path == null`；在 SessionStart 落一个按 session id 的标记文件，
  MCP / Stop / 权限路径读取。不采用“查不到 rollout 文件就当临时线程”的字面方案（rollout 懒创建，开局或旧版本会误判）。
- **originator 名单**：rollout 首行 `session_meta.originator`，仅对持久化线程有效；初始名单 `codex_exec`、`Claude Code`，常量可扩展。rollout 首行的两个信号（`source == exec`、`originator`）**只在 `startup` / `clear` 使用**，`resume` / `fork` 只看当前运行信号（D13）。
- **`serviceName` 不可用**（F4）。
- 这类线程无法隐藏工具，MCP 工具在调用时返回带指引的错误（沿用现有 `isError` 本地拒绝形态，文案需改为非 ambient 语境）。
- 上游诉求：希望 Codex 在 MCP `tools/call._meta` 里提供 `originator` 与 `ephemeral`，已记为 AskHuman 项目 todo #6。

### D7：不分期

用户明确不要分期，本任务一次做完整方案；先把决策记录在 spec，再用 Codex 源码确认最佳方案（D8）。

### D8：协议交付改为 SessionStart hook 动态注入（2026-10-25 源码调研后确认）

Codex 上 AskHuman 交互协议的**唯一载体**改为 SessionStart hook（沿用现有“集成模式托管的恢复 Hook”形态），`~/.codex/AGENTS.md` 不再写 AskHuman 区块。
hook 在同一处完成「委派判定 → 注入或不注入 → 写标记」，D1–D6 的其余行为不变。依据与代价见 §5.2。

### D9：resume / fork 去重规则（2026-10-25）

选定“按 rollout 标记去重”。用户要求先确认 compact 对上一次 developer 上下文的处理，已核对：**压缩不保留 hook 注入的上下文**（F23），
所以下面的规则成立：

- `startup` / `clear` / `compact`：总是注入（compact 后旧注入一定已不在历史里）。
- `resume` / `fork`：读取 `transcript_path` 指向的 rollout，从文件末尾**向前**逐行扫描：某一行是 developer 角色消息且含本协议标记 → 已注入，不再注入；
  遇到 `compacted` 记录即停止并判定“未注入”（压缩已丢弃之前的注入，该记录自身 `replacement_history` 里出现的标记**不算**证据）；没有 `compacted` 记录则扫到文件头。
  找不到、读取失败、文件不可读、超出预算（64 MiB 或 300 ms），一律**注入**（宁可重复，不可缺失）。
  - 2026-10-26 用户裁定：保持实现，D9 文字改成现状。与原文字的差别只在“压缩记录自身带着标记”这一种情形（远程压缩 V2 会保留 app-server 客户端的 `client_authored`
    developer 消息）：按现状多注入一次，不会漏注入；真实压缩路径不受影响（F23：38 次压缩 0 次保留）。
- 标记 = 协议文本里固定的一行 ASCII（含模式与文本哈希，如 `askhuman-protocol:<mode>:<hash8>`）；模式、风格、自定义文本、版本任一变化都会改变哈希，旧注入不再算“已有”。
- 该扫描只在 `resume` / `fork` 且已判定为交互会话时进行，不进入任何每次工具调用的 hook。

### D10：存量迁移由集成页「需更新」触发（2026-10-25）

已有静态区块的用户，集成页显示「需更新」，用户点一次，执行原子迁移：先安装并信任新的 hook，再删除 `~/.codex/AGENTS.md` 里的 AskHuman 区块；
任一步失败回滚到迁移前状态，任何时刻都不能出现“hook 与区块都没有”。不做升级后自动迁移。

### D11：不为低版本 Codex 做回退（2026-10-25）

低于 hook 能力地板的 Codex，集成页提示“该版本不支持协议注入”，不并存静态区块；版本要求与现有 hook 功能一致，地板取 `0.144.1`（D13）。

### D12：并入清理 Stop `thread_source` 死分支（2026-10-25）

Codex 的 hook 输入里没有 `thread_source`（F14），`agents/stop.rs` 读取它的分支从不触发；本任务一并删除，并在注释里写明 Stop 静默只依赖 `transcript_path == null`
与新增的委派判定。MCP 侧读 `_meta` 的 `thread_source` 守卫不受影响。

### D13：计划审阅时确认的实现细节（2026-10-25）

计划里 5 项 spec 未明确决定的细节，用户全部按建议确认：

1. **版本地板 `0.144.1`**（现有 Stop / 权限 / 生命周期 hook 的验证基线）；读不到版本（Codex 未安装）按支持处理。
2. **命令与偏好名**：`AskHuman agents delegated codex [on|off]`（`on` = 允许委派运行使用 AskHuman），偏好文件 `delegated-preferences.json`，与 `permission` / `stop` 并列。
3. **`additionalContextLimit: 0`**（不限）：避免长自定义协作风格被截成预览导致协议静默失效；代价是 hook 身份多一个字段，AskHuman 的信任哈希要同步扩展，并用真实 Codex `hooks/list` 的 `currentHash` 做回归向量。
4. **rollout 首行信号只用于 `startup` / `clear`**：它描述线程“是怎么创建的”而非“现在谁在跑”，避免把用户在 TUI / 桌面端恢复的 exec 旧线程（`resume` / `fork`）误判为委派；`resume` / `fork` 只看当前运行信号（null `transcript_path`、进程探测），`compact` 沿用已有标记。代价：官方插件经 app-server `resume` 自己的持久化线程时识别不到（只能靠 ephemeral 信号）。这是对 D5 / D6 的细化。
5. **Stop / 权限在标记缺失时做一次有界进程探测兜底**，命中则写标记：让 exec 的静默不依赖 SessionStart hook 是否已迁移 / 已信任；这两条路径低频，且本来就要联系人，成本可忽略。

## 5. 源码调研结果与 D8 依据

> 调研对象：`openai/codex` tag `rust-v0.163.0`（commit `83086188ae06`，2026-10-15，与本机 `codex-cli 0.163.0` 一致）的 `codex-rs` 稀疏检出；
> hooks 输入另抽查了 `rust-v0.144.1` / `0.145.0` / `0.155.0` 的 `StopCommandInput`。事实编号见 §2 的 F12–F24。

### 5.1 调研清单结论

- **O1 规则交付**：Codex 没有“按运行模式有条件加载 AGENTS.md”的机制（F1、F13）；`--ignore-user-config` **不会**跳过 `$CODEX_HOME/AGENTS.md`，只清空用户层 `config.toml`，MCP 服务器与 hook 信任哈希随之消失（F13、F18）——调用方用官方手段隔离用户配置后，静态规则仍在上下文里，工具与 hook 却没了。SessionStart `additionalContext` 是 developer 角色条目，位于初始上下文之后、用户首条输入之前，写入 rollout（F16、F17）；压缩**不保留**旧注入，压缩后 `SessionStart(compact)` 会重新触发（F23，源码三条压缩路径 + 本机 38 次压缩实证）；resume / fork 会重放历史并再次触发 SessionStart，需要去重（D9）；回滚不会删掉首条用户输入之前的注入（F24）；全新 subagent 不触发 SessionStart（F16）。**没有比 hook 更合适的外部注入 / 排除机制**（线程级指令提供方是 Codex 内部扩展点，外部不可用）。
- **O2 MCP**：服务器按**会话**启动，主会话 Eager，早于 SessionStart（F16、F19）。`tools/list` 为空只是空工具集，主路径无报错 / 重试 / 失败标记；目录在连接建立后固定（`list_changed` 只记日志），所以“隐藏工具”只能在 MCP 进程启动时决定，**D4 成立**；工具目录缓存是进程内的，`exec` 是新进程，不会让旧目录覆盖动态结果（F19）。最初设想过声明 `codex/tool-catalog-cache` 的 `cacheable=false`，最终**不声明**（2026-10-26 用户裁定，见 §9.3 第 2 项）。MCP 子进程环境里没有 `CODEX_*`，`_meta` 里没有来源 / originator / ephemeral（F20、F21），所以 exec 仍只能靠父进程命令行（D4）。
- **O3 hook 输入**：没有文档未列出的来源字段；所有 hook 输入都没有 `thread_source`（F14）。**顺带发现**：`src-tauri/src/agents/stop.rs` 读取的 Stop 输入 `thread_source` 在 Codex 里不存在（0.144.1–0.163.0 一致），该分支从不触发，Stop 静默实际只靠 `transcript_path == null`；委派判定不能依赖它。hook 子进程仍沿用现有 `hint_pid` 探测路径，本次未改。
- **O4 线程来源**：`exec` 的显式标记是 rollout `session_meta.source == "exec"` 与 `originator == "codex_exec"`，hook 里可读（hook 运行前 rollout 已落盘，F16），MCP 里读不到（F21）；`codex exec resume` 恢复旧线程时首行是否更新**未核实**，所以仍以父进程命令行的 `exec` 子命令为主判据，rollout 信号为补充（与 §7 一致）。`ThreadSource` 没有“非交互”取值（F15）。
- **O5 ephemeral**：ephemeral 线程的 hook 输入 `transcript_path = null`，SessionStart 照常触发；非 ephemeral 线程在 hook 运行前 rollout 已落盘（F16）。MCP 侧“查不到 rollout”不是可靠信号，仍用 SessionStart 落标记文件（D6 不变）。
- **O6 exec 专属**：`exec` 只是进程内 app-server 的客户端，没有专属的 hooks / MCP / AGENTS.md 开关（F12）；调用方能用的只有 `--ignore-user-config`、`-c`、`--disable`、`--ephemeral` 等通用旗标，不能指望。

### 5.2 D8 依据：规则改由 SessionStart hook 动态注入（用户已确认）

**结论（用户已确认，见 D8）**：Codex 上 AskHuman 交互协议的唯一载体改为 SessionStart hook（沿用现有“集成模式托管的恢复 Hook”形态），`~/.codex/AGENTS.md` 不再写 AskHuman 区块。hook 在同一处完成「委派判定 → 注入或不注入 → 写标记」，其余 D1–D6 不变。

**为什么（均为源码事实）**：

1. **静态方案满足不了 D1 / D3**：① CLI 模式下规则必然写着“请运行 `AskHuman` 命令”，委派运行读到就会去调，而 D3 明确“不要在规则里提示它调 CLI”；隐藏 MCP 工具只覆盖 MCP 模式。② `--ignore-user-config` 是调用方隔离用户配置的官方手段，它清掉 MCP 与 hook，**却留下 AGENTS.md**（F13）——静态方案下最“干净”的调用反而得到“必须调用 AskHuman，但工具不存在”的上下文，CLI 模式下还能经 shell 把用户吵醒；动态方案下它得到的是零注入。
2. **hook 能用显式信号一次判定**：`transcript_path == null`、rollout 首行的 `source == exec` / `originator`（F14–F16，hook 运行前 rollout 已落盘），再加有界父进程探测（覆盖 `exec resume`）；判定与注入同处完成，不需要事后“说明抵消”。
3. **注入质量不降反升**：developer 角色高于现用 user 角色的 AGENTS.md，位置紧跟初始上下文、先于用户首条输入（F16、F17）；TUI 不显示模型侧上下文（F17）；全新 subagent 不触发 SessionStart（F16），“规则里写 subagent 例外”的负担大幅下降；压缩不保留旧注入，同一 hook 在 `compact` 时重新注入（F23）。
4. **基建已具备**：用户机上 SessionStart 已有两条 AskHuman hook（生命周期 + `compact` 恢复）；Codex hook 信任写入与漂移检测已有实现；协议文本约 3.2K 字符（≈1K token，低于默认 2500 token 阈值，F17）；CLI / MCP / 无集成三种模式切换变成“只改 hook 输出”，不再改写 AGENTS.md，也不改变 hook 命令身份（不触发重新信任）。

**代价与缓解**：

| 代价 | 缓解 |
| --- | --- |
| 最大风险：失败从显式变**静默**。hook 未信任 / 哈希漂移 / `features.hooks=false` / `allow_managed_hooks_only` 会让**交互会话**也失去协议（F18） | 沿用并加强现有信任同步与「需更新」；集成页把“协议交付”显示为 hook 状态；`AskHuman doctor` 检查；hook 执行失败在 Codex UI 里可见（F17），只有“未信任”是静默的；注入文本带版本标记便于诊断 |
| resume / fork 会重放含旧注入的历史，再注入会重复（压缩不保留旧注入，F23，所以 `compact` 总是重新注入） | D9 的标记去重；重复只浪费 token，不影响正确性 |
| 存量用户 AGENTS.md 里已有静态区块 | 原子迁移：先装好并信任 hook，再删区块，失败回滚；期间不允许出现“两边都没有” |
| developer 条目的模型遵从度未经真实 Agent 验证 | 实现后做小规模真实验证——**需先经你批准的计费测试** |
| 低于 hook 能力地板的 Codex（及云端编排）拿不到协议 | 与现有 hook 功能同一版本要求（D11） |

**对比（供回看）**：

| 方案 | 对委派运行 | 主要代价 / 风险 |
| --- | --- | --- |
| 静态 AGENTS.md + 补丁层（隐藏 MCP 工具、hook 静默、SessionStart 追加“本次不适用”说明） | 规则仍被注入，靠说明抵消；CLI 模式与 `--ignore-user-config` 无法满足 D1 / D3 | 实现小、失败可见；每个会话仍多跑一次 SessionStart hook；matcher 要从 `compact` 扩到全部 source，老用户需点一次“更新” |
| **hook 动态注入（已选定）** | 字面零注入，判定与注入同处完成 | 失败从显式变静默（见上）；重构安装 / 迁移 / 回滚 / doctor / 预览 / i18n；模型遵从度需真实验证 |

### 5.3 子决策结果

| 子决策 | 用户选择（2026-10-25） | 落点 |
| --- | --- | --- |
| D8 本身 | 动态注入：SessionStart hook 成为协议唯一载体，AGENTS.md 不再写 AskHuman 区块 | D8 |
| resume / fork 去重 | 按 rollout 标记去重（前提“compact 对上一次 developer 上下文的处理”已核对，见 F23 与 D9） | D9 |
| 存量迁移触发 | 集成页「需更新」，用户点一次原子迁移，失败回滚 | D10 |
| 低版本 Codex | 不回退，集成页提示“该版本不支持协议注入” | D11 |
| Stop `thread_source` 死分支 | 并入本任务清理 | D12 |

## 6. 被否决 / 暂不做

- 按 `serviceName` 识别：取不到（F4）。
- 依赖 hook 输入的 `thread_source` 判定：Codex 不提供（F14）。
- 仅按 rollout `session_meta.source` 判 exec：`exec resume` 场景不可靠，只作补充信号（O4）。
- “查不到 rollout 文件就当临时线程”：改为显式 ephemeral 信号（D6）。
- 环境变量放行开关：用户选择集成页开关（D2）。
- 隐藏 / 标记委派会话：用户选择保持现状（D3）。
- 分期交付：用户不需要（D7）。
- 拦截 CLI：用户认为应照常响应（D3）。

## 7. 风险与失败模式（初稿）

| 风险 | 说明 | 缓解 |
| --- | --- | --- |
| 误判交互线程为委派 | ephemeral / originator 属启发式信号 | 开关、审计行（含信号名）、fail-open；exec 信号为高置信的进程命令行 |
| `codex exec resume` 与 TUI `codex resume` 互相恢复 | rollout 首行的 `source` 是创建时的值（resume 是否更新首行未核实），而进程是当前运行方式 | 以当前进程命令行为主，rollout 信号仅作补充；每次 SessionStart 触发都重新判定并覆盖 / 清除标记（O4） |
| 规则仍在上下文里 | 静态 AGENTS.md 无法按模式过滤（F1、F13，`--ignore-user-config` 也不能） | D8 动态注入已消除：委派运行零注入 |
| 模型在工具缺失时反复尝试 | 静态规则要求“必须 whats_next / 不得结束”而工具不存在 | 动态注入下委派运行根本没有规则。需真实 Agent 验证（须先经用户批准） |
| hook 信任漂移 / 特性被关 | 动态注入下交互会话也会静默失去协议（F18） | 见 §5.2：信任同步、「需更新」、doctor、集成页展示 hook 状态 |
| resume / fork 重复注入 | 历史重放带着旧 developer 条目（F17）；压缩不保留旧注入（F23），所以 `compact` 总是重新注入、不需要去重 | D9 的标记去重；读失败倾向注入，重复只耗 token |
| 存量迁移出现“两边都没有” | 先删区块后装 hook 失败 | 原子迁移，先装后删，失败回滚 |
| 探测开销 | 每个会话 / MCP 进程一次有界祖先探测 | F10 数据；不进入工具调用级 hook |
| Windows / Linux | 进程命令行读取方式不同 | 复用 `detect.rs` 既有的跨平台 `inspect_process` / `process_chain`，补各平台用例 |

## 8. 决策来源（2026-10-25 问答）

| 议题 | 用户回答 |
| --- | --- |
| 默认策略 | 效果等同没有集成 AskHuman；权限等 hook 不是“拒绝”，而是不介入 |
| 放行开关 | 提供全局设置项；在 Codex 集成页加一个开关，默认关 |
| 静默范围 | 要：Stop 确认、权限审批；CLI 不拦（exec 自己调了就该响应），要做的是不在提示词 / 规则里提示它调 CLI |
| 会话登记 | 保持现状 |
| app-server 系 | 纳入，尽力识别；信号用显式 ephemeral（hook `transcript_path==null` + 标记文件）+ originator 名单，并把上游诉求记为待办 |
| exec 里的 MCP | 工具不出现（`tools/list` 为空） |
| 提示词注入 | 追问是否还会注入、能否对这类运行不注入；倾向动态注入，但要求先记决策、再用源码确认最佳方案；不要分期 |
| D8 协议载体 | 选「动态注入」：SessionStart hook 成为协议唯一载体，AGENTS.md 不再写区块；委派判定与注入同处完成 |
| D9 去重 | 选「按 rollout 标记去重」，并要求先确认 compact 对上一次 developer 上下文的处理（结论：不保留，压缩后由 `SessionStart(compact)` 重新注入，规则成立） |
| D10 迁移 | 选「集成页提示需更新，用户点一次原子迁移，失败回滚」 |
| D11 低版本 | 选「不回退，集成页提示该版本不支持协议注入」 |
| D12 死分支 | 选「并入本任务清理，注释写明 Stop 静默只依赖 transcript_path」 |
| D13 计划细节 | 版本地板 0.144.1；`agents delegated codex [on\|off]` + `delegated-preferences.json`；`additionalContextLimit: 0`；rollout 信号只用于 startup / clear；Stop / 权限做有界探测兜底；计划审阅通过，不分期，开始实现（动真实 `~/.codex` 与任何计费测试前再问） |

## 9. 实现结果、验证与偏差（2026-10-25）

### 9.1 落点

| 决策 | 落点 |
| --- | --- |
| D4 / D5 / D6 判定与标记 | `agents/delegated.rs`（信号、标记文件、`silenced_with`）、`agents/detect.rs`（有界祖先探测 `process_chain_bounded`、精确 argv、`nearest_agent_invocation`） |
| D8 / D9 协议 hook | `agents/context_recovery.rs`（source × 委派 × 模式的输出矩阵、rollout 去重扫描）、`integrations/codex_protocol.rs`（交付状态、版本地板、原子迁移）、`integrations/agent_context_recovery.rs`（Codex 新身份：无 matcher、`timeout 30`、`additionalContextLimit: 0`）、`integrations/agent_permission.rs`（信任哈希覆盖该字段）、`prompts.rs`（标记行） |
| D3 静默 | `mcp/ask.rs`（启动期隐藏 + 调用期标记守卫）、`agents/stop.rs`、`permissions.rs` |
| D2 / D13 开关与界面 | `integrations/agent_delegated.rs`、`cli/agents_cmd.rs`、`cli/doctor.rs`、`commands.rs` / `app/invoke.rs`、`src/views/settings/*`（Codex 行的开关与协议状态） |
| D10 / D11 迁移与版本 | `integrations/agent_mode.rs`、`integrations/agent_rules.rs`、`integrations/codex_protocol.rs` |
| D12 清理 | `agents/stop.rs` 删除 `thread_source` 死分支 |

审计沿用 `askhuman_guard` 行，新增 reason：`codex_transcript_path_null`、`codex_delegated_exec_process`、`codex_delegated_rollout_source`、
`codex_delegated_rollout_originator`、`codex_delegated_marker:<signal>`（MCP 调用期与 Stop / 权限读到标记）、`codex_protocol_injected`、
`codex_protocol_already_injected`、`codex_protocol_legacy_block`。未改 IPC，不涉及 `PROTOCOL_VERSION`。

### 9.2 验证

- **V1**：判定 / 标记 / 去重扫描 / hook 输出矩阵 / 信任哈希向量 / 迁移与回滚故障注入 / MCP 隐藏与守卫 / Stop 与权限静默 / CLI 与界面的单元测试全部通过。
- **V2**（真实 Codex 0.163.0 + 本地 mock Responses，隔离 `HOME` / `CODEX_HOME` / `ASKHUMAN_HOME`，不计费）：
  - `codex exec`：开关关 → 请求体既无协议 developer 条目也无 AskHuman 工具，Stop / 权限不触达 daemon；开关开 → 协议条目位于初始上下文之后、用户输入之前，且有 AskHuman 工具。
  - `exec resume`（有 / 无标记、开关关且带历史）、`--ephemeral`、`codex review`（F28）、`--ignore-user-config`（F29）均符合预期；resume 去重按标记生效，没有重复注入。
  - 遗留 AGENTS.md 区块：hook 在 `startup` 不再叠加注入；用 `agents update codex` 原子迁移后，请求体与“未集成”一致。
  - app-server：ephemeral 线程与持久化的 `Claude Code` originator 线程都被判为委派，`ask` 在调用期被拒绝并返回委派文案、没有弹窗；交互型控制客户端不受影响。
  - 压缩：见 F30，协议在压缩后恰好重新注入一次。
  - `hooks/list` 的 `currentHash` 与 AskHuman 计算的信任哈希一致（F31）。
- **V3**（真实 `~/.codex`，用户批准，2026-10-26）：`AskHuman agents update codex` 一次完成原子迁移（0.5 s；迁移前已备份 hooks.json / config.toml / AGENTS.md）。
  - `hooks.json` 语义上只有 `SessionStart[1]`（原压缩恢复 hook）变了：去掉 `matcher: "compact"`、新增 `additionalContextLimit: 0`，`timeout` 仍为 30；用户自己的 hook 原样保留。
    文本差异大，是因为 AskHuman 自管的条目被按其格式重写、`Stop` 挪到了末尾，纯格式。
  - `config.toml` 只有该 hook 的 `trusted_hash` 变化（`post_tool_use:1:0` 那段只是挪了位置；另有一行 `default_tools_approval_mode`，来自既有的 MCP 配置写入逻辑，与本任务无关）。
    `AGENTS.md` 整个文件原本只有 AskHuman 区块，迁移后是 0 字节空文件（§9.3 第 8 项）。
  - 真实 CLI 0.163.0 与桌面端自带 0.164.0-alpha.11.1 的 `hooks/list`：8 条 AskHuman hook 全部 `trusted`，新 hook 的 `currentHash` 与写入的 `trusted_hash` 一致，两个版本算出同一个哈希。
    `doctor`：协议 需更新 → 正常，委派运行 = 静默；开关保持默认关。对真实 hook 的 dry-run：交互 `startup` 注入 3130 字符，`transcript_path=null` 输出 0 字节。
- **V4**（真实账号，用户批准，计费；模型 `gpt-6-astra`；`~/.askhuman` 的临时偏好已还原；共 3 次 `codex exec`、1 个 app-server 线程的 2 个回合与 1 次真实压缩）：
  - ① 真实 `codex exec`（0.163.0，reasoning=low）、开关关：模型回答 `NONE`（看不到任何 AskHuman 工具）；rollout 首行 `source=exec / originator=codex_exec`，3 条 developer 消息里没有协议标记；
    审计 `protocol_hook suppressed codex_delegated_exec_process`、`stop_confirmation suppressed codex_delegated_marker:exec_process`；无弹窗，18 s 退出。
  - ② 同一命令、开关临时打开：模型列出 `mcp__askhuman` 下全部 6 个工具；rollout 里 3130 字符协议在初始上下文之后、用户输入之前；模型按协议在收尾调用 `whats_next`（弹窗，用户选「结束本轮」），
    行为等同交互会话；测完开关关回。
  - ③ 桌面端自带 0.164.0-alpha.11.1 的 app-server、交互客户端（`originator=askhuman-v4-test`，reasoning=medium）新线程：`startup` 注入协议（rollout 里的 developer 条目）；
    模型遵守协议，先 `ask` 再 `whats_next`，最后 `[user_confirmed_end_turn]`。
  - ④ 同一线程里 `thread/compact/start`（真实远程压缩）：`replacement_history` 的 8 条里没有协议标记（压缩丢弃 hook 注入，F23 在真实服务上成立）；随后 `SessionStart(compact)` 恰好重新注入一次
    （3274 字符 = 3130 + 恢复句）；模型按恢复句先调 `show_last`，再 `ask`、`whats_next`，仍遵守协议。
  - 测试痕迹已于 2026-10-26 清理：3 个 exec 会话（含装入最终构建后的一次开关关回归，同样得到 `NONE`）与 1 个 app-server 线程经 `thread/archive` 归档到 `~/.codex/archived_sessions/`（可恢复），标记文件与偏好文件已删除，迁移备份与临时目录已删除。

### 9.3 与文档 / 计划的偏差及实现期发现

1. **D9 扫描比原文字更严格（已裁定，2026-10-26）**：实现只统计最后一条 `compacted` 记录**之后**的 developer 条目，不把 `compacted` 记录自身 `replacement_history` 里出现的标记当作“已注入”
   （原 D9 文字把该记录的 `replacement_history` 也算进去）。方向是“宁可重复注入”，对真实压缩没有影响（F23：38 次压缩 0 次保留），只在远程压缩 V2 保留了 app-server 客户端的 `client_authored`
   developer 消息时多注入一次。用户裁定保持实现，D9 文字与计划 §6.4 已改成现状。
2. **`codex/tool-catalog-cache` 能力（已裁定，2026-10-26：不声明）**：实现里曾在 MCP `initialize` 声明 `cacheable: false`，现已删除（`mcp/ask.rs` 的 `get_info` 恢复为只开 tools 能力，
   两个服务器测试改为断言没有 `experimental` 能力）。依据 Codex 源码（`codex-mcp/src/tool_catalog_cache.rs`、`connection_manager/tool_catalog.rs`、`rmcp_client.rs`）：
   缓存是进程内 LRU（32 项 / 30 分钟、不落盘、不缓存空目录），按连接身份（传输 / 命令 / 环境等，不含二进制 mtime）取键，每次新建 MCP 连接都会用该连接首次 `tools/list` 的结果刷新；
   只有“可选服务器在首个请求前还没启动好”时才拿缓存给首步用。所以去掉声明**不会让新版本更慢生效**：新线程的实际目录来自新启动的 `AskHuman mcp` 进程，最坏只是升级后长驻 Codex 里
   新线程**首步**还带着上一版的工具描述，下一步就是新版；真正决定新版本何时生效的是已运行的 `AskHuman mcp` 进程的寿命。声明在最终设计里没有用处（exec 是新进程、缓存本来就空；
   app-server 里各会话目录相同，委派静默靠调用期拦截），反而放弃了缓存对 F27 首请求竞态的兜底，还依赖上游标成“仅供开发与测试，生产服务器不应使用”的开关。
3. **低于版本地板的 Codex**：安装 / 更新路径直接返回可读错误（不并存静态区块，D11），集成页与 `doctor` 显示“该版本不支持协议注入”；读不到版本按支持处理。
4. **`AgentReadiness` 多两个信息性字段**（`protocolMinimumVersion`、`protocolSupported`，只给 GUI 用，不影响 `ready`）；前端 Codex 行原来的“上下文恢复 Hook”条目并入“协议（SessionStart Hook）”条目。
5. **MCP 环境白名单（F25）**：开发实例 / 隔离环境里要在 `[mcp_servers.askhuman].env` 显式带 `ASKHUMAN_HOME`，否则 `AskHuman mcp` 读到的是默认配置目录；这是 Codex 的行为，不是新引入的问题，但会让“隔离环境里的 MCP 看起来像委派运行”。
6. **Codex 首请求竞态（F27）**：交互会话在 MCP 服务器就绪前就发出首个模型请求时，本来就拿不到 AskHuman 工具；与委派静默无关，只是做 A/B 验证时要先等 MCP 就绪。
7. **信任 key 路径未规范化（F32）**：既有限制，本任务未改。
8. **迁移后可能留下空的 `~/.codex/AGENTS.md`**：区块是文件里唯一内容时，沿用既有 `remove_block` 约定保留空文件（V3 实测：用户的文件迁移后是 0 字节）。
9. **hook 开销（F10）**：交互会话的 `startup` / `resume` / `compact` 约 42–46 ms（p95 约 52 ms），接近 60 ms 目标上限；若后续需要压低，优先做免进程的 ppid 查询。
10. **迁移时已打开的 Codex 会话会在下一步失去旧协议（V3 前读源码发现）**：Codex 每个采样步骤都会重读 AGENTS.md，变化以 diff 追加（“The previously provided AGENTS.md instructions no longer apply.”）；
    hook 只在 `startup` / `clear` / `compact` / `resume` / `fork` 注入，不会给已打开的会话补发。这些会话需要 `/clear`、压缩、resume 或新开才恢复协议。迁移由用户显式触发（D10），所以不做自动补救，
    但迁移前后要提醒用户。
11. **`agents update` 顺带刷新 `[mcp_servers.askhuman]`（V3 发现，与本任务无关）**：既有写入逻辑补上了 `default_tools_approval_mode`；旧版 AskHuman 写下的 `approval_mode` 旧键当时没有被清理，
    Codex 0.163 对它（以及用户配置里的 `type`）报“忽略未识别配置”，exec 的 JSON 流里会多两条 error 项。现已处理：安装 / 更新写入该条目时顺手移除这个旧键（见 `codex-permission-remember.md` D49）。
