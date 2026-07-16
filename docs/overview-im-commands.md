# IM 命令与主动交互概览

> 本文是 `docs/overview.md` 的专题补充，记录四种 IM 的共享命令模型和代码入口。卡片交互与功能边界以文中引用的 specs 为准。

## 全局模型

- 飞书、钉钉、Telegram、Slack 的长连接与 Router 由 Daemon 独占；每种渠道全局只保留一条连接，再按卡片或用户身份路由到请求。
- Daemon 存活期间持续接收 IM 入站消息，不要求当时已有提问。命令交给共享的 `autochannel` / daemon 处理，平台模块只提供收发、卡片渲染和回调解析。
- 同一提问可以投放到 Popup 和多个 IM，但仍由 Coordinator 保证首个终态回答生效并取消其它渠道。
- `/` 是通用命令前缀；Slack 客户端会拦截未注册 slash command，因此共享命令另支持 `!` 前缀，Slack 的提示文案默认展示 `!`。
- Agent 的数字编号来自 daemon 生命周期内稳定的 seq，供 `/status`、`/watch`、`/msg`、`/diff` 等命令共用。

## 活跃槽与按需发送

`channels.autoActivation` 默认关闭：关闭时，每次提问发送到所有已启用 IM；开启时，通常只投放到当前有效活跃槽。活跃槽也可以是 `popup`，并持久化于 `~/.askhuman/state/auto-channel.json`。Popup 不可用且「活跃槽 ∪ watch 渠道」选不出任何可用 IM 时，为避免零投递，该次提问会兜底发到所有可用 IM；人在其中某个渠道回复后，它会自然成为新活跃槽。

在某渠道发普通消息、回答问题或执行会改变交互上下文的命令，会把它设为活跃槽；切槽时把在途未答请求补推到新渠道。`channels.autoEndWatch` 开启后，切离某个真实 IM 时会结束该渠道的 watch。具体决策见 `docs/specs/im-auto-end-watch.md` 和相关实现计划。

正在 watch 某 Agent 的渠道会加入该 Agent 新提问的投放并集，因此按需发送不会让 watch 用户错过需要回答的问题。

## 命令地图

- `/new`：macOS 上依次选择最近 workspace、三重就绪的 Agent 和权限，并通过渠道原生输入控件提交任务；Slack 显示为 `!new`。提交后新的 Terminal.app 窗口运行交互式 Agent，来源渠道默认自动 watch 新会话。
- `/help`、`/?`：按当前配置和是否有在途问题生成可用命令与作答提示。
- `/here`：把当前 IM 设为活跃槽。
- `/status [编号]`：无编号查看 Agent 列表；带编号查看最近助手文字和当前/最近工具活动。
- `/watch [编号]`：创建或替换实时状态卡；无编号时可用单选卡选择 Agent。
- `/unwatch [编号|all]`：结束关注并定格旧卡；无编号且有多个订阅时可用单选卡选择。
- `/msg <编号> <内容>`：给工作中的非 Grok Agent 排队插话；`/msg <内容>` 可按关注关系直发或弹出目标选择卡。
- `/msg <编号>`：查看待送达内容；`/msg-clear <编号>` 撤回待送达内容。
- `/diff [编号]`：导出目标 workspace 的 unstaged 与 untracked 变更摘要和附件。
- `/stage [编号]`：显示变更确认卡，确认后执行 `git add -A`；不会绕过 Confirm 直接暂存。
- `/transcript [编号]`：按渠道适配的附件格式导出完整会话。
- `/detect`：渠道配置识别流程使用的临时命令；成功后只回执已填字段，不回显完整 ID。

命令支持中文别名的部分由 `autochannel` 统一分类；平台特有前缀、附件格式和卡片能力在传输层适配。

## Watch

Watch 规格见 `docs/specs/im-watch.md`。订阅持久化在 `~/.askhuman/state/watch.json`，daemon 重启后尽量继续编辑原卡。状态卡展示 Agent 工作中、空闲、等待回答或已结束，以及跨 Turn 累计、扣除真正 idle 的工作时长和最近活动；状态变化、提问和回答会唤醒引擎。已在关注的 Agent 进入空闲后保留 5 分钟宽限，期间开始下一 Turn 可由原卡继续。

引擎按结构化帧签名避免无变化编辑，并按平台限制节流。Agent 结束、用户取消、自动切槽或连续发送失败都会让订阅进入终态并释放。共享状态在 `watch.rs`，平台渲染和回调在各自 `<channel>/watch.rs` 与 Router 中。

## 单选卡、Confirm 与导出

无参命令需要选择 Agent 时复用 `select.rs` 的传输无关模型；选项以 session id 为稳定身份，展示 daemon seq。飞书、Telegram、Slack 可就地刷新或把选择卡变成 watch 卡；钉钉受模板能力限制，选择后可能另发目标卡。详细协议见 `docs/specs/im-select-card.md`。

`/stage` 复用 `confirm/` 的跨渠道双动作展示与传输，但 Git 指纹、slot 到业务动作的映射和 `git add -A` 仍由 daemon 台账负责。diff/transcript 内容分别由 `gitutil.rs`、`agents/transcript_full.rs` 生成，再由 `export/` 转成渠道支持的附件格式。详细协议见 `docs/specs/im-diff-stage-transcript.md`。

`/msg` 的目标选择复用同一单选卡模型，实际队列由 `agents/interject.rs` 管理；插话能力见 `docs/specs/agent-interject.md`。

`/new` 的 workspace / Agent / 权限步骤也复用单选卡；最终任务输入复用结构化 Confirm 的渠道原生
输入能力，但只投放到命令来源渠道。启动参数不经过 shell：IM 数据进入一次性 `0600` LaunchRecord，
Terminal shell 只接收 AskHuman 绝对路径和 UUID token。详细边界见 `docs/specs/im-agent-task-launch.md`。

## 主要代码入口

- `src-tauri/src/autochannel.rs`：命令分类、帮助文案、活跃槽与共享回复文案。
- `src-tauri/src/daemon/mod.rs`：入站监听、命令分派、补推、watch/select/confirm 台账。
- `src-tauri/src/watch.rs`、`select.rs`：传输无关状态与视图模型。
- `src-tauri/src/agents/workspaces.rs`、`integrations/agent_launch.rs`：workspace 索引、readiness 与安全终端启动。
- `src-tauri/src/gitutil.rs`、`confirm/`、`export/`：Git 操作、确认和附件导出。
- `src-tauri/src/{telegram,dingtalk,feishu,slack}/`：平台传输、渲染、回调与路由。
