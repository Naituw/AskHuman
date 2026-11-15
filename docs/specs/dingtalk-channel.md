# 需求：新增「钉钉」通信渠道（Channel）

> 状态：待确认（review 后按计划实现）
> 关联计划：`docs/plans/dingtalk-channel.md`

## 1. 背景

`AskHuman` 现已支持两个 Channel：本地弹窗（GUI）与 Telegram（headless / 与弹窗并行抢答）。
本需求新增第三个 Channel —— **钉钉（DingTalk）**，实现与 Telegram 同级的「**Agent 主动发问 → 人在钉钉作答 → 结果回传**」完整双向交互。

钉钉与 Telegram 的本质差异：钉钉机器人**无法像 Telegram 那样简单地长轮询收消息**，要在本地（无公网）收消息，必须走 **Stream 模式**（WebSocket 长连接）。因此本需求选用「**企业内部应用 + 机器人 + Stream 模式 + 单聊**」方案。

本需求同时要求：把现有「多 Channel / 多问题逐条发送」的公共逻辑**抽象出来**，让 Telegram 与钉钉复用，并为未来更多 Channel（飞书、企业微信等）预留扩展点。

## 2. 目标

用户在设置页「通信渠道」中配置钉钉（ClientId / ClientSecret / RobotCode / UserId）并启用后：

```bash
AskHuman "请看看这个改动？" -f ./diff.patch -q "要继续吗？" -o "继续" -o "停止"
```

- 钉钉机器人**主动私聊**用户：先发共享 Message（含 `-f` 文件），再逐题发**互动卡片**（可点选选项 + 「发送」按钮）。
- 用户在钉钉里**点选选项**（多选高亮）、可**补充文字**、也可**发图片/文件**，点「发送」完成该题。
- 多题逐条进行；全部完成后结果回传到 stdout（与弹窗/Telegram 同一契约）。
- 与弹窗 / Telegram **并行抢答**：任一渠道率先完成即采纳，其余收尾。
- 无 GUI 时，钉钉可作为 headless 渠道单独工作。

## 3. 已确认决策

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 渠道能力 | **完整双向交互**（不是单向通知）：可发可收，在钉钉内完成作答 |
| D2 | 接入形态 | **企业内部应用 + 机器人 + Stream 模式**（WebSocket 长连接，零公网/零域名/零证书）；用户需有钉钉组织开发权限，自建内部应用拿 ClientId(AppKey)/ClientSecret(AppSecret) 并发布机器人 |
| D3 | 会话场景 | **单聊**（人与机器人私聊），无需 @；接收消息的 `conversationType="1"` |
| D4 | 作答 UX | **互动卡片普通版（StandardCard）**：`cardTemplateId` 固定填 `StandardCard`，**无需在卡片平台搭建/发布模板**；卡片内容用 `cardData` JSON 下发；选项做成「回传请求」按钮（多选、点选高亮）+ 一个「发送」按钮收尾。回调走 Stream（`callbackType=STREAM`），**无需公网回调地址、无需注册 callbackRouteKey** |
| D5 | 用户配置项（3 项 + 开关） | `enabled` / `clientId`(AppKey) / `clientSecret`(AppSecret) / `userId`；**robotCode 不单独配置**，内部统一取 `clientId`（企业内部应用机器人 robotCode = AppKey） |
| D6 | 鉴权 | `POST https://api.dingtalk.com/v1.0/oauth2/accessToken {appKey,appSecret}` → `accessToken`（有效期 7200s），进程内缓存 + 过期前刷新；同一 token 兼容新接口（header `x-acs-dingtalk-access-token`）与旧 oapi 接口（query `access_token`） |
| D7 | 发送共享 Message | 文本/Markdown 用单聊主动发：`POST /v1.0/robot/oToMessages/batchSend`，`robotCode + userIds:[userId] + msgKey:"sampleMarkdown"/"sampleText" + msgParam` |
| D8 | 发送题目（互动卡片） | 用 StandardCard 互动卡片发到单聊：`singleChatReceiver={"userId":..}` + `robotCode` + `cardBizId`（幂等 ID）+ `cardData`(JSON) + `callbackType=STREAM` |
| D9 | 选项点选 | 卡片选项按钮点击 → Stream 卡片回调 → 切换选中态 → 调**更新卡片**接口刷新卡片显示（选中项打勾/高亮），与 Telegram 的 ✅ 切换体验一致 |
| D10 | 收消息（Stream） | `POST /v1.0/gateway/connections/open {clientId,clientSecret,subscriptions:[...]}` → `endpoint+ticket` → 连 `wss://endpoint?ticket=…`；订阅两个 topic：机器人消息 `/v1.0/im/bot/messages/get`（文字补充 / 图片 / 文件 / userId 识别）与卡片回调 `/v1.0/card/instances/callback`（按钮点选/发送） |
| D11 | Stream 心跳/ACK/重连 | 响应 `SYSTEM` ping（回 code 200）；每条 `CALLBACK` **3 秒内 ACK**（带回原 messageId）；定时 WS ping；断线重连（重新 open 拿新 ticket）。同一 client-id **同一时刻只起一条 Stream** |
| D12 | 每题完成方式 | 卡片上点「发送」按钮即完成该题（点选切换选项、其间可发文字/图片/文件补充）；与 Telegram「发送」语义一致 |
| D13 | 作答-接收图片/文件（人→AI） | **支持**：用户在钉钉发的图片(`picture`/`richText`)、文件(`file`)消息，按 `downloadCode` 调 `POST /v1.0/robot/messageFiles/download {downloadCode,robotCode}` 换临时 `downloadUrl` 下载到本地（需按真实类型修正扩展名），图片进回答 `[图片]`、文件进回答 `[文件]`（与弹窗回答契约一致） |
| D14 | 提问-发送 `-f` 文件（AI→人） | **支持上传**：先 `POST https://oapi.dingtalk.com/media/upload?access_token=..&type=image|file`（multipart，字段名 `media`）拿 `media_id`；图片用 `sampleImageMsg {photoURL:"@media_id"}`、文件用 `sampleFile {mediaId,fileName,fileType}` 发到单聊；上传/发送失败 → 警告并发一条含文件名的失败提示，不中断流程 |
| D15 | userId 识别 | 设置项 userId 可手填；旁置「自动识别」按钮：点击后程序**随机生成一个 4 位数字**并提示「请私聊机器人发送：XXXX」，经 Stream 捕获 `content==XXXX` 的单聊消息，取其 `senderStaffId` 精确回填 userId（带超时） |
| D16 | 测试连接 | 校验 ClientId/Secret 能换到 token，并给配置的 userId **单聊发一条测试消息**，成功返回提示 |
| D17 | 抢答与退出 | 接入现有 Coordinator「首个终态生效，其余 `cancel_by_other` 收尾」；钉钉被抢答 → 关闭 Stream、不投递；退出码语义不变（0/1/3） |
| D18 | 公共抽象 | 抽出「会话型消息渠道」公共接口与「多问题逐条发送」公共驱动逻辑，Telegram 与钉钉复用；为飞书/企业微信等未来 Channel 预留实现点 |
| D19 | headless 泛化 | 把现「仅 Telegram 的 headless 路径」泛化为「任一/多个会话型渠道的 headless 运行」，无 GUI 时钉钉可单独或与 Telegram 并行工作 |
| D20 | 文档同步 | 设置页 UI、`prompts.rs`、`README` 同步说明钉钉配置与使用 |

## 4. 约束与既有规则（不可破坏）

- **stdout 洁净契约不变**：结果仍只输出 `[选择的选项]`/`[用户输入]`/`[图片]`/`[文件]`/`[状态]` 区块；钉钉答案经同一 `render_result` 聚合输出。
- **现有功能契约不变**：弹窗、Telegram、抢答、配置容错（缺字段走默认、未知字段忽略）、`--settings/--help/--version`、退出码（0/1/3）保持。
- **release 构建模式**：生产构建须 `--features custom-protocol`，不回退该修复。
- **新增依赖**：允许新增 WebSocket 相关 crate（`tokio-tungstenite`(rustls) / `futures-util` / `async-trait`）；TLS 沿用 rustls，不引入 OpenSSL。
- **单 Stream 约束**：同一 ClientId 同一时刻仅一条 Stream；提问会话与「自动识别 userId」不应同时占用同一连接（实现需互斥/串行）。

## 5. 验收标准

1. 设置页可填 ClientId/Secret/RobotCode/UserId 并启用钉钉；「测试连接」能换到 token 且收到一条单聊测试消息。
2. 「自动识别」给出 4 位数字，按提示私聊后 userId 被精确回填。
3. 启用钉钉 + 弹窗后发起提问：钉钉机器人私聊先发 Message（含 `-f` 文件，图片可内联、文件可下载），再逐题发互动卡片。
4. 卡片点选选项即时高亮（多选）；可发文字补充；可发图片/文件作为回答；点「发送」完成该题；多题依次进行。
5. 完成后 stdout 正确输出选项/文字/图片/文件区块；被弹窗或 Telegram 抢答时钉钉中止、不重复投递。
6. 无 GUI 时仅启用钉钉也能完成整套问答（headless）。
7. Telegram 行为与现状一致（抽象重构不回归）。
8. 设置页、`prompts.rs`、`README` 反映钉钉用法。

## 6. 反馈意见

（review 中产生的调整意见追加于此，标注日期。）

### 2026-06-04 review 调整

- **去掉单独的 `robotCode` 配置项**：企业内部应用机器人的 `robotCode` 即应用 AppKey，故配置精简为 `clientId`/`clientSecret`/`userId` 三项 + 开关；所有需要 `robotCode` 的接口内部统一取 `clientId`。（用户实测时本就没有单独的 robotCode，正与此一致。）
- **「自动识别 userId」需前置校验**：未填写有效的 ClientId/ClientSecret 时点击「自动识别」必须报错提示，且不进入识别流程、不展示 4 位数字提示；后端命令对空值/换 token 失败返回中文错误，前端以错误样式展示。

### 2026-06-04 实现期调整与已知问题

- **作答 UX 由「互动卡片」改为「纯文本 + 编号选项」（B 方案）**：官方明确「互动卡片**普通版（StandardCard）不支持 Stream 模式回调**，请用 HTTP 模式或改用高级版」。普通版能发能渲染，但**按钮点击回调收不到**，无法在零公网（Stream）下做快捷按钮。故 D4/D8/D9/D12 的卡片点选方案暂不采用，改为：提问以「头部 + 正文 + 编号选项 + 作答提示」纯文本/Markdown 下发，用户**回复一条消息即完成该题**——回复编号（多选用逗号，如 `1,3`）映射预定义选项，或直接输入文字，或发送图片/文件。卡片相关代码（`dingtalk/card.rs`、`client::send_card/update_card`、卡片回调 topic）以 `#[allow(dead_code)]` 保留备用。
- **高级版互动卡片（A 方案）列为后续增强**：要做到「点按钮 = 快捷回复」且走 Stream，须改用**高级版互动卡片**（`创建并投放卡片` + `callbackType:"STREAM"` + 注册 `/v1.0/card/instances/callback`），但需**先在钉钉后台配置一个卡片模板**（一次性）。待确有需要再实现。

### 2026-06-06 A 方案已落地

- 「互动卡片高级版预选答案」已实现，成为钉钉提问的默认形态；卡片投放失败时自动回退本节的 B 方案（纯文本 + 编号选项）。详见 `docs/specs/dingtalk-card-answers.md` 与 `docs/plans/dingtalk-card-answers.md`。
- **【已知问题 · 暂不修】同一 client-id 同一时刻仅允许一条 Stream**：官方排查清单明确「同一个 client-id 同一时间只允许启动一个 Stream 服务，多开会相互干扰」。当前实现是**每次 AskHuman 进程各自开一条 Stream**（同一 client-id）。正常「一次只问一题」无碍，但**连续快速 / 并发提问时**多条 Stream 会抢消息，可能把用户回复投递到错误的连接。经确认**暂不修，仅记录**；未来修复路线二选一：① **文件锁串行化**——同一时刻只允许一个进程持有 Stream，退出时发 Close 帧干净断连再释放锁（轻量，无常驻进程；每次提问需重新建连，并发会排队）；② **常驻 daemon**——后台进程独占持有 Stream，各 AskHuman 经本地 socket 注册等待、由其转发用户消息（真复用 / 连接常热 / 支持并发，但需管理 daemon 生命周期 + IPC，复杂度高）。
