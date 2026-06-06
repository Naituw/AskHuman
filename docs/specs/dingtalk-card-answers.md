# 需求：钉钉渠道支持「互动卡片预选答案」（高级版卡片 · A 方案落地）

> 状态：已确认（按计划实现）
> 关联计划：`docs/plans/dingtalk-card-answers.md`
> 关联既有文档：`docs/specs/dingtalk-channel.md` / `docs/plans/dingtalk-channel.md`（钉钉渠道总体设计；本需求落地其中「后续增强 · A 方案」）

## 1. 背景

钉钉渠道现状是 **B 方案「纯文本 + 编号选项」**：提问以文本下发，用户回一条消息（回编号 / 文字 / 图片 / 文件）作答。

之所以没做「点按钮预选答案」，根因是 **钉钉互动卡片「普通版（StandardCard）」不支持回调**（官方文档明确：普通版「互动卡片回调 = 不支持」；Stream 模式下按钮点击收不到）。

本需求落地 **A 方案**：改用 **互动卡片高级版** + **Stream 回调**（`callbackType=STREAM`，零公网），实现「卡片内勾选预选项 + 补充文字 + 点提交完成作答」。用户已在钉钉卡片平台**搭建并发布**好卡片模板。

## 2. 目标

启用钉钉并配置 ClientId / ClientSecret / UserId 后发起提问：

- 机器人单聊先发共享 Message（如有，含 `-f` 文件，沿用现有发送逻辑），随后**逐题以互动卡片下发**：卡片含标题、问题正文（Markdown）、可勾选的预定义选项（多选）、补充文字输入框、「提交」按钮。
- 用户在卡片内**勾选选项（多选高亮）**、可**补充文字**、作答期间还可在聊天里**发图片/文件**，点「提交」完成该题。
- 多题逐条进行；全部完成后结果回传 stdout（与弹窗/Telegram 同一契约）。
- 与弹窗 / Telegram **并行抢答**，语义不变。

## 3. 已确认决策

| 编号 | 决策项 | 结论 |
|---|---|---|
| DC1 | 作答 UX | 升级为**互动卡片高级版预选答案**（多选 + 补充文字 + 提交），走 **Stream 回调**。普通版不支持 Stream 回调，故必须高级版 |
| DC2 | 卡片模板 | 由用户在钉钉卡片平台**搭建并发布**（已完成，与应用同一 `clientId`）。模板变量：`title`(标题,公有) / `markdown`(正文,公有,Markdown) / `options`(对象数组,公有,每项 `{text}`) / `submitted`(布尔,私有) / `private_input`(字符串,私有)；本地变量 `user_input`、`selected_options`；「提交」按钮 `actionId="submit_action"`，回传 `user_input` + `selected_options` |
| DC3 | 配置项 | 新增 `cardTemplateId`（可空）。**默认即用卡片**：填了用填的；**留空用写死的默认 ID** `6cfe19d3-3b36-4681-827d-e7c1d0574d0a.schema` |
| DC4 | 投放接口 | `POST /v1.0/card/instances/createAndDeliver` 投放到机器人单聊：`openSpaceId="dtv1.card//IM_ROBOT.{userId}"` + `imRobotOpenSpaceModel:{supportForward:true}` + `imRobotOpenDeliverModel:{spaceType:"IM_ROBOT", robotCode:clientId}` + `callbackType="STREAM"` + `userIdType:1` + `outTrackId`(uuid) |
| DC5 | cardData 填充 | `cardData.cardParamMap` 复杂值转 JSON 字符串：`title`/`markdown`/`private_input` 为字符串；`submitted` 初始为 `"false"`；`options` 为 `[{"text":"选项"}]` 的 **JSON 字符串** |
| DC6 | 回调解析 | 订阅 topic `/v1.0/card/instances/callback`；回调 `data` 含 `userId`/`outTrackId`/`content`(或 `value`，为 JSON 字符串)；解析 `content.cardPrivateData.actionIds`(=`["submit_action"]`) 与 `params.{selected_options,user_input}`。按 `outTrackId` 匹配当前卡片、`userId==配置 userId` 归属。`selected_options` 为选项文本数组（**过滤空串**，兼容元素为字符串或 `{text}` 对象） |
| DC7 | 完成 + 回包 | 点「提交」即完成该题。**必须在卡片回调 3 秒内 ACK 时回包** `{cardUpdateOptions:{updatePrivateDataByKey:true}, userPrivateData:{cardParamMap:{submitted:"true"}}}`，否则卡片成功条件 `submitted==true` 不满足、用户会看到「提交失败」toast。Stream 层对**卡片回调改为延迟 ACK**（由会话层算出回包后再 ACK） |
| DC8 | 图片/文件 | 作答期间**累积**用户在聊天里发的图片/文件（bot 消息 topic），点提交时连同 `selected_options`+`user_input` 一并作为答案；聊天里的**纯文字忽略**（请用卡片输入框，避免双输入源冲突） |
| DC9 | 失败兜底 | 卡片投放失败（接口报错等）→ **自动回退**到现有「纯文本 + 编号选项」B 方案问该题（保留 B 方案代码） |
| DC10 | Stream 订阅 | 会话期 Stream 同时订阅 **bot 消息**(`/v1.0/im/bot/messages/get`) + **卡片回调**(`/v1.0/card/instances/callback`) 两个 topic |
| DC11 | 抢答/完成收尾 | 被抢答或完成后**尽力**把当前卡片置为「已提交/关闭」（best-effort 调更新卡片接口），失败不影响主流程 |
| DC12 | 测试连接 | `dingtalk_test` 维持发纯文本测试消息，不改 |
| DC13 | 已知问题 | 「同一 client-id 同一时刻仅一条 Stream」维持**不修**，仅记录（见 `docs/specs/dingtalk-channel.md` 末尾） |

## 4. 约束与既有规则（不可破坏）

- **stdout 洁净契约不变**：仍只输出 `[选择的选项]`/`[用户输入]`/`[图片]`/`[文件]`/`[状态]` 区块。
- **现有功能不回归**：弹窗、Telegram、抢答、配置容错（缺字段走默认、未知字段忽略）、`dingtalk_test`/`dingtalk_detect_*`、退出码（0/1/3）保持。
- **配置容错**：新增 `cardTemplateId` 用 `#[serde(default)]`；旧配置无该字段走默认。
- **release 构建**：生产构建须 `--features custom-protocol`；TLS 沿用 rustls，不引 OpenSSL。
- **前置条件（用户侧）**：卡片模板须在**同一应用（clientId）**下搭建并发布；应用需具备「互动卡片」相关能力/权限。

## 5. 验收标准

1. 启用钉钉并配置 client/secret/userId 后发起提问：机器人单聊先发 Message（如有，含 `-f` 文件），随后逐题以互动卡片下发（标题 / 正文 / 可勾选选项 / 补充输入框 / 提交按钮）。
2. 勾选多选即时高亮、可补充文字、点「提交」完成该题；提交后卡片变「已提交」灰态，**无「提交失败」toast**。
3. 作答期间在聊天发的图片/文件被收进答案；聊天里的纯文字被忽略。
4. 完成后 stdout 正确输出选项/文字/图片/文件区块；被弹窗或 Telegram 抢答时钉钉中止、不重复投递。
5. 卡片投放失败时自动回退到「纯文本 + 编号选项」仍可完成该题。
6. 设置页可填「卡片模板ID」（留空用默认）；其余钉钉配置/测试连接/自动识别不回归。
7. Telegram / 弹窗 / headless 行为不回归。

## 6. 反馈意见

（review 中产生的调整意见追加于此，标注日期。）
