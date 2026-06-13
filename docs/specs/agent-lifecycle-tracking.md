# 需求：Agent 生命周期追踪 + 状态窗口（实验性功能）

> 状态：方案设计（待评审）
> 关联计划：`docs/plans/agent-lifecycle-tracking.md`
> 关联调研：`demo/agent-lifecycle/FINDINGS.md`（三家 hook 事件/env、Cursor 双触发去重、进程存活轮询为唯一不漏的结束信号、身份相关结论、各家标题来源——全部实测）
> 影响面：daemon（新增 agent 注册表 + 存活轮询 + 持久化 + 闲退守卫 + 订阅推送）、IPC（`ipc/mod.rs` 新增消息）、CLI（`cli/mod.rs` 新增 `__agent-hook` 与 `agents` 子命令）、客户端（`client/` ask 顺带上报活动）、新 GUI 窗口（`?view=agents` + `app` 角色 + `commands`）、Hook 集成（新增三家 lifecycle hook 安装/卸载/状态 + Codex 信任哈希 Rust 实现）、配置（`config.rs` 新增 `experimental`）、设置前端（`SettingsView.vue` 实验区 + 新 Tab）、i18n。
> **不改**：stdout 洁净契约、退出码语义（0/1/3）、既有 timeout hook 行为、IM 渠道与弹窗逻辑、graceful-drain 既有判据。daemon 协议仅增量演进、向后兼容。

## 1. 背景

上一阶段在 `demo/agent-lifecycle/` 实测了 Claude Code / Codex / Cursor 三家 CLI 的生命周期信号，结论已写入 `FINDINGS.md`。本需求把这些结论落到产品里，但**单独成一个可独立测试的功能**，**不含** IM 渠道的「激活 / 反激活」逻辑（那是后续的「IM 渠道激活」需求 `docs/todos/im-channel-activation.md`，将构建在本功能之上）。

本功能交付两件事：

1. **设置里一个隐藏的「实验性功能」区**：默认不显示，需先在「通用」Tab 底部打开一个隐蔽开关才出现。展开后是一个「实验」Tab，内含 **Claude Code / Codex / Cursor 三个「生命周期追踪」开关**，开/关即**安装 / 卸载**对应 agent 的**用户级** lifecycle hook。
2. **`AskHuman agents status` 打开一个动态更新的 GUI 窗口**：按 agent 类型分组，展示当前**工作中 / 空闲**以及**最近结束**的 agent，每个含「类型 / 标题 / sessionID / 项目(cwd) / 启动时间 / 最近活动时间 / 状态 / pid」。窗口实时刷新。

## 2. 目标（用法）

```bash
# 打开 agent 状态窗口（动态更新；daemon 不在则自动拉起）
AskHuman agents status
```

- 在「设置 → 通用」底部打开「实验性功能」→ 出现「实验」Tab → 分别为 Claude Code / Codex / Cursor 打开「生命周期追踪」→ 本应用把用户级 lifecycle hook 写入各家配置。
- 之后任意启动 / 使用这些 agent，`agents status` 窗口即按类型分组实时显示其状态。
- 关闭某家开关即移除其 lifecycle hook（不影响既有 timeout hook、不影响其它无关 hook）。

## 3. 术语

- **lifecycle hook**：本功能安装的、上报生命周期事件的 hook（区别于既有的 **timeout hook** `askhuman-timeout.sh`）。
- **agent 记录 / session**：被追踪的一次 agent 会话，**以 `session_id` 为身份**。
- **存活轮询（poller）**：daemon 周期性 `kill -0 <pid>` 判断 agent 进程是否还在。

## 4. 已确认决策

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 功能边界 | **仅**做「生命周期追踪 + 状态窗口 + Hook 安装开关」。**不含** IM 渠道 attach / 激活逻辑（留给后续激活需求） |
| D2 | 平台范围 | **macOS + Linux**（daemon / hook / socket 依赖 Unix）。**Windows 完全不显示该设置**（不提示、不报错，等同功能不存在） |
| D3 | 总架构 | **daemon 为中枢**：内存维护 agent 注册表 + 持久化到 `~/.askhuman/agents.json`；hook 子进程经 daemon socket 上报事件；`agents status` 开一个**长驻订阅** daemon 推送的 GUI 窗口（动态更新） |
| D4 | Hook 命令实现 | 统一走二进制隐藏子命令 `AskHuman __agent-hook <agent> <event>`：读 stdin JSON 取 `session_id`、运行时 `detectRunningAgent` 去重、向上 walk 进程树找 agent pid、连 daemon 上报、**exit 0 + 空 stdout**。**不**再写独立 shell 脚本 |
| D5 | 事件集 | 安装 **sessionStart + turn-start + turn-end + sessionEnd** 四类（Codex 无 sessionEnd，仅前三）。**进程存活轮询是权威的「已结束」判据**（关窗 / kill-9 时 turn-end/sessionEnd 都不触发，全靠它）；turn 起止仅用于切「工作中 / 空闲」 |
| D6 | 鲁棒性（不依赖 sessionStart） | 任意事件（**尤其 turn-start**）都要做 pid 发现 + **幂等登记**；缺 sessionStart 也能正常追踪 |
| D7 | 身份模型 | **身份 = `session_id`**（不同 session_id ⇒ 不同记录）。**pid 仅用于存活轮询**。同一 pid 出现**新** session_id ⇒ 旧 session 标记「已结束」、新 session 复用该 pid 追踪（**一个 pid 同时至多一个活动 session**）。pid 死亡 ⇒ 该 pid 当前活动 session 标记「已结束」 |
| D8 | 状态三态 | **工作中**（turn 进行中）/ **空闲**（等输入）/ **已结束**。由 turn 事件 + 存活/TTL 推导 |
| D9 | 展示字段 | 类型 / 标题 / sessionID / 项目(cwd) / 启动时间 / 最近活动时间 / 状态 / pid |
| D10 | 标题来源（实测） | **Cursor**：`~/.cursor/chats/*/<sid>/meta.json` 的 `.title`（缺失→回退首条用户消息）；**Codex**：`~/.codex/sessions/.../rollout-*-<sid>.jsonl` **无存储标题**→取首条**真实**用户消息（跳过 `<environment_context>`/`<user_instructions>` 等注入块）；**Claude**：`~/.claude/projects/*/<sid>.jsonl` 最后一条 `summary`，否则首条真实用户消息。全取不到→「(未命名)」。按 `session_id` 全局 glob 定位文件 |
| D11 | 「已结束」保留 | 全局**最多 10 条**（按结束时间 FIFO 淘汰），**不设**时间窗 |
| D12 | TTL 兜底 | **仅当拿不到 / 无法轮询 pid 时**（如 Linux 上 Claude `CLAUDE_CODE_ENV_SCRUB` 的 PID namespace 隔离）启用：**超过 1 小时无任何活动**即判「已结束」。**任意 hook 事件**与**每次 `AskHuman` 提问调用**都重置该 session 的活动时间（一个 session 跑超过 1h 很正常，期间可能多次提问）。pid 可轮询时以轮询为准、**不**应用 TTL |
| D13 | 排序 / 分组 | 顶层**按类型分组**（Claude / Codex / Cursor 区块）；区块内**按状态【工作中 → 空闲 → 已结束】**，同状态内按时间倒序（工作中/空闲按「最近活动」，已结束按「结束时间」） |
| D14 | 显示范围 | **跨项目全部** agent（daemon 为 per-user，能看到所有项目） |
| D15 | UI 入口 | 「通用」Tab 底部一个隐蔽开关「实验性功能」（持久化 `config.experimental.enabled`）；打开后出现新「实验」Tab，含三家追踪开关。**Windows 完全不渲染**该开关与 Tab |
| D16 | per-agent 开关语义 | 开 = 安装用户级 lifecycle hook，关 = 卸载；开关状态以**实际安装状态**为准（同既有 hook 卡的 `*_status`）。与既有 timeout hook **各自独立**（不同标记、可共存）。**隐藏**「实验性功能」开关**不**卸载 hook（仅隐藏 UI；追踪继续） |
| D17 | 写入方式 | 沿用既有 hook 的**格式保留编辑**（`claude_hook.rs` 的 jsonc CST 风格）：**只增删本功能自己的条目**，绝不改动其它 hook 的字节 / JSON 转义。Cursor=`~/.cursor/hooks.json`、Claude=`~/.claude/settings.json`、Codex=`~/.codex/config.toml` 的 `[hooks]` + `[hooks.state]` `trusted_hash`（**Rust 实现信任哈希**，参考 `FINDINGS §6.2` + `demo/agent-lifecycle/harness/codex-trust.cjs`） |
| D18 | daemon 闲退 / 持久化 / 重连 | 闲时退出守卫**只受**【工作中 agent 数】与【状态窗口连接】影响（**空闲 agent 不保活**）；**版本更新 graceful-drain 不受 agent 影响**（仅在途 ASK 请求 gate drain，与今一致）；状态持久化 `agents.json`，daemon 重启 / 换新后重载并 `kill-0` 复核、剔除已死；状态窗口断连**自动重连**（必要时拉起 daemon） |
| D19 | CLI 形态 | `AskHuman agents <sub>`，本期仅 `status`（打开 GUI 窗口）。`agents` 设计为**可扩展子命令组**，预留未来子命令。**不**做纯文本 `list`，CLI **不**做 enable/disable（开关只在设置里） |
| D20 | IPC 增量 | 新增 `ClientMsg::AgentEvent` / `ClientMsg::AgentsSubscribe`、`ServerMsg::AgentsState`（快照推送）；`TaskRequest` 增**可选** agent 身份字段（type/session_id/pid）。serde 默认 + 同二进制两端，向后兼容 |
| D21 | ask 调用＝活动信号 | agent 通过 `AskHuman` 提问时，CLI **顺带 best-effort**（不阻塞作答主链路）上报 agent 身份给 daemon → 刷新该 session「最近活动」+ 重置 TTL。**仅刷新已存在的追踪 session，不新建**（尊重「未装 hook = 不追踪」） |
| D22 | 去重细则 | **仅当从 env 明确识别出「不同的」running agent 时**才跳过（`exit 0` 不上报）；env 无法判定 → 按 intended 处理（不跳过），避免漏报。识别顺序 `CURSOR_*`→cursor、`CODEX_*`→codex、`CLAUDECODE`→claude；**`CLAUDE_PROJECT_DIR` 不可作判据**（Cursor 也设它） |
| D23 | stdout 契约 | `__agent-hook` **永远 `exit 0` + 空 stdout**（sessionStart/turn-start 的 stdout 会被注入模型上下文；Cursor `stop` 空输出 = no-op）。失败全部 fail-open |
| D24 | i18n | 新 UI / 窗口中英双语（zh/en），沿用既有 i18n 体系 |

## 5. 非目标（明确不做）

- 不做 IM 渠道的 attach / detach / 激活门控（后续需求）。
- 不做 Windows 支持（且 UI 完全隐藏）。
- 不做 CLI 端的 enable/disable 与纯文本状态输出。
- 不追踪 Pre/PostToolUse 等工具级事件（噪音大、对状态判定无必要）。
- 不在本功能里改既有 timeout hook / 弹窗 / IM 渠道行为。

## 6. 已知风险

- **Codex 信任哈希与 Codex 版本相关**：`trusted_hash` 由 Codex 源码的 hook identity 结构推导；若 Codex 改了该结构，旧哈希失效、hook 会被判 Untrusted 而不执行。需在 `status` 里能识别「未受信任 / 已漂移」并提示重装；算法与出处记于 `FINDINGS §6.2`。
- **Linux 上 Claude 的 PID namespace 隔离**可能让 walk/`kill-0` 失效 → 落到 D12 的 TTL 兜底。
- **空闲 agent 不保活 daemon** 的代价（D18）：agent 空闲超过 daemon 空闲上限后 daemon 退出，下个 turn-start 重新拉起（首事件略有延迟）；窗口开着时无此问题。**用户已确认可接受**。

## 7. 反馈意见

（评审 / 实测中的修改意见追加到此处，标注日期。）
