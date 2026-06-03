# 开发计划：AskHuman 一次提出多个问题（`-q` 多问题）

> 关联需求：`docs/specs/multi-question.md`
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

```
AskHuman "Q1" -o A -f x.png -q "Q2" -o B --no-markdown
  └─ CLI 解析：位置参数=Q1；每个 -q 起新题；-o/-f 归最近题；--no-markdown 全局
       └─ 构造 AskRequest{ id, isMarkdown(全局), questions: [Question{message,options,files}, ...] }
            ├─ GUI 弹窗：逐题展示（标题 [x/n]）；每题独立作答；全部看完→提交；一次性回传整套
            │    └─ submit_popup(answers: [QuestionAnswer, ...]) → Coordinator
            └─ Telegram：逐题串行（头部 [i/n]），答完一题发下一题；全部答完→回传整套
                 └─ run_session 收集 answers → Coordinator
       Coordinator：首个「答完全部」的 channel 整套结果生效，cancel 其余
            └─ emit_result：按 §4 输出契约格式化（单题=现状；多题=# Qn + ---）
```

核心是把「单问题」升级为「问题列表」，并把「单一回答」升级为「按题回答列表」，贯穿数据模型 → CLI 解析 → 弹窗 → Telegram → 输出。

---

## 1. 数据模型（Rust + 前端类型对齐）

`src-tauri/src/models.rs`

- 新增 `Question`（serde camelCase）：
  - `message: String`
  - `predefined_options: Vec<String>`
  - `files: Vec<FileAttachment>`（`#[serde(default)]`）
  - 说明：`is_markdown` **不**放在 Question（全局，见 C4）。
- `AskRequest` 改为：
  - `id: String`
  - `is_markdown: bool`（全局）
  - `questions: Vec<Question>`（`#[serde(default)]`）
  - 移除原 `message` / `predefined_options` / `files` 顶层字段（迁移进 `Question`）。
  - `AskRequest::new(...)`：改为接收 `questions: Vec<Question>` 与 `is_markdown`，内部生成 `id`。
- 新增 `QuestionAnswer`（serde camelCase）：
  - `selected_options: Vec<String>`（`#[serde(default)]`）
  - `user_input: Option<String>`（`#[serde(default)]`）
  - `images: Vec<ImageAttachment>`（`#[serde(default)]`）
  - `files: Vec<String>`（`#[serde(default)]`，回复文件绝对路径）
  - 提供判空方法 `is_empty()`：四者皆空（`user_input` 视为 trim 后为空亦算空）。
- `ChannelResult` 改为：
  - `action: ChannelAction`（Send/Cancel 不变）
  - `answers: Vec<QuestionAnswer>`（替代原 `selected_options/user_input/images/files`）
  - `source_channel_id: String`
  - `ChannelResult::cancel(id)`：`answers` 为空。

`src/lib/types.ts`

- 新增 `Question { message; predefinedOptions: string[]; files: FileAttachment[] }`。
- `AskRequest` 改为 `{ id; isMarkdown; questions: Question[] }`。
- 新增 `QuestionAnswer { selectedOptions: string[]; userInput: string; images: ImageAttachment[]; files: string[] }`。
- `PopupSubmission` 改为 `{ answers: QuestionAnswer[] }`。

> 兼容性：`AskRequest` 是进程内自产自销（CLI 解析 → popup_init），无需读旧磁盘格式；`#[serde(default)]` 仅作健壮性兜底。

## 2. CLI 解析与分发

`src-tauri/src/cli/args.rs`（纯逻辑，可单测）

- `AskArgs` 改为：
  - `questions: Vec<QuestionArgs>`，其中 `QuestionArgs { message: String, options: Vec<String>, files: Vec<String> }`
  - `is_markdown: bool`（全局）
- `parse_ask` 规则：
  - 维护「当前题」游标：遇到位置参数（第一个、且在任何 `-q` 之前）或 `-q <text>` 时新建一题并切换为当前题。
  - `-o`/`-f`：追加到「当前题」；**当前题不存在则报错**（C6①）。
  - 位置参数：仅当 `questions` 为空时允许作为第 1 题；否则报错「位置参数只能作为第一个问题且需在最前」（C6②）。
  - `-q`/`--question`：缺值报错（C6③）。
  - `--no-markdown`：置全局 `is_markdown=false`（可出现在任意位置，C4）。
  - 解析结束 `questions` 为空 → 报错「缺少提问内容」（C6④）。
  - 其余维持：未知 `-` 开头选项报错；`-o`/`-f` 缺值报错。
- 单测补充：
  - 单题（位置参数）= 1 题；位置参数 + `-q` = 2 题且 `-o/-f` 各归其题。
  - 多 `-q`，`-o/-f` 归属正确。
  - `--no-markdown` 全局生效。
  - 报错：`-o` 在任何题前、位置参数在 `-q` 后、`-q` 缺值、无任何题。

`src-tauri/src/cli/mod.rs`

- **dispatch 路由**：当前 `dispatch` 对「argv[1] 以 `-` 开头且非已知 flag」一律报未知选项 → 需放行 `-q`/`--question`，使 `AskHuman -q "..."` 进入提问分支（与位置参数分支统一走 `parse_ask`）。
  - 实现：把 `_ =>`（提问）分支的判定从「argv[1] 不以 `-` 开头」扩展为「argv[1] 不以 `-` 开头 **或** 为 `-q`/`--question`」；其余以 `-` 开头者仍报未知选项。
- 提问分支：对每个 `QuestionArgs` 调 `file_attachment::resolve(&q.files)` 解析其附件（任一失败 → `eprintln!` + exit 1），组装 `Vec<Question>`，再 `AskRequest::new(questions, is_markdown)` → `app::run_ask`。

## 3. 输出格式化（`src-tauri/src/cli/output.rs`）

保留并复用现有 `cancel_output()` 与 `send_output(opts, input, image_paths, file_paths)`（单题区块格式），新增聚合层：

- `pub fn aggregate_output(question_count, answers: &[RenderedAnswer], action) -> String`，其中 `RenderedAnswer` 含已落盘的 `image_paths` 与 `file_paths`（由 emit_result 预处理，见 §6）。
- 规则（对齐 §4 矩阵）：
  - `action == Cancel` → `cancel_output()`（一次）。
  - `action == Send`：
    - 定义某题「空」= 选项/输入/图片/文件皆空。
    - `question_count == 1`：
      - 非空 → `send_output(...)`（无头，现状）。
      - 空 → `[状态]\n用户未回答此问题`（无头）。
    - `question_count > 1`：
      - 全部空 → `cancel_output()`（一次）。
      - 否则 → 对每题生成「`# Q{n}` + 换行 + (非空? `send_output(...)` : `[状态]\n用户未回答此问题`)」，各题之间用 `\n\n---\n\n` 连接。
- 常量新增：`UNANSWERED_STATUS_TEXT = "用户未回答此问题"`；表头 `format!("# Q{}", n)`。
- 单测：单题非空/空；多题全答/部分未答/全未答；分隔与表头逐字校验。

## 4. 输出契约矩阵（实现依据）

| 场景 | 输出 |
|---|---|
| 单题·有回答 | `send_output(...)`（无头）|
| 单题·空回答 | `[状态]\n用户未回答此问题` |
| 多题·全答 | `# Q1\n<区块>\n\n---\n\n# Q2\n<区块>…` |
| 多题·部分未答 | 未答题为 `# Qn\n[状态]\n用户未回答此问题` |
| 多题·全未答 | `cancel_output()`（一次）|
| 主动取消 | `cancel_output()`（一次）|

## 5. 前端弹窗（`src/views/PopupView.vue` + `ipc.ts`）

`src/lib/ipc.ts`

- `popupInit`/`submitPopup` 返回/入参类型随 §1 更新；`submit_popup` 负载改为 `{ submission: { answers } }`。

`src/views/PopupView.vue` 状态重构

- 解析 `request.questions`，维护 `current: number`（当前题索引，0 始）。
- 按题保存作答：用与题数等长的数组 —
  - `chosenByQ: string[][]`、`inputByQ: string[]`、`imagesByQ: ImageAttachment[][]`、`replyFilesByQ: {path;name}[][]`。
  - 当前题的展示/编辑均读写 `*[current]`。
- `visited: Set<number>`，挂载时加入 0；切换题时加入目标索引。`allViewed = visited.size === questions.length`。
- 当前题的附件 `attachments = questions[current].files`；缩略图/拖出图标按题预取（或切题时按需取）。

展示

- 标题：`Question from {sourceName}` 右侧，多题时附 `[{current+1}/{n}]`（单题不显示）。
- 正文：按全局 `request.isMarkdown` 渲染 `questions[current].message`。
- 附件区 / 选项区 / 文本框 / 图片缩略 / 回复文件：均绑定到当前题状态。

底部布局（多题，n>1，见 U6）

- 最左：取消按钮。
- 「添加图片」移到输入框正下方（小按钮，仍触发 `pickFiles`）。
- 右侧区：
  - 未全部查看：`[上一个] [下一个]` 占据右下角；首题禁用上一个、末题禁用下一个。
  - 全部查看后：`[上一个] [下一个] [提交]`，提交出现在最右（原「下一个」位置），上一个/下一个左移。
- 单题（n=1，见 U7）：底部维持现状（添加图片 + spacer + 取消 + 发送），无导航/计数/独立提交。

交互

- 上一个/下一个：切换 `current`（夹取边界），更新 `visited`。
- 提交：把 `answers = questions.map((_,i) => ({ selectedOptions: chosenByQ[i]∩options, userInput: inputByQ[i], images: imagesByQ[i], files: replyFilesByQ[i].path }))` 经 `submitPopup` 一次性提交。
- 键盘（U8）：`⌘↵` → `allViewed ? 提交 : 下一个`；`⌘W` → 取消流程（含确认）。
- 取消确认（U9）：
  - `hasAnyAnswer` = 任一题非空。
  - 取消触发（按钮/`⌘W`/关窗）时：若 `hasAnyAnswer` → 显示**应用内二次确认**（轻量 overlay：「已有回答将丢失，确定取消？」→ 确定 / 继续作答）；确定后调用 `cancelPopup()`。否则直接 `cancelPopup()`。
  - 关窗（后端 `CloseRequested`）路径：见 §7。

> 失败兜底沿用现状：缩略图/图标/打开/预览失败仅 console，不阻断。

## 6. 后端提交命令与图片落盘

`src-tauri/src/commands.rs`

- `PopupSubmission` 改为 `{ answers: Vec<QuestionAnswer-like> }`；`submit_popup` 组装 `ChannelResult{ action: Send, answers, source_channel_id: "popup" }` 投递 Coordinator。
- `cancel_popup` 不变（投 `ChannelResult::cancel("popup")`）。
- `popup_init` 返回的 `request` 为新 `AskRequest`（含 questions）。

`src-tauri/src/app/mod.rs` `emit_result`

- 改为遍历 `result.answers`：逐题调用 `image_writer::save(&answer.images, request_id, question_index)` 落盘，得到该题 `image_paths`；连同 `answer.files`、`selected_options`、`user_input` 组成 `RenderedAnswer`。
- 调 `output::aggregate_output(answer 数量来自 request? 见下, answers_rendered, action)`。
  - **题数来源**：`emit_result` 需知道总题数以判定「单题 vs 多题」。`answers.len()` 即题数（弹窗按题数提交、Telegram 按题数收集），可直接用 `answers.len()`。
  - 取消：`action==Cancel` → 直接 `cancel_output()`，不落盘。

`src-tauri/src/cli/image_writer.rs`

- `save` 增加 per-question 命名空间，避免多题图片文件名冲突：
  - 签名增 `question_index: usize`，落盘目录改为 `temp/humaninloop/<request_id>/q{index+1}/`（或文件名前缀 `q{n}-`）。
  - 单题仍可用 `q1/` 子目录（路径变化对调用方无契约影响，仅是临时文件位置）。
- 单测相应更新（仅纯函数 sanitize/extension 不受影响）。

## 7. 协调器与关窗

`src-tauri/src/app/coordinator.rs` / `app/mod.rs`

- Coordinator 逻辑天然「会话级抢答」：每个 channel 只在「答完全部」后投递一次聚合 `ChannelResult`，首个生效、cancel 其余 —— **无需结构性改动**（仅 `emit_result` 适配 answers）。
- 关窗（`WindowEvent::CloseRequested`）：当前直接投 `cancel("popup")`。多题取消确认（U9）发生在**前端**；用户点窗口关闭按钮属系统级，仍按「取消整个会话」处理（不强行拦截原生关窗）。即：应用内取消按钮/`⌘W` 走前端确认；点红色关闭走直接取消。计划保持此区分（实现简单、语义可接受）。

## 8. Telegram（`src-tauri/src/channels/telegram.rs`）

`run_session` 改为按题循环：

- 入参 `request` 已含 `questions`、`is_markdown`、`source_name`。
- 维护 `answers: Vec<QuestionAnswer>`。
- `for (i, q) in request.questions.iter().enumerate()`：
  - 头部：`「Question from {名称}」` + （`n>1` 时）` [{i+1}/{n}]`。
  - 发送选项消息（inline 键盘按 `q.predefined_options`；markdown 走全局 `is_markdown`，逻辑同现状）。
  - 发送操作消息（含「↗️发送」reply keyboard）。
  - 发送该题 `q.files`（图片 `send_photo`、其它 `send_document`；失败 stderr 警告 + 发失败提示消息，逻辑同现状）。
  - 进入长轮询：复用 `handle_update`（toggle 选项、记录文字、点「发送」结束本题）；结束时把本题 `QuestionAnswer{selected_options, user_input(空→None), images:空, files:空}` 推入 `answers`，`offset` 继续沿用，进入下一题。
  - 每轮检查 `cancelled`（被弹窗抢答）→ 立即结束、不投递。
- 全部题结束 → `sink.submit(ChannelResult{ action: Send, answers, source_channel_id:"telegram" })`。
- `handle_update`：维持「toggle / 文本 / 发送」语义，但作用于「当前题」的 `selected`/`user_input` 局部变量（每题重置）。

> headless 路径（`app/mod.rs run_headless_telegram`）复用同一 `run_session`，自动支持多题。

## 9. 文档同步

- `src-tauri/src/cli/help.rs`：
  - `help_text`：选项段新增 `-q, --question <text>  追加一个问题（第一题可用位置参数）`；注明 `-o`/`-f` 归属最近问题、`--no-markdown` 全局。
  - `agent_help_text`：调用方式补 `-q`；新增「多问题输出」说明（`# Qn` 分组 + `---` 分隔；未答题 `用户未回答此问题`；全未答=取消提示）。
- `src-tauri/src/prompts.rs`：自动复用 `agent_help_text`，无需单独改。
- `README.md`：使用示例补多问题；输出格式补 `# Qn` / `---` / `用户未回答此问题` 说明。

## 10. 涉及文件清单

- `src-tauri/src/models.rs`：`Question` / `QuestionAnswer` / `AskRequest`(questions) / `ChannelResult`(answers)。
- `src-tauri/src/cli/args.rs`：多问题解析 + `--no-markdown` 全局 + 单测。
- `src-tauri/src/cli/mod.rs`：dispatch 放行 `-q`；逐题 resolve 附件、组装 questions。
- `src-tauri/src/cli/output.rs`：`aggregate_output` + 常量 + 单测。
- `src-tauri/src/cli/image_writer.rs`：`save` 增 per-question 命名空间。
- `src-tauri/src/commands.rs`：`PopupSubmission`(answers) / `submit_popup` / `popup_init`。
- `src-tauri/src/app/mod.rs`：`emit_result` 遍历 answers + 逐题落盘 + 聚合输出；`AskRequest::new` 调用处适配（含 settings 占位）。
- `src-tauri/src/channels/telegram.rs`：`run_session` 逐题循环。
- 前端：`src/lib/types.ts`、`src/lib/ipc.ts`、`src/views/PopupView.vue`。
- 文档：`src-tauri/src/cli/help.rs`、`README.md`。

## 11. 任务顺序

1. 数据模型（Rust `Question`/`QuestionAnswer`/`AskRequest`/`ChannelResult`；TS 类型）+ 修正所有编译引用点（settings 占位用空 questions）。
2. CLI：`args.rs` 多问题解析（含单测）+ `mod.rs` dispatch 放行 `-q` 与逐题 resolve。
3. 输出：`output.rs aggregate_output`（含单测）+ `image_writer.rs` per-question 落盘 + `emit_result` 适配。
4. 前端：`PopupView.vue` 逐题状态/导航/计数/提交时机/取消确认 + 底部新布局 + `ipc.ts`/`types.ts`。
5. Telegram：`run_session` 逐题循环 + 头部计数。
6. 文档：`help.rs` / `README`。
7. 构建（前端 `pnpm build` + `cargo build`）、`cargo test`、安装实测（单题回归 + 多题弹窗/键盘/取消确认；如配置可用则 Telegram 实测）。

## 12. 测试策略

- Rust 单测：
  - `args.rs`：单题/多题、`-o/-f` 归属、`--no-markdown` 全局、各报错分支。
  - `output.rs`：§4 矩阵全场景逐字校验。
- 手动 / 端到端：
  - 单题写法回归（无头、无计数、无导航；空回答→「用户未回答此问题」）。
  - 多题：标题 `[x/n]`、前后翻看保留作答、全部看完才出现提交、`⌘↵` 下一个/提交、`⌘W` 取消确认（有/无回答两种）。
  - 多题输出：全答 / 部分未答 / 全未答 三态。
  - 每题 `-f` 附件展示与交互；每题图片回传落盘不冲突。
  - Telegram（如配置）：逐题串行、头部 `[i/n]`、全部答完回传；与弹窗同启的整会话抢答。
  - CLI 报错分支均 exit 1 不弹窗。

## 13. 风险与注意

- **大改面**：`AskRequest`/`ChannelResult` 结构变更牵动 models/commands/coordinator/telegram/output/前端，需一次性修齐编译引用（含 `run_settings` 里 `AskRequest::new` 占位调用）。
- **图片命名冲突**：多题各自 `images` 落盘必须 per-question 命名空间，否则 `img-1.png` 互相覆盖。
- **键盘与输入冲突**：`⌘↵`（下一个/提交）、`⌘W`（取消）需与 textarea、附件区方向键/空格隔离（沿用现有 `onKeydown` 的 typing 判定）。
- **取消确认范围**：仅应用内取消/`⌘W` 走二次确认；系统级关窗仍直接取消（U9 + §7 取舍）。
- **Telegram 长流程**：多题串行使单次会话更久；被弹窗抢答时需在每轮长轮询检查 `cancelled` 及时退出。
