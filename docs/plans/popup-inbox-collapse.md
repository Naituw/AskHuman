# 合并窗口「稍后回答」与小窗：实施计划

> 状态：2026-10-17，用户已审阅并通过本计划（第 8 轮，结论见 §7）；按 P0 → P9 实施，每阶段验证通过后提交一次。
> 需求与全部决策（C1–C32、R1–R16）以 [`docs/specs/popup-inbox-collapse.md`](../specs/popup-inbox-collapse.md) 为准，本文只写怎么做，不重复决策。
> 用户已在第 7 轮试用原型后确认手感并选了「可以进入实施计划」（spec §1.8）。
> 关联：[多请求统一作答窗口](../specs/popup-request-inbox.md)、[多问题纵向模式](../specs/multi-question-vertical.md)、[Popup UI 概览](../overview-popup-ui.md)。

## 1. 目标与边界

做什么：把原型里已被用户认可的行为接进正式的合并窗口（`PopupInboxView` + Popup Host）——

1. 每条请求可标记「稍后回答」（C1、C18、C21、C24）；窗口因此在「全部稍后答」时缩成迷你列表小窗，有主要请求时展开（C2、C4、C13、C14）。
2. 缩小 / 展开动画（macOS，原生逐帧；C5、C15）；Windows / Linux 即时切换（C6）。
3. 答完 / 终结后的跳转、切走缩回、peek 的手动缩回图标、到达与托盘规则（C19、C22、C23、C27、R1–R12）。
4. 点击 Sidebar 条目的正文切换动画（C17，原型已认可，正式实现一并加）。
5. 拖入文件：停留展开小窗、停留切换条目、边缘自动滚动、按题落点（C28、C31、C32、R13、R15、R16）。
6. 新一轮窗口位置不漂移（C30、R14）。

不做什么：胶囊形态；显眼度档位 / 密度 / 行内按钮变体 / 曲线等原型旋钮（正式实现只取用户选定的默认值，见 §2.6）；「稍后回答」快捷键与定时提醒（C12）；Windows / Linux 的动画与实机验证（延期 gate）；不改 Daemon 协议、不改任何渠道的终态契约（R1）；不新增配置项（标记与小窗状态只存在于本轮内存里）。

必须守住的现有不变量：Sidebar / 正文 / 预览三区一次几何事务（`popup_inbox_geometry` 的 `prepare → commit_frame → finish` 与 `pending` 门闩）；固定画布 + 原子提交；每请求草稿 / 多题进度 / 回复附件 / 滚动 / 焦点独立保留（D9）；成功回答的 126 / 174ms 正文接替与 300ms 行收起；到达气泡；`persist_popup_size` 只记正常尺寸；减少动态效果。

## 2. 实现层决策

### 2.1 前端状态与流程

- 新增纯模块 `src/views/popup/inboxCollapse.ts`，由原型的 `collapseModel.ts` 移植：请求 id 用字符串，复用 `inboxGroups` / `nextInboxRequest` 的顺序（D6），不另立一套。承载 `secondary` 集合、`view: "expanded" | "collapsed"`、`hasPrimary`、`defaultOpenTarget`（记住的上次浏览 → 第一条未查看 → 第一条）、到达 / 标记 / 取消标记 / 浏览 / 缩回 / 终结对「窗口效果」的决议（`collapse | expand | switch | sidebar | mini-resize | null`）。原型的 41 个单测按字符串 id 改写后随模块进仓库。
- 新增组合式 `useInboxCollapse.ts` 承担窗口形态切换的编排（缩小 / 展开 / 交叉淡化 / 测量小窗 / 激活状态），把 `PopupInboxView.vue`（已 700 行）里的新增逻辑挡在外面；把 Sidebar 抽成 `InboxSidebar.vue`（纯重构先行，行数据与事件走 props / emits），行上的归档盒、标签、拖放高亮都放进去。
- `PopupInboxView` 持有 `secondary`（`Set<string>`）、`view`、`engaged`（应用激活状态，§2.5）、`collapsing`（动画中，屏蔽 Sidebar 拖动 / 预览开关等入口）。`scopeFor` 给每个请求暴露 `secondary` / `markLater()` / `unmarkLater()`（`PopupScope` 加字段），供底栏与 Sidebar 行使用。`finishRound` 清空这些状态（R9）。
- 接入现有流程（均在现有串行队列 `enqueue` 里，不另起并发）：
  - `completed()` / `terminal()` / `advance()`：选下一条时先找主要请求，没有则按 C22 找下一条稍后答（用户在场：本窗口内答 / 取消，或远端终结时应用处于激活）；没人在看或窗口本来是小窗 → 缩 / 保持小窗，不翻出旧请求。300ms 行收起与正文接替的时序保持不变。
  - `add()`（新到达）：窗口是小窗 → 展开并显示新请求，非激活前置，播到达气泡（C4）；窗口已展开 → 不切正文，只在行上做气泡 / 未读提示（C19、D7）。
  - 托盘 `popup-inbox-focus`（`select(id, true)`）：显式聚焦某条（含稍后答）→ 展开并打开它（R6）。
  - `select()` 中点击切换（点 Sidebar 行，以及底栏稍后答 / 取消稍后答后的跳转，C35）：加 C17 的正文淡出 / 滑入，复用 `answerTransition`（126 / 174ms）的曲线；减少动态效果时直接切。托盘点选、到达、远端终结后的接替保持即时。
- 请求 `visited`（蓝点 / ripple）语义不变：稍后答请求未查看时蓝点与 ripple 继续，真正显示正文才清除（R7）。

### 2.2 窗口事务（Rust + Swift，macOS）

- **Swift**：`swift/PopupCollapse.swift` 里面向产品的入口只留 `ah_collapse_animate`（逐帧 display-link、`cubic-bezier(.2,0,0,1)`、宽高同时、左上角锚定、红绿灯淡出、可被打断并从当前帧继续、动画期间最小尺寸降为 1×1 并在小窗期间去掉 `.resizable`——原型已验证）。其余的——`prepare` / `resume`（只是对产品里已有的 `ah_popup_canvas_prepare` / `resume` 的薄包装，产品直接用后者）、`front`（`popup_transition::front` 已有）、`info`、`capture` / `diff` / `backdrop` / `control` / `burst_*` / `forbid_activation` / `activate_other`——都只给原型验证用，挪到 `swift/prototype/PopupCollapseVerify.swift`，由 `build.rs` 仅在 Cargo feature `collapse-prototype` 打开时编译进静态库；`popup_collapse_demo` 示例加 `required-features`，`scripts/popup-collapse-demo.sh` 传该 feature。产品二进制里不再有测试钩子。（计划初稿曾把 `prepare / resume / front / info` 也列为产品入口，P1 核对后更正：它们不被产品路径使用，见 §8 P1。）
- **Rust**：缩小 / 展开沿用现有三步协议 `prepare → commit_frame → finish`，不另造第二套所有权：`popup_inbox_layout` 增加「形态 + 小窗目标尺寸」入参，`popup_inbox_commit` 增加「动画参数」（时长 / 曲线 / 红绿灯），`commit_frame` 在 macOS 上用 `ah_collapse_animate` 替代瞬时的 `popup_canvas::apply`，`finish` 照常 `resume` 并发布一个带 `collapsed` 标记的 `Allocation`。同一个 `pending` 门闩与 revision 保证它与 Sidebar / 预览 / 外缘缩放事务互斥。P2 开工前先读 `popup_canvas::apply` 评估；若复用会让既有事务变复杂，退回独立的 `app/popup_collapse.rs` 命令，仍共用 `pending` 门闩。
- 小窗尺寸来自页面在小窗布局下测量的真实 DOM（原型的做法），由页面传给 Rust，**不写死**，也**绝不写入尺寸偏好**（R10：`persist_popup_size`、`resized()` 在小窗 / 动画中一律返回 `None`）。展开恢复缩小前记下的稳定展开尺寸（被打断时保留，不记动画中途帧）；放不下则平移（C5，沿用 `allocate` 与工作区夹取）。
- 页面等待首帧用 `paint()`：`requestAnimationFrame` 加定时器上限（窗口被遮挡 / 非激活时 rAF 可能暂停），不得只靠 rAF。
- 小窗期间右侧原生预览（PDF / Quick Look）随正文被裁切隐藏；缩小前先让它隐藏，展开后由现有 `nativePreviewSync` 恢复位置与阅读状态，避免 NSView 漏在小窗外。

### 2.3 Geometry 与「新一轮不漂移」（C30、R14）

- 现状：`anchor` = 正文左上角，窗口 x = `anchor.x − left_span`（D5：Sidebar 出现时窗口向左长、正文不动）；`commit_frame` / `resized` / `moved` 都以 `帧 x + left_span` 回写 `anchor`；`reset()`（`popup_inbox_idle`）把窗口放回 `anchor.x` 并清空状态。其中 `moved()` 用的是 `c.sidebar` 标志与已发布的 `sidebar_width`，**不认识小窗**，必须加分支。
- 小窗的左缘与缩小前一致（左上角锚定，Sidebar 列即小窗的左列）。**实施时没有另设 `Geometry.collapsed` / `restore_frame`**：已发布的 `Allocation` 新增 `collapsed` 字段，小窗期间发布的分配仍保留展开布局的各列宽度与 `main_height`（页面照旧按展开布局排版、视口钉住），只有 `frame` 是小窗尺寸，所以小窗态直接由已发布分配推出（`is_collapsed()`）；`moved()` 用 `窗口 x + 已发布分配的 left_span` 回写 `anchor`（C5：用户拖过的位置本轮内记住、展开时以小窗为锚），展开的目标位置由 `anchor` 与新的 `left_span` 算出，不必保存缩小前的帧。
- **以小窗为锚展开会产生「轮内位移」**：单请求缩成小窗后来了第二条（要出 Sidebar，`left_span` 从 0 变 L）、或两请求缩成小窗后一条被终结（L 变 0），左上角不动意味着正文相对缩小前的位置右 / 左挪 L。这是 C5 的预期（原型即如此），但若 `reset()` 照旧回 `anchor.x`，位移会带进下一轮，逐轮累积，正是 C30 要杜绝的。因此新增 `Geometry.drift: f64`（`anchor.x − home.x`，默认 0，`home` 为空闲窗口应在的 x）：只在「以窗口左缘为锚」的提交里累加 `drift += left_span(新) − left_span(旧)`，普通的 D5 提交不动它；`moved()` 同时平移 `anchor` 和 `home`（用户移动是唯一让 `home` 改变的原因，夹取到工作区除外，沿用现状）；`reset()` 回到 `anchor.x − drift`，一次事务到位。无小窗的现有流程 `drift` 恒为 0，行为与今天完全一致。
- 验收：Rust 表驱动测试模拟多轮序列并断言「无用户移动时，每轮首窗 x 与上一轮结束时相同」：`[Sidebar 增长, 缩小, 展开, 结束]`、`[缩小, 拖动小窗, 展开, 结束]`（用户移动被保留）、`[单请求缩小, 到达展开, 结束]`、`[两请求缩小, 一条终结, 展开, 结束]`、`[在小窗状态下结束一轮]`，每个序列重复 ≥5 轮；`resized()` 在小窗 / 动画中返回 `None`；`moved()` 在 `pending` 期间仍被忽略。

### 2.4 小窗的 DOM 与外观

- 沿用原型策略：WebKit 视口在整个切换中被画布固定，页面不 reflow；Sidebar 元素兼作小窗列表（标题上移让出红绿灯位——小窗里红绿灯隐藏，条目的元信息行用 max-height 收起），主区与列表用不透明度交叉淡化，强调色描边 `mini-ring` 在窗口收拢后淡入。**单请求时没有 Sidebar**：缩小前先让 Sidebar 元素在画布左侧预留区就位（原型 `layoutSidebar` 的做法），展开回无 Sidebar 布局；该路径单独测试（C20、D5）。非 macOS 没有固定画布，视口随窗口变化，小窗布局由页面在 `collapsed` 状态下直接渲染。
- 小窗标题保持与展开时同一个标题加计数胶囊 + 强调色描边（C14、C29）；行数上限与滚动同原型默认（3 行，可滚）；宽度沿用用户拖出的 Sidebar 宽度（R8）；迷你列表里所有行都是稍后答，悬停出现「取消稍后答」（C18）。**标题措辞（用户第 8 轮定）**：展开的 Sidebar 与小窗都用「待答 N」，即把 `popup.inbox.pending` 的中文从「等待回答」改成「待答」（同一个键也是 Sidebar 的 `aria-label`，一并随之变化）；英文用户没有提出改动，保持 "Waiting for answers"。
- Sidebar 行：悬停出现归档盒图标（「稍后回答」），点击后同位置变成常驻「稍后答」标签，悬停该行时变「取消稍后答」按钮（C18、C21）。底栏「取消」旁的按钮随状态在「稍后回答 / 取消稍后答」间切换（C24，改 `PopupFooter.vue`，三种底栏排布都要有）。peek 的缩回图标放 Sidebar 标题栏右侧，占小窗「展开」图标的槽位（同 x）；无 Sidebar 的 peek 放顶栏图钉旁（`PopupNavbar.vue`）（C27）。

### 2.5 应用级激活（C23、R11）

- 「切走缩回」以 **Popup Host 进程**的应用激活状态为准，不以某个窗口为准。macOS：Swift 里观察 `NSApplication.didBecomeActive / didResignActive`，经 Rust 发 `popup-app-active` 事件给页面；页面保留原型的 200ms 抖动保护（过滤同一应用内两个窗口之间的焦点移动）。Windows / Linux：用 popup 窗口的 `Focused` 事件。设置 / 历史等 GUI Host 窗口在另一个进程里，焦点移过去等同「切走」，仅影响 peek，可接受（记入风险）。
- 非激活前置（`popup_transition::front`）打开的窗口，应用可能根本不处于激活：拖放松手后 AskHuman 前置并取焦点（原型 `dropActivates`），之后的切走才能触发缩回。peek 的缩回规则：有主要请求时永不缩、窗口已是小窗无动作、拖拽进行中不触发。

### 2.6 产品取用的原型默认值

小窗形态 = 迷你列表；显眼度 = 中；行内按钮 = 归档盒；次要排序 = 位置不变；红绿灯 = 小窗里隐藏；收起 440ms、展开 480ms、曲线 `cubic-bezier(.2,0,0,1)`、宽高同时、无回弹；切走缩回 = 开（不设开关）；到达气泡 = 播放、展开后显示新请求；拖放：`dragExpandMs` 500、`dragSwitchMs` 250、拖走后缩回 = 开、松手后前置 = 开、边缘区 56px / 上限约 900px/s（平方曲线）、4px 防连环。这些写成常量放在一处（`inboxCollapse.ts` / `inboxDrop.ts`），不做成设置项。

### 2.7 拖放（`PopupInboxView` 窗口级）

- 新增 `src/views/popup/useInboxDrop.ts`，由 `PopupInboxView` 注册**一个**窗口级 `getCurrentWebview().onDragDropEvent`；`usePopupCore` 在有 inbox scope 时不再注册自己的监听（独立模式的 `PopupView` 保持原样），改为向 scope 暴露 `acceptDrop(paths, qIndex)`、`dropTargetAt(point)`（纵向：卡片命中；顺序：当前题）、`currentQuestion()`，落盘仍走既有 `addDroppedPaths`。这样一次拖拽可以落到**非当前**请求；该请求的 core 若尚未挂载，先 `preparedIds.add(id)` 并等它 ready（`prepareRequest` 现成），超时则提示失败、其余文件不丢。
- 事件处理：串行 + 合并（`enter` / `over` / `drop` / `leave` 排成一条链，堆积的 `over` 只保留最新位置；`leave` / `drop` 永不丢）；停留用定时器、不占链；`enter` 与 `over` 都能触发展开等待（`enter` 可能被错过）。落点 y 要按 wry 在 macOS 的算法修正（`webview.frame.height − draggingLocation.y`，小窗 / 动画中 WebView 比窗口高，差值 = 视口高 − 窗口高），x 沿用 `inboxDropPoint`；以 Rust 发布的 `canvas` / `frame` 为准，不新增轮询。
- 目标解析与落点见 spec R13、R15、§6.8 的落点表（小窗行 / 标题 / Sidebar 行 → 该请求当前题；正文 → 当前请求当前题；纵向卡片 → 那一题）。提示：行描边、等待进度条、整块正文或题卡的虚线框与「松手添加到第 N 题」胶囊（单题写「松手添加到当前请求」），小窗在拖放中显示实线描边。边缘滚动覆盖 Sidebar 列表与正文滚动容器，纵向多题的卡片同样适用。
- 保持：拖出自家附件时不接管、预览区 / 分隔线落点不进回复附件、`selectOnly` 请求不接收（现有 `addDroppedPaths` 判断）。读取失败只记录，不影响其余文件；落点不在任何目标上时忽略。

### 2.8 文案（i18n，`src/i18n/zh.ts` / `en.ts`）

中文已由 C21 / C27 / C29 / R15 与用户第 8 轮的「待答」定；英文用户已确认按建议：动作 **Answer later**、状态 **Later**、撤销 **Undo answer later**、展开 **Expand**、缩回 **Shrink to small window**、拖放提示 **Release to add to question {n}** / **Release to add to this request**、提示条 **Added {n} file(s) to “{title}”, question {q}**。所有新增文案走 vue-i18n，不写死中文；可访问名称同步。

## 3. 分阶段实施

每个阶段结束都要：相关自动测试通过、`cargo fmt --check`、`pnpm exec vue-tsc --noEmit`、自己 review diff，然后提交一次（§6）；用户可见行为的阶段再按 §4.1 做隔离实机验证。**全部阶段完成之前不运行默认的 `./scripts/install.sh`**（它会覆盖日常二进制，用户第 8 轮明确「全部完成再覆盖二进制」）。按顺序做，前一阶段未过不进下一阶段。

**P0 准备**：在当前主工作树做（不建 worktree / Dev Instance，§7）；先把已有的 spec、本计划和原型作为独立提交入库（`docs(popup)`、`chore(prototype)`，不进发布说明）；记录基线——`cargo test --lib`（popup 相关）、`pnpm test`、原型 `--auto` 367 项与 `--auto-controls` 18 项（静默，不抢焦点）作为「行为参照」；按 §4.1 试跑一次隔离安装，确认它不碰日常 daemon / 二进制，后续阶段才依赖它。

**P1 Swift 驱动产品化**（不改行为）：拆出 `swift/prototype/PopupCollapseVerify.swift` + feature `collapse-prototype`（§2.2）。验收：产品构建（无 feature）通过且不含验证符号，带 feature 的原型构建通过并仍跑过 `--auto`。

**P2 Geometry 与窗口事务**（Rust，无 UI）：`Geometry.collapsed` / `drift`、`moved` / `reset` / `resized` 的小窗语义、`prepare → commit_frame(animation) → finish` 的小窗形态（macOS 动画 + 非 macOS 即时）、`Allocation.collapsed` 与 TS 类型、`app/mod.rs` 窗口事件对小窗的分发。验收：§2.3 的多轮不漂移表驱动测试；`resized` 过滤与 `persist_popup_size` 不写小窗尺寸；与 `pending` 门闩互斥的测试。

**P3 前端模型与流程**：`inboxCollapse.ts` + 单测；`PopupInboxView` 的 `secondary` / `view` / 流程接入（§2.1）；`PopupScope` 扩展；`finishRound` 重置。验收：vitest 覆盖 R1–R12 的数据规则、答完跳转（含「没人在看」与「窗口已是小窗」）、到达与托盘、远端终结；既有 `PopupInboxView*.test.ts` 不回退。

**P4 UI**：先抽 `InboxSidebar.vue`（纯重构，单独提交，既有测试不改断言）；再做 Sidebar 行归档盒 / 标签 / 悬停按钮、底栏按钮三种排布、peek 缩回图标（含无 Sidebar 版）、小窗列表外观与测量、交叉淡化与 mini-ring、C17 正文切换动画、`UnreadRipple` / 蓝点 / 到达气泡与小窗的衔接、减少动态效果。验收：组件测试；按 §4.1 隔离安装后用真实多请求（并发 `AskHuman`）目测与测量（§4）——**用户第 9 轮定：推迟到 P5 之后与 P5 合并做一次（§7.1）**。

**P5 激活与切走缩回**：Swift 观察者 + `popup-app-active` + 页面规则（§2.5）；非 macOS 的 `Focused` 路径。验收：vitest 驱动事件序列；实机：从小窗展开后点别的应用 → 缩回；有主要请求时不缩。

**P6 拖放**：`useInboxDrop.ts`、scope 扩展、`usePopupCore` 在 inbox 下让位、高亮 / 进度条 / 边缘滚动 / 按题落点、y 修正、串行合并（§2.7）；**把「拖拽进行中不触发切走缩回」接进 P5 的 `collapseOnLeave`**（P5 还没有拖拽状态，§2.5 里的这条规则留到这里补）。验收：vitest 复刻原型 S17 / S18 的断言（停留展开、不连环切换、落点表、边缘滚动、纵向 / 顺序、单题）；实机让用户真拖一次。

**P7 Windows / Linux 即时路径**：非 macOS 的小窗实现与最小尺寸 / 红绿灯处理；本机能做的范围内以 `cfg` 检查覆盖；**实机验证记入延期 gate**（`docs/PROGRESS.md`），不据此宣称这两个平台已验收。

**P8 文案与文档**：i18n 中英；`docs/overview-popup-ui.md` 更新（窗口模型 / 小窗 / 拖放 / 激活）；spec 状态改为「已实现」并链接本计划；本计划 §8 填实施与验证记录；`docs/PROGRESS.md` 删除本事项的 section，补上 Windows / Linux 延期 gate；主 overview 只有全局心智模型变化时才改（预期不改）。

**P9 总验收与覆盖安装**：全部阶段的自动测试再跑一遍（含原型 `--auto` 作行为参照）；然后**先向用户说明实际情况再动手**——默认安装目标 `~/.local/bin` 目前为空、日常 `AskHuman` 是 npm 全局包（§4.1），确认「覆盖」具体怎么做（只安装到 `~/.local/bin`，还是进一步让开发版接管日常 daemon），得到同意后才执行默认 `./scripts/install.sh`，再用新装的 `AskHuman` 提问做最终实机验收（AGENTS.md 流程），并在 §8 记录。

## 4. 验证矩阵

| 层面 | 方式 | 内容 |
|---|---|---|
| 纯逻辑 | vitest | `inboxCollapse` 模型（R1–R12）、拖放目标解析 / 落点 / 防连环 / 边缘滚动速度曲线、串行合并 |
| 组件 | vitest + Vue Test Utils | Sidebar 行三态、底栏三种排布的按钮、peek 图标显隐、小窗标题计数、拖放高亮与提示 |
| 几何 | Rust 单测 | `anchor` / `drift` 不变量与多轮不漂移、`resized` / `persist` 过滤、`pending` 互斥、`allocate` 与小窗尺寸 |
| 原生 | 沿用 `popup_inbox_demo` 示例与 `ASKHUMAN_INBOX_LAYOUT_REVIEW` 几何采样；原型 `--auto` 作行为参照 | 缩小 / 展开左上角锚定、尺寸 = DOM 测量值、视口钉住、页面与原生视口原点一致、帧统计（沿用原型口径）；**静默运行，不抢焦点（C26）** |
| 端到端 | §4.1 的隔离安装（最终用默认安装）+ 并发 `AskHuman` | 真实多请求：稍后回答 → 缩小 → 点开 → 切走缩回 → 新提问展开 → 答完跳转 → 一轮结束 → 下一轮窗口 x 不变；单请求小窗；多项目分组；预览打开时缩小 |
| 手工 | 用户 | 真 Finder 拖拽（停留手感、落点）、动画观感、材质（Solid / Blur / Glass）下的外观 |

需要取走焦点或点击真实窗口的验证（含用 Computer Use 后台驱动真实产品窗口），**先问用户**（C26）。本计划不承诺 Windows / Linux 的实机结论。

### 4.1 中间阶段的实机验证（不覆盖日常二进制）

用户要求全部完成再覆盖日常二进制（§7），所以中间阶段不跑默认的 `./scripts/install.sh`。（P0 实测：这台机器日常用的 `AskHuman` 是 nvm 下的 npm 全局包 `~/.nvm/versions/node/*/bin/AskHuman`，其 daemon 用默认 `~/.askhuman`；`~/.local/bin/AskHuman` 目前不存在。所以默认安装不会替换 PATH 上排前面的那个，真正会产生「覆盖」效果的是开发版二进制以默认 home 运行、与日常实例共用 `~/.askhuman` 的 daemon 与配置——这正是下面的隔离做法要避免的。）需要真实产品窗口时：

```bash
# 只写临时目录；编译仍用 src-tauri/target/local-install，安装脚本在显式 INSTALL_DIR 下不碰 ~/.local/bin
INSTALL_DIR=/tmp/askhuman-collapse-qa/bin ./scripts/install.sh
# 从树外目录运行；显式 ASKHUMAN_HOME 让配置 / daemon 端点 / launchd label 全部独立，不与日常实例互相接管
cd /tmp && ASKHUMAN_HOME=/tmp/askhuman-collapse-qa/home ASKHUMAN_NO_KEYCHAIN=1 \
  /tmp/askhuman-collapse-qa/bin/AskHuman -q "…" -o "…"
```

依据：`docs/specs/dev-instance-parallel.md` §5.3 / §6.2（显式 `ASKHUMAN_HOME`、显式 `INSTALL_DIR` 优先；实例进程不写登录项）。**P0 已试跑通过**（见 §8）：临时实例起了独立 daemon（socket 在 `/tmp/askhuman-collapse-qa/home/`），日常 daemon（pid 与 uptime 不变）未受影响。每次测试结束必须清理：`daemon stop` 后还会留下一个 `--gui-host` 常驻进程，需 `pkill -f "/tmp/askhuman-collapse-qa/bin/AskHuman"`；整个任务结束时再删 `/tmp/askhuman-collapse-qa`。弹窗仍是非激活前置，但真实交互（点击、拖拽）需要用户在场，先问再弹。

## 5. 风险与对策

- **与现有事务竞争**：Sidebar 拖动 / 预览开关 / 外缘缩放在小窗动画期间到达。对策：同一 `pending` 门闩 + revision，冲突按现有 `geometry_conflict`（"preview geometry changed"，页面已当作内部协调结果静默处理）；页面在 `collapsing` 期间屏蔽这些入口。
- **动画偶发跳动**：用户已说「先这样吧」（C25），不额外补救；沿用原型的帧统计口径，若真机回归再启用 spec §6.5 ④ 的候选补救。
- **GUI Host 窗口算「切走」**：peek 时点设置 / 历史会触发缩回，是进程边界带来的结果，可接受；若用户反感，再考虑把 Popup 发起的 GUI Host 窗口视作「应用内」。
- **轮内位移**：以小窗为锚展开、Sidebar 有无不同时正文会相对缩小前挪 `left_span`（§2.3），已通过 `drift` 保证不带进下一轮；这是 C5 的结果，不是缺陷。
- **按需挂载**：拖放落到从未打开过的请求时，目标 core 要先挂载并就绪；超时提示失败、不丢其余文件；列入 P6 测试。
- **产品二进制符号**：验证符号隔离靠 Cargo feature；P1 用 `nm` / 链接检查确认。
- **英文文案**：待用户审阅；不阻塞逻辑实现。

## 6. 文档与提交策略

- 提交遵循 Conventional Commits（AGENTS.md），**逐阶段提交**（用户第 8 轮定）：每个阶段验证通过后一次提交。发布说明只收 `feat` / `fix` 等，所以脚手架类提交用 `refactor` / `test` / `chore` / `docs`，用户可见的只留少数几条带清晰 `Release-Note:` 的 `feat(popup)`：「稍后回答与迷你小窗」「拖入文件到任意待答请求」。预期提交序列：`docs(popup)` spec + 计划 → `chore(prototype)` 原型 → `refactor(popup)` P1 → `refactor(popup)` P2（不可见） → P3 / P4 / P5 合成可见功能的 `feat(popup)` 提交（带 `Release-Note:`），其间的中间提交用 `refactor` / `test` → `feat(popup)` 拖放 → `docs` 收尾；Windows / Linux 路径随 P2 / P7 以不进发布说明的类型提交。
- 本计划随实施更新：每阶段完成后在 §8 记录证据（测试数、命令、结论），未验证的明确写「未验证」。
- 完成后删除 `docs/PROGRESS.md` 中本事项的 section（历史留在 git），仅保留 Windows / Linux 实机 gate。

## 7. 用户审阅结论（2026-10-17，第 8 轮）

用户通过了本计划，并就下列事项作答（AskHuman 原话选项 + 补充）：

1. 计划：通过，按计划实施。
2. 实施位置：**就在当前主工作树做，不建 worktree / Dev Instance**；补充要求「**全部完成再覆盖二进制**」——所以中间阶段不运行默认的 `./scripts/install.sh`，改用 §4.1 的隔离安装验证，全部完成后才覆盖一次（P9）。（提问时我把「日常二进制」写成了 `~/.local/bin` 里的那个，P0 实测并非如此，见 §4.1 与 P9；P9 动手前会再向用户确认「覆盖」的具体含义。）
3. 提交：**逐阶段提交**（§6）。
4. 原型（`src/prototype/` 下的 Collapse*、`popup_collapse_demo.rs`、`scripts/popup-collapse-*`、`swift/PopupCollapse*.swift`）：**进仓库**，作为行为参照与回归基线（`chore` / `test` 类型，不进发布说明；同已有的 `popup_inbox_demo`、`pane_resize_demo` 惯例）。
5. 实验开关：**不留**，直接作为正式行为。
6. 标题措辞：**「待答 N」**，展开的 Sidebar 与小窗都用它（§2.4）。
7. 英文文案：按 §2.8 的建议。

### 7.1 检查点（第 9 轮，P4 完成后，2026-10-17）

用户的回答（详见 spec §1.10，C35–C37）：

1. 换题动画范围：**扩到底栏「稍后答 / 取消稍后答」后的跳转**（§2.1 的「`select()` 中点击切换」随之放宽成「点 Sidebar 行 + 底栏稍后答后的跳转」；托盘点选、到达、终结后接替仍即时）。
2. 实机验证：**P4 + P5 做完后合并做一次**（§4.1 隔离安装），之前只跑静默的自动测试，开始前先告知用户。这修改了 §3 里 P4「验收」中的目视 / 测量项的时间点：它们移到 P5 之后与 P5 的实机项一起做。
3. P2 / P3 / P4 记录里的补空白决定：认可，按现状继续。

### 7.2 实机验证开始前（第 10 轮，P5 完成后，2026-10-17）

用户的回答（详见 spec §1.11，C38）：

1. 实机验证：**现在开始**（知道会有一步短暂抢焦点）。
2. P5 记录里的 3 条补空白决定：认可，按现状继续。

### 7.3 实机验证之后（第 11 轮，2026-10-17）

用户的回答（详见 spec §1.12，C39–C40）：

1. 顺带修复的 ACL 缺陷（`b0c9bbd`，不在原计划内）：保留，并写发布说明。
2. 下一步：继续 P6；**P6 做完后再一起弹真实窗口给用户试**——P4 / P5 的动画观感、真实鼠标点击 / 拖动小窗、切走缩回的宽限等「仍未验证」项并入那一次会话（§8「P4 + P5 实机验证」）。

### 7.4 P6 完成与实机会话（第 12–13 轮，2026-10-17）

用户的回答（详见 spec §1.13，C41–C46）：

1. P6 记录里的 6 条补空白决定：全部认可，按现状继续（C41–C46）。
2. 实机会话（P4 + P5 + P6 合并，§8「P4 + P5 + P6 实机会话」）：动画「顺滑」；P5 行为、P6 的触发 / 投放 / 边缘滚动 / 拖出窗口均「符合预期」；视觉「清晰好看，保持」；报告了一个 UI 缺陷（小窗里悬停时「取消稍后答」盖在标题上），已修。
3. 纵向布局：要再试一轮（3 步），因为按卡落点此前只在自动测试里验证过。**结果（第 14 轮）：落在指针下的卡、卡外落当前题加边缘滚动、落在 Sidebar 里该请求的行，全部「符合预期」；UI 修复在真窗口里看过，没问题。**
4. 实机中暴露的 Stop hook 改写问题：记为项目待办，不在本任务里修。

## 8. 实施与验证记录

每阶段一条：做了什么、验证命令与结果、未验证项。

### P0 准备（2026-10-17）

- 入库：`9723fd4` spec + 本计划，`2b1c053` 原型（`chore(prototype)`，不进发布说明）。
- 基线（正式代码尚未改动）：`pnpm exec vitest run` 57 files / **430 passed**（含原型 41）；`node --test scripts/*.node-test.mjs` 32 passed；`vue-tsc --noEmit`、`cargo fmt --check` 通过；Rust 测试在 bin 目标里（无 lib 目标），`cargo test --manifest-path src-tauri/Cargo.toml --bin AskHuman popup`：**65 passed**（bin 共 1337 个），其中 `popup_inbox_geometry` 8 passed。原型 `--auto` 367 项 / `--auto-controls` 18 项沿用上一轮结果（原型源码自那次通过后未再改动），P1 动 Swift 后重跑。
- 隔离安装试跑（§4.1）：`INSTALL_DIR=/tmp/askhuman-collapse-qa/bin ./scripts/install.sh` 编译 1m12s、签名通过；以 `ASKHUMAN_HOME=/tmp/askhuman-collapse-qa/home ASKHUMAN_NO_KEYCHAIN=1` 从 `/tmp` 运行，起了独立 daemon（socket 在临时目录），日常 daemon（pid 53110、socket `~/.askhuman/daemon.sock`）不受影响；`daemon stop` 后会留下一个 `--gui-host` 常驻进程，需要 `pkill` 清理（已写进 §4.1）。
- 发现：日常 `AskHuman` 是 npm 全局包，不是 `~/.local/bin` 里的（见 §4.1、P9、§7）；P9 前要向用户确认「覆盖」的具体含义。

### P1 Swift 驱动产品化（2026-10-17）

- 改动：`swift/PopupCollapse.swift` 只剩产品驱动 `ah_collapse_animate`（563 行验证代码移走）；新增 `swift/prototype/PopupCollapseVerify.swift`（`prepare` / `resume` / `front` / `info` / `forbid_activation` / `activate_other` / `capture` / `diff` / `backdrop` / `control` / `burst_*`，原样搬移）；`build.rs` 仅在 `CARGO_FEATURE_COLLAPSE_PROTOTYPE` 下收集 `swift/prototype/*.swift`；`Cargo.toml` 加 feature `collapse-prototype`，示例 `popup_collapse_demo` 加 `required-features`；`scripts/popup-collapse-demo.sh` 传该 feature；`docs/development.md` 同步。
- 更正计划初稿：`prepare` / `resume` 只是产品已有的 `ah_popup_canvas_*` 的薄包装，`front` 与 `popup_transition::front` 重复，`info` 是测试读数，产品路径都不用，所以产品只留 `ah_collapse_animate`（§2.2 已改）。
- 验证（均通过）：`swiftc -typecheck` 产品源码集与「产品 + prototype」源码集各一次（仅有既有的弃用 / async 锁警告）；无 feature 的产品配置 `cargo check` 后，该配置的 `libahspeech.a` 只导出 `ah_collapse_animate` 与 `ah_popup_canvas_*`（对照：带 feature 的库另有 `prepare` / `resume` / `front` / `info` / `capture` / `diff` / `backdrop` / `control` / `burst_*` / `forbid_activation` / `activate_other`）；`cargo test --bin AskHuman popup` 65 passed（与基线相同，说明产品配置能链接）；`cargo fmt --check`、`vue-tsc --noEmit` 通过。
- 原型回归（带 feature、静默、有焦点守卫）：`--auto` **367 PASS / 0 FAIL**、`--auto-controls` **18 PASS / 0 FAIL**，帧统计与此前同量级（每次切换丢帧 0–4，位置误差 0.50px）；守卫轮询 `lsappinfo front`，全程前台始终是原来的应用（437 / 69 次采样，未被原型抢走），结束后无 `PopupCollapseDemo` / `vite --port 5199` 残留。
- 副作用：`cargo clippy --all-targets`（Linux CI）不再编译 `popup_collapse_demo`（`required-features`，该示例只在带 feature 时才构建）；其余示例照旧检查。未验证：Linux CI 本身（本机不跑）。

### P2 Geometry 与窗口事务（2026-10-17）

- 改动：
  - `popup_inbox_geometry.rs`：新增 `Mini` / `Form`（`Keep` / `Expanded` / `Collapsed`）/ `Transition`（`Plain` / `Collapse` / `MiniResize` / `Expand`）/ `Animation` / `Request`；`Allocation.collapsed`；纯逻辑的 `Geometry::plan` / `committed` / `finished` / `moved_to`（锁内调用，测试直接驱动同一份代码）；`prepare(window, Request)`、`commit_frame(window, revision, animation)`、`finish`、`commit(window, revision, animation, arrival)`；`resized()` 在 `pending` 或小窗时忽略（`persist_popup_size` 因此不会写入小窗尺寸）；`resize_sidebar` / `resize_preview` 在小窗拒绝；`moved()` 委托 `moved_to`；`reset()` 用 `home` 一次事务回到 `anchor.x − drift` 并还原展开形态（包括 `resizable`、红绿灯、最小尺寸）。
  - 锚定分三种：`Body`（现状 D5：正文不动、窗口向左长）、`Window`（缩小 / 展开 / 小窗改尺寸：窗口左上角不动、视口在屏幕上的起点不动，`canvas.left' = canvas.left − left_span(旧) + left_span(新)`，`drift` 在提交时累加 `left_span(新) − left_span(旧)`）、`Home`（reset）。
  - `popup_canvas.rs`：`animate`（macOS 走 `ah_collapse_animate`，C 回调带回最终帧；非 macOS 为即时路径：缩小时最小尺寸降到 1×1 并关 `resizable`，展开时恢复，`resume` 再设正常最小尺寸）。`commands.rs`：`popup_inbox_layout` 增 `collapsed` / `dragging`，`popup_inbox_commit` 增 `animation`（均可选，现有页面调用不变）。`usePopupCore.ts`：`InboxLayout.collapsed`。
  - `PopupCollapse.swift`：夹取用的屏幕改为「与窗口帧重叠面积最大的屏幕」（不再依赖 `window.screen`，隐藏中的窗口也准确，避免多屏时回到主屏）；窗口不可见或已最小化时一律即时（隐藏窗口的 display link 可能不触发）。
  - `app/mod.rs` 无需改动：窗口事件本来就经 `popup_preview::resized` / `moved` 转到 geometry，过滤放在 geometry 里。
- 与 spec §2.3 / 计划初稿的差异（实施中补的空白）：①不另设 `Geometry.collapsed` / `restore_frame`，小窗态由已发布分配推出（§2.3 已改）；②事务串行，产品里不做动画途中反向（Swift 驱动支持，前端按序提交即可）；③最大化 / 全屏时拒绝缩小（返回错误，页面保持展开）；④展开时把 tao 创建时的最小尺寸（420×480）经 `resume` 还给 `contentMinSize`，避免小窗期间留下过大的最小尺寸；⑤`Request.dragging`：拖入文件时主键本来就按着，事务不能等它松开（否则停留展开会死锁），其余事务照旧等指针。
- 验证（均通过）：`cargo test --manifest-path src-tauri/Cargo.toml --bin AskHuman` **1353 passed / 0 failed / 4 ignored**（基线 1337，新增 20 个，均在 `popup_inbox_geometry`：8 → 28）。新增覆盖：左上角锚定与展开布局保留、四种形态判定、`Keep` 保留小窗尺寸、工作区夹取（含贴右缘）、非法 / 最大化请求被拒且无副作用、事务进行中拒绝下一个与过期 revision、`resized` 在 `pending` / 小窗被忽略、小窗不改任何记忆、小窗移动回写 `anchor`、工作区变化只触发一次 reconcile、`dragging` 只作用于当次事务、窗口锚定变化视口屏幕起点不变（macOS）、五条不漂移序列各 6 轮（Sidebar 增长→缩小→展开→结束；缩小→拖动小窗→展开→结束，用户位移被保留；单请求缩小→到达展开→结束；两请求缩小→一条终结→展开→结束；在小窗态结束一轮）外加仅 Sidebar 的对照。变异检查：去掉 `drift` 累加后「到达展开」与「一条终结」两条序列按预期失败。`pnpm exec vitest run` 57 files / 430 passed、`vue-tsc --noEmit`、`cargo fmt --check` 通过。
- 原型回归（静默、有焦点守卫）：改 Swift 后 `--auto` **367 PASS / 0 FAIL**，376 次采样前台始终是原来的应用，结束后无残留进程。`--auto-controls` 本阶段**没有跑成**：守卫在启动瞬间采到控制器窗口成了最前台应用并立即终止了进程（焦点被取走不超过一个采样间隔，约 0.2 秒；之前几次都没出现，推测与启动时前台应用不同有关）。之后没有再试，避免继续抢焦点；该模式不经过新增的产品路径，Swift 改动已由 `--auto` 覆盖。
- 未验证：macOS 原生 `animate` 的端到端（要等 P4 页面接上后，用 §4.1 隔离安装实测）；非 macOS 即时路径（P7 / 延期 gate）；`prepare` / `commit_frame` 的异步外壳（3 秒看门狗、指针等待、失败回退）沿用原有结构，只被既有路径与上面的纯逻辑测试间接覆盖，没有单测。

### P3 前端模型与流程（2026-10-17）

- 改动：
  - 新增 `src/views/popup/inboxCollapse.ts`：原型 `collapseModel.ts` 的纯逻辑移植（`arrive` / `putAway` / `markSecondary` / `unmark` / `browse` / `collapseWindow` / `finish` / `defaultOpenTarget` / `orderedRequests` / `hasPrimary`）。请求 id 用字符串，字段与 `PopupInboxRequest` 同名（`requestId` / `project` / `sequence`）；顺序与「下一条」直接调用 `inboxQueue` 的 `inboxGroups` / `nextInboxRequest`（两者的参数类型放宽成结构类型，行为不变），没有另立排序。与原型的差异：去掉 `sidebarSticky` 与 `"sidebar"` 效果——Sidebar 是页面自己的 D5 状态，`add()` 照旧处理；`markSecondary` / `unmark` 对不在队列里的 id 不做任何事。原型 41 个单测改写为 **40** 个（删 3：两个 Sidebar 效果、一个 `nextPrimary`；加 2：同组内按 `sequence` 排序与 Sidebar 一致、未知 id 守卫）。
  - `PopupInboxView.vue`：`secondary`（集合）/ `view` / `shifting`（窗口形态事务中，并入 `blocked`）/ `appActive` / `lastBrowsed`；`collapseState()` 把页面状态交给模型（已知将终结的请求不参与，除了决议对象本身）；`select()` 即「浏览」（从小窗点行 / 托盘 / `unmark` 都会展开并保留标记）；`add()`：小窗时先挂载新请求并等它 ready，再展开到它上面（非激活前置，`arrival` 通道）；`completed()` / `terminal()` 的「下一条」改由模型 `finish` 决议（主要请求优先；只剩次要请求时在场的用户跳下一条次要，没人在看则缩成小窗；小窗里别的请求终结 → 仅调整小窗尺寸）；`finishRound()` 清空标记并回到展开（R9）；新增页面监听 `popup-app-active`（默认 `true`；事件契约在本阶段定下，Rust 侧发射在 P5）。`geometry()` 按 `view` 向 `popup_inbox_layout` 传 `collapsed: {width, height}`（展开时不带该键，原生按展开处理）。`PopupScope` 加 `secondary` / `markLater` / `unmarkLater`；小窗态下 `nativePreviewBlocked` 为真（native 预览先隐藏）、正文区 `inert`。
- 与计划初稿 §2.1 的差异（补空白，已落实）：①窗口形态编排**暂留在视图里**，`useInboxCollapse.ts` 推迟到 P4，与测量 / 交叉淡化 / C17 动画一起抽出（现在拆只会得到一个带十几个依赖的壳）；②`collapsing` 实际叫 `shifting`，缩小与展开都覆盖；③小窗尺寸是占位（`miniSize()` = Sidebar 宽 × 200，任何有限且 ≥ 1 的值原生都接受），P4 换成 DOM 测量；④`geometry()` 暂不传 `animation` / `dragging`（用到它们的 P4 / P6 再加，现在没有调用方就无法测）；⑤原生拒绝缩小（最大化 / 全屏，错误含 "cannot shrink"）时页面保持展开、不弹错误，标记保留——用户看到的是按钮状态变化，窗口不动（P4 的行标签 / 底栏按钮会反映）。
- 补空白时做的决定（用户未明确过，需要在检查点告知）：①**小窗下的关闭确认层**（⌘W / `popup-inbox-close`）：先展开到 `defaultOpenTarget`（上次浏览 → 第一条未看 → 第一条）再显示确认层，否则确认层会出现在看不见的区域并锁死小窗行（spec §2.3 / §4 的「需约定入口」未定）；②确认层打开期间窗口不缩小：`shrink` 直接返回，远端终结按「在场」处理（有确认层说明用户在用 AskHuman）；③`completed()` 若在小窗态被触发（理论上到不了）按展开处理，避免 `candidate()` 误判「没有后继」而结束整轮。
- 验证（均通过）：`pnpm exec vitest run` 59 files / **499 passed**（基线 430 → 新增 69：模型 40、页面流程 29）；既有 `PopupInboxView.test.ts` / `.arrival` / `.links` 与 `inboxQueue` 一并 109 passed、断言未改。新增页面测试覆盖：标记（单请求缩小且不答不取消、跳下一条主要请求、标其它行不动窗口、跳过次要并回绕、展开态取消标记只翻标记、关闭确认层期间忽略）；离开小窗（`unmark` / 点行 / 托盘展开并保留标记、焦点恢复）；到达（小窗展开到新请求并等 ready、已展开保持正文、新请求在展开前终结则保持小窗）；当前请求终结（答完跳下一条次要且保持展开、答完优先主要、远端终结且没人在看缩小并可再点开、仍优先主要、应用重新激活后跳转、小窗里别的请求终结仅调整尺寸、记住的请求终结仍保持小窗、非当前请求终结不碰窗口）；轮边界与拒绝（小窗里结束一轮后下一轮展开且无标记、原生拒绝缩小无提示、其它失败提示并保持展开、展开失败回到小窗）；未读与草稿保留；关闭确认层。变异检查：对视图逻辑做了 14 处单点变异（远端终结的 `engaged`、到达展开、缩小失败回退、拒绝识别、`select` 展开、`finishRound` 重置、预览隐藏、`inert`、`mini-resize`、`completed` 的 `engaged`、关闭确认层展开、`shrink` 的确认层早退、`engaged` 含确认层、`removeLocal` 清标记），除最后一项（只影响内存，页面上不可观察）外都被测试抓到；其中 `finishRound` 重置与 `engaged` 含确认层最初没被抓到，补了对应断言后抓到。`vue-tsc --noEmit`、`cargo fmt --check` 通过（本阶段无 Rust 改动，未重跑 Rust 测试）。
- 未验证：真实窗口（要到 P4 页面有 UI 后按 §4.1 隔离安装实测）；`popup_inbox_layout` 的 `collapsed` 入参只由 mock 与 P2 的 Rust 侧各自约定，没有端到端；非 macOS。

### P4 UI（2026-10-17）

- 先行提交 `339c3c2`（`refactor(popup)`）：把 Sidebar 抽成 `InboxSidebar.vue`，既有测试断言未改（59 files / 499 passed）。
- 改动：
  - `InboxSidebar.vue`：标题「待答 N」（小窗里计数是强调色胶囊）；标题栏的两个图标（`.mini-expand`：小窗里「展开」；`.mini-shrink`：peek——展开、没有主要请求、有 Sidebar——里「缩回小窗」，显隐由根元素的 `data-view` / `data-primary` / `data-sidebar` 在 CSS 里决定）；行内归档盒旗标（主要行悬停出图标 → 点击后同位置变常驻「稍后答」→ 悬停变「取消稍后答」；小窗里只在悬停时出现）；还有主要请求时次要行淡显。小窗外观全部取自原型已认可的 CSS：标题上移让出红绿灯位（`.sb-lift`，只在 macOS 原生画布下，`data-lights`）、元信息行收起、`data-settled` 后列表定高可滚动。
  - 新增 `inboxMini.ts`：`MINI_ROWS = 3`、动画参数（缩小 440 / 展开 480 / 小窗改尺寸 240ms，`cubic-bezier(.2,0,0,1)`，减少动态效果时不传动画）、`measureMini`（用 `data-measure` 把根元素同步放进小窗布局量真实 DOM 高度，期间 `data-still` 关掉过渡，不绘制）。
  - `PopupInboxView.vue`：
    - 窗口形态：`geometry(arrival, form)` 带小窗尺寸、动画参数与 `started` 回调；`shrink()` 先测量再缩小；`select()` 的展开路径带 `expand` 动画。**`look`（页面画出来的形态）与 `view`（页面向窗口提出的形态）分开**：`look` 在原生开始变化的瞬间（`popup_inbox_commit` 之前）才翻转，交叉淡化因此与窗口同步而不是抢先；`--ms`（交叉淡化时长）只在形态变更期间非 0，Sidebar 出现、拖宽等其它布局变化仍随原生提交瞬时发生（对应原型的 `growing` 规则）。根元素新增 `data-view / -primary / -sidebar / -settled / -lights / -shifting` 与 `--ms / --mini-w / -h / -nav-h`，以及 `mini-ring`。
    - 单请求：没有 Sidebar 列，缩小前用 `loneList` 把列表按原生给的宽度（`Allocation.sidebar_preferred`）隐藏排好再测量；展开时列表留着淡出，窗口长成后才移除；缩小 / 展开失败或被拒时回退 `look` / `settled` / `sidebarWidth`。
    - 控件：`expandFromList`（打开 `defaultOpenTarget`）与 `shrinkWindow`（经模型 `collapseWindow`，仍有主要请求时什么都不做）；`PopupScope.shrinkWindow`。
    - C17 正文切换：`choose()`（点 Sidebar 行；检查点后又加上底栏稍后答后的跳转，C35）→ `select(..., animate)`：行立即选中，旧正文 126ms 淡出（此间 `blocked`，该请求的原生预览先隐藏）→ 新正文在 `prepare` 态完成几何与预览就位 → 174ms 淡入；复用已有的 `data-answer-phase` CSS；减少动态效果时即时切换。
    - 到达气泡：小窗展开到新请求时气泡落在已打开的那一行（`shownArrivals`，C4 / C19）。
  - 底栏：新增 `PopupLaterButton.vue`（三种排布共用，紧跟「取消」，标签随标记切换；独立窗口不渲染）；顶栏 `.nav-shrink`（无 Sidebar 的 peek，图钉旁）；`usePopupCore` 的 `laterAvailable / laterMarked / toggleLater / shrinkAvailable / shrinkWindow`、`InboxLayout.sidebarPreferred`、`PopupScope.shrinkWindow`；i18n 中英文。Rust：`Allocation.sidebar_preferred` 与测试。
- 与计划初稿的差异：①**没有抽 `useInboxCollapse.ts`**：形态编排和 `blocked` / `active` / `geometry` / `select` 缠在一起，抽出来是带十几个依赖的壳；编排仍在视图里（共 `shrink` / `change` / `smallWindow` / `leaveBody` 几个函数）。②单请求缩小前列表要先按「将来的 Sidebar 宽度」排好版才能测量，这个宽度只有原生知道（用户拖出的偏好 + 当前画布），所以在原生的 `Allocation` 上新增了 `sidebar_preferred`（计划里写的是 D5「预留区」做法，落到原生字段上更直接）。③`look` / `view` 分离与 `--ms` 只在形态变更期间有效，是计划没写到的实现细节。
- 补空白时做的决定（用户未明确过，需要在检查点告知）：①**C17 最初只用于点 Sidebar 行**（C17 原文的范围；原型是所有换题都带这个动画）：答完接替本来就有自己的 126 / 174ms，而「底栏稍后回答后跳下一条」「托盘聚焦」「到达」「终结后接替」先保持即时——怕快速连点底栏按钮时点击被淡出期间的忽略吞掉。**检查点上用户选了给底栏稍后答 / 取消稍后答后的跳转也加上动画（C35）**，见下面「检查点后的调整」；托盘聚焦、到达、终结后接替仍即时。②淡出期间的点击**忽略**而不排队；正文还在淡入时再点，直接替换、不再先淡出；目标在淡出期间被终结，则旧正文留下、不切。③换题时旧请求的原生预览（PDF / Quick Look）在点击瞬间隐藏，因为原生视图淡不出来；新请求的预览在就位后随正文出现。④底栏在「取消」旁多了一个按钮，容器宽度 < 520px 时先隐藏快捷键提示避免挤出（视觉取舍，未目测）。⑤到达气泡「落在已选中的那一行」按 spec §6.4 的描述实现。
- 验证（均通过）：`pnpm exec vitest run` 63 files / **551 passed**（P3 的 499 → 新增 52：`inboxMini` 10、页面形态与正文切换 `PopupInboxView.mini` 27、底栏三种排布 `PopupFooter.later` 10、真实表单 `PopupInboxView.later` 3、气泡 `arrival` 2）；`cargo test --bin AskHuman popup_inbox_geometry` 29 passed（+1）；`vue-tsc --noEmit`、`cargo fmt --check`、`vite build`、`node --test scripts/*.node-test.mjs`（32）通过。**既有测试有改动**：点 Sidebar 行现在先走 126ms 淡出，`PopupInboxView.test.ts` 4 个、`.arrival` 1 个、`.collapse` 1 个测试改成假定时器推进 126ms；`.links` 改用减少动态效果（它测的是表单与草稿）；另有两个测试（`retains mounted drafts…`、`.links`）在新行为下会「空过」——点击被淡出屏蔽但断言仍通过——补了 `isVisible` 断言让它们真正检验切换。变异检查：对页面与 Sidebar 做了 33 处单点变异（形态变更 13、正文切换 10、到达与控件 10），首轮 2 处存活——`shrink` 失败后 `look` 的回退（补「绘制后窗口失败」用例后抓到）、行旗标上多余的 `@click.stop`（旗标是行的兄弟节点，并不需要，删除）；另加 1 处「交叉淡化时长在变更后保留」被新增断言抓到。
- 未验证：**真实窗口**（计划里 P4 的目视与测量项）——像素级外观、动画观感、`measureMini` 在真实字体 / 布局下的高度、`look` 翻转与原生提交的同步、单请求小窗路径端到端、`collapsed` / `sidebarPreferred` 与 Rust 的端到端、Quick Look 预览在换题与小窗时的表现；非 macOS。这些需要按 §4.1 隔离安装后弹出真实窗口，会取走焦点，先问用户（C26）。
- 检查点后的调整（第 9 轮，C35–C37，用户已回答）：①底栏「稍后答 / 取消稍后答」后的跳转也带 C17 换题动画——`applyMark` 的 `switch` / `expand` 结果改为 `select(target, false, false, true)`（由小窗展开时窗口自己在动，仍不做正文切换；展开态下只翻标记的取消稍后答、标其它行都不涉及换题）。②`PopupInboxView.collapse` / `.mini` 两个测试文件改为整文件使用假定时器，`later` / `unlater` 辅助函数推进 126 + 174ms（改动后曾有 20 个测试因第二次点击被淡出期间忽略而失败，调整后全部恢复）；新增 4 个测试：跳转带淡出 / 滑入、淡出期间第二次稍后答被忽略、减少动态效果时即时、淡出期间目标终结则旧正文留下；把 `applyMark` 的动画参数去掉做单点变异，其中 3 个被抓到（减少动态效果那个本来就不该失败）。`PopupInboxView.later`（真实表单）用减少动态效果，未改。③验证：`vitest run` 63 files / **555 passed**，`vue-tsc --noEmit` 通过。④实机验证按用户安排推迟到 P5 之后与 P5 合并做（C36）。

### P5 激活与切走缩回（2026-10-17）

- 改动：
  - 新增 `swift/PopupActivation.swift`：`ah_popup_activation_observe(changed)`——主线程上（一次）注册 `NSApplication.didBecomeActive / didResignActive` 观察者，回调带 `Int32`，并返回当前是否激活。
  - 新增 `app/popup_activation.rs`：macOS 把回调转成 `popup-app-active`（bool）事件（`OnceLock<AppHandle>` 持有句柄）；`start()` 在主线程安装观察者并回读当前状态；非 macOS 用 popup 窗口的 `is_focused()` 作起始状态，`window_focus()` 由 `on_window_event` 的 `Focused(bool)` 发同名事件。新命令 `popup_app_active`（`commands.rs` / `invoke.rs`）：页面挂好监听后问一次，既启动事件又取起始状态。
  - `PopupInboxView.vue`：事件走 `appActivity()`——记录 `appActive`；离开（`false`）时起 200ms 宽限计时器（`LEAVE_GRACE_MS`，回来即取消、卸载时一并清掉），到期调 `collapseOnLeave()`：入串行队列，轮到它时先看用户是否已回来，再让模型 `collapseWindow` 判定（展开且没有主要请求才 `collapse`），`shrink()` 自带确认层早退，所以进行中的窗口变更会先做完、确认层下不缩、已是小窗不动作。起始状态：`popup_app_active` 的回答只在这之前没收到过事件、且是 boolean 时采用（未知 / 失败保持「在场」默认），而且它本身不触发缩回——只有事件才算「离开」。
- 与计划初稿 §2.5 的差异（补空白）：①计划里只有事件、页面默认「在场」；但弹窗可能非激活前置（应用根本不是激活的），C22 的「没人在看」需要起始状态，所以多了 `popup_app_active` 命令。②200ms 保护放在页面（计划本来就这么写）；macOS 用应用级通知后同进程窗口之间切换不产生事件，它主要服务非 macOS 的窗口焦点路径。③非 macOS 的几行（起始状态与 `Focused` 事件）一并写了，但本机无法编译 / 验证，记入 P7 与延期 gate。
- 补空白时做的决定（需要在检查点告知）：①确认层打开时用户离开 → 这次离开被丢弃，用户回来关掉确认层后窗口不补缩，等下一次离开；②起始状态晚到且为「不在场」时不缩回（那时不可能有 peek：peek 需要用户在场操作）；③「拖拽进行中不触发」留给 P6（已写进 P6 清单）。
- 验证（均通过）：`vitest run` 64 files / **571 passed**（555 → 新增 16：`PopupInboxView.leave.test.ts`——peek 在宽限期后缩回并保留标记与草稿、宽限期内回来不缩、宽限期按最后一次离开算、回来再离开会再缩、确认层下不缩、进行中的展开先完成再缩、排队期间用户回来则放弃、卸载后不缩、有主要请求不缩、已是小窗不动作、起始状态各种情形：只问一次、未知按在场、非激活弹窗上远端终结改为缩小、晚到的「不在场」不缩 peek、事件先于回答、回答失败不报错）；`vue-tsc --noEmit`、`cargo fmt --check` 通过；`cargo test --bin AskHuman popup_inbox_geometry` 29 passed（同时编译并链接了新 Swift 符号，`nm` 可见 `_ah_popup_activation_observe`）；`swiftc -typecheck` 通过。变异检查：对新逻辑做了 14 处单点变异（计时器不触发、无宽限、宽限加倍、回来不取消、排队中不看回来、不记录 `appActive`、起始状态盲目采用 / 覆盖事件、事件不标记已听到、失败不吞、起始状态也起计时器等），除 2 处等价变异（回来不清计时器：到期时队列里还会再看一次 `appActive`；连续两次离开保留第一个计时器：不存在连续两个 `false` 的真实序列）外都被抓到，其中「起始状态也起计时器」最初没被抓到，改成「peek 之后才收到晚到的回答」后抓到。
- 未验证：**真实的应用激活**——macOS 上点别的应用 → 200ms 后缩回、点回来取消、有主要请求时不缩、文件选择面板 / Quick Look 打开时不误判离开；起始状态在真实「非激活前置」弹窗上是否为 false；非 macOS 的编译与行为（P7 / 延期 gate）。按 C36 与 P4 的实机项合并做一次（已做，见下一节；剩余的未验证项也在那里）。

### P4 + P5 实机验证（2026-10-17）

- 方法（C36 的合并验证，用户第 10 轮同意开始，spec §1.11）：§4.1 的隔离安装（`INSTALL_DIR` + `ASKHUMAN_HOME`），同项目两个请求 A / B 加另一项目的 C。用 Computer Use 的 app 范围（后台，不动真实光标）点底栏 / 图标（无障碍 `press`）并截图；窗口位置与尺寸用 `CGWindowList` 每 50–160ms 采样；**真实的应用激活**靠向隔离 daemon 的 `daemon.sock` 发一条 `{"type":"focusRequest","request_id":"…"}`（菜单栏「待答」子菜单点击发的同一条消息，无需握手，daemon 转成 `FocusPopup` → 页面 `select(id, explicit)` → `setFocus()`），激活后立刻 `open` 回原来的前台应用。发现缺陷后改代码、重新做了隔离安装，下面的数字都来自修复后的二进制（除缺陷 1 的首次测量）。
- 结果（通过）：
  - 标记与跳转：底栏「稍后回答」→ 跳到下一条主要请求，旧行淡显并带「稍后答」标签；全部稍后答 → 缩成宽 240（= Sidebar 宽）的小窗，标题「待答 2」、强调色边框、分组头 + 单行标题。
  - 左上角锚定：小窗 / 展开窗口的左上角在整个会话里始终是同一点（`x=1606, y=399`），经历了缩小、托盘点选展开、切走缩回、到达展开、远端终结缩小、小窗改尺寸、图标展开、图标缩回至少 8 次形态变化，没有漂移（C30 / R14）。
  - 切走缩回（真实应用激活）：`FocusRequest` 后 QA 实例成为前台应用（0.58s），把焦点还给原应用后，窗口 940×826 → 约 200ms 后开始缩（+0.27s 930×816，+0.43s 368×298，+0.57s 262×200）→ +0.72s 到达 240×179，即 200ms 宽限 + 440ms 动画；两个标记都保留（行上仍是「取消稍后答」）。
  - 小窗里新请求到达：C 到达后窗口 240×179 → 457×380 → 888×778 → 940×826 展开到 C（选中、正文是 C），A / B 两行淡显；前台应用没变（弹窗本身是非激活前置）。
  - 没人在看时远端终结当前请求：应用不在场、剩下的全是稍后答 → C 终结后窗口缩回小窗，没有跳到 A / B（C22）。
  - 小窗里别的请求终结：B 终结 → 小窗只改尺寸（240×179 → 240×141），「待答 1」、一行标题，仍在原位。
  - peek 的手动缩回：小窗「展开」图标 → 展开（940×826）；Sidebar 标题栏的「缩回小窗」图标 → 回到 240×141（C27）。该轮里 Sidebar 已经出现过，所以单请求的 peek 仍显示 Sidebar（既有 D5 行为），没有走到顶栏图标那条路径（无 Sidebar 的 peek，只有 vitest 覆盖）。
  - 小窗标题栏里没有红绿灯，标题上移到原红绿灯位置（`sb-lift`）；点小窗里的行展开回原位（首轮用旧二进制验证，未再复测）。
- 实机发现并修复的缺陷（两处）：
  1. **小窗比内容高约 35pt**（首次测量 214pt，应为约 178pt）：`measureMini` 在「根仍是 expanded、加 `data-measure`」下量高度，而展开态为给「稍后答」标签让位的 `.inbox-row.secondary strong { padding-right: 90px }` 在测量态照样生效，每个稍后答行的标题因此折成两行（2 行 × 17.4pt ≈ 35pt），最终小窗里这个内边距并不存在、标题是一行。原型 CSS 有同一条规则与同一个测量函数（示例标题较短没暴露）。修复：该规则加 `:not([data-measure])`；计数胶囊（小窗里的 `padding: 1px 7px; font-size: 12px`）也纳入测量态，使测量版式与小窗一致；原型 CSS 同步修了那条规则。修复后实测 179pt（两行 + 分组头 + 底部 16pt）。新增 `InboxSidebar.measure.test.ts`：用真实样式表的计算样式比较「expanded + `data-measure`」与「collapsed」下标题 / 计数 / 元信息行的尺寸相关属性必须相同，并守住「两种形态本来就不同」，两处变异（去掉 `:not([data-measure])`、去掉胶囊的测量态）都被抓到。
  2. **托盘点选 / 小窗展开图标的 `setFocus()` 被 ACL 拒绝**（页面提示「Command plugin:window|set_focus not allowed by ACL」）：请求能选中，但窗口不激活、还弹错误提示。**这是既有缺陷**，自合并窗口 `799334f`（v0.14.0）起一直如此，根因是 `capabilities/default.json` 缺 `core:window:allow-set-focus`。它也是 P5 在真实环境成立的前提——否则托盘点选 / 展开图标不会激活应用，也就没有后面的「离开」。修复：补该权限；新增 `src/lib/windowPermissions.test.ts`：扫描非测试源码里所有 `getCurrentWindow()` / `getCurrentWebviewWindow()` 方法调用，要求 `default.json` 有对应的 `core:window:allow-*`（去掉 `set-focus` 后失败）。
- 观察（不是缺陷，但记下来）：
  - 弹窗显示与新请求到达**不会激活应用**（前台应用不变）；显式操作（托盘点选、小窗里的「展开」图标）会调 `setFocus()` 激活应用——是用户主动操作，符合预期。测试里用无障碍 `press` 点「展开」同样会激活应用，所以会取走前台；结束时终止 QA 实例，系统把前台还给了之前的应用。
  - 窗口被遮挡时 Computer Use 的截图会滞后（WebKit 对被遮挡窗口降频渲染，截图可能抓到动画起点或旧尺寸，几秒后再抓才是终态）；所以窗口形态以 `CGWindowList` 帧采样为准，动画是否顺滑只能由用户肉眼判断。
  - 合成的窗口事件无法触发 `startDragging`：对小窗标题栏做 `computer_drag`，窗口没动。
- 测试：`vitest run` 66 files / **577 passed**（571 → 新增 6：`InboxSidebar.measure` 2、`windowPermissions` 4）；`vue-tsc --noEmit` 通过；隔离安装重新编译通过（嵌入新前端与新的 capabilities）。
- 仍未验证：**用户肉眼**看动画是否顺滑（缩小 / 展开 / 切走缩回 / 换题淡入淡出）；真实鼠标点击是否把「非激活前置」的弹窗变成激活（按窗口服务器的惯例应当如此，没有观察到）；真实鼠标拖动小窗与拖动后再展开的位置；「宽限期内回来则取消缩回」「确认层 / 文件选择面板 / Quick Look 打开时不误判离开」的真实表现（只有 vitest）；无 Sidebar 的 peek 里顶栏缩回图标；材质 Solid / Glass；非 macOS（P7 / 延期 gate）；P6 的真 Finder 拖拽。

### P6 拖放（2026-10-17）

- 改动：
  - 新增 `inboxDrop.ts`（纯逻辑，与 Vue / Tauri 无关）：`DRAG` 常量（500 / 250ms、4px、600ms、56px、900px/s、3000ms 挂载上限）、`dropPoints`（原生落点 → 页面点，含 y 修正与非画布的候选顺序）、`dwellTarget`、`edgeSpeed`、`serialDrag`（串行 + 合并）、`acceptsFiles`、`sameHit`。新增 `useInboxDrop.ts`：窗口级控制器（命中检测 `hitAt`、瞄准 `aim`、停留 `dwell`、边缘滚动、拖走缩回、松手投放）。
  - `usePopupCore.ts`：`PopupScope` 加 `dropReceiver` / `dropAim`；有 `dropReceiver` 时 core 不注册自己的 `onDragDropEvent`（独立弹窗不变），把 `DropReceiver`（`currentQuestion` / `draggingOut` / `endDragOut` / `acceptDrop`）借给页面并在卸载时收回；`dropTargetQ` 在 inbox 下取页面的瞄准，新增 `dropCardLabel`（题卡胶囊）与 `dropHint`（整块答案区的框与胶囊）；`addDroppedPaths` 改为返回「真正加了几个」（重复 / 读不出的图片 / 选择题不计数）。
  - `PopupInboxView.vue`：注册唯一的窗口级监听；`deliverDrop`（到按需挂载的请求、`acceptDrop`、提示条、`revealDropped`）、`openForDrag`、`receiverFor`（隐藏挂载 + 3 秒上限）、`peekShrink` 提成共用函数（`collapseOnLeave` 现在在拖拽进行中直接放弃，接上 P5 留给 P6 的那条）；`finishRound` 清控制器；根元素 `data-dragging`；小窗描边在拖放中变实线加粗；提示条不拦截点击。`InboxSidebar.vue`：`can-drop` / `drop-target` 描边与 `dwelling` 进度条。`PopupView.vue` / `QuestionCards.vue` / `popup.css`：虚线框 + 胶囊、题卡胶囊。i18n：`dropToQuestion` / `dropToRequest` / `dropAdded` / `dropAddedQuestion` / `dropFailed`（中英文）。
- 与计划初稿 §2.7 的差异：①**命中检测放在页面里**（读 DOM：Sidebar 行 / 题卡 `.q-card[data-q-index]` / 预览区），core 只借出接收器并显示瞄准；计划写的是 core 暴露 `dropTargetAt(point)`。这样行、题卡、预览区只有一份命中逻辑，core 里原来的 `questionAtPoint` 只留给独立弹窗。②**给 `popup_inbox_layout` 的 `dragging` 参数补上了第一个调用方**（提交 `bb1a0f7`，初稿与 P6 首版都漏了）：原生的 `prepare` / `commit_frame` 在主键按下时会一直等，因为它默认「按着主键 = 用户在拖窗口边缘」；而从 Finder 拖文件时主键同样按着，由停留触发的展开 / 切换、拖走缩回、拖拽中到达的新请求都会等到松手才发生，整个功能在真窗口里等于失效。页面现在对「拖拽进行中」（`drop.dragging`）和「拖走缩回」（拖拽已离开窗口但主键还按着，用 `carrying` 标记，因为 `left()` 先清了 `dragging`）发起的事务传 `dragging: true`；松手后的显示、点击、切走缩回照旧不传。Rust 侧早就支持该参数（`Request.dragging`，且只对被准备的那次事务有效），无需改动；`prepare` 里 3 秒的「前端失联」看门狗遇到主键按着本来就会顺延，不受影响。这个缺陷只有真窗口才会暴露（vitest 里 `popup_inbox_layout` 是 mock），是写完 P6 复核 Rust 调用链时发现的。
- 补空白时做的决定（用户未明确过，需要在检查点告知）：
  1. **哪些请求是落点**：确认类（权限 / 结束确认，没有答案编辑区）与「只选不填」的请求不接收文件——行不画描边，松手落在它们身上被忽略（窗口、焦点、提示条都不变）；但在它们的行上停留仍会打开它们（和点击一样，要看得见）。
  2. **松手后的提示与失败**：提示条「已添加 N 个文件到「标题」」（多题请求加「的第 Q 题」，标题超过 28 个字符截断），2.6 秒后消失、不拦截点击；从没打开过的请求先隐藏挂载（最多等 3 秒），超时则弹错误「无法把文件添加到「标题」」，文件不会改投别处。
  3. **拖放中的视觉**：每个能收文件的行有淡描边，指针下的行加深加填充；等待中的行底边有进度条；小窗描边变实线加粗；展开窗口里单题 / 顺序布局给整块答案区画虚线框 + 胶囊「松手添加到当前请求 / 第 N 题」，纵向布局只给指针下的那张卡画胶囊。
  4. **拖走与缩回的细节**：只有「这次拖拽展开了小窗」且没有主要请求，拖走 600ms 后才缩回（宽限内拖回取消）；松手投放后不再缩回（用户在用它）；拖拽进行中的「切走」不触发缩回。
  5. **边缘滚动的范围**：小窗里的列表也会滚（请求多过小窗放得下的行数时才够得到）；指针在列表 / 正文上方（标题栏、导航栏）按「顶边」全速上滚；指针在 Sidebar 列内滚列表、其余滚正文；滚动让别的行 / 题卡滑到不动的指针下时，它就成为目标并按停留规则计时。
  6. **被忽略的拖拽**：窗口正在变形 / 换正文 / 有确认层时不接收也不计时；用户把自家附件拖出窗口、或指针在附件预览区上，都不算拖入（与原来一致）。
- 验证（均通过）：`vitest run` 68 files / **669 passed**（577 → 新增 92：`inboxDrop.test.ts` 19、核心接收器与瞄准 `usePopupCore.inbox.test.ts` +13、页面 `PopupInboxView.drop.test.ts` 60；既有 5 个页面测试文件补了 `@tauri-apps/api/webview` 的 mock，断言未改）；`vue-tsc --noEmit`、`vite build` 通过（本阶段无 Rust / Swift 改动，未重跑）。页面测试复刻原型 S17 / S18 的断言：停留展开（落在行上 / 标题上）、中途离开不展开、换行重新计时、同一行内移动不重计、行描边、松手落点（行 / 标题 / 非目标 / 空文件 / 按需挂载 / 挂载超时）、拖走缩回与取消、不缩自己打开的窗口与松手后的窗口、新请求到达时不缩、展开态的停留切换与不连环（4px 防连环）、松手到正文 / 另一行 / 当前题、对话层与自家附件拖出与预览区、题卡瞄准与落点（单题 / 顺序 / 纵向 / 两种布局下落在别的请求行上）、边缘滚动（Sidebar / 正文 / 小窗列表、越靠边越快、上限、在中间 / 离开后停、滑到指针下的行与卡成为目标）、画布下的 y 修正、拖拽期间切走不缩、一轮结束清状态、卸载清定时器。变异检查：对新代码做了 57 处单点变异（核心 15、控制器 18、页面 24，其中 5 处针对 `dragging` 的传递：不在拖拽中传、不为拖走缩回传、总是传、`carrying` 不复位、拖走缩回不设 `carrying`，全部被抓到），首轮 9 处存活——重复附件守卫、等窗口变化完成、变化后重新瞄准、松手位置落空时回退到上次瞄准、预览区松手、其他请求的瞄准隔离、一轮结束清状态，这 7 处补测试后抓到；另 2 处（拖回时「取消缩回计时」与计时器里的 `dragging` 检查）互为冗余，单独去掉任何一个都不影响行为，同时去掉才失败，保留双重保护。
- 未验证：**真实的 Finder 拖拽**——事件在小窗固定画布上的真实送达（`enter` / `over` / `drop`）、`dragging: true` 的事务在真实主键按着时确实不等（Rust 单测只覆盖「只对被准备的那次有效」）、y 修正与 x 原点修正（来自对 wry 源码的推导与单测，没有在真窗口上观察）、停留手感与进度条、边缘滚动的真实速度；所有新增 CSS 只通过了构建，没有被肉眼看过（胶囊在半透明底上的可读性、`.drop-hint` 与顶栏 / 输入区的层叠、`overflow: hidden` 的行里进度条的位置、小窗实线描边）；WKWebView 在非激活前置的窗口、应用不在前台时是否照常收到原生拖拽事件；松手后 `setFocus()` 与提示条的实际观感；拖走缩回与 440ms 缩小动画的衔接；非 macOS 的落点单位候选（P7）。按 C40 并入 P4 / P5 剩余的「仍未验证」一起请用户亲手验证。
- 后续（2026-10-17 夜，另一个会话，提交 `0d55c68` / `e717891`）：在 P6 之上加了拖入的可见反馈与 `askhuman_drop` 诊断日志（spec §6.9，新增 `popup_drop.rs`、`dropFeedback.ts`，改动 `PopupInboxView.vue` 的 `deliverDrop` 等）。上面补空白决定 1、2 里「静默忽略」「错误提示」的部分被它修订（见 spec C41 / C42 的后续说明）；它的实现记录与待办在 spec §6.9 和 `docs/PROGRESS.md`，本计划不重复。

### P4 + P5 + P6 实机会话（2026-10-17）

- 方法（C40，spec §1.13）：§4.1 的隔离安装（`INSTALL_DIR=/tmp/askhuman-collapse-qa/bin`，从含 `bb1a0f7` 的提交编译），`ASKHUMAN_HOME` 隔离 + 独立 `HOME`（见下面的 hook 事故），请求超时 4 小时；三个请求 A / B / C 分属三个项目（`proj-x` / `proj-y` / `proj-z`），C 是三题加一长段说明文字（正文可滚动），顺序布局（`verticalQuestions: false`）。**这一次由用户亲手操作，Agent 不碰这些窗口**（不用 Computer Use，C26 / C40），清单分动画、P5 行为、P6 触发与投放、边缘滚动与拖出窗口、视觉五组，加一个开放问题。
- 结果（用户回答，spec §1.13）：动画「顺滑，没有卡顿或跳变」；P5 行为、P6 的触发与投放、边缘滚动与拖出窗口都「符合预期」；视觉「清晰好看，保持」；一个 UI 缺陷（下一条）；纵向布局要再试一轮（3 步）。
- **UI 缺陷：小窗里悬停某一行时，「取消稍后答」按钮透明，盖在标题上，文字穿过按钮**（用户截图）。根因：小窗里按钮的两条规则（`.inbox-root[data-view="collapsed"] .row-flag.flag-archive.is-secondary`，权重 0,5,0）权重高于展开态的悬停底色规则（`.inbox-entry:hover .flag-archive.is-secondary`，0,4,0），小窗里悬停时底色永远是 `transparent`；小窗又没有给按钮留位置（展开态靠标题右侧 90px 内边距，小窗不能加，否则 `InboxSidebar.measure.test.ts` 守的「测量 pass 与小窗同尺寸」不成立）。修复（`97d2eae`，只动 CSS）：悬停 / 键盘聚焦的底色改为不透明（`color-mix(… var(--accent) 14%, var(--surface-overlay))`，`tokens.css` 的约定：浮层用实底，半透明的 `--card-bg` / `--bg-elevated` 会让下面的文字透出来）；在小窗里的按钮前加一条 16px 的 `::before` 渐变，标题在按钮前淡出而不是被截断；小窗的悬停 / `:focus-visible` 规则里把底色重写一遍（因为它们压过展开态的规则）。验证：用无头 Chrome 在临时的 vite 页面上复现（强制 `:hover`，亮 / 暗两种主题）并对比修复前后的截图，展开态外观不变；新增 `InboxSidebar.undo.test.ts`（4 个测试：手写的最小层叠解析读真实样式表，因为 jsdom 的 `getComputedStyle` 不计选择器权重；小窗里静止时按钮不可见、悬停与键盘聚焦时有实底、展开态的常驻标签与悬停外观不变、小窗静止时无底——最后一条防止夹具对什么都成立）；对 CSS 做 6 处变异（去掉小窗悬停底色、去掉聚焦变体、悬停底色改回半透明、小窗常驻按钮、小窗静止态带底色、展开态常驻标签去底色）全部被抓到。
- 事故 1（已处理）：**隔离实例的环境变量泄漏进 Agent 的持久 shell**（`ASKHUMAN_HOME` 等被 `export` 过），之后 Agent 自己的 `AskHuman` 提问被路由到隔离 daemon，指纹不一致触发排空，提问排了约 28 分钟；隔离窗口按 30 分钟超时退出，用户没看到第一份清单。处理：清掉变量与隔离进程，重新布置（请求超时改为 4 小时）并补发清单；之后只在单条命令里内联传环境变量，真实调用前核对 `env`。
- 事故 2（已处理；记为待办 #1 后，另一个会话在同一天修了它，提交 `2224f82`，待办已移除）：**隔离实例的 daemon 改写了用户真实的 Cursor / Codex Stop hook**。`ASKHUMAN_HOME` 只隔离配置 / socket / daemon 标签，`paths::home()` 仍是真实 `$HOME`；daemon 启动时 `agent_stop::migrate_outdated()` 发现 hook 命令与自己的 `current_exe()` 不一致就改写，指向 `/tmp/askhuman-collapse-qa/bin/AskHuman`（登录项有 dev-instance 保护，hook 迁移没有）。从 18:02 起每次启动都改了一遍；发现后按备份恢复为 npm 全局包路径并校验，此后隔离实例用 `HOME=/tmp/askhuman-collapse-qa/fakehome` 运行。不在本任务里修，已用 `AskHuman todo` 记录。
- 顺带：本地 `main` 在 21:51 被另一个会话 rebase 到最新 `origin/main`（并入 PR #16），本任务的提交哈希随之全部改变，spec / 本计划里的引用已按提交标题对应更新（`e4c1a7d` P4、`faff232` P5、`b0c9bbd` ACL、`37bfb86` P6、`bb1a0f7` `dragging`、`97d2eae` 本节的 UI 修复）。
- 测试：`vitest run` 73 files / **694 passed**（含其他会话并入的测试；本任务新增 `InboxSidebar.undo` 4 个）；`vue-tsc --noEmit`、`vite build` 通过（本次只改 CSS 与测试，未重编 Rust）。
- **纵向布局一轮（第 14 轮，spec §1.13）**：隔离实例换成含 UI 修复的新二进制，`verticalQuestions: true`。因为主工作树里有另一个会话没提交的改动（拖入文件的失败提示与诊断，`popup_drop.rs` 等），**这次不从主工作树编译**，而是 `git worktree add --detach /tmp/askhuman-collapse-qa/src HEAD`，在那里 `INSTALL_DIR=/tmp/askhuman-collapse-qa/bin-next ./scripts/install.sh`（冷编译 1m22s），再换进 `bin/`。旧实例先 `pkill`（TERM 后 KILL）并 `launchctl bootout` 干净，真实的 `~/.cursor/hooks.json`、`~/.codex/hooks.json` 核对过没有隔离实例的路径。用户的屏幕在实例启动时是**锁定**的（`CGSSessionScreenIsLocked = 1`；锁屏时页面渲染被暂停，只看到一个 10×8 的占位窗口，没法在 Agent 一侧确认窗口弹出），所以清单的第一问是「窗口正常吗」，约 70 分钟后用户回来，全部通过：窗口正常；落在指针下的卡、卡外落当前题加边缘滚动够到第 3 题、落在 Sidebar 里该请求的行，都「符合预期」；UI 修复在真窗口里看过，没问题。这一轮同时回答了本节「仍未验证」里的「纵向布局的按卡落点只被自动测试验证过」。
- 仍未验证：非 macOS（P7 / 延期 gate）；材质 Solid / Glass 下的小窗按钮观感。

### P7 Windows / Linux 即时路径（2026-10-17 夜）

- 盘点：P2 / P5 / P6 已经带了非 macOS 的几处代码——`popup_canvas` 的 `prepare` / `apply` / `animate` / `resume` 非 macOS 分支（立即 `set_size` + `set_position`；缩小先把最小尺寸降到 1×1、变形后 `set_resizable(false)`，展开先 `set_resizable(true)`，`resume` 恢复最小尺寸）、`popup_activation` 里以窗口 `Focused` 事件作「在场」信号的路径、`dropPoints` 的非 macOS 落点单位候选。前两处整段被 `#[cfg(not(target_os = "macos"))]` 隔开，**macOS 上连类型检查都过不了它们**，写错了本机没人知道。Windows 目标的交叉 `cargo check` 在本机走不通（既有记录：`docs/plans/agent-lifecycle-integration-binding.md`，`ring` 缺 Windows C SDK 而停止），所以用「把非 macOS 路径提出 `cfg`、让 macOS 也编译它」代替。
- 做了什么：
  1. **Rust**：`popup_canvas.rs` 新增无 `cfg` 的 `mod instant`（`apply` / `animate` / `resume` / `apply_frame`，只用跨平台的 `WebviewWindow` 调用；macOS 上靠模块上的 `#[cfg_attr(target_os = "macos", allow(dead_code))]` 压掉死代码警告），公开的四个函数仍按平台分发，macOS 行为不变；Wayland 的「位置被拒不算错」从 `#[cfg(target_os = "linux")]` 改成 `cfg!(target_os = "linux") && WAYLAND_DISPLAY`；`popup_activation::window_focus` 与起始状态读取同样无条件编译；`app/mod.rs` 的 `Focused` 分支改成 `if !cfg!(target_os = "macos")`。
  2. **页面布局的一个真问题**（只在非 macOS 出现；macOS 的固定画布全部绝对定位）：非原生画布时窗口是 CSS grid，`inbox-expanded` 无条件给出「列表 | 6px | 正文」三列；但单请求的列表不是 Sidebar 列（缩小前为了测量而隐藏摆放的那一遍、小窗本身、展开后淡出的那一遍），占着一列就把正文挤窄。按代码推断：缩小前异步的 `popup_inbox_layout` 往返间隙里会绘制出正文被挤成「窗口 − 列表 − 6px」宽的几帧，展开后列表移除时正文又跳回整宽（没有在实机上观察到，只推断自代码）。修复（`PopupInboxView.vue`，3 条 CSS）：`data-sidebar="off"` 且非 `inbox-native` 时网格只剩正文一列，列表改为绝对定位在左边缘（与小窗的列表位置一致），分隔条不占位。
  3. **测试辅助**：把 `InboxSidebar.undo.test.ts` 里手写的最小层叠解析器提成 `src/views/popup/cssCascade.testkit.ts`（`styleBlock` / `createCascade`），新增的 `PopupInboxView.columns.test.ts` 与原测试共用。
  4. **文档**：实机补验清单记进 `docs/PROGRESS.md` 的「待外部验收：Windows / Linux 合并窗口的稍后回答与小窗」，并记为项目待办 #1（计划 §1 / §3 P7 本来就把实机验证延期，用户第 8 轮通过）。
- 验证：
  - Rust：`cargo check --all-targets` 无警告；故意把 `instant::animate` 里的 `set_resizable` 改错，macOS 上立刻编译报错（证明这段现在确实被编译到了），已还原；`cargo test --bin AskHuman popup_` 107 passed；`cargo fmt --check` 通过；`cargo clippy --all-targets -- -D warnings` 通过（本机 1.88 对 `build.rs:68` 报一条既有的 `clippy::uninlined_format_args`，与本阶段无关，用 `-A` 屏蔽后其余全部通过）。
  - 前端：`vitest run` 76 files / **771 passed**；`vue-tsc --noEmit`、`vite build` 通过（新规则在压缩后的 CSS 里仍在）。`PopupInboxView.columns.test.ts` 4 个测试：真 Sidebar 仍是「列表 | 分隔条 | 正文」三列（展开 / 小窗）、单请求的列表不占列且绝对定位在 left 0 / 宽 `--inbox-sidebar` 并隐藏分隔条（展开 / 小窗）、没有列表时只有正文一列、macOS 画布（`inbox-native`）不受影响（仍是块布局、列表仍在 `--inbox-visible-left`）。对 CSS 做 10 处变异，9 处被抓到；存活的 1 处（去掉 `.inbox-root` 基础规则里的单列）是等价变异：新规则同样匹配页面总会带的 `data-sidebar="off"`，没有列表时两条规则给出同一个值。重跑 `InboxSidebar.undo` 原来的 6 处变异，换成共享解析器后仍全部被抓到。
  - 真实渲染引擎里量过：用真实样式表静态拼页面，headless Chromium（≈ WebView2）修复前后各量一次——单请求列表在展开窗口（900 宽）里正文宽度 654 → 900、左边缘 246 → 0；小窗（Chrome 视口最小 500 宽）正文 254 → 500；列表始终 left 0 / 宽 240，分隔条 `display: none`；真 Sidebar 列与没有列表这两种状态前后逐项一致。
- 未验证（即延期 gate 的内容，不据此宣称这两个平台已验收）：这个功能的非 macOS 代码（含本阶段）没有在 Windows / Linux 上编译或运行过，也没有被 CI 编译过（提交都还没推送）——窗口变形的实际效果（`set_resizable(false)` 对 Windows 边框 / 客户区的影响、GTK 上「先改尺寸再锁定」的顺序、Wayland 不接受应用设定位置）、DPI / 多屏、WebView2 / WebKitGTK 里 grid 与 `position: absolute` 的表现（上面只量了 Chromium）、`Focused` 事件作「在场」信号（系统文件对话框会让 popup 失焦）、拖放落点单位候选。清单见 `docs/PROGRESS.md`。

### P8 文案与文档（2026-10-17 夜）

- 文案（i18n）：没有新增 key。逐项核对计划 §2.8：`popup.inbox` 下的 `later` / `laterLabel` / `undoLater` / `laterHint` / `undoLaterHint` / `expand` / `shrink` / `dropToQuestion` / `dropToRequest` / `dropAdded` / `dropAddedQuestion`（以及另一个会话为拖入反馈加的 `dropFailed` 等）中英都在，英文是用户第 8 轮确认的措辞（Answer later / Later / Undo answer later / Expand / Shrink to small window / Release to add to question {n} / Added {n} file(s) to “{title}”…）；`pending` 中文已是「待答」，英文保持 “Waiting for answers”。本功能的 UI 源码（`InboxSidebar.vue`、`PopupInboxView.vue`、`PopupFooter.vue`、`PopupNavbar.vue`、`useInboxDrop.ts`、`inboxDrop.ts`、`inboxMini.ts`、`inboxCollapse.ts`、`dropFeedback.ts`）里除注释外没有写死的中文，唯一例外是既有的开发用按钮（`?layoutReview=1`，不属于本功能）。
- 文档：`docs/overview-popup-ui.md` 新增「稍后回答与小窗」一节（语义、模型与流程、窗口事务、激活、拖入文件、测试与原型的实现地图，指向 spec / 本计划 / PROGRESS）；主 `docs/overview.md` 只补了模块地图里的三行（`PopupInboxView` 的职责、`swift/PopupCollapse.swift`、`popup_activation.rs` + `swift/PopupActivation.swift`），全局心智模型与不变量没有变；spec 顶部状态改为「已实现」，并写明 P9 与 Windows / Linux 实机仍未做。
- `docs/PROGRESS.md`：与上面 §3 P8 的原文不同——**没有删除**「进行中」一节，而是把它改写成「只剩 P9」。P9 还要等用户确认「覆盖」的含义，是真实的未完成事项；Windows / Linux 的 gate 已在 P7 单独成节。P9 完成后再删除该节。
- 验证：本阶段只改了 `docs/` 下的文档，没有代码改动；自动测试沿用 P7 的结果。
