# 开发计划：IM 在世即响应

需求见 `docs/specs/im-always-respond.md`。本计划描述实现方案与全部技术 / 规则细节，不堆砌大段代码。
相关既有设计：`docs/plans/im-channel-activation.md`（活跃槽 / 入站监听 / 命令分派）。

## 0. 涉及文件总览

- `src-tauri/src/daemon/mod.rs`：监听生命周期（`ensure_inbound_listeners` 闸门放宽 + 启动后台拉起）、通用循环 `spawn_listener` 与抽取器签名泛化、`handle_inbound` 重构（命令 / 引导 / 退避判定）、`handle_detect` / `detect_*` 成功回执、活动提问判定 helper。
- `src-tauri/src/autochannel.rs`：新增 `Command::Help` 与未知命令识别；新增**共享文案生成器**（引导文案 `help_text`、作答确认 `answer_ack_text`、识别回执 `detect_ack_text`）。
- `src-tauri/src/channels/{dingding,feishu,slack,telegram}.rs`：**定稿采「会话内就地回执」**——按能力矩阵在「附件被累积 / 文本被接受 / 内容被忽略」处，用该渠道既有发文本原语就地回执（确认 / 引导）。`conversation.rs` 公共驱动**无需改动**（回执是各渠道传输细节，不进编排层）。
- `src-tauri/src/i18n.rs`：新增本地化键（引导 / 确认 / 回执 / `/help` 文案）。
- `docs/overview.md`：更新「IM 会话期自动激活」与命令一节，补「存活即监听 + 任何消息有回复」。

## 1. R1 存活即监听

### 1.1 闸门放宽
- `ensure_inbound_listeners` 删除 `if working_count()==0 { return }` 这道闸门；保留 `any_im_enabled(load_without_secrets())` 闸门（无启用 IM 则不连，零钥匙串）。
- 由「与工作中 agent 绑定」改为「与 daemon 生命周期绑定」：只要有启用 IM 就把各家监听拉起（幂等、按身份认领，复用现有 `inbound_listeners.claim/release`）。

### 1.2 启动即后台拉起（不挡关键路径）
- 在 `serve()` 启动末尾（与 `spawn_gui_host` 同区域）**spawn 一个后台 task** 调 `ensure_inbound_listeners(&state)`，使 daemon 一起来就连 IM、起监听。
- **后台执行**：不在 `handle_submit` 同步路径上等待，确保不回退方案3/4（弹窗 spawn 与 IM 连接并行、零钥匙串路径不变）。`handle_submit` 内现有的 `ensure_inbound_listeners` 调用保留（幂等，作为兜底）。
- 配置变更：现有 `on_config_changed` 已调 `ensure_inbound_listeners` + `invalidate_changed_routers`，放宽闸门后自然按新配置重连；无需新增。

### 1.3 不影响空闲退出
- 空闲退出守卫 `active==0 && working_count==0 && !has_agent_subs` 不引用入站监听 / Router，**保持不变**。监听任务与 Router 都不计入保活，daemon 仍正常空闲退出，退出时 serve 收尾丢弃 Router → 连接断 → 监听循环 `rx.recv()` 返回 `None` 自然结束。

### 1.4 armed 常态化的影响
- 放宽后观察者**常驻存在** → 路由层 `dispatch_observers` 恒返回 armed=true → `/` 前缀文本恒不进卡片（始终当命令）。这正是我们要的（命令任何时候都能用）。
- 既有副作用：文本兜底模式下，以 `/` 开头的「答案文本」会被当命令拦下（极少见）。属可接受的小行为变化，spec 已隐含（命令优先）。

## 2. 抽取器泛化（让观察者能感知非文本消息）

- 现状 `extract_*(ev) -> Option<(String /*sender*/, String /*text*/)>` 仅对文本返回；图片/文件返回 `None` → `handle_inbound` 收不到 → 无法对「空闲期图片探针」回引导（R3 边界场景）。
- 改为返回 `Option<Inbound>`，其中 `Inbound { sender: String, text: Option<String> }`：
  - 文本消息：`text = Some(...)`。
  - 来自期望发送者的**非文本**消息（图片/文件等）：`text = None`（仍带 sender，用于过滤 + 触发引导）。
  - 完全无关 / 无发送者：仍 `None`。
- `spawn_listener` 按 `sender` 过滤（同现状），再把 `Inbound` 交 `handle_inbound`。
- `handle_inbound(state, channel_id, inbound)`：
  - `text=Some` → 走 §3 文本分派。
  - `text=None`（非文本）→ 仅当「该渠道**无**活动在途提问」时回引导（有活动提问时退避，附件由会话确认，见 §4）。

## 3. `handle_inbound` 重构（命令 / 引导 / 退避）

新分派（伪逻辑，非最终代码）：

```
let trimmed = text.trim();
if trimmed.starts_with('/') {
    match parse_command(trimmed) {
        Status => 现状逻辑（始终回状态）
        Here   => auto ? 现状逻辑 : reply(help_text(...))   // 关态 /here 改回引导
        Help   => reply(help_text(...))                      // 新增
        Unknown(None) => reply(help_text(...))               // 未知 / 命令；命令不会被卡片消费，安全
    }
    return;
}
// 非命令文本
if has_active_question_on(state, channel_id) {
    return;   // 退避：交渠道会话（§4）处理确认 / 引导
}
if auto {
    let (switched, n) = set_active_channel(state, channel_id).await;
    if switched { reply(activated_receipt(n)); return; }
    // 未切换（已是活跃槽但无在途提问）→ 落到引导
}
reply(help_text(...));   // 无在途提问 → 引导（liveness）
```

- `parse_command` 扩展：新增 `Help`（`/help`、`/帮助`、`/?`）；并区分「已识别命令」与「未知 `/命令`」（返回一个表示未知的变体或 `Option`）。
- `has_active_question_on(state, channel_id)`：遍历 `registry.in_flight_entries()`，存在 `entry.coordinator.has_channel(channel_id)` 即真。
- `reply(...)` = 现有 `reply_channel_text(channel_id, &config, text)`。

### 3.1 引导文案 `help_text`（`autochannel.rs`）
- 入参：`lang`、`auto: bool`、`has_active_question: bool`、`mode_hint`（卡片 / 文本，可由渠道能力推断，初版可统一为「卡片为主」的措辞，文本兜底场景少）。
- 动态分块拼装（按开关增删，缺省块不输出）：
  1. 命令清单：`/status`、`/help`；`auto` 时加 `/here`。
  2. `has_active_question` 时：如何作答提示（卡片：选择/输入后提交、可发图片/文件补充）。
  3. `has_active_question==false` 时：「暂无进行中的提问」。
  4. `auto` 时：「发任意文字＝把提问切到此渠道接收」。
- 文案全部走 i18n（zh/en）。

## 4. R2 作答确认（渠道会话侧）

定位：内容被接受进答案的**唯一可见处**在渠道会话。需先**逐家审计能力矩阵**并在接受处插回执钩子。

### 4.1 能力矩阵（实现前逐家确认，写进各文件注释）
| 渠道 | 卡片模式接受 | 文本兜底接受 | 备注 |
|---|---|---|---|
| 钉钉 | 图片/文件（聊天累积）；文字取卡片输入框 | 编号/文字/图片/文件 | `DdInbound::Bot` 累积处插钩子 |
| 飞书 | 图片/文件（聊天累积）；文字取卡片输入框 | 编号/文字/图片/文件 | 同构 |
| Slack | 图片/文件（聊天累积）；文字取卡片输入框 | 编号/文字/图片/文件 | 同构 |
| Telegram | 文字（无卡片输入框，自由文字归活动卡片）；**不收图片** | 文字/编号 | 见 `telegram.rs` 头注 |
> 上表为依现有代码的初判，实现首步**逐家核对**后定稿（严格模式一律忽略自由文字/附件 → 不确认，归引导）。

### 4.2 回执注入方式（定稿：会话内就地回执）
- 在各渠道会话的「内容被接受 / 累积」分支就地调 `reply`（用该渠道既有发文本原语，如钉钉 `client.send_text` / 飞书 / Slack / Telegram 对应原语）发一条 `answer_ack_text(kind, mode, lang)`：
  - 卡片模式累积到图片 → `answer_ack_text(Image, Card)`：「✅ 已收到，将把该图片加入你的回答；请在卡片点提交完成」。
  - 卡片模式累积到文件 → `answer_ack_text(File, Card)`。
  - 文本兜底接受到文字/编号 → `answer_ack_text(Text, Fallback)`：「✅ 已收到，将作为你的回答」。
  - 文本兜底接受到图片/文件 → 对应 Image/File、Fallback 措辞。
- **不被接受**的内容（卡片模式纯文字、严格模式忽略项）→ 会话就地回 `help_text(...)`（与观察者共用同一生成器，保证一致）。
- 每条附件各回一条（R2：即时反馈）。回执为 best-effort，发送失败仅日志、不影响作答。
- `answer_ack_text` 放 `autochannel.rs`，入参 `(kind: Text|Image|File, mode: Card|Fallback, lang)`，走 i18n。

### 4.3 与观察者的边界
- 会话与观察者对**同一条非命令文本**都会收到。§3 已让观察者在「该渠道有活动在途提问」时**退避**，故文本的「确认 / 引导」只由会话发一次。
- 图片/文件观察者本就看不到内容（§2 只拿到 `text=None`），且有活动提问时观察者退避 → 仅会话回执。

## 5. R5 自动识别成功回执（`detect_*`）

- 在 `detect_dingtalk/feishu/slack` 里 `wait_*_code` 返回 `Ok(sender_id)` 后、函数返回前，用该流程已持有的 Router/Client 向该 `sender_id` 发一条 `detect_ack_text(field_label, lang)`：「✅ 识别成功，已自动填入 {字段名}」。
  - 字段名：钉钉 `userId` / 飞书 `openId` / Slack `userId`（i18n 友好展示名，不回显 ID 值）。
  - 既有连接复用场景：用观察到的 Router 发；临时连接场景：在 `drop(router)` 前发。
- 失败 / 超时 / 取消路径不发。回执 best-effort，失败仅日志，不影响把 ID 回 `Detected` 给设置进程。

## 6. i18n 键（`i18n.rs`，zh/en）

- `autoChannel.help*`：引导文案各分块（标题 / 命令清单项 / 如何作答 / 无在途提问 / 切槽提示）。
- `autoChannel.ackText` / `ackImage` / `ackFile`（× 卡片 / 文本两套措辞，或用占位拼装）。
- `autoChannel.detectAck`（含 `{field}` 占位）+ 字段展示名（userId / openId）。
- `/help`、`/here`、`/status` 命令同义词表（中英），与 `parse_command` 对齐。

## 7. 测试与验证

- 纯逻辑单测（`autochannel.rs`）：`parse_command`（含 `/help`、未知命令、大小写 / 中文同义词）；`help_text` 按 `auto`/`has_active_question` 组合输出预期分块；`answer_ack_text` / `detect_ack_text` 占位替换。
- daemon 逻辑：`has_active_question_on` 判定。
- 端到端（真机，按 `AGENTS.md` 用 `./scripts/install.sh` 装后用新 `AskHuman`）：
  1. 关生命周期追踪、不提问，daemon 在世（被一个窗口/在途请求保活）时发普通文字 → 收到引导文案（证明 R1 放宽生效）。
  2. 卡片作答期间发图片 → 收「将把该图片加入回答」；发纯文字 → 收引导；点提交正常定稿（R2 不破坏作答）。
  3. `/status`、`/here`（开/关两态）、`/help`、未知 `/foo` → 各自预期回复。
  4. 自动识别：发 4 位码识别成功 → 收识别回执，且设置侧 ID 正常填入。
  5. daemon 退出后发消息 → 无任何回复（区分成立）。
- 回归：自动激活四家既有端到端（切槽 / 补推 / 抢答）不变。

## 8. 实施顺序（建议）

1. R1 闸门放宽 + 启动后台拉起（最小改动、可独立验证「存活即监听」）。
2. `autochannel.rs` 共享文案生成器 + `parse_command` 扩展 + 单测。
3. `handle_inbound` 重构 + 抽取器泛化（§2/§3）。
4. 逐家会话能力审计 + 回执注入（§4）。
5. `detect_*` 回执（§5）。
6. i18n 收口 + overview 更新 + 端到端验证。
