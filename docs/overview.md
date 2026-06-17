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
> - **Phase 0**：IPC 骨架（`ipc/`：NDJSON over Unix socket）+ daemon 生命周期（`daemon/lifecycle.rs`、`daemon/spawn.rs`：flock 单实例 / 二进制**内容指纹**换新（内容哈希，按 (路径,mtime,size) 缓存于 `~/.askhuman/binhash.json`，与路径/mtime 无关，多处安装同版本不会互相误重启）/ 空闲退出）。
> - **Phase 1**：弹窗经 Daemon + 独立 GUI Helper（`--popup`）跑通；CLI 瘦客户端化（`client/`）；Coordinator 解耦为 IPC 回传渲染结果（`RenderOutcome`）。
> - **Phase 2**：四种 IM 渠道迁入 Daemon，**每种全局仅一条长连接**，由各自 Router 独占并按键路由到对应会话（根治历史「同 client-id/app 多开长连接互抢」问题）：`dingtalk/router.rs`（卡片按 `outTrackId`、聊天按 `senderStaffId`）、`feishu/router.rs`（卡片按 `open_message_id`、聊天按 `open_id`）、`telegram/router.rs`（单一 `getUpdates` 长轮询 + 单 offset；callback 按卡片 `message_id`、自由文字归「最新活动卡片」）、`slack/router.rs`（Socket Mode；交互按卡片 `message_ts`、聊天按 `user_id`；ack 在 ws 层收帧即回）。「自动识别 userId/open_id」亦经 Daemon 长连接完成（`ClientMsg::Detect`：复用现有同 app 连接，否则临时开连）。`daemon status` 增报当前常热 IM 连接。
> - **Phase 3**：配置实时生效（`daemon/config_watch.rs`：`notify` 监听 `config.json`、去抖 → 重载；凭据变更/渠道禁用即**惰性失效**对应缓存 Router，下个请求按新配置重连；经 `ServerMsg::ConfigChanged` 给活动 GUI Helper 下发 `general` → 弹窗实时切主题/语言）；临时目录清理（启动 + 每小时清 `temp/askhuman/<id>/` 中超 24h 未改动者）；空闲退出 / 二进制指纹换新 / stop·restart 收尾。
> - **优雅排空换新（graceful drain，见 `docs/specs/daemon-graceful-drain.md`）**：检测到过时（指纹/协议变化）且有在途请求 → 不再立即退出，而是进入 draining：在途请求服务到完结（GuiHello/Answer/Status 照常）、新 Hello 回 `Draining`、新 Submit/Detect 拒绝，全部完结后退出由等待的 CLI 拉起新 Daemon（无在途仍立即换新）。CLI 撞上排空无限等待 + 每 30s stderr 提示（剩余数 + `--force` 提示）。`daemon stop`/`restart` 默认同样 drain，`--force` 立即终止；`daemon status` 标注 draining；install.sh 安装前检测在途请求并提示。协议为增量演进（`HelloStatus::Draining`、`ServerMsg::Draining`、`Stop{force}`、`StatusInfo.draining`，PROTOCOL_VERSION 不变）。
> - **未完成**：Windows named-pipe daemon（非 Unix 仍走单进程回退）、整体 install 实测（Phase 4）。

**动机**：单进程模型下每次 ask 各自开 IM 长连接，违反「同一 client-id/app 同一时刻仅一条 Stream/长连接」的平台限制，并发提问会串扰；且无法在「无提问」时接收渠道消息（未来「渠道主动发起任务」）。

**三类进程（同一二进制按角色切换，本地 IPC 通信）**：

- **AskHuman CLI**（多、短命）：解析 argv（`-f` 在此解析为绝对路径、缺失即退 1）→ 提交 `AskRequest` 给 Daemon → 流式取回结果打到 stdout → 按终态映射退出码 0/1/3。
- **AskHuman Daemon**（每用户 1 个、常驻、**无 GUI**，`askhuman daemon run`）：独占持有所有 IM 长连接（钉钉/飞书/Telegram/Slack，各仅一条、常热）+ Router（按 `out_track_id`/`user_id` 分发）+ 每请求一套 Coordinator/Preemption；跑 `emit_result` 集中落盘；监听 `config.json` 实时重载/重连；管理生命周期（flock 单实例 / 二进制指纹换新 / 空闲退出 / drain）。
- **GUI Helper**（每弹窗 1 个、短命，`askhuman --popup`）：由 Daemon spawn（带一次性 token），自己主线程跑 Tauri 弹窗，收题目发答案、答完即退。把 GUI 留在独立进程，正是为让 Daemon 不必跑 AppKit/主线程。
- **统一 GUI 宿主**（每用户至多 1 个、长命，`askhuman --gui-host`，**Unix only**）：单实例（`gui-host.lock` flock）承载**菜单栏/托盘状态图标** + **设置/历史/Agent 三类窗口**（全局每类唯一）。所有打开窗口的入口（CLI `--settings`/`--history`/`agents monitor`、弹窗导航「设置/历史」按钮、托盘菜单）都**彻底路由**到宿主（自有 `gui-host.sock`），宿主在则聚焦/新建、不在则被拉起。详见下文「菜单栏图标 + 统一 GUI 宿主」节。

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
    views/AgentsView.vue     (实验性) Agent 状态窗口：按类型(Claude/Codex/Cursor)分组、状态优先排序
                             (工作中>空闲>已结束)、相对时间动态刷新；订阅 daemon 推送的 agents-updated
    views/SettingsView.vue   设置：通用（含「回复历史」保留条数 + 超额「立即清理」+ 底部隐蔽开关「实验性功能」）
                             / Agent / 通信渠道（+ 开启实验后出现「实验」Tab）多 Tab
                             （Agent Tab：顶部原理说明 + 「手动集成」参考提示词卡[CLI | MCP 分段切换，MCP 视图附三家 MCP 配置示例片段，绝对路径占位] + 「自动集成」按 Cursor/Claude Code/Codex 分组、每家一个 **CLI | MCP | 未集成** 三态分段控件[一键切换自动卸旧装新；推荐档位带绿色「推荐」标记：Cursor/Claude=CLI、Codex=MCP（Codex 无法延长 CLI 超时）]，下方按当前模式列出产物：CLI=Rule+超时 Hook[Codex 无 Hook 给提示]、MCP=Rule+MCP 配置；产物过期时显示橙色「更新」按钮；文件行「打开」下拉：在 Finder 中显示 / 用默认程序打开）
    views/HistoryView.vue    独立历史窗口：顶部项目下拉 + 清空菜单；左列表（渠道徽标/相对时间/摘要）右只读详情
    components/HistoryDetail.vue 只读还原一条历史（状态横幅 + 消息/附件 + 每题选项高亮/文本/图片/文件，best-effort）
    lib/ipc.ts               invoke 封装（与后端命令一一对应）
    lib/types.ts             与 Rust 模型对齐的 TS 类型
    lib/markdown.ts          markdown-it 渲染
    lib/theme.ts             applyTheme（切类）/ fileToDataUrl
    styles/{tokens,base,controls}.css   设计 token / 重置+Markdown / 控件

  src-tauri/                 Rust 后端
    Cargo.toml               依赖（tauri[macos-private-api]、reqwest、tokio、dark-light、libc、
                             tauri-plugin-drag、rmcp[server,transport-io] + schemars(MCP server)、
                             macOS: objc2 / objc2-foundation / objc2-app-kit…）
    tauri.conf.json          frontendDist=../dist；app.macOSPrivateApi=true
    capabilities/default.json 窗口权限（含 start-dragging / set-always-on-top / drag:default）
    src/
      main.rs                入口：声明模块，调用 cli::dispatch()
      macos_quicklook.rs     (macOS) 原生 QLPreviewPanel 预览 + 文件系统图标(file_icon_png_base64)
      macos_menu.rs          (macOS) -f 附件原生右键菜单（NSMenu，Finder 风格）
      cli/
        mod.rs               argv 分发（--help/--version/--settings/--history[--all]/--gui-host/--agent-help/
                             --scripting-help/daemon/agents/channel/config/doctor/mcp/无参/提问；Unix 下
                             --settings/--history 彻底路由到 GUI 宿主[host_open]、失败兜底本进程建窗；提问请求据
                             `ASKHUMAN_FROM_MCP` 置 `from_mcp`，让 daemon 对 MCP 来源仅刷新活动、不新建会话）
        args.rs              提问参数解析（message / --stdin / -o / -o!(推荐选项) / --no-markdown / -f /
                             --single(单选) / --select-only(严格，须每题有选项) / --output <text|json>）
        cfgio.rs             CLI 配置公共工具：点号路径 get/set（serde_json::Value）+ 类型强制 + 密钥识别 +
                             密钥取值(env/file/stdin/隐藏输入) + 脱敏 + 交互输入 + block_on 助手
        config_cmd.rs        `config show|get|set|unset|path`（通用键值兜底；密钥键自动入钥匙串，值不进 argv）
        channel_cmd.rs       `channel list|set|enable|disable|test|detect`（向导+脚本；复用 commands 的 test/detect）
        agents_cmd.rs        `agents monitor|mode|show|install|uninstall|update`（状态文本/json + 三态模式编排 +
                             细粒度集成 --rules/--hook/--mcp/--lifecycle；复用 integrations）
        doctor.rs            `doctor [--json]` 一屏体检（daemon/渠道/集成，集成含 mode/rules/hook/mcp/lifecycle）
        file_attachment.rs   -f 路径解析/校验（~/相对路径 → 绝对路径 + 元信息）
        output.rs            结果格式化：文本区块字段恒英文常量 MARKER_*（[selected_options]/[user_input]/
                             [files]（图片+文件合并）/[status]）+ render_json（D7：snake_case/省空字段）
        image_writer.rs      图片 base64 落盘 + 文件名 sanitize + ext 映射
        help.rs              帮助/版本文案：--help(提问/管理/帮助三块) + --agent-help + --scripting-help（共享片段组装）
      models.rs              AskRequest(含 files / select_only / single / output_format) / OutputFormat(text|json) /
                             OptionItem(text+recommended，反序列化兼容旧纯字符串) /
                             FileAttachment / ChannelResult(含 files) / ImageAttachment / ChannelAction / source_name()
      config.rs              AppConfig 读写 ~/.askhuman/config.json（原子写、容错解码；旧 ~/.humaninloop 自动回退读取）
      paths.rs               home/config/temp 路径 + history.jsonl/history.lock + cursor_mcp_json/claude_json（MCP 配置）
                             + gui_host_sock/gui_host_lock（统一 GUI 宿主自有 socket / 单实例锁）
      project.rs             项目识别：从 cwd 向上找首个 .git 根，回退 cwd（回复历史归类）
      history.rs             回复历史存储：~/.askhuman/history.jsonl（每行一条 JSON，追加写 + 文件锁裁剪/清空）
      prompts.rs             参考提示词：`cli_reference()`（CLI 版，含 24h 超时/`--agent-help` 等 shell 指引）+
                             `mcp_reference()`（MCP 版，去 shell 指引、改引用 `ask` MCP 工具）
      mcp/                   `AskHuman mcp` STDIO MCP server（rmcp）：mod.rs(tokio runtime + serve) /
                             ask.rs(单工具 `ask`：JsonSchema 入/出参 → build_argv → spawn 现有
                             `AskHuman … --output json`[带 `ASKHUMAN_FROM_MCP=1`] → 解析为 `AskResult`
                             [剔除脚本专用 `selected_indices`] → structuredContent + content[JSON 文本 +
                             图片转 ImageContent]）。薄壳：每次调用新 spawn 子进程，天然跨 daemon 重启重连
      hooks.rs               用户级 hooks：~/.askhuman/hooks/<event> 可执行脚本（首个事件 ask-received）
      sound.rs               内置弹窗提示音（macOS afplay / Linux canberra·paplay / 其它不支持）
      commands.rs            #[tauri::command] 集合（前端调用入口，见下）
      app/
        mod.rs               Tauri 运行时：窗口创建 + 毛玻璃(apply_surface) + 主题 +
                             stderr 静默 + emit_result(输出并退出) + create_settings_window /
                             create_history_window(可带项目过滤) / create_agents_window / run_history /
                             run_gui_host(View::GuiHost) + on_menu_event/exit 钩子路由到 gui_host
        gui_host.rs          (Unix) 统一 GUI 宿主运行时：托盘三态图标 + 菜单(状态/操作) + 自有 IPC 监听
                             (OpenWindow→主线程建/聚焦窗口) + daemon 状态订阅(非保活，驱动图标/菜单) +
                             窗口期续命连接(spec D5) + 配置热更新(模式/语言→建移图标·装卸登录项·切活动策略) +
                             二进制换新(无窗口时切新版) + 单实例锁 + macOS accessory 活动策略
        coordinator.rs       抢答协调器：首个终态结果生效，cancel 其余，输出后退出；
                             在唯一汇聚点旁路写入回复历史（发送 + 用户主动取消）
      channels/
        mod.rs               Channel trait（id/start/cancel_by_other）+ ResultSink + Preemption
        conversation.rs      会话型渠道公共编排（run_conversation + MessagingChannel）
        popup.rs             本地弹窗 Channel（被抢答时关窗）
        telegram.rs          Telegram Channel（发送/长轮询/inline 选项/「发送」键）
        dingding.rs          钉钉 Channel（Stream 收 + 互动卡片高级版 / 文本回退）
        feishu.rs            飞书 Channel（长连接收 + 卡片 JSON 2.0 / 文本回退）
        slack.rs             Slack Channel（Socket Mode 收 + Block Kit 消息内表单 / 文本回退）
      telegram/
        mod.rs               TelegramClient：reqwest 手写 Bot API + 错误类型
        markdown.rs          标准 Markdown → Telegram HTML（粗/斜/删/码/块/引/链 + 表格转等宽代码块 + 列表 •；仅转义 < > &，标签天然配对不回退）
        router.rs            TgRouter：单一长轮询(单 offset) 独占 + 按卡片 message_id / 最新活动分发
      dingtalk/
        mod.rs / token.rs / client.rs / stream.rs / card.rs / textfile.rs / docx.rs
                             钉钉客户端层 + Stream 长连(JSON 帧) + 卡片 + 文本附件处理
                             （card.rs 高级版模板契约：options=[{id,md}] / single / allow_input；提交回传选项 id）
        router.rs            DdRouter：独占 StreamConn + 按 outTrackId/senderStaffId 分发
                             (提交回调带 oneshot 交会话裁决→回成功包；非提交/孤儿回空 ACK)
      feishu/
        mod.rs               错误类型 + 模块声明
        token.rs             tenant_access_token 缓存
        client.rs            OpenAPI：发文本/图片/文件/卡片、媒体上传、资源下载、PATCH 卡片
        ws.rs                长连接(WebSocket)：protobuf 帧(pbbp2) + 心跳/分片/回包/重连
        card.rs              卡片 JSON 2.0 组装（多选=表单内勾选器；单选=勾选器移出表单+toggle 回调互斥；
                             严格去 input；推荐左侧绿色 lark_md 前缀）+ 提交/toggle 回调解析
        router.rs            FsRouter：独占 FeishuWs + 按 open_message_id/open_id 分发
                             (卡片回调带 oneshot 交会话裁决→同步回包更新卡片；孤儿/超时回空 ACK)
      slack/
        mod.rs               错误类型 + 模块声明
        client.rs            Web API：chat.postMessage/update、conversations.open、files 上传下载、auth.test
        ws.rs                Socket Mode 长连接(WebSocket，JSON 帧)：收帧即 ack(envelope_id) + 重连
        blockkit.rs          Block Kit 消息内表单组装（多选=checkboxes / 单选=radio_buttons；严格去 plain_text_input；
                             推荐用原生 description「👍 推荐」+ 文本加粗）+ block_actions 提交解析
        markdown.rs          标准 Markdown → Slack mrkdwn（粗*斜_删~码块引链 + 表格转等宽 + 列表 •）
        router.rs            SlRouter：独占 SlackWs + 按 message_ts/user_id 分发（无 oneshot，ack 在 ws 层）
      integrations/
        cursor_hook.rs       Cursor Hook 安装/更新/移除/状态/reveal（mac/Linux；hooks.json 内嵌脚本）；
                             needs_update：已安装但磁盘脚本 ≠ 内置最新 SCRIPT_CONTENT（或缺失/仅旧版）→ 需更新
        claude_hook.rs       Claude Code Hook：~/.claude/settings.json 注册 PreToolUse(Bash) 脚本 +
                             抬高 env.BASH_MAX_TIMEOUT_MS；命中 AskHuman 时把该次 Bash timeout 设为 24h
                             （幂等纯函数 + 单测；卸载不动 env）；needs_update 同 Cursor（脚本漂移）
        agent_lifecycle.rs   (实验性) 三家生命周期 hook 安装/卸载/状态：**用户级**、与 timeout hook 独立共存
                             （仅增删命令含 `__agent-hook` 标记的条目，保留其它 hook/格式）。Claude
                             `~/.claude/settings.json`(Nested) / Cursor `~/.cursor/hooks.json`(Flat) 用 jsonc CST；
                             Codex `~/.codex/hooks.json` + `config.toml [hooks.state]` 信任哈希用 toml_edit
                             （复刻 codex `version_for_toml`：sha256(紧凑·键排序 JSON(归一 identity))，键
                             `<hooks.json 绝对路径>:<event_snake>:<g>:<h>`，见 demo codex-trust.cjs）；Codex 无 SessionEnd
        agent_rules.rs       Agent 全局 Rules 安装/更新/卸载/状态/open/reveal：三者均用 AskHuman:begin/end
                             托管区块写入，保留区块外用户内容（Cursor ~/.cursor/rules/askhuman.mdc 另带
                             alwaysApply frontmatter，卸载时区块外仅剩 frontmatter/空白才删整文件；旧版
                             独占文件含 MANAGED_FILE_MARK 仍识别为已安装、安装/更新时迁移为区块格式）；
                             Claude ~/.claude/CLAUDE.md、Codex ~/.codex/AGENTS.md。needs_update：区块内
                             正文 ≠ 最新提示词（或旧版无区块）→ 需更新（幂等纯函数 + 单测）。
                             `Variant`(Cli|Mcp) 区分写入 cli_reference/mcp_reference；`installed_variant`
                             探测已装变体、`needs_update_variant`/`install_variant` 变体感知
        mcp_config.rs        MCP server 配置写入（用户级全局，server 名 `askhuman`、args `["mcp"]`）：
                             Cursor ~/.cursor/mcp.json、Claude ~/.claude.json 走 jsonc-parser CST（mcpServers.askhuman）；
                             Codex ~/.codex/config.toml 走 toml_edit（[mcp_servers.askhuman] + startup_timeout_sec
                             /tool_timeout_sec）。最小编辑保留用户其它内容/注释/格式；install/update/uninstall/
                             is_installed/needs_update/display_path/reveal/open（幂等纯函数 + 单测）
        agent_mode.rs        三态模式编排（None|Cli|Mcp 互斥）：Cli=Rule(CLI)+超时 Hook，Mcp=Rule(MCP)+MCP 配置；
                             `current`(**以产物 MCP配置/超时Hook 为首要信号**，互斥且由 set 维护、稳定；产物不
                             明确时才回退 Rule 变体——避免内置提示词改版后已装旧正文失配被错判模式) /
                             `needs_update` / `set`(卸非目标产物→装目标，幂等) / `update`(刷当前模式) /
                             uninstall_all。lifecycle hook 与三态正交。`agent_rules::classify_body` 对漂移正文
                             用结构信号（是否含 `Shell/Bash`）判 CLI/MCP，提示词改版仍稳定归类
      ipc/                   IPC 协议：mod.rs(消息类型，含 ServerMsg::UpdateState/TrayState、ClientMsg::TraySubscribe) /
                             codec.rs(NDJSON) / transport.rs(Unix socket)
      gui_host/              (Unix) 宿主自有 IPC（与 daemon 解耦）：mod.rs(HostMsg{OpenWindow{kind,all,project}/Ping/
                             Shutdown} + gui-host.sock bind/connect + host_open 客户端[连不上则 spawn 宿主再轮询] + 单实例)
      client/                (Unix) CLI 作为 Daemon 客户端：连接/握手/自启/submit/detect/status/stop
      daemon/                (Unix) 常驻 Daemon：mod.rs(分发/serve + 自更新后台检查/广播/指纹感知 +
                             handle_tray_sub[非保活]/broadcast_tray_state/maybe_spawn_gui_host) /
                             lifecycle.rs(单实例·指纹·空闲) / spawn.rs(脱离启动) /
                             request.rs(请求登记表·Coordinator·GUI token·broadcast_to_guis) /
                             config_watch.rs(notify 监听 config.json + 去抖)
      update/                版本自更新：mod.rs(检测/比较/Updater/select/check) / direct.rs(GitHub 资产替换) /
                             npm.rs(npm i -g) / notes.rs(release notes 取/聚合) / state.rs(update.json)
      agents/                (实验性, Unix) Agent 生命周期追踪：mod.rs(AgentKind=claude/codex/cursor +
                             LifecycleEvent=session-start/turn-start/turn-end/session-end) /
                             detect.rs(按 env 判真实运行家族[Cursor 双触发去重] / session_id 解析 /
                             walk 进程树定位 agent pid / kill-0 存活) /
                             title.rs(三家会话标题解析：cursor meta.json / codex·claude jsonl) /
                             registry.rs(AgentRecord 注册表：apply_event 推导 工作中/空闲/已结束、
                             poll_liveness、ttl_sweep[1h 兜底]、ended 最多留 10 条、persist/load
                             ~/.askhuman/agents.json、snapshot 推送) /
                             report.rs(隐藏子命令 `__agent-hook <agent> <event>` 上报器：去重+解析+发 daemon)
      integrations/login_item.rs (Unix) 开机自启登录项（仅「一直显示」模式）：macOS LaunchAgent plist /
                             Linux autostart .desktop 的 install/uninstall/is_installed/needs_update/ensure_installed

  cliff.toml                 git-cliff 配置：Conventional Commits → 面向用户的 release notes
  docs/release-notes/        每版本可选覆盖文件 v<版本>.md（存在即用其内容，否则 git-cliff 生成）
```

## 运行流程

1. `main.rs` → `cli::dispatch()`：**在创建任何窗口前**按 argv 分发。
   - 无参 → stderr 报错 + 通用 `help_text`（直接 `AskHuman` 即见全部用法），exit 1；参数解析失败 / 未知选项 → stderr 报错 + 提问导向 `agent_help_text`，exit 1；`--help`/`--version` → 输出，exit 0。
   - 上述命令的 stdout 文本统一经 `cli::print_line` 输出：对 BrokenPipe（读端提前关闭，如 `AskHuman --agent-help | head`）静默忽略、不 panic。否则 Rust 默认忽略 SIGPIPE → `println!` 写失败 panic → release `panic=abort` 会以退出码 134 退出。
   - `--settings`/`--history [--all]`（Unix）→ 经 `gui_host::host_open` 路由到统一 GUI 宿主（全局单窗，失败兜底 `run_settings`/`run_history`）；非 Unix → 直接 `app::run_settings`/`run_history`。`--gui-host` → `app::run_gui_host`（宿主角色）。其余 → 解析为 `AskRequest` → `app::run_ask`。
2. `app::launch`（提问模式）：启动 Tauri（`generate_context!` 每二进制仅一次），在 setup 中：
   - 建 `Coordinator`；按配置创建弹窗（注册 `PopupChannel`）并/或启动会话型渠道（`TelegramChannel` / `DingTalkChannel` / `FeishuChannel` / `SlackChannel`，各为 tokio 任务）。
   - 弹窗禁用且无可用会话型渠道时兜底开弹窗；GUI 不可用但有会话型渠道时走 headless 并行。
3. 用户在任一 Channel 完成（发送/取消）→ 结果投递 `Coordinator`：**仅首个生效**，对其余 Channel `cancel_by_other()`，由 `emit_result` 把区块写 stdout、图片落盘，`app.exit(code)` 退出。

## 前端 ↔ 后端命令（`commands.rs` ↔ `lib/ipc.ts`）

- 弹窗：`popup_init`（取请求+主题+是否置顶+来源名）、`submit_popup`、`cancel_popup`
- 附件：`open_path`、`preview_attachments` / `close_preview`(QLPreviewPanel)、`read_image_data_url`(缩略图)、
  `file_icon_data_url`(系统图标，拖出预览)、`show_attachment_menu`(原生右键菜单)
- 设置：`get_settings`、`save_settings`、`get_prompt`(可选 `variant`=cli|mcp)、`set_theme`、`update_theme`(持久化+应用)、`open_settings`(Unix 路由到 GUI 宿主、否则同进程建设置窗)、`popup_sound_support`(平台支持 named/toggle/none + 音名列表)、`play_popup_sound`(试听)
- 历史：`open_history`(Unix 路由到 GUI 宿主、带弹窗项目过滤；否则同进程建历史窗)、`history_init`(主题+当前项目)、`get_history`(按项目/全部，倒序)、`get_history_projects`(项目下拉)、`history_count`、`trim_history`(立即裁剪)、`clear_history`(按项目/全部清空)
- Cursor Hook：`cursor_hook_status`（含 outdated）/ `install` / `update` / `uninstall` / `reveal`
- Claude Code Hook：`claude_hook_status`（含 outdated）/ `install` / `update` / `uninstall` / `reveal`
- Agent 全局 Rules：`agent_rule_status`（含 outdated）/ `install` / `update` / `uninstall` / `reveal` / `open`（入参 `agent`：cursor/claude/codex）
- Agent 三态模式：`agent_mode_status`（返回 mode + 各产物装没装/需更新 + 路径）/ `agent_mode_set`(none|cli|mcp) / `agent_mode_update`（刷当前模式产物）/ `mcp_config_reveal`·`mcp_config_open`·`agent_hook_reveal`·`agent_hook_open`（定位/打开配置）/ `mcp_command_path`（当前 exe 绝对路径，供手动卡示例填充）
- MCP 配置：`mcp_config_reveal` / `mcp_config_open`（入参 `agent`：cursor/claude/codex）
- 超时 Hook 文件：`agent_hook_reveal` / `agent_hook_open`（入参 `agent`；Codex 无 Hook 为 no-op）
- Telegram：`telegram_test`
- 钉钉：`dingtalk_test` / `dingtalk_detect_prepare` / `dingtalk_detect_wait`
- 飞书：`feishu_test` / `feishu_detect_prepare` / `feishu_detect_wait`
- Slack：`slack_test` / `slack_detect_prepare` / `slack_detect_wait`
- 版本自更新：`get_app_version` / `update_check`(manual) / `update_get_notes`(aggregate) / `update_apply`(落盘+进度事件) / `update_dismiss` / `popup_update_state`(弹窗拉初值) / `restart_settings`(设置进程重开)
- (实验性) Agent 生命周期：`agents_init`(状态窗口主题+语言) / `agent_lifecycle_status` / `agent_lifecycle_install` / `agent_lifecycle_uninstall`（入参 `agent`：claude/codex/cursor）

窗口拖拽用 `data-tauri-drag-region`（导航栏/底部空白/设置 tab 栏）；置顶用前端 `@tauri-apps/api/window` setAlwaysOnTop。
文件拖入用 `onDragDropEvent`（原生路径）；`-f` 附件拖出用 `tauri-plugin-drag` 的 `startDrag`。
来源名（弹窗标题 / Telegram 消息头「Question from {名称}」）由环境变量 `ASKHUMAN_ENV_SOURCE_NAME` 定制，缺省「the Loop」。

> 推荐选项（`-o!` / `--option!`，见 `docs/specs/recommended-option.md`）：语义同 `-o` 且标记该选项为 AI 推荐答案（一题可多个，不预选中）。弹窗/历史详情在选项文本流开头内联显示「大拇指 SVG +『推荐』」绿色 Badge（`controls.css`：外层 `.rec-badge` 为与 `.label` 行高等高的透明对齐外框、内层 `.rec-badge-pill` 为绿色胶囊；使其与勾选框中线对齐、跨平台稳定，且换行后文本可铺满整行）；IM 渠道显示文本加本地化「👍推荐 」前缀（`channel.recommendedPrefix` + `conversation::display_text`），提交值恒为原文——其中钉钉卡片模板回传显示文本，由 `dingding::restore_selected` 还原原文，其余渠道按下标天然回原文。

## UI / 主题

- 主题三态：`system`(prefers-color-scheme)/`light`/`dark`；前端切根类 + 后端设原生窗口主题。
- macOS：`underWindowBackground` 毛玻璃 + `TitleBarStyle::Overlay` + 隐藏标题（整窗含标题栏皆玻璃），叠 0.2 色罩；Windows/Linux 退化为纯色不透明底。
- Markdown 配色见 `styles/controls.css`（链接/代码块/表头/引用/hr 等）。

## 配置

`~/.askhuman/config.json`（新位置缺失时自动回退旧 `~/.humaninloop/config.json`）：`general`(theme, language, alwaysOnTop, appearAnimation, windowEffect, speechLanguage, speechShortcut, historyLimit, popupSound, menuBarIcon[off|active|always，默认 off，仅 macOS/Linux 桌面，见「菜单栏图标」节]) + `channels.popup`(enabled,width,height,rememberSize) + `channels.telegram`(enabled,botToken,chatId,apiBaseUrl) + `channels.dingding`(enabled,clientId,clientSecret,userId,cardTemplateId,…) + `channels.feishu`(enabled,appId,appSecret,openId,baseUrl) + `channels.slack`(enabled,botToken,appToken,userId) + `channels.autoActivation`(「IM 会话期自动激活」开关，默认 false) + `experimental`(enabled，实验性功能开关，默认 false)。缺字段走默认、未知字段忽略。用户向配置说明见 `docs/wiki/`。

> IM 会话期自动激活（`channels.autoActivation`，默认关；设计 `docs/plans/im-channel-activation.md`）：开关关＝旧「每次提问全发所有启用 IM」。开关开（UI 入口在「实验」Tab，随 `experimental.enabled` 显露；旁注「建议同时开启生命周期追踪以提高状态识别准确性」）后：daemon 在 agent **工作中**才连各启用 IM、默认只监听入站；同一时刻只有「活跃槽」对应的 IM 收提问卡片；在某 IM 发 `/here`（或 `/这里`）把该渠道设为活跃槽，发 `/status`（或 `/状态`）回工作中/空闲 agent 文本，普通消息＝切到此渠道（文本不当答案）。**凡把活跃槽切到某 IM 都会把所有在途未答补推过去**（补推＝渠道激活的固有行为，统一在 `set_active_channel`、与触发方式无关：`/here`/普通消息/`/status` 切槽/作答切槽均同）。活跃槽**持久化**于 `~/.askhuman/state/auto-channel.json`、跨重启保留、仅由入站消息改变。忙/闲/结束判定复用 Agent 生命周期追踪（`agents/registry.rs`）；无 turn hook 时「首次提问起算工作中」（仅开关开时兜底登记）。代码：`autochannel.rs` + `daemon/mod.rs`（`ensure_inbound_listeners`/`spawn_listener` 通用循环 + `handle_inbound`/`backfill_inflight`/`attach_im_channels` 门控）。命令处理一份实现，各渠道只提供传输原语（连 Router + 原始消息观察者 + 抽取 `(发送者,文本)` + 期望发送者 + `build_im_channel`/`reply_channel_text` 分支）。**四家（飞书/钉钉/Slack/Telegram）均已接入并真机端到端验证 OK**。**入站消费随「工作中」起、随守护进程退出而止**：复用 lifecycle turn hook，turn-start/提问即 `ensure_inbound_listeners`（按 `working_count>0` 自门控、与开关无关），使 `/here`、`/status` 在工作期间随时可用；不做主动断连，连接随守护进程空闲退出而释放（D18）。**`/status` 与总开关独立**（开关只管切槽/发卡）；`/here` 在关态静默忽略。Telegram 自由文字既是答案又被观察者收到，斜线前缀文字仅当命令、不路由到在途卡片。**活跃槽统一含 "popup"**：在哪个渠道说话/作答就更新为哪个（弹窗作答 → "popup"，后续只弹窗），切槽时给旧 IM 发反激活提示（点明切到了哪个渠道）、把在途未答补推给新 IM、由调用方给新渠道发激活回执——逻辑统一在 `set_active_channel` 一处（返回 `(是否切换, 补推数)`；`winner_channel_id` 提供作答渠道）。

> 回复历史：`general.historyLimit`（默认 200，0=停止新增并清理已有记录）控制 `~/.askhuman/history.jsonl` 全局保留条数（裁剪与「立即清理」对 0 不再特判：`record` 在 limit==0 时不新增、但按与 >0 相同时机把已有记录裁到 0；`trim(0)` 直接清空）。每次提问在 `Coordinator.finish()`（所有渠道/模式唯一汇聚点）旁路记录一条「发送 / 用户主动取消」（系统取消不记）；只存图片/文件路径（best-effort 展示，缺失显示占位）。项目按「从 CLI cwd 向上找首个 .git 根、回退 cwd」识别，经 `TaskRequest`/`ShowPayload` 贯穿 daemon（revisit A11）。历史窗口 `AskHuman --history [--all]` 或弹窗导航栏「历史」按钮打开。详情只读组件 `HistoryDetail.vue` 完整还原：选项框复用 controls.css（选中=蓝底白 ✓）、附件区与弹窗同款交互（单击选中 / 空格 QuickLook 预览 + 方向键切换 / 双击打开 / 右键菜单 / 拖出）。历史窗口创建时 `watch_history_file` 用 notify 监听 `history.jsonl`，任何进程写入后发 `history-updated` 事件令前端重载并保留当前选中条目（跨进程实时刷新）。注：`preview_attachments` 命令把 QuickLook 控制者插入**调用方窗口**响应链（弹窗或历史窗口皆可），不再硬编码 popup。

> 密钥安全：五项密钥（`dingding.clientSecret`/`feishu.appSecret`/`telegram.botToken`/`slack.botToken`/`slack.appToken`）默认迁入系统钥匙串，`config.json` 中留空；内存 `AppConfig` 始终携带解析后的真值，读取点零改动。文件权限收紧 0600/目录 0700；钥匙串不可用时回退明文。macOS 需稳定签名身份免弹框（本地 `install.sh` 自动探测证书 / 发布走 Developer ID）。详见 `docs/specs/secret-storage-keychain.md`。
>
> `AppConfig::load()` 会解析密钥（读钥匙串）；只需 general/主题/语言/历史上限等非密钥项的路径一律改用 `AppConfig::load_without_secrets()`（读 config.json + 旧路径回退 + 收紧权限，但跳过密钥解析），避免无关命令（如 `--version`/`--help`）触发钥匙串读取、进而在签名不匹配时弹密码框。当前用 `load_without_secrets` 的：`i18n::Lang::current()`（语言）、`--settings`/`--history` 与窗口创建（主题）、`record_history`（history_limit）、`update_theme`/`persist_popup_size`（改后 `save()` 对空密钥字段原样不动、既不读也不写钥匙串）。确需密钥的保持 `load()`：daemon 初始化/attach IM/热重载、`get_settings` 的「已保存」判定、`fallback_secret`、非 unix 的 `run_ask`。

## 版本自更新（self-update）

> 需求/方案见 `docs/specs/self-update.md`、`docs/plans/self-update.md`。核心理念：**apply 只把新二进制落盘，不 restart**；「答完所有在途弹窗后再换新、不打断作答」完全复用既有 daemon graceful-drain。

- **后端模块 `src-tauri/src/update/`**：
  - `mod.rs`：`detect_install_kind()`（读 `current_exe()` 路径含 `node_modules/@humaninloop|askhuman` → Npm，否则 Direct）、`compare_versions`/`normalize_version`、`target_triple`、`Updater` trait、`select_updater`、`check()`（查远端并与本地比较 → `UpdateInfo`）。两类 HTTP 客户端：`http_client()`（仅 npm registry / 资产下载，不带鉴权头）与 `github_client()`（GitHub API 专用，若环境变量 `ASKHUMAN_GITHUB_TOKEN`/`GITHUB_TOKEN` 存在则带 `Authorization: Bearer`，把未认证 60/时/IP 提到认证 5000/时/账号，解决代理共享出口 IP 限流；token 头标 sensitive 不入日志）；`github_status_error()` 把 403/429 归一为带 `rate-limited` 标记的错误，前端据此显示友好文案并引导手动下载 / 设 token（参考项目仅做友好文案、未加 token）。
  - `direct.rs`（`DirectUpdater`）：GitHub Releases `/releases/latest` 查版本；apply 下载平台资产（按目标三元组匹配 `AskHuman-<triple>-v<ver>.{tar.gz,zip}`）→ 解压（shell out tar/unzip）→ 找 `AskHuman` →（macOS）`codesign` 验签 + 校验 TeamID `DMJXDB9H6Q` → 备份 `<exe>.<ver>.bak` → 同目录临时文件 + `chmod 0755` + `rename` 原子替换。Windows 暂不自动替换（仅提示）。
  - `npm.rs`（`NpmUpdater`）：npm registry 查 `latest`；apply 跑 `npm i -g askhuman@latest`，失败/缺 npm → 回带手动命令的错误。
  - `notes.rs`：`latest_notes()` / `notes_for_tag()` / `aggregated_notes(from,to)`（懒加载，拉一次 `/releases` 列表过滤 (from,to] 区间，从新到旧拼接）。
  - `state.rs`：`~/.askhuman/update.json`（`latest_version`/`release_notes`/`checked_at`/`dismissed_versions`/`pending`），原子写。
- **daemon 集成**：启动 +20s 查一次、之后每 24h → 落 `update.json` → 有变化 `broadcast_to_guis(ServerMsg::UpdateState{available,latest_version,pending})`；另起 15s 周期监听盘上二进制指纹（应用内更新 / 外部 `npm i -g`）→ 置 `pending` + 广播；GuiHello 握手即带当前态。`ServerMsg::UpdateState` 变体名 camelCase、字段 snake_case（同二进制两端，与既有 `Final{exit_code}` 一致）。
- **GUI Helper → 前端**：读到 `UpdateState` → 写进程内缓存（`commands::set_pushed_update`）+ emit `update-state`；弹窗挂载先 `popup_update_state` 取初值再监听事件（规避竞态）。
- **前端**：弹窗右上角更新入口（绿点）+ 浮层（版本/日志/「答完才生效、不打断」/更新按钮）+ 顶部「待生效」横条（`PopupView.vue`）；设置「通用」Tab 新增「关于」区（当前/最新版本、检查更新、更新带进度、聚合更新日志 markdown、查看全部发布、更新后「重启设置页面」`restart_settings`）（`SettingsView.vue`）。
- **发布流程**：仓库根 `cliff.toml` 用 git-cliff 从 Conventional Commits 生成 release notes（仅 feat/fix/perf/security/revert；breaking 置顶；scope 粗体前缀；`Release-Note:`/`Release-Note: skip` 单条覆盖，skip 由 body 模板按 footer 过滤以免无 body 提交误伤）。`release.yml`：`fetch-depth:0` + 安装 git-cliff，若 `docs/release-notes/v<版本>.md` 存在用其内容、否则 `git-cliff --latest` 生成，`body_path` 替换 `generate_release_notes`。提交规范见 `AGENTS.md`。

## 实验性功能：Agent 生命周期追踪 + 状态窗口（Unix）

> 需求 `docs/specs/agent-lifecycle-tracking.md` + 计划 `docs/plans/agent-lifecycle-tracking.md`（基于 `demo/agent-lifecycle/FINDINGS.md` 三家实测）。**独立于** IM 渠道激活，只追踪、不做 attach/激活。

- **开关入口**：设置「通用」底部隐蔽开关「实验性功能」(`config.experimental.enabled`，默认关、Windows 不显示) → 出现「实验」Tab，内含 Claude/Codex/Cursor 三家「生命周期追踪」开关；开/关＝安装/卸载**用户级** lifecycle hook（`integrations/agent_lifecycle.rs`，开关真值实时查 `agent_lifecycle_status`，与既有 timeout hook 互不影响）。
- **事件采集**：hook 命令统一为 `AskHuman __agent-hook <agent> <event>`（`agents/report.rs`）；四类事件 session-start/turn-start/turn-end/session-end（Codex 无 session-end）。reporter 按 env 判真实运行家族做**去重**（Cursor 兼容加载 `~/.claude` 致每事件双触发，env 有 `CURSOR_*`→只认 cursor、跳过 claude 那次），并 walk 进程树定位真实 agent pid、解析 session_id（env 专用变量优先，回退 stdin JSON）。
- **状态推导**：daemon 内 `AgentRegistry`（`agents/registry.rs`）以 **session_id 为身份**、pid 仅判存活。turn-start/turn-end 切「工作中/空闲」；**进程存活轮询（1s）是权威的「已结束」判据**（关窗/`kill -9` 时事件全丢，靠它）；1h TTL 兜底（任何 hook 事件或 `AskHuman` 提问会刷新**对应 session** 的活动时间）。ended 最多留 10 条。状态持久化 `~/.askhuman/agents.json`，daemon 换新/重启后重载并 kill-0 复核。
- **闲退守卫**：仅「工作中」agent 或有状态窗口连接才阻止 daemon 闲时退出（空闲 agent 不算）；**graceful drain（版本换新）不受存活 agent 影响**。
- **状态窗口**：`AskHuman agents monitor`（原 `agents status` 改名，spec `cli-config.md` D8）→ 有 GUI 时**路由到统一 GUI 宿主**（`gui_host::host_open(Agents)`，全局单窗；兜底 `app::run_agents` + `create_agents_window`），订阅 `ServerMsg::AgentsState` 推送、前端 `AgentsView.vue` 跨项目按类型分组、状态优先排序、相对时间动态刷新；headless 或 `--json`/`--text` → 取一次 `AgentsState` 快照（`client::request_agents_snapshot`）渲染文本/JSON（`agents_cmd.rs`）。
- **IPC 增量**：`TaskRequest` 加 `agent_kind/agent_session_id/agent_pid`（提问顺带刷新活动）；`ClientMsg::AgentEvent`/`AgentsSubscribe`；`ServerMsg::AgentsState`。

## 菜单栏图标 + 统一 GUI 宿主（Unix；macOS/Linux 桌面）

> 需求 `docs/specs/menu-bar-tray.md` + 计划 `docs/plans/menu-bar-tray.md`。在菜单栏/托盘显示守护进程状态图标并快速打开设置/历史/Agent；同时把所有 GUI 窗口收拢进**单实例宿主进程**保证全局每类窗口唯一。Windows 不支持（设置项隐藏、`--gui-host` 报错退出）。

- **统一 GUI 宿主进程 `AskHuman --gui-host`**（`app/gui_host.rs`，`cli/mod.rs` 隐藏角色）：单实例（`gui-host.lock` flock，重复 spawn 的多余进程抢锁失败即退）。承载托盘图标 + 设置/历史/Agent 三类窗口（每类全局唯一，复用既有 `create_*_window` 的「聚焦或新建」）。macOS 有图标时设 `NSApplicationActivationPolicyAccessory`（不占 Dock/Cmd-Tab）、off 模式设 Regular（窗口正常入坞）。
- **彻底路由（spec D3）**：CLI `--settings`/`--history`/`agents monitor`、弹窗导航「设置/历史」按钮（`commands::open_settings`/`open_history`）一律经宿主自有 IPC（`gui_host::host_open` → `gui-host.sock`）打开窗口；宿主不在则先 `spawn_detached` 再轮询重连，全程失败才回退「本进程直接建窗」兜底。历史窗口的项目过滤经 `OpenWindow{project}` 字段传给宿主（宿主自身 cwd 无意义），URL 携带 `project`/`projectName`，`HistoryView.vue` 优先用之。
- **设置/历史浮于置顶弹窗之上**：`create_settings_window`/`create_history_window` 接受显式 `pin_above_popup`（与置顶弹窗同级，新建获焦后压在其上）。弹窗与设置/历史**同进程**时由 `app::popup_pin`（本进程有 popup 且弹窗置顶）判定；统一 GUI 宿主里弹窗在**另一进程**（daemon 拉起的助手），宿主无 popup 窗口可探测，改据 daemon 在途请求数判定（`always_on_top && TrayState.active_requests>0`），并对新建窗口显式 `set_focus`（宿主是 accessory app，不会自动激活）。
- **三态开关 `general.menuBarIcon`**（`config.rs::MenuBarIconMode`，默认 `off`）：
  - `off`：不显示图标（宿主仍按需被拉起以承载窗口，末窗关闭后退出）。
  - `active`：daemon 运行时显示图标；daemon 空闲退出且无窗口后图标消失、宿主退出。
  - `always`：图标常驻（**装登录项开机自启**，`integrations/login_item.rs`）；daemon 停止时图标转「停止」态、宿主不退。
- **图标三态资源**（`src-tauri/icons/tray/tray-{idle,active,stopped}.png`，48×36 即 @2x）：机器人头造型，idle（睁眼）/ active（机器人+「?」角标，有待答）/ stopped（睡眼+月牙，always 模式 daemon 停）。纯黑 ink + alpha（「?」与月牙缺口为透明洞）；统一单色——macOS 当作模板图（随明暗菜单栏自动反色），Linux 原样显示。三态画布等大、头部同位，托盘按 18pt 高缩放（保宽高比）故三态头大小一致、切换不跳动。这三张**最终图标由设计稿手工合成**；`scripts/gen_tray_icons.py` 仅负责从 `icons/tray/source.png` 抠出透明素材（`icons/tray/cutouts/`）供手工合成，**不会覆盖**最终图标。
- **生命周期解耦（spec D5）**：托盘状态订阅（`ClientMsg::TraySubscribe` → daemon `handle_tray_sub`）是**非保活**的——daemon `handle_tray_sub` 进入时 `active.fetch_sub(1)`、退出时 `fetch_add(1)` 抵消连接计数，且空闲判定不引用 `tray_subs`，故图标本身**不**给 daemon 续命。而宿主里**任一窗口打开期间**会另开一条普通连接（`keepalive_task`）计入 daemon `active` 给其续命，末窗关闭即断、daemon 重新计时空闲退出。宿主退出判定：有窗口绝不退；off=末窗关即退；active=daemon 断连且无窗口即退；always=常驻。
- **daemon ↔ 宿主**：daemon 启动 / 配置变更时按 `menu_bar_icon != off` 且托盘可用兜底 `maybe_spawn_gui_host`（单实例去重）；`ServerMsg::TrayState`（running/version/uptime/active_requests/im_connections/draining/agents_working/agents_idle/update_available/update_latest/pending）连上即推一帧、状态变化即推（提问受理、agent 忙闲、IM 连接、更新态、drain 等触发 `broadcast_tray_state`）。宿主侧重连**事件驱动**：`spawn_daemon_sock_watch` 用 notify 监听 `daemon.sock` 所在目录，daemon 起停（socket 创建/移除）即唤醒订阅循环立即重连，配 30s 兜底超时——取代「daemon 关着时每 2s 盲连」的忙轮询。
- **托盘菜单（spec D7）**：状态区（运行/停止、版本、运行时长、待答数、agent 忙闲、IM 连接、更新可用/待生效，均 disabled）+ 操作区（设置/历史/Agent、检查更新、应用更新[复用 self-update，drain 生效不打断]、启动/重启/停止 daemon）。**菜单稳定性**：宿主长期持有**同一个** `Menu` 对象，刷新时只 `remove_at`+`append` **原地增删条目**，绝不 `tray.set_menu` 换新对象（替换 `NSStatusItem.menu` 会把用户正展开的菜单关掉）；并以「渲染内容签名」(`menu_signature`：语言+在线态+各状态字段) 比对，内容不变就整次跳过、连图标都不动——故 daemon 持续停止时展开的菜单不会被周期刷新挤掉。语言/状态变化即原地重填（宿主监听 `config.json` 实时切换）。
- **宿主二进制换新（spec D11）**：宿主长命，故周期（15s）比对自身盘上二进制指纹，变化且无窗口时释放锁后自我 `spawn_detached` re-exec（macOS always 模式交 launchd KeepAlive 重启）；新实例捕获新指纹，不会循环。

## CLI 配置与 Agent 集成（headless / 无 GUI）

> 需求 `docs/specs/cli-config.md` + 计划 `docs/plans/cli-config.md`。让 Linux 服务器 / 容器 / SSH 等无 GUI 环境**纯命令行**完成全部渠道配置与 agent 集成，且**可脚本化一次性执行**。四个顶层子命令组（`channel`/`agents`/`config` + `doctor`），每组及子命令都有 `help`，所有输出经 `cli/cfgio.rs::t` 中英双语本地化。

- **复用而非重写**：渠道连通性 `test` / userId·openId 自动识别 `detect` 直接调 `commands.rs` 既有 `*_test`/`*_detect_prepare`/`*_detect_wait`（参数结构体字段已 `pub`，密钥经 `fallback_secret` 回退已存值）；配置读写走 `config.rs::{load,load_without_secrets,save}`（save 自动把密钥写钥匙串、文件 0600）；集成走 `integrations::{agent_rules,cursor_hook,claude_hook,agent_lifecycle,mcp_config,agent_mode}`；agent 状态走 daemon `AgentsSubscribe`/`AgentsState`。落盘后 daemon `config_watch` 自动热重载。
- **`channel`**（`channel_cmd.rs`，name ∈ telegram|dingding|feishu|slack）：`list [--json]`（启用/配置齐全/已连接；daemon 未运行时连接态文本显 `—`、JSON 为 `null`）；`set <name>` 二合一——**终端且无 flag → 交互向导**（逐项提示、密钥隐藏输入、留空保留）、**带 flag → 非交互脚本**（`--enable/--disable` + 各非密钥字段 kebab flag）；`enable|disable`；`test`；`detect [--save]`（prepare 取识别码 → 提示发给 bot → wait 经 daemon 单连接捕获 → 可保存；telegram 无 detect）。
- **密钥输入（D4，脚本安全）**：仅 `--<field>-env <VAR>` / `--<field>-file <path>` / `--<field>-stdin`（或值 `-`）；交互时隐藏输入（Unix termios 关 echo）；**密钥明文不进 argv**（避免泄漏 shell 历史 / `ps`）。
- **`agents`**（`agents_cmd.rs`，agent ∈ cursor|claude|codex）：`monitor [--json|--text]`（见上节）；`mode <agent> [none|cli|mcp]`（省略模式则查询当前模式 + 是否需更新，带模式则一键切换、复用 `agent_mode::set` 自动卸旧装新）；`show [<agent>]`（打印 `prompts::cli_reference()` 手动集成提示词 + 各 agent 粘贴位置 + 当前模式/rules/hook/mcp/lifecycle 安装状态）；`install/uninstall/update <agent>` **必须显式** `--rules`/`--hook`/`--mcp`/`--lifecycle`（无默认捆绑，D6；`--hook` 仅 cursor/claude，codex 跳过；`--mcp` 写 MCP server 配置；`--lifecycle` 实验性；lifecycle 无独立 update→重装即刷新）。
- **`config`**（`config_cmd.rs`，兜底）：`show [--json]`（密钥脱敏 `●●●`）/`get`/`set`/`unset`/`path`，点号 camelCase 键。`set` 非密钥键按目标 JSON 类型强制（bool/数字/字符串/枚举）→ 反序列化校验 → save；**密钥键**（5 个，`cfgio::SECRET_KEYS`，与 `secrets::ACCOUNT_*` 一致）自动路由进钥匙串，值仍只从 env/file/stdin 取。`unset` 重置默认（密钥 → `secrets::delete`）。
- **`doctor [--json]`**（`doctor.rs`）：一屏体检 daemon（运行/版本/在途/IM 连接）+ 各渠道（启用·齐全·连接）+ 各 agent 集成（当前 mode + rules·hook·mcp·lifecycle 装没装/需更新）。

## MCP 支持（CLI 模式之外的第二种集成形态）

> 需求 `docs/specs/mcp.md` + 计划 `docs/plans/mcp.md`。动机：Codex 等 agent 无法为 CLI 工具调用延长超时（命中即可能被取消），而 MCP 协议允许配置较长的 `tool_timeout_sec`，让长等待可靠。

- **形态**：`AskHuman mcp` 以 **STDIO** 跑一个 MCP server（rmcp SDK），对外暴露**单一工具 `ask`**（配置中 server 名 `askhuman`）。
- **薄壳复用**：MCP server 不自己实现 ask 逻辑——每次 `ask` 调用就 `spawn` 一个现有 `AskHuman … --output json` 子进程，**复用全部既有 ask 流程**（弹窗 / IM / 抢答 / 历史 / 落盘 / 排空与自动重连）。子进程带 `ASKHUMAN_FROM_MCP=1`，CLI 据此在 `TaskRequest.from_mcp` 标记来源；daemon 对 MCP 来源的会话活动**仅刷新（touch_activity）而非新建工作会话**，避免长寿 MCP server 携带过期 session_id 造成「幽灵工作会话」。全平台同一套；daemon 换新/重启后下次调用自然重连。
- **入参（精简）**：`ask` 仅暴露 `message`（**恒按 Markdown 渲染**）/`questions[{question, options[{text, recommended}]}]`/`files[]`；不暴露 `markdown`（恒 on）、`single`、`selectOnly`（属脚本/纯文本场景）。
- **输出**：`ask` 声明 input/output schema；子进程 JSON 解析为 `AskResult`（**剔除脚本专用的 `selected_indices`**）→ 返回 `structuredContent`（结构化 JSON）+ `content`（序列化 JSON 文本 + 人类回复中的图片读回转 `ImageContent`）。**取消时顶层带 `status` 引导文案**（要求模型重新确认直到用户明确答复，不得当作放行）；该字段由 CLI `--output json` 顶层产出（取消路径才有），薄壳原样透传，脚本侧亦受益。
- **三态集成**：每家 agent 的「自动集成」改为 **None / Cli / Mcp 互斥三态**（`integrations/agent_mode.rs`）。Cli 绑定 Rule(CLI 版)+超时 Hook；Mcp 绑定 Rule(MCP 版)+MCP 配置（`integrations/mcp_config.rs` 写用户级全局：Cursor `~/.cursor/mcp.json`、Claude `~/.claude.json`、Codex `~/.codex/config.toml`，最小编辑保留用户内容）。提示词分 `prompts::{cli_reference,mcp_reference}` 两版。lifecycle hook（turn 追踪）与三态正交，独立开关。
- **MCP 工具超时**（长等待不被取消，各家机制不同）：
  - **Codex**：写 `tool_timeout_sec=86400`(秒) + `startup_timeout_sec=30`。✓
  - **Claude Code（CLI）**：默认 60s（MCP TS SDK `DEFAULT_REQUEST_TIMEOUT_MSEC`），故在 `mcpServers.askhuman` 写 `timeout=86400000`(**毫秒**，24h) 覆盖默认（`CLAUDE_TOOL_TIMEOUT_MS`）；否则等待人类超 60s 会被 `-32001` 取消。`needs_update` 一并校验该值（旧条目无 timeout → 提示更新）。
  - **Cursor**：MCP 工具/elicitation 超时 ~60s **硬编码、不可配置**，无法支撑长等待——故不写 `timeout`，且 Cursor 推荐用 CLI(+Hook) 模式。
- **入口**：设置页 Agent Tab 三态分段控件 + 手动集成卡的 CLI/MCP 切换；headless 走 `agents mode` / `agents install --mcp` / `doctor`（见「CLI 配置与 Agent 集成」节）。手动集成卡的 MCP 配置示例（JSON/TOML）**直接填入当前可执行文件绝对路径**（`mcp_command_path` 命令读 `current_exe()`，取不到时退回占位符）并各带**拷贝按钮**，免用户手改路径。JSON 示例含 `timeout: 86400000` 并注明「仅 Claude 需要」（Cursor 忽略该字段）。

## 用户级 hooks + 内置弹窗提示音

> 代码：`src-tauri/src/hooks.rs`（通用 hooks）、`src-tauri/src/sound.rs`（提示音）。

- **用户 hooks（`~/.askhuman/hooks/`）**：通用机制——每个事件对应一个**同名可执行脚本**（靠 shebang 选解释器）。命中即调用：`ASKHUMAN_EVENT` 等简要字段经环境变量传入，**完整负载经 stdin JSON** 传入。非阻塞 fire-and-forget（后台线程 `wait` 回收子进程，避免 daemon 僵尸）。仅 unix 触发，其它平台空操作。
  - 首个事件 **`ask-received`**：**收到一次提问即触发**，与弹窗是否弹出无关（headless / 仅 IM 也会触发）。触发点在 daemon `handle_submit`（规范单点，每次提问恰一次）。负载含 `requestId/source/project/isMarkdown/message{text,files}/questions[{message,options}]`。
  - daemon 启动时 `hooks::ensure_sample()` 落盘参考示例 `~/.askhuman/hooks/ask-received.sample`（**非可执行、默认不触发**；含播放声音 / 桌面通知示例）。复制为无后缀的 `ask-received` 并 `chmod +x` 即启用。可扩展：新增事件只需定义事件名 + 负载并在对应时机 `hooks::fire(...)`。
- **内置弹窗提示音（`general.popupSound`，默认空=关）**：弹窗出现时（GUI Helper 的 `app::launch`→`View::Popup`，`win.show()` 后）按配置播放。便利项，与 hooks 相互独立。
  - macOS：`afplay /System/Library/Sounds/<name>.aiff`；设置页下拉列**实际可用音名**（读 `/System/Library/Sounds` 等目录）。
  - Linux：仅当检测到播放器（`canberra-gtk-play` / `paplay` / `pw-play` / `ogg123`）时设置页**才显示**该项（`popup_sound_support()` 返回 `toggle`），播放 freedesktop 提示音；检测不到返回 `none` → 整项隐藏。Windows 返回 `none`、不显示。
  - 设置→通用→弹窗行为：下拉（关闭 / 音名或开关）+「试听」；播放统一为非阻塞 spawn 播放器 + 后台线程回收。

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
