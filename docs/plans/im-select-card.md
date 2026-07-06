# 实现计划：通用「单选卡」+ /watch·/status·/unwatch 可点选

> **状态**：P1（飞书 MVP）P1-1..P1-4 已全部落地并 `install.sh` 落盘、425 单测全绿、daemon 已重启用上新二进制，
> **待真机验收**（见 PROGRESS.md）。P2 未开始。实现要点与触点见 PROGRESS.md 与 `docs/overview.md`「IM 单选卡」节。
>
> 设计见 `docs/specs/im-select-card.md`。本文件为落地顺序与触点清单。**MVP 只做飞书**；
> Telegram / Slack / 钉钉为后续批次（P2），接同一套 `select` 抽象。
>
> 关键约定（自 spec，务必遵守）：单击即触发（无二次确认）；选项 id＝session_id、按钮只带下标
> `select:<idx>`；仅**无参**命令弹卡（带编号/all 直达）；单选卡**不持久化**、过期点击**静默无效**
> （空 ACK）；飞书点选就地把单选卡编辑成目标卡（watch 卡 / 刷新后的单选卡）。

## P1-1 通用单选卡纯逻辑层（`select` 模块）

- [ ] `src-tauri/src/select.rs`（新模块，可单测）：
  - `SelectOption { id: String, label: String, badge: Option<String> }`。
  - `SelectView { title: String, options: Vec<SelectOption>, truncated_note: Option<String> }`
    （渲染器消费；本地化在组装期完成）。
  - `SELECT_MAX_OPTIONS: usize = 20`；`build_view(title, options)`：超上限截断 + 置
    `truncated_note`（「（仅列前 N 个）」）。
  - 命令种类标题构造（本地化）：`title_watch(lang) / title_status(lang) / title_unwatch(lang)`。
  - agent 选项文案组装：由注册表快照记录（`serde_json::Value`）→ `SelectOption`
    （label 复用 `autochannel::kind_title_project`，前缀状态圆点；`/watch` 场景传入「本渠道已关注
    session 集合」以决定是否加「· 关注中」徽标）。
- [ ] `i18n.rs`：`select.*`（zh/en）：`titleWatch` / `titleStatus` / `titleUnwatch` /
  `watchingBadge`（「· 关注中」）/ `truncated`（「（仅列前 {n} 个）」）/ `unwatchAllDoneCard`
  （单选卡定格「已全部取消关注」）。其余文本回复复用既有 `watch.*` / `autoChannel.*`。

## P1-2 飞书单选卡渲染 + 回调解析

- [ ] `feishu/card.rs`：
  - `build_select_card(&select::SelectView) -> Value`：标题（plain_text/markdown 头部）+ 逐选项一个
    `button`（`text`＝选项 label，`behaviors` callback `value = { "select": <idx> }`）；截断说明作
    灰色小字。空选项不应走到此（调用方保证）。
  - `SELECT_ACTION_KEY = "select"`；`parse_select_action(event) -> Option<(String /*open_message_id*/,
    usize /*idx*/)>`：读 `context.open_message_id` + `action.value.select`（对象或 JSON 字符串两种，
    与 `parse_watch_action` 同款容错）。
  - 单测：按钮 callback 值、下标解析、截断说明、与 watch 回调互不误解析。

## P1-3 daemon：picker 台账 + 路由 + 命令分支

- [ ] `ServerState` 增 `select: SelectState`：
  - `SelectState { pickers: Mutex<Vec<PickerEntry>>, routes: Mutex<HashMap<String, SelectRouteHandle>> }`
    （`SelectRouteHandle` 同 `WatchRouteHandle`：stop 信号 + Router Weak + 已注册 mid 集合）。
  - `PickerEntry { channel, message_id, kind: PickerKind, options: Vec<PickerOption>, created_at }`；
    `PickerOption { session_id: String, seq: u64 }`；`enum PickerKind { Watch, Status, Unwatch }`。
  - 台账治理：软上限（每渠道，如 10，超则丢最旧）+ TTL 兜底清理（如 30min，`created_at` 起算），
    在增/查时顺带清理；被消费即移除。**不持久化**。
- [ ] `ensure_select_routes(state)` + `ensure_select_route_for(state, config, channel, mids)`：
  与 `ensure_watch_routes` 同构（MVP 仅实现飞书臂）：飞书 `FsRouter::register()` → `set_active(Some(mid))`
  认领各 picker 消息 → 任务循环收 `FsInbound::Card { data, ack }` → `handle_select_card_action(&st, &data, ack).await`。
  - 调用点：每次**增/删/改** picker 后调一次；并在 `ensure_inbound_listeners` 末尾兜底调一次
    （随 Router 重建恢复）。**无周期 tick**——中途 Router 重连若丢路由，点击按 D7 静默无效（可接受）。
- [ ] `handle_select_card_action(state, data, ack)`（异步）：
  - `parse_select_action` 失败 / 找不到 picker（按 feishu + open_message_id）/ 下标越界 → `ack.send(None)`（静默，D7）。
  - 取 `opt = picker.options[idx]`，按 `picker.kind` 分派（详见 spec「点选后的处理」）：
    - **Watch**：定位 `opt.session_id` 当前记录 → 已结束/消失：ack 回**定格终态 watch 卡** + 移除 picker；
      否则：换新卡处理（本渠道同 session 旧订阅 → 编辑其卡为 `FinalKind::Replaced` 并退订）→ 上限校验
      （满且非换新 → ack None + 文本「已达上限」）→ ack 回 **Active watch 卡**（`callback_update_card`）→
      登记 `WatchEntry{ message_id = 本单选卡消息, session_id, seq, last_sig, … }` → 移除 picker →
      `persist_watch_subs` → `state.watch.notify` → `ensure_watch_routes` + `ensure_select_routes`。
    - **Status**：定位 `opt.session_id` 当前记录 → 组装纯文本详情（复用 `status_detail_text` 的活动解析，
      **按 session 定位**避免 seq 漂移；可加一个 `status_detail_by_session` 薄封装或先由 session 求当前 seq）→
      `reply_channel_text` → `ack.send(None)`（单选卡不动）。
    - **Unwatch**：本渠道找 `opt.session_id` 订阅 → 编辑其 watch 卡 `FinalKind::Cancelled` → 移除订阅 →
      `persist_watch_subs` → `state.watch.notify` + `ensure_watch_routes` → 回文本确认（`watch.unwatchDone`）→
      重算本渠道剩余订阅：有 → ack 回**新单选卡**（去掉该项）+ 更新 `picker.options`；空 → ack 回**定格卡**
      （无按钮 + `select.unwatchAllDoneCard`）+ 移除 picker。
- [ ] 复用重构：把 `handle_watch_cmd` 里「定位 rec / 换新卡 finalize / 上限校验 / 建帧 / 登记 WatchEntry」
  抽成 helper（如 `start_watch_core`），供**命令直达发新卡**与**单选卡回调就地变卡**共用（两条路径差异仅在
  「新消息 send」vs「经 ack 同步回卡到既有 message_id」）。
- [ ] `handle_inbound` 三处无参分支改推卡（其余分支不动，见 spec「命令行为」）：
  - `Command::Watch(None)`：有 工作中/空闲 agent → `send_select_card`（kind=Watch，标注本渠道已关注 session）+
    登记 picker + `ensure_select_routes`；无 agent → 既有 `status_text` 空态文本。
  - `Command::Status(None)`：保留既有切槽 + `activated_receipt`；随后有 agent → 发 Status 单选卡 + 登记 picker；
    无 → 既有 `status_text`。
  - `Command::Unwatch(Auto)`：0 → 既有「无关注」文本；1 → 直接取消（既有）；多 → 发 Unwatch 单选卡 + 登记 picker。
  - `Command::Watch(Some)` / `Status(Some)` / `Unwatch(One|All)`：**完全不变**（直达）。
- [ ] `send_select_card(channel, config, view) -> Result<message_id, _>`：MVP 仅飞书
  （`FeishuClient::send_card(build_select_card(view))`，取回 open_message_id）；非飞书 MVP 阶段回既有文本兜底。
- [ ] 闲退：不为 picker 加保活守卫（transient）；daemon 若空闲退出，旧卡点击按 D7 静默无效。

## P1-4 验证

- [ ] 单测：`select.rs`（截断 / 标题 / 选项文案 + 徽标）、`feishu/card.rs`（build/parse select）、
  daemon 分派（picker 查找、越界、kind 分派可测部分）。
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` 全绿 + `./scripts/install.sh`（用 AGENTS.md 要求
  的安装脚本编译进环境）。
- [ ] 真机（用户验收，飞书）：
  - `/watch` 无参 → 单选卡；点某 agent → 就地变实时 watch 卡；点已「· 关注中」的 → 旧卡定格「已由新卡片
    接替」、当前卡变新 watch 卡。
  - `/status` 无参 → 单选卡；点某 agent → 回文本详情、卡不动、可继续点。
  - `/unwatch` 无参（多个关注）→ 单选卡；点某 agent → 旧 watch 卡定格「已取消关注」+ 文本确认 + 单选卡去掉该项；
    取到 0 个 → 单选卡定格「已全部取消关注」。
  - 边界：无 agent 不弹卡（回文本）；`/watch 3`、`/unwatch all`、`/status 3` 仍直达；daemon 重启后点旧单选卡
    静默无效。
  > 注：命令处理与卡渲染都在 daemon，验收前需 `./scripts/install.sh` 后**重启 daemon** 用上新二进制。

## P2（后续批次，未开始）

- Telegram：`telegram/`（inline keyboard 渲染 + `callback_data=select:idx` 解析）+ `ensure_select_route_for`
  telegram 臂（`set_card_route`）+ `send_select_card` telegram 分支 + 点选 `editMessageText` 就地变 watch 卡。
- Slack：`slack/`（Block Kit actions 渲染 + `parse_select_action`）+ route slack 臂 + `chat.update` 变卡。
- 钉钉：与用户一起建**单选卡模板**（变量化选项/按钮列表 + `已选择 [n]` 定格变量）；点选**另发 watch 卡**
  + 定格单选卡（D4）。
- 可选：`/status` 点选改**卡内详情**（+刷新/返回按钮）。
