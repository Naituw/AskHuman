# iOS 大屏适配：iPad mini 与折叠屏 iPhone

2026-11-03 设计定稿。用户希望 iPad mini、折叠屏 iPhone（Duo 内屏）这类大屏获得更好的体验：收件箱用 Split View 的「侧栏列表 ＋ 详情」。范围与交互已通过 AskHuman 确认；此前 `ios-channel.md` 把「iPad 适配」列为暂缓，本文取代该条。

## 1. 范围

- **五个 Tab 都分栏**：收件箱 → Agents → 待办 → 历史 → Macs，共用一个容器（`ListDetailColumns`），按此顺序逐个落地并验证。收件箱先完成并经用户看过真机截图确认交互后（已确认），再铺开其余四个。
- **保持系统 `TabView`**：iPad 顶部胶囊、Duo 横屏右侧竖条、Duo 竖屏底部浮动条，都由系统决定；每个 Tab 在自己内部分栏，不自绘侧栏式导航。
- **原生 iPad**：iPhone + iPad 通用（`TARGETED_DEVICE_FAMILY = 1,2`），iPad 四向旋转，支持分屏与 Stage Manager 改变窗口大小；仍是单窗口（`UIApplicationSupportsMultipleScenes = false`）。iPhone 与 Duo 的方向配置不变。
- 不改线协议，不改 Mac 端。

## 2. 决策表

| 项 | 决定 | 来源 |
|---|---|---|
| 哪些 Tab 分栏 | 全部 5 个 | 用户定 |
| 导航外壳 | 保持系统 `TabView` | 用户定 |
| 设备 | 通用 App，iPad 四向旋转，多任务缩放，单窗口 | 用户定 |
| 窄浮层侧栏 | 用系统行为（`NavigationSplitView`），但进入 Tab 时先看到列表，再进入某条消息 | 用户定 |
| 评审关口 | 收件箱做完先给用户看真机截图（iPad mini 竖／横、Duo 竖／横、折叠切换），确认后再做其余 4 个 | 用户定 |
| 辅助功能字号 | regular 窗口下系统字号为 AX1 及以上时，不分栏，回落成手机的单栏堆栈：列表全宽，点开压栈，返回回到列表（320pt 的列表栏在这些字号下一行放不下几个字） | 用户定 |
| 文档 | 本 spec + 完成后更新 `docs/overview-ios.md`，不写 plan | 用户定 |
| 暂缓项 | 键盘快捷键、指针悬停、附件拖放、Stage Manager 尺寸细调，合记为一条项目待办 | 用户定 |
| 容器选择 | 看 `horizontalSizeClass`（再加上面的字号规则），不看设备：regular 且字号不到 AX1 → `NavigationSplitView`；否则原 `NavigationStack`，iPhone 代码路径不动。判断集中在 `ColumnsOrStack` 与 `Columns.applies` | agent 设计 |
| 导航状态 | 打开的那一项存放在两种容器共读的同一处，折叠／展开／旋转／缩放窗口／改字号时容器重建但打开的不丢：收件箱 `inboxPath`（最多一条）、Agents `agentPath`、待办 `todos.opened` 在 `AppStore` 里；历史与 Macs 原来没有这个状态，现在是各自屏幕上的一个 `@State`，手机的栈也用它当 path | agent 设计 |
| 默认选中 | 不自动选中；右栏显示「选择一条请求」占位。只有真正打开某条才标记已读 | agent 设计 |
| 从列表选一条 | 等同手机「回列表再选另一条」：按当前筛选重新建立上一条／下一条队列；右栏仍保留箭头，侧栏高亮跟随 | agent 设计 |
| 发送之后 | 沿用「自动前进」开关与回执页，都在右栏内完成；没有下一条时清空选中，相当于回到列表 | agent 设计 |
| 到达动效 | 保留；落在系统实际放置的收件箱标签图标上，找不到则落在导航标题。分栏里新请求的那一行就在屏幕上时，改为飞进**那一行的未读蓝点**（与 Mac 飞进侧边栏新行的蓝点一致，2026-11-07 用户指出原先没做），详见 `ios-arrival-motion.md` | agent 设计；2026-11-07 用户反馈后补 |
| 标签栏 | 宽窗口不再整体隐藏；窄窗口沿用手机行为（打开详情时隐藏） | agent 设计 |
| 还没配对 Mac | 宽窗口仍用分栏：引导页放进详情栏，列表栏不出现。配对第一台 Mac 前后容器不换，否则 iPad 顶部的标签栏会停在原处，压在新出现的列表导航栏上 | agent 设计 |
| 选中行样式 | 自绘：被选中的行盖一层淡强调色；其余行是系统在侧栏里给的卡片底色（不是手机上的白色），与同一列表里的提示行、设置分组一致。**两种底色都由行自己画**（`RowCard`），不把没选中的行留给系统（不设背景）：列表看不见时选中项换了（浮层收起时点右栏箭头、点通知；并排时先收起列表再点箭头），那一格的背景要从系统的换成自己的，系统的卡片与淡色会一起丢，列表回来后没有一行像选中的，只有两行时反而没换的那行像选中的；背景一直是自己的、只改内容，就不会丢。不用 `List` 的系统选中态（会整行填强调色、画焦点环、让文字发灰）；行上同时补 `isSelected` 无障碍特征，并把没选中的行显式去掉它（被移走的那一行的格子会被下一行沿用） | agent 设计；2026-11-10 用户反馈后改 |
| 空详情 | 各 Tab 在没有选中时右栏显示占位（图标 ＋ 一句话），不自动选第一项 | agent 设计 |
| 并排还是浮层 | 量窗口宽度（减去系统留在两侧的安全区，如 Duo 横屏的竖条）：≥ 700pt 并排，否则浮层。不能只看某一栏的宽度——浮层时详情栏独占整个窗口 | agent 设计 |
| 内容宽度 | 详情内容最宽约 720pt 居中（背景铺满），空状态与引导页同样限宽，应用内横幅约 520pt | agent 设计 |
| 列表栏 | 复用现有行与分组，选中行淡强调色底，宽度约 320（280–380） | agent 设计 |
| 开发者浮层／表单 | 保持原样，但在设备上逐一验证 | agent 设计 |

## 3. 三档窗口

「宽」只看水平尺寸类。Duo 内屏是 phone 习惯用法但尺寸类为 regular × regular，所以不能靠设备类型判断。

| 窗口 | 例子 | 表现 |
|---|---|---|
| compact 宽度 | iPhone、Duo 外屏、窄的 iPad 分屏窗口 | 与现在的 iPhone 完全一致：单栏列表，请求压栈 |
| 辅助功能字号（AX1 及以上） | 任何窗口，包括 iPad 与 Duo 内屏 | 同上一行：不分栏，单栏列表，页面压栈；打开页面时隐藏标签栏，和手机一样 |
| regular 但并排放不下（宽 < 700pt） | Duo 内屏竖屏（669pt）、较窄的 iPad 窗口 | 系统 `NavigationSplitView` 的浮层侧栏：列表盖在变暗的详情之上；选定后收起，详情占满 |
| 并排（≥ 700pt） | iPad mini 竖／横屏、Duo 内屏横屏 | 列表 ＋ 详情并排，列表栏约 320pt |

### 窄浮层的「列表优先」规则

系统在窄宽度会默认只显示详情，进入 Tab 后看到的是空详情；这不符合「先看到收件箱，再进入某条消息」，所以容器自己管理侧栏可见性：

1. 进入收件箱且没有打开任何请求：自动显示列表浮层；点浮层外的遮罩不能留下「空详情、没有列表」的死屏——没有打开请求时侧栏不可收起。
2. 选中一条：浮层收起，详情占满；系统侧栏按钮与左缘滑动可随时召回列表。再次点当前选中的收件箱标签也会打开列表浮层，保留打开的请求和回答草稿。
3. 手机上会「回到列表」的场景（发送后没有下一条、请求已在别处处理或结束而无后续等）：清空选中，列表浮层重新出现。
4. 旋转、折叠／展开重新套用以上规则，已打开的请求保留。

## 4. 验收标准

收件箱（M1）：

1. iPhone 与 Duo 外屏的行为、截图与改动前一致；原有 iPhone UI 测试全部通过。
2. iPad mini 竖屏与横屏：列表与详情并排；进入收件箱先看到列表和「选择一条请求」占位；点一行详情显示该请求且该行高亮；右栏箭头逐条切换，高亮跟随；发送后按自动前进开关处理。
3. Duo 内屏横屏：并排，右侧竖向标签条不遮挡内容。
4. Duo 内屏竖屏：进入收件箱先看到列表浮层；选一条后详情占满，系统侧栏按钮与再次点收件箱标签都可召回列表。收起浮层后仍是同一条请求，已选答案保留。发送且无下一条时列表浮层自动回来，浮层遮罩不会造成空死屏。
5. 折叠／展开、旋转、改变窗口宽度：已打开的请求不丢，草稿保留，规则 §3 重新套用。
6. 详情内容宽屏下限宽居中，答题底栏与内容对齐，应用内横幅不拉满整屏。
7. 新请求到达动效在宽屏下：收件箱列表可见、新请求的行在屏幕上时飞进该行的未读蓝点（蓝点在气泡落下前不显示，落下后原位出现并柔光一次）；否则落到可见的收件箱标签或标题。不飞出屏幕、不遮挡箭头与更多菜单。
8. 浅色／深色、大字号、中英文下无截断与重叠。
9. 辅助功能字号（AX1 及以上）：宽窗口回落成单栏堆栈，列表占满窗口，点开压栈，返回回到列表；字号改回后恢复分栏，打开的那一项保留。
10. 列表看不见时选中项换了（浮层收起时点右栏箭头；并排时先收起列表再点箭头；通知、横幅同理），列表回来后新选中的那一行是淡强调色，原来选中的和其余的行是卡片底色；浅色与深色都一样。`AskHumanLargeScreenUITests.testThePickThatMovedWhileTheListWasOutOfSightIsTintedWhenTheListIsBack` 按像素验这一条。

其余四个 Tab 沿用同一容器与上述规则（上面各条里「请求」读作该 Tab 打开的那一项），各自「选中项来自哪里、关闭后回到哪里」如下：

| Tab | 右栏打开的是 | 选中项来自 | 它消失（或返回列表）后 |
|---|---|---|---|
| 收件箱 | 请求 | `inboxPath.last`；通知、横幅、箭头、列表点选都经它 | 发送后没有下一条、请求在别处处理：清空选中，右栏回到占位 |
| Agents | Agent 会话 | `agentPath.last`；通知、横幅、新建任务后的跳转都经它 | 会话被 Mac 摘掉：页面显示它最后已知的状态（和手机一致），用户自己返回才清空 |
| 待办 | 项目的待办清单 | `todos.opened`；项目随 Mac 离线或换了 iCloud 的服务 Mac 而打不开时，仓库自己把它清空 | 清空选中，右栏回到占位 |
| 历史 | 一条记录 | 屏幕自己的 `@State`；手机的栈也用它当 path，折叠／展开时打开的那条不丢 | 记录被删除或历史被清空：清除选中，右栏回到占位 |
| Macs | 一台 Mac 的详情 | 同上 | Mac 被解除配对：清除选中，右栏回到占位；窄浮层下列表重新出现 |

Macs 的列表栏仍是整页设置（已配对的 Mac、通知、通用、开发者、流量、关于），只有「已配对的 Mac」那几行能点开右栏；其余设置就地在列表栏里操作。

## 5. 实施落点

- `ios/project.yml`：两个 target 的设备族改为 `1,2`，并为 iPad 单独声明四向方向。
- `Views/Common/ListDetail.swift`：`ListDetailColumns`（容器、窄浮层的可见性状态机、窗口宽度测量）、`ColumnsOrStack`（分栏还是手机的栈：`Columns.applies` 看水平尺寸类与字号）、`Columns` 宽度常量、`readableWidth()`、`DetailPlaceholder`、`hidesTabBarWhenCompact()`（与容器同一个判断），以及列表栏里的行：`pickable` / `pickButton` / `pickedBackground` / `pickedRow`（按钮 ＋ 淡强调色 ＋ `isSelected` 特征）与 `RowCard`（没选中的行自己画的卡片底色）。
- 卡片底色（`RowCard`）照系统在侧栏里给的量出来：iOS 26 起列表栏是一块玻璃，卡片是盖在上面的一层标签色薄纱（浅色约黑 6%，深色约白 6.5%），与系统的差不超过 2 级，挨着系统自己画的行（如设置里的「添加 Mac」）看不出接缝；iOS 17–25 就是 `secondarySystemGroupedBackground`；`.plain` 的列表（历史）没有卡片，用透明（`RowCard.plain`）。量过的有 iOS 18.5 的 iPad mini、iOS 26.2 的 iPad、iOS 27.1 的 Duo（浮层与并排；浅色与深色）；其余系统版本没量，系统以后改了卡片颜色会出现细微色差，到时对一下 `RowCard` 里的数。
- `AppStore.openFromList(_:)`：从仍在屏幕上的列表里选一条，等同手机端「回列表再选」。
- 收件箱：`InboxScreen` 经 `ColumnsOrStack` 走 `InboxColumns`（选中与 `inboxPath` 最后一项双向对应）或原栈。
- Agents：`AgentsScreen` 同样经 `ColumnsOrStack`；`AgentsColumns` 的选中与 `agentPath.last` 双向对应；标题与按钮抽成 `AgentsBar`，两种容器共用；新建任务的表单与进度页仍是 sheet，挂在 `ColumnsOrStack` 外面。
- 待办：`TodosColumns`，右栏是 `TodoProjectScreen`，选中读 `todos.opened`、写 `todos.open(_:)`；列表行去掉手机用的箭头；标题（来源菜单）抽成 `TodosBar`。
- 历史：`HistoryColumns`；`opened` 是 `HistoryScreen` 的 `@State`，两种容器共读。
- Macs：`MacsColumns`；列表是整页设置（`MacsList`），列表栏用 `.insetGrouped` 保持手机上的卡片；`opened` 同历史。
- 每个 Tab 右栏的页面都加了 `.id(<选中项>)`，换一项时不沿用上一项的 `@State`。
- 到达动效：`ArrivalLayout` 只把贴着窗口底边的标签栏当作「占用底部」；标签栏在侧边（Duo 横屏）时气泡落到导航标题。分栏里的收件箱行（`InboxRow(landsArrival:)`，只在 `selection != nil` 的分栏列表里为真）用 `ArrivalDotMarker` 向 `ArrivalDots` 登记自己的蓝点；`ArrivalSceneController` 每帧查登记簿，查得到且落在列表可见区域内就把这条气泡的落点改成蓝点，查不到仍用上面的标签或标题。

## 6. 工具栏按钮与 Duo 侧栏

2026-11-05 补充（实施计划见 `docs/plans/ios-duo-toolbar.md`）。Duo 外屏的右边有一列系统「侧栏」：状态栏的时钟 / Wi-Fi 下面是一排按钮，离拇指近。系统只把**图标或带角色的**顶栏项目放进去，纯文字按钮留在顶栏，够不着。没有公开 API 能强制，也没有「侧栏在不在」的判断，所以不做 Duo 专属分支，而是让 iOS 26+ 的顶栏按钮一律用系统写法。

### 6.1 实测规则（iOS 27.1 的 Duo 模拟器，外屏 466×678pt）

| 顶栏里放什么 | 落在哪 |
|---|---|
| `Image`、带 SF Symbol 的 `Label`（写 `.titleAndIcon` 也只显示图标）、带图标的 `Menu`、系统返回键、`ShareLink` | 侧栏 |
| 无标题的角色按钮 `Button(role: .cancel / .close / .confirm)` | 侧栏：✕ / ✕ / 蓝色 ✓ |
| `Button(role: .confirm)` 配图标 `Label`（如 `Label("Start", systemImage: "arrow.up")`） | 侧栏，蓝色图标，无障碍标签是 Label 的标题 |
| 纯文字按钮（`Button("Edit")`、`EditButton`、`Menu("Sort")`；`Button("Start", role: .confirm)` 只是蓝色药丸） | **留在顶栏** |
| `.principal` 里的内容（如待办首页的来源菜单） | 顶栏 |
| `.bottomBar` | 底栏 |

sheet 里同理：图标 / 角色按钮靠到 sheet 自己右边的一列，文字留在 sheet 顶边。外屏横屏右边也有一列；内屏竖屏没有侧栏（都在顶栏）；内屏横屏的 sheet 是居中卡片，按钮留在卡片顶栏。痛点只在外屏。

### 6.2 决策表

| 项 | 决定 | 来源 |
|---|---|---|
| 写法 | iOS 26+ 用角色按钮 / 图标；iOS 17–25 仍是文字按钮，逐字不变 | 用户定 |
| 取消 | ✕（`.cancel`） | 用户定 |
| 提交新内容（新建任务的 Start、添加待办的 Add） | 蓝色 ↑，与其他 sheet 一致；不做底部整条 CTA | 用户定 |
| 保存、完成（编辑待办的 Save、配对的 Done） | 蓝色 ✓ | 用户定 |
| 关闭（只看内容的页面：任务进度、图片画廊、资源预览） | ✕（`.close`） | 用户定 |
| 待办项目页的 Edit（只用来重排） | 重排图标 ⇅，编辑中变蓝色 ✓ | 用户定 |
| 不改 | 待办首页的来源菜单 “iCloud ▾”（`.principal`）；请求页的上一条 / 下一条 | 用户定（没选） |
| 不做 Duo 专属分支 | 靠安全区去猜侧栏在不在太脆；`.titleAndIcon` 的 Label 在普通顶栏也只显示图标，没有「一份代码两头兼顾」 | agent 设计 |
| 发送中 | 确认键换成 `ProgressView`，留在 sheet 顶栏、✕ 不动 | agent 设计 |

### 6.3 约定

新写 sheet 或全屏页的顶栏按钮，用 `Views/Common/ToolbarButtons.swift` 里的 `CancelToolbarButton`、`CloseToolbarButton`、`ConfirmToolbarButton`、`ReorderToolbarButton`，不要直接写 `Button("…")`；`disabled`、`accessibilityIdentifier` 照旧写在调用点。

- 确认键必须带图标 `Label`：无标题的 `Button(role: .confirm)` 读作 “Done”，Start / Add / Save 会被读错；文字标题加 `.confirm` 又不会进侧栏。
- 配对页完成后的 Done 在 iOS 26+ 换到 `.confirmationAction`（蓝色 ✓ 在右），之前保持原来的左侧位置。
- 任务进度页的 ✕ 放在 `.confirmationAction`，所以在 iOS 26+ 是蓝色的 ✕，与原来蓝色药丸 “Close” 的强调一致。

## 7. 暂缓

键盘快捷键（⌘1–5、↑↓、⌘↩）、指针悬停效果、附件拖放、Stage Manager 尺寸细调，记在一条项目待办里，不在本次范围。

## 8. Duo 折叠过渡的共享栏承载

五个标签保留上述导航、列表选中样式和窄浮层规则；共享栏由最外层标签容器承载。iOS 27.1 起 `RootView` 用 `AppTabs`
的稳定 `UITabBarController` / `UITab` / `UIHostingController` 承载竖向共享栏，更早系统仍用 `TabView`。
普通 UIKit 父控制器按完整 containment 生命周期承载标签控制器，并用公开的
`childForPreferredVerticalBarBehavior` 转交共享栏决策；不直接把标签控制器作为 representable 的根。
UIKit 容器获得全窗口，内部自行处理安全区；根页面同步选择、徽标、收件箱重选和原有紧凑屏
标签栏显隐策略；实际显隐在 UIKit 尺寸类／字号 trait 变化及首次出现时同步，不再依赖 SwiftUI 回传尺寸类。程序同步暂停 delegate，避免默认标签回调覆盖模型或重复触发重选。
控件没有手动搬到侧面，也不按尺寸重建标签。

全局横幅 / 发送提示与标签控制器共同放在普通 UIKit 父容器内；横幅用独立且稳定的
UIHostingController，按 [.intrinsicContentSize](https://developer.apple.com/documentation/swiftui/uihostingcontrollersizingoptions/intrinsiccontentsize) 更新高度，空时不占高度，只有横幅可以点击时
接收触摸。不要在根标签容器外再叠 SwiftUI `.overlay`：同二进制产品对照中，仅恢复这层
横幅 overlay 就在首次折叠丢失整组导航按钮，背景挂载、配对 sheet 与触觉反馈对照均通过。
这是一项容器承载约束，不是延迟刷新或重建导航的补丁。

独立程序在保留原 split → stack 模型时完成十次有效开合；产品验证状态见
[排查记录](../investigations/ios-duo-fold-navigation.md)，不把独立程序通过当成产品回归通过。
More 菜单提供含标题和符号的 `Label("More", systemImage: "ellipsis")`，让系统竖向栏与
溢出菜单有完整项目标签，遵循 [Apple 的 Duo 工具栏指导](https://developer.apple.com/videos/play/tech-talks/111462/)。
