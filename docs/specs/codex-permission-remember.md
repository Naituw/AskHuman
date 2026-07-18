# Codex 权限弹窗“本会话 / 始终允许”——调研与产品技术规格

> 状态：设计中；本文只固化已确认结论与当前源码事实，未定项不会作为实现决定。
>
> 初始调研基线（2026-07-18）：HumanInLoop 当前工作区；相邻目录
> `/Users/wutian/Developer/Codex` 的 `main` 分支，commit `d2d00b6632`。
> Shell 与 App Server 相关结论已在同日按最新 commit
> `6bd3f5e3db8275c10c7e4bbcc1342c32a89b7eee` 复核。
> 调研同时包含源码静态阅读与 Codex CLI 0.144.4 隔离 TUI 实测；尚未修改产品功能。

## 1. 背景与目标

HumanInLoop 当前通过 Codex `PermissionRequest` Hook 接管原本即将出现的权限确认，并在本地 Popup 与
IM 中提供“批准一次 / 拒绝”。Codex TUI 对部分审批还会提供“本会话允许”或“始终允许”，因此同类请求经
AskHuman 处理时可能反复弹窗。

本需求的目标是：在**不要求修改或替换用户标准 Codex** 的前提下，尽可能让 AskHuman 权限确认具备 Codex
原生审批的记忆能力，并覆盖 Codex 原生支持的审批类型：

- 单次批准；
- 本会话允许；
- 永久允许；
- 拒绝 / 取消。

最终选项必须以 Codex 对当前请求真实支持的能力为边界；不能因为 UI 可以渲染任意 choice，就为当前请求
伪造一个 Codex 本身不支持或作用域不同的“始终允许”。

## 2. 已确认的产品决策

| 编号 | 决策 |
|---|---|
| D1 | 覆盖 Codex 原生所有审批类型，不只处理 shell 或文件写入 |
| D2 | 同时支持 Codex 原生的 session scope 与 permanent scope；另为可验证的原生文件编辑增加经明确确认的项目级 / 磁盘级 session scope，不泛化到其它工具 |
| D3 | 必须兼容用户当前安装的标准 Codex；不以维护 Codex fork、私有补丁或定制二进制为前提 |
| D4 | AskHuman 的 session scope 定义为**整个对话树**，根线程与所有协作子代理双向共享 |
| D5 | session scope 在 Resume 后继续有效；以 Codex Hook 提供的共享 `session_id` 分区，不以 `agent_id` 分区 |
| D6 | fork 后的新对话不继承原对话的 session scope |
| D7 | 永久规则应写入 Codex 自己使用的配置 / rules 文件，使未来未启用 AskHuman Hook 的 Codex 也能识别；具体写法必须遵循 Codex 源码语义 |
| D8 | 当前先形成规格与差距分析；尚未确认的匹配键、选项文案和降级策略继续逐项讨论，不在本文中擅自定案 |
| D9 | 原生文件编辑至少提供“本对话不再询问这些文件”，按本次请求涉及的精确文件路径逐个记录 |
| D10 | 若本次请求的所有文件都在项目内，额外提供“本对话允许所有本项目内的文件修改” |
| D11 | 若本次请求任一文件在项目外（包括内外混合请求），额外提供“本对话允许完全磁盘文件修改” |
| D12 | “本项目”沿用现有项目定义：从 Hook `cwd` 找最近 Git 根；非 Git 目录回退 `cwd` |
| D13 | 文件授权的 UI 文案不暴露 `apply_patch`；底层只匹配可验证的 Agent 原生结构化文件编辑，不覆盖 shell / MCP |
| D14 | `SingleSelectSubmit` 在所有渠道统一为“先选择、再提交”；Telegram 不再把选择按钮直接当终态 |
| D15 | session rule 在最后一次实际命中并自动允许后滚动保留 30 天；30 天无命中则清理，普通会话活动不续期 |
| D16 | 在设置 → 高级的最后提供授权管理入口；设置启动与进入高级 Tab 均不加载规则，只有用户打开管理面板时才按需读取 |
| D17 | 管理面板按 Codex 对话分组，只提供查看与“重置整个对话授权”，不提供逐条 / 逐路径撤销 |
| D18 | 普通 Shell 严格沿用 Codex 默认菜单范围：允许一次、有条件出现的永久 command-prefix、拒绝；不增加 AskHuman 自定义的 Shell session 选项 |
| D19 | `apply_patch` 的精确文件 / 项目 / 磁盘 session scope 完全由 AskHuman shadow rules 记录与匹配；不依赖或写入 Codex 当前 thread 的原生审批缓存 |
| D20 | 当前权限记忆需求排除 App Server 中转；AskHuman 不为此成为 Codex TUI 与执行引擎之间的全量通信代理 |
| D21 | 排除在 Hook 内调用 App Server 查询 Shell prefix；当前协议没有只读 exec-policy 评估 RPC，且原生审批事件要等 Hook 未作决定后才会产生 |
| D22 | Shell 下一步先评估按当前 Codex 版本完整复刻权限判断逻辑；在完成依赖、输入和版本同步分析前，不把复刻方式写成既定实现 |
| D23 | 当前 Codex 正常工具路径会先把 `TurnContextItem` 与对应 FunctionCall 持久化并 flush 到 rollout，再执行 PermissionRequest Hook；AskHuman 可以读取该来源，但 I/O 缺失、旧 schema 或无法唯一关联时不得猜测 |
| D24 | 权限记忆是现有“允许一次 / 拒绝”确认之上的可失败增强：记忆分析、规则读取或兼容性判断失败时仍保留现有 AskHuman 弹窗；只有基础确认链本身失败时才以空 stdout 交还其它 Hook / Codex 原生审批 |
| D25 | 用户选择记住但规则保存失败时，本次调用降级为“允许一次”，并在原作答渠道明确报告未保存成功；不得误报已记住，也不要求用户再确认同一次调用 |
| D26 | 记忆选择的保存与校验由 AskHuman daemon 统一负责；短命 PermissionRequest Hook 只负责把请求送入 daemon、等待最终裁决并向 Codex 输出结果。daemon 在保存完成前不得把选择定格为“已记住” |
| D27 | Shell 的复杂解析与 exec-policy 判断在有输入输出上限和硬超时的隔离 worker 中运行，不进入 `panic = "abort"` 的 daemon 进程；worker 只能返回经校验的记忆候选，不能直接产生 Hook allow / deny。worker 任意失败均回到现有弹窗 |
| D28 | Shell permanent 不另建 AskHuman 持久 shadow；Codex 原生 `default.rules` 是唯一授权来源。首版不做 Shell policy 缓存，隔离判断进程每次读取并解析当前原生规则；当前 Codex 再次调用 Hook 时，仅在能够证明这是普通 Shell 请求且最新原生 policy 对该请求为 allow 时自动返回 allow |
| D29 | Shell 的原生规则命中不能单独触发 Hook `allow`：AskHuman 还必须证明本次是普通首次 Shell 审批，并已覆盖当前 Codex 的完整 effective exec policy；strict auto-review、retry / escalation、managed policy 或其它上下文无法排除时保留现有弹窗。该即时判断是对原生规则的无状态重算，不是第二份授权 shadow |
| D30 | 当前 Codex 未 reload 时的即时生效使用 daemon 内存中的临时 policy provenance：按 thread / rollout、Codex 进程代次与 policy context 分区，只证明“已验证基线 + 本次原生追加”的关系，不缓存 policy 或 Allow 结果、不落盘也不直接授权。daemon 重启、摘要不匹配或上下文变化后 fail closed，恢复现有弹窗；不向 `default.rules` 写 AskHuman 私有证明注释 |

### 2.1 “本会话”的最终作用域

这里有意采用比 Codex 当前内存缓存更符合产品预期的语义：

```text
AskHuman session approval scope
  = Codex session_id
  = 根线程 + Resume + 该根线程产生的所有协作子代理
```

Codex 已保证：

- `Session::session_id()` 来自整棵 agent tree 共享的 `AgentControl.session_id`；
- `PermissionRequest` Hook 的 `session_id` 使用上述共享值；
- 协作子代理另有自己的 `agent_id` / thread id，但不改变共享 `session_id`；
- Resume 会从 rollout 的 `SessionMeta.session_id` 恢复原值；
- 新建、clear 或 fork 的新对话生成新的根 thread/session identity。

因此 AskHuman 后续的 session rule 不应包含 `agent_id` 这一隔离维度。具体规则仍必须按文件、命令、host、
MCP 工具等精确资源键匹配；共享 session 并不等于把整个对话切换为无条件放行。

## 3. 当前 HumanInLoop 行为

当前实现位于 `src-tauri/src/permissions.rs`：

- Codex 请求只生成 `approve_once` 与 `deny`；
- Claude Code 若在本次请求中携带可识别的 `permission_suggestions`，才会额外显示其原生 rule suggestion；
- `approve_once` 对 Codex 输出 Hook `allow`；
- `deny` 输出 Hook `deny`；
- Codex 分支不会输出或伪造 `updatedPermissions`。

Popup 与 IM 的通用 Confirm 模型已经支持动态结构化选项，所以主要缺口不在展示控件，而在：

1. 如何判断当前 Codex 原生会显示哪些 scope；
2. 如何构造与 Codex 一致的匹配键；
3. 如何在本进程立即生效并在未来 Codex 会话永久生效。

## 4. Codex 原生审批能力矩阵

下表描述当前调研版本的 TUI / core 能力，不代表 AskHuman 已经能够从 Hook 输入完整重建这些选项。

| 审批类型 | 原生单次 | 原生 session | 原生 permanent | 主要作用域 / 存储 |
|---|---:|---:|---:|---|
| 普通 shell / unified exec | 有 | 协议支持，但普通默认菜单不一定提供 | 有条件提供 command prefix amendment | permanent 写 `~/.codex/rules/default.rules` |
| `apply_patch` | 有 | 有，“不再询问这些文件” | 无独立永久文件规则选项 | 内存键为 `environment_id + path` |
| 网络访问 | 有 | 有，按 host | 有，按 host 写网络 policy rule | session host cache + `default.rules` |
| MCP tool | 有 | 有 | 符合条件时有“始终允许” | session approval store + 用户/项目/plugin/app MCP 配置 |
| 动态 `request_permissions` | 按 turn grant | 有 session grant | 无 | 当前不经过 `PermissionRequest` Hook |
| execve / additional permissions 等特殊审批 | 通常有 | 依请求提供的 decisions 而定 | 依请求能力而定 | Hook 未收到完整 candidate metadata |

### 4.1 原生 session cache 并非全树共享

Codex 的 `SessionServices` 为每个 thread 初始化独立的
`tool_approvals: Mutex<ApprovalStore>`。`apply_patch`、shell / unified exec 与部分 MCP session 决策从当前
thread 的这份 store 查询。

普通 `spawn_agent` 会创建新的 Codex `Session`，只共享 `AgentControl`、环境与继承的 exec policy，不共享
`tool_approvals`。所以在当前原生 Codex 中：

```text
主线程对文件 A 选择“本会话不再询问”
  -> 只写主线程 ApprovalStore
子代理第一次修改同一个文件 A
  -> 子代理 ApprovalStore 为空
  -> 若该操作仍需要审批，会再次询问
```

`codex_delegate.rs` 中存在“由父 session 处理审批”的路径，但它服务于 guardian/review 等内部
sub-Codex，不是普通协作 `spawn_agent`，不能据此推断普通子代理共享批准缓存。

AskHuman 已确认采用 D4–D5 的对话树级语义，主动消除这个原生重复询问点。

### 4.2 AskHuman 文件编辑 session scopes

Codex 原生 `apply_patch` 没有永久允许：TUI 只有批准一次、对本次涉及文件执行 session allow、拒绝。
原生 session key 是 `environment_id + PathUri`，保存在当前 thread 的内存 `ApprovalStore`，不写 rules 或
config。一次 patch 涉及多个文件时，各文件分别入库；未来请求只有在所有涉及路径都已批准时才跳过询问。
Move 的源路径与目标路径都会入库。

AskHuman 保留这个精确文件语义，同时增加两个用户明确要求的聚合 session scope。选项由当前请求的完整路径集
动态决定：

```text
原生结构化文件编辑请求
  -> 所有旧/新路径均可可靠解析？
       -> 否：只显示批准一次 / 精确文件允许（若能安全表达）/ 拒绝
       -> 是：以 Hook cwd 检测最近 Git 根；无 Git 时用 cwd
            -> 所有路径都在 project root 内
                 -> 显示“本对话允许所有本项目内的文件修改”
            -> 任一路径在 project root 外（含内外混合）
                 -> 显示“本对话允许完全磁盘文件修改”
```

三个 session scope 的匹配关系为：

| Scope | 自动允许条件 | 不包含 |
|---|---|---|
| 精确文件 | 未来原生文件编辑请求的全部旧/新路径都在已批准路径集合中 | 未记录的新路径、shell、MCP |
| 本项目 | 未来原生文件编辑请求的全部旧/新路径都位于同一已批准 project root 内 | 项目外路径、其它项目、shell、MCP |
| 完全磁盘 | 同一 `session_id` 下未来所有可验证的原生结构化文件编辑 | shell、MCP、无法识别的编辑载荷 |

UI 使用“文件修改”而不是底层工具名。该文案成立的前提是 adapter 已把请求验证为 Agent 原生结构化文件编辑；
当前 Codex 实际对应规范 `apply_patch`。任意 shell 命令可能同时写文件、执行程序、联网或产生其它副作用，
Hook 无法只批准其中的写入部分，因此不得命中上述文件规则。

路径分类必须纳入新增/删除/移动的全部旧、新路径，解析相对路径时以 Hook `cwd` 为基准，并防止 `..`、
symlink 或不存在目标的父目录绕过 project root 边界。任何不能可靠分类的载荷均不得显示或命中聚合 scope。

### 4.3 选择与提交

文件授权沿用现有结构化 Confirm 的无默认选择表单，顺序为：

1. 允许一次；
2. 本对话不再询问这些文件；
3. 按当前完整路径集动态显示“本对话允许所有本项目内的文件修改”或“本对话允许完全磁盘文件修改”；
4. 拒绝。

本地 Popup、飞书、钉钉与 Slack 已经区分 selection draft 和 submit：选择只改变表单状态，提交才参与跨渠道
首答胜出。Telegram 当前把 `pc:do:<index>` 直接送入 coordinator 终态，是唯一例外；本需求将其统一为：

- 点击选项只更新卡片上的选中标记和 daemon 内 draft；
- 有选择后显示 / 启用“提交决定”；
- 只有提交 callback 才调用 `submit_wire`；
- 拒绝原因仍通过精确回复本卡形成 draft，随后显式提交；
- 其它渠道先完成时，Telegram 的未提交 draft 只定格，不影响首答。

这一修正适用于语义为 `SingleSelectSubmit` 的全部结构化 choice 表单，不为 permission 单独制造 Telegram
特例。完全磁盘选项不再增加第三次确认；“选择 + 提交”本身就是一致的显式确认流程。所有权限选项仍不预选、
不标推荐，完全磁盘项需要危险样式与明确的 Resume / 子代理作用域说明。

## 5. 标准 Codex Hook 的硬边界

### 5.1 Hook 输入没有原生候选项

当前 `PermissionRequest` command input 只有：

- `session_id`、`turn_id`、可选 `agent_id` / `agent_type`；
- `transcript_path`、`cwd`、`model`、折叠后的 `permission_mode`；
- `tool_name`、`tool_input`。

它没有向外暴露 core / TUI 已经算好的：

- `available_decisions`；
- `proposed_execpolicy_amendment`；
- `environment_id`；
- canonicalized command；
- `sandbox_permissions` / `additional_permissions`；
- network `host` / `protocol` / approval context；
- patch `grant_root`；
- approval attempt / retry reason。

Codex 还会把 `OnRequest`、`UnlessTrusted` 与 `Granular` 都折叠成 Hook
`permission_mode = "default"`，Hook 不能用该字段还原更细的原始 approval policy。

### 5.2 Hook 输出只有 allow / deny

当前 command Hook output 只接受：

- `behavior = "allow"`；
- `behavior = "deny"`。

若输出 `updatedPermissions`、`updatedInput` 或 `interrupt: true`，Codex 会把它视为不支持的输出并失败关闭。
Hook 的 `allow` 在 core 中只映射为 `ReviewDecision::Approved`，不能映射为：

- `ApprovedForSession`；
- `ApprovedExecpolicyAmendment`；
- `NetworkPolicyAmendment`；
- MCP `AcceptForSession` / `AcceptAndRemember`。

因此 AskHuman 若要兼容标准 Codex，只能：

1. 对当前请求返回普通 `allow`；
2. 在 AskHuman 自己的规则层记住 session / permanent 决定；
3. permanent 决定另行以 Codex 原生格式写入其配置文件。

### 5.3 Hook 与原生 cache 的调用顺序

对 shell、unified exec、`apply_patch` 等路径，`PermissionRequest` Hook 在 orchestrator 判断
`NeedsApproval` 后先运行，而原生 `with_cached_approval` 查询位于随后执行的 runtime approval 阶段。
因此即便 Codex 当前 thread 的原生 session cache 已经批准过，Hook 仍可能先收到请求。

AskHuman 从会话开始即接管审批时，自己的 shadow rule 可以避免重复展示；但这进一步说明不能把 Codex 私有
`ApprovalStore` 当成 Hook 的可查询真相。

### 5.4 不是所有原生审批都经过该 Hook

Codex 动态 `request_permissions` 有 turn / session grant 菜单，但当前没有进入
`PermissionRequest` Hook。这一类无法仅靠现有 Hook 接管，必须在最终覆盖矩阵中明确标为“标准 Codex
接口不可达”，不能假装已支持。

### 5.5 App Server 不是当前 Hook 的判定服务

App Server 的 command approval server request 确实包含 Hook 缺失的
`availableDecisions` 与 `proposedExecpolicyAmendment`，但它们是 Codex core 处理真实工具调用后主动发给正式客户端的
审批事件，不是客户端可调用的只读判定 API。当前协议没有
`execPolicy/evaluate(command, thread_id)` 一类方法；`config/read` 只读取配置，`command/exec` 与
`thread/shellCommand` 会发起真实执行，不能用作 dry-run 权限计算。

调用顺序同样阻止 Hook 把当前 App Server 当作旁路 oracle：Codex 先等待 `PermissionRequest` Hook；只有 Hook
未返回裁决时，core 才继续生成原生审批事件。Hook 等待该事件会形成互等；先退出 Hook 又会失去通过当前 Hook
返回决定的机会。另启 App Server 则缺少当前 thread 的完整运行状态，还可能真实执行、再次触发 Hook 或得到不同
配置上下文的结果。

让 AskHuman 位于 TUI 与 App Server 之间虽可直接截获完整审批，但这要求转发全部 Codex 协议流量、接管连接与
daemon 生命周期，并扩大故障和数据边界，已超出当前提问工具的产品范围。因此 D20–D21 将两种 App Server
路线排除出本需求；若 Codex 未来向 Hook 暴露最终候选或新增只读评估 RPC，可重新评估。

### 5.6 Rollout 时序与非回归降级

当前 Codex 在 `handle_output_item_done` 识别到工具调用后，先等待
`record_completed_response_item`。该调用经 `record_conversation_items` 把当前 `turn_id` 补到
FunctionCall，并通过 `LiveThread::append_items`、local thread store 的 `durable_write` 与
`RolloutRecorder::flush` 完成 JSONL 写入屏障；之后才构造并调度 tool future。PermissionRequest Hook 位于
tool runtime 的 approval 阶段，因此正常路径中 Hook 开始前，对应 FunctionCall 已可从 `transcript_path` 读取。
当前 turn 的 `TurnContextItem` 还会在首次 model sampling 前持久化。

这项时序保证不等于请求身份总能唯一恢复：Hook 没有 `call_id`，同一 turn 的相同 command 并发调用仍可能歧义；
旧 Codex schema 也可能没有 FunctionCall 的 `turn_id` metadata。Rollout 写入持续失败时，Hook 取 path 前的再次
materialize 也可能无法补齐。因此任何缺失、截断、schema 不支持或多候选情况都只会使记忆增强不可用，不得产生
自动允许。

权限处理分成两个故障域：

1. 基础确认适配器继续只依赖现有 Hook 字段，负责始终可用的“允许一次 / 拒绝”；
2. rollout 关联、Codex 版本适配、原生判断复刻、shadow rule 与永久落盘只负责增加记忆选项或自动命中。

第二层任一步失败时回到第一层，而不是取消 AskHuman 确认。只有基础解析、daemon/IPC、Popup 与 IM 投递等现有
确认链也失败时，`__permission-hook` 才保持当前行为：exit 0 且不输出 stdout，使 AskHuman 不作裁决，由其它匹配
Hook 继续处理；若没有其它裁决，再进入 Codex 原生审批。内部异常不得合成为 `allow` 或 `deny`。

需要落盘的记忆选择不能在用户点击时先显示成功。最终实现必须等规则提交完成后再定格所有渠道：提交成功显示
“已允许并记住”；提交失败按 D25 返回当前调用的普通 `allow`，并显示“本次已允许，但未能保存授权”。具体两阶段
提交由 daemon 持有：某一渠道提交记忆选择后，daemon 先锁定本次请求，执行规则保存与校验，再同时定格所有渠道
并把最终裁决交还仍在等待的短命 Hook。

Shell 记忆能力的分析运行在短命隔离 worker 中，复用当前 permission diff worker 已采用的子进程边界、限长
stdin/stdout、`kill_on_drop` 与硬超时模式。daemon 必须先保有基础“允许一次 / 拒绝”确认能力，再把只读输入交给
worker；worker 崩溃、超时、输出非法、规则读取失败或版本不兼容时只丢弃新增候选，不影响基础弹窗。worker 输出
还需由 daemon 按当前请求绑定信息复核，且没有直接返回 Hook allow / deny 的权限。具体超时预算、部分写入恢复与
诊断 reason code 仍在继续分析，尚未作为实现细节定案。

Shell permanent 的提交结果以 Codex 原生规则写入为准，不再写 AskHuman 持久 shadow。当前尚未 reload 的 Codex
再次进入 Hook 时，AskHuman 由短命隔离判断进程重新读取并解析最新 Codex 文件。
但 `default.rules` 不是完整 effective policy：Codex 会合并所有启用 config layer 的 `.rules`，再叠加
`requirements.toml`、MDM 或 cloud managed exec-policy。AskHuman 只有在能证明这些来源均已纳入判断时，才可把原生
规则的 Allow 作为自动允许依据；缺失或不可读取的 managed overlay 必须视为未知，而不是视为空。

同一个 Bash `PermissionRequest` 还可能来自 strict auto-review、sandbox / network retry 或 execve escalation；Hook
输入没有 `call_id`、`:retry` 后缀、retry reason、sandbox permissions 或 strict 状态。Codex 当前实现中 Hook
`allow` 会先于 Guardian / 用户审批直接返回，因此会绕过 strict auto-review。只有能证明请求属于普通首次 Shell
approval、且完整 effective policy 结果为 Allow 时才自动返回 `allow`；其它情况仍保留现有弹窗。原生规则写入
失败则按 D25 降级为允许一次。

## 6. 已确认的总体实现模型

在 D3“只依赖标准 Codex”的约束下，需要两层记忆：

```text
PermissionRequest arrives
  -> AskHuman 对明确支持的 session 类型查询 session shadow rules
       -> 命中：直接返回 allow，不展示重复弹窗
       -> 未命中或不适用：只展示本请求可证明支持的选项
            -> 批准一次：仅返回 allow
            -> 本对话允许：写精确资源 / 项目 / 磁盘 session rule，再返回 allow
            -> Shell 始终允许：只写 Codex 原生 rules，再返回 allow
            -> 其它永久类型：按各自原生配置能力另行定案
            -> 拒绝：返回 deny
```

### 6.1 Shell permanent 的唯一真相与首版读取策略

直接从 Hook 外修改 `~/.codex/rules/default.rules` 不会让当前已经运行的 Codex
`ExecPolicyManager` reload。AskHuman 不为 Shell 再保存一份持久 shadow，而是把 Codex 原生文件作为唯一真相：

- 首版不设置 daemon policy 缓存、常驻 worker 缓存或完整命令结果缓存；D30 的临时 provenance
  只保存旧 Codex policy 基线与本次原生追加之间的摘要关系，不保存解析结果或授权判断；
- 每次判断都由短命隔离进程读取并解析当前支持版本的完整 policy；
- 当前 Codex 自带的 `codex execpolicy check` 可复用同版本原生 rule parser，但它只评估传入的显式规则文件与
  command tokens，不负责还原 active config layers、managed overlay、shell 分段、heuristics 或请求阶段；这些缺口
  必须由外围证明或安全降级，不能把该命令当作完整审批 oracle；
- 只有实测证明规则读取或解析成为瓶颈后，才另行评估缓存设计；
- 当前 Codex 再次调用 Hook 时，普通 Shell 请求若按最新原生 policy 已为 allow，则 Hook 自动返回 `allow`；
- 若 Hook 可能代表 sandbox retry、strict auto-review 或其它无法还原的阶段，不因 prefix rule 自动放行。

这样未来新 Codex 进程直接使用原生全局规则，当前未 reload 的 Codex 也能由 Hook 按同一真相补足普通请求，同时没有
两份永久规则的同步问题。临时 provenance 按 thread / rollout identity、Codex 进程代次与 policy context 分区；
不同子代理、cwd 或配置上下文不得借用另一份基线。它只保存在 daemon 内存中，daemon 重启、摘要不匹配或其它
policy 文件变化后立即失效并恢复弹窗。规则读取或判断失败时同样按 D24 显示原有弹窗。

MCP、network 等其它 permanent 类型是否需要不同的即时层仍分别待定，不能从 Shell 结论外推。

### 6.2 session rule 的持久化要求

D5 要求 Resume 后继续有效，因此仅保存在 daemon 内存不足以满足语义。最终存储至少需要：

- 以 `session_id` 为顶层 namespace；
- daemon / GUI 重启后仍可读取；
- 不以 `agent_id` 分裂根线程与子代理；
- 对未知或无法稳定 canonicalize 的请求 fail closed：不命中、不自动允许；
- 每条规则记录 `last_used_at`；只在规则实际匹配并自动允许时刷新；
- 规则连续 30 天没有命中后清理，普通聊天、Resume 或无关工具调用不续期；
- 定义硬容量上限与用户撤销机制，防止活跃会话在 30 天内生成无限精确文件键。

存储文件位置与硬容量上限尚未确认。永久 Codex rules 不受 30 天 session 清理影响。

### 6.3 查看与重置

设置页“高级”Tab 的最后一张卡提供静态“管理 Codex 会话授权”入口。该功能使用频率低，必须渐进加载：

1. 设置窗口启动不读取 rule store；
2. 切换到高级 Tab 也不查询 rule store，卡片不显示需要后端统计的动态 count；
3. 用户点击管理按钮后才连接 daemon 并加载对话摘要；
4. 展开某个对话时再加载其完整 scope / 路径详情；
5. 设置搜索只索引静态标题与说明，不触发规则加载。

列表按 Codex `session_id` 分组；优先使用 daemon 已有 agent registry 中的对话标题与项目名，不为这个页面扫描
全部 Codex rollout。标题不可用时显示项目名、缩短的 session id、最后使用时间与预计清理时间。每组展示已有
scope（精确文件数量 / project root / 完全磁盘，以及未来 shell、network、MCP session scope）。

首期唯一修改动作是“重置此对话授权”：一次删除该 `session_id` 下全部 AskHuman session rules，不做逐条或
逐路径编辑。daemon 必须串行完成原子落盘和内存 matcher 失效后再报告成功；此后下一次相关请求重新弹窗。
设置页不直接编辑存储文件。永久 Codex rules 不属于“重置对话授权”，其查看 / 撤销需要随对应永久类型另行
设计，不能在这里暗中修改 Codex 原生配置。

## 7. 尚未定案的问题

以下问题必须继续逐项与产品确认：

1. **shell 完整判断逻辑复刻**：Hook 没有 Codex 计算好的
   `proposed_execpolicy_amendment`；需先确认完整复刻当前版本所需的运行时输入、源码依赖、版本绑定与 fail-closed
   条件，再决定能否可靠提供 D18 的永久 prefix 选项。
2. **文件路径的持久 identity**：已确认精确文件 / Git 根（cwd 回退）/ 完全磁盘三级 scope；仍需定义
   symlink、不存在的新文件、大小写敏感性、remote environment 以及 TOCTOU 下的 canonical key 格式。
3. **网络请求识别**：Hook 缺少 native network approval context；哪些 tool input 足以证明 host / protocol，哪些必须
   回退原生 TUI。
4. **MCP always allow**：如何识别 user/project/plugin/app 配置来源、进行格式保留编辑，并避免写错层级。
5. **非文件选项文案**：文件编辑已确认使用“本对话”及项目/磁盘范围；shell、network、MCP 的 session 文案仍需
   分别确认，并在必要时标注 Resume / 子代理共享作用域。
6. **永久与 session rule 管理**：查看、撤销、冲突提示、保留周期、敏感详情与审计边界。
7. **不可达审批**：`request_permissions` 等不经过 Hook 的类型，是明确保留原生 TUI，还是未来等待 Codex 扩展 Hook
   协议；当前不得暗示已接管。
8. **Hook 信息不足时的降级**：只显示可证明安全的 scope，还是完全退出让 Codex TUI 展示原生候选；需要按类型定案。

## 8. 源码依据

HumanInLoop：

- `src-tauri/src/permissions.rs`：当前 Codex 只提供 approve once / deny；Claude suggestion 回放；Hook stdout。
- `src-tauri/src/integrations/agent_permission.rs`：PermissionRequest Hook 安装、信任与状态管理。

Codex：

- `codex-rs/core/src/hook_runtime.rs`：Hook 输入构造、共享 `session_id`、subagent context、permission mode 折叠。
- `codex-rs/hooks/src/schema.rs`、`events/permission_request.rs`、`engine/output_parser.rs`：command Hook 输入/输出契约及
  unsupported fields。
- `codex-rs/core/src/state/service.rs`、`session/session.rs`：每个 Session 的独立 `ApprovalStore` 与 Resume
  `session_id` 恢复。
- `codex-rs/core/src/agent/control.rs`、`agent/control/spawn.rs`：整棵 agent tree 共享 `AgentControl.session_id`，
  协作子代理另建 Session。
- `codex-rs/core/src/tools/sandboxing.rs`：`with_cached_approval` 的 session cache 行为。
- `codex-rs/core/src/tools/runtimes/apply_patch.rs`、`shell.rs`、`unified_exec.rs`：各 runtime 的 approval key 与
  cache 调用位置。
- `codex-rs/core/src/tools/network_approval.rs`：网络 session / permanent 决策。
- `codex-rs/core/src/mcp_tool_call.rs`：MCP session / remember 决策与缓存。
- `codex-rs/core/src/codex_delegate.rs`：内部 delegated sub-Codex 审批路径及其与普通 `spawn_agent` 的区别。
