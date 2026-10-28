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

## 2. 验证命令

同 `docs/plans/ios-web-markdown.md` §2；包测试 `cd ios/Packages/AskHumanKit && swift test`；UI 测试按用例名过滤。

## 3. 备忘

- 弹窗对照代码：`src/views/popup/usePopupCore.ts` 的 `goNext` / `goPrev` / `onCmdEnter` / `scrollQuestionIntoView` / `flashCard` / `lastSeen`。
- 卡片间距：`AnswerScreen` 把 16 pt 间距放进 `.id(...)` 的容器里，所以贴底滚动时卡片下方自带呼吸位。
- 题卡高度会因 Message 的 WebView 排版（含 Mermaid 换图）而变：导航算法每次点按都用**当时**的 frame，不缓存。
