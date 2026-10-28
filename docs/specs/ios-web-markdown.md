# 需求：iOS 请求正文改用 WebView 渲染 Markdown

> 状态：已实现（2026-10-28；Swift 包与模拟器 UI 测试通过；真机验收待用户，见 `docs/PROGRESS.md`）
> 关联计划：`docs/plans/ios-web-markdown.md`
> 关联既有：`docs/specs/ios-channel.md` §6.3 / §6.4 / §6.5（原为原生渲染、Mermaid 按代码块）、
> `docs/plans/mermaid-rendering.md`（Mac 端 Mermaid 的限额与安全决定）、`docs/overview-ios.md`

## 1. 背景

用户看过 iOS 上的提问页后指出三点：Markdown 渲染没有 Mac 弹窗好（格式不一样）；流程图（Mermaid）渲染不出来；
正文不能选字。分析结论：原生管线（Foundation `AttributedString(markdown:)` → 自绘 SwiftUI）与 Mac 的
markdown-it 在语法与渲染规则上有结构性差距——软换行被合并成空格（Mac `breaks: true` 会换行）、行内代码没有底、
长表格单元格不换行、Mermaid 只能显示成代码、选字只有代码块能选。逐项补原生永远追不上。

用户让我做了一个 WebView 原型（`HumanInLoop-ios-web-md-proto`，不在仓库里），并在 iPhone Air 上实测后选了「正式实现」：
**请求消息正文改用 WebView（一页一个；题干、列表卡片、实时卡片保持原生）**。

原型实测要点（iPhone Air，iOS 27.0.1，Debug）：页面就绪 111–181 ms，全高度 129–229 ms（含 Mermaid 库
加载 52–59 ms + 绘制 26–41 ms）；内存（App 外的 WebKit 辅助进程）1 个纯文字页约 36 MB，带 Mermaid 约 86 MB，
同时 4 个 WebView 约 130 MB；每个 `WKWebView` 各有一个 WebContent 进程（公开 API 不能共享，`WKProcessPool`
自 iOS 15 起不再有效），所以**一页只放一个 WebView** 是对的粒度；离开页面后辅助进程数秒内退出。

## 2. 已确认决策（用户）

| 编号 | 决策项 | 结论 |
|---|---|---|
| U1 | 采用与否 | 正式实现 |
| U2 | 范围 | 请求**消息正文**用 WebView；**题干、列表卡片、实时（Watch）卡片保持原生** |
| U3 | 粒度 | 一页一个 WebView，不为每个题干 / 每张卡片各开一个 |

## 3. 我自行决定的细节（用户睡前授权全权处理，可调）

| 编号 | 决策项 | 结论 |
|---|---|---|
| W1 | 用在哪些页面 | 提问页的 Message（`AnswerScreen`）、确认页的详情正文（`ConfirmScreen` 的 `bodyMd`）、历史详情里的同两处（提问的 Message、确认的详情）。每页至多一个。其余（题干、列表摘要、WatchCard、Markdown 附件预览）仍用原生 |
| W2 | 引擎与版本 | 与 Mac 弹窗同一套：markdown-it（`html:false, linkify:true, breaks:true`）+ Mermaid；版本与根 `package.json` 一致（当前 markdown-it 14.3.0、Mermaid 11.16.1），文件随 App 打包、完全离线。`ios/Tools/web-markdown/sync.sh` 从 `node_modules` 同步并写出 `VERSIONS.txt`，升级 npm 依赖后重跑 |
| W3 | 页面资源 | `ios/AskHuman/Resources/WebMarkdown/`（`index.html`、`style.css`、`render.js`、两个库）作为**文件夹引用**打进 App；一个页面实例 = 一个 `WKWebView` 加载 `index.html`，之后用 `AH.render(source, options)` 渲染。Mermaid 库只在文本里真有 ` ```mermaid ` 代码块时才加载 |
| W4 | 高度 | 页面用 `ResizeObserver` 上报内容高度（点）；SwiftUI 按它设 `frame(height:)`，高度变化走动画。原生侧把高度夹在 0…20000，防止页面脚本报出离谱的数。同一段文字上一次的高度缓存起来作为下次的起始高度（回到旧页不跳）。（原型里还有「第一行基线」探针，用来对齐题干前的原生角标；题干保持原生后用不到，已去掉。） |
| W5 | 图片 | 与现状一致：**不联网取图**。`data:image/*` 内联图片显示；其余（网络、本地路径）显示「图片说明」占位胶囊。理由：Agent 写的 Markdown 不可信，联网取图会泄露 IP / 被追踪；本地图片的 `ahasset://` 懒加载另案（见 `ios-channel.md` §6.5） |
| W6 | 安全 | 页面除 `ready` / `height` / `rendered` / `copy` / `failed` 五个消息外没有任何原生能力；`index.html` 带 CSP（`default-src 'none'`，只放行本页脚本 / 样式与 `data:` 图片），断网也能用；导航被锁死（只允许最初的文件加载，链接点击一律交给原生）；`copy` 消息限长。Mermaid 用 `securityLevel:'strict'` + `htmlLabels:false` + `suppressErrorRendering` + `maxTextSize 40000` + `maxEdges 400`，最多 10 张图、单图源文最多 40000 字符（与 Mac 同限额）。**与 Mac 的差别**：Mac 的 M10 要求 `sandbox` 并重封装 iframe，因为那个页面带着 Tauri 的特权接口；手机上这个页面是一次性、无特权的，所以用 `strict` + CSP + 锁死导航。如要对齐 `sandbox`，另案 |
| W7 | 链接 | 沿用现状：http(s) 先弹「打开此链接？」显示域名，再用应用内浏览器打开；`mailto:` / `tel:` 交系统；其它协议（含 `javascript:`、`file:`、`data:`）忽略 |
| W8 | 选字与复制 | 网页原生选区（长按选词、拖动手柄、复制 / 查询 / 翻译）；代码块右上角保留复制按钮（点击复制整块，轻触感） |
| W9 | 外观 | 跟随系统深色模式（CSS `prefers-color-scheme`，Mermaid 主题随之，系统切换时重绘）；字号用 `font: -apple-system-body`，App 内 Dynamic Type 变化时重新渲染；样式与 Mac `.markdown-body` 对齐（标题 / 列表 / 引用 / 行内代码底 / 代码块 / 表格 / 链接 / 分隔线），颜色用 iOS 系统色 |
| W10 | 表格 | 外包一层可横向滚动的盒子；表格宽度 = 内容宽度，上限约 34em，单元格最小 4.5em，长单元格在上限处换行 |
| W11 | Mermaid 体验 | 先显示源码并带「正在绘制图表…」，绘制完成换成图，高度随之增长（动画过渡）；图宽按容器缩放、下限 75%，更宽的横向滚动；每张图下有「显示源码 / 显示图表」；语法错误 / 超限 / 库加载失败都回落为源码并说明原因。说明文字（按钮、状态）由原生传入，按手机语言本地化 |
| W12 | 失败兜底 | 页面进程被系统回收 → 自动重载并重渲（最多 2 次）；加载失败或 5 秒内（启动参数 `-AHWebTimeout`，默认 5；模拟器默认 30，它冷启动 WebContent 要十来秒，回落了就看不到网页）没有第一个高度 → 该页回落到原生 Markdown 渲染并不再尝试。用户无感知，只是退回到旧样式 |
| W13 | 与「看过」判定的配合 | 多题表单靠题卡位置判定「看过」（`ios-answer-flow.md`）。Message 的高度要等页面报出后才确定，所以**在 Message 第一次排好版（或回落为原生）之前不统计「看过」**，否则 Message 还只有一行高时下面的题卡会被误当成已在屏幕上。等待上限 = 页面的看门狗时间 + 1 秒（默认 6 秒），页面一直不说话也不会永远等；排好版后再多等约 0.15 秒让新高度传到各题卡的位置 |
| W14 | 原生渲染顺带修的 | 仍用原生的地方（题干、Watch 卡片、附件预览）：软换行改为换行（与 `breaks:true` 一致）；行内代码加浅底；文字可选。表格长单元格换行、Mermaid、本地图片留给 WebView 或另案 |
| W16 | 加载中的样子 | 页面在报出第一个高度前整个透明（高度沿用同一段文字上次的高度，没有就是一行高）；超过 0.6 秒还没好，卡片左上角出现一个小转圈，好了就淡入、转圈消失。真机上页面约 0.1–0.2 秒就绪，几乎看不到转圈；冷启动的模拟器要 4–12 秒（机器很忙时更久），所以模拟器上看门狗默认放宽到 30 秒，网页的 UI 测试传 60；其余 UI 测试直接走原生渲染（`-AHWebMarkdown NO`），因为网页晚到几秒会把下面的版面整体推走，点按会落空 |
| W17 | 测试挂钩 | `-AHWebMarkdown NO` 强制走原生渲染（回落路径的测试用），`-AHWebTimeout <秒>` 改看门狗时间；Mock 场景 `markdown`（一份带软换行、行内样式、内联图片、长单元格表格、Mermaid 流程图、代码、引用、嵌套列表和三道题的迁移评估报告）与 `tall`（同一份高 Message + 两道很短的仅选择题，用来证明 W13：不等 Message 排版的话，那两道题会在页面还是一行高时被误判为已看过）。WebView 的内容在无障碍树里是一串平铺的普通元素（没有 WebView 节点），UI 测试靠只有网页才有的标志来区分渲染器：复制按钮标签为「Copy」（原生是「Copy code」）、Mermaid 图是带图中文字的 `Image` |
| W15 | 无障碍 | WebView 内容由 VoiceOver 原生朗读，不额外包装；代码块复制按钮有 `aria-label`（本地化）；Mermaid 图带 `role="img"` 与可读标题（取自图中文字） |

## 4. 不做 / 后置

- 题干、列表卡片、Watch 卡片、Markdown 附件预览改 WebView（用户明确要求保持原生；附件预览若要统一另案）。
- 联网取图、`ahasset://` 本地图片（W5）。
- 预渲染（请求到达时就在后台算好高度）：首屏有约 0.15–0.25 秒的占位，用淡入 + 高度动画掩盖；若实机仍觉得跳，再做预渲染。
- Mermaid `sandbox` 对齐（W6）。
- 搜索 / 查找（Mac 弹窗有 Find，手机用系统「在页面上查找」不在范围）。

## 5. 验收标准

1. 提问页 / 确认页 / 历史详情的正文与 Mac 弹窗在同一份 Markdown 上排版一致：软换行换行、行内代码有底、表格（含长单元格）可读、引用、嵌套列表、代码块（可复制、横滚）、内联图片、Mermaid 图。
2. 长按可选字并复制；点链接先出域名确认。
3. 深色模式、Dynamic Type（含最大辅助字号）下可读；系统切换深浅色时图表重绘。
4. 一页只有一个 WebView；离开页面后 WebView 被释放。
5. 带 Message 的多题表单在 Message 排好版之前不会出现「发送」；排好版后按位置正确判定。
6. WebView 不可用时（加载失败 / 进程被回收超过 2 次 / 超时）自动回落原生渲染，页面照常可答。
7. 断网也能渲染（离线资源、CSP 禁网）。
8. Swift 包测试、模拟器 UI 测试（含新增的 Markdown 场景用例）通过；真机冒烟通过。

## 6. 风险与对策

| 风险 | 对策 |
|---|---|
| 首屏高度未知造成跳动 | 缓存上次高度；高度变化用动画；Message 卡片内容淡入 |
| Mermaid 切换使高度突变约 +500 pt | 绘制前保留源码占位；变化走动画；「看过」判定对内容上方的高度变化本来就是按位置重算 |
| WebContent 进程内存（每个约 20–70 MB） | 一页一个；离开即释放；Mermaid 懒加载 |
| 仓库体积 +≈3.7 MB（Mermaid） | 可接受（仓库 31 MB；Git 压缩后约 1 MB）；升级才增加 |
| 页面脚本异常 / 被利用 | W6：无特权、禁网、锁导航、限长限高 |
| 回落路径从来没人走到 | UI 测试用启动参数强制回落（`-AHWebMarkdown NO`），保证原生路径也被测到 |

## 7. 反馈意见

（后续 review 中产生的调整意见追加于此，标注日期。）
