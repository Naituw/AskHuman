# 开发计划：AskHuman 区分 Message 与 Question + 统一布局

> 关联需求：`docs/specs/question-overview.md`
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

把当前「问题列表」模型升级为「**一个可选的共享 Message（描述 + 附件）+ 一组 Question（实际问题，恒 ≥1）**」：

```
AskHuman "Message" -f x.png -q "Q1" -o A -q "Q2" -o B --no-markdown
  └─ CLI 解析：位置参数=Message 文本；-f→Message 附件（位置不限）；
       -q 起新题；-o 归最近题；--no-markdown 全局
       └─ 归一化：无任何 -q 时，第一个参数“提升”为唯一问题（AskHuman "X" ≡ AskHuman -q "X"）
            └─ AskRequest{ id, isMarkdown, message{text,files}, questions:[{message,predefinedOptions}...] }
                 ├─ 弹窗：顶部按需常驻 Message（text/files）；逐题切换 Question + 选项 + 输入框
                 ├─ Telegram：单题且无 Message=现状一条；否则先发 Message，再逐题串行
                 └─ 输出(stdout)：不变，按"答案数"聚合（单题无头 / 多题 # Qn + ---）
```

核心概念（**已确认**）：
- **第一个位置参数始终是 Message**（所有问题的共享描述），可带展示附件（`-f`）。
- **完全没有 `-q` 时，第一个参数等价于 `-q`**：`AskHuman "X"` ≡ `AskHuman -q "X"`，即被当作唯一的问题，`-o` 归这个（被提升的）问题、`-f` 仍归 Message。
- 内部模型统一：**Message 只含 `text` + `files`，不持有 options**；选项永远挂在 Question 上；**`questions` 恒 ≥ 1**（无 `-q` 时由第一个参数提升而来）。
- 「答案」只对应 Question，数量 = `questions.len()`；Message 不收集答案、也不进入 stdout 输出。

---

## 1. 已确认决策（实现依据）

模型 / CLI：
- D1：第一个位置参数 = **Message 文本**（可选）。`-q` 声明实际问题。
- D2：**无任何 `-q` 时，第一个参数提升为唯一问题**（`AskHuman "X"` ≡ `AskHuman -q "X"`）；此时其后的 `-o` 归该问题。
- D3：`-o` 归「最近声明的问题」；**存在 `-q` 时，`-o` 不能出现在第一个 `-q` 之前**（报错）。
- D4：`-f` **只属于 Message**，位置不限（可在任意 `-q` 之后），一律归 Message。
- D5：`--no-markdown` 全局。
- D6：**有效性校验**——至少满足下列之一才有效：① Message 文本非空；② 至少一个 `-q`；③ 至少一个 `-f`。**仅有 `-o`（无文本/`-q`/`-f`）视为无效**，报错「缺少提问内容」。

输出（stdout）：
- D7：**保持不变**。按答案数（=问题数）聚合：单答案=无 `# Qn` 头的现状区块；多答案=每题 `# Qn` + `---` 分隔；未答题=「用户未回答此问题」；全部未答=单次取消提示。Message **不进入**输出。

计数与文案：
- D8：计数用英文 **`Question {i}/{n}`**；仅当 **问题数 > 1** 时显示。

弹窗布局（单/多统一）：
- D9：**取消按钮**固定在**左下角**（单/多一致）。
- D10：**添加图片**改为「补充内容输入框**内部右下角**的小图片图标」（单/多一致），并加 **tooltip / aria-label「添加图片」**；移除底部与输入框下方的旧入口。
- D11：输入框**随文本自动增高**，设最大高度（约 `240px` / ~10 行），超出则框内滚动；**底部预留约 `36px`** 给图片图标，避免图标压住文字。
- D12：当存在共享 Message（有文本或附件）时，**顶部常驻显示 Message**（文本 + 其 `-f` 附件）；切换「上一个/下一个」只换 **Question + 选项 + 输入框**；`Question {i}/{n}` 计数显示在 **Message 下方、问题上方**（仅 >1 题时）。
- D13：单题带附件（如 `AskHuman "X" -f f.png`，无 `-q`）：附件作为 Message 显示在**问题上方**（附件在上、问题在下，已确认可接受）。**（2026-11-16 起，弹窗里这一形态已改为题干当作 Message、排在附件上面且没有「Question」小标题；见 `docs/specs/message-annotations.md` D20 与 §11。）**
- D14：保留既有多题交互——**全部问题被查看过后才出现「提交」**、`⌘↵`=未看完为「下一个」/看完为「提交」、`⌘W`/取消在有回答时弹**二次确认**；`-f` 附件为 **Message 级**（顶部常驻，不随题切换）。

Telegram：
- D15：**单题且无 Message**（`questions.len()==1` 且 Message 无文本无附件）：按**现状**单条发送（头部「Question from {名}」+ 问题文本 + 选项键盘 + 操作消息）。
- D16：**否则**（问题数 >1，或存在 Message 文本/附件）：**先发一条 Message**——头部「Question from {名}」+（Message 文本，若有）+ 随后发送 Message 的 `-f` 文件；再**逐题串行**：每题以 **`Question {i}/{n}`** 作头部（仅 >1 题时带计数）+ 问题文本 + 选项键盘 + 操作消息；点「发送」进入下一题；**问题消息不带来源头部**。全部答完才回传。

---

## 2. 数据模型（Rust + 前端类型对齐）

`src-tauri/src/models.rs`

- 新增 `MessagePrompt`（serde camelCase）：
  - `text: String`（可空）
  - `files: Vec<FileAttachment>`（`#[serde(default)]`）
  - **不含 options**。
- `AskRequest` 改为：
  - `id: String`
  - `is_markdown: bool`
  - `message: MessagePrompt`
  - `questions: Vec<Question>`（`#[serde(default)]`；**恒 ≥1**，由 CLI 归一化保证）
  - `AskRequest::new(message: MessagePrompt, questions: Vec<Question>, is_markdown)`，内部生成 `id`。
- `Question`：**移除 `files` 字段**，仅保留 `message` + `predefined_options`。
- `QuestionAnswer` / `ChannelResult` / `ChannelAction`：**不变**。

`src/lib/types.ts`

- 新增 `MessagePrompt { text: string; files: FileAttachment[] }`。
- `AskRequest` 改为 `{ id; isMarkdown; message: MessagePrompt; questions: Question[] }`。
- `Question` 改为 `{ message: string; predefinedOptions: string[] }`（去掉 `files`）。

> 兼容性：`AskRequest` 进程内自产自销（CLI 解析 → popup_init），无磁盘旧格式，`#[serde(default)]` 仅作兜底。

## 3. CLI 解析（`src-tauri/src/cli/args.rs`，纯逻辑可单测）

- `AskArgs` 改为：
  - `message_text: String`（可空）
  - `message_files: Vec<String>`（`-f` 原始路径，按出现顺序）
  - `questions: Vec<QuestionArgs>`，`QuestionArgs { message, options }`
  - `is_markdown: bool`
- `parse_ask` 规则：
  - 预扫描判定是否存在任一 `-q`（`has_q`）。
  - 位置参数：仅允许作为**第一个 token** 赋给 `message_text`；出现第二个裸位置参数 → 报错「位置参数只能作为 Message，且需在最前」。
  - `-q/--question <text>`：缺值报错；每次新建一题。
  - `-o/--option <val>`：缺值报错；`has_q` 为真——若尚未出现任何 `-q` → 报错「存在 -q 时，-o 不能出现在问题之前」，否则归最近题；`has_q` 为假 → 暂存到 `lead_options`（供归一化时挂到被提升的问题）。
  - `-f/--file <path>`：缺值报错；**始终**追加到 `message_files`（位置不限）。
  - `--no-markdown`：置全局 `is_markdown=false`。
  - 其它 `-` 开头 → 「未知选项」。
  - **有效性校验**（归一化前，对应 D6）：`message_text` 去空白为空 且 `questions` 为空 且 `message_files` 为空 → 报错「缺少提问内容」（注意：仅有 `lead_options` 不算有效）。
  - **归一化**（核心，实现 `≡ -q`）：若 `has_q` 为假，则 `questions = [QuestionArgs{ message: message_text, options: lead_options }]`，并将 `message_text` 置空（Message 退化为「仅附件/空描述」，第一个参数成为唯一问题）。这样 `questions` 恒 ≥1，且无 `-q` 时不会再额外渲染描述区。
- 单测补充：
  - 提升等价：`["X"]` 与 `["-q","X"]` 解析后 `questions==[{X,[]}]`、`message_text==""`。
  - 单题选项：`["X","-o","A"]` → `questions==[{X,[A]}]`、`message_text==""`。
  - 单题附件：`["X","-f","f.png"]` → `message_text==""`、`message_files==["f.png"]`、`questions==[{X,[]}]`。
  - 多题：`["M","-q","Q1","-o","A","-q","Q2","-o","B"]` → `message_text=="M"`、两题各自选项。
  - `-f` 在 `-q` 之后仍归 Message：`["M","-q","Q1","-f","x.png"]`。
  - 可选 Message：`["-q","Q1","-q","Q2"]`（`message_text==""`）、`["-f","x.png","-q","Q1"]`。
  - 报错：`-o` 在任一 `-q` 之前（如 `["M","-o","A","-q","Q1"]`）、第二个位置参数、`-q`/`-o`/`-f` 缺值、未知 flag、仅 `-o`（`["-o","A"]`）、空输入。

`src-tauri/src/cli/mod.rs`（dispatch）

- 提问分支 argv[1] 放行集合**保持现状**（`-q/-o/-f/--no-markdown` + 非 `-` 开头都进 `parse_ask`，由其报精确错误）。
- 解析成功后：`file_attachment::resolve(&message_files)` 解析 Message 附件（失败 `eprintln!`+exit 1）；组装 `MessagePrompt{ text: message_text, files }` 与 `Vec<Question>`；`AskRequest::new(message, questions, is_markdown)` → `app::run_ask`。

## 4. 输出与落盘（基本不变）

- `src-tauri/src/cli/output.rs`：`aggregate_output` / `send_output` / `cancel_output` / `RenderedAnswer` **不变**（按答案数聚合语义与现状完全一致；单题=1 答案无头，多题=n 答案 `# Qn`）。
- `src-tauri/src/cli/image_writer.rs`：`save(images, request_id, question_index)` **不变**（多题各题子目录，单题用 `q1/`）。
- `src-tauri/src/app/mod.rs::emit_result`：**不变**（遍历 `result.answers` 逐题落盘 + `aggregate_output`，题数取 `result.answers.len()`）。
- `run_settings` 里 `AskRequest::new(Vec::new(), false)` 调用点改为新签名（空 `MessagePrompt` + 空 `questions`）。

## 5. 前端弹窗（`src/views/PopupView.vue` + `ipc.ts`/`types.ts`）

派生状态：
- `items = request.questions`（恒 ≥1）；`count = items.length`。
- `showCounter = showNav = count > 1`。
- `showDescription`（顶部 Message 区）：`request.message.text.trim() !== "" || request.message.files.length > 0`。
- `attachments = request.message.files`。
- 按 `count` 初始化作答数组（`chosenByQ/inputByQ/imagesByQ/replyFilesByQ/visited`），逻辑同现状但作用于 `items`。

展示结构（自上而下）：
- 顶部导航栏：标题「Question from {名}」**移除原 `[x/n]` 计数**（计数下移）。
- **Message 区（仅 `showDescription`）**：按 `isMarkdown` 渲染 `message.text`（若非空）；其下渲染 `message.files` 附件区（保留缩略图/拖出/右键/预览等现有能力）。空文本则只渲染附件。
- **计数（仅 `showCounter`）**：`Question {current+1}/{count}`，位于 Message 区下方、问题上方。
- **问题区**：渲染当前 `items[current].message` 正文（按 `isMarkdown`；空文本则不渲染正文元素）+ 其 `predefinedOptions` 选项 + 输入框区。附件只在顶部 Message 区，问题区不重复。
- **输入框区（统一）**：
  - 外层 wrapper 内置「图片图标按钮」绝对定位右下角（`title`/`aria-label`「添加图片」），点击触发 `pickFiles`。
  - `textarea` 随内容自增高（`input` 时按 `scrollHeight` 调整，封顶约 `240px` 后内部滚动），`padding-bottom` 预留约 `36px` 容纳图标。
  - 其下保留：图片缩略图、回复文件 chip。

底部 footer：
- 单题（`count===1`）：**`[取消(左)] … [发送(右)]`**（不含添加图片/导航/计数）。
- 多题（`count>1`）：**`[取消(左)] … [上一个][下一个]`**，全部查看后追加 **`[提交]`**（最右）；导航与提交时机沿用现状（D14）。

交互（沿用现状，作用对象为 `items`）：
- 上一个/下一个切换 `current` 并更新 `visited`、切附件预取（附件为 Message 级，实际不随题变，可一次性预取）。
- `⌘↵`：`count>1` 未看完=下一个 / 看完=提交；`count===1`=发送。
- `⌘W`/取消按钮/关窗：有回答→二次确认 overlay，否则直接取消（系统级红色关窗仍直接取消）。
- 提交：`items.map((q,i)=>({ selectedOptions: q.predefinedOptions ∩ chosenByQ[i], userInput: inputByQ[i], images: imagesByQ[i], files: replyFilesByQ[i].path }))` 经 `submitPopup` 一次性提交。

`ipc.ts`：随类型更新（无新增命令）。

## 6. 后端命令（`src-tauri/src/commands.rs`）

- `popup_init` 返回新结构 `AskRequest`（含 `message`/`questions`），serde 自动适配；`source_name` 维持。
- `submit_popup` / `cancel_popup` / `PopupSubmission{answers}`：**不变**。

## 7. Telegram（`src-tauri/src/channels/telegram.rs`）

`run_session` 分流：
- `has_message = !request.message.text.trim().is_empty() || !request.message.files.is_empty()`；`n = request.questions.len()`。
- **若 `n==1 && !has_message`**：单题现状——把 `questions[0]` 当作唯一问题，发送「头部『Question from {名}』+ 问题文本 + 选项键盘 + 操作消息」（无计数、无 Message、无附件），收 1 个答案后回传。
- **否则**：
  1. **先发 Message**：`send_message("「Question from {名}」" + (\n\n+message.text 若非空))`；随后逐个发送 `message.files`（图片 `send_photo`/其它 `send_document`，失败 stderr 警告 + 提示消息，逻辑同现状）。
  2. **逐题循环** `for (i,q) in questions`：每题头部 `format!("Question {}/{}", i+1, n)`（仅 `n>1` 时带；`n==1` 则无头部）+ `q.message` + `q.predefined_options` 选项键盘 + 操作消息；**不带**来源头部、不发文件。长轮询复用 `handle_update`（toggle / 文本 / 「发送」结束本题）；`offset` 跨题递增；每轮检查 `cancelled`（被弹窗抢答）→ 立即返回不投递。
  3. 全部答完 → `sink.submit(ChannelResult{ Send, answers, "telegram" })`。
- 重构提示：把「发送一道题（选项消息 + 操作消息）+ 长轮询收一个答案」抽成可复用函数，单题路径与多题各题共用；头部文案在调用处区分「来源头部」与「`Question i/n` 头部」。

> headless 路径（`app/mod.rs::run_headless_telegram`）复用同一 `run_session`，自动支持新流程。

## 8. 文档同步

- `src-tauri/src/cli/help.rs`：
  - `help_text`：`<message>` 改述为「Message：所有问题的共享描述（可选；**无 -q 时即作为唯一问题**）」；`-q` 说明「声明实际问题（可多次）」；`-o` 说明「归最近问题；无 -q 时归被提升的那个问题」；`-f` 说明「仅附加在 Message 上，位置不限」。输出格式段保持。
  - **`agent_help_text`（重点更新，`--agent-help` 与「参考提示词」都依赖它）**：
    - **调用方式**：`{prog} "<Message>" [-f "<文件>" ...] [-q "<问题>" [-o "<选项>" ...] ...] [--no-markdown]`。
    - **参数说明**逐条改写：
      - `<Message>`：所有问题的共享描述（可选）；**完全不写 -q 时，它等价于 -q，即作为唯一问题**（`AskHuman "X"` ≡ `AskHuman -q "X"`）。
      - `-q, --question <text>`：声明一个实际问题（可多次）。
      - `-o, --option <text>`：为「最近声明的问题」加预定义选项；**无 -q 时归被提升的那个问题**；**存在 -q 时不能出现在第一个 -q 之前**。
      - `-f, --file <path>`：仅附加在 Message 上的展示文件/图片（**位置不限**，可放在 -q 之后），可多次。
      - `--no-markdown`：全局关闭 Markdown。
    - **用户回应**区块说明：保持（`[选择的选项]/[用户输入]/[图片]/[文件]/[状态]`）。
    - **多问题输出**说明：保持（每题 `# Qn`、`---` 分隔、未答题文案、全未答=取消、单题无头），并说明 **Message 不进入输出**。
    - **使用示例**改/补：单题 `"X" -o A -o B`；多题 `"Message" -q "Q1" -o A -q "Q2" -o B`；附件归 Message `"看看改动?" -f ./diff.patch -q "要继续吗?" -o 继续 -o 停止`；可选 Message `-q "Q1" -q "Q2"`；`--no-markdown` 示例。
- `src-tauri/src/prompts.rs`：自动复用 `agent_help_text`，**无需单独改**。
- `README.md`：使用示例与说明同步新模型（Message/Question、`≡ -q` 等价、`-o`/`-f` 归属、计数 `Question i/n`、Telegram 先发 Message 流程）；若内嵌旧 `--agent-help` 文案需一并对齐。

## 9. 涉及文件清单

- `src-tauri/src/models.rs`：`MessagePrompt{text,files}`、`AskRequest{message,questions}`、`Question` 去 `files`。
- `src-tauri/src/cli/args.rs`：新解析 + 归一化 + `lead_options`/`message_files` + 单测。
- `src-tauri/src/cli/mod.rs`：组装 `MessagePrompt` + `questions`。
- `src-tauri/src/app/mod.rs`：`run_settings` 的 `AskRequest::new` 调用点适配（`emit_result` 不变）。
- `src-tauri/src/channels/telegram.rs`：单/多题分流、先发 Message、`Question i/n` 头部、可复用「发一题收一答」函数。
- `src-tauri/src/cli/help.rs`、`README.md`：文档。
- 前端：`src/lib/types.ts`、`src/lib/ipc.ts`、`src/views/PopupView.vue`（统一布局、Message 区/计数下移、输入框内图片图标+tooltip+自增高、footer 调整）。
- 不变：`output.rs`、`image_writer.rs`、`commands.rs`（除类型透传）、`coordinator.rs`。

## 10. 任务顺序

1. 数据模型（Rust `MessagePrompt`/`AskRequest`/`Question` + TS 类型），修齐所有编译引用点（含 `run_settings` 占位）。
2. CLI：`args.rs` 新解析 + 归一化（含单测）+ `mod.rs` 组装。
3. 前端：`PopupView.vue` 派生状态、Message 区 + 计数下移、输入框内图片图标 + 自增高、footer（取消左下 / 单题发送 / 多题导航提交）、`ipc.ts`/`types.ts`。
4. Telegram：单/多题分流 + 先发 Message + `Question i/n` 头部。
5. 文档：`help.rs` / `README`。
6. 构建（`pnpm build` + `cargo build`）、`cargo test`、安装实测。

## 11. 测试策略

- Rust 单测：
  - `args.rs`：§3 全部用例（提升等价、单题选项/附件、多题、`-f` 归属、可选 Message、各报错分支含「仅 -o 无效」）。
  - `output.rs`：现有矩阵回归（语义未变）。
- 手动 / 端到端：
  - 单题 `"X" -o A`：无计数、无导航、无 Message 区；底部取消左下 + 发送右下 + 输入框内图片图标（带 tooltip）；自增高与底部留白；空回答 → 「用户未回答此问题」。
  - 单题带附件 `"X" -f f.png`：附件在问题上方（Message 区），单题 footer。（2026-11-16 起改为题干在上、附件紧随其后、没有「Question」小标题，见 D13 的注。）
  - 「Message + 1 题」`"M" -q Q1`：顶部 Message 常驻、无计数、单题 footer（发送）。
  - 多题 `"M" -q Q1 -o A -q Q2`：顶部 Message + `Question i/n` 下移显示、前后翻看保留作答、全部看完才出现提交、`⌘↵` 下一个/提交、`⌘W` 取消二次确认。
  - 可选 Message：`-q Q1 -q Q2`（无顶部 Message）、`-f x.png -q Q1`（仅附件的 Message 区）。
  - 多题输出：全答 / 部分未答 / 全未答 三态（# Qn / --- / 单次取消）。
  - Telegram（如配置）：单题且无 Message=现状一条；含 Message 或多题=先发 Message（头部/文本/文件）→ 逐题 `Question i/n` 串行、问题不带来源头部 → 全部答完回传；与弹窗同启的整会话抢答。
  - CLI 报错分支均 exit 1 不弹窗。

## 12. 风险与注意

- **结构大改**：`AskRequest`/`Question` 字段变更牵动 models/cli/commands/telegram/前端，需一次性修齐编译引用（含 `run_settings`）。
- **`-o` 归属**：需先确定 `has_q`（预扫描）——无 `-q` 时 `-o` 暂存 `lead_options` 待归一化挂到被提升的问题；有 `-q` 时 `-o` 不得在第一个 `-q` 之前。
- **归一化一致性**：归一化后 `questions` 恒 ≥1 且无 `-q` 时 `message.text` 为空；弹窗 `showDescription`、Telegram `has_message`、stdout 题数三处口径需一致。
- **输入框内图片图标**与自增高需协调：`padding-bottom` 预留区不可遮挡文字；粘贴/拖入图片路径行为不变。
- **空文本降级**：被提升的空文本问题（如仅 `-f`）、多题空 Message 等需优雅渲染（不渲染空正文元素）。
- **Telegram 抢答**：多题串行更久，每轮长轮询需检查 `cancelled` 及时退出。
