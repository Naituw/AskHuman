# PROGRESS

记录需要跨会话保留的未完成 / 延期事项和明确下一步。任务 / 需求完成后删除其 section
（历史留在 git）。

## 待实际重新登录观察：macOS 守护进程进入 Aqua

2026-11-13 的弹窗超时发生在旧 Aqua 域退出、新域尚未建立时。旧版回退到 `setsid` 后继续持有
单实例锁，随后的登录项无法接管。主工作树已修复并安装，见 [Daemon 架构 §5.2.1](specs/daemon-architecture.md#521-macos-登录切换与无桌面启动)。

守护进程首轮回归 349 项通过。最终构建的启动与状态解析 5 项检查通过。私有实例用真实
`setsid` 验证了在途远程作答、启动后继失败后恢复、Aqua 接管后真实弹窗就绪，以及并发启动只留
一个托管进程。日常 daemon 已运行最终构建。

下次正常注销重登后运行 `AskHuman hello`，核对弹窗可答，daemon PID 与当前 Aqua launchd
任务一致。私有实例覆盖启动与接管，真实的注销重登时序仍待观察。

## 待日常观察：Codex 空闲消息的成功回执

2026-11-12 用户反馈手机给空闲 Codex 会话发消息后，Agent 正常回复却每次提示可能失败。
本机 4 条原生操作均为 `accepted`，AskHuman 对应回执均误留 `unknown`；只读真实快照证实
操作 UUID 在 `turn.params.clientUserMessageId`／`userMessage.clientId`，旧代码查的是别的字段。
已在主工作树修复精确关联，运行中与快速完成的回合都能确认送达，并保留会话隔离及防重发规则。
脱敏真实结构的回归在原代码下失败，修复后消息控制 24 项通过、生产目标 Clippy 与安装通过；
装机后两条真实历史错误回执已自动变成 `started`，回合 ID 与原生快照完全相同，没有发送新模型请求。
规格见 [Codex 桌面接入](specs/codex-desktop-session-integration.md)。证据在
`/tmp/askhuman-codex-receipt-regression-before-run.log`、`/tmp/askhuman-codex-receipt-tests.log`。
这项修复只需更新 Mac；下一步在手机日常给空闲 Codex 会话发一条消息，确认不再误报，完成后删除本节。

## 待真机验收：iOS 标签栏常驻

2026-11-12 用户确认在主工作树取消请求／确认、Agent、项目待办详情的主动隐藏规则；列表与详情都保留系统标签栏，不再由尺寸类、字号或导航深度控制显隐。键盘与模态呈现仍由系统管理。约定见 [作答流程 §3.9](specs/ios-answer-flow.md#39-详情页保留标签栏2026-11-12) 与 [大屏规格](specs/ios-large-screen.md)。

375pt iPhone SE 模拟器：iOS 26.2 最终 8 项 UI 检查、iOS 18.5 的 5 项常驻回归全部通过，覆盖操作栏／键盘避让、请求与插话草稿在 Tab 切换后保留、待办添加、发送及返回列表。新增 `AskHumanPersistentTabsUITests`；旧用例改为检查列表入口，不能再用 Tabbar 是否存在判断已返回列表。
iOS 27.2 Duo 外屏（466×678pt）的 XCTest 运行器在用例开始前因 `XCTWaiter.handleStalledWait` 崩溃，用户确认改用 duoctl 直接验收；请求、确认、Agent、待办页的五个 Tab 与操作栏不重叠，键盘避让、请求／插话草稿跨 Tab 保留及 Mock 待办添加均已用实际操作、截图和无障碍帧核对。当前源码模拟器构建、`./scripts/install.sh` 及安装后的正常 AskHuman 提问通过。

额外回归的两个既有 `AskHumanBarUITests` 严格尺寸断言仍失败：默认字号快捷回复按钮为 54×50.5pt、辅助字号待发计数文字宽 39pt（断言要求至少 40pt）；改动前的已构建 App 在同一模拟器上也复现相同数值。

已随内外 TestFlight `0.1.0 (21)` 分发（源码 `a5379ab`，见 [Build 21 交付记录](plans/ios-channel.md#2026-11-13testflight-内部与外部构建-21)）。下一步在真机看底部两层栏的空间与侧边 Tab 的切换手感，含键盘弹出／收起；确认后删除本节。

## 待真机验收：iOS 已结束页的 3 秒倒计时

2026-11-12 用户将已结束页的自动前进倒计时由 5 秒改为 3 秒，「下一条／关闭」沿用同一时长。
默认值、既有测试与当前规格引用已同步，`EndedCountdownTests` 29 项通过（含会话历史带来的
「倒计时还没开始就选了停留」一项）。约定见
[iOS 作答流程 §3.8](specs/ios-answer-flow.md#38-已结束页的下一条--关闭按钮与-3-秒倒计时2026-11-12-修订)。
Mac 端：此前 `./scripts/install.sh` 被会话历史开发中的 `RequestView.agent_key` 缺失挡住，2026-11-12
该功能补齐后已补跑，成功，日常版已换上。已随内外 TestFlight `0.1.0 (19)` 分发，见
[Build 19 交付记录](plans/ios-channel.md#2026-11-12testflight-内部与外部构建-19)。下一步在真机确认 3 秒后切换／关闭。已有项目待办
`666e7f5c-2ebb-45eb-9ee3-04957f742366`，完成后删除本节。

2026-11-12 用户再次反馈 Agent 取消后不倒计时。已在 iOS 26.2 模拟器复现：点按真实网页正文后，
没有键盘，`WKContentView` 仍成为第一响应者并符合 `UITextInput`，旧 `PageEditing.isCovered` 因而
误认为正在输入、永久放弃倒计时；原生正文对照通过，网页正文用例在原逻辑下失败，运行日志确认了视图类型。
现只保护可编辑的 `UITextView`／启用的 `UITextField`，保留原有键盘、弹层、停留与历史阅读规则。
修复后 6 项 iOS UI 回归全过，含原失败场景、网页阅读后后台取消再返回、实际输入、弹层与停留。
证据在 `/tmp/askhuman-ended-web-repro.xcresult`、`/tmp/askhuman-ended-web-repro-diagnostics/`、
`/tmp/askhuman-ended-web-fixed.xcresult`。`./scripts/install.sh` 已成功安装日常版；iOS 修复已随内外 TestFlight `0.1.0 (21)` 分发
（见 [Build 21 交付记录](plans/ios-channel.md#2026-11-13testflight-内部与外部构建-21)），仍需手机日常观察。

## 待真机验收：iOS 请求页的「此会话的历史」

2026-11-11 用户想在收到消息的请求页上更容易回看会话历史：作答页 / 确认页（含已结束页）下拉加载这个 Agent 会话更早的问答，「⋯」菜单里有等价入口；不做任何提示，定位是 Power User 功能。数据读 Mac 的完整历史，手机不落盘。[规格](specs/ios-session-history.md)（用户逐项确认了 S6 / S10 / S13，并保留「回到当前提问」胶囊）与[计划](plans/ios-session-history.md)已通过；用户选择在主工作树开发，让 agent 一次做完、次日早上查收。

2026-11-12 P0–P6 全部完成，提交、偏差、验证数字见计划文末「实施记录」；Mac 已用 `./scripts/install.sh` 装为日常版。手机端界面已在模拟器的 Mock Mac 上验证，并随内外 TestFlight `0.1.0 (19)` 分发，见 [Build 19 交付记录](plans/ios-channel.md#2026-11-12testflight-内部与外部构建-19)。还没做的：

1. **真机一轮**。Build 19 里是第一版的下拉（系统转圈，用户用过后要求重做）；自绘版（[规格 S18、S19](specs/ios-session-history.md)，做法与验证见[计划](plans/ios-session-history.md#实施记录r1r4下拉的重新设计)）已随内外 TestFlight `0.1.0 (20)` 分发（源码 `6cd396fc`，见 [Build 20 交付记录](plans/ios-channel.md#2026-11-12testflight-内部与外部构建-20)），手机更新与真机手感待用户反馈。**2026-11-12 用户试用时发现**两栏并排（iOS 26 起详情栏的滚动视图铺满窗口、侧栏占它的左安全区）时，下拉中的字居中在整个窗口、与加载中那一行不在同一条中线：已修（下拉头按滚动视图的安全区排版，原因与前后对比见计划「追加：两栏并排时下拉头偏离右栏」），并随内外 TestFlight `0.1.0 (21)` 分发（见 [Build 21 交付记录](plans/ios-channel.md#2026-11-13testflight-内部与外部构建-21)），待在两栏并排的设备上再看一眼。要在真机上看的是手感：手指拉着页面时才露出「下拉加载更早的记录」一行（图标随拉动逆时针转，没有蓝色）；拉过阈值（页面越界约 72 pt，手指约走 145 pt）字变深、轻震一下，松手才加载；松手后页面留出约 64 pt 的一行，灰字「正在加载历史…」上有一道高光扫过（浅色与深色各看一眼），页面可以照常往下滚，这一行跟着内容走；历史到达时请求纹丝不动、这一行换成历史；没拉够松手什么也不发生；短页也拉得动。阈值、空地高度、回弹节奏、高光速度、轻震力度都是初值，不满意改 `HistoryPull` 与 `PullMetrics` 里的常量，不动结构。其余同前：菜单入口、卡片展开 / 收起、「回到当前提问」胶囊、已结束页加载历史后不被倒计时带走、深色与大字号（最大无障碍字号下那一行与阈值按字号放大，要拉得明显更长，值不值得封顶也请看一眼）、iPad 与折叠屏。模拟器验过 iOS 18.5（iPhone 16 与 iPad 的最终代码全过）；iOS 26.2 与 27.2（27 的运行时只带 iPhone Duo，用它折起来的外屏）完整跑过的是改补偿窗口之前的代码，之后的 Duo 外屏那几轮碰上被饿死的模拟器（开机后一阵 Poster 扩展风暴，叠上别的会话的编译），手指按着那一条通过、别的有过有不过，失败都是模拟器的毛病，见计划「偶发」6，没有一轮干净的。没有 27 上的普通 iPhone，用户的真机若是 27.x，留意松手后页面有没有跳、历史前插时请求有没有动。
2. **真实 Mac + 手机的 UI 一轮**：同一个 Agent 会话里问两三个问题答掉，再发第三个，在第三个的请求页下拉，看到前两个的真实答案。引擎到真实 daemon 的读取已有端到端测试（`SessionHistoryE2ETests`），界面这一段只在 Mock 上验过；真实服务加签名的模拟器 App 没有做。
3. 最大无障碍字号下「回到当前提问」胶囊会折成三行并覆盖部分历史卡片，见计划文末的截图观察；待用户看过真机表现后确定字号处理。

## 待真机验收：iOS 历史里的图（会话历史卡片与「历史」标签详情页）

2026-11-12 用户发现历史记录里的图怎么都看不见：会话历史卡片与手机「历史」标签的详情页只有数量与文件名。[规格](specs/ios-history-pictures.md)与[计划](plans/ios-history-pictures.md)经用户逐项确认（折叠卡片也露一排最多 4 张、点缩略图直接看大图；被 Mac 清掉的图显示灰色占位「已不在 Mac 上」；这一版只读现有路径、不改 Mac 的历史存储）；Mac 与手机一起做。提交、与起草时的偏差、验证数字见计划文末「实施记录」。

还没做的：

1. **已随内外 TestFlight `0.1.0 (21)` 分发**（源码 `a5379ab`，见 [Build 21 交付记录](plans/ios-channel.md#2026-11-13testflight-内部与外部构建-21)），手机更新后即可看到；Mac 端要含 `09862243`（本机日常版已含），别的 Mac 不更新就没有缩略图。要在真机上看：折叠卡片的一排缩略图（4 张与「+N」、手指好不好点、点缩略图不展开卡片）、展开的网格、点开后的查看器翻页与分享、灰色占位的样子、「历史」标签详情页的图，以及大字号、深色、iPad 与折叠屏。
2. **真实 Mac + 带签名模拟器 / 真机的一轮**：用 Agent 带两三张图问一个问题、回答里也贴一张，再发下一个，在下一个的请求页下拉；再把其中一张原文件（`/tmp` 下的）删掉，看占位。引擎到真实 daemon 的整条链已有端到端测试（`HistoryPicturesE2ETests`，含多窗口下载逐字节比对、文件被删后的 `notFound`、daemon 重启后同一 `assetId` 仍可下载），界面这一段只在 Mock Mac 上验过。
3. 缩略图格是 6 KiB 以内的小图，格子大时偏软；若真机上觉得糊，调 `ios/views.rs` 里 `history_thumbnail` 的（长边，质量）阶梯与 `HISTORY_THUMB_MAX_BYTES`（页预算 256 KiB 还有余量，见规格 HP6），不动结构。

## 待真机验收：iOS 图片查看器让图躲开顶栏

2026-11-12 用户反馈「ios 端查看图片时，顶岛可能挡住图片」。我理解为查看器顶上的栏（关闭、文件名与「N / M」、分享；iOS 26 起是玻璃按钮）盖住了竖长图的上端；现在每页从栏的下沿开始排，放大后图被裁在栏的下沿（`ImageGallery.swift`；约束写在 `overview-ios.md` 的「请求附带的图片」一条，取舍与验证见 [`ios-channel.md` §15 的 2026-11-12 一节](specs/ios-channel.md#2026-11-12-图片查看器图从顶栏的下沿开始)）。模拟器上红 → 绿都验过（iPhone 16 / iOS 18.5、iPad Pro 11 (M4)、iPhone 17 / iOS 26.2），键盘守卫用例连跑 8 次通过。

还没做的：

1. **真机没看过**（已随内外 TestFlight `0.1.0 (21)` 分发，见 [Build 21 交付记录](plans/ios-channel.md#2026-11-13testflight-内部与外部构建-21)）。更新后在手机上打开一张竖长截图（高过 sheet 的那种），确认上端在栏的下面；再放大、平移几下，看「放大后被裁在栏的下沿」的手感是否可接受（系统照片 App 能把放大的图拖到栏的后面，这里不能；想要就在 `ZoomableImage` 里改用内容内边距，改之前先问用户）。
2. **「顶岛」的读法**是我定的：iPhone 竖屏下查看器是 sheet，顶上留着一条缝，灵动岛碰不到它的内容，所以没有为岛另做处理。如果用户指的是别的屏幕（例如某个全屏页面），要请他指明。

## 待真机验收：iOS 到达动效的第一帧不再停在窗口角上

2026-11-12 用户反馈「iOS 端收到新消息的时候，那个动画会先在右上角出来，然后再闪到中间」。我理解为到达动效（气泡）的第一帧：新建的气泡在被画出来之前先在窗口原点露出 1–2 帧，再跳到中间。模拟器上逐帧看的屏幕录像里，这个现象在 iPhone 17 / iOS 26.2 与 Duo 外屏 / iOS 27.1 上每次都有，位置是**左上角**（用户说的是右上角）；修复（创建即透明、先摆在起点、计时从第一次画出来那一帧起算，`ArrivalSceneHost.swift`）后，同一台 Duo 的录像里一帧也没有。规格见 [到达动效的「第一帧」一节](specs/ios-arrival-motion.md#第一帧被画出来之前不可见2026-11-12)，验证记录见[实施计划](plans/ios-arrival-motion.md#第一帧修复2026-11-12)。

还没做的：

1. **真机没看过**（已随内外 TestFlight `0.1.0 (21)` 分发，见 [Build 21 交付记录](plans/ios-channel.md#2026-11-13testflight-内部与外部构建-21)）。更新后在手机上让几条新请求到达，看有没有东西先在角上露一下。**如果用户说的「右上角」是另一个东西**（别的屏幕或方向，或不是白色气泡本身），请他说明机型、方向、哪个页面、每次还是偶尔，再查；我只在模拟器上复现了左上角这一种。
2. iOS 26.2 上修复后的录像没拍（磁盘只剩几 GB，临时的 26.2 模拟器已删）；Duo 与 iOS 18.5 的结果覆盖同一机制。

## 待办：iOS 会话历史的 MCP 兜底分区（后置）

[会话历史规格](specs/ios-session-history.md#11-后置本期不做)本期只对 Mac 知道真实会话 id 的请求提供（S6，用户 2026-11-11 同意收窄）。MCP 实例兜底分区（Mac 历史窗口里「近似」的分组，见 `reply-history.md` F2）与没有会话信息的请求没有入口，所以只有拿不到真实会话 id 的调用方用不了这个功能。要补的话：把 MCP 实例 id 与项目一路带到 iOS 的投递点（现在只有真实会话才有 `SessionRef`，4 处投递点都得改），给会话键增加一种形式，并在界面上明示「近似」。没有排期，等有人真的因此用不了再做。

## 待实际鼠标确认：作答弹窗整页滚动偏移

2026-11-11 用户反馈罗技鼠标在弹窗中滚动会让整张页面（含 Sidebar）偏离窗口，停滚后不恢复。已在主工作树限定独立／合并弹窗的根节点滚动与越界回弹，正文和 Sidebar 继续独立滚动；[约定与证据](specs/popup-request-inbox.md#3-窗口与导航)。原生 WebKit 19 项检查、真实合并界面滚动、99 项相关测试、类型检查及 `./scripts/install.sh` 通过。未发布。

用户暂时无法测试。下一步用实际罗技鼠标滚到正文／Sidebar 边界，确认整页不再移走；当前证据来自合成连续事件，尚不能确认驱动实际发送的事件序列。

## 待真机验收：iOS 附件跨 App 拖出

2026-11-11 用户批准从提问附件列表与预览页拖出，并要求文件在 item provider 中异步下载。实现与边界见 [规格](specs/ios-attachment-drag-export.md)。256 项相关 Swift 测试、模拟器编译、4 条真实跨 App 拖放用例与 4 条原附件界面回归通过；独立接收进程获得原文件名、类型及相同 SHA-256 的文件。文件正文长按仍可选字并唤起复制菜单；`./scripts/install.sh` 已通过。已随内外 TestFlight `0.1.0 (19)` 分发，见 [Build 19 交付记录](plans/ios-channel.md#2026-11-12testflight-内部与外部构建-19)。

下一步：更新到 Build 19 后在真机验证「文件」、备忘录与常用聊天 App，含未下载附件、取消与较慢网络。Duo 模拟器未安装这些 App，本轮跨进程实测使用仓库内独立接收测试 App，目标 App 兼容性仍待验收。

## 待真机确认：iOS 收件箱使用提问时间

2026-11-11 用户确认列表显示的是手机接收时间，详情页的 Agent 提问时间才正确；已在主工作树修复列表相对时间、组内排序与项目分组排序，统一使用 `summary.createdAt`，接收时间保留给内部到达逻辑。
约定见 [iOS 规格 §6.2](specs/ios-channel.md#62-收件箱)。72 项相关 Swift 测试、iOS arm64 模拟器编译和 `./scripts/install.sh` 通过。
已随内外 TestFlight `0.1.0 (18)` 分发（源码 `0e177795`，见 [Build 18 交付记录](plans/ios-channel.md#2026-11-11testflight-内部与外部构建-18)）；下一步在手机确认离线补同步的旧题仍显示原提问时间，完成后删除本节。

## 待日常观察 Cursor 离开汇报后结束

2026-11-09 已完成主树实现并安装，开关位于 Cursor 集成，默认关闭。
真实 CLI/MCP 私有回归已验证提前回执、当前列表待阅、已阅、关停恢复、设置关闭及结束离开后的保留、⚡ 优先和来源隔离。
Rust 2925 passed / 11 ignored，前端 1799 passed，Node 209 passed / 21 skipped，严格 clippy 与安装通过。
契约和证据见 [带理由升级 §9](specs/away-completion-guidance.md#9-cursor-离开升级转待阅)。

下一步在用户实际启用后，观察真实 Cursor 收到回执能否自然结束本轮，次日可在当前列表阅读并点已阅。
本轮未另外启动真实模型会话，确定性验证中的 Cursor 来源来自私有绑定夹具。

## 待日常关机观察 作答宿主静默退出

2026-11-09 已在主工作树修复并安装，用户完成现有待答后，已正常重启 daemon 并确认旧作答宿主退出、新宿主启用。
AppKit 的 applicationShouldTerminate 读取系统 Quit 原因，配合 workspace 通知，在窗口退出前提交本实例的停止与取消。
Rust 2921 项、Node 209 项、严格 clippy、私有实窗的关机／重启／注销／独立弹窗及普通崩溃恢复、既有接班 smoke 均通过。
契约与验证见 [合并作答窗口 §5.1](specs/popup-request-inbox.md#51-系统结束会话)。

下一步在日常真实 Mac 关机时确认作答窗口安静退出，不再显示「作答窗口已恢复」提示。
本轮系统退出事件只定向发送给私有测试进程，整机关机仍待观察。

## 正式实例账户路由的原生平台验收

2026-11-09 已完成主工作树实现与 Astra xhigh 复审，macOS 核心及 Swift helper 的本机验证见
[账户主目录路由](specs/system-instance-home-routing.md#11-当前状态)。
完成的实施记录已移出本文件，保留以下原生验证步骤。

- Linux 上核对临时／空／缺失 HOME 的正式根、冷启动与继承旧临时实例的原地 exec，运行路由和 Dev 转送 smoke。
- Windows 上核对 Profile Known Folder、reparse 拒绝、后台启动环境、named pipe 身份与 fixture 换新保护。


## 待真机验收：iOS 新建任务记住 Mac

2026-11-08 已在主工作树实现：手动选择 Mac 时立即保存手机本地偏好，取消创建或重启 App 仍保留；
已记忆 Mac 不可用时临时回退，不覆盖偏好，入口明确指定的 Mac 优先。约定见 [iOS 规格 §6.8](specs/ios-channel.md#68-新建任务)。
`AgentStoreTests` 33 项通过，含 6 项新增回归；`./scripts/install.sh` 已通过并安装日常版（不更新 iOS App）。
已随 TestFlight `0.1.0 (14)` 分发。下一步在真机确认取消与重启后仍选上次手动选择的 Mac。
已有项目待办 `b9be6812-78b1-4381-8bec-8589aa1a9da3`，真机确认后删除本节。

## 待真机验收：Agent 观察最新思考

2026-11-08 已按用户确认在主工作树实现 Mac Agent 控制台与 iPhone 实时观察的最新正文 / 公开思考展示。
采用整段定稿更新、固定 10 秒节流窗口，以及展开 / 选字 / 上滑阅读锁。约定与验证见
[Agent 观察中的最新正文与思考](specs/agent-observation-latest-text.md)。
Mac 日常版已安装，并通过新安装 AskHuman 交互及真实 Cursor 只读观察帧验证。
Rust 全量 2845 passed / 8 ignored，最终 Cursor 专项 12 项、定稿候选 4 项、控制台 36 项、Swift 状态 4 项通过，
前端类型检查与 iOS arm64 模拟器编译通过。用户已看过真实会话内容的原生回放并确认密度，连续末尾预览已修正。
已随 TestFlight `0.1.0 (14)` 分发。下一步在手机确认思考及时更新、展开阅读保持稳定及「有新内容」切换。
已有项目待办 #11 `e8ddc583-a8c9-4e75-b84c-cb994f256d62`，真机确认后删除本节。

## 待真机确认：Build 11 回答框焦点卡死

2026-11-07 用户提供的三份 0.1.0 (11) / iOS 27.3 Beta 日志均为 watchdog 终止，主线程同时包含
AttributeGraph 更新与 `UITextView.becomeFirstResponder`；用户确认主要在点回答输入框时发生。
已将 `CaretTextView.updateUIView` 的焦点获取 / 释放统一延后并合并，执行时读取最新绑定，避免在更新中重入及过期请求抢焦点。
实现与有限验证见 [iOS 实施记录](plans/ios-channel.md#2026-11-07build-11-回答框焦点卡死的本地修复)。
模拟器编译与 Mock 下点输入框、按键输入、发送一题通过；两条新增 UI 回归没有取得完整通过结果。
2026-11-07 修复已随内部 TestFlight `0.1.0 (13)` 分发，用户确认已更新、原有 Mac 连接正常，见 [Build 13 交付记录](plans/ios-channel.md#2026-11-07testflight-内部构建-13)。

下一步：在 iOS 27.3 Beta 手机上观察点输入框、切题、后台返回与发送；
有可用资源时补跑 `AskHumanFocusUITests` 的两条回归。完成真机确认后删除本节。

## 待界面验收：桌面新建任务设置与 iOS 对齐

2026-11-06 用户确认在主工作树实施，模型、思考强度、Codex Fast、Cursor Max 与桌面本机偏好已补齐，
覆盖待办 ▶、托盘新建任务和控制台「＋」。规则见 [GUI 规格 §8](specs/gui-agent-task-launch.md#8-桌面新建任务设置与-ios-对齐2026-11-06)，
实现与验证见 [实施记录](plans/gui-task-settings.md)。

已完成：前端全量 1293 项、Node 139 项、Rust 2614 项通过；最终表单专项 36 项通过，类型检查与生产构建成功；
已 `./scripts/install.sh` 安装到日常环境并用新安装的 AskHuman 请求验收。未发真实模型任务。
用户此时开启离开模式，尚无真人界面反馈；原生窗口工具无法定位未注册 App bundle 的 GUI 宿主，不能以组件测试代替实窗验收。

下一步：从待办 ▶ 选 Codex 查看模型 / 强度 / Fast，切 Cursor 看模型 / Max 的显示，再核对托盘与控制台共享入口。
已有旧窗口需关闭后让宿主换新，或使用托盘「重启以完成更新」。真实计费启动由用户另行授权或手动执行。
用户已接受当前实现与自动化验证作为本次交付，明确选择保留实窗未确认记录、日常使用时观察；
后续验收已登记项目待办 #9（`90e4972e-cf41-4c59-9a0b-a216989915fb`）。
完成界面确认后删除本节；用户已授权将本次变更单独提交并推送 main，主树另有并行待办同步改动。

## 待首位朋友真机验收：iOS 外部 TestFlight

用户已批准 [开发计划](plans/ios-external-testflight.md)：首批 1–5 位朋友，各自 Mac 与 EMQX，开发者提供现有推送中继；外部邮件邀请，无公开加入链接，先不提供演示模式、直接尝试 Beta 审核。
工作树 `HumanInLoop-ios-external-testflight`（`codex/ios-external-testflight`），不挂渠道预设。工具、公开材料与 App 内链接已完成；24 项工具回归、iOS 页面回归、最终安装及 Dev 转送通过。Pages 与 Mac 预览包已公开、Apple 审核资料与中英文测试说明已读回。
用户已批准启用 GitHub Pages、发布独立 Mac 预览包并继续上传/送审；审核联系人选定 Spatialify，个人信息只存私有输入并提交 Apple。
完成标准与分阶段验证见计划；未经批准不加演示模式、不迁移中继、不自动创建定时任务。
2026-11-05 23:45（北京时间）已提交 **0.1.0 (10)**：Apple build `81b235eb-4db3-46ee-8372-e74ca5bcbe0a`，VALID、APP_STORE_ELIGIBLE、`WAITING_FOR_BETA_REVIEW`（审核提交 `WAITING_FOR_REVIEW`）。源码与本地标签 `ios-testflight-10` 固定 `d3bb833`；后续 main 的已结束问题页面改动不在此包。首次上传最终确认 401 已通过同归档受控重试解决，不再重传。
2026-11-06 Build 10 获批后已向用户指定的一位朋友发出邀请。2026-11-09 Build 15 已在 `AskHuman Friends` 进入 `IN_BETA_TESTING`，沿用原邀请，本次 2026-11-09 只读查询已显示 `INSTALLED: 1`；独立配对与推送验收尚未确认。见 [Build 15 交付记录](plans/ios-channel.md#2026-11-09testflight-内部与外部构建-15)。2026-11-10 Build 16 同样进入两组 `IN_BETA_TESTING`，成员未变，见 [Build 16 交付记录](plans/ios-channel.md#2026-11-10testflight-内部与外部构建-16)。2026-11-11 Build 17 已在两组 `IN_BETA_TESTING`，继续沿用现有成员，见 [Build 17 记录](plans/ios-channel.md#2026-11-11testflight-内部与外部构建-17)。
下一步：首位朋友按[中文接入指南](https://naituw.github.io/AskHuman/beta-zh/)安装[最新版 AskHuman](https://github.com/Naituw/AskHuman/releases/latest)，完成配对、前台回答、锁屏推送、切网与首答验证。已有项目待办 `73650ea1-449e-42bd-91e4-61a18478d84d`。

## 待平台验收：Daemon 更新接班的 Windows / Linux 原生路径

2026-11-05 [更新接班与错误卡修复](specs/daemon-update-and-failure-recovery.md)已合并 main 并装为日常版。
macOS：相关 325 条测试、提交前 clippy、Dev 正式转送、日常二进制的接班与十场景错误卡回归均已通过，运行指纹已核对。
尚未做的原生验收：

- [ ] Windows：接班子进程等单实例锁、并发 CLI、启动失败保留旧服务、30 秒等待截止；需要 Windows 环境，不以 macOS 代验。
- [ ] Linux：稳定安装入口的 ELF 替换、exec 时锁释放、同 PID 后继、stop/信号不自启；隔离脚本已提供，但未在 Linux 上实际执行。

真实 Cursor 仍仅观察自然失败，不触发 Agent 验证。崩溃自愈是用户定下的另一个设计任务，已记项目待办，不在这次实现内。

## 待验收：Agent 最终失败确认（已合并并装为日常版；Cursor 的真实失败路径已验，其余未验）

2026-11-02 已快进合并进 `main` 并 `./scripts/install.sh` 装为日常版。需求
[规格](specs/agent-failure-confirmation.md)，计划与合并后的验证见
[计划 §10](plans/agent-failure-confirmation.md#10-合并与日常版安装2026-11-02)。
共享「结束／出错时确认」开关：最终失败进入普通错误卡，有原会话通道时可「要求继续」，否则仅「知道了」；
Grok 仅错误，Antigravity 不宣称支持；Codex CLI 只读观察共享服务，不投递队列。装机后真实 hook 配置未变，
只迁移了 Pi 扩展（已在运行的 Pi 会话要重启）；隔离链路脚本对日常二进制通过。已做的自动化与手机预选项验收
用的都是模拟终态，没有调用模型。

2026-11-03：手机 Agents 页的「需要处理」分组与「要求继续」按钮已去掉，错误卡成为唯一处理入口（iOS 未出包）。
本机三次真实 Cursor 出错（05:47／08:59／12:53，都在等待 AskHuman 回答期间）证实：Cursor 重试用尽后既不发 `stop`
也不发 `sessionEnd`，这类失败原本没有卡，会话一路变成「已结束」。按用户确认的 I4–I9 已实现：只读尾随 Cursor 结构化日志
检测出错（macOS，共用 Cursor 的确认开关）、`incomplete` 也出卡（不能接续的当作通知）、未处理的卡写入
`~/.askhuman/state/failure-cards.json` 跨 daemon 换新保留且不拦换新排空、「要求继续」的指令末尾带一句中性的
`--show-last` 提示。规格与决定见 [specs/agent-failure-inbox.md](specs/agent-failure-inbox.md)。

2026-11-04：上述实现已验证并 `./scripts/install.sh` 装为日常版，已推送：全量 `cargo test` 通过、CI 同参数 clippy 通过、
隔离冒烟 `scripts/agent-failure-cursor-smoke.mjs` 对装出的二进制通过（八个场景，规格 §7）。顺带修了一处：held 卡不计入
`active_count()` 后，菜单栏图标与托盘「待答」列表曾少算它们，现在改用含 held 卡的 `in_flight_count()`。
新 daemon 接管后（05:32）当天就遇到两次自然的 Cursor 出错（07:08、07:56）：检测、登记、卡内容都对，但**卡在约 2 秒内被自己
撤销**——开着「按需发送」时卡自己的提交把失败的会话拉回「工作中」（规格 §5.3）；隔离验证当时都关着该开关，所以没发现。
已修复并加了冒烟场景 8。修复版装机后又自然遇到三次 Cursor 出错（11-04 09:39、09:58、10:58），都核实了：卡留到用户处理
（出错后约 2 分钟、50 秒、13 秒，用户在手机上点了「要求继续」；修复前的两张卡 07:21、07:56 在 `history.jsonl` 里是 `cancel | caller`，
之后再没有）、`daemon.log` 有 `cursor log: failed turn …`、「要求继续」经 Cursor 桥让原会话开始新一轮（`agent-controls`
日志里三条都是 `started`，10:58 那条就是本会话自己的新一轮）、没有 `unreadable agent.turn.outcome` 告警（日志格式没漂移）。
10:58 那次还逐项对过 Cursor 自己的结构化日志：`error_text: Connection failed repeatedly`、`retries_attempted: 0`，
与卡文本和 `retries=0` 一致。

下一步：
- [ ] Cursor 出错卡只剩一条路径没有单独核实：**回答挂着的提问之后**立刻出现（上面三次里有没有碰到没单独查）。
  等下一次自然出错时顺手看一眼（Cursor 永不实测），核实后写回这里。
- [ ] 已知边界（规格 §4.2、§3）：会话已被挤出 registry「最近结束」列表（10 条）时对应失败不出卡；进程在回合中途消失
  （崩溃、被杀、终端关闭）没有任何来源报告失败，未纳入，等用户决定要不要作为仅通知的卡。
- [ ] 真实失败路径（真实模型请求须先经用户授权；Cursor 永不实测）：日常使用中遇到真实的 `selected model is at capacity`
  （或其它最终失败）时核对——Popup／iOS 收件箱出现错误卡、Mac 空闲门槛之后才推送、Codex Desktop 上「要求继续」
  确实让原会话开始新一轮、「知道了」保留失败状态。不必专门造故障。
- [ ] Codex CLI 现在只观察、只给「知道了」：0.163.0 的队列不能把继续指令原子绑定到「失败的那一轮」，普通 `turn/start`
  还可能插进电脑刚开始的新一轮。Codex 提供可绑定的接口（或能证明竞态不会发生）后再考虑开放继续（规格 §4.1）。
- [ ] （合并时读代码的推断，未触发）`agents/codex_cli.rs` 的 `Observer::failed_turn` 连接失败不退避：共享服务 socket
  残留而没人监听（本机 `~/.codex/app-server-control/` 里就是 6 月的旧 socket）时，daemon 会每 5 秒为每个「工作中」的
  非 Desktop Codex 会话起一次 `codex app-server proxy` 再退出。本机当前没有这类会话，采样 20 秒未见子进程；
  要处理就加按 home 的失败冷却（30–60 秒）。

## 待验收：已结束 / 空闲 Cursor Desktop 聊天的消息（手机 + Mac 控制台，真实 Cursor 里一次都没试过）

2026-11-04：registry 标为「已结束」的 Cursor Desktop 聊天（其实只是一小时无活动的判定，聊天还在 Cursor 里）手机现在也能发；
Mac 控制台对空闲 / 已结束的 Cursor Desktop 聊天不再只提示「回到原始 Agent 界面」，而是直接发（快照 `canMessage`、
`agents_message`、`ClientMsg::AgentMessage`，并入尚未发布的协议号 5）。规格见
[specs/agent-lifecycle-results-and-recovery.md §5.4](specs/agent-lifecycle-results-and-recovery.md)。
开发期间不得用 Cursor 触发 Agent（计费），所以只有 Cursor 替身的单测（状态闸、回执幂等、拒绝用语、registry 复活为运行中）、
控制台组件测试和 IPC 形状测试；没有向真实 Cursor 发过任何消息。iOS App 没改（已结束的会话本来就列在 Mac 上，`live` 不受影响）。

下一步：
- [ ] 用户首次真实使用时核对（不要为此专门触发 Agent）：给一个「已结束」或空闲、且**已在 Cursor 窗口里打开**的聊天发消息，
  Cursor 里是否出现这条用户消息并开始新一轮、控制台 / 手机是否转为运行中；聊天没在任何窗口里打开时，是否得到
  「Cursor 里没有打开这个聊天，没有发送」；桥对隔了很久的聊天（`isQuietForBackgroundSubmit`）是否照常放行。
- [ ] 若「聊天没在窗口里打开」很常见：考虑让桥先打开聊天（需要新的桥能力，须单独评估）；眼下按桥的既有前提直接拒绝。

## 待验收：项目开发环境的专用测试渠道与登录后的私有 Agent 会话

[PR #27](https://github.com/Naituw/AskHuman/pull/27) 已于 2026-11-01 合并，普通用户不需迁移，本机全局集成未改。
真实 Codex 与 iOS 转送证据见[计划 §7](plans/dev-verification-routing.md#7-真实-codex-与-ios-转送验证2026-10-31)。
2026-11-03 在临时 worktree 补做了本机界面、正式渠道转送与登录后 Grok 私有会话的验收（本机界面待办已完成），
并修了验收发现的五处问题——Dev 设置窗口仍显示 Agents 页、取消的转送请求等满 5 秒才收尾、`--resume` 把已死的验收显示为 `pending`、
`dev session grok` 传了 Grok 不接受的 `--add-dir`、终端回执被 `osascript` 的输出污染；
证据见[计划 §8](plans/dev-verification-routing.md#8-合并后的本机验收与修复2026-11-03)。仍未验证：

- Dev 专用测试渠道（`--delivery channels`）的真实收发：没有专用 bot／配对，用户决定先不配置。
  已做代码走查和「无渠道时明确失败」的实测，没有真实卡片往返。配好后用隔离测试转送端验收问答、权限、Stop、取消与发送失败，不替换日常正式 daemon。
- 其他 Agent 的登录后私有会话：Grok 的问答（Dev → 正式渠道 → 回到 Grok）与生命周期已验收；Grok 没有权限／Stop 卡，
  它的权限提示是 Grok 自己的终端提示，不归 AskHuman。Codex 暂无额度，Claude Code／Pi 未测，Cursor 永不实测；
  须由用户自己在私有 profile 里登录，不复制认证。Grok 的 Fork／转录控制与最终失败提醒也没在登录后实测。
  项目 CLI 的实际加载不代表当前 App 会话已刷新工具目录。

以上完成后删除本节。

## 进行中：iOS 原生渠道（原型已完成，待真实服务与真机验收）

2026-10-24 用户要求整理交接（[移动渠道交接](investigations/native-ios-channel-handoff.md)：杭州 EMQX 传输验证包、
Mac / iOS 交互原型、双向消息与进程重启补发已验证）。2026-10-25 经多轮问答定稿设计：需求
[`docs/specs/ios-channel.md`](specs/ios-channel.md)，计划 [`docs/plans/ios-channel.md`](plans/ios-channel.md)
（作为新渠道 `ios`；EMQX 杭州 + 腾讯云云函数推送中继 + 端到端加密；多 Mac 配对同一部手机；回答 / 确认 / 附件 /
Watch / Follow / 插话 / 新任务），实现地图见 [`docs/overview-ios.md`](overview-ios.md)，用户说明见
[`docs/wiki/ios-setup.md`](wiki/ios-setup.md)。实现在 worktree `HumanInLoop-ios`（分支 `feat/ios-channel`），2026-10-26 已快进合并进 `main`（worktree 与分支 2026-11-03 已删除，本地与 `origin` 都删了；删前分支指向 `15f1c491`，已完整并入 `main`，也没有 PR；渠道默认关闭：受 `experimental.enabled` 隐藏，`channels.ios.enabled` 默认 `false`；所有 `feat` / `fix` 提交都带 `Release-Note: skip`，不进发布说明）。代码已随 v0.16.0 发出，发布说明未提及该渠道。

**状态（2026-10-25）**：Mac Gateway / IPC 3→4 / 设置页 / CLI、推送中继 `relay/`、iOS App（收件箱 · 作答 · 确认 ·
附件 · Agents · 关注 · 实时观察 · 插话 · 新任务 · 历史 · 配对 · 通知扩展）全部实现，通过 Rust / Swift / 中继 / 前端测试、
模拟器 UI 测试（en + zh-Hans）和「真实 daemon + 本地 broker」端到端测试。真实服务接入进度（见 [P0 记录](investigations/ios-channel-p0.md)）：
**EMQX 国区实例已接通并验证**（Mac 连得上、账号互相隔离、延迟已量），**APNs 密钥已创建**，**腾讯云中继已部署并验证**（香港 Web 函数；
`/v1/health` 从 Mac 量 100 / 100 成功、p95 1.4 s，沙盒与生产的 APNs 凭据都被接受；函数 URL 只存在 Mac 的本机配置里，不入库），
腾讯云计费已由用户接受（第 0.5 项完成），真机签名构建已通过；同一套端到端对真实 EMQX + 中继跑了一遍，33 项全过（前台延迟：回答 → Mac 确认 p95 195 ms、AskHuman 调用 → 手机显示 p95 254 ms，达标），
顺带修了一个 Mac 端竞态（先 hello、后上报前台的手机漏收中间的变化，计划 §13）。
**模拟器上的真 App 对真实服务的完整场景已通过**（配对 + 校验码、通知权限 → token → 中继登记 → Mac 持有句柄、前台单题 / 多题、权限确认允许 / 带理由拒绝、
Agents 列表 + 实时观看 + 关注 + 插话读取回执 + 完成横幅、任务关闭时的表单提示、真 APNs 沙盒推送 → 通知扩展解密 → 点击打开请求、被关注 Agent 的 `follow.done` 推送、历史、解除配对；
基础场景连续三次、完整场景两次，[P0 记录 §3.7](investigations/ios-channel-p0.md)），并找出四个真实缺陷已修复（点击横幅崩溃、扩展解不开中继的信封、快速 inactive → active 后永久断线、推送句柄没可靠送到 Mac，计划 §13）；
**真机（iPhone Air，2026-10-26，手工，单样本）**：前台提问、锁屏真 APNs 推送（横幅带解密后的问题文字，点横幅直达详情页）、强退后真 APNs 推送（冷启动并直达该问题）各通过一次，答案逐字一致；
真机上又找到第五个缺陷并修复：`inactive` 也告别——控制中心 / 通知中心 / 多任务界面盖在 App 上会让 Mac 看到手机离线 5.9–6.9 秒（现在 `inactive` 超过 10 秒才告别，提交 `fix(ios): keep the connection while the app is only inactive`，
[P0 记录 §3.8](investigations/ios-channel-p0.md)）。**剩下的是真机矩阵的其余项与手机侧测量。**

约束沿用交接：两个 EMQX 实例的消费上限保持 0，不创建管理 API Key，不泄露任何密钥或本地私有配置；不得把模拟器 /
演示 Agent 验证当作完整渠道验收；真实 Agent 的启动 / 发 prompt 测试必须先经用户批准；密码 / `.p8` 只由用户自己输入，Agent 不读取。

下一步（需要用户账号 / 真机，一次一步经 AskHuman 引导）：
- [ ] P0 门禁剩余项（记在 `docs/investigations/ios-channel-p0.md`）：手机在 Wi-Fi / 蜂窝（关闭代理）上调 `/v1/health` 100 次、手机到 EMQX 的 TLS 与 WSS 往返各 100 次、
  真机 APNs（≥ 30 次送达耗时分布、空闲 ≥ 30 分钟后第一条推送的耗时；加密预览、扩展解密、点击回到 App 已在真机各看过一次；Mac 侧空闲 3 小时后的 `/v1/health` 首次请求已量：1305 ms）；任一项不达标先停下问用户，不自行换方案。
  公司网络端口已验证：2026-10-26 用户口头确认「公司发的提问，我的手机可以收到」，没有用 `ios/Tools/broker-probe/probe.cjs` 量化，也没有区分 8883 / 8084 / 中继域名；要精确数据再让用户在那个网络运行探针（它不需要账号与密码）。
  **测量方式**（独立探针 App，还是产品 App 内只在 DEBUG 构建里存在的诊断与时间线）须先经 AskHuman 与用户确认；倾向后者——量的就是产品实际走的 TLS / WSS / 中继客户端与通知扩展，不需要另一个 Bundle ID 与签名，Release 构建不含
  （DEBUG 里记录 `scenePhase` 与连接事件的手机侧时间线，也能补上下面 `inactive` 三项的精确核对）
- [ ] 中继的函数 URL 被腾讯云标为「仅供测试使用」（限速 / 下线条件未公开）：原型期不阻塞，给朋友长期用之前建议绑自定义域名（香港地域不需要备案），由用户决定
- [ ] 真机矩阵其余项（计划 §9 手工矩阵；前台 / 锁屏 / 强退后的真 APNs 推送已各做过一次）：Wi-Fi / 蜂窝 / 切网、通知权限被拒、token 刷新、跨渠道同时回答、Mac 睡眠恢复、设备撤销、重装遗留配对。
  **做法**：用户 2026-10-26 明确反馈「已经测了很多遍，再测很浪费时间」——一次设计好、合并成尽量少的几轮，能用 `devicectl` 自己做的自己做，只有必须用手的动作才叫用户（备忘见 P0 记录末尾「真机手测备忘」）
- [ ] `inactive` 修复在真机上**未验证**的三项（用户决定不再手测，只在文档里记为未验证）：停留短于 10 秒不离线；锁屏 / 回主屏后 Mac 多快看到离线（告别从 `inactive` 挪到 `background`，估计晚 0.1–0.3 秒；最坏有「3 秒无确认补发推送」兜底）；
  强退后 Mac 多久看到离线（靠 Last Will）。有手机侧时间线（见上面 P0 门禁的测量方式）或做真机矩阵其余项时顺带量，不单独叫用户测
- [ ] 性能口径（需求 D10 四条）与流量预算（典型场景 ≤ 300 MB / 月）分段实测，报告原始分布与全部超时
- [ ] 合并进 main 之后仍待处理：
  - `.cargo/audit.toml` 里忽略了 5 条 `rustls-webpki 0.102.8` 的公告（`rumqttc 0.25.1` 带入；它只用这个 crate 的错误类型，证书校验走已修复的 0.103.16，理由与删除条件写在文件注释里）。
    `rumqttc` 换到用 webpki 0.103 的版本、或换掉该客户端后，**删掉这 5 条**并重跑 `cargo audit --file src-tauri/Cargo.lock`。合并后 `cargo audit` 比之前多一条 `unmaintained` 警告：`rustls-pemfile 2.2.0`（RUSTSEC-2025-0134，同样由 `rumqttc` 带入），只是警告，不影响 CI
  - 本机默认的 `cargo` 是 Homebrew 的 1.94.1，比 CI 的 `stable`（现为 1.99.0）旧，clippy 的结论会不同（新 clippy 对 `async_trait` 生成的方法报了 `double_must_use`，已在三个 trait 上 `allow`）。
    提交前要对齐 CI 时：`PATH=~/.rustup/toolchains/1.99.0-aarch64-apple-darwin/bin:$PATH CARGO_TARGET_DIR=/tmp/<独立目录> cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
    （在持久 shell 里用 `export` 会把工具链环境泄漏给后面的命令，写成行内前缀）
  - 决定是否公开该渠道（README 链接、去掉提交的 `Release-Note: skip`）
  - 这条分支的历史已在公开仓库（`origin/main`；分支 `feat/ios-channel` 2026-11-03 已删除）：推送前从历史里清掉了真实 EMQX 实例 ID 与 APNs Key ID（提交哈希因此改写过一次，文档里改用提交标题指代）。
    **不得再改写**（不要 rebase，也不要 force-push `main`）；iOS 的后续工作直接在 `main` 上做，或从 `main` 开新分支

已知延期（不阻塞原型）：
- Pi 的扩展只有在 Stop 确认开着时才会随 daemon 启动自动迁移（2026-10-26 实测：日志出现 `migrated outdated Stop confirmation hooks: pi`），只开了生命周期跟踪的用户仍要在设置里重新保存 / 刷新才会上报失败的回合；已在运行的 Pi 会话要重启后才加载新扩展。
- IM `/new` 与 iOS 新任务共用的启动流水线 `daemon/runtime/task_launch.rs::start_task` 没有自动化测试（该文件只有门槛顺序的 1 个测试）。2026-10-26 在真实飞书上走通过一轮 Codex 终端路径；TODO 出队、终端打不开时的清理、Codex App 路径只靠对照新旧实现读代码确认。要补自动测试，得先把 `open_terminal` / `todos` 等副作用抽成可注入的接缝。
- 提问正文里的本地图片尚未改写为 `ahasset://`，列表项的墓碑展示也未做；撤回插话只取回文字、不取回附件。
- 发行给朋友之前：EMQX 自动开户与额度共享、TestFlight 外部测试说明、隐私政策网址与联系邮箱。

## 进行中：Mac 互通（弹窗网）——需求与计划已评审通过，等 `ios-alert-timing` 合并后开工

2026-10-26 用户提出用 iOS 渠道的 MQTT 设施让多台 Mac 互通（在公司 Mac 上回答家里 Agent 的提问）。经 6 轮问答定稿设计，2026-10-27 写成需求
[`docs/specs/mac-mesh.md`](specs/mac-mesh.md) 与计划 [`docs/plans/mac-mesh.md`](plans/mac-mesh.md)，同日经用户评审通过（结论见需求 §17）；**目前没有任何代码改动**。

要点：每台 Mac 的 Daemon 新增「答题端」，其他 Mac 的提问进入同一个合并作答窗口（带 Mac 标签、按「Mac + 项目」分组）；双向对称，v1 只回答提问与权限 / Stop 确认；
对端是「电脑出口」，永远参与，不受 `channels.autoActivation` 与活跃槽管辖；配对用「异步邀请 + 手机 / 本机代批」，新 Mac 经一次批准加入整个圈子
（两两独立密钥 + 票据转交式引荐）；提问方先发摘要、对端点开才拉正文，不用群密钥广播。依赖实验功能 iOS 渠道的 EMQX 与加密设施，不改 iOS App 与中继。
前置：并行的 `ios-alert-timing`（已批准、实现中，`docs/specs/ios-alert-timing.md`）要先合并——它把手机移出活跃槽，本需求按它落地后的语义写（需求 §6.4）。

下一步：
- [ ] 等 `ios-alert-timing` 合并（它改同一批代码，需求按它落地后的语义写）；合并前不开工
- [ ] 开工：先读 `docs/agent-worktree-setup.md`，经 AskHuman 确认 worktree 与渠道预设，再做 P0 预研（计划 §5）；P0.5 需要用户授权对真实 EMQX 跑保留消息检查；P4 之后有用户 gate（评审时已确认保留）

## 待验收：iOS 通知时机（已实现并装机，真机一轮只验了一部分）

2026-10-26 实现完成：iOS 不再受活跃槽 / `channels.autoActivation` 约束（每个提问和确认都挂到手机收件箱），手机没在看时的推送要等 Mac 键盘 / 鼠标空闲满
`channels.ios.alertAfterSecs`（默认 900 秒，`0` = 立即）才发，每题每手机最多一条、不重复提醒。规格 [`docs/specs/ios-alert-timing.md`](specs/ios-alert-timing.md)，
计划 [`docs/plans/ios-alert-timing.md`](plans/ios-alert-timing.md)。自动化（Rust 全量、前端、`vue-tsc`、clippy 1.99 `-D warnings`、fmt）全绿，已 `./scripts/install.sh` 装机；
在用户的 Mac 上确认过：活跃槽为 `popup`、`autoActivation` 开时，提问也会挂到手机（`AskHuman channel ios status` 的「待回答请求」≥ 1）；daemon 读到的「Mac 已空闲」与独立读数一致，
并随着停手而增长（0 → 5 秒）。iOS App、线协议、中继都没改。

2026-10-27 真机一轮（`alertAfterSecs` 临时设为 60 秒；Mac「在用」用 `CGEventPost` 的 mouse-moved 事件模拟；两个测试提问用 `AskHuman -q … &` 发出、`kill` 取消；
期间另有两个 Agent 会话的真实提问按同一规则被推送；配置已逐字节还原）：
- 已验证：Mac 空闲越过 60 秒的时刻前后，手机收到两条通知（测试提问一 + 另一个会话的真实提问）；取消提问一后，已送达的那条被改写成「已取消 / 已处理」类文字 ——
  **第一次真机证据：`collapse-id` 替换可用**。daemon 读到的空闲值无人操作时正常增长（到近 5 分钟），合成输入会把它清零。
- 没观察到（用户没盯着看或没回答）：Mac 在用期间手机**没有**通知；推送之后**不重复**；提问到达时 Mac 已空闲则**几秒内**推。这三条目前只有自动化测试（虚拟时钟）覆盖。

下一步：
- [ ] （可选）补测上面三条没观察到的：这次要盯着手机，只看这三点，约 3 分钟。做法：把配置里的 `channels.ios.alertAfterSecs` 临时改成 60（设置页会显示成「1 分钟（自定义）」）；
  「Mac 在用」用 `CGEventPost` 发 mouse-moved 事件模拟（实测会把空闲清零，不需要额外权限）；用 `AskHuman -q … &` 在后台发测试提问、`kill` 取消。时间线：Mac 在用时发一个提问（手机**不应**有通知）→
  停止输入，空闲满 60 秒时来**一条**且不重复 → 再发一个（此时 Mac 已空闲，应几秒内就推）。手机要关专注模式、App 在后台，Mac 上别有别的待答提问（否则它也会被推）。
  做完删掉配置里的键（原文件逐字节还原，可用 `shasum` 核对）。**不要用 `AskHuman help` 之类的词试命令**——它会被当成提问弹窗。
- [ ] 设置页（设置 → 实验 → iOS App 渠道 → 推送时机）的实际外观：下拉、说明、「Mac 已空闲 N 分钟」。组件测试只覆盖结构与文案，没有看过截图（这台机器没给命令行 / Computer Use 屏幕录制权限，没法自己截）。
- [ ] 规格「已知局限」9 里没验证的：远程桌面 / 屏幕共享的输入是否计入空闲；daemon 由 SSH 等非图形会话拉起时读到的空闲值是否可靠。有结果后写回规格，全部验完删掉本节。

后续（本期不做，细节在规格「后续」一节；Follow 的去留已另立一节，见下）：
- [ ] IM 统一（每个渠道各自的「提醒时机」）；保底封顶；有待答题时保持唤醒；D 到点时合并成一条「N 个提问在等你」；Time Sensitive、通知扩展送达回执、划掉追踪、手机上展示「Mac 在用」

## 待验收：iOS 手机端自愈（握手卡住 / 连接半死时自己恢复；已实现并过自动化，真机未验）

2026-10-29 用户反映：在电脑上工作、手机停在收件箱时，弹窗弹出，手机收到了推送，收件箱却是空的。没有任何日志（Mac 的 `daemon.log` 与手机端都不记握手 / 重连），
根因没能证实；最可能是手机的握手卡住（旧版 `hello` 一轮 3 次后放弃，晚到的确认被丢），其次是连接半死（看着连着、什么都进不来）。用户选了「改手机行为、不依赖找到根因」：
握手不放弃（30 / 60 / 120 秒封顶地继续问，认最近 8 条 `hello` 中任何一条的回应）、下拉刷新对没握手的 Mac 重新握手、App 在前台收到收件箱里没有的请求的推送时探测连接
（令牌回声，4 秒没回来就立刻重连）并重新握手 / 重拉快照。规格 [`docs/specs/ios-alert-timing.md`](specs/ios-alert-timing.md) D12，实现要点见 `docs/overview-ios.md`。
Swift 包全量通过（新增 16 个进程内用例，主要几条已逐条改坏实现确认会变红；另有 2 个对真实 daemon + 真实 MQTT 客户端 + 本地 broker 的端到端用例，端到端全量 43 个通过）、
App 目标编译通过；Mac 端、线协议、中继、推送规则都没改。探测依赖 broker 把自己发布的消息送回自己，EMQX 上这一点在 P0 验过，本次没有对真实 EMQX 再跑。**新版 App 没装到真机上。**

下一步：
- [ ] （需要用户在场）把新版 App 装到手机，之后留意「收到推送但收件箱是空的」是否还会出现、出现时是否几秒内自己补上。真机上没有可靠的办法造出「握手卡住」，所以这条只能靠使用中观察；
  复发而又没补上，说明根因不在这两处，下一步是加诊断日志（Mac 的 `daemon.log` 记握手 / 重连 / 3 秒补推，手机端留一份环形日志），而不是再猜。

## 待真机观察：iOS 同步补齐不播放到达动效

2026-11-01 用户确认：收件箱在前台实时新到的请求继续弹跳；回到 App、刷新和重连时同步补出的请求不弹跳，也不补到达震动。
手机端已区分快照与实时新增，并按手机单调接收时刻排除跨活动期排队的旧事件；轨迹、并发与落点保持原设计。
规格及验证见 [到达动效](specs/ios-arrival-motion.md#触发时机2026-11-01-修订) 和 [实施记录](plans/ios-arrival-motion.md#触发资格修复2026-11-01)。
2026-11-01 已通过 TestFlight 分发 `0.1.0 (4)`（源码 main `4381628c`），用户已更新并确认原有 Mac 连接正常。
触发时机已有真实引擎回归与模拟器验证；后续日常使用观察“回来补齐不弹、前台实时新到仍弹”，确认后删除本节。

## 待验收：iOS 开发者模式（P1–P5 已实现并过模拟器自动化；内测包 0.1.0 (5)、含新版项目选择的 (6) 已出，等用户真机试用反馈）

2026-11-02 用户要求给 iOS App 加「开发者模式」：Macs 标签里的开关（默认关）打开后，所有界面之上出现一个可拖动的浮动按钮，按住它、滑到条目松手；
v1 只有「快速添加待办」（不离开当前界面，把一句话记成某个项目的待办）。需求 [`docs/specs/ios-developer-mode.md`](specs/ios-developer-mode.md)
（D1–D21；其中「agent 设计」的部分用户已认可，原话在 §11；2026-11-03 起按钮与菜单是系统的 `UIMenu`，D18 / D19；同日项目选择改成与待办页同一套来源规则，D20 / D21），计划 [`docs/plans/ios-developer-mode.md`](plans/ios-developer-mode.md)。没有 Rust 与线协议改动。
2026-11-07 起菜单多了第二项「复制诊断信息」（D22、§12；起因见下面「待取证：iPhone Duo 副屏右侧栏的返回键等按钮突然不见」一节），已随 build 12 出去。

已完成：AHCore（位置 / `QuickTodoComposer`）与单测、App 的开关与两个独立窗口（按钮窗口、快速添加窗口）、项目选择（来源 → 项目两级，与待办页一样把开着同步的 Mac 并成「iCloud」、同步项目只出现一次、iCloud 目标按仓库地址记、写入时服务它的 Mac 掉线就换另一台）、中英文案、
`AskHumanDeveloperUITests`（18 条；iOS 27.0 的英文与中文、iOS 18.5 的项目选择相关 6 条都跑过，其中有 1 条偶发失败，见下）。按钮是系统按钮加系统菜单（iOS 26 起 Liquid Glass，更早毛玻璃）。实现中顺带修了一个既有的布局死循环：键盘从另一个窗口盖到全屏图片查看器上时 App 卡死
（`ImageGallery.swift` 每页改取外层 `GeometryReader` 的尺寸，图片位置比以前高约 11 pt；原因与约束见 `docs/overview-ios.md` 的不变量）。
2026-11-03 用户同意后用 `ios-testflight` 技能出了内测包 `0.1.0 (5)`（源码 main `2295390a`；Apple 状态 `VALID`，已进 `AskHuman Internal`），记录见 `docs/plans/ios-channel.md`。**用户还没有在真机上用过。**

下一步：
- [ ] P6 真机一轮（内测包已出，等用户装上试用并反馈；要看的是）：系统菜单的弹出延迟与拖动的分工手感（模拟器上菜单约 75–180 ms 出现，按下后立刻移动则是拖动；
  按下后立刻划向菜单会变成拖按钮，是设计内的代价；真机上没有可调的常量）、iOS 26 玻璃的观感（按钮与提示条）、真机键盘与「关掉快速添加页后原输入框重新编辑」、
  按钮盖在 Safari / 图片查看器 / 系统弹窗之上的表现、触觉、VoiceOver。改手势规则前先经 AskHuman 确认，结果写回规格 §11。
- [ ] 新版项目选择（2026-11-03，与待办页同一套来源规则）已随内测包 `0.1.0 (6)` 出去（2026-11-04）。用户此前看过汇报后选了「先不出：你再看看，需要时再说」，但 build 6 是为 Cursor Max 开关出的 main 整包，
  出包前没有单独拿这一点问用户，出包后已如实告知。真机上请看：来源列表与待办页是否一致、iCloud 目标在服务它的 Mac 掉线后换机写入是否顺（规格 §8 记了换机的边界）、半屏高的 sheet 里项目列表是否太挤（用户定：保持现状，真机上看了再定；现在 sheet 是 `.medium` 与 `.large` 两档，打开选择时不会自动拉高；若要自动拉到大档，方案是选项目时拉到 `.large`、回到文本页时收回半屏，需先问用户）。
- [ ] 已知局限（规格 §8，真机用下来觉得别扭再提）：按钮盖在 App 内的系统弹窗（alert、确认对话框）之上，可能遮住弹窗边缘的按钮；iCloud 项目换机时若第一台其实已存下、只是应答丢了，重发去另一台会多出一条；换机时尚未同步到另一台的项目找不到。
- [ ] iOS 27.0 模拟器（`AskHuman iOS Dev`，iPhone 18 Pro）上大多失败的 UI 测试：`AskHumanDeveloperUITests.testTheButtonIsAboveASheetAndItsKeyboardAndRidesAboveTheKeyboard`——第一步在新建任务 sheet 上把按钮往下拖，按钮没动或停在半路（`AskHumanDeveloperUITests.swift` 的 `midY > screen.height - 200` 断言）。与项目选择无关：`6b59ebaa`（这次改动之前）的干净 worktree 上同样失败（出结果的 3 次全失败，另 2 次进程启动即被杀），改动后的树上 7 次里 1 次通过；iOS 18.5 / iPhone 16 上 2 次都通过，之前在 iPhone Air / iOS 27.0 上整班跑也过。原因没查：可能是 XCUITest 在 iOS 27 玻璃按钮上合成的拖动不稳（同一班其他拖动用例都通过），也可能是真实的拖动识别有问题；真机上留意「拖按钮」是否偶尔拖不动，尤其在 sheet 打开时（P6 已有这一项）。用户定：先放着，真机上拖按钮有问题再查。
- [ ] 模拟器回归（iPhone 16 / iOS 18.5，开发者模式关）的对照结果：其余 134 条 UI 测试里 127 条通过、3 条跳过、4 条失败；这 4 条在功能开发之前的提交（`190a76ad`）上用同一台模拟器跑，**同样失败或偶发**，与本功能无关，原因都没查：
  `AskHumanTodosUITests.testAMacThatPredatesTodosAsksForAnUpdate`（上文已记）与 `testMacsThatBothSyncAreOneICloudSourceWithTheirOwnProjectsAndTheAgentsDots`（`AskHumanTodosUITests.swift:162`，断言「iCloud names its Macs」；与前一条同样走来源菜单，疑似同一个 18.5 上菜单无障碍报法的问题）；
  `AskHumanInputVisibilityUITests.testShortAnswerAndAttachmentFitWithAccessibilityText` 与 `testEachQuestionRevealsItsOwnAttachmentEntryPoint`（都是 `Failed to synthesize event: Neither element nor any descendant has keyboard focus`：点了输入框没拿到键盘焦点；前一条稳定，后一条在基线上 2 次里 1 次失败）。第一条在 iOS 27.0 上通过（上文有记），其余三条没有换别的系统版本试过。

## 待验收：iOS 出包前的变更清单与 TestFlight 测试内容（build 7 已首次走完整条链，等用户在手机上看测试内容的显示）

2026-11-04 用户要求给 `ios-testflight` 技能补一步：出包前列出自上个构建以来的 iOS 提交，让用户确认包里有什么（起因：build 6 带上了用户选过「先不出」的改动）；
随后又要求把确认过的内容写进 TestFlight 的「测试内容」。需求 [ios-testflight-build-notes](specs/ios-testflight-build-notes.md)，计划与实施记录 [ios-testflight-build-notes](plans/ios-testflight-build-notes.md)。

已完成并验过：`changes.mjs`（基线标签、清单、「被丢掉的集合」校验）、签名 helper 的 `notes` 操作（`read`／`write`／`probe`）与 `helper.py notes`、skill 流程改写；
本机已装新 helper（换掉的旧版留作 `testflight-helper.previous`）；本地附注标签 `ios-testflight-3/4/5/6` 已补；对 build 6 做过 `read` 与 `probe`（两种语言都「写入→读回→删除」通过，事后没有残留）；
`status` 与 `upload.py … --check-signing` 在新 helper 下正常；离线测试 19 条（`npm test` 会跑）。
出包预检已放宽为只认 iPhone＋iPad（设备族 `1,2`，App 与通知扩展都要），并检查 iPad 方向声明，需求与计划 [ios-testflight-ipad-preflight](specs/ios-testflight-ipad-preflight.md)：
用 build 7 候选归档在本地验过预检与 `--check-signing`，build 6 的仅 iPhone 归档被预检与 helper 拒绝；helper 因此再次重装（换掉的版本留作 `testflight-helper.previous`），另有离线测试 3 条；候选归档已删除，没有上传。

2026-11-04 出 build 7 时整条链首次真用并走通：清单与确认门（走了一次）→ 归档、预检、上传 → Apple `VALID`（也是 Apple 第一次校验 iPad 构建，没有新的校验错误）→
中英文测试内容写入并读回逐字一致 → 标签 `ios-testflight-7` → 出包记录（见 `docs/plans/ios-channel.md` 的「TestFlight 内部构建 7」）。
同日用户改规则：今后确认门默认不经——用户要求出包就直接出，清单与测试内容在出包后的汇报里给出，要求先看才确认（规格 T12）。

下一步：
- [ ] 用户在手机 TestFlight 里看 build 7 的测试内容（简体中文与 English 各一份，多行与「•」）显示是否正常；有问题就改写法（规格「测试内容的写法」）并记进出包记录，没问题就删除本节。

## 待验收：iOS 大屏适配（iPad 与折叠屏内屏的两栏；已随内测包 0.1.0 (7) 出去，真机一轮未做）

2026-11-04 合并进 main，随内测包 `0.1.0 (7)` 第一次交付：App 设备族是 iPhone＋iPad，iPad 四向旋转、可分屏，仍是单窗口。需求 [specs/ios-large-screen.md](specs/ios-large-screen.md)，
说明在 `docs/overview-ios.md`。收件箱、Agents、待办、历史、Macs 五个标签页在窗口宽度 ≥ 700 pt 时左列表右详情；更窄时是系统的浮层侧栏；辅助功能字号（AX1 及以上）回落成单栏；
iPhone 与折叠屏外屏不变。只在 iPad mini 与折叠屏（Duo）模拟器上验过（竖屏、横屏、折叠／展开、辅助功能字号回落），没有在真实 iPad 或折叠屏手机上看过。

下一步：
- [ ] 有 iPad 或折叠屏手机时真机看一轮：竖屏、横屏、折叠／展开、分屏改变窗口大小时两栏是否正常、打开的那一项是否保留；点列表行的手感；键盘（含外接键盘）；辅助功能大字号下回落单栏。意见记进 `specs/ios-large-screen.md`。
- [ ] 2026-11-07 补上浮层分栏里再次点收件箱标签的入口。Duo 内屏竖屏模拟器已验证列表弹出、当前请求高亮、收起后草稿保留，已随内测包 `0.1.0 (12)` 出去（2026-11-07），待真机上确认。
- [ ] 2026-11-11 用户进一步复现：冷启动未选消息，横划收起后连收件箱 Tab 也无法找回列表；Macs 的「选择一个 Mac」状态同样受影响。已用 Duo iOS 27.1 + SDK 27.2 手动及 UI 测试在旧实现复现。空详情保护把 SwiftUI 可见性写回 `.all`，UIKit 的横划仍会收起实际侧栏，重复赋 `.all` 没有动作；原先只有收件箱接收 Tab 重选事件。主树现按 Tab 经环境向共用容器发送重选序号，并用公开的 `UISplitViewController.show(.primary)` 打开实际侧栏，保留空详情按钮不能关闭、横划可以关闭的行为。规格见 [窄浮层规则](specs/ios-large-screen.md#窄浮层的列表优先规则)。最终 11 项 UI 测试全过：五个 Tab 的冷启动未选内容下先验侧栏按钮不能收起、横划收起后连续三次点 Tab 找回；五个 Tab 已选内容后重选仍保留原项，另验草稿、发送后回列表、重新选项后收起和无配对引导。iOS arm64 模拟器编译与 `./scripts/install.sh` 通过。验证为 iOS 27.1 Duo + SDK 27.2，未跑全量 UI、其他系统或真机。已随内外 TestFlight `0.1.0 (18)` 分发（源码 `0e177795`），手机更新与实际表现待确认。
- [ ] 收件箱列表可见时来了新请求，气泡飞进那一行的蓝点（2026-11-07，用户：大屏分栏、收件箱可见时，弹跳动画之后消息没有飞到对应的「栏点」上，和设计的动画不同；「栏点」我按 Mac 的设计理解为新行的未读蓝点）：窗口宽到收件箱是一栏时，气泡收成蓝点飞进那一行，落地前蓝点不显示；行滚出可视区、不在收件箱页或单栏时仍落在标签 / 标题（规格 `specs/ios-arrival-motion.md`「宽屏分栏：飞进那一行的蓝点」）。只在 iPad 模拟器上看过（截图与 `AskHumanArrivalUITests.testInAWideWindowARequestFliesToTheUnreadDotOfItsRow`）；已随内测包 `0.1.0 (11)` 出去（2026-11-07），真机与折叠屏内屏没看过；若「栏点」指的是别的东西请纠正。
- [ ] 未定的小事（用户还没表态）：两栏里列表行的底色是系统在侧栏里给的卡片底色，不是手机上的白色（规格「选中行样式」）；真机上觉得别扭再提。
- [ ] 2026-11-10 修了「列表看不见时选中项换了，回来后选中行没高亮」（用户反馈；没选中的行现在自己画卡片底色，`RowCard`）。模拟器上验过 iOS 18.5 iPad mini、iOS 26.2 iPad、iOS 27.1 Duo（浮层与并排，浅色与深色），真机没看过；iOS 17、26.0 / 26.1 / 26.3 以上、27.0 / 27.2 的卡片底色没量过，真机或新系统上看到行与行之间有色差时对一下 `RowCard` 里的数。

## 待验收：iOS 顶栏按钮进折叠屏侧栏（已随内测包 0.1.0 (8) 出去，真机一轮未做）

2026-11-05 用户反馈：顶栏里很多文字按钮在 iPhone Duo 外屏上不会移到侧栏，操作不便。iOS 26 起，sheet 的取消 / 确认 / 关闭与待办项目页的「编辑」
改成系统的图标 / 角色按钮（✕、蓝色 ↑、✓、重新排序图标），外屏上会被系统放进侧栏；iOS 17–25 仍是文字按钮，逐字不变。
实测规则与写法约定在 [specs/ios-large-screen.md](specs/ios-large-screen.md) §6，计划与实施记录 [plans/ios-duo-toolbar.md](plans/ios-duo-toolbar.md)。
只在模拟器上验过：Duo 外屏竖屏逐屏（iOS 27.1）、iPhone 27.0 的 UI 测试、iOS 18.5 的文字回退；没有在真机上看过。

下一步：
- [ ] Duo 真机看一轮：新建任务、添加待办、编辑待办、配对、看图、待办项目页的重排，✕ / ↑ / ✓ 是否都在侧栏、顶栏里还有没有得伸手去点的文字按钮；蓝色 ↑（开始任务）是否看得懂。意见记进 `specs/ios-large-screen.md` §6。
- [ ] 外屏横屏、内屏竖屏／横屏没有用最终包逐屏复查（早先的探针已经定下规则：内屏竖屏没有侧栏，内屏横屏的 sheet 按钮留在卡片顶栏）；`AskHumanLargeScreenUITests` 也没跑。真机上顺手看一眼。
- [ ] 未定的小事（用户还没表态）：任务进度页的「关闭」在 iOS 26 起是蓝色 ✕（`.confirmationAction` 位置上的 `.close` 角色），延续了原先蓝色「Close」药丸的样子；觉得抢眼可以换成顶栏右侧的灰色 ✕。

## 待真机验收：Duo 折叠后的导航按钮

2026-11-08 用户补齐复现：内屏横屏从列表打开提问，再切外屏，返回、前后切题与 More 消失。
本地 iOS 27.1 Duo + SDK 27.2 已复现并修复；用户真机为 TestFlight / iOS 27.3 Beta，尚未用新代码复验。
同二进制对照定位到标签根与外层全局横幅 overlay 的共享栏承载：仅换原生 tabs 仍不足，只有恢复外层横幅 overlay 的对照会丢按钮。
最终由同一 UIKit 父容器托管稳定 UITab / UIHostingController 和独立横幅，保留原导航、列表和草稿；没有按尺寸重建或延迟强刷。
实现约束在 [specs/ios-large-screen.md](specs/ios-large-screen.md) §8，证据与完整结果在 [排查记录](investigations/ios-duo-fold-navigation.md)。

本地验证已完成：六条真实开合全部通过（请求每次另查四个按钮、草稿、切题、前后台与实际返回）；
Duo 适用的大屏 / 窄内屏用例、AX 返回、配对 / 解除配对 / 未配对、到达动效均通过；外屏原始五秒横幅实际点击通过。
宽屏横幅的瞬时自动化受换页和计时窗口影响，另以同 SHA-256 产品动态库的完整 RootView、独立 SwiftUI App Mock 持续横幅夹具，验证实际点击打开 Agent 并关闭横幅；没有修改产品代码或默认计时。
`install.sh` 再次成功，Mac 二进制内容无变化。临时诊断开关已移除，修复已随 TestFlight `0.1.0 (14)` 分发。
本次只操作 Duo `9E037F77-7859-4EF0-BB14-553A45928E69`，未操作其他会话的设备 / 进程。已有 AskHuman 待办 #10（`636dd45b-d512-4946-aa35-6db52918adc6`）。

下一步：
- [ ] 安装 Build 14 后，在真机 iOS 27.3 Beta 重走内屏横屏点开提问 → 外屏，复验四个按钮、草稿、实际返回、连续开合与前后台。
- [ ] 真机确认后删除本节；保留已有诊断功能。

## 待验收：iOS「做完 / 出错」通知收窄（Mac 端与 App 端都已实现，真机一轮未做）

2026-10-27 用户决定：App 里去掉手动「关注」星标（Agents 页本来就能看到所有 Agent 的状态），只保留「从手机创建的任务做完 / 出错」的自动通知，
并且和提问一致——Mac 键鼠空闲满 D（默认 15 分钟）才推，你在电脑前不打扰；你回到 Mac 前没推出去的通知作废；任务第一次出结果后就不再通知该会话（一次性）；
App 里用只读小铃铛标出「会通知你」。需求 [`docs/specs/ios-follow-narrowing.md`](specs/ios-follow-narrowing.md)、计划 [`docs/plans/ios-follow-narrowing.md`](plans/ios-follow-narrowing.md)，
同日全部批准。

已完成：Mac 端（P1–P2，已 `./scripts/install.sh` 装机）、App 端（P3，提交 `feat(ios): drop the follow star and mark notified Agents with a bell`）、文档回写（P4）。
自动化全绿：Rust 全量与 clippy / fmt、Swift 包全量、对真实 daemon + 本地 broker 的端到端、模拟器 UI 测试（en + zh-Hans）、
模拟器上真 App 对真实 EMQX + 中继 + APNs 沙盒的完整场景（含被通知的 Agent 第一次结果的推送，之后铃铛消失）。线协议、中继、配置项都没改；
旧版 App 的星标连新版 Mac 照常能加，同样一次性、同样遵守 D。**新版 App 没有装到用户的真机上，Mac 端的新规则也没有在真机上验证过。**

下一步：
- [ ] （可选，需要用户在场）真机一轮：把新版 App 装到手机（Xcode / `devicectl`），`alertAfterSecs` 临时设成 60 秒，从手机「新任务」发一个很快做完的任务
  （Face ID；真实 Agent 的启动要用户批准）——列表行出现铃铛、详情页有一行说明；Mac 在用时做完应**不**推；停手满 60 秒才来一条；
  做完之前回到 Mac 敲键盘则不应补推；做完后铃铛消失（`state.json` 的 `followSet` 为空）。配置用后还原；不要用 `AskHuman help` 之类的词试命令。

## 待真机验证：消息附注（在消息里选字写附注，随回答发给 Agent；Mac、弹窗与 iOS 模拟器已验，真机未验）

需求 [`docs/specs/message-annotations.md`](specs/message-annotations.md)，计划与各期实施记录 [`docs/plans/message-annotations.md`](plans/message-annotations.md) §14（含 2026-11-08 的验证记录）。
Mac（守护进程、CLI 输出、MCP、iOS 网关）、弹窗（在真实弹窗里点过）、iOS（协议、草稿、网页引擎、选字菜单、编辑面板、清单、底栏圆钮、历史、直接提问）都已入库；
iOS 在 iPhone 17（402 pt）/ 26.2 与 iPhone SE（375 pt）/ 18.5 的模拟器上过了自动化。375 pt 底栏按需求 D21 实测：浮动栏达标（辅助字号下一行比边距宽 12 pt，用户接受）；
经典栏原先计数被挤没，已按用户的选择改成「放不下就把计数挪到按钮行上方」。2026-11-09 经用户确认，随 TestFlight `0.1.0 (14)` 分发。

下一步：

1. 安装 Build 14 后在真机看 375 pt 机型或辅助字号下的底栏、多词与跨块的拖选、系统选字菜单里的「添加附注」入口与句柄、
   链接文字的菜单、整套手感。
2. 模拟器上还欠的（机器不抖时再补，一次只开一台）：改完经典栏之后，浮动栏在 iPhone SE / 26.2 上复测 `AskHumanBarUITests`（`TEST_RUNNER_AH_UI_CONTINUE=1`，
   `TEST_RUNNER_AH_UI_SHOTS` 收 `frames.txt`，对照需求 §4.3 的实测列）与附注用例；iOS 27.0 的对照（SE / 27.0 的测试运行器在内存吃紧时反复崩，一次没成功）；`AskHumanBarUITests` 的 zh-Hans。
3. 与附注无关、HEAD 上就失败的 iOS 26.x 量法：`AskHumanBarUITests` 里 `answered-count` 量到的是文字（23–39 pt，断言要求 ≥ 40 pt）、`quick-reply-menu` 框 54 pt（断言要求约 50 pt 的圆）；
   `AskHumanInputVisibilityUITests.testShortAnswerAndAttachmentFitWithAccessibilityText` 的键盘焦点事件。先在 27.0 上看是不是一样，再决定改断言的量法。
4. 整库 `vitest run`（`vitest.config.ts` 没排除 `todo-sync-helper/build/**`，另一会话处理）与整库 `cargo test` 在本功能之后没有重跑。

没有人在真环境里点过的：Windows / Linux 的真实弹窗；真实 macOS 弹窗里鼠标拖选（自动化的拖动不产生文字选区，只用过双击与三击）；浅色主题与 420 px 窄窗口的真实弹窗；
真实输入法组词；「查看源码」模式重新定位、历史窗口里的附注、共享窗口的未答标记（只有单元测试）；iPhone 真机上的多词与跨块选区、系统选字菜单里的「添加附注」入口、整套手感；
真实 Agent（Claude Code、Codex、Cursor）收到 `[user_annotations]` 区块后的反应（会调用模型，要用户授权）。iOS 17 用户决定不验，不记。

## 待验收：iOS 请求正文网页渲染、选项手感、「下一题」逻辑、图片查看与底栏（已实现并过模拟器测试，真机一轮待用户看）

2026-10-28 用户在真机上提了三点（提问页 Markdown 渲染不如 Mac 弹窗且流程图画不出来、不能选字；选项选中是渐变的、手感软；「下一题」会跳过看过没答的题），
选定「正式实现 WebView」后去休息，实现与取舍由我自行决定，记在需求 [`docs/specs/ios-web-markdown.md`](specs/ios-web-markdown.md)（W1–W17）、
[`docs/specs/ios-answer-flow.md`](specs/ios-answer-flow.md)（O1–O6、F1–F11）与 `docs/specs/ios-channel.md` §15 的 2026-10-28 一节，计划在 `docs/plans/ios-web-markdown.md`、`docs/plans/ios-answer-flow.md`。
已通过 Swift 包测试与模拟器 UI 测试；**没有在真机上看过**：凌晨手机能无线连上，Debug 构建已装到 iPhone Air，但启动不了（应是锁屏），我没有去点亮它。

下一步：
- [ ] 用户在真机上验收：Message 的渲染（表格、流程图、选字、链接）、选项点按手感、「下一题」往回滑后不跳题、辅助字号下的底栏；有意见记进 `ios-channel.md` §15
- [ ] 输入后点「下一题」与键盘接力（2026-11-04 用户在真机上反馈：在一道题里输入完点「下一题」，落下来的还是当前这题）：已修键盘收起时布局卡住的根因，并加了「在哪题输入就从哪题往后」与键盘接力（下一题是纯文本题时光标直接带过去、键盘不收，页面滑到那一题），见 `ios-answer-flow.md` §2.5、F4、F12、F13。只过了模拟器测试，已随内测包 `0.1.0 (7)` 出去（2026-11-04），**真机上没试**。真机上请看：输入后点「下一题」是否稳稳落在下一题、接力时页面滑动（约 0.3 秒）与闪一下是否顺手、带外接键盘的 iPad 上是否正常
- [ ] 真机冒烟：新版已装在 iPhone Air 上（10-29 凌晨 03:40 前后装过一次，10-30 凌晨又覆盖装了 `77697b7` 的 Debug 构建，含此前所有提交）；后一次启动过，只看了 Mock 的 Agent 页，Markdown 页还没人在真机上看；解锁后直接打开 App 就是新版。想在真机上看 Markdown 展示页（Mock，不碰真实数据）：
  `xcrun devicectl device process launch --device 00008150-00043D6A2E12401C --terminate-existing com.naituw.humaninloop.ios -- -AHMock YES -AHScenario markdown -AHOpen req-markdown`，从主屏再打开 App 就回到真实模式（见 `docs/plans/ios-web-markdown.md` §1.6）
- [ ] 图片缩略图与查看器（2026-10-29 用户在真机上反馈：图片缩略图不是 1:1、溢出格子，多张会叠在一起；查看图片时不能横滑到下一张）：已改成 1:1 方块缩略图与左右滑动的图片查看器（`ios-channel.md` §15 的 2026-10-29 一节），只过了模拟器测试（iOS 27.0 / 18.5），**真机上没滑过**。
  **这一版已装到 iPhone Air**（2026-10-30 凌晨装的，`77697b7` 的 Debug 构建，含此前所有提交），但没人在真机上滑过图片。装机做法：`xcodegen generate` 后 `xcodebuild build -project AskHuman.xcodeproj -scheme AskHuman -configuration Debug -destination 'generic/platform=iOS' -derivedDataPath <自己的目录> -allowProvisioningUpdates`，再 `xcrun devicectl device install app --device 00008150-00043D6A2E12401C <目录>/Build/Products/Debug-iphoneos/AskHuman.app`（别与别的会话共用 `ios/build/DerivedDataDevice`；手机解锁时 `devicectl device process launch` 才能起 App）。
  想看 Mock（四张不同比例的图 + 一份说明）：`xcrun devicectl device process launch --device 00008150-00043D6A2E12401C --terminate-existing com.naituw.humaninloop.ios -- -AHMock YES -AHScenario gallery -AHOpen req-gallery`。已知没做：放大后拖到边缘不会接着翻页（要先缩回原大小）；用户自己回复时附的图片仍不能点开查看
- [ ] 提问页 / 确认页的底栏（2026-10-29 用户：底栏不符合 Liquid Glass、不该有背后的蒙层）：iOS 26 起改成浮动的玻璃栏，栏自己没有底板与分隔线，内容滚到它后面（需求 `ios-answer-flow.md` §3.4 的 G1–G10，`ios-channel.md` §15 的 2026-10-29 底栏两节），iOS 17–25 仍是经典栏；计数块与按钮同高、提示在行上方的小条（G10，用户看过第一版后改的）。只过了模拟器测试；2026-10-30 凌晨已装到 iPhone Air（`77697b7` 的 Debug 构建），但用户只手点了 Agent 页的「Answer」（见下面「两条模拟器用例」），**外观还没得到反馈**。真机上请看：栏下面系统的柔和渐隐是否可以接受（我只去掉了自画的底板，保留了渐隐，因为关掉后文字读不清；若连它也不要，见 §3.4 G2）、「下一题 → 发送」换身份时的过渡、辅助字号下的叠放、提示小条与计数块的上下关系是否顺眼。Agent 页的插话栏与待办页的添加栏（用户随后从下一步选项里选的，需求 G11）也做成了同样的浮动玻璃栏：输入框是玻璃胶囊、附件 / 自动执行 / 发送是 44 pt 的玻璃圆钮，同样只过了模拟器（真机上只确认了点「Answer」不丢点按），外观上请看：输入框与圆钮同高是否顺眼、点输入框里的空白处是否弹键盘、发送中转圈的灰圈在禁用的灰钮上是否看得清；操作结果的提示胶囊（`interject-notice`、`todo-notice`）没改，仍是 `thinMaterial`，要不要也换成玻璃可以再定
- [ ] 小屏上的底栏与两个圆形图标按钮（2026-11-06 / 07，用户：小屏上多题表单底部空间不够、按钮被撑高；随后又指出快捷回复与其他按钮高矮、大小都不一样）：已改成逐级退让——放得下是「Answered 0/3」在左、按钮在右，其次「0/3」，再放不下计数块挪到按钮上方单独一行，按钮在任何情况下都是常规高度；快捷回复与「下一题」是与「发送」同高、宽等于高的圆（需求 `ios-answer-flow.md` §3.4 G12 / G13、验收第 15–16 条）。同一次改动里修了「多题表单输入后点发送，App 卡一分钟」（根因与修法见 G13）。**只过模拟器**（iOS 27 的 iPhone 18 Pro Max 与 iPhone SE、iOS 18.5 的经典栏、五档字号、中英文，`AskHumanBarUITests`）；已随内测包 `0.1.0 (11)` 出去（2026-11-07），还没上真机。真机上请看：SE 一类小屏上「0/3」与按钮的排布是否顺眼、辅助字号下计数独占一行是否合适、输入后点发送不再卡。已知没动：辅助字号下选了「发送并离开」，按钮上的字仍会被截成「Send and go aw…」，计数退成「0/3」（改动前就这样）。iOS 17–25 的经典栏也没动：390 pt 的 iPhone 16e（iOS 18.5）上选了「发送并离开」，计数仍被挤成「A…」、「Answered 0/3」默认就折成两行（按钮没有被撑高；逐级退让只做在 iOS 26 起的浮动栏上）；要不要让经典栏也用同一套，等用户定
- [ ] iOS 26.2 模拟器上 `testCreatingATaskWalksThroughTheFormAndOpensTheNewAgent`（新建任务页）稳定失败：一打开「开始」按钮（`task-start`）就是可点的，测试期望不可点（`AskHumanUITests.swift:1020`）。27.0 上通过；改动底栏之前的提交上也是这样，与底栏无关。原因没查（可能是 26.x 上工具栏按钮的 `isEnabled` 报法不同，也可能新建任务页在 26.x 上真的一打开就可开始），要先在 26.2 上看一眼页面再定
- [ ] 两条模拟器用例在非 27.0 的系统上不稳（2026-10-29 做输入栏时发现）：① `AskHumanTodosUITests.testAMacThatPredatesTodosAsksForAnUpdate` 在 iOS 26.2 与 18.5 上稳定失败（`AskHumanTodosUITests.swift:260`，源菜单里没找到「需要更新」），改动之前的代码上同样失败，27.0 通过，没去查（可能是菜单项在这两个系统上的无障碍报法不同，也可能菜单真的没写）；② `AskHumanUITests.testAnAgentWaitingForAnAnswerLinksToItsQuestion` 在 26.2 上会丢点按：Agent 页点「Answer」没有任何反应（按钮的动作根本没触发），改动之前的代码上 20 次失败 1 次，输入栏改成浮动玻璃栏之后 24 次失败 5 次，没能确定新栏是否让它更容易丢（见 `ios-channel.md` §15 该节）；真机（iPhone Air，2026-10-30，Mock 数据）上用户手点了若干次，每次都正常跳转，没有丢点按，所以更可能是 26.2 模拟器或 XCUITest 在负载下的毛病，不像新栏让真机变差；但次数没数、也没拿改动前的构建在真机上对照，不算定论。以后想把这条用例变稳，要先弄清点按为什么到不了按钮（可从栏的 `safeAreaBar` 下手查）
- [ ] 若真机上首屏仍觉得跳（页面报出第一个高度前是占位，真机约 0.1–0.2 秒）：做预渲染（请求到达时在后台先算好高度），见需求 §4
- [ ] 后置（需求 §4）：Mermaid 对齐 Mac 的 `sandbox`、`ahasset://` 本地图片、Markdown 附件预览改网页渲染、页内查找
- [ ] 仓库外的原型目录 `/Users/wutian/Developer/HumanInLoop-ios-web-md-proto/`（约 399 MB，正式实现已从中移植）可以删除，删之前请用户确认

## 待真机验收：iOS 消息里的链接（任意协议先确认再跳转、确认框指向被点的链接；已实现并过模拟器自动化，真机未验）

2026-11-11 用户要求：消息里特殊 Scheme 的链接要能跳转，且不预置白名单；并指出确认框是个 popover，却不指向被点的链接、弹在别处。
设计与取舍记在 [`docs/specs/ios-web-markdown.md`](specs/ios-web-markdown.md) 的 W7 与 §7 反馈意见：`LinkPolicy`（`AHMarkdown`）统一决定，其它协议先显示完整地址、确认后交给系统，没人认领时弹横幅；确认框改由 `LinkPrompt` 弹出，`LastTouch` 记下触摸位置作锚点。
已随内外 TestFlight **0.1.0 (17)** 分发，源码与分发状态见 [Build 17 记录](plans/ios-channel.md#2026-11-11testflight-内部与外部构建-17)。

已通过：`swift test --filter LinkPolicyTests`；`AskHumanWebUITests` 的 9 条链接用例（含既有的 `testALinkOnThePageAsksBeforeItOpens`）在 iPhone 18 Pro Max（iOS 27.0）与 iPhone 16（iOS 18.5）上全过（宽屏那条在手机上按设计跳过），iPad Pro 11 英寸（M5，iOS 27.0）上除「中文界面」与「交给 Maps」两条没跑外都过；选字、附注、「下一题」几条既有用例回归通过。**真机没看过，iOS 26.x 与 17 没有模拟器，没验。**

下一步：
- [ ] 真机验收：在 iPad（或展开的 Duo）上点网页里的链接、题干里的链接、附件预览里的链接，看 popover 的箭头是否都指着被点的位置；iPhone 上 iOS 27 是指向链接的小气泡、iOS 18 是底部 action sheet；点一个真装了对应 App 的自定义 Scheme（如 `shortcuts://`、`obsidian://…`）看是否跳过去，点一个没装的看是否出「没有 App 可以打开这个链接」横幅。想用 Mock 数据看（不碰真实数据）：`xcrun devicectl device process launch --device <设备> --terminate-existing com.naituw.humaninloop.ios -- -AHMock YES -AHScenario links -AHOpen req-links`
- [ ] 已知没做（要的话再定）：裸写的 `x://…` 不会被 markdown-it 的 `linkify` 认成链接（它只认内置协议），要写成 `[文字](x://…)` 或 `<x://…>`；`mailto:` / `tel:` 打不开（没有邮件 App 等）仍然静默，不弹「没有 App 可以打开」；没有「记住这个协议、以后不再问」的选项；键盘或旁白触发的链接没有触摸可指，确认框居中显示、不带箭头

## 待独立账号与稳定系统验收：iCloud 待办同步

2026-11-07 已按用户授权随 **v0.17.0** 公开发布；仍为实验性功能，默认关闭，要求 macOS 14+。
规格 [todo-sync](specs/todo-sync.md)、[分发规格](specs/todo-sync-release.md)，实现与完整验证记录见
[同步计划 §10](plans/todo-sync.md)与[分发计划](plans/todo-sync-release.md)。

已完成工程与分发：组件嵌入 Mac 程序、双架构 Developer ID 签名与 Production 公证、npm / Direct
资源传播、升级 / 回滚 / 停用保护；四平台候选与正式发布 CI、常规 CI 均通过。
本机已升级至 0.17.0，Production / 0 未上传 / 无安装错误；正式 Mac 包两架构信任与分发 smoke、
隔离环境从 npm 全新安装通过。源码、可达历史与实际二进制的密钥检查未发现真实凭据。

下一步仅保留尚未实测的边界：

- 独立 iCloud 账号从空状态首次使用，稳定版 macOS 14+，以及系统要求批准登录项的首次安装形态。
  已有朋友验收项目待办；没有真实记录前不标为通过。两台自用 Mac 的注册都直接 enabled。
- 日常观察双机真实待办是否重复、缺失或迟迟不到；异常先看 `AskHuman todo-sync status` 与
  `~/Library/Logs/AskHumanTodoSync/helper.log`。停用保留本机 / 云端数据；装前备份为
  `~/.askhuman/state/todos.json.before-todo-sync-2026-10-28`。
- 原 P0 性能基线仍未校准、未写 baseline；繁忙环境完整 A/B 未通过，本轮前主线的 stress 对比通过，
  未能证明原 P0 的差异来自本轮组件。保留[分发计划](plans/todo-sync-release.md)中的原始结果与限制。

用户明确取消新版运行满 24 小时的等待门槛，并要求本轮不做断网相关与睡眠测试；不为此等待或安排回查。
SSH 到 Mac B 的旧授权已用完，需要再次操作时当次确认。上述未验边界不阻止用户已批准的本次实验性发布。

## 待验收：iOS App 管理待办（已合并进 main，两台 Mac 与 iPhone Air 都已换上新版，决策全部是 agent 在用户睡觉时代定的，等用户审阅与真机验收）

需求 `docs/specs/ios-todos.md`（决策 I1–I16 在 §3，可整条否决）、计划 `docs/plans/ios-todos.md`；分支 `feat/ios-todos`（worktree `../HumanInLoop-todosync`，两者 2026-10-29 已清理）已于 2026-10-29 早上经用户同意快进合并进 main（本地，**未推送**），并用 `./scripts/install.sh` 装到了这台 Mac（装机后一次真实同步 tick 正常）；合并前已把 main 合进分支并在合并后的树上复测过。同日早上又应用户要求装到了 Mac B 与 iPhone Air：B 的二进制在 B 上用 B 自己的 Developer ID 重签后换入（指定要求与旧版逐字相同，旧版备份在 B 的 `~/.askhuman/backups/pre-ios-todos-install-20261029-080650/`），手机上是 Debug 构建（开发签名，用现成的描述文件，没有改 Apple 账号；没启动、没点亮手机，直接打开 App 就是新版，配对与数据不变）。用户在手机上打开待办页后发现状态行一直停在「正在同步…」：它取自 Mac 回答那一刻的快照，而打开页面时的订阅会让 Mac 立刻拉一次 iCloud（约 1 秒），第一份回答恰好撞上，同步结束后又没有任何通知；已修（`3e52cbb`，分支 `fix/ios-todos-sync-line`，已快进合并进 main，未推送）：手机见到 `syncing` 为真就静默复查，约 35 秒，见规格 §7；手机已换成修复后的构建（在干净 worktree 里构建）。又按用户的选择改了 iCloud 来源的失败表现（`56948da`，分支 `fix/ios-todos-icloud-failure`，已快进合并进 main，未推送）：一次请求没得到回答时不再清空列表，保留上次的行、打开着的项目和同步状态行，只在页面顶部提示一句；手机也已换成这个构建（同样没启动手机）。随后又按用户的选择改了决策 I3 的代价一栏（`8955ce8`，分支 `fix/ios-todos-icloud-offline`，已快进合并进 main，未推送）：iCloud 接入的 Mac 离线、又没有别的 Mac 能代答时，已加载的行与打开着的项目也保留（只读，页面顶部说它暂时连不上），与选某台 Mac 做来源时一致；同步被关掉或那台 Mac 被取消配对时仍然清空；这些行只在这次启动的内存里（I13），重启后没有行可留；手机已换成这个构建（同样没启动手机）。
用户 2026-10-29 夜里说「手机 App 想支持待办：能选设备、能选 iCloud 模式、还有项目来源的问题——你都设计，实现验证，我明天早上验收」，所以没有经过 AskHuman 问答。
做法：手机经已有的 iOS 网关（MQTT + 端到端加密）读写 Mac 的 `todos.json`，新增 `todo.*` 线消息；「iCloud」是一个**来源**，由开着同步且在线的 Mac 代答、掉线自动换，手机不直连 CloudKit（那要给 iOS App ID 开 iCloud 容器，是 Apple 账号上的改动，要用户授权）；项目列表由 Mac 按桌面侧栏同一算法算好，手机只看到不透明 id 与同名后缀，不暴露路径；手机不落盘待办。

2026-10-29 晚用户又反馈「待办里的状态比较混乱，如果我的 Mac 都开了 iCloud 存储，手机默认还是会选中一台 Mac，另一台 Mac 上的 Agent 就不会标绿点了」，经两轮 AskHuman 定了做法（规格 I17–I19、§13 第 11 条）：配对了多台 Mac 时，开着同步的 Mac 并进「iCloud」一个来源、不再单独出现，默认来源就是 iCloud（没手动选过时先用第一台在线的 Mac 起步，它说自己开着同步就换成 iCloud 并记住）；各 Mac 的「仅本机」项目在 iCloud 视图里按 Mac 分段列出（点进去在那台 Mac 上读写）；项目行的点复用 Agents 页的状态（橙 = 等你、绿 = 工作、灰 = 空闲），按仓库地址（Agent 项目信息里新增的可选 `repo`，Mac 开着同步时才带）或同机目录对到行。已实现并过自动化（Rust、Swift 包、模拟器 UI 测试），当晚装到了 Mac A（这台）、Mac B 与 iPhone Air：B 上用 B 自己的 Developer ID 重签后换入（指定要求与旧版逐字相同，旧版备份在 B 的 `~/.askhuman/backups/pre-todos-merge-install-20261029-233412/`），B 的旧 daemon 在换二进制后空闲退出，经用户同意用一次 `daemon start` 拉起新版（新 daemon 启动时按产品自带的迁移升级了 B 上 Codex 与 Cursor 的旧版 Stop 确认钩子）；手机是干净 HEAD 快照的 Debug 构建，没启动、没点亮手机。用户在真机上看过：来源菜单只有「iCloud」、副标题写着两台 Mac，提问挂着时这个仓库那一行有橙点，B 上 Agent 所在的行有点、颜色与 Agents 页一致。

自测（2026-10-29，都在这台机器上、没碰真实的 `~/.askhuman`）：Rust `cargo test ios::` 252 条 + `ios_todos` 25 条，clippy / fmt 无告警；Swift 包全套（合并 main 之后的树上）431 条（协议 27、存储 21、传输 14、Markdown 10、核心 144、应用 174、端到端 41，其中 4 条要真实服务、已跳过）无失败；模拟器 UI 测试 `AskHumanTodosUITests` 7 条，英文与中文各跑一遍并看过截图；合并后整个 `AskHumanUITests` 目标 90 条（待办 7、模型 15、主体 56、网页 11）全过，只有要真实服务的 `AskHumanLiveUITests` 1 条跳过；真实 daemon 端到端（本地 broker）5 条；另把真实 `todos.json` 的一份拷贝放进一次性 Mac 对账（15 行与文件逐项一致，最大一份整表 17 KB，拷贝已删）。UI 测试里发现并修复了两个缺陷：待办页先于 Mac 列表上屏时，旧 Mac 会让页面一直转圈（需求 §13 第 7 条）；合并 main 后，字符串目录脚本里「Copy」被本分支的「拷贝」盖过了 main 的「复制」，main 的网页渲染 UI 测试因此失败，已删掉重复项，脚本现在遇到同一个键两种译法会直接报错。**老 Mac 要先装带这个功能的版本**，否则手机上会显示「这台 Mac 需要更新」。

下一步：
- [ ] **用户验收**：看 `docs/specs/ios-todos.md` §3 的 16 条决策（尤其 I2 不直连 CloudKit、I4 设备模式不列仅云端项目、I5 手机不能新建项目、I13 手机不落盘），不同意的整条否决；真机上装一遍（`ios/` 里 `xcodegen generate`，配对后进「待办」标签）；多台 Mac 并成「iCloud」与行上的 Agent 点已在真机上看过（2026-10-29 夜），「仅在 X」分段、iCloud 来源自动换机、Mac 离线时的表现还要真机才看得全；打开待办页后状态行应当在几秒内从「正在同步…」变成「已同步 · 刚刚」，请确认上面的修复有效。
- [ ] 决定：默认来源记着「iCloud」之后，若所有 Mac 都关了同步，页面现在会说「iCloud 同步没开」、菜单里 Mac 重新出现，需要手动选一台（规格 §10 第 8 条）；要不要改成自动退回某台 Mac，等用户定。
- [ ] 后置（需求 §11）：手机直连 CloudKit（Mac 都不在线也能用，需用户授权开 iCloud 容器 + Swift 重实现同步核心）、附件的查看与添加、「全部待办」聚合页、清空历史、从 iCloud 移除项目、待办推送、离线编辑队列。
- [ ] 已知局限（需求 §10）：Mac 都不在线时手机看不到待办；线协议里 `helper` 没有「未安装」状态，手机上「没装」与「没在跑」显示成同一句；单条超大待办（桌面端 / CLI 不限长度，手机端限 4000 字）可能让那个项目的 `todo.list` 装不进一条消息而超时。

## 待验收：页内待办清单的完成 / 删除 / 添加（弹窗 + iOS 请求页；Mac 弹窗与模拟器已验，iPhone 真机与 Windows / Linux 未验）

需求 `docs/specs/todo-list-actions.md`（取代 `todo-quick-add.md` 的弹窗顶栏浮层与 iOS 请求页半屏添加页）。2026-11-13 在主工作树实现，本地提交、**未推送**，并 `./scripts/install.sh` 装为日常版；iOS 已应用户要求出了 TestFlight 内测包 **0.1.0 (22)**（Internal 与 Friends，源码 `9ed70abf`，记录见 `docs/plans/ios-channel.md`），等用户在真机上验。

- [ ] iPhone 真机：滑动手感——右滑完成（含一路滑到头松手）、左滑删除、滑开后再点、与页面纵向滚动、左缘返回手势、长按菜单会不会被自绘的拖动吃掉、键盘弹出时添加输入行是否可见。模拟器 UI 测试（`AskHumanTodoPickUITests`，中英文各一遍）只覆盖逻辑与手势的基本路径，需求 §6.1 要求真机看一次；手感不对先看 `SwipeActionRow`（水平分量 > 垂直 × 1.2 才接管、左缘 20pt 让给返回手势）与 `SwipeCoordinator.swallowsTap`（滑动之后吞掉整行的点按，否则松手会顺带选中）。
- [ ] Windows / Linux：弹窗里 Ctrl+T 聚焦页内清单的添加行（确认卡里则打开待办窗口）、添加输入行里 Enter / Esc 不漏出去（需求 §5.1「键盘隔离」；macOS 已按过）。
- [ ] 真实弹窗里没有按过的两处（只有单元测试）：Stop 确认卡的实时队列（要由真实 Agent 的 Stop 钩子触发）、权限确认卡上 ⌘T 与顶栏按钮开待办窗口。
- [ ] 对应的 Mac 要装带这次改动的版本（本机日常版已装）：旧 Mac 上手机的 Stop 确认卡仍是快照选项 + 折叠待办卡，碰到「不在待办候选里的在途项目」还会回「这个项目已不在」。
- 不是本功能带来的已有问题：`src-tauri/src/daemon/runtime/ios_tasks.rs:699` 的 `index % 2 == 0` 被新版 clippy 的 `manual_is_multiple_of` 拦下（本功能的验证用 `-A clippy::manual_is_multiple_of`，没有改它）。

## 待验收：特殊回复（手机选待办 / 离开模式 / 本会话不再使用 / 快捷回复）——P1–P4 已实现，已快进合入 main（本地，未推送）并装为日常版，已随内测包 `0.1.0 (9)` 出去

用户 2026-11-04 提出，经十轮 AskHuman 问答定稿；2026-11-05 授权「按顺序全部实现」（当时人不在，所以开工前的几个问题由实施者按默认值定下，记在计划的「开工前定下的事」里，
用户回来后可否决）。需求 [`docs/specs/special-replies.md`](specs/special-replies.md)
（界面稿在 `docs/assets/special-replies/`，浏览器原型在 `src/prototype/`，入口见 `docs/development.md`），计划
[`docs/plans/special-replies.md`](plans/special-replies.md)（文末「实施记录」按期记差异与验证结果，「总体没有验证的部分」是下面清单的出处）。
实现在分支 `feat/special-replies`（worktree 里开发，Dev Instance），2026-11-05 经用户同意快进合入本地 main（到 `6d6aed5f`，**未推送**），并 `./scripts/install.sh` 装为日常版（装机时日常 daemon 有一个别的会话的在途请求，换新走原地交接，请求没有被打断，日志里可见 fp `a81bff16` → `aaa29bbe`）；worktree 与分支随后清理。协议号 5 是并入未发布的那一版（v0.16.0 是 4）。

做了什么：四块能力共用 Daemon 里的「模式引擎」（`daemon/away.rs`，状态落盘 `~/.askhuman/state/away.json`）——① iOS 在提问里选项目待办、What's next 看完整队列；
② 离开模式（Agent 的每次提问与 whats-next 都被自动回复，带 `--escalate` 才像没离开一样送达；只能手动结束；Mac 桌面胶囊 / 托盘 / 控制台，手机常驻条）与会话级「不再使用 AskHuman」；
③ 弹窗与手机的快捷回复菜单（预设在 Mac 设置里编辑）；④ CLI `AskHuman away on/off/status` 与 IM `/away`、`/back`。全貌见 `docs/overview.md`「离开模式、本会话不再使用与快捷回复」。

自测（合并 main 之后；都在隔离的 Dev Instance / 临时 `ASKHUMAN_HOME` 里，没碰日常 daemon，没发到用户手机）：`cargo fmt --check` 与 `clippy -D warnings` 干净，`cargo test --bin AskHuman` 2596 通过 / 0 失败；
`vue-tsc` 通过、vitest 1276 条全过；Swift 包 672 条 0 失败；真实 Daemon 端到端 41 通过（3 条 `AgentE2ETests` 失败，在 main 上同样失败，见下）；
模拟器 UI 测试 128 条通过（`AskHumanAwayUITests` 中英文、`AskHumanTodoPickUITests`、主类 `AskHumanUITests` 63 条等，细节见计划「合并 main 与最终验证」；iOS 18.5 的经典条另验过，`AskHumanAwayUITests` 16/16，见「iOS 18.5 经典条补验」）；
隔离 Daemon 里用 `CLAUDECODE` + 会话 id 冒充 Claude 会话跑通了离开 / 升级 / 不再使用的端到端；
2026-11-05 又用**真实 Codex**（`codex exec`，隔离的一次性 Mac + 假手机，没碰日常 daemon）跑通四个场景：离开 → 提问与 whats-next 被自动回复 → `escalate` 送达手机；不再使用 → 后续调用即时回提示、Stop 确认静音；从手机解除 → 插话让它再次提问，细节见计划「真实 Codex 端到端」。

**不是本功能带来、但这次验证里撞到的 main 上的已有问题**（留给对应的同学 / 会话处理）：
- `AHE2ETests/AgentE2ETests` 的三条用例在 main 上就失败：`testAMessageToAnAgentThatIsNotWorkingIsRefused`、`testSessionsOfTheMacAreListedWithTheirState`（期望 `notWorking`，`agent_control.rs::can_interject` 现在对空闲会话返回 `idleUnsupported`）、`testAFollowedSessionReportsWhenItsTurnEnds`（等不到 `done` 事件）；用 main 的日常版二进制单独跑得到同样的失败。
- `AskHumanInputVisibilityUITests.testEachQuestionRevealsItsOwnAttachmentEntryPoint` 偶发失败（第二题输入框点按后没有键盘焦点），在 main 上同样复现。
- `AskHumanRequestNavigationUITests.testManualReturnRestoresTheReadingPositionOfALongForm`（`AskHumanRequestNavigationUITests.swift:101`，返回后长表单的阅读位置 15 秒内没回到原处）在 iOS 27.0 模拟器上多数时候失败，在 main（`b03606fc`，临时 worktree 单独构建）上同样 7/8；偶尔能过（重启模拟器后单跑过过一次），是已有的不稳定用例，原因没查（可能与 `-AHWebMarkdown YES` 下网页内容高度异步变化有关）。

下一步：
- [ ] **用户复核**需求 §3.10 里**没有单独确认过**的默认做法 N1–N10（实现按原文采用），其中「欢迎回来」的空闲门槛取 5 分钟（第 5 轮图文）；第 9 轮的提问正文误写成 15 分钟，以 5 分钟为准
- [ ] **真实 Agent 端到端的剩余部分**：Codex 一轮已跑通（见上），驱动测试已入库，要复跑：`AH_E2E_CODEX=1 AH_E2E_BIN=… AH_E2E_BROKER_DIR=… swift test --filter CodexAgentE2ETests`（付费，细节见 `docs/overview-ios.md`）。还没做的：Claude Code、Cursor 各一轮（开离开 → 提问与 whats-next 被自动回复 → `--escalate` 送达 → 结束；「不再使用」→ 即时提示、Stop 确认静音、解除后插话），要先有用户对真实 Agent 调用的授权；以及权限审批卡 / 出错卡在离开与不再使用时仍送达——`codex exec` 固定为不审批，触发不了，要在交互式会话里看（现在只有单测与 Stop hook 级的验证）
- [ ] **桌面胶囊在真实桌面上的交互**：悬停 / 点击展开、拖动后位置记忆、「欢迎回来」的触发（5 分钟空闲后敲键盘）、全屏应用与多桌面上的可见性。这台机器截不了屏、Computer Use 也列不出 AskHuman 的后台应用，只验了窗口几何（`CGWindowListCopyWindowInfo`）与浏览器里的真实组件；Windows / Linux 的托盘月亮与控制台也没看过
- [ ] **iOS 真机验收**：已随内测包 `0.1.0 (9)` 出去（2026-11-05，更新 TestFlight 即可；Mac 日常版已是带这次改动的版本，入口由 `caps.away` / `caps.answerTodos` 把关：新 App 连老 Mac 不显示入口，老 App 没有这些界面）。待用户在真机上看：离开模式的青绿色条（2026-11-07 由橙色改成系统 teal，见 `docs/specs/special-replies.md` D31，已随内测包 `0.1.0 (11)` 出去）与 Mac 的状态、计数一致，「发送并离开 / 发送并停用」不被截断，底栏菜单的展开方向与顺序，选项目待办后 Agent 收到的回答与 Mac 上待办的变化。iOS 17 的降级条样式、iPad / Duo 大屏上的常驻条没有看过（iOS 18.5 模拟器的经典条已逐屏看过并跑过 `AskHumanAwayUITests`，修了「发送并离开」按钮被截断，见计划「iOS 18.5 经典条补验」）
- [ ] 控制台里 Agent 页头的横幅 / 会话菜单在真实数据上的样子没再核对（组件与逻辑有测试）

## 待验收：iOS 已结束的请求页（钉住的状态条 + 正文变灰 + 控件停用，另有「下一条 / 关闭」按钮与 5 秒倒计时；已实现并过模拟器自动化，手机上没看过；按钮与倒计时已随内测包 `0.1.0 (16)` 出去，2026-11-10）

用户 2026-11-05 在用过离开模式的版本后反馈：别处已回答的问题，手机上「不够明显」，读着读着才发现不用回答了。范围他选了「整个请求已在别处结束（Mac 弹窗 / IM 答了、Agent 取消、超时），手机上的页面还开着」，
形式让我设计（「不一定是整体变淡」）；之后他问「往下滑它会消失吗」，我答不会（钉住），他选了「保持钉住」。需求 [`docs/specs/ios-answer-flow.md`](specs/ios-answer-flow.md) §3.7（E1–E8）、验收第 13–14 条；
实现在 `ios/AskHuman/Views/Request/EndedState.swift`（提问页与确认页各挂一次）。已推送到 `origin/main`（2026-11-05，`f5167d3a`、`2c752ed6`、`9666acaf`），还没有带它的 TestFlight 包。

做了什么：页面开着时请求在别处结束，导航栏下钉一条带色状态条（「无需回复」+ 原因句；颜色跟结果走，与历史页一致），正文（消息、各题、确认页的详情 / 选项 / 输入框）不透明度 0.6，控件停用，确认页的过期倒计时隐藏；
眼前由进行中变已结束时一次触感 + VoiceOver 播报；自己发出的回答（页面是回执）与历史详情不变。条里的字最大到辅助功能第一档。mock 里用 `-AHOpen <id> -AHEndAfter <秒> -AHEndHow … -AHEndBy …` 触发（见 `docs/overview-ios.md`）。

自测：`AskHumanEndedUITests`（中英文各 5 条）在 iOS 27.0 全过，英文 5 条在 iOS 18.5 也全过；iOS 26.2 只用截图看过外观（与 27.0 一致）；浅色、深色、最大辅助字号在 27.0 上看过，18.5 的经典条看过滚动后的样子（为此在条后垫了 `.bar`）；Swift 包 `AHAppTests` 399 条全过。
相关回归（iOS 27.0）：请求导航 / 连接、待办挑选三组，加主类里 22 条提问页与确认页的用例，共 39 条，38 条过；唯一失败的是 `AskHumanRequestNavigationUITests.testManualReturnRestoresTheReadingPositionOfALongForm`（`:101`），就是上面「特殊回复」一节记下的 main 上已有的不稳定用例，换成不带状态条的构建它也是时过时不过，带状态条的构建用 `test-without-building` 复跑时也至少过了三次。没跑的：离开模式用例的后 11 条、`AskHumanRequestTodoUITests`、`AskHumanInputVisibilityUITests`、主类里与请求页无关的用例、iPad 两栏（`AskHumanLargeScreenUITests`）；也没跑 `./scripts/install.sh`（这次只动了 iOS App 与文档，Mac 端二进制没有变化）。

2026-11-09 追加「下一条 / 关闭」按钮与 5 秒倒计时（用户：「当一个消息在其他端完成或被取消，ios 端顶部的横幅加一个按钮，可以继续跳转下一条或关闭消息」，随后要求「5 秒倒计时，自动按这个按钮的操作」，再加一个「停留」按钮）。
需求 [`ios-answer-flow.md`](specs/ios-answer-flow.md) §3.8（E9–E15）、验收第 18–19 条；`ios-request-switching.md` §2 与 `overview-ios.md` 随之改（推翻了「请求结束后不自动跳走」）。已提交并推送 main（`aeb59922`、`78d19d49`）；2026-11-10 已随内外 TestFlight `0.1.0 (16)` 出去（源码 `78d19d49`，Internal 与 Friends 均 `IN_BETA_TESTING`，见 [Build 16 交付记录](plans/ios-channel.md#2026-11-10testflight-内部与外部构建-16)）；没跑 `./scripts/install.sh`（只动了 iOS App 与文档，Mac 端二进制没有变化）。
做了什么：条的右边一枚按钮，还有别的待答请求写「下一条消息」、没有写「关闭」（去向同提交后自动前进，`nextPending(after:)`）；设置里的「自动打开下一个请求」开着时（文案已改成覆盖两种情形，加了一行说明），条出现后数 5 秒自动按它，旁边有「停留」；弹层或键盘开着不倒计时（写回答时请求结束也算：键盘随表单变只读而收起，所以在收到「已结束」的同一轮里查）；按钮与句子并排放不下（窄屏、大字号）就排到句子下面（`EndedStripLayout`）。
自测（模拟器）：iOS 27.0 / iPhone 18 Pro Max 上 `AskHumanEndedUITests` 14 条，英文、中文各全跑一次全过；Swift 包 504 条全过（`EndedCountdownTests` 23 条）；请求导航类 8 条全过（`testArrivalHintLeavesTheRightHandControlsUsableDuringFlight` 在机器换页最凶时因等动效阶段超时失败过一次，单独重跑通过）；主类里发送与自动前进的 6 条（回执后滑入下一条、从头开始且未读、关自动前进回列表、多题发送、菜单取消、确认页选择）全过。窄屏与经典栏：iPhone SE（3 代，375 pt，iOS 18.5，临时建的模拟器，已删）上字号、倒计时、停留、滚动、关闭五条全过；暗色与三档大字号只看了截图。
没跑的：主类 `AskHumanUITests` 其余用例、离开模式用例、iPad（`AskHumanLargeScreenUITests`）、iOS 26.2；中文只在 440 pt 宽的机型上看过。
2026-11-11 用户进一步反馈没有倒计时、下一条没有发送成功时的横滑，并明确：「等我看到这个横幅的时候开始倒计时，倒计时开始之后进后台也不打断」。当前主树据此把请求结束改为等待横幅显示，真正可见后才开始 5 秒；开始后切后台不取消或重置。手动下一条与计时完成共用发送后的横滑，以旧页临时原生快照作离场内容，不显示发送回执。100 项相关 Swift 测试与 11 条相关 UI 用例通过，arm64 模拟器编译及 `./scripts/install.sh` 通过；Duo iOS 27.1 外屏与内屏均验证了两种真实前后台时序（激活系统设置并检查应用状态）；脱离 XCTest 录屏确认手动与计时横滑，网页 Markdown 的表格、代码、图片和附件在离场时保持。用户选择本次先提交、不发布；新 TestFlight 包与真机验收待后续安排。其余尚未选择的 E12 项保留：VoiceOver / 切换控制开着时仍会自动跳；手指碰屏幕不算停留；页面上有没发出的草稿但键盘已收起时照数（想留下点「停留」；结束的那页一走，草稿随页丢弃）。

下一步：
- [ ] 真机验收（按钮与倒计时，已随内测包 `0.1.0 (16)` 出去，出包时用户还没更新）：开着「自动打开下一个请求」，让 Mac 弹窗先答掉一条、手机停在那页，看 5 秒是否合适、「停留」是否好找、数完换页是否太突然、写回答时被别处答掉是否确实不跳；关掉开关后只剩按钮；有意见记进 `ios-answer-flow.md` §5。上面「没做」的四项真机上觉得需要再加
- [ ] 真机验收（状态条本身，已随内测包 `0.1.0 (11)` 出去，2026-11-07）：让 Mac 弹窗先答掉一条、手机停在那页，看状态条是否一眼能看到、正文变灰到什么程度合适（现在 0.6，想更淡 / 更不淡改 `EndedDimming.level`）、触感与播报是否合适；有意见记进 `ios-answer-flow.md` §5
- [ ] 没看过：iOS 17、iPad / 折叠屏内屏上的样子；VoiceOver 实际读出的内容（只验了无障碍标识与标签，没开 VoiceOver 听）
- [ ] 取舍待用户看过再定：条里白字在橙 / 灰上对比不高（约 2.4 / 3.4 比 1；离开模式的条也是白字压在带色玻璃上，改成青绿色后约 2.6 比 1，做法一致），若觉得难读，可把橙与灰的底色调深一档（会与历史页的颜色略不同）

## 待验收：Cursor 任务的 Max 模式控制与 Agents 窗口新建任务（已合回 main 并装为日常版；真实验证阶段 1–2 已过，阶段 3–4 等用户自己做）

2026-11-03 用户说旧「按次计费」计划里 Cursor 的 Max 模式按 API 费率扣 Credit，所以不敢用 AskHuman 新建 Cursor 任务，要求分析并支持控制 Max，
并强调**研发与调研期间严禁触发任何计费**（不运行 `cursor-agent` 任务、不驱动 Cursor 跑 Agent；测试只用桩可执行文件、临时配置与合成目录）。
经 5 轮 AskHuman 问答定下范围：需求 [`docs/specs/cursor-max-control.md`](specs/cursor-max-control.md)（M1–M8）与
[`docs/specs/cursor-desktop-task-creation.md`](specs/cursor-desktop-task-creation.md)（D1–D5），计划同名在 `docs/plans/`，各带「验证记录」。
分支 `feat/cursor-max-control`（worktree `../HumanInLoop-cursor-max`；`73527b9c` 文档、`2600561f` 功能，之后是测试与文档提交；2026-11-04 已把当时的 main
合入分支，见提交 `chore: merge main into feat/cursor-max-control`），**2026-11-04 已 fast-forward 合入 main（`7e07a728`）并装为日常版**（daemon 协议 5）；Dev Instance 已在 worktree 里验证过非计费部分。

做法：
- **Max 控制**：新 Cursor 任务默认不允许 Max；GUI 新建表单与 iPhone 各有一个「允许 Max」开关（默认关、不记忆）。关着时：启动前把 Cursor CLI 配置里持久化的
  `maxMode` 强制关成 `false`（最小文本替换，不读 `authInfo`）；将用的模型按 Cursor 本地模型目录只能在 Max 下运行则**拦截启动**并说明；判不出（缓存缺失等）照常启动并提示；
  启动后 45 秒内复查，Max 开着就发一张「知道了」卡（GUI、IM、iOS 三入口）。IM `/new` 无开关，按默认策略。设置 `agentTasks.cursorMax.{policy: enforce-off|inherit,
  plan: auto|legacy|token-based}`；新计划（按 token 计费）下整套 Max 界面隐藏、不干预；检测不出按旧计划。GUI 表单为 Cursor 新增模型选择器（带 Max 标记）；Desktop 会话信息里 Max 开着标红。
- **Agents 窗口新建任务**（实验、仅 macOS、默认关 `cursorDesktop.launchPreference: terminal`、Dev Instance 按设计不启用）：daemon 的 Desktop 消息桥在 Agents 窗口里调它自己的
  `agentRepositoryService.createAgent`，带提交前闸门——读回会话逐项核对（本地目标、项目、模式、模型、`maxMode`、权限覆盖），任一项不符就抛错、Cursor 在发模型请求之前丢弃会话；
  `dryRun` 在闸门通过后同样抛错。回执 `cursor-desktop-actions/<id>.json` 保证同一操作 ID 重试不重复创建。新增 `ClientMsg/ServerMsg::CursorDesktop`，**`PROTOCOL_VERSION` 4→5**
  （overview 的「IPC 协议号」历史与 `daemon-graceful-drain.md` §8 已记）。
- 本机只读观察（零请求）：Cursor 本地缓存 `hasTokenBasedPricing=false`（旧计划）；47 个模型里 27 个可不开 Max 用、20 个按缓存仅限 Max，**含 CLI 当前默认的 Grok 4.7 256K High Fast**；
  `cli-config.json` 的 `maxMode` 与 `model.maxMode` 都是 `false`。

自测（2026-11-03～04，都在这台机器上；没碰真实的 `~/.askhuman`，没运行过 `cursor-agent`，没驱动 Cursor 跑任何 Agent）：Rust `cargo test` 2409 通过、clippy／fmt 干净；`vue-tsc` 通过；
vitest 1111 条里 5 条在机器负载约 900 时 5 秒超时（`PopupInboxView.drop/later`，与本分支无关），放宽超时重跑这两个文件 98/98 通过；Swift 包 569 通过（43 条要真实服务的端到端按设计跳过）；
Desktop 适配器脚本测试 19/19；全量 node 测试里两条安装脚本测试在重负载下 `spawnSync bash` 30 秒超时（本分支没动 `scripts/install*`）。iOS 模拟器 UI 测试见两份计划的「验证记录」。
一次性 Chromium 加假 Tauri 桥看过新建表单的 Cursor 行（浅色／深色、中英文，未提交）。
2026-11-04 合入 main 后复测：`cargo test` 2448 通过、clippy／fmt 过钩子、`vue-tsc` 通过、vitest 1115 条全过（超时放宽到 120 秒）、node 114 通过／19 跳过／0 失败、
Swift 569 通过（43 条按设计跳过）、两条 Max 的 iOS UI 测试英文通过（中文没重跑，核对了字符串目录）。main 之后又进了两个提交，
第二次合入（无冲突）后重跑了 fmt／clippy／`cargo test`（2449 通过），其余沿用上面的结果。

下一步：
- [ ] **真实验证（2026-11-04 用户裁定）**：阶段 1–2（日常版只读检查、Desktop `dryRun`，按设计 0 请求）由 Agent 执行；阶段 3、4（各 1 个普通非 Max 请求）
  由**用户**按 [计划](plans/cursor-desktop-task-creation.md)「真实验证计划」里的「阶段 3、4：用户自行执行的步骤」自己做。Agent 不执行阶段 3、4，除阶段 1–2 外不起 Cursor 任务。
  - ✅ 阶段 1（2026-11-04，日常版 `7e07a728`，0 请求）：设置页检测为「旧计划」；新建表单里默认模型被拦、「允许 Max」解除拦截、换小模型解除拦截，
    24 个模型的 Max 标记与 Cursor 缓存逐个一致；没有启动任何任务。细节见计划「验证记录」。
  - ✅ 阶段 2（2026-11-04，日常版 `7e07a728`，`gpt-5.4-nano`、不允许 Max，0 请求）：第一次 `dryRun` 因 Agents 窗口不认识验证项目 `~/Developer/AskHumanCursorVerify`
    （空 git 仓库）被拒（`rejected`／`projectNotFound`）；Computer Use 不能操作 Cursor，由用户自己在 Agents 窗口里把项目加进去后，第二次（新 ID）约 1 秒返回 `dryRun`、
    `tidied: archived`，观察到模型 `gpt-5.4-nano`、`maxMode:false`、无权限覆盖；回执 `dryRun`；状态库只多一条空聊天（无消息、无用量），验证项目没有新 worktree 或分支。
    **没有直接看到 Agents 窗口的界面**：请用户做阶段 3 时顺便看一眼侧栏里没有「dry run」会话（状态库里查不到归档标志位）。
  - 阶段 3、4（用户做）：命令已按验证项目 `~/Developer/AskHumanCursorVerify` 与模型 `gpt-5.4-nano` 填好，在计划的「阶段 3、4：用户自行执行的步骤」里；等用户把现象告诉 Agent。
- [ ] iOS：内测包 `0.1.0 (6)` 已出（2026-11-04，`VALID`、已在 `AskHuman Internal`，含「允许 Max」开关与 Max 标记）。**它是 main 的整包**，同时带上了新版项目选择（用户此前选过「先不出」）
  与「去掉 Agents 页需要关注分组」，详见 `docs/plans/ios-channel.md`。等用户在手机上更新后看「允许 Max」开关与模型的 Max 标记，中英文各一次。
- [ ] 真实验证通过后，用户在设置里把 `cursorDesktop.launchPreference` 改为 `desktop`（D5）。
- [ ] 已知局限（规格 §3、§4.4）：缓存只是建议性证据，可能比服务端目录旧；CLI 对「默认模型本身需要 Max」只在内存里推导，改配置关不掉，只靠创建时的预判拦截。
  上面两条观察合起来的含义：当前 CLI 默认模型 Grok 4.7 按缓存是「仅 Max」，而配置里 `maxMode:false`，不运行 Cursor 无法判断哪个对。**用户可以在 Cursor 网页用量里看一眼过去 Grok 4.7 的请求有没有按 Max／API 费率计**。
  新任务不选模型（用 Cursor 默认）又不开「允许 Max」时，现在会被拦截，要换一个不需要 Max 的模型或打开开关。

## 进行中：iOS 新建任务时选模型与强度——Mac 端与 iOS App（P1–P3）已完成，下一步 P4（文档回写与真机一轮）

2026-10-27 用户提出「iOS 版创建任务时可以选择模型」，经 6 题问答定下范围：模型 + 强度一起做；清单以动态发现为主（Claude 用固定别名）；每次都从「默认」开始、不记忆；
详情页显示实际模型、进度页显示所选；先只做 iOS，启动管线放共享层。**2026-10-30 用户修订 M3：模型与强度作为手机偏好按 Agent 记住，所有 Mac 共用，点击开始时保存（失败也保留），取消不更新；缺模型临时回默认且不抹掉原偏好。** 用户同时强调**严禁触发计费**，并且**Cursor 严禁实测**。同日做了只读探测（没有起任何 Agent、没有发提示词、
没有带真实凭据调模型），写成需求 [`docs/specs/ios-task-model.md`](specs/ios-task-model.md) 与计划 [`docs/plans/ios-task-model.md`](plans/ios-task-model.md)，用户评审批准，
并对六条待确认点作了选择（Grok 用 `grok models`、Codex 一律走 `app-server` 的 `model/list`、Pi 三档强度、做 M10、实现完做一轮真实验证但每次要批准）。

已完成：
- P1（Mac 启动管线：`ModelChoice` 贯穿 `TaskSpec` → `LaunchRecord` → argv 与 Codex 桌面应用；没选时对外行为与以前逐字节相同）。
- P2（Mac 清单 + 线协议 + 网关 + 详情页实际模型，已装机）：`task.options` 多了可选的 `models` / `defaultModel`，`task.create` 多了可选的 `model` / `effort`，
  `watch.frame` 多了可选的 `model` / `reasoning`；旧版 App 连新 Mac 与以前完全一样。实施记录与偏差见计划「P2 实施记录」。
- P3（iOS App，在 `/tmp` 隔离副本里做、对着新 HEAD 重新验证后合入）：新建任务表单在 Agent 与权限之间多了「模型」行（进选择页，超过 12 项有搜索）与「强度」行（菜单），
  按 2026-10-30 修订恢复这部手机为该 Agent 保存的模型与强度，Mac 没给 `models`（旧 Mac / 没拿到清单）时两行都不出现且保留偏好；进度页有一行所选，Agent 详情页有一行它实际用的模型 / 强度；中文文案齐全；共享夹具有了新用例。
  实施记录、验证结果与已知局限见计划「P3 实施记录」。**2026-10-30 用户已通过 TestFlight 安装 `0.1.0 (2)`，确认原有 Mac 连接正常**；真实 Agent 矩阵仍待验证。

下一步：
- [ ] P4：文档回写（`docs/specs/ios-channel.md` §6.8 与「不能选模型」的局限、`docs/overview-ios.md` 的线消息 / 模块 / 启动参数 `-AHNoModels` `-AHFailTask`、`docs/wiki/ios-setup.md` 与 `.en.md`、
  `docs/overview.md` 的 `agents/` 清单行）；Mac 与手机均已装机，后续分发复用项目 skill `.agents/skills/ios-testflight/SKILL.md`。
- [ ] （实现完做，**每次执行前**经用户批准，会产生模型费用；永远不含 Cursor）真机一轮：见计划 P4。同时验证两件我在本机看不到的事：已登录账号下 `grok models` 的输出
  是否带逐模型档位（带了才补 Grok 的强度行与 M10）、带真实凭据的 `codex app-server` 的 `model/list`。
- [ ] `ios_tasks::create()` 里「读就绪 → 取清单来源 → `resolve_pick` → `TaskSpec.choice`」这段胶水没有单测（`create()` 会真的起任务）；各部件各有测试，整条由上面那一轮覆盖。
- [ ] （可选，等用户决定）强度行的「默认（中）」提示：手机上取的是所选模型自己的默认档位；但 Mac 配置里固定了强度、且该模型支持它时（目前只会出现在 Codex），Agent 实际用的是配置的档位（M10），
  两者可能不同。要精确得让 Mac 在 `task.options` 里按模型各算一个「默认时实际用的档位」。本期按需求原样实现。

## 待观察：拖入文件失败（`daemon.log` 的 `askhuman_drop` 行）

2026-10-17 用户在日常版（v0.14.5）遇到几次「拖图片进窗口没加附件」（来源访达的下载 / 桌面 / 文稿，第二次拖就可以）。已加每种情形的可见提示和不含路径的诊断日志，
决策与读日志的办法见 `docs/specs/popup-inbox-collapse.md` §6.9。
2026-10-22 查到并修复了主因：拖拽开始时 AskHuman 不是前台，wry 在 `draggingEntered:` 里读文件被系统拒绝（`permission_denied`）；macOS 上现在带文件的拖拽进入窗口时先取前台、
拖走不松手再还回（`popup_drag_front.rs` + `swift/PopupDragFront.swift`，根因与验证见 spec §6.10）。Dev Instance 里用自动化的 Dock 堆栈拖拽验证过；同日日常版覆盖安装含修复的构建后，用户亲自把 Dock「下载」堆栈里的图片拖进弹窗成功，`askhuman_drop` 记 `added`。

真实窗口验证（2026-10-17，隔离安装，用户随手拖了 4 次「下载」里的文件）：落到普通请求正文 / 多问题卡片各一次 → `added`，落到「只能选择」的请求两次 →
`not_accepting` 并确认看到橙边提示「…不接收附件」；日志的位置类别、落点、请求编号都对得上，没有读取失败。用户决定不再补测，下面这些**没在真窗口里看过**，只有自动测试覆盖：
桌面文件与系统授权弹窗 / 拒绝后的提示、网页里的图片（应提示改按粘贴）、拖到附件预览区、读取失败 / 部分成功 / 全部重复的提示、成功提示条「已添加 N 个文件到「…」」的外观
（用户没答）、独立弹窗标题栏下的一行。等日常版里真遇到时再对号。

待做：
- 日常版已装含修复的构建，继续观察还有没有「拖不进去」，有就先 `rg askhuman_drop ~/.askhuman/daemon.log` 对号入座（这时再出现 `permission_denied` 才是真的权限问题）。
- 修复没在真机上验证过的部分：Finder 窗口 / 桌面作为拖拽来源（只测了 Dock「下载」堆栈）、其他 macOS 版本。钩子靠 wry 的类名与方法名，升级 wry 后要确认它还装得上
  （`daemon.log` 里出现 `askhuman_drag_front` 行、`installed` 为 false 就是没装上）。
- **日志出来再定**（目前都不做）：图片读取改成异步（现在是一次同步整文件读取，大图 / 网络盘会卡界面）；预览区接收文件（现在落上去只提示、不收）；拖入过程中对预览区显示「不可放」的视觉。

## 待观察：Cursor 会话防护（运行中会话保护；已实现并装为日常版，「失败后不再被归档」只能等下一次自然失败来验证）

2026-11-07 起，Agents 窗口里生成中的会话不再被卸载消息（同一个开关 `cursorDesktop.suppressBackgroundCompletionFollowUp` 下的第二个独立守卫；
规格 `docs/specs/cursor-desktop-background-followup.md` §7，实现与验证记录见 §7.12）。动机：10-25 起本机至少 8 次，长时间运行的首轮最终失败后，会话从侧栏消失、
首条消息被删除并被归档。接入、计数、开关关→开循环在真实 Cursor 3.27.22 里都验过；**不调用模型就无法验证「失败后不再被归档」**，所以靠下面这些观察。
2026-11-11 Cursor 升到 3.29.15 后复核，本项一直保持生效；另外两项守卫的升级兼容问题已修复并安装，三项均报告 `active`，见规格 §8.4。自然失败后的结果仍待观察。

待做：
- 下一次自然出现首轮最终失败时核对：会话是否还在侧栏、首条消息是否还在；同一回合期间 `AskHuman doctor --json` 的 `cursorDesktop.windows[].retention.skipped` 与 `recent` 有没有内容。
- **证伪条件**：守卫生效期间开始的回合（开始时间晚于激活，期间窗口没有重载）失败后仍出现「首条用户消息被删除 + 会话被归档」→ 规格 §7.1 的推断不成立；
  撤回对本守卫的承诺，另行评审（候选 S3 放宽尾部、S5 事后取消归档），不继续堆补丁。
- `recent` 里若出现范围 `all`（整会话被卸载），说明隐藏会话驱逐（B 路径）在生成期间也在发生，与源码阅读矛盾，要重新审视。目前只见到范围 `partial`（A 路径，单次最多 922 条气泡）。
- 观察生成中的可见大会话的内存与滚动手感；异常时关开关即还原。
- 真实环境里没验过的：IDE 窗口（应为「不适用」）、窗口重载后重新接入（只有假窗口测试）。Cursor 升级后看 `AskHuman doctor` 的 `running-chat-protection` 一行：`unavailable` 表示结构检查没过
  （设置页显示部分生效，安全）；有生成中会话时 `unloads skipped` 长期为 0，则可能是 Cursor 换了卸载入口（静默无效），按证伪处理。
- 与「待办：Cursor 等待期保活」互相独立：保活是减少失败，本项是失败后别被误归档。
- 以上都有结论（确认生效，或证伪并处理）后删除本节。

## 待自然样本：Cursor Desktop 停滞等待

5 分钟等待与 10 分钟连续停滞窗口已提交 `753497b7` 并安装。Cursor 3.27.22 两种窗口与 Agent Host 的断网原生测试通过。11 月 8 日 07:29，已安装的 `AskHuman doctor --json` 报告日常 Agents 窗口两项计时成对生效。
2026-11-11 已移除版本白名单及固定压缩符号／模块编号，改为识别运行接口并做隔离行为预检；修复版安装后，在 Cursor 3.29.15 日常 Agents 窗口重新确认 300 秒／600 秒成对生效（适配器修订 `f797c0ed2960564d`，规格 §8.4）。本次验证未发起模型请求。

下一次自然停滞时，对同一请求 ID 核对是否跨过原 90 秒阈值后继续收到业务消息，以及真正持续无入站消息时何时重试。见 [会话防护规格 §8](specs/cursor-desktop-background-followup.md#8-延长无入站消息的等待与重试窗口)。

## 待外部验收：统一作答窗口的真实渠道与其他平台

macOS 合并/可选独立模式、原生画布过渡、普通外缘缩放修复、10pt 未查看蓝点和持续圆形扩散已接入并安装。
行为规格见 `docs/specs/popup-request-inbox.md`；实施与验证记录见
`docs/plans/popup-request-inbox.md` §10–24。最近 Rust 1256 passed / 3 ignored、
最新前端 252 项通过；原生几何 harness 84 项通过，真实 Sidebar 拖动及宽度恢复已验证。
自动化外缘拖动未成功改变窗口尺寸，
程序化几何证据不代替物理拖动观感验收。

2026-10-08 已在 Windows 11 24H2 VM 的交互式桌面验证合并窗口核心流程、到达气泡、草稿与选区、
最小化恢复、Sidebar 拖动、附件三栏和混合类型取消，并复现修复 #15 的空白窗口残留；见计划 §27。
用户已同意延期的真实 IM 终态、Linux 实机、DPI/多屏矩阵仍待补齐（AskHuman 项目
todo #4：a7d02da0-72d5-4191-954d-d0bcb77d322e）。上述 Windows popup-only VM 证据
不能代替这些外部 gate；补齐后删除本节。

2026-10-09 macOS 又完成气泡两次弹跳 / 750ms 勾线飞行和正文本地 Markdown 图片的安装与实窗
验收，见 `docs/plans/popup-arrival-notice.md` 和 `docs/plans/popup-markdown-images.md`。
2026-10-10 正文本地图片又改为有界原文件 URL / 懒加载 / 超限回退，并经安装和用户实窗验收。
其他平台补验应使用最新动效参数，覆盖正文图片的本地 / 项目相对路径、专用协议 URL、
超限 / 解码失败提示和点击打开原图。Windows / Linux 已按用户要求核对官方资料及依赖源码；
本轮没有对应实机证据，详见 `docs/investigations/base64-usage-audit.md`。
后续平台补验还应覆盖历史、待办草稿与 Interject 的纯显示图片 URL、移除 / 切换 / 卸载清理、
超限文件胶囊与提交原路径；本机自动验证记录见 `docs/plans/display-image-urls.md`。
右侧附件大图也应补验原文件 URL、特殊格式临时 PNG、缓存淘汰 / 过期读取释放和 GIF / WebP 动画；
历史继续保存原路径，记录见 `docs/plans/attachment-image-urls.md`。

## 待外部验收：Windows / Linux Popup 附件预览面板

实现、macOS 安装及用户体验验收已完成，自动回归和真实 WKWebView / AppKit 验证记录见
`docs/plans/popup-attachment-preview-panel.md` §9；行为规格见
`docs/specs/popup-attachment-preview-panel.md`。已验证固定右侧、分隔线、外缘缩窄后的主区恢复、
最大化后恢复、阅读状态、图片动画 / 平移、原文件打开及多题草稿提交。展开总尺寸未污染主区偏好。

2026-10-08 已补齐 Windows 11 VM 的安装与 Markdown 三栏展开、到达动效排除预览区、Sidebar
分隔线和草稿保留，见 `docs/plans/popup-request-inbox.md` §27；未据此验收全部附件格式。
剩余为 Windows 完整附件 / 浏览器 / 拖放矩阵、Linux 实机安装、DPI / 多屏、X11 / Wayland：保留已确认的外部
gate（AskHuman 项目 todo #3），完成后删除本节。Markdown 浏览器打开也应在这两平台补验系统默认浏览器、HTML 文件关联不同、启动失败和真实菜单；本地记录见 `docs/plans/markdown-browser-open.md`。不得以 macOS 或纯计算测试代替这些平台实机证据。

2026-10-09 分区搜索已实现并通过 macOS 安装/交互验收，见 `docs/specs/popup-find-scopes.md` §9。
这两平台补验附件列表打开后 Ctrl+F 的区域路由、文本/Markdown/长 diff 的计数与定位、
窄区布局及输入法；PDF 沿用明确的不支持提示。当前自动测试不能代替这些平台实机证据。

2026-10-22 历史详情接入同一块面板（规格 §10，实施记录见 `docs/plans/history-attachment-preview.md` §6）。
这两平台还需补验历史窗口：向右展开与窗口内分栏回落（含条目列表折叠）、关闭恢复、用户移动后保持、
DPI / 多屏、X11 / Wayland 的定位限制，以及右键 / 更多菜单（`popup_preview_actions.rs` 现按所有者 ID 找窗口；
本机没有非 macOS 编译目标，这部分只由 CI 编译）。同样不得以 macOS 证据代替。

## 待外部验收：Windows / Linux 合并窗口的稍后回答与小窗

macOS 的「稍后回答」标记、小窗「待答 N」、拖入文件的停留展开 / 切换、切走缩回已实现，并由用户在真实窗口验收；需求决策见
`docs/specs/popup-inbox-collapse.md`，实施与验证记录见 `docs/plans/popup-inbox-collapse.md` §8。Windows / Linux 的代码路径
（窗口即时变形 `popup_canvas::instant`、以窗口 `Focused` 事件作「用户在 AskHuman」信号、CSS grid 布局、拖放落点单位候选）已写好，
在 macOS 上通过类型检查和自动测试，并在 headless Chromium 里量过：单请求的列表不再挤占正文的网格列。但**没有在任何一台 Windows / Linux
机器上运行过**；用户已同意延期（计划 §1、§3 P7，AskHuman 项目 todo #1）。实机补验清单：

- 窗口变形：缩小 / 展开 / 小窗里来新请求时，小窗内容区是否恰好是页面量出的宽高（Windows：`set_resizable(false)` 去掉粗边框后客户区有无变化；
  GTK：先改尺寸再锁定的顺序，窗管会不会把尺寸钉住或还原）；展开后位置是否仍以小窗左上角为锚、最小尺寸是否恢复；拖动小窗后再展开的位置。
  Wayland 合成器不接受应用设定的位置（代码忽略被拒的位置），看窗口落在哪里是否可接受。
- DPI 与多屏：125% / 150% / 200% 缩放；在缩放不同的显示器之间拖动小窗再展开，尺寸有无取整偏差。
- 页面布局（WebView2 / WebKitGTK）：小窗的列表宽度、3 行高度与滚动，「待答 N」标题和按钮；这两个平台没有红绿灯，顶部留白与展开的 Sidebar 相同；
  系统标题栏下的小窗是否被截断或难看；单请求缩小 / 展开的瞬间正文不闪、不被挤。
- 切走缩回（窗口 `Focused` 事件）：peek 时切到别的应用 → 200ms 后缩回，回来则取消；系统文件选择框 / 确认层打开时是否被误判为离开
  （Windows 的文件对话框会让 popup 失去焦点）；弹窗不抢焦点地弹出时起始状态是否为「不在场」。
- 拖入文件：Windows（物理像素）与 GTK（CSS 像素）的落点单位是否都选对（`dropPoints` 先试一种、失败再退另一种）；停留展开 / 切换 / 边缘滚动的手感。
- 自家附件拖给另一个请求（R17，spec §6.11）：窗口自己发起的拖拽在 WebView2（OLE）和 WebKitGTK 里是否也把 `enter` / `over` / `drop` 送回本窗口（macOS 上会，旧日志里有 `own_drag_out`）；
  送到的话，别的请求行的亮起、停留切换、松手落点、来源请求不收、来源正文不自滚是否和 macOS 一致。事件不送回本窗口的话，这条在该平台上等于没有，要另想办法（例如拖拽开始时就让页面进入拖拽态）。

不得以 macOS 或纯计算测试代替以上实机证据；完成后删除本节。

## 待外部验收：Windows 发布候选

功能与架构实现已在 `codex/windows-platform-parity` 完成；设计、实施记录和 Win11 证据见
`docs/specs/windows-platform-parity.md`、`docs/plans/windows-platform-parity.md` §17.9 与
`docs/plans/windows-unsigned-update-policy.md` §12。当前 Win11 24H2 VM 已通过 PS5/PS7 install、
named-pipe daemon、完整 Rust tests（1127 passed / 2 ignored）、Windows update 专属测试（19/19）、
Clippy、165 Vitest + 5 Node tests、production/release build、真实 authenticated Codex 0.147 E2E、
卸载维护链和未签名 binary fail-closed / `update prepare` 文件锁闭环。交互式桌面已覆盖统一图标、Ctrl
快捷键、Advanced、设置稳定性、真实飞书取消、Windows Terminal 精确 focus、Dev Instance 以及无闪窗
login/logout。外部平台逻辑 Review 的 correctness 项也已收口：daemon metadata watcher、共享 Windows
path identity、launchId 平台标记和 rename 覆盖语义均有 Windows 原生回归证据。

发布认证仍依赖仓库外状态：

- 准备干净 Windows 10 22H2 x64 VM 并复跑核心矩阵；
- 生产 Authenticode、timestamp、publisher identity pinning 与 SmartScreen 认证已决定暂缓，后续作为
  独立签名项目恢复；当前 unsigned Windows 已固定关闭自动 apply，并提供 `update prepare` 手动闭环；
- 补齐真实交互式 Windows 桌面的 DPI/多屏/输入法/文件选择/声音完整矩阵；
- 如发布认证要求覆盖每个 IM provider，以真实凭据补跑飞书以外渠道；当前飞书真实取消链路已通过，
  其余渠道为 deterministic mock 覆盖。

以上 gate 完成后删除本节。Windows ARM64、原生 installer、Windows Server/RDS 多会话是已确认后置
项目，不属于当前 release candidate blocker。

## 待验收：本地 Markdown Mermaid 图表的跨平台实机运行

完整实现、自动测试、本地浏览器 sandbox / 布局验证与 bundle spike 已完成，详见
`docs/plans/mermaid-rendering.md` 的实施记录。仍需在 Catalina 级 WKWebView、Windows WebView2 与
Linux WebKitGTK 分别跑一次计划 §8.3 的图型、错误、主题、Find 和回答流程矩阵；当前 macOS Tauri
Popup 已完成本机验收。2026-10-09 又补齐 Markdown 附件预览与离线浏览器快照，用户确认两入口与
深色背景正常；实现、Safari Find / 打印和背景修复记录见 `docs/plans/attachment-mermaid-rendering.md`。
跨平台补验应包含新增附件入口。实机 gate 未齐前不降低安全等级或提高系统要求。

## 定期同步：Codex Shell 判定复刻（codex-permission-remember §6.4）

权限记忆功能复刻了 Codex 的 Shell 判定逻辑（`src-tauri/src/shell_safety.rs` +
`permission_shell.rs`）。版本门控只设下限（`VERIFIED_CODEX_VERSION_FLOOR` = **0.122**，
hook 引入版）；`VERIFIED_CODEX_VERSION_CEILING`（当前 **0.146**，对拍来源 Codex upstream
main `1a817bb95d`，2026-07-24；同批吸收 0.145.0 的 #34271 禁选前缀扩容与 #32232
hook-before-guardian 语义，见 spec D50-D53）是最近一次逐行对拍的版本，用户装机超出它时
功能**保持启用**、worker stderr 记一条日志。因此同步不再是紧急事项，但仍需**定期**
（Codex 新 minor 发布后）对拍以下上游文件并抬升已审计版本（相对 codex-rs/）：

- `shell-command/src/bash.rs`（`bash -lc` 脚本拆分）
- `shell-command/src/command_safety/is_safe_command.rs`、`is_dangerous_command.rs`（heuristics）
- `core/src/exec_policy.rs`（fallback 判定 / amendment 派生 / `BANNED_PREFIX_SUGGESTIONS`）
- `config/src/loader/`（配置层叠与项目信任，影响 rules 文件发现与 managed 检测）
- `codex execpolicy check` 的 CLI 契约（参数与 JSON 输出；有 ignored 集成测试
  `permission_shell::tests::real_codex_cli_contract_when_available` 可拿真机验证）

无差异则只改常量 + 记录新 commit；有差异先改 port 再抬已审计版本。若上游出现我方未携带的
**放宽**类变更（新增 safe 命令等），只影响覆盖率；出现语义级破坏（拆分格式、hook 契约）时
fail-closed 机制会自动降级为基础弹窗，届时按 D35 修订的证据链重新评估。

## 待办：Cursor 全局 Rules 迁移为用户级 always-on Skill

调查与候选设计见 `docs/investigations/cursor-global-rule-user-skill.md`。无 workspace folder 的 Cursor IDE
不创建项目 Rules 加载器，因此不会读取 `~/.cursor/rules/askhuman.mdc`。未来改为用户级
`~/.cursor/skills/askhuman/SKILL.md`，旧安装显示“需更新”，迁移时先写新 Skill、再清理旧托管 MDC。
Grok 默认会扫描 Cursor Skills，候选 frontmatter 已设计为对 Cursor 常驻、对 Grok 不可调用。

## 待办：Cursor 等待期保活（避免“无进展”规则中止长等待，待讨论）

调查与候选设计见 `docs/investigations/cursor-idle-wait-keepalive.md`。AskHuman 长时间阻塞时回合内没有新
checkpoint；网络中途断开后，Cursor 连续两次“流已建立却无 checkpoint”的自动续跑会被它自己中止
（`Agent stopped retrying …`），实测 5 个长回合里 4 个因此终止。候选：① 规则层改为后台 AskHuman + `AwaitShell`
分段等待；② 程序层让 AskHuman 每 N 分钟返回“仍在等待，请重跑”，daemon 保持问题挂起。2026-10-26 用户决定先记录、
后续讨论；尚未修改任何规则 / 代码，也未做需计费的真实验证（需先经用户批准）。
2026-11-03：同一类中止在本机又出现 3 次（05:47／08:59／12:53；Cursor 结构化日志 `agent.turn.outcome` 记为 error，
重试 6／4／0 次）。保活是预防；出错后的入口（日志检测加错误卡）是兜底，见 [specs/agent-failure-inbox.md](specs/agent-failure-inbox.md)，两条线互不依赖。
同类失败后会话被 Cursor 误归档的防护（运行中会话保护）已另行实现，见「待观察：Cursor 会话防护」，与保活也互不依赖。

下一步：与用户确认方案取向和周期 N；`transportCap=10` 仍是硬上限，要延长还需降低断线频率；方案 ② 可与
“daemon 即时换新与在途提问恢复”共用请求身份 / 重连基础设施。已登记 AskHuman 项目 todo #8。

2026-10-29 用 5 天 / 171.5 流小时的样本重算（调查文档 §2.3b）：没有固定
寿命墙；孤立失败恒定约 0.95 次/流小时、与时段和日期无关（占 81%）；约 19% 的失败成簇，`read ETIMEDOUT` 多在簇内；
HTTP 兼容模式下到 api2 的新建连接较忙时约 2.55 条/秒（`lsof` 被动采样）；Cursor 自己的 `bidiAppend` 在 10-27、10-29 的
09:08–10:03 出现 7 段“连本机网关都握不上手”的 `connect ETIMEDOUT`（Mac ↔ 网关一段，根因未知）。早先“失败率随年龄
上升 / 不同流不成簇”的结论已修正。

10-30 合成流的结果、11-07 宽带与单流排查的证据、以及联通局端和代理逻辑关闭的未决项，统一见
[Cursor 断线排查与处理](investigations/cursor-disconnect-2026-11-07.md)。探针与分析脚本在仓库外。

- **探针收尾**（用户已授权 SSH，仅限这几样）：探针所在的那台海外 VPS 上停掉探针服务端进程（带 `--max-hours 16`，
  约 16 小时后自行退出），删除 `/root/probe_server.py`、`/root/probe_out/`、`/root/probe_server.stdout`，其它一律不碰；
  用户自己删掉 Surge 里为它加的那条 `IP-CIDR,<VPS>/32,Cursor,no-resolve`；用完删 `/tmp/chain_probe`（含 token）。
- **上午 09:08–10:03 的网关握手黑障**：探针附带的 ICMP 哨兵（每秒 ping 网关虚拟机与其宿主）会在 10-30 上午区分
  局域网 / 宿主 / Surge 进程三类；要查根因还需要 10-27、10-29 同时段网关与宿主的负载和 Surge 日志（只读 SSH，需用户
  再次明确授权）。

计费口径待核对：Dashboard 事件对应“带模型调用的请求流”，10-19 有 2 次只有心跳的续跑没有事件，所以“空等中的续跑
不计费”是个待验证的假设；若成立，保活会把免费的空等续跑变成计费请求（调查文档 §2.4）。零成本验证：重新导出
Dashboard CSV（覆盖 10-26 00:47 之后）再对账。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询**缓存的安装入口指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希；Linux 保留旧 ELF 时不检查旧映像）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。

## 待验收：daemon 即时换新与在途提问恢复

2026-11-08 用户确认旧题保留、新题正常接收且后台及时更新，接受短暂重连；桌面优先保留原窗口和
输入位置，窗口程序可稍后更新；iOS 与 IM 都需覆盖，IM 原卡继续可答，已选内容尽量保留并避免重复推送。
分析与分阶段实施计划见 [即时换新计划](plans/daemon-live-update.md)，关联既有 AskHuman 项目待办
`416b9760-cf6e-4cf1-bed1-22a04fab1df5`。用户随后要求开始实现，并选择在主工作树开发。
2026-11-09 已完成本轮代码并运行 `./scripts/install.sh` 安装，正式后台已接管；剩余渠道和平台验收见下。
实现涵盖 SQLite 请求/终态/渠道检查点、稳定 CLI 重连、四个 IM 普通题与确认卡接管、
桌面宿主保留表单/焦点顺序并续送未确认提交、iOS 原题快照屏障/通知事实/上传恢复/答案回执同事务、
daemon 自持错误卡与 Codex 原生消费、Telegram offset，以及自动交接/可恢复失败回退。
实现契约见 [即时换新规格](specs/daemon-live-update.md)。用户要求简洁界面，恢复暂时失败时保留原题和 Agent 等待。

Rust 全量 2911 项通过（11 项既有忽略），Vue 1797 项通过，Node 203 项通过（21 项平台跳过），
前端类型检查通过，Swift 核心与协议 216 项通过，iOS App 双架构模拟器编译成功。
真实私有 daemon + loopback Telegram 已通过 A 挂起时换代/B 到达/原卡选项、确认卡、旧版调用旁继续接新题、
stop 优先、禁用重启、实际 exec 失败恢复与坏替换修复，共 8 个场景。
最新一次测量从安装检测到新后台可见 12816ms（含 15s 轮询），B 卡到达 104ms。
合并和独立原生窗口均实际保留同一 Host/Helper 进程、草稿和中间光标，独立窗口连续两次换代后正常提交，
并已验证完成后清除旧更新横条。验收临时进程已停止，日志在 `/tmp/askhuman-live-*`。

首次旧版排空已结束。2026-11-09 08:13 用户报告新提问与手机同时不可用：早期 schema 1
数据库缺少 `cancelled_calls`、`calls.daemon_owned`、`calls.legacy`，后续迁移只推进版本号，
导致新 daemon 一直留在恢复阶段。本轮开发修改了已执行的初始建表内容却漏增迁移，是本轮引入的故障。
08:16 已以 SQLite backup 私有备份后在事务内补齐结构，未取消原调用；积压 4 个调用随后受理，
用户通过正式 AskHuman 确认桌面和 iOS 均已恢复。永久修复为 schema 5 独立事务迁移，
并以冻结 SQL 夹具覆盖 schema 1–5 的升级、原题/终态/渠道检查点保留及取消操作；恢复模块 19 项通过。
已重新运行 `./scripts/install.sh` 安装，正式后台数据库已升至 schema 5，自动交接保留了原待答请求，
四个 IM 连接正常，旧恢复错误没有重现。故障前真实数据库的隔离副本也由新程序自动迁移至 5，
真实私有 daemon 随后成功受理新稳定调用；日志在 `/tmp/askhuman-recovery-schema5-*`。
`AGENTS.md` 已补充必须增加版本和新事务迁移、保留冻结夹具并验证已有数据升级的规则。
用户要求提交推送并 review 全部改动，功能提交 `f4318959` 已推送 main。复查发现并修复：
iOS 更新状态提前解除导致附件不续传、IM 已答内容遇暂时写入失败后不再提交、exec 失败后误读新磁盘指纹。
前两类分别以四种在线/epoch 恢复组合和真实 SQLite 写失败回归，exec 问题已用真实私有后台复现。
窗口交接另增加写出屏障、每轮通知身份、恢复状态重置和旧空闲宿主的协议保护，交接前重新保存宿主租约。
窗口专项 30 项通过（1 项既有忽略）；Windows/Linux CI 的 Clippy 错误已定位修正。最终源码 `a7d15e24` 的七项 CI 全部通过，已公开发布 [补丁 v0.17.1](https://github.com/Naituw/AskHuman/releases/tag/v0.17.1)（npm 主包与四平台包均为 latest）。
iOS 新提示已随内外 TestFlight `0.1.0 (15)` 发布，用户已更新并确认原有 Mac 连接正常。
真实手机更新中留题/附件续传/推送、四个 IM 原卡交接与 Windows/Linux 原生界面验证仍需完成；
源码与分发证据见 [Build 15 记录](plans/ios-channel.md#2026-11-09testflight-内部与外部构建-15)。
Windows 安装 worker 已按恢复能力选择保留旧服务或旧版安全排空，替换后由现有后台交接，尚未经 Windows 实机。
此前 Windows 范围询问收到离开模式指导，后续探索得到上述小范围接法并继续原目标，未视为用户同意延期。
本机已选择后台构建并持续报告，禁止为了加快编译而关闭用户其他任务。

## 待办：多份安装互顶刹车的收尾（Windows 与配置扫描）

2026-10-22 落地了 Daemon 的 hold（同一对二进制 10 分钟内互顶 ≥2 次后不再让位）、换新原因记录和
`AskHuman doctor install`，决策与验收见 `docs/specs/daemon-graceful-drain.md` D10–D12、§6。macOS 上做过
隔离 e2e 与真机 PATH 扫描。下面两项用户决定之后再做（AskHuman 项目 todo #2、#3）：

- `scripts/install-windows.ps1` 装完后调用 `AskHuman doctor install --warn-only`。Rust 侧已跨平台编译，
  但 Windows 上的 PATH 扫描（只查 `AskHuman.exe`，不解析 npm 的 `.cmd` shim；`canonicalize` 会带
  `\\?\` 前缀）与 `--version` 探测没在 Windows 实机验过。
- 扫描各 Agent 的 hooks / MCP 配置里写死的 AskHuman 路径，与 Daemon 实际运行的二进制对比。
  现有补救是装完用 `AskHuman agents update` 重写。

## 待办：Linux 运行中切换字体 DPI 的比例与窗口几何

2026-10-30 复查 PR #19 时发现：`app/linux_dpi.rs` 仅在初始化时缓存页面比例，运行中修改字体 DPI 后，
WebKit 可刷新页面缩放，但 Popup / 历史窗口的几何仍沿用旧值。用户决定单独设计并在 Linux 实机验证，
不纳入本轮六项 PR 回归修复；已登记 AskHuman 待办 `94898352-1092-46e4-93ac-cd5510b9568f`。

下一步：按 WebView 观测有效页面比例（保留初始化估算），处理 GTK DPI / 原生 scale 变化和过期回调；
复用现有几何队列完成合并窗口、独立弹窗与历史预览的瞬态重排，避免把比例切换存成用户尺寸。
覆盖折叠 / 展开、预览开关、约束 / 最大化、在途几何事务和多屏；自动测试与 Linux 原生运行都通过后删除本节。


## 待真机核验：多设备报文流量统计

2026-11-12 用户批准的 [流量统计修正](specs/ios-traffic.md) 已在主工作树实现并运行
`./scripts/install.sh` 安装。本机正式后台已使用新状态字段，原有累计量保留。
各 Mac 与手机分别计量实收与 PUBACK 确认的发送，端点累计量去重求和，手机原子保存自身计数与
副本集合，旧历史和未确认发送标为不完整。两台 Mac 核实时合计约 489.85 MiB，原乘 2 公式
得到 0.957 GiB，高于 EMQX 当时显示的 0.77 GiB；现已删除固定乘 2 的口径。

Rust iOS 专项 331 项、Vue 专项 26 项、Swift 全量 894 项通过（另 64 项环境测试默认跳过），
真实双 daemon + 手机 + 本地 Broker 的三来源记录同步、确认发送计数和解绑保留已单独通过。
iOS App 双架构模拟器编译、前端类型检查、Clippy 与编译器生成的 663 条本地化目录检查通过。
高资源压力时两项 Swift 计时测试曾超时，单独及最终全量重跑均通过；用户最后选择顺序后台验证。

手机端已随内外 TestFlight `0.1.0 (20)` 分发（源码 `6cd396fc`，见 [Build 20 交付记录](plans/ios-channel.md#2026-11-12testflight-内部与外部构建-20)）。
下一步在手机核验：更新后打开 Macs 页，核对本月流量合计与 EMQX 控制台的差距，并确认「历史记录不完整」
的提示合理，完成后删除本节。用户随后授权升级第二台 Mac
`192.168.31.128`，已将通过真实双 daemon 流量测试的同签名 arm64 构建（SHA-256 前缀
`70b8584a2592`）原子替换到现有 npm 包目标，后台已 exec 接管新文件；原 Mac 与手机身份、
2 条待答请求的 ID、本月累计均保留，TLS 连接和手机推送状态正常，统计保存成功。
本月早期手机丢失的统计无法从现有文件重建，界面保留历史不完整提示。
