# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 待验收：通用「单选卡」+ /watch·/status·/unwatch 可点选（飞书 MVP 已落地）

用户诉求：三个命令无参时不再回纯文本编号列表（需再敲一次带编号命令），而是**推一张通用单选卡**列出可选
agent，点一下即执行。设计见 `docs/specs/im-select-card.md`（D1–D8），落地顺序 `docs/plans/im-select-card.md`。

**飞书 MVP 已实现并 `install.sh` 落盘、425 单测全绿**：
- `select.rs`（新，传输无关，无标记语言）：`SelectDot{Working,Idle}` / `SelectAction{Watch,Status,Unwatch}`（含
  本地化按钮文案）/ `SelectOption{id, dot, seq, primary(=类型·工作目录名), badge, secondary(=标题)}` / `SelectView` /
  `SELECT_MAX_OPTIONS=20` 截断 / 按命令种类本地化标题 / `agent_options`（工作中在前、跳过已结束、`· 关注中`徽标）+
  `agent_option_by_session`（unwatch 按订阅列举、记录消失时降级）。+ `i18n` `select.*`（含 `btnWatch/Status/Unwatch`）。
- `feishu/card.rs`：`build_select_card`（**用户定稿「方案A」**：每选项一行 `column_set` = 左侧小字号两行富文本
  ［第一行 markdown 彩色圆点`●` + `**[编号]**` + `类型·工作目录名` + `· 关注中`徽标；第二行灰色标题］+ 右侧紧凑
  `size:tiny` 按钮［watch=primary/status=default/unwatch=danger，文案随动作］，回调 `{select:<idx>}`，行间细分隔线）/
  `build_select_final_card`（unwatch 全取消定格）/ `parse_select_action`（读 `context.open_message_id`+`action.value.select`）。
- `daemon/mod.rs`：`SelectState{pickers,routes}` + `PickerEntry{channel,message_id,kind,options(session_id 快照),created_at}`
  + `PickerKind{Watch,Status,Unwatch}`；`register_picker`（TTL 30min + 每渠道软上限 10）/ `send_select_card`（四渠道）
  / `send_agent_picker`（空/渠道不支持回文本兜底）/ `ensure_select_routes`+`ensure_select_route_for`（复用 watch 路由句柄）
  / `handle_select_card_action`（解析→找 picker→按 kind 分派；过期/越界静默空 ACK）/ `select_pick_watch`（就地变 watch 卡，
  含 D8 换新卡：`register_watch_at` 抽出的换新卡收尾复用）/ `select_pick_unwatch`（旧卡定格 Cancelled+文本确认+就地刷新，
  取到 0 定格「已全部取消关注」）；`handle_inbound` 三处无参分支改推卡（带编号/all 仍直达）。
- **递归规避**：卡回调 recv-loop 内不再调 `ensure_select_routes`（会与 spawn 形成 `!Send` 递归）；watch 认领靠
  `register_watch_at` 的 `notify` → watch 引擎 `ensure_watch_routes`，残留 select 认领无害、下次 `send_agent_picker`/
  监听重建时收敛。

**未做（待你真机验收，飞书；daemon 已重启用上新二进制）**：
- `/watch` 无参→单选卡；点 agent→就地变实时 watch 卡；点「· 关注中」的→旧卡定格「已由新卡片接替」、本卡变新 watch 卡。
- `/status` 无参→单选卡；点 agent→回文本详情、卡不动可继续点。
- `/unwatch` 无参（≥2 关注）→单选卡；点 agent→旧卡定格「已取消关注」+文本确认+卡去掉该项；取到 0→卡定格「已全部取消关注」。
- 边界：无 agent 不弹卡（回文本）；`/watch 3`、`/status 3`、`/unwatch all` 仍直达；daemon 重启后点旧单选卡静默无效。

**P2 — 钉钉单选卡（✅ 已落地 + 真机点选验证通过）**：
- **模板**：`docs/assets/dingtalk-select-card-template.json` = 用户后台发布件（`Card[标题 BaseText, Loop[选项
  Markdown + 关注按钮 SingleButton + Divider], 定格 BaseText]`，按钮**单独成行**）。ID
  `43e7b261-997d-45de-ac5e-92e49d59cad8.schema`（＝`dingtalk/select.rs::DEFAULT_SELECT_CARD_TEMPLATE_ID`）。
  按钮回传 param `sid` 绑到**循环项字段** `loop_object_list[i].sid`（这条是钉钉侧唯一需人工确认点，已验证）。
- **变量契约**：全局 `title/btn_text/btn_color(blue|red)/finalized(bool→字符串)/final_label`；循环
  `loop_object_list[]{option_md(markdown), sid}`（复杂值→JSON 字符串下发，与提问卡 options 同规）。
- **代码**：`dingtalk/select.rs`：`option_md`（`<font sizeToken/colorTokenV2>` 两行**同 footnote 字号**、绿/灰
  圆点、`\n\n` 断行）/ `button_color`（watch·status=blue、unwatch=red）/ `build_select_param_map`(6 变量) /
  `build_select_final_param_map`(按 key 更新 finalized+final_label) / `parse_select_action`(读
  `content|value.cardPrivateData.actionIds=select` + `params.sid`) + 5 单测。i18n 加 `select.pickedCard`。
- **daemon 接线**：`send_select_card` 加 `dingding`（create_and_deliver + 自铸 `select-<uuid>`）；
  `ensure_select_route_for` 泛化飞书+钉钉（钉钉 recv-loop 先空 ACK 满 3s、再 `handle_select_dd_action`）；
  D4 不能就地变身 → `dd_select_pick_watch`（另发新 watch 卡 + `register_watch_at` 换新卡收尾 + 单选卡 OpenAPI
  定格「已选择 [n]」）/ `dd_select_pick_unwatch`（旧卡定格 + 文本 + OpenAPI 刷新 loop / 取 0 定格）/
  `dd_finalize_select_card`；status 点选＝回文本详情。
- **踩坑（已修，务必记住）**：`dingtalk/router.rs` reader 的卡回调门禁原来**只放行**「提问卡提交 /
  watch 按钮」两类、其余一律空 ACK 丢弃 → 单选卡点选静默无效。修法：门禁再加一条
  `select::parse_select_action(&data).is_none()`（即也放行 `actionId=select` 回调）。**以后钉钉再加新卡种
  务必同步更新这道门禁**，否则新回调会被悄悄吞掉。
- 真机日志确认：`select card sent … kind=Watch/Status` + `parse_select_action` 正确取回每项 `sid`；点「关注」
  →新 watch 卡 + 单选卡定格「已选择 [n]」。样式定稿：选项两行同 footnote 字号、按钮单独成行。

**P3 — Telegram / Slack 单选卡（✅ 已落地、full build + 单测全绿；⚠️ 尚未 install，待你确认后再装 + 真机验收）**：
- **就地编辑**：TG/Slack 与飞书同属「可就地变身」——点「关注」把这条单选卡消息本身编辑成实时 watch 卡
  （`WatchClient::edit`，非另发新卡；区别于钉钉的另发+定格）。
- `telegram/select.rs`：`render_select_html`（HTML 正文：标题 +每选项两行［🟢/⚪ 圆点 + `<b>[编号]</b>` +
  类型·目录 + 徽标 / 斜体标题］）/ `inline_keyboard`（每选项一枚按钮「<动作> [编号]」独占一行，
  `callback_data=sel:<idx>`）/ `render_select_final_html`（无按钮定格）/ `parse_select_action`（读 `sel:` 前缀→下标）
  +3 单测。用户定案布局＝「正文两行 + 每 agent 一枚按钮」。
- `slack/select.rs`：`build_select_blocks`（Block Kit：标题 section + 每选项一个 section＝两行 mrkdwn +
  右侧 button accessory；`action_id=select_<idx>` 保证整卡唯一，watch=primary/unwatch=danger）/
  `build_select_final_blocks`（section+context 定格）/ `parse_select_action`（读 `select_` 前缀→`(ts, 下标)`）+3 单测。
- **daemon 接线**：`send_select_card` 加 `telegram`（send_message 返回 message_id 转字符串）/ `slack`
  （open_dm + post_message 返回 ts）；`ensure_select_route_for` 补 TG（`set_card_route`，只认领卡回调不抢提问卡
  自由文字）/ Slack（`set_active(mid,"")` 只认领卡交互）两臂，复用 watch 的 `TgRouter`/`SlRouter` 与 `WatchChannelRouter`
  枚举；`handle_select_tg_action`（应答 callback 消转圈→解析下标→分派）/ `handle_select_slack_action`（ws 层已 ack→
  解析 (ts,下标)→分派）→共用 `dispatch_select_pick`（找 picker→按下标取 session_id→按 kind 分派）；点选实现
  `select_pick_watch_inplace`（就地变 watch 卡，含结束态定格 Ended、上限校验、换新卡收尾 `register_watch_at`）/
  status＝回文本详情、卡不动 / `select_pick_unwatch_inplace`（旧卡定格 Cancelled + 文本确认 + `refresh_select_card_edit`
  就地刷新 / 取 0 走 `finalize_select_card_edit` 定格「已全部取消关注」）。
- **待办**：install（用户确认后）+ 真机验收 TG/Slack 三命令点选（尤其就地变身与 unwatch 刷新）。
- 可选把 `/status` 点选改卡内详情（暂沿用回文本，四渠道一致）。

## 待验收：守护进程「保活模式」（实验 Tab）

在「实验」Tab 加**分段控件**（与状态栏图标一致）选 daemon 生命周期：`activity`（默认＝当前行为：按需拉起、5min 空闲退出）/ `keepalive`（保活）。已全量落地：
- **保活 = 装 daemon 登录项（开机自启，`~/Library/LaunchAgents/*.daemon.plist` `RunAtLoad`、无 `KeepAlive`；Linux `daemon start`）+ 空闲循环跳过退出（每轮 `load_without_secrets` 读一次）+ 打开开关即立即启动 daemon（宿主 `apply_config` 换挡即 `ensure_running`，一次触发）**。
- **关闭 = 卸登录项（仅删 plist/.desktop 文件、不 bootout 以免强杀）+ 让 daemon 按原 5min 空闲策略自然退出（策略不改）**。
- 登录项同步（`sync_daemon`）由 daemon 自身在 `serve()` 启动 + `on_config_changed` 幂等执行（宿主只管「立即起」）。
- 提示文案：保活可让 IM 随时收消息，但持续占少量资源 + IM 通道；多设备同时用建议配不同 IM 机器人。
- 触点：`config.rs`(DaemonLifecycleMode + general.daemonLifecycle + test) / `daemon/mod.rs`(idle 循环跳过 + `sync_daemon_login_item` 于 startup/on_config_changed) / `integrations/login_item.rs`(daemon 变体，file-only，+2 test) / `app/gui_host.rs`(HostState.daemon_lifecycle 跟踪 + apply_config 换挡即 ensure_running) / `types.ts` / `SettingsView.vue` / i18n zh+en。Unix only。
- **未做（待你验收）**：未真机端到端——开保活看 daemon 是否立即起且不再空闲退出、`~/Library/LaunchAgents` 是否落 plist、关保活后 plist 消失且 daemon 自然退出。验收前需用新二进制重启 daemon。

## 待验收：弹窗头部显示「提问时间」（相对时间，满一天转绝对）

`install.sh` 通过、vue-tsc 通过。在弹窗头部「Message from …（含胶囊）」之后加一枚灰色小字时间：
- **锚点**：提问创建时刻（epoch ms）。daemon 建请求时 `RequestRegistry::create()` 记录，经
  `ShowPayload.created_at_ms` → `PopupInit.createdAtMs` 透传；冷/单进程弹窗取弹窗构造时刻兜底；
  非弹窗窗口置 0。预热弹窗领用时得到的即为提问真正到达时刻（非热进程 spawn 时刻）。
- **显示**：`<5s 刚刚 / <60s N 秒前 / <60min N 分钟前 / <24h N 小时前`，满 24h 改绝对
  `toLocaleString()`（跟随系统）。前端每秒 tick 走字；hover(title) 给精确绝对时间。
- **窄窗**：`.brand-time` 给远高于标题/胶囊的收缩权重（flex-shrink 100000）+ overflow 裁剪，
  空间不足时**最先**被压没。
- 触点：`ipc.ShowPayload` / `daemon/request.rs` / `app::AppState`（7 处构造）/ `commands.popup_init`
  / `types.ts` / `PopupView.vue` / i18n `popup.time.*`。
- **未做**：未真机看弹窗（需重启 daemon 用上新二进制）——待你验收。

## 待验收：/status 当前活动（一期 transcript 尾部 + 二期 hook 实时工具 + 会话层斜线修复）

一期 `docs/plans/im-status-activity.md`、二期 `docs/plans/im-status-realtime-hook.md` 均已全量落地，
372 单测通过、`install.sh` 通过：

**一期（transcript 尾部）**：
- `registry`：`AgentRecord.seq` 稳定数字编号（daemon 生命周期内单调不复用、`load()` 重排），snapshot 暴露。
- `title.rs`：`transcript_path`/`find_file_recursive` 提为 `pub(super)` 供复用。
- `agents/activity.rs`（新）：尾部读取（256KiB）+ 四家解析 + 「永远给最后一段助手文字、末尾是工具调用再附工具」
  规则 + 工具归一化（仅 读/写/运行命令，其余原名+参数前段）+ 500 字截断 + `Activity.at`（transcript mtime）。
- `autochannel`：`Command::Status(Option<u64>)`；全局行 `[编号] 类型 — 标题（项目）`；`status_detail_text`
  头部 + 空行 +「最近动态（相对时间）」分区标签 + 文字 + `▸` 工具行；i18n。
- `daemon`：`handle_inbound` 按 `sel` 分派全局/详情。

**二期（hook 实时当前工具）**：Pre/PostToolUse hook 经 `report.rs`（解析 stdin 判 pre/post + `classify_tool`
归一化）随 `AgentEvent.tool` 上报；`registry.set/clear_current_tool` 存 `AgentRecord.current_tool`
（`serde(skip)` 不落盘、`snapshot()` 注入 `currentTool`；回合/会话结束清除）；`status_detail_text` 融合
「实时工具 vs transcript 尾部工具」取较新者、相对时间随之取实际展示事件时间——解决 Cursor「工具跑完才落盘」
滞后、覆盖编译等 in-flight 场景。

**会话层斜线修复**：作答期（armed 卡片）收到斜线命令时会话层不再回引导（`answer_inbound_reply` 返回
`Option`），统一交 `handle_inbound`（已注册→命令输出、未注册→help），四家 11 处调用点同步；`helpCmdStatus`
拆两行。

- **未做**：未 live 端到端实测（需真机发 IM `/status`/`/status <编号>` + 触发一次工具看实时工具）——待你验收。
  注意：IM 处理与 hook 上报都跑在 daemon，验收前需**重启 daemon** 用上新二进制。

## 待验收：Grok 集成（仅 MCP）—— 实现已落地并 live 验证

grok skill「不被模型主动加载」已修复并 install 落盘：根因是 skill 懒加载 + 自指悖论（按「需要提问时加载」写，
模型在最需要时意识不到需要它）。处置：把 skill 从「AskHuman 提问技能」重定位为「**无条件必读的交互协议**」——
frontmatter `description` 第一句无条件要求「每 session 先读」+ 前置兜底事实「普通输出人类不可见」；skill 更名
`askhuman` → `interaction-protocol`（`paths.rs` 目录同步）；`grok_skill_body()` 改为复用 `mcp_reference()` +
末尾一段**通用**「联系人类降级阶梯」（MCP 优先→没列出先搜→仍够不到退回其它提问渠道如 CLI，绝不退化成普通输出；
刻意不写死 harness/工具名）。已实测排除两条路线：`when-to-use` 仅拼在 description 后无价值、hook 无法注入
always-on 上下文（三证）。文档 overview.md / 调研 §6.3/§6.4/§7.2 已更新，全量 350 单测通过，install.sh + 落盘
`~/.grok/skills/interaction-protocol/SKILL.md` 已核对。**未做**：未 live 实测两 harness 首触发（避免 grok 子进程
再次卡住 shell）——待你需要时可 `grok inspect` 看 skill 是否加载。

计划 `docs/plans/grok-integration.md`（Q1–Q5 + P1/P2/P3 定案）已按序全量实现：
- P1（MCP 集成）：`AgentTarget::Grok` + `paths` grok 路径族 + `mcp_config`（三超时键
  `startup_timeout_sec=30`/`tool_timeout_sec=86400`/`tool_timeouts={ask=86400}`，且比较容忍整值浮点，
  顺带修好 Codex 因 CLI 归一化 `30→30.0` 造成的「永远需更新」）+ `grok_skill.rs` 指令载体
  （`prompts::grok_skill_body`，P2 措辞：找人一律走 `ask` MCP 工具、其它 shell 不受限）+
  `agent_mode` 两态（None|Mcp，拒 Cli）+ CLI(`agents mode/show/install`)/`doctor`/前端卡片(types/i18n)。
- P2（生命周期）：`AgentKind::Grok` 全链路（`mod`/`detect` 优先判 Grok/`registry`/`title` 解析
  `summary.json` 与解包 `<user_query>`）+ `agent_lifecycle` 原生 hook（`~/.grok/hooks/askhuman-lifecycle.json`，
  7 事件 Nested）+ `report.rs` P1 去重（`running==Grok && intended!=Grok` 跳过 claude/cursor 兼容 hook）。

Live 验证（`grok inspect`）：`askhuman` skill 已加载、MCP Server `askhuman(stdio) config` 已加载、
grok 原生 hook 已加载；同时确认 Grok 确会兼容读取 `~/.claude/CLAUDE.md`（P2 场景，靠 skill 措辞压制）
与触发 `~/.claude` 兼容 hook（P1 场景，靠 reporter 去重）。全部单测通过、`install.sh` 通过。

## 待办：install.sh 换新后 daemon 与 GUI 宿主「换新不同步」→ 旧 GUI 重建旧路径产物

现象（本轮 grok skill 改名 `askhuman`→`interaction-protocol` 时踩到）：`install.sh` 换二进制后 daemon 会自动
drain+重启到新版（`ASKHUMAN_DAEMON_AUTORESTART`），但 **GUI 宿主（`--gui-host` 菜单栏 app）有独立的二进制
换新监视（`gui_host.rs::start_binary_watch`/`maybe_refresh_binary`，每 15s，且仅在「无打开窗口」时才换）**，
可长时间滞留旧二进制（实测滞留 6h+）。分裂期内 **旧 GUI 按旧代码的产物路径反复重建托管产物**：删掉
`~/.grok/skills/askhuman` 后，每逢 daemon 重连/配置事件它又按旧路径补回（内容为旧版 `name: askhuman`），
即便 daemon 已是新版。手动退出并重开 app（GUI 切到新二进制）后复现消失，重启 daemon 回归验证通过。

风险点：任何「产物落点/命名变更」的发布，在用户 GUI 未及时换新前都可能被旧 GUI 以旧路径重建，产生「新旧两份
并存」。待评估修法：install.sh/daemon 换新时主动通知 GUI 宿主换新（而非仅靠其自身 15s+无窗口门控）；或让 GUI
换新不被「有窗口」长期阻塞；或产物 reconcile 统一由单一新二进制来源执行。

## 待验收：Codex app-server 共享 pid 隔离（生命周期追踪）

已实现并 `install.sh` 落盘，376 单测全绿。方案见 `docs/specs/agent-lifecycle-tracking.md`（D25/D26/D27 +
§8 源码+实测坐实 + 风险）与 `docs/plans/agent-lifecycle-tracking.md`（补丁节 P-1..P-4）；overview 生命周期章节已同步。

结论：新版 Codex TUI 经 UDS 连**长寿共享 app-server 守护**（reparent 到 PID 1、多 TUI 共用），hook/工具/MCP
都跑在 app-server 进程树内 → walk **永远拿不到 TUI pid**（源码：hook stdin/env 无 pid、握手 `ClientInfo` 无 pid、
UDS 不读对端凭证、落盘无 pid；实测：会话 rollout 由无 tty 的 app-server 持有、TUI 无 rollout/无 session_id）。
且 Codex **无** interrupt/关窗/SessionEnd hook（`Stop` 仅正常完成触发，Esc 打断走 `TurnAborted` 在 Stop 前返回）。

已落地（＝跟 Claude 无 pid / 被 scrub 时**同一路径**，不改状态机）：
- `agents/detect.rs`：`is_shared_app_server`（判据：命令行里 `codex` 令牌**紧邻下一个**为 `app-server` 子命令，
  覆盖 `node …/codex app-server …` 包装器；提示词里含 app-server 不误伤）；`walk_agent_pid(Codex)` 命中 app-server
  → `None`；`walk_any_agent` 跳过 app-server 节点。+3 单测。
- `agents/registry.rs`：状态机**无改动**——pid=None 天然跳过 D7 轮换与存活轮询，由 TTL + `working_backstop_sweep`
  治理；补模块注释。新增 `refresh_by_session_ids`（在途豁免 session_id 版）+1 单测。
- `daemon/request.rs`：`RequestEntry` 加 `agent_session_id` + `create` 填入 + `in_flight_agent_session_ids()`。
- `daemon/mod.rs`：tick 里 `refresh_by_pids` **并列** `refresh_by_session_ids`（无 pid 的 Codex/Claude-scrub
  等人回答>30min 也不掉空闲，与 Claude-有pid 完全一致）。

**未做（待你验收）**：未真机端到端——需重启 daemon 用上新二进制，对一个 app-server 模式的 Codex 会话触发一次
工具/提问，核对 `~/.askhuman/agents.json` 里该会话 `pid=null`、状态随 `Stop`→空闲与兜底超时正确流转、不再并发轮换误杀。

## 弹窗启动延迟性能优化（埋点 + harness + 基线 + 首轮 + 次轮 + 方案6 已落地；性能已暂停 → 远期余方案8/markdown-it）

文档：`docs/specs/popup-launch-performance.md`（调用链、等待点、优化方案、度量方法论 §7）。
harness 计划：`docs/plans/perf-harness-deterministic-mock-im.md`。
优化计划：`docs/plans/popup-launch-low-risk-optimization.md`（首轮 1/2/7）、`docs/plans/popup-launch-daemon-optimization.md`（次轮 3/4/5）。

**已完成：埋点 + 确定性 harness**（`ASKHUMAN_PERF` 门控默认关；`scripts/perf-popup.mjs` 无脑单命令：隔离 daemon
+ `ASKHUMAN_NO_KEYCHAIN=1` + 全 4 渠道 mock IM（`perf-mock-im.mjs`，建连/发送各注入 ~150ms 探针）+ 冷热双跑
+ 端到端 p90 ±20% 回归闸 + 锁屏/息屏守卫；基线 `docs/perf/baseline.json` 含 cold/warm）。

**已完成：首轮（方案1/2/7 + 支撑 S）** —— 前端侧：main.ts 不阻塞挂载、PopupView.onMounted 先取内容渲染、
Settings/History/Agents 异步组件、popup_init 作弹窗唯一非钥匙串配置源（弹窗路径零 `get_settings()`）；
附带 HistoryView 改用 `history_init.lang`，main.ts 自此零 IPC。

**已完成：次轮（方案3/4/5）** —— daemon/CLI 侧：
- 方案3 daemon 提前 spawn 弹窗（移到 Accepted 后、attach/inbound 前）→ WebView 初始化与 IM 建连并行。
- 方案4 attach/inbound 用 `any_im_enabled`(`load_without_secrets`) 门控，无启用 IM 时跳过 `AppConfig::load()`（零钥匙串）。
- 方案5(b) detect 移 daemon 异步：CLI 只读 env 家族/会话 + 上送 `caller_pid`；daemon spawn 弹窗后独立 task 从
  caller_pid walk 出家族/pid（MCP `walk_any` 兜底），经新 `ServerMsg::AgentResolved` 后推弹窗 badge（缓存 + 事件
  + 握手补发覆盖竞态）。badge 端到端验证通过（本仓 AskHuman 弹窗显 cursor 且可点 ↗）。

**当前基线**（`docs/perf/baseline.json`，次轮后 `--update-baseline` 刷新，屏幕解锁+唤醒+勿遮挡下采）：
- COLD 端到端 p90 ≈ **578ms**（首轮后为 ~1188）：方案3 让 `daemon recv→spawned` 466→1ms，~467ms IM 建连现与弹窗并行、不再进端到端。
- WARM 端到端 p90 ≈ **520ms**（首轮后 ~583）：大头仍是 `GUI total show→painted` ≈496（window visible ~250 + page boot ~435），即 WebView/页面加载固有冷成本。
- CLI `detect` 两路均 ~1ms（方案5：原 COLD ~39 / WARM ~27ms 的 ps 游走已离开 CLI）。

**余下（性能已暂停，远期）**：方案8 延后 show/骨架屏（改观感不减时长，热路径已并入方案6）、markdown-it 仅 `isMarkdown`
时按需懒加载（见 spec §4/§6）。

**已完成：方案6 弹窗预热（进程池）** —— daemon 预热 1 个 `--popup --warm` helper 隐藏待命，`dispatch_popup` 领用喂
`Show` 直接上屏、用后后台重建；默认开可关、非实验；并发第 2+/无显示/未就绪/drain 透明回退冷 spawn；热连接非保活、
idle/换新 `recycle_warm` 重补。关键修正：隐藏窗（ordered-out）rAF 不回调 → 改「领用时 `nextTick` 等正文进 DOM 后直接
后端 `popup_show_window` 上屏」（不依赖 rAF，息屏/锁屏也上屏）。macOS：待命期 helper 设 `Accessory`（不占 Dock/Cmd-Tab），
领用切 `Regular` 并**补设内置图标**（否则 Dock 显通用命令行图标）。三档基线（`docs/perf/baseline.json`）：**hot e2e p90 ≈161ms
vs warm 505（-68%）**、`show→painted` 476→135（-72%），cold/warm 无回归。视觉（无闪现/主题/回退）+ Dock 图标人眼确认 OK。
详见 `docs/specs/popup-prewarm.md`、`docs/plans/popup-prewarm.md`。

**待办**：headless 预热仅 Linux 可验（mac N/A）。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
