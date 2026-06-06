# 开发计划：钉钉「互动卡片预选答案」（高级版卡片 · A 方案落地）

> 关联需求：`docs/specs/dingtalk-card-answers.md`
> 关联既有：`docs/plans/dingtalk-channel.md`（钉钉渠道总体）。本计划落地其中「后续增强 · A 方案」，并替换该渠道在「有卡片模板时」的提问形态。
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

```
配置：channels.dingding { enabled, clientId, clientSecret, userId, cardTemplateId? }
        cardTemplateId 留空 → 用写死默认 6cfe19d3-...schema（默认即用卡片）
                                   │
run_conversation（公共驱动，不变）
   ├─ send_message_prompt：共享 Message + -f 文件（沿用现有逻辑）
   └─ ask_question（钉钉，改造）：
        ① createAndDeliver 投放互动卡片到机器人单聊（callbackType=STREAM, outTrackId=uuid）
        ② 在同一条 Stream 上循环收事件：
             · 卡片回调(本卡片+submit) → 组装答案(selected_options+user_input+累积图片/文件)
               → 3 秒内 ACK 回包 submitted=true（避免「提交失败」toast）→ 返回答案
             · bot 消息(本用户)：图片/文件 → 下载累积；纯文字 → 忽略
             · 卡片回调(非本卡片/非 submit) → 空 ACK 跳过
             · cancelled → best-effort 关闭卡片 → 返回 None
        ③ 投放失败 → 回退 B 方案（纯文本+编号选项）问该题
```

核心改动：①新增 `cardTemplateId` 配置（含默认 ID）；②`client` 增「创建并投放 / 更新」高级版卡片接口；③`card` 重写为「高级版 cardData 组装 + 回调解析」；④`stream` 支持卡片回调**延迟 ACK + 回包**；⑤`DingTalkSession.ask_question` 改为卡片流程（失败回退文本）；⑥设置页/类型/i18n/文档同步。

---

## 1. 卡片契约（与用户已发布模板对齐）

模板 ID（默认）：`6cfe19d3-3b36-4681-827d-e7c1d0574d0a.schema`。变量与交互固定如下，**实现须严格按此键名**：

- 公有变量（放入 `cardData.cardParamMap`）：
  - `title`：字符串。卡片标题/题首（多题用 `Question i/n`；单题无 Message 用来源头部；否则可空）。
  - `markdown`：字符串（Markdown 组件）。问题正文。
  - `options`：对象数组，每项 `{ "text": "<选项文案>" }`。**复杂类型 → 转成 JSON 字符串**后放入 cardParamMap。
  - `submitted`：布尔，初值 `"false"`（字符串）。控制勾选禁用 + 按钮「提交/已提交」。
  - `private_input`：字符串，初值 `""`。已提交文本回显。
- 本地变量（用户交互产生，提交时回传，**不需要我们下发**）：`user_input`（补充文字）、`selected_options`（勾选项，存的是选项 text）。
- 提交按钮：`actionId="submit_action"`，回传 `cardPrivateData.params = { user_input, selected_options }`、`actionIds=["submit_action"]`。

> 注：模板内 `${tag}` 变量未在变量列表声明，不下发即可（不影响渲染）。`is_markdown=false` 时正文仍走 `markdown` 变量、按原文展示（不额外转义）。

### 1.1 下发 cardData 示例（cardParamMap，值均为字符串）

```
title         = "Question 1/3"
markdown      = "要继续吗？"
options       = "[{\"text\":\"继续\"},{\"text\":\"停止\"}]"
submitted     = "false"
private_input = ""
```

### 1.2 提交回调内容（content/value 为 JSON 字符串）

```
{"cardPrivateData":{"actionIds":["submit_action"],
 "params":{"user_input":"...","selected_options":["继续"]}}}
```

映射答案：`selected_options`(过滤空串) → `QuestionAnswer.selected_options`；`user_input` → `QuestionAnswer.user_input`；图片/文件来自作答期间累积。`selected_options` 元素兼容字符串或 `{text}` 对象（取 `text`）。

---

## 2. 配置层（Rust + TS）

### 2.1 `config.rs`
- `DingTalkChannelConfig` 增 `card_template_id: String`（camelCase `cardTemplateId`，`#[serde(default)]`，默认空串）。
- 提供取「有效模板 ID」的规则：`card_template_id` 去空白后非空则用之，否则用常量默认 ID。常量定义在钉钉模块（见 §4）。
- 单测：默认值含空 `card_template_id`；旧 JSON 无该字段仍可加载。

### 2.2 `src/lib/types.ts`
- `DingTalkChannelConfig` 增 `cardTemplateId: string`。

> `is_dingding_active` 判定不变（仍是 enabled + client 三项可构造）；`cardTemplateId` 不参与 active 判定（留空有默认）。

---

## 3. 钉钉客户端层 `dingtalk/`

### 3.1 `client.rs`：高级版卡片接口（替换原 StandardCard 的 send/update）
- `create_and_deliver_card(out_track_id, card_template_id, card_param_map, callback_stream) -> Result<()>`：
  - `POST /v1.0/card/instances/createAndDeliver`，body：`cardTemplateId` + `outTrackId` + `cardData:{cardParamMap}` + `openSpaceId:"dtv1.card//IM_ROBOT.{userId}"` + `imRobotOpenSpaceModel:{supportForward:true}` + `imRobotOpenDeliverModel:{spaceType:"IM_ROBOT", robotCode:clientId}` + `callbackType:"STREAM"` + `userIdType:1`。
- `update_card(out_track_id, card_param_map, private_param_map) -> Result<()>`：
  - `PUT /v1.0/card/instances`，body：`outTrackId` + `cardData:{cardParamMap}`（可空）+ `privateData:{ <userId>:{cardParamMap} }` 或 `userPrivateData`（按更新接口要求）+ `cardUpdateOptions:{updateCardDataByKey:true, updatePrivateDataByKey:true}`。
  - 用途：被抢答/收尾时 best-effort 置 `submitted=true`（DC11）。
- 原 `send_card` / `update_card`（StandardCard，`#[allow(dead_code)]`）删除或改造为以上接口。
- `cardParamMap` 组装工具：把 `serde_json::Map` 内非字符串值 `to_string()`（对象/数组→JSON 字符串、bool→`"true"/"false"`、数字→字符串）。

### 3.2 `card.rs`：高级版 cardData 组装 + 回调解析（重写）
- `build_card_param_map(header, text, options, is_markdown) -> Map`：产出 `title/markdown/options/submitted/private_input` 五键（`options` 为 `[{text}]`、`submitted="false"`、`private_input=""`），复杂值转 JSON 字符串。
- `parse_card_submit(data) -> Option<CardSubmit>`：从回调 `data` 取 `userId`/`outTrackId`，解析 `content`（优先）或 `value`（字符串/对象皆容错）→ `cardPrivateData`：要求 `actionIds` 含 `"submit_action"`；取 `params.selected_options`（→ `Vec<String>`，过滤空串，兼容 `{text}`）与 `params.user_input`（→ `Option<String>`）。
  - `CardSubmit { user_id, out_track_id, selected_options, user_input }`。
- 解析需健壮：缺字段不 panic，返回 `None`（非 submit/非本类回调由会话层空 ACK 跳过）。

### 3.3 `stream.rs`：卡片回调延迟 ACK + 回包
- `StreamEvent::CardCallback` 改为携带 **messageId**（如 `CardCallback { data: Value, message_id: String }`）；SYSTEM ping 与 BotMessage 仍**自动 ACK**（行为不变）。
- 卡片回调（topic `/v1.0/card/instances/callback`）在 `handle_frame` 中**不自动 ACK**，将 `message_id` 随事件上抛。
- 新增公开方法 `respond(message_id, data_value)`：发送 `{code:200, headers{messageId,...}, message:"OK", data:<json 字符串>}`（即现有 `ack` 的可带 body 版本；保留无 body 的空 ACK 给跳过场景）。
- 约束：会话层必须在 **3 秒内** 对每个卡片回调调用 `respond`（带回包）或空 ACK（跳过），否则钉钉会重推。
- `commands.rs::dingtalk_detect_wait` 只用 BotMessage，不受影响；其匹配 `StreamEvent` 处补齐新 `CardCallback` 分支（直接空 ACK 跳过即可）。

---

## 4. 钉钉会话渠道 `channels/dingding.rs`

### 4.1 常量
- 模块内定义 `DEFAULT_CARD_TEMPLATE_ID = "6cfe19d3-3b36-4681-827d-e7c1d0574d0a.schema"`；`effective_template_id(config)` 返回配置值或默认。

### 4.2 `DingTalkSession`
- `open()`：取 token + 建 Stream，**同时订阅 bot 消息 + 卡片回调两个 topic**（`&[TOPIC_BOT_MESSAGE, TOPIC_CARD_CALLBACK]`）。
- `send_message_prompt()`：不变（共享 Message + `-f` 文件）。
- `ask_question(ctx, cancelled)` 改为**卡片流程**：
  1. `out_track_id = uuid`；`card_param_map = build_card_param_map(ctx.header, ctx.text, ctx.options, ctx.is_markdown)`。
  2. `create_and_deliver_card(...)`：
     - 失败 → 记日志（i18n 警告）→ **回退 B 方案**：调用保留的 `ask_question_text(...)`（原纯文本+编号逻辑）问该题并返回其结果。
  3. 成功 → 进入事件循环（每 `POLL_INTERVAL` 检查 `cancelled`）：
     - `CardCallback`：
       - `parse_card_submit` 命中且 `out_track_id` 匹配、`user_id==配置 userId` →
         - 组装 `QuestionAnswer { selected_options, user_input, images(累积), files(累积) }`；
         - `stream.respond(message_id, {cardUpdateOptions:{updatePrivateDataByKey:true}, userPrivateData:{cardParamMap:{submitted:"true"}}})`；
         - 返回 `Some(answer)`。
       - 否则（非本卡片/非 submit/解析失败）→ `stream.respond(message_id, {})` 空 ACK，继续等待。
     - `BotMessage`（`senderStaffId==userId`）：
       - `picture` → 下载转 base64 累积进 `images`（沿用现有 `download_image`）。
       - `file` → 下载落地累积进 `files`（沿用现有逻辑）。
       - `text`/其它 → **忽略**（DC8）。
     - 超时/连接断开/`cancelled` → 见收尾。
  4. `cancelled` 置位 → best-effort `update_card(out_track_id, submitted=true)`（DC11，失败仅日志）→ 返回 `None`。
- `close()`：断开 Stream（不变）。
- **保留** B 方案函数：把现有纯文本提问主体抽成 `ask_question_text(...)`（含 `build_question_text` / `message_to_answer` / `parse_reply`），供回退使用；不再作为默认路径。

> 累积状态（images/files）为**单题局部**变量，随每次 `ask_question` 重置（多题互不串味）。

---

## 5. 命令与设置页（前端）

### 5.1 `commands.rs`
- 无新增命令。`dingtalk_test` 不变。`dingtalk_detect_*` 不变（其 `StreamEvent` match 补 `CardCallback` 跳过分支，见 §3.3）。

### 5.2 设置页 `SettingsView.vue`（钉钉卡片）
- 在 UserId 字段下新增「卡片模板ID」输入框：`v-model="config.channels.dingding.cardTemplateId"`，`@change="persist"`，`placeholder` 提示「留空使用默认模板」。
- 不新增按钮；测试连接/自动识别不变。

### 5.3 `types.ts` / `ipc.ts`
- `types.ts`：见 §2.2。`ipc.ts` 无新增调用。

---

## 6. i18n

- 新增设置页键：`settings.channels.cardTemplateId`（标签）、`settings.channels.cardTemplateIdPlaceholder`（占位提示）——`zh.ts` / `en.ts` 各补。
- 新增/复用 Rust 端（`i18n.rs`）警告键：卡片投放失败回退、卡片更新失败等（如 `channel.ddCardDeliverFailed`、`channel.ddCardUpdateFailed`）。沿用 `warn_prefix` 输出。
- B 方案相关键（`ddHintOptions` 等）保留（回退时仍用）。

---

## 7. 运行编排 `app/mod.rs`

- 不变：`is_dingding_active` / `active_messaging_channels` / `run_headless` 均不感知卡片（卡片是 `DingTalkSession.ask_question` 内部行为）。
- 抢答/退出码语义不变。

---

## 8. 文档

- `prompts.rs` / `README.md`：补充钉钉卡片用法与「卡片模板ID」配置说明，并注明前置条件（用户在卡片平台搭建并发布模板、应用具备互动卡片能力）。
- `docs/plans/dingtalk-channel.md` / `docs/specs/dingtalk-channel.md`：在「A 方案后续增强」处加一行指向本文档（A 方案已落地）。

---

## 9. 依赖

- 无新增 crate（复用 `reqwest`/`serde_json`/`tokio`/`tokio-tungstenite`/`uuid`/`base64`）。

---

## 10. 涉及文件清单

改动：
- `src-tauri/src/config.rs`：`DingTalkChannelConfig.card_template_id` + 单测。
- `src-tauri/src/dingtalk/client.rs`：`create_and_deliver_card` / `update_card`（高级版）+ cardParamMap 工具；删除/改造 StandardCard 旧接口。
- `src-tauri/src/dingtalk/card.rs`：重写 `build_card_param_map` + `parse_card_submit`。
- `src-tauri/src/dingtalk/stream.rs`：`CardCallback` 带 messageId + 卡片回调延迟 ACK + `respond`。
- `src-tauri/src/channels/dingding.rs`：卡片流程 `ask_question` + 失败回退 `ask_question_text` + 默认模板常量。
- `src-tauri/src/commands.rs`：`StreamEvent` match 补 `CardCallback` 跳过分支。
- `src-tauri/src/i18n.rs`：新增警告键。
- `src/lib/types.ts`：`cardTemplateId`。
- `src/views/SettingsView.vue`：卡片模板ID 输入框。
- `src/i18n/zh.ts` / `src/i18n/en.ts`：设置页键。
- `src-tauri/src/prompts.rs` / `README.md` / `docs/...dingtalk-channel.md`：文档。

新增：
- `docs/specs/dingtalk-card-answers.md` / `docs/plans/dingtalk-card-answers.md`（本两份）。

---

## 11. 任务顺序

1. 配置：`config.rs` + `types.ts`（含默认值单测）。
2. 客户端：`client.rs` 高级版投放/更新 + cardParamMap 工具（纯函数单测）。
3. 卡片：`card.rs` 组装 + 回调解析（纯函数单测：cardParamMap 组装、回调解析、selected_options 过滤/兼容）。
4. Stream：`stream.rs` 卡片回调延迟 ACK + `respond`；`commands.rs` match 补分支。
5. 会话：`dingding.rs` 卡片 `ask_question` + 回退 `ask_question_text` + 默认模板常量。
6. 设置页 + i18n + 文档。
7. 构建（`--features custom-protocol`）+ 端到端实测。

---

## 12. 测试策略

- Rust 单测：`config` 默认/容错；`card` 纯函数（cardParamMap 组装、`parse_card_submit` 各形态、空串/对象兼容）；`client` 纯函数（cardParamMap 值转字符串）。
- 手动/端到端（需真实内部应用 + 已发布模板）：
  - 单题 / 多题 / 带 Message + `-f` 文件。
  - 卡片多选高亮、补充文字、点提交完成；提交后变「已提交」且无「提交失败」toast。
  - 作答期间发图片/文件被收进答案；聊天纯文字被忽略。
  - 卡片投放失败回退纯文本编号选项。
  - 与弹窗 / Telegram 抢答（钉钉先答 / 被抢答中止）。
  - headless 单跑钉钉。
  - Telegram / 弹窗回归。

---

## 13. 风险与注意

- **3 秒 ACK**：卡片回调须 3 秒内 `respond`（带 submitted=true 回包或空 ACK），否则重推；会话层同步组装答案后立即 respond。
- **提交失败 toast**：必须回包 `submitted=true`（私有数据，`updatePrivateDataByKey:true`），否则卡片成功条件不满足。
- **selected_options 形态**：以真机回调为准（可能是文本数组或 `{text}` 对象数组），解析两者兼容并过滤初始空串 `[""]`。
- **单 Stream 约束**（DC13）：维持不修；连续/并发提问可能相互干扰，仅记录。
- **断线重连**：长等待期间 WS 可能断，沿用现有重连；重连后继续等本卡片回调。
- **回退路径**：投放失败回退 B 方案须复用现有纯文本逻辑，行为与现状一致。
- **配置容错**：旧配置无 `cardTemplateId` 走默认（`#[serde(default)]` + 默认 ID）。
