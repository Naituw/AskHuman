# 计划：iOS 作答页的选项手感与「下一题 / 发送」逻辑

> 关联需求：`docs/specs/ios-answer-flow.md`
> 只改 iOS（`ios/`）。计划描述方案，代码以实现为准。进度在每步前打勾，做完一步就改。

## 0. 起点

- `AnswerComposer`（`ios/Packages/AskHumanKit/Sources/AHCore/AnswerComposer.swift`）：`seen` / `markSeen` / `hasRead` /
  `nextUnread` / `hasReachedEnd`；`nextUnread` 是错误导航依据（依赖历史）。
- `AnswerScreen.swift`：`ReadingTracker`（量题卡 frame、`markSeen`）、`next(...)`、`SendBar`、`OptionRow`。
- UI 测试里与「下一题」相关的：`AskHumanUITests.swift` 的 `testSendingWaitsUntilTheLastQuestionWasSeen`、
  `testScrollingToTheLastQuestionBringsSendUp`、`testAnAnsweredLastQuestionBringsSendUpAtOnce`、
  `testTheNextRequestStartsAtItsTopAndIsNotReadYet`；`AskHumanLiveUITests.swift` 里有一处循环点「下一题」直到出现「发送」。

## 1. 步骤

### 1.1 导航算法（纯逻辑，AHCore）
- [x] 新增 `QuestionNavigator`：输入 = 各题卡 frame、可见区上下沿、题数；输出 = 「下一题」目标（索引 + 贴顶 / 贴底）或 `nil`。
  规则见需求 F1 / F2：目标 = 底部（`maxY`）还没进入可见区下沿的第一张卡；放得下 → 贴底；放不下 → 题顶还在可见区上沿之下就贴顶，否则贴底。
  frame 缺失 / 未布局 → 回退到「第一张没有 `seen` 的卡」。
- [x] `AnswerComposer`：删 `nextUnread`；加 `firstWithoutSelection`（「仅选择」下第一道没选的题，F7）。`hasRead` / `hasReachedEnd` / `seen` 保留（只管发送闸门）。
- [x] `AnswerComposerTests` / 新增 `QuestionNavigatorTests`：看过 Q2 Q3 未答、滑回 Q1 → 目标 Q2；Message 把 Q1 顶出屏 → 目标 Q1；
  长卡片两步；末尾无目标；frame 缺失回退；`firstWithoutSelection`。

### 1.2 作答页接线
- [x] `ReadingTracker`：提供「可见区 + 各卡 frame」快照给 `QuestionNavigator`（`viewport()`），`moreBelow` 决定底栏是否留次要的「下一题」；Message 排好版前不统计 `seen`（与 Web 计划 W13 对接：`begin(_:awaitingMessage:patience:)`）。
- [x] `AnswerScreen.next`：收键盘（若开着，等约 0.3 秒）→ 取目标 → `scrollTo` → 触发落点闪烁；「减少动态效果」不闪。
- [x] `QuestionCard`：`flash` 覆盖层（强调色 12% → 0，0.8 秒）。
- [x] `SendBar`：多题且已到末尾 → 「发送」为主；下方还有题时左侧加次要「下一题」图标按钮（F6）；提示文字可点（F7）→ 滚到 `firstWithoutSelection` 并闪烁。标识：`next-button`（两种形态同一标识，主 / 次由状态决定）、`send-button`、`hint-jump`。

### 1.3 选项行
- [x] `OptionRowStyle`（自定义 `ButtonStyle`）：按下加深 + 0.98 缩放、无透明度变化；选中态与指示器自绘（O3 / O4）；`.transaction { $0.animation = nil }` 去掉继承的动画；弹跳仅用于内部标记且受「减少动态效果」控制。
- [x] 点按计数触发 `.sensoryFeedback(.selection)`（O5）；`minHeight 44`；`@ScaledMetric` 指示器尺寸（O6）。
- [x] 前后对比：模拟器 `simctl io recordVideo` 录一次点按，抽帧：旧实现的选中要约 12 帧才渐变完成，新实现点按后的第一帧行就已是选中态（描边 + 淡底），内部圆点在随后一两帧弹出，没有中间态；亮 / 暗截图各一张。

### 1.4 UI 测试
- [x] 新增：`testNextDoesNotSkipQuestionsThatWereOnlySeenAfterScrollingBack`（滑到底再滑回顶，每次点「下一题」只前进一题）、`testNextStaysBesideSendWhileQuestionsAreLeftBelow`、
  `testTheHintOfAnIncompleteSelectOnlyFormLeadsToTheFirstOpenQuestion`（新 Mock 场景 `strict`：三道纯选择题）、`testAnOptionTurnsOnAtOnceAndAChoiceOfOneMovesWithTheTap`（点按后立即 selected，单选换项只有一个 selected）；
  Message 把 Q1 顶出屏的情形由 `QuestionNavigatorTests` 与 `AskHumanWebUITests.testNextWalksDownFromATallMessageToTheLastQuestion` 覆盖。
- [x] 调整既有用例：`testSendingWaitsUntilTheLastQuestionWasSeen` 的点按次数范围；其它与「下一题」有关的断言。

### 1.5 文档与收尾
- [x] `docs/overview-ios.md`：改「多题表单的发送要等看过最后一题」一条，写清「下一题按位置、发送闸门按看过」；
  `docs/specs/ios-channel.md` §6.3 底部操作条一段 + 2026-10-28 反馈记录；`docs/specs/multi-question-vertical.md` §6 追加一行「iOS 端的对应实现见 `ios-answer-flow.md`」。
- [x] 本地提交（Conventional Commits，不 push）：`feat(ios): make next follow the screen and options select at once`。

### 1.6 底栏改成 Liquid Glass（需求 §3.4，2026-10-29）
- [x] 看现状：iOS 26.2（新建的 iPhone Air 模拟器）与 27.0 各截一张作「之前」。
- [x] `Views/Common/BottomBar.swift`：`.actionBar`（`#available(iOS 26)`：`scrollEdgeEffectStyle(.soft)` + `safeAreaBar`，否则 `safeAreaInset`）、`classicBarBacking()`、`barStatusChip(compact:fillsHeight:)`、`barPrimaryButton()` / `barSecondaryButton()`、`GlassGroup`。
- [x] `AnswerScreen.SendBar`：计数 / 上传进度进行内的玻璃块，提示是行上方的小条（G3、G10）；「下一题 → 发送」合成一个按钮（G5）；辅助字号仍上下排（F11）。`ConfirmScreen`：状态行进小块，提交按钮玻璃。
- [x] 变体比较（在模拟器上各截图）：渐隐样式 soft / hard / hidden；纯文字与小块；确定「soft + 小块」。试验用的启动参数与临时 UI 测试（`ScratchBarShots`）不进仓库。
- [x] 逐帧看「下一题 → 发送」的录屏（`simctl io recordVideo` + 抽帧）：两个按钮互换时会闪空胶囊 → 合并成一个按钮后胶囊不变、只有文字交叉淡入。
- [x] 截图：iOS 26.2 与 27.0 ×（顶部、滚到底、可发送、键盘弹起、确认页、深色、最大字号含深色、离线、发送中）；26.2 另有上传进度（用一个临时启动参数造出进度，之后删掉）；iOS 18.5（经典栏）×（顶部、滚到底、可发送、键盘弹起、确认页、深色、最大字号）。
- [x] UI 测试：iOS 27.0 整套 `AskHumanUITests` 60 条与 `AskHumanWebUITests` 11 条通过；26.2 整套 59/60（余下一条新建任务页的用例在改动前也失败）；18.5 只跑了碰到作答栏与确认栏的 20 条，通过（机器满载时有几条超时，单独重跑通过）。细节见 `docs/specs/ios-channel.md` §15 该节的验证一行。
- [x] 本地提交（不 push）：`feat(ios): float the answer and confirm bars as Liquid Glass`。
- [x] 反馈修正（用户：「你左边那块和右边高度不一样，有点难看呀」，需求 G10）：行内计数块与按钮同高——行 `fixedSize(horizontal: false, vertical: true)`，块与各按钮的标签 `maxHeight: .infinity`（仅浮动栏、非上下排）；提示不再进计数块，改成行上方的一小条（`barStatusChip(compact: true)`）。同款用于确认页的状态行。
- [x] 修正时踩到的坑：可点的提示小块把玻璃放进按钮的标签里，iOS 26.2 上点它没有反应（`testTheHintOfAnIncompleteSelectOnlyFormLeadsToTheFirstOpenQuestion` 稳定失败，27.0 与 18.5 上通过）；改为玻璃包在按钮外面、内边距与点按区域留在标签里（`barChipButtonLabel()` + `barChipButtonGlass()`）。
- [x] 重新验证：用无障碍帧量了高度，截图 26.2 / 27.0 / 18.5；与作答栏、确认栏有关的 20 条 UI 测试按系统各跑一遍。数字与没通过的情况见 `docs/specs/ios-channel.md` 该节的验证一行。
- [x] 本地提交（不 push）：`fix(ios): make the answer bar's count chip and buttons one height`。
- [ ] 真机：iPhone Air 当时 `unavailable`，没装、没看。

### 1.7 输入栏改成 Liquid Glass（需求 §3.4 G11，2026-10-29）
- [x] 看现状：iOS 26.2 与 27.0 各截 Agent 页与待办页的底栏作「之前」。
- [x] `Views/Common/BottomBar.swift`：`BottomBar.rowHeight`（44）与 `roundGlyphBox`、`classicBarBacking(hairline:)`、`barPanel()`（附件与上传进度的玻璃面板）、`barFieldBox(focus:)`（玻璃输入框，整框可点）、`barRoundButton()` / `barRoundGlyph()`（玻璃圆钮）、`BarSendButton`（着色玻璃的发送钮）。
- [x] `InterjectBar` 与 `TodoAddBar` 用这些件重排；`AgentDetailScreen` / `TodoProjectScreen` 改用 `.actionBar`；「不能发」「Mac 不可用」提示条改成 `barStatusChip(compact: true)`；`AttachmentMenu` 紧凑样式的玻璃圆钮。经典栏仍走原来的样式。
- [x] 量高度（无障碍帧）：第一版圆钮 52 pt、输入框 44 pt → 改 `.regular` 控件尺寸 + 30 pt 的图标框后，圆钮与输入框的最小高度都是 44 pt（26.2 与 27.0）。
- [x] 量点按区域：点输入框的上、左空白处键盘不弹（`keyboard=false`）→ 浮动栏上整框挂点按手势去设 `@FocusState`，重量后都弹（Agent 页、待办页，26.2 与 27.0）；经典栏不动。
- [x] 截图：26.2 与 27.0 ×（顶部、键盘弹起、折行、附件与上传进度、字数计数、自动执行打开、「不能发」与只读提示、发送中、深色、最大字号）；18.5（经典栏）17 个状态与改动前逐像素比对（栏所在的底部 700 px）：14 个完全一致，3 个只差会动的菊花与 Home 指示条的色调。试验用的启动参数与临时 UI 测试（`ScratchBars2`）不进仓库。
- [x] UI 测试（最终代码，Agent 页相关的 6 条 + 整个 `AskHumanTodosUITests` 的 9 条，三个系统各一遍）：27.0 全过；18.5 与 26.2 各有几条第一次没过——`testAMacThatPredatesTodosAsksForAnUpdate` 在 26.2 与 18.5 上改动前也失败，`testAddingFinishing…` / `testDraggingATodoUp…` 是满载时的偶发、再跑通过，`testAnAgentWaitingForAnAnswerLinksToItsQuestion` 在 26.2 上会丢点按（改动前的代码上也出现过，但我的代码上更常见，原因没查清）。逐条数字见 `docs/specs/ios-channel.md` §15 该节的验证一行。
- [x] 本地提交（不 push）：`feat(ios): float the agent message bar and todo add bar as Liquid Glass`。
- [x] 真机点按：2026-10-30 凌晨装到 iPhone Air（`77697b7` 的 Debug 构建，Mock 数据），用户手点 Agent 页的「Answer」若干次，没有丢点按（模拟器 26.2 上那条用例偶尔丢点按，见 `docs/specs/ios-channel.md` §15 该节）。
- [ ] 真机外观：输入框与圆钮同高是否顺眼、点输入框空白处弹不弹键盘、栏下的渐隐、发送中的灰圈——用户没有反馈，未确认。

## 2. 验证命令

同 `docs/plans/ios-web-markdown.md` §2；包测试 `cd ios/Packages/AskHumanKit && swift test`；UI 测试按用例名过滤。

## 3. 备忘

- 弹窗对照代码：`src/views/popup/usePopupCore.ts` 的 `goNext` / `goPrev` / `onCmdEnter` / `scrollQuestionIntoView` / `flashCard` / `lastSeen`。
- 卡片间距：`AnswerScreen` 把 16 pt 间距放进 `.id(...)` 的容器里，所以贴底滚动时卡片下方自带呼吸位。
- 题卡高度会因 Message 的 WebView 排版（含 Mermaid 换图）而变：导航算法每次点按都用**当时**的 frame，不缓存。
