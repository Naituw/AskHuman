# 计划：iOS 开发者模式（浮动按钮 + 快速添加待办）

> 需求：`docs/specs/ios-developer-mode.md`（决策编号 D1–D19 指其 §3）。状态：2026-11-02 起草；用户已审阅通过，P1–P5 已实现（见「实现记录」），P6：2026-11-03 用户同意后已出内测包 `0.1.0 (5)`，**等用户试用反馈**。2026-11-03 起按钮与菜单改用系统 `UIMenu`（需求 D18、D19）：下文已按新做法改写，自绘版的做法与它的验证结果只留在「实现记录」里。
> 分支：直接在 `main` 上按阶段提交。改动只在 `ios/` 与 `docs/`：不碰 Rust、不需要 Dev Instance、不跑 `./scripts/install.sh`。
> 工程由 XcodeGen 生成（`*.xcodeproj` 不入库）：每次新增 App 侧文件后先 `cd ios && xcodegen generate`。
> 开发开始后，任何方案改动都先经 AskHuman 与用户确认；用户的审阅意见会追加到需求文档末尾（标注为反馈意见）。

## 方案概述

- 业务逻辑放进本地 Swift Package `AHCore`（不依赖 UIKit，能在 macOS 上 `swift test`，CI 的 `ios-kit` 作业会跑）：开发者状态、按钮位置几何、快速添加的 Composer。
- App 层（`ios/AskHuman/`）只做壳：两个独立的 `UIWindow`（按钮窗口、快速添加窗口）加几个 SwiftUI / UIKit 视图，以及 Macs 设置里的开关。
- 窗口宿主沿用 `ArrivalSceneHost` 的做法：根视图 `.background` 里放一个零尺寸的 `UIViewRepresentable`，借它找到真实 `UIWindow` 与 `UIWindowScene`，再自建窗口；开关关闭时窗口全部拆掉。
- 写入沿用现有通路：`RemoteBackend.todoProjects(macId:)` 列项目，`RemoteBackend.todoApply(macId:operation:)` 发 `todo.op add`；`MockBackend` 已全部实现，UI 测试直接用。**没有新的线消息、能力位或 Mac 端改动。**
- 约束：iPhone、iOS 17+、竖屏、单场景（`project.yml` 已是 `UIApplicationSupportsMultipleScenes: false`、仅 Portrait）；Swift 5 语言模式 + 严格并发。

## 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 需求与计划文档 | 用户审阅通过后提交 |
| P1 | AHCore：开发者状态、位置 / 吸附几何、`QuickTodoComposer` 与单测 | `cd ios/Packages/AskHumanKit && swift test` |
| P2 | App：Macs 设置里的开发者开关、启动参数、按钮窗口（按钮与系统菜单 / 拖动 / 提示 / 键盘避让） | `xcodebuild build` + 模拟器目测截图 |
| P3 | App：快速添加窗口、页面、项目选择、焦点保存与恢复 | `xcodebuild build` + 模拟器目测 |
| P4 | Mock 场景接线与 UI 测试（含手势、盖住 sheet、焦点恢复） | `xcodebuild test`（模拟器，前台跑） |
| P5 | 本地化（en + zh-Hans）、`overview-ios.md`、`PROGRESS.md` | 脚本无缺项 + 提交 |
| P6 | 真机一轮（经用户同意后用 `ios-testflight` 技能出内测包） | 用户在场 |

## P1：AHCore

文件放在 `Sources/AHCore/`，平铺命名 `Developer*.swift`、`QuickTodo*.swift`；测试放 `Tests/AHAppTests/`（那里已有 `TodosStoreTests`、`Support.swift` 与 `MockBackend` 的用法）。几何只用 CoreGraphics 类型与自定义的小结构（安全区四边、键盘上沿等），不引入 UIKit，保证 CI 的 macOS 上能编译。

**`DeveloperTools`**（`@MainActor @Observable`，由 `AppStore` 持有，做法与 `todos` 完全一致）

- 接线：`AppStore.init(backend:drafts:defaults:)` 里在 `todos = TodosStore(…)` 旁创建 `developer = DeveloperTools(backend:defaults:)`（`defaults` 共用，`@ObservationIgnored`）；`AppStore` 处理状态更新处现有的 `todos.update(macs:foreground:)` 旁加 `developer.update(macs:)`，让 Composer 总能看到当前的 Mac 列表与连接状态。
- `enabled`：持久化键 `ah.developerMode`，默认 `false`（`didSet` 写盘，与 `autoAdvance` 同一写法）；`AppStore.developerMode` 转发读写，方便设置页用 `$store.developerMode` 绑定。
- `placement`：持久化键 `ah.dev.placement`（JSON）；缺失或损坏回到默认。
- `toast`：`id` + 文案参数；由 UI 层展示 1.8 秒后清除。
- `quickTodoPresented`：快速添加页是否应当显示；`presentQuickTodo()` / `dismissQuickTodo()` 改它。
- `quickTodo`：`QuickTodoComposer`。
- `invoke(_ item: DeveloperMenuItem)`：v1 只有 `.quickTodo` → `presentQuickTodo()`。

**`DeveloperMenuItem`**：枚举，v1 一项 `.quickTodo`；带稳定的字符串 id，标题与图标在 App 层按本地化给；菜单本身是系统的 `UIMenu`，这里只列条目。

**`DeveloperPlacement`**（`Codable`、`Equatable`）：`edge`（`.leading` / `.trailing`）+ `y`（0…1）。默认 `(.trailing, 0.4)`。纯函数几何都以一个 `DeveloperLayout`（屏幕尺寸、安全区四边、可选的键盘上沿）为输入，度量取自 `DeveloperMetrics`（按钮 44、贴边 6、纵向留白 8、空闲 2.5 s → 50%、提示 1.8 s），数值见需求 §5.1：

- `restingCenter`：x 由侧边决定（离边缘 贴边间距 + 半径），y 在 [安全区上沿 + 留白 + 半径, 安全区下沿 − 留白 − 半径] 内按比例取。
- `displayedCenter`：在 `restingCenter` 基础上避让键盘：按钮下缘 + 8 超过键盘上沿时上移到键盘上沿之上 8，且不低于可用上沿；不改 `placement`。
- `snapped(from dragCenter)`：x 小于屏宽一半 → `.leading`，否则 `.trailing`；`y` 夹进可用范围后换算成比例。
- `clampedDragCenter`：拖动中按钮中心夹在「离边缘 贴边间距 + 半径」的矩形内。

**`QuickTodoComposer`**（`@MainActor @Observable`；依赖 `RemoteBackend`、`defaults`，Mac 列表由 `update(macs:)` 推入）。复用现有零件，不另造一套：项目行用 `TodoListedProject`，搜索用 `TodoRows.matches(_:query:)`，文字校验用 `TodoText.validate`，写入消息用 `TodoMutation`。

- `target: QuickTodoTarget?`：`macId`、`macName`、`projectId`、`projectName`；持久化键 `ah.dev.quickTodoTarget`（JSON）。目标 Mac 已不在配对列表里时视为没有目标（不删存档，重新配对同一台 Mac 时仍可用）。
- `text`：只在内存；`send` 成功才清空。
- `groups`：项目选择的数据，按 Mac 一组；`loadProjects()` 对每台**在线且 `caps.todos`** 的 Mac 并行调一次 `todoProjects(macId:)`，组内顺序保持 Mac 给的顺序，**不过滤**仅云端 / 仅本机的项目（需求 §6.2），同一个 iCloud 项目在每台开了同步的 Mac 下各出现一次、不去重；离线 / 太旧 / 超时的 Mac 各自带原因；`query` 过滤。
- `choose(_ project: TodoListedProject)`：记下目标并持久化。
- `unavailable: QuickTodoUnavailable?`：由文字校验（空 / 超过 `Wire.todoTextMaxChars`）、目标、目标 Mac 的 `link` 与 `caps.todos` 推导，对应需求 §6.1 的四行文案。
- `send() async`：状态 `editing → sending → (成功 | 失败)`。用 `TodoMutation.add(localId:text:auto: false).operation(clientMsgId:projectId:)` 拼出 `todo.op`（因此不带 `auto`），每次发送用新的 `clientMsgId`，调 `backend.todoApply(macId:operation:)`；`BackendError.timedOut` 时用同一个 id 再问一次（与 `TodosStore.send` 一致）；两次都超时后记下这次尝试（id + 文字 + 目标），用户再点「添加」时若三者没变就沿用这个 id。结果映射成 `QuickTodoFailure`：`macOffline` / `unknownMac` → `unreachable`；两次都超时 → `timedOut`；ack `notFound`（原因 `unknownProject`）→ `projectGone`，同时清掉已存的目标，视图看到 `projectGone` 就进入项目选择（已不存在的项目不该留作目标）；ack `rejected` 或其他抛错 → `refused(原因)`；ack 其余非 `ok` → `notSaved`。成功后清空文字并让 `DeveloperTools` 出提示。
- 没有离线队列：`unavailable != nil` 时 `send()` 直接返回。

**单测（`AHAppTests`）**

- `DeveloperPlacementTests`：默认值与 JSON 往返、损坏数据回默认；`restingCenter` 在安全区边界与不同侧边；`snapped` 的左右分界与纵向夹取；`displayedCenter` 避让键盘且不改 `placement`；拖动中的夹取。
- `QuickTodoComposerTests`（`MockBackend`）：校验与 `unavailable` 各情形；`loadProjects` 只问在线且有能力的 Mac、分组与搜索；目标持久化与 Mac 被取消配对；`send` 成功 / 超时重发同一 `clientMsgId` / Mac 离线 / `unknownProject` / `failed` / `rejected`；不带 `auto`；失败保留文字、成功清空；`ok` 之后 `MockBackend` 里该项目多了一条。

## P2：App 开关、窗口宿主与按钮

**设置**（`Views/Macs/MacsScreen.swift`）：在「General」分区之后加「Developer」分区：一个 `Toggle("Developer mode")` 绑定 `$store.developerMode`，标识 `developer-mode-toggle`，脚注说明「按住按钮出菜单、滑到条目松手；按下后立刻拖动可移动按钮」。

**启动参数**（`App/AppEnvironment.swift` 的 `LaunchOptions`）：`-AHDeveloperMode YES|NO` 强制开 / 关（与 `-AHAutoAdvance` 同一做法，UI 测试要固定值）。

**`DeveloperOverlayHost`**（`Views/Developer/`，`UIViewRepresentable`）：放在 `RootView` 的 `.background` 里、`ArrivalSceneHost` 旁边；入参是 `store.developer` 的 `enabled`、`placement`、`toast`、`quickTodoPresented` 与回写闭包。`updateUIView` 同步状态；`dismantle` 拆窗口。多次 `updateUIView` 不得重复建窗口。

**`DeveloperButtonWindow`**（`UIWindow` 子类）

- `canBecomeKey = false`；只设 `isHidden = false`，**绝不调用 `makeKeyAndVisible`**。
- 层级 = 主窗口 + 2（到达动效窗口是 +1，所以按钮盖在飞行的气泡之上）；frame 跟随主窗口。
- `hitTest`：命中按钮才返回视图，否则返回 `nil`，让触摸落到下面的窗口（菜单是系统呈现的，不在这个窗口里）。
- 无障碍保持开启（UI 测试要读得到）。

**`DeveloperButton`**（`UIButton` 子类）与 **`DeveloperOverlayView`**（窗口的内容）

- 外观：44 pt 圆形，SF Symbol `hammer.fill`。iOS 26 起是 `UIButton.Configuration.glass()`（Liquid Glass）；更早是 `UIBackgroundConfiguration.visualEffect`（系统薄材质）+ 细描边 + 阴影。2.5 秒无触摸降到 50%（由 `DeveloperOverlayView` 管）。
- 菜单：`menu = UIMenu(children: DeveloperMenuItem.allCases.map { UIAction(…) })`，`showsMenuAsPrimaryAction = true`；条目被选中时回调 `onInvoke`。按住滑动、高亮、触觉与 VoiceOver 都是系统的。
- 拖动：`DeveloperOverlayView` 在按钮上加一个 `UIPanGestureRecognizer`（最多一指）。开始：记下按钮中心，放大 1.12 倍；移动：按抓取时的偏移跟手，用 `clampedDragCenter` 夹在安全区内；结束：`snapped` 出新的位置并保存，弹簧吸附；取消：回到开始前的位置，不保存。「减少动态效果」打开时吸附改为淡入淡出。
- 触摸回调：按钮的 `touchesBegan / Ended / Cancelled` 只用来唤醒与重新计时空闲降透明（菜单与拖动都会让触摸以 cancelled 结束，所以不能指望 `Ended`）。
- 提示视图：顶部安全区下一个胶囊（对勾 + 「已添加到 <项目名>」），iOS 26 起是 `UIGlassEffect`，更早是厚材质；1.8 秒淡出，同时 `UIAccessibility.post(.announcement)`；标识 `dev-toast`。
- 布局更新：监听窗口 bounds / 安全区变化与 `keyboardWillChangeFrame` / `keyboardWillHide`，用 `displayedCenter` 重排；拖动中不重排。
- 无障碍：按钮标签「Developer tools」、标识 `dev-button`；菜单条目由系统给出（没有 `accessibilityIdentifier`，UI 测试按标题找）。

## P3：快速添加窗口与页面

**`QuickTodoPresenter`**（`Views/Developer/`，由宿主的 coordinator 持有）

- `present()`：先记下主窗口当前的第一响应者（用 `UIApplication.shared.sendAction(_, to: nil, from: nil, for: nil)` 取出，弱引用保存）；再建 `QuickTodoWindow`（`canBecomeKey = true`，层级 = 主窗口 + 3，frame 跟随主窗口），根控制器是透明的 `UIViewController`；窗口 `makeKeyAndVisible`（这里需要键盘，允许成为 key）；从根控制器 `present` 一个 `UIHostingController(rootView: QuickTodoView)`，样式 `.pageSheet`，detents `[.medium(), .large()]`，显示抓手。
- `dismiss` / 下拉收起 / 成功后：拆窗口，`mainWindow.makeKey()`，若记住的第一响应者仍在视图层级里就 `becomeFirstResponder()`；清空 `quickTodoPresented`。**所有关闭路径必须走同一个收尾函数**。
- 注入 `environment(store)`；开关被关闭时同步关闭本页。

**`QuickTodoView`**（SwiftUI，`NavigationStack`）

- 顶栏：左「Cancel」、中「Quick todo」、右「Add」（`unavailable != nil` 或发送中禁用，发送中显示转圈）；标识 `quick-todo-cancel` / `quick-todo-send`。
- 项目行：第一行是项目名（带同名区分后缀），第二行小字是 Mac 名，无目标时是强调色的「Choose a project」；点按推入项目选择；标识 `quick-todo-project`。
- 文本框：`TextField("New todo", axis: .vertical)`、`lineLimit(3...10)`、出现即聚焦，发送中只读；标识 `quick-todo-field`。
- 发送中：「Cancel」置灰、`.interactiveDismissDisabled(true)`，转圈标识 `quick-todo-sending`（Mac 的拒绝若在页面走了之后才到，就没处可说）。
- 说明行：`unavailable` 的原因或失败原因（红色），标识 `quick-todo-note`。
- 首次（无目标）时，页面一出现就推入项目选择（不带转场动画）；`projectGone` 时也推入；选定后弹回并聚焦文本框。
- 成功：`DeveloperTools` 设置提示 → 窗口收尾 → 按钮窗口显示提示。

**`QuickTodoProjectPicker`**：`List` 按 `groups` 分段（段头 = Mac 名；离线 / 太旧的段没有行、原因写在段脚注，无应答的段一行「Did not answer」加可重试的「Try again」），行 = 与待办页同款（图标、项目名、后缀、仓库地址、待办数，当前目标带对勾）；`searchable`；加载中显示进度；选中调用 `choose` 并返回；标识 `quick-todo-picker`、`quick-todo-project-<id>`、`quick-todo-loading-<macId>`、`quick-todo-retry-<macId>`。

**文案**放 `Design/DeveloperTexts.swift`（不可用原因与失败原因的映射，与 `TodoTexts.swift` 同风格）。已有的句子直接复用：`notSaved`、`refused(原因)` 用 `TodoNotice.message`（含「The Mac could not save that.」与各拒绝原因）；需求要求带 Mac 名的句子新增：连不上、没有应答、已不在该 Mac 上、请先更新该 Mac 上的 AskHuman。

## P4：Mock 与 UI 测试

- 数据：沿用 `demo` 场景的两台 Mac（Home MacBook Pro 走 iCloud、Office iMac 本地；后者有两个同名项目「AskHuman · work / playground」，正好覆盖同名后缀）；`offline` 场景（两台 Mac 都连不上）与 `outdated` 场景（两台 Mac 都没有待办能力）测项目选择里的原因分组；`bothSync` 场景测同一 iCloud 项目在两台 Mac 下各出现一次。不新增 Mock 数据，除非用例发现缺项再补。「目标 Mac 掉线」没有运行时开关可在 UI 测试里翻转，由 P1 的 `QuickTodoComposerTests` 用 `MockBackend.setLink(_:for:)` 覆盖。
- `AskHumanUITests` 新增用例（前台跑，约定见 `overview-ios.md` 的验证表）：
  1. 开关：默认没有 `dev-button`；Macs 设置里打开后出现、关闭后消失；`-AHDeveloperMode YES` 直接出现。
  2. 手势：用 `XCUICoordinate.press(forDuration:thenDragTo:withVelocity:thenHoldForDuration:)`：起点取 `dev-button` 的中心，终点取菜单行的位置。手势开始时系统菜单还没有出来，所以先用一次轻点打开菜单、读出行的 frame（行没有标识，按标题找）、点空白处关上；然后按住 1 秒、慢速滑到那里并停一下再松手 → 快速添加页出现；在菜单外松手 → 没有页面，按钮没动。
  3. 轻点：点按钮 → 菜单出现；等一会儿还在；点菜单外（落在一个项目行上）→ 收起，且那一点没有触发下面的控件；下一次点同一个行才进入项目页。
  4. 拖动：把按钮拖到屏幕左侧 → 吸附到左边；位置记住（重启后同一侧）；拖动期间没有菜单。
  5. 盖住 sheet：先打开 Agents 页的新建任务 sheet，再用按钮添加待办 → 成功后新建任务 sheet 仍在、内容未丢。
  6. 快速添加：首次进入项目选择 → 搜索 → 选「AskHuman · work」→ 输入 → 添加 → 页面收起、出现 `dev-toast`；再打开时直接显示目标并聚焦文本框；在待办页该项目里能看到新条目。
  7. 不可用：`offline` 场景下打开快速添加页，项目选择里两台 Mac 都写明连不上、没有可选项，「添加」置灰并提示先选项目；`outdated` 场景同理，写明要更新；输入的文字不丢。
  8. 焦点恢复：打开一个请求页并聚焦回复输入框、输入几个字，用按钮打开并取消快速添加页 → 回复输入框重新获得焦点，文字仍在。
  9. 键盘避让：键盘弹出时按钮不在键盘区域内。
  10. 与到达动效共存：用 `-AHArrival` 与 `-AHArrivalDelay` 让新请求在快速添加页打开期间到达 → 页面与键盘不被打断、已输入的文字不丢，关闭页面后新请求已在收件箱里。
- 结束时按需求 §10 逐条自查，并存几张截图（`TEST_RUNNER_AH_UI_SHOTS=<dir>`：按钮常态 / 菜单 / 快速添加页 / 项目选择；浅色深色，`TEST_RUNNER_AH_UI_LANG=zh-Hans` 看中文）。
- 不新增真实 daemon 端到端用例：写入通路与待办页相同，`TodoE2ETests` 已覆盖 `add`。

## P5：本地化与文档

- 本地化：先 `xcodebuild build -derivedDataPath build/DerivedData …`，再跑 `ios/Tools/make-catalog.py`；缺中文翻译会让脚本失败（`overview-ios.md` 的约定）。新增文案至少包括：开关与脚注、菜单条目、快速添加页各处、项目选择各段原因、所有不可用 / 失败原因、提示。
- `docs/overview-ios.md`：「手机能做的事」补一句开发者模式；iOS App 一节说明 `Views/Developer/`；「Mock 模式启动参数」那一行补 `-AHDeveloperMode YES|NO`；「不变量与注意点」补：按钮窗口不得 `makeKeyAndVisible`、窗口层级（主 +1 到达动效 / +2 按钮 / +3 快速添加）、快速添加窗口收尾必须恢复 key 与第一响应者、按钮度量集中在 `DeveloperMetrics`（拖动与菜单的分工是系统与 UIKit 的，没有手势常量）。需求文档的决策记录（§3）只追加、不改旧条。
- `docs/PROGRESS.md`：新增「待验收：iOS 开发者模式」一节，写已完成与自动化覆盖、以及真机一轮要看的项（见 P6）。
- 提交：实现提交 `feat(ios): …` 带 `Release-Note: skip` 脚注；文档提交 `docs(ios): …`。每阶段一个提交。

## 实现记录（P1–P5）

与上面写的计划不同的地方（面向使用的那部分在需求 §11）：

- **窗口宿主分三层**：`DeveloperOverlayAnchor`（放进 `RootView` 背景，只读 `store.developer`，免得整个标签视图跟着重算）→ `DeveloperOverlayHost`（零尺寸 `UIViewRepresentable`）→ `DeveloperOverlayController`（建与拆两个窗口，管键盘与场景通知）。位置、拖动与键盘避让在 `DeveloperOverlayView`，按钮 `DeveloperButton` 与提示 `DeveloperToastView` 是 `DeveloperButtonViews.swift` 里的两个 UIKit 类（自绘版的菜单行、菜单与触觉类已在改用系统菜单时删除，见下一节）。
- **快速添加页**：发送中不能取消、不能下拉收起；文本框默认三行；项目行两行；超时后再发沿用同一个 `clientMsgId`；`notFound` 清掉已存的目标，由 `projectGone` 驱动视图进入项目选择（上文各处已改）。
- **无障碍**：浮动按钮的标签是「Developer tools」，菜单是系统的 `UIMenu`（VoiceOver 由系统处理）；菜单条目没有标识，UI 测试按标题找。
- **图片查看器**（`ImageGallery.swift`）：每页的尺寸改取外层 `GeometryReader`（`.ignoresSafeArea()`）。这是 P4 的 UI 测试在「键盘盖在全屏查看器上」时发现的：竖直方向的 `containerRelativeFrame` 在容器被另一个窗口的键盘反复改动时让分页永远排不稳，App 主线程被布局占满。试过的、**没有**用的办法：给滚动视图或 `NavigationStack` 外面加 `.ignoresSafeArea(.keyboard)`、只忽略容器的安全区、`frame(maxHeight: .infinity)`（会让版面差 17 pt）。图片因此比以前高约 11 pt，其余像素不变。
- **UI 测试**：`AskHumanDeveloperUITests` 共 17 条，覆盖开关、手势（按住滑动、轻点、在菜单外松手、拖动吸附与不出安全区）、盖住 sheet 与它的键盘、盖住全屏图片查看器、快速添加全流程、记住目标与换项目、焦点恢复（请求页的回答框、sheet 里的输入框）、离线与太旧的 Mac、到达动效共存。XCUITest 的几个坑已记在 `overview-ios.md`。
- **验证结果（自绘菜单版，已被下一节取代）**（2026-11-03，模拟器 iPhone 16 / iOS 18.5）：Swift 包全量 314 条无失败（`DeveloperPlacementTests`、`DeveloperPressTests`、`DeveloperToolsTests`、`QuickTodoComposerTests` 在内；要真实 daemon 与本地 broker 的 43 条端到端用例照例跳过）；`AskHumanDeveloperUITests` 英文与中文各整班跑过一次，17 / 17 全过（英文第一次整班跑时有 3 条因测试进程被系统杀掉而失败，单独重跑 3 / 3 通过、再整班跑 17 / 17 通过，属环境偶发）；中文截图目测过设置分区、项目选择的原因脚注与提示；`make-catalog.py` 无缺项，再跑一遍没有新的差异。开发者模式关着时，其余 134 条 UI 测试 127 条通过、3 条跳过、4 条失败，这 4 条在功能开发之前的提交上同样失败或偶发（对照方法与名单见 `docs/PROGRESS.md` 的开发者模式一节）。

## 实现记录（改用系统 UIMenu，2026-11-03）

需求 D18、D19；用户的问题与选择原话见需求 §11。这里记做法与验证。

- **起因与探针**：用户看过 Liquid Glass 截图后问系统 `UIMenu` 能否替代自绘菜单。先做了一次不提交的探针（同一个按钮挂系统菜单与拖动手势；XCUITest 加 `simctl io screenshot`；iOS 27.0 / iPhone Air 与 iOS 18.5 / iPhone 16 的结果一致）：按住后菜单约 75–180 ms 出现；按下后立刻移动，拖动识别器在 64–108 ms 内开始、菜单不出现（快、默认、慢三种速度都一样）；按住 → 滑到条目 → 松手即触发，在菜单外松手菜单保持打开；菜单盖在 sheet 之上时键盘不收，快速添加页打开、关闭后 sheet 里的输入保留、键盘回来；菜单盖在全屏图片查看器之上，点外面收起且不穿透；iOS 26 起玻璃按钮变形成菜单（期间按钮隐藏），iOS 18 是按钮下方的经典菜单、按钮仍在。
- **做法**：AHCore 删 `DeveloperPress.swift`（状态机与 `DeveloperPressTuning`）、`DeveloperPressTests.swift`，`DeveloperGeometry.swift` 删 `DeveloperMenuLayout`、`DeveloperHitMap` 与菜单度量，`DeveloperPlacementTests` 去掉菜单几何的用例；App 删 `DeveloperMenuRowView`、`DeveloperMenuView`、`DeveloperHaptics`，`DeveloperButtonView` 换成 `UIButton` 子类 `DeveloperButton`（iOS 26 起 `.glass()` 配置，更早 `UIBackgroundConfiguration.visualEffect` + 描边 + 阴影），`DeveloperOverlayView` 去掉按住状态机、命中图、触觉、收起层与无障碍自定义动作，改为按钮上的 `UIPanGestureRecognizer` 拖动、`hitTest` 只认按钮；提示条 iOS 26 起用 `UIGlassEffect`（`cornerConfiguration = .capsule()`），更早保持厚材质。共净减约 770 行。
- **UI 测试的改动**：菜单条目按标题找（`app.buttons`，label 等于「Quick todo」/「快速待办」）；按住滑动那一条先用一次轻点读出条目的位置，再 `press(forDuration:thenDragTo:withVelocity:thenHoldForDuration:)`；「点菜单外面」改用屏幕上方一块没有菜单的空地（`spotAwayFromTheMenu`）——iOS 18 的菜单在按钮下方，屏幕中心可能落在菜单里，点到条目上去（到达动效那条第一次就这样在 18.5 上失败了）；去掉「菜单朝内侧、与按钮同高」的几何断言，换成「菜单打开、整个在屏幕内」；在菜单外松手的那条不再断言菜单是否关闭（系统的决定），只断言没有选中任何条目、按钮没动。条数不变，17 条。
- **验证结果**（2026-11-03）：Swift 包全量 290 条无失败（比自绘版少的 24 条是 `DeveloperPressTests` 与菜单几何的用例；要真实 daemon 与本地 broker 的端到端用例照例跳过）；`AskHumanDeveloperUITests` 在 iOS 27.0 / iPhone Air 上英文与中文各 17 / 17 通过；在 iOS 18.5 / iPhone 16 上英文 16 / 17，失败的是上面说的到达动效那条；那一条改了点按位置之后，在两个系统上都单独重跑通过（整班没有为它再各跑一遍）；目测过 iOS 27.0 浅色与深色（玻璃按钮、玻璃提示条、菜单从按钮长出来）与 iOS 18.5（毛玻璃按钮、经典菜单、提示条）的截图；`make-catalog.py` 无变化（没有新增或删除文案）。开发者模式关着时按钮窗口根本不建，其余 UI 测试没有重跑——它们走的代码这次没有动。

## P6：真机一轮（需要用户在场，先问再做）

自动化验证不了的：系统菜单的弹出延迟与拖动的分工手感（按下后立刻划向菜单会变成拖按钮，这是设计内的代价）、玻璃的观感、真机键盘与窗口焦点恢复、按钮盖在 Safari / 图片查看器 / 系统弹窗之上的表现、触觉、VoiceOver 可用性。计划：问过用户后用 `ios-testflight` 技能出一个内测包装到手机，用户试用并反馈；改手势规则（例如拖动识别的灵敏度）前先经 AskHuman 确认，结果写进 `PROGRESS.md`。

2026-11-03 用户在系统菜单改造完成后选了「现在做」：从 main `2295390a` 出了内测包 `0.1.0 (5)`，Apple 状态 `VALID`，已进 `AskHuman Internal`（记录见 `docs/plans/ios-channel.md`）。接下来等用户装上试用并反馈。

## 风险与应对

- **窗口 + 键盘 + 焦点**是最大的不确定项：先在 P3 用模拟器做最小验证（只含窗口与文本框，覆盖「在请求页输入时触发」），失败则停下来与用户确认方案，不自行换方案。
- **与 `ArrivalSceneHost` 共存**：按钮窗口不成为 key、到达动效窗口 `hitTest` 恒为空，两者互不抢触摸；快速添加窗口（主 +3）在动效窗口（主 +1）之上，打开期间到达的动效会被它盖住，要确认动效不抢焦点、不改键盘状态（P4 用例 10）；在 `-AHArrival` 的动效场景下同时打开开发者模式，确认互不影响。
- **状态栏与导航栏表现**：新增的窗口都带根控制器，要确认它们不改变状态栏样式与大标题滚动行为（浅色、深色各看一次）；有影响就停下与用户确认处理办法。
- **XCUITest 里的按住拖动**对元素在手势中途才出现不友好（系统菜单的行也是手势开始后才有），所以目标用坐标而不是元素，位置先用一次轻点读出来；若在模拟器上不稳定，停下与用户确认测试策略。
- **同一个 iCloud 项目会在每台开了同步的 Mac 下各出现一次**（项目选择按 Mac 分组、不去重；选哪台就经哪台写入，目标 Mac 掉线时不改道，需求 D14）。这是需求既定行为，真机用下来觉得别扭再提出调整。
- **本期不加任何 Rust 改动**：若实现中发现必须改，先与用户确认（会牵涉 `PROTOCOL_VERSION` 与 `ios/wire.rs`）。
