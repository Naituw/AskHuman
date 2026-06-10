# 开发计划：推荐选项显式声明（`-o!`）

> 关联需求：`docs/specs/recommended-option.md`
> 计划描述方案与技术 / 规则细节，具体代码以实现为准。

## 0. 方案总览

```
AskHuman "Q" -o! "A" -o "B"
  └─ cli/args.rs：-o! / --option! → 选项携带 recommended=true（归属规则同 -o）
       └─ cli/mod.rs：QuestionArgs → models::Question { predefined_options: Vec<OptionItem> }
            ├─ unix：TaskRequest 经 daemon → GUI Helper（OptionItem 随 serde 透传）
            └─ 各端展示：
                 · 弹窗 / 历史详情：选项文本前 Badge（SVG 大拇指 + 本地化「推荐」）
                 · IM 渠道：显示文本 = 本地化「👍推荐 」前缀 + 原文；提交值 = 原文
```

推荐标记只影响**入参与展示**；`QuestionAnswer.selected_options`、stdout 结果区块、退出码全部不变。

---

## 1. 数据模型（`src-tauri/src/models.rs` + `src/lib/types.ts`）

- 新增 `OptionItem`：

  ```rust
  pub struct OptionItem { pub text: String, pub recommended: bool }
  ```

  - `Serialize`：恒为对象 `{ "text": …, "recommended": … }`（camelCase）。
  - `Deserialize`：**自定义实现**，接受两种形态：纯字符串（旧格式 → `recommended=false`）和对象（`recommended` 缺省 false）。实现方式：`#[serde(untagged)]` 的私有中间 enum（`String | { text, recommended }`）+ `From` 转换，附单测。
- `Question.predefined_options: Vec<String>` → `Vec<OptionItem>`；`Question::new` 签名随之调整。
- 旧负载兼容路径（全部经由上述 Deserialize 自动覆盖，无需迁移代码）：旧 `history.jsonl` 记录、新旧二进制短暂并存时的 IPC `TaskRequest`/`ShowPayload`（`ipc/mod.rs` 复用 `models::Question`，结构不动）。
- TS（`src/lib/types.ts`）：

  ```ts
  export interface OptionItem { text: string; recommended: boolean }
  // Question.predefinedOptions: OptionItem[]
  ```

  前端只接收 Rust 序列化后的对象形态，不需要字符串兼容。

## 2. CLI 解析（`src-tauri/src/cli/args.rs` + `cli/mod.rs`）

- `QuestionArgs.options: Vec<String>` → `Vec<OptArg>`（`{ text: String, recommended: bool }`，解析层私有结构，避免 args 依赖 models）。
- match 新增分支 `"-o!" | "--option!"`：行为与 `-o` 完全一致（含「有 `-q` 时不得出现在第一个 `-q` 之前」报错、`lead_options` 暂存归提升问题），仅 `recommended=true`。
- `cli/mod.rs`：
  - dispatch 的「未知前导选项」allowlist（`first.starts_with('-') && !matches!(…)`）**必须加入 `"-o!" | "--option!"`**，否则 `AskHuman -o! …` 打头会被判为未知选项（实际该形态仍会因「-o 在 -q 前」报错，但要走 parse_ask 的精确错误而非 unknown option）。
  - `QuestionArgs → models::Question` 的 map 处把 `OptArg` 转 `OptionItem`。

## 3. 渠道公共层（`src-tauri/src/channels/conversation.rs` + `src-tauri/src/i18n.rs`）

- `QuestionCtx.options: &'a [String]` → `&'a [OptionItem]`。
- Rust i18n 新增词条 `channel.recommendedPrefix`：EN `"👍Recommended "` / ZH `"👍推荐 "`（尾随空格即分隔，与原文直接拼接）。
- 提供公共小助手（位于 `conversation.rs`）：`display_text(opt, lang) -> String`（recommended 时加前缀，否则原文），各渠道统一调用。

## 4. 各渠道展示与回传

所有渠道的**提交值恒为 `opt.text` 原文**；推荐前缀只进显示文本。

- **Telegram**（`channels/telegram.rs`）：卡片正文清单行 `A. {opt}` 改为 `A. {display_text}`；inline 键盘仍只放字母、`callback_data=toggle:{i}` 按下标还原原文，不变。终态卡片正文同样用显示文本。
- **钉钉卡片**（`dingtalk/card.rs` + `channels/dingding.rs`）：
  - `build_card_param_map` 的 `options` 对象数组 `{text}` 填**显示文本**（模板不动，对已发布模板零影响）。
  - ⚠️ 钉钉模板的提交回调 `params.selected_options` 回传的是**显示文本**。在渠道层解析提交后做「显示文本 → 原文」映射：用本题 options 构建 `display → text` 查找表，逐个还原；查不到（理论不发生）原样保留。附单测。
- **钉钉文本回退**（`channels/dingding.rs` 的编号清单）：`{i}. {opt}` 改为 `{i}. {display_text}`；`parse_reply` 按编号映射原文，不变。
- **飞书卡片**（`feishu/card.rs`）：checker 的 `text.content` 用显示文本；`name=opt_{i}` 按下标还原原文，`parse_card_submit` 不变。终态卡片 `build_finalized_card` 中勾选比对（`selected.contains(...)`）必须用**原文**比对、显示仍用显示文本。飞书文本回退（如有编号清单）同钉钉处理。
- **Slack**（`slack/blockkit.rs`）：checkbox option 的 `text` 用显示文本；`value=opt_{i}` 按下标还原原文，`parse_submit` 不变。
- 各渠道函数签名中 `options: &[String]` 相应改为 `&[OptionItem]`（或在渠道入口拆成 `texts` + `displays` 两个本地 Vec，以最小化内部改动——以实现时改动面小者为准，对外行为一致）。

## 5. 弹窗与历史详情（前端）

- **`src/views/PopupView.vue`**：
  - 选项遍历改为 `opt.text` / `opt.recommended`；`chosenByQ` / `submit` 中的选中集合仍存原文字符串（`selectedOptions` 提交契约不变）。
  - 选项行结构：Badge 放在 `.label` **之前**（`check` ✓ 之后）：内联 SVG 大拇指图标（**非 emoji**）+ 本地化文字（i18n key `popup.recommended`：zh「推荐」/ en「Recommended」）。
  - Badge 样式（`PopupView.vue` scoped 或 `styles/controls.css`，与现有 `.opt-sc` 风格协调）：accent 色调 pill（`color-mix` 淡底 + accent 前景）、小号字、`flex: 0 0 auto` 不挤压选项文本、选中态下与蓝底白字兼容（选中时 Badge 转白色系）。
  - 推荐**不预选中**：不改任何初始选中逻辑。
- **`src/components/HistoryDetail.vue`**：选项遍历同步改 `opt.text`，并复用同一 Badge（样式提到 `styles/controls.css` 共享，或在两组件内重复一份——以现有「选项框复用 controls.css」的先例，放 `controls.css`）。
- **前端 i18n**（`src/i18n/en.ts` / `zh.ts`）：新增 `popup.recommended`。

## 6. 帮助与提示词

- **`src-tauri/src/cli/help.rs` `agent_help_text`**（zh + en）：
  - Invocation 行的 `-o` 部分保持简洁不动；Arguments 在 `-o` 行后新增：
    - EN：`  -o!, --option! <text> Same as -o, and marks that option as your recommended answer`
    - ZH：`  -o!, --option! <text> 同 -o，并把该选项标记为你的推荐答案`
  - Examples 第一条改为体现 `-o!`：`{prog} "Proceed with deploy?" -o! "Proceed" -o "Stop"`（zh 同形）。示例保持 `-o!` 与值之间留空格。
- **`src-tauri/src/prompts.rs` `cli_reference`**：把 `provide predefined options whenever applicable, include your recommended answer, and briefly explain your rationale` 改为 `provide predefined options whenever applicable, mark your recommended option(s) with `-o!` (instead of writing "recommended" in the option text), and briefly explain your rationale`。
  - 该文案同时是设置界面参考提示词与 Agent rules 安装内容的来源；已安装的 rules 文件需用户在设置界面重新安装才会更新（无自动迁移，不在本需求范围）。
- **README**（及 `README.en.md`）：使用示例补 `-o!`。

## 7. 测试

- `models.rs`：`OptionItem` 反序列化兼容单测（纯字符串 → `recommended=false`；对象缺 `recommended` → false；序列化恒对象）；`Question` 旧 JSON（字符串数组）反序列化通过。
- `cli/args.rs`：`-o!` 归属（提升问题 / 多题）、与 `-o` 混用顺序保持、`--option!` 等价、`-o!` 在首个 `-q` 前报错、一题多个 `-o!`；既有用例全部改用新结构断言。
- `dingtalk/card.rs` / `channels/dingding.rs`：cardData options 填显示文本；提交回调「显示文本 → 原文」映射（含未命中原样保留）。
- `feishu/card.rs`：build 卡 checker 显示文本带前缀、`parse_card_submit` 仍回原文；finalized 勾选比对用原文。
- `slack/blockkit.rs`：option text 带前缀、value 不变、parse 回原文。
- `help.rs`：agent-help 含 `-o!`。
- 端到端（install 后手动）：弹窗 Badge / 不预选中 / 提交原文；至少一个 IM 渠道实测前缀与回传。

## 8. 涉及文件清单

- `src-tauri/src/models.rs`：`OptionItem` + 兼容 Deserialize + `Question` 字段 + 单测。
- `src-tauri/src/cli/args.rs`：`OptArg` + `-o!`/`--option!` 分支 + 单测。
- `src-tauri/src/cli/mod.rs`：allowlist + `Question` 组装。
- `src-tauri/src/i18n.rs`：`channel.recommendedPrefix`（zh+en）。
- `src-tauri/src/channels/conversation.rs`：`QuestionCtx.options` 类型 + `display_text`。
- `src-tauri/src/channels/{telegram,dingding,feishu,slack}.rs`、`src-tauri/src/dingtalk/card.rs`、`src-tauri/src/feishu/card.rs`、`src-tauri/src/slack/blockkit.rs`：显示文本 + 回传原文。
- `src-tauri/src/cli/help.rs`、`src-tauri/src/prompts.rs`、`README.md`（及 `README.en.md`）。
- `src/lib/types.ts`、`src/views/PopupView.vue`、`src/components/HistoryDetail.vue`、`src/styles/controls.css`、`src/i18n/{en,zh}.ts`。

## 9. 任务顺序

1. `models.rs`：`OptionItem` + 兼容反序列化 + `Question` 改型 + 单测（先让全仓编译错误暴露所有改动点）。
2. `cli/args.rs` + `cli/mod.rs`：解析 + allowlist + 单测。
3. `conversation.rs` + `i18n.rs`：`QuestionCtx` 改型 + 前缀词条 + `display_text`。
4. 四渠道 + 卡片/blockkit 组装与回传（含钉钉显示文本→原文映射）+ 单测。
5. 前端：types + PopupView Badge + HistoryDetail + i18n + 样式。
6. `help.rs` + `prompts.rs` + README。
7. `cargo test` + `./scripts/install.sh` 编译安装，用 `-o!` 实测弹窗与渠道，并以新装 `AskHuman` 继续后续提问。

## 10. 风险与注意

- **dispatch allowlist**：漏加 `-o!`/`--option!` 会让以其打头的调用报 unknown option，而非 parse_ask 的精确错误。
- **钉钉回传显示文本**：唯一一个「模板回传显示文本」的渠道，映射还原必须在渠道层完成且有单测兜底；其余渠道均按下标还原，天然是原文。
- **历史兼容**：自定义 Deserialize 是唯一兼容点，务必覆盖「字符串数组」「对象数组」「混合数组」三种输入的单测。
- **飞书终态卡片**：勾选回显比对若误用显示文本会导致终态全不勾选（selected 是原文）。
- **前端选中集合**：继续存原文字符串，避免触碰 `selectedOptions` 提交契约与历史还原逻辑。
