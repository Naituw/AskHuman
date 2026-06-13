# 开发计划：严格选择模式 + 结构化输出（`--select-only` / `--single` / `--output json`）

> 关联需求：`docs/specs/strict-choice-and-structured-output.md`
> 计划描述方案与技术 / 规则细节，具体代码以实现为准。

## 0. 方案总览

```
AskHuman -q "Q" -o "A" -o "B" --select-only --single --output json
  └─ cli/args.rs：解析 --select-only / --single / --output；校验"严格需每题有选项"
       └─ cli/mod.rs：AskArgs → 透传到 TaskRequest(unix) / AskRequest(非 unix)
            ├─ ipc TaskRequest + models AskRequest：新增 select_only / single / output_format(serde 默认)
            ├─ ShowPayload.request 透传 → GUI Helper 弹窗按 select_only/single 调整渲染
            ├─ QuestionCtx 增 select_only/single → 各 IM 渠道按之渲染（单选/严格/推荐）
            └─ 结果渲染 app/mod.rs::render_result(request, result, lang)：
                 · output_format=text → 现有区块（字段改英文、不本地化）
                 · output_format=json → 结构化 JSON（snake_case、省空字段、selected_indices…）
```

三个开关只影响**入参 / 展示 / 输出渲染**；`ChannelResult` / `QuestionAnswer` 结构与退出码不变。

---

## 1. 数据模型与协议透传

### `src-tauri/src/models.rs`
- `AskRequest` 新增（均 `#[serde(default)]`，向后兼容）：
  - `select_only: bool`
  - `single: bool`
  - `output_format: OutputFormat`
- 新增枚举 `OutputFormat { Text, Json }`（`#[serde(rename_all="lowercase")]`，`Default = Text`）。
- TS（`src/lib/types.ts`）：`AskRequest` 增 `selectOnly` / `single` / `outputFormat`（弹窗只需 `selectOnly`/`single`）。

### `src-tauri/src/ipc/mod.rs`
- `TaskRequest` 同步新增 `select_only` / `single` / `output_format`（`#[serde(default)]`）。
- `ShowPayload` 复用 `AskRequest`，无需单独加字段（随 `request` 透传给 GUI Helper）。

## 2. CLI 解析与分发

### `src-tauri/src/cli/args.rs`
- `AskArgs` 增 `select_only: bool`、`single: bool`、`output_format: OutputFormat`（解析层可用本地枚举或字符串，组装时映射到 models）。
- `parse_ask` 新增 match 分支：
  - `--select-only` → `select_only=true`（布尔，位置自由）。
  - `--single` → `single=true`。
  - `--output <v>` → 取下一个值，`text`/`json` 之外 → `Err`（本地化"不支持的输出格式"）。
- **归一化后校验**：`select_only=true` 且任一 `question.options` 为空 → `Err`（本地化"严格模式要求每个问题都有选项"）。
- 单测保持纯函数（无 IO）。

### `src-tauri/src/cli/mod.rs`
- dispatch 的「未知前导选项」allowlist（`first.starts_with('-') && !matches!(…)`）**加入** `--select-only` / `--single` / `--output`，避免以其打头被判 unknown option。
- 新增 help 分发分支：`"--scripting-help" => { print_line(&help::scripting_help_text(lang)); exit(0); }`。
- `AskArgs → TaskRequest`(unix) / `AskRequest`(非 unix) 时带上三新字段。

## 3. 结果渲染（文本字段改名 + 新增 JSON）

### `src-tauri/src/cli/output.rs` + `src-tauri/src/i18n.rs`（文本侧，D6 / D6b）
- 把 `marker.options/input/files/status` 由「本地化 pick」改为**固定英文常量**：`[selected_options]` / `[user_input]` / `[files]` / `[status]`（可在 `output.rs` 定义 const，或 i18n 中两语言返回同一英文串；倾向 const，去掉 `marker.*` 词条）。
- **合并附件字段（D6b）**：原 `[图片]` 与 `[文件]` 两块**合并为单一 `[files]`**，值为「落盘图片路径 + 透传文件路径」拼接的本地路径列表（图片/文件/目录，模型按后缀区分）；删除原 `marker.images` 词条。
- `status.cancel` / `status.unanswered` / `status.confirmContinue` 这些**值文案保持现状**（仍可本地化，属内容非字段名）。
- 文本结构（区块顺序、`# Qn`、`---`）不变；**不新增** `selected_indices`/`action`/`channel`。

### `src-tauri/src/app/mod.rs`（`render_result`）
- 签名改为接收**完整 `&AskRequest`**（现仅 `request_id`），以便 JSON 需要题序、选项原文→下标映射、以及读 `output_format`。调用点：`coordinator.rs::finish`（已有 `request`）、`emit_result`、非 unix `run_ask` 路径。
- 分支：
  - `OutputFormat::Text` → 现有 `output::aggregate_output` / `cancel_output`（字段已改名）。
  - `OutputFormat::Json` → 新增 `output::render_json(request, result)`：
    - 顶层 `action`（Send→`answer` / Cancel→`cancel`）+ `channel`（`result.source_channel_id`）。
    - `answers`：遍历 `result.answers`，**跳过空回答**（`QuestionAnswer::is_empty`）；每项：
      - `question_index`（answers 下标，对应输入题序）；
      - `selected_options`（非空才写）；
      - `selected_indices`（按 `request.questions[i].predefined_options` 原文匹配求 0 基下标，非空才写）；
      - `user_input`（非空才写）；
      - `files`（**合并字段，D6b**：落盘图片路径 + 透传文件路径拼接；非空才写）。
    - 用 serde 结构 + `#[serde(skip_serializing_if)]` 省略空字段；`serde_json::to_string_pretty`。
    - 图片落盘逻辑复用现有（JSON 用落盘后的路径并入 `files`，与文本一致）。
- `RenderOutcome.exit_code` 维持（JSON 的 answer/cancel 均 0）。

> JSON 在 Daemon 内产出（D9）；CLI `client` 仍只打印 `Final.stdout`，无需改动。

## 4. 渠道公共层

### `src-tauri/src/channels/conversation.rs`
- `QuestionCtx` 增 `select_only: bool`、`single: bool`（由 `run_conversation` 从 `request` 透传）。
- 已有 `display_text(opt, lang)`（推荐前缀）保留作 Telegram / 文本回退用；原生控件渠道（Slack/飞书/钉钉）改走各自原生推荐展示（见下），不再拼前缀。

## 5. 各 IM 渠道

### Telegram（`channels/telegram.rs`）
- `--single`：`toggle` 时若已选则清空再加（互斥单选）；键盘渲染只高亮当前一项。
- `--select-only`：`handle_event` 对 `TgInbound::Text` 直接忽略（不并入 `user_input`）。
- 无选择点「提交」：`answer_callback_query` 带 alert 文案提示需先选择（i18n）。
- 推荐展示：维持 👍 emoji 前缀（`display_text`）。

### 飞书（`feishu/card.rs` + `channels/feishu*`）
- **多选**：沿用现有 `form`（checker + input + submit）；`--select-only` → `build_form` 不加 `input` 组件。
- **单选**：新代码路径——checker **移出 form**、每项 `behaviors:[callback {action:toggle,index:i}]`；会话维护 `selected`（单值）；收到 toggle 回调 → 同步回包"更新卡片"，仅命中项 `checked`；另置一个「提交」按钮（callback `{action:submit}`）收尾。`--select-only` 下无 input；非严格单选下补充文字经 input 自身回调并入会话态（实现时确认 input 在表单外的取值时机）。
- 推荐展示：checker 增 `icon`（`standard_icon` 绿色 👍 token，**暂定**）；保留可一键回退到文字前缀的开关点。
- 终态卡片：单选 / 严格分别复刻其结构（禁用态）。

### Slack（`slack/blockkit.rs`）
- `--single`：复选框块换 `radio_buttons`（`initial_option` 取已选），解析 `state.values` 时读 `selected_option`（单值）兼容 `selected_options`（数组）。
- `--select-only`：不加 `plain_text_input` 块。
- 推荐展示：recommended 项 option `text` 用 mrkdwn 加粗、`description` = 本地化「👍 推荐」（注意 75 字符上限，超长降级）。

### 钉钉（`dingtalk/card.rs` + `channels/dingding.rs`）
- **新模板**（用户在卡片平台搭建并发布；按 §6 契约）。`DEFAULT_CARD_TEMPLATE_ID` 升级为新模板 ID。
- `build_card_param_map`：`options` 改为 `[{text, recommended}]`；新增公有变量 `single`、`allow_input`、`input_placeholder`。
- `build_card_private_map`：维持 `submitted` / `private_input`。
- `parse_card_submit`：兼容单选回传（单值或长度 1 数组）→ 归一为 `selected_options`。
- 文本回退（`dingding.rs` 编号清单）：严格忽略自由文字、单选仅接受一个编号；推荐行用 `display_text`。

## 6. 钉钉模板变量契约（交付用户搭建）

- 公有输入：`title`、`markdown`、`options=[{text,recommended}]`、`single`(bool 条件渲染 单选框/多选框)、`allow_input`(bool 条件渲染 输入框)、`input_placeholder`、`submit_status`。
- 私有：`submitted`、`private_input`。
- 提交按钮回传 `params`：`{ selected_options(数组), user_input }`。
- recommended 项：模板内联渲染彩色加粗「推荐」标记（保持"首行带标记、换行回左"；暂定文字，备选内联图片见 spec D18）。
- 单选 / 多选、输入框显隐**全部同一张模板**靠变量切换；新版统一用新模板。

## 7. 弹窗前端

### `src/views/PopupView.vue`
- 读 `request.selectOnly` / `request.single`：
  - `single`：选项渲染为 radio（单选互斥，选中集合至多一项）。
  - `selectOnly`：隐藏补充文本框与回复附件拖拽区；提交按钮在"未选中"时禁用（仍可取消 / 关窗）。
- 推荐徽标不变。
### `src/lib/types.ts`、`src/i18n/{en,zh}.ts`
- 类型加字段；新增必要的 UI 文案（如严格模式占位/禁用提示，按需）。
- 注：`HistoryDetail.vue` 与历史标签（`selectedOptions` 等 UI label）**不改**（属人面向标签，非 CLI 字段）。

## 8. help 文案与组装（`src-tauri/src/cli/help.rs`）

- `help_text`（`--help`）：重排为「Asking / Management / Help」三块（zh+en），列新参数、指向 `--agent-help` 与 `--scripting-help`（见 spec §4.1）。
- `agent_help_text`：结果字段标记改为英文 `[selected_options]`/`[user_input]`/`[files]`/`[status]`（`[files]` 含图片/文件/目录，按后缀区分）；去掉「separated by blank lines」之类多余表述，保持简洁；其余维持。
- 新增 `scripting_help_text`（zh+en）：见 spec §4.3。
- **共享片段组装**：把"调用式 / 参数说明 / 结果字段 / 示例"等抽成共享 `const` 或小函数（如 `arg_lines()`、`result_field_lines()`、`examples()`），`agent_help_text` 与 `scripting_help_text` 各取所需拼装，避免重复维护。
- `prompts.rs` **不改**（仅指向 `--agent-help`，未内嵌字段）。

## 9. i18n（`src-tauri/src/i18n.rs`）

- 删/改 `marker.*`（改名见 §3，倾向移到 `output.rs` 常量；**删除 `marker.images`**，图片并入 `[files]`）。
- 新增：Telegram 严格忽略自由文字 / 无选择 alert、单选相关提示；飞书 / Slack 严格 & 单选所需文案；CLI 错误（"不支持的输出格式"、"严格模式要求每题有选项"）。

## 10. 测试

- `cli/args.rs`：`--select-only` / `--single` / `--output` 解析；正交组合；非法 `--output` 报错；"严格无选项"报错；既有用例回归。
- `cli/output.rs`：文本字段已改英文且不本地化；`render_json` —— answer/cancel、省空字段、`selected_indices` 映射（含推荐原文匹配、重复取首个）、单选数组 ≤1、多题仅含已作答题、美化格式。
- `feishu/card.rs`：单选（callback 互斥结构）/ 严格（无 input）构卡 + 解析；推荐 icon。
- `slack/blockkit.rs`：`radio_buttons`（单选）/ 严格（无文本块）构卡 + 解析 `selected_option`；推荐 `description` + 加粗。
- `dingtalk/card.rs`：param map 含 `single`/`allow_input`/`options[{text,recommended}]`；解析单选回传归一。
- `help.rs`：`--help` 含两块与新参数；`--agent-help` 字段为英文（含合并后的 `[files]`）且不含脚本参数；`--scripting-help` 含脚本参数 / JSON / 退出码。
- 端到端（install 后手动）：各渠道单选 / 严格 / 推荐展示 + JSON 输出实测；钉钉用新模板。

## 11. 涉及文件清单

- 后端：`models.rs`、`ipc/mod.rs`、`cli/args.rs`、`cli/mod.rs`、`cli/help.rs`、`cli/output.rs`、`i18n.rs`、`app/mod.rs`（`render_result` 签名 + JSON）、`app/coordinator.rs`（调用点）、`channels/conversation.rs`、`channels/telegram.rs`、`channels/feishu.rs`(及 `feishu/card.rs`)、`channels/slack.rs`(及 `slack/blockkit.rs`)、`channels/dingding.rs`(及 `dingtalk/card.rs`)。
- 前端：`src/lib/types.ts`、`src/views/PopupView.vue`、`src/i18n/{en,zh}.ts`。
- 文档：`README`（及 `README.en.md`）使用示例补新参数；`docs/overview.md` 更新（输出契约改英文字段 + 附件字段合并 + 新参数 + scripting-help + 钉钉新模板）。
- 外部交付：钉钉新卡片模板（用户搭建发布）→ 回填 `DEFAULT_CARD_TEMPLATE_ID`。

## 12. 任务顺序（分阶段，demo 先行）

卡片若干「暂定」项（钉钉内联「推荐」、飞书单选 radio + 推荐 icon、Slack 单选 / 推荐 description、Telegram 单选高亮）**只能看真实效果才能敲定**，故先做 demo、经真实渠道发卡给用户确认，再正式编写。

### 阶段 0 — 卡片 demo & 待定项敲定（必须先于阶段 2 的渠道实现）
- [ ] 列出每个渠道在「单选 / 严格 / 推荐展示」下的预计卡片结构（落到本计划 §5 已有，按需补细节）。
- [ ] 写最小可发卡原型（demo 脚本或临时代码路径），把预计样式**经各真实渠道**发出：
  - [ ] 钉钉：搭一版新模板（§6 契约）并发卡，看单/多选条件渲染 + 内联「推荐」样式。
  - [ ] 飞书：单选真 radio（移出表单 + 回调互斥）+ checker `icon` 推荐，发卡看观感。
  - [ ] Slack：`radio_buttons` 单选 + option `description`「👍 推荐」+ 加粗，发卡看观感。
  - [ ] Telegram：单选按钮互斥高亮 + 👍 emoji，发卡看观感。
  - [ ] 弹窗：radio + 严格隐藏补充区 + 推荐徽标本地预览。
- [ ] 通过 `AskHuman` 把各渠道实拍 / 截图发给用户评审，**逐项敲定**推荐展示与单选交互（不满意则迭代：钉钉文字↔图片、飞书 icon↔文字前缀等）。
- [ ] 把最终敲定结论回填到 spec 决策表（D13–D18）并清除「暂定」。

### 阶段 1 — 数据 / CLI / 渲染骨架（可与阶段 0 并行）
- [ ] `models.rs` + `ipc` + TS 类型：新增 `select_only` / `single` / `output_format`（serde 默认）。
- [ ] `cli/args.rs` + `cli/mod.rs`：解析 / 校验（严格需每题有选项）/ allowlist / 透传 / `--scripting-help` 分发 + 单测。
- [ ] `output.rs` + `i18n.rs`：文本字段改英文 + **附件合并 `[files]`** + `render_json`；`app/mod.rs` `render_result` 改签名接 `&AskRequest` + 调用点 + 单测。
- [ ] `conversation.rs` `QuestionCtx` 透传 `select_only` / `single`。

### 阶段 2 — 正式编写各端（基于阶段 0 敲定的样式）
- [ ] 弹窗前端：radio / 严格隐藏补充区。
- [ ] 各 IM 渠道单选 / 严格 / 原生推荐（Telegram→Slack→飞书→钉钉）+ 单测。
- [ ] 钉钉新模板正式发布 + `DEFAULT_CARD_TEMPLATE_ID` 升级。
- [ ] `help.rs` 共享片段重构 + 三套 help；README / overview。

### 阶段 3 — 测试与实测验证
- [ ] `cargo test` + `npm run build` 全绿。
- [ ] `./scripts/install.sh` 安装新版二进制，用新 `AskHuman` 跑各渠道单选 / 严格 / 推荐 + JSON 输出全链路实测，并以其继续后续提问。

## 13. 风险与注意

- **render_result 改签名**：多处调用点（coordinator/emit_result/非 unix run_ask）需同步；JSON 与文本走同一图片落盘，避免重复落盘。
- **飞书单选交互模型切换**：单选移出表单走回调，与多选的表单模型并存；需处理点击延迟、3s 回包、连点 / 乱序 / 重连竞态，及非严格单选下 input 取值时机。
- **钉钉模板**：单选 / 多选 / 输入框靠条件渲染，提交回传需对单选归一；模板未升级前新参数在钉钉端无效（回退文本仍生效）。
- **文本字段改英文 + 附件合并是刻意的契约变更**：影响读取文本输出的既有 Agent（字段名变化、`[图片]`/`[文件]` 合并为 `[files]`）；`--agent-help` 同步更新可缓解。已安装的 Agent rules 不受影响（`prompts.rs` 未内嵌字段）。
- **Slack 75 字符上限**：option `text` / `description` 超长需降级（去加粗 / 截断）。
- **正交组合**：务必覆盖"单选非严格""严格但 text 输出""仅 JSON 非严格"等组合的解析与渲染。
