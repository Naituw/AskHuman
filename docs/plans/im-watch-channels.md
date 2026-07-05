# /watch 多渠道扩展开发计划（Telegram / Slack / 钉钉）

> 前置：P1 飞书已完成（`docs/specs/im-watch.md`，含跟底重发 / 足迹时间线 / TODO 折叠面板 /
> 回合时长等全部定案）。本计划把 /watch 推广到其余三渠道，按能力矩阵排定优先级与实现路径。
> 状态：待评审（优先级顺序、TODO 展示降级方案、钉钉模板路径等待用户定案，见文末问题清单）。

## 0. 渠道能力矩阵（就地编辑 / 按钮回调 / 富文本 / 折叠）

| 能力 | 飞书（已做） | Telegram | Slack | 钉钉 |
| --- | --- | --- | --- | --- |
| 就地编辑 | `patch_card`（14 天） | `editMessageText`（48 小时内可编辑） | `chat.update`（无明确期限） | 互动卡片实例更新 `PUT /v1.0/card/instances` |
| 按钮回调 | 卡片 callback（WS 长连接） | inline keyboard + `callback_query`（getUpdates 长轮询，**已有**：提问卡在用） | Block Kit actions + Socket Mode interactive（**已有**：`SlInbound::Interactive`） | 卡片回调 Stream topic（**已有**：提问卡在用） |
| 状态圆点颜色 | `<font color>` 彩色 ● | ✗ → emoji 🟢/⚪/🔴 | ✗ → emoji 🟢/⚪/🔴 | 模板内富文本（取决于模板变量设计） |
| 粗体/斜体 | markdown | HTML `<b>/<i>`（**已有** markdown→HTML 管道） | mrkdwn `*b*` `_i_`（**已有** blockkit/markdown 管道） | 模板变量 markdown |
| TODO 折叠面板 | collapsible_panel | ✗（无折叠组件） | ✗（无折叠组件） | 模板条件渲染（能力存疑，需实测） |
| 编辑频控 | 宽松（每卡 1s 限速够用） | 每 chat ≈1 条/s + 全局 30/s | `chat.update` Tier 3 ≈50/min | 未知，需实测（提问卡未高频更新过） |
| 新增客户端 API | — | 无（`edit_message_text` 等全齐） | `chat.update`（一个方法） | 卡片实例更新 + 流式（新增 client 方法） |

结论：**Telegram 最低成本**（所有传输原语已存在）、**Slack 次之**（补一个 `chat.update`）、
**钉钉最高风险**（需在开发者后台建「watch 专用模板」+ 新增实例更新 API + 能力实测），与
P1 设计时「钉钉走专用模板留 P2+」的判断一致。

## 1. 公共重构（渠道无关化，任何渠道动工前先做）

现状：`watch.rs` 的帧在 `build_frame` 时就渲染成了**飞书 markdown**（`<font>` 圆点步行、
TODO 行），`daemon/mod.rs` 的 `WatchState`/`watch_tick`/`ensure_watch_routes` 硬编码飞书
（`FsRouter`/`patch_card`/`send_card`），扰动信号 `disturb_ms` 是单渠道全局值。

- **R1 帧中立化**：`WatchFrame` 保存结构化数据（`Vec<ToolStep>`、`Vec<TodoItem>`），渲染下沉到
  各渠道 renderer（飞书現有 `render_step_feishu`/`render_todo_feishu` 移到渲染层；签名改为对
  结构化内容计算，跨渠道一致）。
- **R2 传输抽象**：`WatchTransport` trait —— `send(frame) -> message_id`、`edit(message_id, frame,
  mode)`、`finalize(message_id, frame, kind)`；按钮回调仍走各渠道 Router（回调事件归一为
  `WatchAction::{Unwatch, Refresh, …}`，飞书已有 `parse_watch_action` 模式照搬）。
- **R3 订阅带渠道**：`WatchEntry` 增 `channel` 字段（`PersistedWatch` 已有，恢复时按渠道重建）；
  `disturb_ms` 改**按渠道**记录（`HashMap<channel, AtomicU64>` 或固定四槽）；`handle_inbound` /
  `attach` / 完结豁免等扰动标记按事件来源渠道打点。
- **R4 门控放开**：`autochannel::help_text` 与 `watch.unsupported` 按「渠道是否已支持」逐个放开；
  每渠道关注上限仍 5。
- **R5 频控参数化**：每卡最短编辑间隔从常量 1s 改为渠道参数（TG 1s / Slack 2s / 钉钉待实测）；
  连续 5 次编辑失败自动退订逻辑复用。

公共重构不改变飞书行为（回归验证：飞书全流程 + 全量单测）。

## 2. P2 Telegram（推荐先做）

- **发送/编辑**：`send_message`（HTML）+ `edit_message_text`（均已有）；活动态按钮
  inline keyboard `[取消关注][立即刷新]`，终态编辑为无按钮 + 终态行文案（TG 不支持禁用按钮，
  终态=移除按钮，与提问卡收尾同模式）。
- **渲染**：状态行/标题/最近动态与飞书同文案；足迹步行 `🟢 <b>运行命令</b>: <i>cargo test</i>`
  （emoji 圆点代替彩色 ●）；「… 已省略 N 步」普通灰不可做 → 斜体。
- **TODO 展示（用户定案）**：仅摘要行（与 /status 一致），不做展开。
- **跟底重发**：同飞书语义（30s 节流、提问期间抑制、答完豁免）；旧卡 `edit_message_text` 定格
  「已移至最新卡片 ⬇」。扰动源：TG 渠道的 inbound 消息 / 提问卡 attach。
- **回调路由**：`TgRouter` 增 watch 卡分支（按 message_id 精确路由，与提问卡 callback 并存；
  `callback_query.data` 放 `watch:unwatch|refresh|todo`）。
- **约束**：48h 后不可编辑 → 编辑失败计入 fails（≥5 自动退订，已有机制兜底）；频控每 chat 1/s
  与现有每卡 1s 限速天然匹配。

## 3. P3 Slack

- **发送/编辑**：`chat.postMessage`（已有）+ **新增** `chat.update`（client 一个方法）；Block Kit
  section/context/actions 组卡（`blockkit.rs` 已有全套构件与提问卡先例）。
- **渲染**：mrkdwn `*粗*` `_斜_`；emoji 圆点；「已省略 N 步」用 context block（灰色小字，Slack
  唯一的「弱化文本」载体，更新时刻行同）。
- **按钮**：actions block 两按钮；终态 `chat.update` 替换为纯静态 blocks（Slack 不支持禁用按钮，
  与提问卡终态同模式——文件头注释已有此定案）。
- **TODO 展示（用户定案）**：仅摘要行。
- **回调路由**：`slack/router.rs` 的 `SlInbound::Interactive` 分支增 watch 卡路由（按
  `container.message_ts` 精确匹配；`action_id` 放 `watch_unwatch|watch_refresh|watch_todo`）。
- **频控**：`chat.update` Tier 3（≈50/min）→ 每卡最短编辑间隔建议 2s + 渠道级预算（同渠道多卡
  合计不超 ~40 次/min，超出顺延到下一 tick）。

## 4. P4 钉钉（最高风险，建议最后；PoC 先行）

**已被现有代码坐实的能力**（提问卡在用，无需 PoC）：
- 实例更新 API 已实现且可用：`client::update_card_private`（`PUT /v1.0/card/instances`，
  按 outTrackId + `updateCardDataByKey` 就地改变量）——提问卡收尾/抢答就是靠它置终态。
- 模板条件渲染可行：提问模板已按变量条件渲染 单/多选、输入框显隐、submitted 终态。
- 变量内富文本可行：选项 md 已用 `<font sizeToken/colorTokenV2>`（→ 彩色状态圆点可能可做）。
- 按钮回调可行：Stream topic `/v1.0/card/instances/callback` + actionId（`DdRouter` 已有）。
- 内置默认模板 ID 模式可行（`DEFAULT_CARD_TEMPLATE_ID` 先例，设置项允许覆盖）。

**PoC 只验剩下的唯一核心未知：高频反复更新**（提问卡一生只更新一次，watch 要分钟级持续
每 2–10s 更新）。三步：

1. **建 watch 模板**（开发者后台卡片平台，半天）：变量 `header`/`state_line`/`title_line`/
   `body_md`（动态文字+足迹行 markdown）/`todo_summary`/`updated_line`/`mode`("active"|"final")/
   `final_label`；两按钮（actionId=`watch_unwatch`/`watch_refresh`）按 `mode` 条件渲染，
   final 态只显 `final_label` 灰字——全部复刻提问模板已验证的条件渲染手法。
2. **探针命令**（半天）：加隐藏调试子命令 `AskHuman debug dd-watch-poc`（不进 help；PoC 后保留
   作回归工具）——用现有 client 建卡投放，随后循环 60 次、间隔 2s 更新 `state_line`/`body_md`/
   `updated_line`，逐次记录 API 耗时与错误码；期间人在钉钉端观察。中途穿插：点两个按钮验证
   回调路由、发几条普通消息把卡顶上去后确认更新仍生效、最后置 `mode=final` 验证终态渲染。
3. **验收标准 + go/no-go**（半天实测）：① 更新端上可见延迟中位 ≤3s；② 1 次/2s 持续 5 分钟
   无频控报错（有则测出安全间隔，回填 R5 渠道频控参数）；③ 按钮回调往返正常；④ 淹没后更新
   仍生效；⑤ 终态渲染正确。结论写进本计划 + `docs/specs/im-watch.md` 渠道差异表，通过才排期
   全量（M4）。任一不达标 → 评估降级（如更新间隔放宽到 10s）或放弃钉钉渠道。

## 5. 里程碑与验收

1. **M0 公共重构**（R1–R5）：飞书行为不变，全量单测 + 飞书真机回归。
2. **M1 Telegram**：/watch 全流程（订阅/静默刷新/按钮/跟底/终态/重启恢复）真机验收。
3. **M2 Slack**：同上。
4. **M3 钉钉 PoC** → 评审实测结论 → **M4 钉钉全量**。

每里程碑单独提交（`feat(watch,telegram): …` 等），文档同步 `docs/specs/im-watch.md`
（渠道差异表）与 `docs/overview.md`。

## 6. 定案记录 / 待定案问题

- **已定案**：TG/Slack 的 TODO 只显示摘要行，不做展开（用户定案）。
- **Q1 优先级顺序**：建议 Telegram → Slack → 钉钉（成本/风险递增），待确认。
- **Q3 钉钉 PoC**：方案见 §4（模板半天 + 探针命令半天 + 实测半天，核心只验高频更新），待确认。
- **Q4 频控参数**：Slack 每卡 2s / 渠道预算 40 次每分钟，是否接受。
