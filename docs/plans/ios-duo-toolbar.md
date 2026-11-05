# 计划：iOS 顶栏文字按钮在 iPhone Duo 侧栏上的改法

> 起因：2026-11-05 用户说顶栏里有很多文字按钮，在 iPhone Duo 上不会移到侧栏，操作不便，让分析并设计优化。
> 分析与选项经 AskHuman 给了用户（附 Duo 模拟器截图），用户选了方案 A，并说「计划文档不用给我确认，主要是给你自己防遗忘；
> 直接在主分支改；我等着你改完，生成新的 TestFlight 版本」。本文是给实施者自己看的备忘，不是评审稿。
> 工作方式：直接在 `main` 上提交（不开分支）；不推送（用户没要）；做完出内测包，走 `ios-testflight` 技能。

## 1. 实测事实（Duo 模拟器 iOS 27.1，外屏 466×678pt；探针 App ＋ 真实页面）

Duo 外屏竖屏的右侧有一列「侧栏」（时钟 / Wi-Fi 下面）。系统按**顶栏项目的类型**决定去留，没有公开 API 能强制：

| 顶栏里放什么 | 落在哪 |
|---|---|
| `Image`、带 SF Symbol 的 `Label`（写 `.titleAndIcon` 也只显示图标，标题被丢掉）、带图标的 `Menu`、系统返回键、`ShareLink` | 侧栏 |
| 无标题的角色按钮 `Button(role: .cancel/.close/.confirm)` | 侧栏：✕ / ✕ / 蓝色 ✓（无障碍标签：`.cancel` 读 Cancel，`.confirm` 读 **Done**） |
| `Button(role: .confirm) { Label("Start", systemImage: "arrow.up") }` | 侧栏，蓝色 ↑，无障碍标签是 Label 的标题 |
| 纯文字按钮（`Button("Edit")`、`Menu("Sort")`、`EditButton`；`Button("Start", role: .confirm)` 只是蓝色药丸） | **留在顶栏** |
| `.principal` 里的东西（如待办首页的来源菜单） | 顶栏 |
| `.bottomBar`（文字、图标一样） | 底栏 |

- sheet 里同理：图标 / 角色按钮靠到 sheet 自己右边一列，文字留在 sheet 顶边（`.large` 时就是屏幕最上沿）。
- 外屏横屏右侧也有一列；内屏竖屏没有侧栏（全在顶栏）；内屏横屏的 sheet 是居中卡片，按钮留在卡片顶栏。**痛点集中在外屏。**
- 发送中把确认键换成 `ProgressView`：转圈留在 sheet 顶栏、✕ 不动（探针 O，没有跳变）。
- 没有公开的「侧栏在不在」API（iOS 27 API diff 里没有）；`ListDetail` 靠窗口安全区扣掉竖条，但拿它去分叉 UI 太脆，**不做 Duo 专属分支**。

## 2. 决策（2026-11-05 用户经 AskHuman 选定）

- **Q1 方案 A**：按 iOS 26 系统写法重做顶栏按钮：取消 → ✕，提交新内容 → 蓝色 ↑，保存 / 完成 → ✓，关闭 → ✕（`.close`）。**iOS 17–25 保持文字，逐字不变。**
- **Q2**：新建任务的 Start 也是侧栏蓝色 ↑，与其他 sheet 一致；不做底部整条 CTA（方案 B 不做）。
- **Q3**：除了 sheet 的 Cancel / 确认 / Close，只多处理「待办项目页 Edit → 重排图标（编辑中变 ✓）」。
  **没选**（不要顺手做）：待办首页来源菜单 “iCloud ▾” 改成侧栏图标菜单；请求页上一条 / 下一条改上下箭头。
- **Q4**：写本计划；直接在 main 上改；改完出新的 TestFlight 包。

## 3. 映射

| 位置（文件） | 现在 | 改成（iOS 26+） | id（保留） |
|---|---|---|---|
| 新建任务 `Agents/NewTaskSheet.swift` | Cancel（`.cancellationAction`，发送中禁用）、Start（半粗体，`!canStart` 禁用，发送中是 ProgressView） | ✕ `.cancel`；蓝色 ↑ `Label("Start", "arrow.up")` | `task-cancel`、`task-start` |
| 任务进度 `Agents/TaskProgressScreen.swift` | Close（`.confirmationAction`） | ✕ `.close`（位置不变） | `task-close` |
| 添加待办 `Request/RequestTodoSheet.swift` | Cancel / Add（`!canSend` 禁用，发送中 ProgressView） | ✕；蓝色 ↑ `Label("Add", "arrow.up")` | `request-todo-cancel`、`request-todo-send` |
| 快速待办 `Developer/QuickTodoView.swift` | 同上 | 同上 | `quick-todo-cancel`、`quick-todo-send` |
| 编辑待办 `Todos/TodoEditSheet.swift` | Cancel / Save（`!canSave` 禁用） | ✕；✓ `Label("Save", "checkmark")` | Cancel 原本没有 id；`todo-edit-save` |
| 配对 `Pairing/PairingSheet.swift` | 一个按钮：`isDone ? "Done" : "Cancel"`（`.cancellationAction`） | 没完成：✕ `.cancel`（`.cancellationAction`）；完成：✓ `Label("Done", "checkmark")` **换到 `.confirmationAction`**（蓝色 ✓ 不该在左上） | `pairing-close`（两态都挂） |
| 扫码页 `QRScannerScreen` | Cancel | ✕ `.cancel` | 无 |
| 图片画廊 `Attachments/ImageGallery.swift`、资源预览 `AssetViews.swift` | Close（`.cancellationAction`），分享是图标 | ✕ `.close` | 新增 `gallery-close`、`asset-close`（UI 测试原先按文字 `buttons["Close"]` 找） |
| 待办项目页 `Todos/TodoProjectScreen.swift` | `EditButton()` | 没在编辑：`Label("Reorder", "arrow.up.arrow.down")`；编辑中：✓ `.confirm` `Label("Done", "checkmark")`；点按切换 `editMode`（带动画，与 `EditButton` 一致） | `todo-reorder`（两态都挂） |

## 4. 实现

- 新文件 `ios/AskHuman/Views/Common/ToolbarButtons.swift`，四个小视图，调用点各一行，修饰符（`.disabled`、`.accessibilityIdentifier`）留在调用点：
  - `CancelToolbarButton(action:)`：iOS 26 `Button(role: .cancel, action:)`；否则 `Button("Cancel", action:)`。
  - `CloseToolbarButton(action:)`：iOS 26 `Button(role: .close, action:)`；否则 `Button("Close", action:)`。
  - `ConfirmToolbarButton(_ title:, systemImage:, emphasized:, action:)`：iOS 26 `Button(role: .confirm, action:) { Label(title, systemImage:) }`；否则 `Button(title, action:)`，`emphasized` 为真时 `.fontWeight(.semibold)`（只给新建任务的 Start，保持老样子）。
  - `ReorderToolbarButton()`：iOS 26 读 `@Environment(\.editMode)` 切换；否则 `EditButton()`。
- **坑**：
  - 确认键不能用无标题的 `Button(role: .confirm)`：它读作 “Done”，Start / Add / Save 都会被读错；必须带 `Label`。
  - 文字标题 ＋ `.confirm` 角色不会进侧栏（只是蓝色药丸），一定要图标 Label。
  - 工具栏里的 Label 即使 `.titleAndIcon` 也只显示图标，所以没有「一份代码两头兼顾」，iOS < 26 才走文字。
  - 配对页换位置时用 `ToolbarItem(placement: isDone ? .confirmationAction : .cancellationAction)`。
  - 新建任务 / 添加待办发送中的 `ProgressView` 保持原样（顶栏里转圈，✕ 不动）。
  - `AskHumanTodosUITests.testDraggingATodoUpChangesTheOrder` 用「标签含 Reorder」找拖动手柄：我们的工具栏按钮在没进编辑态时也叫 “Reorder”，进了编辑态就变 “Done”，查询发生在编辑态里，所以不撞；若真撞了，把查询限定到 `cells` 里。
- 本地化：`ios/Tools/make-catalog.py` 的 `ZH` 里加 `"Reorder": "重新排序"`；`xcodebuild build -derivedDataPath build/DerivedData …` 后跑脚本重生成 `Localizable.xcstrings`。其余字符串（Cancel / Close / Start / Add / Save / Done）已有。
- UI 测试：`AskHumanUITests.swift` 画廊用例的 `app.buttons["Close"].tap()` 改按 `gallery-close`。其余用例都按 id，不动。

## 5. 验证

0. 先 `node scripts/machine-health.mjs`（上一次是 tight：交换区 12.7/14 GiB）；`xcodebuild` 长任务放后台、分段轮询，说明在等什么；UI 测试必须前台跑（后台会被工具杀掉）。
1. `xcodegen generate` → `xcodebuild build`（Duo 模拟器 `9E037F77-7859-4EF0-BB14-553A45928E69`，iOS 27.1）→ `make-catalog.py` → 再 build。
2. **Duo 外屏竖屏逐屏截图**（Mock，`-AHMock YES`）：新建任务（`-AHNewTask YES`）、任务进度（点 Start）、请求页「⋯ → 添加待办…」（`-AHOpen req-next`）、待办首页 → 项目 → 点一行（编辑待办）、项目页重排、画廊（`-AHScenario gallery -AHOpen req-gallery`）、配对页。判据：取消 / 确认 / 关闭都在侧栏，没有文字按钮留在顶栏；发送中转圈正常；禁用态的 ✓ / ↑ 看得出灰。
3. 抽查外屏横屏、内屏竖屏、内屏横屏各一两屏（内屏竖屏没有侧栏，应该与普通 iPhone 一样；内屏横屏的 sheet 在卡片顶栏）。
4. iOS < 26 回退：`AskHuman iOS Dev 18.5`（`32A29065-26A6-469A-9ADB-370FD9F3994A`）上截一两屏，确认仍是文字按钮、行为不变。
5. UI 测试（前台跑，`-parallel-testing-enabled NO`）：按 id 引用了被改按钮的类——`AskHumanModelUITests`（新建任务 / 进度）、`AskHumanRequestTodoUITests`、`AskHumanTodosUITests`（编辑、重排）、`AskHumanUITests`（画廊 `Close`、新建任务 `task-*`）、`AskHumanDeveloperUITests`（快速待办、`task-cancel`）、`AskHumanLargeScreenUITests`（大屏上的新建任务 / 编辑待办）、`AskHumanArrivalUITests`（`task-cancel`）；`AskHumanLiveUITests` 要真 broker，跳过。`AskHuman iOS Dev`（iOS 27.0）为主，必要时 18.5 再跑一遍；已知在 18.5 / 26.2 上不稳的几条见 `docs/PROGRESS.md`，对照基线再判断。注意 `AskHumanInputVisibilityUITests` / `AskHumanVideoAttachmentUITests` 里的 `Done` / `Add` 是系统照片选择器的按钮，不是我们的。
6. Swift 包没改（没动 AHCore），不用重跑；若碰到共享代码再跑 `swift test`。

## 6. 文档与进度

- `docs/specs/ios-large-screen.md`：加一节「工具栏按钮与 Duo 侧栏」，写下实测规则与我们的写法约定（新写工具栏按钮时该用哪个组件）。
- `docs/overview-ios.md`：「大屏」那条或不变量里加一句指向该节与 `ToolbarButtons.swift`（只有影响心智模型的才写）。
- `docs/PROGRESS.md`：加「待验收：Duo 外屏顶栏按钮」一节，写已随哪个 build 出去、真机上请看什么；用户没选的两个相邻问题**不要**记成待办（没被接受的建议）。
- 出包后的记录写进 `docs/plans/ios-channel.md`（技能第 10 步）。

## 7. 出包（用户要求，技能 `ios-testflight`）

1. 工作区干净、`HEAD` 就是要出的提交（`src/prototype/*` 等不是我的未跟踪文件不进包，也不提交；技能要求干净树，必要时问用户）。
2. `status.mjs`、`changes.mjs`（基线标签 `ios-testflight-N` 要对得上 Apple 的最大 build 号，对不上就停下问用户）。
3. 起草中英文测试内容，**不用问用户确认**（2026-11-04 起的规则），出包后在汇报里给出。
4. `xcodegen generate` → `xcodebuild archive … CURRENT_PROJECT_VERSION=<N+1>` → `upload.py` 预检 → `--upload` → `status.mjs` 等 VALID → `helper.py notes` 写测试内容 → 本地标签 `ios-testflight-<N>`（不推送）→ 记录进 `docs/plans/ios-channel.md`。
5. 汇报：版本 / build 号、基线、清单、两份测试内容；提醒用户在 Duo 上看 ✕ / ↑ / ✓ 是否都在侧栏。

## 8. 不做

- Duo 专属分支（靠安全区猜侧栏）；方案 B（新建任务底部 Start）；来源菜单改图标菜单；上一条 / 下一条改上下箭头；iOS < 26 的外观改动；线协议、Mac 端、AHCore。

## 9. 实施记录

2026-11-05 全部做完，直接在 `main` 上提交（没推送）。

- 提交：`a6c79f29` 本计划；`72bac227` `feat(ios)` 组件 ＋ 9 个文件的调用点 ＋ 字符串 ＋ 画廊测试改按 id；`77959702` 规格 §6「工具栏按钮与 Duo 侧栏」与 `overview-ios.md` 一条；`c9095887` 配对页完成态的测试。
- 实现与 §3、§4 一致，多出来的 id：`todo-edit-cancel`、`scanner-cancel`、`asset-close`、`gallery-close`（原先这几个按钮没有 id，UI 测试按文字找）。`AskHumanUITests` 画廊用例改按 `gallery-close`，其余用例本来就按 id，没动。
- 验证：
  - Duo 外屏竖屏（iOS 27.1）逐屏截图：新建任务（空 / 输入后可发）、任务进度、添加待办、编辑待办、配对、画廊、待办项目页与重排中。取消 / 确认 / 关闭都进了侧栏，顶栏没留文字按钮；禁用态的 ↑ 是灰的、可发后是蓝的。
    发送中转圈留在 sheet 顶栏、✕ 不动，这一条来自早先的探针（§1），真实的新建任务页没有单独截发送中。
  - 普通 iPhone（iOS 27.0）：sheet 左上 ✕、右上蓝色 ✓ / ↑；UI 测试 `AskHumanTodosUITests` 10、`AskHumanRequestTodoUITests` 8、`AskHumanUITests`（画廊、文件、新建任务、离开表单、配对两条）、`AskHumanModelUITests` 4、`AskHumanDeveloperUITests` 2、`AskHumanArrivalUITests` 1 全绿。
  - iOS 18.5：新建任务与项目页截图，仍是 Cancel / Start / Edit 文字按钮。
  - 配对页的完成态：测试里等到 ✕ 变成蓝色 ✓（`.confirmationAction`）再点它关闭 sheet。
- iOS 18.5 上无障碍树把每个工具栏按钮报成「容器＋按钮」两个同 id 节点（没改的 `new-task` 也一样），所以 `AskHumanRequestTodoUITests` 里按 id 读 `isEnabled` 的那条在 18.5 上不可靠；这是原来就有的，没有为此改测试。
- 没做：`AskHumanLargeScreenUITests`（要 iPad / Duo 模拟器）、`AskHumanLiveUITests`、用最终包复查外屏横屏与内屏、任何真机。
- 出包：build 8（`0.1.0 (8)`），本地标签 `ios-testflight-8` → `c9095887`；记录在 `docs/plans/ios-channel.md`「2026-11-05：TestFlight 内部构建 8」。与 §7 的差别：主检出里有别的会话的未跟踪文件，没有停下来问用户，而是从 `c9095887` 的干净临时 worktree 归档（包的内容就是那个提交）。
  测试内容第一次被 Apple 拒了：`✓`、`✕` 不被接受（HTTP 409），改成文字后通过，已写进 `specs/ios-testflight-build-notes.md` 与技能。
- 剩下的事（真机看 Duo 侧栏、外屏横屏与内屏复查、任务进度页「关闭」的蓝色）在 `docs/PROGRESS.md`「iOS 顶栏按钮进折叠屏侧栏」。
