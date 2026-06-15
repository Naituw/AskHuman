# 需求：MCP 模式支持（Codex / Claude Code / Cursor）

> 状态：设计已与用户对齐，待审（未开工）
> 关联计划：`docs/plans/mcp.md`
> 影响面：新增 `AskHuman mcp` 子命令（STDIO MCP server）、新增 MCP 版参考提示词、新增 MCP 配置集成（三家配置文件读写）、设置「Agent」Tab 自动集成改为三态模式选择、`agents`/`doctor` CLI 子命令纳入 MCP 状态、i18n、新增依赖 `rmcp`。**不改** stdout 结果区块契约、退出码语义、daemon IPC 协议、四个 IM 渠道、弹窗与历史逻辑。

## 1. 背景与动机

参考提示词要求 AI 用 Shell 调 `AskHuman` 并把该次工具调用超时设到 24h。这条对 **Cursor / Claude Code** 可行（二者有 PreToolUse/Bash 超时 Hook 把 Shell 工具调用 timeout 抬到 24h），但对 **Codex 不可行**：Codex 没有 Shell 超时 Hook，CLI（Shell）调用超时短且无法延长，等待人类回应时会被强制取消。

调研结论（外部核实）：

- **Codex 的长超时只能靠 MCP**：Codex MCP 工具调用默认超时 `tool_timeout_sec = 60s`，但**可在 `~/.codex/config.toml` 的 `[mcp_servers.<name>]` 里调大**（如 `tool_timeout_sec = 86400`）。即「Codex 用 MCP 时超时可以很长」是指**可配置**，而非默认。
- **MCP 模式不需要超时 Hook**：超时改由 MCP 配置项（Codex `tool_timeout_sec`）控制；Cursor/Claude 的 MCP 工具调用不受 Shell 10 分钟硬上限约束（具体上限实现期复核）。
- **图片可直接返回**：MCP 协议支持 `ImageContent`(base64 + mimeType) 作为工具结果；Codex 近期版本（2025-10 起，PR #5600 等）、Claude 均能把 MCP 图片喂给模型。CLI 模式只能把图片落盘后回传路径让 AI 再读——**直返图片是 MCP 模式相对 CLI 的实质增益**。
- **Rust 官方 SDK `rmcp`**（0.16，`server` + `transport-io` + 宏）可低成本实现 STDIO server。
- **MCP（STDIO）拿不到 turn 级生命周期**：MCP server 由客户端在 **session 期间**拉起常驻，协议无 turn-start/turn-end 通知，server 只在「工具被调用」时知情。故 MCP **替代不了** lifecycle hook 的 turn 追踪。

因此本需求为三家 Agent 增加 **MCP 模式**，与现有 **CLI 模式**互斥（同一 agent 二选一）。

## 2. 目标形态

- 新增子命令 `AskHuman mcp`：以 STDIO 运行 MCP server，暴露**单个工具 `ask`**，覆盖现 CLI `AskHuman` 的全部提问能力。
- MCP server 为**薄壳**：每次 `ask` 调用就 spawn 一个现有的 `AskHuman --output json …` 子进程，复用全部既有 ask 流程（弹窗 / IM / 抢答 / 历史 / 落盘 / 排空与重连），再把人类回复中的图片读回、转 `ImageContent` 直接返回给模型。
- 自动集成：每家 Agent 改为「**CLI | MCP | 未集成**」三态互斥选择。CLI 模式绑定 `Rule + 超时 Hook`，MCP 模式绑定 `Rule + MCP 配置`。
- 手动集成：参考提示词提供 **CLI 版 / MCP 版**两份可切换展示；MCP 版同时展示各家 **MCP 配置实例**。

## 3. 已确认决策

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 模式互斥 | 每个 agent 三态「CLI / MCP / 未集成」，互斥。同一 agent 不同时安装 CLI 与 MCP 产物（避免双触发/冲突） |
| D2 | MCP server 形态 | **薄壳**：不自带提问/弹窗/IM 逻辑；每次 `ask` 调用 spawn 现有 `AskHuman --output json …` 子进程复用全流程。MCP 专属新代码仅三块：①入参 Schema↔argv 映射；②解析子进程 JSON；③图片文件→`ImageContent` |
| D3 | 启动方式 | 新增 busybox 角色子命令 `AskHuman mcp`（与 `daemon`/`--popup`/`__agent-hook` 并列），用 `rmcp` 跑 STDIO server |
| D4 | 工具与 Schema | 单工具 `ask`，入参覆盖 CLI 全部能力：`message`、`questions[{question, options[{text, recommended}]}]`、`files[]`、`markdown`、`single`、`selectOnly`（见 §5） |
| D5 | 输出（结构化 + 图片直返）| `ask` 工具**声明 output schema** 并返回**结构化 JSON**（`action`/`channel`/`answers[{questionIndex, selectedOptions, userInput?, files[]}]`）：内部子进程以 `--output json` 调用、解析后规整为 `structuredContent`（**剔除仅供脚本用的 `selectedIndices`**），并按 MCP 规范在 `content` 里附一段序列化 JSON 文本（向后兼容）。人类回复中的图片读出后以 `ImageContent`(base64+mimeType) 一并放入 `content` 数组直返模型；非图片文件以路径出现在 JSON `files` 中 |
| D6 | 超时 | MCP 模式**不需要超时 Hook**。Codex 在 `[mcp_servers.askhuman]` 写 `tool_timeout_sec`（大值）与 `startup_timeout_sec`；Cursor/Claude 依其默认（实现期复核上限） |
| D7 | MCP 配置落点 | **用户级全局**（与现有 Rules/Hook 一致）：Codex `~/.codex/config.toml`、Claude `~/.claude.json`（top-level `mcpServers`）、Cursor `~/.cursor/mcp.json` |
| D8 | 模式切换 | **一键切换**：切到另一模式时自动卸载旧模式全部产物，再安装新模式。选「未集成」= 卸载当前模式全部产物 |
| D9 | turn 生命周期 | 保持**正交**：turn 追踪仍只靠现有实验性 lifecycle hook，可与 MCP 模式并行独立开启，互不影响（MCP 拿不到 turn 周期） |
| D10 | 双版本提示词 | 新增 `prompts::mcp_reference()`：把「用 Shell 调 AskHuman、设 24h 超时、先跑 --agent-help」改为「调用 MCP 工具 `ask`」；其余交互纪律（必须提问、推荐选项、附件、结束前回执等）保留。手动集成卡支持 CLI/MCP 切换显示 |
| D11 | 自动重连 | MCP server **不持长连接**：每次 `ask` 都新起子进程→新走 `ensure_running`/排空等待/提交。daemon 因版本更新 drain/重启后，MCP server 进程继续存活，下一次 `ask` 自动连到新 daemon |
| D12 | 平台范围 | **全平台**。统一用 spawn 子进程：Unix 子进程是「瘦客户端→daemon」，Windows 子进程是现有「单进程弹窗」回退。MCP server 自身不直接弹窗，绕开 Windows 上「stdio 主循环 vs Tauri 主线程」冲突 |
| D13 | 漂移检测 | `needs_update` 覆盖 MCP Rule + MCP 配置：已安装但内置提示词/配置模板有更新时显示「更新」 |
| D14 | CLI/doctor | `agents show/install/uninstall/update` 与 `doctor` 纳入 MCP 模式状态与操作（headless 一致可用） |
| D15 | 命名 | 子命令 `mcp`；工具名 `ask`；各家配置中 server 名 `askhuman` |
| D16 | 配置 command | 配置里的 `command` 写**当前可执行文件绝对路径**（`current_exe()`，与 Hook 脚本写绝对路径一致），因部分客户端不继承 shell PATH |

## 4. MCP server 运行流程（薄壳）

```
客户端(Codex/Claude/Cursor)
  └─ 按配置 spawn: <AskHuman 绝对路径> mcp        （STDIO，session 期常驻）
       └─ rmcp STDIO server，暴露工具 `ask`
            └─ 收到 ask 调用：
                 1. 入参 Schema → argv（message / -q / -o / -o! / -f / --no-markdown / --single / --select-only / --output json）
                 2. spawn 子进程: <AskHuman 绝对路径> <argv...>
                      · Unix：瘦客户端 → daemon（弹窗/IM/抢答/历史/落盘/排空重连全复用）
                      · Windows：单进程弹窗回退
                 3. 等子进程结束，读 stdout(JSON) + exit code
                 4. 解析 JSON → 重建文本块(TextContent) + 图片文件读出转 ImageContent
                 5. 组 CallToolResult 返回
```

- **不改 daemon IPC**：子进程就是普通的一次 CLI ask，daemon 视角与现状完全一致。
- **并发**：客户端若并发调用 `ask`，各自 spawn 独立子进程，daemon 已支持并发请求（每请求独立 Coordinator）。
- **环境透传**：子进程继承 MCP server 的 env（agent 探测变量、`ASKHUMAN_ENV_SOURCE_NAME`）与 cwd（项目归类按 cwd 向上找 .git，行为同现状）。

## 5. `ask` 工具 Schema（草案）

入参（JSON Schema，camelCase）：

```jsonc
{
  "message": "string?",                 // 所有问题的共享描述（可选）
  "questions": [                         // 省略或空时：message 作为单个问题（与 CLI 归一化一致）
    {
      "question": "string",
      "options": [                       // 可选预定义选项
        { "text": "string", "recommended": false }
      ]
    }
  ],
  "files": ["string"],                  // 可选，-f 展示附件（AI→人；绝对/相对/~ 路径）
  "markdown": true,                     // 默认 true；false → --no-markdown
  "single": false,                      // 单选 → --single
  "selectOnly": false                   // 严格选择 → --select-only
}
```

argv 映射：`message`→首个位置参数（或经 `-q` 拆分）；每个 question→`-q`；option→`-o`（`recommended` 时 `-o!`）；每个 file→`-f`；`markdown:false`→`--no-markdown`；`single`→`--single`；`selectOnly`→`--select-only`；恒附 `--output json`。子进程以 argv 数组 spawn（无 shell，免引号转义）。

返回（结构化）：`ask` 声明 **output schema**，内部以 `--output json` 调子进程并解析，结果：

- `structuredContent` = 规整后的 JSON（`{action, channel, answers:[{questionIndex, selectedOptions, userInput?, files[]}]}`）。注意：子进程 `--output json` 含 `selectedIndices`（供脚本用），**MCP 输出不需要、予以剔除**；
- `content` 数组：①一段序列化 JSON 的 `TextContent`（MCP 规范要求返回结构化结果时同时给文本兜底）；②每张人类回复图片一个 `ImageContent`（从 `answers[].files` 按图片扩展名取路径、读文件 → base64 + mimeType）；
- 非图片回复文件仍以路径出现在 JSON `files` 中。

退出码/动作映射：子进程 exit 0=已作答、1=取消/未作答、3=系统错误。`ask` 工具对「取消/未作答」仍返回正常结构化结果（`action:"cancel"` 或空 answers），仅对真正的执行错误（如子进程无法启动）返回 MCP 错误。

## 6. 自动集成 UI（设置「Agent」Tab）

- 每家 Agent（Cursor / Claude Code / Codex）从「分散的 Rule/Hook 卡」改为一个**三态模式选择**：`CLI | MCP | 未集成`（互斥）。
- 选中某模式后，其下展示该模式**绑定的产物**及安装/更新状态：
  - **CLI**：CLI 版 Rule + 超时 Hook（Codex 无 Hook，仅 Rule）。
  - **MCP**：MCP 版 Rule + MCP 配置（展示落点路径、可「打开/定位」）。
- 切换模式：一键（自动卸旧装新）。选「未集成」：卸载当前模式全部产物。
- 产物内容过期（提示词/配置模板更新）时显示橙色「更新」。
- 手动集成区：参考提示词卡支持 **CLI / MCP** 切换；MCP 版附**该家 MCP 配置实例**片段。

## 7. 各家 MCP 配置写入规范（用户级全局）

均使用「最小化编辑、保留用户其它内容、解析失败即中止不覆盖」的原则（复刻现有 hook/rule 集成做法），并以托管标记识别自有条目以便幂等更新/卸载。

- **Codex**：`~/.codex/config.toml`，`toml_edit` 写
  ```toml
  [mcp_servers.askhuman]
  command = "<AskHuman 绝对路径>"
  args = ["mcp"]
  startup_timeout_sec = 30
  tool_timeout_sec = 86400
  ```
- **Cursor**：`~/.cursor/mcp.json`（与 hooks.json 不同文件），`jsonc` CST 写
  ```json
  { "mcpServers": { "askhuman": { "command": "<绝对路径>", "args": ["mcp"] } } }
  ```
- **Claude Code**：`~/.claude.json` top-level `mcpServers`（用户级），同 Cursor 形态。当前 Claude 版本已支持用户级加载（用户确认），按用户级写入，无需回退项目级。

## 8. 约束与既有规则（不可破坏）

- **不改 daemon IPC 协议与既有契约**：stdout 洁净、结果区块、退出码、配置容错全部不变。MCP 路径完全经由「spawn 现有 CLI 子进程」复用。
- **互斥安装的幂等与最小化编辑**：所有配置写入只触碰自有托管条目，保留用户其它内容；解析失败中止、不整文件覆盖（沿用 `cursor_hook`/`claude_hook`/`agent_rules` 的纯函数 + 单测做法）。
- **CLI 模式行为完全不变**：现有 Rule/Hook 安装/更新/卸载逻辑保留，仅在 UI 与 `agents` 命令层并入「模式」抽象。
- **lifecycle hook 正交**：实验性 turn 追踪独立于 CLI/MCP 模式选择，不被互斥逻辑波及。
- **跨平台**：Windows 经子进程单进程回退；MCP server 不直接持有 Tauri 主线程。

## 9. 验收标准

1. `AskHuman mcp` 启动一个 STDIO MCP server，`tools/list` 含工具 `ask`，Schema 同 §5。
2. 在 Codex 中：写入 `[mcp_servers.askhuman]`（含大 `tool_timeout_sec`）后，调用 `ask` 能弹窗/经 IM 提问、长时间等待不超时；人类回复正常返回。
3. `ask` 覆盖 CLI 能力：多问题、`options`/`recommended`、`files`、`markdown=false`、`single`、`selectOnly` 均按 CLI 语义生效。
4. 人类回复图片：模型侧收到 `ImageContent`（可见图像），非图片文件以路径出现在文本中。
5. daemon 因版本更新 drain/重启：已运行的 MCP server 不退出，下一次 `ask` 自动连到新 daemon（撞排空时等待后成功）。
6. 设置「Agent」Tab：三态模式互斥；一键切换自动卸旧装新；选「未集成」清除全部产物；产物过期显示「更新」。
7. 手动集成：CLI/MCP 提示词可切换；MCP 版显示三家配置实例。
8. `agents show/install/uninstall/update`（含 `--mcp`/模式参数）与 `doctor` 正确反映 MCP 状态；headless 可用。
9. 三家 MCP 配置写入为最小化编辑：保留用户其它条目/注释；重复安装幂等；卸载只移除自有条目；解析失败不破坏文件（单测覆盖）。
10. Windows：`AskHuman mcp` 经子进程单进程弹窗回退完成提问（无 daemon）。
11. 既有 CLI 模式（Rule/Hook）与所有现有功能回归正常。

## 10. 待实现期复核 / 开放细节

- Cursor / Claude Code 的 MCP 工具调用是否有超时上限、是否可配（Codex 已确认可配）。
- `ImageContent` 在三家客户端的实际渲染/喂模型表现（Codex 已确认 OK）。
- rmcp 声明 output schema + 返回 `structuredContent` 同时携带 `ImageContent` 的确切 API（实现期对照 rmcp 文档）。

> 已定（本轮）：`ask` 输出统一走 **JSON / 结构化 + output schema**（内部子进程 `--output json` → 解析 → `structuredContent` + 序列化 JSON 文本 + `ImageContent`）；Claude 用户级 `~/.claude.json` 当前版本支持，无需回退。

## 11. 反馈意见

- （待用户审阅后补充）
