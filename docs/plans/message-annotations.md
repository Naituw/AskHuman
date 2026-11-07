# 计划：消息附注（在消息正文里选字写附注，随回答一起发给 Agent）

> 需求：`docs/specs/message-annotations.md`（决策编号 D1–D26、E1–E4、N1–N13 均指其 §3；本计划的 P0–P6 是实施阶段，与需求里的编号无关）。状态：2026-11-07 起草，同日获用户批准（审批轮的回答与据此的改动见需求 §11 反馈意见：否决 N15 并删除 N14、不验 iOS 17、在主工作树做）。
> 分支 / 工作方式：主工作树（`main`），见「开工前定下的事」。每一期单独验收、单独提交（D26）。
> 文件名、函数名是起草时读代码得出的；开工时以代码为准，差异记在文末「实施记录」。

## 1. 方案摘要（脱离需求文档也能读懂）

### 1.1 做什么

弹窗（macOS / Windows / Linux）和 iOS 上，选中 Message 正文里的文字，就地写一条附注；提交回答时，附注连同被选中的原文一起交给 Agent。用户读正文时有疑问当场写下，不必读完凭记忆重述（D1）。附注不是回答，Agent 自行判断怎么对待（D4）。

### 1.2 哪些文字可以附注（D2、D3、N1、N2、N9）

- **Message 的文字**：弹窗里渲染后的 Markdown、纯文本、「查看源码」下的源码文字都可以（D16）；iOS 上只有 Markdown 且网页渲染成功的 Message 可以（N9）。
- **直接提问的题干**：没有 Message（既无文字也无附件）、恰好一个问题（带不带预选项都算）、且不是 What's next 时，这个问题的题干。命令行里 `AskHuman "长文" -o A -o B` 就是这种形态。弹窗里它本来就放在最上面、不带「问题」标题，版面不动；iOS 上把题干搬进上方的 Message 区（D19、D20）。
- **不可附注**：多题提问、或题目与 Message 并存时的题干；选项文字、附件、代码块的复制按钮、Mermaid 图（源码与图都不算）；What's next 的固定问句（它的 Message 可以）；确认卡（权限 / Stop / 出错）、`--select-only` 严格选择、IM 渠道；出错卡 / 复查卡、Codex 原生问题、开发中继。其中除 IM 外都靠请求级开关 `annotatable = false` 表达（E2）。

### 1.3 Agent 收到什么（D5、D7、D8、D9、D23、N6、N10、N11）

- **新区块 `[user_annotations]`**（标记恒英文、不本地化）：每条是一行 `> 原文` 加附注正文，条与条之间空一行。文本区块里原文超过 300 字时中间截成「…」（前 149 字 + 「…」 + 后 150 字，恰好 300 字；已经是 300 字以内不动）。
- **位置**：单题放在其余区块（`[selected_options]`、`[user_input]`、`[files]`）之后，空一行；多题放在最后一题之后，用 `---` 隔开，不带 `# Qn`；只写了附注（什么都没选没写）时只输出这个区块，不是「未作答」，也不是「取消」。
- **`--output json`**：顶层多一个 `user_annotations` 数组，每项 `{ "quote": "…", "note": "…" }`，原文是保存下来的原文（≤ 2000 字，不再缩短）；没有附注时不出现；只有附注时 `action` 为 `answer`、没有 `answers`；不带 `question_index`（N6）。
- **What's next**（只有文本输出）：「派活」「建议」在原输出（含 `[files]`）之后追加区块；选了「结束本轮」、或什么都没选又写了附注时**不批准结束**——输出里没有「结束本轮」那行，只有 `[files]`（如有）和附注区块，由 Agent 回应后再次 `--whats-next`（D5）。取消从不带附注。
- **说明保持简洁**（D9）：`--agent-help` 的结果区块清单里只多一行，MCP 工具描述只多一句，不新增参数，不讲用法。
- **回看**：`AskHuman --show-last`（自己的缩进写法：每次往来末尾多一段 `user_annotations:`，原文同样缩到 300 字）、弹窗历史窗口（含「复制为文本」）、iOS 历史详情都带上附注（D23）。控制台对话记录把附注作为回答文字的第三段原样显示，不做独立卡片（N11）。Claude 内置提问（AskUserQuestion）的接管：附注并进该题的答案文字，只有附注时放进 `deny` 的理由（N10）。

### 1.4 弹窗里怎么用（D10–D16、N3、N5、N7）

- 选中文字（必须完全落在可附注的正文里、不是空白，N7），选区旁浮出小按钮「添加附注」；点它才展开输入浮层，因为选字也常用来复制（D10）。选区碰到已有附注时，按钮变成「编辑该附注」，附注之间不重叠（D14）；已满 20 条时按钮置灰并说明上限。
- 输入浮层在选区下方：被选原文在上、输入框在下；添加键跟随「发送快捷键」设置（默认 ⌘/Ctrl+Enter；设成「Enter 发送」时 Enter 添加、带修饰键的 Enter 换行）；输入法组词中的 Enter 不提交；Esc 只关浮层；打开期间弹窗其余快捷键暂停（Enter 不会误提交答案，⌘/Ctrl+W 照常关窗）；被选的那段用虚线框标出（D11）。
- 已加附注的文字有紫色下划线底色和编号小圆点（紫色避开查找的黄色与选中的蓝色）；点高亮打开同一个浮层（编辑模式，可改可删）（D12）。
- 页脚「附注 N」小按钮在「快捷回复」与「发送」之间（单题与多题页脚都有），有附注才出现；点开清单：每行编号 + 原文摘要 + 附注，点一条定位到原文并闪一下，鼠标悬停出现编辑 / 删除；键盘：↑↓ 移动、Enter 定位（同点击）、E 编辑、Delete / Backspace 删除、Esc 关闭；清单与浮层对弹窗其余快捷键模态（做法同快捷回复菜单）；不新增全局快捷键（D13、N5）。
- 附注按在正文里的位置编号并输出（最靠前是 1），新增 / 删除后重排；找不到位置的排在最后（N3）。
- 「查看源码」下也能加附注；高亮按原文匹配，匹配不到就不显示高亮，清单里仍在，提交照常发出（D16）。原文取所见文字：不含 Markdown 符号，换行与连续空白压成一个空格；选区中间夹着复制按钮或 Mermaid 图时，它们的文字不算原文（N4、N7）。
- 与页内查找共存：查找的黄色标记与附注的紫色高亮互不清除。

### 1.5 iPhone 上怎么用（D17–D21、N9）

- Message 仍是网页（`WKWebView`）渲染：长按选字，系统编辑菜单里多一项「添加附注」（选区碰到已有附注时是「编辑附注」）；空选区、长按链接或图片时不出现。实现靠子类化 `WKWebView` 覆写 `buildMenu(with:)`，**P1 先验证，不行就停下问用户，不擅自换方案**（D17）。
- 点菜单项：底部小面板，被选原文在上、输入框在下，键盘常驻；原文高亮带编号，点高亮可再编辑（D18）。
- 底栏按钮行里第三个圆钮「附注」，与「发送」同高，有附注才出现，紫色数字角标；顺序 [快捷回复][附注][下一题][发送]；点开清单：点一条回到原文并闪一下，左滑编辑 / 删除（D18、D21）。
- 窄屏沿用现有渐进降级（G12 / G13：计数缩写、计数独占一行、圆钮随字号变大）。需求 §4.3 的宽度是估算，落地后用 `AskHumanBarUITests` 在 375 pt、五档字号、中英文下实测；**不达标就回来找用户，不自己换方案**（D21）。
- 附注随草稿保存，离开页面再回来还在（D6）。非 Markdown 消息与网页没起来时的原生降级渲染没有新增入口，但草稿里已有的附注仍保留、仍在清单里、仍随回答发出（N9）。Mac 太旧（没有 `Caps.annotations`）时没有入口（D24）。
- 没有 Message（无文字、无附件）、只有一个问题、且不是 What's next 时，题干一律显示在上方的 Message 区（网页渲染），不论长短，选项卡里只剩选项与输入框（D19）。**没有别的条件**：不看 Mac 有没有 `Caps.annotations`、是不是 Markdown、是不是严格选择——用户在审批轮明确不要条件规则，这一形态的布局与以前不同是他接受的代价；What's next 的固定问句不搬（N1）。非 Markdown、网页没起来时题干在 Message 卡片里是原生文字；不可附注时（老 Mac、严格选择、非 Markdown 等）题干同样在上方，只是系统菜单里没有「添加附注」。代价：Markdown 的这类提问也要起一个网页进程，用户已接受。

### 1.6 附注与回答的关系（D4、D5、D6、D15、N8）

- 只写了附注、其余什么都没填也能提交，**不加任何提示**，不改变其余答案的含义，由 Agent 自行判断。
- 附注不占「已回答 N/M」，不影响发送按钮对回答的要求；但算「有内容」：弹窗的取消 / ⌘W / 关窗二次确认、共享窗口里的未答标记、手机的草稿标记都认它。
- 弹窗点取消 = 丢弃附注；手机附注随草稿保存。任何提交方式（发送、快捷回复、挑待办、带指令的发送……）都带上当时已写的附注。
- What's next 里选「结束本轮」又写了附注：不批准结束，只把附注交给 Agent。

### 1.7 上限与清洗（D22、E3）

- 每个请求最多 20 条；每条附注最多 2000 字；原文最多 2000 字（取选区时就中间截断：前 999 字 + 「…」 + 后 1000 字）。字数按字符计，不是字节。
- 清洗只在 `Coordinator::try_submit` 一处做（所有渠道的回答在这里决出胜负、存下结果），之后渲染、历史、`--show-last` 看到的是同一份：不是「发送」、或请求不可附注时整体丢弃；去掉原文或附注为空（去空白后）的项；原文的换行与连续空白压成一个空格、去首尾空白，超过 2000 字就中间截断；附注超过 2000 字就截到 2000 字；超过 20 条只留前 20 条。清洗是幂等的。Daemon 不重排：发送方按正文位置排好序（N3）。
- 文本区块里的 300 字缩短在渲染时做，不动保存下来的原文。

### 1.8 协议与兼容（E1、E2、D24）

- 全是可选字段，缺省为空 / false，为空 / false 不序列化：`UserAnnotation { quote, note }`、`ChannelResult.annotations`、IPC `ClientMsg::Answer.annotations`、`PopupSubmission.annotations`、`HistoryEntry.annotations`、手机 `AnswerSubmit.annotations` 与本地历史条目的 `annotations`；请求级 `AskRequest.annotatable` 与手机 `AskView.annotatable`（缺省 false）。**`PROTOCOL_VERSION`（现为 5）与 iOS `PROTO` 都不动**——这些全是带缺省值的可选字段，老端缺字段按空 / false 读，同一个协议号内保持互通（`docs/overview.md`「IPC 协议号」）。
- 能力位 `Caps.annotations` 说明这台 Mac 收附注（手机据此决定要不要出现入口）；`annotatable` 说明这条提问可以带附注：= Agent 自己的提问（CLI、MCP、Claude 内置提问接管，与快捷回复菜单同一个判定 `away::is_agent_ask`）且不是严格选择。手机两者都满足才出现入口，弹窗只看 `annotatable`。
- `answer_hash`（手机重发幂等）只在附注非空时才把它算进哈希：不带附注的回答哈希与旧版逐字相同，旧发件箱里的重发不会被判成冲突（做法同 `todoIds`）。

### 1.9 我加的默认做法（用户没有单独确认过；原 N15 被用户否决、N14 随之删除，见需求 §11）

N1 What's next 的固定问句不挪、不可附注，它的 Message 可以。N2 出错卡 / 复查卡、Codex 原生问题与开发中继不开放（`annotatable = false`）。N3 按正文位置编号并输出。N4 原文取所见文字、压成一行。N5 弹窗键盘如 §1.4。N6 JSON 不带 `question_index`。N7 选区必须完全落在附注根里才出现入口。N8 未发送的附注算「有内容」。N9 手机上非 Markdown 与降级渲染不提供入口。N10 Claude 内置提问接管把附注并进答案文字。N11 控制台对话记录认识新区块。N12 重复提问重放的是第一次渲染好的输出，不另处理。N13 Daemon 对附注内容原样转交，Agent 读到的和 `[user_input]` 一样是不可信的人类输入。

## 2. 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 需求、计划、界面稿与浏览器原型入库（只改文档与 `src/prototype/`、`docs/assets/`，本次已写好） | `vue-tsc --noEmit` |
| P1 | iOS 选字菜单小验证（一次性工程，不入库）：长按选字 → 系统菜单里的「添加附注」 | UI 测试 + 截图，iOS 18.5 / 26.2 / 27.0 / 27.1；任一版本不行就停下问用户 |
| P2 | Mac 协议与输出：数据模型、汇聚点清洗、文本 / JSON / What's next 输出、CLI help 与 MCP 描述、历史与 `--show-last`、控制台对话记录、Claude 接管、IPC 与弹窗提交载荷、iOS 网关的 Mac 侧 | `cargo test`、`cargo clippy -D warnings`、`cargo fmt --check` |
| P3 | 弹窗：附注引擎、选字入口、输入浮层、高亮、页脚清单、历史窗口 | `vitest`、`vue-tsc`、浏览器里点真实组件、安装后在真实弹窗里点；Windows / Linux 见 §6.4 |
| P4 | iOS 协议与 Message：P4a 协议、草稿、发送（Swift 包）；P4b 网页引擎与选字菜单；P4c 回答页界面（编辑面板、底栏圆钮、清单、历史） | `swift test`、`xcodebuild test`（英文与 zh-Hans）、真实 daemon 端到端、窄屏实测 |
| P5 | iOS 直接提问：题干并入 Message 区 | 模拟器 UI 测试 |
| P6 | 文档同步（需求 §8）、`PROGRESS.md` | 提交 |

P1 先于 P2 做：它只决定 P4 的路是否走得通，早知道比晚知道好；P2 到 P6 按 D26 的顺序做，期与期之间除 P4 依赖 P1 与 P3 的引擎外互不阻塞。

## 3. P0：文档与原型入库

工作区里已有（未提交）：需求 `docs/specs/message-annotations.md`、本计划、`docs/development.md` 里两条原型说明、`src/prototype/` 六个文件（`popup-annotations.html`、`popup-annotations-main.ts`、`AnnotationsFrame.vue`、`annotations.css`、`annotationsMock.ts`、`ios-annotations.html`）、`docs/assets/message-annotations/` 十张图。批准后用显式路径分两次提交：`docs(prototype): add message annotation prototypes`（原型、图片、`docs/development.md`）与 `docs(annotations): add spec and plan for message annotations`。主工作树里还有别的会话留下的未提交改动（iOS 开发者模式、onboarding、Diagnostics），提交一律列路径，不用 `git add -A`。

## 4. P1：iOS 选字菜单小验证

**为什么先做**：整个 iOS 方案押在「`WKWebView` 子类覆写 `buildMenu(with:)` 能把一项加进长按选字的系统编辑菜单」上（D17）。这一点在文档里查不到可靠结论，必须在各系统版本的模拟器上实测。

**做法**：在 `/tmp` 下建一次性的 SwiftUI 小工程（xcodegen，不进仓库），页面用真实的 `ios/AskHuman/Resources/WebMarkdown`（`index.html`、`render.js`、`style.css`、`markdown-it.min.js` 拷过去），渲染一段含正文、链接、图片、代码块的 Markdown。`SpikeWebView: WKWebView` 覆写 `buildMenu(with:)`，当页面（`selectionchange`，经脚本消息）报告有非空选区时，在编辑菜单里插一项「添加附注」（选区落在一条模拟的已有附注上时是「编辑附注」）。

**逐项检查**（全部写成 UI 测试断言 + 截图存 `/tmp`）：

1. 长按一个词，系统编辑菜单里出现「添加附注」；点它，原生拿到页面最近一次报告的选区（原文与偏移）。
2. 空选区、长按链接、长按图片时菜单里没有这一项。
3. 菜单项标题按页面最近一次报告的选区状态在「添加附注」与「编辑附注」之间切换。
4. 放进 SwiftUI 的 `UIViewRepresentable`、`isScrollEnabled = false`、高度由内容撑开、外面套 `ScrollView`（与 `WebMarkdownView` 同构）时依然成立。
5. 点菜单项后弹出 SwiftUI `.sheet`，输入框能拿到键盘；收起 sheet 后原生能通过脚本清掉页面选区。
6. 在本机已装的 iOS 18.5、26.2、27.0、27.1 模拟器上各跑一遍。

**结果**：全部通过，就把结论（含菜单项插入的位置，如 `.standardEdit` 起首）与四个系统版本的截图路径记进实施记录，继续 P4；**任何一个版本不行，立刻停下，用 AskHuman 把现象与可选办法交给用户定，不擅自换方案**（D17）。本机没有 iOS 17 运行时，用户决定不验 iOS 17。

## 5. P2：Mac 协议与输出

### 5.1 数据与汇聚点清洗

- `models.rs`：`UserAnnotation { quote, note }`；`ChannelResult.annotations`（`#[serde(default, skip_serializing_if = "Vec::is_empty")]`）；`AskRequest.annotatable`（`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`，与 `whats_next` 同写法，`AskRequest::new` 里为 false）。`ChannelResult` 的构造点分散在 daemon、iOS 网关、待办、渠道会话、`cli/output.rs`、开发中继等十余处，加字段后按编译器提示逐个补 `annotations: Vec::new()`（`ChannelResult::cancel` 同）。
- 新文件 `src-tauri/src/annotations.rs`：`sanitize`（1.7 的清洗规则，纯函数）、`middle_truncate`（按字符中间截断，幂等）、`quote_for_text(&str)`（300 字缩短）、文本区块渲染 `render_block`、`show-last` 用的缩进渲染。数字常量 20 / 2000 / 2000 / 300 放在这里。
- `daemon/request.rs::create_escalated`：`request.annotatable = quick_reply && !task.select_only`（`quick_reply` 已有，`away::is_agent_ask(&task)`）。
- `app/coordinator.rs::try_submit`：在处理待办交付（`apply_todo_deliveries`）之后、写入 `self.result` 之前，用 `sanitize` 处理 `result.annotations`（规则见 1.7）。

### 5.2 输出

- `cli/output.rs`：`aggregate_output(lang, answers, annotations)`：单题与多题按 1.3 的位置规则追加区块；全部未答而有附注时只输出区块（不走「未作答」与 `cancel_output`）。`render_json` 加顶层 `user_annotations`（缺省省略，原文不缩短）。`whats_next_reply` 加变体 `NotesOnly`：发送路径、且有附注、且回答是「结束本轮」或空时用它；`whats_next_output` 对 `Task` / `Suggested` 在 `[files]` 之后追加区块，对 `NotesOnly` 输出 `[files]`（如有）与区块、没有「结束本轮」那行；取消不带附注。`append_status` 追加的指令说明仍然接在最后。
- `app/mod.rs::render_result` 与 `render_whats_next`：把 `result.annotations` 传进上面的函数。
- `cli/help.rs::result_field_lines` 加一行（英文：`[user_annotations]  The user's notes on passages of your message: a quoted passage (> …) followed by the note`，中文版同义）；`mcp/ask.rs` 的 `ask` 与 `whats_next` 工具描述各加一句，不加参数。`agent_help_markers_match_output` 要求 help 里的区块标记都出现在输出里，随之更新。

### 5.3 回看与消费方

- `history.rs`：`HistoryEntry.annotations`（缺省为空、为空不序列化），`record_history` 从清洗后的结果拷入；`show_last.rs` 在每次往来末尾加 `user_annotations:` 段；`commands.rs` 的历史接口随条目原样带出。
- `agents/transcript_full.rs`：识别 `[user_annotations]`，附注行不再并进 `[user_input]`，作为回答文字的第三段（N11）。
- `ask_question.rs`（Claude 内置提问接管）：单题的附注并进答案文字（后接 `[user_annotations]` 区块）；只有附注时放进 `deny` 的理由（新增 i18n `askQuestion.notesOnly` 加区块）；带 `directive` 的路径里区块跟在已答列表之后、状态说明之前。
- `ipc/mod.rs`：`ClientMsg::Answer` 加可选的 `annotations`（不抬 `PROTOCOL_VERSION`）；`commands.rs::submit_popup` 的 `PopupSubmission` 加 `annotations`，共享弹窗走 `inbox.rs`（约 L459）、独立弹窗走 `GuiBridge::send_answer`，helper 路径走 `daemon/runtime/mod.rs`（约 L3305–3325），三条都把附注带进 `ChannelResult`。

### 5.4 iOS 网关的 Mac 侧

- `ios/wire.rs`：`Caps.annotations`（缺省 false，与 `answer_todos` 同写法）；`AskView.annotatable`（false 不序列化）；`AnswerSubmit.annotations: Vec<UserAnnotation>`（可选字段）。`daemon/runtime/ios.rs`（约 L72）与 `ios/gateway/testkit.rs`（约 L187）设 `annotations: true`；`ios/views.rs` 从 `AskRequest.annotatable` 填 `AskView.annotatable`。
- `ios/gateway/requests.rs`：`try_answer` 把 `submit.annotations` 放进 `ChannelResult`；`answer_hash` 只在附注非空时才算进哈希（1.8）。清洗不在网关做，统一交给 `try_submit`。
- Swift 一侧与 `ios/Fixtures/wire-v1/messages.json` 的样例放到 P4a 一起加，这样 Rust 与 Swift 两边的往返测试在同一个提交里一起变绿。

### 5.5 测试

- 清洗：空项、全空白、上限（20 条、2000 字）、原文截断与幂等、取消丢弃、不可附注丢弃、中英文与 emoji 的字符计数。
- 输出：单题（有 / 无其余区块）、多题、只有附注、What's next 的派活·建议·结束·空提交·取消、JSON、300 字缩短边界（299、300、301 字）、与 `append_status` 的先后。
- 往返与兼容：`ClientMsg::Answer` 与 `ChannelResult` 的 serde 往返、老消息缺字段的反序列化、`annotatable` 缺省与 false 不序列化。
- 回看与消费方：历史条目与 `--show-last`；`agent_help_markers_match_output`；MCP 描述断言，以及 `files_block_paths`（`mcp/ask.rs`，`[files]` 区块以空行结束）遇到后面的 `[user_annotations]` 区块不把它当路径；`transcript_full` 解析；`ask_question.rs` 的 `decide`（有答案加附注、只有附注、带 directive）。
- `annotatable` 判定：Agent 自己的提问为 true；确认卡、出错卡、Codex 原生、开发中继为 false；`--select-only` 为 false；What's next 为 true。
- iOS 网关（`tests_requests.rs`）：附注随回答进入 `ChannelResult`；`answer_hash` 无附注时与旧版逐字相同；`Caps.annotations` 与 `annotatable` 的输出。

## 6. P3：弹窗

### 6.1 引擎 `src/lib/annotations.ts`（E4）

纯 TypeScript，不依赖 Vue，也不 import 别的模块（手机打包要用，见 P4b）；只用 Safari 13（Catalina）就有的运行时 API——不用 CSS Custom Highlight API、`Array.prototype.at`、`String.prototype.replaceAll`，高亮用 `<mark>` 包裹。

- **坐标系**：附注根下「未被跳过的文本节点依次拼接」的字符串；两个相邻文本节点的最近块级祖先不同、或它们之间隔着 `<br>` 时补一个虚拟 `\n`（块级元素：`p div li ul ol blockquote pre h1–h6 tr td th table hr section article details summary`）。**跳过的是静态清单** `script, style, textarea, input, button, svg, iframe, .mermaid-block`，不沿用页内查找随 Mermaid 状态变化的动态跳过（`[data-find-skip]` / `[data-find-atomic]`）：Mermaid 渲染完成、切换「显示源码」时 DOM 会变，坐标系必须不跟着变，所以整个 `.mermaid-block`（源码与图）从头到尾都不计入。`<mark>` 本身不跳过。
- **选区 → 锚点**：`{start, end, quote}`；`start` / `end` 去掉首尾空白；去掉后为空就不是有效选区；选区必须完全落在附注根内（N7）；`quote` 取自坐标系文字，换行与连续空白压成一个空格；超过 2000 字中间截断（前 999 + 「…」 + 后 1000）。
- **高亮**：先把所有目标区间算成（文本节点, 偏移）对，再从后往前切分文本节点、包 `<mark data-annotation-id class="an-mark">`；只包含非空白字符的片段（块与块之间的空白节点不包），跨块的选区每块各包一段；最后一片带编号属性 `data-annotation-n`，编号小圆点用 CSS 伪元素（`::after`，内容取自该属性）画，不引入文本节点，所以不改变坐标系。先清掉再重绘，幂等。
- **重新定位**：每次重绘按 `{start, end, quote}` 校验——偏移处的文字（同样压空白）等于 `quote` 就用原偏移；对不上，就在压空白后的全文里找 `quote` 的所有出现位置，取离旧位置最近且不与已定位附注重叠的一处；再找不到就标「未定位」：不高亮，仍在清单里，仍随回答发出（D16）。
- **其余接口**：`clear`、`markDraft`（输入浮层打开期间的虚线框）、`locate`（第一片的位置）、`flash`、`overlapping`（选区碰到哪条已有附注）、`middleTruncate`、常量（20 / 2000 / 2000）。
- **测试** `src/lib/annotations.test.ts`（jsdom，不依赖几何）：坐标系（块间虚拟换行、`<br>`、跳过清单、Mermaid 块不计入）、跨节点选区、包裹与还原（幂等）、同一文本节点里多条附注、跨块、重绘后重新定位、「找不到原文」、与页内查找标记共存、截断。

### 6.2 类型与状态

- `src/lib/types.ts`：`UserAnnotation`；`AskRequest.annotatable?`；`PopupSubmission.annotations?`；`HistoryEntry.annotations?`。
- 新组合式函数 `src/views/popup/useAnnotations.ts`，由 `usePopupCore` 创建（收件箱里每个请求一个 `usePopupCore` 实例，所以每个请求自然一份状态），通过 `context.ts` 暴露：
  - 目标：`annotationTarget` = 请求 `annotatable` 为 false → 无；有 Message（`showDescription`）→ Message 正文；无 Message 且 `!isMulti && !whatsNext` → 那个问题的题干；其余无。
  - 状态：`notes`（`id`、`start`、`end`、`quote`、`note`、`anchored`；按位置排序，未定位的排最后）、`pending`（当前选区的锚点与碰到的附注）、`editor`（新增 / 编辑、锚点、草稿文字、浮层位置）、`listOpen`、`listIndex`、`limitReached`。
  - 动作：`add`、`update`、`remove`、`reapply`（重绘高亮）、`locate`（滚动到并闪）、`handleAnnotationKeydown`（键盘规则同 `useTodoPopover.handleTodoPopoverKeydown`：输入法组词中的按键一律吞掉；⌘/Ctrl+W 放行；Esc 关浮层；Enter 的换行 / 提交按 `submitWithBareEnter` 与修饰键，规则 `newline = bare ? anyModifier : anyModifier && !primaryOnly`、`send = bare ? !anyModifier : primaryOnly`）。
- `usePopupCore.ts`：
  - `onKeydown` 里把 `annotations.handleAnnotationKeydown(e)` 与现有的 `quickReply.handleQuickReplyKeydown(e)`、`todoPopover.handleTodoPopoverKeydown(e)` 并列（浮层或清单打开时先处理、吞掉其余按键）。
  - `collectAnswers` 的提交载荷加 `annotations: notes.map(({quote, note}) => …)`（已按位置排好序）。
  - 「已有回答」的判断（取消 / 关窗二次确认用的 `requestCancel`、共享窗口的 `hasDraft`）改为 `hasAnyAnswer || notes.length > 0`；「已回答 N/M」与发送按钮的要求不变（N8）。

### 6.3 界面

- **附注根**：`MessageSection.vue` 里渲染 Markdown 的 `MarkdownContent` 与纯文本 `<pre class="plain-body">`（不含下面的附件区）、`SequentialPane.vue` 里单题题干的同样两个元素，在可附注时带 `data-annotation-root`。
- **浮层与按钮**：新组件 `AnnotationLayer.vue`（挂在弹窗外壳里，`position: fixed`）：`selectionchange` 经短去抖后更新浮出按钮的位置（选区末尾下方，贴边时翻到上方，滚动与窗口缩放时重算）；点按钮打开输入浮层（原文在上、输入框在下、虚线框标出被选段）；点高亮（选区已折叠时才响应，避免劫持拖选）打开编辑模式。样式进 `popup.css`，颜色用 `--an-ann`（浅色 #af52de、深色 #bf5af2）。
- **页脚**：新组件 `AnnotationChip.vue`，放进 `PopupFooter.vue` 的单题与多题两种页脚，在 `QuickReplyMenu` 之后、发送之前；清单弹层仿 `QuickReplyMenu.vue`（背景遮罩、滚动保持在窗口内、roving focus、焦点移出关闭）。
- **重绘时机**：`PopupView.vue` 里 `.content` 现在绑 `@markdown-content-updated="refreshFind"`，改成先 `annotations.reapply()` 再 `refreshFind()`；附注增删改之后、`viewSource` 切换之后也 `reapply()` 并 `refreshFind()`。
- **历史窗口**：`src/components/HistoryDetail.vue`（含「复制为文本」）显示附注；`HistoryView.vue` 的列表不变。
- **文案**（`src/i18n`，中英各一份）：「添加附注」/ Add annotation；「编辑该附注」/ Edit annotation；「附注 N」/ Annotations N；清单标题「附注 · N 条」/ Annotations · N；上限提示、清单里的「编辑」「删除」「找不到原文位置」。
- **Mac 弹窗里直接提问的版面不动**，只是题干多了选字附注的能力。

### 6.4 测试与验证

- `PopupView.annotations.test.ts`（参照 `PopupView.quickReply.test.ts` 的 Tauri mock）：选字浮出按钮；浮层键盘与模态（组词、Esc、⌘W、Enter 误提交）；重叠变「编辑」；清单（定位、编辑、删除、键盘）；页脚按钮只在有附注时出现；取消 / 关窗二次确认；直接提问可附注而多题、严格选择、不可附注的请求没有入口；提交载荷的内容与顺序；源码视图。
- `vitest`（全量）、`vue-tsc --noEmit`。
- **浏览器里点真实组件**：`popup-annotations.html` 里的 `AnnotationsFrame.vue` 去掉原型自带的假按钮 / 浮层 / 高亮 / 清单，只留假窗口与 `mockIPC`，场景参数改为在真实组件上程序化选字；在 Cursor 内置浏览器里点一遍选字、浮层、高亮、清单、提交、取消确认，浅色与深色、窄窗口各一遍，并把 `docs/assets/message-annotations/popup-*.jpg` 换成真实产品的截图；`docs/development.md` 里 popup 那一条说明同步改。
- **安装后在真实弹窗里点**：`./scripts/install.sh` 装好后，用新装的 AskHuman 发一条带长 Markdown 的提问，选字加附注并发送，核对 Agent 一侧输出的文本（用户在场时请他也点一遍）。
- **Windows 与 Linux**：本机只有 macOS，无法实点。逻辑靠 vitest 覆盖（含 `platform=win` 的修饰键与布局断言，修饰键沿用既有 `primaryModifierPressed`）；实点留给用户方便时验证，P6 在 `docs/PROGRESS.md` 记一条待验项，实施记录里写明没验。

## 7. P4：iOS 协议与 Message

### 7.1 P4a 协议、草稿、发送（Swift 包 `ios/Packages/AskHumanKit`）

- `AHProtocol`：`Caps.annotations`（`decodeIfPresent`）；`AskView.annotatable`（`@OmitFalse`）；`AnnotationItem { quote, note }`；`AnswerSubmit.annotations`（`@OmitEmpty`）；`Wire` 常量（20 / 2000 / 2000）。
- `AHStore`：`DraftAnnotation { id, start, end, quote, note }`；`Draft.annotations`（`decodeIfPresent`，旧草稿照常解码）；`HistoryEntry.annotations`（`@DefaultEmpty`，旧历史文件照常解码）。
- `AHCore`：`AnswerComposer.annotations` 与增 / 改 / 删 / 上限 20 / 按位置排序 / 页面回报的偏移更新；`hasContent` 把附注算上、`isAnswered` 与 `canSend` 不变（N8）；`Draft` 的保存与 `flush` 带上附注；`Backend.submitAnswer(…, annotations:)` 与 `AppStore.send`（有附注就带上，入口已经被能力位挡住）；发送成功写历史时一并带上；快捷回复、挑待办、带指令的发送都走同一条路径。
- `ios/Fixtures/wire-v1/messages.json` 加三个样例：`pair.accept` 的 caps 带 `annotations: true`、`request.new` 带 `annotatable: true`、`answer.submit` 带 `annotations`；Rust 的 `ios::fixtures` 与 Swift 的 `AHProtocolTests` 同时往返通过。
- 测试：`AnswerComposer` 的附注增改删与草稿往返、`hasContent` / `isAnswered` / `canSend`、`items` 与附注并存、线协议往返、`Caps.annotations` 与 `annotatable` 的门控。

### 7.2 P4b 网页引擎与选字菜单

- **引擎打包**：新 `vite.annotations.config.ts`（IIFE，导出 `AHAnnotations`，目标 `["es2019", "safari13"]`，做法同 `vite.attachment.config.ts`）；`ios/Tools/web-markdown/build-annotations.sh` 构建并写到 `ios/AskHuman/Resources/WebMarkdown/annotations.js`，文件头 `// Generated from src/lib/annotations.ts (sha256 …)`；产物入库；`src/lib/annotations.bundle.test.ts` 比对头里的哈希与当前源码，不一致就失败并提示重跑脚本（引擎不 import 别的模块，所以只看这一个文件）。
- **页面**：`index.html` 在 `render.js` 之后加载 `annotations.js`、`notes.js`（CSP 的 `script-src 'self'` 已允许）。新 `notes.js`（手写，风格同 `render.js`）：
  - 包一层 `AH.render`：先调原函数（它同步设置 `content.innerHTML`），再按已有附注立刻重绘高亮（深浅色切换会重绘整页，同理）。
  - 原生到页面：`AH.setNotes(list)`（存下并重绘，随后回报定位结果）、`AH.setPending(anchor | null)`（输入面板打开期间的虚线框）、`AH.locate(id)`（返回数字：第一片相对内容顶端的位置，找不到返回 -1，不返回 undefined）、`AH.flash(id)`、`AH.clearSelection()`。
  - 页面到原生（新增三条脚本消息 `selection`、`noteTap`、`notes`）：`selectionchange` 经 `requestAnimationFrame` 合并后发 `selection`（`{quote, start, end, overlapId}` 或 `null`）；点高亮（选区已折叠）发 `noteTap`（附注 id）；重绘后发 `notes`（各附注实际用的偏移与是否定位成功）。
  - `style.css` 加高亮、编号小圆点、虚线框、闪一下的样式（颜色与弹窗一致，深色模式用 `prefers-color-scheme`），保证 `#content` 的 `user-select: text`。
- **原生**：
  - `WebPage.swift`：脚本消息名单加 `selection`、`noteTap`、`notes`；`makeUIView` 里创建 `AnnotatingWebView`（新文件 `Views/Markdown/AnnotatingWebView.swift`，`WKWebView` 子类，持有 `weak var host: AnnotationHost?`，覆写 `buildMenu(with:)`：host 允许添加、页面最近一次报告的选区非空时，按 P1 实测的位置插入「添加附注」，选区碰到已有附注时插入「编辑附注」）。
  - 新 `AnnotationHost`（`@MainActor @Observable`，每个请求一份，新文件 `Views/Markdown/AnnotationHost.swift`）：持有页面最近的选区、编辑面板状态（新增 / 编辑哪一条）、对页面的脚本通道（`setNotes` / `setPending` / `locate` / `flash`）、「能否添加」（`annotatable`、`Caps.annotations`、未满 20 条、网页渲染成功）；点菜单项走 `beginAdd()`、点高亮走 `beginEdit(id)`。
  - `WebMarkdownView(text:, onSettled:, annotations: AnnotationHost? = nil)`：不传 host 时与现在完全一样（历史页、其他 Markdown 用处不受影响）；网页没起来（`phase == .failed`）时 host 报告「不能添加」，草稿里已有的附注不动（N9）。
- 测试：`AskHumanWebUITests` 里加一条长按取词 → 菜单项 → 面板 → 高亮出现的用例（沿用 P1 的断言写法）；新 `src/lib/notesPage.test.ts`（vitest + jsdom）读入 `annotations.js` 与 `notes.js`，配一个只设置 `#content` 的假 `AH.render`，检查 `AH.render` 的包装会重绘高亮、`setNotes` 幂等、`locate` 恒返回数字、`selection` / `noteTap` / `notes` 消息的内容。

### 7.3 P4c 回答页界面

- `AnswerScreen.swift`：创建每个请求的 `AnnotationHost`，传给 `MessageCard`；`.sheet` 弹出编辑面板（新文件 `Views/Request/AnnotationEditorSheet.swift`：被选原文在上、输入框在下、键盘常驻；输入框有字时禁用下滑关闭、取消要确认丢弃；删除不确认）与清单（`AnnotationListSheet.swift`：每行编号 + 原文摘要 + 附注，点一条回到原文并闪一下，左滑编辑 / 删除）。回到原文：向页面要 `AH.locate(id)` 的位置，在 `MessageCard` 的覆盖层里放一个带这段内边距的锚点视图，用 `ScrollViewProxy.scrollTo` 滚过去，再 `AH.flash(id)`。
- `SendBar`：`buttonItems` 里 `DirectiveMenu` 之后、`nextIconButton` 之前加「附注」圆钮（有附注才出现，紫色数字角标，与「发送」同高；经典底栏用 `barSecondaryButton()`）。`ButtonRow` 与 `CountAndButtons` 按子视图个数通用排布，加一个圆钮要实测——落到 `AskHumanBarUITests`：393 pt、375 pt、辅助字号；有 / 没有「下一题」圆钮；选了带指令的发送；英文与 zh-Hans；浮动底栏（iOS 26+）与经典底栏（18.5）。需求 §4.3 的宽度估算以实测为准，**不达标就回来找用户，不自己换方案**（D21）。
- `HistoryDetailScreen.swift`：回答之后加附注段（原文引用 + 附注）。
- 文案：`Localizable.xcstrings` 经 `ios/Tools/make-catalog.py` 生成（先 `xcodebuild build`，再运行脚本）；新增 Swift 文件后 `xcodegen generate`。
- **窄屏验证设备**：创建两台 iPhone SE（第三代）模拟器（宽 375 pt）：iOS 26.2 的一台验浮动底栏（浮动底栏要 iOS 26+），iOS 18.5 的一台验经典底栏；与 393 pt 的常规机型对照。只有创建不成才退回到「更大字号档」这一现有做法，并在实施记录里写明。不碰别人的模拟器（AH-Duo-Repro、Duo-Scratch、Duo-UI-Verify、iPhone Duo、iPhone 18 Pro Max、VideoProbe-QA、iPad 系列）。
- 测试：`AskHumanBarUITests`（见上）；新 `AskHumanAnnotationUITests`（Mock 后端的可附注请求：新增、编辑、删除、清单定位与闪动、草稿保留、没有附注时底栏无圆钮、不可附注的请求没有入口、非 Markdown 没有入口）；真实 daemon 端到端 `AHE2ETests/AnnotationE2ETests`（一次性 Mac 上挂一条可附注的提问，手机引擎发 `answer.submit` 带 `annotations`，期望 Agent 一侧输出有 `[user_annotations]` 区块；只有附注；What's next 的「结束本轮」加附注）。
- 手机上的整套体验在真机上再点一遍（需要用户的手机，见「开工前定下的事」）。

## 8. P5：iOS 直接提问

- `AHCore` 里一个纯函数（可单测，`AskView` 的扩展，不带参数）判断这一形态：Message 去空白后为空、没有附件、恰好一个问题、不是 What's next（N1）。**只有这四条**，不看 `annotatable`、Mac 能力、是否 Markdown、是否严格选择（审批轮定案）。
- `AnswerScreen`：满足时 `MessageCard` 显示这个问题的题干（走网页渲染）；`QuestionCard` 不再显示题干（单题没有题号圆标，所以整个题干行去掉），只剩选项与输入框。`MessageCard.drawnByWebPage`、阅读追踪（`ReadingTracker` 等「网页排版完成」的逻辑）按「有效的 Message 文字」判断，照旧工作。非 Markdown 或网页没起来时，题干在 Message 卡片里用原生文字，仍在上方。不满足这一形态时版面与现在完全一样。
- 测试：纯函数的每个条件各一条；`AskHumanWebUITests` / `AskHumanUITests`：短题干与大段题干都在 Message 区、选项卡里没有题干、严格选择 / 老 Mac / 非 Markdown 的直接提问也一样在上方、What's next 保持原样、有 Message 的提问与多题保持原样、降级渲染。
- `docs/specs/ios-web-markdown.md` 的 U2「题干保持原生」补一句：直接提问这一形态下题干并入 Message 网页（P6 一起改）。

## 9. P6：文档同步

按需求 §8：`docs/overview-popup-ui.md`（新增「消息附注」一节）、`docs/overview-ios.md`（附注、直接提问挪题干、底栏圆钮、`Caps.annotations` / `AskView.annotatable`，「回答里的指令与待办……」一条里的 `answer_hash` 说明补上附注）、`docs/specs/ios-web-markdown.md`、`docs/specs/ios-answer-flow.md`、`docs/specs/strict-choice-and-structured-output.md`、`docs/specs/todo-whats-next.md`、`docs/specs/mcp.md`、`docs/wiki/ios-setup.md` 与 `ios-setup.en.md`（回答页底栏一条旁补一句）。`docs/overview.md` 不改（全局地图与不变量没变，只核对 L348、L451 附近的提法仍然准确）。`docs/PROGRESS.md` 只记没验完的项（真机、iOS 17 实测等），做完删除。实施记录补全，需求里与实现不一致的地方改需求。

## 10. 验证清单

- **Rust**：`cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`。
- **前端**：`vitest run`、`vue-tsc --noEmit`；引擎打包不漂移的校验。
- **Swift**：`swift test`（`ios/Packages/AskHumanKit`）、`xcodebuild test`（英文与 zh-Hans 各一遍）、真实 daemon 端到端（`AHE2ETests`）。
- **窄屏**：`AskHumanBarUITests` 在 375 pt、393 pt、五档字号、中英文、浮动与经典底栏下实测。
- **安装与实点**：`./scripts/install.sh` 编译安装到日常环境，之后用新装的 AskHuman 发提问在真实弹窗里点（不调用任何模型）；浏览器里点真实组件（§6.4）；真机一轮；Windows 与 Linux 本机无法实点（§6.4）。
- **真实 Agent 端到端**（Claude Code、Codex、Cursor 各一轮：弹窗里选字加附注后发送，Agent 收到区块并回应；只有附注；What's next 里「结束本轮」加附注；Claude 内置提问接管里加附注）会真的调用模型：只在用户授权后做；没做就在实施记录里写明没做。
- 每次 `./scripts/install.sh`、`cargo test`、`cargo clippy`、`xcodebuild` 之前先跑 `node scripts/machine-health.mjs`；退出码是 2（机器在抖动）就通过 AskHuman 把报告告诉用户并请他腾内存，任务放后台、分短段轮询，说明在等什么，不静默等待。

## 11. 提交

显式路径，Conventional Commits，英文、祈使句、小写开头、不超过 72 字符；只有 `feat` / `fix` 等类型会进用户可见的发布说明，所以只给「用户第一次能用上」的那个提交写面向用户的主题，其余加 `Release-Note: skip`。

- P0：`docs(prototype): …`、`docs(annotations): …`（见 §3）。
- P2：`feat(daemon,cli): deliver user annotations to the agent`，`Release-Note: skip`。
- P3：`feat(popup): annotate message text and send the notes with the answer`（面向用户的那一行）。
- P4a、P4b：`feat(ios): …`，`Release-Note: skip`。P4c：`feat(ios): annotate message text on the answer screen`（面向用户的那一行）。
- P5：`feat(ios): show a lone question in the message area`，`Release-Note: skip`。
- P6：`docs(annotations): sync overview, specs and wiki`。
- 不推送远端（用户没有要求）。

## 12. 风险

- **`buildMenu(with:)` 对 `WKWebView` 是否生效**：P1 先验证，不行就停下问用户（D17）。
- **iOS 17 不验**：本机模拟器是 iOS 18.5、26.2、27.0、27.1，没有 iOS 17 运行时，用户决定不验（需求 §11）。
- **Windows / Linux 无法在本机实点**：只有 macOS；逻辑靠 vitest 覆盖，实点待验项记进 `docs/PROGRESS.md`（§6.4）。
- **底栏宽度是估算**：落地后实测，不达标回来商量（D21）。
- **回到原文的滚动**：网页本身不滚动（高度由内容撑开，整页由外层 `ScrollView` 滚），所以要靠页面给出位置、原生放锚点；UI 测试里验证定位偏差。
- **页内查找与附注高亮**：查找按文本节点匹配，一个查找词恰好跨过附注 `<mark>` 的边缘时匹配不到；风险低，接受，不处理。
- **Message 重绘会换掉 DOM**：弹窗靠 `markdown-content-updated` 之后重绘、手机靠 `AH.render` 的包装重绘；Mermaid 异步完成不影响坐标系（整块跳过）。
- **主工作树里别的会话的未提交改动**（iOS 开发者模式、onboarding、Diagnostics、`Localizable.xcstrings`、`make-catalog.py`）：用户选了主工作树，没有隔离。提交一律列显式路径；`xcodebuild` 与 `swift test` 会把别人未提交的 iOS 改动一起编进来，那份改动半途编不过时先看是不是它的问题，不是我的就停下问用户；`Localizable.xcstrings` 与 `make-catalog.py` 若到 P4c 时仍有别人的未提交改动，重新生成目录会把它们的字符串一起带进文件，提交前先问用户怎么拆。
- **Agent 不认识 `[user_annotations]`**：靠区块自解释与 help 里的一行；不教用法是用户的明确选择（D9）。

## 13. 开工前定下的事

开工第一步先读 `docs/PROGRESS.md`（起草时没有与附注相关的条目），再按第 1 条问工作树。

1. **工作树**：用户在审批轮选了**在主工作树做**（没有选我推荐的 Dev Worktree）。主工作树没有启用 Dev Instance，沿用它现有的安装与验收：`./scripts/install.sh` 装到日常环境，之后用新装的 AskHuman 提问；日常讨论、审批与交接仍走正常的 AskHuman 连接。
2. **iOS 17**：不验（用户决定）。
3. **TestFlight 与真机**：不主动发 TestFlight；P4c 做完后问用户要不要发一个内部构建（走项目技能 `ios-testflight`）供真机验证。
4. **默认做法**：用户否决了 N15（并连带删除 N14），其余 N1–N13 没有异议，按默认做。
5. **提交与推送**：显式路径提交，不推送。

## 14. 实施记录

（开工后补：每期的差异、P1 的结论与截图、窄屏实测数据、没做的验证及原因。）
