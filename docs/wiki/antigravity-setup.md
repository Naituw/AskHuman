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
- 子 Agent 暂时各自作为一个会话出现，标题为空；如何归并还在决定中。

## 平台与客户端范围

- Antigravity 插件与配置变更由 Unix 集成路径提供。Windows 会明确返回不支持，不会写入半套集成。
- 宿主契约（hooks 载荷、MCP 元数据、插件加载）在 macOS 上用 Antigravity App 2.32.1 与 `agy` 1.5.9 观察过；Linux 沿用同一实现但尚未实测。本文不宣称已通过托管 CI、Windows、IDE、扩展验收。
- 插件不保证每个 Antigravity 客户端都会读取规则、提供命令工具或支持 MCP；请在实际使用的 App 或 `agy` 中检查行为。

## 未支持能力

Antigravity 目前没有以下 AskHuman 集成：

- Stop 结束确认
- 插话与空闲消息
- Watch
- 原生权限拦截
- 上下文恢复 Hook
- 手机 / IM 新建任务与 Fork

当前可用的是提问、按会话恢复历史、生命周期追踪（控制台状态与标题）以及下一任务交接。Stop 确认、插话、Watch 与新建任务已有设计，分期实现。

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
