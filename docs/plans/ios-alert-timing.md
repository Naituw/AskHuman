# 实现计划：iOS 通知时机（活跃槽解耦 + Mac 空闲后推送）

> 状态：**P1–P5 已完成，P6 的自动化部分已通过；真机一轮（2026-10-27）验证了一部分**（2026-10-26 批准并实现，待验项见 `docs/PROGRESS.md`）。需求与决策见 `docs/specs/ios-alert-timing.md`（D1–D11）。
> 地图：`docs/overview-ios.md`（Gateway 在 `src-tauri/src/ios/gateway/`，daemon 侧接线在 `src-tauri/src/daemon/runtime/ios.rs`）。

核心只有三处改动：**iOS 不再进活跃槽的候选筛选**、**Gateway 的推送从「立即」改成「待通知 → Mac 空闲到点再推」**、
**一个读 Mac 空闲时长的小模块**。iOS App、线协议、推送中继都不动。

## 总览

| 步 | 内容 | 主要文件 |
|---|---|---|
| P1 | Mac 空闲时钟 | `src-tauri/src/idle.rs`（新）、`main.rs` |
| P2 | 配置 + 设置页 | `config.rs`、`ios/gateway/mod.rs`（`GatewayState`）、`daemon/runtime/ios.rs`、`src/lib/types.ts`、`src/views/settings/IosChannelCard.vue`、`src/i18n/{zh,en}.ts` |
| P3 | iOS 脱离活跃槽 | `daemon/runtime/mod.rs`、`daemon/runtime/ios.rs` |
| P4 | Gateway：待通知与按空闲推送 | `ios/gateway/{mod,requests,testkit,tests_requests}.rs`、`daemon/runtime/ios.rs` |
| P5 | 文档同步 | 规格 / overview / wiki / PROGRESS |
| P6 | 验证 | 自动化 + 一轮真机 |

顺序 P1 → P2 → P3 → P4 → P5，每步能单独编译、测试、提交。**P3 之后、P4 之前**的中间态是「iOS 恒投递、仍立即推」，
等价于今天 `autoActivation = false` 的行为，安全。

## P1 — 空闲时钟

新文件 `src-tauri/src/idle.rs`，`main.rs` 加 `mod idle;`：

- `pub fn secs_since_last_input() -> Option<u64>`
  - macOS：FFI `CGEventSourceSecondsSinceLastEventType(state, event_type) -> f64`，`state = 0`
    （`kCGEventSourceStateCombinedSessionState`），`event_type = u32::MAX`（`kCGAnyInputEventType`）。非有限 / 负值 → `None`，
    否则取整。链接用 `#[link(name = "CoreGraphics", kind = "framework")]`（先例：`app/popup_canvas.rs`）。
  - 其他平台：`None`（文件注释写明 Windows 之后可用 `GetLastInputInfo`）。
- 测试：macOS 上返回 `Some` 且不 panic（CI 无图形会话时只断言不 panic）；其他平台返回 `None`。
- 提交：`feat(ios): read the Mac's keyboard and mouse idle time`。

## P2 — 配置与设置页

- `config.rs`：`IosChannelConfig` 加 `alert_after_secs: u64`（camelCase `alertAfterSecs`），`Default` 为 900；容器已是
  `#[serde(default)]`，旧配置缺字段回落 900。读取统一走一个 `fn alert_after_secs(&self) -> u64`（`min(86400)`），配置原样保存。
  - `ensure_ios_gateway` 只用 `ConnKey`（`daemon/runtime/ios.rs`，只含 host / 端口 / 账号 / 中继 / Mac 名等连接项）判断要不要换网关，
    **不要**把新字段加进去：改它不重连。
  - 测试：默认 900、缺字段回落、往返、超上限被钳制。
- `GatewayState`（`ios/gateway/mod.rs`，经 `IosResult::State` 到设置页与 `AskHuman channel ios status`）加
  `mac_idle_secs: Option<u64>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`；`state_view()` 在组装状态时
  经 `Host::mac_idle_secs()` 现读（`DaemonHost` 转到 `idle::secs_since_last_input()`）。**可选字段，不升
  `PROTOCOL_VERSION`**（`docs/overview.md`「IPC 协议号」表第一行：给已有结构体加带 `#[serde(default)]` / `Option` 的字段不 +1）。
  `channel ios status` 多打印一行 Mac 空闲时长，便于无界面排查。
- 前端：
  - `types.ts`：`IosChannelConfig.alertAfterSecs`、`IosGatewayState.macIdleSecs?`。
  - `IosChannelCard.vue`：在「连接」与「设备」之间加「推送时机」小节：下拉 `0 / 300 / 900 / 1800 / 3600` 秒；配置里是别的值
    （手改文件）就多显示一项「N 分钟（自定义）」，按大小插在预设之间。小节下方显示「Mac 已空闲 N 分钟」（来自卡片已有的状态轮询；
    `undefined` 时不显示；不满 1 分钟显示秒，不满 1 小时显示分钟，其余显示小时）。预设与时长拆分放在纯函数模块
    `iosAlertTiming.ts`（带单测）。
  - i18n `zh.ts` / `en.ts`：`settings.experimental.ios.{alertTimingTitle, alertAfter*, macIdle, duration.*}`。
  - `IosChannelCard.test.ts`：下拉渲染与写回、自定义值、空闲行、中文。
- 提交：`feat(ios,settings): add a push timing setting`。

## P3 — iOS 脱离活跃槽（`daemon/runtime`）

- `mod.rs::available_im_channels`：不再把 `"ios"` 放进 `available`（它只服务 IM 的活跃槽选择）。
  `select_im_delivery_candidates` 与「Popup 不可用且交集为空 → 兜底全部可用 IM」只看 IM，逻辑不动。
- 新增纯函数 `with_ios(candidates, ios_deliverable) -> Vec<&'static str>`：`ios_deliverable` 时追加 `"ios"`。
  `attach_im_channels` 与 `confirm_im_candidates` 在选完 IM 候选后调用它；`ios_deliverable = ios_can_deliver(state, config)`
  （`is_ios_active(config) && gateway.has_active_phone()`）。
  - `attach_im_channels` 里的 `candidates.is_empty() → return false` 在 iOS 并入后仍成立。
  - `backfill_inflight` 对已挂着 iOS 渠道的在途请求本就跳过（`has_channel` / `has_delivery`），无需改；它的 `ios` 分支保留
    （槽被别处设成 `ios` 的老路径）。
- `ios.rs::DaemonHost::slot()`：`auto_activation` 恒报 `false`（`active` 照旧）——现有 App 因此显示「每个提问都会发到这部手机」
  且不出接管按钮（唯一消费者是 `MacDetailScreen`）；**iOS App 不改**。
- 保留：`set_active_channel("ios")`（手机作答 / 新任务）仍会改 IM 的活跃槽，这是 IM 侧的规则，不动。
- 测试：`with_ios`（可投递 / 不可投递）；`autoActivation` 开 / 关 × 活跃槽为 `popup` / IM / `ios`，iOS 都在候选里；IM 候选与
  兜底的既有单测不变；`slot()` 恒 `autoActivation = false`。
- 提交：`feat(ios,daemon): deliver questions to phones regardless of the active slot`。

## P4 — Gateway：待通知与按空闲推送

`Host` trait（`ios/gateway/mod.rs`）加两个带默认实现的方法：

```rust
/// Seconds since the last keyboard / mouse input; `None` when unknown (then pushes go out at once).
fn mac_idle_secs(&self) -> Option<u64> { None }
/// Idle time after which an unseen request is pushed; `0` = push at once.
fn alert_after_secs(&self) -> u64 { 0 }
```

默认值保持「立即推」，既有测试一行不用改。`DaemonHost` 实现：`mac_idle_secs` 读 P1；`alert_after_secs` 读当前配置快照
（改设置立即生效，不重启网关）。`testkit` 的 host 增两个可设置字段。

`ios/gateway/requests.rs`：

「待通知」**不存标志位，按现状推导**：一道题对某部手机处于待通知，当且仅当
`!seen && !pushed && !is_looking(手机) && can_push(手机)`，其中 `is_looking = is_live && connected`。这样手机上下线、
broker 抖动、`request.seen` 之类的事件都不用逐个维护一个会漂移的状态——条件不成立，题自然就不在待通知里；
成立时由每 250 ms 的 `tick_requests` 与 `deliver()` 末尾检查是否该推（原先在计划里设想的 `notify_pending` 字段与
`phone_left_live` 钩子因此不需要）。

1. 把 `spawn_push` 开头「这部手机能不能推」的判断（有登记、未 `gone`、未过期、有中继地址与密钥）抽成 `can_push(phone_id, now)`；
   `spawn_push` 先过它。
2. `deliver()`：手机活跃且已连接 → 不变（`request.new`）。否则若 `can_push` → 只记 `reached = true`
   （**原来在这里立即推**）。题入表后调 `flush_due_notifications(now)`——Mac 已空闲 ≥ D、D = 0、空闲未知时仍是立即推。
   `reached` / `unreachable_since` 语义不变（有推送通道就算可达）。
3. `flush_due_notifications(now)`：先收集所有待通知的（手机，题）对，没有就直接返回（不读系统时钟）；否则读
   `alert_after_secs()` 与 `mac_idle_secs()`，`D > 0` 且空闲已知且 `idle < D` 时返回；其余情况对每一对调用现有
   `push_request` 并置 `delivery.pushed = true`（一次性，之后不再满足待通知条件，因此天然没有提醒）。
   借用写法沿用 `tick_requests` 的「先收集、后执行」。
4. `tick_requests()` 末尾调 `flush_due_notifications`。现有的 3 秒补推（前台手机没确认）、过期、不可达逻辑不动，
   补推仍然立即（手机前台但没收到是传输问题，与 Mac 在不在用无关）。
5. `on_request_seen` 与结算（答 / 取消 / 过期）路径**不用改**：`seen` 置位后不再满足待通知条件；结算时「`pushed && !seen`
   才发『已在 X 回答』」的现有判断不动（没推过就没有要替换的）。

测试（`tests_requests.rs`，虚拟时钟 + 可设置的空闲值），每条对应规格「通知规则」表的一行：

- 到达时手机活跃 → 只发 `request.new`、不推（Mac 空闲再久也不推）；
- 到达时手机不活跃、Mac 在用 → 不推；hello 快照里仍有这道题；
- 空闲推进到 D → 推一次；再推进任意时间 → 不再推（无提醒）；
- 到达时已空闲 ≥ D → 立即推；`D = 0` → 立即推；空闲未知（`None`）→ 立即推；
- 等待中途有输入（空闲回 0）→ D 重新计；
- 手机在 D 前变活跃 → 不推；已 `request.seen` 的题之后手机离开 → 不推；
- 手机活跃时到达、没打开就离开前台 → 回到待通知，空闲到点才推；
- 推前被别处答 → 无任何推送；推后被答且未打开 → 发替换推送（既有行为）；
- 两部手机各自独立；确认请求同样走新规则；
- 手机前台但 3 秒无确认 → 仍立即补推（与 D 无关）；
- 状态里的 `mac_idle_secs` 反映 host 读到的空闲时长（读不到为 `None`）。
- 没有推送登记（`can_push` 为假）→ 不进待通知、不推，题仍在收件箱里。

提交：`feat(ios): push only after the Mac has been idle`。

## P5 — 文档同步

批准后一并改（只改确实不准的地方，遵循 `docs/overview.md`「文档边界」）：

- `docs/specs/ios-channel.md`：D11 改为「iOS 不受活跃槽约束，见 `ios-alert-timing.md`」；§6.7 的「接管提问」入口、
  §6.10 的「接管提问」开关与活跃槽展示、§6.11 通知时机、§7 第 2 条（投递）与第 6 条（活跃槽）。
- `docs/overview-ios.md`：iOS 恒投递、推送由 Mac 空闲 ≥ D 触发、新模块 `idle.rs`、「待通知是推导出来的」不变量。
- `docs/overview.md`：模块地图加 `idle.rs` 一行；「iOS 渠道」一节加一条「投递与通知分开」不变量（iOS 不参与活跃槽）。
  IM 一节（「`channels.autoActivation` 关闭时向所有启用 IM 投放…」）本就只说 IM，不改。
- `docs/overview-configuration.md`：`channels.ios.alertAfterSecs`。
- `docs/plans/im-channel-activation.md`：加一句交叉引用。
- `docs/wiki/ios-setup.md` 与 `ios-setup.en.md`：「通知什么时候来」。
- `docs/PROGRESS.md`：加一节记真机待验项与「后续（本期不做）」——Follow 的去留 / 「绕过 D」、IM 统一（每渠道「提醒时机」）、
  Time Sensitive、送达回执 / 划掉追踪、合并通知、手机上展示 Mac 在用。
- 提交：`docs(ios): document push timing`。

## P6 — 验证

- 自动化：`cargo test --manifest-path src-tauri/Cargo.toml`（重点 `ios::` 与 `daemon::runtime`）、
  `npx vitest run src/views/settings`、clippy（按 `PROGRESS.md` 的 1.99 工具链口径）、`./scripts/install.sh`。
- 真机一轮（合并着做；能用 `devicectl` 的自己做，只在必须动手的地方——锁屏、看通知——叫用户）。设置页下拉没有 60 秒，
  测试时直接把配置文件里的 `channels.ios.alertAfterSecs` 临时改成 60（设置页会显示成「自定义」）：
  1. Mac 持续有输入、手机在后台，发一个提问 → **不**推；打开 App 能看到、能答；
  2. 手停键鼠超过 60 秒 → 推一次，点通知直达该题；再等 60 秒 → 不再推；
  3. 提问到达时 Mac 已空闲超过 60 秒 → 立即推；
  4. 推送后在 Mac 上答 → 通知被改写成「已在 Mac 回答」（**这一条同时验证 `collapse-id` 替换，此前没有真机证据**）；
  5. 手机在前台 → 只有应用内提示、不推；切到后台且没打开 → 空闲到点才推；
  6. 设为「立即」→ 回到旧行为。
- 结果写回规格的「已知局限」：哪些现在有了真机证据。

## 风险与回退

- 空闲检测失灵（被合成输入清零 / 读不到）的最坏结果是**不推送**，不会丢题；回退办法：设置里选「立即」。
- 第一条推送延迟最长 D；嫌慢调小 D。
- 整体回退：把 P4 的 `deliver()` 非活跃分支改回立即 `push_request`（并去掉 `flush_due_notifications`），等价于今天
  `autoActivation = false`；P3 的 `with_ios` 与 `slot()` 报值可独立回退。

## 影响面小结

- 新增 `idle.rs`（几十行）；改动 `config.rs`、`ios/gateway/{mod,requests,testkit,tests_requests}.rs`、
  `daemon/runtime/{mod,ios}.rs`、`src/lib/types.ts`、`IosChannelCard.vue` 与测试、`src/i18n/{zh,en}.ts`、docs。
- 无新增 IPC 消息，不升 `PROTOCOL_VERSION`（只给 `GatewayState` 加可选字段）；iOS 线协议与 `ios/Fixtures/wire-v1/` 不变；
  iOS App、推送中继不改。
- 所有 `feat` / `fix` 提交带 `Release-Note: skip`（沿用 iOS 渠道未公开的约定，见 `PROGRESS.md`）。
