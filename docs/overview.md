# AskHuman 项目概览（供 agent 参考）

> 跨平台「Human-in-the-loop」工具：命令行 `AskHuman` 在需要人类确认/补充时弹出窗口收集回应，并把结果按固定区块格式写到 stdout 供 AI 读取。

## 技术栈与形态

- **Tauri 2**：Rust 后端 + WebView 前端，单一可执行文件 `AskHuman`，跨 macOS / Windows / Linux。
- **前端**：Vue 3 + Vite + TypeScript，纯手写 macOS 风 CSS（无组件库）。
- **运行模型**：单进程。既是 CLI（纯信息命令直接终端输出、不起 GUI），又能进程内启动 Tauri 事件循环弹窗。**stdout 只输出结果区块，所有日志走 stderr。**

## 目录结构

```
AskHuman/
  vite.config.ts  package.json  tsconfig.json    （Vite root=src，构建产物输出到根 dist/）
  scripts/                   install.sh / install-windows.ps1 / publish.sh / bump-version.mjs
  docs/wiki/                 用户向配置文档（中英双语）；docs/specs · docs/plans 为开发文档
  .github/workflows/         build.yml（三平台 CI 构建）/ release.yml（发版）

  src/                       前端（Vite 根目录）
    index.html               前端入口（含消除白闪/毛玻璃的内联关键样式 + 平台探测脚本）
    main.ts                  挂载 App，引入三套样式
    App.vue                  按 URL ?view=popup|settings 路由
    views/PopupView.vue      弹窗：顶部导航栏 + Markdown/选项/文本/图片 + -f 附件区(选中/打开/
                             预览/拖出/右键) + 拖入回复文件胶囊 + 底部操作条
    views/SettingsView.vue   设置：通用 / 集成 / 通信渠道 三 Tab
    lib/ipc.ts               invoke 封装（与后端命令一一对应）
    lib/types.ts             与 Rust 模型对齐的 TS 类型
    lib/markdown.ts          markdown-it 渲染
    lib/theme.ts             applyTheme（切类）/ fileToDataUrl
    styles/{tokens,base,controls}.css   设计 token / 重置+Markdown / 控件

  src-tauri/                 Rust 后端
    Cargo.toml               依赖（tauri[macos-private-api]、reqwest、tokio、dark-light、libc、
                             tauri-plugin-drag、macOS: objc2 / objc2-foundation / objc2-app-kit…）
    tauri.conf.json          frontendDist=../dist；app.macOSPrivateApi=true
    capabilities/default.json 窗口权限（含 start-dragging / set-always-on-top / drag:default）
    src/
      main.rs                入口：声明模块，调用 cli::dispatch()
      macos_quicklook.rs     (macOS) 原生 QLPreviewPanel 预览 + 文件系统图标(file_icon_png_base64)
      macos_menu.rs          (macOS) -f 附件原生右键菜单（NSMenu，Finder 风格）
      cli/
        mod.rs               argv 分发（--help/--version/--settings/无参/提问）
        args.rs              提问参数解析（message / -o / --no-markdown / -f）
        file_attachment.rs   -f 路径解析/校验（~/相对路径 → 绝对路径 + 元信息）
        output.rs            结果区块格式化（[选择的选项]/[用户输入]/[图片]/[文件]/[状态]）
        image_writer.rs      图片 base64 落盘 + 文件名 sanitize + ext 映射
        help.rs              帮助/版本文案
      models.rs              AskRequest(含 files) / FileAttachment / ChannelResult(含 files) /
                             ImageAttachment / ChannelAction / source_name()
      config.rs              AppConfig 读写 ~/.askhuman/config.json（原子写、容错解码；旧 ~/.humaninloop 自动回退读取）
      paths.rs               home/config/temp 路径
      prompts.rs             CLI 参考提示词常量
      commands.rs            #[tauri::command] 集合（前端调用入口，见下）
      app/
        mod.rs               Tauri 运行时：窗口创建 + 毛玻璃(apply_surface) + 主题 +
                             stderr 静默 + emit_result(输出并退出) + create_settings_window
        coordinator.rs       抢答协调器：首个终态结果生效，cancel 其余，输出后退出
      channels/
        mod.rs               Channel trait（id/start/cancel_by_other）+ ResultSink + Preemption
        conversation.rs      会话型渠道公共编排（run_conversation + MessagingChannel）
        popup.rs             本地弹窗 Channel（被抢答时关窗）
        telegram.rs          Telegram Channel（发送/长轮询/inline 选项/「发送」键）
        dingding.rs          钉钉 Channel（Stream 收 + 互动卡片高级版 / 文本回退）
        feishu.rs            飞书 Channel（长连接收 + 卡片 JSON 2.0 / 文本回退）
      telegram/
        mod.rs               TelegramClient：reqwest 手写 Bot API + 错误类型
        markdown.rs          标准 Markdown → Telegram MarkdownV2（保护代码块/转义）
      dingtalk/
        mod.rs / token.rs / client.rs / stream.rs / card.rs / textfile.rs / docx.rs
                             钉钉客户端层 + Stream 长连(JSON 帧) + 卡片 + 文本附件处理
      feishu/
        mod.rs               错误类型 + 模块声明
        token.rs             tenant_access_token 缓存
        client.rs            OpenAPI：发文本/图片/文件/卡片、媒体上传、资源下载、PATCH 卡片
        ws.rs                长连接(WebSocket)：protobuf 帧(pbbp2) + 心跳/分片/回包/重连
        card.rs              卡片 JSON 2.0 组装（表单+勾选器+输入框+提交）+ 回调解析
      integrations/
        cursor_hook.rs       Cursor Hook 安装/移除/状态/reveal（mac/Linux；含内嵌脚本）
```

## 运行流程

1. `main.rs` → `cli::dispatch()`：**在创建任何窗口前**按 argv 分发。
   - 无参 → stderr 报错 + 帮助，exit 1；`--help`/`--version` → 输出，exit 0。
   - `--settings` → `app::run_settings(config)`；其余 → 解析为 `AskRequest` → `app::run_ask(request, config)`。
2. `app::launch`（提问模式）：启动 Tauri（`generate_context!` 每二进制仅一次），在 setup 中：
   - 建 `Coordinator`；按配置创建弹窗（注册 `PopupChannel`）并/或启动会话型渠道（`TelegramChannel` / `DingTalkChannel` / `FeishuChannel`，各为 tokio 任务）。
   - 弹窗禁用且无可用会话型渠道时兜底开弹窗；GUI 不可用但有会话型渠道时走 headless 并行。
3. 用户在任一 Channel 完成（发送/取消）→ 结果投递 `Coordinator`：**仅首个生效**，对其余 Channel `cancel_by_other()`，由 `emit_result` 把区块写 stdout、图片落盘，`app.exit(code)` 退出。

## 前端 ↔ 后端命令（`commands.rs` ↔ `lib/ipc.ts`）

- 弹窗：`popup_init`（取请求+主题+是否置顶+来源名）、`submit_popup`、`cancel_popup`
- 附件：`open_path`、`preview_attachments` / `close_preview`(QLPreviewPanel)、`read_image_data_url`(缩略图)、
  `file_icon_data_url`(系统图标，拖出预览)、`show_attachment_menu`(原生右键菜单)
- 设置：`get_settings`、`save_settings`、`get_prompt`、`set_theme`、`update_theme`(持久化+应用)、`open_settings`(同进程建设置窗)
- Cursor Hook：`cursor_hook_status` / `install` / `uninstall` / `reveal`
- Telegram：`telegram_test`
- 钉钉：`dingtalk_test` / `dingtalk_detect_prepare` / `dingtalk_detect_wait`
- 飞书：`feishu_test` / `feishu_detect_prepare` / `feishu_detect_wait`

窗口拖拽用 `data-tauri-drag-region`（导航栏/底部空白/设置 tab 栏）；置顶用前端 `@tauri-apps/api/window` setAlwaysOnTop。
文件拖入用 `onDragDropEvent`（原生路径）；`-f` 附件拖出用 `tauri-plugin-drag` 的 `startDrag`。
来源名（弹窗标题 / Telegram 消息头「Question from {名称}」）由环境变量 `ASKHUMAN_ENV_SOURCE_NAME` 定制，缺省「the Loop」。

## UI / 主题

- 主题三态：`system`(prefers-color-scheme)/`light`/`dark`；前端切根类 + 后端设原生窗口主题。
- macOS：`underWindowBackground` 毛玻璃 + `TitleBarStyle::Overlay` + 隐藏标题（整窗含标题栏皆玻璃），叠 0.2 色罩；Windows/Linux 退化为纯色不透明底。
- Markdown 配色见 `styles/controls.css`（链接/代码块/表头/引用/hr 等）。

## 配置

`~/.askhuman/config.json`（新位置缺失时自动回退旧 `~/.humaninloop/config.json`）：`general`(theme, language, alwaysOnTop, appearAnimation, windowEffect, speechLanguage, speechShortcut) + `channels.popup`(enabled,width,height,rememberSize) + `channels.telegram`(enabled,botToken,chatId,apiBaseUrl) + `channels.dingding`(enabled,clientId,clientSecret,userId,cardTemplateId,…) + `channels.feishu`(enabled,appId,appSecret,openId,baseUrl)。缺字段走默认、未知字段忽略。用户向配置说明见 `docs/wiki/`。

## 构建 / 开发 / 测试

```bash
pnpm install
pnpm tauri dev                                   # 调试（Vite + Tauri）
pnpm build && cargo build --release --manifest-path src-tauri/Cargo.toml   # release（前端资源在 cargo 编译时嵌入二进制）
cargo test --manifest-path src-tauri/Cargo.toml  # Rust 单测
./scripts/install.sh                              # 安装到 ~/.local/bin（mac/Linux）
```

## 注意事项

- **stdout 洁净**：GUI 阶段把 stderr 重定向到 /dev/null（`app/mod.rs` 的 `stderr_redirect`，Unix），自身错误用 `eprintln_real` 走原 stderr。
- **首帧不白闪**：`src/index.html` 内联关键底色；macOS 毛玻璃下 body 透明叠色罩。
- **macOS 透明/毛玻璃**依赖 `tauri` 的 `macos-private-api` feature 与 `macOSPrivateApi: true`。
- **release 自包含**：前端资源在 `cargo build` 时由 `generate_context!` 嵌入，故安装后无需 dev server。
- Telegram 不接收图片；Cursor Hook 仅 mac/Linux（Windows 禁用并提示）。
