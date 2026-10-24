# 行为规格：Antigravity 集成

> 状态：Unix 上已实现 None / CLI / MCP 三种互斥模式；CLI 为推荐模式，MCP 可选择。
> 范围：用户级 Antigravity 规则、MCP 配置、模式切换与当前明确支持/不支持的运行时能力。
> 本文只描述当前产品行为，不记录验收过程或历史调查。

## 1. 模式与产物

| 模式 | 托管产物 |
| --- | --- |
| None | 删除当前实例可证明所有权的共享规则；移除 `mcpServers.askhuman` |
| CLI | 安装 CLI 版共享规则；不安装 MCP 配置或超时 Hook |
| MCP | 安装 MCP 版共享规则；写入 `mcpServers.askhuman` |

- 三种模式互斥。重复设置当前模式会幂等协调其产物。
- CLI 与 MCP 共用规则文件 `~/.gemini/config/rules/askhuman-antigravity.md`。
- MCP 配置位于 `~/.gemini/config/mcp_config.json`。
- Antigravity 的规则与配置写入适配器仅支持 Unix。Windows 上的 Antigravity 变更返回明确的不支持错误，不写入部分产物。

## 2. CLI 模式

规则要求 Antigravity 通过其实际可用的命令执行工具调用：

```bash
AskHuman --caller-agent antigravity <command>
```

`--caller-agent antigravity` 只声明调用家族：

- 它不提供可信原生会话 ID，也不授予权限。
- 如果继承的其它 Agent 家族标记与本提示冲突，CLI 拒绝调用。
- CLI 模式不安装超时 Hook，也不承诺宿主具备可配置的超时。
- 异步让出、转后台或返回任务句柄都不构成人类回答。Agent 必须继续取得原始请求的实际结果。
- 对 `agy` 的 `run_command` / `manage_task` 指引只适用于实际提供这些工具契约的宿主；不能外推到 Antigravity IDE、扩展或其它客户端。

## 3. MCP 模式

托管条目固定为：

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

- `command` 使用当前 AskHuman 可执行文件的绝对路径。
- `timeoutSeconds: 86400` 是 24 小时配置预算，也影响连接与工具列表操作；它不是已经验证的真实等待时长。
- 配置编辑保留无关 JSONC 条目和注释。空文件按空对象处理；根值或 `mcpServers` 不是对象、JSONC 无法解析或外部写入造成预备内容失配时，操作中止并保留原文件。
- MCP server 仍通过 `AskHuman mcp` 暴露通用工具集，并复用 CLI 的提问、渠道、历史和 Daemon 生命周期。

## 4. 会话与历史边界

Antigravity 当前没有可信原生会话身份。

- CLI：当检测到 Antigravity 家族但没有可信 session ID 时，`--show-last` 必须拒绝项目级历史回退，并让 Agent 重新向人类确认。
- MCP：没有可信绑定时，`show_last` 以 MCP server 实例 + 项目为作用域。一个实例跨多个会话使用时可能产生歧义；server 重启后旧实例历史不再属于新实例。不能确认时应重新询问人类。
- 会话限制不影响普通 `ask` 和 `whats_next` 的执行，但依赖精确会话归属的历史恢复必须保守处理。

## 5. 规则所有权

共享规则由 AskHuman 严格托管：

- 规则文件必须是普通文件，包含版本化托管标记、预期 frontmatter 与单一 CLI/MCP 变体。
- Unix 写入通过固定目录描述符拒绝符号链接，并核验文件身份、所有者、模式、链接数与 SHA-256。
- 实例账本记录路径、生成版本、变体、摘要与文件身份。当前实例只能清理其可证明所有权的规则。
- 用户修改、外部替换、账本身份漂移、异实例账本或待恢复事务都不得被自动认领或覆盖。
- 读取或身份校验失败必须作为状态错误显示，不能降级成“未安装”。

最终检查与 rename/unlink 之间没有针对同一用户其它进程的原子 compare-and-swap 保证；所有权检查缩小覆盖风险，但不等同于跨进程原子事务。

## 6. 模式切换

Antigravity 模式切换顺序为：

1. 读取并验证 MCP 配置，计算目标配置。
2. 恢复当前规则实例已有的待处理日志。
3. 确认规则所有权后发布 MCP 配置。
4. 安装、更新或删除规则。

这不是跨两个文件的单一原子事务：

- MCP 配置发布失败时，规则保持不变。
- 规则变更失败时，先恢复规则日志；只有恢复到切换前的同一逻辑规则后，才尝试条件恢复 MCP 配置。
- 如果外部进程已改写配置，恢复会拒绝覆盖。
- 如果规则已提交但清理失败，或恢复结果不确定，错误必须报告保留状态与显式重试命令：

```bash
AskHuman agents mode antigravity <none|cli|mcp>
```

用户看到失败或不完整更新时应检查错误所述状态，再重试明确的目标模式；不得假定切换已原子完成。

## 7. 支持边界

当前实现支持提问、CLI/MCP 传输、历史恢复的安全降级和下一任务交接。以下能力没有 Antigravity 适配器：

- 生命周期追踪与 Agent Console 状态
- Stop 结束确认
- 原生权限拦截
- 上下文恢复 Hook
- 插话 / Watch
- 新建任务 / Fork

因此，Antigravity 模式不安装 lifecycle、Stop、permission、recovery、interject 或 launch/fork 产物，也不应展示为这些能力可用。

## 8. 平台与验证口径

- Antigravity 集成变更：Unix 支持；Windows 明确不支持。
- MCP server 运行时本身沿用 AskHuman 的跨平台进程与 Daemon 传输，但这不构成 Windows Antigravity 集成验收。
- 当前产品证据限于 Linux/NixOS 的本地运行验证。没有据此声明托管 CI、macOS、Windows、IDE、扩展、Antigravity 2.0 或真实 24 小时等待通过验收。
- 共享规则是家族级指令；各 Antigravity 客户端是否读取规则、提供命令工具、支持 MCP 或正确等待，仍须按客户端验证。

相关 MCP 通用契约见 [MCP 模式规格](./mcp.md)。
