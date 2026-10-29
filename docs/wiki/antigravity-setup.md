# Antigravity 集成

简体中文 | [English](./antigravity-setup.en.md)

AskHuman 可在 Unix 主机上集成 Antigravity。打开 **设置 → Agents → Antigravity**，从三种互斥模式中选择一种：

| 模式 | 行为 |
| --- | --- |
| **CLI**（推荐） | 安装 Antigravity 家族共享规则，让 Agent 调用 AskHuman 命令行。Antigravity 没有由 AskHuman 管理的超时 Hook。 |
| **MCP** | 安装共享规则，并把 AskHuman 注册为 MCP server。 |
| **未集成** | 移除当前 AskHuman 实例拥有的共享规则，以及 `mcpServers.askhuman` 条目。 |

也可以从命令行切换：

```bash
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity none
AskHuman agents mode antigravity
AskHuman agents update antigravity
```

## 共享规则与 CLI 模式

CLI 与 MCP 模式都会安装：

```text
~/.gemini/config/rules/askhuman-antigravity.md
```

该文件包含供 Antigravity 客户端使用的 always-on 规则。CLI 模式下，规则要求 Agent 带家族提示调用 AskHuman：

```bash
AskHuman --caller-agent antigravity "请确认下一步"
```

该提示只声明 Antigravity 家族，不认证具体会话，也不授予权限。如果继承了其它 Agent 家族的标记，AskHuman 会放弃这次调用的身份绑定并继续执行；依赖可信调用方的历史恢复仍会拒绝。

CLI 模式不安装超时 Hook。等待能力取决于所选 Antigravity 客户端提供的命令工具；异步让出或转入后台不代表人类已回答。生成的提示词只在宿主确实提供 `run_command` 与 `manage_task` 时给出条件性指引，不会假定 IDE 或扩展使用相同的工具名、任务 ID、参数或超时控制。

## MCP 模式

MCP 模式把 AskHuman server 写入：

```text
~/.gemini/config/mcp_config.json
```

托管条目使用 AskHuman 可执行文件的绝对路径、`mcp` 参数与 24 小时超时预算：

```json
{
  "mcpServers": {
    "askhuman": {
      "command": "/absolute/path/to/AskHuman",
      "args": ["mcp"],
      "timeoutSeconds": 86400
    }
  }
}
```

`timeoutSeconds` 是 AskHuman server 的配置预算，也会影响连接与工具列表操作；它不代表 AskHuman 已验证过真实 24 小时的人类等待。

AskHuman 只编辑 `askhuman` 条目，并保留其它 JSONC 条目与注释。如果文件格式错误，或 `mcpServers` 不是对象，操作会中止且不覆盖原文件。

若 `mcp_config.json` 是符号链接或硬链接，AskHuman 不会自动改写它。stow / chezmoi 用户应在配置源文件中手工维护上面的 `askhuman` 条目；Home Manager 用户应在声明式配置中添加并 rebuild，不要直接修改 Nix store。状态查询和无需写入的幂等操作仍可读取有效链接。整个 `.gemini/config` 为链接时，共享规则路径检查仍可能拒绝模式切换；详见 [链接配置边界](../specs/antigravity.md#31-链接配置与声明式配置管理)。

## 会话与历史限制

AskHuman 当前拿不到可信的 Antigravity 原生会话 ID。

- CLI 模式下，`AskHuman --caller-agent antigravity --show-last` 无法区分不同会话，因此会拒绝项目级回退。请让人类重新确认此前问答。
- MCP 模式下，历史恢复回退到「MCP server 实例 + 当前项目」。若同一实例服务多个会话，恢复结果可能不明确；重启 server 后会开始新的历史作用域。无法确认时应请人类重新回答，不能猜测当前会话。

这些限制只影响历史恢复；正常提问与任务结束交接仍可使用所选调用方式。

## 平台与客户端范围

- Antigravity 规则与配置变更由 Unix 集成路径提供。Windows 会明确返回不支持，不会写入半套集成。
- 共享规则不保证每个 Antigravity 客户端都会读取它、提供命令工具或支持 MCP；请在实际使用的 CLI、IDE 或扩展中检查行为。
- 当前运行时验证仅覆盖 Linux/NixOS。本文不宣称已通过托管 CI、macOS、Windows、IDE、扩展或 Antigravity 2.0 验收。

## 未支持能力

Antigravity 当前没有以下 AskHuman 集成：

- 生命周期追踪或 Agent Console 状态
- Stop 结束确认
- 原生权限拦截
- 上下文恢复 Hook
- 插话或 Watch
- 新建任务或 Fork

当前可用的是提问、在存在安全历史作用域时恢复历史，以及下一任务交接。依赖可信会话或原生生命周期 Hook 的能力仍不可用。

## 安全切换模式

Antigravity 先准备 MCP 配置，再协调规则。规则使用基于内容与 uid 的所有权检查及恢复日志；如果规则操作失败，MCP 条目仅在可确认时恢复。两个文件不是一个原子事务。

如果模式切换报告更新中断或不完整，请检查错误中给出的状态，并显式重试目标模式：

```bash
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity none
```

对于符号链接、被外部修改、正文或所有者与本实例账本不一致，或无法恢复的规则，AskHuman 会拒绝覆盖。所有权与事务契约见 [Antigravity 行为规格](../specs/antigravity.md)。
