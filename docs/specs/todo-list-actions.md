# 需求：所有出现待办清单的地方，都能完成、删除、添加待办（弹窗 + iOS）

> 状态：**2026-11-13 设计经用户三轮 AskHuman 问答定稿（选项与原话见 §10），并已实现**（Mac + 弹窗 + iPhone，在主工作树里做、本地提交；iPhone 真机的滑动手感、Windows / Linux 的按键尚待验收，见 `docs/PROGRESS.md`）。第一轮里我列出的「默认做法」用户没有异议，已并入 §3；实现时与设计稿有出入的几处已改写进正文（D7 加完第一条之后的样子、iPhone 草稿与整滑规则、写入通路）。图稿是原型渲染的状态图（桌面是真弹窗组件 + 真样式，iPhone 是静态示意），原型源码在 `src/prototype/popup-todo-actions*.html` / `ios-todo-actions.html`，不随产品发布。
> 用户原话（2026-11-13）：「我希望弹窗和 iOS 端，所有出现待办列表的场合都可以完成、删除、新增待办。……这个只是一个独立的入口，虽然我可以在下面选待办，但是如果我觉得这个待办的列表需要更新，我没有办法去完成或者删除其中的一些待办。所以你帮我统一设计一下合适的交互，就是所有的这种列表，我都可以去进行这种操作。」
> 关联：[`todo-quick-add.md`](todo-quick-add.md)（落地后其桌面浮层 D4–D9、确认卡范围 D1 与 iOS 半屏添加页 D10–D13 被本文取代；D14 Mac 反查在途项目仍需要）、[`todo-whats-next.md`](todo-whats-next.md)（D5 Stop 卡、D7 弹窗待办区）、[`ios-todos.md`](ios-todos.md)（`todo.*` 线协议）、[`special-replies.md`](special-replies.md)（D28–D29 手机上选待办）。
> 平台：弹窗三平台（macOS / Windows / Linux）；iOS 17+；Mac 端是现有 Daemon。

## 1. 背景与目标

待办现在出现在这些「清单」里，能做的事各不相同：

| 位置 | 选 | 完成 | 删除 | 新增 |
|---|:-:|:-:|:-:|:-:|
| 弹窗 · 提问里的折叠「待办」区 | ✓ | ✗ | ✓（两步） | ✗ |
| 弹窗 · What's next 的 TODO 选项 | ✓ | ✗ | ✗ | ✗ |
| 弹窗 · Stop 确认卡 | ✓（生成时的快照选项；下面另有一份折叠区，同一批待办摆了两遍） | ✗ | 只有折叠区那份（两步） | ✗ |
| 弹窗 · 顶栏浮层（⌘T，`todo-quick-add`） | — | ✗ | ✓（两步） | ✓ |
| iPhone · 提问页「待办」卡 | ✓ | ✗ | ✗ | ✗ |
| iPhone · What's next 队列 | ✓ | ✗ | ✗ | ✗ |
| iPhone · Stop 确认卡 | ✓（快照选项 + 待办卡，同样摆了两遍） | ✗ | ✗ | ✗ |
| iPhone · 请求页 ⋯ → 添加待办…（半屏页） | — | ✗ | ✗ | ✓ |
| 待办窗口 / iPhone 待办页 | — | ✓ | ✓ | ✓ |

结果是：回答问题的当下，看见一份已经过时的清单，却只能选，不能顺手把做完的勾掉、把作废的删掉、把刚想到的记上；要整理就得离开页面。

**目标**：每一份「出现在请求页里的待办清单」都带同一套动作——**选**（仅限属于回答的清单）、**完成**、**删除**、**添加**，手势与视觉在弹窗与 iPhone 上各自统一；多余的、零散的独立入口（顶栏浮层、手机半屏添加页）清掉。

**非目标**：

- 不在清单里编辑文字、调序、设 ⚡ 自动执行、管附件——这些仍属于待办窗口 / iPhone 待办页（页内清单是「随手整理 + 选」，窗口是「完整管理」）。
- 不动「新建任务」里的待办选择器（它选的是来源，不是整理清单）；不动 IM 卡片（`/todo`、`/todo-rm`、What's next / Stop 的 IM 卡仍是快照选项）。
- 不新增线消息、不改 `PROTOCOL_VERSION` 与手机 `PROTO`（D14）。

## 2. 术语

| 术语 | 含义 |
|---|---|
| 页内清单 | 请求页里属于「请求所在项目」的待办清单：弹窗的折叠「待办」区、What's next / Stop 卡里的 TODO 选项；iPhone 的「待办」卡、What's next / Stop 的队列行 |
| 添加行 | 清单末尾的虚线「＋ 添加待办」行，点它就地变成输入行 |
| 空入口 | 项目没有待办时，页内清单的唯一形态：一个「＋ 添加待办」（弹窗是折叠标题位置的药丸，iPhone 是一行卡片） |
| 实时队列选项 | 选项里的待办不是生成请求时的快照，而是随项目待办实时刷新（What's next 已有，见 `whatsNextTodos.ts` 的 `refreshWhatsNextTodos`）；本文让 Stop 确认卡也采用，请求上用一个可选标记 `todoQueue` 表明（名称实现时可调） |

## 3. 决策记录

| 编号 | 决策项 | 结论 | 理由 / 否决后的代价 |
|---|---|---|---|
| D1 | 范围 | 弹窗：普通提问的折叠「待办」区、What's next 与 Stop 确认卡的 TODO 选项；iPhone：提问页「待办」卡、What's next 与 Stop 的队列行。权限 / 出错确认卡没有页内清单，保持没有 | 用户定：「所有出现待办列表的场合」。Stop 卡用户明确要求这一期一起做（我原推荐先不做） |
| D2 | 动作集合 | 选（只有属于回答的清单有）、完成、删除、添加；不做编辑文字 / 调序 / ⚡ / 附件 | 页内清单管「随手整理」，完整管理留在待办窗口 |
| D3 | 选与动作的关系 | **点整行＝选**（不变）；完成 / 删除只在行的边缘（弹窗：悬停按钮；iPhone：滑动 + 长按菜单），不抢点击。被完成或删除的待办自动从已选里去掉，不会发出一个已不存在的待办；新加的待办不自动选中 | 用户定（选项：滑动 + 长按菜单；悬停按钮）。「添加不等于作答」与 `todo-quick-add` D3 一致 |
| D4 | 完成 | 与待办窗口同一语义：`take`——出队并写入执行历史（弹窗 `todos_complete`，手机 `todo.op complete`）。**弹窗**：点 ✓ 后整行立刻划线、勾变绿，停约 1 秒可点「撤销」，之后提交并收起（沿用待办窗口 `TodosView.vue` 的 `COMPLETE_DELAY_MS = 1000`、`onPendingCheck` / `commitComplete`）。**iPhone**：立即发送并收起，底部胶囊「已完成「…」· 撤销」约 4 秒；撤销＝`restore` 加一次 `reorder` 放回原位（`restore` 本身把条目放到队尾） | 完成可逆是用户可接受的（待办窗口一直如此）；手机没有「宽限期」是因为 App 可能在 4 秒内被切走，不能把写入押在计时器上 |
| D5 | 删除 | **两步确认、不可撤销**（沿用待办窗口 / 现有 ✕）。弹窗：悬停出现 ✕，点后变红色「确认删除」，再点才删，点别处取消。iPhone：左滑只露出红色「删除」，点它才删（整滑不删，「滑开再点」就是确认）；长按菜单里的「删除」本身就是菜单里的一次显式选择，直接执行 | 删除是真删（`todos_remove`），没有历史可找回 |
| D6 | 添加 | 清单末尾的虚线「＋ 添加待办」行，点击就地变输入行（多行，自动长高）。弹窗提交键沿用「发送快捷键」设置、规则与答案框相同（设为 ⌘↵ 时 ⌘↵ 添加、Enter 换行；设为 Enter 时 Enter 添加、带修饰键的 Enter 换行；输入法组词中不处理），Esc 收起输入行（不关窗）；iPhone 多行输入、右侧 ↑ 添加。添加后输入行保持打开、清空，新行高亮片刻（约 1.2 秒，沿用浮层的时长），可连续添加；只填文字，追加在队尾，不带 ⚡ 与附件 | 用户定：原地输入（iPhone 选项 A）。与 `todo-quick-add` D3 / D6 同一做法，只是挪到了清单里 |
| D7 | 空入口 | 项目没有待办时，普通提问页只显示一个「＋ 添加待办」（弹窗：与折叠标题同位置的药丸；iPhone：一行卡片），点它变输入行；加了第一条后清单展开、新行高亮，输入行保持打开可以接着加（不收回成折叠的「待办 1」，免得刚加的东西看不见）。清单被清到 0 条时收回成空入口。What's next / Stop 的添加行始终在 TODO 选项之后、静态选项之前，0 条待办时也在 | 用户定：「没有的时候显示添加待办可以」。**否决**「折叠标题旁常驻一个 ＋」：用户不要 |
| D8 | 顶栏待办按钮与 ⌘T | **去掉顶栏浮层**（`TodoPopover.vue`、`useTodoPopover.ts` 及其模态键盘处理），按钮改回**直接打开待办窗口**（`openTodosWindow()` → `openTodos(project)`，预选当前项目；请求无项目则不预选）。⌘T（Windows / Linux 为 Ctrl+T）改为「添加待办」：当前面板有页内清单（含空入口）→ 展开、滚入可见、光标放进添加行；没有（权限 / 出错确认卡、无项目的请求、多题顺序模式的非末题面板）→ 打开待办窗口。已把语音快捷键设成 ⌘T 的用户，⌘T 仍归语音（沿用现有规则） | 用户定：「现在我们有统一的入口了，……否则这个功能挺重复的」。代价（用户认可）：确认卡上不能再就地记一条，要点顶栏进待办窗口 |
| D9 | What's next | 与普通提问同款的行内动作；添加行在最后一条待办之后、「结束本轮」之前（顺序仍是：建议、待办、添加行、结束项）；添加 / 完成 / 删除后清单立即刷新，已选中的选项保持不变（被完成 / 删除的除外） | 待办本来就是这个问题的选项 |
| D10 | Stop 确认卡 | 改成与 What's next 同一种**实时队列选项**：只留一份实时清单（TODO 标记 + ✓ ✕ + 添加行），**去掉下面重复的折叠「待办」区**；选项顺序不变（待办在前，然后是添加行，再是「继续对话」「结束对话」）。请求新增一个可选标记（`todoQueue`），让弹窗与手机知道这张卡的待办选项是实时队列。实时选中的待办与 What's next 一样以原文 + id 提交（没有冻结 chip 的下标），所以 Stop 的结果解析要补一处（§5.2）：按冻结 chip 同样的规则交付——原文不截断、附件随提示一起交付。IM 渠道与旧弹窗的 Stop 卡仍是快照选项 + 下标映射，不变 | 用户定做；设计时发现今天的 Stop 卡把同一批待办摆了两遍（图 3 对照），统一后不再重复 |
| D11 | 严格选择（仅选择） | 弹窗：待办区仍显示，行不可选，但可完成 / 删除 / 添加；iPhone：本来就不在「仅选择」提问里显示待办卡，不变 | 保持现有边界 |
| D12 | iPhone 手势 | 整行点按＝选；右滑露出绿色「完成」（滑出停住点一下，或一路滑到头松手＝完成并给一下触觉）；左滑露出红色「删除」；长按＝系统菜单「完成 / 复制 / 删除」，VoiceOver 自定义操作同样三项。手势与待办页一致，但提问页不是系统 `List`，要自绘（§6.1）。脚注「选中的待办会和你的回答一起发给 Agent，并从待办里移除。」之后加一句「右滑完成，左滑删除。」 | 用户定（滑动 + 长按菜单） |
| D13 | iPhone 半屏添加页 | **移除**请求页 ⋯ 菜单里的「添加待办…」和它的半屏页（`RequestTodoSheet` / `RequestTodoComposer`）；确认类页面、仅选择类提问不再有就地添加入口，要记待办去待办页 | 用户定：「我们已经统一了交互，就把之前零散的给清理掉」。代价（用户认可）：手机上这两类页面不能就地记 |
| D14 | 协议 | 不改 `ipc/mod.rs` 的 `PROTOCOL_VERSION`、手机 `PROTO`：只有**可选字段**（`AskRequest.todo_queue`、`AskView.todoQueue`，缺省为假），能力靠已有的 `caps.todos` / `caps.answerTodos` 判断；新旧组合降级见 §6.3 | 规则见 `docs/overview.md`「IPC 协议号」与 `docs/overview-ios.md` 不变量 |
| D15 | 写入失败与并发 | 弹窗沿用现有做法（出错不打断、重读清单）；手机沿用 `todoGone` 等拒绝话术与 `clientMsgId` 幂等：条目已被别处删掉时，行消失并在顶部胶囊提示「这条待办已经不在了」；Mac 不在线（或太旧）时，行上操作停用并说明原因，写入不排队 | 与待办页一致 |
| D16 | 超出可见范围 | 弹窗 What's next 最多 10 个选项（含建议与结束项）、Stop 最多 10 条待办（沿用现有上限），超出的待办仍写在问题文字末尾「还有 x 条待办未列出」；添加后新条目若落在可见范围外，添加行内短暂提示「已添加，排在第 N 位（未显示）」，完成 / 删除后后面的条目自动补进。iPhone 折叠只显示前 5 条时，添加会自动展开并滚到新行 | 默认做法（用户未单独讨论，评审时可改） |

## 4. 用户视角

### 4.1 弹窗

**普通提问**（图 1）：折叠「待办 N」区展开后，每行与今天一样点整行选中；**悬停一行**（或用 Tab 聚焦）出现 ✓（标记完成）和 ✕（删除），出现在行尾、快捷键标注之前。点 ✓：整行划线、勾变绿，行尾变成「撤销」，约 1 秒后收起；若这条原本被选中，选择同时撤掉。点 ✕：变红色「确认删除」，再点才删；点别处取消。列表末尾是虚线「＋ 添加待办」，点击（或按 ⌘T）原地变成输入行，⌘↵ 添加、Esc 收起，可连续添加，新行短暂高亮。项目没有待办时只有一个「＋ 添加待办」药丸（图 4）。

![普通提问](../assets/todo-list-actions/desktop-ask.jpg)

**What's next**（图 2）：TODO 选项行与建议、「结束本轮」在同一个列表里，✓ ✕ 放在 `⌘n` 标注左边；「＋ 添加待办」在最后一条待办之后、「结束本轮」之前。

![What's next](../assets/todo-list-actions/desktop-next.jpg)

**Stop 确认卡**（图 3）：只留一份实时清单，行为与 What's next 完全一致；今天的快照选项 + 下面的折叠区合并成这一份。

![Stop 确认卡](../assets/todo-list-actions/desktop-stop.jpg)

**项目没有待办**（图 4）：普通提问是一个「＋ 添加待办」药丸（位置与折叠标题相同），What's next 里是建议与「结束本轮」之间的添加行。

![没有待办](../assets/todo-list-actions/desktop-entry.jpg)

**顶栏**：待办按钮点击直接打开待办窗口（预选当前项目），悬停提示「待办窗口」；⌘T 见 D8。权限 / 出错确认卡里没有页内清单，想记待办就点这个按钮。

### 4.2 iPhone

**提问页「待办」卡**（图 5）：行内整行点按是选中；右滑露出绿色「完成」，左滑露出红色「删除」；完成后底部浮出「已完成「…」· 撤销」约 4 秒。

![提问页待办卡](../assets/todo-list-actions/ios-ask.jpg)

**长按、添加、What's next**（图 6）：长按一行弹出系统菜单（完成 / 复制 / 删除）；点「＋ 添加待办」原地变输入行，键盘弹出，↑ 添加，新行高亮，输入行保留；What's next 的待办行就是选项行，所以手势与添加行完全一样（添加行在「显示全部 N 条待办」之后、结束选项之前）。

![长按、添加、What's next](../assets/todo-list-actions/ios-more.jpg)

**空入口与 Stop**（图 7）：项目没有待办时是一行「＋ 添加待办」卡片；Stop 确认卡与 What's next 同款，只留一份实时清单。

![空入口与 Stop](../assets/todo-list-actions/ios-entry.jpg)

## 5. 实现要点

### 5.1 弹窗（前端）

- **行内动作**：新增 `TodoRowActions`（✓ ✕ 与「撤销」「确认删除」两个状态，供三处清单共用）与 `TodoAddRow`（虚线行 ↔ 输入行）。`TodoSection.vue` 里现有的 `.todo-del` / `.todo-del-confirm` 换成它；`QuestionCards.vue` / `SequentialPane.vue` 里带 `opt.todoId` 的选项行同样渲染它，添加行作为清单里的非选项子节点（不占 `⌘n`，不参与选项计数）。
- **`usePopupCore.ts`**：新增 `completeTodo(id)`（乐观划线 → 1 秒宽限 → `todosComplete`，撤销取消提交，沿用 `TodosView.vue` 的做法）、`addTodo(text)`（`todosAdd(project, text)` 后重读）；`removeTodo` 保持。三者都同时清掉 `todoChosenIds` / `todoChosenSnapshots`（普通提问）或 `chosenByQ[0]`（实时队列选项）里的该条。`todoSectionVisible` 的条件改为「有项目 + 有请求 + 不是实时队列选项的请求 + （纵向或末题）」，**不再要求有待办**（0 条渲染空入口）。
- **实时队列选项泛化**：新增 `liveTodoOptions = whatsNext || todoQueue`，替换下面这些「待办是选项」的判断——`usePopupCore` 里的 `todoSectionVisible`（隐藏折叠区）、`loadTodos`（刷新选项）、`collectAnswers`（`selectedWhatsNextTodos` 的原文 + id 提交）、`renderInit`（记录静态选项与问题正文，`whatsNextStaticOptions` / `whatsNextBaseMessage`）；`optionDisplay.ts`（去掉「执行待办：」前缀）、`QuestionCards.vue` / `SequentialPane.vue` 的 TODO 标记与附件角标、`usePopupFind.ts`（查找段落用的显示文字）。`refreshWhatsNextTodos` 要支持两种排布：What's next 是「建议、待办、结束项」，Stop 是「待办、继续对话、结束对话」（待办上限 10，沿用 `MAX_OPTION_TODOS`）。与 What's next 语义强相关、Stop 不需要的分支保持只认 `whatsNext`：结果渲染成纯文本、快捷回复（Stop 本来就没有）、批注、结束项互斥（Stop 是单选）。
- **去掉浮层**：删除 `TodoPopover.vue`、`useTodoPopover.ts`、`useTodoPopover.test.ts`、`PopupView.todoPopover.test.ts`（约 1300 行），以及 `popup.css` 里的 `.todo-popover` / `.tp-*` / `.todo-backdrop`、`zh.ts` / `en.ts` 的 `popup.todoPopover.*`；`PopupNavbar.vue` 的按钮 `@click` 直接 `openTodosWindow`，提示改 `popup.nav.todos`；`usePopupCore.ts` 里所有 `todoPopover.*` 引用一并清掉（`useTodoPopover(...)` 实例、`quickReply` 与 `annotations` 的 `suspended`、`onKeydown` 里 `handleTodoPopoverKeydown` 的位置、`renderInit` 的 `resetTodoPopover()`、`nativePreviewBlocked`、`onTodoButtonClick` / `openTodosWindowFromPopover`、返回对象里的 `todoPopover*` 字段）。`loadTodos()` 的守卫恢复为「有项目且有 `request`」（`todo-quick-add` D9 为确认卡加的 `confirmRequest` 放行不再需要）。
- **⌘T**：`onKeydown` 里换掉浮层的位置——⌘T → `focusTodoAdd()`：当前面板有页内清单则展开、滚入可见并聚焦添加行，否则 `openTodosWindow()`；输入行已聚焦时 ⌘T 不做事。`shortcut.ts` 继续把 ⌘T 列为弹窗保留键（语音快捷键冲突提示照旧）。
- **键盘隔离**（`todo-quick-add` §6.2 的风险缩小版）：浮层是整页键盘模态，这里不需要那么重；沿用同一个位置的拦截——添加输入行聚焦时，`onKeydown` 先交给它：Enter 家族按 D6 的规则（添加或换行）、Esc 收起，并且都 `preventDefault` + 不再往下走，所以 Enter / ⌘Enter / Esc 不会漏到「提交答案 / 关窗」；输入法组词中一律放行不处理；其余快捷键（⌘1–9、⌘W、查找……）照常。输入行 `@paste.stop`（与浮层一致：全局粘贴监听会把剪贴板里的图片收进答案，这里只收文字）。行内 ✓ ✕ 是普通 `button`，可 Tab 聚焦，不新增全局快捷键。
- i18n（`zh.ts` / `en.ts`）：沿用 `popup.todos.*`（删除、确认删除），新增 `popup.todos.complete`（标记完成）、撤销、`popup.todos.add`（添加待办）、占位 / 提示、`popup.todos.addedHidden`（已添加，排在第 N 位）。

### 5.2 Mac（Rust）

- **标记**：`models.rs::AskRequest` 增 `todo_queue: bool`（`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`，与 `whats_next` 同款），在 `daemon/request.rs` 把 `TaskRequest` 转成 `AskRequest` 处（现有 `request.whats_next = task.whats_next;` 旁）置为 `task.popup_kind() == Stop && !task.project.is_empty()`——`popup_kind()` 已经把旧版 Stop 钩子也认进来；不论生成时有没有待办都置（0 条时也要有添加行）。字段只加在 `AskRequest` 上、不动 `TaskRequest`，不影响恢复存储里的任务哈希与 `SCHEMA_VERSION`，实现时仍按 `AGENTS.md`「Changing recovery storage」核对一遍。
- **Stop 结果解析**：`agents/stop.rs::build_task` 不改（IM 渠道与旧弹窗仍按快照选项 + 下标工作）。新弹窗 / 新手机提交的实时选中待办只有原文（进 `user_input`）和 id（进 `todo_ids`），今天的 `parse_ask_decision` 会把它当成手写指令：截断到 `MAX_INSTRUCTION_CHARS`（1000 字，而待办最长 4000 字，违背 `todo-whats-next` D11「待办原文完整交付」）、附件文件被丢掉（冻结 chip 路径走 `task_with_attachments`）。需要补一条路径：让 `parse_ask_decision` 知道这条回答带了待办（建议：`cli/output.rs` 只对 `todo_queue` 的请求在每条回答的 JSON 里多带 `todo_ids`——Stop 钩子自己消费，普通提问的公开 JSON 不变），按冻结 chip 同样的规则交付：原文不截断，`files` 用 `task_with_attachments` 附上。出队与附件交付仍由 Coordinator 的 `ids_to_dequeue` / `apply_todo_deliveries` 统一做（已按 `todo_ids` / `todo_selections` 处理）。
- **手机视图与网关**：`ios/wire.rs` 的 `AskView` 增可选 `todo_queue`（`#[serde(default, skip_serializing_if = "is_false")]`，与 `whats_next` / `annotatable` 同款），`ios/views.rs` 从 `AskRequest` 映射。`ios/gateway/requests.rs` 里 `merge_todo_picks`（`ask.whats_next` 决定「待办原文在前、手写在后」还是「手写在前、加『另外看一下这个待办任务：』行」）与 `resolve_todo_picks`（`ask.whats_next && ask.single` 限一条）两处，把 `ask.whats_next` 换成「`whats_next || todo_queue`」。
- `daemon/runtime/ios_todos.rs` 的 `World::resolve`（`todo-quick-add` D14：在途请求的项目也能反查）**保留**——手机在请求页上完成 / 删除 / 添加都靠它。
- `daemon/ask_dedup.rs::composes_todo_question`（`whats_next || Stop`，决定合流指纹是否忽略待办队列）已有同一个谓词，可直接复用来定义「实时队列」，不另造一份。

### 5.3 iOS

- `TodoPickViews.swift`：`TodoPickCard` / `TodoQueueRows` 的行换成带手势的 `TodoActionRow`；新增 `TodoAddRow`（虚线行 ↔ 多行输入 + ↑）；空入口卡片；脚注加手势提示。
- **`RequestTodoPicker`**（AHCore）：它不经 `TodosStore`，自己用 `backend.todoList(macId:projectId:)` 读队列、不订阅 `todo.watch`；`phase`、`macTakesPicks`（`caps.todos && caps.answerTodos`）、`mac?.link` 已经在那里。新增 `complete(_:)` / `remove(_:)` / `add(_:)` / `undoComplete()`，写入走 `backend.todoApply(macId:operation:)`（`TodoAddCall` 已用的同一个接口，`TodoMutation` 生成操作与 `clientMsgId`）：完成 / 删除先从本地队列拿掉，添加先放一行「发送中」，之后用 ack 里的 `list`（写入后的整份项目清单，拒绝时也带）对账，失败则 `reload()`。`mac?.link == .online` 才允许写（与 `TodosStore.canWrite` 同一条规则，写入不排队）。完成 / 删除同步取消选中（`AnswerComposer.setTodoIds`）。`isVisible` 现在是 `listsQueue && phase == .loaded && !todos.isEmpty`，改成不要求非空（0 条显示空入口卡）。
- **`whatsNext` 泛化**：给 `AskView` 加 `todoQueue`（`Messages.swift`，`@OmitFalse`），并加一个 `listsTodoQueue = whatsNext || todoQueue`；`AnswerComposer`（单选限一条、选待办时清掉其它选项、结束项互斥、`dropTodoOptionSelections`——约 63、81–83、196–198、211–216、229 行）、`RequestTodoPicker.singleSelection`、`AnswerScreen`（约 67、486 行：什么时候显示待办卡 / 什么时候列队列）改认它；`DirectQuestion` 与快捷回复相关的 `whatsNext` 判断不动。
- **移除**：`RequestTodoSheet.swift`、`RequestTodoComposer.swift`、`RequestScreen.swift` 里的菜单项与成功胶囊、`AppStore.todoComposer(for:)` / `closed(id:)`、`RequestTodoComposerTests`、`TodoE2ETests` 与 UI 测试里相应的用例、`Localizable` 里仅它用的文案；`TodoAddCall` 是共享件（开发者模式快速添加也用），保留。
- **手势实现**：提问页是 `ScrollView` 而不是 `List`，没有 `.swipeActions`，所以自绘（`SwipeActionRow.swift`）：SwiftUI `DragGesture(minimumDistance: 12)` 作为 `.simultaneousGesture` 挂在行上，起手时判一次方向（水平分量 > 垂直 × 1.2、且起点不在左缘 20pt 内才接管），要与页面纵向滚动、左缘返回手势、行按压反馈并存；按钮垫在行后面（`.background`），不参与行的布局，滑开时行高与下面的行都不动；滑动期间页面不滚（`SwipeCoordinator` + `.scrollDisabled`），松手那一下整行的点按被吞掉（`Button` 在触点抬起于自身时触发，滑动不能顺带选中）；长按用 `.contextMenu`；VoiceOver 用 `.accessibilityActions`。

## 6. 边界、风险与已知限制

### 6.1 iPhone 手势与滚动（最大的实现风险）

自绘滑动要与三件事共存：页面纵向滚动（只在水平位移明显大于纵向时才接管）、导航栏左缘的返回手势（行的右滑不能从最左缘起手）、整行的点按与按压反馈。`.contextMenu` 与自绘 pan 同时挂在一行上时，长按识别是否会被 pan 吃掉，需要在真机上验证；模拟器测试覆盖逻辑，手感必须真机看一次（落地时记入 `docs/PROGRESS.md`）。

### 6.2 iPhone 内联输入与键盘

添加输入行在滚动页里，要走现有的键盘避让与「输入可见」机制（`InputRevealTarget` 一类）；输入框获得焦点算「正在编辑」，已结束请求的倒计时不能把页面收走；收起键盘或点别处时，输入行里没有字就变回虚线一行；写了一半的草稿则保持输入行的样子，不藏起来（草稿只在内存里，请求页存续期间不丢，不落盘）。弹窗同理：Esc 把输入行收成一行，写了一半的字留着，下次打开还在。

### 6.3 新旧组合

| 组合 | 表现 |
|---|---|
| 新手机 + 旧 Mac（缺 `caps.todos` 或 `caps.answerTodos`） | 页内清单不显示（与今天一致，`macTakesPicks` 为假） |
| 新手机 + Mac 暂时离线 | 清单留在屏幕上（已选的保留），行上操作停用并说明原因，不排队 |
| 旧手机 + 新 Mac | Stop 卡仍是快照选项 + 折叠「待办」卡（今天的样子）；不认 `todoQueue`；其余照旧 |
| 新弹窗宿主 + 旧 daemon | 没有 `todoQueue` 标记，Stop 卡保持今天的样子（快照选项 + 折叠区），其余动作照常 |
| 旧弹窗宿主 + 新 daemon | 忽略 `todoQueue`，同上；顶栏仍是旧浮层 |

### 6.4 其它

- **实时刷新依赖 `todos-updated`**：弹窗自己写入后先乐观更新本地，再等监听事件重读，不让界面等文件监听。手机请求页没有推送，靠写入的 ack、回到前台与「被拒绝时」`reload()`。
- **并发**：别处（待办窗口、手机、CLI、IM）改了清单，本页随 `todos-updated` 刷新；被移除条目的本地选中态同步清除（已有）。
- **老 Mac 反查不到在途项目**：没有 `todo-quick-add` D14 那条 Mac 小改的老版本，对「正在等你回答的 Agent 的项目」可能回 `unknownProject`，行上操作沿用快速添加的「项目已不在」话术（`todo-quick-add` §6.3），不新增文案。
- **「完成」不等于「执行」**：选中并发送是「让 Agent 去做」，✓ 是「我自己处理了 / 不要了」，两者都会把待办从队列移走；悬停提示与菜单文案用「标记完成」，避免混淆。
- **全局按键逻辑**：弹窗里若有「键入即聚焦答案框」之类按目标元素判断的逻辑，要认得添加输入行是可编辑目标（实现时核对 `onKeydown` 的目标判断）。
- **深色模式 / 三平台**：样式只用现有主题变量；Windows / Linux 的 ⌘T 显示为 Ctrl+T。

## 7. 测试与验证

**弹窗（vitest + vue-tsc）**：

- 行内动作：悬停 / 聚焦出现 ✓ ✕；完成的乐观划线、1 秒宽限（假计时器）、撤销取消提交、提交后重读；删除两步与点别处取消；完成 / 删除时取消选中（普通提问与 What's next / Stop 两条路径）；严格选择下行不可选但动作可用；`submitting` 时按钮禁用。
- 添加：空白拒绝、成功后清空并保持打开、失败保留草稿、提交键两种设置、输入法组词、Esc 只收输入行；**键盘隔离**——普通提问 / What's next / Stop 里输入行聚焦时 Enter、⌘Enter、Esc 都不触发提交或关窗。
- 空入口：0 条时药丸、加第一条后变回折叠区、清到 0 条收回药丸；What's next / Stop 的添加行位置（含 0 条）。
- Stop 实时队列：请求带 `todoQueue` 时折叠区不渲染、TODO 标记、选项随 `todos-updated` 刷新（待办在前、静态选项在后）、选中待办以原文 + id 提交；不带标记时保持旧行为。
- 顶栏按钮直接开窗；⌘T 的三个分支（有清单→聚焦添加行、无清单→开窗、语音快捷键占用时让路）；浮层相关的旧测试随删除一并移除。

**Mac（cargo test）**：`todo_queue` 只在 Stop 且有项目时置真；旧 JSON 缺字段反序列化为假；手机视图带 `todoQueue`；网关对 Stop 的待办按 What's next 规则合并并限一条；`parse_ask_decision` 对带 `todo_ids` 的回答：长待办原文不截断、附件文件随提示交付，冻结 chip 与「继续 / 结束」的既有用例不变。`cargo clippy -D warnings`、`cargo fmt --check`。

**iOS**：`RequestTodoPicker` 单测（完成 / 删除 / 添加的乐观覆盖与回滚、选中联动、撤销＝`restore` + `reorder`、离线停用、`todoGone`、空入口可见性）；`AnswerComposer` 的队列泛化（Stop 单选）；Mock UI 测试（en + zh-Hans）：滑动、长按菜单、内联添加与键盘、空入口、Stop 卡；原有 `RequestTodoComposerTests` 随功能移除。**真机**：滑动 / 滚动 / 返回手势的手感（§6.1）。

**装机验证**（`./scripts/install.sh`，先 `node scripts/machine-health.mjs`）：用真实弹窗按一遍——普通提问 / What's next / Stop 卡里悬停、完成、撤销、删除、添加；权限确认卡里 ⌘T 与顶栏按钮开待办窗口；项目 0 条待办；待办窗口里同时改清单，弹窗实时跟上。

**实测记录（2026-11-13，主工作树，macOS）**：vitest 1824 条与 node 测试 223 条通过；`cargo test` 3018 条通过，`cargo fmt --check` 与 `clippy -D warnings` 干净（`ios_tasks.rs:699` 的旧 `manual_is_multiple_of` 提示不属于本功能，验证时 `-A` 掉）；Swift 包 538 条通过；模拟器 UI 测试 `AskHumanTodoPickUITests`（en 20 条、zh-Hans 19 条）通过，回归的十个类（主类、批注、输入可见、操作栏、直接提问、已结束、会话历史、到达、离开、折叠导航）与开发者模式类共 198 条通过（少数按原有设计跳过）；其中两条在回归那一轮因模拟器键盘事件没送达偶发失败（输入的字少了一个字母、「没有键盘焦点」），单独重跑通过。装机后在隔离的临时 `ASKHUMAN_HOME` 里用真实弹窗按过：普通提问里完成（划线 + 撤销，约 1 秒后收起并写入）、两步删除（同一时刻只有一行在等第二下）、添加行（⌘↵ 添加、Esc 只收输入行、不会把答案发出去）、⌘T 聚焦添加行、顶栏按钮开待办窗口且预选项目、What's next 的清单带添加行、外部清空待办后弹窗实时变成「＋ 添加待办」药丸。按的过程中发现两处并已修：输入行展开 / 长高时只把文本框滚进了视野，行的下沿压在底栏下面（改为滚整行）；输入框自己带了一圈全局焦点环，与行的焦点环叠成两层框（输入框不再画）。**没有按过**：Stop 卡的实时队列与权限确认卡上的 ⌘T / 顶栏按钮（只有单元测试覆盖）、Windows / Linux、iPhone 真机。

## 8. 落地后要同步的文档

- [`todo-quick-add.md`](todo-quick-add.md)：状态改写为「桌面浮层 D4–D9、确认卡范围 D1 与 iOS 半屏添加页 D10–D13 已被 `todo-list-actions.md` 取代」，D14（Mac 反查在途项目）保留；§4 标注已被取代，不删除历史。
- [`todo-whats-next.md`](todo-whats-next.md)：D5 Stop 卡改为实时队列选项；D7「折叠待办区」补上页内清单的完成 / 添加。
- [`ios-todos.md`](ios-todos.md)：I20（请求页新增）改为页内清单；写明请求页上的完成 / 删除 / 添加走哪几条线消息。
- [`special-replies.md`](special-replies.md)：D28–D29 的「手机上不删除待办」改为「手机上可完成 / 删除 / 添加」。
- `docs/overview-ios.md`、`docs/wiki/ios-setup.md` / `ios-setup.en.md`：去掉「请求页 ⋯ → 添加待办」，补页内清单的手势；`docs/overview.md`「输入面」一行里的「Popup 内新增」改为「Popup / iOS 页内清单」。

## 9. 后置（本期不做）

页内清单里的编辑文字、调序、⚡ 开关、附件；折叠标题旁常驻「＋」（用户否决）；IM 卡片上的完成 / 删除 / 添加；确认卡（权限 / 出错）上就地添加（用户认可去掉）；Stop 卡在 IM 渠道的实时化。

## 10. 用户原话与定案轮次

**起点**（2026-11-13）：见文首引用。

**第 1 轮**（交互方式与范围）：
- iPhone 行动作：「滑动手势 + 长按菜单（推荐：和待办页同一手势，行上不加任何按钮；提问页不是系统 List，滑动要自绘，需在真机验证与滚动 / 返回手势不打架）」
- iPhone 添加：「原地变输入行 + 键盘（推荐：连续添加最顺）」
- 桌面按钮显示：「悬停 / 键盘聚焦时才出现（推荐：与现有 ✕ 一致，列表保持干净）」
- Stop 确认卡：「这一期一起做（范围明显变大，但「所有列表」就真的全齐了）」（我推荐的是先不做）
- 自由补充：「之前那个弹窗里面，右上角的待办加的那个小窗口，就是为了支持在提问的时候新增待办。现在我们有统一的入口了，是不是那个就可以去掉了？就是改回点那个直接打开待办弹窗。否则这个功能挺重复的。我如果是想当前项目的待办，我直接在下面就可以了。如果我想快速进其他的待办，我点上面。」

**第 2 轮**（顶栏浮层、⌘T、Stop、iPhone 菜单）：
- 顶栏浮层：「去掉浮层：按钮改回直接打开待办窗口（预选当前项目）；提问页补「折叠标题旁的＋」和「没有待办时的一行入口」……」并补充：「不要在折叠标题旁加加号，没有的时候显示添加待办可以。」
- ⌘T：「⌘T = 添加待办：页内有待办区就展开并把光标放进添加行；没有（确认卡等）就打开待办窗口」
- Stop 卡：「统一成 What's next 的做法：只留一份实时清单，去掉下面重复的折叠区」
- iPhone「⋯ → 添加待办…」：「也去掉：手机上也只在清单里添加（确认页、仅选择页就不能就地添加）」，补充：「我们已经统一了交互，就把之前零散的给清理掉。」

**默认做法**（第 1 轮我列出、用户没有异议）：清到 0 条后收回空入口；离线 / Mac 太旧时行上操作停用并说明原因；严格选择提问桌面只剩完成 / 删除 / 添加；不做编辑文字、排序、附件；「新建任务」里的待办选择器不动；键盘上 ✓ ✕ 可 Tab 聚焦、不新增全局快捷键（⌘T 除外，见 D8）。

**写 spec 时对照代码做的两处修正**（不是用户的决定，评审时请留意）：
- D10 / §5.2：我在问答里说过「Stop 结果解析不变」，对照 `agents/stop.rs` 后发现实时选中的待办走「手写指令」分支，会被截到 1000 字并丢附件，所以补了一条解析路径。
- §5.3：iPhone 请求页的待办清单不走 `TodosStore`（没有订阅与统一的写入覆盖层），写入与对账要在 `RequestTodoPicker` 里自己做。
