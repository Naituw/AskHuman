# 主 Agent 专属 AskHuman 协议与 Sub Agent Guard 实施计划

> 对应需求：`docs/specs/main-agent-only-interaction-protocol.md`
> 当前阶段：已完成；未进行 Agent / Sub Agent 实测。

## 实施原则

- 分成两部分：先建立共享提示词的主 / 子角色边界，再实现 Claude / Codex 的 SubagentStart Hook 兜底。
- 共享语义只有一个来源；CLI、MCP、Grok skill 与 Hook 文案通过函数复用或语义断言避免漂移。
- Guard 属于 Agent 集成的指令包，不属于 lifecycle tracking，也不新增设置开关或前端行。
- Cursor 3.7.36 与 Grok 0.2.93 没有可靠的启动上下文注入通道，首期只依赖提示词。
- 自动化测试只执行纯函数、配置文件和构建验证；真实 Agent 测试保持显式人工授权门禁。

## Part 1：更新共享提示词

### 1.1 在原 protocol 内增加两条规则

在 `src-tauri/src/prompts.rs` 的 CLI / MCP `<mandatory_interaction_protocol>` 最前面加入同样的两条加粗规则：

```text
**This protocol does not apply to subagents. If you are a subagent, do not use AskHuman.**
**When starting a subagent, tell it that it is a subagent and must not use AskHuman.**
```

它们位于现有 “must apply under all circumstances” 文字之前。其余 protocol 原文不改。

### 1.2 更新 Grok skill 常驻描述

修改 `src-tauri/src/integrations/grok_skill.rs::frontmatter()`，只替换一句：

```diff
- It ALWAYS applies, with no exceptions.
+ It ALWAYS applies, with one exception: if you are a subagent, it does not apply to you.
```

`grok_skill_body()` 继续原样复用 MCP 版正文和 Grok 工具发现降级说明。

### 1.3 提示词测试

扩充 `prompts.rs` 与 `grok_skill.rs` 单测：

- 三个产物都含 Sub Agent exemption、Sub Agent AskHuman prohibition、委派提醒三项语义。
- 两条新增规则出现在 mandatory “under all circumstances” 文案之前。
- 主 Agent 分支仍含所有既有协议关键句与结束标记。
- MCP 版不重新引入 shell 专属文本；CLI 版仍保留 timeout / help 指令。
- Grok frontmatter 保留 `ALWAYS applies` 强度并只增加 Sub Agent 例外。

## Part 2：实现 Claude / Codex SubagentStart Hook

### 2.1 增加 Hook 专用提示与隐藏子命令

在 `prompts.rs` 增加 `subagent_guard_context()`，固定返回：

```text
You are a subagent. Do not use AskHuman.
```

新增 CLI 隐藏角色：

```text
AskHuman __subagent-hook <claude|codex>
```

在 `src-tauri/src/cli/mod.rs` 分发中直接打印 `hookSpecificOutput.additionalContext` JSON 后退出 0。
实现不启动 daemon、不读取 stdin、不调用 AskHuman 提问流程；参数无效或序列化失败时保持 fail-open，
输出空对象或空 stdout，不阻止上游创建 Sub Agent。

为 JSON 生成写纯函数测试，断言 event name、additionalContext 和合法序列化。

### 2.2 新增 Guard 集成模块

新增 `src-tauri/src/integrations/agent_subagent_guard.rs` 并从 `integrations/mod.rs` 导出，接口与现有
Permission Hook 保持相似：

- `supported(target)`：Claude Code / Codex；不包含 Cursor / Grok。
- `status(target)`：报告 installed / outdated / needs_update 所需的内部状态。
- `install_unlocked(target)` / `uninstall_unlocked(target)` / `reconcile_unlocked(target, mode)`。
- 独立 marker：`__subagent-hook`。
- 事件：nested `SubagentStart`。
- command：当前二进制绝对路径 + marker + agent family。
- 使用短、显式 timeout；状态比较锁定 handler type、command、timeout、唯一数量和 Codex trust。

复用 `hook_edit::upsert_nested_group()`、`remove_nested_marker()`、`nested_groups()` 与
`atomic_write()`，不自行重写 JSON。若现有 helper 无法表达“无 statusMessage 的短 timeout”之外的精确状态，
只增加最小的共享查询 helper。

### 2.3 Codex trust 与回滚

Guard 安装 / 更新 Codex hooks.json 后调用现有
`agent_permission::reconcile_codex_trust(old, new, &["__subagent-hook"])`：

- 新 Guard handler 获得信任。
- 已受信的其它 Hook 通过旧 hash 迁移机制保留。
- hooks.json 或 config.toml 任一步失败时恢复两份原始字节。
- status 同时校验 Guard handler 的实际 hash 与 `[hooks.state]` 条目。

为 SubagentStart label、handler 索引变化、同事件用户 Hook 共存、trust 缺失 / 过期和回滚辅助纯函数补测试。

### 2.4 接入 Agent 三态模式与旧用户更新提示

修改 `src-tauri/src/integrations/agent_mode.rs`：

- `Cli` / `Mcp` set：安装 rules 后 reconcile Guard；Cursor / Grok 为 no-op。
- `None` / uninstall：删除 Guard marker 拥有的 handler。
- `artifact_updates().rule`：在原 rules 缺失 / 过期之外，并入当前模式下 Guard 缺失 / 过期。
- `update_artifact(Rule)`：同时刷新规则正文与 Guard。
- `update()`：继续复用 `set(current)`，自然补装 Guard。

不把 Guard 并入现有 `Artifact::Hook`，因为该 bucket 当前服务 timeout / permission capability，且 MCP
模式没有可见的通用 Hook 行。归入 Rule 使旧用户直接在现有 Rules / Skill 行看到并完成一次更新，不需要
前端类型、i18n 或设置布局改动。

同时更新 CLI doctor / JSON 诊断，使 Claude / Codex 的聚合模式能说明 Guard 是否 installed / outdated；
面向用户的 `agents mode` 与 `agents update` 继续沿用现有输出。

IM `/new` readiness 不把 Rule/skill 正文或 Guard 过期作为硬门控；mode、Rule/skill 与实际 CLI/MCP
通道仍必须可用。集成 Tab 的更新提示保持不变。

### 2.5 模式与配置测试矩阵

增加临时 HOME 下的集成测试或模块单测，覆盖：

| 场景 | Claude | Codex | Cursor | Grok |
| --- | --- | --- | --- | --- |
| 新设 CLI / MCP | 安装 Guard | 安装 Guard + trust | 不安装 | 不安装 |
| 旧模式缺 Guard | Rule 显示需更新 | Rule 显示需更新 | 不因 Guard 报警 | 不因 Guard 报警 |
| 更新 Rule | 规则 + Guard 同时最新 | 规则 + Guard + trust 同时最新 | 只更新规则 | 只更新 skill |
| 切换 None | 只删 Guard 自有 handler | 删 handler + trust，保留其它 Hook | no-op | no-op |
| 重复安装 | 恰好一个 handler | 恰好一个 handler / trust | no-op | no-op |

另覆盖用户自有 SubagentStart handler、JSONC 注释、重复旧 marker、可执行路径变化、invalid JSON fail-safe，
确保不误删或覆盖外部配置。

## 文档同步

实现完成时更新：

- `docs/overview.md` 的 integrations 模块地图：加入 Sub Agent Guard 模块；仅此全局模块地图发生变化时修改。
- 与 Agent 集成安装产物相关的 wiki / CLI doctor 文档：说明完整协议只约束主 Agent，Claude / Codex
  带自动 Guard，Cursor / Grok 仅靠提示词。
- 若 Hook 上游版本能力变化，仅更新本 spec 的静态事实与能力矩阵，不把版本细节堆进主 overview。

## 验证顺序

实现阶段按以下顺序验证；本次计划阶段不执行：

1. `cargo fmt --check`。
2. 运行 `prompts`、`grok_skill`、`hook_edit`、`agent_subagent_guard`、`agent_mode`、Codex trust 的定向单测。
3. 运行 `src-tauri` 完整 Rust 测试；若前端没有变化，不新增前端构建要求，仍由安装脚本覆盖整体构建。
4. 按仓库规定运行 `./scripts/install.sh`，把新代码编译安装进当前环境。
5. 使用新安装的 AskHuman 做最终人工反馈确认。
6. 真实 Claude / Codex / Cursor / Grok Sub Agent 行为验证属于计费实测，默认不执行；必须先通过
   AskHuman 获得用户明确许可，并单独记录测试范围与预计调用次数。

## 完成条件

- spec 的九条验收标准全部由自动化测试、配置文件检查或用户明确批准的实测证据覆盖。
- 已安装模式的更新提示和更新动作在四家矩阵中行为正确。
- 未触碰用户自有 Hook，Codex trust 无残留或丢失。
- `docs/PROGRESS.md` 清除本任务进行中 section；完成历史留在 git。
