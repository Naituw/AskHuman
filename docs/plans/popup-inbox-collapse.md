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
  - `select()` 中点击切换：加 C17 的正文淡出 / 滑入，复用 `answerTransition`（126 / 174ms）的曲线；减少动态效果时直接切。
- 请求 `visited`（蓝点 / ripple）语义不变：稍后答请求未查看时蓝点与 ripple 继续，真正显示正文才清除（R7）。

### 2.2 窗口事务（Rust + Swift，macOS）

- **Swift**：`swift/PopupCollapse.swift` 里面向产品的入口只留 `ah_collapse_animate`（逐帧 display-link、`cubic-bezier(.2,0,0,1)`、宽高同时、左上角锚定、红绿灯淡出、可被打断并从当前帧继续、动画期间最小尺寸降为 1×1 并在小窗期间去掉 `.resizable`——原型已验证）。其余的——`prepare` / `resume`（只是对产品里已有的 `ah_popup_canvas_prepare` / `resume` 的薄包装，产品直接用后者）、`front`（`popup_transition::front` 已有）、`info`、`capture` / `diff` / `backdrop` / `control` / `burst_*` / `forbid_activation` / `activate_other`——都只给原型验证用，挪到 `swift/prototype/PopupCollapseVerify.swift`，由 `build.rs` 仅在 Cargo feature `collapse-prototype` 打开时编译进静态库；`popup_collapse_demo` 示例加 `required-features`，`scripts/popup-collapse-demo.sh` 传该 feature。产品二进制里不再有测试钩子。（计划初稿曾把 `prepare / resume / front / info` 也列为产品入口，P1 核对后更正：它们不被产品路径使用，见 §8 P1。）
- **Rust**：缩小 / 展开沿用现有三步协议 `prepare → commit_frame → finish`，不另造第二套所有权：`popup_inbox_layout` 增加「形态 + 小窗目标尺寸」入参，`popup_inbox_commit` 增加「动画参数」（时长 / 曲线 / 红绿灯），`commit_frame` 在 macOS 上用 `ah_collapse_animate` 替代瞬时的 `popup_canvas::apply`，`finish` 照常 `resume` 并发布一个带 `collapsed` 标记的 `Allocation`。同一个 `pending` 门闩与 revision 保证它与 Sidebar / 预览 / 外缘缩放事务互斥。P2 开工前先读 `popup_canvas::apply` 评估；若复用会让既有事务变复杂，退回独立的 `app/popup_collapse.rs` 命令，仍共用 `pending` 门闩。
- 小窗尺寸来自页面在小窗布局下测量的真实 DOM（原型的做法），由页面传给 Rust，**不写死**，也**绝不写入尺寸偏好**（R10：`persist_popup_size`、`resized()` 在小窗 / 动画中一律返回 `None`）。展开恢复缩小前记下的稳定展开尺寸（被打断时保留，不记动画中途帧）；放不下则平移（C5，沿用 `allocate` 与工作区夹取）。
- 页面等待首帧用 `paint()`：`requestAnimationFrame` 加定时器上限（窗口被遮挡 / 非激活时 rAF 可能暂停），不得只靠 rAF。
- 小窗期间右侧原生预览（PDF / Quick Look）随正文被裁切隐藏；缩小前先让它隐藏，展开后由现有 `nativePreviewSync` 恢复位置与阅读状态，避免 NSView 漏在小窗外。

### 2.3 Geometry 与「新一轮不漂移」（C30、R14）

- 现状：`anchor` = 正文左上角，窗口 x = `anchor.x − left_span`（D5：Sidebar 出现时窗口向左长、正文不动）；`commit_frame` / `resized` / `moved` 都以 `帧 x + left_span` 回写 `anchor`；`reset()`（`popup_inbox_idle`）把窗口放回 `anchor.x` 并清空状态。其中 `moved()` 用的是 `c.sidebar` 标志与已发布的 `sidebar_width`，**不认识小窗**，必须加分支。
- 小窗的左缘与缩小前一致（左上角锚定，Sidebar 列即小窗的左列）。新增 `Geometry.collapsed: Option<Collapsed { left_span, restore_frame }>`，`left_span` 取缩小那一刻的值（单请求时为 0）。小窗期间 `moved()` 用 `窗口 x + collapsed.left_span` 回写 `anchor`（C5：用户拖过的位置本轮内记住、展开时以小窗为锚）。
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

**P4 UI**：先抽 `InboxSidebar.vue`（纯重构，单独提交，既有测试不改断言）；再做 Sidebar 行归档盒 / 标签 / 悬停按钮、底栏按钮三种排布、peek 缩回图标（含无 Sidebar 版）、小窗列表外观与测量、交叉淡化与 mini-ring、C17 正文切换动画、`UnreadRipple` / 蓝点 / 到达气泡与小窗的衔接、减少动态效果。验收：组件测试；按 §4.1 隔离安装后用真实多请求（并发 `AskHuman`）目测与测量（§4）。

**P5 激活与切走缩回**：Swift 观察者 + `popup-app-active` + 页面规则（§2.5）；非 macOS 的 `Focused` 路径。验收：vitest 驱动事件序列；实机：从小窗展开后点别的应用 → 缩回；有主要请求时不缩。

**P6 拖放**：`useInboxDrop.ts`、scope 扩展、`usePopupCore` 在 inbox 下让位、高亮 / 进度条 / 边缘滚动 / 按题落点、y 修正、串行合并（§2.7）。验收：vitest 复刻原型 S17 / S18 的断言（停留展开、不连环切换、落点表、边缘滚动、纵向 / 顺序、单题）；实机让用户真拖一次。

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

## 8. 实施与验证记录

每阶段一条：做了什么、验证命令与结果、未验证项。

### P0 准备（2026-10-17）

- 入库：`a894605` spec + 本计划，`3c6962e` 原型（`chore(prototype)`，不进发布说明）。
- 基线（正式代码尚未改动）：`pnpm exec vitest run` 57 files / **430 passed**（含原型 41）；`node --test scripts/*.node-test.mjs` 32 passed；`vue-tsc --noEmit`、`cargo fmt --check` 通过；Rust 测试在 bin 目标里（无 lib 目标），`cargo test --manifest-path src-tauri/Cargo.toml --bin AskHuman popup`：**65 passed**（bin 共 1337 个），其中 `popup_inbox_geometry` 8 passed。原型 `--auto` 367 项 / `--auto-controls` 18 项沿用上一轮结果（原型源码自那次通过后未再改动），P1 动 Swift 后重跑。
- 隔离安装试跑（§4.1）：`INSTALL_DIR=/tmp/askhuman-collapse-qa/bin ./scripts/install.sh` 编译 1m12s、签名通过；以 `ASKHUMAN_HOME=/tmp/askhuman-collapse-qa/home ASKHUMAN_NO_KEYCHAIN=1` 从 `/tmp` 运行，起了独立 daemon（socket 在临时目录），日常 daemon（pid 53110、socket `~/.askhuman/daemon.sock`）不受影响；`daemon stop` 后会留下一个 `--gui-host` 常驻进程，需要 `pkill` 清理（已写进 §4.1）。
- 发现：日常 `AskHuman` 是 npm 全局包，不是 `~/.local/bin` 里的（见 §4.1、P9、§7）；P9 前要向用户确认「覆盖」的具体含义。

### P1 Swift 驱动产品化（2026-10-17）

- 改动：`swift/PopupCollapse.swift` 只剩产品驱动 `ah_collapse_animate`（563 行验证代码移走）；新增 `swift/prototype/PopupCollapseVerify.swift`（`prepare` / `resume` / `front` / `info` / `forbid_activation` / `activate_other` / `capture` / `diff` / `backdrop` / `control` / `burst_*`，原样搬移）；`build.rs` 仅在 `CARGO_FEATURE_COLLAPSE_PROTOTYPE` 下收集 `swift/prototype/*.swift`；`Cargo.toml` 加 feature `collapse-prototype`，示例 `popup_collapse_demo` 加 `required-features`；`scripts/popup-collapse-demo.sh` 传该 feature；`docs/development.md` 同步。
- 更正计划初稿：`prepare` / `resume` 只是产品已有的 `ah_popup_canvas_*` 的薄包装，`front` 与 `popup_transition::front` 重复，`info` 是测试读数，产品路径都不用，所以产品只留 `ah_collapse_animate`（§2.2 已改）。
- 验证（均通过）：`swiftc -typecheck` 产品源码集与「产品 + prototype」源码集各一次（仅有既有的弃用 / async 锁警告）；无 feature 的产品配置 `cargo check` 后，该配置的 `libahspeech.a` 只导出 `ah_collapse_animate` 与 `ah_popup_canvas_*`（对照：带 feature 的库另有 `prepare` / `resume` / `front` / `info` / `capture` / `diff` / `backdrop` / `control` / `burst_*` / `forbid_activation` / `activate_other`）；`cargo test --bin AskHuman popup` 65 passed（与基线相同，说明产品配置能链接）；`cargo fmt --check`、`vue-tsc --noEmit` 通过。
- 原型回归（带 feature、静默、有焦点守卫）：`--auto` **367 PASS / 0 FAIL**、`--auto-controls` **18 PASS / 0 FAIL**，帧统计与此前同量级（每次切换丢帧 0–4，位置误差 0.50px）；守卫轮询 `lsappinfo front`，全程前台始终是原来的应用（437 / 69 次采样，未被原型抢走），结束后无 `PopupCollapseDemo` / `vite --port 5199` 残留。
- 副作用：`cargo clippy --all-targets`（Linux CI）不再编译 `popup_collapse_demo`（`required-features`，该示例只在带 feature 时才构建）；其余示例照旧检查。未验证：Linux CI 本身（本机不跑）。
