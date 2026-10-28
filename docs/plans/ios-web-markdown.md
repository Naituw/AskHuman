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
- [ ] `ios/AskHuman/Resources/WebMarkdown/`：`index.html`（CSP、无缩放视口）、`style.css`、`render.js`、`markdown-it.min.js`、`mermaid.min.js`、`VERSIONS.txt`。
- [ ] `ios/Tools/web-markdown/sync.sh`：从 `node_modules/markdown-it/dist/markdown-it.min.js`、`node_modules/mermaid/dist/mermaid.min.js` 复制，并写 `VERSIONS.txt`（含版本号、来源路径、许可证名）；`NOTICE.md` 说明第三方来源与许可证（markdown-it MIT、Mermaid MIT 及其打包依赖）。
- [ ] `project.yml`：把 `WebMarkdown` 作为**文件夹引用**加入 App target（并从普通 sources 里排除，避免被摊平到 bundle 根目录）；`xcodegen generate`。
- [ ] `render.js`：本地化文案由原生传入（`AH.render(source, {semibold, strings})`）；复制按钮用 `aria-label`；限额与安全设置按需求 W6 / W11；`copy` 只在点击里发。

### 1.2 SwiftUI 视图
- [ ] `WebMarkdownView.swift`：对外的 View（`text`、`semibold`、`onSettled`）；度量（高度 / 基线）与缓存；占位高度；高度变化动画；回落到 `MarkdownView`（W12）。
- [ ] `WebMarkdownRepresentable.swift`：`WKWebView` 配置（非持久数据存储、透明、禁滚动 / 缩放、`allowsLinkPreview=false`、Debug 下 `isInspectable`）；`ScriptProxy`（弱引用）；导航锁定与链接转交（W6 / W7）；进程终止重载（最多 2 次）；Dynamic Type 与深浅色变化后重渲；`copy` 限长 + 触感。
- [ ] 本地化文案：`String(localized:)` 取「复制」「显示源码」「显示图表」「正在绘制图表…」「图表太多，显示源码」「图表太大，显示源码」「无法绘制，显示源码」「图表库不可用，显示源码」，并进 `make-catalog.py`。

### 1.3 接入各页面
- [ ] `AnswerScreen.MessageCard`：`ask.isMarkdown` 时用 `WebMarkdownView`；Message 排好版前不统计「看过」（W13；`ReadingTracker.begin` 等 `onSettled`，最多等 3 秒）。
- [ ] `ConfirmScreen.DetailCard`、`HistoryDetailScreen`（Message、确认详情）改用 `WebMarkdownView`；题干仍 `MarkdownView`。
- [ ] 启动参数 `-AHWebMarkdown NO` 强制原生（UI 测试走回落路径）；`AppEnvironment` / `LaunchOptions` 里读取。

### 1.4 原生渲染顺带修（W14）
- [ ] `MarkdownParser`：软换行 → 换行；行内代码加底（`backgroundColor`）与等宽；`AHMarkdownTests` 补测。
- [ ] `MarkdownView`：`MarkdownBlocks` 根上 `.textSelection(.enabled)`。

### 1.5 Mock 场景与测试
- [ ] `MockData.markdownAsk` / `markdownOnly` + `MockBackend.Scenario.markdown`（`-AHScenario markdown`），内容取原型里的迁移评估报告（软换行、行内样式、内联 PNG、长单元格表格、Mermaid 流程图、bash 代码、引用、嵌套列表、三道题）。
- [ ] UI 测试：Markdown 场景能打开并渲染（`app.webViews` 里有正文文字）、长按选字、点链接出域名确认、`-AHWebMarkdown NO` 回落、深色模式截图、最大 Dynamic Type 截图、Message 排好版前没有「发送」。
- [ ] 手工截图（亮 / 暗、中 / 英）备查。

### 1.6 收尾
- [ ] `make-catalog.py` 补中文并重生成 `Localizable.xcstrings`。
- [ ] 文档：`docs/overview-ios.md`（模块表、Markdown 管线、Mock 启动参数 `-AHWebMarkdown`、场景 `markdown`、资源同步命令）、`docs/specs/ios-channel.md`（§6.3 Mermaid 不再是代码块、§6.5 Markdown 预览说明、2026-10-28 反馈记录）。
- [ ] 真机冒烟：装到 iPhone Air，开 Markdown 场景（Mock 构建）或真实请求，看渲染、选字、链接、Mermaid、深色。
- [ ] 本地提交（Conventional Commits，不 push）。

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
- 基线用零尺寸 inline-block 探针放在第一个文本容器开头取；列表 / 引用 / div 嵌套要往里找。
- `simctl io screenshot` 启动后第一帧常是空白，隔一两秒重拍；后台构建要 `block_until_ms: 0`。
- XCUITest 的无障碍查询会拖慢应用；WKWebView 里的文字用 `app.webViews.staticTexts` 查。
