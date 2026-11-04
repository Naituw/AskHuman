# 计划：在收到的请求里直接给当前项目添加待办

> 需求：`docs/specs/todo-quick-add.md`（决策编号 D1–D15 均指其 §3）。状态：2026-11-04 起草，**等用户评审需求与本计划**，通过前不写代码。
> 分支 / 工作方式：待用户定（见文末「开工前要定的事」）。下文各阶段的文件名是起草时的设想，**实际以文末「实施记录」为准**（实现后补）。

## 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 需求与计划文档 | 提交 |
| P1 | Mac：`RequestRegistry::in_flight_projects()`；`ios_todos.rs` 的 `World` 多查一份在途请求的项目（D14）；测试 | `cargo test`（`ios_todos`、`daemon::request`）、`cargo clippy -D warnings`、`cargo fmt --check` |
| P2 | 弹窗逻辑：`useTodoPopover.ts`；`usePopupCore.ts` 的 `loadTodos` 守卫、`onKeydown` 接入、重置与收起；`shortcut.ts` 的 ⌘T 冲突；i18n | `npx vitest run src/views/popup src/lib`、`npx vue-tsc --noEmit` |
| P3 | 弹窗界面：`TodoPopover.vue`；`PopupNavbar.vue` 的按钮与背板；`popup.css`；集成测试（三种请求的按键隔离、whats-next、确认卡、无项目、切换请求） | 同上 + `npm run build` |
| P4 | iOS 引擎：`RequestTodoComposer`、共享发送函数、`AppStore.todoComposer(for:)`、`MockBackend` 解析在途项目 | `cd ios/Packages/AskHumanKit && swift test` |
| P5 | iOS 界面：`RequestTodoSheet`、`RequestScreen` 的菜单项 / sheet / 成功胶囊、本地化、UI 测试（en + zh-Hans） | `xcodebuild build` → `make-catalog.py` → `xcodebuild test`（模拟器，前台跑） |
| P6 | 装机与真实验证：`machine-health` → `./scripts/install.sh`；真实弹窗里按一遍；真实 daemon 的端到端测试 | 见 §验证清单 |
| P7 | 文档同步（需求 §8），`PROGRESS.md` 只在有遗留项时才写 | 提交 |

## P1 细节（Mac）

- `daemon/request.rs`：`RequestRegistry::in_flight_projects(&self) -> Vec<String>`：锁一次，取 `by_id` 各项的 `show.project` 与 `confirm_by_id` 各项的 `project`，去掉空串与重复，保持稳定顺序（先提问后确认，各按 `seq`）。单测：空登记表、提问与确认各一、重复项目合并、空项目略过。
- `daemon/runtime/ios_todos.rs`：`World { …, in_flight: Vec<String> }`；`World::gather(env, workspaces, agents, in_flight)`；`resolve` 在 `ranked`→`stored`→`synced` 之后链上 `in_flight`，返回的仍是**请求自己的 key 字符串**（与弹窗 `todosAdd(init.project)` 写的是同一个 key，所以两边落在同一份待办里）。`projects()` / `list()` / `apply()` 三个入口从 `state.registry.in_flight_projects()` 取。**不改项目列表的内容与排序**（只是让 id 可反查）。
- 现有测试里 `World::gather` 的调用补第四个参数；新增：①在途项目不在任何候选里时 `apply` 的 `add` 写入该 key、`ack` 为 `ok`；②同一个 id 在没有在途请求时仍是 `notFound/unknownProject`；③在途项目是被隐藏的 workspace 时可反查；④`list_by_id` 也能读到它。
- 不碰 `ipc/mod.rs`、`ios/wire.rs`、夹具；`PROTOCOL_VERSION` 与 `PROTO` 不动。

## P2 细节（弹窗逻辑）

- `src/views/popup/useTodoPopover.ts`：
  - 状态：`todoPopoverOpen`、`todoDraft`、`todoAdding`、`todoAddError`、`todoJustAddedId`；`todoPopoverAvailable = !!projectPath`。
  - `toggleTodoPopover()`：无项目时返回 `false`（调用方改为开窗）；打开时 `await loadTodos()`、`nextTick` 后聚焦输入框、记下此刻的 `document.activeElement`；关闭时还原焦点。
  - `addTodoFromPopover()`：修剪后为空则忽略；`todosAdd(projectPath, text)`；成功→清空草稿、记 `todoJustAddedId`（约 1.2 秒后清除）、`await loadTodos()`；失败→`todoAddError`，草稿保留。并发点击由 `todoAdding` 挡住。
  - `removeTodoFromPopover(id)`：复用 `removeTodo`（它先本地移除再 `todosRemove`），随后 `await loadTodos()`，使 whats-next 选项与已选中状态同步。
  - `handleKeydown(e): boolean`：①组词（`isComposing` / `keyCode 229`）且浮层打开→消费不处理；②⌘/Ctrl+T（无 Alt / Shift）且有项目、查找条没开（`findActive` 为假）、且未被语音快捷键占用→`preventDefault`，开合，消费；③浮层未开→不介入；④浮层已开：⌘/Ctrl+W 放行（返回 `false`）；Esc→关浮层；Enter 在输入框内按「发送快捷键」设置判断提交（`submitWithBareEnter ? !anyMod : primary 且无 Shift / Alt`），应当换行的组合（`enter` 模式下带修饰键、`cmdEnter` 模式下的 Shift / Option+Enter）照弹窗的做法调 `insertNewlineInFocusedTextarea`（作为依赖传入），`cmdEnter` 模式下的裸 Enter 交给浏览器换行；其余全部消费（不 `preventDefault`，字符照常输入）。
  - `reset()`：`renderInit` 里调用（清状态、不清 `projectPath`）；`watch(() => !acceptsInput() || showCancelConfirm || showConfirmCloseWarning)` 为真时 `close()`（不还原焦点）：覆盖 `popupActive` 变 false、合并窗口「关闭全部」对话框挡住（`scope.blocked`）、两个确认模态出现。
- `usePopupCore.ts`：构造 `useTodoPopover({ projectPath, todos, loadTodos, removeTodo, submitWithBareEnter, findActive, speechTaken, insertNewline })`；`loadTodos` 守卫改为 `!projectPath.value || (!request.value && !confirmRequest.value)`；`onKeydown` 在 **`find.handleFindKeydown` 一行之前**（IME 守卫之后）插入 `if (!showCancelConfirm.value && !showConfirmCloseWarning.value && todoPopover.handleKeydown(e)) return;`——放在查找之前，浮层打开时 ⌘F / ⌘G / Esc 才会先被它吃掉；`nativePreviewBlocked` 的表达式并上 `todoPopover.open`（原生预览盖在网页之上，会挡住浮层）；返回值导出浮层成员。**`speechTaken(e)`** = `speech.speechSupported.value && speech.speechShortcut.value && matchShortcut(e, speech.speechShortcut.value)`。
- `src/lib/shortcut.ts`：`shortcutConflict` 增加 `t`（`mod && !alt && !shift && key === "t"`）→ `{ key: "todos", params: { shortcut: primaryShortcutLabel("t") } }`；`shortcut.test.ts` 加断言。
- i18n：`zh.ts` / `en.ts` 的 `popup.nav.todos`（改成带 `{shortcut}`）、`popup.todoPopover.{title,placeholder,hintSubmit,add,adding,empty,openWindow,addFailed}`、`shortcut.conflict.todos`。

## P3 细节（弹窗界面）

- `TodoPopover.vue`：标题行（项目名超长省略）；`<textarea>`（1–4 行自适应，沿用 `textareaAutosize.ts`）；提示行（`{submitKeyLabel} 添加`，`submitKeyLabel` 沿用核心里已有的）与「添加」按钮；列表（`role="list"`，每行文字两行截断、`common.attachmentBadge` 徽标、两段式删除沿用 `TodoSection` 的 `confirmDeleteId` 做法与样式类）；空态；底部「打开待办窗口」按钮（调用 `openTodosWindow` 后收起）。无障碍：容器 `role="dialog"`、`aria-label`，输入框与列表各有标签。
- `PopupNavbar.vue`：待办按钮保持原位置与图标，加 `@mousedown.prevent`、`@click.stop`：有项目→`toggleTodoPopover()`，无项目→`openTodosWindow()`；`title` 用带快捷键的文案。浮层 `v-if="todoPopoverOpen"`，作为 `.nav-actions-btns` 的子元素渲染（该元素不是定位元素，绝对定位会落在 `position: relative` 的 `.nav-actions` 上，于是右缘对齐窗口右缘；`update-wrap` 是相对自己的按钮定位，待办按钮不在最右，不能照搬）：`top: 34px; right: 0; width: min(340px, calc(100vw - 24px)); pointer-events: auto`，沿用 `--surface-overlay` 与阴影；根元素 `role="dialog"`、`@click.stop`、**`@paste.stop`**（全局粘贴监听会把剪贴板图片收进答案）、`@focusout`（`relatedTarget` 非空且在浮层与待办按钮之外 → 关闭且不还原焦点；为空则不处理）。透明背板像 `update-backdrop` 那样放在 `</header>` 之后（`position: fixed; inset: 0; z-index: 40`，`pointer-events: auto`），点击关闭。**注意** `PopupNavbar.sessionTitle.test.ts` 用手写的假上下文挂载导航栏，新成员读不到时必须按「未打开」处理（浮层只在 `todoPopoverOpen` 为真时渲染）。
- `popup.css`：`.todo-popover`、`.tp-*`（标题、输入区、列表、行、高亮渐隐、空态、底部链接），深浅色都用现有变量；`@media (prefers-reduced-motion)` 下去掉高亮动画。
- 集成测试 `PopupView.todoPopover.test.ts`（参照 `PopupView.cancel.test.ts` 的 Tauri mock）：
  1. 普通提问：打开浮层、输入、`Enter`（`popupSubmitKey = enter`）→ 只调用 `todos_add`，不调用任何提交；`cmdEnter` 设置下 `⌘Enter` 同理；空白不提交；`Esc` 只关浮层。
  2. 权限确认卡：浮层打开时 `Enter` / `⌘Enter` / `Esc` 都不触发 `confirm` 提交或 `requestConfirmClose`；关掉后 `Esc` 恢复原含义；确认卡里浮层能列出 `todos_list` 的结果。
  3. whats-next：添加后 `todos_list` 返回新列表，选项里出现新条目且原选中不丢；删除同理。
  4. 无项目：点按钮调用 `open_todos`，没有浮层，⌘T 无事发生。
  5. 合并窗口（`PopupInboxView`）：切换请求收起浮层，草稿各自保留。
  6. 取消确认模态出现时浮层收起，⌘T 不介入；合并窗口的「关闭全部」对话框（`scope.blocked`）挡住时同样收起。
  7. 语音快捷键被设成 ⌘T 时，⌘T 仍触发语音、不开浮层。
  8. 浮层打开时 ⌘F 不会打开查找条、Esc 先关浮层；查找条已开时 ⌘T 无反应。
  9. 浮层里粘贴图片：`paste` 事件到不了 `window`（答案里不会多出附件）；粘贴文字照常。
  10. Tab 把焦点移出浮层后浮层收起，之后 Enter 恢复页面原有含义；点浮层里的空白处（焦点落到 `body`）不收起。
  11. 浮层打开期间 `nativePreviewBlocked` 为真，关闭后恢复。

## P4 细节（iOS 引擎）

- `AHCore/TodoAddCall.swift`（共享的发送函数）：`static func send(backend:, macId:, macName:, projectId:, body:, clientMsgId:) async -> Result`，包含「回包丢了就用同一个 id 再发一次」与错误映射（`unreachable` / `timedOut` / `refused` / `notSaved`）；`QuickTodoComposer.send()` 改为调用它（**纯重构，`QuickTodoComposerTests` 必须保持全绿**）。若抽取牵动过多，改为在新 Composer 里复制这一段，并在实施记录里写明。
- `AHCore/RequestTodoComposer.swift`：`@MainActor @Observable`；`init(backend:, macId:, projectKey:, projectName:)`，`projectId = TodoProjectId.of(key: projectKey)`；`text`、`phase`、`failure`、`update(mac: MacState?)`、`unavailable`（`macOffline` / `macNeedsUpdate` / `tooLong`；Mac 已不在配对里按 `macOffline`）、`canSend`、`send() async -> Bool`；`notFound` ack → `failure = .refused("unknownProject")`；成功清空文字。
- `AppStore`：`todoComposers: [String: RequestTodoComposer]`（内存）；`todoComposer(for item: InboxItem) -> RequestTodoComposer?`（项目 key 为空返回 `nil`）；`closed(id:)` 里与 `composers` 一同清理已结束的请求。
- `AHMock/MockBackend+Todos.swift`：`applyTodo` 在 `todoLists` 里找不到该项目时，若它是该 Mac 上某条在途请求的项目（`TodoProjectId.of(key:)` 相等）就当场建一份空列表并继续（对应 D14）；其他仍回 `unknownProject`。
- 单测 `ios/Packages/AskHumanKit/Tests/AHAppTests/RequestTodoComposerTests.swift`：目标 id 现算且与 `TodoProjectId` 一致；不可用原因各一；成功后 `todoOps` 里恰有一条 `add`（无 ⚡、`id` 正确）并清空文字；`loseNextTodoAnswers` 丢回包后重发同一个 `clientMsgId`、Mock 只存一条；被拒绝 / 项目已不在保留文字且原因正确；草稿按请求缓存、请求结束后清理；Mac 中途掉线后 `unavailable` 变化。

## P5 细节（iOS 界面）

- `Views/Request/RequestTodoSheet.swift`：`NavigationStack` + `Form`：只读项目行（文件夹图标、项目名、Mac 名，`request-todo-project`）→ `TextField("New todo", axis: .vertical)`（`lineLimit(3...10)`、`@FocusState`、`request-todo-field`）→ 脚注显示 `failure` 或 `unavailable`（`request-todo-note`）；工具栏「Cancel」（`request-todo-cancel`）与「Add」/ 转圈（`request-todo-send`）；`.presentationDetents([.medium, .large])`、`.interactiveDismissDisabled(sending)`；`.onChange(of: store.macs)` 里刷新 `composer.update(mac:)`。成功回调把项目名交给 `RequestScreen`。
- `RequestScreen.swift`：`@State addingTodo`、`@State addedProject: String?`；`hasMore` 改为「有项目 ∨ 原条件」；`moreMenu` 里在最前面加 `Button("Add todo…", systemImage: "text.badge.plus")`（只在有项目时），其余两项沿用；`.sheet(isPresented:)`；`.overlay(alignment: .top)` 的成功胶囊（`checkmark.circle.fill` + 「Added to \(project)」，`regularMaterial` 胶囊，`todo-added-pill`，2 秒后自行消失、不拦截触摸）；`.onChange(of: id)` 里收起 sheet。
- 文案：只新增「Add todo…」（中文「添加待办…」）；「New todo」「Add」「Cancel」「Added to %@」与快速添加的各条失败 / 不可用话术都复用。先 `xcodebuild build -derivedDataPath build/DerivedData`，再跑 `ios/Tools/make-catalog.py`（缺中文会失败）。
- 新增 Swift 文件后在 `ios/` 下 `xcodegen generate`（`AskHuman.xcodeproj` 不入库，由 `project.yml` 生成）。
- UI 测试（Mock，`ios/AskHumanUITests/AskHumanRequestTodoUITests.swift`，en + zh-Hans）：有项目的请求菜单里有「添加待办…」、没有项目的没有；打字→添加→胶囊出现、sheet 收起、Mac 一侧 `todoOps` 记到那条；Mac 离线时 sheet 说明原因且「添加」不可点；Mac 太旧同理；取消后再打开文字还在；确认卡页面同样可用。

## P6 验证清单

1. `node scripts/machine-health.mjs`；退出码 2 时先告诉你、等你腾出内存再编译。
2. `./scripts/install.sh`；之后的提问都走新装的 `AskHuman`。
3. **真实弹窗里手动按一遍**（我用 `AskHuman` 自己发提问；能自动化的部分用 computer-use 截图自查）：
   - 普通提问：按钮与 ⌘T 开合；添加两条；Esc 只关浮层；`Enter` / `⌘Enter` 在浮层里不提交答案；删除两段式；「打开待办窗口」带预选项目。
   - whats-next（`AskHuman --whats-next`）：添加后立刻出现新选项。
   - 权限确认卡：用一次真实的权限请求（Claude / Codex hook）或测试夹具，验证浮层里 Enter / Esc 不批准也不拒绝。
   - ⌘T 确实到达网页（没被系统菜单吃掉）。
4. 真实 daemon 端到端：`AHE2ETests/TodoE2ETests` 加一条——在一次性 Mac 上造一条 `waiting` 态 Agent 的在途请求（项目不在任何候选里），手机引擎对它的项目 id 发 `todo.op add`，期望 `ok` 且 `todos.json` 里有这条。
5. iOS：`swift test`；模拟器 `xcodebuild test`（`AskHumanRequestTodoUITests`、`AskHumanTodosUITests`、`AskHumanDeveloperUITests` 里与待办相关的，en + zh-Hans）。**TestFlight 不出**，除非你要。
6. `cargo test`（相关模块）、`cargo clippy -D warnings`、`cargo fmt --check`、`npx vitest run`、`npx vue-tsc --noEmit`。

## 提交（Conventional Commits，英文、祈使句）

1. `docs(todos): spec and plan for adding todos from a request`
2. `fix(ios): resolve in-flight request projects for phone todo writes`（带 `Release-Note: skip`，它是下面第 4 条的前置）
3. `feat(popup): add project todos from a navbar popover (⌘T)`
4. `feat(ios): add a todo to a request's project from the request page`
5. `docs(todos): sync overview, specs and wiki with request-page todo adding`

## 风险

1. **按键泄漏到确认卡**（最大风险，需求 §6.2）：靠 `handleKeydown` 的模态消费 + 集成测试钉住；装机后在真实确认卡上再按一遍。
2. **`PopupNavbar.sessionTitle.test.ts` 的假上下文**：新成员缺省时浮层必须视为关闭，否则旧测试崩。
3. **`loadTodos` 在确认卡里开始工作**：只多一次读文件，且确认卡正文不显示待办区；`PopupView.cancel.test.ts` 等已把 `todos_list` mock 成返回 `[]`，应无影响，P2 跑全套 popup 测试确认。
4. **合并窗口里多个请求页同时挂载**：每页各有一份核心与浮层状态；只有 `popupActive` 的页响应按键，其余页浮层强制收起。
5. **抽取共享发送函数牵动 `QuickTodoComposer`**：它有 36 条单测兜底；不顺就复制，不硬抽。
6. **机器很忙**：全量编译与模拟器测试在这台机器上可能要十几分钟；开发中只跑定向测试，阶段结束跑一遍完整的。
7. **⌘T 被系统吃掉**：不太可能（弹窗没有自定义原生菜单加速键），但 P6 必须在真机上确认；若被吃掉，回来找你换键，不自己换。

## 开工前要定的事

- 工作方式：在主检出里建分支 `feat/todo-quick-add`、做完合并回 main（我的建议：只有这一个任务在动、不需要并行 worktree 与 Dev Instance；装机验证直接装到日常环境，按 `AGENTS.md`），还是开 worktree + Dev Instance 隔离。
