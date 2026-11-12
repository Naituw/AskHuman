# 协作风格（对齐 / 自主 / 自定义）

> 全局风格初版约定见 §1–§9。§10 的按 Agent 扩展已于 2026-11-12 在主工作树实现并安装到本机日常环境。
> 关联：`src-tauri/src/prompts.rs`、设置「Agent」集成、rules/skill 安装与 outdated 检测。

按 Agent 设置的配置、界面、应用范围与验收以 §10 为准。

## 1. 需求

用户反馈 Agent 经 AskHuman 询问过频。希望内置**协作风格**，在不破坏「必须经 AskHuman 联系人类」通道纪律的前提下，调节「问什么、问多勤」：

| 风格 | 意图 |
|---|---|
| **对齐**（默认） | 现网行为：需求对齐追问 + 改方案必确认 |
| **自主** | 合理默认、少中途确认；阻塞/不可逆/安全仍问 |
| **自定义** | 用户编辑一段替换文案（默认对齐段），自行约定策略 |

切换风格后：

1. **已开启的自动集成**全部按新风格重写可更新的 AskHuman 托管块（rules / skill 等）；  
2. **手动集成区**参考 Prompt 即时按新风格预览；  
3. **之后新开的集成**直接用新风格；  
4. **版本更新 / outdated 检查**把风格纳入指纹，旧风格正文视为需更新。

## 2. 与「接入模式」正交

| 维度 | 配置 | UI |
|---|---|---|
| 接入模式 | 每 Agent：`none` / `cli` / `mcp` | 各 Agent 卡上 |
| **协作风格** | **全局**一项（+ 自定义正文） | 集成页**顶部** |

协作风格**不**替代 CLI/MCP；生成 prompt 时：固定协议段 + 风格段；CLI/MCP/Grok 仅工具称呼不同。

## 3. Prompt 结构

```text
<mandatory_interaction_protocol>
  …通道纪律：必须经 AskHuman、选项/附件、whats-next、结束 marker、子 agent…
</mandatory_interaction_protocol>

- todo add 约定（固定）

<collaboration_style name="aligned|autonomous|custom">
  …可替换协作段落…
</collaboration_style>
```

### 3.1 固定段（所有风格相同）

- 必须用 AskHuman（CLI / MCP 对应文案）提问；禁止普通输出 / 结束回合冒充提问  
- 预定义选项 + 推荐；人类只看见经 AskHuman 送达的内容  
- **whats-next** 任务交接；结束 marker  
- 子 agent 不用 AskHuman  
- `todo add` 约定  
- 离开自动回复的优先说明（2026-11-06）：收到 AskHuman 明确的离开指引后，围绕原目标继续，常规选择无需按下方协作风格逐项确认；明确任务范围仍有效，升级前先完成回复要求的自查并提供理由。此条不修改所选风格或自定义正文，不能从超时推定离开。完整条款见[离开模式修订 §5](away-completion-guidance.md#5-交互协议新增条款)。

### 3.2 可替换段（风格决定）

对应现网 `prompts.rs` 中：

```text
- Interview me … relentlessly … shared understanding.
  - Walk down each branch…
  - If answerable from codebase, explore instead.
- Do NOT change plan/design/scope/strategy on your own; ask before changing.
```

#### 对齐（aligned）— 默认

保持上述语义（措辞可与现网一致，装入 `collaboration_style` 标记便于检测）。

#### 自主（autonomous）

要点（英文契约，实现时定稿）：

- Prefer reasonable defaults; do **not** interview relentlessly on every design branch.  
- Ask via AskHuman only when blocked, ambiguous with high blast radius, irreversible, security-sensitive, or the user explicitly asked to decide.  
- Prefer codebase exploration over asking.  
- You may adjust minor implementation details without asking; if you change the agreed plan, scope, or user-visible behavior in a material way, ask first.  
- At whats-next, briefly note key defaults you took.

#### 自定义（custom）

- 存储用户字符串 `collaboration_style_custom_text`（英文为主，面向模型）。  
- **默认值** = 对齐段正文（不含外层 XML 时便于编辑；写入 rules 时再包 `<collaboration_style name="custom">`）。  
- 用户可改可存；空串回退对齐段并在 UI 提示。  
- **不**解析用户文案是否安全；UI 提示勿删通道纪律（通道纪律不在此框内）。

## 4. 配置

```jsonc
// general（或 integrations 段，实现时二选一；推荐 general 旁独立字段便于 get_settings）
{
  "collaborationStyle": "aligned",           // aligned | autonomous | custom
  "collaborationStyleCustomText": "..."      // custom 时用；缺省/空 = 对齐正文
}
```

- `#[serde(default)]`：缺字段 → `aligned`。  
- 切换 `aligned`/`autonomous` 不要求改 custom 正文（保留用户草稿）。  
- 首次进入「自定义」：若 custom 文案为空，填入当前对齐段默认。

## 5. UI（设置 → Agent 集成）

1. **页顶**（在「自动集成 / 手动集成」之上）卡片：  
   - 标题：协作风格 / Collaboration style  
   - Segmented：**对齐 | 自主 | 自定义**  
   - 对齐/自主：下方**只读短描述**（中英文案，给人看，不是模型英文段全文）  
   - 自定义：多行输入框（等宽字体），保存按钮或失焦/debounce persist  
2. 改风格或保存自定义文案后：  
   - `persist` 配置  
   - 对所有 `mode != none` 的 Agent 跑现有「更新集成」路径（写 rules/skill）  
   - 失败按现有 modeError 展示  
   - Antigravity 的严格状态读取失败（符号链接、不可读路径）时，跳过它并向 stderr 记一条告警，不让整次应用失败；状态错误仍由 Antigravity 卡片自己显示  
   - 手动区 `prompt` 计算属性依赖风格，立即变  
3. 描述文案示例（人读）：  
   - 对齐：需求与方案多确认，理解一致后再大规模改动。  
   - 自主：少打断；仅阻塞、高风险或不可逆时询问，其余合理默认并在交接时说明。

## 6. 安装 / 更新 / outdated

1. **生成**：`cli_reference()` / `mcp_reference()` / `grok_skill_body()` 读当前配置拼 fixed + style 段。  
2. **安装/更新**：与今日相同入口，正文变为含风格的新串。  
3. **切换风格**：视为配置变更 → 对已启用集成执行 update（等同用户点「更新」）。  
4. **outdated 检测**：  
   - 现有「路径/超时/marker」检查保留；  
   - 增加：托管块内 `collaboration_style` 的 `name`（及 custom 时正文 hash 或全文）与配置不一致 → 需更新；  
   - 旧安装无 `<collaboration_style>` 包一层时，对齐风格可迁移识别「relentlessly interview」旧段为 aligned，避免无谓全员红灯；或一律标需更新一次（实现选更简单的「一律需更新」也可）。  

## 7. 初版范围

- 每 Agent 不同协作风格的后续扩展见 §10。
- 关掉 whats-next / 权限卡 / Stop 确认  
- 包揽作为第三固定档（可用自定义文案近似）  
- 云端同步自定义文案  

## 8. 测试要点

- 三档 + custom 生成串含固定协议且风格段不同  
- custom 空回退 aligned  
- 切换风格触发已装集成更新（mock）  
- outdated：风格不匹配 → true  
- MCP/CLI/Grok 工具名差异仍成立  

## 9. 实现顺序（建议）

1. `prompts.rs` 拆段 + 按配置渲染  
2. config 字段 + 默认  
3. 集成页顶 UI + persist 时更新已装  
4. outdated 指纹  
5. i18n + 单测 + wiki 一句说明  

## 10. 按 Agent 设置协作风格

### 10.1 目标与已确认决策

用户希望不同 Agent 分别使用自定义协作风格。具体场景是将另一台 Mac 上已有的完整自定义文案专供 Codex 使用，其余 Agent 使用默认风格。

2026-11-12 已通过 AskHuman 确认以下产品决策。

| 决策 | 约定 |
|---|---|
| 配置关系 | 保留默认风格，各 Agent 可跟随默认或单独设置 |
| 编辑入口 | 放在每个 Agent 自己的卡片中 |
| 编辑位置 | 位于每个 Agent 卡片内容的最后。已集成卡片中为「详情」的最后一项 |
| 折叠摘要 | 卡片收起时显示当前协作风格摘要 |
| 显示条件 | 按用户后续反馈，单家编辑入口、折叠摘要与搜索项仅随已集成且状态可读取的 Agent 显示 |

用户随后审阅本节完整设计并确认「按这版设计定稿」。协作风格继续涵盖提问频率、需求理解、独立判断、授权后的推进和汇报要求。

### 10.2 配置与解析

保留 `general.collaborationStyle` 和 `general.collaborationStyleCustomText` 作为默认值。在 `general` 下增加 `agentCollaborationStyles`，按现有 Agent ID 保存偏好。

```json
{
  "general": {
    "collaborationStyle": "aligned",
    "collaborationStyleCustomText": "",
    "agentCollaborationStyles": {
      "codex": {
        "style": "custom",
        "customText": "Act as a proactive, thoughtful collaborator..."
      }
    }
  }
}
```

示例正文为缩写，实际保存用户编辑的完整文案。

- Agent ID 沿用 `cursor`、`claude`、`codex`、`grok`、`pi`、`antigravity`。配置按 Agent 家族生效，Codex Desktop 与 CLI 使用同一份偏好。
- Agent 条目缺省，或其 `style` 缺省 / 为 `null` 时，解析为跟随默认。`style` 的显式值复用 `CollaborationStyle` 的三档。
- `customText` 可缺省。缺省表示尚未保存该 Agent 的自定义草稿，字符串表示已保存的草稿，包含用户有意保存的空串。
- 跟随默认时，从默认配置同时取得风格与正文。
- 显式选择对齐 / 自主时，使用相应内置正文。
- 显式选择自定义时，使用该 Agent 的 `customText`。缺省或纯空白沿用现有的对齐正文回退，并在编辑器旁提示实际回退结果。
- 自定义草稿独立保存。切换为对齐、自主或跟随默认后，保留该 Agent 的草稿。
- 已保存的单家自定义正文作为完整文本使用。默认正文后续的修改通过「跟随默认」关系传播。

Rust 可用 `Option<CollaborationStyle>` 表示继承，用 `Option<String>` 区分尚无草稿与已保存空串。新增映射缺省为空，旧配置自然解析为全部跟随默认。

### 10.3 界面与编辑行为

页顶现有全局卡片改名为「默认协作风格」，提供「对齐 / 自主 / 自定义」，说明为「供选择‘跟随默认’的 Agent 使用」。默认档位点击即保存并应用，自定义正文通过保存按钮提交。

各 Agent 的风格编辑区位于已集成卡片详情的最后一个功能区，动作反馈紧随该区。编辑入口、折叠摘要和对应搜索结果均以已集成且状态可读取为显示条件。

```text
Codex                                         MCP
  协作风格 · 自定义
  [详情                                      ▾]

  协议、权限、生命周期等现有集成项目
  …

  协作风格                    [自定义        ▾]
  [完整自定义正文                             ]
  [                                          ]
                                [保存并应用]
```

- 下拉选项为「跟随默认 / 对齐 / 自主 / 自定义」。跟随默认时，旁边显示解析后的风格，例如「当前为对齐」。
- 折叠摘要使用「协作风格 · 跟随默认（对齐）」或「协作风格 · 自定义」等简短文本，展示已保存状态。
- 首次进入单家自定义编辑且尚无草稿时，复制该 Agent 切换前实际使用的正文作为初始文本。之后优先恢复已保存草稿。
- 每家编辑器持有独立草稿。风格选择与正文经同一个「保存并应用」动作提交。编辑中的正文与共享已保存配置分开，其他设置的自动保存读取已保存值。
- 自定义编辑框支持完整长文，使用现有等宽文本样式与可滚动编辑区域。
- 保存失败时保留编辑草稿并展示错误。应用失败时展示配置已保存及对应集成的错误，提供重试入口。
- 设置搜索与深链能定位到风格区，并沿用卡片的展开和高亮机制。

### 10.4 保存与应用范围

保存分为默认配置和指定 Agent 配置两种作用域。一次操作使用保存前后的配置快照，比较各目标解析后的风格段，确定需应用的已集成 Agent。

| 操作 | 应用范围 |
|---|---|
| 修改默认风格 / 默认正文 | 跟随默认且生成的风格段发生变化的已集成 Agent |
| 修改单家风格 / 单家正文 | 该 Agent 在风格段发生变化且已集成时更新 |
| 保存当前未使用的自定义草稿 | 保存该草稿，待选中自定义时使用 |
| 将单家改为跟随默认 | 保存继承关系，按当前默认生成该家的提示词 |

- 保存时合并目标作用域的字段，复用配置原子落盘与集成变更锁。并发保存不同 Agent 时，应保留双方的偏好。
- 默认风格应用返回逐家结果，一家的失败与其错误独立记录，其他目标继续应用。
- 配置保存成功后作为期望状态。静态托管正文更新失败时，由现有 outdated 检测显示待更新，重试按照该期望状态执行。
- 对齐 / 自主的标识及自定义正文共同参与最终提示词比较。只改变继承关系且生成内容相同的操作，保存该关系即可。
- Antigravity 的严格文件所有权和平台可用性继续通过现有 adapter 判定，状态异常反馈在对应卡片。

### 10.5 提示词生成与各载体

在 `prompts.rs` 建立共用的风格解析入口，输入为配置快照和可选 Agent 类型。带目标的调用解析该 Agent 的偏好，通用参考解析默认风格。固定协议段与解析出的风格段共同生成最终正文。

| 载体 | 设计接入点 |
|---|---|
| Cursor、Claude、Pi 的规则 | `cli_reference_for` / `mcp_reference_for` 读取目标风格。安装、更新与 outdated 使用同一生成结果 |
| Antigravity 规则 | 由现有目标专用 adapter 使用其实际风格 |
| Grok skill | `grok_skill_body()` 使用 `AgentKind::Grok` 的 MCP 正文，再追加 Grok 联络说明 |
| Codex SessionStart | `codex_protocol()` 使用 Codex 的实际风格生成正文和 hash。同家风格的变化进入既有重复注入判定 |
| Codex 遗留 AGENTS.md 托管块 | 按既有风格刷新入口改写正文，迁移仍由显式集成更新入口执行 |
| 通用手动参考 | 使用默认风格 |
| 指定 Agent 的手动参考 / CLI 参考 | 使用所选 Agent 的实际风格和工具文案 |

Codex 的 Hook 在下一次 `startup`、`clear`、`resume`、`fork` 或 `compact` 时按现有注入规则读取配置。`resume` / `fork` 对照新正文的 hash 判断是否已经注入。运行中已加载的模型上下文通过原有会话事件获取新文案，界面应提示明确的生效时机。

规则 / skill 型 Agent 在保存后更新盘上的托管正文，界面提醒重新开启会话以确保加载。最终实测需按各家载体区分「已写入」和「已由会话读取」。

### 10.6 手动集成、向导与兼容

- 手动提示词卡新增目标选择「默认 / Cursor / Claude Code / Codex / Grok / Pi / Antigravity」。预览和复制都使用已保存的实际风格。目标可用的 CLI/MCP 组合沿用现有能力表。
- `get_prompt` 可增加可选 Agent 参数，缺省时继续返回通用默认参考。
- GUI 与文字安装向导中的风格选择作用于默认配置，并说明已有单家偏好的继承关系。应用默认变化时复用 §10.4 的目标筛选。
- 升级时沿用已有默认值，新增偏好表为空。用户之后逐家设置。
- 接入方式切换、集成更新和重新接入时，使用已保存的单家偏好。已保存的偏好与编辑草稿用于重新接入后的恢复。
- 相关新增配置通过默认字段兼容现有配置。实现入口位于配置、Tauri command 与 Agent 指令生成链路。

### 10.7 用户场景

将远端已有文案专供 Codex 使用时，先在 Codex 卡片最后选择自定义，保存完整正文，再把默认协作风格设为其余 Agent 需要的档位。Codex 使用自己的完整文案，其他跟随默认的 Agent 使用所选默认。

同一功能也覆盖 Cursor 使用自主、Claude 使用另一份自定义正文、其余跟随默认的组合。

### 10.8 实施与验证

实施涉及以下现有入口。

1. `config.rs` 与 `src/lib/types.ts` 增加单家偏好及缺省兼容。
2. `prompts.rs` 解析实际风格，接入规则、Grok skill、Codex Hook 和参考提示词。
3. 风格保存 / 应用命令按作用域写配置，返回逐家应用结果。
4. `CollaborationStyleEditor.vue` 持有各自编辑草稿，`useIntegration.ts` 管理按作用域保存与应用状态。`IntegrationTab.vue` 在各卡最后放编辑器与折叠摘要，补齐搜索、中英文案和手动预览。
5. 向导与文字流程将原风格选择明确为默认风格，并使用统一应用规则。

验证至少覆盖以下行为。

- 旧配置加载后，各 Agent 的最终提示词与升级前逐字一致。
- 单家自定义、内置风格、继承切换、空白回退与草稿恢复。
- 修改默认时，继承者更新，单独设置者按自身偏好生成。
- 修改 Codex 时，其最终正文与 hash 按新文案变化，其余目标使用各自配置生成。
- Grok skill、规则文件、Codex 旧托管块、新 Hook、手动预览与 CLI 参考的目标一致性。
- 默认设置的部分应用失败、按家重试、并发保存不同 Agent、集成状态切换的可见性和重新接入。
- 编辑草稿与其他设置的自动保存相互独立，保存后的摘要和预览反映实际持久化值。
- Agent 卡片最后的实际布局、折叠摘要、长文编辑、搜索展开与错误反馈。
- 功能实现完成后，按仓库要求运行机器健康检查、相关验证和安装，再用新安装的 AskHuman 完成交互验证。

### 10.9 实现与验证记录

2026-11-12 按定稿方案完成配置、共用解析、按作用域保存与应用、六家集成、卡片最后的编辑区与折叠摘要、独立草稿、手动预览和向导接入。切换接入状态时，编辑器实例保留未提交的草稿。接入模式的结构性识别读取固定协议段，用户正文中提及工具称呼时也能正确归类。

- Rust 全量测试 2969 passed / 11 ignored，包含私有配置下的单家更新、继承、并发保存、Codex 旧托管块与协议 hash、Grok skill、部分失败和重试。
- 前端全量 1827 passed，类型检查通过。生产代码及包含测试的严格 Clippy 均通过。
- WebKit 在 760 和 480 宽度下验证卡片最后的位置、长文编辑、单家保存及横向布局，页面错误为零。按用户界面反馈补齐六类 Agent 的未集成、已集成与读取异常可见性回归，并验证未集成卡片的实际 WebKit 布局。界面验证使用私有测试数据。
- `./scripts/install.sh` 完成构建、签名与本机日常版安装，daemon 已切换到新构建。
- 安装后用新程序和隔离配置执行 `agents show`，Codex、Cursor、Grok 的各自正文以及 Codex 切回继承均通过。
