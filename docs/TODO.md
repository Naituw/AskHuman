# 待办 / 已知问题

记录暂不处理但需跟踪的问题与后续增强。

## 已知问题（钉钉渠道）

### 1. 同一 client-id 同一时刻仅允许一条 Stream（多开相互干扰）

> ⚠️ **由 daemon 架构修复中**：本问题正由 `docs/specs/daemon-architecture.md`（Phase 2：IM 渠道迁入 Daemon + 长连接单实例复用）根治。**待 daemon 架构需求全部开发完成后，删除本条目。**

- **根因**：钉钉官方限制——同一个 client-id 同一时间只允许启动一条 Stream 服务，多开会相互干扰（见官方排查清单）。
- **现状**：当前每次 `AskHuman` 进程各自开一条 Stream（同一 client-id）。正常「一次只问一题」无碍，但**连续快速 / 并发提问时**多条 Stream 会抢消息，可能把用户回复投递到错误的连接。
- **候选修复**：
  1. 文件锁串行化——同一时刻只允许一个进程持有 Stream，退出时发 Close 帧干净断连再释放锁（轻量、无常驻进程；每次提问需重新建连，并发会排队）。
  2. 常驻 daemon——后台进程独占持有 Stream，各 `AskHuman` 经本地 socket 注册等待、由其转发用户消息（真复用 / 连接常热 / 支持并发，但需管理 daemon 生命周期 + IPC，复杂度高）。← **采用此方案（daemon 架构）。**
- **状态**：修复中（daemon 架构 Phase 2）；完成后删除本条目。

## 已知问题（daemon 分支回归，待修）

### 钉钉卡片「提交」误报『请求失败』toast（实际已成功）

> 🐞 **现象**：daemon 架构下点击钉钉互动卡片的「提交」按钮，客户端弹出红色「请求失败」提示，但答案**其实已成功送达**（AI 侧正常拿到回应，`daemon.log` 对应 request 显示 `done`）。仅卡片端误报，功能不受影响。

- **根因**：daemon 架构里钉钉长连接由 `dingtalk/router.rs` 的 Reader 独占，Reader 收到卡片回调后**对所有回调一律回空包 `{"response":{}}`**（当初按 spec §11 / Q-A① 的「即时空 ACK」）。但钉钉**互动卡片「提交」按钮**要求那条 3 秒内的同步回包必须是**非空的成功/更新回包**，否则客户端判定提交失败并弹红条。
- **对比旧单进程行为**（`channels/dingding.rs` @ 35efac6）：提交回调的同步回包带了
  `cardUpdateOptions{updateCardDataByKey,updatePrivateDataByKey} + cardData.submit_status(本地化文案) + userPrivateData.submitted="true"`（成功 + 即时置灰）；**只有选项切换等非提交回调才回空 `{}`**。Router 把这两类合并成「全回空」，于是提交按钮报失败。
- **证据**：`daemon.log` 中对应 request 均 `accepted`→`done`；toast 为卡片端误报。
- **推荐修复**（忠实还原旧行为、保持 Router 架构不变）：
  1. Router 区分回调类型（用 `card::parse_card_submit(&data).is_some()` 判定）：
     - **提交回调** → 同步回**成功包** `{"cardUpdateOptions":{"updatePrivateDataByKey":true}, "userPrivateData":{"cardParamMap":{"submitted":"true"}}}`（满足 3 秒 + 消除失败提示 + 提交者按钮即时置灰）；
     - **非提交回调（选项切换等）** → 仍回空 `{}`（与旧版一致）。
  2. 卡片**公有终态文案**（「已提交」/「已在 X 回答」）继续由会话经 OpenAPI `update_card_private` 写入（即 Q-A① 的异步置灰，仅这部分文案保留 ~0.1–0.3s 延迟）。
  - 备选（方案②）：会话经 oneshot 把完整回包回传 Reader，由 Reader 在 3 秒内同步写回，连公有文案也即时——代价是 Reader↔会话耦合，不推荐。
- **影响范围**：仅钉钉。飞书走不同的 ack 模型（卡片更新靠 OpenAPI `patch_card`，空 ack 通常不报错），Telegram 用 `answerCallbackQuery`，二者预计不受影响，但修复后建议一并回归。

## daemon 架构：待补充的人工实测

> 已通过的真机实测（install 后经新 daemon→GUI Helper 链路）：① 单题弹窗作答（退出 0）；② **并发两请求弹窗不串台**（A→A、B→B）；③ 取消返回 `[Status]` 再问指引（退出 0）；④ `daemon status` 显示 running 且 `im conns: dingtalk, feishu, telegram`（三连接常热单实例）。下列为尚未逐项跑过、建议后续补做的人工测试：

- [ ] **真实 IM 并发（真 TODO#1）**：同时发起两个请求 → 分别在钉钉 / 飞书 / Telegram 的卡片上作答，验证：(a) 回复不串台（按 `outTrackId`/`open_message_id`/callback `message_id` 路由到正确请求）；(b) 自由文字归属正确（Telegram 归「最新活动卡片」、钉钉聊天按 `senderStaffId`、飞书按 `open_id`）；(c) 同一 client_id 仅一条长连接、无多开互抢。
- [ ] **被抢答 / 跨渠道抢答**：一个请求同时挂多渠道（弹窗 + IM），在某一渠道作答后，其余渠道卡片应即时置灰为「已在 X 回答」（走 OpenAPI `updateCard`/`patchCard`）。
- [ ] **Phase 3 实时配置（验收 #7）**：弹窗开着时修改 `config.json` 的主题 / 语言（或在设置窗口改并保存）→ 验证打开中的弹窗**实时切换**主题/语言（daemon `config_watch` → `ConfigChanged` → 前端 `settings-updated`）。
- [ ] **Phase 3 凭据热重载（惰性失效）**：修改某渠道凭据 / 禁用某渠道 → 观察 `daemon.log` 出现 `config reloaded`；下一个请求按新配置重连（旧缓存 Router 被丢弃），进行中的请求保留其原连接直到结束。
- [ ] **临时目录清理（A10）**：确认 `temp/askhuman/<id>/` 中超过 24h 未改动的目录会在 daemon 启动时 / 每小时被清理，且不会误删刚产出的图片。
- [ ] **生命周期**：空闲超时自动退出；`daemon stop/restart` 正常；二进制指纹换新（重装后旧 daemon 自动让位、新 daemon 接管）。
- [ ] **自动识别 userId/open_id（Q6）**：设置窗口点「自动识别」→ 经 daemon `Detect`：若已有同 app 长连接则复用观察（零冲突），否则 daemon 临时开连；非 Unix 走进程内回退。

## 后续增强 / 性能优化

### A. 钉钉卡片「变灰」延迟（daemon 架构引入）

- **背景**：daemon 架构下钉钉长连接由 Router 独占共享，卡片回调由 Router 即时空 ACK（满足 3 秒约束），卡片置灰（「已提交」/「已在 X 回答」）改走 OpenAPI `updateCard`（见 `docs/specs/daemon-architecture.md` §11）。
- **影响**：相比单进程时代「stream 同步回包即时变灰」，现在变灰是一次独立 HTTPS 调用，慢约 100–300ms（仅视觉延迟，功能一致）。
- **可选优化**：如需即时变灰，可让会话经 oneshot 把回包回传给 Router 的 Reader，由其在 3 秒内用长连接写回（代价：Reader↔会话耦合、Reader 读循环需短暂等待会话算回包）。当前判断收益有限，暂不做。
