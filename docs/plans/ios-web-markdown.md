# 计划：iOS 请求正文改用 WebView 渲染 Markdown

> 关联需求：`docs/specs/ios-web-markdown.md`
> 只改 iOS（`ios/`）；不动 Mac 与线协议。计划描述方案，代码以实现为准。进度在每步前打勾，做完一步就改。

## 0. 起点

- 原生管线：`AHMarkdown`（`MarkdownParser` → `MarkdownBlock`）+ `ios/AskHuman/Views/Markdown/MarkdownView.swift`。
- 原型（仓库外，`/Users/wutian/Developer/HumanInLoop-ios-web-md-proto/ios/`）已验证：页面三件套（`index.html` / `style.css` /
  `render.js`）、`WebMarkdownView.swift`（`UIViewRepresentable` + 高度 / 基线上报）、Mermaid 懒加载、表格外框。
  正式实现从原型移植，去掉原型专用的 `WebStats` / `WebWarmUp` / 日志 / `debugTables`。
- 模拟器 UDID `32AE9DA4-B06A-4972-A5BC-A9EB5CA3EA9E`（AskHuman iOS Dev，iOS 27.0）；真机 iPhone Air `00008150-00043D6A2E12401C`。

## 1. 步骤

### 1.1 资源与同步脚本
- [x] `ios/AskHuman/Resources/WebMarkdown/`：`index.html`（CSP、无缩放视口）、`style.css`、`render.js`、`markdown-it.min.js`、`mermaid.min.js`、`VERSIONS.txt`。
- [x] `ios/Tools/web-markdown/sync.sh`：从 `node_modules/markdown-it/dist/markdown-it.min.js`、`node_modules/mermaid/dist/mermaid.min.js` 复制，并写 `VERSIONS.txt`（含版本号、来源路径、许可证名）；`NOTICE.md` 说明第三方来源与许可证（markdown-it MIT、Mermaid MIT 及其打包依赖）。
- [x] `project.yml`：把 `WebMarkdown` 作为**文件夹引用**加入 App target（并从普通 sources 里排除，避免被摊平到 bundle 根目录）；`xcodegen generate`。
- [x] `render.js`：本地化文案由原生传入（`AH.render(source, {strings})`，没有 `semibold` 选项：题干保持原生后用不到）；复制按钮用 `aria-label`；限额与安全设置按需求 W6 / W11；`copy` 只在点击里发；Mermaid 图的 `aria-label` 取图中 `text` / `foreignObject` 的文字。

### 1.2 SwiftUI 视图
- [x] `WebMarkdownView.swift`：对外的 View（`text`、`onSettled`）；占位高度与上次高度缓存（`WebHeights`）；高度变化动画；超过 0.6 秒还没好显示小转圈；回落到 `MarkdownView`（W12）。基线探针已去掉（题干保持原生，用不到）。
- [x] `WebPage.swift`（原计划叫 `WebMarkdownRepresentable.swift`）：`WKWebView` 配置（非持久数据存储、透明、禁滚动、`allowsLinkPreview=false`、Debug 下 `isInspectable`）；`ScriptProxy`（弱引用）；导航锁定与链接转交（W6 / W7）；进程终止重载（最多 2 次）与看门狗；Dynamic Type 变化后重渲（深浅色由页面自己的 `prefers-color-scheme` 监听重绘）；`copy` 限长 + 触感。
- [x] 本地化文案：`String(localized:)` 取「复制」「显示源码」「显示图表」「正在绘制图表…」「图表太多，显示源码」「图表太大，显示源码」「无法绘制，显示源码」「图表库不可用，显示源码」，并进 `make-catalog.py`。

### 1.3 接入各页面
- [x] `AnswerScreen.MessageCard`：`ask.isMarkdown` 时用 `WebMarkdownView`；Message 排好版前不统计「看过」（W13；`ReadingTracker.begin(awaitingMessage:patience:)` 等 `onSettled`，最多等看门狗时间 + 1 秒）。
- [x] `ConfirmScreen.DetailCard`、`HistoryDetailScreen`（Message、确认详情）改用 `WebMarkdownView`；题干仍 `MarkdownView`。
- [x] 启动参数 `-AHWebMarkdown NO` 强制原生（UI 测试走回落路径），`-AHWebTimeout <秒>` 改看门狗时间；`AppEnvironment` / `LaunchOptions` 里读取。

### 1.4 原生渲染顺带修（W14）
- [x] `MarkdownParser`：软换行 → 换行；行内代码加底（`backgroundColor`）与等宽；`AHMarkdownTests` 补测。
- [x] `MarkdownView`：`MarkdownBlocks` 根上 `.textSelection(.enabled)`。

### 1.5 Mock 场景与测试
- [x] `MockData.markdownAsk` / `markdownOnly` + `MockBackend.Scenario.markdown`（`-AHScenario markdown`），内容取原型里的迁移评估报告（软换行、行内样式、内联 PNG、长单元格表格、Mermaid 流程图、bash 代码、引用、嵌套列表、三道题）；另有 `tall`：同一份 Message + 两道很短的纯选择题，证明 W13。
- [x] UI 测试（`AskHumanWebUITests`）：Markdown 场景由网页渲染（用只有网页才有的标志判断，见需求 W17）、`-AHWebMarkdown NO` 回落、点链接出域名确认、长按选字出「Copy」、确认详情由网页渲染、最大 Dynamic Type、深色、中文页面文字、Message 排好版前不出现「发送」（`tall`，做过变异验证：去掉等待后这条会失败）、「下一题」从高 Message 走到最后一题。
- [x] 手工截图（亮 / 暗、中 / 英、辅助字号、现场切换 Dynamic Type）：模拟器，见验收报告。

### 1.6 收尾
- [x] `make-catalog.py` 补中文并重生成 `Localizable.xcstrings`。
- [x] 文档：`docs/overview-ios.md`（模块表、Markdown 管线、Mock 启动参数 `-AHWebMarkdown`、场景 `markdown`、资源同步命令）、`docs/specs/ios-channel.md`（§6.3 Mermaid 不再是代码块、§6.5 Markdown 预览说明、2026-10-28 反馈记录）。
- [ ] 真机冒烟：装到 iPhone Air，开 Markdown 场景（Mock 构建）或真实请求，看渲染、选字、链接、Mermaid、深色。（2026-10-29 凌晨 iPhone Air 没有连着，`devicectl list devices` 里没有它，所以这一项没做；手机连上后用 `ios/` 下的 `xcodebuild … -destination 'platform=iOS,id=00008150-00043D6A2E12401C'` 构建并 `devicectl device install app` 即可。）
- [x] 本地提交（Conventional Commits，不 push）：`feat(ios): draw request bodies with a web page`、`fix(ios): keep line breaks and mark inline code in native markdown`。

## 2. 验证命令

```bash
cd ios && xcodegen generate
(cd Packages/AskHumanKit && swift test)
xcodebuild build -project AskHuman.xcodeproj -scheme AskHuman -derivedDataPath build/DerivedData \
  -destination 'platform=iOS Simulator,id=32AE9DA4-B06A-4972-A5BC-A9EB5CA3EA9E'
xcodebuild test -project AskHuman.xcodeproj -scheme AskHuman -derivedDataPath build/DerivedData \
  -destination 'platform=iOS Simulator,id=32AE9DA4-B06A-4972-A5BC-A9EB5CA3EA9E' -only-testing:AskHumanUITests/<类>/<用例>
```

## 3. 已知坑（来自原型）

- 表格不加外框与 `width:max-content; max-width:34em; min-width:4.5em` 时，短列会被挤成一个字宽。
- Mermaid 换图后高度约 +500 pt，发生在首次布局后约 0.1 秒：占位 + 动画。
- （原型里用过的「第一行基线」探针已在正式版去掉：题干保持原生，用不到。）
- `simctl io screenshot` 启动后第一帧常是空白，隔一两秒重拍；后台构建要 `block_until_ms: 0`。
- WKWebView 里的内容在无障碍树里是一串平铺的普通元素（StaticText / Link / Image / Button），**没有 WebView 节点**，`app.webViews` 永远找不到：测试要靠只有网页才有的标志（复制按钮「Copy」、Mermaid 图的 `Image`）；XCUITest 的无障碍查询还会拖慢应用。
- 冷启动的模拟器第一次起 WebContent 要 4–12 秒（机器忙时更久），超过 5 秒的看门狗就会回落原生：模拟器上看门狗默认放宽到 30 秒，网页的 UI 测试传 `-AHWebTimeout 60`；通用 UI 测试用 `-AHWebMarkdown NO`（网页晚到会把下面的版面整体推走，点按落空）。
- `evaluateJavaScript` 的 async 重载遇到 `undefined` 结果会崩，用回调版。
