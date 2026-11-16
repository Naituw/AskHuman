# Antigravity 集成

简体中文 | [English](./antigravity-setup.en.md)

AskHuman 可在 Unix 主机上集成 Antigravity，形式是**一个 Antigravity 插件**：Antigravity App 与 `agy` 命令行共用它，里面有交互规则、AskHuman 的 MCP server 和生命周期 hooks。
打开 **设置 → Agents → Antigravity**，从三种互斥模式中选择一种：

| 模式 | 行为 |
| --- | --- |
| **MCP**（推荐） | 安装插件：规则、MCP server（`askhuman_mcp`）与 hooks。App 与 `agy` 都能用。 |
| **CLI** | 安装插件：规则（让 Agent 调用 AskHuman 命令行）与 hooks，不含 MCP。Antigravity App 的默认沙箱可能挡住命令行调用，所以不作为推荐。 |
| **未集成** | 移除 AskHuman 托管的插件，并清理更早版本留下的共享规则与 `mcpServers.askhuman` 条目。 |

也可以从命令行切换：

```bash
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity none
AskHuman agents mode antigravity
AskHuman agents update antigravity
```

## 插件

插件目录是：

```text
~/.gemini/config/plugins/askhuman/
```

里面有 `plugin.json`、`rules/AGENTS.md`（交互规则）、`hooks.json`（生命周期开启时）、MCP 模式的 `mcp_config.json`，以及 AskHuman 的所有权标记 `.askhuman-managed`。
放进这个目录后 Antigravity 会热加载，无需注册；`agy plugin list` 也能列出它。

- **生命周期追踪**：hooks 把 Antigravity 会话报给 AskHuman，会话会出现在 Agent 控制台，显示「工作中 / 空闲」和标题（取第一条用户请求）。可以在设置页单独关闭。
- **在 Antigravity 里停用插件**：Antigravity 会把停用写进自己的配置。AskHuman 只读检测它，并在设置页、`AskHuman agents`、`AskHuman doctor` 里提示「插件已在 Antigravity 中停用」；它不会改动 Antigravity 的配置。
- **只改 AskHuman 自己的文件**：插件文件只在内容变化时重写；用户改过的文件视为冲突，保持原样。重写文件不会重启运行中的 MCP server，也不会打断在途的提问；但 `mcp_config.json` 的改动要等 Antigravity 重启才作用于已运行的 server。

## 首次授权（MCP 模式，App）

Antigravity App 里 MCP 工具默认「询问」：第一次调用 AskHuman 时会弹出原生提示，请选「始终允许」；也可以在 App 的权限设置里手工加 `mcp(askhuman_mcp/*)`。
在 AskHuman 还没看到这条授权时，设置页与 `AskHuman agents` 会提示这一步。AskHuman 只读地查看这条授权，不写入。

## CLI 模式与规则

规则要求 Agent 带家族提示调用 AskHuman：

```bash
AskHuman --caller-agent antigravity "请确认下一步"
```

该提示只声明 Antigravity 家族，不授予权限。如果继承了其它 Agent 家族的标记，AskHuman 会放弃这次调用的身份绑定并继续执行；依赖可信调用方的历史恢复仍会拒绝。

CLI 模式不安装超时 Hook。等待能力取决于所选 Antigravity 客户端提供的命令工具；异步让出或转入后台不代表人类已回答。生成的提示词只在宿主确实提供 `run_command` 与 `manage_task` 时给出条件性指引，不会假定 IDE 或扩展使用相同的工具名、任务 ID、参数或超时控制。

## MCP 模式

插件里的 `mcp_config.json`：

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

宿主把它命名为 `askhuman_mcp`。`timeoutSeconds` 是 AskHuman server 的配置预算，也会影响连接与工具列表操作；它不代表 AskHuman 已验证过真实 24 小时的人类等待。

插件目录是 AskHuman 托管内容，不支持符号链接、硬链接或非当前用户的文件，遇到时会报告冲突并保持原样；详见 [链接与声明式配置管理](../specs/antigravity.md#31-链接与声明式配置管理)。

## 会话与历史

AskHuman 用 Antigravity 宿主给出的**根会话 ID** 区分会话：

- MCP 模式下，每次工具调用都带宿主注入的会话信息，`show_last` 与历史按会话作用域，子会话归到根会话。
- CLI 模式下，环境里的会话 ID 只作候选，AskHuman 仅在该会话已由生命周期 hook 登记为活会话时才采信；否则 `AskHuman --caller-agent antigravity --show-last` 会拒绝项目级回退，请让人类重新确认此前问答。
- 带子 Agent 的任务只显示根会话一行：AskHuman 从父会话的记录里认出子 Agent，把它们的活动并进根会话；结束确认与插话也都对根会话生效。

## 结束确认、插话与 Watch

- **结束时确认**：设置 → Agents → Antigravity 里打开，或 `AskHuman agents stop antigravity on`；默认关闭。Agent 自然结束一轮时，AskHuman 弹出确认卡：继续（可附一段后续指令）或结束；出错、取消或 24 小时无人答复都按结束处理。带子 Agent 的任务要等整棵任务都结束后才问一次，子 Agent 自己不会弹卡。App 与 `agy` 通用；改这个开关会重写插件文件（规则文件随之重新生成）。
- **插话**：对工作中的会话（App 与 `agy`），在 Mac 控制台、托盘、手机或 IM 的 `/msg` 里发消息，会在 Agent 下一次调用模型之前送达；Agent 正在跑一个很长的工具调用时要等它结束。对空闲或已结束的 App 会话发消息，经 AskHuman 的 sidecar 送进原会话；`agy` 会话没有这条通道。
- **Watch**：手机与 Mac 控制台实时显示会话里的回复和工具步骤。部分模型带有「思考」文字，目前不显示。

## 创建任务

Antigravity 也出现在新任务入口里（手机、IM `/new`、Mac 新任务窗口）。有两条路线，**设置 → Agents → Antigravity** 里的「默认运行方式」只决定先用哪一条，用不了时用另一条：

- **App**（仅 macOS）：任务在 Antigravity App 里创建并自动发送。App 没有「在指定文件夹里新建会话」的公开入口，所以 AskHuman 用 App 自己的 **sidecar** 机制：每个文件夹一个小帮手，登记在 `~/.gemini/config/sidecars/`，名字都以 `askhuman-` 开头，在 App 里显示为各自的项目。需要先在设置里**一次性允许「AskHuman 管理 Antigravity sidecar」**；没允许时会明确拒绝，不会偷偷创建。
- **agy（终端）**：打开系统终端，运行 `agy -i <任务>`。`agy` 只对你信任过的目录报告工作目录，没信任的目录里会话仍会出现，但不知道所在文件夹。

权限：

- **Agent 默认**（App 路线）：任务只能做该文件夹 sidecar 里已允许的操作——AskHuman 自己的工具，以及你在设置里为该文件夹添加的规则，其余自动拒绝。你在 Antigravity 里已有的授权不会被改动。
- **YOLO**：App 路线为该文件夹建一个名字带「YOLO」的宽授权 sidecar（命令、读写文件、联网与 MCP 都不再询问），设置页标为「宽授权」，可以一键移除；`agy` 路线加 `--dangerously-skip-permissions`。

管理 sidecar：设置页的 sidecar 列表（连接状态、规则、移除），或命令行：

```bash
AskHuman agents sidecars
AskHuman agents sidecars remove <id>
AskHuman agents sidecars remove --all
```

最多保留 20 个。Antigravity 里已经创建的会话由 Antigravity 管理，AskHuman 不会替你删除。任务入口里的模型清单就是 `agy models` 的列表（没装 `agy` 时只有 Flash Lite / Flash / Pro 三个别名）。

## 平台与客户端范围

- Antigravity 插件与配置变更由 Unix 集成路径提供。Windows 会明确返回不支持，不会写入半套集成。
- 宿主契约（hooks 载荷、MCP 元数据、插件加载）在 macOS 上用 Antigravity App 2.32.1 与 `agy` 1.5.9 观察过；Linux 沿用同一实现但尚未实测。本文不宣称已通过托管 CI、Windows、IDE、扩展验收。
- 插件不保证每个 Antigravity 客户端都会读取规则、提供命令工具或支持 MCP；请在实际使用的 App 或 `agy` 中检查行为。

## 未支持能力

Antigravity 目前没有以下 AskHuman 集成：

- 原生权限拦截
- 上下文恢复 Hook
- Fork
- Windows，以及 Linux 上的 App 新建任务（App 路线仅 macOS）

## 安全切换模式与迁移

切换模式时 AskHuman 先发布（或卸载）整个插件，再清理更早版本留下的共享规则 `~/.gemini/config/rules/askhuman-antigravity.md` 与全局 `~/.gemini/config/mcp_config.json` 里的 `askhuman` 条目，所以任何时刻都不会没有交互规则，最多短暂重复。
旧文件被你改过时 AskHuman 会保留它们并报告，插件仍已就位。多个文件不是一个原子事务。

如果模式切换报告更新中断或不完整，请检查错误中给出的状态，并显式重试目标模式：

```bash
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity none
```

对于符号链接、被外部修改、内容或所有者与本实例账本不一致的文件，AskHuman 会拒绝覆盖。所有权与事务契约见 [Antigravity 行为规格](../specs/antigravity.md)。
