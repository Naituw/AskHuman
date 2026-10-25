# 开发计划：iOS 原生渠道（Mac Gateway + 推送中继 + iOS App）

> 关联需求：`docs/specs/ios-channel.md`（决策编号 D1–D24 均指该文档 §3）
> 前置资料：`docs/investigations/native-ios-channel-handoff.md`（EMQX 杭州实测、原型）
> 关联既有：`docs/plans/slack-channel.md`（渠道接入范式）、`docs/specs/popup-request-inbox.md`、`docs/specs/agent-interject.md`、`docs/specs/im-agent-task-launch.md`、`docs/plans/daemon-graceful-drain.md`
> 计划描述方案与技术/规则细节，具体代码以实现为准。**开工前**读 `docs/agent-worktree-setup.md` 并按其要求用 AskHuman 询问是否挂渠道预设（iOS 渠道不可预设，故推荐「不挂」）。

## 0. 方案总览

```
iOS App ──MQTT(TLS 8883 / WSS 8084)── EMQX Serverless（杭州）──MQTT── Mac Daemon
 │  └ 通知扩展(NSE)                                                    └ ios Gateway
 │        ▲                                                                 │ HTTPS：句柄 + 密文
 └─ APNs ◄┴──────── HTTP/2 + JWT ◄──── 推送中继（腾讯云云函数，Node） ◄──────┘
```

核心五件事：

1. **Mac 端 `ios` Gateway**（Daemon 内常驻任务，`src-tauri/src/ios/`）：MQTT 传输、配对、端到端加密协议、状态投影，以及对 watch / 插话 / 任务启动现有能力的复用；
2. **`IosChannel`**（`channels/ios.rs`，实现 `Channel`）+ `confirm::start_ios`：接入 Coordinator「首个终态获胜」；
3. **推送中继**（`relay/`，Node 零依赖，部署腾讯云云函数）：持有 APNs 密钥，Mac 只拿句柄；
4. **iOS App**（`ios/`：SwiftUI + 通知扩展 + 本地 SPM `AskHumanKit`）；
5. **跨语言协议夹具 + 分阶段验收**，P0 先用真机与真实网络过门禁，再写产品代码。

关键不变量：

- 权威状态始终在 Mac Daemon；手机是遥控与展示端，答案是否被采纳以 Coordinator 的 `try_submit` 为准。
- 实时通道只在手机前台保持；后台只靠 APNs；点击通知后由前台连接补同步。
- 所有业务消息端到端加密；broker 与中继只见密文和最小元数据。
- 状态变更类消息（答案、确认、插话、新任务）一律带 `clientMsgId` 并在 Mac 端持久化去重。

## 1. 交付物与仓库结构

```
src-tauri/src/ios/            Mac 端 Gateway（见 §4）
src-tauri/src/channels/ios.rs IosChannel
relay/                        推送中继（Node ≥ 20，零 npm 依赖，node --test）
  src/core.js                 句柄编解码、请求校验、载荷构造、JWT；运行时无关（只依赖 Node 内置 `crypto`，不碰 HTTP 与环境变量）
  src/adapters/scf.js         腾讯云 Web 函数适配：HTTP 服务（监听 9000）+ 环境变量 + HTTP/2 发 APNs
  test/*.test.js
  scripts/build-zip.sh        产出 scf 上传包（含 scf_bootstrap）
ios/
  project.yml                 XcodeGen（*.xcodeproj 已被 .gitignore 忽略）
  AskHuman/                   App target（SwiftUI，iOS 17.0+，仅 iPhone）
  AskHumanNotification/       通知扩展 target
  Packages/AskHumanKit/       本地 SPM：AHProtocol（加密/信封/消息/模型）、AHTransport（MQTT 适配）、
                              AHStore（SwiftData + Keychain）、AHMarkdown（渲染）、AHMock（Mock 后端）
  Fixtures/wire-v1/           跨语言夹具（Rust 与 Swift 测试共用，唯一来源）
  Probes/                     P0 门禁探针（脚本与最小工程，不属于产品 target）
  scripts/                    build / test / 模拟器辅助脚本
docs/wiki/ios-setup.md(.en.md) 用户配置指南（EMQX、腾讯云、Apple、配对）——P8
```

- 通知扩展只链接 `AHProtocol`（内存预算约 24 MB）；`AHTransport`（CocoaMQTT）不进扩展。
- 依赖（iOS）：CocoaMQTT（emqx，固定版本，含 `CocoaMQTTWebSocket` 用于 WSS 备用）、swift-markdown（Apple，固定版本）。其余用系统框架：CryptoKit、Network、LocalAuthentication、PhotosUI、AVFoundation / VisionKit（扫码）、UserNotifications、SwiftData。
- 依赖（Rust，新增）：`rumqttc`（`use-rustls` + `websocket`）、`chacha20poly1305`、`x25519-dalek`（`static_secrets`）、`hkdf`、`hmac`、`qrcode`；复用已有 `sha2`、`base64`、`reqwest`、`tokio`、`uuid`、`serde_json`。P0 必须验证与现有依赖的 rustls 加密后端一致（避免进程内出现两个 `CryptoProvider`）、三平台可编译、记录 release 体积增量。

## 2. 标识与常量

| 名称 | 定义 |
|---|---|
| `macId` / `phoneId` | 16 字节随机，小写 base32（无填充，26 字符）；Mac 的存于 `~/.askhuman/ios/state.json`（Dev Instance 各自独立），手机的存于钥匙串且跨重装保留 |
| `ticketId` / `ticketSecret` | 16 / 32 字节随机，一次性，5 分钟有效 |
| `kid` | 密钥编号；配对密钥为 1，配对通道的票据密钥为 0，预留轮换 |
| `epoch` | Gateway 每次启动生成的随机 64 位，随快照下发；用于识别 Mac 重启 |
| `rev` | 每个 (epoch, stream) 单调递增，stream ∈ inbox / agents / follow / slot |
| `agentKey` | `"{kind}:{sessionId}"`（kind 为 claude / codex / cursor / grok / pi） |
| `msgId` | 16 字节随机 base64url；消息级去重窗口 2048 条 / 10 分钟 |
| `clientMsgId` | 手机生成的 UUID；Mac 端幂等日志保留 30 天、最多 5000 条 |
| MQTT clientId | `mac-{macId}` / `ios-{phoneId}`（同 id 会被 broker 踢旧连接，故不得复制 `~/.askhuman`） |

限额（Mac 与手机共用常量，写入 `wire` 模块与 Swift `AHProtocol`）：

| 项 | 值 |
|---|---|
| 信封最大 | 900,000 字节（EMQX 单条 1 MB） |
| 明文消息最大 | 512 KB；请求正文超限则尾部截断并置 `truncated` |
| `request.new` 内联正文 | ≤ 128 KB 内联完整正文，否则只发摘要，由手机 `request.get` 拉取 |
| 缩略图 | 单张 ≤ 24 KB JPEG，随请求下发 |
| 附件分块 | 原始 ≤ 192 KB / 块；拉取与上传单文件 ≤ 10 MB；每题回复附件 ≤ 5 个；图片长边 ≤ 2048 px |
| Mac 发布节流 | 令牌桶 6 msg/s、突发 10（EMQX 单客户端 10 msg/s） |
| Watch 帧 | 每 (手机, 会话) ≤ 1 帧 / 2 秒，≤ 8 KB（`text` 从头部截断保留最新） |
| 同时实时观察 | 每部手机 ≤ 2 |
| Follow | 每台 Mac 每部手机 ≤ 5 |
| MQTT keepalive | 30 秒；重连退避 1,2,4,8,15,30 秒（上限 30，带抖动） |
| 推送回退 | 手机在线但 3 秒内无 `request.ack` → 补发推送 |
| 句柄 | 有效期 180 天，手机满 90 天刷新 |
| 配对 | 票据 5 分钟；`pair.done` 2 分钟内未到则清理半成品配对 |

## 3. 线协议 v1

### 3.1 话题与 ACL

命名空间 `ah/{u}/`，`{u}` = broker 用户名（用户名只用字母数字，不得为 `+` `#` `+/#`）。

| 话题 | 方向 | 说明 |
|---|---|---|
| `ah/{u}/d/{phoneId}` | Mac → 手机 | 下行；手机订阅自己的；QoS 1，不保留 |
| `ah/{u}/u/{macId}` | 手机 → Mac | 上行；Mac 订阅自己的；QoS 1，不保留 |
| `ah/{u}/s/{deviceId}` | 双向 | 在线状态，保留消息 + 遗嘱（LWT）；明文 `{"o":0|1,"t":unix,"r":"mac"|"phone","fg":0|1,"e":<epoch>}`，`fg` 仅手机，`e` 仅 Mac（Gateway 本次启动的随机 epoch，不含任何内容） |
| `ah/{u}/p/{ticketId}` | 双向 | 配对通道，仅配对期间订阅；用票据密钥加密 |

订阅数：Mac ≤ 3（`u/{macId}`、`s/+`、配对期 `p/{ticketId}`），手机 ≤ 3（`d/{phoneId}`、`s/+`、配对期 `p/{ticketId}`），均在 EMQX 的 10 个订阅上限内。
EMQX Serverless 控制台配置（见 §10）：认证加一组用户名 / 密码；授权：「用户名」规则允许该用户名发布 / 订阅 `ah/<该用户名>/#`，「全部用户」规则兜底「拒绝 `#`」。实例的授权模式默认是黑名单（未命中规则放行），所以这条兜底必须有。实测采用的是每个用户名一条字面规则（而不是「全部用户」下的 `${username}` 占位符规则），已用 `ios/Tools/broker-probe/acl-check.cjs` 验证隔离，记录见 `docs/investigations/ios-channel-p0.md`。
会话：MQTT 3.1.1，`clean session = true`，不依赖 broker 排队；离线期间的状态一律靠重连后的快照对齐。

### 3.2 信封与加密

信封（UTF-8 JSON）：`{"v":1,"f":"<发送方 id>","k":<kid>,"n":"<base64url 12B>","c":"<base64url 密文‖标签>"}`。`z` 字段保留给未来压缩，v1 出现即拒绝。

- AEAD：ChaCha20-Poly1305，每条消息 12 字节随机 nonce；AAD = `"ah1|" + 完整话题 + "|" + f + "|" + k`；推送信封的 AAD 为 `"ah1|push|" + f + "|" + k`。
- 明文消息：`{"i":<msgId>,"t":<type>,"ts":<毫秒>,"b":<body>}`。
- 方向密钥：Mac → 手机用 `k_m2p`，手机 → Mac 用 `k_p2m`；接收方按 `f` 找配对、按 `k` 找密钥，找不到或认证失败一律静默丢弃（只记计数，不记内容）。
- **重放防护**：明文消息的 `ts` 与接收方本地时间相差超过 15 分钟则丢弃（上行消息由 Mac 回 `error{code:"clock_skew"}`，下行由手机忽略并在 Mac 管理页提示校准时间）；`hello` 带随机 `nonce`，`hello.ack` 回显，手机只采纳与未完成 `hello` 匹配的 `hello.ack`，并只接受该 `hello.ack` 中 `epoch` 对应的快照与 delta。幂等日志的保留期（≥ 30 天）远大于时间窗口，二者合起来使 `task.create` 等状态变更类消息无法被重放。
- 配对密钥派生：
  - `k_t = HKDF-SHA256(ikm = ticketSecret, salt = "ah1-ticket", info = ticketId, L = 32)`，仅用于配对通道；
  - `ss = X25519(eMac_priv, ePhone_pub)`（双方均为本次配对的临时密钥）；
  - `okm = HKDF-SHA256(ikm = ss, salt = ticketSecret, info = "ah1-pair" ‖ 0x00 ‖ macId ‖ 0x00 ‖ phoneId ‖ 0x00 ‖ ticketId ‖ 0x00 ‖ eMac_pub ‖ ePhone_pub, L = 96)`，依次切出 `k_m2p`、`k_p2m`、`k_sas`；
  - 校验码 = `HMAC-SHA256(k_sas, "ah1-sas")` 前 4 字节按大端取整后对 1,000,000 取余，补零到 6 位；
  - 编码约定：`macId` / `phoneId` / `ticketId` 取其文本的 UTF-8 字节，`ticketSecret` 与两把临时公钥取原始 32 字节；夹具里给出每一步的十六进制中间值。
- 临时私钥配对完成后立即销毁；`k_*` 存钥匙串。

### 3.3 配对协议

配对码（二维码内容，也可复制为文本）：`ahpair1.` + base64url(JSON)，JSON 字段：协议号 `v`、broker 主机 `h`、TLS 端口 `p`、WSS 端口 `w`、用户名 `u`、密码 `pw`、中继地址 `r`、`macId` `m`、Mac 名称 `mn`、`ticketId` `t`、`ticketSecret` `ts`、Mac 临时公钥 `ek`、过期秒时间戳 `x`。

流程（所有 `p/{ticketId}` 消息用 `k_t`、`k = 0`）：

1. Mac：生成票据与临时密钥对，订阅 `p/{ticketId}`，展示配对码；
2. 手机：解析并连上 broker（凭据来自配对码）；生成 / 读取 `phoneId`、临时密钥对；订阅 `d/{phoneId}` 与 `p/{ticketId}`；发 `pair.request {phoneId, name, ek, appVer, proto}`；本地立即算出校验码并显示；
3. Mac：解密成功 → 票据置为已消费（之后同票据的请求一律忽略，攻击者对 6 位校验码最多只有一次猜测机会）→ 在 `p/{ticketId}` 回 `pair.pending {macId}` → 算出同一校验码 → 弹确认（手机名称 + 校验码 + 允许 / 拒绝）；
4. 拒绝 / 超时：Mac 在 `p/{ticketId}` 发 `pair.reject {reason}`，丢弃一切；允许：Mac 先把配对（状态 `pendingConfirm`）与密钥落盘，再在 `d/{phoneId}` 用 `k_m2p`（`k = 1`）发 `pair.accept {macId, macName, caps}`；
5. 手机：解密 `pair.accept` 成功（证明两端派生一致）→ 写钥匙串 → 经中继登记句柄 → 在 `u/{macId}` 用 `k_p2m` 发 `pair.done {name, handle?, env, expiresAt}`；
6. Mac：收到 `pair.done` → 配对转 `active`，保存句柄；2 分钟内未收到 → 删除半成品配对。

### 3.4 消息目录

通用约定：下行消息带 `rev` 的 stream 须按 §3.5 校验；所有「提交类」上行消息带 `clientMsgId`，Mac 以对应 `*.ack` / `*.status` 回应（含重复提交时返回首次结果）。

**握手与同步**

| 类型 | 方向 | body 要点 |
|---|---|---|
| `hello` | 手机→Mac | `nonce, proto, appVer, deviceName`；手机在所有订阅的 SUBACK 到达后发送：进入前台 / 重连 / 看到某 Mac 的在线状态由离线翻为在线或其 `e` 与已知不同时 |
| `hello.ack` | Mac→手机 | `nonce, epoch, macName, caps{newTask, interject, watch}, slot{active, autoActivation}`；随后依次发 `inbox.snapshot` / `agents.snapshot` / `follow.state` / `slot.state`；手机收到 `hello.ack` 并应用快照后才发布 `fg=1` 的在线状态 |
| `error` | Mac→手机 | `ref(msgId 或 clientMsgId), code(clock_skew/unknown_type/not_paired/rate_limited/internal), message?`；用于无法给出业务级 ACK 的拒绝 |
| `resync` | 手机→Mac | `stream`；Mac 回对应快照 |
| `ping` / `pong` | 双向 | RTT 估算（诊断用） |
| `mac.info` | Mac→手机 | Mac 重命名 |
| `unpair` / `unpaired` | 双向 | 一端解除配对（尽力通知） |
| `push.register` | 手机→Mac | `handle, env, expiresAt`，句柄变化时发送 |
| `push.state` | Mac→手机 | `ok / missing / gone / error`，提示手机刷新句柄 |

**请求与作答**

| 类型 | 方向 | body 要点 |
|---|---|---|
| `inbox.snapshot` | Mac→手机 | `epoch, rev, items[RequestSummary]`（id、kind、项目、Agent、创建时间、预览、`contentHash`、大小） |
| `request.new` | Mac→手机 | `rev, request: RequestView, full`；`full=false` 时只含摘要 |
| `request.get` / `request.full` | 手机→Mac / Mac→手机 | 拉取完整 `RequestView` |
| `request.ack` | 手机→Mac | 手机已持久保存该请求（取消推送回退） |
| `request.seen` | 手机→Mac | 请求已在 UI 打开（用于替换通知与未读） |
| `request.resolved` | Mac→手机 | `rev, requestId, how(answered/cancelled/expired/answeredElsewhere), by` |
| `answer.submit` | 手机→Mac | `clientMsgId, requestId, action(send/cancel), answers[{q, sel[选项下标], text?, images[uploadId], files[uploadId]}]` |
| `answer.ack` | Mac→手机 | `clientMsgId, requestId, status(accepted/lost/expired/invalid), reason?` |
| `confirm.decide` | 手机→Mac | `clientMsgId, requestId, wireIndex 或 dismiss=true, input?` |

`RequestView`：`id, kind, createdAt, agent{kind,label}, project{key,name}, source, contentHash, truncated`；
- 提问：`markdown, message{text, files[AssetRef]}, questions[{text, options[{text, recommended, todo?}]}], selectOnly, single, whatsNext`；
- 确认：`title, context[{id,label,value,kind}], detail{summary, bodyMd}, choices[{wireIndex,label,description,role,recommended}]`（只下发推荐变体）、`defaultIndex?, input?{id,label,placeholder,maxChars,visibility}, dismissIndex, expiresAt`；
- `AssetRef = {assetId, name, size, mime, isImage, thumb?, tooLarge?}`。

**Agent、Follow、Watch、插话**

| 类型 | 方向 | body 要点 |
|---|---|---|
| `agents.snapshot` / `agents.delta` | Mac→手机 | `AgentView{key, kind, kindLabel, sessionId, title, project, state(working/idle/waiting/ended), startedAt, lastActivityAt, activeElapsedSecs, turnSteps, currentTool?, waitingRequestId?, waitingPreview?, pendingInterject, lastTurnFailed, canInterject{ok, reason?}, followed}`；delta 为 upsert / remove，500 ms 合并，仅发给前台手机 |
| `follow.set` / `follow.state` | 手机→Mac / Mac→手机 | 开关关注；Mac 校验 ≤ 5 并回当前集合 |
| `follow.event` | Mac→手机 | `agentKey, kind(done/waiting/failed), preview`；手机前台时用，后台走推送 |
| `watch.open` / `watch.close` | 手机→Mac | 开始 / 停止实时帧；每手机 ≤ 2 |
| `watch.frame` | Mac→手机 | `agentKey, seq, frame`：沿用 `frame_detail_json` 的字段（phase、title、project、text、steps、todos、activeElapsedSecs…）并追加 `failed`，≤ 8 KB |
| `watch.closed` | Mac→手机 | `agentKey, reason(ended/limit/notFound)` |
| `interject.send` | 手机→Mac | `clientMsgId, agentKey, text, files[uploadId]` |
| `interject.ack` | Mac→手机 | `status(queued/delivered/rejected), reason(notWorking/unsupported/empty/...), pending` |
| `interject.clear` / `interject.query` / `interject.state` | 手机→Mac / Mac→手机 | 撤回、取待送达全文 |
| `interject.consumed` | Mac→手机 | 排队插话被 hook 消费（对应「已送达 Agent」） |

**新任务与活跃槽**

| 类型 | 方向 | body 要点 |
|---|---|---|
| `task.options.get` / `task.options` | 手机→Mac / Mac→手机 | `enabled, blocked?{reason}, permissionMode(ask/agentDefault/yolo), workspaces[{id,label,subtitle,pinned,lastUsedAt}], agents[{kind,label,ready,subtitle,permissionSelectable,reason?}]`；工作目录 `id` 为规范路径哈希，Mac 缓存 30 分钟 |
| `task.todos.get` / `task.todos` | 同上 | `workspaceId, todos[{id,text,auto,attachments}], overflow`（≤ 10 条；`attachments` 为附件数；Mac 连同附件快照缓存 30 分钟） |
| `task.create` | 手机→Mac | `clientMsgId, workspaceId, agentKind, permission(default/yolo), todoId?, text` |
| `task.status` | Mac→手机 | `clientMsgId, stage(accepted/launching/launched/watching/unmatched/failed), reason?, agentKey?` |
| `slot.take` | 手机→Mac | `clientMsgId`，等价 `/here` |
| `slot.state` | Mac→手机 | `rev, active, autoActivation` |

**附件**

| 类型 | 方向 | body 要点 |
|---|---|---|
| `asset.get` | 手机→Mac | `assetId, from, count(≤ 8)` |
| `asset.chunk` / `asset.error` | Mac→手机 | `assetId, index, total, size, sha256(首块), data` / `reason` |
| `upload.begin` | 手机→Mac | `uploadId, name, mime, size, chunks, sha256, scope{requestId 或 agentKey}` |
| `upload.chunk` | 手机→Mac | `uploadId, index, data`；每 4 块一次 `upload.ack` |
| `upload.ack` | Mac→手机 | `uploadId, nextIndex, status(ok/complete/error), reason?` |
| `upload.cancel` | 手机→Mac | 放弃上传 |

### 3.5 同步、顺序与幂等

- 手机对每个 stream 记 `(epoch, rev)`：`epoch` 相同且 `rev == last+1` 才应用 delta；`rev <= last` 丢弃；出现缺口或 `epoch` 变化（Mac 重启）且收到的不是快照 → 发 `resync`。快照整体替换该 stream。
- Mac 在 `hello` 之后、以及自己重连成功后，主动向在线手机补发全部快照。快照与 delta 都由同一发送队列串行化。
- **去重**：接收方按 `msgId` 去重（窗口见 §2）。
- **幂等日志**（Mac，`~/.askhuman/ios/journal.json`，0600，原子写）：键 `clientMsgId` → `{phoneId, type, 请求内容 sha256, 结果, 时间}`。
  - 重复且内容相同：直接重发首次结果，不重复执行；
  - 重复但内容不同：回 `invalid`（`reason=conflict`）；
  - 新消息：先执行并把结果写入日志，再回 ACK（ACK 即「Mac 已持久化」）；
  - `task.create` 先落 `started` 再启动终端，完成后更新为 `done`；重启后遇到 `started` 但无结果的，回 `failed(reason=interrupted)`，不自动重试。
- **答案校验**（Mac）：请求必须在待答集合内且已投递给该手机；`selectOnly` 拒绝文字与附件、每题须有选项；`single` 每题至多一项；下标越界、附件 `uploadId` 不属于该请求 / 手机一律 `invalid`。
- 手机的待发消息放本地发件箱，30 秒内未收到 ACK 则标记失败并允许重试（同 `clientMsgId` 重发）。Mac 离线时不入队（见需求 §7）。
- **四种「已送达」**：手机已保存 → `*.ack` 到手（Mac 已持久化）→ `answer.ack accepted`（Coordinator 已采纳）→ `interject.consumed`（Agent 已消费）；UI 逐级区分，不把 MQTT PUBACK 或 APNs 200 当成后两者。

### 3.6 在线状态与推送回退

- 手机进入前台并完成 `hello` 后发布保留消息 `{"o":1,"fg":1}`；进入后台前先发 `{"o":0}` 再断开（用 `beginBackgroundTask` 保证发出）；连接异常断开由 LWT 发布 `{"o":0}`。Mac 同样发布 `{"o":1,"r":"mac","e":<epoch>}` 与遗嘱（`{"o":0,"r":"mac"}`），优雅退出时先主动发布 `{"o":0}`。
- Mac 投递规则（每个手机独立判断）：
  - `fg=1`：经 MQTT 发 `request.new`，3 秒内未收到 `request.ack` 则补发推送；
  - 否则：不走 MQTT，直接发推送（手机回前台后靠 `hello` 快照看到请求）。
- 同一 (请求, 手机) 至多发一次提醒推送；请求被别处处理时，若该手机收到过提醒且未 `request.seen`，再发一条同折叠标识的被动级别「替换推送」。

### 3.7 附件传输

- Mac 维护 `AssetRegistry`：`assetId` → `{path, size, mime, isImage, requestId}`。登记来源只有：待答请求的 `message.files`，以及该请求 Markdown 中引用的本地图片（绝对路径或项目相对路径、必须存在、是图片、每请求 ≤ 20 张）。请求结束后保留 10 分钟（手机可能还在看）。
- `asset.get` 只服务登记过的资产；超过 10 MB 的文件在 `AssetRef` 里置 `tooLarge`，不下发内容。分块带窗口（每次最多 8 块），首块附整文件 sha256，手机整体校验。
- 上传：`upload.begin` 校验 scope 合法、大小 ≤ 10 MB、磁盘空间足够，写入既有「回复附件」临时目录规则下的 `ios-uploads/<uploadId>/`（0700 / 0600，24 小时清理）；最后一块到达时校验 sha256；答案提交时：图片读入并转为 `ImageAttachment`（base64），非图片把绝对路径放进 `QuestionAnswer.files`；30 分钟内未被引用的上传、以及所属请求已结束的上传被清理。

## 4. Mac 端

### 4.1 配置、密钥与存储

- `config.rs`：`ChannelsConfig` 增 `ios: IosChannelConfig { enabled, host, port(8883), wsPort(8084), username, password, relayUrl, macName }`（camelCase、`#[serde(default)]`、旧配置缺字段可加载）；`password` 加入 `SECRET_SPECS`（数组长度 5 → 6，账号 `channels.ios.password`，`secrets.rs` 增常量，命名与现有 `channels.<渠道>.<字段>` 一致；账号必须等于配置字段的点号 JSON 路径，CLI 的脱敏与明文回退都按这个路径查，由 `every_secret_account_is_the_json_path_of_its_field` 守住）；`ChannelsConfig.default()` 与单测补齐。传输自动选择：先 TLS，TCP 5 秒内不通改 WSS，并记住最近成功的方式（存 `state.json`）。
- `~/.askhuman/ios/state.json`（0600，原子写）：`macId`、配对列表 `{phoneId, name, aliasName?, createdAt, lastSeenAt, state(pendingConfirm/active), followSet[agentKey], push{handle, env, expiresAt, status}}`、流量月累计、最近传输方式。`journal.json`：幂等日志（§3.5）。
- 配对密钥 `{kid, m2p, p2m}`：钥匙串账号 `ios.pairing.<phoneId>`（经 `secrets.rs`）。钥匙串不可用（Linux 无 Secret Service、`ASKHUMAN_NO_KEYCHAIN`、Dev Instance 模式）时回退到 `~/.askhuman/ios/keys.json`（0600，明文），并在设置页明确标注「配对密钥以明文保存」。
- 不在日志里输出任何 token、句柄、密钥、消息明文；只记计数、类型、大小与错误码。

### 4.2 传输层 `ios/transport.rs`

- `BrokerTransport` trait：`connect / publish(topic, payload, retain, qos) / subscribe / events() / disconnect`；实现：`RumqttcTransport`（TLS，失败回退 WSS）与测试用的进程内 `LoopbackTransport`（模拟 broker：话题匹配、保留消息、遗嘱、可注入丢包 / 乱序 / 断线）。
- 重连退避与保活见 §2；认证失败（连接被拒 / 未授权）→ 健康表报「凭据错误或 EMQX 额度可能已用尽」并按 60 秒退避；其它网络错误按常规退避。
- 发布经全局令牌桶节流；入站 / 出站字节计入流量统计（月累计，落 `state.json`）。
- 不实现 HTTP 代理穿透（P0 在公司网络实测；若 8883 与 8084 都被封，停下来请用户决定）。

### 4.3 Gateway `ios/gateway.rs`

- Daemon 内一个长期任务，随 `channels.ios` 配置变更热重建（`invalidate_changed_routers` 同类逻辑）；持有：传输、配对表、在线状态表、`PendingRequest` 表、Follow 状态、资产注册表、上传表、发布队列。
- 入站流水线：解析信封 → 按 `f` 找配对 → AEAD 解密（失败静默丢弃）→ `msgId` 去重 → 按 `type` 分派到处理器（hello、resync、request.*、answer.submit、confirm.decide、follow.*、watch.*、interject.*、task.*、slot.take、asset.get、upload.*、push.register、unpair、ping）。
- 出站：每手机一个有序发送队列，快照与 delta 串行；仅向 `fg=1` 的手机发 delta / watch 帧 / follow.event，离线手机靠 `hello` 快照对齐。
- `epoch` 在启动时生成；`rev` 在内存中单调递增。
- 生命周期：新增 `ensure_ios_gateway(state)`（幂等：按配置创建 / 重建 / 停止），与 `ensure_inbound_listeners` 在同样的位置调用（`serve()` 启动后台、`on_config_changed`、受理请求处）；`enabled` / broker 主机端口 / 用户名 / 密码 / 中继地址任一变化即重建。优雅退出（空闲退出、drain、stop）前先发布 `{o:0}` 并断开，崩溃由遗嘱兜底。
- Gateway 对外接口：`IosGateway::pending_requests` / `deliver(request)` / `resolve(request, how)` / `on_agent_event(kind, session, event, failed)` / `on_slot_changed(prev, new)` / `notify_consumed(phone, session)` / `live_watch_count()`，由 `ServerState` 持有 `Option<Arc<IosGateway>>`。

### 4.4 视图投影 `ios/views.rs`

- `RequestView` 来自 `AskRequest` + `ConversationOrigin`（Agent、项目）；`OptionItem.todo_id` 标出 TODO 选项。**答案映射**：手机返回选项下标，Mac 还原为 `selected_options`（选项原文，与 IM 一致，TODO 出队由现有 `todos::ids_to_dequeue` 按原文恢复），`text` → `user_input`，图片 / 文件按 §3.7 转成 `ImageAttachment` / 路径。
- 确认 `RequestView` 来自 `ConfirmRequest`，经现有 `ChoiceFormView`（只取推荐变体、`wire_index`、默认项、输入规则）。
- `AgentView` 来自 `agents_snapshot_for_gui`（含 `pendingInterject`、`waitingRequestId`、`waitingPreview`）+ Gateway 内的「最近一轮是否失败」小表（`turn-start` 清除）；`canInterject` 复用 `is_working_non_grok`（含 Codex Desktop 连接态）。
- Watch 帧由 `watch::build_frame` + `frame_detail_json` 的字段投影；Idle 时不 tick。
- 任务选项复用提取出的 `task_readiness`（§4.7）。

### 4.5 `IosChannel` 与确认接入

- `channels/ios.rs`：`IosChannel::shared(gateway)`，`id() = "ios"`。
  - `start(request, origin, sink)`：建 `RequestView` → 登记 `PendingRequest`（持有 `sink` 与答案映射所需的选项表）→ 按 §3.6 对每部已配对手机投递 → 立即返回；答案由 Gateway 入站处理器调用 `sink.try_submit(ChannelResult{ action, answers, source_channel_id:"ios" })`，据返回值回 `answer.ack(accepted/lost)`。
  - `interrupt(reason)`：移出待答表，向所有手机发 `request.resolved`，必要时发替换推送。
  - 投递面丢失：`start` 时若 5 秒内 broker 仍未连通 → `sink.surface_lost("ios", "broker unreachable")`；等待期间断线不上报。
- `channels/confirm.rs::start_ios(entry, gateway)`：与 `start_feishu` 同构的一个 spawn 任务。`gateway.deliver_confirm(entry)` 登记 `PendingConfirm`（持有 `Arc<ConfirmEntry>`）并按 §3.6 投递；至少投到一部手机 → `entry.mark_ready("ios", requestId)`（返回 false 表示请求已终结 → 立即发 `request.resolved`）；否则 `fail(&entry, "ios", 原因)`。手机的 `confirm.decide` 由 Gateway 入站处理器直接调用 `entry.coordinator.submit_wire(index, comment, "ios")`（`Ok` → `confirm.ack accepted`，`Err` → `invalid`，已终结 → `lost`）。任务本身只等 `entry.cancel.notified()`，触发后向各手机发 `request.resolved`（文案取 `final_status`）并清理。`entry.deadline` 到期由 Coordinator 终结，不另设计时器；等待期间 broker 断线不判失败。
- 候选渠道（`daemon/runtime/mod.rs`）：
  - `app::is_ios_active(config)` 只检查 `enabled` 且 broker / 中继字段齐全（纯配置，同其它渠道）；`any_im_enabled` 增 `ios.enabled`。
  - `available_im_channels(config)` 是纯配置函数，而「至少一个 `active` 配对」属 Gateway 状态，所以增加参数 `ios_ready: bool`，由调用方从 `state.ios` 取（Gateway 在运行且至少一个 `active` 配对）；两者都满足才追加 `ios`，顺序排在 `slack` 之后。
  - `attach_im_channels` 与 `confirm_im_candidates` 里的 `watching` 集合追加 `ios`（有手机正在 Follow 该请求所属的 Agent 会话时；仅自动激活开时才影响结果）。
  - `attach_im_channels` 新增 `candidates.contains(&"ios")` 分支：`attach_channel(entry, Arc::new(IosChannel::shared(gateway)), …)`；Gateway 缺失时回一条 Warn 并跳过（与路由器启动失败同样处理）。`attach_confirm_im_channels` 新增 `ios` 分支调用 `start_ios`，失败走 `entry.mark_failed("ios", …)` 与 `fallback(NoAvailableChannel)`。
  - iOS 渠道只在 Daemon 内运行，无单进程回退。

### 4.6 Follow、通知触发与 `AgentEvent.failed`

- `ipc::ClientMsg::AgentEvent` 增 `failed: bool`（`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`，与 `interject_poll` 同写法）；**不升协议号**。
- 判定：新增纯函数 `agents::stop::turn_failed(kind, &input)`（与 `is_natural_stop` 同文件、同口径）：Claude / Grok 的 `hook_event_name` 忽略大小写等于 `StopFailure`；Cursor 的 `status` 等于 `error`（`aborted` 是用户中断，不算失败）；Pi 的 `stop_reason` 等于 `error`；Codex 恒为 false。
- 现有 turn-end 有三条上报路径，都要覆盖：
  1. `agents/report.rs::run`（`__agent-hook`；覆盖 Claude / Grok 的 `StopFailure`，以及 Cursor 未启用 Stop 确认时的 `stop`）：对 `TurnEnd` 事件按 stdin 计算 `failed`；超大 stdin 时的精简解析结构 `HookInputSummary` 补 `stop_reason` 字段并在 `into_value` 里还原。
  2. `agents/stop.rs::run_inner`（`__stop-hook`；Cursor 启用 Stop 确认时 `stop` 事件走这里）：在现有 `!is_natural_stop` 分支计算 `failed`；`report_simple_event` 增加 `failed` 参数，其余调用点（自然完成、确认后结束）传 false。
  3. Pi 扩展模板 `resources/integrations/pi/index.ts`：`agent_settled` 里非自然完成分支的 turn-end 上报目前不带 `stop_reason`，改为一并带上 `run?.stopReason`。模板内容变化会让已安装扩展按现有机制被判为 `outdated`（`status()` 比对渲染文本），用户在设置里更新后生效；更新前扩展不带 `stop_reason`，Pi 的出错回合按 `failed = false` 处理，即被当作「完成」通知（无法与自然完成区分）。
- `AgentRegistry` 状态机、GUI、IM 行为均**不变**；daemon 里解构 `ClientMsg::AgentEvent` 的位置随字段增加同步更新。
- daemon 在 `apply_event` 之后调用 `gateway.on_agent_event(kind, session_id, event, failed)`（Gateway 未启用时零开销）。Gateway 维护每个 Agent 的小状态机，向**正在 Follow 该 Agent 的手机**生成：
  - 完成：`turn-end`（非失败）→ 3 秒去抖，期间若出现 `turn-start` / `activity` 则取消；
  - 出错：`turn-end` 且 `failed` → 立即；
  - 等待你：该 Agent 的会话出现在途提问 / 权限 / Stop 确认，且该请求**没有**投给 ios（Gateway 的待答表里没有）→ 2 秒后发一次；检测点挂在 `waiting_badge_tick`（它本来就比对「等待回答」集合的变化），预览取 `waitingPreview`；
  - 摘要取自 `watch::build_frame` 的 `text`（最后一段助手文字）前 120 字符与标题。
- 「完成 / 出错」只来自生命周期 `turn-end` 事件：Codex 的错误 / 中断没有 hook，既不会误报完成也不会报错；不经 hook 上报的会话（例如只被 Codex 桌面应用通道发现的会话）不触发这两类通知（需求 §12 第 15 项已列出）。
- 手机前台 → `follow.event`（MQTT）；否则 → 提醒推送。Follow 集合持久化在 `state.json`；Agent 会话结束后静默移除；从手机创建的任务匹配到会话后自动加入（名额满则跳过并在 `task.status` 里说明）。
- Follow 与 IM watch 订阅互不影响；`auto_end_watch` 对 Follow 无效。

### 4.7 复用 watch / 插话 / 任务启动

- **Watch**：Gateway 维护 `ios_focus` 表（手机、`agentKey`、上次签名）。`watch.rs` 的引擎循环现在由 IM 订阅与 GUI 焦点两个节奏来源决定睡眠时长，再加第三个来源 `ios_focus`（有观察中的 Agent 在工作 → 2 秒，否则 10 秒，没有观察 → 纯等 Notify），并在 `watch_tick` 里紧跟 `gui_focus_tick` 调用 `ios_focus_tick`：用 `build_frame` + `signature` 门控，签名变化才发，每会话 ≤ 1 帧 / 2 秒，字段投影沿用 `frame_detail_json`。观察中（`live_watch_count() > 0`）阻止 daemon 空闲退出（加进空闲守卫的条件，与 `has_agent_subs` 并列）；Follow 与普通 MQTT 连接不阻止。
- **插话**：`interject.send` → `InterjectStore::append(session, text, attachments, Some("ios:<phoneId>"))`；仅「工作中且非 Grok」可用（复用 `is_working_non_grok`，含 Codex Desktop 连接态），其余回 `rejected` 与原因；返回 0 表示已交给等待中的 hook（`delivered`，不登记回执），>0 表示排队（`queued`，登记回执渠道）。调用后与 IM `/msg` 一样 `persist()` 并 `broadcast_agents_state`。`spawn_read_receipts` 增加 `ios:` 前缀分支：调用 `gateway.notify_consumed` 发 `interject.consumed`。撤回走 `interject.clear`（同样不回执）。
- **新任务**：从 `inbound.rs` 抽取两个与渠道无关的函数，IM 与 iOS 共用：
  - `task_readiness(state, config)`：封装 `start_new_task_flow` 的门禁（`agent_tasks.enabled`；keepalive + 登录项；终端或 Codex Desktop 可用；`workspaces::refresh()` 非空；`agent_launch::all_readiness()` 至少一个就绪），返回 `Result<{workspaces, ready agents, diagnostics}, BlockReason>`；
  - `launch_agent_task(state, source, request, report)`：封装启动尾部（`resolve_task_input`、TODO 交付 `prepare_delivery_consistent`、`codex_desktop::resolve_launch` / `Operation::Create` 或 `agent_launch::create_record_with_files` + `register_pending_launch_watch` + `open_terminal`、TODO 出队、失败清理）；`report` 是回调，IM 传 `reply_channel_text`，iOS 传 `task.status` 发送。
  - `PendingLaunchWatch` 增内部字段 `ios: Option<{phoneId, clientMsgId}>`：匹配到会话后走 Follow 加入分支（不创建 IM watch 卡）；60 秒未匹配发 `task.status(unmatched)`。
  - 选项规则与 IM 流程一致：工作目录取 `workspaces::refresh()` 且 `!hidden`、目录仍存在（置顶优先，其余按最近使用）；Agent 中 Codex 以 `codex_desktop::launch_status(false).target` 是否存在判断可选（副标题取其 `description(lang)`），其余按 `all_readiness()` 的 `ready`（副标题 `integration_mode · executable`，不可选的给原因）；Pi 没有权限步骤（固定 Agent 默认）；权限步骤按 `agent_tasks.permission_prompt`（Ask 时手机二选一、不预选）；TODO 取 `project::detect_from(workspace)` 的 `todos::list` 前 `MAX_OPTION_TODOS`（10）条并给出超出数量；不提供 Codex 终端 / 桌面应用的目标选择（`target = None`，由 `resolve_launch` 按 Mac 设置决定，与 IM 默认路径一致）。
  - `task.create` 校验：`workspaceId` 必须出自最近一次 `task.options`、`todoId` 必须出自最近一次 `task.todos`（用缓存的附件快照作 `prepare_delivery_consistent` 的 `expected`，与 IM 一致）；提交时重跑 `task_readiness`；任务 ≤ 3000 字符、无 NUL（沿用 `resolve_task_input`）；接受后 `set_active_channel(state, "ios")`（对应 IM `/new` 把来源渠道设为活跃槽，不受自动激活开关影响）。不支持 fork 流程。

### 4.8 推送客户端 `ios/push.rs`

- 每部手机一份句柄；`push.register` 更新句柄；有效期到期前 30 天、`push.state=gone` 时手机会刷新。
- 构造推送明文 `{"k":"ask|confirm|follow.done|follow.waiting|follow.failed|resolved","rid":…,"mac":…,"agent":…,"project":…,"text":…(≤120 字符),"ts":…}`，用 `k_m2p`（AAD `ah1|push|<macId>|<kid>`）加密成 `ah = {f,k,n,c}`（≤ 3 KB），POST 中继 `/v1/push`。
- 超时 5 秒；5xx / 网络错误重试 1 次（带抖动）；4xx 不重试；`410 gone` → 标记该手机推送失效，经下一次 MQTT 连接通知手机，并在设置页显示「推送不可用」。同一折叠标识的待发推送合并。

### 4.9 IPC、命令、CLI、设置页

- **IPC**：新增 `ClientMsg::Ios { op: IosOp }` / `ServerMsg::Ios { result: IosResult }`（范式同 `PermissionRules`：`client::ios_op(op)` 对应 `permission_rules_op`，daemon 侧新增 `handle_ios_op`）。除 `State` 外的 op 与 `permission_rules_op` 一样会拉起 Daemon；`State` 不拉起（设置页轮询不应顺带启动 Daemon），Daemon 未运行时由客户端直接读 `state.json` 返回「Daemon 未运行」的离线视图（配对列表可看，不能配对）。`IosOp`：`State`、`PairStart`、`PairCancel`、`PairDecision{ticketId, allow}`、`Revoke{phoneId}`、`Rename{phoneId, name}`。`IosResult::State` 含：启用与连接状态、最近错误、`macId` / `macName`、配对列表（名称、时间、在线、推送状态）、当前票据（配对码、过期时间、待确认的手机名称与校验码）、流量统计。设置页在有票据时约 1 秒轮询 `State`，否则约 3 秒。
- **协议号 3 → 4**：新增枚举变体属「旧端可能收到」，按 `docs/overview.md` 的规则 +1：改常量；在 overview 的「历史」补一行（日期、提交、变更点）；在 `docs/plans/daemon-graceful-drain.md` 记下升级过渡期影响。`AgentEvent.failed` 是可选字段，不触发 +1。「测试连接」在设置进程内直接执行（同 `channel test`），不经 IPC。
- **命令**（`commands.rs` ↔ `lib/ipc.ts` ↔ `types.ts`）：`ios_state`、`ios_pair_start`、`ios_pair_cancel`、`ios_pair_decision`、`ios_revoke`、`ios_rename`、`ios_test`（broker 回环 + 中继 `/v1/health`）、`ios_export_connection` / `ios_import_connection`（连接串 `ahconn1.` + base64url(JSON{host, port, wsPort, user, pass, relay})，复制 / 导入）。二维码由 Rust `qrcode` 生成 SVG 交给前端。
- **设置页**：`ExperimentalTab.vue` 现在只有空状态占位，改为放置「iOS 渠道」卡，内容见需求 §9；i18n 中 / 英；Dev Instance 下卡内提示「实例使用独立配对」。`experimental.enabled` 只控制该 Tab 是否可见，不影响 Gateway 是否运行（与现有语义一致，Gateway 只看 `channels.ios.enabled`）。
- **CLI**：`channel set|enable|disable|test ios`（`channel_cmd.rs` 的 `is_enabled` / `is_configured` / `conn_name` / `apply_flags` / 向导增 `ios` 分支；密码与连接串经 `--password-{env|file|stdin}` / `--connection-{env|file|stdin}`，不接受明文参数）、`ios pair`（终端二维码 + 校验码 y/N）、`ios devices`、`ios revoke`。内测期隐藏（D22）：`CHANNELS` 数组保持 4 项，`channel list` 与 help 只在 `experimental.enabled` 时才多列 `ios`，命令本身始终可用。

### 4.10 Daemon 生命周期、健康与触点

- **空闲退出**：只有「手机正在实时观察」计入阻止项（见 §4.7）；iOS 渠道不强制 keepalive，不安装登录项。
- **健康**：`channels::health::report("ios", …)` / `clear("ios")`（渠道 id 是自由字符串，无需改枚举）上报：凭据 / 额度、broker 不可达、中继不可用、推送句柄失效；Tray 的 `channel_issues` 自然带出（展示名经 `channel_label`）。
- **硬编码渠道名触点清单**（逐项加 `ios` 分支或确认无需）：

| 位置 | 处理 |
|---|---|
| `daemon/runtime/mod.rs` | `any_im_enabled`、`available_im_channels(…, ios_ready)`、`attach_im_channels` / `attach_confirm_im_channels` 的 `ios` 分支与 `watching` 集合、`confirm_im_candidates`、`active_im_connections`（连通时上报 `ios`）、`invalidate_changed_routers` 与 `on_config_changed`（`ios` 配置变化重建 Gateway）、`ServerState` 新字段 `ios`、`serve()` 启动时 `ensure_ios_gateway`、`AgentEvent` 解构与 `apply_event` 之后的钩子、空闲守卫（观察中不退出） |
| `daemon/runtime/inbound.rs` | `build_im_channel("ios")`（补推用）；`reply_channel_text` 对 `ios` 返回 Err（无文本通道，调用方已忽略错误）；`set_active_channel` 末尾调用 `gateway.on_slot_changed`；`spawn_read_receipts` 的 `ios:` 前缀分支；`task_source_target("ios")`；抽取 `task_readiness` / `launch_agent_task`；`ensure_inbound_listeners` 旁挂 `ensure_ios_gateway` |
| `daemon/runtime/watch.rs` | `PendingLaunchWatch.ios` 与匹配 / 超时分支；引擎循环增 `ios_focus` 节奏来源；`watch_tick` 调 `ios_focus_tick`；`waiting_badge_tick` 通知 Gateway |
| `autochannel.rs::channel_label`、`app/coordinator.rs::display_name`、`channels/confirm.rs::source_name` | `ios` → 「手机」/「iPhone」展示名（i18n `channel.sourceIos`） |
| `watch::channel_supported` | `ios` 返回 false（不使用 IM watch 卡） |
| `cli/channel_cmd.rs` | 见 §4.9 |
| `dev_presets.rs` | `redact_channels` 的脱敏键增 `ios` 的 `password`；`configured_channel_ids` / `extract_configured_channels` 不含 `ios`（渠道预设不携带 iOS），加测试断言 |
| `ipc/mod.rs` 注释、`channels/health.rs` 注释 | 渠道 id 列表的注释补 `ios` |
| `confirm/transport.rs`、`daemon/runtime/{select,todo,fork,detect}.rs`、IM 卡片 / 命令代码 | 不涉及（IM 卡片发送、pickers、fork 流程、自动检测，iOS 不使用） |
| 设置前端 | `ChannelsTab` / `useChannels` / `context` / `types` 不增 iOS（首期只在「实验」Tab，转正时再迁） |

## 5. 推送中继（腾讯云云函数）

### 5.1 形态与运行时

- 腾讯云云函数 **Web 函数**（Node.js 20，内存 128 MB，超时 15 秒），监听 `0.0.0.0:9000`，启动文件 `scf_bootstrap`（启动命令中的 Node 路径以部署时腾讯云当前运行时文档为准）；通过**函数 URL**（公网、授权类型「开放」）访问；地域首选香港 `ap-hongkong`（不绑自有域名、无需备案；中继地址只是 Mac 设置 / 配对码里的配置项）。
- `core.js` 与运行时解耦：只含句柄编解码、请求校验、载荷构造、ES256 JWT、错误映射；`adapters/scf.js` 承担 HTTP 服务、环境变量读取、HTTP/2 连接 APNs。将来换云只需新增适配文件。零 npm 依赖，只用 Node 内置 `http`、`http2`、`crypto`。
- 不使用任何管理 API 密钥：部署是控制台上传 `build-zip.sh` 产出的 zip，配置环境变量；仓库与日志不出现任何密钥。

### 5.2 环境变量

`APNS_KEY_P8`（PEM，支持字面 `\n`）、`APNS_KEY_ID`、`APNS_TEAM_ID`、`APNS_TOPIC`（= `com.naituw.humaninloop.ios`）、`RELAY_HANDLE_KEY`（32 字节随机，base64）、`REVOKED_RIDS`（可选，逗号分隔，手动黑名单）。

### 5.3 接口（HTTPS JSON）

| 接口 | 调用方 | 说明 |
|---|---|---|
| `GET /v1/health` | Mac 设置页 / 手机 | `{ok, v:1, apns:<是否已配置>}`，不含任何密钥信息 |
| `POST /v1/register` | 手机 | 请求 `{deviceToken, env(dev/prod), bundleId, appVer}`（`bundleId` 必须等于 `APNS_TOPIC`，否则 400，避免中继被当成同团队其它 App 的推送代理）；返回 `{handle, expiresAt}`。句柄 = `v1.` + base64url(AES-256-GCM(`RELAY_HANDLE_KEY`, `{dt,e,b,i,x,r}`))（`r` 为随机 id，`x` = 180 天后） |
| `POST /v1/push` | Mac | 请求 `{handle, kind(alert/replace), locKey, threadId, collapseId, ah:{f,k,n,c}}`；中继校验 `locKey` 在白名单、总载荷 ≤ 4096 字节，**自行构造 APNs 载荷**（Mac 不能指定任意提醒文案）：`aps.alert.loc-key`、`mutable-content=1`、`thread-id`、`alert` 带 `sound`，`replace` 为 `interruption-level=passive` 且无声音；`ah` 原样透传 |

错误映射：`400 bad_request`、`401 bad_handle`（解密失败 / 过期 / 在黑名单）、`410 gone`（APNs 返回 `BadDeviceToken` / `Unregistered` / `DeviceTokenNotForTopic`）、`413 too_large`、`429 rate_limited`、`502 apns_error{status, reason}`、`503 not_configured`。

### 5.4 APNs 细节

- `https://api.push.apple.com`（`e=prod`）或 `api.sandbox.push.apple.com`（`e=dev`）；TestFlight 与 App Store 构建用 prod，Xcode 调试构建用 dev；头：`authorization: bearer <jwt>`、`apns-topic`、`apns-push-type: alert`、`apns-priority: 10`、`apns-collapse-id`（≤ 64 字节）、`apns-expiration`（`alert` 约 1 小时，`replace` 约 10 分钟）。
- JWT（ES256）的 `iat` 按 30 分钟窗口取整并在实例内缓存，避免多个冷启动实例触发 APNs 的 token 更新频控（`TooManyProviderTokenUpdates`）；遇到该错误时 1 秒后用同一 token 重试一次。
- HTTP/2 会话在存活实例内复用，断开 / GOAWAY 后重建；冷启动现连。
- 限流：每句柄内存令牌桶（尽力而为，实例间不共享）；日志只记状态码与计数，不记句柄、token、载荷。
- 句柄轮换：更换 `RELAY_HANDLE_KEY` 使所有句柄失效，手机下次前台自动重新登记并把新句柄加密发给各 Mac；单个句柄失效靠 `REVOKED_RIDS` 手动加入。

## 6. iOS App

### 6.1 工程与构建

- XcodeGen：`App`（`com.naituw.humaninloop.ios`）、`NotificationService`（`…ios.notification`）、本地包 `AskHumanKit`；iOS 17.0+，仅 iPhone；Swift 5 语言模式 + 完整并发检查（先作警告）；Team `DMJXDB9H6Q`，自动签名。
- Capabilities：Push Notifications；Keychain 共享组 `$(AppIdentifierPrefix)com.naituw.humaninloop.ios.shared`（App 与扩展共用，存配对密钥）。Info.plist：`NSCameraUsageDescription`、`NSFaceIDUsageDescription`；出口合规键按 App Store Connect 问卷结论设置。
- 本地化：`Localizable.xcstrings`（zh-Hans + en），含推送的 `loc-key`：`PUSH_ASK`、`PUSH_CONFIRM`、`PUSH_FOLLOW_DONE`、`PUSH_FOLLOW_WAITING`、`PUSH_FOLLOW_FAILED`、`PUSH_RESOLVED`。
- 分发：Xcode Archive → Organizer 上传 TestFlight，不使用 App Store Connect API 密钥。
- 本地验证用**新建的专用模拟器**（不复用原型或其它 Agent 正在使用的模拟器）；真机用于配对 / 推送 / 网络验证。

### 6.2 分层

- `AHProtocol`：信封、AEAD、HKDF / X25519、校验码、消息模型（Codable）、限额常量；Mac 夹具互测；可在 macOS 上 `swift test`。
- `AHTransport`：`BrokerClient`（actor）包装 CocoaMQTT：TLS 8883 失败回退 WSS 8084；手动重连退避；QoS 1 发布回调；LWT；订阅管理；统计流量。对外只暴露协议化接口，便于 Mock。
- `AHStore`：Keychain（配对密钥、`phoneId`、broker 配置、推送句柄；`AfterFirstUnlockThisDeviceOnly`）、SwiftData（草稿、本地历史、请求缓存、发件箱、流量月统计，均为扁平模型）。
- `MacSession`（每个配对一个 actor）：加解密、`hello` / 快照 / delta 应用、`rev` 校验、发件箱重试、上传 / 拉取附件状态机、presence 观察。
- `AppStore`（`@Observable`）：把多个 `MacSession` 聚合成收件箱、Agents、Mac 列表等视图状态；UI 只依赖 `AppStore` 与 `RemoteBackend` 协议（真实实现与 `AHMock` 可互换）。
- 生命周期：`scenePhase == .active` → 连接 → `hello`；进入后台 → 发 `{o:0}` 并断开；冷启动由通知点击进入时先连接再按 `rid` 深链跳转。

### 6.3 界面实现要点（行为见需求 §6）

- 导航：`TabView`（收件箱 / Agents / 历史 / Mac），Agents 页的「＋」打开新任务流程。
- 作答页：`ScrollView` 纵向长表单 + 底部固定发送条；推荐徽标不预选；草稿按请求 id 存 SwiftData，输入变化后 300 ms 去抖落盘；提交走发件箱，收到 `answer.ack` 前显示进度。
- Markdown：`swift-markdown` 解析 → 自绘 SwiftUI 视图（标题、列表、引用、代码块横向滚动、表格、链接用 `SFSafariViewController` 并先显示域名、本地图片按 `ahasset://<assetId>` 懒加载）；Mermaid 先显示为代码块。
- 附件预览：图片（`ImageIO` 解码，缩放 / 平移）、文本 / 代码（等宽）、Markdown、diff（+/- 着色）、其它类型走 `QuickLook`；分块拉取显示进度、可取消 / 重试。
- 回复附件：`PhotosPicker`、相机、`fileImporter`；图片转 JPEG 并按限额缩放；分块上传带校验与进度。
- 配对：`DataScannerViewController` 扫码（不可用时提供「粘贴配对码」；模拟器测试同样用粘贴）。
- 新任务提交前 `LAContext.evaluatePolicy(.deviceOwnerAuthentication)`，每次提交都验证。
- 通知权限在首次配对成功后请求；`UNUserNotificationCenterDelegate.willPresent`：前台且请求已在收件箱则不弹横幅。

### 6.4 通知扩展

- 只处理带 `mutable-content` 的提醒推送：读取 `ah`，按 `f` 在共享钥匙串取 `k_m2p`，AEAD 解密（AAD 为推送域）；成功则设 `title = "Mac · Agent · 项目"`、`body = 摘要`、`threadIdentifier = macId`、`userInfo = {rid, mac, kind}`；`resolved` 类设为被动级别、去掉声音；失败保留系统的 `loc-key` 通用文案。
- 内存与耗时极小：不链接 MQTT、SwiftData、Markdown；不做网络请求。

## 7. 端到端推送链路

1. 手机配对成功后请求通知权限，拿到 device token，调用中继 `/v1/register` 得到句柄，用 `push.register` 经加密通道发给该 Mac；token 变化或句柄接近到期时自动重做并通知各 Mac。
2. Mac 事件（新请求、Follow 事件、请求被别处处理）→ 构造推送明文并加密 → `/v1/push` → APNs → 手机；扩展解密展示。
3. 用户点击通知 → App 启动 / 进入前台 → 连接 broker → `hello` → 快照 → 按 `rid` 打开对应请求 / Agent。
4. 请求在别处被处理：若该手机收到过提醒且未 `request.seen`，Mac 发 `replace` 推送改写旧通知；App 回前台时对已结束请求调用 `removeDeliveredNotifications`。

## 8. 阶段与验收

每个阶段结束前：跑自动测试；Mac 端改动按 AGENTS.md 先 `./scripts/install.sh`，再用新装的 `AskHuman` 验证。涉及真实 Agent 的启动 / 发送 prompt 的测试必须先经 AskHuman 获批。每个阶段的提交遵循 §12。

### P0 门禁（不写产品功能；产出 `docs/investigations/ios-channel-p0.md`）

探针脚本与最小 Xcode 工程放在 `ios/Probes/`（不属于产品 target，报告引用后保留作回归探针）；`relay/` 在 P0 先交付最小可部署版本（`/v1/health`、`/v1/register`、`/v1/push` 主路径与核心单测），P4 再补限流、黑名单、完整错误映射与全部单测。

- 0.1 Rust 依赖探针：`rumqttc` 等新依赖在现有工程分支 `cargo check` 通过（macOS / Windows / Linux 目标），rustls 后端与现有依赖一致，记录 release 体积增量。
- 0.2 EMQX（用户控制台配置）：新建用户名 / 密码与 ACL；探针验证同用户名内收发、跨用户名被拒（再建一个测试用户名）；Mac（家 / 公司）TLS 8883 与 WSS 8084 可达；真机 Wi-Fi / 蜂窝的 TLS 与 WSS 回环 RTT（各 100 次，记录原始数据与超时）。
- 0.3 中继最小版部署（用户按 §10 部署）：Mac（家 / 公司）与真机（家庭 Wi-Fi / 蜂窝，**关闭代理**）各调用 `/v1/health` 100 次，成功率 ≥ 99%、p95 ≤ 1.5 秒。
- 0.4 真机 APNs：最小工程（正式 Bundle ID + 通知扩展）真机安装 → 注册 token → `/v1/register` → `/v1/push`（加密预览）→ 通知出现、扩展解密成功、点击回到 App；至少 30 次送达耗时分布；确认 HTTP/2 → APNs 从所选地域可用。
- 0.5 用户核对腾讯云账号的计费规则（无固定月费）。
- 0.6 用户在公司网络执行 `curl` 探测 broker 端口。
- **门禁**：任一项不达标，立即停下，经 AskHuman 把原始数据与选项交给用户，不自行换方案。

### P1 协议与密码学核心

- Rust `ios/{crypto, envelope, wire}` 与 Swift `AHProtocol` 同步实现；`ios/Fixtures/wire-v1/`：票据密钥、DH / HKDF 向量、校验码、信封加解密、AAD 篡改拒绝、每种消息类型一个样本。
- 验收：`cargo test` 与 `swift test` 都以同一批夹具通过；篡改任一字节的信封必拒；两端 `msgId` 去重与限额常量一致。

### P2 iOS UI 原型 + 评审门（Mock 数据）

- 在 `AHMock` 后端上完成全部主要界面与状态：收件箱（多 Mac 混排、筛选、分组、未读、草稿）、作答（多问题、推荐、仅选择、单选、附件）、确认、Agent 列表 / 详情 / 观察 / 插话、新任务（含 Face ID 步骤）、本地历史、Mac 管理、配对、设置、各类空态 / 离线 / 错误态。
- 验收（**用户 gate**）：产出模拟器截图包与关键流程录屏，经 AskHuman 请用户评审并确认；确认前不接真实数据，评审意见追加到需求文档「反馈意见」。

### P3 Mac Gateway 骨架 + 配对 + 在线状态

- config / secrets / state / journal；`BrokerTransport`（rumqttc + loopback）；配对状态机；在线状态；`hello` 与空快照；IPC 3→4（含 overview 历史与 drain 文档）；设置页 iOS 卡 + CLI；iOS App 接真实连接、配对、Mac 列表、解除、重装遗留配对检测。
- 验收：真机与两台 Mac（家 / 公司）配对成功；票据 5 分钟失效且一次性；校验码不一致可识别并拒绝；撤销后旧二维码 / 旧设备失效；`channel test ios` 通过；Rust 单测覆盖配对状态机、幂等日志、过期票据。

### P4 问答闭环

- `IosChannel` + 视图投影 + 答案校验 + Coordinator 接入 + 确认 `start_ios` + 中继补全（限流、黑名单、完整错误映射与单测）+ 通知扩展 + 活跃槽（`slot.take`、`backfill_inflight`）；iOS 收件箱 / 作答 / 确认 / 附件双向 / 本地历史 / 草稿 / 推送。
- 验收：与 Mac 弹窗、飞书抢答正确收敛；多题 / 推荐 / 仅选择 / 单选 / whats-next TODO 出队与 IM 输出一致；后台与锁屏的真 APNs 送达与解密预览；替换通知有效；前台延迟口径达标并附原始数据。
- **用户 gate**：交付第一个可日常使用的 TestFlight 构建（问答），收集使用反馈后再继续。

### P5 Agents

- `AgentEvent.failed` 及其单测；Agents 列表；Follow 与三类通知；实时观察（`ios_focus`）；插话（含 `interject.consumed`）。
- 验收：≤ 5 关注；完成 / 等待你 / 出错推送正确（Codex 无出错）；实时流仅前台、退后台即停；插话「已入队 / 已送达」分别显示；Watch 变化显示 p95 ≤ 2.5 秒。

### P6 新任务

- 抽取 `task_readiness` / `launch_agent_task`（IM `/new` 回归测试先行，确保 IM 行为不变）；`task.options` / `todos` / `create` / `status`；Face ID；自动关注与 60 秒告警。
- 验收：与 IM `/new` 同语义；重复 `task.create` 只启动一次；真实 Agent 启动测试经用户批准后进行。

### P7 加固

- 故障矩阵（§9）逐项自动化 / 手工验证；流量统计与预警；健康展示；性能口径测量（拆分各段耗时，两台设备按 RTT 估算单向值并同时记录原始数据）；安全自检（日志无密钥、AAD 篡改、跨配对访问、已撤销设备）。

### P8 收尾

- `docs/wiki/ios-setup.md`（+ 英文）；`docs/overview.md`（仓库地图增 `ios/` 与 `relay/`、渠道清单、协议号历史）与本规格状态更新；`docs/PROGRESS.md` 清理 iOS 延期项；是否在 CI 增加 `relay` 的 `node --test` 与 `swift test` 作业（实施前经 AskHuman 询问，推荐只加这两项、不加 `xcodebuild`）；TestFlight 外部测试说明；最终验收报告。

## 9. 测试矩阵

- **自动化（Rust）**：信封与密钥派生夹具；配对状态机（过期、重复、拒绝、半成品清理）；幂等日志（重复、冲突、`task.create` 中断）；答案校验；视图投影（选项原文、TODO、确认变体）；Follow 状态机（去抖、失败、等待你去重）；`LoopbackTransport` 注入丢包 / 乱序 / 断线 / 重复；`AgentEvent.failed` 解析矩阵（四家 + Codex）。
- **自动化（Swift）**：夹具互测；`MacSession` 的 `rev` / `epoch` / resync；发件箱重试与幂等；草稿持久化；Markdown 渲染快照；限额。
- **自动化（中继）**：句柄编解码与过期、载荷白名单、JWT 窗口取整、错误映射、令牌桶。
- **手工 / 真机**：跨渠道同时回答；重复及同 ID 不同内容；乱序旧快照；提交后断网；ACK 丢失；Mac 与手机分别重启；Mac 睡眠恢复；Wi-Fi / 蜂窝 / 切网；通知权限被拒；token 更新与句柄刷新；跨配对访问拒绝；设备撤销；重装遗留配对；公司网络；EMQX 额度用尽的提示；前台 / 后台 / 锁屏 / 强制退出的送达差异（分别记录，不用模拟器通知或本地通知代替真 APNs）。
- **性能**：按需求 D10 四条口径测量，分段记录；报告给出原始分布、最慢值与全部超时，不只统计成功样本。

## 10. 用户手动步骤（我不接触任何密钥）

1. **Apple**：创建 App ID `com.naituw.humaninloop.ios`（开启 Push Notifications）与 `…ios.notification`；创建 APNs 认证密钥（.p8），记下 Key ID 与 Team ID；在 App Store Connect 建 App 记录与 TestFlight；首次上传时回答出口合规问卷；准备外部测试所需的隐私政策网址与联系邮箱。
2. **EMQX 控制台**：为本部署建一个用户名 / 密码（只含字母数字）；授权加「用户名」规则（允许发布 / 订阅 `ah/<该用户名>/#`）与「全部用户」兜底「拒绝 `#`」；给朋友另建各自的用户名 / 密码并各加一条自己的允许规则；确认消费上限保持 0。
3. **腾讯云**：先在费用中心核对云函数计费规则；新建 Web 函数（Node.js 20，`ap-hongkong`，128 MB，超时 15 秒）；上传 `relay/scripts/build-zip.sh` 产出的 zip；在函数配置里填 §5.2 的环境变量（p8 内容直接粘贴到控制台）；创建函数 URL（公网、授权「开放」）；把 URL 填进 Mac 设置的中继地址。
4. **设备**：用 Xcode 把开发构建装到真机；P4 起用 TestFlight（朋友可作为外部测试员，首个构建需 Beta 审核；也可作为内部测试员，无需审核但需 App Store Connect 角色）。
5. **公司 Mac**：执行 P0 的网络探测命令并回传结果。

## 11. 风险与开放验证项

| 风险 / 待验证 | 处理 |
|---|---|
| 公司网络封锁 8883 / 8084 | P0 第 0.6 项实测；若都被封，停下来请用户决定 |
| 香港函数到 APNs 的 HTTP/2 出口不可用 | P0 第 0.4 项实测；不通则停下来请用户决定是否换地域 |
| 腾讯云账号计费含固定套餐 | P0 第 0.5 项核对；红线是不接受固定月费 |
| rustls 加密后端冲突、新增依赖体积 | P0 第 0.1 项实测并记录 |
| CocoaMQTT 在后台 / 切网下的重连行为 | `BrokerClient` 自管重连；P7 切网矩阵验证 |
| 通知扩展内存限制 | 只链接 `AHProtocol`；P0 第 0.4 项真机验证 |
| SwiftData 在 iOS 17.x 的稳定性 | 只用扁平模型、不用关系；P2 / P4 若出现问题，经 AskHuman 请用户决定是否换存储 |
| 钥匙串遗留配对 | 首次启动检测并让用户选择；「忘记所有配对」 |
| `scf_bootstrap` 的 Node 路径与运行时变更 | 部署时核对腾讯云当前文档；`build-zip.sh` 单点维护 |
| 复制 `~/.askhuman` 造成 `macId` 冲突 | 文档提示；Gateway 发现 clientId 被踢连续 3 次时在设置页警告 |
| 请求正文极大 | 摘要 + `request.get`；超限尾部截断并标注 |

## 12. 提交与文档规范

- 提交信息遵循 Conventional Commits；本功能的所有提交（Mac 端、`ios/`、`relay/`、文档）带 `Release-Note: skip` 尾注，直到用户决定对外发布该渠道；类型按改动性质选 `feat` / `fix` / `test` / `docs` / `chore`，范围用 `ios` / `relay` / `channels` / `ipc` / `settings`。
- 代码注释用英文；本计划与需求文档用中文。
- 改 `ipc/mod.rs` 前再读一遍 overview 的「IPC 协议号」；协议号 +1 时同步 overview 历史与 drain 文档。
- 每阶段完成后检查 `docs/overview.md`、本需求与计划是否已不准确，只改对应文档；P8 统一更新 overview。
- 不改动用户其它进行中的未提交工作（当前工作区里的 `codex_desktop`、`popup` 相关修改与 `docs/investigations/native-ios-channel-handoff.md` 等）。
- 开发开始后，计划与需求不得擅自变更；需要调整先经 AskHuman 确认，并把结论追加到需求文档「反馈意见」。

## 13. 实施记录（与计划的差异）

截至 2026-10-25：P1、P3、P4、P5、P6 的代码与自动化验证已完成（Rust / Swift / 中继 / 前端测试、模拟器 UI 测试 en + zh-Hans、「真实 daemon + 本地 broker」端到端 30 项，另对真实 EMQX + 中继跑同一套并加 3 项只在真实服务上的检查，共 33 项全过；再加「模拟器上的真 App 对真实 EMQX + 中继 + 真 APNs 沙盒」的完整场景，基础场景连续三次、含 Agents / 权限确认 / 关注推送的完整场景又两次全过），P0 的账号侧门禁（EMQX、APNs 密钥、中继、计费）已通过（见 `docs/investigations/ios-channel-p0.md`），**P0 的手机侧门禁、P7 的真机 / 真网络部分与 P8 的发布事项尚未进行**（见 `docs/PROGRESS.md`）。实现与上文不同或超出上文的地方：

- **iOS 本地存储不用 SwiftData**：`AHStore` 用文件型 JSON（每个键一个文件，`completeFileProtectionUntilFirstUserAuthentication`；`JSONFile` / `JSONDirectory`）。它能在 macOS 上作为纯 SwiftPM 包测试，通知扩展也无需模型容器。§6.2 / §11 里与 SwiftData 相关的表述以此为准。
- **通知扩展链接 `AHProtocol` + `AHStore`**（原计划只链接前者）：`PushDecryptor` 与钥匙串 `Vault` 在 `AHStore`；扩展仍不链接 MQTT、Markdown，也不联网。
- **任务启动抽取成共享模块** `daemon/runtime/task_launch.rs`（`task_gate` / 启动记录 / 失败映射），IM `/new` 与 iOS 共用。popup-only 的 Dev Instance 不能操作 IM，所以 `/new` 的真实回归留给合并前的人工检查。
- **`failed` 标志**：`AgentEvent.failed` 与 Watch 帧的 `failed` 字段按计划实现；Pi 的扩展需要在设置里重新保存 / 刷新后才会上报失败的回合。
- **推送点击路由**：`follow.*` 推送的 `rid` 是 Agent key，点击打开 Agent 页，其余打开请求；App 在前台且已显示对应内容时不再弹系统横幅（`AppStore.openNotification` / `coversNotification`）。
- **流量预警**（P7）：本机统计的流量按「发布 + 投递各计一次」折算成 EMQX 额度（1 GiB / 月）的占比，80% 起黄色提示、100% 起红色，Mac 设置页与手机「Mac」页 / 收件箱都有；拒绝连接的提示里点明「免费额度可能用完」。
- **`insecureLocal` 只对回环地址生效**（`ios/transport.rs::first_via`）：此前只有判断函数没有强制。
- **安全自检**：端到端测试 `testSecretsNeverReachTheLogsOrTheCommandOutput` 在配对 + 问答之后，检查 broker 密码、配对码与两个方向密钥（base64 / base64url / hex）不出现在任何 `*.log` 与命令输出里。
- **隔离规则**：任何隔离运行的 Mac / daemon 必须设置 `ASKHUMAN_HOME`（只改 `HOME` 曾导致隔离的 daemon 复用了正式 launchd 标签并顶掉正式 daemon）；`launchd_label` 现在在只覆盖 `HOME` 时也用独立标签（单独提交），测试台在启动前断言 `ASKHUMAN_HOME` 位于临时目录，并只清理携带该目录的进程。
- **工具**：`ios/Tools/make-catalog.py` 由编译器的 `.stringsdata` 生成中英字符串目录（缺中文翻译会失败）；UI 测试新增启动参数 `-AHOpenAgent` / `-AHNewTask` / `-AHHoldInterjects` / `-AHFollowEvent` 与场景 `quota`；UI 测试用 `-collect-test-diagnostics never`。
- **任务表单**：权限（行动前先询问 / 自动执行）没有预选，必须明确选择后才能提交。
- **端到端测试可对真实服务运行**（用户 2026-10-25 允许）：`AH_E2E_REAL_FROM` / `AH_E2E_BUNDLE_ID` 让一次性 Mac 导入已部署实例的连接串（只在内存里），同一套用例对真实 EMQX + 中继跑一遍，另有 3 项只在真实服务上的检查（真中继 + 真 APNs 沙盒的 `BadDeviceToken` → `gone` → 新 token 恢复、别的 Bundle ID 被中继拒绝、前台往返延迟）。做法见 `docs/overview-ios.md`；数字见 `docs/investigations/ios-channel-p0.md` §3.6。这不替代 §9 要求的真机真 APNs 矩阵。
- **修复：先 hello、后上报前台的手机漏收中间的变化**（真实服务的延迟暴露的 Mac 端竞态，设计不变）：App 在首次同步后才发布 `fg=1`，而 Gateway 以「在线 + 前台 + 已 hello」判断「活着」。hello 与前台状态之间创建的请求（以及 Agent / Follow 的变化）既没被实时发出，hello 的快照又早于它，手机就一直看不到，直到下一次重连。现在 `Actor::on_presence` 在手机由此变为「活着」时补发四个快照；两个 Rust 用例（请求、Agent）撤掉修复就会失败。本地 broker 延迟太低，此前一直没撞上。
- **测试台修正**：`Cmd` 的 `readabilityHandler` 在 EOF 时必须清掉，否则已结束的子进程会让运行器空转到 500% CPU；E2E 里「请求出现在收件箱」与「请求带着内容」是两步（慢链路上先摘要后全文），用例改为等内容再作答，历史条目在 Mac 确认之后才写，Follow 要等 Mac 的列表回来再点下一个。
- **模拟器上的真 App 对真实服务的完整场景**（用户 2026-10-25 要求先在模拟器测完再上真机；不替代 §9 的真机矩阵）：`SimulatorLiveTests`（宿主，一次性 Mac，同「真实服务」模式）通过 `xcodebuild test-without-building` 启动 `AskHumanLiveUITests`（XCUITest，驱动真正的 App、钥匙串与通知扩展）。场景：配对且两端校验码一致 → 通知权限 → token → 中继登记 → Mac 持有句柄；前台单题；多题 + 自由文本；Agent 的权限确认（`__permission-hook`）一次允许、一次带理由拒绝（理由到达 Agent）；一个 Agent 会话（`__agent-hook` 上报）出现在 Agents 列表、实时观看出现「Live」、关注由 Mac 确认、给它排队一条消息并在它的下一步被带走（读取回执、排队卡片消失、列表里出现当前工具）、回合结束时应用内横幅「Agent 已完成」并点开 Agent 页；Mac 关着 Agent 任务时新任务表单说明原因；App 退到后台后 Mac 察觉手机离开；一次真 APNs 沙盒推送 → 通知扩展解密出文本 → 点击横幅打开请求 → 回答；被关注的 Agent 在手机离开时结束回合的 `follow.done` 推送 → 解密出「Finished」→ 点击打开 Agent 页；历史里有已回答的请求；手机侧解除配对。基础场景（到推送与解除配对为止）连续三次通过（各约 2 分钟），加上 Agents / 权限确认 / 关注推送之后的完整场景又通过两次（172 s、195 s）。模拟器收得到真 APNs 沙盒推送且会运行通知扩展，`simctl push` 不会运行扩展，所以不用它。范围（只有几个样本，不是分布）：Mac 提问 → 手机显示请求 1.15–1.22 s，点发送 → 行消失 1.11–1.13 s（两者含 UI 自动化的轮询，是上界），Mac 察觉手机离开 0.43–0.76 s，Mac 提问 → 横幅出现 2.1–2.3 s，关注推送（Mac 回合结束 → 横幅）4.3–4.5 s（含 Mac 的 3 s 完成去抖）。
  做法与护栏：配对码含 broker 密码，只经命名管道（FIFO）从宿主交给 UI 测试——不写文件、不进环境变量、不进剪贴板，码在屏幕上时不截图；`xcresult` 结束时删除（只在配对码出现之前失败时保留供诊断），结束后在日志与模拟器键盘日志里扫描配对码，发现即删除并判失败；两端用「会合点」文件互相等待，Mac 的五分钟配对窗口从 UI 测试准备好收码时才开始。模拟器每次先卸载 App（同时清掉通知权限）、`simctl keychain reset`（卸载不清钥匙串），再等设备 CPU 静下来；默认不 `simctl erase`（刚抹掉的设备首次开机要占满一个核好几分钟，饿死 xcodebuild），需要时设 `AH_E2E_SIM_ERASE=1`。设备由 `AH_E2E_SIM_UDID` 指定，要给它一台专用设备。构建需要签名（推送要真 token）：先 `xcodegen generate`，再 `xcodebuild build-for-testing … -derivedDataPath ios/build/DerivedDataLive -allowProvisioningUpdates`；必须前台跑，机器负载高时耗时成倍增加。
- **这次运行找出的四个真实缺陷**（均已修复，设计不变；每个都先有失败的确定性测试）：
  1. 点击推送横幅崩溃（iOS 27 的 UIKit 主线程断言）：`UNUserNotificationCenterDelegate` 的 async 方法是 `nonisolated`，系统的完成回调落在协作线程池上。改为 `@MainActor` 方法 + `@preconcurrency` 一致性。
  2. 通知扩展解不开真实推送（一律 `malformed`，退回通用文案）：中继转给 APNs 的是 `ah = {f,k,n,c}`（§3.4 / §5.3），没有 Mac 封装时带的 `v`。`PushDecryptor` 缺 `v` 时按本协议版本处理，带了别的版本仍拒绝；单元测试改用中继的形状（此前用 Mac 的形状，所以没发现）。教训：跨语言 / 跨进程契约的测试夹具要取自真实下游收到的形状。
  3. 快速 inactive → active 之后永久断线：`setForeground(false)` 在 `goodbye()` 处挂起，其间返回前台的那次调用看到「连接还起着」什么都不做，随后 goodbye 把连接断了。现在 goodbye 之后若应在前台就重连。
  4. 推送句柄没有可靠送到 Mac：`pushSent` 在发送前置位且不随会话重置、握手完成但尚未同步（`.acked`）的 Mac 被跳过、发送失败不回滚。现在每次握手清空并重发，失败回滚，token 在 ack 与首个快照之间到达也立即发送。
- **保留的取舍（没改，待用户决定）**：`scenePhase == .inactive`（系统弹窗、通知中心、控制中心）会触发 `goodbye` 并重连，每次系统弹窗都有一次连接抖动；把 `.inactive` 视为仍在前台（或加去抖）属于设计变更。另外 Mac 只会发 `push.state` 的 `ok` / `gone`，不发 §3.4 里写的 `missing` / `error`；由手机在每次握手重发句柄覆盖。一处外观小瑕疵也没改：请求到达的应用内横幅会停满约 5 秒，即使用户已经点开了那个请求（横幅盖在请求页上方）。

