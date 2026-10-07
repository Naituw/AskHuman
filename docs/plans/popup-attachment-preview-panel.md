# 实现计划：Popup 附件预览面板

> 关联规格：[popup-attachment-preview-panel.md](../specs/popup-attachment-preview-panel.md)
> 状态：实现、自动验证、macOS 安装及用户体验验收已完成；已确认 Windows / Linux 实机 gate 保留。
> 以 2026-10-05 当前工作区为起点；macOS 独立原型只作为交互参考，不能替代 Tauri / 三平台验收。

## 0. 已确认目标与当前接入点

首版仅替换 Popup 提问附件的预览：同窗展开，默认紧凑，主回答区完整，统一新面板，不保留
Quick Look 模式设置。其他入口继续使用现有实现。窗口规则、图片拖放分工与内容边界以 spec 为准。

| 当前文件 | 已核对的职责 | 本次改动 |
|---|---|---|
| `src/views/PopupView.vue` | Navbar、content、ComposerDock、Footer、Overlays 的编排 | 外加主区 / 预览区布局，保留原主区 DOM 与状态 |
| `src/views/popup/MessageSection.vue` | 提问附件胶囊、点击 / 双击 / 拖放 / 右键 | 点击激活入口、同源菜单、继续拖出原文件 |
| `src/views/popup/useAttachments.ts` | 选择、高亮、Quick Look 调用与事件、拖出状态 | 分离焦点 / 激活，接入新面板与每附件阅读状态 |
| `src/views/popup/usePopupCore.ts` | 快捷键、答案 owner、拖入落点、清理 | 按焦点路由；排除预览区的回复拖入；终态清理 |
| `src-tauri/src/app/mod.rs` | 创建、预热领用、窗口事件、尺寸落盘 | 预览几何控制与主区域尺寸投影 |
| `src-tauri/src/app/popup_size.rs` | 420×480 下限、恢复 / 延迟事件过滤 | 在原规则上补充临时展开和布局事务的过滤 |
| `src-tauri/src/commands.rs` / `app/invoke.rs` | 自定义命令与分组路由 | 新增 Popup 专用命令，继续使用 Core handler 分组 |
| `src-tauri/src/attachment_diff.rs` | 有界读取、unified diff 解析与静态 HTML | 抽出可复用解析模型，Quick Look HTML 与新面板共用 |
| `src-tauri/src/macos_quicklook.rs` | 原生预览、Markdown 转 HTML | 保留其他入口；抽出可跨平台复用的 Markdown 渲染 |
| `src-tauri/src/macos_menu.rs` | macOS 附件原生菜单 | 显式携带调用窗口和预览目标，不再从任意入口猜 Popup |

当前 `show_attachment_menu` 在非 macOS 是空操作。新 Popup 的更多按钮不能调用这个空实现：
macOS 保留原生完整菜单，Windows / Linux 为本次 Popup 入口补齐可用的公共菜单动作，右键与
更多使用同一构造器。当前文件图标也只有 macOS 提供，其他平台拖出需要有效的 PNG 图标回退。

## 1. 代码组织与状态职责

新增模块与实际职责：

| 新模块 | 职责 |
|---|---|
| `src-tauri/src/app/popup_preview.rs` | 原生几何事务、事件对账、主区尺寸投影、平台降级 |
| `src-tauri/src/app/popup_preview_geometry.rs` | 固定右侧布局、矩形、缩放、主区恢复的纯计算 |
| `src-tauri/src/attachment_preview.rs` | 请求内附件解析、有界读取、类型判定与返回模型 |
| `src-tauri/src/attachment_markdown.rs` | 平台无关的静态 Markdown 渲染，复用已有 pulldown-cmark |
| `src/views/popup/AttachmentPreviewPanel.vue` | 单行标题栏、内容区、加载 / 失败 / 不支持状态 |
| `src/views/popup/AttachmentImagePreview.vue` | 图片适应 / 缩放 / 溢出平移 / 文件拖出 |
| `src/views/popup/useAttachmentPreview.ts` | 激活、几何意图、前端绘制握手和布局事件订阅 |
| `src/views/popup/useAttachmentContent.ts` | 读取代次、内容缓存、每附件阅读状态 |
| `src/views/popup/AttachmentDiffPreview.vue` | 共享 diff 模型的行虚拟显示 |
| `src-tauri/src/app/popup_preview_actions.rs` | 原文件操作与非 macOS Popup 菜单 |

### 1.1 单一状态所有者

- 前端 `useAttachments` 管理附件列表焦点与用户动作；`useAttachmentPreview` 管理唯一激活索引和几何，`useAttachmentContent` 使用该索引管理内容与阅读状态。
- 后端管理原生窗口几何。前端发送展开 / 收起 / 分隔线意图，依据后端返回的实际布局绘制；
  不同时从前端调用 setSize / setPosition 与后端控制窗口。
- 阅读状态以本次 request + 附件索引为键，包含原文 / 预览模式、各模式的滚动位置、图片的
  适应或缩放倍率与查看位置。索引对应原请求附件，允许同路径重复出现而不误串状态。
- DOM 中的键盘焦点与附件激活分别处理。收起清除激活高亮，阅读状态保留到本次 Popup 结束。
- 加载状态区分 loading、ready、unsupported、readError、encodingError、tooLarge；不同状态
  使用简短文案，均保留原文件操作。失败结果不自动调用外部应用。

### 1.2 命令边界

实际命令为 `popup_preview_prepare` / `popup_preview_layout`、`popup_preview_read` /
`popup_preview_cancel_read` / `popup_preview_thumbnail`、`popup_preview_reveal` / `popup_preview_menu`。
布局带 version，读取带 requestId / index / generation；文件动作按 requestId / index 派生原路径。
菜单使用当前原生指针位置，并在 macOS 绑定调用 Popup 的 contentView。

从调用窗口、当前请求取得提问附件列表：冷 helper 使用 `AppState.interaction.ask()`；预热
helper 使用已领用的 `WarmPopup.show`，不能读取空占位请求。校验窗口为有效 Popup、请求仍未
收尾、索引有效、目标为普通文件。读取与文件操作不接受一条任意前端路径来替代请求附件。
`requestId` 只用于拒绝过期调用，不替代后端当前请求的授权范围；`loadGeneration` 原样关联前端
这次加载，后端读取完成时也复核所捕获请求仍有效。返回内容 DTO 带 request / attachment /
load generation，布局 DTO 带 layout version。布局与菜单命令同样校验当前请求代次，避免预热
helper 被下一次请求领用后执行前一次的异步动作。

`app/invoke.rs` 保持分组 handler，不将所有新命令合并回一个巨大 generate_handler。
窗口修改在 Rust 完成，因此不需要全局增加前端 setSize / setPosition capability。

## 2. 原生窗口几何与尺寸记忆

### 2.1 坐标与工作区

几何算法在原生物理坐标中工作，逻辑常量按目标屏幕 scale 转换；返回前端的区域尺寸才转为
逻辑 / CSS 像素。不要把不同 DPI 屏幕的全局位置分别除以 scale 后混在同一坐标系计算。

同时测量 outer rect、inner size / position 与装饰占用，区分主区域内容尺寸和带边框的外框。
当前 Tauri 2.11.5 本地源提供 `Monitor.work_area()`，可排除 Dock / 任务栏；选择与主区域
相交面积最大的屏幕，处理负坐标与无有效 monitor 的降级。
[Tauri Monitor 文档](https://v2.tauri.app/reference/javascript/api/namespacewindow/#monitor) 也明确
工作区和屏幕坐标为物理像素。

纯函数固定右侧布局，默认预览 700 + 分隔线 6，可读门槛 320 + 6；有有效记忆值时优先使用。
优先保留原主区宽度，
必要时整窗左移，整块工作区较窄时先减预览宽度，最后才临时缩窄主区。macOS monitor 坐标
按窗口 scale 统一归一化，其他平台沿用全局物理坐标。

### 2.2 几何控制器

控制器保存可恢复的主区尺寸、展开前位置、期间是否由用户移动、最近普通窗口 frame、布局
version、pending target 和已发布布局。流程为：

1. 首次展开快照主区域和原位置；切附件不再次展开。
2. prepare 返回当前可见主区的固定尺寸，前端等待两次 animation frame 后再 commit。
3. 原生 setters 不等于完成通知。macOS tao 会异步排队 resize / move，事务通过后续主线程
   采样确认 frame；等待期间继续使用上一次已发布布局，不把旧尺寸误判为系统拒绝。
4. 对账 Moved / Resized / DPI 的实际值，过滤排队旧事件与程序恢复事件；操作失败只回滚一次
   并使用窗内横向分区，不反复重排。
5. 所有开放布局都是横向。临时缩窄只改变渲染宽度，不替换恢复宽度或持久化偏好。
6. 移动不触发自动重排；正常开放期实际位置变化设置用户移动标记，程序事务和最大化事件排除。
7. 收起恢复原位置，用户移动过则保持新位置；恢复主区域有效尺寸。提交 / 销毁只清理。

最大化、全屏或不可定位状态使用窗内横向分区；恢复普通状态后重新读取实际窗口，不能将
最大化尺寸存成主区偏好。布局事件不请求激活，不影响 daemon 焦点 owner。

### 2.3 持久化接线

`persist_popup_size` 目前观察 Resized 后直接保存整窗 inner size，必须在这里接入控制器：

- 预热、未展示、最小化 / 最大化、收尾过滤全部保留。
- 收起态继续使用原 SizeMemory 逻辑；展开态传入主区域投影，绝不传总宽 / 高。
- 原生布局事务期间不落盘；完成后更新恢复基线，防止延迟回调覆盖另一个 helper 的已存尺寸。
- 用户显式改变主区尺寸时才形成新的记忆值；仅改变预览宽度不能改主区记忆。
- `channels.popup.previewWidth` 单独保存用户调整的预览内容宽度，旧配置缺省为 700。
  外缘与正常分隔线调整保存有效值；展开事务、临时受限、最大化、DPI 换算不保存缩窄结果。
  外缘调整与主尺寸写入合并，分隔线命令返回前保存，避免紧接着提交时丢失宽度。
  预览宽度观察基线只由缩放回执更新；左外缘拖动即使先触发 Moved，也不提前消耗宽度变化。
- 保持加载最新 rememberSize 开关与无钥匙串保存路径；关闭该开关后使用默认 700。

### 2.4 Linux / Wayland

优先依据实际窗口后端确认能力，不仅凭环境变量猜测。在无法可靠读取 / 控制全局位置的
Wayland 环境直接选择 inside，不先移动窗口再尝试回滚。GTK 文档说明窗口管理器可以忽略
move 请求，因此 X11 也要核对实际结果。[GTK Window.move](https://docs.gtk.org/gtk3/method.Window.move.html)

inside 仅改变同窗区域分配，保留原尺寸和完整作答流程。可精确定位的平台拒绝某次位置请求时
也采用这一兜底，不进行反复 resize / move 循环。

## 3. 文件读取、模型与渲染

### 3.1 类型与编码

按大小写不敏感扩展名依次识别现有 Markdown 五种扩展、diff / patch、现有七种图片扩展；其余
有界读取后判定普通文本，包含无扩展名和代码文件。

UTF-8 严格解码并识别 BOM；带 BOM 的 UTF-16LE / BE 严格解码，拒绝奇数字节和无效 surrogate。
普通文本中的 NUL / 非文本控制字符用于判定不可靠或二进制；不对未知编码进行猜测替换。
diff 仍保持既有 UTF-8 及上限契约。文件不足以可靠判定时显示 unsupported / encodingError。

先检查普通文件与元数据，再通过有上限的 reader 实际读取，不能仅信任 metadata 后无界
read_to_string。读取、解析在 blocking worker 执行；每 helper 限制并发工作量。切换或关闭使
请求代次失效，结果到达时再次检查 request、索引与激活状态，旧结果只能丢弃。

### 3.2 渲染选择

- **diff**：抽出 `parse(text) -> ParsedDiff`（文件段、元数据、行 kind、原始文本、两侧行号、
  notice）。现有 `render` 使用同一个模型生成 Quick Look HTML；新面板返回模型供 Vue 文本
  插值渲染，沿用现有测试与样式语义。不能为新面板再写一套 hunk 解析器，也不借用权限 diff
  模型改变附件原始前缀或解析降级规则。
- **Markdown**：把 Quick Look 的 pulldown-cmark 静态渲染抽到平台无关模块，保留 tables、
  task list、strikethrough、footnotes 和原始 HTML 转义语义。新面板使用生成的受限片段及应用
  主题样式，不直接把附件原文当 HTML 注入。现有原始 HTML 转义不等于对生成链接的完整
  清洗；新面板须显式限制链接 / 图片 URL scheme，并验证危险链接不能借助主 WebView 执行。
  不为附件新增任意文件读取、脚本或远程执行能力。仅抽共用渲染，面板策略单独接入，不改变
  其他入口行为。
- **原文 / 文本**：Vue 文本插值或 textContent，等宽字体，保留空白、独立滚动，不添加代码
  语法高亮。按附件保存原文与预览各自滚动位置，等待布局稳定后恢复并限制到实际可滚范围。
- **图片**：读取限定大小的字节与可信 MIME；显示为 Blob / data URL 的 img，不将 SVG 插到
  主页面。SVG 活动 / 外部引用边界用真实 WebView 和恶意样例验证，不直接复用 Mermaid 的
  专用 allowlist 假装所有 SVG 都同构。当前使用 data URL，内容随有预算的缓存逐出或 Popup 销毁释放。

当前 Rust `image` 只启用 PNG / JPEG / WebP features，不能假定它已能探测 GIF / BMP。图片
头部尺寸检查须覆盖所有支持格式，按需启用对应轻量读取能力；SVG 另走有界文本检查。确认
WebView 对这些格式的实际解码结果，不能因为 Rust 不支持某格式便悄悄缩减已确认范围。

图片完整 bytes 保留动画播放，不为了缩略图解码成静态 PNG。主列表缩略图、预览和拖出图标
需要各自资源控制，不重复无界加载同一张图；拖出图标只需小型 PNG，拖出的 payload 仍是原路径。

### 3.3 性能参数验证

已落入实现的预算：

| 资源 | 限制 |
|---|---|
| 文本 / Markdown | 2 MiB、20,000 行；严格 UTF-8 或 BOM UTF-16 |
| Markdown 渲染 | 最多 50,000 parser events、生成 HTML 8 MiB |
| diff | 沿用 2 MiB、20,000 行、单行 16 KiB |
| 位图 | 20 MiB、4,000 万 canvas 像素 |
| 动画 | 最多 500 帧，canvas 像素 × 帧数不超过 8,000 万 |
| SVG | 2 MiB、5,000 XML nodes、最大 65 层；禁止 DTD，img 隔离 |
| 内容缓存 | 每 Popup 64 MiB，估算 JSON 的 UTF-16 字符串大小；LRU 逐出 |
| 列表缩略图 | 单文件 2 MiB、400 万像素，顺序读取，总 JS payload 8 MiB |
| 后台内容读取 | 每 helper 最多 2 个并发 blocking worker；入队后和读取后复核代次 |

手工基准 `preview_resource_benchmark`（dev profile、opt=0）实测：1.76 MB / 19,000 行
密集 diff 27.34 ms，2 MiB 长行文本 26.26 ms，2 MB Markdown 33.17 ms，overview.webp
1.72 ms。19,000 行 Markdown 表格因 parser event 上限返回完整超限状态，20.99 ms。
这些是读取 + 解析时长，不是 WebView 绘制时长或进程峰值内存结论；不据此放宽既有 diff 边界。
高压缩图片、动画帧预算与缓存 / 失效代次通过确定性测试覆盖，diff 的真实 DOM 采用行虚拟化。

每附件保存小型阅读状态，内容采用有预算的缓存，仅保留当前文件及少量最近文件；总预算按
实际原文 + 解析模型 / HTML + 图片 payload 计算，不按磁盘文件大小替代内存占用。缓存逐出不
清除阅读状态。不要用同时挂载所有大附件来实现阅读位置记忆。

## 4. 前端布局、交互与作答兼容

### 4.1 主区域与面板

在 PopupView 外层建立 grid / flex 容器，原 Navbar、content、ComposerDock、隐藏 file input
和 Footer 在常驻主区；新面板与分隔线是兄弟区域。改变 CSS 排列而不是卸载 / 重建主区。
保留 contentRef、composer Teleport target、答案 owner 与多题状态。

顶部拖拽区仍属于主区；预览工具栏按钮不标成 drag-region。面板可以懒加载内容组件，但主区
默认宽度不等待新 renderer / 图片加载。PopupOverlays 的遮罩应明确覆盖整个同窗根，不被预览
区绕过；弹层打开时保持现有提交 / 取消流程。

标题栏左侧为文件名 + 带边框的原文切换，右侧计数 / 导航 / 打开 / 更多 / 关闭占固定宽度。
始终一行、文件名 min-width:0 和 ellipsis。英文 / 中文与窄宽度测试必须保证按钮可操作；不能
用按内容自然换行导致切换文件后按钮移动。纯文本、图片不显示切换按钮，但不改变右侧位置。

### 4.2 用户动作拆分

原 `selectFile` 同时改变选择和焦点，不能直接把它改成“再次调用就收起”，否则右键、键盘和
原生菜单都会误触发收起。新增显式 click-toggle 与 show-preview 意图；焦点移动保持单独函数。

双击仅执行第一次 click 的预览切换，忽略同一连续点击手势的后续 click，移除原文件打开绑定；
拖出也不能因随后 click 取消激活。事件计数、拖拽标记与 click 仲裁需要实机验证，
不能靠点击延迟损害单击响应。此双击规则于 2026-10-07 替代旧的双击打开要求，回车仍打开原文件。
右键 / 更多调用显式查看目标；菜单的快速查看在 Popup 中展开新面板，目标已激活时仍保持打开。

快捷键处理放在 find / IME / 语音等已有优先级之后，按 event target / activeElement 所在区域
区分附件列表、标题栏、正文和输入控件。正文方向键只滚动，标题栏左右键切附件；禁止带
修饰键的无关操作被附件键盘逻辑吞掉。确认面板不接入这一普通 Ask 的快捷键分支。

切附件和收起不主动 focus 附件；仅用户在预览区域发起关闭、需要回到合理控件时才恢复相应
焦点，不能抢走正在使用的 textarea。读取完成、主题变化和 DOM 尺寸变化也不移动答案焦点。

### 4.3 图片操作

- 默认 fit，倍率随有效 viewport 调整，完整居中；100% 按自然图像像素的既有浏览器语义显示。
- 缩小 / 放大围绕可见中心调整，位置限制在可滚范围；紧凑控件，不加常驻提示。
- 通过实际 scrollWidth / Height 与 clientWidth / Height 的溢出判断拖拽用途，容忍像素舍入。
  没有溢出时 draggable 走原文件拖出；有溢出时禁用 HTML 图片默认拖放，pointer capture
  走 scrollLeft / Top 平移。一次手势锁定用途，resize 不能在手势中途改成另一种拖动。
- 回到 fit 恢复文件拖出，缩放后的查看状态按附件存储；fit 模式始终随 viewport 重新适应。
- 原文件拖出共用已有 startDrag，统一 draggingOut、完成 / 取消 / 失败清理及有效 PNG 图标。
  原生入站 drop 落在预览区时不添加为用户回复；主区原有落点和按题目归属规则保留。

既有拖出插件支持三平台文件 drag；这一能力有维护方文档和本地 2.1.1 源码依据，但必须实际
验证各目标应用接收的是原文件。[CrabNebula drag-rs](https://docs.crabnebula.dev/plugins/drag-rs/)

## 5. 文件操作和同源菜单

- 打开使用现有原文件打开能力；新增 reveal command 绑定请求索引，路径含空格、引号和特殊
  字符用结构化参数，避免拼接 shell 脚本。
- macOS reveal 复用 NSWorkspace / open -R；Windows 复用 Explorer 选择文件能力；Linux
  验证目标桌面文件管理器的选择能力。若只能打开所在目录，记录具体限制并经 AskHuman 确认
  降级文案与行为，不把打开目录默认为已经定位原文件。不得把一个没有效果的按钮当作实现完成。
- macOS 菜单 Target 携带显式调用窗口和 Popup 附件目标。“快速查看”发给该 Popup 的预览
  控制器；其他入口继续调用 Quick Look，不全局替换原命令。
- Windows / Linux 的新 Popup 菜单用现有 Tauri Menu / ContextMenu，提供打开、预览、文件
  管理器定位、拷贝路径等能实现的动作。菜单事件绑定当前 request / index，结束后释放上下文；
  不伪装支持 macOS 独有的打开方式列表或文件剪贴板动作。
- 更多与胶囊右键共用入口与构造器。菜单锚点从实际按钮 / 指针得到，CSS 坐标按当前窗口 scale
  转换；异步菜单动作检查请求仍有效，不将旧路径应用到新附件。
- 不支持状态正文的打开 / 文件管理器按钮复用这些动作，标题栏继续保持正常导航与关闭。

## 6. 实施顺序与每阶段完成条件

### 阶段 A：Tauri 几何验证

先接入纯几何模型、窗口事务和 SizeMemory 投影，用轻量预览占位区验证固定右侧 / 左移 /
inside，不提前接大文件渲染。验证拖动结束判定、程序回调对账、标题栏占用、跨 DPI、收起恢复
和 rememberSize 不污染。先完成 macOS 安装验收；Windows、Linux X11 / Wayland 的实际窗口能力作为已确认外部 gate 保留，不阻塞后续阶段接入。

完成条件：打开 / 收起 / 用户缩放 / 分隔线 / 跨屏 / 预热 / 并发窗口的尺寸序列可解释，下一次
提问仍是有效主区尺寸。若 Tauri 原生行为需要改变已确认窗口策略，先 AskHuman，不绕过 spec。

### 阶段 B：读取与内容模型

实现请求内附件解析、有界读取、文本编码、类型识别和异步代次；抽出 diff 模型与 Markdown
渲染。先完成普通文件、错误、超限、不支持及原文 DTO，再接入前端各内容组件和资源预算。

完成条件：既有 Quick Look diff 测试继续通过；普通文本 / Markdown / 图片限制有性能测量，
并发旧结果不能覆盖新附件，异常文件不能阻断主区域作答。

### 阶段 C：完整交互与原生动作

接入激活 toggle、固定标题栏、原文切换与阅读状态，图片缩放 / 拖放，打开 / reveal / 菜单，
以及快捷键焦点规则。统一终态清理，排除预览区的回答文件拖入，保持主区所有既有行为。

完成条件：spec 的可见交互完整可验，更多 / 右键同源，unsupported 两按钮可用；其他入口的
Quick Look 与既有 IM / stdout 契约未被替换。

### 阶段 D：安装、实机验收与文档收口

按第 7 节验证，再在各平台运行安装脚本并用安装后的 AskHuman 验收。将实际资源阈值、平台
差异和真实验收证据写回计划，更新 Popup 当前实现概览与附件规格；完成后删 PROGRESS 条目。
有外部平台 gate 未完成时如实保留，不以纯计算 / mock 测试宣称实机通过。

## 7. 验证安排

### 7.1 自动验证

- Rust 几何测试：右侧固定、自动左移、先缩预览再缩主区、工作区非零 / 负坐标、装饰、DPI、主区恢复、inside、
  事务版本和延迟事件。扩展 SizeMemory 测试覆盖临时展开、用户分隔线、多 helper、rememberSize。
- Rust 文件测试：范围校验、冷 / 热请求、文件替换 / 删除、UTF-8 / UTF-16、NUL / 控制字符、
  超限、取消和 diff 解析不回归。测试实际派生的模型，不只复述函数内部条件。
- Vitest：点击 / 再次点击、切换导航、模式 / 位置恢复、旧异步结果、焦点优先级、菜单同源、
  主区保留与预览区 drop 排除、图片溢出下不同拖动语义。
- 构建与静态检查：`pnpm build`、`pnpm test`、Rust 相关测试及完整测试、
  `cargo fmt --check`、`cargo clippy --features custom-protocol -- -D warnings`（在 src-tauri）。
  现有 hook / CI 有额外必需项时按仓库配置执行，不为样式微调反复跑全套。

### 7.2 原生安装验收

macOS / Linux 用 `./scripts/install.sh`，Windows 用 `./scripts/install-windows.cmd`；随后通过
新安装 AskHuman 提供真实文件与问题，检查答案能正常返回。若后续采用并行工作树，先遵守
`docs/agent-worktree-setup.md`，经 AskHuman 选择渠道 preset 后 dev enable；仍是 install → AskHuman。

| 验收组 | 必需场景 |
|---|---|
| 几何 | 右侧与受限横向分区、屏幕边缘、标题栏拖动、外缘 / 分隔线、关闭恢复、最小化 / 最大化、DPI / 多屏 |
| 尺寸记忆 | 展开后下一 Popup 紧凑；预览恢复上次用户宽度；默认 700；受限宽度不覆盖偏好；预热迟到事件、多个并发请求、rememberSize 关闭 |
| 内容 | Markdown、既有 patch fixture、长行代码 / JSON、无扩展名、图片 / 动画 / SVG、ZIP、错误 / 超限 |
| 图片动作 | fit 拖原文件、放大溢出平移、fit 恢复拖出、从胶囊拖出、取消拖动、目标应用接收原路径 |
| 作答 | 单题 / 顺序多题 / 纵向题卡、固定编辑器、输入法、查找 / 语音 Esc、草稿 / 选项保留 |
| 文件动作 | 直接打开、reveal、右键 / 更多、切附件后目标变化、文件消失、特殊字符路径 |
| 兼容 | 历史 / 待办仍用原入口、Confirm 不改变、深浅色 / 应用材质、终态关闭与焦点仲裁 |

首屏无附件预览时不承担大文件读取或所有图片解码；对比改动前后的正常 Popup 启动和输入
响应，不以加载附件牺牲作答。

## 8. 文档与交付边界

- 第 9 节记录已完成实施与证据；未完成的平台实机项仍保留外部 gate。
- 当前阶段保留 `overview.md`；实现后更新 `overview-popup-ui.md` 和对应附件 spec。只有实际
  改动了全局模块地图 / 不变量，才更新主 overview。
- 不新增用户模式开关，不扩大到历史 / 待办 / 回复附件，不加入代码语法高亮、双栏 diff 或
  图片编辑能力。现有 Quick Look 保留给其他入口。
- 实施分阶段可独立审查；出现需改变已确认方案的新证据时，通过 AskHuman 说明具体影响后
  再调整。提交遵守 Conventional Commits，只有实际完成的用户功能使用 feat / fix。

## 9. 实施与验证记录（2026-10-05）

- A：固定右侧、预览优先缩小、最后才 inside；关闭恢复原宽度 / 位置，用户移动标记保持到关闭。macOS Tao setters 异步排队，通过后续采样确认实际 frame；前端 prepare → 两次绘制 → commit → 实际 frame 回执，消除原先向左 / 上下换边和回答区短暂变宽的问题。
- B：静态 Markdown、共享 diff 模型、可靠文本、原字节图片与动画接入。正文有界后台读取和代次校验，每附件状态与 LRU 内容分开；超限不展示部分内容。SVG 按 W3C 的 img secure animated mode 隔离，源文件不插入主 DOM，参考 [SVG2 secure animated mode](https://www.w3.org/TR/SVG2/conform.html#secure-animated-mode)。
- C：提问附件激活、标题栏、原文、导航、原文件菜单 / 定位、图片 fit / 100% / 平移、输入焦点与终态清理接入。macOS 定位复用 NSWorkspace；非 macOS 通过 Explorer / FileManager1.ShowItems，服务不可用时显示失败，未以打开目录冒充定位成功。
- 最终全量回归：Rust **1,222 passed / 0 failed / 3 ignored**（其中一个为本次手工资源基准）；Vitest **205 passed / 35 files**，Node **5 passed**。`pnpm build`、`cargo clippy --features custom-protocol --all-targets -- -D warnings`、格式及 diff 检查通过。`./scripts/install.sh` 成功编译、签名并安装；以下原生验收使用此次安装的最终二进制。
- 原生验收使用安装后二进制的隔离副本（私有 ASKHUMAN_HOME 与测试 IPC，不改生产 daemon / 用户偏好），真实 WKWebView 与 AppKit；独立交互原型不计作证据。
- Windows / Linux 实机安装、窗口管理器、DPI / 多屏与目标应用拖放为明确外部 gate，记录于 PROGRESS 与已接受的项目 todo #3；不以 macOS 或 mock 结果宣称这些平台通过。

### 9.1 macOS 最终安装后的真实验证

| 验收组 | 观察与结果 |
|---|---|
| 标题 / diff | 中文浅色、英文深色下的红绿增删、旧 / 新行号、文件头和原始前缀可读；查看原文按钮有边框且紧邻文件名；导航与关闭位置稳定 |
| 窗口 / 记忆 | 默认主区 560×620；分隔线显式改为 790 宽后，关闭得到 790×620，私有配置记住 790；调回 560 后，外缘将总宽压到 856，主区临时变窄，关闭恢复 560×620；最大化时关闭预览后恢复普通窗口仍为 560×620；未把展开总宽写入主区偏好 |
| 阅读状态 | Markdown 滚动后切换附件再回来，正文位置恢复；diff 原文模式切换附件、关闭重开后保持；图片 100% 平移后切换回来，倍率与查看位置恢复 |
| 内容边界 | 19,000 行 diff 正常阅读；UTF-16 文本保留中文 / 缩进；二进制、不支持格式、超过 2 MiB 文件展示说明与原文件操作，不显示乱码或部分内容 |
| 图片 / SVG | fit 完整居中、100% 溢出拖动平移；GIF 与动画 WebP 观察到两个交替帧；SVG 作为 img 显示，脚本未改变父 DOM，外部图片 / fetch 未命中测试服务；对照请求命中服务，排除监听失效 |
| 原生动作 | 更多菜单绑定当前 Popup contentView，实际显示完整菜单；快速查看保持当前附件打开；打开 UTF-16 附件后 TextEdit 显示正确正文，URL 为原附件路径 |
| 作答 / 结束 | 附件切换与 Esc 关闭预览保留编辑器草稿；真实发送回传两题选项和草稿；另一请求取消正常回传，helper 退出；测试窗口与测试服务已清理 |

原生定位调用复用现有 NSWorkspace 文件选择 API。初次本机系统弹出 **Finder 没有响应 Show in
Finder 服务请求** 的错误；后经用户授权重启 Finder，直接按钮实际选中 `icon.icns`，切换附件后
菜单实际选中原 `utf16.txt`，该环境验收项已解决。未以打开目录替代选择原附件。
最终安装后通过 AskHuman 提供实际附件复查宽度记忆、关闭位置恢复和原附件拖出，用户确认
「通过，完成本轮」。负坐标 / 不同 DPI 与用户移动标记已有纯计算回归，但不能代替多屏实机验证。

### 9.2 最终反馈：预览宽度记忆

用户要求默认预览增加为 700，下一次尽量恢复上次宽度，仅空间不足时缩窄；确认遵守现有
「记住窗口尺寸」开关。已加入独立配置值、旧配置默认值、自动受限 / DPI 过滤，Rust 全量回归
更新为 **1,224 passed / 0 failed / 3 ignored**；前端类型检查 / production build 和 Clippy
all-targets 通过，`./scripts/install.sh` 编译 / 签名 / 安装通过。

最终二进制的真实 WKWebView / AppKit 验证：

- 缺少 `previewWidth` 的旧配置首次展开为 700；主区 560，总宽 1,266（含 6 分隔线）。
- 分隔线调整到主区 460 / 预览 800，配置立即分别保存这两个值；结束请求后新 Popup 收起
  为 460，展开恢复 800，总宽仍为 1,266，没有把总宽写为主区尺寸。
- 保留保存值 800 并关闭 rememberSize 后，新 Popup 展开预览为 700；分隔线再次调整后，
  配置仍保留主区 460 / 预览 800，不写入关闭记忆期间的临时调整。
- 移动 / 缩放基线修正后再安装，真实 Popup 仍恢复 800；分隔线调整后立即保存主区 480 /
  预览 780。最终 Rust 全量 1,224 passed，Clippy all-targets 与安装再次通过。
- Finder 重启后直接按钮和切换附件后的菜单选择结果通过；测试 Finder 窗口已关闭，用户
  原有窗口保留；测试 Popup、私有 IPC 与 HTTP 服务已清理。

本次原生截图与 IPC 证据保存在临时验收目录 `/tmp/askhuman-popup-qa`，不是发布资源或长期
回归依赖。完整验收矩阵仍以 §7.2 与规格 §9 为准，表中只记录实际执行并观察到的场景。

### 9.3 预览区窗口拖拽

用户确认右侧标题栏及安全空白均应能拖动窗口。标题栏背景、文件名、模式按钮旁空位、附件计数、
图片控件栏、图片周围和加载 / 不支持状态背景使用显式的 `data-tauri-drag-region`。按钮、正文、
图片本身、滚动条、分隔线和原生文档正文没有继承拖拽标记，保留各自交互。

`./scripts/install.sh` 完成生产前端构建 / 类型检查及二进制安装；Vitest **208 / 36 文件**、
Node **5**、diff 检查通过。未修改 Rust 逻辑，也未增加仅检查模板属性的镜像测试。
使用安装二进制的隔离 QA bundle 执行标题栏、图片空白处和原生 PDF 标题栏的拖动手势；
标题文件名与图片空白的系统双击最大化 / 恢复响应确认窗口拖拽区域已被原生机制识别。
标题导航按钮切换到下一附件；UTF-16 正文可选择文字；100% 图片拖动实际平移内容，
原生 PDF 可滚动，拖动后正文与草稿保留。收起恢复主窗口为 560×700 逻辑像素。
测试窗口与私有 IPC 服务已清理；Windows / Linux 实机验证仍归既有外部 gate。
通过安装后的 AskHuman 提供图片、文本与 PDF 样例交付，用户随后授权提交本次修复。

### 9.4 移除附件双击打开（2026-10-07）

用户确认提问附件双击按第一次单击处理，回车仍打开原文件。`MessageSection.vue` 移除
`dblclick` 打开绑定；保留 `selectFile` 的连续点击计数过滤，第一击立即切换预览，后续
click 不再切换，不新增单击延迟。标题栏、右键菜单和 Markdown 的阅读模式动作保持既有规则。

回归使用真实挂载的 MessageSection 与附件状态，发送 click(detail=1)、click(detail=2)、
dblclick、click(detail=3)，覆盖关闭、同附件已激活和另一附件已激活三种初态，并检查独立
下一次单击仍可切换。修复前这三项均因调用原文件打开而失败，修复后通过。
完整前端验证为 Vitest **258 passed / 42 files**、Node **5 passed**；安装脚本完成
类型检查、production build、macOS 二进制编译、签名及安装。Rust 逻辑未变，未重跑全量 Rust tests。
安装后通过 AskHuman 提供 Markdown、WebP 与 patch 附件复查三种双击状态，用户确认
「符合预期，验收通过」。Windows / Linux 实机证据仍归既有外部 gate。
