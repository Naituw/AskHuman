# Popup UI 概览

> 本文是 `docs/overview.md` 的专题补充，记录 Popup 当前实现地图；具体功能的需求与设计仍以对应 spec 为准。

## 窗口与附件交互

Popup 提问附件采用同窗右侧预览，具体决策见 `docs/specs/popup-attachment-preview-panel.md`。
默认窗口紧凑，点击附件才扩展；切换不重建主区，作答、选项和当前题目状态独立保留。

- 原生窗口下限仍为 420×480。`app/popup_preview.rs` 和 `popup_preview_geometry.rs` 管理固定右侧、必要时整窗左移、受限横向分区与关闭恢复；前端 prepare 后等待绘制，再提交原生几何并异步对账。关闭恢复展开前位置，用户移动过则保持新位置；临时缩窄和展开总宽不写入主区尺寸偏好。`persist_popup_size` 继续过滤预热、收尾、最大化和迟到事件，并读取最新 rememberSize。
- 预览默认 700 宽，`channels.popup.previewWidth` 单独记忆用户的外缘 / 正常分隔线调整，遵守同一个 rememberSize 开关；空间限制或 DPI 变化造成的临时宽度不覆盖偏好。
- `useAttachments.ts` 管理列表焦点、点击激活和原文件动作；`useAttachmentPreview.ts` 管理几何意图与布局订阅；`useAttachmentContent.ts` 管理读取代次、64 MiB 内容缓存及每附件的模式、滚动和图片缩放。收起保留本次阅读状态，下一请求清空。
- `attachment_preview.rs` 从当前冷 / 热 Popup 的请求附件按索引读取，不接受任意路径；限制字节、文本行数、图片像素与动画帧预算，后台执行并丢弃失效代次。列表缩略图另有读取与缓存预算。Markdown 与普通文本支持 UTF-8、带 BOM 的 UTF-16，无法可靠识别或超限时保留打开和文件管理器定位入口。
- `AttachmentPreviewPanel.vue` 提供单行标题栏、带边框的原文切换、固定右侧操作区。Markdown 使用 `attachment_markdown.rs` 的静态受限片段；diff 使用 `attachment_diff.rs` 的共享解析模型，由 `AttachmentDiffPreview.vue` 虚拟显示行；代码与原文作为文本展示。`AttachmentImagePreview.vue` 保留动画原字节，以实际溢出决定文件拖出或平移查看，SVG 只通过隔离 img 显示。
- 窗口拖拽仍用主区 `data-tauri-drag-region`；文件拖出共用 `startDrag` 并使用原路径与有效 PNG 图标。原生拖入的预览区 / 分隔线落点不进入回复附件。附件列表空格和方向键、标题栏左右键与正文滚动分别路由；输入法、查找、语音及既有发送 / 取消优先级保留。
- Popup 更多与右键共用 `popup_preview_menu`。macOS 保留完整原生菜单并将快速查看路由到当前 Popup；Windows / Linux 提供公共文件动作，定位失败显示错误。历史、待办及其他入口继续用已有 Quick Look；它们与 Popup 共用 diff 解析和 Markdown 静态渲染，范围见 `docs/specs/diff-attachment-preview.md`。

## 并发窗口焦点与级联

daemon 的 `PopupFocusArbiter` 是跨 helper 的唯一焦点所有者：最早派发的 Popup 可自动前置并取得键盘焦点，后续 Popup 立即显示但不得激活应用。它们按请求序号进入 FIFO；当前 owner 完成、取消、断连或窗口销毁后，最早存活的等待窗口接力。用户直接点击等待窗口或从托盘选择请求会显式转移 owner，原 owner 回到等待队首。

冷、热 helper 都先隐藏建窗并在内容 `nextTick` 后发送 `PopupReady`，收到 daemon 的 `PresentPopup` 才上屏，从而避免 ready 次序反转焦点归属。macOS 用跨进程 `NSWindow.windowNumber`、`NSWindowBelow` 和系统 `cascadeTopLeftFromPoint:` 在前驱后方非激活级联；Linux 复用同一仲裁协议，采用非主动聚焦和 24 逻辑点级联，精确层级服从 X11/Wayland 窗口管理器。终态与 `PopupDismissed` 分离，并以连接 EOF / 750ms 超时兜底，避免旧窗口关闭期间与下一窗口争抢焦点。完整实施记录见 `docs/plans/popup-focus-arbitration.md`。

## 来源标题与上下文

来源名（弹窗标题与渠道消息头共用）的解析优先级为 **自定义环境变量 `ASKHUMAN_ENV_SOURCE_NAME` > 探测到的发起 Agent 展示名（Claude Code/Codex/Cursor/Grok）> 默认「the Loop」**。后端入口为 `models::source_name_for_agent`；MCP 模式无法从 env 判断家族时先回退默认名称，再由 daemon 异步进程树解析补齐 Agent。

当探测到 Agent 且未定制来源名时，`PopupView` 按 `popup.messageFrom/questionFrom` 的 `{source}` 占位把文案拆成前后两段，将 Agent 与 workspace 胶囊内联在标题中。未探测到 Agent 时仍显示默认来源；设置了自定义来源名时，标题使用自定义文本，胶囊继续作为上下文显示。窄窗下优先保留 Agent 名，再依次收缩项目名、标题前缀与后缀。

`.brand-time` 显示提问创建时刻的相对时间，满 24 小时后转绝对时间，hover 显示精确时间。时间锚点由 daemon `RequestRegistry::create()` 记录，经 `ShowPayload.created_at_ms` 和 `PopupInit.createdAtMs` 送到前端；缺少旧协议字段时以弹窗构造时刻兜底。

- **Agent badge**：来自 `AppState.agent_kind`。若 `PopupInit.agentTerminal` 表明对应终端可激活，badge 可调用 `focus_agent_terminal(agentPid)` 聚焦 Agent 终端。
- **Agent Window 入口**：daemon 仅在调用方 `(agent_kind, agent_session_id)` 精确命中活动
  `AgentRegistry` 记录时下发 `agentConsoleSessionId`；五家 Agent 共用同一门控，不按 pid / cwd
  模糊猜测。顶栏右侧据此显示快捷按钮，经 GUI Host 打开全局唯一 Agent Window 并定位该 session，
  Popup 本身保持等待。置顶 Popup 场景下目标窗口临时使用同级置顶，避免开在其后方。
- **workspace badge**：来自 `AppState.project`（git 根或 cwd），显示目录名、hover 展示完整路径，点击通过 `open_path` 在文件管理器打开。

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
