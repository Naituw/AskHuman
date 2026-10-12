# Grok CLI 集成调研

> 调研日期：2026-07-03（2026-07-04 复核补充）  
> 验证版本：Grok CLI `0.2.82`，macOS arm64  
> 状态：调研结论，尚未实现  
> 复核补充（2026-07-04）：定位了 Composer 丢弃全局 `~/.grok/AGENTS.md` 的根因，并验证出一个**持久化、无 wrapper**
> 的全局 rules 入口——把 `GROK_HOME` 指向一个指向 `~/.grok` 的**软链**。详见 §4.4，根因见 §5 末。

## 1. 目标与结论

本调研回答三个问题：

1. Composer 2.5 与 Grok Build 是否使用相同的 rules、skills、hooks 和 MCP 路径；
2. AskHuman 的全局使用说明应安装到哪里；
3. 等待用户回复时，如何避免 Grok 提前终止调用。

结论如下：

- 两个模型使用不同的 agent harness。Grok Build 使用 `grok-build-plan`，Composer 2.5
  使用 Cursor 兼容的 `cursor` harness。不能假设二者共享 rules 注入行为或 MCP 工具界面。
- Grok Build 会加载 Claude 兼容的用户规则；Composer 虽能发现相关用户文件，却不会把**默认路径**下的
  `~/.grok/AGENTS.md` 渲染进最终 prompt。**根因**：Composer(cursor harness) 把 agent 文件按其所在
  `GROK_HOME` 的**字面路径**分类，凡是落在默认 `~/.grok/` 的判为 global 并丢弃，只保留项目祖先链（project）。
- 复核后修正“无持久化全局入口”的旧结论：把 `GROK_HOME` 指向一个**指向 `~/.grok` 的软链**（如
  `~/.grok_link`），该软链路径 ≠ 默认 `~/.grok`，于是 `~/.grok/AGENTS.md`（经软链同一文件）被重新判为
  project，**稳定注入 Composer 的 `<always_applied_workspace_rules>`**（已实测）。这是当前唯一无需启动
  wrapper、又持久生效于 Composer 的全局 rules 入口，但属钻“按路径判 global”的空子，升级后须回归（见 §4.4）。
- `--rules` 和 ACP `session/new._meta.rules` 能覆盖 Composer，但都要求控制启动或 session 创建。
  项目不采用启动 wrapper，因此不选这条路径。
- 两模型都能发现用户级 Grok skill。最终建议是安装一个全局 AskHuman skill，并在其中分别描述两套
  harness 的 MCP 调用方式。skill 的加载由模型匹配决定，约束力弱于 mandatory global rules，不能声称
  百分之百强制。
- AskHuman 应通过 MCP 接入。Grok 原生支持每个 MCP server 及每个 tool 的调用超时；
  `tool_timeout_sec` / `tool_timeouts.ask` 能覆盖正常存活会话中的长时间等待，不需要 timeout hook。
- hooks 在两种 harness 下共用同一个 Grok hook engine，但 PreToolUse 只能 allow/deny，不能像
  Claude/Cursor 那样改写工具输入，因此不适合延长 AskHuman 超时。

## 2. 调研方法与证据范围

本次使用了四类证据：

- Grok 自带用户文档：`~/.grok/docs/user-guide/05-configuration.md` 与
  `07-mcp-servers.md`；
- `models_cache.json`、实际 session 的 `prompt_context.json`、`chat_history.jsonl` 和
  `updates.jsonl`；
- 隔离 HOME/XAI 环境下的受控 prompt 注入测试；
- Hopper 静态分析，以及对 Mach-O `__LINKEDIT` 中残留 export trie 的恢复。

所有受控测试生成的临时 rules、配置、session 和日志均已清理。逆向结论仅适用于已验证的
`0.2.82` arm64 二进制；Grok 升级后必须重新做行为测试，不能依赖地址稳定。

## 3. 两种模型对应的 harness

本机模型缓存显示：

| 模型 | `agent_type` / harness | 主要兼容层 |
|---|---|---|
| Grok Build | `grok-build-plan` | Grok Build |
| Composer 2.5 Fast | `cursor` | Cursor |

这不只是模型名称差异。harness 决定最终 prompt 模板、rules 选择、MCP 工具暴露方式及部分内置规则。

## 4. Rules 的实测行为

### 4.1 Grok Build

Grok Build 的实际 `chat_history.jsonl` 中出现了：

- 用户级 `~/.claude/Claude.md` 中的 AskHuman 规则；
- 仓库中的 `AGENTS.md`，以 user message 的 `<system-reminder>` 形式注入。

因此 Build 当前具有 Claude 兼容行为。不过这是 harness 行为，不应外推到 Composer。

### 4.2 Composer 2.5

Composer session 的 `prompt_context.json` 能列出用户和项目层文件，包括：

- `~/.grok/Agents.md`；
- `~/.claude/Claude.md`；
- 项目祖先链上的 `Agents.md` / `AGENTS.md`。

但“发现文件”不等于“注入 prompt”。最终 `chat_history.jsonl` 只把项目祖先链文件渲染进
`<always_applied_workspace_rules>`；上述两个用户级文件没有进入实际消息。

Composer 另有五条固定 `<user_rule>`。它们来自二进制内置的
`composer_user_rules`，不是从用户目录读取。

以下候选入口均已做隔离测试，未进入 Composer 最终 prompt：

- `~/.grok/AGENTS.md`；
- `~/.grok/rules/*.md`；
- `~/.claude/Claude.md`；
- `~/.cursor/rules/*`；
- 自定义 Cursor agent profile；
- 在隔离的 `XAI_ROOT` / `XAI_USER` 目录中放置上述文件。

自定义 profile 还会被模型要求的 `cursor` harness 重新解析覆盖，不能作为稳定入口。

### 4.3 可用但未采用的动态入口

`grok --rules <text>` 已实测会原样进入 Composer system message 的 `<human_rules>`。
ACP 的 `session/new._meta.rules` 属于相同类别。

这两个入口都有效，但要求 AskHuman 控制 Grok 启动命令或 ACP session 创建。若提供 wrapper，可以在每次
启动时注入 mandatory rules；当前产品决策是不引入 wrapper，因此将其作为已知替代方案保留，不纳入实现。

### 4.4 GROK_HOME 路径别名：持久化的 Composer 全局 rules 入口（2026-07-04 复核）

这是复核后新确认、且**无需启动 wrapper**的入口，修正了 §1 旧结论。

**现象（同一份 `~/.grok/AGENTS.md`，仅改 `GROK_HOME` 指向，CWD 均为全新临时 git 仓库，模型
`grok-composer-2.5-fast`）**：

| `GROK_HOME` 取值 | Composer 是否注入全局 rule |
|---|---|
| 未设置（默认 `~/.grok`） | 否（`always_applied_workspace_rules`=0） |
| `$HOME/.grok`（显式指到默认目录本身） | 否 |
| `$HOME/.grok_link`（**软链 → `~/.grok`**） | **是**（进 `<always_applied_workspace_rules>`） |
| 任意非默认真实路径的副本 | 是 |

**判据非其它变量**：已逐一排除 `GROK_HOME` 是否设置、`sessions`/`logs`/`projects`/`worktrees.db` 等
runtime 目录、`downloads`（内含真实二进制，`bin/grok` 是指向它的软链）、以及插件数量（软链与真目录均为
39 plugins / 14 hooks）。唯一决定注入与否的是 **`GROK_HOME` 字面路径是否等于默认 `~/.grok`**。

**证据链**：会话落盘的 `prompt_context.json` 中，即便是 Composer（`"system_prompt":"cursor"`），
`agents_md_files` **确实包含** `~/.grok/Agents.md`（采集阶段未丢）；但最终 `chat_history.jsonl` 里没有它。
二进制内 agent 文件结构只有 `file_name/file_path/content`，**无 scope 字段** → global/project 是渲染时按
路径现算并在 cursor 模板处丢弃 global（见 §5 末）。软链让路径 ≠ 默认 `~/.grok`，绕过该丢弃。

**用法（`.zshrc`，软链与真目录同 inode，插件/登录/会话全共享，可随时撤销）**：

```zsh
ln -sfn "$HOME/.grok" "$HOME/.grok_link"   # 建一次即可（放 .zshrc 亦幂等无害）
export GROK_HOME="$HOME/.grok_link"
```

之后 `~/.grok/AGENTS.md`（= `~/.grok_link/AGENTS.md`）的规则即进入 Composer。

**风险**：本质是钻“按 `GROK_HOME` 字面路径判 global”的空子（可视作 Grok 的一个 bug）。若后续版本对
`GROK_HOME` 做路径规范化（canonicalize），软链会被解析回 `~/.grok` 而失效；届时回退到 §4.3 的
`grok --rules "$(cat …)"` wrapper。每次 Grok 升级都应重跑本表回归。

## 5. Hopper 与二进制逆向结果

Grok CLI 是原生 Rust Mach-O 可执行文件，不是 JavaScript 包装应用。Hopper 初次分析尚未完成时，先从
Mach-O 的符号结构和残留 export trie 定位关键函数，随后结合汇编与 session 行为交叉验证。

在 Grok CLI `0.2.82` arm64 中恢复到以下私有函数地址：

| 地址 | 恢复名称 | 作用 |
|---|---|---|
| `0x101d016f0` | `composer_user_rules` | 构造 Composer 固定的五条用户规则 |
| `0x101d01888` | `xai_monorepo_user_dir` | 读取 XAI 内部用户目录环境 |
| `0x101d01a20` | `resolve_monorepo_user_dir` | 解析 XAI monorepo 用户目录 |
| `0x101d01d94` | `cursor_user_template` | 渲染 Cursor/Composer user message |
| `0x101d01ee4` | `find_agent_files` | 搜索 agent 文件 |
| `0x101d02030` | `find_rules_files` | 搜索 rules 文件 |

从 ARM64 XOR 解码循环恢复出的 `cursor_user_template` 长 4467 字节。模板明确区分：

- `workspace_rules` → `<always_applied_workspace_rules>`；
- `user_rules` → `<user_rules>`。

实际 session 与模板一致：项目祖先链 agent 文件进入 `workspace_rules`，而 Composer 的
`user_rules` 由固定内置规则提供。`prompt_context` 中列出某个用户文件，并不能证明它被选择为
`user_rules`。

`xai_monorepo_user_dir` 通过 `std::env::_var_os` 读取精确的 `XAI_ROOT` 和 `XAI_USER`，
join 后检查目标是否为目录。隔离实验表明，即使该目录存在并包含各种 rules 文件，也没有成为 Composer
可用的全局 user-rules 入口。该逻辑更像 xAI 内部 monorepo 支持，不应作为公开集成接口。

### 5.1 global/project 分类与 §4.4 的根因（2026-07-04 复核）

结合 §4.4 的行为差分与 session 落盘产物，可确定 Composer 丢弃全局 `~/.grok/AGENTS.md` 的机制：

- 相关模块：`xai-grok-agent/src/prompt/agents_md.rs`、`xai-grok-tools/src/implementations/cursor_rules_on_read.rs`，
  以及已恢复的 `find_agent_files` / `find_rules_files` / `cursor_user_template`（§5 表）。
- agent 文件在采集阶段不带 scope（`prompt_context.json` 的 `agents_md_files` 只有
  `file_name/file_path/content`）；global vs project 是在渲染 `cursor_user_template` 时按 `file_path`
  是否位于当前 `GROK_HOME`（默认 `~/.grok`）**字面路径**下现算。
- 判为 global 者不进入 `<always_applied_workspace_rules>`（Composer 只渲染 project 祖先链），因此默认
  `~/.grok/AGENTS.md` 对 Composer 失效；而 Grok Build(`grok-build-plan` harness) 会照常加载 global。
- 该分类只比字面路径、不做规范化，故 §4.4 的软链（路径 ≠ 默认 `~/.grok`）令同一文件被判为 project 而注入。
  该结论来自行为差分 + 产物比对；未逐指令定位比较分支（二进制已 strip 符号、Rust 字符串合并成 blob 使字面
  子串 xref 不可靠），升级后应以行为回归为准。

## 6. Skills 方案

### 6.1 安装位置与能力

用户级 Grok skills 位于：

```text
~/.grok/skills/<skill-name>/SKILL.md
```

两种模型的 skill 列表均能出现用户级 skill，因此它是当前无需 wrapper、又能覆盖两个 harness 的共同入口。

建议 skill 至少包含：

- 当需要提问、澄清、审核、批准或等待用户决定时，使用 AskHuman MCP；
- `ask` 是阻塞式人机交互，必须等待返回，不得因耗时而自行假设；
- 有适用选项时提供 options，并标记推荐项及简短理由；
- 不要通过普通回复代替 AskHuman；
- 对 Composer 与 Build 分别写明工具发现和调用流程。

skill 的名称和 description 应包含 `AskHuman`、human input、question、approval、clarification 等强触发词，
以提高模型在第一次需要提问前加载它的概率。

### 6.2 约束边界

skill 不是 always-on mandatory rule。模型必须先根据 skill metadata 判断它与当前任务相关，之后才会读取
`SKILL.md`。因此：

- 它适合提供跨 harness 的操作说明；
- 它不能严格保证每一次提问都经过 AskHuman；
- 不应在 UI 或文档中将其描述为与 Claude/Codex 全局 rules 等价的强制策略；
- Grok 更新后应分别用 Composer 与 Build 做“首次需要澄清时是否主动加载 skill”的回归测试。

如果未来产品要求强制执行，仍需重新考虑启动 wrapper、ACP rules，或等待 Grok 提供持久化 global
user-rules 接口。

### 6.3 复核（2026-07-04）：description 触发弱 → skill 重定位为「无条件必读的交互协议」

首版把 skill 写成「AskHuman 提问技能」，description 用「当你需要 input / 决策 / 批准 / 审核时使用」这类
**条件触发词**，实测发现模型基本不主动加载。根因分析：

- **自指悖论（结构性、最核心）**：skill 是懒加载 + 相关性门控，为**任务能力型** skill 设计（匹配任务内容）；
  而「向人提问」对模型是**内建行为、不存在能力缺口**。按「需要提问时加载」写，模型在最需要该 skill 的时刻
  （默认不问人、直接输出结束）恰恰意识不到需要它。
- **时机悖论**：最重的条款「结束回合前必须 ask」生效于回合收尾——模型已进「总结交付」态，最不会去翻 skill 列表。
- **description 措辞弱点**：触发词描述的是模型内部状态而非可观察任务事件；只写「何时用」不写「为何必须用」；
  缺主动性/强制信号词与事件锚点。

**处置（已实施）**：把 skill 从「提问技能」**重定位为「无条件必读的交互协议」**——

- frontmatter `description` **第一句无条件要求「每个 session 先读本 skill」**（不设「需要提问才读」这类条件），
  消解自指悖论；
- 前置一条兜底事实「**普通输出人类不可见，只能经 `ask` MCP 工具送达**」——即便「必读」被忽略，这句仍可能在
  提问/收尾时刻触发加载；
- 把情境锚点（asking / decision / assumption / approval / review / plan-summary-result / ending turn）并入同句。

**为何全写进 `description`、不用 `when-to-use`（实测）**：Grok skill frontmatter 的 `when-to-use` 字段，实测
（grok 0.2.82，Composer 与 Grok Build 落盘 prompt 比对）**仅以 `Use when:` 标签紧跟在 `description` 之后、拼进
同一段常驻文本**，无独立注入位置或可观察的独立匹配增益。对「无条件必读」策略（靠的就是这段常驻可见文本）而言
拆分无价值，故合并为单一 `description`。

**注意**：这些改动只**提高**首次加载概率，**不改变 skill 弱约束的本质**——仍不能保证每轮强制。真正的 always-on
只能靠 rules 级通道（§4.3/§4.4，均需版本回归或侵入 shell），或等 Grok 提供持久化 global user-rules 接口。

### 6.4 复核（2026-07-04）：Grok hook 无法注入 always-on 上下文（排除该路线）

评估过「用 hook 在每个 session 注入协议上下文」作为绕开 skill 门控的 always-on 通道，实测 + 文档 + 二进制三路
证据一致**否定**：

- **实测（决定性）**：隔离 `GROK_HOME` + 全新 git 仓库，`SessionStart` / `UserPromptSubmit` hook 分别 echo 含
  sentinel 的纯文本、以及 Claude 风格 `{"hookSpecificOutput":{"additionalContext":...},"systemMessage":...}` JSON；
  问模型上下文里是否有 sentinel，**Composer 与 Grok Build 均答「无」**，而 `updates.jsonl` 确认 hook `status=success`
  （执行了但输出不进上下文）。
- **文档**：`10-hooks.md` 明说被动 hook（`SessionStart` / `PostToolUse` 等）「**stdout is ignored**」；只有
  `PreToolUse` 能 allow/deny，无任何事件支持注入上下文。
- **二进制**：strings 中**不存在** `additionalContext` / `hookSpecificOutput`；hook 输出只解析 `decision` / `reason`
  （`systemMessage` 仅出现在 hub 转发 envelope，不进模型）。
- 顺带确认 `Stop` hook 亦为被动，不能像 Claude 那样 block stop 并把 reason 喂回模型 → 「结束回合前必须 ask」无法靠
  hook 兜底强制。

结论：Grok 0.2.82 **无任何 hook 通道能把内容注入模型上下文**；always-on 仅剩 §4.3/§4.4 的 rules 级入口。

### 6.5 宿主隔离（2026-10-13）：避免 Cursor 误加载 Grok 的 MCP 协议

Cursor 3.24.12 启用第三方扩展兼容时，会扫描用户及项目的 `.grok/skills/`。真实 Cursor 会话
`a2a346c7-a679-412f-99eb-bcf82b451f68` 首先读取了本 Skill，随后在 Cursor CLI 集成下反复搜索
并不存在的 AskHuman MCP 工具，最终才改用本地 CLI。这是不同宿主之间的协议交叉加载，与所选
模型是否为 Grok 无关。

受管 Skill 的 frontmatter 增加字符串元数据：

```yaml
metadata:
  surfaces: grok
```

Cursor 当前加载器接受字符串或数组形式的 `metadata.surfaces`，在加入 Rule 和 Skill 清单前
过滤不匹配当前 `ide` / `cli` surface 的文件。Grok 1.0.50 将 metadata 作为普通元数据；隔离
`GROK_HOME` 下的 `grok inspect --json` 已确认增加该字段后仍能发现 `interaction-protocol`。
使用字符串符合 Grok 文档的 metadata 字段格式，不增加依赖或关闭任何宿主的第三方兼容开关。

只增加元数据，description 与协议正文保持原样，不添加宿主说明。旧受管 Skill 沿用内容比较
显示需更新，更新后写入新 frontmatter。已读入旧协议的会话上下文不会被撤回，验收使用新会话。
该过滤依赖 Cursor 当前实现，升级后须复核其 surface 判断；静态加载验证与 Grok inspect 不
替代真实模型会话验收。本轮不发起付费模型请求。

安装验证：两个既有 Grok Skill / 共享协议测试通过，Rust 格式及 diff 检查通过；已运行
`./scripts/install.sh` 并通过新安装的 `AskHuman agents update grok` 更新本机 Skill。
更新前后逐字节比较确认只有上述元数据新增，description 与正文不变；使用 Cursor 安装包的
实际 surface 过滤函数验证 `ide` / `cli` 均排除该 Skill，Grok inspect 仍返回原生 Skill 及
相同 description。其它 Cursor / Grok 配置文件已核对内容哈希一致，模式仍为 Cursor CLI、Grok MCP。

反向核对：本机 Grok 1.0.50 的 `inspect` 仍只发现原生 Grok 的 AskHuman Skill，当前 Cursor 的
`~/.cursor/rules/askhuman.mdc` 未进入其指令清单。这不代表所有 Cursor 兼容规则都被隔离：
Grok 仍默认扫描 Cursor skills 和 `*.md` 规则。将来迁移 Cursor 规则载体时，仍需按
`docs/investigations/cursor-global-rule-user-skill.md` 单独验证反向兼容性。

## 7. MCP 在两种 harness 下的差异

两种模型最终都通过 Grok 的 MCP client manager 调用 server，但暴露给模型的工具界面不同。

### 7.1 Grok Build

实际 prompt 要求模型：

1. 用 `search_tool` 搜索 MCP 工具；
2. 用 `use_tool` 调用找到的工具。

AskHuman skill 应明确让模型先查找 AskHuman 的 `ask`，再通过 `use_tool` 调用。

### 7.2 Composer 2.5

Cursor harness 将 MCP schema 暴露为工具描述文件，并提供 `CallMcpTool`。模型需要：

1. 读取 AskHuman `ask` 对应的工具描述；
2. 按 schema 组装参数；
3. 用 `CallMcpTool` 执行。

现有通用 `mcp_reference()` 假设 `ask` 是直接可见工具，不足以准确指导 Grok 的工具发现路径。

**最终实现（2026-07-04 复核修正上文）**：skill 正文 = `mcp_reference()` **原样复用** + 末尾追加一小段
Grok 说明（单一来源，避免协议措辞在两处漂移；见 `prompts::grok_skill_body`）。追加段**刻意保持通用、
不写死具体 harness / 工具名**（不提 `search_tool` / `use_tool` / `CallMcpTool` / Composer / Grok Build——
这些名字与机制会随 Grok 版本变化，写死会过时误导），只声明一条「联系人类」的降级阶梯：① 优先 MCP `ask` 工具
（P2：MCP 优先于 shell/CLI，仅限「联系人类」，不禁止一般 shell）；② 若 `ask` 未列在当前可用工具里，先用工具
搜索/发现机制找到；③ 仍够不到 MCP 时，**退回其它可用提问渠道**（如 CLI 版 `AskHuman` 命令），**绝不**把给人类
的内容写进普通输出（人类看不见）或直接结束回合。上文 §7.1/§7.2 的具体机制仅作背景事实保留，不进 skill 文案。

## 8. 超时机制

### 8.1 MCP tool timeout

Grok 官方本地文档给出的配置为：

```toml
[mcp_servers.askhuman]
command = "/absolute/path/to/AskHuman"
args = ["mcp"]
enabled = true
startup_timeout_sec = 30
tool_timeout_sec = 86400
tool_timeouts = { ask = 86400 }
```

语义：

- `startup_timeout_sec`：MCP server 启动及握手超时，默认 30 秒；
- `tool_timeout_sec`：该 server 单次工具调用的 fallback timeout，单位秒，默认 6000 秒；
- `tool_timeouts.ask`：`ask` 工具的单独覆盖，单位秒。

`tool_timeout_sec` 与 `tool_timeouts.ask` 同时写在 24 小时场景中有一定冗余，但能同时表达 server 默认值和
AskHuman 特例，且避免未来增加其他工具后语义不清。若只希望 `ask` 为长超时，可以保留默认 fallback，
只写 `tool_timeouts = { ask = 86400 }`。

Composer 的 `CallMcpTool` 与 Build 的 `use_tool` 最终都走同一 MCP manager，因此同一份 server 配置能
覆盖两个模型，没有发现第二层、模型专属的 MCP timeout 配置。

### 8.2 “完全控制”的准确边界

这些字段控制的是 **Grok MCP client 等待一次 tool call 的 deadline**。只要 Grok session、Grok
进程和 AskHuman MCP server 都正常存活，它足以避免 AskHuman 因默认工具超时被中断。

它不能保证以下情况：

- 用户主动取消当前 turn；
- 用户关闭或杀死 Grok；
- Grok 崩溃；
- AskHuman MCP 子进程或 daemon 异常退出；
- 操作系统终止任一进程。

文档没有提供“无限等待”的特殊值。因此 86400 秒是可配置的 24 小时有限等待，不是永久等待；如产品需要
跨天，可使用更大的有限秒数。`startup_timeout_sec` 只解决冷启动，不会延长用户回复等待。

### 8.3 CLI 模式

若未来仍支持通过 shell 执行 `AskHuman`，Grok 的前台命令超时由以下配置控制：

```toml
[toolset.bash]
timeout_secs = 86400.0
```

它会放宽所有前台 shell 命令，而非只影响 AskHuman，作用域明显大于 MCP per-tool timeout。因此当前建议
只支持 MCP 模式。

## 9. Hooks 调研

### 9.1 两模型共用 hook engine

在 Composer 与 Build 的实际 `updates.jsonl` 中，以下 hook 都成功执行：

- `SessionStart`；
- `UserPromptSubmit`；
- `Stop`。

Grok 原生用户级 hooks 的入口为：

```text
~/.grok/hooks/*.json
```

因此生命周期追踪可以在未来作为独立功能评估；它不解决 rules 或 AskHuman 等待超时。

### 9.2 不能复用现有 timeout hook

Grok PreToolUse hook 输出只支持：

```json
{"decision":"allow"}
```

或：

```json
{"decision":"deny"}
```

没有验证到 Claude/Cursor 的 `updatedInput` 或 `hookSpecificOutput` 输入改写能力。被动 stdout 也不会修改
tool call。

另外，Grok hook payload 使用 `toolInput`，现有 Claude/Cursor timeout scripts 读取的是
`tool_input`。即使兼容字段名，缺少输入改写能力仍使该方案不可行。

hook 自身还有独立的短进程 timeout（默认约 5 至 10 秒，具体 hook 类型/配置不同）。它只约束 hook
脚本执行时间，应该保持短小，不能用来等待用户回复。

## 10. 推荐的产品集成

当前建议的 Grok 集成模式只有一个：

```text
Grok MCP mode
├── ~/.grok/config.toml
│   └── [mcp_servers.askhuman]
│       ├── AskHuman mcp command
│       ├── startup_timeout_sec
│       ├── tool_timeout_sec = 86400
│       └── tool_timeouts.ask = 86400
└── ~/.grok/skills/interaction-protocol/SKILL.md
    ├── 通用 AskHuman 交互约定
    ├── Build: search_tool → use_tool
    └── Composer: descriptor → CallMcpTool
```

不建议：

- 把**默认路径**下的 `~/.grok/AGENTS.md` 当成对 Composer 生效的全局 rules（默认它只对 Grok Build 生效；
  要覆盖 Composer 须用 §4.4 的 `GROK_HOME` 软链别名，或 §4.3 的 `--rules` wrapper，且都需版本回归）；
- 复用 Claude/Cursor timeout hook；
- 为 AskHuman 单独放宽全部 bash 命令；
- 在没有版本回归测试的情况下依赖逆向得到的私有函数或 XAI 环境变量；
- 对用户承诺 skill 具有 mandatory rules 的强制性。

## 11. 后续实现与验证清单

若进入实现，至少需要：

1. 在 agent 类型和 UI 中增加 Grok，当前只提供 `MCP | 未集成`；
2. 用保留注释/格式的 TOML 编辑器最小修改 `~/.grok/config.toml`；
3. 安装、更新、卸载并检查 `~/.grok/skills/interaction-protocol/SKILL.md`；
4. 产物状态分别显示 Skill 与 MCP Config，支持单项更新；
5. 不把 Grok hook 计入 MCP mode 的必要产物；
6. 分别用 Composer 2.5 与 Grok Build 验证：
   - 新 session 能发现 AskHuman skill；
   - 首次出现澄清/批准需求时会加载 skill；
   - 两种 MCP 工具路径都能成功调用 `ask`；
   - 等待超过 6000 秒时仍不触发 Grok 默认 timeout；
   - 用户取消 turn 和 MCP server 异常时能正确收敛；
7. Grok CLI 版本变化后重复 rules 注入、skill 触发、MCP 路由和 timeout 回归测试。

## 12. 反馈意见（2026-07-04，计划评审）

> 实现计划见 `docs/plans/grok-integration.md`；以下为评审中对「Grok 兼容读取 Claude/Cursor 的坑」的定案，
> 已同步进计划 §6.2。

- **Q1 指令载体**：只装 Skill（`~/.grok/skills/interaction-protocol/SKILL.md`，2026-07-04 由 `askhuman/` 重命名，
  并把 skill 重定位为「无条件必读的交互协议」，见 §6.3），不写 `~/.grok/AGENTS.md`（Composer 不读）。
- **Q2 模式态**：Grok 只做 `None | Mcp` 两态（Composer 的 CLI 会自动后台化，不可靠，不提供 Cli 档）。
- **Q3 生命周期**：本轮一并做（`AgentKind::Grok` + 原生 `~/.grok/hooks`）。
- **Q4 MCP 超时**：`startup_timeout_sec=30` + `tool_timeout_sec=86400` + `tool_timeouts={ask=86400}`。
- **Q5 入口**：GUI 卡片 + CLI + doctor 全套。
- **P1（生命周期错标）**：只用 `report.rs` 去重——`running==Grok` 时凡 `intended!=Grok` 一律跳过；**不**改动
  用户 `[compat.*]`。
- **P2（指令交叉污染）**：不改用户配置，靠强化 skill 正文。**措辞要点（用户明确）**：一般 shell 调用照常可用，
  **仅在调用 AskHuman 的 `ask` 时声明「MCP 优先于 shell」**，不得禁止普通 shell 用法。
- **P3（MCP 来源重复）**：视为无害（同名 `askhuman` server，grok 按名去重），仅文档记一笔，不特殊处理。

