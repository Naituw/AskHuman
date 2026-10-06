# 三栏过渡的基础验证

## 问题与结论边界

2026-10-06，正式窗口的慢速检查暴露 Sidebar 提前绘制、正文逐帧闪烁、原生 Preview 错位。
隔离重叠绘制和补齐坐标确认后，用户仍观察到正文轻微横向抖动、分割线明显抖动。
这些路径没有通过视觉验收。用户要求停止反复修补正式窗口，先用 Vue 3 + Tauri 2 做
独立的左右栏 / 固定正文 demo，验证基础方案。

`nextTick`、绘制确认或 `CATransaction` 单独不能保证整个系统同帧。旧路径同时涉及
NSWindow / WindowServer 几何、WebKit viewport 和远端绘制、原生快照位置、以及独立的
PDF/Quick Look sibling。窗口位置和内容补偿若在不同帧呈现，或存在分数像素差异，正文
就会位移；快照与实时 DOM 的分割线同时存在时，边界也可能重复或错位。这是需要分别
验证的机制分析；没有逐帧像素证据，不把它写成已证实的唯一根因。

## 固定原生 viewport，改变窗口裁切

入口 `./scripts/pane-foundation-demo.sh`；Vue 为 `src/prototype/PaneFoundationDemo.vue`，
Rust 为 `src-tauri/examples/pane_foundation_demo.rs`。不复用正式 Popup、附件组件、业务状态
或截图过渡模块，不连接 Daemon / IM，不加载产品配置。独立 app ID 为
`com.naituw.askhuman.pane-foundation-demo`。

固定 WKWebView 画布包含 240pt 左栏、1pt 分割线、560pt 正文、1pt 分割线、400pt 右栏，
总宽始终 1202pt。左右栏一直挂载。关闭 Wry 默认宽高 autoresizing，只改变窗口裁切和
整张画布的原生位置。逐帧不改变 WebView 尺寸或 Vue grid、不复制正文和分割线、不更换
输入组件、不做截图。Vue 仅发送一次目标状态，逐帧处理在原生侧。

设原正文屏幕左边为 S，画布内正文左边为 L，左右已露出宽度为 l、r：

```
窗口左边 = S - l
画布在窗口内的左边 = l - L
正文在画布内的左边 = L
正文屏幕左边 = (S - l) + (l - L) + L = S
窗口宽度 = 正文宽度 + l + r
```

两条分割线属于同一画布，与正文距离恒定。左右边界先对齐当前 backing pixel grid，
画布补偿读取实际 NSWindow frame，避免请求值或整数 readback 累积误差。窗口和画布
在同一 AppKit 回调中更新，层属性禁用隐式动画。这消除了 WebKit resize/reflow 和截图
补偿交接，但 NSWindow 与 Core Animation / WindowServer 的实际呈现仍必须测量；
几何恒等式本身不是视觉零抖动的充分证明。

## 对照与测量

- 原生扩窗：窗口随左右栏展开，固定画布的中间部分保持屏幕位置。
- 固定窗口对照：窗口和画布均固定，仅裁切左右栏 HTML 内容，用于排除窗口几何，
  验证正文排版、输入和细线；此模式不是完整产品窗口行为。

界面提供左右 / 同时 / 连续六次切换、300 / 1200 / 3000ms 速度、输入框、滚动正文和
高对比细线。分别检查 AppKit 模型坐标、SkyLight 只读窗口 bounds、CALayer presentation
坐标和 WebView 尺寸；前端检查 page resize、草稿、选择及滚动。私有接口仅查询自身
窗口，不调用私有 transform 或其他窗口的接口。

首轮 `--auto` 运行 30 次（24 次扩窗、6 次固定窗口对照）、443 个采样。模型、WindowServer、
呈现层和 WebView 宽度偏差均为 0；page resize 为 0，草稿 / 选择 / 滚动保持。
类型检查和 backing pixel invariant 单测通过。首轮冷启动有一次约 189ms 采样间隔，
后续原生循环大多约 14–19ms；这些数据**不证明动画节奏已经满意**。日志为
`.askhuman-dev/pane-foundation.jsonl`，脚本读取报告，不用 macOS 进程退出码单独判成功。

第二轮自动运行补齐最终高度检查：30 次过渡的模型、窗口服务、呈现层坐标偏差与 page
resize 仍均为 0。冷启动采样间隔曾达 328ms，暖态约 18ms，不能宣称所有机器的节奏无卡顿。
随后 CUA 在 3000ms 下分别切换左栏、右栏及连续六次；8 次过渡均无坐标漂移、viewport
变化或 page resize，各组最大采样间隔约 23.4 / 18.4 / 23.8ms。基础 demo 的坐标验证
与慢速画面检查完成。用户表示 demo 看起来没有问题，授权自主验证后正式实现。

## 正式接入与 SDK 差异

正式窗口采用同一固定画布机制，原生 PDF/Quick Look 与 WebKit 共同挂在 PopupCanvas。
正式慢速过渡采样已覆盖打开 PDF、展开/收起 Sidebar、第二题到达及切换附件上下文：
模型、WindowServer、呈现层正文 X 和 PDF X 均无漂移，viewport 保持 4174×620pt；
草稿与原生 PDF 在切换和统一关闭确认层往返后保留。正式 700ms 过渡的采样最大间隔
约 21–29ms。只读诊断仅在 Dev Instance 且开启专用环境变量时输出，不成为产品依赖。

接入时确认了一个 SDK 差异：Tauri-runtime-wry 2.11.4 在 macOS 单 WebView 窗口的
`inner_size()` 和 resize 事件读取 WKWebView 尺寸。固定虚拟 viewport 必须以 NSWindow
`outer_size()` 读取可见窗口；本项目 Overlay titlebar 使用 full-size content，二者没有
内容装饰差额。误把 4174pt 虚拟画布当作 560pt 可见窗口会计算出负宽度；正式路径已
改用真实 NSWindow extent。旧的失败样本不能计入通过统计。

macOS 系统 Zoom/恢复还有中间 resize 事件，`isZoomed` 此时可能尚未更新。尺寸记忆
必须以 AppKit `inLiveResize` 区分实际外缘拖动；临时系统几何只更新可见分配，不覆盖
正文/预览偏好。分隔线拖动仍由明确的用户几何提交保存。

最终正常 220ms 的 PDF/Sidebar 展开分别有 19/18 个采样，最大间隔约 20.1/19.7ms，
上述正文/原生预览坐标仍为 0 漂移。系统 Zoom/还原后偏好保持 560×620 / 700；
最小化后的到达恢复保留当前 PDF/草稿。完整测试、输入自动化边界及测试副本 TCC
问题见 `docs/plans/popup-request-inbox.md` §11。

基础 demo 和本机正式验证不替代 Windows/Linux、DPI/多屏或真实 IM 终态验收；这些
项目已由用户同意延期并记录项目 todo #4。

## 普通缩放与过渡的交接

用户实际外缘拖动发现固定画布方案误把宽度变化给 Sidebar，且每个 resize 事件异步重设
画布造成抖动。现将两种布局明确分开，保持正文坐标 L=606 不变：

- 过渡：前端先测量并固定当前实际正文/预览宽度，native 关闭全部相关 autoresizing，
  扩大离屏 reserve；随后仅提交窗口裁切和画布原点。
- 交接：DOM 仍固定时 native 移除 reserve，把画布宽度设为 window.width + L − leftSpan，
  启用宽高 autoresizing，最后发布自适应 CSS。两种布局的实际使用宽度一致。
- 普通缩放：Sidebar 宽度固定，闭预览时主区右端贴画布右端；开预览时正文固定、预览
  右端贴画布右端。AppKit 同步调整 Canvas、WKWebView 和 PDF/QL，Rust 仅对账/记忆。
- 原生预览的迟到 DOM 测量不能用旧宽高覆盖实时 frame；自适应模式以当前父视图的
  右/底边确定尺寸。工作区受限和系统 Zoom 仍不覆盖用户尺寸偏好。

`scripts/pane-resize-demo.sh --auto` 使用同一 Swift canvas 桥，12 次 pane 切换及 72 次
宽高缩放，84 项检查通过。模型正文 X、WebView / HTML / native PDFView 尺寸偏差和
固定/自适应交接偏差均为 0；草稿、选区、滚动保留。该自动检查是原生程序化几何证据，
不代替真实拖动的主观视觉验收。正式代码随后安装，Rust 1249 passed / 3 ignored、
前端 228 项通过；用户复看后继续提出未查看提示的样式迭代，记录见 Spec §16。

## 一手参考

- [Apple NSView autoresizingMask](https://developer.apple.com/documentation/appkit/nsview/autoresizingmask-swift.property)：关闭 viewport autoresizing。
- [Apple CATransaction](https://developer.apple.com/documentation/quartzcore/catransaction)：层树事务边界，不能据此推断全链路同帧。
- [Apple NSWindow setFrame](https://developer.apple.com/documentation/appkit/nswindow/setframe(_:display:animate:))：窗口几何与 display 行为。
- 本机 Wry 0.55.1 `src/wkwebview/mod.rs`：主 WebView 默认宽高 autoresizing，本实验显式关闭。
- 本机 Tauri-runtime-wry 2.11.4 `src/lib.rs`：单 WebView `InnerSize` / macOS resize 事件来源。
- [Apple NSWindow inLiveResize](https://developer.apple.com/documentation/appkit/nswindow/inliveresize)：区分用户实时拖动与程序/系统尺寸变化。

### 后续正式交互：即时 pane 切换

用户明确要求 Sidebar / Preview 即时展开后，正式宿主几何 commit 固定为 0ms，并
删除正常/Dev 的慢速开关。固定画布保留为布局交接机制，普通外缘缩放继续自适应。
真实 PDF / Sidebar 展开、收起、自动出现及请求切换的 11 条原生记录均为 0ms / 单次
提交，正文目标与模型坐标漂移为 0；原生预览位置与草稿保留。详见实施记录 §18。
独立 harness 的动画实验与上面旧 220/700ms 记录仅作为历史诊断，不描述当前正常行为。
