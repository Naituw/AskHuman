# 行为规格：Antigravity 集成

> 状态：Unix 上已实现 None / CLI / MCP 三种互斥模式；三种集成模式都安装同一个 AskHuman 托管插件，推荐 MCP。
> 范围：用户级 Antigravity 插件（规则、MCP 配置、生命周期 hooks）、模式切换、迁移，以及当前明确支持/不支持的运行时能力。
> 本文只描述当前产品行为，不记录验收过程或历史调查。会话身份之后的能力（Stop 确认、插话、Watch、新建任务、模型、YOLO）的设计、验证与未验项见
> [Antigravity 深度集成](./antigravity-deep-integration.md)，计划见 [plans/antigravity-deep-integration.md](../plans/antigravity-deep-integration.md)；
> 该规格的 §6 是插件的权威描述，本文是其中已落地部分的行为摘要。

## 1. 模式与产物

| 模式 | 托管产物 |
| --- | --- |
| None | 删除当前实例可证明所有权的插件目录；清理更早版本留下的共享规则与 `mcpServers.askhuman` |
| CLI | 插件：`plugin.json`、`rules/AGENTS.md`（CLI 版）、`hooks.json`（生命周期开启时）、`.askhuman-managed`；不含 MCP 配置 |
| MCP | 同上，规则为 MCP 版，另含 `mcp_config.json` |

- 三种模式互斥。重复设置当前模式会幂等协调其产物：只重写内容变了的文件。
- 插件目录是 `~/.gemini/config/plugins/askhuman/`，Antigravity App 与 `agy` 共用，放进去即被热加载，无需注册。
  用户可以在 Antigravity 里整体停用它（宿主在 `~/.gemini/config/config.json` 写 `plugins.askhuman.enabled=false`）；
  AskHuman **只读**检测这一点并在设置页、`agents`、`doctor` 里提示，从不写这个文件。
- `hooks.json` 随生命周期开关生成：开启时有 `SessionStart`、`PreInvocation`、`Stop` 三个事件（后两个 `timeout` 为 86400 秒，
  因为插话要等 composer 提交、Stop 确认要等人回答）；关闭时不写这个文件。`Stop` 走共享的 `AskHuman __stop-hook antigravity track`；
  「结束时确认」开关打开（`AskHuman agents stop antigravity on`，默认关）时再加一步 `confirm`，由它出确认卡。改这个开关会重写整个插件，规则文件随之重新生成。
- 推荐 MCP，因为 App 的默认沙箱里 CLI 变体连不上 AskHuman 的 Daemon socket；MCP 在 App 与 `agy` 里都能用。
- Antigravity 的插件写入器仅支持 Unix。Windows 上的 Antigravity 变更与状态查询返回明确的不支持错误，不写入部分产物。

## 2. CLI 模式

规则要求 Antigravity 通过其实际可用的命令执行工具调用：

```bash
AskHuman --caller-agent antigravity <command>
```

`--caller-agent antigravity` 只声明调用家族，不授予权限：

- 会话来自宿主环境里的 `ANTIGRAVITY_CONVERSATION_ID`，它只是**候选**，是否采信见 §4。
- 如果继承的其它 Agent 家族标记与本提示冲突，CLI 放弃本次调用的身份绑定并继续执行；历史恢复仍安全拒绝，详见 §4。
- CLI 模式不安装超时 Hook，也不承诺宿主具备可配置的超时。
- 异步让出、转后台或返回任务句柄都不构成人类回答。Agent 必须继续取得原始请求的实际结果。
- 对 `agy` 的 `run_command` / `manage_task` 指引只适用于实际提供这些工具契约的宿主；不能外推到 Antigravity IDE、扩展或其它客户端。

## 3. MCP 模式

插件里的 `mcp_config.json` 固定为：

```json
{
  "mcpServers": {
    "mcp": {
      "command": "/absolute/path/to/AskHuman",
      "args": ["mcp"],
      "timeoutSeconds": 86400
    }
  }
}
```

- 宿主把插件里的 MCP server 命名为 `<插件名>_<server>`，即 `askhuman_mcp`；授权写作 `mcp(askhuman_mcp/*)`。
- `command` 使用当前 AskHuman 可执行文件的绝对路径。
- `timeoutSeconds: 86400` 是 24 小时配置预算，也影响连接与工具列表操作；它不是已经验证的真实等待时长。
- 重写插件文件不会重启运行中的 MCP server，也不会打断在途的 `ask`；但 `mcp_config.json` 的改动只作用于**之后**启动的 server，已运行的要等 Antigravity 重启。
- App 里 MCP 工具默认「询问」：第一次调用会弹原生提示，用户点「始终允许」后持久（或在其权限设置里加 `mcp(askhuman_mcp/*)`）。
  AskHuman 只读地看 `config.json` 的 `userSettings.globalPermissionGrants.allow` 是否已含 `mcp(*)` 或 `mcp(askhuman_mcp/…)`，仅用于设置页与 `agents` 的提示，不依赖、不写入。
- MCP server 仍通过 `AskHuman mcp` 暴露通用工具集，并复用 CLI 的提问、渠道、历史和 Daemon 生命周期。

### 3.1 链接与声明式配置管理

插件目录是 AskHuman 托管内容，不支持符号链接、硬链接或非当前用户的文件：遇到时状态为冲突，保持原样，不覆盖、不删除，错误写明原因与显式重试命令。
stow / chezmoi / Home Manager 等工具若要管理这个目录，应自行维护一个独立插件（换一个插件名），并对 AskHuman 选择 None；不要让两者指向同一目录。

## 4. 会话与历史边界

会话身份是**根会话 ID**（UUID）。可信来源与用途：

- hook 载荷的 `conversationId`：生命周期登记（会话出现在 Agent 控制台，状态与标题）。
- MCP 调用的 `_meta`（`antigravity.google/root_conversation_id`，缺失时取 `conversation_id`，须是 UUID）：提问、`show_last` 与历史按这个会话作用域；子会话取根。
- CLI：环境里的 `ANTIGRAVITY_CONVERSATION_ID` 只作为候选随提问带给 Daemon，Daemon 仅在该会话已由生命周期 hook 登记为活会话时才采信；
  不一致、已结束或没有登记都按「只认家族」处理，因此被无关进程继承的环境变量不能冒充会话。
- 都没有时回到只认家族：CLI `--show-last` 必须拒绝项目级历史回退并让 Agent 重新向人类确认；MCP `show_last` 以 MCP server 实例 + 项目为作用域，
  一个实例跨多个会话使用时可能产生歧义，server 重启后旧实例历史不再属于新实例。不能确认时应重新询问人类。
- 会话限制不影响普通 `ask` 和 `whats_next` 的执行，但依赖精确会话归属的历史恢复必须保守处理。
- 继承 `ANTIGRAVITY_CONVERSATION_ID` 本身不得阻止其它 Agent 的 hooks、MCP 或 CLI 入口；无显式 hint 时保留原有家族探测和可信 MCP 绑定。显式 `--caller-agent antigravity` 与其它家族标记冲突时，命令继续运行并在 stderr 提示，当前 CLI 调用不携带家族、会话或 MCP 实例绑定；`--show-last` 拒绝恢复，不得回退项目历史。非法、缺值或重复的 caller flag 仍属于参数错误。
- 显式 hint 的冲突检查仅使用非空会话标记（`CODEX_THREAD_ID`、`CLAUDECODE` / `CLAUDE_CODE_SESSION_ID`、`CURSOR_CONVERSATION_ID`、`GROK_SESSION_ID`、`PI_SESSION_ID` / `PI_SESSION_FILE`）及 MCP 家族绑定。`CODEX_HOME` 等配置/工作区变量不触发冲突；这不授予可信 session，也不改变无 hint 时既有家族探测顺序。
- 带子 Agent 的会话归并成一棵树（深度集成规格 D17）：hooks 对根会话和子会话各自上报，daemon 从父会话 transcript 里 `invoke_subagent` 的结果认出子会话并折进根——
  控制台、Stop 确认和插话都只有根这一行；子会话自己的 `Stop` 不出卡，它的活动让根保持「工作中」。根在整棵树都空闲、且没有未读的跨会话消息之后才算结束。树只在 daemon 内存里，不持久化。
- `agy` 在没有信任过的目录里，hook 载荷的 `cwd` 与 `workspacePaths` 是空的：会话照常登记和追踪，只是不知道所在文件夹；信任该目录后才有。

控制台里的会话标题取该会话第一条用户请求：读 `~/.gemini/<surface>/brain/<会话 ID>/.system_generated/logs/transcript_full.jsonl`
（App 是 `antigravity`，`agy` 是 `antigravity-cli`）的第一个用户输入步骤，去掉宿主加的 `<USER_REQUEST>` 外壳；只有规范形式的 UUID 才会拼进路径，读取有字节与行数上限。

## 5. 插件所有权

插件目录由 AskHuman 严格托管：

- **实例账本**：每个 AskHuman 实例在自己的配置目录里有 `antigravity-plugin.json`，记录路径、生成版本、模式与能力清单、每个文件的 SHA-256 与所有者 uid。
  持久所有权按「路径相同、摘要相同、uid 相同、没有链接」判定；备份恢复、`cp -a` 产生的新 inode 或跨重启设备号变化不会丢失所有权，内容被改过则摘要不同，仍为冲突。
- 状态为：不存在、当前、已过期（内容与本版本应发布的不同）、需修复（账本有记录但目录或文件已不在）、冲突（那里有无法证明属于 AskHuman 的内容）。
  用户改过文件、外部替换、异实例账本、符号链接或硬链接、非当前用户所有，都是冲突：不自动认领、不覆盖、不删除。
- 新建时在插件目录同一文件系统的私有临时目录里整体写好（目录 0700、文件 0600），再用一次 `rename` 发布，宿主看不到半成品。
  更新时先把意图写进账本，再按固定顺序（规则 → MCP → hooks → `plugin.json` → 标记）逐文件原子替换，只重写内容变了的文件；中途中断后下一次对账会完成它，而不是回滚。
- 卸载只删账本证明属于 AskHuman 的文件与空目录；用户在目录里加的其它文件保留，目录因此不会被删。
- 读取或身份校验失败必须作为状态错误显示，不能降级成「未安装」。

最终检查与 rename/unlink 之间没有针对同一用户其它进程的原子 compare-and-swap 保证；所有权检查缩小覆盖风险，但不等同于跨进程原子事务。

## 6. 模式切换与迁移

模式切换顺序为：

1. 发布（或在 None 时卸载）插件。
2. 协调生命周期偏好与产物。
3. 清理更早版本留下的共享规则 `~/.gemini/config/rules/askhuman-antigravity.md` 与全局 `~/.gemini/config/mcp_config.json` 里的托管条目 `mcpServers.askhuman`。

先发布插件再清理旧产物：任何时刻都不会出现没有交互协议的窗口，短暂并存只是规则重复。

- 旧产物按它们原来的严格所有权规则处理（实例账本、摘要、uid、不跟随链接、保留 JSONC 注释与无关条目）；任何一步证明不了所有权（用户改过）就保留、报告冲突、不继续清理，插件保持已发布。
- 两者都不存在时不读不写，所以一个无关的、格式错误的全局 MCP 配置不会阻止插件安装。
- 在插件发布之前，旧规则与全局 MCP 条目仍然定义当前模式，并显示为「需更新」；发布后状态以插件为准，旧产物残留只让规则显示需更新，不影响任务就绪判断。
- 这不是跨多个文件的单一原子事务。失败时错误必须报告保留状态与显式重试命令：

```bash
AskHuman agents mode antigravity <none|cli|mcp>
```

用户看到失败或不完整更新时应检查错误所述状态，再重试明确的目标模式；不得假定切换已原子完成。

整包更新和单项更新与状态查询使用同一套判定。读取失败、配置解析错误、所有权冲突和待恢复日志不会降级为 None 或静默卸载。

## 7. 支持边界

当前支持：提问、CLI/MCP 传输、可信会话绑定与历史恢复、生命周期追踪（会话在控制台出现、工作中 / 空闲、标题），App 与 `agy` 通用；以及：

- **Stop 结束确认**（App 与 `agy`）：根会话、`terminationReason == NO_TOOL_CALL`、整棵会话树 `fullyIdle` 后出确认卡，继续 = `decision: "continue"`；开关 `agents stop antigravity`，默认关。
- **插话**：工作中经 `PreInvocation` 的 `injectSteps` 注入（App 与 `agy`）；App 的空闲／已结束会话经 sidecar 的 `agentapi send-message`，内容前加固定的 `[USER MESSAGE]` 包装；`agy` 没有空闲通道。
- **Watch**：读会话的 `transcript_full.jsonl`，显示回复与工具步骤；`thinking` 文字目前不显示。
- **新建任务**（手机、IM `/new`、Mac 新任务窗口）与**模型选择**：App 经 AskHuman 管理的全局 sidecar，终端用 `agy -i`；权限按 sidecar 白名单，可选 YOLO（App 用宽授权 sidecar，`agy` 用 `--dangerously-skip-permissions`）；
  模型清单是 `agy models` 的平铺列表。App 路线需要用户一次性允许 AskHuman 管理 sidecar（`antigravityDesktop.manageSidecars`）。

以下能力不提供，也不应展示为可用：

- 原生权限拦截
- 上下文恢复 Hook
- Fork

设计、验证与未验项见深度集成规格 §9–§12、§17–§18；原生权限拦截与 Fork 只记录调研结论（§13），上下文恢复不在范围内。

### 7.1 App 新建任务与 sidecar

App 没有「在指定文件夹里新建会话」的公开入口，但它会为 `~/.gemini/config/sidecars/<id>/` 里的每个 sidecar 启动一个进程，该进程能运行 `agentapi` 并带着清单里授予的权限。AskHuman 据此：

- 每个文件夹一个全局 sidecar：`askhuman-ws-<h8>`（只允许白名单）、`askhuman-yolo-<h8>`（宽授权，名字带「YOLO」，设置页标注、可一键移除）；另有不属于任何用户文件夹、没有授权、只用来传空闲消息的 `askhuman-ctl`。最多 20 个。
- AskHuman 只写 `~/.askhuman/antigravity-sidecars.json`、各 sidecar 目录与 `config.json` 里的 `sidecars.askhuman-*` 条目（读-改-写，一次 `rename`，其余字节不动，用户的授权与插件开关原样保留）。
- sidecar 进程从不启动 daemon，只连 daemon 并执行 daemon 下发的固定 `agentapi` 调用；不接受任意命令。
- 创建 sidecar 要用户一次性允许（`antigravityDesktop.manageSidecars`，默认关）；未允许时明确拒绝，不静默创建，也不静默改走 `agy`。
- Antigravity 里已经创建的会话由 Antigravity 管理，AskHuman 不替用户删除；移除 sidecar（`AskHuman agents sidecars remove`）只移除它自己的文件与配置条目。
- 管理命令 `AskHuman agents sidecars [list|remove <id>|remove --all]`，设置页 Antigravity 卡里有同样的列表与规则编辑。

## 8. 平台与验证口径

- Antigravity 集成变更：Unix 支持；Windows 明确不支持。
- MCP server 运行时本身沿用 AskHuman 的跨平台进程与 Daemon 传输，但这不构成 Windows Antigravity 集成验收。
- 插件与 hooks 的契约在 macOS 上用 Antigravity App 2.32.1 与 `agy` 1.5.9 实测；实测证据与未验证项（V 编号）记录在深度集成规格 §17 与附录 A。
  `run_command` / `manage_task` 等工具契约的既有观察版本为 `agy 1.2.9`，这些维护信息只记录在规格中，不注入 always-on 规则。不能据此推断其它客户端或版本提供相同接口。
- Linux 沿用 Unix 实现但尚未实测；没有据此声明托管 CI、Windows、IDE、扩展或真实 24 小时等待通过验收。
- 插件是家族级指令；各 Antigravity 客户端是否读取规则、提供命令工具、支持 MCP 或正确等待，仍须按客户端验证。

相关 MCP 通用契约见 [MCP 模式规格](./mcp.md)。
