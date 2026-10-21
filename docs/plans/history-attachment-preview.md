# 实现计划：历史详情接入附件预览面板

> 关联规格：[popup-attachment-preview-panel.md §10](../specs/popup-attachment-preview-panel.md#10-历史详情接入)
> 状态：已实施（2026-10-22）；macOS 自动验证结果见 §6，真实历史窗口验收（§4）与 Windows / Linux 实机 gate 待做。
> 实施中与最初设想不同的几处（独立布局事件、Swift 侧改动、按窗口的槽位）已在 §1–§2 改写为现状。
> 面板本体、几何控制器、读取与原生预览的设计沿用 [popup-attachment-preview-panel.md](popup-attachment-preview-panel.md)，
> 本计划只写把它们接到历史窗口所需的改动，不重复原设计。

## 0. 已确认目标与现状

目标：历史详情的提问附件与回复图片使用同一个同窗面板（向右展开，放不下时窗口内分栏），macOS 系统预览同窗嵌入，
交互完全对齐 Popup（单击 / 空格切换，双击不打开），不再使用旧 Quick Look。回复里的普通文件、待办窗口不变。

现状里妨碍直接复用的点（均已读码核对）：

| 现状 | 影响 | 处理 |
|---|---|---|
| `popup_preview::request` / `assigned_request` 只认 `popup` 窗口与活跃提问请求 | 历史窗口所有预览命令都会被拒 | 新增所有者层（§1.1），Popup 路径行为不变 |
| 几何控制器 `Controller` 是进程内唯一的受管状态；`wait_idle` / `published` / `resized` / `moved` 只要进程里有 `Inbox` 就转给收件箱几何 | 历史窗口可能在 Popup 进程里兜底建窗（宿主不可达时），两者会互相污染 | 历史用独立的控制器状态；Inbox 转发只对 `popup` 窗口生效（`uses_inbox`，§1.2） |
| 控制器带 Popup 副作用：写 `popup` 尺寸 / 预览宽度配置、`SizeMemory`、主区下限 420、`wait_idle` 要求 `GuiBridge` | 历史展开会污染 Popup 偏好或在宿主里直接失败 | 窗口差异收进 `Policy`：历史不持久化、下限 600；存活判断改看窗口是否存在（§1.2） |
| 布局事件 `popup-preview-layout` 不带所有者 ID，修订号按控制器计数 | 两个窗口同时有面板时会收到对方的布局 | 历史用独立事件 `history-preview-layout`（§1.2） |
| 窗口事件只给 `popup` 接了 `Resized` / `Moved` / `ScaleFactorChanged` / `Focused` | 历史窗口的几何不会被对账 | `on_window_event` 增加 `history` 分支，销毁时清状态（§1.2） |
| 菜单 / 原生按键回调把窗口硬编码为 `popup`，校验 `popup_preview::request` | 历史的右键菜单、⌘W / Esc 原生按键失效 | 经所有者层按 ID 找到窗口并校验（§1.3） |
| Swift 侧 `nativePreview` 是进程级单例，按键 / 查找回调不带所有者 | 两个窗口先后显示系统预览会互相顶掉，回调无法区分来源 | 按窗口各存一份预览，回调带所有者 ID（§1.3） |
| `ReadGeneration` / `NativePermits` 是单槽，同一所有者 ID 可能在窗口重建后以更小的代次回来 | 重开历史窗口再预览同一条目会被判为“过期读取”；Popup 与历史互相顶掉对方的槽位 | 按窗口各一个槽位，历史窗口销毁时清空历史槽位（§1.4） |
| 面板组件从 `usePopupContext()` 读约 30 个字段 | 历史没有 Popup 上下文 | 抽出面板专用上下文接口，Popup 与历史各自提供（§2.1） |
| `HistoryDetail.vue` 自管 Quick Look（`preview-index` / `preview-closed`、双击打开、点空白关闭） | 与规格 §10.2 冲突 | 改接共享的预览控制器（§2.3） |

## 1. 后端

### 1.1 所有者层与授权

新增 `src-tauri/src/app/attachment_owner.rs`，是预览命令的唯一授权入口：

| 函数 | 说明 |
|---|---|
| `window_label(owner_id)` | `history:` 前缀 → `history`，否则 `popup`；菜单 / 原生按键回调据此找回窗口 |
| `files(window, owner_id, require_presented)` | 触碰文件的命令使用。`popup` 窗口沿用 `popup_preview::assigned_request`；`history` 窗口要求 `history:<条目 ID>` 且条目仍在 `history.jsonl` 中，返回其 `message.files`；其他窗口拒绝 |
| `file(window, owner_id, index, require_presented)` | `files` 加索引取值，路径只来自这里 |
| `check_window(window, owner_id)` | 只调窗口几何 / 取消读取的命令使用：`popup` 同上严格校验；`history` 只要求窗口是历史窗口且 ID 形如 `history:*`，这样条目被删除后窗口仍能收回 |
| `forget_history(app)` | 历史窗口销毁时清空历史所有者的读取代次 / 原生许可 / 几何状态 |

`history.rs` 新增 `attachment_files(id) -> Option<Vec<FileAttachment>>`：按文件指纹（长度 + mtime，Unix 加 inode）缓存
“条目 ID → 提问附件”索引，文件变化才重读，命令热路径只做一次 `stat`。无附件的条目也收录（空列表），因为回复图片预览
同样需要窗口几何命令；条目缺失才是错误。

`commands.rs` 里所有 `popup_preview::request(&window, id)` 取 `request.message.files` 的调用点（`open_browser`、`prepare`、
`layout`、`read`、`native`、`find`、`cancel_read`、`thumbnail`、`reveal`、`menu`）改走所有者层；`popup_preview_prepare` /
`layout` / `native` 里的 `Inbox` 判断改用 `uses_inbox`（§1.2）。命令名与前端协议不变，只有布局事件另设（§1.2）。

### 1.2 几何控制器

`popup_preview.rs` 保持同一套状态机与纯几何（`popup_preview_geometry.rs` 不改）。每个窗口一个 `Controller`，窗口之间的差异收进
`Policy`，按窗口标签取用：

| | Popup | 历史 |
|---|---|---|
| 布局事件 | `popup-preview-layout` | `history-preview-layout` |
| 展开期间主区下限 | 420 | 600（即整个历史界面；可读预览门槛仍是 320 + 分隔线 6） |
| 是否记忆用户尺寸（写 Popup 偏好与 `SizeMemory`） | 是 | 否 |
| 存活判断（`alive` / `abandoned`） | `GuiBridge` 未结束 | 历史窗口仍存在 |

- 受管状态：Popup 仍用原 `Mutex<Controller>`，历史用 newtype `HistoryController`；访问统一经 `controller(window)` / `policy(window)`。
- `uses_inbox(window)` = 标签为 `popup` 且存在 `Inbox`，替换 `wait_idle` / `published` / `resized` / `moved` 以及
  `popup_preview_prepare` / `layout` / `native` 里的 `Inbox` 判断；历史窗口永远走自己的控制器。
- 历史不写任何配置：`persists` 为假时跳过 `save_dimensions` / `remember_baseline`；初始预览宽度只读 Popup 配置的
  `restored_preview_width`（遵守“记住窗口尺寸”开关）；`special` 窗口没有 `normal` 记录时回退到窗口创建尺寸 `HISTORY_SIZE`（820×600），
  不用 Popup 的 `restored_size`。
- “可持久的右侧分栏”门槛与主区 `clamp` 由常量 `420 / 746` 改为 `min_main / min_main + MIN_PANE`（`MIN_PANE = 320 + DIVIDER`）；
  `can_split` 抽成纯函数，Popup 既有边界（745 / 746）与历史边界（925 / 926）一并有单测。
- `on_window_event` 新增 `history` 分支：`Resized` / `ScaleFactorChanged` → `popup_preview::resized`（结果丢弃，不调用
  `persist_popup_size`），`Moved` / `Focused(true)` → `moved`；`CloseRequested` 先释放该窗口的原生预览再清 Liquid Glass；
  `Destroyed` → `attachment_owner::forget_history`（重置 `HistoryController`、读取代次与原生许可）。
- 事件独立：布局载荷与 Popup 相同，但事件名不同；前端 `useAttachmentPreview` 新增 `layoutEvent` 参数（默认 Popup 的事件），
  `useHistoryPreview` 传 `history-preview-layout`。其余预览事件仍带所有者 ID，由前端按 ID 过滤。

### 1.3 菜单、原生视图与按键

- `macos_menu.rs`：`show_popup` 改名 `show_attachment`，目标元组含义为 `(所有者 ID, 索引)`；`Target::perform` 用 `window_label` 找窗口、用
  `attachment_owner::file` 校验路径（菜单可能晚于附件存在），`popup-preview-show` / `popup-preview-open-browser` 发给该窗口。
- `popup_preview_actions.rs`（非 macOS）：`menu` / `handle_menu` 同样改为按所有者 ID 找窗口并校验；菜单项 ID 里的所有者 ID
  已含冒号，沿用 `rsplitn(3, ':')` 从右侧取索引与动作即可。
- `macos_attachment_preview.rs`：原先的单个 `KEY_TARGET` 改为按所有者 ID 的 `TARGETS`（每个窗口至多一项，`update_view` 先清掉同窗口旧项）；
  `native_key` / `native_find` 经所有者 ID 找回窗口，存活校验改走 `attachment_owner::files`；新增 `release_window`，在 `CloseRequested`
  时释放该窗口的原生视图、按键监视器与目标。
- Swift 侧（实施中确认必须改，原设想“不改”）：`AttachmentPreview.swift` 的 `nativePreview` 单例改为按窗口的 `nativePreviews`，
  `KeyCallback` 增加所有者 ID 参数（两个回调都带），新增 `ah_preview_release_window`，`popupPreviewResizing` 取目标窗口，
  窗口释放后的残留预览在下次更新时清理；`PopupCanvas.swift` 只补 `popupCanvasWindow()` 并给 `popupPreviewResizing` 传窗口。
  非 Popup 窗口没有 `PopupCanvas`，`popupPreviewParent` 回落到 `window.contentView`，与独立 Popup 相同。
  `scripts/popup-find-native-regression.swift` 增补“两个窗口各一份预览、互不顶替、释放只影响自己、回调带所有者”的断言。

### 1.4 读取代次与许可

`ReadGeneration` 与 `NativePermits` 内部改为两个槽位（Popup / 历史，按所有者 ID 前缀 `history:` 选槽），一个窗口的读取与许可不会顶掉另一个
窗口的；二者各增 `forget_history()` 只清历史槽位。`attachment_owner::forget_history` 在历史窗口销毁时调用它们并重置 `HistoryController`
（原生视图由 `CloseRequested` 的 `release_window` 释放；`local_image` 的窗口 scope 沿用既有的 `Destroyed` 通用清理）。

## 2. 前端

### 2.1 面板上下文

- 新增 `src/views/popup/attachmentPanelContext.ts`：定义 `AttachmentPanelContext`（面板实际使用的字段，类型取自 `useAttachments` 返回值和
  最小的只读 ref 类型）与 `AttachmentPanelKey`。`createPopupContext` 额外 `provide` 它（`PopupContext` 结构上是超集）。
- `AttachmentPreviewPanel.vue` 改用 `useAttachmentPanelContext()`；新增可选的 `findSupported`（缺省为真）控制查找图标，历史传 `false`，
  其余查找字段给惰性实现，面板内部的 `useAttachmentFind` 因 `findActive` 恒假而不工作。
- 面板 CSS 仍在 `popup.css`；`HistoryView.vue` 显式 `import "./popup/popup.css"` 声明依赖（`App.vue` 已静态加载，不改变现有行为）。

### 2.2 预览控制器与回复图片

- `useReplyImagePreview` 增加可选 `resolve(image) => Promise<string>`：给出时先异步取得显示 URL，再沿用原有的探测与阅读状态逻辑；
  不给时行为与现在逐行一致（Popup 不受影响）。`useAttachments` 新增 `replyResolve` 依赖并透传。
- `useAttachmentPreview` 不改协议；历史不传 `active`（恒活跃），条目消失由历史侧先收起再切换（见下）。
- 新增 `src/views/history/useHistoryPreview.ts`，作为历史预览的唯一状态所有者：
  - 输入：当前条目 ref 与 `blocked`（确认层是否打开）。所有者 ID = `history:<条目 ID>`，无条目时为 `history:`。
  - **收起再切换**：内部持有 `owner` 引用，监听输入条目：ID 变化且预览已打开 → 先 `stopPreview()`（用旧所有者 ID 发出收起意图），
    再把 `owner` 指向新条目；同 ID 的对象替换（`history-updated` 重载）只更新引用。这样子 composable 的各个同步 watcher 看到
    的永远是“已收起”的状态，不依赖 watcher 执行顺序，也不会对新条目发出多余读取。
  - 回复图片：每个条目内按 `题号 / 序号 / 路径` 缓存稳定的 `ImageAttachment` 形对象（`data` 存路径；控制器按对象身份记选中项，
    同条目重载必须还回同一批对象），`replyResolve` 经专用 `createLocalImageLoader` 把路径换成显示 URL，条目切换时 `reset`、卸载时 `dispose`。
  - 键盘：窗口级 `keydown` 交给 `handleAttachmentKey`（Esc 收起预览并 `preventDefault`，所以不会清掉搜索框；其余键按焦点所在的附件项处理），
    确认层打开时忽略。`popup-preview-native-key`（原生视图持有键盘时由后端转发）：`Escape` 合成 `keydown` 交回同一处理函数，
    `w`+⌘ 关闭历史窗口，其余（`interaction` / `f` / `g` / `Enter`）忽略。原生监视器会吞掉 ⌘F / ⌘G，只把焦点还给 WebView，历史没有面板内查找，无副作用。
  - 生命周期：`onMounted` 初始化预览监听（布局、显示、复制路径、动作失败、浏览器打开），卸载时 `disposeAttachments`。
  - 输出：`{ attach, panel, ownerId }`。`attach` 是 `useAttachments` 的完整返回（附件列表交互、回复图片选择、拖出、菜单、浏览器错误提示，
    以及 `previewLayout` / `previewOpen` / `previewTransition` / `resizePreview`），`panel` 是交给面板的上下文（查找关闭），
    整个对象经 `HistoryPreviewKey` 注入 `HistoryDetail`。
- 抽出 `src/views/popup/usePreviewDivider.ts`（分隔线拖动与展开期间冻结主区的样式），`PopupView.vue` 与 `HistoryView.vue` 共用，
  避免复制 25 行指针逻辑；Popup 独有的 inbox 分配计算留在 `PopupView.vue`。

### 2.3 历史视图与详情

- `HistoryView.vue`：根节点改为 `history-shell`（`position: relative; display: grid; height: 100%`），原 `.history` 变为 `grid-area: main`
  的主区域；右侧分隔线与面板按 `previewLayout.side !== 'closed'` 渲染，样式沿用 `.attachment-preview-divider`。
  确认层移到 shell 一级，保证覆盖整个窗口（含预览）并通过 `blocked` 隐藏原生视图。主区宽度写入 CSS 变量
  `--history-main-width`，`.scope-btn` / `.scope-menu` / `.clear-menu` 用它代替 `vw`，窗口变宽时下拉菜单仍不超出主区域。
  分隔线拖动与展开期间冻结主区复用 `usePreviewDivider`（`beginDivider` / `frozenMainStyle`），窗口为面板缩放期间主区保持原宽（宽度判断
  取过渡里的值），与 Popup 一致。主区窄于 560（`LIST_COLLAPSE_BELOW`，分栏回落）且预览打开时给 `.history` 加 `list-collapsed`，
  隐藏 `.entry-list` / `.empty`，只留详情；这条阈值由前端按主区宽度判断，后端只发布几何。
- `HistoryDetail.vue`：提问附件按 `MessageSection.vue` 的结构渲染（`data-attachment-index`、`selected`、`selectFile(i, $event)`、
  `onAttachmentContextMenu`、`onAttachmentDragStart`、浏览器错误提示），去掉 `previewAttachments` / `closePreview` / `preview-index` /
  `preview-closed` / 双击打开 / 点空白关闭；回复图片缩略图改为 `thumb previewable` + `.thumb-preview` 按钮（与
  `ComposerAttachments.vue` 同结构，复用全局样式）；回复文件仍单击 `openPath`；缩略图 URL 继续由 `useLocalImageUrls` 懒加载。

## 3. 实施顺序与每阶段完成条件

| 阶段 | 内容 | 完成条件 |
|---|---|---|
| A 后端授权 | `history::attachment_files` + 指纹缓存、所有者层、`commands.rs` 改接、`ReadGeneration` / `NativePermits` 的按窗口槽位与 `forget_history` | `cargo test` 覆盖：历史所有者只在历史窗口有效、条目缺失 / 越界拒绝、几何命令在条目消失后仍可收回、缓存随文件变化刷新、两个窗口的槽位互不顶替且 `forget_history` 只清历史槽位 |
| B 几何 | `Policy` / `HistoryController`、`uses_inbox`、存活判断、窗口事件接线、销毁清理、独立布局事件 | `popup_preview` 单测补：Popup 与历史的分栏门槛边界、历史下限 600 与不持久化、两个事件名不同；Popup 既有单测不变 |
| C 菜单 / 原生 | `macos_menu.rs`、`popup_preview_actions.rs`、`macos_attachment_preview.rs`、Swift 按窗口预览与 `release_window` | 编译通过（非 macOS 分支只能由 CI 与实机验证，本机无对应 target）；Swift 回归脚本含两窗口断言 |
| D 前端 | 面板上下文、`useReplyImagePreview.resolve`、`useHistoryPreview`、`usePreviewDivider`、`HistoryView` / `HistoryDetail` | Vitest：收起再切换（含回复图片）、历史重载保留预览、回复图片 URL 解析与释放、键盘与原生按键、`HistoryDetail` 新交互；`pnpm build` 类型检查通过 |
| E 回归与安装 | 全量 `pnpm test`、`cargo test`、`cargo fmt --check`、`cargo clippy -- -D warnings`；`./scripts/install.sh` | 安装后由用户在真实历史窗口验证 §4 清单 |
| F 文档 | 面板 spec / plan 状态、`overview-popup-ui.md`、`overview.md`、`file-attachments.md`、`diff-attachment-preview.md`、`reply-history.md`、`PROGRESS.md` | 只改受影响段落；主 overview 只改模块地图、命令归属与“其他入口”一句 |

## 4. 验证安排

自动：见 §3 完成条件。安装后的真实验证（macOS）由用户完成，逐项对应规格 §10.7：

1. 展开 / 收起的窗口尺寸与位置、放不下时的分栏与列表隐藏、关闭恢复；
2. 切换条目、删除 / 清空条目、筛选改变选中项时预览收起；
3. 提问附件 ⇄ 回复图片切换；双击不打开；回车、标题栏打开、右键 / ⋯ 菜单；
4. PDF / Office / 媒体同窗显示，确认层打开时隐藏，Esc 收起，⌘W 关闭窗口；
5. 展开历史预览后弹出的 Popup 尺寸与预览宽度没有被改动；关闭再开历史窗口无遗留；
6. 待办窗口附件交互不变。

Windows / Linux：编译由 CI 覆盖；窗口几何、原生菜单、DPI / 多屏、X11 / Wayland 的实机验证保留为外部 gate
（`docs/PROGRESS.md`），不以 macOS 证据代替。

## 5. 文档与交付边界

- 需求规则以规格 §10 为准；本计划只记实现与验证。
- 主 overview 只更新模块地图、命令归属（预览命令的所有者含 Popup 请求与历史条目）与“其他入口”一句，不记录阶段。
- 已知保留项：历史窗口面板内查找、待办窗口接入面板（所有者层已按窗口标签设计，可后续扩展）、Windows / Linux 实机验证。

## 6. 实施与验证记录

### 自动验证（2026-10-22，macOS arm64）

- `cargo fmt --check`、`cargo clippy --all-targets --features custom-protocol -- -D warnings` 通过（本机工具链对 `build.rs:68` 既有的
  `uninlined_format_args` 提示另用 `-A` 屏蔽，与本改动无关）。
- `cargo test --features custom-protocol`：1415 通过、4 忽略、0 失败。
- `pnpm build`（`vue-tsc --noEmit` + 两个 `vite build`）通过；`vitest run`：78 个文件 801 个用例通过；`node --test scripts/*.node-test.mjs`：32 通过。
- Swift 回归 `scripts/popup-find-native-regression.swift` 在真实 AppKit / WKWebView / PDFKit 宿主中通过，含两个窗口各一份预览、互不顶替、
  释放只影响自己、回调带所有者的新增断言。
- `useHistoryPreview` 的测试做过变异检查（改成始终收起、不先收起再切换、不重置加载器、忽略 `blocked`，各自让对应用例失败），确认用例有效。
- Windows 目标：试过 `cargo check --target x86_64-pc-windows-msvc`，`ring` 的 C 代码在 macOS 主机上缺 Windows 头文件，无法完成，已撤销该目标。
  非 macOS 分支（`popup_preview_actions.rs`，`commands.rs` / `popup_preview.rs` 的平台分支）只做了逐段阅读核对，编译交给 CI。

### macOS 安装验证

2026-10-22 装进日常环境（`./scripts/install.sh`，GUI Host 换成新构建）后，用户在真实历史窗口按 §4 清单逐项验证，并回归了弹窗的附件预览、
回复图片预览、查找与分隔线，全部通过。

### 保留的外部 gate 与已知保留项

Windows / Linux 实机验证见 `docs/PROGRESS.md`。历史窗口面板内查找与待办窗口接入面板仍是保留项（§5）。
