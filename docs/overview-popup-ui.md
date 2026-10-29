# Popup UI 概览

> 本文是 `docs/overview.md` 的专题补充，记录 Popup 当前实现地图；具体功能的需求与设计仍以对应 spec 为准。

## 窗口与附件交互

Popup 提问附件和回复图片采用同窗右侧预览，具体决策见 `docs/specs/popup-attachment-preview-panel.md`。
默认窗口紧凑，点击附件才扩展；切换不重建主区，作答、选项和当前题目状态独立保留。

- 原生窗口下限仍为 420×480。`app/popup_preview.rs` 和 `popup_preview_geometry.rs` 管理固定右侧、必要时整窗左移、受限横向分区与关闭恢复；前端 prepare 后等待绘制，再提交原生几何并异步对账。关闭恢复展开前位置，用户移动过则保持新位置；临时缩窄和展开总宽不写入主区尺寸偏好。`persist_popup_size` 继续过滤预热、收尾、最大化和迟到事件，并读取最新 rememberSize。
- 预览默认 700 宽，`channels.popup.previewWidth` 单独记忆用户的外缘 / 正常分隔线调整，遵守同一个 rememberSize 开关；空间限制或 DPI 变化造成的临时宽度不覆盖偏好。
- `useAttachments.ts` 管理列表焦点、点击激活和原文件动作；`useAttachmentPreview.ts` 管理几何意图与布局订阅；`useAttachmentContent.ts` 管理读取代次、WebView 共享的 64 MiB / 8,000 万显示像素 / 128 项内容缓存及每附件的模式、滚动和图片缩放。收起或切换请求保留各自阅读状态，终态销毁对应上下文。
- 回复缩略图由 `ComposerAttachments.vue` 的可选预览按钮接入，Popup 与历史详情启用（历史在 `HistoryDetail.vue` 里用同结构的按钮）。`useReplyImagePreview.ts` 直接显示既有回复图片数据（历史只存路径，经可选的 `resolve` 先换成显示 URL），按图片身份保存尺寸、缩放与平移；导航只覆盖所点击题目的回复图片，与提问附件互相切换但不重复展开窗口。删除当前图切到相邻图，最后一张删除后收起；没有原路径的回复图片隐藏原文件操作，不生成临时文件。
- 提问附件的双击只执行第一次单击的预览切换，忽略后续 click，不打开外部应用；单击立即响应。Markdown 的主按钮依阅读模式选择浏览器快照或原文件，列表回车继续打开原文件。`popup_preview_open_browser` 校验请求 / 索引，`attachment_browser.rs` 重新有界读取、生成快照并按 HTTPS 关联选择默认浏览器；`attachment_html.rs` 与 Quick Look 共用文档样式及 24h 临时目录。菜单动作固定且带请求归属，失败保留明确的原文件入口。规则见 `docs/specs/markdown-browser-open.md`。
- `attachment_preview.rs` 从显式 request ID 对应的待答附件按索引读取，不接受任意路径；限制字节、文本行数、图片像素与动画帧预算，后台执行并丢弃失效代次。列表缩略图另有读取与缓存预算。Markdown 与普通文本支持 UTF-8、带 BOM 的 UTF-16，无法可靠识别或超限时保留打开和文件管理器定位入口。
- 附件大图由 `attachment_preview::load_url` 注册到 `local_image.rs` 的窗口 scope，普通格式读原文件、转换格式读有界临时 PNG；前端只收 token 和成本元数据。缓存与当前显示共同管理引用，过期 / 终态 / 窗口关闭撤销，最后引用删除临时 PNG；历史继续保存原路径。列表小缩略图 Base64 保留，详见 `docs/plans/attachment-image-urls.md`。
- `AttachmentPreviewPanel.vue` 提供单行标题栏、带边框的原文切换、固定右侧操作区。Markdown 使用 `attachment_markdown.rs` 的静态受限片段；diff 使用 `attachment_diff.rs` 的共享解析模型，由 `AttachmentDiffPreview.vue` 虚拟显示行；代码与原文作为文本展示。`AttachmentImagePreview.vue` 保留动画原字节，以实际溢出决定文件拖出或平移查看，SVG 只通过隔离 img 显示。
- macOS 的 `macos_attachment_preview.rs` / `swift/AttachmentPreview.swift` 经已有 Swift 构建桥复用 Image I/O、PDFKit 与 QuickLookUI。系统图片生成有界 PNG；PDF 后台创建、保存页坐标与缩放；其他系统文档 / 媒体使用嵌入的 `QLPreviewView`，静态文档视图最多两个，媒体切换后卸载。`useNativeAttachmentPreview.ts` 同步正文矩形、遮罩与代次；命令根据请求索引和有界读取许可验证目标，原生焦点快捷键回到 Popup 业务处理。多图系统图片说明数量和完整原文件入口。范围与系统差异见规格 §5.4。
- 预览能力与 CLI / IM 的 `isImage` 分类分开：新增系统图片可在 Popup 列表显示缩略图，但发送分类沿用原有七种扩展名。ICO / TGA / PNM 只启用现有 image 的轻量 feature，无新增 codec crate；macOS 系统图片能力不承诺在 Windows / Linux 可用。
- 窗口拖拽使用主区和预览标题栏、图片周围 / 控件栏及状态背景的显式 `data-tauri-drag-region`，不覆盖按钮、正文、图片本身、滚动条或原生预览；文件拖出共用 `startDrag` 并使用原路径与有效 PNG 图标，开始前先经 `attachments_exist` 确认原文件还在，已不存在则胶囊标出、不开始拖拽并在列表下提示（弹窗与历史一致，规格 `file-attachments.md` §6）；合并窗口里拖出的自家附件可以投给窗口内**别的**请求（沿用停留切换与松手落点，来源请求自己不收，R17）。原生拖入的预览区 / 分隔线落点不进入回复附件。拖入没加成功的各种情形（没有路径、读取失败并区分没权限、落在预览区、请求不收附件、窗口忙、全部重复、部分成功）都有提示，每次拖拽的结果写进 `daemon.log` 的 `askhuman_drop` 行（不含路径），见 [`specs/popup-inbox-collapse.md`](specs/popup-inbox-collapse.md) §6.9。附件列表空格和方向键、标题栏左右键与正文滚动分别路由；输入法、查找、语音及既有发送 / 取消优先级保留。
- Popup 更多与右键共用 `popup_preview_menu`。macOS 保留完整原生菜单并将快速查看路由到当前 Popup；Windows / Linux 提供公共文件动作，定位失败显示错误。待办及其他入口继续用已有 Quick Look；它们与 Popup 共用 diff 解析和 Markdown 静态渲染，范围见 `docs/specs/diff-attachment-preview.md`。
- 历史详情共用同一块面板（规格 §10，实现见 `docs/plans/history-attachment-preview.md`）。后端 `app/attachment_owner.rs` 是预览命令的授权入口：所有者 ID 为 Popup 请求 ID 或 `history:<条目 ID>`，路径只来自后端（历史取自 `history.jsonl`，`history::attachment_files` 带文件指纹缓存），窗口标签与所有者种类必须一致。`popup_preview.rs` 每个窗口一个控制器，`Policy` 区分布局事件（`popup-preview-layout` / `history-preview-layout`）、主区下限（420 / 600）与是否记忆尺寸（历史不写任何配置，只读 Popup 的预览宽度偏好）；读取代次、原生许可与 macOS 原生预览按窗口各一份，原生回调带所有者 ID，历史窗口销毁时清空自己的那份。前端 `views/history/useHistoryPreview.ts` 是历史唯一的预览控制器（换条目先收起再切换），`popup/attachmentPanelContext.ts` 把面板与 Popup 上下文解耦；`HistoryView.vue` 编排主区 / 分隔线 / 面板，分栏回落到窗口内且主区窄于 560 时折叠条目列表。历史窗口 v1 不提供面板内查找。
- 提问正文与各题、历史正文共用 `MarkdownContent.vue`：本地图片按可见区域注册 `local_image.rs` 的窗口 / 文档 scope，再通过 `convertFileSrc(token, "askhuman-image")` 加载原文件二进制。项目相对路径使用请求项目目录，切消息或卸载时撤销 scope。`image_resource.rs` 与附件共用有界读取、尺寸和动画检查；大图、缺失或解码失败保留说明与原图入口，点击图片由系统默认应用打开。实现、预算和跨平台证据见 `docs/plans/popup-markdown-images.md` 与 `docs/investigations/base64-usage-audit.md`。

## 多请求统一作答窗口

默认合并模式的 Daemon 派发使用 `daemon/popup_inbox.rs` / `daemon/runtime/inbox.rs` 的单一宿主，
以一次性 token 和 generation 认证共享连接；普通题、权限和 Stop 共用 `--popup-host`。
请求账本及终态仍属于 Daemon，IPC submission ACK 按 ID 区分合法提交、渠道抢答和失败。
同一终态只移除一个请求；连接故障有界重建 pending 请求，明确提示内存草稿丢失。

`PopupInboxView.vue` 管理按完整项目路径分组的导航、10pt 未查看蓝点、草稿提示、同项目
下一条和固定 ID 快照的统一关闭。条目保留问题摘要主标题，下面仅一行「Agent / 会话图标 +
任务名」，以紧凑的来源关系表达可信 session 标题；权限 / Stop 在摘要前标记类型。
Daemon 首屏可传已有 Agent 标题，每次展示时异步读取当前会话名并按 request ID 补齐，
原生名称优先于缓存，读取失败保留缓存；不持续订阅改名或轮询标题，未识别时隐藏。
只有一条时用普通窗口；本轮曾展开的 Sidebar 保持到
空队列。新到达不切当前正文。首次查看或发送期间的后继准备阶段挂载 `PopupView` / `PopupContext`，后来仅隐藏，
因此每请求草稿、多题进度、回复附件、滚动、焦点/选区及阅读模式独立保留。窗口级键盘、
粘贴、拖放和原生预览只路由到当前且未被关闭层阻塞的请求；上下文/附件命令显式携带 ID。
蓝点以 2.8s 周期只改变两个实心蓝色，不改变尺寸或透明度，查看后清除；占位仍为 6pt，
不移动既有标题和来源行。减少动态效果时静态显示。
新到条目高亮覆盖层连续闪烁五次，每轮 400ms，包含亮起、停留、熄灭和间隔；
2s 后结束，选中背景与文字保持，蓝点的未查看状态独立保留。
随后由 `popup/UnreadRipple.vue` 在未查看行内绘制从蓝点向外扩散的圆形渐隐：扩散 2s、
800ms 开始渐隐 1.5s、休息 1s；柔化宽度随扩散平方从 0 增至 100px。圆形被行圆角裁切，
不改变文本或布局。真正查看后停止；屏幕外条目及隐藏文档暂停，透明休息段不逐帧绘制，
卸载时清理观察器、帧回调和定时器。减少动态效果时保留静态浅蓝背景和蓝点。

成功回答不显示成功提示或设置确认停留，直接让旧正文向上 14px 淡出 126ms、新正文
从下方 14px 淡入 174ms；完成行（及空项目组）同期收起 300ms。
标题栏与页脚位置固定。`popup/inboxTransition.ts` 管理时序和可取消等待；发送时下一条隐藏
准备，原生预览同步完才显示，最后恢复焦点、选区和草稿。输入阻塞与原生预览准备
分别控制，减少动态效果时直接切题。`app/popup_inbox.rs` 从本地提交意图与
权威终态生成成功类型，取消、外部完成和校验失败不会使用成功动画；迟到终态与新到
请求在事件入口登记，避免串行队列等待期间选中失效表单或提前清空本轮。

`popup_inbox_geometry.rs` 扩展 `popup_preview` 所有者，将 Sidebar / 主区 / 预览分配为一次
事务。优先缩预览、再缩 Sidebar、最后主区，自动几何不覆盖正常尺寸偏好；外缘和正常
分隔线调整独立记忆。macOS `popup_canvas.rs` / `swift/PopupCanvas.swift` 仅在 pane 过渡中
关闭 WebKit autoresizing，以固定 viewport 及原生窗口裁切完成展开。WebKit、PDFKit /
Quick Look 共用一个画布父视图；逐帧仅移动画布和窗口 frame，不使用截图遮罩。
过渡前先固定 DOM 的实际宽度，结束后移除离屏 reserve，再切回自适应 CSS。普通外缘
缩放由 AppKit 同步改变画布/WebKit/原生预览，Rust resize 事件只对账和保存，不能异步
重设原生 frame。Sidebar 仅由分隔线改宽；预览关闭时正文吸收宽度变化，打开时预览吸收。
Sidebar / 正文分隔线固定窗口外框，在两区之间分配宽度，Preview 的宽度和屏幕位置保持；
macOS 同步调整 DOM 原点，保留同一原生 viewport。松开后将 Sidebar 宽度写入
`channels.popup.sidebarWidth`（默认 240，通常范围 180–600），遵守 rememberSize；
正常正文尺寸随分配保存，下一轮和冷启动恢复。自动受限分配不覆盖正常正文偏好。
正文 / Preview 分隔线同样使用专用内部调整路径，保持外窗、Sidebar 与响应式 viewport；
不进入展开 / 收起事务，也不等待鼠标释放。合并 / 独立模式共用最新位置合并队列，松手
保存最终有效尺寸。问题复现与验证记录见 `plans/popup-preview-divider-response.md`。
Sidebar / Preview 展开与收起的原生几何提交固定为 0ms，即时切换；固定画布交接
仍保持正文与原生附件位置，正式 Dev 窗口也不再显示慢速动画开关。
`popup_transition.rs` 负责非激活前置及最小化恢复，保留其他应用键盘焦点；托盘指定请求
允许用户显式聚焦。每轮首条从隐藏窗口出现时应用 `general.appearAnimation` 的 macOS 原生
动画，先 `orderFront:` 启动系统出现效果，再非激活前置；可见窗口的新到达只前置。
Windows 前置先调用 Tauri `show()` 同步 Tao 的可见状态，再执行原生非激活显示；仅调用
`ShowWindow` 会使后续 `hide()` 被隐藏状态缓存跳过，留下空白窗口。空队列隐藏失败记录
`popup_host / idle_hide_failed`。原生 HWND 回归入口为 `scripts/popup-visibility-regression.ps1`。
第一条不播中央气泡，后续请求在原生布局提交并前置后，由
`InboxArrivalNotice.vue` / `inboxArrival.ts` 播放中央提问气泡弹跳两次、沿勾线飞入侧栏蓝点的提醒。
中心由 Sidebar 和正文的实际 DOM 边界计算，排除右侧原生附件预览及离屏画布 reserve。
每请求独立并发播放，落位后高亮一次并启动已有未读扩散；查看或终态会取消对应动效。
只在启动时滚动 Sidebar 使目标可见，飞行期间不再次抢滚动；动效层透过鼠标，位于关闭
确认层下方。换题期间的新到达在换题结束后一起播放。减少动态效果时保留静态未读提示。
提示音继续合并短时突发。生产路径已移除 WindowServer 整窗缩放；旧 `popup_pulse.rs`
仅供缩放回归样例引用。完整时序及生命周期见 `specs/popup-arrival-envelope.md`。

GUI Host 保留独立的设置/历史/托盘职责；空宿主不保活 Daemon，沿用 popupPrewarm 控制
空队列后的待命/退出。共享宿主、独立冷热 Helper 和 GUI Host 的启动统一走
`daemon/spawn.rs::spawn_and_reap`；Unix 在启动前分配等待线程，子进程退出后异步回收。
`channels.popup.windowMode` 默认 `merged`，在设置「通用 → 弹窗行为」
第一项「提问窗口模式」可改为 `independent`；设置搜索可按模式名称或 Sidebar 定位。
设置对新提问生效；在途窗口和草稿保留。独立模式继续使用 `PopupFocusArbiter` / 冷热
Helper 及原有级联；只对所选模式补热，闲置的旧模式宿主回收。当前规格与实施记录见 `docs/specs/popup-request-inbox.md`、
`docs/plans/popup-request-inbox.md`；旧级联记录见 `docs/plans/popup-focus-arbitration.md`。

共享宿主收到 `ConfigChanged` 时先同步原生窗口主题与材质，再发 `settings-updated`，
使在途窗口的原生背景和前端颜色一致；切换主题不重建正文或清除草稿。

内部布局失效 `preview geometry changed` 不透传为用户提示；具体阶段记录到既有
`daemon.log` 的 `popup_geometry` 事件，不额外重排窗口。成功切题和本轮结束清除旧队列
错误，取消失败仍保留在关闭确认内。规则见合并窗口规格 §19。

Message、题干及 Confirm 的 Markdown 链接共用 `lib/markdownLinks.ts`，读取原始 `href`
而不是 WebView 解析后的应用资源 URL。本地文件引用剥离行号 / 列号和片段，解码路径，
相对路径以该请求的项目目录为基准，经 `open_path` 交给系统默认程序；缺少项目或不支持
的链接保持原界面。Popup 建窗时注入 `popup_navigation.js`，在主文档捕获阶段取消链接
的默认 click / auxclick 导航，继续让正文处理器打开目标，不影响 Mermaid sandbox 子帧。
合并与独立模式使用相同保护，避免链接替换整个作答页面及丢失草稿；规则见规格 §20。

## 稍后回答与小窗

合并窗口里可把一条请求标记为「稍后回答」：只是本轮内存里的显示标记，不回答、不取消，仍可浏览和作答，一轮结束即清。
**所有待答请求都被标记时**，整窗缩成只有 Sidebar 列表的小窗（标题「待答 N」，跟随置顶设置，以左上角为锚、可拖动）；
只要有一条没标记的请求（含新到达的），窗口就是展开的；展开后无待答主要请求时的窗口叫 peek，用户离开 AskHuman 后会缩回小窗。
需求决策 C1–C46 与规则 R1–R16 见 `specs/popup-inbox-collapse.md`，实施与验证记录见 `plans/popup-inbox-collapse.md` §8，
Windows / Linux 的实机补验清单见 `PROGRESS.md`。

- **模型与流程**：`popup/inboxCollapse.ts` 是不依赖 Vue / Tauri 的纯决策（标记 / 取消 / 浏览 / 答完跳转 / 收窗，给出
  `collapse` / `expand` / `switch` / `mini-resize` 之一；从小窗展开的默认目标是最后打开的 → 第一条未查看 → 列表第一条）。
  `PopupInboxView.vue` 把结果落到窗口：所有会改窗口形态的动作进同一条串行队列（`enqueue`：`shrink` / `select` / `peekShrink` /
  `collapseOnLeave`），沿用布局事务 `popup_inbox_layout → popup_inbox_commit(animation) → popup_inbox_finish`。
  `popup/InboxSidebar.vue` 兼作小窗列表；小窗尺寸（宽 = Sidebar 宽，高 = 前 3 行，其余滚动）由 `popup/inboxMini.ts::measureMini`
  从真实列表量出，不写入窗口尺寸偏好。单请求没有 Sidebar 列：缩小前先把列表隐藏摆放出来量尺寸，展开后让它淡出（`loneList`）。
- **窗口事务**：`popup_inbox_geometry.rs` 的 `Allocation.collapsed` 表示窗口是小窗（宽度与 `main_height` 仍描述页面保持排好的展开布局，
  只有 `frame` 变小）；`Geometry.drift` 抵消「以小窗为锚展开」造成的正文位移，加上小窗被拖动后的 `moved` / `reset`，保证下一轮窗口位置不漂移；
  `ignores_resize()` 让小窗的 resize 事件不进尺寸偏好。
  macOS 由 `popup_canvas.rs` → `swift/PopupCollapse.swift` 的显示链路逐帧动画窗口 frame 与固定画布，页面同时交叉淡化列表与正文
  （`--ms` 驱动，减少动态效果时即时）。Windows / Linux 没有固定画布，由 `popup_canvas::instant` 立即改尺寸与位置（缩小先降最小
  尺寸、变形后禁用缩放边缘，展开相反），页面是 CSS grid，单请求的列表绝对定位、不占网格列；这条路径在 macOS 上也被编译，
  但尚未在真实的 Windows / Linux 上验证。
- **激活与切走缩回**：「用户在 AskHuman」是进程级状态。macOS 由 `popup_activation.rs` + `swift/PopupActivation.swift` 观察
  `NSApplication` 的激活通知，命令 `popup_app_active` 给起始值、事件 `popup-app-active` 推变化；Windows / Linux 用 popup 窗口的
  `Focused` 事件。页面对离开留 200ms 抖动保护；有主要请求、窗口已是小窗、拖拽进行中都不触发缩回。
- **拖入文件**：`popup/useInboxDrop.ts` 注册一个窗口级 `onDragDropEvent`（inbox 下 `usePopupCore` 不再自己监听），事件在
  `inboxDrop.ts::serialDrag` 里串行并合并。小窗上停留 500ms 展开到指针下那一行的请求（否则默认目标），展开后停留 250ms 切到另一行，
  Sidebar 列表与正文靠近上下边缘 56px 内自动滚动；松手落到指针下的请求，纵向布局落到指针下的题卡、顺序布局落当前题。
  拖拽打开的窗口在没有主要请求时，拖走 600ms 后缩回。常量集中在 `inboxDrop.ts` 的 `DRAG`；落点换算（macOS 固定画布要修正 y，
  Windows 是物理像素、GTK 是 CSS 像素）在 `dropPoints`。失败提示与 `askhuman_drop` 日志见 spec §6.9。
  macOS 上 wry 在 `draggingEntered:` 里就读文件列表，AskHuman 此时不是前台的话系统会拒绝之后的读取，所以
  `popup_drag_front.rs` + `swift/PopupDragFront.swift` 给 popup 的 web view 的三个拖拽方法加了钩子：带文件的拖拽进入时先取前台、
  拖走不松手则把前台还给原来的应用、松手则留在前台；根因与验证见 spec §6.10。
- **测试与原型**：模型与拖放表驱动测试在 `inboxCollapse.test.ts` / `inboxDrop.test.ts`，页面流程在
  `PopupInboxView.{collapse,later,leave,drop,mini,columns}.test.ts`。jsdom 不按选择器权重排序规则，依赖层叠结果的 CSS 用
  `popup/cssCascade.testkit.ts` 的最小层叠解析器测（`InboxSidebar.undo.test.ts`、`PopupInboxView.columns.test.ts`）。
  独立原型 `swift/prototype/PopupCollapseVerify.swift` + `examples/popup_collapse_demo.rs`（feature `collapse-prototype`，
  `./scripts/popup-collapse-demo.sh` 运行，不进产品构建）保留作动画与几何的行为参照：`--auto`（367 项）与 `--auto-controls`（18 项）
  静默运行、不取焦点；`--focus`、`--burst`、`--video` 和交互模式会取走键盘焦点，运行前先问用户（spec C26）。

## 来源标题与上下文

来源名（弹窗标题与渠道消息头共用）的解析优先级为 **自定义环境变量 `ASKHUMAN_ENV_SOURCE_NAME` > 探测到的发起 Agent 展示名（Claude Code/Codex/Cursor/Grok）> 默认「the Loop」**。后端入口为 `models::source_name_for_agent`；MCP 模式无法从 env 判断家族时先回退默认名称，再由 daemon 异步进程树解析补齐 Agent。

当探测到 Agent 且未定制来源名时，`PopupView` 按 `popup.messageFrom/questionFrom` 的 `{source}` 占位把文案拆成前后两段，将 Agent 与 workspace 胶囊内联在标题中。未探测到 Agent 时仍显示默认来源；设置了自定义来源名时，标题使用自定义文本，胶囊继续作为上下文显示。窄窗下优先保留 Agent 名，再依次收缩项目名、标题前缀与后缀。

`.brand-time` 显示提问创建时刻的相对时间，满 24 小时后转绝对时间，hover 显示精确时间。时间锚点由 daemon `RequestRegistry::create()` 记录，经 `ShowPayload.created_at_ms` 和 `PopupInit.createdAtMs` 送到前端；缺少旧协议字段时以弹窗构造时刻兜底。

- **Agent badge**：来自按请求 ID 获取的 PopupContext。若 `PopupInit.agentTerminal` 表明对应终端可激活，badge 可调用 `focus_agent_terminal(agentPid)` 聚焦 Agent 终端。
- **Agent Window 入口**：daemon 仅在调用方 `(agent_kind, agent_session_id)` 精确命中活动
  `AgentRegistry` 记录时下发 `agentConsoleSessionId`；五家 Agent 共用同一门控，不按 pid / cwd
  模糊猜测。顶栏右侧据此显示快捷按钮，经 GUI Host 打开全局唯一 Agent Window 并定位该 session，
  Popup 本身保持等待。置顶 Popup 场景下目标窗口临时使用同级置顶，避免开在其后方。
- **workspace badge**：来自按请求 ID 获取的 PopupContext.project（git 根或 cwd），显示目录名、hover 展示完整路径，点击通过 `open_path` 在文件管理器打开。

Agent 会话标题显示在作答区的标题位置：macOS 在标题栏（`PopupNavbar.vue` 里红绿灯旁的 `.session-strip`，
绝对定位、不占高度），Windows / Linux 写进原生窗口标题（页面经 `popup_set_session_title` 上报，
`app/popup_title.rs` 合成 `<标题> — AskHuman`）。不增加常驻标题行，也不预留高度。原生名称来源与缓存刷新规则、
决策见 `docs/specs/popup-session-info.md`；Agent / 项目胶囊的悬浮信息见下文「来源胶囊悬浮卡」。

这些字段通过 `PopupInit{project, projectName, agentKind, agentPid, agentConsoleSessionId, agentSessionId, agentSessionTitle}` 上送；
终端类型在首屏后由 `popup_agent_terminal` 异步解析；预热 Popup 的上下文读取边界另见
`docs/specs/popup-prewarm.md`。

普通 IM Message / Question 卡通过独立的每请求 `ConversationOrigin` 复用相同 source / Agent / 项目，项目
显示 basename，标题规则与 MCP 最多 200ms 的 IM-only 解析等待见
`docs/specs/im-request-origin.md`。结构化确认卡不走这套标题。

## Mermaid 图表

本地 Agent 内容中的显式 ```` ```mermaid ```` fenced code block 会渐进增强为图表，覆盖 Popup 的
Message / Question、回复历史详情以及 Agent 控制台的 Watch / 完整会话；Permission Confirm、产品更新
日志与四个 IM 渠道仍展示原始代码，不执行远程渲染。各入口共用 `MarkdownContent.vue`，普通 Markdown
或没有 Mermaid fence 时不会加载完整 Mermaid 实现；完整会话还用 IntersectionObserver 只调度可见区
附近的内容。

Markdown 提问附件的同窗预览也识别显式 Mermaid fence：`attachment_markdown.rs` 只增加内部标记，
`AttachmentMarkdownContent.vue` 在保留后端 HTML / URL 限制的基础上增强图表，不重新解析附件文本。
浏览器快照仅在有图表时携带自包含的本地经典脚本，按 nonce 精确允许产品脚本；普通快照仍禁用脚本。
两者共用 `attachmentMermaid.ts` 的源码切换与布局，并复用同一个 Mermaid adapter、安全清洗及资源上限。
浏览器按系统外观重绘，离线运行；旧系统 Quick Look 不在这一增强入口内。详见
`docs/plans/attachment-mermaid-rendering.md`。

每个 Markdown body 最多渲染 10 张图，单图源文最多 40,000 字符，并锁定 `maxEdges=400`。源码先于
图表可用；单图可复制源码、切换图表 / 源码，加载、语法、清洗或资源限制失败只让该图回退到代码块。
图表读取所在 Markdown 容器的实际正文字号，随 light / dark / system 有效主题重绘。宽图优先缩到
可用宽度，但以 12px 可见字号为缩放下限；达到下限仍放不下时才在自身容器横向滚动，窗口改变宽度
会重新计算。渲染采用 Mermaid sandbox 后再解码并 fail-closed 验证 SVG，以自有 CSP 和
`sandbox=""` 的无权限 iframe 重新封装；HTML label、回调、
外部链接与远程资源都不启用。

## 多问题纵向模式

设计见 `docs/specs/multi-question-vertical.md`，实现计划见 `docs/plans/multi-question-vertical.md`。该模式由 `experimental.verticalQuestions` 控制，出厂默认开启（设置「通用」Tab 可关），仅在开启且问题数大于 1 时生效；关闭时保留一次一题的左右切换。

纵向模式由 `PopupView` 同时渲染所有题卡。scroll-spy `current` 表示视口题，统一动作目标为 `actionQ = focusedQ ?? current`：textarea 仍聚焦时，被动滚动不会改变快捷键、选项角标、语音或页脚导航的题目归属；失焦后才交回视口题。`⌘1–9` 选择动作题后会把该题滚回可见，显式点击另一题选项或导航才 blur 旧编辑器并移交上下文。若聚焦题卡完全滚出内容视口且未固定，则焦点与 owner 一并结束；固定判定先执行，部分可见或已固定的编辑器不受影响。程序化导航期间有短暂锁定避免抖动，composer-only 几何测量不能触发 scroll-spy。每题用 visited 状态跟踪是否看过，最后一题可见后才显示发送按钮。选项、文本、图片和回复文件均按题目索引保存；拖放图片按原生落点归属题卡（悬停期间高亮目标卡；落点坐标 macOS/Linux 为 CSS 像素、Windows 为物理像素，两种解释互为兜底），粘贴图片归当前聚焦题。单题不启用这些纵向模式样式与状态。

## 回看时固定答案编辑器

普通问答的单题、顺序多题和纵向多题共用 `AnswerComposer.vue`。纵向题卡的折叠空态与聚焦空态保持和单行预设答案相同的紧凑高度，未聚焦时 hover 也沿用预设答案的高亮底色。空白聚焦态的语音 / 图片按钮同行靠右；出现第一个字符后，文字区恢复整行宽度，按钮移到输入框内部的下一行，后续多行只增长文字区。输入自增高不在 live textarea 上经过 `height:auto`：普通输入先用 live scrollHeight 快速排除无需增长的情况，疑似增长与删除 / undo 等缩短路径均使用固定定位的隐藏镜像确认内容高度后才写最终高度，避免 Wayland / WebKitGTK 的边框尺寸误差逐次累加，避免 WebKit 因临时塌缩反复锚定 `.content`。focus ring 由非滚动的 `.input-wrap::after` 按 `:focus-within` 绘制，避免 WebKit 在聚焦 textarea 增高时只重绘阴影的局部脏区。blur 时已展开输入框保留输入阶段测得的高度，只有确实折叠时才清除内联高度，避免 WebKit 滚动锚定在点击期间移动题卡。最后一个选项到输入框的布局间距等于选项间距加 focus-ring 宽度，使激活后的可见间距仍与答案之间一致。单题 / 顺序模式上屏时只有 textarea home 在 `.content` 内至少可见 50% 才自动 focus；不足时保持未激活，后来滚入视口也不追补自动 focus。textarea 获得焦点后成为最近激活的编辑器；它仍有实际 focus，具备“用户手动激活过”或“曾完整显示”任一资格，并在激活后发生向上滚动时，原输入位置落到 `.content` 视口下方会把同一个编辑器 DOM 通过 Teleport 移到 `.content` 与 footer 之间的底部固定区。点击一个已被底边裁切的输入框本身不立即固定；弹出时的自动聚焦、异步布局或 resize 也不会自行触发固定。未固定前先失焦再滚动不会固定；已经固定后 blur 不清除编辑器归属，因此选择或复制 Message 文字不会让固定区消失。高输入框的固定与回位都按固定态可见高度投影：向上回看时先由内容区底边逐步裁切到约 120px 再停靠，向下时在原位能承接这段高度后回位并继续逐步露出，从而避免 `240px ↔ 120px` 引起顶部与预设答案间距跳变。

固定判定与小幅滞回在 `composerDock.ts`，owner、占位高度、ResizeObserver、焦点 / 选区和输入法组合态保护在 `usePopupCore.ts`，固定区外壳由 `ComposerDock.vue` 提供。合并窗口会在同一文档里同时挂载多份 `PopupView`，各自带一个固定区，所以 Teleport 目标 id 按实例唯一（`composerDockTargetId`），不能共用固定 id，否则 `querySelector` 会把编辑器送进文档里第一个、可能已被隐藏的固定区；Teleport 之后若编辑器没落进本实例的固定区，就回退为内联编辑器并不再重试。纵向多题的 composer owner 与 scroll-spy `current` 解耦；固定区显示 `Question i/n` 并可回到原题。固定编辑器仍有焦点时，统一动作目标留在该题：`⌘↵` 跳过题卡 reveal-first，`⌘1–9` 选择后将题卡滚回可见，显式跨题动作才结束旧焦点。完整行为见 `docs/specs/popup-pinned-composer.md`，实施记录见 `docs/plans/popup-pinned-composer.md`。

## 页内查找（⌘F / Ctrl+F）

规格见 `docs/specs/popup-find.md` 与 `docs/specs/popup-find-scopes.md`。⌘F（Windows/Linux 为 Ctrl+F）
按最近主动操作的区域选择「提问 / 当前附件」，搜索条在该区域标题栏下占位展开；附件标题栏也有
查找图标。同一时刻只有一条搜索条，共享关键词与 Aa，切范围清另一范围高亮。提问范围搜索共享
Message、题干、预设选项、附件名和 Confirm 详情/选项，排除答案草稿；顺序多题跨题定位。渲染后的
Mermaid 图按可见 label 作为一个原子命中并高亮整张图，不修改 sandbox 内 SVG；切到单图源码后
恢复普通文本逐次匹配。附件搜索当前模式的 Markdown、文本/代码、完整虚拟 diff，以及 macOS PDF
可提取文字；不支持和无文字状态明确提示并保留原文件入口。Esc 结束会话并清除高亮。

`usePopupFind` 管理每请求会话和路由，`FindBar` 共用控件，`lib/findInDom` 处理 DOM 高亮；
`useAttachmentFind` 适配正文、diff 模型及带请求/附件/查询代次的 macOS PDFKit 桥。切附件定位首个
命中；切模式、重渲染、顺序切题挂载和请求恢复只重画高亮，不主动滚动或抢焦点。纵向模式
scroll-spy 改写当前题也不视为内容变化（spec F13）。原生正文布局避让搜索条且保持 PDF 阅读位置。

## 推荐选项

规格见 `docs/specs/recommended-option.md`。`-o!` / `--option!` 与普通选项语义相同，只增加“AI 推荐”标记；一题可有多个推荐项，但不会自动预选。Popup 与历史详情显示绿色推荐 badge，IM 渠道显示本地化推荐前缀；无论展示怎样变化，提交值始终恢复为原始选项文本。

## 来源胶囊悬浮卡

`SourcePopover.vue` 为 Agent / 项目胶囊各提供一张 100ms 悬浮卡，也支持 Tab 聚焦、移入操作、复制与 Esc；
WebKit 载入页面时给第一个控件的初始焦点和程序性聚焦不算用户意图，不会打开卡片。
Agent 展示完整原生标题、Session ID 及可靠的模型 / 上下文；项目展示稳定路径、不同调用目录及 Git 摘要。
终端动作只接受本请求的已知目录。浮层不占常驻高度；每次打开按需读取，短缓存、无轮询、跨请求失效。
完整决策与来源边界见 `specs/popup-session-info.md`。
