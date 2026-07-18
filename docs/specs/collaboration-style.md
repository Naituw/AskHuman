# 协作风格（对齐 / 自主 / 自定义）

> 状态：设计定案草案（2026-07-18，待词表与实现验收）。  
> 关联：`src-tauri/src/prompts.rs`、设置「Agent」集成、rules/skill 安装与 outdated 检测。

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

## 7. 非目标（本期）

- 每 Agent 不同协作风格  
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
