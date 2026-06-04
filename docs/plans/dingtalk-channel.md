# 开发计划：新增「钉钉」通信渠道（Channel）

> 关联需求：`docs/specs/dingtalk-channel.md`
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

```
配置(设置页) ──► AppConfig.channels.dingding { enabled, clientId, clientSecret, userId }
                 （robotCode 不单独配置，内部统一取 clientId = AppKey）
                                   │
AskHuman "..." -q ... -o ...       ▼
   └─ run_ask 决策渠道：弹窗(若GUI) + 会话型渠道(telegram/dingding 各自 active 时)
        └─ 各会话型渠道 = 复用「公共驱动 run_conversation」+ 各自「MessagingChannel 实现」
             ├─ Telegram：长轮询(现状重构入抽象)
             └─ 钉钉：DingTalk Stream 长连(收) + OpenAPI(发)
                  发：oToMessages/batchSend(Message 文本) · 互动卡片(StandardCard，逐题)
                      media/upload(-f 文件) · 更新卡片(点选高亮)
                  收：Stream 双 topic —— bot 消息(文字/图片/文件/userId识别) + 卡片回调(点选/发送)
                       图片/文件经 messageFiles/download 下载落地
        └─ Coordinator 抢答：首个终态生效 → emit_result → 退出
```

核心两件事：①**新增钉钉渠道**（Stream 收 + OpenAPI 发 + 互动卡片）；②**抽象公共会话逻辑**，Telegram 与钉钉复用，未来 Channel 易扩展。

---

## 1. 公共抽象重构（先行，Telegram 平迁）

目的：把「多问题逐条发送 / 单题特例 / 收集答案 / 投递」这套**与传输无关**的逻辑抽出来，各渠道只实现「发一条消息 / 发一道题并等到一个最终答案」等**传输相关**原语。

### 1.1 新增 `MessagingChannel` 接口（会话型消息渠道）

位置：`src-tauri/src/channels/mod.rs`（或新 `channels/conversation.rs`）。用 `async-trait`。

```text
trait MessagingChannel (async, Send)
  fn id(&self) -> &str
  async fn open(&mut self) -> Result<(), String>           // 建连/校验（TG: 校验; 钉钉: 取token+开Stream）
  async fn send_message_prompt(&mut self, msg: &MessagePrompt, is_markdown: bool, source: &str)
  async fn ask_question(&mut self, q: &QuestionCtx<'_>, cancelled: &AtomicBool) -> Option<QuestionAnswer>
  async fn close(&mut self)                                 // 收尾/断连（被抢答或完成）
```

`QuestionCtx { header: &str, text: &str, options: &[String], is_markdown: bool, index: usize, total: usize }`
（`header` 规则见 1.2：单题无 Message 用来源头部；多题用 `Question i/n`。）

### 1.2 新增公共驱动 `run_conversation`

位置：`src-tauri/src/channels/conversation.rs`。把现 `telegram::run_session` 的编排逻辑搬过来，**去 Telegram 化**：

- `n==1 && !has_message`：单题特例，`header="「Question from {source}」"`，调一次 `ask_question`。
- 否则：`send_message_prompt` 共享 Message → 逐题 `ask_question`（`header = n>1 ? "Question i/n" : ""`）。
- 任一 `ask_question` 返回 `None`（被抢答）→ `close()` 并 return（不投递）。
- 全部完成 → `sink.submit(ChannelResult{Send, answers, source_channel_id=channel.id()})`。
- `has_message` 判定沿用：`!message.text.trim().is_empty() || !message.files.is_empty()`。

签名：`async fn run_conversation(ch: &mut dyn MessagingChannel, request: &AskRequest, cancelled: Arc<AtomicBool>, sink: ResultSink)`。

### 1.3 外层 `Channel` 适配（接 Coordinator 不变）

保留现有 `Channel` trait（`id/start/cancel_by_other`）。每个会话型渠道用一个**轻量外层结构**实现 `Channel`：
- 持有 `config + cancelled: Arc<AtomicBool>`。
- `start()`：`spawn` 一个 task —— 构造对应 `MessagingChannel`（从 config），`open()` 成功后调 `run_conversation`；`open()` 失败则警告并不投递（与现 Telegram 配置无效跳过一致）。
- `cancel_by_other()`：`cancelled.store(true)`（驱动里在 `ask_question` 轮询/等待点感知并 `close`）。

> 备注：未来可提供泛型适配器消除两份外层样板；当前 Telegram/钉钉 各一份薄外层即可。

### 1.4 Telegram 平迁

- 新增 `TelegramSession`（持 `TelegramClient` + 跨题 `offset` + 每题 message_id 状态）实现 `MessagingChannel`：
  - `open` 校验/构造 client；`send_message_prompt` = 现 `send_message_prompt`；`ask_question` = 现 `ask_question`（选项消息+操作消息+长轮询+✅切换+「发送」）；`close` 空操作。
- `channels/telegram.rs` 的 `TelegramChannel` 改为薄外层（`start` → `run_conversation`）。删除 `run_session`（逻辑入 `run_conversation` + `TelegramSession`）。
- 行为不变（验收第 7 条回归）。

---

## 2. 钉钉客户端层 `dingtalk/`（HTTP/OpenAPI）

新增模块目录 `src-tauri/src/dingtalk/`：

### 2.1 `dingtalk/token.rs`：access_token 缓存
- `get_token(client_id, client_secret) -> Result<String>`：`POST https://api.dingtalk.com/v1.0/oauth2/accessToken {appKey,appSecret}` → `accessToken`+`expireIn`；进程内缓存（`Mutex<Option<(token, expire_at)>>`），过期前（留 60s 余量）刷新。
- 同一 token 用于：新接口 header `x-acs-dingtalk-access-token`，旧 oapi query `access_token`。

### 2.2 `dingtalk/client.rs`：`DingTalkClient`（reqwest）
- `new(config) -> Result<Self, DingTalkError>`：校验非空（clientId/secret/userId）；构造 `reqwest::Client`。
- **robotCode 统一取 `clientId`（AppKey）**：所有需要 `robotCode` 的接口（oToMessages/互动卡片/messageFiles/download）内部用 `clientId`，不单独配置。
- `send_oto_text(text)` / `send_oto_markdown(title,text)`：`POST /v1.0/robot/oToMessages/batchSend`，body `{robotCode, userIds:[userId], msgKey, msgParam(JSON字符串)}`。
- `send_oto_image(media_id)`：`sampleImageMsg {photoURL:"@media_id"}`；`send_oto_file(media_id,name,ext)`：`sampleFile {mediaId,fileName,fileType}`。
- `upload_media(path, kind: image|file) -> media_id`：`POST https://oapi.dingtalk.com/media/upload?access_token=..&type=..`，multipart 字段 `media`（读盘字节 + filename）。返回 `media_id`。
- `send_card(card_biz_id, card_data_json) -> outTrackId`：StandardCard 单聊互动卡片，`callbackType=STREAM`，`singleChatReceiver={"userId":userId}`、`robotCode`、`cardBizId`、`cardData`。
  （候选接口：机器人互动卡片发送 `/v1.0/im/v1.0/robot/interactiveCards/send` 或卡片实例 `/v1.0/card/instances/createAndDeliver`；实现期取在「单聊 + Stream 回调」下可跑通者，两者回调/更新处理一致。记录 `outTrackId`/`cardBizId` 供更新。）
- `update_card(card_biz_id_or_track, card_data_json)`：点选后刷新卡片显示（更新接口对应上面所选发送接口）。
- `download_message_file(download_code) -> 本地路径`：`POST /v1.0/robot/messageFiles/download {downloadCode, robotCode}` → `downloadUrl` → GET 字节落地到临时目录，按消息真实类型修正扩展名。
- 错误类型 `DingTalkError`（EmptyXxx / Api(code,msg) / Network / BadResponse），中文 Display。

### 2.3 `dingtalk/card.rs`：StandardCard 卡片 JSON 构造
- `build_question_card(header, text, options, selected, is_markdown) -> cardData(JSON)`：
  - 头部（加粗 header）+ 正文（markdown 组件）+ 选项区（每个选项一个 button，`actionType=回传请求`，`params={kind:"toggle", value:<option>}`，选中态在 label 前加 ✅/变色）+ 「发送」button（`params={kind:"submit"}`）。
- 选项 → 按钮 id/params 映射，便于回调里解析点了哪个选项。
- 纯文本（`is_markdown=false`）时正文用普通文本组件，不渲染 markdown。

### 2.4 `dingtalk/stream.rs`：Stream 长连接
- `open_connection(client_id, client_secret, topics) -> (endpoint, ticket)`：`POST /v1.0/gateway/connections/open`，body `{clientId, clientSecret, subscriptions:[{type:"CALLBACK",topic}], ua, localIp}`。
- `connect(endpoint, ticket) -> WebSocket`：`tokio-tungstenite` 连 `endpoint?ticket=…`（wss + rustls）。
- 帧循环 `StreamConn`：
  - 解析帧 `{ specVersion, type, headers{topic,messageId,...}, data }`。
  - `type=SYSTEM`：`topic=ping` → 回 `{code:200, headers{messageId}, data}`；`CONNECTED/REGISTERED/KEEPALIVE` → 记录/标记活跃；`DISCONNECT` → 触发重连。
  - `type=CALLBACK/EVENT`：按 `headers.topic` 分发：
    - `/v1.0/im/bot/messages/get` → 解析 bot 消息（见 3.3）。
    - `/v1.0/card/instances/callback` → 解析卡片回调（`value.cardPrivateData.actionIds/params`、`outTrackId`、`userId`）。
    - **每条 3 秒内 ACK**：回 `{code:200, headers{messageId,contentType:"application/json"}, data:<json>}`。
  - 定时 WS ping；读错/`DISCONNECT` → 重连（重新 `open_connection` 拿新 ticket）。
- 对上层暴露：`async fn recv() -> StreamEvent`（`StreamEvent::BotMessage{...}` / `CardAction{...}`），内部封装 ACK/心跳/重连。
- **单连接约束**：一个 client-id 同一时刻一条 Stream（提问会话与「自动识别」串行复用或各自短连，避免并发）。

---

## 3. 钉钉会话渠道 `channels/dingding.rs`

### 3.1 外层 `DingTalkChannel`（实现 `Channel`，见 1.3）
- 持 `DingTalkChannelConfig + cancelled`；`start` → spawn → 构造 `DingTalkSession`、`open` → `run_conversation`；`cancel_by_other` 置 cancelled。

### 3.2 `DingTalkSession`（实现 `MessagingChannel`）
- 持有：`DingTalkClient`、`StreamConn`（在 `open` 建立并订阅两个 topic）、当前题状态（`selected`、`user_input`、本题 `outTrackId/cardBizId`、待收的图片/文件累积）。
- `open`：取 token + 建 Stream（双 topic）。失败 → `Err`（外层警告跳过）。
- `send_message_prompt`：
  - 文本/markdown 发 `oToMessages/batchSend`（头部「Question from {source}」+ 文本）。
  - `-f` 文件：逐个 `upload_media` → 图片 `send_oto_image`、其它 `send_oto_file`；失败 → 警告 + 发一条含文件名的失败提示文本（不中断）。
- `ask_question`：
  1. `build_question_card` → `send_card`（记 `cardBizId/outTrackId`）。
  2. 进入事件循环（直到 `cancelled` 或用户点「发送」）：从 `StreamConn.recv()` 取事件——
     - **卡片回调** 且属本卡片（`outTrackId` 匹配、`userId==配置userId`）：
       - `params.kind=="toggle"` → 切换 `selected` → `update_card` 刷新高亮。
       - `params.kind=="submit"` → 收尾，返回 `QuestionAnswer{selected, user_input, images, files}`。
     - **bot 消息** 且 `senderStaffId==userId` 且单聊：
       - `text` → 累积/覆盖 `user_input`。
       - `picture`/`richText(图片)` → `download_message_file` → 读字节转 base64 → 进 `images`（`ImageAttachment`）。
       - `file` → `download_message_file` → 落地路径进 `files`。
     - `cancelled` 置位 → 返回 `None`。
- `close`：断开 Stream。

### 3.3 bot 消息解析（关键字段）
- `senderStaffId`(=userId)、`conversationType`("1"=单聊)、`msgtype`(text/picture/file/richText)、`text.content`、图片/文件的 `downloadCode`、`robotCode`。

---

## 4. 配置、命令与设置页 UI

### 4.1 配置 `config.rs` / 类型 `types.ts`
- 新增 `DingTalkChannelConfig { enabled, client_id, client_secret, user_id }`（serde camelCase + `#[serde(default)]` 容错；不含 robotCode，内部取 clientId）。
- `ChannelsConfig` 增加 `dingding: DingTalkChannelConfig`。
- TS `types.ts` 同步 `DingTalkChannelConfig` + `ChannelsConfig.dingding`。
- `config.rs` 单测补充默认值。

### 4.2 命令 `commands.rs`（+ `app/mod.rs` 注册 + `ipc.ts`）
- `dingtalk_test(args) -> Result<String,String>`：取 token（校验 ClientId/Secret）+ `send_oto_*` 一条测试消息到 userId；缺 userId 给中文提示。
- `dingtalk_detect_userid(args) -> Result<String,String>`：
  - **前置校验**：`clientId`/`clientSecret` 为空 → 立即返回中文错误（如「请先填写 ClientId 和 ClientSecret」）；换 token / 建 Stream 失败 → 返回带原因的中文错误。**校验不通过则不进入识别流程、不展示 4 位数字提示**。
  - 校验通过后：随机 4 位 code（前端先取 code 展示，再调识别；或命令返回 code 后前端轮询，实现期取其一）；开短连 Stream（bot 消息 topic），等 `content==code` 的单聊消息，超时（~120s）→ 错误；成功返回 `senderStaffId`。
- 均为 `async` 命令（reqwest/WS）。

### 4.3 设置页 `SettingsView.vue`（「通信渠道」tab）
- 仿 Telegram 卡片：`enabled` 开关 + 字段 ClientId(AppKey) / ClientSecret(AppSecret) / UserId（无 RobotCode）。
- UserId 行旁置「自动识别」按钮：点击 → 先校验 ClientId/ClientSecret 是否已填（未填即在前端给出错误提示，复用 `result err` 展示，不继续）→ 展示生成的 4 位数字与提示「请私聊机器人发送：XXXX」→ 调 `dingtalk_detect_userid` → 成功回填 userId；后端返回的错误（key/secret 无效、超时等）同样以 `result err` 展示。
- 「测试连接」按钮 → `dingtalk_test`，复用现有 `result ok/err` 展示。

---

## 5. 运行编排泛化（`app/mod.rs`）

- 新增 `dingding_active(config)`（enabled 且 clientId/clientSecret/userId 三项非空且 client 可构造）。
- 抽 `active_messaging_channels(config) -> Vec<Arc<dyn Channel>>`：按 active 收集 `TelegramChannel` / `DingTalkChannel`。
- `run_ask`：`gui 可用 && popup_enabled` → GUI 路径（弹窗 + 全部 active messaging 渠道并行）；否则若存在 active messaging 渠道 → **泛化 headless**（无 GUI，注册并行运行全部 active messaging 渠道，Process 退出）；都无 → 报错 `EXIT_NO_CHANNEL`。
- 删除/替换 `run_headless_telegram` 为 `run_headless(channels)`；`launch` 的 setup 里注册 popup（如开）+ 遍历 active messaging 渠道 `register + start`。
- 退出码/抢答语义不变。

---

## 6. 依赖（`Cargo.toml`）

- `tokio-tungstenite`（default-features=false，启用 `connect` + rustls：`rustls-tls-webpki-roots`）。
- `futures-util`（WS sink/stream）。
- `async-trait`（`MessagingChannel`）。
- 复用：`reqwest`(json/multipart/rustls)、`base64`、`serde_json`、`tokio`、`uuid`（cardBizId/outTrackId）。
- 无需 hmac/sha2（单聊用 token 鉴权，不走自定义机器人加签）。

---

## 7. 涉及文件清单

新增：
- `src-tauri/src/channels/conversation.rs`：`MessagingChannel` trait + `run_conversation`。
- `src-tauri/src/channels/dingding.rs`：`DingTalkChannel` + `DingTalkSession`。
- `src-tauri/src/dingtalk/{mod.rs,token.rs,client.rs,card.rs,stream.rs}`。

改动：
- `src-tauri/src/channels/mod.rs`：导出新模块；`Channel` trait 保留。
- `src-tauri/src/channels/telegram.rs`：重构为 `TelegramSession`(MessagingChannel) + 薄外层。
- `src-tauri/src/config.rs` / `src/lib/types.ts`：钉钉配置。
- `src-tauri/src/commands.rs` / `src/lib/ipc.ts`：`dingtalk_test` / `dingtalk_detect_userid`（+ 注册）。
- `src-tauri/src/app/mod.rs`：active 判定、`active_messaging_channels`、headless 泛化、注册启动。
- `src/views/SettingsView.vue`：钉钉配置卡片 + 自动识别/测试。
- `src-tauri/src/prompts.rs` / `README.md`：文档。
- `src-tauri/Cargo.toml`：新依赖。

---

## 8. 任务顺序

1. **抽象重构**：`MessagingChannel` + `run_conversation`；Telegram 平迁；构建 + 回归（验收 7）。
2. **配置/类型**：`DingTalkChannelConfig`（Rust + TS）+ 默认值单测。
3. **钉钉客户端层**：`token` → `client`（发文本/图片/文件 + media 上传 + 下载）→ 单测可测部分（msgParam 组装、扩展名修正等纯函数）。
4. **Stream 长连**：`stream.rs`（open/connect/帧循环/ACK/心跳/重连）；先用 `dingtalk_test` 验证发、用「自动识别」验证收（bot 消息 topic）。
5. **互动卡片**：`card.rs` 构造 + 发送/更新 + 卡片回调解析；打通点选/发送。
6. **钉钉会话渠道**：`DingTalkSession` 串起发 Message / 逐题卡片 / 收文字图片文件 / 完成；接入 `run_conversation`。
7. **编排泛化**：`app/mod.rs` active 判定 + headless 泛化 + 注册启动。
8. **设置页 UI + 命令注册 + ipc/types**。
9. **文档**：`prompts.rs` / `README`。
10. **构建**（`--features custom-protocol`）+ 端到端实测（单聊收发、卡片点选、图片/文件双向、抢答、headless）。

---

## 9. 测试策略

- Rust 单测：`config` 默认值；`client` 纯函数（msgParam JSON 组装、`sampleFile` 扩展名、下载文件扩展名修正、card_data 选项→按钮映射、回调 params 解析）。
- 手动/端到端（需真实内部应用）：
  - 测试连接 / 自动识别 userId。
  - 单题 / 多题 / 带 Message + `-f` 文件（图片内联、文件可下载打开）。
  - 卡片多选高亮、文字补充、发图片/文件作答、点「发送」完成。
  - 与弹窗 / Telegram 抢答（钉钉先答 / 被抢答中止）。
  - headless（无 GUI）单跑钉钉。
  - Telegram 回归。

---

## 10. 风险与注意

- **单 Stream 约束**：同一 client-id 仅一条 Stream；提问会话与自动识别需串行/互斥，避免「踢连接」。
- **3 秒 ACK**：收到 CALLBACK 立即 ACK，再异步处理（卡片更新/下载），避免超时重推；对重复推送按 messageId 去重。
- **断线重连**：长等待期间 WS 可能断；需重连且不丢失「当前题等待中」状态（重连后继续等卡片回调/消息）。
- **卡片发送/更新接口选型**：StandardCard + STREAM 回调，发送/更新接口须配套（同族）；实现期以能跑通单聊 Stream 回调为准，二者择一并固定。
- **媒体大小限制**：上传 image/file ≤ 20MB、消息引用文件 ≤ 10MB；超限走失败提示，不崩溃。下载文件需按真实类型补扩展名。
- **token 并发刷新**：缓存加锁，避免并发重复换取。
- **抽象重构回归**：Telegram 平迁须逐项对齐（offset 跨题、message_id 边界、MarkdownV2 回退、✅ 切换、「发送」判定），以现有行为为准。
- **配置容错**：旧配置无 `dingding` 字段须走默认（`#[serde(default)]`）。
