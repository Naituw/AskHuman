# 计划：去掉手动关注，「做完 / 出错」通知并入「Mac 空闲才推」

> 状态：**已批准；P1–P3 已实现，P4 文档已回写，只剩可选的真机一轮**（2026-10-27）。
> P1–P2（Mac 端）已装机：Rust 全量 1759 项、clippy 1.94 / 1.99、fmt 全绿；新增 14 个网关测试，另把 2 个「在等你」用例换成 1 个反向用例；
> 并对「回到 Mac 作废 / Agent 又开始工作作废 / 会话结束作废 / 一次性 / 手机在看」几条规则做过变异检查——破坏规则后对应测试确实失败。
> P3（App）：Swift 包单元与进程内集成全量（`swift test`，AHCore 107 / AHApp 113，含 `AHAppTests`）；对已装机真实 daemon + 本地 broker 的端到端先全量跑过一遍（33 / 34，
> 1 个与本需求无关的陈旧断言已修），这次改过的几条（含新增一条）改完后重跑通过；模拟器 UI 测试：合并后的树 en 50 / 50，zh-Hans 在合并前全量跑过（改动涉及的 3 条改完预期文案后 en / zh 都通过）；
> 模拟器上真 App 对真实 EMQX + 中继 + APNs 沙盒的完整场景全过（214 秒）；`Localizable.xcstrings` 重新生成后无缺译、无多余键、再生成无差异。
> 做法上的偏差：P3 在 `/tmp` 里的隔离副本中开发和验证（另一个 iOS 会话当时有未提交的并行改动，也在占用模拟器），验证完再作为补丁合进主树。
> 需求与决策：`docs/specs/ios-follow-narrowing.md`。
> 地图：`docs/overview-ios.md`。前置：`docs/plans/ios-alert-timing.md`（已实现，本计划复用它的 `is_looking` / `can_push` /
> `alert_after_secs` / `mac_idle_secs` 与测试台）。

## 阶段

| 阶段 | 内容 | 主要文件 | 验证 |
|---|---|---|---|
| P1 | Mac：待通知队列；作废 / 送达规则；D 规则；一次性移出 | `ios/gateway/agents.rs`、`mod.rs`（`tick`）、`testkit.rs`、`tests_agents.rs` | 网关单测（虚拟时钟）、clippy、fmt |
| P2 | Mac：删「在等你」推送与 `tick_waiting` 死代码 | `agents.rs`、`tests_agents.rs` | 单测、clippy |
| P3 | iOS App：去星标 / 过滤 / 名额条；只读铃铛；文案；UI 测试 | `ios/AskHuman/Views/…`、`AHCore`、`Localizable.xcstrings`、`ios/**/Tests` | `swift test`、`xcodebuild test`（模拟器） |
| P4 | 文档回写、`./scripts/install.sh` 装机、真机一轮 | 需求头部列出的文档 | 用户在场 |

- P1–P2 只动 Mac（Rust）：线协议、`PROTOCOL_VERSION`、夹具、中继、`AppConfig` 都不变，**不需要**动 IPC。装机后旧版 App（仍有星标）也按新规则走。
- P3 只动 App，不动协议与引擎里的 `setFollow` / `FollowSet` / `Wire.maxFollow`（兼容旧 Mac，不做破坏性清理）。
- 建议顺序：P1 → P2 → 装机 → P3 → P4。P3 与另一个 iOS 会话（App 外观打磨）会改同一批 View 文件，开工前先 `git status` / `git log ios/`，
  确认没有未提交的并行改动再动手。

## P1 细节（`ios/gateway/agents.rs` 为主）

1. **数据**：`Live.follow_notices: Vec<FollowNotice>`，`FollowNotice { phone_id, agent_key, loc_key, plain: PushPlain, at }`。
   推送内容在结果发生时定下，之后不再读 Agent 状态。同一 `(phone_id, agent_key)` 至多一条，新结果覆盖旧的。
2. **`announce(agent_key, kind, now)`**：按手机分流——`is_looking` → 发应用内 `FollowEvent` 并 `deliver_follow`（见 5）；
   否则 `can_push` → 入队（随即调一次 `flush_follow_notices`，D = 0 / 已空闲时当场推出，口径同 `deliver()` 末尾的 `flush_due_notifications`）；
   两者都不满足 → 什么都不做、不移出。
3. **作废**：`on_agent_event` 里 `TurnStart` / `Activity` / `SessionEnd` 清掉该 Agent 的全部待通知（与 `done_at` 同口径）；
   `prune_follows` 移出会话时一并清；手机被取消配对时按 `phone_id` 清。
4. **`flush_follow_notices(now)`**：在 `tick` 里 `tick_follow` 之后调用。没有待通知就直接返回（不读系统时钟）。逐条：
   - 手机已不存在或不再关注该 Agent → 删；
   - `is_looking` → 删并 `deliver_follow`；
   - 读 `alert_after_secs()` 与 `mac_idle_secs()`：`idle*1000 + 1000 <= now - at`（空闲读数是向下取整的整秒，留 1 秒余量，宁可多留）→
     结果之后有过输入，删并 `deliver_follow`；
   - `after == 0`、空闲未知、或 `idle >= after` → `spawn_push(phone, "alert", loc_key, "fw-{agent}", &plain, now)`，删并 `deliver_follow`；
   - 否则留着。
5. **`deliver_follow(phone, agent)`**：从该手机的 `follow_set` 移出该 Agent，`revs.follow += 1`、`save_state`，对在线手机发 `follow.state`
   （写法复用 `prune_follows`）。
6. **轮询**：`tick_agents` 只在有人关注或有手机在线时轮询，一次性之后通知集自己清空，轮询随之停下，不用改。

**测试**（`tests_agents.rs`，虚拟时钟 + `TestHost::{set_alert_after, touch_input, idle_for}`；testkit 默认 `alert_after = 0`，旧用例的「立即推」仍成立）：

| 用例 | 期望 |
|---|---|
| 做完，Mac 在用 | 不推；空闲满 D 的那一刻推**一次**，之后不再推；会话移出通知集 |
| 做完，`D = 0` / 空闲未知 / 结果时 Mac 已空闲满 D | 当场（几个 tick 内）推 |
| 结果之后有键鼠输入 | 作废、不推、移出 |
| 待通知期间 Agent 又开始工作（`TurnStart` / `Activity`） | 作废；仍在通知集；下一次结果重新开始 |
| 待通知期间会话结束 | 作废 |
| 出错 | 与做完同规则；入队不立即推；D 到了才推 |
| 手机在前台 | 应用内横幅；不入队；移出 |
| 待通知期间手机变成在看 | 作废；移出 |
| 两部手机，一部在看一部不在 | 互不影响 |
| 旧 App 手动关注 | 同样一次性、同样遵守 D（回归） |
| 同一 Agent 连续两次结果（第一条还没推） | 只剩一条，推送 `collapse-id` 为 `fw-{agent}` |

既有用例里依赖「做完之后关注集保持不变」的，按一次性改写。

## P2 细节

- 删除 `tick_waiting`、`Live.waiting_since` / `waiting_notified`、`WAITING_DELAY_MS`、`announce` 里 `"waiting"` 分支与 `PUSH_FOLLOW_WAITING` 的发射，
  以及 `waiting_is_announced_once_for_a_question_that_is_not_on_ios`、`a_question_that_reached_the_phone_is_not_announced_twice`。
- **不动**：`AgentView.waiting_request_id` / `waiting_preview`（列表里仍显示「在等你」）；线上 `FollowEvent.kind = "waiting"` 与 `PUSH_FOLLOW_WAITING`
  的 loc key（旧 Mac 仍会发，App 与中继照收，以后再清）。

## P3 细节（iOS App）

| 文件 | 改动 |
|---|---|
| `Views/Agents/AgentDetailScreen.swift` | 删「关注」按钮与 `toggleFollow` / `followTick`；`item.view.followed` 时显示一行只读说明（铃铛） |
| `Views/Agents/AgentRow.swift` | 星标换成铃铛（只读） |
| `Views/Agents/AgentsScreen.swift`、`AHCore/AppStore+Agents.swift` | 删「已关注」过滤（`AgentFilter.followed`、`followedCount`）；`followed` 排序 / 分组如有也一并检查 |
| `Views/RootView.swift`、`AHCore/AppStore.swift` | 删 `.followLimit` 横幅；「Agent 做完 / 出错」应用内横幅保留 |
| `Views/Agents/TaskProgressScreen.swift` | 「你已关注它」改成「Agent 已经在运行，做完或出错时会通知你（Mac 空闲一阵后）」；名额满的句子改成不提「关注」 |
| `Localizable.xcstrings` | 经 `ios/Tools/make-catalog.py`（构建后）重新生成，补中文，删除不再使用的键 |
| 测试 | `AgentTests`、`AgentStoreTests`、`AgentE2ETests` 里走引擎 `setFollow` 的用例保留；`AskHumanUITests` / `AskHumanLiveUITests` 里点 `follow-button` 的步骤去掉：mock 场景直接断言铃铛与说明行，真服务场景改由 App 启动参数 `-AHFollowAgent` 经引擎 `setFollow` 种下关注（UI 里不再有入口） |

不改：`Wire.maxFollow`、`FollowSet` / `FollowState` / `FollowEvent`、`PhoneEngine.setFollow`、`MockBackend` 的 follow 场景（`-AHFollowEvent` 参数仍用来截横幅）。

## P4

- 回写文档：`docs/specs/ios-channel.md`（术语、D18、§6.6、§6.7、局限 4 / 15、验收 8 / 10）、`docs/specs/ios-alert-timing.md`（D11、局限 8、后续）、
  `docs/overview-ios.md`（Follow 描述、`agents.rs` 一行、模拟器场景一行）、`docs/wiki/ios-setup.md` 与 `.en.md`（Agents 页那一句）。
  `docs/overview.md` 主文档**不动**（全仓地图与不变量没变）。
- `./scripts/install.sh` 装机，`AskHuman channel ios status` 看 daemon 正常。
- 真机一轮（需要用户在场）：手机上用「新任务」发一个会很快做完的任务（Face ID；真实 Agent 的启动**必须先获用户批准**），
  `alertAfterSecs` 临时设成 60 秒：Mac 在用时做完不应推；停手满 60 秒时来一条；做完之前就回到 Mac 键鼠则不应补推；做完后会话已不在通知集
  （`state.json` 的 `followSet` 为空）。配置用后还原。
- 更新 `docs/PROGRESS.md`，已完成项删除。

## 不做

- 不改线协议、不改 `PROTOCOL_VERSION`、不改中继；不做破坏性的 App 协议清理（旧 Mac 兼容）。
- 不做 IM 统一，不做「Mac 上自己启动的任务也通知」（需求「后续」）。
