# 「按需发送」子开关：活跃槽切走时自动结束该渠道的 watch

> 痛点：开启「IM 渠道按需发送」（`channels.autoActivation`）后，人回到电脑作答时活跃槽会切回本地/别处，
> 但该 IM 上已建立的 `/watch` 实时关注**不会自动停止**，需要手动 `/unwatch`，容易忘、造成打扰。
>
> 目标：在「按需发送」下增加**子开关「自动结束 watch」**（默认开）。当某 IM 渠道**不再是活跃槽**
> （活跃槽从它切走——切到本地弹窗或别的 IM 都算）时，自动结束**该渠道**上的全部 watch，并把这些 watch
> 卡就地定格为「已切换到 XX · 自动结束关注」。

## 决策记录（用户经 AskHuman 定案）

- **D1 触发时机 = 活跃槽从某 IM「切走」**：不局限于「弹窗作答」。凡是 `set_active_channel` 使某个真实 IM
  渠道**不再是当前活跃槽**（切到 `popup`，或切到另一个 IM），就结束它的 watch。用户原话：「跟当前激活的
  Channel 切换走。如果主开关和子开关打开，如果这个 Channel 不是激活的 Channel 了，就结束它的 watch。」
- **D2 结束范围 = 仅「被切走的那个渠道」（prev 活跃槽）**：只结束刚失去活跃槽的那个渠道的 watch，
  **不动**其它渠道的 watch。
- **D3 watch 卡终态 = 新增专属终态**（与手动取消区分），动态文案 **「已切换到 XX · 自动结束关注」**，
  其中 `XX` = 新活跃槽的展示名（复用 `autochannel::channel_label`，含「本地弹窗」等）。
- **D4 不额外发文字回执**：只把 watch 卡定格即可；活跃槽切换本就会给旧渠道发「反激活提示」
  （`deactivated_receipt`），无需再发「已自动结束 N 个关注」文字。
- **D5 开关与默认**：子开关挂在「按需发送」下、**默认开**、**仅「按需发送」开时生效**。
  生效条件 = `autoActivation && autoEndWatch`。

## 触发点与行为

唯一改动挂在活跃槽切换的统一入口 `daemon/mod.rs::set_active_channel(state, new_id)`：

- 该函数已有一段「旧渠道反激活提示」，条件为 `old != "popup" && old != new_id`（即 `old` 是真实 IM 且确实
  切走）。**自动结束 watch 复用同一条件**，并额外要求 `cfg.channels.auto_activation && cfg.channels.auto_end_watch`。
- 命中后，对 `old` 渠道上的**每个** watch 订阅：
  1. `WatchClient::edit` 就地把卡片定格为 `FinalKind::AutoStopped(channel_label(new_id))`（失败仅记日志）；
  2. 从 `state.watch.subs` 移除该 `old` 渠道的订阅；
  3. `persist_watch_subs` + `state.watch.notify` + `ensure_watch_routes`（撤掉已无订阅的 watch 路由）。
- 上述逻辑与 `handle_unwatch_cmd` 的收尾几乎一致，抽一个共享 helper 复用（见计划）。
- **不发**任何额外文字（D4）；原「反激活提示」保持不变。

## 决策补记（真机复验后，用户经 AskHuman 定案）

- **D6 根因**：初版按 D2 只结束「prev 活跃槽」的 watch，但 **`/watch` 命令不调用 `set_active_channel`**
  （watch 与活跃槽解耦）。故被 watch 的渠道通常**根本不是**活跃槽（活跃槽多为上次弹窗作答留下的 `popup`），
  从来不会「从活跃槽被切走」→ 自动结束**永不触发**。用户在 Slack 发消息（prev=popup ≠ 飞书）时飞书 watch
  完全没被波及即此因。
- **D7 修复 = 「在某渠道操作即把它设为活跃槽」**（用户放弃放宽 D2，改为让 watch 渠道成为活跃槽）：新增
  `daemon/mod.rs::activate_channel_on_action`（`autoActivation` 开→`set_active_channel(本渠道)`，真切换才回
  激活回执），接到用户勾选的操作：**`/watch` 命令 + 三渠道单选卡「关注」点选 + 单选卡「查看」点选
  （补齐与 `/status` 文本命令一致）+ `/msg`·`/msg-clear` 插话**。**不**纳入：`/unwatch`、`/help`/未知命令、
  非文本消息（用户明确不选）。调用点统一放在**点选/命令输出之后**，避免激活的补推/回执拖慢飞书单选卡的同步 ACK。
- **D8 连带影响（用户已知悉并接受）**：`/watch 某渠道` 现在会：① 若旧活跃槽是别的 IM，给它发反激活提示、
  且（`autoEndWatch` 开时）结束它的 watch；② 把在途未答提问**补推**给新渠道（该渠道会同时有 watch 只读卡 +
  可作答提问卡）；③ 新渠道收激活回执。换言之「只远程 watch、人留在别处」的用法不再可行——watch 即激活。

## 已知局限与后果（用户已知悉）

- **「任何切走都触发」的副作用**：因触发点是活跃槽切换，下列场景也会结束旧 IM 的 watch——
  在另一个 IM 发 `/here` / 普通消息 / `/watch` / `/msg` 把活跃槽切过去；在另一个 IM 发 `/status` 且因此切槽。
  这是 D1 的直接后果，用户已确认接受。
- **无任何切槽动作时不会触发**：活跃槽只由入站消息 / 弹窗作答 / 上述操作改变。若人回到电脑时**恰好没有任何
  切槽动作**（没有弹窗可作答、也不在 IM/弹窗发起任何操作），活跃槽不变、不会触发自动结束。用户未要求补充
  其它触发信号（如「GUI 主窗获得焦点」），本期不做。

## 配置

- 新增 `channels.autoEndWatch: bool`，**默认 `true`**（序列化键 `autoEndWatch`）。
  - 缺省即 `true`：老配置文件缺该字段时按开处理（字段级 serde 默认 + `ChannelsConfig` 的 `Default` 需返回 `true`）。
  - 语义上是「按需发送」的子项：`autoActivation` 关时该值无效（不影响任何行为）。
- 前端 `SettingsView.vue` 实验区「IM 渠道按需发送」卡片内增加一枚**子开关**（缩进/次级样式），
  `autoActivation` 关闭时**置灰禁用**。`types.ts` 增字段，i18n zh/en 增标题与说明。

## 终态卡片文案

- 新增 i18n `watch.btnAutoStopped`：
  - zh：`已切换到 {to} · 自动结束关注`
  - en：`Auto-stopped (switched to {to})`（措辞待定稿时可调）
  - `{to}` = `autochannel::channel_label(new_id, lang)`。

## 测试

- `watch.rs`：`final_label_text` / `card_view` 对 `FinalKind::AutoStopped` 生成正确动态文案的单测。
- `config.rs`：`autoEndWatch` 默认 `true`、缺字段反序列化为 `true` 的单测。
- 逻辑层面（可选，视既有测试基建）：`set_active_channel` 在 `autoActivation && autoEndWatch` 且切走真实 IM 时，
  该渠道订阅被清空、其它渠道不受影响。
