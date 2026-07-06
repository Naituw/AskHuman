# 通用「单选卡」+ /watch·/status·/unwatch 可点选（IM）

> IM 里 `/watch`、`/status`、`/unwatch` **无参**时，不再回一段纯文本编号列表（需用户肉眼找编号、再
> 重敲一次带编号的命令），而是**推一张「单选卡」**：卡上把可选 agent 列成一组**可点按钮**，用户
> **单击某个 agent 即触发**对应动作（开始 watch / 查看 status / 取消 unwatch）。
>
> 「单选卡」被设计为一个**与具体命令无关的通用组件**（用户明确要求：后续还有别的命令会复用同一张
> 卡）。命令侧只提供「选项列表 + 选中后做什么」，卡片渲染 / 点击回调路由是共享的。
>
> **分期**：飞书先做完整 MVP；Telegram / Slack / 钉钉后续批次接同一套抽象（钉钉因卡片模板绑定有
> 特殊处理，见「渠道差异」）。

## 决策记录（用户经 AskHuman 定案）

- **D1 通用单选卡**：新增一个**传输无关**的「单选卡」抽象（不是 watch 专用）。一张卡 =
  `标题 + 一组选项{稳定 id, 展示文案, 可选状态徽标}`；每渠道一个渲染器把选项渲染成按钮；
  新增**一类 `select` 点击回调**按「卡片 message_id + 选项下标」路由回 daemon。命令侧只负责
  「给出选项列表」+「定义选中后做什么」。与既有「提问-作答卡」（多选/单选 + 提交）**是两套东西，不混用**。
- **D2 交互模型＝单击即触发**：单选卡点一下**立即执行**，没有「选中再点确认」的二次步骤。
- **D3 选项身份用稳定键**：选项 id 用 **agent 的 session_id**（不是会漂移的展示编号 seq）。
  卡片存下当次快照的 (session_id, seq) 列表，按钮只带**下标**（`select:<idx>`）以规避 Telegram
  `callback_data ≤ 64 字节` 限制；点击时 `下标 → 该选项的 session_id → 动作`。
- **D4 点选后卡片流转（按渠道能力分档）**：
  - **飞书 / Telegram / Slack**：「编辑消息」可**自由重写内容** → `/watch` 点选后**就地把这张
    单选卡编辑成实时 watch 卡**（复用同一条消息，零多余消息）。
  - **钉钉**：卡片实例在**创建时绑定了模板 ID**，之后只能更新模板变量、**不能换成另一个模板**
    → 做不到「单选卡就地变 watch 卡」。钉钉的做法：点选后**另发一张 watch 卡** + 把单选卡用一个
    变量**定格成「已选择 [n]」**。此项待与钉钉模板一起落地（MVP 不含钉钉）。
- **D5 三命令的点选结果**：
  - `/watch` 点选 → 就地变实时 watch 卡（飞书；重复关注同一 agent 复用既有「换新卡」语义，见 D8）。
  - `/status` 点选 → **回一条纯文本详情**（即既有 `/status <编号>` 的详情），**单选卡保持不动、可继续点**。
    （后续可再优化为卡内详情 + 刷新/返回按钮。）
  - `/unwatch` 点选 → 取消该关注（旧 watch 卡定格「已取消关注」）+ **回一条文本确认** +
    **就地刷新单选卡**（移除刚取消的选项；无剩余则把单选卡定格为「已全部取消关注」）。
- **D6 触发边界（卡 vs 直达）**：**仅无参命令弹卡**——`/watch`、`/status`、`/unwatch`（且 unwatch
  有**多个**关注时）。**带编号 / all 的命令仍直达执行**（`/watch 3`、`/status 3`、`/unwatch 5`、
  `/unwatch all` 一律不弹卡，行为不变）。
- **D7 过期策略＝不持久化 + 静默无效**：单选卡是**一次性选择器，不持久化**。daemon 重启（或单选卡
  被清理）后，旧卡的路由不复存在，点击其按钮**静默无效**（飞书回**空 ACK**、卡片无变化）。理由：
  重启少见，且被关注 agent 的 watch 订阅本就各自持久化、重启后多半已有自己的实时卡，影响很小；要做到
  「点了回一句『已过期』」需给孤儿回调加兜底路由，性价比低。
- **D8 已在关注中的 agent**：`/watch` 单选卡**照常列出**已在本渠道关注中的 agent，加一枚
  「· 关注中」徽标；点它＝**等价于重新 `/watch <相同编号>`**——复用既有「换新卡」语义：旧 watch 卡
  定格「已由新卡片接替」（`FinalKind::Replaced`），当前单选卡就地变成新的实时 watch 卡。

## 通用单选卡（`select` 模块，传输无关）

一张单选卡的抽象数据：

- `SelectPrompt`：卡片**标题/提示**（本地化，按命令种类不同：见下）。
- `SelectOption { id, label, badge? }`：
  - `id`：**稳定标识**（agent 场景＝session_id）。用于点击后定位领域对象。
  - `label`：按钮展示文案（agent 场景＝`[编号] 类型 — 标题（项目）`，见「选项文案」）。
  - `badge`：可选的状态/标注小字（agent 场景＝状态圆点 🟢/⚪ 前缀 + 可选「· 关注中」）。
- 渲染：每渠道一个渲染器把「标题 + 选项列表」渲染成**一组按钮**（每个选项一个按钮，单击回调）。
  - 按钮回调 value 命名空间：**`select`**（飞书 `{ "select": <idx> }`；与 watch 卡的 `{ "watch": … }`
    不冲突，且卡片按 message_id 精确路由、一张消息非 watch 即 select，天然可辨）。
  - 选项数上限：单卡最多渲染 **`SELECT_MAX_OPTIONS`（暂定 20）** 个；超出则截断并在标题追加
    「（仅列前 N 个）」。日常 agent 数一般 < 10，MVP 不做分页。

「单选卡」**不做实时刷新**（不像 watch 卡有引擎/签名/tick）：它是某一刻的快照，只在**用户点击**时
改变（watch 点选把它变成 watch 卡；status 点选不动它；unwatch 点选就地刷新剩余选项）。

### 选项文案（agent 场景）

- 单行：`[编号] 类型 — 标题（项目）`，与 `/status` 文本列表单行同源（复用
  `autochannel::kind_title_project`）。
- 徽标（可选，作为 label 前缀或 badge）：状态圆点 `🟢 工作中 / ⚪ 空闲`；`/watch` 卡里已在本渠道
  关注中的追加「· 关注中」（D8）。
- 排序：工作中在前、空闲在后（与 `status_text` 一致）。

### 命令种类 → 标题 / 选项来源 / 选中动作

| 命令（无参） | 卡标题（本地化） | 选项来源 | 选项 id | 单击动作 |
|---|---|---|---|---|
| `/watch` | 「选择要实时关注的 Agent（点一下即开始）」 | 快照里 工作中 + 空闲 的 agent | session_id | 就地变实时 watch 卡（D5/D8） |
| `/status` | 「选择要查看的 Agent」 | 快照里 工作中 + 空闲 的 agent | session_id | 回纯文本详情（卡不动） |
| `/unwatch` | 「选择要取消关注的 Agent」 | **本渠道**当前 watch 订阅 | session_id | 取消 + 文本确认 + 就地刷新卡 |

## 命令行为（改动点）

无参命令的分派改动（`daemon::handle_inbound`）：其余命令（带编号 / all、`/here`、`/help`、
普通文本）**完全不变**。

- **`/watch`（无参）**：
  - 无可选 agent（快照里无 工作中/空闲）→ 维持既有**文本**提示（`autochannel::status_text` 的空态：
    「需开启生命周期追踪」），不发卡。
  - 有 agent → 发**单选卡**（kind=Watch，选项＝工作中+空闲；已在本渠道关注中的加「· 关注中」徽标）。
    登记一条 picker（见「daemon 侧」）。**不再**附加旧版的「正在关注」文本段（改由徽标表达）。
  - `/watch` 命令本身**不切活跃槽**（与现状一致）。
- **`/status`（无参）**：
  - 保持既有「按需发送开启且因此切了槽 → 先回激活回执文本」的行为（`set_active_channel` +
    `activated_receipt`）；随后：
    - 无可选 agent → 回既有 `status_text` 空态文本，不发卡。
    - 有 agent → 发**单选卡**（kind=Status，选项＝工作中+空闲）。登记 picker。
- **`/unwatch`（无参，即 `WatchSel::Auto`）**：
  - 本渠道订阅 0 个 → 回既有「无关注」文本。
  - 恰 1 个 → **直接取消它**（既有行为，不发卡）。
  - **多个 → 发单选卡**（kind=Unwatch，选项＝本渠道各订阅），替代旧版的「回列表让用户指定编号」文本。
    登记 picker。

## 点选后的处理（`select` 回调 → daemon）

飞书回调经既有卡路由机制（`FsRouter` 按 `open_message_id` 精确路由 + **oneshot 同步回卡**）。
新增 `handle_select_card_action`（异步）：解析 `(open_message_id, idx)` → 按 message_id 找 picker →
取 `options[idx]`（越界 / 无 picker → **空 ACK**，静默，即 D7）→ 按 `PickerKind` 分派：

- **Watch**：
  1. 由 `option.session_id` 在当前快照定位记录（配合当前 seq）。记录**已消失/已结束** → `ack` 直接
     回一张**定格终态 watch 卡**（`FinalKind::Ended`），**不订阅**、移除 picker（等价既有
     `handle_watch_cmd` 对 ended agent 的处理）。
  2. **本渠道已在关注同一 session** → 复用既有「换新卡」：把那条旧订阅的卡**定格为
     `FinalKind::Replaced`**（编辑旧消息）并退订（D8）。
  3. 关注上限校验（`watch::MAX_WATCHES`，每渠道；换新卡不计新增）。达上限且非换新 → 空 ACK +
     回文本「已达上限」，保留单选卡。
  4. 否则：`ack` 回**实时 watch 卡**（`CardMode::Active`，`callback_update_card`）→ **这条单选卡消息
     就地变成 watch 卡**；随后登记一条 `WatchEntry`（**message_id = 该单选卡消息**、session_id、seq、
     首帧签名…）、移除 picker、`persist_watch_subs`、`notify` watch 引擎、`ensure_watch_routes`
     （让 watch 路由认领这条消息的后续按钮回调）+ `ensure_select_routes`（撤掉已消费的 picker 路由）。
     > 落地上应把 `handle_watch_cmd` 里「定位/换新/上限/建卡/登记订阅」的核心抽成可复用 helper，
     > 供「发新卡（命令直达）」与「就地变卡（单选卡回调，经 ack 回卡）」两条路径共用。
- **Status**：由 `option.session_id` 定位当前记录 → 渲染**纯文本详情**（复用
  `autochannel::status_detail_text` 的解析；按 session 定位以避免 seq 漂移）→ `reply_channel_text` 发文本
  → `ack` 空 ACK（**单选卡保持不动**，可继续点其它 agent）。
- **Unwatch**：
  1. 本渠道找 `session_id` 对应订阅：编辑其 watch 卡**定格 `FinalKind::Cancelled`** → 移除订阅 →
     `persist_watch_subs` → `notify` watch + `ensure_watch_routes`。回文本确认（复用
     `watch.unwatchDone`）。
  2. **就地刷新单选卡**：重算本渠道剩余订阅 → 若仍有 → `ack` 回**新的单选卡**（去掉刚取消项）并更新
     picker 的选项快照；若已空 → `ack` 回一张**定格卡**（无按钮 + 文案「已全部取消关注」）并移除 picker。

## 渠道差异（同一 `select` 抽象，各渠道渲染器 + 传输）

| 维度 | 飞书（MVP） | Telegram（后续） | Slack（后续） | 钉钉（后续，需模板） |
|---|---|---|---|---|
| 载体 | 卡片 JSON 2.0，每选项一个 `button`（callback `{select:idx}`） | inline keyboard（每选项一行按钮，`callback_data=select:idx`） | Block Kit `actions`（button，`action_id=select`、`value=idx`；单块 ≤25） | 互动卡片高级版**专用模板**（变量含选项列表/按钮 — 需新建） |
| 点选就地变 watch 卡 | ✅ oneshot 同步回卡（消息内容自由重写） | ✅ `editMessageText` 重写 | ✅ `chat.update` 重写 | ❌ 模板绑定 → **另发 watch 卡 + 单选卡定格「已选择 [n]」** |
| 回调路由 | `FsRouter` 卡路由（按 open_message_id）+ oneshot | `TgRouter::set_card_route`（仅卡回调） | `SlRouter` 按 message_ts | `DdRouter` 按 outTrackId |
| 过期点击 | 空 ACK 静默（D7） | callback answer 后无变化 | 无变化 | 空 ACK |

传输抽象：复用 watch 已有的每渠道「卡回调路由任务」骨架（`ensure_watch_routes` / `WatchRouterRef` /
`is_same_alive`）**同构**一份 `ensure_select_routes`（MVP 只实现飞书臂）。选项按钮的构建与回调解析
各渠道各写一份（飞书 `feishu/card.rs::{build_select_card, parse_select_action}`；后续 TG/Slack/钉钉
各自模块）。

## 边界与降级

- **无 agent**：不发卡，沿用既有文本空态提示。
- **点到已结束/消失的 agent**：watch→定格终态卡；status→详情显示「已结束」；unwatch→该项本就来自订阅、
  取消即可。
- **daemon 重启 / picker 被清理后点旧卡**：静默无效（D7）。
- **picker 台账治理**：不持久化；内存里每渠道保留有界（软上限 + TTL 兜底清理，避免长期累积；被消费
  即移除）。
- **Slack `/` 拦截**：命令仍用 `!watch` 等 `!` 前缀发起；单选卡本身（Block Kit 按钮）不受影响。
- **活跃槽**：发单选卡、点选回调都**不额外改活跃槽**（`/status` 命令本身既有的切槽逻辑保留）。
- **「跟底/淹没」**：发单选卡是一条机器人消息，与既有「状态文本回执」同性质，沿用既有
  `mark_watch_disturbed`（在 `handle_inbound` 入口已标记），无需特殊处理。

## 复用与新增触点（细节见 plan）

- **复用**：`autochannel::{status_text, status_detail_text, kind_title_project, cmd_prefix}`；
  `watch::{build_frame, card_view, signature, MAX_WATCHES, FinalKind, CardMode, WatchPhase}`；
  `feishu::card::{build_watch_card, callback_update_card}`；`FsRouter::register` + 卡路由；
  watch 的 `WatchEntry` 登记 / `persist_watch_subs` / `ensure_watch_routes` / `handle_watch_cmd`
  的核心（抽 helper）。
- **新增**：`src-tauri/src/select.rs`（`SelectOption` / `SelectPrompt` / 本地化标题 + 选项文案组装）、
  `feishu/card.rs::{build_select_card, parse_select_action}`、daemon `SelectState`（pickers 台账 +
  select 路由表）+ `handle_select_card_action` + `ensure_select_routes` + 三命令无参分支改推卡、
  i18n `select.*`。

## 后续

- Telegram / Slack 各自的单选卡渲染 + 回调（同抽象）。
- 钉钉：与用户一起建「单选卡」模板（支持变量化的选项/按钮列表 + `已选择 [n]` 定格变量）。
- 可选优化：`/status` 点选改为**卡内详情**（+刷新/返回列表按钮），而非纯文本。
- 复用扩展：其它未来命令接入同一 `select` 抽象（新增一个 `PickerKind` + 其选中动作即可）。
