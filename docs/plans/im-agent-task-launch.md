# 实现计划：从 IM 创建可在电脑端接续的 Agent 任务

> 需求：`docs/specs/im-agent-task-launch.md`（D1–D32）。
> 本计划按依赖给出唯一落地方案；没有待实现者自行选择的产品分支。

## 总览

```text
IM /new
  → readiness gate（Terminal + workspace + binary/lifecycle/integration Agent）
  → Workspace select
  → Agent select
  → Permission select（全局设置为 ask 时）
  → 渠道原生 Task Input（可选项目 TODO；提交即启动）
  → 0600 LaunchRecord(token) + PendingLaunchWatch
  → Terminal.app 新 window
  → AskHuman __agent-launch <token>
  → claim + validate + chdir + argv exec 交互式 Agent
  → turn-start 匹配 session
  → 来源 IM 自动 watch
  → 现有 AgentRegistry 继续生命周期管理
```

| 模块 | 职责 |
|---|---|
| `agents/workspaces`（新） | 四家冷扫描、lifecycle 增量、workspace index |
| `integrations/agent_launch`（新） | 三重 readiness、LaunchRecord、Agent argv / permission adapter、helper |
| `integrations/terminal_launch`（新） | Terminal.app 探测、自检与新窗口启动 |
| `autochannel` | 无参 `/new` 解析、help / readiness 文案 |
| `select` + 四渠道 | Workspace / Agent / Permission 三类选择卡 |
| `agent_task` daemon 领域段 | flow / task-input / pending-watch 台账与状态机 |
| 四渠道 task input | 飞书 form、钉钉模板、Slack input block、Telegram 来源选择 + ForceReply/确认卡 |
| lifecycle reporter | launch id + prompt hash 上报，不传 task 正文 |
| 设置 / doctor | enable+keepalive、permissionPrompt、Terminal、workspace、Agent readiness |

## P1-1 配置、路径与 keepalive 门控

**触点**：`config.rs`、`paths.rs`、设置前后端类型。

1. 新增：

```text
AgentTasksConfig {
  enabled: bool = false,
  permission_prompt: ask | agent-default | yolo = ask
}
```

作为 `AppConfig.agent_tasks`；serde 缺字段兼容旧配置，未知枚举回默认 `ask`。

2. 新路径：
   - `agent_workspaces_file()` → `~/.askhuman/agent-workspaces.json`；
   - `agent_workspaces_lock()`；
   - `agent_launch_dir()` → `~/.askhuman/state/agent-launch/`。
3. 开启事务：
   - 写 `agentTasks.enabled=true`；
   - 写 `general.daemonLifecycle=keepalive`；
   - `login_item::install_daemon()` / refresh；
   - `client::ensure_running()`；
   - 任一步失败返回设置页，不显示 ready。
4. 关闭只关 enabled，不回退 lifecycle mode；后续普通设置保存按用户当前值 reconcile 登录项。
5. daemon `/new` 分派再次读 enabled / keepalive；旧卡提交也重检，关闭后不能启动。

**测试**：旧 config 默认、枚举容错、enable 强制 keepalive、disable 不回退、旧 flow 在关闭后失败。

## P1-2 WorkspaceIndex 领域模型与持久化

**新**：`src-tauri/src/agents/workspaces/mod.rs`。

1. `WorkspaceRecord`：canonical path、last seen、agent kinds、last kind、source、pinned、hidden。
2. `WorkspaceIndex`：
   - 容错 load；原子 save；目录 0700、文件 0600、跨进程 lock；
   - `observe(kind, cwd, ts, source)`；
   - canonical path 去重，要求 absolute directory；
   - `candidates(20)` 每次过滤不存在 / hidden；pinned 优先、时间倒序；
   - 非隐藏最多 50 条；hidden tombstone 有界保留，防冷扫描立即补回；
   - lifecycle 可更新时间但不解除 hidden；电脑端 unhide / manual add 才恢复。
3. 展示消歧：basename + 一层 parent；同名逐级扩展；HOME 显示 `~`；完整路径仅设置用。
4. 不向上找 git root，保留启动 cwd、子目录与 worktree。
5. dirty debounce + daemon tick / shutdown flush，避免 Activity hook 高频写盘。

**测试**：canonical/symlink 去重、同名消歧、排序/置顶/隐藏、删除过滤、裁剪、权限、worktree fixture。

## P1-3 四家冷启动 scanner

**新**：`agents/workspaces/{claude,codex,cursor,grok}.rs`。

公共约束：

- 每家最多检查最新 50 个 artifact、贡献最多 20 个 cwd；
- 先 metadata / mtime 排序再读取；JSONL 只读前 128 KiB 或直到 cwd；
- 不调用 transcript/title 内容解析，不读取用户 prompt；
- blocking worker 执行，单家失败不影响其它家；
- index 空或 scan waterline 超过 1h 时由 `/new` 触发；设置「刷新」可强制；
- 合并现有 `agents.json` 的 active / retained ended cwd；不从无 Agent 身份的普通回复历史补种。

分家：

1. Claude：`~/.claude/projects/*/*.jsonl`，找首个合法顶层 `cwd`，timestamp 回退 mtime。
2. Codex：`~/.codex/sessions/**/rollout-*.jsonl`，解析开头 `session_meta.payload.cwd/started_at`。
3. Grok：`~/.grok/sessions/*/*/summary.json` 的 `info.cwd` 与 `last_active_at|updated_at`；目录名
   URL decode 只作 cwd 缺失回退，仍需 existing-dir。
4. Cursor：
   - 只使用 `<encoded>/agent-transcripts/**` 的 session / mtime；
   - `chats/*/*/meta.json` 不含 cwd，不作路径来源；
   - `decode_existing_cursor_path` 从 `/` 按真实 child 名做有界 DFS，恰一个 existing directory 才成功；
   - 0 / 多解、已删除项目、仅有 repo 目录但无 Agent transcript 全部跳过。

**fixture**：四家最小/损坏/超大/缺 cwd；Cursor `foo-bar` vs `foo/bar` 唯一/歧义/不存在；读取 cap。

## P1-4 lifecycle → workspace 增量

**触点**：`agents/registry.rs`、daemon AgentEvent 分派。

1. `ServerState` 持有共享 WorkspaceIndex service。
2. lifecycle event 经 AgentRegistry 接受后，用 kind/cwd/ts observe；cwd 缺失不写。
3. launch helper claim 也把 cwd 提到最近，但不替代 lifecycle Agent 身份。
4. workspace index 不影响 daemon idle / drain / Agent state 判据。

**回归**：原 AgentRegistry snapshot、seq、ended retention、poller、watch 消费者不变。

## P1-5 Agent readiness probe

**新领域**：`integrations/agent_launch::readiness`。

每家固定定义：

```text
AgentLaunchAdapter {
  kind,
  command: claude | codex | cursor-agent | grok,
  lifecycle_status,
  integration_mode_status,
  permission argv builder
}
```

1. binary probe：
   - 用用户 `$SHELL` 启 login shell，对固定 command 执行有界 `command -v`；
   - IM 无法影响 command string；timeout（初版 2s，现为查找 6s、超时单独报告，见 spec §4.2）；过滤 shell rc 噪音，只接受 absolute executable file；
   - executable owner 不作硬门控，但必须当前用户可执行；结果缓存 5min；
   - `$SHELL` 无效时回退 daemon PATH + 已知用户 bin 目录的固定文件检查。
2. lifecycle probe：`agent_lifecycle::status(kind)` 必须 supported + installed + !outdated。
   - daemon 启动既有 `migrate_outdated` 先执行；若仍 outdated 则 unavailable，不在 `/new` 隐式改配置。
3. integration probe：`agent_mode::current(target)` 必须为 CLI 或 MCP，Rule/skill 必须已安装，且
   CLI timeout Hook / MCP config 通道产物必须已安装并 current。Rule/skill 正文和 Subagent Guard
   过期只保留集成更新提示，不阻止 `/new`；这里只检查，不自动 install/update。
4. ready = binary && lifecycle && integration；PermissionRequest capability 与认证状态只作诊断，不门控。
5. `readiness_all()` 并行探测四家，返回稳定 reason code：binary_missing、lifecycle_off、
   lifecycle_outdated、integration_off、integration_outdated、ready。

**测试**：临时 HOME/PATH/login-shell fixture、rc 噪音、timeout、alias/function 非 executable、binary ×
lifecycle × integration mode/update 组合。

## P1-6 `/new` 解析、来源激活与总 readiness gate

**触点**：`autochannel.rs`、daemon `handle_inbound`、i18n。

1. `Command::NewTask` 无 payload：
   - `/new` / `!new` 成功分类；
   - `/new <anything>` 仍归 NewTask 但携带 `has_extra=true`，分派只回「`/new` 不带参数」并停止；
   - 不增加中文命令别名，保持唯一入口 `/new`（Slack 展示 `!new`）。
2. `/new` 先 `activate_channel_on_action`，使来源成为 active slot；这也遵守既有 autoEndWatch 行为。
3. readiness 并行：
   - feature/keepalive/login item；
   - macOS GUI + Terminal.app probe；
   - WorkspaceIndex candidates；空/过期则冷扫描后再取；
   - `readiness_all` ready Agent。
4. 成功条件：workspace ≥1 且 ready Agent ≥1；成功直接发 Workspace picker。
5. 失败：发送一个合并诊断，不创建 flow：
   - workspace 为空原因 / 扫描时间；
   - 四家 binary/lifecycle/integration 状态；
   - Terminal/keepalive 修复路径。
6. 部分 Agent unavailable 不阻止流程；Workspace 卡 footer 简短列「另有 N 个 Agent 未就绪」，具体
   原因用设置 / doctor 查看。

**测试**：无参/带参/Slack、active question 时命令优先、激活槽、各 readiness 组合、scanner 失败。

## P1-7 Flow 台账与三阶段选择卡

**触点**：`select.rs`、daemon `PickerKind/PickerEntry`、四渠道 select renderer。

1. `AgentTaskFlowEntry`：flow id、source channel、workspace?、agent?、permission?、stage、created_at；
   task 在最后提交前不存在。TTL 30min，每渠道最多 10 个；flow 不持久化。
2. 泛化 `PickerEntry.options` 为 stable domain id，不再只称 session_id；既有 picker 行为不变。
3. `SelectAction / PickerKind` 墨：
   - `TaskWorkspace`；
   - `TaskAgent`；
   - `TaskPermission`。
4. Workspace picker：option id 为 flow 内随机 token，daemon 映射 canonical path，callback 不暴露路径；
   primary 为消歧路径，secondary 为最近 Agent + 时间；首卡最近 5 个 +「显示更多」，展开卡最多 20。
5. Workspace 点击：CAS `SelectingWorkspace → SelectingAgent`，定格旧卡，发 ready Agent picker。
6. Agent picker：只列 `/new` readiness 快照里 ready 的 Agent；workspace last kind 排第一。点击后：
   - global `permissionPrompt=ask` → CAS 到 SelectingPermission，发二选一卡；
   - `agent-default|yolo` → 写 permission，直接进入 TaskInput。
7. Permission picker：两个 option，无预选：
   - Agent 默认行为；
   - YOLO（危险 dot/style；secondary 为该 Agent 实际 flag 与风险）。
8. 每步 callback 都验证 channel、message id、flow stage、option token；旧/重复 callback 只 ACK 不推进。

**测试**：stage CAS、并行 flow、TTL/cap、option token 不泄露路径、permission 三态跳转、四家 YOLO 文案。

## P1-8 四渠道原生 Task Input

**新 transport-neutral view**：

```text
TaskInputView {
  flow_id,
  workspace_label,
  agent_label,
  permission_label,
  todo_project?,
  todo_choices: [{ id, text_snapshot, auto }],
  manual_default = true,
  placeholder,
  max_chars = 3000,
  submit_label = 启动任务
}
```

业务台账 `TaskInputEntry` 记录 channel message identity + flow id + created_at；复用 flow TTL。

1. 进入 task-input stage 时把 selected workspace 交给 `project::detect_from`，只读取该 git-root project 的
   `todos::list`。保存前 10 条 TODO 的 id、完整文本快照与 auto 标记，并沿用现有溢出提示；找不到项目
   或没有 TODO 时保持原纯输入行为。
2. 扩展通用 `ConfirmInput` / `ChoiceFormView`：支持 input 始终可见，同时只在「直接输入新任务」被选中
   时要求非空。现有条件显示输入与普通 Confirm/Ask 行为不变。
3. 飞书、Slack、钉钉：有 TODO 时渲染「直接输入新任务」+ TODO 单选和同一卡 input，默认选中手动输入；
   TODO 沿用 whats-next 的本地化前缀，飞书/钉钉改用琥珀色 `【TODO】`，auto TODO 再加 `⚡`；
   无 TODO 时不渲染选项。各渠道继续复用当前 form state / nonce / private data，
   终态禁用 input/actions 并显示「正在启动 / 已启动 / 失败」。
4. Telegram：
   - 有 TODO 时先发送来源 picker：「输入新任务」+ TODO；无 TODO 时直接进入当前 ForceReply；
   - 选择手动输入后发送含 workspace/Agent/permission 的 ForceReply + inline Cancel，reply 即提交；
   - 选择 TODO 后发带「启动任务」/「取消」的确认卡；点击可直接启动，回复该卡则把文本作为可选补充
     并以默认「启动任务」提交；普通 text 与错 reply 不消费；
   - picker/确认卡都保存 TODO 完整快照，过期和重复回调仍由 flow CAS 拦截。
5. 提交解析抽成 transport-neutral helper：手动输入外围 trim 后必须非空；TODO 可无补充，有补充时产出
   `todo_snapshot + "\n\n" + supplement.trim()`；保留内部换行，拒绝 NUL，并对最终文本执行 3000
   Unicode 字符上限。失败保持 input active。
6. submit 是最终确认：CAS `AwaitingTask → Launching` 成功者才创建 record；cancel CAS 到 Cancelled。
   Terminal 打开成功后才对所选 project/id 调用 `todos::take` 并写既有执行历史；Terminal 失败保留 TODO。
   TODO 已被并发删除时 `take` 作为 best-effort 成功处理，launch 仍使用卡片快照。

**测试**：项目范围、前 10 条/溢出/`⚡`、无 TODO 纯输入、默认手动、TODO + 空/非空补充、组合超长/NUL、
四渠道 view/parser、callback actor/message identity、Telegram 来源 picker/非 reply/错 reply/确认卡、
Terminal 成功出队/失败保留/并发删除、submit/cancel 竞态、重复 submit 最多一次。

## P1-9 LaunchRecord、权限 adapter 与隐藏 helper

**新**：`src-tauri/src/integrations/agent_launch.rs`；  
**触点**：`cli/mod.rs` 隐藏分派、IPC ack。

1. `LaunchRecord`：id/TTL/source/task/task_sha256/canonical cwd/kind/permission/current exe/dev home。
2. `create`：UUID、create-new/atomic rename、0600；launch dir 0700；TTL 2min。
3. `claim(token)`：
   - 只接受 canonical UUID；原子 rename；校验 owner/mode/TTL/instance home；
   - 重新 canonicalize cwd 并要求 identity 不变；
   - 读取后立即 unlink；双 claim 只有一个成功。
4. `AskHuman __agent-launch <token>`：
   - stdio 继承 TTY；claim 失败打印错误 exit 1；
   - `set_current_dir`；从 Terminal login-shell PATH 定位固定 executable；
   - 构造 argv：
     - agent-default：`<command> <prompt>`；
     - yolo：`<command> <fixed yolo flag> <prompt>`；
   - 用已验证的 `--` / positional boundary 处理前导 `-`；task 永不进 shell；
   - 设置 `ASKHUMAN_AGENT_TASK_LAUNCH_ID=<uuid>`；
   - claim ack 发 daemon 后 `CommandExt::exec` 替换为 Agent；不 wait/restart/poll。
5. 固定 YOLO 映射：Claude dangerous skip；Codex dangerous bypass approvals+sandbox；Cursor `--yolo`；
   Grok `--always-approve`。不得把其它 flags 写入 record。
6. daemon tick 清理过期 record；不保留 task 历史副本。

**安全测试**：task 含单双引号、反引号、`$()`、`;`、换行、NUL、前导 `-`；断言 argv。token
穿越、symlink、权限、过期、双 claim、四家 default/yolo adapter。不运行 Agent。

## P1-10 Terminal.app adapter

**新**：`integrations/terminal_launch.rs`；可抽取 `terminal_focus.rs` 的 AppleScript runner 公共段。

1. `probe()`：macOS GUI/Aqua session、`/System/Applications/Utilities/Terminal.app` 可用、Automation
   只能通过 setup test 预先验证。
2. shell command 的唯一动态值：
   - `current_exe()` 绝对路径，经严格 POSIX 单引号；
   - UUID token 字符白名单；
   - 不含 task/cwd/Agent/permission。
3. AppleScript `do script` 明确创建新 window，不复用 tab，不 `activate`。
4. 默认 login shell 执行 helper；helper/Agent 退出后 shell 保留。
5. `open_new_window` 返回 OS accepted receipt；daemon 异步等 claim ack，不能阻塞 Router。
6. 设置 `terminal_test` 使用专用 self-test token：只打印成功与 helper cwd，绝不构造 Agent / prompt；
   用于触发 TCC Automation 授权。
7. AppleScript builder / shell escape 纯函数测试；自动测试不实际打开 Terminal。

## P1-11 自动 watch 关联

**触点**：`ipc/mod.rs` AgentEvent、`agents/report.rs`、daemon lifecycle 分派、既有 watch 启动函数。

1. task submit 创建 LaunchRecord 前同时登记 `PendingLaunchWatch`：launch id、channel、kind、cwd、
   SHA-256、created/claimed time、60s deadline。
2. helper claim ack 把 pending 标为 claimed；Terminal launch 失败则删除 pending。
3. reporter 只在 turn-start 提取 prompt：兼容 `prompt` / `user_prompt` / camelCase；计算 SHA-256；读取
   `ASKHUMAN_AGENT_TASK_LAUNCH_ID`（UUID 才接受）。`ClientMsg::AgentEvent` 增两个 optional 字段，旧消息兼容。
4. daemon 先 `AgentRegistry::apply_event`，拿到 session/seq，再匹配：
   - exact launch id；
   - 否则 kind + canonical cwd + hash + claimed-before-event + 60s；
   - 相同 Codex 并发按 pending claim 顺序与 hook arrival 顺序一对一消费。
5. 命中后调用既有 watch start，channel 固定为 source channel；不广播到所有 enabled channel；
   不改变 watch persistence / autoEndWatch / update engine。
6. 60s timeout 或 watch send failure：回来源 IM warning；launch 仍成功；不创建 AgentRecord、不重启 Agent。
7. flow/task input 卡终态与 watch 卡分开：前者定格「已启动」，后者是正常实时卡。

**测试**：launch id 精确、hash fallback、wrong cwd/kind、超时、重复 hook、相同 Codex FIFO、watch send
失败、task 正文不入 IPC/日志。

## P1-12 设置 UI 与 doctor

**触点**：`SettingsView.vue`、`lib/ipc.ts/types.ts`、`commands.rs`、`cli/doctor.rs`、i18n。

「实验 → 从 IM 创建 Agent 任务」：

- enable + keepalive/login item 说明；
- 权限选择方式：每次询问（默认）/总是 Agent 默认/总是 YOLO；YOLO 持久选择需醒目风险提示；
- Terminal.app 状态 +「测试 Terminal」；
- workspace：主卡片只保留数量与管理入口；独立面板左上角「完成」、右上角 `+`，用 macOS 系统目录选择器添加；列表展示完整路径、最近时间、Agent badges，每行用统一 `…` 菜单处理 pin/hide/unhide/forget；
- Agent readiness 四行：binary path、lifecycle status、CLI/MCP integration status、ready reason；
  PermissionRequest 只作辅助信息；
- 总 readiness：keepalive、Terminal、workspace count、ready Agent count。

同一 UI 变更调整 Agent 集成 Tab：

- overview 后先渲染完整「自动集成」标题、更新总览与四家 Agent 卡；
- 再渲染完整「手动集成」标题、CLI/MCP 提示词与 MCP 配置示例；
- 只移动现有模板块，事件与状态引用不复制、不改集成语义；
- `overviewDesc` 中英文改为先推荐自动按钮，手动复制作为后备，避免文案仍指向旧顺序。

commands：读取 workspace、scan refresh、workspace mutation、agent readiness、Terminal test；workspace
mutation 独立持久化，不通过整份 config 覆盖。

doctor 增只读段：feature、keepalive/login item、Terminal、workspace、四家 binary+lifecycle+
integration；不得启动 Agent、不得触发登录或网络模型请求。

## P1-13 help、overview、用户文档

1. `/help` 仅在 enabled 时列 `/new`；关闭时直接输入仍回启用指引；Slack 展示 `!new`。
2. `overview-im-commands.md` 增 readiness、flow、task input、auto-watch 与代码入口。
3. 功能实现后更新 `overview.md`：命令列表、模块地图、顶层 `agentTasks`、macOS Terminal 边界。
4. `overview-configuration.md` 增 `agentTasks.enabled/permissionPrompt` 与动态 state 边界。
5. wiki 中英双语：出门前 checklist（enable/keepalive、Terminal test、workspace、ready Agent）；权限
   Default vs YOLO 风险；自动 watch 失败含义。
6. 钉钉新模板附 README、变量/action id、导入/更新说明。

## P1-14 验证

### 自动验证（不启动 Agent、不计费）

- Rust：config、WorkspaceIndex、四家 scanner fixture、Cursor path recovery、三重 readiness、flow CAS、
  workspace/Agent/权限/Telegram 任务来源 picker、项目 TODO 快照与合并、四渠道 task input、Terminal
  成功/失败后的 TODO 出队语义、LaunchRecord、argv/permission adapter、shell escape、Terminal AppleScript、
  pending-watch matcher；
- 前端 build、`cargo test`；
- `./scripts/install.sh`（功能/逻辑变更后项目强制）；
- 新安装 `AskHuman doctor`；临时 HOME / mock channel API 集成测试；
- Terminal adapter dry-run 只检查固定 helper command。

### 人工验证（仍不启动 Agent）

- 四渠道 `/new` 到 task input 后验证：无 TODO 不显示选项；有 TODO 时项目范围、`⚡`、默认手动、TODO
  直接启动/补充、留空/过期；Telegram 另验来源 picker、ForceReply、TODO 确认与取消；
- ready/unready 诊断、权限全局三态、task input 空/组合超长、重复 callback、Terminal 失败不出队；
- 设置「测试 Terminal」确认新 window、Automation 授权、shell 留存（只跑 self-test helper）。

### 真实 Agent 验收（强制 AskHuman 授权门）

以下任何动作前必须通过 AskHuman 明确询问并获得许可：

- 在 Terminal 中启动 Claude / Codex / Cursor / Grok；
- 向任一家发送任何 prompt；
- 验证 default / YOLO 的真实工具权限；
- 验证真实 lifecycle prompt hash 与自动 watch；
- 特别是 Cursor 的任何会话测试。

获批后按用户指定 Agent、workspace、task 与预算做最小样本。未获批时可凭自动验证 + Terminal self-test
交付，但必须注明「真实 Agent E2E / 自动 watch 未实测」。

## P2 后续能力（不在首版）

1. iTerm2 adapter，再扩 Ghostty / WezTerm / Kitty；逐个验证新 window、TTY、login shell、shell 留存。
2. Linux：优先 `xdg-terminal-exec`，再做 GNOME Terminal / Konsole 等 adapter。
3. Windows：先补 lifecycle，再接 Windows Terminal；否则不能满足启动后现有生命周期接管。
4. 可选 Agent 专属更多权限档位；只有能跨产品解释清楚且不削弱 Default/YOLO 安全文案时再加。
5. 可选 task attachment；需逐家定义 CLI attachment argv，不把文件路径拼 shell。

## 实现顺序

```text
P1-1 config/path/keepalive
  → P1-2 WorkspaceIndex
  → P1-3 vendor scanners
  → P1-4 lifecycle observe
  → P1-5 Agent readiness
  → P1-6 /new + total gate
  → P1-7 flow + three picker kinds
  → P1-8 channel-native task input
  → P1-9 LaunchRecord/helper/permission argv
  → P1-10 Terminal.app
  → P1-11 auto-watch correlation
  → P1-12 settings/doctor
  → P1-13 docs/template
  → P1-14 install + non-billed verification
  → AskHuman approval gate → optional real-Agent E2E
```

## 明确不做

对照 spec §11：不做 headless manager、进程控制、resume/retry、自动 worktree、raw path/flags、自动安装
hooks、附件 task、iTerm2/Linux/Windows，以及未经批准的真实 Agent 测试。

## 用户定案摘要

见 spec D1–D32。核心是：`/new` 无参；先 readiness；workspace → ready Agent
（binary+lifecycle+integration）→ Default/YOLO → 原生
task input，提交即启动；task input 可从所选 workspace 的项目 TODO 选择并附加补充，Terminal 成功打开
才出队；Telegram 使用来源选择后分流；系统 Terminal 新窗口；来源设为 active 并默认自动 watch；
厂商索引冷启动 + lifecycle 增量；开启功能强制 keepalive；未经 AskHuman 许可不启动任何真实 Agent。
