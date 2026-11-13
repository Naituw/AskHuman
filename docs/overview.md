# AskHuman 项目概览（供 agent 参考）

> 跨平台「Human-in-the-loop」工具：命令行 `AskHuman` 在需要人类确认/补充时弹出窗口收集回应，并把结果按固定区块格式写到 stdout 供 AI 读取。

## 文档边界

- 主 overview 是每个任务必读的架构、模块地图、跨模块不变量和能力边界。
- 只对局部任务有用的当前实现地图放 `docs/overview-<topic>.md`；需求决策放 `docs/specs/`，实现步骤放 `docs/plans/`。
- 任务进度只放 `docs/PROGRESS.md`，不在 overview 记录 Phase 或验收历史；代码变化只有影响全局心智模型时才更新主 overview。

## 技术栈与形态

- **Tauri 2**：Rust 后端 + WebView 前端，单一可执行文件 `AskHuman`，跨 macOS / Windows / Linux。
- **前端**：Vue 3 + Vite + TypeScript，纯手写 macOS 风 CSS（无组件库）。
- **运行模型**：macOS、Linux 与 Windows 共用「常驻 Daemon + 瘦客户端 CLI + 独立 Popup Host / GUI Host」；Unix domain socket 与 Windows named pipe 只在传输、进程和桌面适配层分叉。
- **输出契约**：CLI 的 stdout 只输出结果区块，所有日志走 stderr。

## 运行架构

> 详见 `docs/specs/daemon-architecture.md`。新需求设计时应按当前架构考量：渠道、长连接和抢答归 Daemon，弹窗归独立 Helper，CLI 只做入参与结果转发。

**进程职责（同一二进制按角色切换，本地 IPC 通信）**：

- **AskHuman CLI**（多、短命）：解析 argv（`-f` 在此解析为绝对路径、缺失即退 1）→ 提交 `AskRequest` 给 Daemon → 流式取回结果打到 stdout → 按终态映射退出码 0/1/3。
- **AskHuman Daemon**（macOS/Linux/Windows 每用户 1 个、常驻、**无 GUI**）：独占四种 IM 的 Router/长连接和实验性 iOS 渠道的 Gateway（一条 MQTT 连接，端到端加密，见 `docs/overview-ios.md`），承载每请求的 Coordinator/Preemption，集中落盘，监听配置变更，并管理空闲退出、二进制换新和排空。
- **Codex App 适配器**（macOS）：由 Daemon 独占原生 IPC，将桌面会话状态、控制与新建任务接入现有 registry / Watch / 问答协调；随 Codex 整体集成启停，新任务默认优先 App、可改为优先 CLI。详见 `docs/specs/codex-desktop-session-integration.md`。
- **Popup Host**（默认合并模式，每 Daemon / Dev Instance 至多 1 个）：独立 Tauri 进程，通过一次认证连接承载统一作答窗口；普通提问、权限和 Stop 按请求 ID 路由。答完本轮隐藏，按 popupPrewarm 决定保留空闲宿主或退出；故障由 Daemon 有界重建。见 `docs/specs/popup-request-inbox.md`、`docs/specs/popup-prewarm.md`。
- **独立 Popup Helper**（可选 `channels.popup.windowMode=independent`）：每请求一个窗口，沿用旧冷热池及焦点仲裁；模式切换只影响新请求，保留在途窗口和草稿。
- **GUI Host**（macOS/Linux/Windows 每用户至多 1 个、长命）：承载菜单栏/托盘，以及全局唯一的设置、历史、待办、Agent、Interject、新建任务与 Fork 窗口；各打开入口通过自有 IPC 路由到宿主。

**关键约定**：每种 IM 渠道全局只保留一条连接；每个请求仅首个终态回答生效；IPC 和运行状态均为用户私有；既有 stdout、结果区块、退出码和配置兼容契约保持不变。Daemon 排空换新见 `docs/specs/daemon-graceful-drain.md`。自身盘上更新在全部在途调用与窗口具备恢复能力时保存原题并即时交接，CLI/窗口重连，IM 接管原卡，iOS 保留草稿与提交；能力不足时保留旧服务并继续受理兼容新题，旧调用结束后接班。macOS/Linux exec 稳定安装入口，不依赖托盘或新 CLI；显式停止、信号和闲退不自启，崩溃自愈仍未提供。恢复身份、可靠首答、交接屏障及首次上线边界见 [即时换新](specs/daemon-live-update.md)。

**多份二进制并存**：握手比较的是二进制内容指纹，所以内容不同的两份 AskHuman（npm 全局安装 + `install.sh` 本地构建等）会轮流把对方的 Daemon 顶掉。Daemon 因此记录每次让位的原因与客户端，并在同一对二进制 10 分钟内互顶 ≥2 次后不再为「内容不同但协议号相同」的客户端让位（hold）；`daemon status`、`doctor` 与 `install.sh` 会指出多份安装、PATH 遮蔽和 hold。设计见同一份 spec 的 D10–D12 与 §6。

普通调用与显式 `--instance system` 从操作系统账户记录选择正式根，内部以解析后的 `Target.home` 派生数据与 IPC 路径。
System 的用户级 Agent 集成及后台 HOME 跟随账户，前台输入 `~` 保留调用方 HOME 语义。
旧临时 HOME daemon 继承的 v1 System 目标在原根续接，配置与共享凭据只读。
内部 System 测试通过显式 fixture v2 固定私有账户，跨程序执行与 endpoint 先校验能力和身份。
兼容、路径及测试入口见 [账户主目录路由](specs/system-instance-home-routing.md)。

**重复提问收敛**（`docs/specs/duplicate-ask-coalescing.md`）：agent 那一轮被中断后常原样重发同一个提问，而被中断的 CLI 进程不会被杀。Daemon 因此在建请求前先按「会话键 + 提问指纹」判重——同一提问仍在等人回答就合流到那张卡（多等待者共享一个终态，全部离场才取消），5 分钟内已答过则直接重放上次答案并在 `status` 说明。历史、待办出队与 `ask-received` 仍只发生一次。

**Dev Instance（并行 / WorkTree 开发）**：AskHuman 自身开发通过项目规则与独立 `askhuman-dev` MCP
准备每树私有程序、home、daemon 和测试 profile；普通用户级规则、MCP 六工具和设置保持原流程。
幂等 `dev enable` 写本树 `workspace.json`，它不触发普通 cwd 改道；旧 `enabled` 在本树明确升级前保留旧行为。
安装后显式 Dev 验收默认由正式 daemon 的唯一远程连接转送；权限记忆、Stop 和待办动作只在原 Dev 执行。
缺失、旧构建或不兼容正式端均失败，不切换日常会话，不隐式回退。真实渠道实现测试用独立 bot／配对。
`ASKHUMAN_HOME` 仍不隔离用户级 Agent 文件；项目 Dev 拒绝改日常集成，完整测试用独立 profile。
实现与能力边界见 [开发验收实现地图](overview-development.md)，操作见 [工作树准备流程](agent-worktree-setup.md)，
需求见 [项目级开发验收](specs/dev-verification-routing.md)。

## 目录结构

```
AskHuman/
  vite.config.ts  package.json  tsconfig.json    Vite/前端构建配置
  scripts/                   安装、发布、版本与性能脚本
  docs/wiki/                 用户文档（中英双语）；specs/plans 为开发文档
  .github/workflows/         三平台构建与发布
  ios/                       iOS App（XcodeGen 工程 + 本地 Swift Package AskHumanKit + 通知扩展 + UI 测试）与跨语言线协议夹具
  relay/                     推送中继（零依赖 Node.js，腾讯云云函数）
  todo-sync-helper/          iCloud 待办同步的 CloudKit helper（XcodeGen 工程 + Swift 包 Kit + RPC 夹具；专题见 overview-todo-sync.md）

  src/                       Vue 前端（Vite 根目录）
    index.html               前端入口、首帧关键样式与平台探测
    main.ts                  挂载 App、引入全局样式
    App.vue                  按 URL 路由 popup/settings/setup/history/agents/interject/todos/newtask/forktask
    views/PopupInboxView.vue  共享作答窗口、请求导航、稍后回答与小窗（专题见 overview-popup-ui.md）和统一关闭
    views/PopupView.vue      每请求提问与回答正文（编排层；状态与区块组件在 views/popup/）
    views/AgentsView.vue     Agent 控制台（双栏：边栏+详情；spec gui-agent-console）
    views/console/           控制台子组件（边栏/指示器/Watch 帧/完整会话/diff 条/交互区/内嵌任务）
    views/InterjectView.vue  Agent 插话编辑器
    views/TodosView.vue      项目待办窗口（侧栏切项目 + 全部待办 + 增删清空；iCloud 模式见 overview-todo-sync.md）
    views/todos/             待办窗口子组件（侧栏、同步状态槽）与纯函数（侧栏视图模型、交互、同步状态映射）
    views/NewTaskView.vue    新建 Agent 任务窗口壳（表单主体在 views/newtask/NewTaskForm.vue，与控制台内嵌版共用）
    views/ForkTaskView.vue   原生会话 Fork 独立窗口（固定源会话 + 权限 + 分支指令）
    views/newtask/LaunchPermission.vue  新建/Fork 共用权限控件
    views/SettingsView.vue   设置页（编排层；各 tab 组件与域逻辑在 views/settings/）
    views/SetupView.vue      安装后设置向导（三页：接入 Agent / 远程回答 / 完成；页面与状态在 views/setup/，设置页的横幅与入口卡在 views/settings/Setup*.vue）
    views/HistoryView.vue    回复历史列表、搜索与筛选；右侧附件预览面板与分栏
    views/history/useHistoryPreview.ts  历史窗口唯一的附件预览控制器（面板本体复用 views/popup/）
    views/history/useAttachmentFilter.ts · AttachmentFilter.vue · AttachmentChips.vue  附件筛选（状态、按钮 + 面板、条件标签）
    components/HistoryDetail.vue  单条历史的只读详情
    lib/history.ts           历史记录展示相关纯函数
    lib/historyAttachments.ts  附件事实（数量 / 后缀）与筛选的纯函数
    lib/ipc.ts               Tauri invoke 封装
    lib/types.ts             与 Rust 模型对齐的 TS 类型
    lib/markdown.ts          Markdown 渲染与代码复制
    lib/theme.ts             主题应用与文件 Data URL
    styles/{tokens,base,controls}.css   设计 token、基础与控件样式

  src-tauri/                 Rust 后端
    Cargo.toml               Rust 依赖与 feature
    vendor/drag/             drag 2.1.1 的本地补丁副本（[patch.crates-io]，只改 objc2 特性声明；原因见 docs/development.md）
    tauri.conf.json          Tauri 构建和窗口配置
    capabilities/default.json 窗口能力权限
    src/
      main.rs                入口，调用 cli::dispatch()
      dev_instance.rs · dev_presets.rs  WorkTree Dev Instance 与渠道预设
      macos_quicklook.rs     macOS Quick Look 与文件图标
      macos_menu.rs          macOS 附件原生右键菜单
      attachment_preview.rs Popup 请求内附件有界读取、类型与资源判定
      image_resource.rs · local_image.rs 共用图片预算与窗口/文档范围的原图二进制 URL
      attachment_diff.rs · attachment_markdown.rs  Popup / Quick Look 共享解析与静态渲染

      cli/
        mod.rs               argv 总分发与各运行角色入口
        args.rs              提问参数解析
        cfgio.rs             CLI 配置读写、密钥输入与本地化工具
        config_cmd.rs        config 子命令
        channel_cmd.rs       channel 配置、测试与识别子命令
        ios_cmd.rs           channel ios 子命令（status / pair / allow / devices / revoke）与连接串导入
        agents_cmd.rs        agents 模式、状态与 capability 子命令
        setup_cmd.rs         setup 子命令（GUI 向导，无图形会话或 --text 时走终端文字流程）与无参 `AskHuman` 的一行欢迎语
        doctor.rs            daemon、已装二进制、渠道和 Agent 集成体检
        install_check.rs     已装 AskHuman 二进制 / PATH 遮蔽 / Daemon 互顶检查（doctor install）
        debug_cmd.rs         不进入 help 的调试子命令
        todo_cmd.rs          todo add/list/attach/detach/rm/clear 子命令
        todo_sync_cmd.rs     隐藏的 todo-sync 子命令（排障与 helper 拉起 tick 的入口）
        away_cmd.rs          away on/off/status 子命令（status 直读状态文件、不拉起 Daemon）
        file_attachment.rs   -f 路径解析与校验
        output.rs            文本/JSON 结果格式化
        image_writer.rs      回复图片落盘
        help.rs              CLI 帮助与版本文案

      models.rs              请求、选项、结果与 Confirm 共用模型
      config.rs              AppConfig 读写、默认值与旧路径兼容
      paths.rs               home/config/temp/history/IPC 等路径，paths/account.rs 查询账户与验证 fixture
      secrets.rs             系统钥匙串与密钥回退
      project.rs             可信会话工作区 / cwd 回退的统一项目归属
      project_info.rs        Popup 项目卡的只读 Git 与终端目录动作
      history.rs             JSONL 回复历史存储
      show_last.rs           按 Agent session/MCP instance 恢复最近完成问答
      context_binding.rs     MCP 会话的一次性 token 与 Grok 安全旁路绑定
      todos.rs               项目级待办队列（todos.json 直读直写 + 文件锁）
      todo_attachments.rs    Todo 附件托管、缩略图、生命周期与执行交付副本
      todo_sync/             跨 Mac 待办同步核心（实验，仅 macOS；专题见 overview-todo-sync.md）
      autochannel.rs         IM 命令分类、活跃槽与共享文案
      perf.rs                跨进程性能埋点
      prompts.rs             CLI/MCP Agent 参考提示词
      mcp/
        mod.rs               STDIO MCP server 运行时
        ask.rs               ask / whats_next / show_last / todo_add / todo_list / todo_update 工具
      hooks.rs               用户级 hooks
      watch.rs               四渠道 /watch 共用状态与文案
      select.rs              跨渠道单选卡模型
      gitutil.rs             IM /diff 与 /stage 的 Git 操作
      confirm/               跨渠道双动作确认模型与传输
      export/                diff/transcript 附件渲染
      agents/transcript_full.rs  五家 Agent 完整会话解析
      agents/workspaces.rs       IM 新任务的最近工作目录索引与五家冷扫描
      dingtalk/confirm.rs    钉钉双动作确认卡
      sound.rs               跨平台弹窗提示音
      idle.rs                Mac 键盘 / 鼠标空闲时长（macOS 读 CoreGraphics，其它平台未知）；目前只供 iOS 网关决定何时推送
      commands.rs            前端调用的 Tauri command 集合
      popup_drop.rs          Popup 拖入文件：失败带原因与位置类别的图片读取，以及不含路径的拖拽诊断日志
      codex_desktop/               可选 macOS Codex App 原生会话与创建适配器
      cursor_desktop/              可选 macOS Cursor Desktop 会话防护（后台完成续跑拦截与 Agents 窗口运行中会话保护两个独立守卫）、私有运行桥，以及 Agents 窗口新建任务（create/launch）
      integrations/agent_launch.rs  新建/Fork 共用一次性 LaunchRecord、readiness 与固定 argv adapter

      app/
        mod.rs               Tauri 运行时与各类窗口创建
        gui_host.rs          GUI Host、托盘和 daemon 状态订阅
        tray_menu.rs         托盘菜单模型与最小 diff
        away_capsule.rs      macOS 离开模式桌面胶囊与「欢迎回来」（展示数据与判定是平台无关的纯函数，窗口只在 macOS）
        coordinator.rs       首答胜出与其它渠道取消
        popup_recovery.rs    窗口 IPC 重连与未确认提交续送，保留已挂载表单
        popup_inbox.rs · popup_inbox_geometry.rs  共享宿主请求状态及三区几何事务
        popup_canvas.rs · swift/PopupCanvas.swift  过渡固定画布与普通原生自适应布局；小窗的缩放由 swift/PopupCollapse.swift 逐帧动画
        popup_activation.rs · swift/PopupActivation.swift  Popup Host 的应用级激活状态（小窗的切走缩回）
        popup_drag_front.rs · swift/PopupDragFront.swift  macOS：文件拖进 Popup 时先取前台再让 web view 读文件，拖走不松手则还回原前台应用
        popup_transition.rs  非激活前置（到达提醒由前端气泡动效承担）
        popup_preview.rs · popup_preview_geometry.rs  附件面板的窗口几何（Popup 与历史窗口各一个控制器）
        popup_preview_actions.rs  附件面板（Popup / 历史）的原文件菜单与跨平台定位
        attachment_owner.rs  预览命令的授权入口：所有者 ID（Popup 请求 / 历史条目）→ 附件，路径只来自后端

      channels/
        mod.rs               Channel、ResultSink 与 Preemption 抽象
        conversation.rs      会话型渠道公共编排
        popup.rs             本地弹窗 Channel
        telegram.rs          Telegram Channel 适配
        dingding.rs          钉钉 Channel 适配
        feishu.rs            飞书 Channel 适配
        slack.rs             Slack Channel 适配
        ios.rs               iOS Channel 适配（把请求交给 Gateway）
        confirm.rs           Confirm 的四种 IM adapter

      telegram/
        mod.rs               Telegram Bot API 客户端
        markdown.rs          Markdown 到 Telegram HTML
        router.rs            Telegram 单长轮询与会话路由

      dingtalk/
        mod.rs / token.rs / client.rs / stream.rs / card.rs / textfile.rs / docx.rs
                             钉钉认证、Stream、卡片与附件
        watch.rs             钉钉 /watch 卡渲染与回调
        router.rs            钉钉单 Stream 连接与会话路由

      feishu/
        mod.rs               模块与错误类型
        token.rs             tenant_access_token 缓存
        client.rs            飞书 OpenAPI 客户端
        ws.rs                飞书 WebSocket 长连接
        card.rs              飞书卡片组装与回调解析
        router.rs            飞书单长连接与会话路由

      slack/
        mod.rs               模块与错误类型
        client.rs            Slack Web API 客户端
        ws.rs                Slack Socket Mode 长连接
        blockkit.rs          Block Kit 组装与回调解析
        markdown.rs          Markdown 到 Slack mrkdwn
        router.rs            Slack 单长连接与会话路由

      ios/                   iOS 渠道（实验）：线协议与密码学、MQTT 传输、持久状态、Gateway actor（`gateway/`）；专题见 overview-ios.md
      integrations/
        hook_edit.rs         JSONC Hook 的最小编辑
        mutation_lock.rs     集成配置写入的跨进程锁
        agent_permission.rs  Claude/Codex 权限审批集成
        cursor_hook.rs       Cursor 超时 Hook 管理
        claude_hook.rs       Claude Code 超时 Hook 管理
        agent_lifecycle.rs   五家已适配 Agent 的生命周期管理（原生 Hook + Pi Extension；Antigravity 不支持）
        agent_launch.rs      Agent readiness、一次性启动记录与 Terminal.app / Windows Terminal helper
        agent_start.rs       「这家 Agent 现在能不能启动任务」的唯一判定（CLI 就绪，或 Codex 桌面 App / Cursor Agents 窗口路线可用），手机 / IM / 向导 / 文字向导 / 设置页共用，并给出不能启动时的原因
        agent_rules.rs       Agent 全局 Rules 管理
        agent_subagent_guard.rs  Claude/Codex SubagentStart 提示 Hook
        agent_context_recovery.rs  集成 mode 托管的压缩恢复/会话绑定 Hook
        antigravity_rule.rs Antigravity 共享规则的严格所有权、恢复日志与 Unix 写入
        codex_protocol.rs    Codex 协议载体（SessionStart Hook）的状态、版本门槛与旧 AGENTS.md 区块迁移
        agent_delegated.rs   「允许被委派的 Codex 运行使用 AskHuman」偏好（delegated-preferences.json）
        grok_skill.rs        Grok interaction-protocol skill 管理
        mcp_config.rs        五家 MCP server 配置管理
        pi_extension.rs      Pi CLI/生命周期/插话/Stop 共用 Extension 管理
        agent_mode.rs        None/CLI/MCP 模式编排
        agent_stop.rs        Agent Stop 结束确认配置
        login_item.rs        macOS/Linux/Windows GUI Host 与 daemon 登录项

      onboarding/            安装后设置向导的后端（spec onboarding）：Agent / 远程渠道 / 后台三域的「已决定」判定（只把用户的明确选择写进 ui-state.json 的 `onboarding` 字段，其余从机器现状推导）、Agent 表 · 检测 · 应用 · 将改文件预览、环境体检、终端文字流程 `text.rs`，以及可选验证 `probe*.rs`

      ipc/
        mod.rs               CLI/Daemon/Popup 消息类型
        codec.rs             NDJSON 编解码
        transport.rs         Unix socket / Windows named-pipe 传输与用户隔离
      gui_host/
        mod.rs               GUI Host 自有 IPC 与窗口路由
      client/
        mod.rs               CLI 到 Daemon 的连接与管理操作
        composer.rs          Interject 窗口的 daemon 连接
      daemon/
        mod.rs               跨平台 daemon 子命令入口与共享 server core 映射
        recovery.rs · recovery/  私有事务恢复存储、进程存活证明与 daemon 自持消费身份
        runtime/
          mod.rs             状态与类型、serve 主循环、连接分发与请求提交
          live_update.rs     可恢复调用、恢复屏障、渠道暂停/接管与后台自动交接
          watch.rs           watch 订阅持久化、tick 刷新与卡片回调
          select.rs          跨渠道单选卡发送、路由与回调分发
          inbound.rs         IM 入站命令层与共享命令处理
          fork.rs            IM /fork 选择、权限、指令与启动流程
          todo.rs            IM /todo·/todo-rm·/todo-auto 命令与待办管理卡
          subs.rs            GUI/托盘/Agent 订阅广播与 Interject 连接
          detect.rs          渠道自动识别流程
          ios.rs · ios_agents.rs · ios_tasks.rs  iOS Gateway 的 Host 实现（启停、Agent / Watch / 插话、新任务）
          task_launch.rs     IM /new 与 iOS 共用的 Agent 任务启动
          away_ops.rs        离开 / 静音的自动回复、AwayOp 应用、回答里的 directive 与状态广播
        away.rs              离开 / 静音引擎：纯判定 judge、AwayEngine、state/away.json、引导语
        lifecycle.rs         单实例、指纹与空闲生命周期
        restart_guard.rs     换新原因记录，以及两份二进制互相顶替时拒绝再让位（hold）
        spawn.rs             Daemon 脱离启动
        request.rs           请求登记、Coordinator 与 GUI token
        config_watch.rs      config.json 监听与重载
      update/
        mod.rs               更新检测与 updater 选择
        direct.rs            GitHub Release 直接更新
        npm.rs               npm 全局更新
        notes.rs             release notes 获取与聚合
        state.rs             update.json 状态
      agents/
        mod.rs               Agent 类型、事件与模块入口
        detect.rs            Agent 家族、会话和进程探测
        report.rs            生命周期 Hook 上报与去重
        result.rs            统一回合结果、原生回合关联与保守分类
        control.rs           原会话消息的一次性回执与 Pi 控制连接
        context_recovery.rs  压缩后提示、MCP token 注入与 Grok pending 上报；Codex 协议注入决策
        delegated.rs         Codex 委派运行判定（信号、有界探测、会话标记）与静默策略
        title.rs             五家会话标题解析
        session_info.rs      原生工作区 / 模型 / 上下文与 Hook 私有快照
        activity.rs          transcript 尾部活动解析
        session_paths.rs     Pi 自定义 sessionDir transcript 路径校验与缓存
        cursor_vscdb.rs      Cursor IDE 会话实时源（state.vscdb；活动/标题/完整会话）
        cursor_log.rs        Cursor 结构化日志的只读尾随（重试用尽、不发 stop 的最终出错）
        cursor_max/          旧按次计费计划下 Cursor Max 模式的判定、强制关闭与启动后复查（规格 cursor-max-control）
        registry.rs          Agent 状态推导、持久化与快照
        interject.rs         插话队列、等待与持久化
        stop.rs              Stop Hook 捕获与原生 continuation

  cliff.toml                 git-cliff 配置
  docs/release-notes/        每版本可选的 release notes 覆盖文件
```

## 运行流程

1. `main.rs` → `cli::dispatch()`：在创建任何窗口前按 argv 分流纯信息、管理、GUI 和提问命令。
2. macOS、Linux 与 Windows 上，CLI 把参数规范化为请求，连接或拉起 Daemon 后提交；本地传输分别使用用户私有的 Unix socket 或 Windows named pipe。
3. Daemon 登记请求，按配置派发到共享 Popup Host，并把请求交给已启用的 IM Router；各渠道并行等待回答。
4. Coordinator 只接受首个终态结果，随即取消其它渠道；历史、回复图片和文件在这一汇聚点统一处理。
5. Daemon 把最终结果回传 CLI；CLI 只负责写 stdout 并按终态返回退出码。

## IPC 协议号（`PROTOCOL_VERSION`）

> 常量在 `src-tauri/src/ipc/mod.rs`；换新、排空与互顶的约定见 `docs/specs/daemon-graceful-drain.md`（D5、D10–D12）。

CLI、Popup Host、GUI Host 与 Daemon 共用 NDJSON 消息类型（`ClientMsg` / `ServerMsg` 及其载荷）。原有 `ClientHello` 要求**协议号严格相等**，指纹不同时按排空、更新意图与多安装 hold 规则处理。可恢复调用和窗口先读 Status 的恢复能力，再使用独立版本的 `Recover` 入口跨后台换代，避免旧进程把新版后台换回自己的映像。hold 也会让同号不同内容的客户端与当前 Daemon 通信，所以**同一协议号内必须保持互通**。

改 `ipc/mod.rs` 里的消息类型前，先按下表判断要不要 +1：

| 改动 | 是否 +1 |
|---|---|
| 给已有结构体加字段，带 `#[serde(default)]`（或 `Option`），缺省值对旧端行为无害 | 不 +1：新端读得懂旧消息，旧端忽略多出来的字段 |
| 新增枚举变体（`ClientMsg` / `ServerMsg` / `HelloStatus` 等） | 旧端可能收到它就 +1（旧端解码失败会断连）；能证明旧端永远收不到才可不加 |
| 删除 / 重命名字段或变体、改字段类型或含义、新增必填字段、改握手或消息先后顺序 | 必须 +1 |
| 只改 Rust 内部实现、不改线上 JSON 形态 | 不 +1 |

+1 的代价是每个跨号的客户端与 Daemon 互相让位、旧客户端无法与新 Daemon 共存，所以能用「加可选字段」解决的不要 +1。+1 时：改常量，在下面的历史补一行（日期、提交、变更点），并在 `daemon-graceful-drain.md` 记下升级过渡期的影响。

**历史**（当前为 5）：1 = Daemon 骨架（`9660532`，2026-06-07）；2 = 结构化确认（`2c76c44`，2026-07-11）；3 = 共享作答窗口，新增 `PopupHostHello` / `PopupHostIdle` / `PopupHostAccepted` / `PopupSubmissionAck` / `PopupRequestKind` 等（`799334f`，2026-10-06）；4 = iOS 渠道管理，新增 `ClientMsg::Ios` / `ServerMsg::Ios`（`IosOp` / `IosResult`）（`128bffd`，2026-10-25）；5 = Cursor Desktop 新建任务，新增 `ClientMsg::CursorDesktop`（载荷 `Operation`）/ `ServerMsg::CursorDesktop`（载荷 `Result<Value, String>`），与 Codex Desktop 同构（`2600561`，2026-11-03）；协议 5 还含 Mac 控制台给 Cursor Desktop 聊天直发消息的 `ClientMsg::AgentMessage`（载荷 `ConsoleMessage`）/ `ServerMsg::AgentMessage`（载荷 `Result<Value, String>`）（2026-11-04）；还有离开模式的 `ClientMsg::Away`（载荷 `AwayOp`）/ `ServerMsg::Away`（载荷 `AwaySnapshot`）（2026-11-05；`Answer.directive`、`TaskRequest.escalate`、`AskRequest.escalated` / `quick_reply` / `quick_replies` 与托盘 / Agent 快照里的 `away` 都是带 `#[serde(default)]` 的可选字段，不另计）——这些变体在 0.17.0 发布前一并纳入 5；5 已随 0.17.0 发布，之后新增旧端可能收到的变体就必须 +1。对应发布版：0.4.3–0.8.2 为 1，0.9.1–0.13.2 为 2，0.14.2–0.15.1 为 3，0.16.0 为 4，0.17.0 起为 5。排空相关的增量（`HelloStatus::Draining`、`ServerMsg::Draining`、`Stop.force`、`StatusInfo.draining`）和 `StatusInfo` 的二进制信息字段都没有 +1。

恢复能力另用带默认值的 `StatusInfo.recoveryVersion` 协商，`Recover` / `Recovery` 信封只在声明能力的连接间使用。
旧端始终走原入口并只收到原消息，所以保持协议号 5。稳定恢复入口绕过指纹让位，避免存活旧 CLI/窗口将新后台换回旧构建。
每次扩展继续审计全部发送点和窗口广播，具体约束见 [即时换新](specs/daemon-live-update.md)。

## 前端 ↔ 后端命令（`commands.rs` ↔ `lib/ipc.ts`）

- 弹窗：按 request ID 的 `popup_init`、`submit_popup`、`confirm_popup`、`cancel_popup`；共享容器 `popup_inbox_init` / `activate` / `layout` / `commit` / `idle`
- 附件面板（Popup 提问附件与历史详情）：`popup_preview_prepare` / `layout`、`popup_preview_read` / `cancel_read` / `thumbnail`、`popup_preview_reveal` / `menu`（命令名沿用；`request_id` 实为所有者 ID：Popup 请求 ID，或历史窗口的 `history:<条目 ID>`）；原文件打开仍用 `open_path`
- Popup 拖入文件（`popup_drop.rs`）：`popup_drop_read_image`（读拖入的图片，失败时拒绝出 `{reason, where, message}`）、`popup_drop_log`（把每次拖拽的结果写进 `daemon.log` 的 `askhuman_drop` 行，只记标签与计数，不记路径和文件名）
- 其他附件入口：`preview_attachments`、`close_preview`、`read_image_data_url`、`file_icon_data_url`、`show_attachment_menu`、`attachments_exist`（逐路径答复原文件是否还在，只有 `NotFound` 算不存在；弹窗与历史的胶囊据此标出已不存在的附件并拒绝拖出）
- 离开模式与快捷回复：`away_end`（弹窗 / 控制台里「结束离开模式」，经 `client::away_op`）、`quick_reply_defaults`（设置卡的出厂预设，中英各一份）、`capsule_init` / `capsule_resize` / `capsule_dismiss_welcome` / `capsule_focus_waiting`（桌面胶囊窗口，仅 macOS）
- 设置：`get_settings`、`save_settings`、`get_prompt`、`set_theme`、`update_theme`、`open_settings`、`popup_sound_support`、`play_popup_sound`
- 历史：`open_history`、`history_init`、`get_history`、`get_history_projects`、`history_count`、`trim_history`、`resolve_history_session_titles`、`delete_history_entries`、`clear_all_history`
- Cursor / Claude 超时 Hook 与 Pi Extension：前两者保留专用命令；统一设置入口走 `agent_mode_*` 与 `agent_hook_reveal` / `open`
- Agent Rules：`agent_rule_status` / `install` / `update` / `uninstall` / `reveal` / `open`
- Agent 模式与配置文件：`agent_mode_status` / `set` / `update`、`mcp_config_reveal` / `open`、`agent_hook_reveal` / `open`、`mcp_command_path`（`agent_mode_status` 只聚合本地集成产物；Agent CLI 是否在 PATH / Pi 版本由 `agent_task_readiness` 负责，它同时带上统一的「现在能不能启动任务」结论 `startable` / `startRoute`，见 `integrations/agent_start.rs`）
- 渠道测试与识别：`telegram_test`；钉钉、飞书、Slack 各自的 `*_test` / `*_detect_prepare` / `*_detect_wait`；共用 `detect_cancel`
- iOS 渠道（设置页「实验」Tab）：`ios_op`（状态 / 配对 / 撤销 / 重命名，经 IPC 的 `ClientMsg::Ios`）、`ios_test`、`ios_pair_qr`、`ios_connection_export` / `ios_connection_import`
- iCloud 待办同步（设置页「实验」Tab 与待办窗口侧栏）：`todo_sync_*`（状态 / 启用 / 停用 / 立即同步 / 清除 / 修复，以及 iCloud 模式窗口用的项目列表、详情与移除；全部在阻塞线程上跑），见 `docs/overview-todo-sync.md`
- 版本自更新：`get_app_version`、`update_check`、`update_get_notes`、`update_apply`、`update_dismiss`、`popup_update_state`、`restart_settings`
- Agent 生命周期：`agents_init`、`agent_force_idle`；自动集成卡内的 capability 通过
  `agent_mode_status` 聚合，并由 `agent_lifecycle_status` / `install` / `uninstall` 查询或切换
- Agent 控制台（spec gui-agent-console）：`agents_focus`（焦点会话，daemon 推 `agent-detail` 帧）、`focus_request`（去回答）、`interject_append` / `interject_peek`（输入框追加与待送达气泡）、`agents_message`（空闲或已结束的 Cursor Desktop 聊天——快照带 `canMessage` 的那些——直接送进原聊天，经 `ClientMsg::AgentMessage`，答复 `accepted` / `rejected` / `unknown`）、`console_transcript`（完整会话分页）、`console_diff_stat` / `console_diff_file` / `console_stage`（项目 diff 状态条与暂存）
- 新建 Agent 任务（GUI）：`open_new_task`、`new_task_init`、`new_task_projects`(+`_refreshed`)、`project_key_of`、`new_task_launch`

Popup 的窗口、附件、来源标题与交互实现地图见 `docs/overview-popup-ui.md`：

- 弹窗支持 Markdown、附件拖入 / 拖出、同窗右侧附件预览与原文件菜单；历史详情共用同一块预览面板，待办等其他入口保留既有预览。
- 来源名优先级为自定义环境变量 > 发起 Agent > 默认「the Loop」；头部同时展示 Agent、workspace 和提问时间。
- 多问题默认纵向平铺全部问题（`experimental.verticalQuestions` 默认开，设置可关回一次一题），设计见 `docs/specs/multi-question-vertical.md`。
- 推荐选项不自动预选，提交值始终为原文，规格见 `docs/specs/recommended-option.md`。

## iOS 渠道（实验）

Daemon 里的又一个渠道：手机上的 AskHuman App 回答问题、看 / 关注 / 观察 / 插话 Agent、创建 Agent 任务。专题实现地图见 `docs/overview-ios.md`，需求与计划见 `docs/specs/ios-channel.md`、`docs/plans/ios-channel.md`，用户说明见 `docs/wiki/ios-setup.md`。

- **形态**：Mac Gateway ↔ 自备 EMQX（MQTT TLS / WSS）↔ iOS App；推送经 `relay/` 的无状态云函数到 APNs。Mac 不持有任何 APNs 密钥，中继不存任何状态。
- **不变量**：端到端加密（每配对一对方向密钥，broker 与中继只见密文）；一份 broker 凭据多台 Mac 共用；状态变更消息按 `msgId` 幂等；渠道藏在 `experimental.enabled` 后，`channel list` 与帮助默认不列出；EMQX 消费上限保持 0，不自动付费。
- **投递与通知分开**：iOS 不参与 IM 的活跃槽 / `channels.autoActivation`——每个提问和确认都挂给所有已配对手机，手机一打开就能拉到；手机没在看时的系统推送只在 Mac 键盘 / 鼠标空闲满 `channels.ios.alertAfterSecs`（默认 15 分钟，`0` = 立即）后才发，每题每手机最多一条，不重复提醒。规格见 `docs/specs/ios-alert-timing.md`。
- **手机上的回答与离开模式**：提问里可以选项目待办（`AnswerSubmit.todoIds`，Mac 网关按提问的项目反查并合并进回答，What's next 里看完整队列；`Caps.answerTodos` 缺失的旧 Mac 不显示），回答的底栏菜单有快捷回复与「离开模式 / 不再使用」两条指令（`AskView.quickReplies`、`AnswerSubmit.directive`），Agents 页有离开模式的常驻条、开启页与会话行（`away.state` / `away.set` / `away.session.set`、`Caps.away`）。细节见 `docs/overview-ios.md`，规格 `docs/specs/special-replies.md`。
- **跨语言**：Rust `ios/{crypto,envelope,wire}.rs` 与 Swift `AHProtocol` 共用 `ios/Fixtures/wire-v1/`，限额常量两边一致；改线协议先改夹具。
- **验证**：Rust Gateway 测试台、Swift 包测试、真实 daemon 端到端（一次性 Mac + 本地 broker）、模拟器 UI 测试与中继 `node --test`，命令见 `docs/overview-ios.md`。

## UI / 主题

- 主题三态：`system`(prefers-color-scheme)/`light`/`dark`；前端切根类 + 后端设原生窗口主题。
- macOS 窗口材质三态：Solid 为完整不透明主题底且不使用 Visual Effects；Blur 使用 Tauri `underWindowBackground`；Glass 仅在 macOS 26+ 使用 `NSGlassEffectView`，旧系统的 `glass` 配置有效值为 Blur。三态共用 `TitleBarStyle::Overlay` + 隐藏标题；Windows/Linux 维持纯色不透明底。
- Markdown 配色见 `styles/controls.css`（链接/代码块/表头/引用/hr 等）。代码块 hover 右上角有拷贝按钮（`.code-copy`，点击复制 `<code>` 文本，弹窗/历史详情/设置发布说明共用 `renderMarkdown` 故都生效）。

## 配置

`~/.askhuman/config.json` 是主配置文件，模型与默认值在 `src-tauri/src/config.rs`；新位置缺失时自动回退旧 `~/.humaninloop/config.json`。
顶层分为 `general`、`channels`、`agentTasks` 和 `experimental`；缺字段走默认、未知字段忽略。完整字段地图见 `docs/overview-configuration.md`，用户向说明见 `docs/wiki/`。

### IM 命令与主动交互

四种 IM 共用 Daemon 内的入站命令层，平台模块只负责传输、渲染和回调；Daemon 存活时即持续监听消息，不要求已有提问。详细能力与代码入口见 `docs/overview-im-commands.md`。

- `channels.autoActivation` 关闭时向所有启用 IM 投放，开启时以当前活跃槽为主；切槽会补推在途请求，watch 渠道仍会加入对应 Agent 新提问的投放并集。
- 共享命令包括 `/new`、`/fork`、`/help`、`/here`、`/status`、`/watch`、`/unwatch`、`/msg`、`/msg-clear`、`/yolo`、`/diff`、`/stage`、`/transcript`、`/todo`、`/todo-rm` 和 `/todo-auto`；Slack 使用 `!` 作为可输入的备用前缀。
- macOS 或 Windows 开启 `agentTasks` 后，`/new` 依次选择 workspace、能启动的 Agent 与权限（「能启动」只有一个判定，见 `integrations/agent_start.rs`：CLI 就绪，或 Codex 桌面 App / Cursor Agents 窗口路线可用；手机、IM、设置向导与设置页都用它），在新的 Terminal.app 或 Windows Terminal 窗口（或 Codex / Cursor 的桌面路线）启动真实交互会话；Pi 没有内置权限模式，选中后跳过权限选择并固定使用 Agent 默认行为。Daemon 只负责启动前流程，之后复用 lifecycle/watch。
- macOS 与 Windows 本地 GUI 亦可创建任务（不要求开启 `agentTasks`）：待办窗口行内按钮与托盘「新建 Agent 任务」打开统一的新建任务窗口，就绪判定与启动链路与 `/new` 相同；见 `docs/specs/gui-agent-task-launch.md`。
- Cursor 任务另有两项：① **Max 控制**（`agents/cursor_max`，规格 `docs/specs/cursor-max-control.md`）——旧按次计费计划里 Max 按 API 费率扣额度，所以 GUI 表单、`/new` 与 iOS 创建 Cursor 任务时默认强制关闭 Max（需要 Max 的模型拦截启动，GUI/iOS 可按任务打开「允许 Max」），启动后复查，新计划与「沿用 Cursor 设置」策略下不干预；② **Agents 窗口路线**（`cursor_desktop/`，规格 `docs/specs/cursor-desktop-task-creation.md`，实验）——`cursorDesktop.launchPreference` 只决定两条路谁优先：首选路线用不了、另一条可用时自动改用另一条（装了 Cursor Desktop 即可启动，不必先改偏好；与 Codex 对称）；走 Agents 窗口时，新任务经 daemon 持有的私有桥在已打开的 Cursor Agents 窗口里创建并自动发送，所有核对在提交前闸门里完成，失败即拒绝，任务一旦开始不会静默改走另一条路。
- Claude Code、Codex、Grok、Pi 的 Working/Idle 原生会话可通过 IM `/fork`、Agent 控制台或托盘立即分叉；源会话继续运行，新分支共用 cwd。继承标题的子会话在控制台、托盘和 IM 选择列表中前置显示直接父序号；watch 卡不增加 Fork 按钮，Cursor CLI 不支持；详见 `docs/specs/agent-session-fork.md`。
- Agent 数字编号在 daemon 生命周期内稳定，供状态、关注、插话和 Git/会话导出共用；无参目标选择复用跨渠道单选卡模型。
- Watch 订阅持久化并就地更新原卡；`/stage` 必须经过跨渠道 Confirm，不能直接执行暂存。

### 回复历史

- `general.historyLimit` 控制 `~/.askhuman/history.jsonl` 的全局保留数；0 停止新增，裁剪/立即清理时清空已有记录。
- 历史在 Coordinator 的唯一汇聚点记录“发送”和“用户主动取消”，系统取消不记。
- 新请求按可信 Agent 会话工作区的 git 根归项目，无可信来源时回退调用 cwd；实际调用目录独立保存，
  临时 cd / 跨仓库调用不改变原会话项目。Sidebar、历史、待办与 IM 使用同一归属，不迁移旧记录。
  图片和文件只保存路径，读取对旧记录和缺失附件保持兼容。见 `docs/specs/popup-session-info.md`。
- 完成问答额外记录可用的 Agent session ID 和 MCP instance ID；`AskHuman --show-last`
  或 MCP `show_last` 可在上下文压缩后读取当前会话最近一条提问语境与实际答案（省略空字段、
  未选候选项和 recommended 标记）。取得真实 session 后只做精确查询，不向弱键级联；
  只有完全非 Agent 的 CLI 才按项目回退。
- 历史窗口用单一两级菜单按“项目 → 单个 session → 多关键词 AND 搜索”组合筛选；真实 Agent session 使用
  `agentKind + agentSessionId`，缺失时才以 `project + mcpInstanceId` 作为明确标注的近似分组。
  从弹窗进入会优先定位本次 ask 的真实历史绑定，已打开的全局单窗也会重新定位。
  搜索框旁的“附件”筛选（有无附件 / 提问·回复来源 / 后缀多选，纯前端、不落盘）与它们叠加。
- 清理菜单按上下文显示“删除当前搜索结果 / 清空所选会话 / 清空所选项目”（关键词或附件条件生效即视为搜索）；范围清理只提交确认时
  冻结的 entry ID，全量清空是独立动作，两者都只改
  `history.jsonl`，不触碰 daemon 重放缓存或待答卡片。完整约束见 `docs/specs/reply-history.md`。

### 密钥安全

- 钉钉、飞书、Telegram、Slack 的五项密钥优先存系统钥匙串，`config.json` 留空；钥匙串不可用时才回退明文，Unix 配置文件/目录权限收紧。
- macOS 钥匙串读取依赖 Aqua 安全会话；非 Aqua 来源拉起 daemon 时，经用户 GUI launchd 域启动，避免后台 daemon 读不到密钥。
- 非密钥路径使用 `AppConfig::load_without_secrets()`，只有 Daemon、IM 和设置密钥状态等确需凭据的路径使用 `load()`。读取边界见 `docs/overview-configuration.md`，完整设计见 `docs/specs/secret-storage-keychain.md`。

## 版本自更新（self-update）

> 需求/方案见 `docs/specs/self-update.md`、`docs/plans/self-update.md`。

- 支持 Direct（GitHub Release）与 npm 两种安装来源，`src-tauri/src/update/` 按 adapter 实现。
- macOS/Linux 支持应用内自动 apply：只把新二进制落盘，不主动 restart；Daemon 通过二进制指纹和
  graceful drain 在所有在途请求结束后换新。
- Windows 当前发布未签名，因此源码 capability 固定关闭自动 apply；仍支持检查、日志和忽略版本。
  Direct 打开 GitHub Release，npm 提供固定命令；`AskHuman update prepare` 会等待在途请求完成并关闭
  daemon/GUI Host，释放 `.exe` 文件锁后再由用户手动安装。
- 更新状态持久化到 `~/.askhuman/update.json`，Daemon 后台检查并推送给 Popup/GUI Host。
- release notes 默认由 Conventional Commits + git-cliff 生成，可用 `docs/release-notes/v<version>.md` 覆盖。

## Agent 自动集成能力：生命周期追踪 + 状态窗口（macOS / Linux / Windows）

> 需求 `docs/specs/agent-lifecycle-tracking.md`，结果与接续 `docs/specs/agent-lifecycle-results-and-recovery.md`，计划 `docs/plans/agent-lifecycle-tracking.md`。

- macOS、Linux 与 Windows 通过用户级 hooks 跟踪 Claude Code、Codex、Cursor、Grok，并通过
  `~/.pi/agent/extensions/askhuman/index.ts` 跟踪 Pi。Lifecycle 是 CLI/MCP 自动集成拥有的可选
  capability：首次集成默认开，显式偏好跨模式切换保留，None 必须移除实际产物；它与 IM
  autoActivation 相互独立。
- Daemon 的 `AgentRegistry` 以 session id 为主身份，推导工作中、空闲、已结束；pid/liveness 与超时只做兜底。
- 上轮结果独立持久化为 completed／failed／interrupted／incomplete／unknown；只有真实新回合开始才清除。手机 Agents 页只显示结果与原因（失败的处理动作只在收件箱的错误卡），用户中断静默，未完成使用独立提醒。
- 生命周期状态被 `/status`、watch、插话、托盘/状态窗口和 Daemon 空闲退出共同使用；修改事件或状态模型时必须检查这些消费者。
- 生命周期缺失、过期、显式关闭漂移与 None 旧孤立产物都进入 Agent 卡的更新状态，不由 lifecycle
  自身在 daemon 启动时后台迁移。只有工作中 Agent 或状态窗口连接阻止闲退；graceful drain 不受
  Agent 存活影响。
- 入口为设置「Agents」Tab 各自动集成卡、`AskHuman agents lifecycle`、`AskHuman agents monitor`
  和 `agents/registry.rs`；状态窗口由 GUI Host 承载并订阅 Daemon 快照。
- 状态窗口已改版为双栏「Agent 控制台」（spec `docs/specs/gui-agent-console.md`）：边栏项目分组会话
  + 详情区 Watch 帧（agents 订阅上的焦点会话子订阅复用 IM Watch 引擎推帧）+ 完整会话分页 +
  项目 diff 状态条 + 插话输入 + 内嵌新建任务 + 原生 Fork 入口；快照对 GUI 额外注入
  `waitingRequestId`/`waitingPreview`/`forkReady`，Fork 子会话持久化直接父 `forkedFromSessionId`。

## Agent 插话（Interject，macOS / Linux / Windows）

> 需求 `docs/specs/agent-interject.md`，计划 `docs/plans/agent-interject.md`。

- 插话依赖生命周期追踪，对工作中的 Claude Code、Codex、Cursor、Pi 开放；Grok 与 Antigravity 不支持。Pi Extension 在 `tool_call` 等待同一 daemon 裁决，并把消息转换为原生 `{block, reason}`。
- Daemon 按 session 维护并持久化待送达队列；GUI composer 整体覆盖，IM `/msg` 追加。
- Agent 下一次 PreToolUse 读取并原子消费消息；composer 已打开时 hook 可等待提交或取消，热路径不做文件 IO。
- 排队消息被实际读取后向来源 IM 发阅读回执；即时送达、撤回、覆盖或未消费的消息不回执。
- 手机另有按原生能力开放的空闲消息（错误卡的「要求继续」走同一条原生通道）：macOS Codex Desktop／Cursor Desktop，以及存活的 Pi Extension；由 `daemon/runtime/agent_control.rs` 联合 `agents/control.rs` 处理，普通工具前插话队列不能唤醒空闲 CLI。registry 标为「已结束」的 Cursor Desktop 聊天（一小时无活动的判定，聊天本身还在）只要仍在 Cursor 的会话库里也算数；Mac 控制台对这类聊天走同一个提交服务直发（快照 `canMessage`，见 `docs/specs/agent-lifecycle-results-and-recovery.md` §5.4）。真实新回合开始后，来源手机只登记该轮的一次结果通知。
- 入口为 Agent 状态窗口、托盘菜单和 IM `/msg`；队列实现位于 `agents/interject.rs`。

## 项目待办 + whats-next

> 规格 `docs/specs/todo-whats-next.md`、`docs/specs/todo-attachments.md`。

- 待办按统一项目身份（可信会话工作区的 git 根 / cwd 回退）归属，`~/.askhuman/state/todos.json` 是唯一数据源：所有进程直读直写 + 文件锁串行化，不依赖 daemon 存活，跨平台。GUI / CLI / MCP 可在创建时或对已有 Todo 独立管理附件；GUI 默认折叠附件摘要，支持新增区 / 每条 Pending Todo 拖入、新增输入框粘贴图片，以及选中行后粘贴剪贴板图片。单文件 ≤10 MiB 由 AskHuman 托管，>10 MiB 保留绝对路径引用（无稳定源路径的剪贴板图片超过阈值则拒绝），每条最多 20 个，并生成最长边 128 px 的图片缓存缩略图。用户可见写操作在落盘失败时必须报错。
- Agent 完成任务后必须调 `AskHuman --whats-next`（MCP 为 `whats_next` 工具）：固定提问 + 可选的 Agent 建议任务 + 待办 chip + 恒有「结束本轮」；顺序固定为建议任务、待办、结束，总选项最多 10 条。建议任务仅在确有建议时通过 `-o`/`-o!`（MCP `options`）传入，选择结果保持普通 Ask 的 `[selected_options]` 语义；待办派活为 `[user_input]`，准许结束为 `[selected_options]`（带附注的「结束本轮」不算准许结束，只回 `[user_annotations]`，见 `docs/specs/message-annotations.md`），取消为 `[status]`。选中的待办按 id best-effort 出队（Coordinator 汇聚点统一处理）。标记为「自动执行」（⚡）的待办优先级不变：whats-next 时不发卡、直接按队列顺序派发最靠前一条。
- 送达面：whats-next / 普通提问 Popup 折叠待办区 / Stop 确认卡（兜底）都以选项形式呈现待办（Popup 与 iOS 上的 Stop 卡是随项目待办实时刷新的队列选项，请求带可选标记 `todo_queue`；iOS 在提问里另有「待办」卡、What's next 里看完整队列，选中的 id 随回答上报，由 Mac 网关按同一规则并进回答，见 `docs/specs/special-replies.md` D28–D29）；GUI / IM 新建 Agent 任务也可直接执行待办。所有出口在执行前按卡片附件快照与最新 Todo 求交集，托管文件建立请求级交付副本，失效项转为 warning，选项只显示 `【N 个附件】`（支持富文本颜色时为灰色）而不向 IM 上传本地文件。输入面：CLI `todo` 子命令、请求页里的页内待办清单（Popup 的折叠待办区 / What's next / Stop 卡的待办行与 iPhone 提问页的待办卡，都能就地选、完成、删除、添加，见 `docs/specs/todo-list-actions.md`）、GUI 待办窗口（托盘/AgentsView 入口，Popup 顶栏的待办按钮也直接打开它）、IM `/todo`；IM 创建/管理首版仍只支持文字。
- IM `/todo`（管理卡：飞书代码卡自带输入表单，钉钉复用提问卡模板 `allow_input`，TG/Slack 文本 + 命令提示）、`/todo-rm`（复用单选卡逐条删除、就地刷新）与 `/todo-auto`（切换自动执行标记）由跨平台 daemon 承载，实现在共享 server core 的 `daemon/runtime/todo.rs`。
- macOS 与 Windows 上，待办窗口每条待办另有「创建任务」按钮：预选项目与该待办打开新建任务窗口，Terminal.app 或 Windows Terminal 启动成功后按快照出队（spec `docs/specs/gui-agent-task-launch.md`）。
- **跨 Mac 同步（实验，仅 macOS，每台 Mac 单独开启）**：同一 Apple ID 的多台 Mac 经 iCloud（CloudKit）共享待办与执行历史，规格 `docs/specs/todo-sync.md`，实现地图 `docs/overview-todo-sync.md`。不变量：`todos.json` 仍是本机唯一数据源且 `todos.rs` 的写函数不变，同步核心 `todo_sync/` 经旁路状态 `state/todo-sync/` 做字段级合并（项目身份 = git remote，同仓库的多个目录是副本，无 remote 的目录不同步），云端访问全部经 CloudKit helper `.app`（Rust 侧只有 `Transport` 抽象）；开关是标记文件，关闭时各入口只多一次 `stat`，Dev Instance 恒关闭；读取钩子只做本地阶段，唯有 whats-next 在上次拉取超过 20 秒时最多等 1.5 秒；待办窗口的打开路径不得联网、拉起 helper 或 git。本机没有目录的云端项目以虚拟键 `icloud:<身份>` 住在 `todos.json` 里。

## 离开模式、本会话不再使用与快捷回复

> 需求 `docs/specs/special-replies.md`，计划与实施记录 `docs/plans/special-replies.md`，用户说明在 `docs/wiki/settings.md`（桌面端的快捷回复与离开模式）和 `docs/wiki/ios-setup.md`（手机端）。

- **两种状态，一个引擎**：离开模式（全局，或单个会话）让 Agent 的提问与 whats-next 被 Daemon 自动回复「我不在，按你自己的判断推进；实在无法继续再问」；「本会话不再使用」（静音）让该会话得到「用户选择不再使用 AskHuman」的回复。引擎在 `daemon/away.rs`（纯判定 `judge` + `AwayEngine`），状态持久化到 `~/.askhuman/state/away.json`（临时文件 + 原子替换）；会话键是 `sid:<Agent 会话 id>`，没有就 `mcp:<实例 id>`；`Directive{kind,scope,note}` 是用户在弹窗 / 手机里选的「发送并…」，在回答被接受之后才生效。
- **拦截点**：`daemon/runtime/mod.rs::handle_submit`，在刷新 Agent 会话活动之后、去重挂靠与建请求**之前**，只拦截 `is_agent_ask`（Ask 类弹窗，非开发中继、非 Codex 原生问题）。Stop 确认、权限审批、出错卡与 Codex 原生问题既不被拦截，也没有快捷回复菜单（`AskRequest.quick_reply = false`）。自动回复是 `Accepted` + `Final`，历史 channel 为 `away` / `mute`，不进重放缓存。
- **升级与防呆**：`--escalate "理由"`（MCP `escalate: "理由"`）以非空理由主动升级；Daemon 在共享正文前置 Agent 理由，Mac / iOS / IM / 历史一致。旧 true、无值 flag 和空理由回自查指导；与普通请求共用同会话 10 分钟内前三次自动回复、第 4 次系统兜底的计数，兜底明确注明无 Agent 理由。升级不解除离开，静音仍优先；托盘「要求提问」仍给该会话 30 分钟的一次放行。IPC 保留旧 bool 并新增可选 `escalationReason`，不抬协议号。完整边界见 `docs/specs/away-completion-guidance.md`。
- **Hook 不经 IPC**：Stop 确认对静音会话放行、Claude `AskUserQuestion` 接管的 `decide()` 都直接读 `state/away.json`（读不到或解析失败一律当作正常模式，宁可多问一次）。**解除静音只能走插话队列**（不能用向原会话发消息的办法，那会触发新一轮计费）；会话结束与 15 秒一次的清扫清掉会话覆盖与计数。
- **承载面都调同一个引擎接口**（`ClientMsg::Away` → `daemon/runtime/away_ops.rs`）：托盘（月亮图标、离开分组、Agent 子菜单）、控制台（提示条、侧栏图标、会话页头下拉）、设置「离开模式」卡、macOS 桌面胶囊与欢迎回来（`app/away_capsule.rs`）、CLI `AskHuman away on|off|status`、IM `/away` `/back`、iOS（`away.*` 线消息）。Agent 进程里的 `away off` 被拒绝（用户决定什么时候回来）。离开期间升级提问推到手机的等待取 `general.away.pushAfterSecs` 与 iOS 渠道自己的等待的较小值。
- **快捷回复**：`general.quickReplies`（`null` = 出厂 5 条，随界面语言）；Daemon 在建请求时解成 `AskRequest.quick_replies`，手机从 `AskView.quickReplies` 读同一份。弹窗里一点即发、填给所有还没有答案的题；What's next 与严格选择没有预设。前端见 `docs/overview-popup-ui.md`，配置见 `docs/overview-configuration.md`，手机见 `docs/overview-ios.md`。

## 菜单栏 / 托盘图标 + 统一 GUI Host（macOS / Linux / Windows 桌面）

> 需求 `docs/specs/menu-bar-tray.md`，计划 `docs/plans/menu-bar-tray.md`。

- 每个交互式桌面会话使用单实例 GUI Host；Daemon 保持无 GUI。Host 承载设置、设置向导、历史、待办、Agent、Interject、新建任务、Fork 与托盘，所有入口路由到它以保证每类窗口唯一。
- Host 使用独立的用户私有 `gui-host.sock` 或 Windows named pipe，加跨进程锁做单实例；即使 Daemon 未运行，设置和历史仍能打开。
- `general.menuBarIcon` 支持 `off|active|always`；Windows 使用原生托盘，Linux 桌面 best-effort。
- 托盘状态订阅不保活；只有打开窗口的独立连接给 Daemon 续命。`general.daemonLifecycle=keepalive` 是正交的常驻策略。
- `TrayState` 汇总待答、IM、Agent、更新与 drain；菜单可聚焦待答 Popup、打开 Agent/插话、控制 Daemon 和应用更新。
- GUI Host 启动与 daemon 停→运行时复用 Agents 设置页口径检查集成更新；待更新时菜单显示可点击警告，无待答时 template 图标显示右上实心圆，设置内更新成功后即时清除。
- GUI Host 只在无窗口时换到新二进制；GUI Host always 与 Daemon keepalive 的登录项分别管理。`menuBarIcon = always` 的登录项只在已存在、或用户对「后台」做过选择之后才由 GUI Host 启动时维护（做过选择 = 向导点「完成」、在设置里切换菜单栏图标或点「启用」、`config set|unset general.menuBarIcon`）：新用户在选择前默认值虽是 `always`，也不会被静默写入登录项，已有登录项的用户行为不变（需求 `docs/specs/onboarding.md` D13）。

## CLI 配置与 Agent 集成（headless / 无 GUI）

> 需求 `docs/specs/cli-config.md`，计划 `docs/plans/cli-config.md`。

- headless/SSH 可用 `channel`、`agents`、`config`、`doctor` 完成渠道配置与 Agent 集成；各子命令提供 help 和 JSON 输出。
- 新用户从 `AskHuman setup [--text]` 起步（需求 `docs/specs/onboarding.md`，计划 `docs/plans/onboarding.md`）：有图形会话时打开三页向导（GUI Host 的 `setup` 窗口），无图形会话或带 `--text` 时在终端走同一流程；两者共用 `onboarding/` 与 `agents` / `channel` 的同一批写入函数，不另有一套逻辑。向导按 Agent / 远程渠道 / 后台三个域各自记「已决定」（Agent 域驱动设置页横幅、托盘项与无参 `AskHuman` 的欢迎语，远程域只驱动弹窗底部提示条，后台域不驱动提示）；可选验证用带 `setup-probe:` 前缀的原生请求 ID 标记一次普通提问（`AskRequest.setup_probe`，可选字段、不抬协议号），Daemon 对它不记历史、渠道可限定为一家。
- `channel set` 无 flag 走交互向导，有 flag 走脚本；密钥只从 env/file/stdin 或隐藏输入读取，不进 argv。
- `agents mode` 维护 None/CLI/MCP 整包；permission/stop 的 preference 独立保存，但只有集成 mode
  才安装交互接管产物，None 隐藏对应设置并卸载其 active 能力。Lifecycle 也是 mode 拥有的可选
  capability：默认开、可显式关闭、None 卸载而保留偏好。Grok 仅 None/MCP 且不支持 stop/interject；
  Pi 仅 None/CLI、不提供 MCP 或内置权限模式，最低支持版本为 0.82.0。Antigravity 在 Unix 支持
  None/CLI/MCP，推荐 CLI；Windows 集成变更明确不支持，且 Antigravity 不提供 lifecycle、Stop、
  permission、recovery、interject 或 launch/fork 能力。详见 `docs/specs/antigravity.md`。
- 上下文恢复 Hook 由当前 mode 独立托管，不依赖可关闭的 lifecycle tracking；CLI/MCP
  提示严格只指向用户当前选中的入口，缺失/过期并入当前 Hook 或 MCP 产物的更新状态。
  Codex 的这条 SessionStart Hook 同时是交互协议的**唯一载体**（`~/.codex/AGENTS.md` 不再写 AskHuman
  区块）：每个会话启动先判定是否为委派运行（`codex exec` / `e` / `review`、ephemeral app-server 线程、
  `Claude Code` 等 originator），委派运行零注入并写 `state/codex-delegated/<session>.json` 标记，
  其余运行注入协议（首行 `askhuman-protocol:<mode>:<hash8>`，`compact` 附恢复句，`resume` / `fork` 按 rollout 标记去重）。
  存量 AGENTS.md 区块显示「需更新」，由用户一键原子迁移，升级后不自动迁移。规格
  `docs/specs/codex-delegated-run-silence.md`。
- 全局交互协议把 Sub Agent 作为唯一例外并禁止其使用 AskHuman；Claude/Codex mode 另带 `SubagentStart` 提示 Hook，Cursor/Grok/Pi/Antigravity 只依赖协议文本。
- `agents update [<agent>]` 按当前 mode 重新 reconcile 单家或全部托管产物；重复设置相同 mode 也会完整更新，但不改独立保存的 capability 偏好。
- `config` 是通用键值兜底，`doctor` 汇总 Daemon、已装二进制、渠道和集成；两者复用同一配置与集成模块，不维护第二套逻辑。`doctor install` 单独检查已装二进制（`install.sh` 装完后调用）。

## MCP 支持（CLI 之外的第二种集成形态）

> 需求 `docs/specs/mcp.md`，计划 `docs/plans/mcp.md`。

- `AskHuman mcp` 暴露 `ask`、`whats_next`、`show_last`、`todo_add`、`todo_list`、`todo_update`：`ask`/`whats_next` 每次调用 spawn 现有 CLI 流程，复用 Popup、IM、抢答、历史和 drain；`show_last` 只读恢复最近精确问答，三个 Todo 工具按当前项目直接读写 `todos.json`，其中 list/update 使用稳定 Todo 与附件 ID。
- `ask` 入参为 message/questions/files；输出为 CLI 同款结果区块文本（原样透传，无 output schema / structuredContent）加图片 ImageContent。MCP 取消会终止子 CLI，并通过 socket EOF 取消 Daemon 请求。
- Agent 自动集成通常是 None/CLI/MCP 互斥；Grok 仅 None/MCP，其 MCP 产物是 skill + config；Pi 仅 None/CLI，其 CLI 产物是 Rules + 单个受管 Extension，自动集成 UI 不展示 MCP。
- Codex、Grok、Claude 分别写适配自身的长超时配置；Cursor MCP 超时不可配置，推荐 CLI 模式。
- Antigravity 在 Unix 支持 None/CLI/MCP，CLI 推荐、MCP 可选；共享规则位于
  `~/.gemini/config/rules/askhuman-antigravity.md`，MCP 配置位于
  `~/.gemini/config/mcp_config.json` 并写 `timeoutSeconds=86400`。Antigravity 没有可信原生
  session：CLI `show-last` 拒绝项目级回退，MCP 只能回退到 instance + project。Windows 集成
  变更明确不支持；MCP server 本身跨平台不等于 Antigravity 集成已获 Windows 验收。详见
  `docs/specs/antigravity.md`。
- MCP 没有通用的 Agent session 字段：Codex 使用每调用 `_meta.threadId`，Claude/Cursor
  使用 schema 隐藏的短命一次性 token，Grok 只在进程+项目分区内参数指纹候选唯一时认领；
  日常工具共用每调用绑定；`dev_ask` 使用单独的验收关联标识；无可信 session 时才回退到当前 MCP instance + project。详见
  `docs/plans/agent-context-compaction-retention.md`。
- Codex 配置把 `mcp__askhuman` 加入 Code Mode direct-only namespace，确保 ask 顶层阻塞；最小编辑与所有权记录避免卸载用户原有配置。
- Codex 委派运行（偏好「允许被委派的 Codex 运行使用 AskHuman」，默认关，`agents delegated codex [on|off]`）与未集成表现一致：
  `codex exec` / `review` 里 MCP 工具不出现（`AskHuman mcp` 启动时一次有界祖先探测，`tools/list` 为空），其余委派线程的
  MCP 调用、Stop 确认与权限审批按 SessionStart 写下的标记静默；CLI 不拦、会话登记不变。

## Agent 原生权限审批（Claude Code / Codex）

> 设计与实现计划见 `docs/plans/agent-permission-approval.md`。

- macOS、Linux 与 Windows 支持 Claude Code / Codex 的原生 `PermissionRequest` Hook；基础设施或渠道失败、24h 到期时不输出裁决，让 Agent 回到原生审批。
- 固定动作是批准一次/拒绝；Claude 只原样回放本次请求携带且通过白名单的 allow suggestion，Codex 永不伪造 `updatedPermissions`。
- 权限请求走独立 Confirm 模型；Popup/四 IM 中首个 Ready 的合法回答胜出，其它端定格，轻量 tombstone 只保留到原 deadline。
- permission 是默认开启的正交 preference，不是第四种 mode；CLI/MCP mode 按 preference 管理 Hook，None 卸 Hook 但保留偏好。
- “已配置”不等于已生效；blocked policy 和其它 Hook 只做可读范围内的正向提示。Claude 决策可能互相覆盖，Codex 保持 deny-wins。

## Agent Stop 结束确认

> 规格 `docs/specs/agent-stop-confirmation.md`；最终失败处理见 `docs/specs/agent-failure-confirmation.md`。

Cursor Desktop 另有默认关闭的实验性“Cursor 会话防护”（同一开关），含两个独立守卫：禁止后台完成后自动续跑，
以及 Agents 窗口里不再卸载运行中会话的消息（降低长首轮失败后被误归档的风险）。各自原生行为预检通过后才接入，
一个不可用时另一个照常，设置页显示“部分生效”。
由 Daemon 管理普通启动、窗口接入和还原，不依赖结束确认或逐会话结束状态。范围与验收真值见
`docs/specs/cursor-desktop-background-followup.md`（§7 为运行中会话保护）；终端 Cursor CLI 不在该能力范围内。

- 正常 Stop 处理 Claude Code、Codex、Cursor、Pi 的自然完成；同一「结束／出错时确认」开关另使整轮最终失败（`failed`，以及 Grok 因运行上限／没有进展停下的 `incomplete`）进入普通作答卡。有原会话通道时可要求继续，否则仅「知道了」（当作通知）。用户中断、工具错误、自动重试中静默；Grok 仅支持出错提醒，Antigravity 尚无可靠来源。
- Claude Code、Codex、Cursor、Grok 的各家开关默认关，Pi 默认开，已有偏好保留；自然完成询问“继续/结束”，24h 或基础设施失败 fail-open，投放沿用活跃槽 ∪ watch 渠道。错误卡也进入 iOS 普通收件箱，沿用其空闲推送门槛，不要求手机发起资格。
- Stop preference 独立保存，但只在 CLI/MCP mode 生效；关闭全量 lifecycle 时仍可安装必要的结果观察，错误卡由 daemon 持有。None 隐藏开关并删除共享 handler 与观察产物，重新集成时
  自动恢复。Active mode 内若只开 lifecycle，共享 Stop handler 只带 `track`、不弹确认卡。
- 错误卡是处理失败的唯一入口（见 `docs/specs/agent-failure-inbox.md`）：未处理的卡写入 `~/.askhuman/state/failure-cards.json`，跨 daemon 换新保留，且作为 daemon 自持的卡（`RequestEntry::held`）不计入排空判定。未处理错误与原生接续资格从检测起独立保留 24 小时，不因会话一小时 TTL 或普通 ended 历史容量被淘汰；卡本身不刷新会话活动，临时不可用保留明确重试，发送不明则防重放。Cursor 重试用尽后既不发 `stop` 也不发 `sessionEnd`，由 `agents/cursor_log.rs` 只读尾随它的结构化日志补上；该日志只作用于 hook 已登记的会话，从不启动或探测 Cursor。
- 错误后接续复用 `daemon/runtime/agent_control.rs` 的持久操作与实际开始回执；`agents/codex_cli.rs` 只读观察已知共享服务的最终失败，暂不向 CLI 队列投递。继续时使用各家原生 continuation；Pi Extension 在同一会话空闲且无排队消息时调用 `pi.sendUserMessage`。下一次自然 Stop 再询问，不做外部 resume。
- Stop 确认复用普通 Ask 单选链路并写入回复历史，已提交问答参与 `show_last`；它与 lifecycle 共用单一 Stop handler，避免并发 Hook 提前置空闲。
- `[user_confirmed_end_turn]` 出现在最后回复任意位置即表示用户已明确同意结束，命中后直接放行，避免再次确认。

## 接管 Claude 内置提问（AskUserQuestion）

> 规格 `docs/specs/claude-ask-user-question.md`。

- 只有 Claude Code 有内置选择题工具。`PreToolUse`（matcher 限定该工具）把题目交给 AskHuman 的多问题卡，作答后按官方 `allow` + `updatedInput.answers` 回传，终端不再弹原生选择框；headless 下同样可答。
- 独立 capability preference（`ask-question-preferences.json`，默认开），与 permission / stop 并列，同样只在 CLI/MCP mode 下安装 hook。
- 取消＝`deny` + 「必须重新询问」；弹窗与 IM 都不可用等基础设施失败则不输出，让位给原生选择框。
- 权限卡对该工具**一律让路**（不接管、不输出），与开关无关。

## 用户级 hooks + 弹窗提示音

- 用户 hook 位于 `~/.askhuman/hooks/<event>`；Unix 执行对应可执行脚本，Windows 支持 `.exe`、`.ps1`、`.cmd` 与 `.bat`。摘要走环境变量，完整负载走 stdin JSON，并以 10 秒边界和输出隔离执行。
- `ask-received` 在 Daemon 接收一次提问时恰触发一次，与是否显示 Popup 或投放 IM 无关。
- Daemon 会生成 `ask-received.sample`；Unix 复制去后缀并 `chmod +x`，Windows 改为受支持的扩展名即可启用，新增事件沿用同一机制。
- `general.popupSound` 是独立便利功能：macOS 使用系统音名，Linux 使用可用播放器，Windows 使用原生 `MessageBeep`；它与用户 hooks 相互独立。

## 构建 / 开发 / 测试

> 完整的依赖、Dev Instance、发布与平台说明见 `docs/development.md`。

```bash
pnpm install
pnpm tauri dev
pnpm build && cargo build --release --manifest-path src-tauri/Cargo.toml --features custom-protocol
cargo test --manifest-path src-tauri/Cargo.toml
./scripts/install.sh                    # macOS / Linux
.\scripts\install-windows.cmd           # Windows
node scripts/perf-popup.mjs             # 固定 canonical 弹窗性能场景
```

- 功能或逻辑变更后按项目规则运行安装脚本，再使用新安装的 `AskHuman` 验证。
- 本地 `install.sh` 使用独立的快速 `local-install` Cargo profile，并按前端输入指纹复用未变化的
  `dist/`；正式发布 / CI 仍使用 `release`。安装后按 profile 预算经 Cargo 协调回收 target，
  优先保留三方依赖；完整调试信息通过 `full-debug` profile 显式开启。详细说明见
  `docs/development.md`。
- macOS 安装使用稳定签名身份以维持钥匙串信任；证书与会话边界见 `docs/specs/secret-storage-keychain.md`。
- 性能 harness、埋点与基线见 `docs/specs/popup-launch-performance.md` 和 `docs/perf/baseline.json`。

## 注意事项

- **stdout 洁净**：GUI 阶段把 stderr 重定向到 /dev/null（`app/mod.rs` 的 `stderr_redirect`，Unix），自身错误用 `eprintln_real` 走原 stderr。
- **首帧不白闪**：`src/index.html` 内联关键底色；macOS 建窗 URL 携带有效材质，Solid 首帧完整实色，Blur/Glass 首帧透明叠色罩。
- **macOS 窗口材质**依赖 `tauri` 的 `macos-private-api` feature 与 `macOSPrivateApi: true`；运行时原生层变更必须在主线程执行。
- **release 自包含**：前端资源在 `cargo build` 时由 `generate_context!` 嵌入，故安装后无需 dev server。
- Telegram 不接收图片；各 Agent 是否提供特定 timeout/permission/stop Hook 由其自身 capability 决定，不再按 Windows 整体禁用。
