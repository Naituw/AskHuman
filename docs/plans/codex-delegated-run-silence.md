# Codex 委派运行静默与协议动态注入计划

> 状态：**已实现，V1 / V2 完成**（2026-10-25）；§15 的 5 项细节均按建议确认（spec D13）。V3（真实 `~/.codex` 迁移）与 V4（计费的真实 Agent 验证）
> 仍待用户批准；结果、偏差与待裁定项见 spec §9。
> 需求与决策真值：`docs/specs/codex-delegated-run-silence.md`（决策 D1–D13、事实 F1–F32）；本文只写实现方案。
> 范围：Codex 自动集成的**协议交付载体**（`~/.codex/AGENTS.md` 区块 → SessionStart hook）；委派运行（`codex exec` / `review` / `e`、
> app-server 系的 ephemeral 线程与官方插件线程）里的 MCP 工具、Stop 确认、PermissionRequest 的静默；集成页开关与命令行；存量迁移；诊断与文档。
> 不在范围：Claude Code / Cursor / Grok / Pi；Codex 的手动集成（用户自己把提示词贴进 AGENTS.md，AskHuman 管不到）；任何 Codex 本体改动。
> 验证基线：本机 `codex-cli 0.163.0`；桌面端内置 `0.163.0-alpha.10.2` / `0.164.0-alpha.11.1`。

## 1. 问题与已验证事实

事故、现有防线为何失效、F1–F24 的完整证据见 spec §1–§2。本计划直接依赖的事实：

| 设计点 | 依赖的事实 |
| --- | --- |
| 协议载体改为 SessionStart hook | F13：`--ignore-user-config` 不跳过 `AGENTS.md`，却清掉 MCP 与 hook 信任，静态规则会留下“必须调用却没有工具”的上下文；F16 / F17：hook 注入是 developer 条目，位于初始上下文之后、用户首条输入之前，TUI 不显示；F23：压缩**不保留**旧注入，压缩后 `SessionStart(compact)` 重新触发（源码三条路径 + 本机 17 个会话 38 次压缩实证） |
| 判定信号 | F14：hook 输入没有 `thread_source` / `originator`，但 `transcript_path` 对 ephemeral 为 `null`；F16：hook 运行前 rollout 已落盘，首行 `session_meta` 可读；F12 / F22：exec 的 rollout 为 `source=exec`、`originator=codex_exec` |
| MCP 工具隐藏 | F19：目录在连接建立后固定、空目录无副作用、缓存是进程内的；F20 / F21：MCP 里读不到来源信息（无 `CODEX_*`、无 originator） |
| 信任与静默失败 | F18：未信任 / 被改动的 hook 静默跳过，信任哈希含事件、matcher、命令、超时 |

本轮为写计划新核实的事实（均来自 `rust-v0.163.0` 源码与本机 CLI）：

- **`codex review` 与别名 `codex e` 走和 `exec` 同一条路径**（`cli/src/main.rs` 把 `review` 改写成 `ExecCommand::Review` 后调 `codex_exec::run_main`；`codex --help` 显示 `exec [aliases: e]`）。所以进程层的委派子命令集合是 `exec` / `e` / `review`，不能只写 `exec`。0.163.0 已没有 `codex mcp-server` 子命令。
- hook handler 支持 `additionalContextLimit`：未设 = 2500 token，`0` = 不限；非默认值**计入信任哈希**；`HookHandlerConfig` 没有 `deny_unknown_fields`。
- 没有 matcher 的 SessionStart hook 匹配全部 `source`（`matches_matcher(None, _) == true`）。
- AskHuman 自己经 app-server 启动的 Codex 任务是 `ephemeral:false`、`clientInfo.name="askhuman"`（`codex_desktop/runtime.rs`），不会被本方案的信号误伤。

## 2. 目标

1. 委派运行里 AskHuman 的交互面与“未集成”一致：不注入协议、不出现 MCP 工具（exec）、不弹 Stop 确认、不弹权限审批、不产生会写入的 MCP 调用。
2. 交互会话零回归；任何识别失败都按交互处理（fail-open）。
3. 集成页 / 命令行有一个默认关闭的开关，可恢复委派运行里的 AskHuman。
4. 协议交付换成 hook 后，三态模式（None / CLI / MCP）、协作风格变更、压缩恢复、resume / fork、迁移、诊断仍然成立。

## 3. 非目标与边界

- 不拦 CLI、不改会话登记、不为委派线程造假回答（D3）。
- 不为低版本 Codex 保留静态区块（D11）；不做升级后自动迁移（D10）。
- 不改 IPC：全部状态走文件与进程内判断，**不动 `ClientMsg` / `ServerMsg`，不升 `PROTOCOL_VERSION`**。
- 不触碰工作区里与本任务无关的在途改动（`codex_desktop/*`、iOS 渠道、`InboxSidebar.vue` 等）。

## 4. 方案总览

| 环节 | 进程 | 时机 | 行为 |
| --- | --- | --- | --- |
| 协议 hook | `AskHuman __context-recovery-hook codex session-start` | 每个会话的 turn 开始前（`startup` / `resume` / `clear` / `compact` / `fork`） | 判定委派 → 写 / 清标记 → 委派则无输出；否则按 mode 注入协议（首行带标记，resume / fork 去重，compact 附恢复句） |
| MCP server | `AskHuman mcp` | 随 Codex 会话启动一次；每次工具调用 | 启动时有界祖先探测 exec → 空 `tools/list`；声明 `cacheable=false`；调用时按标记拦截 |
| Stop hook | `AskHuman __stop-hook codex …` | 每次自然结束 | 委派则不弹确认（`track` 仍上报 TurnEnd） |
| PermissionRequest hook | `AskHuman __permission-hook codex` | 每次审批 | 委派则无输出，回落 Codex 原生审批 |
| 开关 | 集成页 Codex 卡片 / `AskHuman agents delegated codex [on\|off]` | 用户操作 | 偏好文件；打开后上述四处全部按交互处理 |

新增 / 改动的模块一览：

| 模块 | 变化 |
| --- | --- |
| `agents/delegated.rs`（新） | 信号、判定、标记存储、`silenced()` 统一入口、审计 reason |
| `agents/detect.rs` | 新增有界祖先探测 `nearest_agent_invocation`，复用 `codex_subcommand` |
| `agents/context_recovery.rs` | Codex 的 `session-start` 从“仅 compact 恢复句”改成“判定 + 协议注入” |
| `integrations/agent_context_recovery.rs` | Codex 的 SessionStart hook 改为无 matcher + `additionalContextLimit:0` |
| `integrations/agent_rules.rs` | Codex 分支委托给协议 hook（Grok skill 同款委托缝）；保留遗留区块的识别与删除 |
| `integrations/agent_mode.rs` | `current()` 对 Codex 改用协议 hook 作为 CLI 模式信号；`artifact_updates` 的 rule 语义；迁移入口 |
| `integrations/agent_permission.rs` | `trust_entries` / `trusted_hash` 支持 `additionalContextLimit` |
| `integrations/agent_delegated.rs`（新） | 偏好读写、状态、`allowed()`（照 `agent_stop.rs` 的偏好模式） |
| `mcp/ask.rs` | 启动期隐藏工具、`cacheable=false`、调用期委派拦截与文案 |
| `agents/stop.rs`、`permissions.rs` | 接入 `delegated::silenced`；Stop 删除 `thread_source` 死分支 |
| `prompts.rs` | Codex 协议注入文本构造（标记行、哈希、compact 恢复句） |
| `commands.rs`、`app/invoke.rs`、`cli/agents_cmd.rs`、`cli/doctor.rs`、前端集成页、i18n | 开关、状态、诊断、文案 |

## 5. Part A：判定与标记（`agents/delegated.rs`）

### 5.1 信号

| 信号 | 取得方式 | 用于 | 适用 source | 审计 reason |
| --- | --- | --- | --- | --- |
| 无 rollout（ephemeral / 非本地存储） | hook 输入的 `transcript_path` **键存在且为 `null`**（沿用 `stop.rs` 现有写法） | SessionStart / Stop / 权限 | 全部 | `codex_transcript_path_null`（沿用） |
| exec 进程 | 有界祖先探测：最近的 Agent 祖先是 Codex，且子命令位 ∈ {`exec`, `e`, `review`} | SessionStart / Stop / 权限（标记缺失时兜底）/ MCP 启动 | 全部 | `codex_delegated_exec_process` |
| rollout 首行 | `session_meta.payload.source == "exec"`；或 `originator` ∈ 名单（初始：`codex_exec`、`Claude Code`，常量可扩展） | SessionStart | **仅 `startup` / `clear`** | `codex_delegated_rollout_source` / `codex_delegated_rollout_originator` |
| 标记命中 | 按 session id 的标记文件存在 | MCP 调用 / Stop / 权限 | 全部 | `codex_delegated_marker`（带原信号名） |

规则：

- 任一信号为真即判委派；信号缺失、读取失败、冲突都按交互处理（D5）。
- **rollout 信号只在 `startup` / `clear` 使用。** 它描述线程“是怎么创建的”，不是“现在谁在跑”：用户在 TUI / 桌面端恢复一个 exec 创建的旧线程（`resume` / `fork`）属于交互，若按首行判定会误伤。`resume` / `fork` 只看当前运行信号（null transcript、进程探测）；`compact` 沿用已有标记的结论，没有标记时才按当前运行信号重算。这是对 spec D5 / D6 的细化，见 §15 待确认项 4。
- rollout 首行读取有界：只读到第一个换行，上限 256 KiB；非 JSON、缺字段、超长一律忽略该信号。
- 名单里没有 `askhuman` 与 `Codex Desktop`；回归测试要覆盖 AskHuman 自己启动的 Codex 任务不被静默。

### 5.2 有界祖先探测（`agents/detect.rs`）

- 新增 `nearest_agent_invocation(start_pid, max_hops) -> Option<AgentInvocation { kind, pid, subcommand }>`：逐跳向上，遇到第一个**非自身**且命中任一 Agent 家族的祖先即停；`max_hops = 6`。
- Unix 每跳一次 `ps -o ppid=,command= -p <pid>`（现有 `process_chain` 每跳两次 `ps` 且一直走到根，hook 里实测 253–297 ms，见 `docs/plans/hook-perf-walk-optimization.md`，不能复用到这里）。Windows 复用现有 Toolhelp 快照，找到即返回。
- 子命令解析复用 `codex_subcommand`（已处理 `-c k=v`、`--flag=value` 等前导全局选项，`codex exec "…app-server…"` 仍得 `exec`）。委派集合 `DELEGATED_CODEX_SUBCOMMANDS = ["exec", "e", "review"]`；`app-server`、`resume`、裸 `codex` 都不算。
- 只在 SessionStart（每会话一次）、Stop（每次自然结束一次）、权限请求（本来就要联系人）和 MCP 进程启动时调用，**不进入 PreToolUse 等高频 hook**。

### 5.3 标记存储

- 位置：`<config_dir>/state/codex-delegated/<id>.json`（`config_dir()` 已尊重 `ASKHUMAN_HOME`，开发实例互不串）。`<id>` 是 session id 清洗后的值：只允许 `[A-Za-z0-9_-]{1,64}`，否则用其 SHA-256 前 32 位十六进制。
- 内容：`{"v":1,"signal":"exec_process","ts":<ms>}`；**文件存在即委派，不存在即未知**。不缓存“交互”结论。
- 写入用 `hook_edit::atomic_write`；读取容错（损坏按不存在）；每次写入顺带清理 mtime 超过 14 天的条目（目录扫描上限 500 项）。
- 写入 / 清除时机：SessionStart 每次触发都按本次判定覆盖或清除（`resume` 后换了运行方式也能纠正）；Stop / 权限兜底探测得到肯定结论时写入。

### 5.4 统一入口

```rust
/// None = treat as interactive. Some(reason) = stay silent.
pub fn silenced(session_id: &str, input: &Value) -> Option<SilenceReason>
```

顺序：开关 `agent_delegated::allowed(Codex)` 为开 → `None`；标记命中 → `Some`；`transcript_path` 为 null → `Some`；有界探测为 exec → 写标记并 `Some`；否则 `None`。Stop 与权限都只调它；SessionStart 用它的子集（见 §6.2）。

## 6. Part B：协议 hook（动态注入）

### 6.1 注册形态（`integrations/agent_context_recovery.rs`）

- 复用现有 mode 托管的 hook（命令身份 `"<exe>" __context-recovery-hook codex session-start` 不变，Windows 仍用 `commandWindows`）。
- Codex 的 `EventSpec` 改成：`matcher: None`（全部 source）、`TIMEOUT_SECS` 不变、handler 增加 `"additionalContextLimit": 0`（协作风格的自定义文案可能超过默认 2500 token，超限会被截成预览，协议静默失效；`0` 是 Codex 文档明确的“不限”）。Claude 的 spec 保持 `compact` 不动。
- `status_from_text` / `reconcile_text` 的“exact”判定同时校验 matcher、`additionalContextLimit`、超时；旧身份（`matcher=compact`、无该字段）自然落入 `outdated`，这就是 D10 的“需更新”触发源。
- `hook_edit::upsert_nested_group_matched_with_windows` 增加可选 handler 附加字段参数（仅此处使用），不改其它调用方。
- Mode 为 None 时照旧卸载。

### 6.2 hook 输出逻辑（`agents/context_recovery.rs`）

```text
codex_session_start(mode, input, env):
  mode == None                      -> no output
  source  = input.source            (缺省按 startup)
  id      = input.session_id
  allowed = agent_delegated::allowed(Codex)

  if !allowed:
      verdict = classify(source, input, id)        // §5.1 的 source 规则
      if verdict.delegated:
          marker_write(id, verdict.signal); audit(protocol_hook, suppressed, reason)
          return no output
  marker_clear(id)

  text = protocol_text(mode, source)               // §6.3
  if source in {resume, fork} and already_injected(input.transcript_path, text.marker):
      return no output                              // §6.4
  return {"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext": text.body}}
```

- 任何内部错误（读配置失败、写标记失败）都不输出、退出码 0；失败写一条 `log_runtime_event`，不影响会话。
- `session_info::capture_hook` 与 `report::should_skip` 保持现有调用顺序。
- Claude 分支完全不变（仅 `compact` 恢复句）。

### 6.3 协议文本与标记行（`prompts.rs`）

- 正文 = `cli_reference_for(Codex)` 或 `mcp_reference_for(Codex)`（协作风格仍由 `collaboration_section_from_config()` 在 hook 运行时读配置，所以改风格后新会话立即生效，不再需要改写文件）。
- 首行标记：`<!-- askhuman-protocol:<mode>:<hash8> -->`；`hash8` 是不含标记行的正文 SHA-256 前 8 位。模式、风格、自定义文案、提示词版本任一变化都会改变哈希。
- `source == compact`：正文后追加现有恢复句（`compact_recovery_cli_prompt` / `compact_recovery_mcp_prompt`），因为压缩后整条协议都不在历史里（F23）。
- 全新 subagent 不触发 SessionStart（F16），仍由现有 `SubagentStart` guard 负责；fork 出的 subagent 继承父历史，协议里的 subagent 例外句保持不变。

### 6.4 resume / fork 去重

- 仅在 `resume` / `fork` 且 `transcript_path` 非空时执行；`startup` / `clear` / `compact` 一律注入（compact 后旧注入一定已不在历史里）。
- 从文件末尾**向前**分块读取（块 64 KiB），逐行：行内含当前标记 → 判定已注入、跳过注入；行内含 `"type":"compacted"` → 先检查该行本身是否含标记，然后停止、判定未注入；读到文件头都没遇到 → 未注入。
- 预算：最多回读 64 MiB 或 300 ms，超出、读失败、文件不可读一律**注入**（宁可重复，不可缺失）。标记只含 ASCII 的 `:` `-` 与字母数字，JSON 转义不会改变它。
- 效果：同模式同文案的 resume 不重复；改了模式 / 风格 / 文案后的 resume 会注入新版（旧版留在历史里但哈希不同，新版在后）。

### 6.5 mode 判定与 Rules 委托（`agent_mode.rs`、`agent_rules.rs`）

今天 Codex 没有超时 hook，`current()` 在 `(mcp=false, hook=false)` 时回退到“AGENTS.md 区块是 CLI 还是 MCP 变体”。区块移除后这条回退会把 CLI 模式误判成 None，所以必须换信号：

- `current(Codex)`：MCP 配置已装 → `Mcp`；否则协议 hook 已装（只看 `hooks.json` 里有无 marker，不做信任检查）→ `Cli`；否则才回退到遗留区块变体；都没有 → `None`。其它 Agent 的逻辑不变。遗留区块变体的读取（`classify_body` 那一套）保留为独立函数，因为 `installed_variant(Codex)` 委托给 hook 之后不再返回区块信息。
- `agent_rules` 的 Codex 分支照 `is_grok_skill` 的委托缝改为委托协议 hook：`is_installed` = 协议 hook 已装；`needs_update` = hook 缺失 / 过期 / 未信任，或遗留区块仍在；`install_variant` = 协议 hook reconcile + 迁移遗留区块（§9）；`uninstall` = 卸 hook + 删遗留区块；`display_path` / `reveal` / `open` 指向 `~/.codex/hooks.json`。调用方（`agent_mode`、`commands.rs`、CLI）不用改。
- `artifact_updates` 对 Codex：协议 hook 的缺失 / 过期并入 **rule** 标志（集成页“Rules”那一行的“更新”按钮就是迁移入口），不再同时计入 hook / mcp 标志，避免重复提示。
- `collaboration_style_apply_integrations()`（改协作风格后重写所有非 None Agent）对 Codex 必须区别对待：协议 hook 已是新身份 → 跳过（文案由 hook 运行时读配置，无需改写）；仍是遗留区块 → 只刷新区块文案，**不触发迁移**。否则用户改一次风格就会被隐式迁移，违背 D10。
- `agents mode codex <mode>` / `agents update codex` / 集成页“更新”是用户的显式动作，允许执行迁移。

### 6.6 信任哈希（`agent_permission.rs`）

- `trust_entries` 读取 handler 的 `additionalContextLimit`，`trusted_hash` 把它放进规范化身份（键名 `additionalContextLimit`）；值等于默认 2500 或缺失时不放入（与 Codex 的 `.filter(!= DEFAULT)` 一致）。
- 回归向量用真实 Codex 产生：隔离 `CODEX_HOME` 下启动 `codex app-server`，只发 `initialize` + `hooks/list`，读取 `currentHash`（不调模型、不计费），写进单测，和现有 0.144.1 的向量并列。

### 6.7 性能

SessionStart 现在每个会话都会跑，而不只是 compact。预算：进程启动约 14 ms + 读配置与 hooks.json 约 2 ms + rollout 首行约 1 ms + 探测（仅当前面信号都为假时）每跳约 5–10 ms、通常 2–3 跳。目标 macOS 上 p95 < 60 ms；实现后按 `hook-perf-walk-optimization.md` 的方法实测并把数字记进 spec F10。

## 7. Part C：委派静默

### 7.1 MCP（`mcp/ask.rs`）

1. **启动期隐藏（exec）**：`AskServer::new()` 先算 `hidden = !allowed(Codex) && nearest_agent_invocation(parent) 是 Codex 且子命令 ∈ 委派集合`。`hidden` 时用空 `ToolRouter::new()`，`tools/list` 为空（含只读的 `todo_list`，整体等同未集成，D4），并跳过 `register_instance()`（避免向 daemon 登记一个没有工具的实例）。判定只算一次、随进程结束；之后改开关只影响新启动的进程。
2. **缓存声明**：无论是否 `hidden`，`ServerCapabilities` 都加 `experimental["codex/tool-catalog-cache"] = {"cacheable": false}`（rmcp 2.2.0 的 `ServerCapabilities::builder()` 提供 `enable_experimental*`），避免长驻 app-server 里别的会话复用缓存的工具目录。
3. **调用期拦截（app-server 系）**：`codex_thread_guard` 在现有 `thread_source` 判断之后增加一步：用 `_meta` 里的 session id（`x-codex-turn-metadata.session_id`，缺失时用 `_meta.threadId`）查标记；命中且开关为关 → 返回 `isError` 结果与 `suppressed` 审计行。覆盖 `ask` / `whats_next` / `show_last` / `todo_add` / `todo_update`，`todo_list` 不走该守卫。现有 `system` / `ambient_suggestions` 守卫不动。
4. **文案**：现有常量写的是“system-generated background thread”，不适合委派语境。新增一条：`AskHuman is not used in this non-interactive Codex run. No human is attending it. Do not retry or contact the human; continue the task and return the result directly.`
5. MCP 子进程里没有 `CODEX_*`、没有 originator（F20 / F21），所以 exec 只能靠祖先探测，app-server 系只能靠 SessionStart 写下的标记——SessionStart 总是早于首次采样，因此首次工具调用时标记已就位。

### 7.2 Stop（`agents/stop.rs`）

- `confirmation_suppression_reason` 之后（或并入其中）调用 `delegated::silenced`；命中则写 `stop_confirmation · suppressed · <reason>` 审计，`track` 路径仍上报 `TurnEnd`。其余逻辑不变。
- 旧的 `transcript_path` 为 null 判断并入 `silenced`，保留原审计 reason 以延续日志。

### 7.3 PermissionRequest（`permissions::run`）

- 解析出 `session_id` 后、进入 `parse_permission` 的自动放行 / 记忆层**之前**，对 Codex 调 `silenced`；命中返回 `None`（无 stdout），Codex 走原生审批。自动放行也不例外：D1 要求“与未集成一致”。

### 7.4 审计

沿用 `askhuman_guard` 行（`daemon::lifecycle::log_guard_audit`）。新增 component：`protocol_hook`、`mcp_server`、`permission_hook`；新增 reason 见 §5.1。不记录提问正文、提示词或任何未识别的 metadata。

## 8. Part D：开关、命令与界面

- **偏好**：`paths::delegated_preferences_file()` → `<config_dir>/delegated-preferences.json`，`{"codex": bool}`，缺省 `false`（= 静默）。`integrations/agent_delegated.rs` 照 `agent_stop.rs` 实现 `allowed` / `set_enabled` / `status`（`set_enabled` 持 `IntegrationMutationLock`，写偏好即可，不触碰任何 hook）。
- **Tauri**：`agent_delegated_set(agent, enabled)`（注册到 `app/invoke.rs`）；`AgentModeStatus` 增加 `delegated { supported, enabled }`；`agent_mode_status_sync` 填充。
- **前端**：`src/lib/ipc.ts` / `types.ts` 增加调用与类型；`IntegrationTab.vue` 的 Codex 卡片（仅 `mode !== 'none'` 时）加一行开关，标题“允许被委派的 Codex 运行使用 AskHuman”，提示文案说明“关 = 委派运行与未集成一致；只影响新启动的运行”；`useIntegration.ts` 增加 toggle；`useSearch.ts` 登记搜索项；`src/i18n/zh.ts` / `en.ts` 加文案。
- **命令行**：`AskHuman agents delegated codex [on|off]`（`on` = 允许），照 `permission_cmd` / `stop_cmd` 写；`agents show codex` 与 `doctor` 增加两行：`协议交付：SessionStart hook · 已信任 / 未信任 / 需更新 / 该版本不支持`，`委派运行：静默（默认）/ 允许`。Rules 那一行对 Codex 改名为“协议（SessionStart Hook）”，路径显示 `~/.codex/hooks.json`。`agents show codex` 仍保留“手动集成”所需的提示词全文与粘贴位置说明。
- 后端 / CLI 用户可见文案走 `src-tauri/src/i18n.rs`，规则见 `docs/specs/i18n.md`。

## 9. Part E：迁移、版本与回滚

### 9.1 原子迁移（D10）

触发：集成页 Rules 行的“更新”、`agents update codex`、`agents mode codex <mode>`。全程持 `IntegrationMutationLock`：

1. 快照 `~/.codex/hooks.json`、`~/.codex/config.toml`、`~/.codex/AGENTS.md` 的原始字节。
2. 安装 / 升级协议 hook 并写入信任（`agent_context_recovery::reconcile_unlocked`，已有失败回滚）。
3. 重新读取并确认 `codex_marker_trusted` 为真。
4. 删除 `AGENTS.md` 里的 AskHuman 区块（`remove_block`，原子写）。
5. 任一步失败：按相反顺序把三个文件恢复到快照，返回错误；**任何时刻都不出现“hook 与区块都没有”**。步骤 2–3 与 4 之间的瞬间“两边都有”只会造成协议重复一次，不影响正确性。

`Mode::None` 的卸载顺序相反（先删区块与 hook，再删标记目录，尽力而为）。

### 9.2 版本与提示（D11）

- 版本地板：沿用现有 hook 功能的研究基线 `0.144.1`（代码里目前没有 Codex hook 的版本门槛，见 §15 待确认项 1）。读取方式复用 `agent_task_readiness` 的 `version` / `binaryReady`，不新增第二套探测；读不到版本（Codex 未安装）按支持处理。
- 低于地板：集成页 Rules 行显示“该版本不支持协议注入”，不提供迁移、**不写静态区块**；已有遗留区块原样保留不动。

### 9.3 遗留状态的矩阵

| 状态 | 判定 | UI / 行为 |
| --- | --- | --- |
| 遗留：区块在、hook 是旧身份 | 区块存在 + `status.outdated` | “需更新”；旧行为继续生效；MCP 的 exec 隐藏不依赖迁移，立即生效；Stop / 权限靠兜底探测，同样不依赖迁移 |
| 新：协议 hook 已装且已信任，无区块 | 正常 | “已安装” |
| 中间态：hook 新、区块还在 | 迁移步骤 3 与 4 之间或第 4 步失败后 | 回滚；若人工造成则显示“需更新” |
| hook 未信任 / 被改动 / `features.hooks=false` | `codex_marker_trusted` 为假 | “未信任 / 需更新”，doctor 警告“协议未交付”（F18 静默失败的显式化） |

## 10. Part F：Stop `thread_source` 死分支清理（D12）

- 删除 `agents/stop.rs` 的 `codex_thread_source`、`confirmation_suppression_reason` 里的 `codex_blocked_thread_source` 分支，以及 `GuardAudit.thread_source: codex_thread_source(&input)`（改为 `None`）。
- 在函数注释写明：Codex 的 Stop 输入没有 `thread_source`（`StopCommandInput` 在 0.144.1–0.163.0 字段一致），Stop 静默只依赖 `transcript_path == null` 与委派判定。
- 重写 `codex_system_and_ephemeral_stops_suppress_confirmation` 等用例，去掉 `thread_source` 输入；`CODEX_BLOCKED_THREAD_SOURCES` 仍被 MCP 守卫使用，保留。

## 11. 兼容性与风险

| 风险 | 说明 | 缓解 |
| --- | --- | --- |
| 失败从显式变静默 | hook 未信任 / 哈希漂移 / `features.hooks=false` / `allow_managed_hooks_only` 会让交互会话也失去协议 | 信任同步沿用；集成页与 doctor 显示“协议交付”状态；hook 执行失败在 Codex UI 里可见，只有“未信任”是静默的 |
| 误判交互线程为委派 | 无 rollout 的线程不一定都是后台（远程控制 / 非本地存储的会话也会 `transcript_path=null`）；originator 名单是启发式 | 开关可整体放行；审计行含信号名；fail-open；rollout 信号只用于 `startup` / `clear` |
| 漏判委派 | app-server 系的持久化线程且 originator 不在名单；SessionStart hook 未生效时 app-server 系 MCP 无标记 | 名单可扩展；上游诉求已记项目待办 #6；文档写明为尽力识别 |
| 模型对 developer 角色协议的遵从度未经真实验证 | developer 通常优先级更高，但没有实测 | 实现后做经你批准的小规模真实验证（§13 V4）；不通过则回到本计划重新决策 |
| 协议 hook 增加会话启动开销 | 现在每会话都跑 | §6.7 预算与实测 |
| resume / fork 重复注入 | 历史重放带着旧 developer 条目 | §6.4 标记去重；读失败倾向注入 |
| 压缩后协议缺失 | 压缩不保留旧注入 | `compact` 总是重新注入并附恢复句（F23 实证 38/38） |
| 隐式迁移 | 改协作风格等路径会调用 `agent_mode::update` | §6.5 对 Codex 区别对待 |
| 旧版 Codex 对新字段的反应 | `additionalContextLimit` 在 0.163.0 无 `deny_unknown_fields`，更老版本未核实 | 版本地板之下不迁移；真实环境只在 0.163.0 验证 |
| Windows / Linux | 进程命令行与路径差异 | 探测在 Windows 复用快照；标记路径用 `PathBuf`；补各平台用例 |
| 开发实例 | 标记目录与偏好文件都在 `config_dir()` | 已随 `ASKHUMAN_HOME` 隔离；hook / MCP 进程的环境与现有偏好读取方式一致 |

## 12. 文档同步

实现完成后同步（只改不准确的地方）：

- `docs/specs/codex-delegated-run-silence.md`：确认 §15 细化项后更新 D5 / D6；状态改为已实现；F10 补实测数字。
- `docs/specs/mcp.md`：exec 空 `tools/list`、`cacheable=false`、调用期委派拦截与新文案、`todo_list` 的例外。
- `docs/specs/agent-stop-confirmation.md`：删除 `thread_source` 描述，新增委派静默与审计 reason。
- `docs/specs/agent-rules-config.md`：D7 / D10 的 Codex 落点与“已安装”判定改为协议 hook，遗留区块的识别与迁移。
- `docs/specs/main-agent-only-interaction-protocol.md`：Codex 的交付载体与 subagent 例外的传递方式。
- `docs/specs/cli-config.md`：`agents delegated`。
- `docs/plans/agent-context-compaction-retention.md`：更正“压缩可能保留注入”的概括（F23）。
- `docs/overview.md`：只改“上下文恢复 Hook”与“Agent 自动集成产物”两处的 Codex 描述，并补一句 exec 隐藏 MCP 工具；其余不动。
- `docs/PROGRESS.md`：完成后删除本任务那一节。

## 13. 验证顺序

**V1 单元与夹具测试（无网络、临时目录）**

- `detect`：命令行解析表（`codex -c k=v exec`、`codex e`、`codex review`、`codex exec resume`、`codex app-server`、`codex resume`、裸 `codex`、`node …/codex.js exec`、提示词里含 `app-server` / `exec` 的 TUI）；`nearest_agent_invocation` 在真实进程链上的用例（测试专用桩可执行文件，具体构造实现时定）。
- `delegated`：标记读写 / 清理 / 损坏容忍 / session id 清洗；`silenced` 各分支与开关；rollout 首行边界（超长、非 JSON、缺字段）；source 矩阵（startup / clear 用 rollout 信号，resume / fork 不用，compact 沿用标记）；`originator=askhuman` 与 `Codex Desktop` 不被静默。
- 协议 hook：mode × source × 委派 × 开关的输出矩阵；标记行随 mode / 风格 / 自定义文案变化；compact 附恢复句；去重（最后一次 `compacted` 之后 / 之前、无 `compacted`、读失败、超预算）。
- `agent_context_recovery`：Codex 新身份 exact；旧 `compact` matcher、缺 `additionalContextLimit` → outdated；Windows 变体。
- 信任哈希：`additionalContextLimit` 的向量（来自真实 Codex `hooks/list`）。
- `agent_mode`：Codex 的 `current()` 映射（Mcp / Cli / None / 遗留区块）、`artifact_updates` 的 rule 语义；迁移与回滚的故障注入（写 hooks 失败 / 写 config 失败 / 删区块失败 → 三个文件与迁移前逐字节一致）；`collaboration_style_apply_integrations` 不触发迁移。
- MCP：`hidden` 时 `tools/list` 为空且不登记实例；`cacheable=false` 声明；调用期拦截（标记命中、开关开、非 Codex 客户端、`todo_list` 不受影响）；既有 `thread_source` 守卫不回归。
- Stop / 权限：静默与审计 reason；`track` 仍上报；权限自动放行也被静默。
- 死分支清理后的 Stop 用例。

**V2 无模型集成验证（隔离 `CODEX_HOME`，不计费）**

- `codex app-server` 的 `hooks/list`：核对 `currentHash` 与 AskHuman `trusted_hash` 一致。
- mock Responses 端点夹具：用自定义 `model_providers` 指向本地端点，端点返回一条固定助手消息；隔离环境要同时设置 `HOME` 与 `CODEX_HOME`（AskHuman 的 `paths::codex_dir()` 取 `$HOME/.codex`，不读 `CODEX_HOME`，只隔离后者会让 hook 读到真实配置），夹具里放 hooks.json、信任、AGENTS.md 有 / 无区块、可选 MCP 配置，并用 `ASKHUMAN_HOME` 隔离标记与偏好。场景：`codex exec`（开关关）请求体无协议且无 askhuman 工具、Stop / 权限不触达 daemon；开关开时请求体含协议 developer 条目（位于初始上下文之后、用户输入之前）与 askhuman 工具；`exec resume`、`--ephemeral`、`codex review`；`--ignore-user-config` 下请求体无任何 AskHuman 文本；遗留区块迁移前后的请求体对比。压缩路径用 rollout 夹具验证，不构造真实压缩请求。
- 夹具可行性（本机版本是否允许无登录态使用自定义 provider）在实现第一步先验证；不可行则这一层降级为 V1 + V3，并如实记录。

> **V2 实际结果（2026-10-25）**：夹具可行（真实 Codex 0.163.0 + 本地 mock Responses 端点，`HOME` / `CODEX_HOME` / `ASKHUMAN_HOME` 隔离，夹具路径须是规范路径 `/private/tmp/…`）。
> 计划里的场景全部完成，并且**比计划多做**了两项：压缩路径不止用 rollout 夹具，而是在真实二进制上用 mock 摘要端点触发了 `thread/compact/start`
> （旧协议被丢弃、`SessionStart(compact)` 重新注入、请求里恰好一条协议）；app-server 驱动的 ephemeral 与持久化 `Claude Code` originator 线程也跑了调用期守卫。
> 夹具要点：MCP 子进程环境是白名单，隔离环境要在 `[mcp_servers.askhuman].env` 里带 `ASKHUMAN_HOME`；做 A/B 前要等 MCP 就绪（首请求竞态）；`codex review` 要在 git 仓库目录内运行；
> 夹具 daemon 会在用户域注册 launchd 任务，收尾必须 `daemon stop` + `launchctl bootout`。细节见 spec F25–F32 与 §9。

**V3 安装验证（仓库规则）**：`./scripts/install.sh` 后，用新装的 `AskHuman` 验证开关、`agents delegated`、`agents show codex`、`doctor`、迁移按钮。对**真实 `~/.codex`** 做迁移前先问你，因为它会删除你 AGENTS.md 里的区块、改写 hooks.json 与信任。

> **V3 现状**：新二进制已装（`~/.local/bin/AskHuman`），只读检查确认它对真实 `~/.codex` 显示「需更新」（旧恢复 hook + AGENTS.md 区块），未改动任何真实文件；迁移、开关写偏好、迁移按钮都等你批准后再做。

**V4 计费的真实 Agent 验证（需你明确批准后才做）**：① 交互的 TUI / 桌面端新会话收到协议并按规则使用 AskHuman（developer 角色遵从度）；② 真实 `codex exec`（开关关）没有 AskHuman 工具、没有 Stop 弹窗；③ 开关开时行为等同交互；④ 实际触发一次压缩后协议仍在。

## 14. 完成条件

- D1–D12 的行为全部落地，V1 全绿，V2 按可行性完成并如实记录，V3 完成；V4 经你批准后完成或明确记为待办。
- 存量迁移原子且可回滚；改协作风格不会隐式迁移；`doctor` 与集成页能显示协议交付状态。
- 文档按 §12 同步，`docs/PROGRESS.md` 本任务一节删除。
- 提交遵循 Conventional Commits（拟 `feat(codex): …` 分若干提交：判定与标记 / 协议 hook 与迁移 / MCP 与 Stop 与权限静默 / 开关与界面 / 文档）。

## 15. 已确认的细节（spec D13）

以下几项是写计划时发现、spec 里没有明确决定的实现细节；用户 2026-10-25 全部按建议确认，已写回 spec D13。

1. **版本地板**：建议取 `0.144.1`（现有 Stop / 权限 / 生命周期 hook 的验证基线，代码里目前没有 Codex hook 的版本门槛）。
2. **命令与偏好名**：建议 `AskHuman agents delegated codex [on|off]`（`on` = 允许委派运行使用 AskHuman），偏好文件 `delegated-preferences.json`，与 `permission` / `stop` 并列。
3. **`additionalContextLimit`**：建议写 `0`（不限），避免长自定义协作风格被截成预览；代价是 hook 身份多一个字段，需扩展信任哈希（§6.6）。
4. **rollout 信号只用于 `startup` / `clear`**：这是对 D5 / D6 的细化，避免把你在 TUI / 桌面端恢复的 exec 旧线程误判为委派；代价是官方插件经 app-server `resume` 自己的持久化线程时识别不到（只能靠 ephemeral 信号）。
5. **Stop / 权限在标记缺失时做一次有界探测兜底**（命中则写标记）：让 exec 的静默不依赖 SessionStart hook 是否已迁移 / 已信任；这两条路径低频，且本来就要联系人，单次探测成本可忽略。
