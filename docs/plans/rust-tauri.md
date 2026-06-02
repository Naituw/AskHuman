# 开发计划：HumanInLoop 迁移到 Rust + Tauri（跨平台）

> 对应需求：`docs/specs/rust-tauri.md`
>
> 本计划自包含：实现所需的全部技术与规则细节均写明，可脱离需求文档执行。功能基线 = 当前 Swift 版（`docs/specs/swift-native.md` + 现有 `Sources/AskHuman/` 代码）。

## 一、总体方案与运行形态

用 **Tauri 2.x（Rust 后端 + Vue3 前端）** 实现一个 **单一可执行文件 `AskHuman`**，跨 macOS / Windows / Linux。

- 它是一个安装在 PATH（如 `~/.local/bin/AskHuman`）的命令行可执行文件，同时内嵌 WebView 用于显示窗口。
- 纯信息命令（`--help` / `--version` / 无参报错）**不启动 GUI**，直接终端输出后退出。
- 需要界面时（提问 / 设置），进程内启动 Tauri 事件循环，创建窗口；Rust 侧同时并行运行其他 Channel（Telegram）。拿到**首个终态结果**后，格式化输出到 stdout，`std::process::exit` 退出。
- **单进程模型**（与当前 Swift 版一致；不采用原 Rust 版的「CLI 父进程 + GUI 子进程」方案）。
- **stdout 洁净约束**：所有日志/调试信息一律走 stderr（`eprintln!` / `log` 到 stderr），WebView 不向 stdout 写入；stdout 只输出最终结果区块。

部署目标：macOS 11+ / Windows 10+ / 主流 Linux（带 WebKitGTK）。

## 二、技术选型与依赖

Rust（`Cargo.toml`）：

- `tauri = "2"`（features 按需：窗口、事件；**不**启用 tray-icon / updater）
- `tauri-build = "2"`（build-dependency）
- `serde` / `serde_json`：配置与 IPC 数据
- `tokio = { version = "1", features = ["rt-multi-thread","macros","sync","time"] }`：Telegram 轮询、抢答 oneshot
- `reqwest = { version = "0.12", features = ["json"] }`：Telegram Bot API（手写）
- `uuid = { version = "1", features = ["v4"] }`：request_id
- `dirs = "5"`：home 目录跨平台
- `base64 = "0.22"`：图片解码
- `regex = "1"`：（仅 hook 脚本内用 grep；Rust 侧若需可选）
- `anyhow` / `thiserror`：错误处理
- `dev-dependencies`：`tempfile`（hook / config 单测用临时 HOME）

> 不引入：rmcp、teloxide、rodio、env_logger（用极简 stderr 日志）、schemars、rust-embed（前端走 `frontendDist`）。`profile.release` 用较快设置（`opt-level="z"`、`codegen-units=16`、`incremental=true`、`strip=true`），避免原 Rust 版的慢编译；可另设 `profile.distribution` 做体积优化（按需）。

前端（`package.json`，用 pnpm）：

- `vue@^3`、`vite@^5`、`@vitejs/plugin-vue`、`typescript`、`vue-tsc`
- `@tauri-apps/api@^2`（invoke / event / window）
- `@tauri-apps/cli@^2`（或用 `cargo tauri`）
- `markdown-it`（Markdown 渲染）+ `@types/markdown-it`
- 样式：**纯手写 CSS**（不引组件库）

## 三、目录结构

新工程与现有 Swift 代码并存于仓库根目录（互不冲突）：

```
HumanInLoop/
  # ===== 新增 Tauri 工程 =====
  Cargo.toml
  build.rs
  tauri.conf.json
  capabilities/
    default.json                 # 窗口/事件等权限
  icons/                         # 应用图标（含 .icns/.ico/.png）
  index.html                     # 前端入口
  vite.config.ts
  tsconfig.json
  package.json
  pnpm-lock.yaml
  install.sh                     # mac/Linux 安装（裸二进制 -> ~/.local/bin）
  install-windows.ps1            # Windows 提示/拷贝（可选）
  src/                           # 前端（Vue）
    main.ts
    App.vue                      # 按模式路由到 Popup / Settings
    styles/                      # macOS 风设计系统 CSS
      tokens.css                 # 颜色/间距/圆角/字体变量（深浅色）
      base.css                   # 重置 + 基础排版 + markdown 样式
      controls.css               # 按钮/输入框/开关/分段/卡片
    lib/
      ipc.ts                     # 封装 invoke/event
      markdown.ts                # markdown-it 封装
      types.ts                   # 与 Rust 对齐的 TS 类型
    popup/
      PopupView.vue
      OptionList.vue
      ImageAttachments.vue
    settings/
      SettingsView.vue
      GeneralTab.vue
      IntegrationTab.vue
      ChannelTab.vue
  src-tauri-rs/                  # Rust 后端（见下；实际放 src/rust 或独立，见“注”）
    main.rs
    lib.rs
    cli/
      args.rs                    # 参数解析 + help/version 文案
      output.rs                  # 结果区块格式化
      image_writer.rs            # 图片落盘 + sanitize
    core/
      models.rs                  # AskRequest / ChannelResult / ImageAttachment
      config.rs                  # AppConfig + 读写 ~/.humaninloop/config.json
      paths.rs                   # home/config/temp/cursor 路径
      prompts.rs                 # CLI 参考提示词常量
      version.rs
    app/
      runtime.rs                 # 启动 Tauri、创建窗口、退出 run loop
      coordinator.rs             # 并行启动 Channel + 抢答门闩
    channels/
      channel.rs                 # Channel trait + 终态语义
      popup.rs                   # 弹窗 Channel（与前端 IPC 对接）
      telegram/
        mod.rs                   # Telegram Channel
        client.rs                # reqwest Bot API
        markdown.rs              # MarkdownV2 转义
    integrations/
      cursor_hook.rs             # 安装/移除/状态/打开 + 内嵌脚本字符串
    commands.rs                  # #[tauri::command] 集合（前端调用入口）

  # ===== 现有 Swift 代码（保留至对齐后删除）=====
  Package.swift  Package.resolved  Sources/  Tests/
  # docs/ 共用
```

> **注（路径约定）**：为符合 Tauri 习惯，Rust 源码统一放在 `src-tauri/` 还是 `src/rust/` 由实现时按 Tauri 模板决定；`tauri.conf.json` 的 `build.frontendDist` 指向前端构建产物（`dist/`），`devUrl` 指向 Vite。本计划其余章节用「Rust 侧 / 前端」指代，不依赖具体目录名。前端与 Rust 不要都叫 `src/` 造成混淆——前端用 `src/`，Rust 用 Tauri 默认 `src-tauri/`（最终以此为准）。

## 四、进程 / 运行模型

### 4.1 入口分发（`main.rs`）

读取 `std::env::args()`，**在创建任何窗口前**分发：

- 无参 → stderr `错误: 缺少提问内容\n\n` + 打印帮助 → `exit(1)`
- `--help` / `-h` → 打印帮助 → `exit(0)`
- `--version` / `-v` → 打印版本 → `exit(0)`
- `--settings` → 启动 Tauri（设置模式）
- 第一个 token 以 `-` 开头但未知 → stderr `错误: 未知选项 <x>\n\n` + 帮助 → `exit(1)`
- 其余 → 提问模式

### 4.2 提问主流程

1. 解析参数 → 构造 `AskRequest { id: uuid_v4, message, predefined_options, is_markdown }`
2. 读取配置，得到启用的 Channel 列表（popup / telegram；都没启用兜底 popup）
3. 启动 Tauri 事件循环（`app/runtime.rs`）：
   - 在 `setup` 钩子里：若含 popup Channel，按配置创建弹窗窗口（加载前端 popup 页），并把 `AskRequest` 暂存于全局状态供前端拉取
   - 并行 `start` 所有 Channel（telegram 在 tokio 任务里轮询）
4. **抢答门闩**（`coordinator.rs`）：线程安全（`Mutex<Option<ChannelResult>>` + 原子 `finished`），仅接受**首个**终态结果；收到后对其余 Channel 调 `cancel_by_other()`（关闭窗口 / 停轮询）
5. 拿到首个 `ChannelResult` → 由 `cli/output.rs` 组装区块，图片由 `cli/image_writer.rs` 落盘 → `println!` 输出 → `std::process::exit(code)`

> 退出：因本工具是 CLI，结果产生后直接 `std::process::exit`，无需优雅关闭 Tauri；退出码由结果决定（send/cancel=0，异常=1）。

### 4.3 设置流程

启动 Tauri，创建设置窗口（加载前端 settings 页）。设置窗口关闭即进程退出（`exit(0)`）。

### 4.4 IPC（前端 ↔ Rust，`commands.rs`）

`#[tauri::command]` 列表：

- `get_popup_request() -> AskRequest`：前端 popup 页加载后拉取请求内容
- `submit_popup(payload)`：前端「发送」——payload 含 `selected_options: string[]`、`user_input: string`、`images: ImageAttachment[]`；Rust 转为 `ChannelResult{action: Send, source:"popup"}` 投入门闩
- `cancel_popup()`：前端「取消」/关闭 → `ChannelResult{action: Cancel, source:"popup"}`
- `get_settings() -> AppConfig` / `save_settings(config)`：设置读写（save 立即落盘）
- `cursor_hook_status() -> { installed, hooks_json_exists, supported }`
- `cursor_hook_install() -> Result<String>` / `cursor_hook_uninstall() -> Result<String>` / `cursor_hook_reveal()`
- `telegram_test(cfg) -> Result<String>`：测试连接
- `get_prompt() -> String`：参考提示词
- `app_version() -> String`

窗口关闭事件（`on_window_event` → `CloseRequested`）：popup 窗口被用户关闭按钮关闭时，等价 `cancel_popup()`（除非是被抢答程序化关闭）。

## 五、核心数据模型（`core/models.rs`）

与前端 TS 类型（`lib/types.ts`）一一对应，serde 命名用 camelCase（`#[serde(rename_all="camelCase")]`）以贴合前端：

- `AskRequest { id: String, message: String, predefined_options: Vec<String>, is_markdown: bool }`
- `ImageAttachment { data: String /*base64，可带 data: 前缀*/, media_type: String, filename: Option<String> }`
- `ChannelAction { Send, Cancel }`
- `ChannelResult { action, selected_options: Vec<String>, user_input: Option<String>, images: Vec<ImageAttachment>, source_channel_id: String }`
- `request_id` 用 uuid v4。

## 六、CLI 层（规则需逐字对齐当前 Swift 版）

### 6.1 参数解析（`cli/args.rs`）

手工解析：

- 收集**唯一**位置参数 `message`（多个 → 报错「仅允许一个提问内容参数」）
- `-o` / `--option` 追加选项，缺值 → 报错「`<flag>` 选项缺少参数值」
- `--no-markdown` → `is_markdown=false`（默认 true）
- 未知 flag → 报错「未知选项: `<x>`」
- 缺 message → 报错「缺少提问内容」

帮助 / 版本文案沿用现风格（中文，列出用法/参数/选项/输出格式说明）；`版本` 取 `env!("CARGO_PKG_VERSION")`，格式 `HumanInLoop v<x>`。

### 6.2 输出格式（`cli/output.rs`）

- 成功路径依次输出非空区块，**区块间空行**：
  - `[选择的选项]\n<逗号+空格分隔>`（`", "`）
  - `[用户输入]\n<trim 后原文>`（仅在 trim 后非空）
  - `[图片]\n<路径换行分隔>`
- 三块皆空（且动作=发送）→ `[用户输入]\n用户确认继续`
- 取消路径 → `[状态]\n用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复`
- 异常 → stderr `错误: <描述>`，退出码 1

### 6.3 图片落盘（`cli/image_writer.rs`）

- 目录：`temp_dir()/humaninloop/<request_id>/`（`std::env::temp_dir()`），`create_dir_all`，不清理
- 文件名：优先 `filename`（sanitize），为空则 `img-{index+1}.{ext}`
- `ext` 由 `media_type` 映射：`image/png→png`，`image/jpeg|image/jpg→jpg`，`image/gif→gif`，`image/webp→webp`，`image/bmp→bmp`，`image/svg+xml→svg`，其它→`bin`
- sanitize：取最后一段路径（按 `/` 和 `\` 切分），去掉 `< > : " | ? * \0`，去首尾空白与 `.`；为空则 `img.<ext>`
- base64 解码：若含 `base64,` 取其后；去除所有空白字符后 `STANDARD.decode`
- 返回绝对路径字符串

## 七、Channel 抽象与协调器

### 7.1 Channel trait（`channels/channel.rs`）

```
trait Channel {
    fn id(&self) -> &str;
    fn start(&self, request: AskRequest, sink: ResultSink); // 终态时向 sink 投递一次 ChannelResult
    fn cancel_by_other(&self);                              // 被抢答时收尾，不再投递
}
```

- 「终态」= 用户明确**发送**或**取消**，仅投递一次。
- `ResultSink`：对协调器门闩的句柄（线程安全）。

### 7.2 协调器（`app/coordinator.rs`）

- 输入：`AskRequest` + 已启用 Channel 列表
- 门闩：`Arc<Mutex<...>>`，记录首个结果；后续投递被丢弃
- 收首个结果 → 对 `id != source` 的 Channel 调 `cancel_by_other()` → 触发进程输出与退出
- popup Channel 需在主线程操作窗口；telegram 在 tokio 任务里。门闩跨线程安全。

## 八、本地弹窗 Channel（`channels/popup.rs` + 前端 `popup/`）

### 8.1 Rust 侧

- `start`：把 `AskRequest` 放入全局状态；在主线程创建/显示弹窗窗口（若窗口已由 `setup` 创建则复用）。
- 窗口属性来自配置：`width/height`、`alwaysOnTop`、主题。
- `cancel_by_other`：程序化关闭窗口（标记 closing，避免 CloseRequested 再触发 cancel）。
- 用户操作经 `submit_popup` / `cancel_popup` 命令转成 `ChannelResult`（`source_channel_id="popup"`）投门闩。
- **窗口尺寸记忆**：监听窗口 `Resized`（live resize 结束）事件；当 `popup.rememberSize` 为真，读取当前 inner size 写回配置（`width/height`）。

### 8.2 前端 popup 页（`popup/PopupView.vue`）

布局（从上到下，可滚动 + 固定底部操作条）：

1. 提问内容：`is_markdown` 时用 markdown-it 渲染（HTML），否则纯文本（保留换行、可选中）
2. 预定义选项（非空时）：多选列表，勾选样式（方块 ✓），点击切换（`OptionList.vue`）
3. 「补充说明」多行文本框
4. 「图片附件」区（`ImageAttachments.vue`）：
   - 支持**粘贴**（监听 `paste` 事件，从 `ClipboardItems` 读图片）、**拖拽**（`drop` 读 `dataTransfer.files`）、**选择文件**（`<input type=file accept=image/*>`）
   - 缩略图横向预览 + 删除按钮
   - 每张图片转 base64 → `ImageAttachment{ data, media_type, filename? }`
5. 底部：「取消」「发送」按钮
   - 快捷键：`Cmd/Ctrl+Enter` = 发送；`Esc` = 取消

发送 → 调 `submit_popup`；取消 / 关闭窗口 → `cancel_popup`。

> 图片 media_type 推断：从 File/Blob 的 `type` 或扩展名映射，与 §6.3 一致。

## 九、Telegram Channel（`channels/telegram/`，逐项对齐当前 Swift 版）

### 9.1 会话流程（`mod.rs`）

`start`（在 tokio 任务里）：

1. **发送选项消息**：
   - 文本：`is_markdown` 时用 `markdown.rs` 处理为 MarkdownV2，`parse_mode="MarkdownV2"`；否则原文、无 parse_mode
   - 有预定义选项时附 inline keyboard（每行最多 2 个；按钮文案选中态前缀 `✅ `；`callback_data = "toggle:<option>"`）
   - MarkdownV2 发送失败 → 回退为纯文本重发
   - 记录返回的 `optionsMessageId`
2. **发送操作消息**：文本 `在键盘上点「发送」完成回复，或直接回复文字补充说明`，附 reply keyboard（仅一个按钮 `↗️发送`，`resize_keyboard=true`、`one_time_keyboard=true`）；记录 `operationMessageId`
3. **长轮询**：`getUpdates(offset, timeout=0)`，循环：
   - 正常：处理每条 update，`offset = update_id + 1`，间隔 `sleep(1s)`
   - 出错：`sleep(5s)` 后重试
   - 被 `cancel_by_other` 取消（task abort）时退出

处理单条 update：

- `callback_query`：若带 `data` 且以 `toggle:` 开头 → 切换该选项选中态；`editMessageReplyMarkup` 用新选中态刷新；`answerCallbackQuery`。（chat 不匹配则忽略）
- `message`：chat 匹配且 `message_id > operationMessageId`：
  - 文本 == `↗️发送` → 终态：投递 `ChannelResult{Send, selected_options, user_input?(空则None), images:[]}`（`source="telegram"`）
  - 其他文本 → 累积为 `user_input`
  - 不处理 photo（不接收图片）

`cancel_by_other`：abort 轮询任务。

### 9.2 Bot API 客户端（`client.rs`，reqwest）

- 构造校验：token 非空、chatId 非空且非 `@` 开头、可解析为 `i64`（否则错误「Chat ID 格式无效…」）；`apiBaseUrl` 为空回退 `https://api.telegram.org`
- 方法：`send_message(text, parse_mode?, reply_markup?) -> message_id`、`get_updates(offset) -> Vec<Update(JSON)>`、`answer_callback_query(id)`、`edit_message_reply_markup(message_id, markup)`、`test_connection()`（发一条测试消息，返回成功文案）
- 请求：`POST {base}/bot{token}/{method}`，JSON body，超时 30s；解析 `ok`/`result`/`description`
- 错误文案与现版一致（Bot Token 为空 / Chat ID 为空 / 无效 / API 错误 / 网络错误 / 无法解析响应）

### 9.3 MarkdownV2 转义（`markdown.rs`）

移植当前 Swift 的 `TelegramMarkdown.process`：

1. 保护代码块（```` ``` ... ``` ````）与行内代码（`` `..` ``）为占位符 `CODEBLOCK{i}`
2. 转换：标题 `^#{1,6}\s+(.+)$` → `>$1`；粗体 `**x**` → `*x*`
3. 转义特殊字符（**不**转义 `*`、`>`、`` ` ``）：`_ [ ] ( ) ~ # + - = | { } . !` 前加 `\`
4. 还原占位符

## 十、Markdown 渲染（前端 `lib/markdown.ts`）

- 用 `markdown-it`（建议开启 `linkify`、`breaks` 视情况；默认安全 HTML 转义）渲染弹窗提问内容。
- 代码块/行内代码、表格、引用、列表、标题等样式由 `styles/base.css` 提供（深浅色两套），参考当前 Swift `HTMLMarkdownRenderer` 的配色与排版（代码块不重复背景、链接色、引用左边框、表格边框等）。
- 外部链接点击用系统浏览器打开（Tauri `shell:open` 或拦截 `<a target=_blank>`）。
- `--no-markdown`（`is_markdown=false`）→ 不过 markdown-it，按纯文本渲染（保留换行、可选中）。

## 十一、配置（`core/config.rs`）

- 路径：`~/.humaninloop/config.json`（`dirs::home_dir()`，所有平台一致）；首次不存在用默认值（读时不强制落盘，保存时 `create_dir_all`）
- 写入：临时文件 + 原子 rename；JSON pretty + 键排序
- 容错解码：缺字段走默认、未知字段忽略
- 结构与默认值（serde camelCase）：

```
general:
  theme: "system" | "light" | "dark"     (默认 "system")
  alwaysOnTop: bool                       (默认 true)
channels:
  popup:
    enabled: bool        (默认 true)
    width: number        (默认 560)
    height: number       (默认 620)
    rememberSize: bool   (默认 true)
  telegram:
    enabled: bool        (默认 false)
    botToken: string     (默认 "")
    chatId: string       (默认 "")
    apiBaseUrl: string   (默认 "https://api.telegram.org")
```

> 相比当前 Swift 版：**移除** `general.markdownRenderer` 字段。旧配置含该字段时按「忽略未知字段」处理，不报错。

## 十二、设置界面（前端 `settings/`）

`SettingsView.vue`：顶部分段 Tab（General / 集成 / Channel），内容区滚动。所有改动调 `save_settings` 立即落盘；主题改动实时应用到当前窗口（前端切 CSS + 调 Tauri 设置窗口主题）。

- **General（`GeneralTab.vue`）**：
  - 主题：分段控件（跟随系统 / 浅色 / 深色）
  - 弹窗置顶：开关
  - （不含 Markdown 渲染方式选项）
- **集成（`IntegrationTab.vue`）**：
  - 参考提示词卡片：展示 `get_prompt()` 文案（等宽、可选中、滚动区），「复制」按钮（复制后短暂显示「已复制」约 1.5s）
  - Cursor Hook 卡片：状态徽标（已安装/未安装）、安装 / 移除按钮（互斥显示）、「打开 hooks.json」按钮（`hooks.json` 不存在时禁用）、操作结果文案（成功绿/失败红）
    - **平台不支持（Windows）**：按钮禁用并提示「Windows 暂不支持 Cursor Hook」
- **Channel（`ChannelTab.vue`）**：
  - 本地弹窗卡片：启用开关；启用时显示「记住窗口尺寸」开关 + 默认尺寸调节（宽 360–1200 步进 20、高 360–1400 步进 20）
  - Telegram 卡片：启用开关；启用时显示 Bot Token / Chat ID / API Base URL 输入框 + 「测试连接」按钮（测试中禁用，结果文案 绿/红）
  - 未来扩展占位卡片：「更多通信 Channel 敬请期待」

## 十三、Cursor Hook（`integrations/cursor_hook.rs`）

仅 mac/Linux 生效；Windows 上 `supported=false`，前端禁用按钮。

### 13.1 路径

- 脚本：`~/.cursor/hooks/humaninloop-timeout.sh`
- 配置：`~/.cursor/hooks.json`
- 识别标记（marker）：`humaninloop-timeout.sh`

### 13.2 脚本内容（内嵌字符串，安装时写入 + chmod 0755）

逐字沿用当前 Swift 版脚本（`Sources/AskHuman/Integrations/CursorHook.swift` 的 `scriptContent`）。要点：

- 从 stdin 读 JSON，提取 `.tool_input.command`，按 `python3 → jq → 原样` 回退
- 用 `grep -Eq` 正则识别是否含 `AskHuman` 调用，兼容行首 / 链式 / 引号 / 绝对路径前缀，且不误命中 `AskHumanFoo`：
  - 正则：`(^|[[:space:];&|()\`\"'\\]|/)AskHuman([[:space:]]|$|[\"'\\])`
- 命中 → 输出 `{"updated_input": {"timeout": 86400000}}`；否则 → `{}`；异常一律 `{}` 并 `exit 0`（fail-open）

### 13.3 hooks.json 操作（serde_json）

- 读取或初始化 `{"version":1,"hooks":{}}`
- **安装（upsert）**：在 `hooks.preToolUse` 数组中，若已有条目 `command` 含 marker 则覆盖该条，否则追加 `{"command": <脚本绝对路径>, "matcher": "Shell"}`；保留其他条目与未知字段；原子写回（pretty + 键排序）
- **移除**：过滤掉 `command` 含 marker 的条目；若 `preToolUse` 变空则删该键；并删除脚本文件本身
- **状态**：`preToolUse` 任意条目 `command` 含 marker → 已安装
- **打开 hooks.json（跨平台 reveal）**：
  - macOS：`open -R <hooks.json>`
  - Windows：`explorer /select,<hooks.json>`（本期 Windows 整体禁用 Hook，按钮不可达）
  - Linux：`xdg-open <hooks 目录>`（无「选中」语义，定位到目录）

### 13.4 纯函数可测试性

`upsert_entry` / `remove_entries` / `has_marker` 设计为纯函数（输入/输出 JSON 值），便于单测（用临时 HOME 验证增删幂等、保留他人条目）。

## 十四、参考提示词（`core/prompts.rs`）

逐字沿用当前 Swift 版 `Prompts.cliReference`：

```
- 必须通过 Shell 工具调用 `AskHuman` 命令对我进行询问，禁止直接询问或结束任务询问

AskHuman 命令使用细节：
- 调用方式：`AskHuman "<提问内容>" [-o "<选项1>" -o "<选项2>" ...] [--no-markdown]`
  - 提问内容默认按 Markdown 渲染，需要纯文本时加 --no-markdown
  - 通过 -o 可以追加多个预定义选项，方便我快速点选
- 命令会等待我回应，结果按以下区块结构返回（仅在有内容时出现）：
  [选择的选项]   我勾选的预定义选项
  [用户输入]     我输入的自由文本
  [图片]         我附带图片的本地路径（你可以直接读取这些文件）
  [状态]         仅在我取消时出现，请按其中说明继续询问
- 需求不明确时使用 `AskHuman` 询问澄清，提供预定义选项
- 在有多个方案的时候，需要使用 `AskHuman` 询问，而不是自作主张
- 在有方案/策略需要更新时，需要使用 `AskHuman` 询问，而不是自作主张
- 即将完成请求前必须调用 `AskHuman` 请求反馈
- 在没有明确通过使用 `AskHuman` 询问并得到可以完成任务/结束时，禁止主动结束对话/请求
```

## 十五、UI / 样式（手写 macOS 风设计系统）

`styles/tokens.css` 定义 CSS 变量（深浅色两套，跟随主题切换根类 `light`/`dark`）：

- **字体**：`-apple-system, "SF Pro Text", system-ui, "Segoe UI", "Helvetica Neue", Arial, "PingFang SC", "Microsoft YaHei", sans-serif`
- **强调色**：以 macOS 系统蓝为默认；尽量用 CSS `accent-color` 让原生表单控件着色；focus 用蓝色光晕（`box-shadow` 0 0 0 3px rgba(accent,0.35)）
- **度量**：圆角 6px（卡片 8–10px）、控件高度贴近 macOS、分隔线/次级文字色用半透明
- **配色**：浅色 `#1d1d1f` 文本 / 卡片浅灰底；深色 `#e8e8ea` 文本 / 深灰底（参考当前 `HTMLMarkdownRenderer` 的明暗配色）

`controls.css` 手写：按钮（普通 / prominent）、文本框 / 多行框（圆角描边 + focus 光晕）、开关（Switch，圆形滑块动画）、分段控件（Segmented）、卡片（GroupBox 风）、勾选项。

主题与毛玻璃：

- 主题三态：`system` 用 `prefers-color-scheme` + Tauri 主题事件；`light/dark` 强制
- **毛玻璃**：在 `tauri.conf.json` / 窗口 builder 为 macOS 启用 `windowEffects`（如 `hudWindow` / `sidebar` / `underWindowBackground`），窗口背景半透明；**Windows/Linux 不启用，退化为不透明背景**（CSS 用纯色底）

窗口标题：弹窗 `HumanInLoop`，设置 `HumanInLoop 设置`。

## 十六、跨平台差异汇总

| 能力 | macOS | Windows | Linux |
| --- | --- | --- | --- |
| 弹窗 / WebView | WKWebView | WebView2 | WebKitGTK |
| 毛玻璃 | windowEffects（vibrancy） | 不启用（不透明） | 不启用（不透明） |
| 置顶 / 主题 / 尺寸记忆 | ✓ | ✓ | ✓ |
| 图片粘贴 / 拖拽 / 选择 | Web API | Web API | Web API |
| Telegram | ✓ | ✓ | ✓ |
| Cursor Hook | bash 脚本 | **禁用 + 提示** | bash 脚本 |
| 打开 hooks.json | `open -R` | （禁用） | `xdg-open` 目录 |
| 安装到 PATH | `~/.local/bin`（install.sh，需重签名 codesign） | `.exe` 手动加 PATH | `~/.local/bin`（install.sh） |

## 十七、构建 / 安装 / 分发

- 开发：`pnpm install` → `pnpm tauri dev`（或 `cargo tauri dev`）
- 构建裸二进制（不打包）：`cargo tauri build --no-bundle`（前端先 `pnpm build`）
- `install.sh`（mac/Linux）：构建 → 拷贝 `target/release/AskHuman` 到 `~/.local/bin/` → `chmod +x` →（macOS）`codesign --force --sign -` 重签名 → PATH 提示
- `install-windows.ps1`：构建后拷贝 `AskHuman.exe` 到用户目录并提示加入 PATH（可选）
- GitHub Actions（`.github/`）：三平台矩阵构建产物（mac x64/arm64、win x64、linux x64），供 Win/Linux 用户下载

## 十八、实施步骤（每步可独立验证）

1. **Step 1 脚手架**：Tauri 工程初始化（Cargo + Vite + Vue + TS）、`tauri.conf.json`、`capabilities`、空 `App.vue`、能 `pnpm tauri dev` 起一个窗口；`main.rs` 先实现 `--help`/`--version`/无参报错（纯 CLI，不起 GUI）。
2. **Step 2 核心层**：`core/models`、`config`（读写 + 默认 + 容错 + 单测）、`paths`、`prompts`、`version`。
3. **Step 3 CLI 纯逻辑**：`cli/args`、`cli/output`、`cli/image_writer`（含 sanitize / base64 / ext 映射，全部单测，对齐 Swift/原 Rust 测试用例）。
4. **Step 4 Tauri 运行时 + 弹窗最小闭环**：`app/runtime` + `app/coordinator` + `channels/popup` + 前端 popup 页 + IPC 命令，打通「`AskHuman "问题" -o A -o B` → 弹窗 → 选项/文本/图片 → stdout 区块 / 取消」。
5. **Step 5 设置界面**：三 Tab（General + 集成提示词 + Channel 弹窗部分）+ `get_settings`/`save_settings`，主题实时生效、尺寸记忆。
6. **Step 6 Cursor Hook**：`integrations/cursor_hook`（脚本内嵌、hooks.json 增删幂等、状态、跨平台 reveal、Windows 禁用）+ 集成到设置页 + 单测。
7. **Step 7 Telegram**：`telegram/client` + `markdown` + Channel 会话 + 设置项 + 测试连接 + 与弹窗**并行抢答**联调。
8. **Step 8 样式打磨**：macOS 风设计系统、深浅色、毛玻璃、Markdown 配色。
9. **Step 9 收尾**：README、`install.sh` / Windows 脚本、GitHub Actions、macOS 端到端验证；确认对齐后再删除 Swift 代码（`Package.swift`/`Package.resolved`/`Sources/`/`Tests/`）。

## 十九、测试与验证

- **单元测试（Rust）**：参数解析、输出格式化、图片命名/落盘 sanitize、base64 解码、Telegram MarkdownV2 转义、Cursor Hook 的 hooks.json 增删幂等（临时 HOME）、config 容错解码。
- **手动验证（macOS 优先）**：
  - `AskHuman "问题" -o A -o B`：弹窗多选/文本/图片（粘贴+拖拽+选择），验证三类区块与取消路径、退出码
  - `--no-markdown` 纯文本渲染
  - 设置三 Tab：主题/置顶实时生效、尺寸记忆、提示词复制、Telegram 测试连接
  - Cursor Hook 安装后在 Cursor 中真实触发 24h timeout；移除后仅删本应用条目（保留他人 hook）
  - Telegram 启用后与本地弹窗并行，任一端先答即采用、另一端自动收尾
  - stdout 仅含结果区块（无日志污染）
- **跨平台**：Win/Linux 通过 CI 构建产物冒烟（窗口能起、提问闭环可用、Hook 在 Win 正确禁用）。

## 二十、风险与对策

- **stdout 被 Tauri/WebView 日志污染** → 所有日志走 stderr；CI/手动校验 stdout 洁净；必要时在进入 GUI 前重定向底层库噪音到 /dev/null（参考 Swift 版对 AppKit 噪音的处理）。
- **裸二进制（无 .app bundle）能否正常起 WebView/获焦** → 原 Rust 版已用 `--no-bundle` 装到 `~/.local/bin` 验证可行；macOS 拷贝后需 `codesign` 重签名。
- **Linux WebKitGTK 依赖** → 文档注明系统依赖；CI 安装对应库。
- **窗口尺寸记忆与多显示器** → 仅记尺寸不记位置（与当前 Swift 版一致，窗口 `center`）。

## 二十一、编译性能分析（开发完成后进行）

> 用户反馈：先按本计划实现，**完成后再专门分析编译性能**，根据实测决定是否引入下列优化（现阶段不实施）。

- **现状预期**：已砍掉重依赖（无 teloxide / rmcp / rodio / schemars / env_logger），依赖树远小于原 Rust 版；dev 走 `cargo tauri dev`（增量）+ Vite HMR（改前端不重编 Rust）；`install.sh` 的 release **不启用 LTO**。Tauri / wry 的首次冷编译（数分钟）属固有成本，无法消除。

### 实测结果（2026-06-03）

机器：Apple M1 Ultra（16P+4E，20 核）/ 64GB / macOS 26.5.1；rustc 1.88.0、cargo 1.88.0、node 20.19.2、pnpm 10.11.1。

| 场景 | 命令 | 耗时（real） |
| --- | --- | --- |
| 冷编译 debug（全量，`cargo clean` 后） | `cargo build` | **42.5s** |
| 冷编译 release（`install.sh` 等效，opt-level=z 无 LTO） | `cargo build --release` | **44.6s** |
| 增量 debug（改 1 个 `.rs`） | `cargo build` | **2.2s** |
| 增量 release（改 1 个 `.rs`） | `cargo build --release` | **1.3s** |
| 仅改前端 | `pnpm build`（vue-tsc + vite，vite 自身 0.54s） | **1.9s** |

产物：`AskHuman`（release，已 strip）**5.1MB**；`target/`（debug+release）约 2.1GB。

**结论**：冷编译约 45s（M1 Ultra），远优于原 Rust 版（teloxide/rmcp/rodio 等重依赖导致的数分钟级 install.sh 编译）；增量与前端改动均为秒级。**当前不引入下列候选优化**（依赖已足够精简，收益有限、复杂度更高）。后续若在更弱机器（如 CI、Intel mac）或冷编译显著变慢时，再按需启用。

- **候选优化（按需再引入，当前不实施）**：
  - 用 `ureq`（阻塞 HTTP）+ `std::thread` 替代 `tokio` + `reqwest`，去掉两大异步依赖（Telegram 长轮询改阻塞线程实现）
  - 本地构建缓存 `sccache`；更快链接器（macOS `lld` / Linux `mold`）
  - 精简 `tauri` / `reqwest` 的 features；调优 `profile.dev` 与 release（无 LTO）参数
- **产出**：见上「实测结果」。最终采纳的优化项：**无**（保持现状）。
