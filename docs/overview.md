# AskHuman 项目概览（供 agent 参考）

> 跨平台「Human-in-the-loop」工具：命令行 `AskHuman` 在需要人类确认/补充时弹出窗口收集回应，并把结果按固定区块格式写到 stdout 供 AI 读取。

## 技术栈与形态

- **Tauri 2**：Rust 后端 + WebView 前端，单一可执行文件 `AskHuman`，跨 macOS / Windows / Linux。
- **前端**：Vue 3 + Vite + TypeScript，纯手写 macOS 风 CSS（无组件库）。
- **运行模型（当前）**：单进程。既是 CLI（纯信息命令直接终端输出、不起 GUI），又能进程内启动 Tauri 事件循环弹窗。**stdout 只输出结果区块，所有日志走 stderr。**
- **运行模型（规划中）**：正迁往「常驻 Daemon + 瘦客户端 CLI + 独立 GUI Helper」三进程架构（见下节）。本文「运行流程」等小节描述的仍是当前单进程实现，迁移完成后再整体替换。

## 架构演进：常驻 Daemon（开发中，影响后续所有需求）

> 详见 `docs/specs/daemon-architecture.md`（需求/设计）与 `docs/plans/daemon-architecture.md`（实现计划）。分支 `feat/daemon-architecture`。
> **新需求设计时应按此目标架构考量**：渠道/长连接/抢答归 Daemon，GUI 归独立短命进程，CLI 仅做入参与结果转发。
>
> **实现进度（本分支，Unix 已落地；非 Unix 仍走单进程回退）**：
> - **Phase 0**：IPC 骨架（`ipc/`：NDJSON over Unix socket）+ daemon 生命周期（`daemon/lifecycle.rs`、`daemon/spawn.rs`：flock 单实例 / 二进制指纹换新 / 空闲退出）。
> - **Phase 1**：弹窗经 Daemon + 独立 GUI Helper（`--popup`）跑通；CLI 瘦客户端化（`client/`）；Coordinator 解耦为 IPC 回传渲染结果（`RenderOutcome`）。
> - **Phase 2**：三种 IM 渠道迁入 Daemon，**每种全局仅一条长连接**，由各自 Router 独占并按键路由到对应会话（根治历史「同 client-id/app 多开长连接互抢」问题）：`dingtalk/router.rs`（卡片按 `outTrackId`、聊天按 `senderStaffId`）、`feishu/router.rs`（卡片按 `open_message_id`、聊天按 `open_id`）、`telegram/router.rs`（单一 `getUpdates` 长轮询 + 单 offset；callback 按卡片 `message_id`、自由文字归「最新活动卡片」）。「自动识别 userId/open_id」亦经 Daemon 长连接完成（`ClientMsg::Detect`：复用现有同 app 连接，否则临时开连）。`daemon status` 增报当前常热 IM 连接。
> - **Phase 3**：配置实时生效（`daemon/config_watch.rs`：`notify` 监听 `config.json`、去抖 → 重载；凭据变更/渠道禁用即**惰性失效**对应缓存 Router，下个请求按新配置重连；经 `ServerMsg::ConfigChanged` 给活动 GUI Helper 下发 `general` → 弹窗实时切主题/语言）；临时目录清理（启动 + 每小时清 `temp/askhuman/<id>/` 中超 24h 未改动者）；空闲退出 / 二进制指纹换新 / stop·restart 收尾。
> - **未完成**：Windows named-pipe daemon（非 Unix 仍走单进程回退）、整体 install 实测（Phase 4）。

**动机**：单进程模型下每次 ask 各自开 IM 长连接，违反「同一 client-id/app 同一时刻仅一条 Stream/长连接」的平台限制，并发提问会串扰；且无法在「无提问」时接收渠道消息（未来「渠道主动发起任务」）。

**三类进程（同一二进制按角色切换，本地 IPC 通信）**：

- **AskHuman CLI**（多、短命）：解析 argv（`-f` 在此解析为绝对路径、缺失即退 1）→ 提交 `AskRequest` 给 Daemon → 流式取回结果打到 stdout → 按终态映射退出码 0/1/3。
- **AskHuman Daemon**（每用户 1 个、常驻、**无 GUI**，`askhuman daemon run`）：独占持有所有 IM 长连接（钉钉/飞书/Telegram，各仅一条、常热）+ Router（按 `out_track_id`/`user_id` 分发）+ 每请求一套 Coordinator/Preemption；跑 `emit_result` 集中落盘；监听 `config.json` 实时重载/重连；管理生命周期（flock 单实例 / 二进制指纹换新 / 空闲退出 / drain）。
- **GUI Helper**（每弹窗 1 个、短命，`askhuman --popup`）：由 Daemon spawn（带一次性 token），自己主线程跑 Tauri 弹窗，收题目发答案、答完即退。把 GUI 留在独立进程，正是为让 Daemon 不必跑 AppKit/主线程。
- 设置窗口 `askhuman --settings` 仍是独立 GUI 进程，不经 Daemon；改设置后 Daemon 经 config watch 感知生效。

**关键约定**：单一可执行文件（busybox 风格多角色，`daemon run/start/stop/restart/status/logs` + 隐藏 `--popup`）；IPC 用 NDJSON over Unix socket / Windows named pipe（用户私有）；CLI↔Daemon 与 Daemon↔GUI 复用同一套任务契约；落盘 `~/.askhuman/`：`daemon.sock`/`daemon.lock`/`daemon.json`/`daemon.log`。既有契约全部不变（stdout 洁净、结果区块、退出码、配置容错、向后兼容）。

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
    App.vue                  按 URL ?view=popup|settings|history 路由
    views/PopupView.vue      弹窗：顶部导航栏（含「历史」按钮）+ Markdown/选项/文本/图片 + -f 附件区
                             (选中/打开/预览/拖出/右键) + 拖入回复文件胶囊 + 底部操作条
    views/SettingsView.vue   设置：通用（含「回复历史」保留条数 + 超额「立即清理」）/ 集成 / 通信渠道 三 Tab
    views/HistoryView.vue    独立历史窗口：顶部项目下拉 + 清空菜单；左列表（渠道徽标/相对时间/摘要）右只读详情
    components/HistoryDetail.vue 只读还原一条历史（状态横幅 + 消息/附件 + 每题选项高亮/文本/图片/文件，best-effort）
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
        mod.rs               argv 分发（--help/--version/--settings/--history[--all]/无参/提问）
        args.rs              提问参数解析（message / -o / --no-markdown / -f）
        file_attachment.rs   -f 路径解析/校验（~/相对路径 → 绝对路径 + 元信息）
        output.rs            结果区块格式化（[选择的选项]/[用户输入]/[图片]/[文件]/[状态]）
        image_writer.rs      图片 base64 落盘 + 文件名 sanitize + ext 映射
        help.rs              帮助/版本文案
      models.rs              AskRequest(含 files) / FileAttachment / ChannelResult(含 files) /
                             ImageAttachment / ChannelAction / source_name()
      config.rs              AppConfig 读写 ~/.askhuman/config.json（原子写、容错解码；旧 ~/.humaninloop 自动回退读取）
      paths.rs               home/config/temp 路径 + history.jsonl/history.lock
      project.rs             项目识别：从 cwd 向上找首个 .git 根，回退 cwd（回复历史归类）
      history.rs             回复历史存储：~/.askhuman/history.jsonl（每行一条 JSON，追加写 + 文件锁裁剪/清空）
      prompts.rs             CLI 参考提示词常量
      commands.rs            #[tauri::command] 集合（前端调用入口，见下）
      app/
        mod.rs               Tauri 运行时：窗口创建 + 毛玻璃(apply_surface) + 主题 +
                             stderr 静默 + emit_result(输出并退出) + create_settings_window /
                             create_history_window / run_history
        coordinator.rs       抢答协调器：首个终态结果生效，cancel 其余，输出后退出；
                             在唯一汇聚点旁路写入回复历史（发送 + 用户主动取消）
      channels/
        mod.rs               Channel trait（id/start/cancel_by_other）+ ResultSink + Preemption
        conversation.rs      会话型渠道公共编排（run_conversation + MessagingChannel）
        popup.rs             本地弹窗 Channel（被抢答时关窗）
        telegram.rs          Telegram Channel（发送/长轮询/inline 选项/「发送」键）
        dingding.rs          钉钉 Channel（Stream 收 + 互动卡片高级版 / 文本回退）
        feishu.rs            飞书 Channel（长连接收 + 卡片 JSON 2.0 / 文本回退）
      telegram/
        mod.rs               TelegramClient：reqwest 手写 Bot API + 错误类型
        markdown.rs          标准 Markdown → Telegram HTML（粗/斜/删/码/块/引/链 + 表格转等宽代码块 + 列表 •；仅转义 < > &，标签天然配对不回退）
        router.rs            TgRouter：单一长轮询(单 offset) 独占 + 按卡片 message_id / 最新活动分发
      dingtalk/
        mod.rs / token.rs / client.rs / stream.rs / card.rs / textfile.rs / docx.rs
                             钉钉客户端层 + Stream 长连(JSON 帧) + 卡片 + 文本附件处理
        router.rs            DdRouter：独占 StreamConn + 按 outTrackId/senderStaffId 分发
                             (提交回调带 oneshot 交会话裁决→回成功包；非提交/孤儿回空 ACK)
      feishu/
        mod.rs               错误类型 + 模块声明
        token.rs             tenant_access_token 缓存
        client.rs            OpenAPI：发文本/图片/文件/卡片、媒体上传、资源下载、PATCH 卡片
        ws.rs                长连接(WebSocket)：protobuf 帧(pbbp2) + 心跳/分片/回包/重连
        card.rs              卡片 JSON 2.0 组装（表单+勾选器+输入框+提交）+ 回调解析
        router.rs            FsRouter：独占 FeishuWs + 按 open_message_id/open_id 分发
                             (卡片回调带 oneshot 交会话裁决→同步回包更新卡片；孤儿/超时回空 ACK)
      integrations/
        cursor_hook.rs       Cursor Hook 安装/移除/状态/reveal（mac/Linux；含内嵌脚本）
      ipc/                   IPC 协议：mod.rs(消息类型) / codec.rs(NDJSON) / transport.rs(Unix socket)
      client/                (Unix) CLI 作为 Daemon 客户端：连接/握手/自启/submit/detect/status/stop
      daemon/                (Unix) 常驻 Daemon：mod.rs(分发/serve) / lifecycle.rs(单实例·指纹·空闲) /
                             spawn.rs(脱离启动) / request.rs(请求登记表·Coordinator·GUI token) /
                             config_watch.rs(notify 监听 config.json + 去抖)
```

## 运行流程

1. `main.rs` → `cli::dispatch()`：**在创建任何窗口前**按 argv 分发。
   - 无参 → stderr 报错 + 通用 `help_text`（直接 `AskHuman` 即见全部用法），exit 1；参数解析失败 / 未知选项 → stderr 报错 + 提问导向 `agent_help_text`，exit 1；`--help`/`--version` → 输出，exit 0。
   - `--settings` → `app::run_settings(config)`；`--history [--all]` → `app::run_history(project, all, config)`（默认当前项目）；其余 → 解析为 `AskRequest` → `app::run_ask(request, config)`。
2. `app::launch`（提问模式）：启动 Tauri（`generate_context!` 每二进制仅一次），在 setup 中：
   - 建 `Coordinator`；按配置创建弹窗（注册 `PopupChannel`）并/或启动会话型渠道（`TelegramChannel` / `DingTalkChannel` / `FeishuChannel`，各为 tokio 任务）。
   - 弹窗禁用且无可用会话型渠道时兜底开弹窗；GUI 不可用但有会话型渠道时走 headless 并行。
3. 用户在任一 Channel 完成（发送/取消）→ 结果投递 `Coordinator`：**仅首个生效**，对其余 Channel `cancel_by_other()`，由 `emit_result` 把区块写 stdout、图片落盘，`app.exit(code)` 退出。

## 前端 ↔ 后端命令（`commands.rs` ↔ `lib/ipc.ts`）

- 弹窗：`popup_init`（取请求+主题+是否置顶+来源名）、`submit_popup`、`cancel_popup`
- 附件：`open_path`、`preview_attachments` / `close_preview`(QLPreviewPanel)、`read_image_data_url`(缩略图)、
  `file_icon_data_url`(系统图标，拖出预览)、`show_attachment_menu`(原生右键菜单)
- 设置：`get_settings`、`save_settings`、`get_prompt`、`set_theme`、`update_theme`(持久化+应用)、`open_settings`(同进程建设置窗)
- 历史：`open_history`(弹窗→建历史窗)、`history_init`(主题+当前项目)、`get_history`(按项目/全部，倒序)、`get_history_projects`(项目下拉)、`history_count`、`trim_history`(立即裁剪)、`clear_history`(按项目/全部清空)
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

`~/.askhuman/config.json`（新位置缺失时自动回退旧 `~/.humaninloop/config.json`）：`general`(theme, language, alwaysOnTop, appearAnimation, windowEffect, speechLanguage, speechShortcut, historyLimit) + `channels.popup`(enabled,width,height,rememberSize) + `channels.telegram`(enabled,botToken,chatId,apiBaseUrl) + `channels.dingding`(enabled,clientId,clientSecret,userId,cardTemplateId,…) + `channels.feishu`(enabled,appId,appSecret,openId,baseUrl)。缺字段走默认、未知字段忽略。用户向配置说明见 `docs/wiki/`。

> 回复历史：`general.historyLimit`（默认 200，0=停止新增但保留旧记录）控制 `~/.askhuman/history.jsonl` 全局保留条数。每次提问在 `Coordinator.finish()`（所有渠道/模式唯一汇聚点）旁路记录一条「发送 / 用户主动取消」（系统取消不记）；只存图片/文件路径（best-effort 展示，缺失显示占位）。项目按「从 CLI cwd 向上找首个 .git 根、回退 cwd」识别，经 `TaskRequest`/`ShowPayload` 贯穿 daemon（revisit A11）。历史窗口 `AskHuman --history [--all]` 或弹窗导航栏「历史」按钮打开。详情只读组件 `HistoryDetail.vue` 完整还原：选项框复用 controls.css（选中=蓝底白 ✓）、附件区与弹窗同款交互（单击选中 / 空格 QuickLook 预览 + 方向键切换 / 双击打开 / 右键菜单 / 拖出）。历史窗口创建时 `watch_history_file` 用 notify 监听 `history.jsonl`，任何进程写入后发 `history-updated` 事件令前端重载并保留当前选中条目（跨进程实时刷新）。注：`preview_attachments` 命令把 QuickLook 控制者插入**调用方窗口**响应链（弹窗或历史窗口皆可），不再硬编码 popup。

> 密钥安全：三项密钥（`dingding.clientSecret`/`feishu.appSecret`/`telegram.botToken`）默认迁入系统钥匙串，`config.json` 中留空；内存 `AppConfig` 始终携带解析后的真值，读取点零改动。文件权限收紧 0600/目录 0700；钥匙串不可用时回退明文。macOS 需稳定签名身份免弹框（本地 `install.sh` 自动探测证书 / 发布走 Developer ID）。详见 `docs/specs/secret-storage-keychain.md`。

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
