# 开发计划：HumanInLoop Swift Native 版本

> 对应需求：`docs/specs/swift-native.md`

## 一、总体方案

用 Swift Package Manager 构建一个 **仅 macOS** 的单一可执行文件 `AskHuman`。应用生命周期由 AppKit（`NSApplication`）控制，界面用 SwiftUI 嵌入 `NSHostingView`/`NSHostingController` 呈现。提问能力通过可扩展的「通信 Channel」抽象实现，初期内置「本地弹窗」和「Telegram」两个 Channel。不实现 MCP 服务器。

运行形态：

- 这是一个安装在 PATH（如 `~/.local/bin/AskHuman`）的普通命令行可执行文件。
- 当需要展示窗口时，进程在内部启动 `NSApplication`（设置 activation policy 为 `.regular`），显示窗口、运行 run loop，直到拿到结果后打印并退出。
- 纯信息类命令（`--help` / `--version` / 无参报错）不启动 GUI，直接走终端输出。

部署目标：macOS 13+（确保 SwiftUI/AttributedString 能力可用）。

## 二、目录结构

```
HumanInLoopNative/
  Package.swift
  Sources/AskHuman/
    main.swift                      // 入口：解析 argv，分发
    CLI/
      ArgumentParser.swift          // 解析提问参数与各 flag
      OutputFormatter.swift         // 组装 [选择的选项]/[用户输入]/[图片]/[状态]
      ImageWriter.swift             // 图片附件落盘到临时目录
      Help.swift                    // 帮助/版本文案
    Core/
      Models.swift                  // AskRequest / ChannelResult / ImageAttachment 等
      AppConfig.swift               // 配置模型 + 读写 ~/.humaninloop/config.json
      MarkdownRenderer.swift        // markdown -> AttributedString
      Paths.swift                   // 临时目录、配置目录、可执行文件路径
    App/
      AppBootstrap.swift            // 启动 NSApplication、激活、运行 run loop
      AppDelegate.swift             // NSApplicationDelegate
    Channels/
      InteractionChannel.swift      // Channel 协议 + 结果/事件类型
      ChannelCoordinator.swift      // 并行启动启用的 Channel，抢答取首个终态结果
      PopupChannel.swift            // 本地弹窗 Channel
      TelegramChannel.swift         // Telegram Channel
      Telegram/
        TelegramClient.swift        // 基于 URLSession 的 Bot API 客户端
        TelegramMarkdown.swift      // MarkdownV2 转义
    UI/
      Popup/
        PopupWindowController.swift
        PopupView.swift             // SwiftUI 弹窗界面
        PopupViewModel.swift
      Settings/
        SettingsWindowController.swift
        SettingsView.swift          // TabView：General / 集成 / Channel
        SettingsViewModel.swift
      Theme.swift                   // 主题应用（appearance）
    Integrations/
      CursorHook.swift              // 安装/移除/状态/打开 hooks.json + 内嵌脚本字符串
    Constants/
      Prompts.swift                 // CLI 参考提示词文案
      Version.swift                 // 版本号常量
  docs/specs/swift-native.md
  docs/plans/swift-native.md
  README.md
```

`Package.swift`：定义一个 `executableTarget` `AskHuman`，`platforms: [.macOS(.v13)]`，不引入第三方依赖（Markdown 用 Foundation 内置 AttributedString，Telegram 用 URLSession）。

## 三、核心数据模型（`Core/Models.swift`）

- `AskRequest`：`id: String`、`message: String`、`predefinedOptions: [String]`、`isMarkdown: Bool`
- `ImageAttachment`：`data: String`（base64）、`mediaType: String`、`filename: String?`
- `ChannelResult`：表示某 Channel 的终态回答
  - `action: .send | .cancel`
  - `selectedOptions: [String]`
  - `userInput: String?`
  - `images: [ImageAttachment]`
  - `sourceChannelId: String`
- `request_id` 用 UUID 生成。

## 四、CLI 层

### 4.1 入口分发（`main.swift`）

读取 `CommandLine.arguments`，按需求表分发：

- 无参 → stderr 输出 `错误: 缺少提问内容`，打印帮助，`exit(1)`
- `--help` / `-h` → 打印帮助，`exit(0)`
- `--version` / `-v` → 打印版本，`exit(0)`
- `--settings` → 进入设置流程（启动 GUI）
- 其他 → 进入提问流程

### 4.2 参数解析（`ArgumentParser.swift`）

手工解析（不引入 ArgumentParser 依赖，保持与原版风格一致）：

- 收集位置参数 message（仅允许一个，多个报错）
- `-o`/`--option` 追加选项，缺参报错
- `--no-markdown` 置 `isMarkdown = false`
- 未知 flag 报错

### 4.3 提问主流程

1. 解析参数 → 构造 `AskRequest`（含新 UUID）
2. 读取配置，得到启用的 Channel 列表
3. 调用 `ChannelCoordinator.run(request:)` 启动 GUI（如本地弹窗启用）并并行启动其他 Channel，等待首个终态 `ChannelResult`
4. 拿到结果后由 `OutputFormatter` 输出区块，图片由 `ImageWriter` 落盘
5. 按结果设置退出码

### 4.4 输出格式（`OutputFormatter.swift`）

- 成功路径：依次输出 `[选择的选项]` / `[用户输入]` / `[图片]`，仅非空区块输出，区块间空行
- 三块皆空但为「发送」：输出 `[用户输入]\n用户确认继续`
- 取消路径：输出 `[状态]\n用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复`，退出码 0
- 异常路径：stderr 输出 `错误: <描述>`，退出码 1

### 4.5 图片落盘（`ImageWriter.swift`）

- 目录：`FileManager` 的临时目录 `humaninloop/<request_id>/`
- 命名：优先用 `filename`（去路径分隔符与危险字符做 sanitize）；为空则 `img-{index}.{ext}`，`ext` 由 `mediaType` 映射（png/jpg/gif/webp/bmp/svg/bin）
- base64 解码（兼容 `data:...;base64,` 前缀）后写盘，返回绝对路径

## 五、AppKit 生命周期（`App/`）

- `AppBootstrap`：负责 `NSApplication.shared`、`setActivationPolicy(.regular)`、设置 `AppDelegate`、`activate(ignoringOtherApps: true)`，并 `app.run()`。
- 提问/设置流程都在该 run loop 内进行。
- 提问流程在拿到结果后，通过自定义机制（在主线程回调里 `NSApp.stop(nil)` + 关闭窗口）退出 run loop，回到 CLI 层打印结果。
- 通过 `.regular` 策略确保窗口可成为 key window、文本框可输入；交互期间会短暂出现 Dock 图标，结束即退出。

## 六、通信 Channel 抽象（`Channels/`）

### 6.1 协议（`InteractionChannel.swift`）

定义一个协议，关键能力：

- `var id: String`
- `func start(request: AskRequest, completion: @escaping (ChannelResult) -> Void)`：发起本 Channel 的询问；得到终态回答时回调一次 `ChannelResult`
- `func cancelByOtherChannel()`：当其他 Channel 已抢答，用于收尾（关闭窗口 / 停止轮询），不再回调结果

并定义 Channel 的「终态」语义：仅在用户明确「发送」或「取消」时回调一次结果。

### 6.2 协调器（`ChannelCoordinator.swift`）

- 输入：`AskRequest` + 已启用的 Channel 实例列表
- 行为：
  - 在主线程启动 GUI（若包含本地弹窗 Channel）
  - 并行 `start` 所有 Channel
  - 用一个「只接受首个结果」的门闩（线程安全，已完成则丢弃后续回调）记录首个 `ChannelResult`
  - 收到首个结果后，对其余 Channel 调用 `cancelByOtherChannel()`，结束 run loop
- 输出：首个 `ChannelResult`
- 若没有任何 Channel 启用（异常配置）：回退为强制启用本地弹窗，保证可用

### 6.3 本地弹窗 Channel（`PopupChannel.swift` + `UI/Popup/`）

- `start` 时在主线程创建 `PopupWindowController`，加载 `PopupView`（SwiftUI），把请求注入 `PopupViewModel`
- `PopupViewModel` 持有：`selectedOptions`、`userInput`、`images`、以及发送/取消回调
- `PopupView` 布局：
  - 顶部：Markdown 渲染后的提问内容（`MarkdownRenderer` → `AttributedString` → SwiftUI `Text`）
  - 中部：预定义选项（多选，按钮/勾选样式）
  - 文本输入框（多行）
  - 图片区：支持粘贴（监听 `NSPasteboard`）、拖拽（drop）、文件选择；缩略图预览 + 删除
  - 底部：「发送」「取消」按钮
- 置顶：根据 General 设置设 `window.level = .floating`
- 主题：根据 General 设置设置 `NSApp.appearance`
- 「发送」→ 收集状态构造 `ChannelResult(action: .send, ...)` 回调；「取消」/关闭窗口 → `ChannelResult(action: .cancel)`
- `cancelByOtherChannel` → 关闭窗口、不回调

图片采集：把 `NSImage`/文件数据转为对应 `mediaType` + base64，封装 `ImageAttachment`。

### 6.4 Telegram Channel（`TelegramChannel.swift` + `Channels/Telegram/`）

行为对齐原版：

- `start`：
  1. 发送「选项消息」：内容经 `TelegramMarkdown` 处理（开启 markdown 时用 MarkdownV2，并做转义）；有预定义选项时附 inline keyboard（每行最多 2 个，`callback_data = "toggle:<option>"`）
  2. 发送「操作消息」：附 reply keyboard，仅含「↗️发送」按钮（无「继续」，因为不做继续回复），记录其 message_id 用于后续消息过滤
  3. 启动长轮询任务
- 长轮询（`getUpdates` offset 递增，约 1s 间隔，出错退避 5s）：
  - `callback_query` 且 `data` 以 `toggle:` 开头 → 切换该选项的选中态，`answerCallbackQuery`，并 `editMessageReplyMarkup` 用 ✅ 前缀反映选中态
  - `message` 且 chat 匹配、id 大于操作消息 id：
    - 文本为「↗️发送」→ 终态：构造 `ChannelResult(action: .send, selectedOptions, userInput)` 回调
    - 其他文本 → 累积为 `userInput`
  - 不处理 photo（与原版一致，不接收图片）
- `cancelByOtherChannel`：停止轮询任务（取消 Task / 关闭定时器）

`TelegramClient.swift`（URLSession）：实现 `sendMessage`、`sendMessage(replyMarkup:)`、`getUpdates(offset:)`、`answerCallbackQuery`、`editMessageReplyMarkup`、连接测试 `getMe`/发送测试消息；支持自定义 `apiBaseUrl`（默认 `https://api.telegram.org`）。Chat ID 解析为 Int64，`@username` 不支持时报错（与原版一致）。

`TelegramMarkdown.swift`：移植原版 `process_telegram_markdown` 的 MarkdownV2 转义规则。

## 七、Markdown 渲染（`Core/MarkdownRenderer.swift`）

- 用 Foundation `AttributedString(markdown:options:)` 解析（`interpretedSyntax = .full`，失败时回退为按纯文本展示），转为 SwiftUI `Text` 显示。
- 关闭 markdown（`--no-markdown`）时直接按纯文本渲染。
- 该渲染仅用于本地弹窗；Telegram 侧用 `TelegramMarkdown`。

## 八、配置（`Core/AppConfig.swift`）

- 路径：`~/.humaninloop/config.json`（首次读取不存在时用默认值并落盘）
- 结构（Codable）：
  - `general`：`theme`（`system|light|dark`）、`alwaysOnTop: Bool`
  - `channels`：
    - `popup`：`enabled: Bool`（默认 true）、窗口尺寸相关字段
    - `telegram`：`enabled: Bool`（默认 false）、`botToken`、`chatId`、`apiBaseUrl`
- 提供加载/保存方法；保存采用「写临时文件 + 原子 rename」避免半写入。
- 未知字段保留（用容错解码，缺字段走默认值）。

## 九、设置界面（`UI/Settings/`）

- `SettingsView`：`TabView`，三个 Tab。
- **General**：
  - 主题选择（浅色 / 深色 / 跟随系统）→ 实时应用 `NSApp.appearance`
  - 置顶开关
- **集成（Integration）**：
  - 参考提示词卡片：展示 `Constants/Prompts.swift` 的 CLI 提示词，提供「复制」按钮（写入 `NSPasteboard`）
  - Cursor Hook 卡片：状态（已安装 / 未安装）、安装、移除、打开 hooks.json
- **Channel**：
  - 弹窗 Channel：启用开关、窗口尺寸设置
  - Telegram Channel：启用开关、Bot Token、Chat ID、API Base URL、测试连接（调用 `TelegramClient` 发送测试消息，提示成功/失败）
  - 预留区块用于未来扩展 Channel
- 所有改动写回配置文件；`SettingsViewModel` 负责绑定与持久化。

## 十、Cursor Hook（`Integrations/CursorHook.swift`）

对齐原版逻辑（仅 macOS）：

- 脚本内容以 Swift 字符串常量内嵌；安装时写入 `~/.cursor/hooks/humaninloop-timeout.sh` 并 `chmod 0755`。
- 脚本行为：从 stdin 读 JSON，提取 `tool_input.command`，用正则识别是否含 `AskHuman` 调用（兼容行首、链式、绝对路径、带引号；不误命中 `AskHumanFoo`、`AskHuman_log.txt`）；命中输出 `{"updated_input": {"timeout": 86400000}}`，否则输出 `{}`；任何异常一律输出 `{}` 并 0 退出（fail-open）。优先 `python3` 解析 JSON，缺失时退化为 grep 兜底。
- `hooks.json` 操作（`serde_json` 等价用 `JSONSerialization`/Codable）：
  - 安装：读取或初始化 `{"version":1,"hooks":{}}`；在 `hooks.preToolUse` 数组中，若已有 `command` 含 `humaninloop-timeout.sh` 则覆盖该条，否则追加 `{"command": <脚本绝对路径>, "matcher": "Shell"}`；原子写回，保留其他条目。
  - 移除：过滤掉 `command` 含 `humaninloop-timeout.sh` 的条目；若 `preToolUse` 空则删键；并删除脚本文件本身。
  - 状态：`preToolUse` 中任意条目 `command` 含 `humaninloop-timeout.sh` 即「已安装」。
  - 打开 hooks.json：`open -R <path>`（Finder 中定位）；文件不存在时按钮禁用并提示。
- 写入使用临时文件 + 原子 rename。

## 十一、参考提示词（`Constants/Prompts.swift`）

提供 CLI 调用提示词文案（语气对齐原版 CLI 版），核心要点：

- 必须通过 Shell 调用 `AskHuman` 询问，禁止直接询问或结束任务询问
- 调用方式 `AskHuman "<提问内容>" [-o "<选项>" ...] [--no-markdown]`
- 结果区块结构说明：`[选择的选项]` / `[用户输入]` / `[图片]` / `[状态]`
- 需求不明确 / 多方案 / 方案更新 / 即将完成前都要用 `AskHuman` 询问
- 未得到明确结束许可前禁止主动结束

## 十二、实施顺序

每一步可独立 `swift build` 验证：

1. **Step 1**：`Package.swift` + 目录骨架 + `main.swift` 仅处理 `--help`/`--version`/无参报错（纯 CLI，可编译运行）
2. **Step 2**：`Core/Models`、`AppConfig`、`Paths`、`Prompts`、`Version`
3. **Step 3**：`ArgumentParser` + `OutputFormatter` + `ImageWriter`（提问参数解析与输出格式，纯逻辑可单测）
4. **Step 4**：AppKit 引导（`AppBootstrap`/`AppDelegate`）+ `MarkdownRenderer`
5. **Step 5**：Channel 抽象 + 协调器 + 本地弹窗 Channel + `PopupView`（打通「提问 → 弹窗 → 输出」最小闭环）
6. **Step 6**：设置界面三 Tab（General + 集成 + Channel 的弹窗部分）
7. **Step 7**：Cursor Hook（安装/移除/状态/打开）
8. **Step 8**：Telegram Channel（client + markdown + 设置项 + 测试连接 + 并行抢答联调）
9. **Step 9**：README、安装脚本（`swift build -c release` → 拷贝到 `~/.local/bin/AskHuman`）、收尾测试

## 十三、测试与验证

- 单元测试：参数解析、输出格式化、图片命名/落盘 sanitize、Telegram MarkdownV2 转义、Cursor Hook 的 hooks.json 增删幂等（临时 HOME）
- 手动验证：
  - `AskHuman "问题" -o A -o B` 弹窗交互，验证三类输出与取消路径
  - 设置界面三 Tab 行为；主题与置顶生效
  - Cursor Hook 安装后在 Cursor 中真实触发 24h timeout，移除后仅删除本应用条目
  - Telegram 启用后与本地弹窗并行，任一端抢答正确收尾
