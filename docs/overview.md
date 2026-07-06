# AskHuman 项目概览（供 agent 参考）

> 跨平台「Human-in-the-loop」工具：命令行 `AskHuman` 在需要人类确认/补充时弹出窗口收集回应，并把结果按固定区块格式写到 stdout 供 AI 读取。

## 技术栈与形态

- **Tauri 2**：Rust 后端 + WebView 前端，单一可执行文件 `AskHuman`，跨 macOS / Windows / Linux。
- **前端**：Vue 3 + Vite + TypeScript，纯手写 macOS 风 CSS（无组件库）。
- **运行模型（当前）**：单进程。既是 CLI（纯信息命令直接终端输出、不起 GUI），又能进程内启动 Tauri 事件循环弹窗。**stdout 只输出结果区块，所有日志走 stderr。**
- **运行模型（规划中）**：正迁往「常驻 Daemon + 瘦客户端 CLI + 独立 GUI Helper」三进程架构（见下节）。本文「运行流程」等小节描述的仍是当前单进程实现，迁移完成后再整体替换。
- **Grok CLI 集成调研**：Composer 2.5 与 Grok Build 使用不同 harness，当前没有经验证可同时覆盖二者的持久化全局 rules；建议采用用户级 Grok skill + MCP per-tool timeout。完整证据与边界见 `docs/specs/grok-cli-integration-research.md`。

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
  - **弹窗预热（方案6，`general.popupPrewarm` 默认开，Unix）**：Daemon 另常驻 1 个 `askhuman --popup --warm` helper——已建窗（隐藏）+ 已挂载前端、无请求待命（入 daemon「热池」`warm_pool`，**非保活**）。来请求时 `dispatch_popup` 优先把请求**领用**给热实例（发既有 `Show` 直接上屏，省掉冷 spawn + WebView 初始化），用后后台 `maybe_topup_warm` 重建；池空/并发第 2+/无显示(`has_display`)/drain 时透明回退冷 spawn。热实例**单次使用**杜绝旧内容残留；领用时前端 `nextTick` 等正文进 DOM 后**直接后端 `popup_show_window` 上屏**（show 不依赖 rAF，故锁屏/息屏也能上屏）。**注意领用上下文（project/source/agent_*）只写入 `WarmPopup.show`、不回写只读托管态 `AppState`**（热 helper 的 `AppState.project` 恒为空串）——故所有需要这些字段的命令（`popup_init`、`open_history` 等）都必须优先读 `WarmPopup.show`、无领用槽再回退 `AppState`，否则热弹窗下会拿到空值（历史按钮曾因读 `AppState.project` 默认过滤到空项目而看不到最近历史）。开关关 / daemon 空闲退出 / drain 换新即 `recycle_warm` 回收。**macOS**：待命期热进程设 `ActivationPolicy::Accessory`（不入 Dock/Cmd-Tab，否则常驻一个空 Dock 图标），领用上屏时 `finalize_popup_show` 切回 `Regular`，与冷弹窗一致入坞 + Dock 角标。详见 `docs/specs/popup-prewarm.md`。
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
                             (选中/打开/预览/拖出/右键) + 拖入回复文件胶囊 + 底部操作条；
                             message 下方右对齐小工具条（复制 Message + Markdown/源码切换，切换作用于整篇 message+所有问题，默认 Markdown、纯前端视图态不回传；仅在有共享 Message 时显示，直接提问无 message 则整条隐藏）；
                             多问题(n>1)且开实验开关 experimental.vertical_questions 时纵向平铺所有题卡片（.q-card），当前题(active)左侧 accent 高亮，上一个/下一个=滚动定位（详见下文「多问题纵向列表」）；开关关=旧版一次一题左右滑动
    views/AgentsView.vue     (实验性) Agent 状态窗口：按类型(Claude/Codex/Cursor/Grok)分组、状态优先排序
                             (工作中>空闲>已结束)、相对时间动态刷新；订阅 daemon 推送的 agents-updated；
                             每卡「发送消息」入口（grok/ended 无）+ 待送达「插话」徽标 + 行内确认撤回
    views/InterjectView.vue  (实验性) 插话 composer 窗口：头部（agent 家族胶囊+项目名）+ 预填待送达全文的
                             textarea + 取消/发送（提交=整体覆盖队列；Cmd/Ctrl+Enter 发送、Esc 取消）
    views/SettingsView.vue   设置：通用（含「回复历史」保留条数 + 超额「立即清理」+ 3 题综合示例的「弹出测试窗口」
                             + 底部隐蔽开关「实验性功能」）
                             / Agent / 通信渠道（+ 开启实验后出现「实验」Tab）多 Tab
                             （Agent Tab：顶部原理说明 + 「手动集成」参考提示词卡[CLI | MCP 分段切换，MCP 视图附三家 MCP 配置示例片段，绝对路径占位] + 「自动集成」按 Cursor/Claude Code/Codex/Grok 分组、每家一个 **CLI | MCP | 未集成** 三态分段控件[一键切换自动卸旧装新；推荐档位带绿色「推荐」标记：Cursor/Claude=CLI、Codex/Grok=MCP（Codex 无法延长 CLI 超时；Grok Composer CLI 自动后台化）。**Grok 仅 MCP | 未集成 两态**（隐藏 CLI 分段）、指令载体为 skill]，下方按当前模式列出产物：CLI=Rule+超时 Hook[Codex 无 Hook 给提示]、MCP=Rule/skill+MCP 配置[Grok=skill]；**产物过期/缺失时按产物粒度提示**：每个产物行「打开」左侧出现橙色「更新」按钮（仅刷该项，`agent_mode_update_artifact`，与 `needs_update` 同口径的 `artifact_updates` 驱动 `ruleNeedsUpdate/hookNeedsUpdate/mcpNeedsUpdate`），「自动集成」标题下另有跨三家的**待更新总览**（统计各 X 个 Rule/Hook/MCP 需更新 + 「全部更新」一键逐家 `agent_mode_update`），有任意待更新时给「Agents」Tab 标加橙色提示点；文件行「打开」下拉：在 Finder 中显示 / 用默认程序打开）
    views/HistoryView.vue    独立历史窗口：顶部项目下拉 + 清空菜单 + 搜索条（关键字空格分割 AND 匹配 message/题干/选中选项/输入回答/文件名/workspace 名与完整路径/agent 家族名(中英)/来源名/渠道名，作用于项目筛选结果之上、左列表摘要与 workspace 行高亮命中词）；左列表（渠道徽标/agent 徽标/相对时间/摘要/workspace 灰字行）右只读详情
    components/HistoryDetail.vue 只读还原一条历史（状态横幅 + 来源 meta 胶囊行[agent 家族/自定义来源名/workspace 点击在文件管理器打开] + 消息/附件 + 每题选项高亮/文本/图片/文件，best-effort；「复制」文本含 meta 行）
    lib/history.ts           历史共用纯函数：agentKindOf（agentKind 缺省时按旧记录 source 家族名回退）/ workspaceNameOf / customSourceOf
    lib/ipc.ts               invoke 封装（与后端命令一一对应）
    lib/types.ts             与 Rust 模型对齐的 TS 类型
    lib/markdown.ts          markdown-it 渲染（fence/code_block 包 `.code-block` + hover 拷贝按钮 `.code-copy`，
                             labels 经 env 本地化；`handleCodeCopyClick` 委托复制；样式在 controls.css 全局）
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
        args.rs              提问参数解析（message / --stdin / -o / -o!(推荐选项) / -f /
                             --single(单选) / --select-only(严格，须每题有选项) / --output <text|json>）。
                             Markdown 渲染恒开（`--no-markdown` 已移除，is_markdown 恒 true，弹窗内可临时切源码视图）
        cfgio.rs             CLI 配置公共工具：点号路径 get/set（serde_json::Value）+ 类型强制 + 密钥识别 +
                             密钥取值(env/file/stdin/隐藏输入) + 脱敏 + 交互输入 + block_on 助手
        config_cmd.rs        `config show|get|set|unset|path`（通用键值兜底；密钥键自动入钥匙串，值不进 argv）
        channel_cmd.rs       `channel list|set|enable|disable|test|detect`（向导+脚本；复用 commands 的 test/detect）
        agents_cmd.rs        `agents monitor|mode|show|install|uninstall|update`（状态文本/json + 三态模式编排 +
                             细粒度集成 --rules/--hook/--mcp/--lifecycle；复用 integrations）
        doctor.rs            `doctor [--json]` 一屏体检（daemon/渠道/集成，集成含 mode/rules/hook/mcp/lifecycle）
        debug_cmd.rs         隐藏调试组 `AskHuman debug …`（不进 help）：dd-watch-poc 钉钉 watch 卡
                             高频更新探针（建卡+循环更新+终态+回调打印；--count 0 只发样式预览卡）
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
      watch.rs               `/watch` 实时关注纯逻辑（四渠道）：订阅持久化(state/watch.json) + 结构化帧/签名 +
                             共享文案构件 + 飞书卡片视图 + 渠道门控 + 本地时区绝对时刻（TG/Slack/钉钉
                             渲染在 telegram/watch.rs、slack/watch.rs、dingtalk/watch.rs；见「IM /watch」节）
      select.rs              通用「单选卡」纯逻辑（传输无关，spec `docs/specs/im-select-card.md`）：
                             SelectOption{id=session_id,label,badge} / SelectView + 上限截断 + 按命令种类
                             本地化标题 + agent_options（工作中在前、跳已结束、`· 关注中`徽标）。渲染在
                             feishu/card.rs::build_select_card + dingtalk/select.rs + telegram/select.rs +
                             slack/select.rs（飞书/TG/Slack 就地回卡·变身 / 钉钉模板+变量另发卡、见「IM 单选卡」节）
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
        watch.rs             /watch 卡钉钉渲染：专用模板变量 build_watch_param_map + parse_watch_action
        router.rs            DdRouter：独占 StreamConn + 按 outTrackId/senderStaffId 分发
                             (提交/watch 回调带 oneshot 交认领方裁决→回包；其余/孤儿回空 ACK)
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
        agent_lifecycle.rs   (实验性) 四家生命周期 hook 安装/卸载/状态：**用户级**、与 timeout hook 独立共存
                             （仅增删命令含 `__agent-hook` 标记的条目，保留其它 hook/格式）。Claude
                             `~/.claude/settings.json`(Nested) / Cursor `~/.cursor/hooks.json`(Flat) 用 jsonc CST；
                             Codex `~/.codex/hooks.json` + `config.toml [hooks.state]` 信任哈希用 toml_edit
                             （复刻 codex `version_for_toml`：sha256(紧凑·键排序 JSON(归一 identity))，键
                             `<hooks.json 绝对路径>:<event_snake>:<g>:<h>`，见 demo codex-trust.cjs）；Codex 无 SessionEnd。
                             Grok `~/.grok/hooks/askhuman-lifecycle.json`(Nested，与 Claude 同构、全局恒受信任无需哈希)，
                             事件最全（含 StopFailure+SessionEnd）。**坑**：Grok 默认兼容加载 `~/.claude`/`~/.cursor`
                             的 hook，会与其原生 hook 一起触发 → 靠 reporter 去重（见 agents/report.rs）
        agent_rules.rs       Agent 全局 Rules 安装/更新/卸载/状态/open/reveal：三者均用 AskHuman:begin/end
                             托管区块写入，保留区块外用户内容（Cursor ~/.cursor/rules/askhuman.mdc 另带
                             alwaysApply frontmatter，卸载时区块外仅剩 frontmatter/空白才删整文件；旧版
                             独占文件含 MANAGED_FILE_MARK 仍识别为已安装、安装/更新时迁移为区块格式）；
                             Claude ~/.claude/CLAUDE.md、Codex ~/.codex/AGENTS.md。needs_update：区块内
                             正文 ≠ 最新提示词（或旧版无区块）→ 需更新（幂等纯函数 + 单测）。
                             `Variant`(Cli|Mcp) 区分写入 cli_reference/mcp_reference；`installed_variant`
                             探测已装变体、`needs_update_variant`/`install_variant` 变体感知。
                             `AgentTarget::Grok` 的指令载体**不是** rules 文件而是 skill，故 grok 的所有
                             查询/安装/卸载/打开一律**委托** `grok_skill`（Variant 对 grok 无意义、恒 MCP 语义）
        grok_skill.rs        (Grok 专属) 指令载体 `~/.grok/skills/interaction-protocol/SKILL.md`：Composer 不读
                             全局 `~/.grok/AGENTS.md`，改以 skill 承载交互协议（`prompts::grok_skill_body`，带
                             frontmatter + `SKILL_MARK` 归属标记）。**skill 定位为「无条件必读的交互协议」**：
                             frontmatter `description` 是唯一常驻注入 prompt 的字段（skill 正文需判定相关后才加载，
                             实测两 harness 均如此；`when-to-use` 仅以 `Use when:` 拼在 description 后，无额外价值故不用），
                             故其第一句无条件要求「每个 session 先读本 skill」，并前置兜底事实「普通输出人类不可见、
                             只能经 `ask` MCP 工具送达」——消解「需要提问才加载」的自指悖论。正文 = `mcp_reference()`
                             原样 + 末尾一小段 Grok 说明(单一来源、协议措辞不漂移):只声明一条「联系人类」降级阶梯——
                             ①优先 `ask` MCP 工具(MCP 优先于 shell/CLI;但其它 shell 调用不受限);②没列出时先用搜索机制找到;
                             ③仍够不到 MCP 则退回其它提问渠道(如 CLI 版 AskHuman),**绝不**退化成普通输出/直接结束回合。
                             **刻意不写死具体 harness / 工具名**(Grok 后续会变)。
                             content/is_installed/needs_update/install/update/uninstall/reveal/open
        mcp_config.rs        MCP server 配置写入（用户级全局，server 名 `askhuman`、args `["mcp"]`）：
                             Cursor ~/.cursor/mcp.json、Claude ~/.claude.json 走 jsonc-parser CST（mcpServers.askhuman）；
                             Codex ~/.codex/config.toml、Grok ~/.grok/config.toml 走 toml_edit（[mcp_servers.askhuman] +
                             startup_timeout_sec/tool_timeout_sec；**Grok 额外写 `tool_timeouts = { ask = 86400 }`**
                             per-tool 超时，对默认模型 Composer 更精准）。TOML 数值比较**容忍整值浮点**（Codex/Grok
                             CLI 会把 `30`→`30.0`，只按整数比会陷入「永远需更新」死循环）。最小编辑保留用户其它内容/
                             注释/格式；install/update/uninstall/is_installed/needs_update/display_path/reveal/open（幂等纯函数 + 单测）
        agent_mode.rs        三态模式编排（None|Cli|Mcp 互斥）：Cli=Rule(CLI)+超时 Hook，Mcp=Rule(MCP)+MCP 配置；
                             `current`(**以产物 MCP配置/超时Hook 为首要信号**，互斥且由 set 维护、稳定；产物不
                             明确时才回退 Rule 变体——避免内置提示词改版后已装旧正文失配被错判模式) /
                             `needs_update` / `set`(卸非目标产物→装目标，幂等) / `update`(刷当前模式) /
                             uninstall_all。lifecycle hook 与三态正交。`agent_rules::classify_body` 对漂移正文
                             用结构信号（是否含 `Shell/Bash`）判 CLI/MCP，提示词改版仍稳定归类
      ipc/                   IPC 协议：mod.rs(消息类型，含 ServerMsg::UpdateState/TrayState、ClientMsg::TraySubscribe) /
                             codec.rs(NDJSON) / transport.rs(Unix socket)
      gui_host/              (Unix) 宿主自有 IPC（与 daemon 解耦）：mod.rs(HostMsg{OpenWindow{kind,all,project,
                             session,agent,cwd}/Ping/Shutdown}，kind 含 Interject[插话 composer，每 session 唯一，
                             label=interject_label(session) 哈希] + gui-host.sock bind/connect +
                             host_open 客户端[连不上则 spawn 宿主再轮询] + 单实例)
      client/                (Unix) CLI 作为 Daemon 客户端：连接/握手/自启/submit/detect/status/stop；
                             composer.rs(插话 composer 窗口 ↔ daemon 专属连接管理：open 登记+预填查询、
                             submit[连接死则一次性连接兜底]、close_by_label[窗口销毁即断开=放行等待 hook])
      daemon/                (Unix) 常驻 Daemon：mod.rs(分发/serve + 自更新后台检查/广播/指纹感知 +
                             handle_tray_sub[非保活]/broadcast_tray_state/maybe_spawn_gui_host) /
                             lifecycle.rs(单实例·指纹·空闲) / spawn.rs(脱离启动；macOS 非 Aqua 会话经 gui/uid launchd 拉起) /
                             request.rs(请求登记表·Coordinator·GUI token·broadcast_to_guis) /
                             config_watch.rs(notify 监听 config.json + 去抖)
      update/                版本自更新：mod.rs(检测/比较/Updater/select/check) / direct.rs(GitHub 资产替换) /
                             npm.rs(npm i -g) / notes.rs(release notes 取/聚合) / state.rs(update.json)
      agents/                (实验性, Unix) Agent 生命周期追踪：mod.rs(AgentKind=claude/codex/cursor/grok +
                             LifecycleEvent=session-start/turn-start/turn-end/session-end/activity[回合内
                             Pre/PostToolUse 工具心跳，刷活动时间+保持工作中、不结束回合]) /
                             detect.rs(按 env 判真实运行家族[**Grok 最先判**：它每个 hook 都注入 CLAUDE_PROJECT_DIR
                             且兼容触发 claude/cursor hook，靠 GROK_HOOK_EVENT/GROK_SESSION_ID 认出；其后 Cursor 双触发去重] /
                             session_id 解析[grok=GROK_SESSION_ID] / walk 进程树定位 agent pid[grok 按 comm/argv0 含 `grok`] /
                             walk_any_agent[env 判不出时按进程树兜底拿 kind+pid，MCP 模式专用] / kill-0 存活) /
                             report.rs(reporter 去重：`running==Grok && intended!=Grok` 一律跳过[只认 grok 原生 hook，
                             滤掉兼容触发的 claude/cursor]；`intended==Claude && running==Cursor` 跳过[cursor 兼容 claude 双触发]) /
                             title.rs(四家会话标题解析：cursor meta.json / codex·claude jsonl / grok summary.json 的
                             session_summary|generated_title，回退 chat_history.jsonl 解包 `<user_query>`) /
                             registry.rs(AgentRecord 注册表：apply_event 推导 工作中/空闲/已结束[Activity
                             状态不变时返回 false 不落盘/广播，避免工具心跳刷屏]、
                             touch_activity[按 session 刷新] / touch_activity_by_pid[MCP 兜底：按 pid 刷新已存在 session]、
                             refresh_by_pids[按 pid 刷新活动，供在途 ask 豁免]、
                             working_backstop_sweep[WORKING_BACKSTOP_SECS=30min 兜底：工作中且久无活动且无在途 ask
                             → 降级空闲，兜 Claude 打断/API 错误等无 hook 场景]、
                             poll_liveness、ttl_sweep[1h 兜底]、ended 最多留 10 条、persist/load
                             ~/.askhuman/agents.json、snapshot 推送、tray_agent_infos[托盘 Agent 子菜单摘要]) /
                             report.rs(隐藏子命令 `__agent-hook <agent> <event>` 上报器：去重+解析+发 daemon；
                             PreToolUse 事件带 `interject_poll=true` 等 daemon 一帧插话裁决 → deny JSON 带话) /
                             interject.rs(插话队列 InterjectStore：内存 per-session entries+composer 计数+
                             等待 hook，poll 三态[Message 原子出队/Hold 挂起/None 放行]、submit 覆盖/append 追加/
                             clear 撤回、变更时原子落盘 ~/.askhuman/state/interject.json)
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
- Agent 全局 Rules：`agent_rule_status`（含 outdated）/ `install` / `update` / `uninstall` / `reveal` / `open`（入参 `agent`：cursor/claude/codex/grok；grok 委托 `grok_skill` 走 skill 文件）
- Agent 三态模式：`agent_mode_status`（返回 mode + 各产物装没装/需更新 + 路径）/ `agent_mode_set`(none|cli|mcp) / `agent_mode_update`（刷当前模式产物）/ `mcp_config_reveal`·`mcp_config_open`·`agent_hook_reveal`·`agent_hook_open`（定位/打开配置）/ `mcp_command_path`（当前 exe 绝对路径，供手动卡示例填充）
- MCP 配置：`mcp_config_reveal` / `mcp_config_open`（入参 `agent`：cursor/claude/codex/grok）
- 超时 Hook 文件：`agent_hook_reveal` / `agent_hook_open`（入参 `agent`；Codex/Grok 无 Hook 为 no-op）
- Telegram：`telegram_test`
- 钉钉：`dingtalk_test` / `dingtalk_detect_prepare` / `dingtalk_detect_wait`
- 飞书：`feishu_test` / `feishu_detect_prepare` / `feishu_detect_wait`
- Slack：`slack_test` / `slack_detect_prepare` / `slack_detect_wait`
- 自动识别取消：`detect_cancel`（三家共用）。识别「等待」最多阻塞 120s，UI 在识别中显示「取消」按钮调用本命令。机制：`commands.rs` 进程内单槽 `Notify`，`*_detect_wait` 经 `detect_with_cancel` 与 `notified()` 竞速；取消即 drop 掉等待 future——走 daemon 的路径会关掉控制连接，daemon `handle_detect` 用 `select!`（识别 vs `wait_conn_closed`）感知断连即中止并释放临时长连接；进程内回退路径则直接 drop 临时 WS。
- 版本自更新：`get_app_version` / `update_check`(manual) / `update_get_notes`(aggregate) / `update_apply`(落盘+进度事件) / `update_dismiss` / `popup_update_state`(弹窗拉初值) / `restart_settings`(设置进程重开)
- (实验性) Agent 生命周期：`agents_init`(状态窗口主题+语言) / `agent_lifecycle_status` / `agent_lifecycle_install` / `agent_lifecycle_uninstall`（入参 `agent`：claude/codex/cursor/grok）/ `agent_force_idle`(按 session_id 手动把卡「工作中」的 agent 置空闲，发 `ClientMsg::AgentForceIdle` 给 daemon)

窗口拖拽用 `data-tauri-drag-region`（导航栏/底部空白/设置 tab 栏）；置顶用前端 `@tauri-apps/api/window` setAlwaysOnTop。
文件拖入用 `onDragDropEvent`（原生路径）；`-f` 附件拖出用 `tauri-plugin-drag` 的 `startDrag`。
来源名（弹窗标题 / Telegram 消息头「Question from {名称}」）的解析优先级为 **自定义环境变量 `ASKHUMAN_ENV_SOURCE_NAME` > 探测到的发起 Agent 展示名（Claude Code/Codex/Cursor/Grok）> 默认「the Loop」**（后端 `models::source_name_for_agent`，CLI 构造 `TaskRequest` 时以 `detect_caller_agent` 的家族解析；供渠道消息头 + 历史共用。MCP 模式 env 判不出家族 → 回退「the Loop」）。弹窗导航栏标题旁还显示两枚浅灰圆角胶囊（`.brand-chip`，`pointer-events:auto` 以便 hover/点击，导航栏其余可拖拽；窄窗时标题先截断、胶囊尽量保留完整）：

> 标题「胶囊内联」（前端 `PopupView`）：当探测到 agent 且未定制来源名时（`sourceName` 等于默认或等于 `agentLabel`），把标题里的来源文字替换为 **agent 胶囊本身**——按 `popup.messageFrom/questionFrom` 的 `{source}` 占位把文案切成前后两段，胶囊（agent + workspace）夹在中间，故中文渲染为「来自 [Cursor] [Project] 的消息」、英文为「Message from [Cursor] [Project]」。`.brand.inline` 下截断优先级为 **后缀 > 前缀 > 项目名**（agent 品牌名不缩，靠 flex-shrink 权重 1000≫50≫1 近似分级）。未探测到 agent → 仍「Message from the Loop」；设了自定义来源名 → 标题用自定义名文本、胶囊照旧跟随其后。

> 头部弹窗时间（`.brand-time`，紧跟头部之后的灰色小字）：显示**提问创建时刻**的相对时间（`<5s 刚刚 / <60s N 秒前 / <60min N 分钟前 / <24h N 小时前`，满 24h 转绝对 `toLocaleString()`），前端每秒 tick 走字、hover 给精确绝对时间。锚点为 epoch ms，daemon `RequestRegistry::create()` 记录 → `ShowPayload.created_at_ms` → `PopupInit.createdAtMs`（冷/单进程弹窗取构造时刻兜底；预热弹窗领用得到的即真正到达时刻）。窄窗时该时间收缩权重最高（flex-shrink 100000 + overflow 裁剪），空间不足**最先**让位于标题/胶囊。

> 多问题纵向列表（前端 `PopupView`，设计 `docs/specs/multi-question-vertical.md` / 计划 `docs/plans/multi-question-vertical.md`；**实验开关门控**）：仅当 `experimental.vertical_questions`（`config.rs`→`PopupInit.vertical_questions`→前端 `verticalEnabled`，设置页「实验」Tab，默认关）开 **且** n>1 时启用，即 `verticalMode = verticalEnabled && isMulti`；关或单题走旧版「一次一题 + 上/下一步左右滑动」（`.question-pane` + `q-slide-*` 过渡 + `qHeaderRef`/`goToSeq`/`onQuestionEntered`/`allViewed`）。纵向模式：n>1 时不再「一次一题 + 左右切换」，而是把所有题用 `v-for` 纵向平铺成卡片（`.q-card`，共享 Message/附件/工具条仍在最上方）。**当前题(active)** 指针 `current` 决定高亮 + ⌘1–9/语音目标：滚动时取「最后一个完整可见的题」（无完整可见则取首个与视口相交者，兼容超长题；`updateActiveFromScroll` 节流 + rAF），点击卡片(`@mousedown`)/聚焦输入(`onTextareaFocus`)/键盘导航(`setActive`)也切 active；键盘/按钮切换后 450ms 内锁定不被滚动回改（`activeLockUntil`）。`.q-card.active` 用 `::before` 画左侧 accent 细条 + 极淡背景（`inset` 横向铺到 `.content` 内边距边缘，纯视觉、不改排版无抖动）。**「已看到」** `visited[i]`：每题底部哨兵 `.q-sentinel` 进视口（`IntersectionObserver`，root=`.content`）或曾被设为 active 即置真；多问题发送按钮仅在最后一题已看到后出现（`lastSeen`）。**导航**：上一个/下一个 + ⌘[ / ⌘] → `goRel(±1)` 相对滚动定位（`scrollIntoView({block:"nearest"})`，不强制贴顶）；⌘↵ 非末题前往下一题、末题提交；⌘1–9 切 active 题选项。**折叠输入**：多问题补充输入框默认 1 行（`.textarea.collapsed`，scoped 提升特异性覆盖全局 `.textarea`），`expandedQ(i)` 在聚焦或有内容时还原多行高度。**每题作答状态**按索引存于 `chosenByQ/inputByQ/imagesByQ/replyFilesByQ`；DOM 引用按索引登记于 `inputRefs/cardRefs/sentinelRefs/thumbsRefs`（函数 ref），`inputRef`=active 题 textarea 供语音/autoGrow 复用。**图片归属**：拖放按原生落点命中的卡片（`questionAtPoint`，physical 坐标除 DPR → `elementFromPoint` 找 `.q-card[data-q-index]`）；粘贴归 `focusedQ ?? current`。单问题(n=1)恒 1 卡片、无高亮/不折叠/无哨兵，外观与旧版一致。

- **来源 agent badge**（在 workspace 之前）：取 `AppState.agent_kind`（提问时 CLI `detect_caller_agent` 探到的家族，随 `TaskRequest→ShowPayload→AppState` 贯穿），前端显示本地化家族名（Claude Code/Codex/Cursor/Grok）；识别不到则不显示。若该 agent 终端可激活 tab（`PopupInit.agentTerminal` = `terminal_kind(agent_pid)` ∈ 共享集 `lib/terminals.ts`）则 badge 变可点按钮 + ↗ 箭头，点击 → `focus_agent_terminal(agentPid)`。
- **来源 workspace**：取 `AppState.project`（git 仓库根 / 回退 cwd 绝对路径）经 `project::display_name` 得目录名，`title` hover 出完整路径；带 ↗ 箭头、整块可点 → `open_path(项目路径)` 在文件管理器打开。`project` 为空则隐藏。

以上字段经 `PopupInit{project, projectName, agentKind, agentPid, agentTerminal}` 上送（`commands::popup_init`，终端类型在弹窗进程对给定 pid 现取）。

> 推荐选项（`-o!` / `--option!`，见 `docs/specs/recommended-option.md`）：语义同 `-o` 且标记该选项为 AI 推荐答案（一题可多个，不预选中）。弹窗/历史详情在选项文本流开头内联显示「大拇指 SVG +『推荐』」绿色 Badge（`controls.css`：外层 `.rec-badge` 为与 `.label` 行高等高的透明对齐外框、内层 `.rec-badge-pill` 为绿色胶囊；使其与勾选框中线对齐、跨平台稳定，且换行后文本可铺满整行）；IM 渠道显示文本加本地化「👍推荐 」前缀（`channel.recommendedPrefix` + `conversation::display_text`），提交值恒为原文——其中钉钉卡片模板回传显示文本，由 `dingding::restore_selected` 还原原文，其余渠道按下标天然回原文。

## UI / 主题

- 主题三态：`system`(prefers-color-scheme)/`light`/`dark`；前端切根类 + 后端设原生窗口主题。
- macOS：`underWindowBackground` 毛玻璃 + `TitleBarStyle::Overlay` + 隐藏标题（整窗含标题栏皆玻璃），叠 0.2 色罩；Windows/Linux 退化为纯色不透明底。
- Markdown 配色见 `styles/controls.css`（链接/代码块/表头/引用/hr 等）。代码块 hover 右上角有拷贝按钮（`.code-copy`，点击复制 `<code>` 文本，弹窗/历史详情/设置发布说明共用 `renderMarkdown` 故都生效）。

## 配置

`~/.askhuman/config.json`（新位置缺失时自动回退旧 `~/.humaninloop/config.json`）：`general`(theme, language, alwaysOnTop, appearAnimation, windowEffect, speechLanguage, speechShortcut, historyLimit, popupSound, menuBarIcon[off|active|always，默认 off，仅 macOS/Linux 桌面，见「菜单栏图标」节], popupPrewarm[默认 true，Unix 弹窗预热，见「GUI Helper」节], daemonLifecycle[activity|keepalive，默认 activity，Unix，见「守护进程生命周期」]) + `channels.popup`(enabled,width,height,rememberSize) + `channels.telegram`(enabled,botToken,chatId,apiBaseUrl) + `channels.dingding`(enabled,clientId,clientSecret,userId,cardTemplateId,…) + `channels.feishu`(enabled,appId,appSecret,openId,baseUrl) + `channels.slack`(enabled,botToken,appToken,userId) + `channels.autoActivation`(「IM 会话期自动激活」开关，默认 false) + `experimental`(enabled[实验性功能开关，默认 false], vertical_questions[多问题弹窗纵向同时显示，默认 false])。缺字段走默认、未知字段忽略。用户向配置说明见 `docs/wiki/`。

> IM 会话期自动激活（`channels.autoActivation`，默认关；设计 `docs/plans/im-channel-activation.md`）：开关关＝旧「每次提问全发所有启用 IM」。开关开（UI 入口在「实验」Tab，随 `experimental.enabled` 显露；旁注「建议同时开启生命周期追踪以提高状态识别准确性」）后：同一时刻只有「活跃槽」对应的 IM 收提问卡片；在某 IM 发 `/here`（或 `/这里`）把该渠道设为活跃槽，发 `/status`（或 `/状态`）回工作中/空闲 agent 列表（每行带**稳定数字编号** `[n]`，编号在当前 daemon 生命周期内单调不变、`load()` 重排），发 `/status <编号>` 回该 agent 的**当前活动**：头部 + 空行 +「最近动态（相对时间）」分区标签 + 「最后一段助手文字」+（末尾若是工具调用再附 `▸ 工具行`）；文字/工具解析各家 transcript 尾部（工具只归一化 读取/写入/运行命令，其余显原名+参数前段；`agents/activity.rs` 复用 `title.rs::transcript_path`；设计 `docs/plans/im-status-activity.md`）。**实时当前工具**：Pre/PostToolUse hook 经 `report.rs` 把「当前工具」（原始名 + 归一化短对象）随 `AgentEvent` 上报，daemon 存 `AgentRecord.current_tool`（`serde(skip)` 不落盘、由 `snapshot()` 注入 `currentTool`；PostToolUse/回合/会话结束清除），渲染时与 transcript 尾部工具取**较新者**——解决 Cursor「工具跑完才落盘」的滞后、覆盖编译等 in-flight 场景，「最近动态」的相对时间随之取实际展示事件的时间（设计 `docs/plans/im-status-realtime-hook.md`）。**作答期收到命令**：会话层不回引导、统一交 `handle_inbound`（已注册→命令输出、未注册→help），避免「引导 + 命令输出」重复。**命令前缀**：`/` 一律视作命令；备用 `!`（`!status`/`!watch 3`）四渠道通用但仅**已知命令**生效（未知 `!xxx` 按普通文本）——为 Slack 而设（其客户端把一切 `/` 输入拦截为本地 slash command，未注册发不出来），提示/引导文案仅 Slack 展示 `!`（`autochannel::cmd_prefix`，i18n `{p}` 占位）。普通消息＝切到此渠道（文本不当答案）。**凡把活跃槽切到某 IM 都会把所有在途未答补推过去**（补推＝渠道激活的固有行为，统一在 `set_active_channel`、与触发方式无关：`/here`/普通消息/`/status` 切槽/作答切槽均同）。活跃槽**持久化**于 `~/.askhuman/state/auto-channel.json`、跨重启保留、仅由入站消息改变。忙/闲/结束判定复用 Agent 生命周期追踪（`agents/registry.rs`）；无 turn hook 时「首次提问起算工作中」（仅开关开时兜底登记）。代码：`autochannel.rs` + `daemon/mod.rs`（`ensure_inbound_listeners`/`spawn_listener` 通用循环 + `handle_inbound`/`backfill_inflight`/`attach_im_channels` 门控）。命令处理一份实现，各渠道只提供传输原语（连 Router + 原始消息观察者 + 抽取 `Inbound{发送者, 文本?}`（非文本消息 `文本=None`，仅在无在途提问时回引导）+ 期望发送者 + `build_im_channel`/`reply_channel_text` 分支）。**四家（飞书/钉钉/Slack/Telegram）均已接入并真机端到端验证 OK**。**存活即监听（spec `docs/specs/im-always-respond.md`，R1）**：daemon **启动即**后台 `ensure_inbound_listeners` 并贯穿整个生命周期（不再按 `working_count` 门控、与开关无关），随空闲退出而释放（D18）——目的是 daemon 在世时**任何消息都给回复**，让用户能与「守护进程已退出」区分。**改 IM 凭据/收件人即时重建入站监听**：`invalidate_changed_routers` 对变更渠道（凭据连带 Router；或仅 open_id/user_id/chat_id 等收件人变更）`take`+`notify` 停掉旧监听并释放认领，`on_config_changed` 末尾再 `ensure_inbound_listeners` 按新连接重建——`inbound_listeners` 改为带 stop 信号的注册表，释放按 `Arc::ptr_eq` **身份安全**（旧任务迟到退出不会误删配置变更后新建监听的认领），从根上修掉「改 App ID 后 inbound（/here、切槽）绑死旧连接、需等在途请求结束或 daemon 重启才恢复」。**`/status` 与总开关独立**（开关只管切槽/发卡）；`/here` 在关态改回**动态引导**（不再静默忽略），`/help`（`/帮助`、`/?`）与未知 `/命令` 同样回**动态引导**——引导由 `help_text(auto, has_active_question)` 按开关裁剪可用命令/作答指引/切槽提示拼装，**不含「已收到」**（能回复即代表收到且在运行）。**作答确认（R2）**：内容**确实被接受进答案**时由各渠道会话**就地**发一条确认（图片/文件/文字 × 卡片/文本兜底，`autochannel::answer_ack_text`）；卡片模式下不被当答案的纯文字、未接受的兜底消息则回动态引导（R3）。**入站协调**：普通文本会同时广播给守护进程观察者并路由到在途卡片会话——该渠道**有活动在途提问时观察者退避**（`has_active_question_on`，交会话就地确认/引导，避免重复回复）；命令始终由观察者处理。**识别 ID 成功回执（R5）**：`/detect` 凭四位码识别到用户后，`send_detect_ack` 经该 IM 回「✅ 识别成功，已自动填入<字段>」（不回显完整 ID）。Telegram 自由文字既是答案又被观察者收到，斜线前缀文字仅当命令、不路由到在途卡片。**活跃槽统一含 "popup"**：在哪个渠道说话/作答就更新为哪个（弹窗作答 → "popup"，后续只弹窗），切槽时给旧 IM 发反激活提示（点明切到了哪个渠道）、把在途未答补推给新 IM、由调用方给新渠道发激活回执——逻辑统一在 `set_active_channel` 一处（返回 `(是否切换, 补推数)`；`winner_channel_id` 提供作答渠道）。

> IM `/watch` 实时关注（spec `docs/specs/im-watch.md`，**四渠道全支持：飞书 + Telegram + Slack + 钉钉**）：`/watch <编号>`（`/关注`）关注某 agent（编号同 `/status`）——机器人发一张「实时状态卡」，此后该 agent 的进度**只就地编辑这张卡**（纯静默、零新消息）：样式化头部（飞书/钉钉模板 icon、TG/Slack 🤖 + `实时关注 [n] 类型 — 项目`）+ 状态行四态（🟢 工作中 / ⚪ 空闲 / 🙋 正在等待你的回答[该 agent 有在途 AskHuman 提问时优先] / ⏹ 已结束）+ **整体运行时长**（`· 已运行 X`，agent 会话 `startedAt` 起算，非结束态显示——用户定案：回合时长迷惑）+ 标题 + 「最近动态（绝对时刻）」（复用 `/status` 的 transcript 尾部 × hook 实时工具融合，`autochannel::activity_parts`）+ 灰色「最后更新」行 + **动态按钮**（活动态 `[取消关注][立即刷新]`；飞书终态单个禁用按钮、TG/Slack 终态移除按钮 + 终态标签、钉钉 `finalized` 变量条件显隐）。**帧与渲染**：`watch.rs::WatchFrame` 结构化（steps/todos，无标记语言）+ 共享文案构件；飞书 `card_view`+`build_watch_card`、Telegram `telegram/watch.rs`（HTML + inline keyboard）、Slack `slack/watch.rs`（Block Kit）、钉钉 `dingtalk/watch.rs`（专用模板 `docs/assets/dingtalk-watch-card-template.json` 11 变量 + `build_watch_param_map`/`parse_watch_action`，彩色 `<font colorTokenV2>` 圆点、h5 字号、NBSP 须放 font 标签内、TODO 走 CollapsePanel；模板 ID 内置默认；实测 2s×150 次更新零频控 p50 ≈60–95ms，隐藏探针 `AskHuman debug dd-watch-poc` 保留于 `cli/debug_cmd.rs`，`--count 0` 样式预览），差异表见 spec。**引擎**（`daemon/mod.rs::watch_*`，传输经 `WatchClient` 枚举）：AgentEvent / 提问创建 / 答复完结即时 Notify + 自适应 tick（有工作中订阅 2s、只有空闲 10s、无订阅零空转）；每帧算**签名**（结构化内容，跨渠道一致），签名不变不编辑；每卡最短编辑间隔按渠道（飞书/TG/钉钉 1s、Slack 2s）；连续 5 次编辑失败自动退订；agent 结束 → 终态定格 + 自动退订。**按钮回调**按渠道路由（`WatchState::routes`）：飞书 RoutedFs ack 同步回新卡；TG `set_card_route`（仅卡回调、不抢自由文字）→ 应答 + 就地编辑；Slack `set_active(ts,"")` → 就地编辑；钉钉 RoutedDd `set_active(otid,"")`（DdRouter Reader 放行 watch actionId，空 ACK 后 OpenAPI 就地编辑）。**提问投放并集（M4 定案）**：autoActivation 开启时提问卡投放渠道 = 最后活跃渠道 ∪ 正在 watch 提问 agent 的渠道（`attach_im_channels` 按 `agent_session_id` 匹配订阅；收尾走既有抢答，不动机制）。`/watch` 无参列当前关注、`/unwatch <编号>|all|无参`（`/取消关注`）取消（旧卡定格 + 确认文本）；重复 watch 同一 agent＝换新卡（旧卡定格「已由新卡片接替」）；上限**每渠道** 5 个、订阅/扰动/跟底水位（`WatchState::disturb`）全按渠道；help 按渠道门控（`watch::channel_supported`）；Slack 客户端拦截 `/` 输入，命令用 `!watch` 等 `!` 前缀（见自动激活节）。**订阅持久化** `~/.askhuman/state/watch.json`（`watch.rs`，原子写，含 channel）：daemon 重启后恢复并**继续编辑同一张卡**（seq 按 session 重解析）；有活跃订阅时 daemon 不因空闲退出（订阅随 agent 结束消亡，有界）。
>
> IM 单选卡（`/watch`·`/status`·`/unwatch` 无参可点选，spec `docs/specs/im-select-card.md`，**四渠道**）：三命令**无参**时不再回纯文本编号列表（需再敲带编号命令），而是推一张**通用「单选卡」**列出可选 agent、**单击某项即触发**。抽象在 `select.rs`（传输无关、无标记语言：一张卡＝`标题 + 一组选项 + 一个动作`；选项＝`{稳定 id=session_id, 状态圆点, 展示编号, 主文本=类型·工作目录名, 徽标, 次行=标题}`，动作∈{Watch,Status,Unwatch} 决定每行按钮文案/样式）。飞书渲染/解析在 `feishu/card.rs`（**用户定稿「方案A」**：`build_select_card` 每选项一行 `column_set`＝左侧小字号两行富文本［markdown 彩色圆点`●`＋`**[编号]**`＋`类型·工作目录名`＋`· 关注中`徽标／灰色标题］＋右侧 `size:tiny` 触发按钮［watch=primary/status=default/unwatch=danger］，回调 `{select:<idx>}`；`parse_select_action` 读 `context.open_message_id`+下标；`build_select_final_card` 定格）。daemon（`SelectState{pickers,routes}` + `PickerEntry{channel,message_id,kind∈{Watch,Status,Unwatch},options=session_id 快照,created_at}`）：`send_agent_picker` 发卡+登记台账（TTL 30min + 每渠道软上限 10，**不持久化**）+ `ensure_select_routes`（复用 watch 路由句柄，飞书 `FsRouter` 认领各 picker 消息）；`handle_select_card_action` 解析→按 open_message_id 找 picker→按下标取 session_id→按 kind 分派。**点选结果**：`/watch`→就地把单选卡编辑成实时 watch 卡（经 oneshot 回卡；D8 已在关注中的 agent 带「· 关注中」徽标、点它＝换新卡，旧卡定格「已由新卡片接替」——`register_watch_at` 抽出的换新卡收尾与命令直达发卡共用）；`/status`→回纯文本详情（按 session 定位避免 seq 漂移）、单选卡不动可继续点；`/unwatch`→旧 watch 卡定格「已取消关注」+文本确认+就地刷新单选卡（移除该项，取到 0 则定格「已全部取消关注」）。**钉钉**（D4 模板固定、不能就地变身，也无「回调同步回卡」；已真机点选验证通过）：渲染/解析在 `dingtalk/select.rs`（模板 `docs/assets/dingtalk-select-card-template.json` = 用户后台发布件，`Card[标题 BaseText, Loop[选项 Markdown + 关注按钮 SingleButton + Divider], 定格 BaseText]`；按钮**单独成行**、其回传 param `sid` 绑到**循环项字段** `loop_object_list[i].sid`；ID `DEFAULT_SELECT_CARD_TEMPLATE_ID`；`build_select_param_map` 出 6 变量 `title/btn_text/btn_color/finalized/final_label/loop_object_list`(复杂值→JSON 字符串)，`option_md` 用 `<font sizeToken/colorTokenV2>` 两行**同 footnote 字号**富文本，`parse_select_action` 读 `actionId=select`+`params.sid`）；daemon `handle_select_dd_action`（recv-loop 先空 ACK 满 3s）→ `/watch` **另发新 watch 卡**+`register_watch_at` 换新卡收尾+单选卡 OpenAPI 定格「已选择 [n]」；`/unwatch` 旧卡定格+文本+OpenAPI 刷新 loop（取 0 定格）；`/status` 回文本详情。**关键坑（已修）**：`dingtalk/router.rs` reader 原只放行「提问卡提交 / watch 按钮」两类回调、其余直接空 ACK 丢弃——须再放行 `select::parse_select_action` 命中的 select 回调，否则点选静默无效。**边界**：仅无参弹卡（`/watch 3`、`/status 3`、`/unwatch all`/`<编号>` 仍直达；`/unwatch` 无参需 ≥2 关注才弹卡）；无 agent / 不支持渠道(TG·Slack)回既有文本兜底；**过期＝静默无效**（daemon 重启后旧卡点击回空 ACK、卡片无变化，D7）。**递归规避**：卡回调 recv-loop 内不调 `ensure_select_routes`（会与 spawn 形成 `!Send` 递归）——watch 认领靠 `register_watch_at` 的 `notify`→watch 引擎 `ensure_watch_routes`，残留 select 认领无害、下次发卡/监听重建时收敛。**Telegram / Slack**（与飞书同属**可就地变身**，非另发卡；full build + 单测已过，尚未 install/真机验收）：渲染/解析在 `telegram/select.rs`（HTML 正文＝标题＋每选项两行［🟢/⚪＋`<b>[编号]</b>`＋类型·目录＋徽标／斜体标题］＋每选项一枚 inline 按钮「<动作> [编号]」独占一行，`callback_data=sel:<idx>`；`render_select_final_html` 定格）与 `slack/select.rs`（Block Kit＝标题 section＋每选项一个 section［两行 mrkdwn］＋右侧 button accessory，`action_id=select_<idx>` 保证整卡唯一、watch=primary/unwatch=danger；`build_select_final_blocks` 定格）；daemon `handle_select_tg_action`（应答 callback 消转圈）/ `handle_select_slack_action`（ws 层已 ack）→共用 `dispatch_select_pick`（下标→session_id→按 kind 分派）；`/watch`→`select_pick_watch_inplace`（`WatchClient::edit` 就地把本消息变实时 watch 卡，含结束态定格 Ended、上限校验、换新卡收尾）、`/status`→回文本详情卡不动、`/unwatch`→旧卡定格 Cancelled＋文本＋`refresh_select_card_edit` 就地刷新（取 0 走 `finalize_select_card_edit` 定格）。路由复用 watch 的 `TgRouter`/`SlRouter`（TG `set_card_route` 只认领卡回调不抢提问卡自由文字；Slack `set_active(mid,"")` 只认领卡交互）。
>
> 回复历史：`general.historyLimit`（默认 200，0=停止新增并清理已有记录）控制 `~/.askhuman/history.jsonl` 全局保留条数（裁剪与「立即清理」对 0 不再特判：`record` 在 limit==0 时不新增、但按与 >0 相同时机把已有记录裁到 0；`trim(0)` 直接清空）。每次提问在 `Coordinator.finish()`（所有渠道/模式唯一汇聚点）旁路记录一条「发送 / 用户主动取消」（系统取消不记）；只存图片/文件路径（best-effort 展示，缺失显示占位）。项目按「从 CLI cwd 向上找首个 .git 根、回退 cwd」识别，经 `TaskRequest`/`ShowPayload` 贯穿 daemon（revisit A11）。每条另存**来源 agent 家族** `HistoryEntry.agentKind`（`skip_serializing_if` 缺省兼容旧行）：初值取 CLI env 探测的 `task.agent_kind`，daemon `spawn_agent_resolve` 异步 walk 出家族后经 `Coordinator::set_agent_kind` **回填**同一槽位（MCP `env_clear` 场景只有这条路拿得到），`finish()` 落盘取最新值；旧记录无该字段时前端按 `source` 命中已知家族名回退识别（`lib/history.ts::agentKindOf`）。历史窗口 `AskHuman --history [--all]` 或弹窗导航栏「历史」按钮打开。详情只读组件 `HistoryDetail.vue` 完整还原：选项框复用 controls.css（选中=蓝底白 ✓）、附件区与弹窗同款交互（单击选中 / 空格 QuickLook 预览 + 方向键切换 / 双击打开 / 右键菜单 / 拖出）。历史窗口创建时 `watch_history_file` 用 notify 监听 `history.jsonl`，任何进程写入后发 `history-updated` 事件令前端重载并保留当前选中条目（跨进程实时刷新）。注：`preview_attachments` 命令把 QuickLook 控制者插入**调用方窗口**响应链（弹窗或历史窗口皆可），不再硬编码 popup。

> 密钥安全：五项密钥（`dingding.clientSecret`/`feishu.appSecret`/`telegram.botToken`/`slack.botToken`/`slack.appToken`）默认迁入系统钥匙串，`config.json` 中留空；内存 `AppConfig` 始终携带解析后的真值，读取点零改动。文件权限收紧 0600/目录 0700；钥匙串不可用时回退明文。macOS 需稳定签名身份免弹框（本地 `install.sh` 优先固定 `Developer ID Application` / 发布走 Developer ID）。**钥匙串还依赖「安全会话」上下文**：只有用户 GUI(Aqua) 会话能静默读登录钥匙串；从非 Aqua 会话（SSH、Codex app-server 等后台上下文）直接拉起的 daemon 会继承该非 Aqua 会话、**运行时取到空密钥** → 渠道被判「未配置」不建长连接（现象：飞书 `/status` 无回复、`daemon status` 显 `im conns none`）。故 `daemon/spawn.rs` 在 macOS 侦测 `launchctl managername != "Aqua"` 时改经 `gui/<uid>` launchd 域 bootstrap `daemon run`（透传 HOME/TMPDIR/PATH/`ASKHUMAN_*` 保住 perf/隔离语义），使 daemon 落在 Aqua 会话、能读钥匙串；GUI 会话或纯 headless 回退原 setsid。详见 `docs/specs/secret-storage-keychain.md`。
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

- **开关入口**：设置「通用」底部隐蔽开关「实验性功能」(`config.experimental.enabled`，默认关、Windows 不显示) → 出现「实验」Tab，内含 Claude/Codex/Cursor/Grok 四家「生命周期追踪」开关；开/关＝安装/卸载**用户级** lifecycle hook（`integrations/agent_lifecycle.rs`，开关真值实时查 `agent_lifecycle_status`，与既有 timeout hook 互不影响）。
- **事件采集**：hook 命令统一为 `AskHuman __agent-hook <agent> <event>`（`agents/report.rs`）；事件 session-start/turn-start/turn-end/session-end + **activity**（回合内 Pre/PostToolUse 工具心跳，四家都装）。Claude/Grok 另加 **StopFailure→turn-end**（API 错误结束回合时官方文档明确 Stop 不触发，靠它补结束）；Codex 无 session-end / 无 StopFailure。reporter 按 env 判真实运行家族做**去重**：Cursor 兼容加载 `~/.claude` 致每事件双触发，env 有 `CURSOR_*`→只认 cursor、跳过 claude 那次；**Grok 兼容加载 `~/.claude`/`~/.cursor` hook**，`running==Grok` 时凡 `intended!=Grok` 一律跳过（只认 grok 原生 hook）。并 walk 进程树定位真实 agent pid、解析 session_id（env 专用变量优先，回退 stdin JSON）。
- **状态推导**：daemon 内 `AgentRegistry`（`agents/registry.rs`）以 **session_id 为身份**、pid 仅判存活。turn-start/activity 切「工作中」、turn-end 切「空闲」；**进程存活轮询（15s）是权威的「已结束」判据**（关窗/`kill -9` 时事件全丢，靠它）；1h TTL 兜底（任何 hook 事件或 `AskHuman` 提问会刷新**对应 session** 的活动时间）。ended 最多留 10 条。状态持久化 `~/.askhuman/agents.json`，daemon 换新/重启后重载并 kill-0 复核。
- **Codex 共享 app-server → pid 归一为 None（spec D25–D27）**：新版 Codex TUI 经 UDS 连一个**长寿共享 app-server 守护**跑 agent，hook/工具/MCP 都在守护进程树里 → walk **只会命中守护、拿不到 TUI pid**（守护多会话共用、reparent 到 PID 1；源码+实测坐实见 spec §8）。`agents/detect.rs::is_shared_app_server` 识别命中的 Codex 祖先是否为 `codex app-server …`，是则 `walk_agent_pid`/`walk_any_agent` 记 **pid=None**，让该会话落到「无 pid」路径——**与 Claude 被 PID-scrub 时同一条路径**：不做 D7 pid 轮换、不做存活轮询，纯由 TTL + 「工作中」兜底超时治理。消除了共享 pid 导致的并发会话互相轮换误杀、幽灵会话、`touch_activity_by_pid` 跨 session 串味。代价：Codex 会话「聚焦终端」按钮因无 tty 恒隐藏；打断/关窗结束有延迟（同 Claude）。
- **「工作中」兜底超时（30min）**：Claude **用户打断回合**时没有任何 hook 触发（官方确认 Stop/Notification 均不来）、**Codex 打断同样无任何 hook**（源码坐实：`Stop` 仅回合自然完成触发，Esc 走 `TurnAborted` 在 Stop 前返回；无 Interrupt/SessionEnd 事件），transcript 也只在完成时整条原子写入、不能当心跳——故无法即时检测打断。兜底：daemon 每 15s tick 跑 `working_backstop_sweep(WORKING_BACKSTOP_SECS=30min)`（注册表全量、与家族/pid 无关），把「工作中且距上次活动超 30min」的记录降级为空闲（活动时间由 turn-start/activity[Pre/PostToolUse]/提问刷新，故执行长编译等工具的合法长回合不会被误判）。**在途 AskHuman 豁免**：tick 先刷新「正等待人类回答」的 agent 活动，使其期间永不被降级——`refresh_by_pids`（`in_flight_agent_pids`，覆盖有 pid 的）**并列** `refresh_by_session_ids`（`in_flight_agent_session_ids`，覆盖无 pid 的 Codex 共享 app-server / Claude 被 scrub）。打断后走开 → 30min 内自愈；下次提问/关窗也会即时纠正。
- **hook 升级迁移**：版本升级新增事件后，已开启的用户**无需手动关开开关**——daemon 启动时 `agent_lifecycle::migrate_outdated()` 对**已安装但过期**的家族幂等重装（仅改含 `__agent-hook` 标记的条目）；「实验」Tab 每家额外提供一键『更新』按钮（outdated 时显露，仿 rules/hook 卡）。
- **手动置空闲**：状态窗口每个「工作中」卡片有一个「置为空闲」按钮（行内二次确认），点确认后经 `agent_force_idle` 命令发 `ClientMsg::AgentForceIdle{session_id}` → daemon `registry.force_idle`（仅改状态、刷活动时间、不结束会话）→ 持久化 + 推回新快照。用于兜底超时之外的即时人工纠正（如确知某 agent 实际已空闲却仍卡「工作中」）。
- **闲退守卫**：仅「工作中」agent 或有状态窗口连接才阻止 daemon 闲时退出（空闲 agent 不算）；**graceful drain（版本换新）不受存活 agent 影响**。
- **状态窗口**：`AskHuman agents monitor`（原 `agents status` 改名，spec `cli-config.md` D8）→ 有 GUI 时**路由到统一 GUI 宿主**（`gui_host::host_open(Agents)`，全局单窗；兜底 `app::run_agents` + `create_agents_window`），订阅 `ServerMsg::AgentsState` 推送、前端 `AgentsView.vue` 跨项目按类型分组、状态优先排序、相对时间动态刷新；headless 或 `--json`/`--text` → 取一次 `AgentsState` 快照（`client::request_agents_snapshot`）渲染文本/JSON（`agents_cmd.rs`）。**未开启生命周期追踪时入口收敛**：托盘隐藏「Agent 状态」项（见「菜单栏图标」节），命令 `agents monitor` 仍可运行（保留直接打开场景），但窗口空状态提示「只有开启生命周期追踪的 Agent 启动后才会在此显示」。CLI help 不做条件隐藏，靠此空状态兜底。**「聚焦终端」（macOS）**：状态窗口每行（有 pid、非 ended、**且所在终端被支持**）一个图标按钮 → `focus_agent_terminal(pid)` 命令（`integrations/terminal_focus.rs`）由存活 agent pid 取控制终端 tty（`ps -o tty=`）+ 终端类型，按类型分派 AppleScript 精确匹配 `tty`：**Terminal.app**（`tab` 的 `tty`，选中标签页 + 窗口置前）/ **iTerm2**（`session` 的 `tty`，用 bundle id `com.googlecode.iterm2` 定位，选中 session/tab/window），命中后激活该 App（首次需「自动化」TCC 授权）；不支持的终端 / 无 tty / 未授权 / 找不到一律静默（前端 console.warn）。**按钮显隐按终端类型**：daemon 快照对每个活动记录惰性识别终端（`agents::detect::terminal_kind`：由 pid 沿进程链匹配 `apple-terminal`/`iterm2`/`vscode`/`cursor`/`tmux`/… 缓存进 `AgentRecord.terminal`），前端 `SUPPORTED_TERMINALS` 集合（当前 `apple-terminal` + `iterm2`，见 `lib/terminals.ts`）决定是否展示按钮；加新终端支持时同时补「前端集合 + 后端聚焦实现」。kitty/WezTerm/tmux/编辑器内置终端等留待后续。
  - **订阅生命周期**：仅由前端在 `agents-updated` 监听就绪后经 `agents_start_subscription` 命令触发（不在开窗时启动，否则 daemon 首帧立即快照会早于监听而丢失）。长命的 GUI 宿主里订阅与窗口绑定——`start_agents_subscription` 检测到 `HostState` 即走 `gui_host::restart_agents_subscription`：每次挂载都**重启**订阅（daemon 重推一帧立即快照，避免复用旧订阅导致首屏长 Loading），窗口关闭（`recount_windows` 发现无 `agents` 窗口）即 `stop_agents_subscription` 停掉（释放 daemon 连接，不再借订阅给 daemon 续命）。独立 agents 进程 / 弹窗兜底则一次性启动、随进程退出。
- **IPC 增量**：`TaskRequest` 加 `agent_kind/agent_session_id/agent_pid`（提问顺带刷新活动）；`ClientMsg::AgentEvent`/`AgentsSubscribe`；`ServerMsg::AgentsState`。

## 实验性功能：Agent 插话（Interject，Unix）

> 需求 `docs/specs/agent-interject.md`（D1–D9）+ 计划 `docs/plans/agent-interject.md`。在 agent 工作中**主动**给它捎话（不等它 AskHuman 提问）：消息进 daemon 队列，agent **下一次工具调用**（PreToolUse hook）时送达。依赖生命周期追踪开关（同一个 hook 承载）。**Grok 首期排除**（PreToolUse 无法可靠向模型传话，D1）。

- **消息模型（D2）**：daemon 内存 per-session 队列（`agents/interject.rs::InterjectStore`）。弹窗提交＝**整体覆盖**（打开时预填现有全文可编辑）；IM `/msg`＝**追加**（IM 看不到旧文本，覆盖会静默丢内容）；送达时全部条目按空行拼一条消息、随后清空。留队直到消费或会话结束（ended 自动清空 + 周期 `retain_sessions` 兜底）。变更时原子落盘 `~/.askhuman/state/interject.json`（daemon 重启恢复；composer 打开态是连接态不持久化）。
- **hook 三态协议（D3/D4，性能红线：热路径零文件 IO）**：`__agent-hook` 的 PreToolUse 事件（stdin 判定 pre 阶段、Grok 除外）在既有 `AgentEvent` 上捎带 `interject_poll=true`，等 daemon 回一帧 `InterjectDecision`（300ms 超时 + daemon 不可达一律**放行**，不影响正常工具调用）：`none`→放行；`message`→stdout 输出该家 deny JSON（Claude/Codex `hookSpecificOutput.permissionDecision=deny` + reason；Cursor `permission=deny` + `user_message`/`agent_message` **双字段同放完整协议文本**——live 实测喂回模型的是 `user_message`，与官方文档相反，勘误见 spec D3）；`hold`（composer 窗口打开中）→挂起等第二帧 `message`/`release`（**混合等待**：用户已开窗表示要说话，hook 长等；取消/关窗即放行）。deny 文案 `prompts::interject_deny_reason`：`[USER INTERJECTION]` 前缀 + 说明「拒绝仅为送达用户消息、非工具被禁，仍合适可重发同调用」+ `<user_message>` XML 块 + 「不清楚就 ask the user as instructed」。
- **安装/迁移**：PreToolUse hook 带 `timeout: 86400`（等待人类量级；Claude/Cursor hook JSON 字段、Codex `notify` 无超时概念但 trust hash 纳入 timeout 参数）；`agent_lifecycle::needs_update`/`migrate_outdated` 识别旧安装（无长超时）→ daemon 启动时自动重装，「实验」Tab 有手动『更新』兜底。
- **GUI（D7）**：AgentsView 每卡「发送消息」按钮（grok/ended 无）→ `open_interject` 经宿主路由 `WindowKind::Interject`（每 session 全局唯一窗，`interject_label` 哈希 label）→ `InterjectView.vue` composer（预填全文、提交覆盖/取消）。窗口持一条 daemon 专属连接（`client/composer.rs`）：打开即 `InterjectComposer` 登记（该 session 的 hook 开始 Hold）、查询预填；**连接断开＝composer 关闭**（放行等待 hook）；提交走同连接保序，死连接退一次性连接。AgentsState 快照注入 `pendingInterject` → 「插话」徽标 + 行内确认撤回（`InterjectClear`）。
- **托盘（D7）**：`TrayState.agents`（`TrayAgentInfo` 摘要：seq/kind/标题/项目/状态/待送达/可聚焦；旧 daemon 缺字段→空）驱动「Agent 状态」父项变**子菜单**：首项「打开状态窗口」+ 分隔线 + 每 agent 子菜单（条目文案与 `/status` 列表行同构 `[编号] 类型 — 标题（项目）`、标题截断 24 字符，编号可直接用于 `/msg <编号>`；子项「发送消息…[有待送达时标注]」grok 无 +「聚焦终端」focusable 时；两者皆无列只读行）；宿主本进程直接开 composer / 后台线程聚焦终端；摘要入 `menu_signature` 走最小 diff。
- **IM `/msg`（D9）**：`/msg <编号> <内容>`（`/插话`）＝追加（回执排队条数；恰有 hook 挂起等待则**立即送达**）；`/msg <编号>`＝回显待送达全文；`/msg-clear <编号>`（`/撤回`）＝清空。编号复用 `/status` 稳定 seq；与 `/status` 同门控（daemon 存活即可用、进 `/help`）；grok 回「不支持插话」、ended 回「会话已结束」；内容保留原始换行。
- **IPC 增量**：`ClientMsg::AgentEvent.interject_poll` / `InterjectComposer` / `InterjectSubmit` / `InterjectClear` / `InterjectQuery`；`ServerMsg::InterjectDecision{action,text}` / `InterjectState{text,entries}`；`TrayState.agents`。

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
- **托盘菜单（spec D7）**：状态区（运行/停止、版本、运行时长、IM 连接、更新可用/待生效，均 disabled）+ 操作区（设置/历史/Agent、检查更新、应用更新[复用 self-update，drain 生效不打断]、启动/重启/停止 daemon）。**agent 忙闲已并入操作区「Agent 状态」入口标题**（开启生命周期且有 agent 时显示「Agent 状态（工作 {w} · 空闲 {i}）」，否则纯「Agent 状态」）——不再单列只读忙闲行；有活动 agent（`TrayState.agents` 摘要）时该入口变**子菜单**（打开状态窗口 + 每 agent 的「发送消息/聚焦终端」，见「Agent 插话」节），无活动 agent / 旧 daemon 退回普通条目（点击开窗口）。**「待答」改为子菜单**：`TrayState.pending_requests`（每条 `{id, preview}`，预览取 Message 首个非空行/第一题题干、截断 24 字符，daemon 按创建 `seq` 稳定排序）逐条列在「{n} 个待答」子菜单下；点击某条 → 宿主开一条连接给 daemon 发 `ClientMsg::FocusRequest{request_id}` → daemon 经该请求的弹窗连接转发 `ServerMsg::FocusPopup` → 弹窗进程 `set_focus()` 并 emit `popup-flash`（`PopupView.vue` 播放边框 accent 蓝脉冲 2 次）。无弹窗连接（拉起失败/IM-only）的请求仍列出、点击静默无效。旧 daemon 缺 `pending_requests` → 退回只读计数行。**「Agent 状态」入口（含标题内的忙闲计数）仅在开启了生命周期追踪时显示**（任一家装了 lifecycle hook，`agent_lifecycle::any_installed()`，纳入 `menu_signature`）；未开启即隐藏（否则窗口必为空，徒增困惑）。生命周期 hook 装/卸后即时刷新：`agent_lifecycle_install`/`uninstall` 命令成功即主线程 `refresh_tray`（设置窗与宿主同进程；非宿主进程无 `HostState`、自动 no-op）。**菜单稳定性（diff 更新）**：菜单抽象成声明式 `Node` 列表（每个节点带稳定 `key`，可点条目 `key` 即事件 id），由 `build_specs()` 产出期望列表、`TrayMenu::apply`（`app/tray_menu.rs`）与影子树做 **diff**：文字 / 可用性变化只 `set_text`/`set_enabled`（**不动结构 → 不关已展开菜单**），结构变化才按 `key` 做**最小** `insert`/`remove`（子菜单子项递归 diff），绝不整段清空重填、更不 `tray.set_menu` 换对象。diff 算法（`reconcile`/`build_live`/`update_live`）对底层菜单实现**泛型**（`MenuOps` trait）：生产用 `TauriBackend` 包 Tauri 菜单对象，单测用 mock 后端记录每步操作 → 既验证 diff 结果、又验证操作**最小性**（`tray_menu.rs` 末尾 14 个用例：初次全插、相同零操作、仅文字/可用性变化、中间增删、连续删、尾删、同槽换 key、子菜单子项增删/改标题、整菜单 uptime 跳动只 1 次 `set_text` 等）。外层再以「渲染内容签名」(`menu_signature`：语言+在线态+`build_specs`/图标的每个输入；uptime 取分钟级文案) 比对，**内容不变就整次跳过**（连 diff/图标都不做）。背景：早先「整段 `remove_at`+`append` 重填」在 macOS 上仍会关掉已展开菜单，且 `fmt_uptime`/忙闲计数等让签名每 15s/每分钟变一次 → 定时触发重建关菜单（实测见 git 历史）；改 diff 后周期性/装饰性变化（uptime、计数）就地改文字、菜单纹丝不动。残留：真·结构变化（待答出现/消失、IM 接入、daemon 起停等）时 macOS 仍会关一次已展开菜单，属系统限制、且少见。语言/状态变化即 diff 重排（宿主监听 `config.json` 实时切换）。
- **宿主二进制换新（spec D11）**：宿主长命，故周期（15s）比对自身盘上二进制指纹，变化且无窗口时释放锁后自我 `spawn_detached` re-exec（macOS always 模式交 launchd KeepAlive 重启）；新实例捕获新指纹，不会循环。
- **守护进程生命周期 `general.daemonLifecycle`**（`config.rs::DaemonLifecycleMode`，默认 `activity`；实验 Tab 分段控件，Unix only）：`activity`＝现行行为（按需拉起 + 空闲退出）；`keepalive`＝**保活**——① daemon 空闲循环每轮 `load_without_secrets` 读一次，为 keepalive 时**跳过空闲退出**（让 IM 随时可收）；② 装 **daemon 登录项**开机自启（`integrations/login_item.rs::sync_daemon`，与 guihost 登录项**分开**：macOS `~/Library/LaunchAgents/*.daemon.plist` 带 `RunAtLoad`、**不带 `KeepAlive`**，Linux `askhuman-daemon.desktop` 跑 `daemon start`；**纯文件写/删、绝不 `launchctl bootout`**——关保活时只删文件、让在跑的 daemon 按原空闲策略自然退出，不强杀）；③ 登录项同步由 daemon 自身在 `serve()` 启动 + `on_config_changed` 幂等执行（`sync_daemon_login_item`）。**打开开关即视为一次触发立即起**：GUI 宿主 `apply_config` 跟踪 `HostState.daemon_lifecycle`，检测到切入 keepalive 即 `client::ensure_running` 拉起 daemon（否则开了看不到效果）。注意保活 daemon 登录项刻意**不带** `KeepAlive`（避免与「自然退出/换挡」打架），故它只保证下次登录自启、当前会话靠宿主 `ensure_running`。

## CLI 配置与 Agent 集成（headless / 无 GUI）

> 需求 `docs/specs/cli-config.md` + 计划 `docs/plans/cli-config.md`。让 Linux 服务器 / 容器 / SSH 等无 GUI 环境**纯命令行**完成全部渠道配置与 agent 集成，且**可脚本化一次性执行**。四个顶层子命令组（`channel`/`agents`/`config` + `doctor`），每组及子命令都有 `help`，所有输出经 `cli/cfgio.rs::t` 中英双语本地化。

- **复用而非重写**：渠道连通性 `test` / userId·openId 自动识别 `detect` 直接调 `commands.rs` 既有 `*_test`/`*_detect_prepare`/`*_detect_wait`（参数结构体字段已 `pub`，密钥经 `fallback_secret` 回退已存值）；配置读写走 `config.rs::{load,load_without_secrets,save}`（save 自动把密钥写钥匙串、文件 0600）；集成走 `integrations::{agent_rules,cursor_hook,claude_hook,agent_lifecycle,mcp_config,agent_mode}`；agent 状态走 daemon `AgentsSubscribe`/`AgentsState`。落盘后 daemon `config_watch` 自动热重载。
- **`channel`**（`channel_cmd.rs`，name ∈ telegram|dingding|feishu|slack）：`list [--json]`（启用/配置齐全/已连接；daemon 未运行时连接态文本显 `—`、JSON 为 `null`）；`set <name>` 二合一——**终端且无 flag → 交互向导**（逐项提示、密钥隐藏输入、留空保留）、**带 flag → 非交互脚本**（`--enable/--disable` + 各非密钥字段 kebab flag）；`enable|disable`；`test`；`detect [--save]`（prepare 取识别码 → 提示发给 bot → wait 经 daemon 单连接捕获 → 可保存；telegram 无 detect）。
- **密钥输入（D4，脚本安全）**：仅 `--<field>-env <VAR>` / `--<field>-file <path>` / `--<field>-stdin`（或值 `-`）；交互时隐藏输入（Unix termios 关 echo）；**密钥明文不进 argv**（避免泄漏 shell 历史 / `ps`）。
- **`agents`**（`agents_cmd.rs`，agent ∈ cursor|claude|codex|grok）：`monitor [--json|--text]`（见上节）；`mode <agent> [none|cli|mcp]`（省略模式则查询当前模式 + 是否需更新，带模式则一键切换、复用 `agent_mode::set` 自动卸旧装新；**grok 仅 none|mcp，请求 cli 报错**）；`show [<agent>]`（打印手动集成参考提示词——单查 grok 时打印 `grok_skill_body`、否则 `cli_reference()`——+ 各 agent 粘贴位置 + 当前模式/rules|skill/hook/mcp/lifecycle 安装状态）；`install/uninstall/update <agent>` **必须显式** `--rules`/`--hook`/`--mcp`/`--lifecycle`（无默认捆绑，D6；`--rules` 对 grok = 安装 skill；`--hook` 仅 cursor/claude，codex/grok 跳过；`--mcp` 写 MCP server 配置；`--lifecycle` 实验性；lifecycle 无独立 update→重装即刷新）。
- **`config`**（`config_cmd.rs`，兜底）：`show [--json]`（密钥脱敏 `●●●`）/`get`/`set`/`unset`/`path`，点号 camelCase 键。`set` 非密钥键按目标 JSON 类型强制（bool/数字/字符串/枚举）→ 反序列化校验 → save；**密钥键**（5 个，`cfgio::SECRET_KEYS`，与 `secrets::ACCOUNT_*` 一致）自动路由进钥匙串，值仍只从 env/file/stdin 取。`unset` 重置默认（密钥 → `secrets::delete`）。
- **`doctor [--json]`**（`doctor.rs`）：一屏体检 daemon（运行/版本/在途/IM 连接）+ 各渠道（启用·齐全·连接）+ 各 agent 集成（当前 mode + rules·hook·mcp·lifecycle 装没装/需更新）。

## MCP 支持（CLI 模式之外的第二种集成形态）

> 需求 `docs/specs/mcp.md` + 计划 `docs/plans/mcp.md`。动机：Codex 等 agent 无法为 CLI 工具调用延长超时（命中即可能被取消），而 MCP 协议允许配置较长的 `tool_timeout_sec`，让长等待可靠。

- **形态**：`AskHuman mcp` 以 **STDIO** 跑一个 MCP server（rmcp SDK），对外暴露**单一工具 `ask`**（配置中 server 名 `askhuman`）。
- **薄壳复用**：MCP server 不自己实现 ask 逻辑——每次 `ask` 调用就 `spawn` 一个现有 `AskHuman … --output json` 子进程，**复用全部既有 ask 流程**（弹窗 / IM / 抢答 / 历史 / 落盘 / 排空与自动重连）。子进程带 `ASKHUMAN_FROM_MCP=1`，CLI 据此在 `TaskRequest.from_mcp` 标记来源；daemon 对 MCP 来源的会话活动**仅刷新（touch_activity）而非新建工作会话**，避免长寿 MCP server 携带过期 session_id 造成「幽灵工作会话」。全平台同一套；daemon 换新/重启后下次调用自然重连。
- **MCP 模式下的生命周期识别（env 清空 → 退用进程树 pid）**：agent 启动 STDIO MCP server 时会 `env_clear()`（实测 Codex：`rmcp-client` 仅注入 `HOME/PATH/...` 约 10 个系统变量），故 ask 子进程**看不到任何 `CODEX_*`/`CURSOR_*`/`CLAUDE*` 变量**——既判不出家族、也**拿不到会话 ID**（`CODEX_THREAD_ID` 本就只注入 codex 的 shell 工具子进程，连 codex 自身进程 env 都没有，配置 `env` 转发也无济于事）。兜底：`detect::walk_any_agent_from_self()` 向上 walk 进程树定位最近的 agent 祖先 → 拿到 `(kind, pid)`（无 session_id，pid 是当次现取、真实存活）；daemon `handle_submit` 据此走 `AgentRegistry::touch_activity_by_pid(kind, pid)`：按 `(kind,pid)` 匹配**已存在**的 session 刷新 `last_activity`，**只更新、绝不新建**（pid 真实存活 → 无幽灵会话）。命中前提是该会话已被 lifecycle hook 追踪（hook 的 turn 事件把同一 codex pid 写进 registry）。三家通用（仅 `from_mcp` 启用兜底，零影响普通 CLI 调用）。
- **入参（精简）**：`ask` 仅暴露 `message`（**恒按 Markdown 渲染**）/`questions[{question, options[{text, recommended}]}]`/`files[]`；不暴露 `markdown`（恒 on）、`single`、`selectOnly`（属脚本/纯文本场景）。
- **输出**：`ask` 声明 input/output schema；子进程 JSON 解析为 `AskResult`（**剔除脚本专用的 `selected_indices`**）→ 返回 `structuredContent`（结构化 JSON）+ `content`（序列化 JSON 文本 + 人类回复中的图片读回转 `ImageContent`）。**取消时顶层带 `status` 引导文案**（要求模型重新确认直到用户明确答复，不得当作放行）；该字段由 CLI `--output json` 顶层产出（取消路径才有），薄壳原样透传，脚本侧亦受益。
- **三态集成**：每家 agent 的「自动集成」为 **None / Cli / Mcp 互斥三态**（`integrations/agent_mode.rs`）。Cli 绑定 Rule(CLI 版)+超时 Hook；Mcp 绑定 Rule(MCP 版)+MCP 配置（`integrations/mcp_config.rs` 写用户级全局：Cursor `~/.cursor/mcp.json`、Claude `~/.claude.json`、Codex `~/.codex/config.toml`，最小编辑保留用户内容）。提示词分 `prompts::{cli_reference,mcp_reference}` 两版。lifecycle hook（turn 追踪）与三态正交，独立开关。**Grok 例外：仅 None|Mcp 两态**（Composer 的 CLI 会自动后台化、超时不可靠 → 无 Cli 档、无超时 Hook；`set(Grok, Cli)` 直接报错），其 Mcp 产物 = skill（`grok_skill`，非 rules 文件）+ `~/.grok/config.toml`。
- **MCP 工具超时**（长等待不被取消，各家机制不同）：
  - **Codex**：写 `tool_timeout_sec=86400`(秒) + `startup_timeout_sec=30`。✓
  - **Grok**：写 `tool_timeout_sec=86400` + `startup_timeout_sec=30` + **`tool_timeouts = { ask = 86400 }`**（per-tool，对默认模型 Composer 更精准）。✓ live 验证 `grok inspect` 确认 `askhuman(stdio) config` 已加载。
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

- `scripts/install.sh` 在 macOS 自动使用本机 codesigning identity 和固定 identifier 正式签名；
  身份优先选 **`Developer ID Application`**（确定性挑选，不再取 `find-identity` 首个）：机器上同时有
  `Developer ID Application` 与 `Apple Development` 时，若每次装取到的身份漂移，会翻转二进制的
  designated requirement，令钥匙串 ACL 不再信任它、静默取密钥失效（尤其后台 daemon）；Developer ID 的 DR
  又与 cdhash 无关且不过期，固定它可让 ACL 跨重装保持有效。`CODESIGN_IDENTITY` 可覆盖；两者皆无则回退首个
  可用证书、再退 ad-hoc。后台 Codex app-server 无法访问签名私钥时，通过一次性 LaunchAgent 同步委托用户 GUI
  launchd domain 完成（不打开 Terminal），失败则中止安装，避免留下无法执行或破坏钥匙串信任的产物。

```bash
pnpm install
pnpm tauri dev                                   # 调试（Vite + Tauri）
pnpm build && cargo build --release --manifest-path src-tauri/Cargo.toml   # release（前端资源在 cargo 编译时嵌入二进制）
cargo test --manifest-path src-tauri/Cargo.toml  # Rust 单测
./scripts/install.sh                              # 安装到 ~/.local/bin（mac/Linux）
node scripts/perf-popup.mjs --runs 20            # 弹窗启动延迟 harness（见下「性能埋点」）
```

### 性能埋点（弹窗启动延迟）

- 环境变量 `ASKHUMAN_PERF=1` 开启（默认关、零开销）。CLI 铸 `perf_id` 经 `TaskRequest` 透传，daemon spawn helper 时再以 env 传给 helper/前端；CLI/daemon/helper/前端共 16 个里程碑统一写 `~/.askhuman/perf.log`（`<epoch_ms>\t<perf_id>\t<stage>\t<pid>`），按 `perf_id` 串成一条时间线。实现：`src-tauri/src/perf.rs` + 前端 `src/lib/perf.ts`（命令 `perf_mark`）。
- harness `scripts/perf-popup.mjs`：零交互（弹窗画完首帧 `ASKHUMAN_PERF_AUTODISMISS=1` 自动取消）跑 N 次、聚合中位/p90、存/比基线，端到端 p90 超阈（默认 20%）退出码 1。方法论与基线见 `docs/specs/popup-launch-performance.md` §7。

## 注意事项

- **stdout 洁净**：GUI 阶段把 stderr 重定向到 /dev/null（`app/mod.rs` 的 `stderr_redirect`，Unix），自身错误用 `eprintln_real` 走原 stderr。
- **首帧不白闪**：`src/index.html` 内联关键底色；macOS 毛玻璃下 body 透明叠色罩。
- **macOS 透明/毛玻璃**依赖 `tauri` 的 `macos-private-api` feature 与 `macOSPrivateApi: true`。
- **release 自包含**：前端资源在 `cargo build` 时由 `generate_context!` 嵌入，故安装后无需 dev server。
- Telegram 不接收图片；Cursor Hook 仅 mac/Linux（Windows 禁用并提示）。
