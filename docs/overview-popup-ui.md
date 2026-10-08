# Popup UI 概览

> 本文是 `docs/overview.md` 的专题补充，记录 Popup 当前实现地图；具体功能的需求与设计仍以对应 spec 为准。

## 窗口与附件交互

Popup 提问附件采用同窗右侧预览，具体决策见 `docs/specs/popup-attachment-preview-panel.md`。
默认窗口紧凑，点击附件才扩展；切换不重建主区，作答、选项和当前题目状态独立保留。

- 原生窗口下限仍为 420×480。`app/popup_preview.rs` 和 `popup_preview_geometry.rs` 管理固定右侧、必要时整窗左移、受限横向分区与关闭恢复；前端 prepare 后等待绘制，再提交原生几何并异步对账。关闭恢复展开前位置，用户移动过则保持新位置；临时缩窄和展开总宽不写入主区尺寸偏好。`persist_popup_size` 继续过滤预热、收尾、最大化和迟到事件，并读取最新 rememberSize。
- 预览默认 700 宽，`channels.popup.previewWidth` 单独记忆用户的外缘 / 正常分隔线调整，遵守同一个 rememberSize 开关；空间限制或 DPI 变化造成的临时宽度不覆盖偏好。
- `useAttachments.ts` 管理列表焦点、点击激活和原文件动作；`useAttachmentPreview.ts` 管理几何意图与布局订阅；`useAttachmentContent.ts` 管理读取代次、64 MiB 内容缓存及每附件的模式、滚动和图片缩放。收起或切换请求保留各自阅读状态，终态销毁对应上下文。
- 提问附件的双击只执行第一次单击的预览切换，忽略后续 click，不打开外部应用；单击立即响应。Markdown 的主按钮依阅读模式选择浏览器快照或原文件，列表回车继续打开原文件。`popup_preview_open_browser` 校验请求 / 索引，`attachment_browser.rs` 重新有界读取、生成快照并按 HTTPS 关联选择默认浏览器；`attachment_html.rs` 与 Quick Look 共用文档样式及 24h 临时目录。菜单动作固定且带请求归属，失败保留明确的原文件入口。规则见 `docs/specs/markdown-browser-open.md`。
- `attachment_preview.rs` 从显式 request ID 对应的待答附件按索引读取，不接受任意路径；限制字节、文本行数、图片像素与动画帧预算，后台执行并丢弃失效代次。列表缩略图另有读取与缓存预算。Markdown 与普通文本支持 UTF-8、带 BOM 的 UTF-16，无法可靠识别或超限时保留打开和文件管理器定位入口。
- `AttachmentPreviewPanel.vue` 提供单行标题栏、带边框的原文切换、固定右侧操作区。Markdown 使用 `attachment_markdown.rs` 的静态受限片段；diff 使用 `attachment_diff.rs` 的共享解析模型，由 `AttachmentDiffPreview.vue` 虚拟显示行；代码与原文作为文本展示。`AttachmentImagePreview.vue` 保留动画原字节，以实际溢出决定文件拖出或平移查看，SVG 只通过隔离 img 显示。
- macOS 的 `macos_attachment_preview.rs` / `swift/AttachmentPreview.swift` 经已有 Swift 构建桥复用 Image I/O、PDFKit 与 QuickLookUI。系统图片生成有界 PNG；PDF 后台创建、保存页坐标与缩放；其他系统文档 / 媒体使用嵌入的 `QLPreviewView`，静态文档视图最多两个，媒体切换后卸载。`useNativeAttachmentPreview.ts` 同步正文矩形、遮罩与代次；命令根据请求索引和有界读取许可验证目标，原生焦点快捷键回到 Popup 业务处理。多图系统图片说明数量和完整原文件入口。范围与系统差异见规格 §5.4。
- 预览能力与 CLI / IM 的 `isImage` 分类分开：新增系统图片可在 Popup 列表显示缩略图，但发送分类沿用原有七种扩展名。ICO / TGA / PNM 只启用现有 image 的轻量 feature，无新增 codec crate；macOS 系统图片能力不承诺在 Windows / Linux 可用。
- 窗口拖拽使用主区和预览标题栏、图片周围 / 控件栏及状态背景的显式 `data-tauri-drag-region`，不覆盖按钮、正文、图片本身、滚动条或原生预览；文件拖出共用 `startDrag` 并使用原路径与有效 PNG 图标。原生拖入的预览区 / 分隔线落点不进入回复附件。附件列表空格和方向键、标题栏左右键与正文滚动分别路由；输入法、查找、语音及既有发送 / 取消优先级保留。
- Popup 更多与右键共用 `popup_preview_menu`。macOS 保留完整原生菜单并将快速查看路由到当前 Popup；Windows / Linux 提供公共文件动作，定位失败显示错误。历史、待办及其他入口继续用已有 Quick Look；它们与 Popup 共用 diff 解析和 Markdown 静态渲染，范围见 `docs/specs/diff-attachment-preview.md`。

## 多请求统一作答窗口

默认合并模式的 Daemon 派发使用 `daemon/popup_inbox.rs` / `daemon/runtime/inbox.rs` 的单一宿主，
以一次性 token 和 generation 认证共享连接；普通题、权限和 Stop 共用 `--popup-host`。
请求账本及终态仍属于 Daemon，IPC submission ACK 按 ID 区分合法提交、渠道抢答和失败。
同一终态只移除一个请求；连接故障有界重建 pending 请求，明确提示内存草稿丢失。

`PopupInboxView.vue` 管理按完整项目路径分组的导航、10pt 未查看蓝点、草稿提示、同项目
下一条和固定 ID 快照的统一关闭。条目保留问题摘要主标题，下面仅一行「Agent / 会话图标 +
任务名」，以紧凑的来源关系表达可信 session 标题；权限 / Stop 在摘要前标记类型。
Daemon 优先传已有 Agent 标题，缺失时异步解析并按 request ID 补齐，未识别时隐藏。
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
`InboxArrivalNotice.vue` / `inboxArrival.ts` 播放中央提问气泡飞入侧栏蓝点的提醒。
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

这些字段通过 `PopupInit{project, projectName, agentKind, agentPid, agentConsoleSessionId}` 上送；
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

每个 Markdown body 最多渲染 10 张图，单图源文最多 40,000 字符，并锁定 `maxEdges=400`。源码先于
图表可用；单图可复制源码、切换图表 / 源码，加载、语法、清洗或资源限制失败只让该图回退到代码块。
图表读取所在 Markdown 容器的实际正文字号，随 light / dark / system 有效主题重绘。宽图优先缩到
可用宽度，但以 12px 可见字号为缩放下限；达到下限仍放不下时才在自身容器横向滚动，窗口改变宽度
会重新计算。渲染采用 Mermaid sandbox 后再解码并 fail-closed 验证 SVG，以自有 CSP 和
`sandbox=""` 的无权限 iframe 重新封装；HTML label、回调、
外部链接与远程资源都不启用。

## 多问题纵向模式

设计见 `docs/specs/multi-question-vertical.md`，实现计划见 `docs/plans/multi-question-vertical.md`。该模式仅在 `experimental.verticalQuestions` 开启且问题数大于 1 时生效；关闭时保留一次一题的左右切换。

纵向模式由 `PopupView` 同时渲染所有题卡。scroll-spy `current` 表示视口题，统一动作目标为 `actionQ = focusedQ ?? current`：textarea 仍聚焦时，被动滚动不会改变快捷键、选项角标、语音或页脚导航的题目归属；失焦后才交回视口题。`⌘1–9` 选择动作题后会把该题滚回可见，显式点击另一题选项或导航才 blur 旧编辑器并移交上下文。若聚焦题卡完全滚出内容视口且未固定，则焦点与 owner 一并结束；固定判定先执行，部分可见或已固定的编辑器不受影响。程序化导航期间有短暂锁定避免抖动，composer-only 几何测量不能触发 scroll-spy。每题用 visited 状态跟踪是否看过，最后一题可见后才显示发送按钮。选项、文本、图片和回复文件均按题目索引保存；拖放图片按原生落点归属题卡（悬停期间高亮目标卡；落点坐标 macOS/Linux 为 CSS 像素、Windows 为物理像素，两种解释互为兜底），粘贴图片归当前聚焦题。单题不启用这些纵向模式样式与状态。

## 回看时固定答案编辑器

普通问答的单题、顺序多题和纵向多题共用 `AnswerComposer.vue`。纵向题卡的折叠空态与聚焦空态保持和单行预设答案相同的紧凑高度，未聚焦时 hover 也沿用预设答案的高亮底色。空白聚焦态的语音 / 图片按钮同行靠右；出现第一个字符后，文字区恢复整行宽度，按钮移到输入框内部的下一行，后续多行只增长文字区。输入自增高不在 live textarea 上经过 `height:auto`：普通输入只在确需增长时写一次最终高度，删除 / undo 等缩短路径使用固定定位的隐藏镜像测量，避免 WebKit 因临时塌缩反复锚定 `.content`。focus ring 由非滚动的 `.input-wrap::after` 按 `:focus-within` 绘制，避免 WebKit 在聚焦 textarea 增高时只重绘阴影的局部脏区。blur 时已展开输入框保留输入阶段测得的高度，只有确实折叠时才清除内联高度，避免 WebKit 滚动锚定在点击期间移动题卡。最后一个选项到输入框的布局间距等于选项间距加 focus-ring 宽度，使激活后的可见间距仍与答案之间一致。单题 / 顺序模式上屏时只有 textarea home 在 `.content` 内至少可见 50% 才自动 focus；不足时保持未激活，后来滚入视口也不追补自动 focus。textarea 获得焦点后成为最近激活的编辑器；它仍有实际 focus，具备“用户手动激活过”或“曾完整显示”任一资格，并在激活后发生向上滚动时，原输入位置落到 `.content` 视口下方会把同一个编辑器 DOM 通过 Teleport 移到 `.content` 与 footer 之间的底部固定区。点击一个已被底边裁切的输入框本身不立即固定；弹出时的自动聚焦、异步布局或 resize 也不会自行触发固定。未固定前先失焦再滚动不会固定；已经固定后 blur 不清除编辑器归属，因此选择或复制 Message 文字不会让固定区消失。高输入框的固定与回位都按固定态可见高度投影：向上回看时先由内容区底边逐步裁切到约 120px 再停靠，向下时在原位能承接这段高度后回位并继续逐步露出，从而避免 `240px ↔ 120px` 引起顶部与预设答案间距跳变。

固定判定与小幅滞回在 `composerDock.ts`，owner、占位高度、ResizeObserver、焦点 / 选区和输入法组合态保护在 `usePopupCore.ts`，固定区外壳由 `ComposerDock.vue` 提供。纵向多题的 composer owner 与 scroll-spy `current` 解耦；固定区显示 `Question i/n` 并可回到原题。固定编辑器仍有焦点时，统一动作目标留在该题：`⌘↵` 跳过题卡 reveal-first，`⌘1–9` 选择后将题卡滚回可见，显式跨题动作才结束旧焦点。完整行为见 `docs/specs/popup-pinned-composer.md`，实施记录见 `docs/plans/popup-pinned-composer.md`。

## 页内查找（⌘F / Ctrl+F）

规格见 `docs/specs/popup-find.md`。弹窗支持浏览器式页内查找：⌘F（Windows/Linux 为 Ctrl+F）在
导航栏右侧操作区叠放查找条（动作按钮渐隐，条自上方滑入），对共享 Message、题干、预设选项、
附件名以及 Confirm 详情/选项做连续子串匹配（默认不区分大小写，条上 Aa 可切换），高亮全部命中
并支持上/下一条与循环；顺序多题会跨题匹配并自动切题。渲染后的 Mermaid 图按可见 label 作为一个
原子命中并高亮整张图，不修改 sandbox 内 SVG；切到单图源码后恢复普通文本逐次匹配。Esc 关闭并清除高亮。实现为
`usePopupFind` + `FindBar` + `lib/findInDom`，不搜用户答案草稿。视口只归用户导航（打开 / 输入 / 上下条 / Aa）
所有：DOM 变化（Markdown 重渲染、顺序切题、纵向模式 scroll-spy 改写当前题）只触发 `repaintHighlights` 重画高亮，
不切题也不滚动（spec F13）。

## 推荐选项

规格见 `docs/specs/recommended-option.md`。`-o!` / `--option!` 与普通选项语义相同，只增加“AI 推荐”标记；一题可有多个推荐项，但不会自动预选。Popup 与历史详情显示绿色推荐 badge，IM 渠道显示本地化推荐前缀；无论展示怎样变化，提交值始终恢复为原始选项文本。
