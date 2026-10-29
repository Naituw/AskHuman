# 开发计划：Mac 互通（弹窗网）

> 关联需求：`docs/specs/mac-mesh.md`（决策编号 M1–M12 均指该文档 §3）
> 前置资料：`docs/plans/ios-channel.md`（线协议 §3、Mac 端 §4）、`docs/overview-ios.md`（实现地图与测试台）
> 关联既有：`docs/specs/popup-request-inbox.md`、`docs/plans/im-channel-activation.md`、`docs/specs/daemon-graceful-drain.md`
> **前置**：`docs/plans/ios-alert-timing.md`（已批准、实现中）与本计划改同一批代码（活跃槽候选筛选、`Host` trait、Gateway 的 `deliver()`），应先合并；本计划按它落地后的语义写（手机不受活跃槽管辖，推送由提问方 Mac 空闲决定）。开工（建 worktree）前确认它已合并到基线分支，否则先等它。
> 计划描述方案与技术 / 规则细节，具体代码以实现为准；标「P0 确认」的是我只读过接口、还没验证的集成点。**评审已于 2026-10-27 通过；等 `ios-alert-timing` 合并后开工。**
> **开工前**读 `docs/agent-worktree-setup.md`，按其要求用 AskHuman 询问是否挂渠道预设（iOS 渠道不可预设，故推荐「不挂」），在独立 worktree（建议 `HumanInLoop-mesh`，分支 `feat/mac-mesh`）+ Dev Instance 里开发。不要碰主工作区里其他会话尚未提交的改动。

## 0. 方案总览

```
Mac H（家）                       EMQX（同一账号）                    Mac O（公司）
Gateway（提问方）◀── 摘要 / 正文 / 回执 / resolved ──▶ 答题端
答题端 ◀── 摘要 / 正文 / 回执 / resolved ──▶ Gateway（提问方）
        配对握手 peer.*（保留消息，异步）· 引荐与撤销 circle.*（持久化发件箱）
```

核心六件事：

1. **提问方扩展**（`ios/gateway/`）：设备类型 `mac`——在线状态、hello、摘要、加载门、消息白名单、暂停、条目上限。
2. **答题端引擎**（同一个 Gateway Actor 里新增的角色）：每个提问方一个会话（hello / epoch / rev / resync / 发件箱），对标 Swift `PhoneEngine`，但只做收件箱与作答。
3. **配对与圈子**：邀请码、异步握手、批准卡、引荐、撤销，状态全部持久化。
4. **Daemon 接入**：`RemoteEntry`（把对端请求放进现有请求登记表与弹窗通道）、`MacPeersChannel`（提问方侧的「电脑出口」）、激活联动。
5. **弹窗前端**：Mac 标签、「Mac + 项目」分组、未加载 / 加载中 / 已加载、确认卡提示。
6. **设置页与 CLI**。

关键不变量：

- 权威状态始终在提问方；答题方只是遥控与展示端，答案是否被采纳以提问方 Coordinator 的 `try_submit` 为准。
- 内容消息两两加密、每个收件人一份；话题扇出只用于在线状态（M10）。
- 状态变更类消息（答案、确认）一律带 `clientMsgId`，由提问方持久化去重（沿用）。
- 对端 Mac 不是活跃槽里的一个值：电脑出口永远参与；活跃槽只剩 IM 在用（手机已由 `ios-alert-timing` 移出，M6）。
- 答题端不转发；不改 iOS App、不改推送中继；`PROTO` 仍为 1。
- 评审已通过（2026-10-27）；计划与需求不得擅自变更，需要调整先经 AskHuman 确认并追加到需求「反馈意见」。

## 1. 交付物与代码落点

```
src-tauri/src/ios/
  wire.rs                  增量：RequestNew.summary、RequestSummary.questions/attachments、Hello.paused（均为可选字段）
  mesh/                    纯逻辑与线格式（不依赖 daemon，可单测）
    wire.rs                peer.* / circle.* 消息、ahmac1 邀请码编解码
    keys.rs                互惠密钥派生 derive_mesh_keys、安全码
    notice.rs              摘要视图与 RequestSummary 构造（提问方）
  gateway/
    peers.rs               提问方侧：kind=mac 设备的 presence / hello / 摘要 / 加载门 / 白名单 / 暂停 / 上限
    answerer.rs            答题端：每个提问方一个会话、通知表、快照对齐
    answerer_submit.rs     答题端：发件箱、answer.submit / confirm.decide、资产与上传
    mesh_pairing.rs        邀请 / 加入 / 批准 / 异步握手
    mesh_circle.rs         引荐、撤销、持久化发件箱
  store.rs                 增量：PhoneRecord.kind；hosts / invites / joins / mesh_outbox / answer_outbox / seen
src-tauri/src/channels/mac_peers.rs       MacPeersChannel（电脑出口）
src-tauri/src/daemon/runtime/ios_mesh.rs  RemoteBridge：Host 回调 → RemoteEntry；弹窗提交 → 答题端
src-tauri/src/daemon/request.rs           增量：RequestEntry / ConfirmEntry 的 remote 链接、create_remote*
src-tauri/src/ipc/mod.rs                  增量：ShowPayload.remote、IosOp / GatewayState 增量、远端加载 / 附件消息
src/                                      弹窗（Sidebar、摘要卡、状态、确认卡提示）与设置页（IosChannelCard）
ios/Fixtures/mesh-v1/                     仅 Rust 使用的夹具（密钥向量、邀请码、消息样本）
ios/Tools/mesh-e2e/                       端到端驱动（两台一次性 Mac + 本地 broker；P0 定形态）
docs/                                     本计划与需求；完成时更新 overview-ios / overview / wiki / PROGRESS
```

## 2. 标识与常量

| 常量 | 值 | 说明 |
|---|---|---|
| `MESH_VERSION`（线上 `mv`） | 1 | 邀请码与握手里的互通协议版本 |
| 邀请码前缀 | `ahmac1.` | 后接 base64url(JSON) |
| `INVITE_TTL_SECS` | 86 400 | 邀请 24 小时 |
| `INTRO_TICKET_TTL_SECS` | 604 800 | 引荐票据 7 天 |
| `MAX_PEERS` | 8 | 每台 Mac 的对端上限 |
| `MAX_PENDING_PER_PEER` | 20 | 每个对端的待答条目上限 |
| `NOTICE_PREVIEW_MAX_CHARS` | 120 | 即 `PUSH_PREVIEW_MAX_CHARS` |
| `HELLO_TIMEOUT` / `HELLO_ATTEMPTS` | 5 s × 3 | 同手机 |
| `ANSWER_ACK_TIMEOUT` | 30 s | 同手机 |
| `MESH_KDF_LABEL` | `ah1-mesh` | 互惠派生的 HKDF info 前缀（与 `ah1-pair` 区分，避免跨协议复用密钥） |
| 设备 `kind` | `phone` / `mac` | 存储里设备记录的类型，缺省 `phone` |
| 消息类型 | `peer.request` / `peer.accept` / `peer.reject` / `peer.done`；`circle.intro` / `circle.ticket` / `circle.relay` / `circle.revoke` / `circle.ack` | §3 |

## 3. 线协议增量（在 v1 之上）

### 3.1 话题与角色

没有新增话题。一个 Mac 同时订阅 `u/{自己}`（提问方收件箱）、`d/{自己}`（答题方收件箱）、`s/+`（在线状态）与各个未过期邀请 / 引荐票据的 `p/{ticketId}`。

| 方向 | 话题 | 发送者 | 密钥 |
|---|---|---|---|
| 提问方 → 答题方（`request.new` / `request.full` / `request.resolved` / `hello.ack` / 快照 / `circle.*` …） | `ah/{user}/d/{答题方 macId}` | 提问方 Gateway | 该对的 `m2p` |
| 答题方 → 提问方（`hello` / `request.get` / `request.ack` / `answer.submit` / `confirm.decide` / `asset.get` / `upload.*` …） | `ah/{user}/u/{提问方 macId}` | 答题方 | 该对的 `p2m` |
| 在线状态 | `ah/{user}/s/{macId}` | 各 Mac（保留消息 + 遗嘱） | 明文 |
| 配对期 | `ah/{user}/p/{ticketId}` | 邀请方 / 加入方 | 票据密钥 |

规则：**控制消息（`circle.*`）总是走发送者自己的提问方角色**（`d/{接收者}`，用发送者作为提问方的密钥），由接收者的答题方入口处理——这样两个方向各只有一种封装方式，不用约定谁是「主动方」。

### 3.2 互惠密钥派生

一次握手同时建立两个方向的「提问方 → 答题方」通道（邀请方 H、加入方 O；引荐里 X 扮邀请方、P 扮加入方）：

```
输入：票据秘密 ts、票据 ID、H 与 O 的标识、双方临时公钥 eH / eO、X25519 共享秘密 dh
info  = "ah1-mesh" 00 H 00 O 00 ticketId 00 eH 00 eO
输出（HKDF-SHA256，盐 = ts，160 字节）= 5 × 32：
  [0] H 作提问方：H→O      [1] H 作提问方：O→H
  [2] O 作提问方：O→H      [3] O 作提问方：H→O
  [4] 校验码密钥（沿用 HMAC-SHA256(k, "ah1-sas") 取前 4 字节 mod 1e6）
```

- H 的记录：`phones[kind=mac, id=O]`：`m2p=[0]`、`p2m=[1]`；`hosts[id=O]`（H 作答题方）：`m2p=[2]`、`p2m=[3]`。O 对称。`kid` 沿用 `KID_PAIR`。
- 这样提问方侧的 Gateway 代码把对端当作「phone 记录」处理，不必改信封、重放窗口、去重与幂等日志的任何一行；答题方侧的代码是 `PhoneEngine` 的 Rust 对应物。
- 已知向量放 `ios/Fixtures/mesh-v1/crypto.json`（仅 Rust 读取）。

### 3.3 邀请码

```
ahmac1.<base64url(JSON)>
{ "v":1, "m":<邀请方 macId>, "mn":<名称>, "t":<ticketId>, "ts":<票据秘密>, "ek":<临时公钥>,
  "x":<过期 unix 秒>, "bf":<broker 指纹>, "i":<引荐 id，仅引荐票据> }
bf = base64url(SHA-256("mesh-bf" 00 host 00 port 00 username)[0..8])
```

- **不含** broker 主机 / 用户名 / 密码；加入方本地用自己的 broker 配置算 `bf` 比对，不一致直接提示「需要同一个 EMQX 账号」，不发请求。
- 持有邀请码就能用票据密钥封装一个 `peer.request`——所以邀请码按敏感信息对待（不写日志、不进命令输出），仍需邀请方点「允许」。

### 3.4 异步握手（`p/{ticketId}`）

- 邀请方每次（重）连都订阅所有未过期邀请 / 引荐票据的话题。消息用票据密钥封装（`kid = KID_TICKET`，`f = macId`）。
- `peer.request`（加入方 → 邀请方）与 `peer.accept` / `peer.reject`（邀请方 → 加入方）都以**保留消息**发布；对方处理完（或取消 / 过期）后发空载荷清除。`peer.request`：`{macId, name, ek, appVer, proto, mv, intro?}`；`peer.accept`：`{macId, macName, mv}`；`peer.reject`：`{reason}`（`denied` / `expired` / `contested` / `full`）。
- 加入方发出请求时就能算出密钥与安全码（邀请码里有邀请方的临时公钥）并显示；邀请方收到请求后算出同一组，安全码用于批准卡。
- `peer.done`（加入方 → 邀请方）在新建的 `u/{邀请方}` 通道上用新密钥封装，证明加入方已存好密钥；邀请方据此把记录从未完成置为 active（沿用手机配对的 `PendingConfirm → Active`）。未完成的记录 24 小时内无 `done` 也无 hello 则清理。
- **时间戳窗口**：配对话题上的消息 `ts` 须落在 `[票据创建 − 15 分钟, min(票据过期, 现在) + 15 分钟]`，而不是通常的「与现在相差 ≤ 15 分钟」——保留消息可能已经等了好几个小时。正常通道的 15 分钟防重放窗口不动。
- **竞争保护**：邀请方记录第一个请求的 `(macId, ek)`；之后出现不同的请求 → 邀请置为 `contested`，提示用户并拒绝全部。批准卡以 `ticketId` 做幂等键，每张票据只批准一次。

### 3.5 引荐与撤销（`circle.*`）

| 类型 | 方向 | 载荷 | 说明 |
|---|---|---|---|
| `circle.intro` | M → X | `{introId, peer:{macId, name}}` | 请 X 为 P 准备一张引荐票据 |
| `circle.ticket` | X → M | `{introId, code}` | X 的引荐票据（`ahmac1` 编码，含 `i`） |
| `circle.relay` | M → P | `{introId, from:{macId, name}, code}` | M 转交 X 的票据 |
| `circle.revoke` | 任一 → 其余 | `{macId, name?}` | 撤销某台 Mac |
| `circle.ack` | 接收者 → 发送者 | `{id}` | 控制消息已处理 |

- 控制消息都带 id，进**持久化发件箱**：对方上线（在线状态为在线且已 hello）后发送，收到 `circle.ack` 或过期（7 天）后删除；接收者按 id 幂等。
- 授权：只接受已配对对端的控制消息（经两两密钥认证）。`circle.intro` → X 生成 `intro` 类票据，**只预批 `peer.macId`**，并向用户展示「M 引荐了 P」+「撤销」；`circle.relay` → P 据此向 X 发起 `peer.request`；`circle.revoke` → 自动删除对应记录并提示用户（需求 M14）。被引荐的 `peer.request` 必须与票据绑定：`macId` 等于 `intro.peer.macId`，否则拒绝。
- M 看得到引荐票据的秘密，但没有 X 与 P 的任何临时私钥，算不出 P↔X 的密钥（单测证明，见 P5）。M 若主动替换临时公钥做中间人，属于需求 §15 已接受的「信任集中」风险。

### 3.6 摘要与加载门（提问方 → Mac 对端）

- `request.new{rev, request: <桩视图>, full:false, summary}`：桩视图 = `RequestView` 去掉 `ask` / `confirm`（保留 id、kind、created_at、agent、project、source、content_hash）；`summary` 为 `RequestSummary`（沿用现有结构），**新增**可选字段 `questions: u16`、`attachments: u16`（`#[serde(default)]`）。`inbox.snapshot` 对 Mac 对端同样用它。
- 对端关闭「预览」时 `preview` 置空；其余字段不变。
- `request.get` → `request.full` 沿用；提问方记录 `loaded[(peer, requestId)]`。`confirm.decide` 来自 `kind=mac` 且未 `loaded` → `confirm.ack{status:"rejected", reason:"not_loaded"}`。
- 手机路径零改动，由回归测试保证（P2）。

### 3.7 hello 与在线状态增量

- Mac 的在线状态 `fg`：1 = 接收中，0 = 已暂停（全局「暂停接收所有 Mac」）；现状恒为 0，iOS App 不读 Mac 的 `fg`。
- `Hello.paused: bool`（`#[serde(default)]`）：答题方对某个提问方单独暂停；提问方记 `paused`，回 `hello.ack` + 空快照，此后不向该对端投递；恢复 = 新 `hello{paused:false}` → 提问方补发快照。
- 答题方 hello 的 `deviceName` 用本机名称。

### 3.8 兼容与夹具

- 只新增可选字段与 Mac 专用类型；iOS App 对未知类型与多出的字段应忽略（P0 核对 Swift 解码）。
- `ios/Fixtures/wire-v1/` 保持不变（现有样本不含新字段）；新字段与 Mac 专用类型的样本放 `ios/Fixtures/mesh-v1/`（`UPDATE_MESH_FIXTURES=1 cargo test` 生成并校验，仅 Rust 读取）。改线协议先改夹具。

## 4. Mac 端

### 4.1 存储与密钥

- `PhoneRecord.kind`（`phone` 默认 / `mac`）；Mac 记录没有推送与 follow；`state` 沿用 `PendingConfirm | Active`；`mesh.preview: bool`（该对端的摘要是否带预览，默认 true）。
- `hosts: Vec<HostRecord>`：我作为答题方的记录（`mac_id, name, alias, created_at, last_seen_at, via?, paused`）。
- `invites`（邀请与引荐票据：`ticket_id, kind, expires_at, state, requester?, intro?`）、`joins`（我发出的加入：`host_id, sas, state, expires_at`）、`mesh_outbox`（`circle.*` 持久化发件箱）、`answer_outbox`（待发的答案 / 确认）、`seen`（每个提问方最近见过的请求 id，用于不重复提醒）。
- 秘密：四把方向密钥沿用钥匙串 `ios_pairing_account(id)`（提问方角色）并新增 `ios_mesh_host_account(id)`（答题方角色）；票据秘密与临时私钥存 `ios_mesh_ticket_account(ticketId)`；钥匙串不可用时回退 `keys.json`（0600，沿用）。`state.json` 只放标识、时间与状态。
- 旧版本读到新的 `state.json`：未知字段被忽略，Mac 记录被当成没有推送句柄的手机显示（无害）。
- 日志（`daemon.log`）加 `ios-mesh` 标签，不含邀请码、票据秘密、密钥。

### 4.2 提问方扩展（`gateway/peers.rs`）

- **在线状态**：`on_presence` 现在只认 `r == "phone"`；扩展为对已知 `kind=mac` 设备接受 `r == "mac"`（`fg` = 接收中）。`presence_cache` 的上限逻辑沿用。
- **投递**：`deliver()` 的目标循环按类型分流——手机沿用现有逻辑（含 `ios-alert-timing` 的待通知规则）不变；Mac 发摘要（§3.6），不走推送回退、不等 3 秒确认；对端不在线 / 暂停 / 未 hello 时请求留在待答表，hello 快照补齐。Mac 对端的 `request.ack` 可有可无。
- **白名单**：`dispatch` 对 `kind=mac` 只放行 hello、ping、`request.get` / `ack` / `seen`、`answer.submit`、`confirm.decide`、`asset.get`、`upload.*`、`unpair`；其余（`agents.*`、`follow.*`、`watch.*`、`interject.*`、`task.*`、`slot.*`）回 `error{not_supported}`，单测逐一覆盖。
- **加载门、暂停、上限**：见 §3.6、§3.7；每个对端待答条目超过 `MAX_PENDING_PER_PEER` 时丢弃新到的摘要并在状态里记一笔。
- **对端不进 Watch 并集、不参与 `slot.state`**；`hello.ack` 的 `slot` 填默认值（答题方忽略）。
- **视图**：`GatewayState` 增 `peers`（名称、在线 / 暂停、最后在线、待答数、引荐人、偏好）、`invites`、`joins`、`approvals`。

### 4.3 答题端引擎（`gateway/answerer.rs`、`answerer_submit.rs`）

在同一个 Gateway Actor 里（共用传输、发件箱令牌桶、时钟、presence 与存储）新增角色，对标 `PhoneEngine`：

- **会话**：每个提问方一个 `HostRt`——在线状态、同步阶段（none → helloSent → acked → synced）、epoch、inbox rev、去重窗口、通知表 `notices: requestId → RemoteRt{summary, view?, state: Unloaded | Loading | Loaded | Failed}`。hello 重试（5 s × 3）、epoch 变化重新 hello、`request.new` 按 rev 顺序应用（重复丢弃、断档请求 resync）、快照对齐（已结束的移除、新增的建通知）全部沿用手机侧语义。
- **加载**：`load(host, request)` → 发 `request.get` → `request.full` 填 `view` 并通知 daemon；自动加载开关打开时收到摘要即加载。重复 `request.get` 防抖。
- **作答**：答案与确认进持久化 `answer_outbox`，带 `clientMsgId` 发送，30 秒无回执标失败可重试（同一标识）；回执 `accepted` / `lost` / `expired` / 其它拒绝分别回调 daemon。提交时对端离线直接拒绝并保留草稿。
- **资产与上传**：`asset.get` 拉附件到本机临时目录（带大小上限与 TTL 清理）；回复附件走 `upload.*`（限额沿用手机路径）。
- **不做**：推送句柄、Agent 列表、Follow、Watch、插话、任务、本机历史。
- **与 daemon 的接口**：经 `Host` trait 增加的回调与 `GatewayHandle` 增加的命令（具体签名以实现为准）：
  - 回调：`remote_upsert`（建 / 更新未加载条目，附「新到 / 唤醒批量 / 静默」的提醒类型）、`remote_loaded`、`remote_load_failed`、`remote_removed`（别处作答 / 过期 / 取消 / 提问方解除 / 条目超限）、`remote_submit_state`；
  - 命令：`load_remote`、`submit_remote_answer`、`submit_remote_confirm`、`fetch_remote_asset`、`set_host_paused`。

### 4.4 配对与圈子（`gateway/mesh_pairing.rs`、`mesh_circle.rs`）

- **邀请方状态机**：`Waiting → Requested{macId, name, ek, sas} → Approved → Done`，另有 `Rejected` / `Expired` / `Cancelled` / `Contested`；`Requested` 时向 daemon 发起批准卡（§4.6），决定回来后发 `peer.accept` / `peer.reject`。状态与秘密落盘，Daemon 重启后重新订阅各 `p/{ticketId}`、重放保留消息。
- **加入方状态机**：`Joining{code, sas} → Accepted → Done`，另有 `Rejected` / `Expired`；发出 `peer.request` 后订阅同一话题等 `peer.accept`（保留消息，晚上线也能收到）；收到后存密钥、发 `peer.done`、清除自己的保留请求。
- **引荐**：批准完成后，批准方对圈内每个其他成员入队 `circle.intro`；X 回 `circle.ticket`，M 转 `circle.relay` 给 P；P 对每张票据发 `peer.request`，X 凭 `intro` 绑定自动批准。M / X / P 任一方离线都只是排队。
- **撤销**：本地删除两个方向的记录与密钥，并入队 `circle.revoke` 给圈内其他成员；收到方自动删除对应记录并提示用户（需求 M14）。被撤销方在各端的远端条目一并清除。
- **清理**：过期邀请 / 票据 / 未完成记录由现有 tick 清理；保留消息用空载荷清除。

### 4.5 Daemon 接入（`daemon/runtime/ios_mesh.rs`、`request.rs`、`channels/mac_peers.rs`）

- **RemoteEntry**（P0 确认）：倾向在 `RequestEntry` / `ConfirmEntry` 上加 `remote: Option<RemoteLink{host_id, host_name, request_id, state}>`，用新的 `create_remote*` 创建，而不是给 `InteractionEntry` 加第三个变体（后者要改所有 `match`）。它复用登记表、`seq`、托盘、弹窗宿主分配、草稿与稍后答；`ShowPayload` 带占位内容（摘要卡所需的预览 / 计数）与新的可选 `remote` 信息。关闭历史记录（`record_history_enabled = false`），不挂 IM / 手机渠道。
- **终态由提问方裁决**：弹窗的 `Answer` / `ConfirmAnswer` 到达 daemon 时，若条目是远端，则交给 `RemoteBridge::submit`（答题端发件箱），**不调用** `coordinator.try_submit`（否则会立刻成为本地终态，也会误触本机待办出队）；收到 `accepted` 或 `request.resolved` 后，由 bridge 走与「被其它渠道抢答」相同的关窗路径结束条目。具体分流点与关窗路径 P0 确认。
- **MacPeersChannel**（`id = "mac"`）：提问方侧的 `Channel` 实现，登记到每个本机请求的 Coordinator，不经过 `select_im_delivery_candidates`（所以不受活跃槽管辖）；`interrupt` 时由 Gateway 向已收到摘要的对端发 `request.resolved`。与现有 `IosChannel`（手机）并列、共用同一个 Gateway（P0 确认是拆成两个 Channel 还是给 `IosChannel` 加范围参数）。
- **激活联动**：①`select_im_delivery_candidates` 的兜底条件改为「Popup 不可用 **且没有在线对端** 且候选为空」；②赢家 id 为 `mac` 时，`finish_request_bookkeeping`（含确认路径）把活跃槽置为 `popup`；③`reply_channel_text("mac")` 与 `channel_label` 对 `mac` 的处理（无反激活文字；回执文案用对端 Mac 名称，来自 `ChannelResult.source`）；④对端不进 `autoEndWatch` 与 Watch 并集（不需要改，仅加测试）；⑤与 `ios-alert-timing` 的衔接（需求 §6.4）：对端对某题的 `request.get` 在提问方走与手机 `request.seen` 同一条路径（记为已看过，手机不再待通知）；Mac 对端没有推送句柄，不进推送对象。
- **闲退**：远端条目在登记表里，天然算在途；按需求 M13，存在已配对对端且 iOS 渠道启用即视同常驻（不改写 `general.daemonLifecycle`；沿用 Cursor Desktop 集成「独立保活」的先例，P0 确认具体守卫点）。
- **隔离**：回复历史、待办出队、重复提问收敛、Git 信息、Agent 注册表、`ask-received` hook 对远端条目都不触发（需求 §8.3），每项在 `create_remote*` 路径上有单测。
- **项目键命名空间**：远端条目的 `project` 带 `mac:{hostId}:` 前缀，避免两台 Mac 上同路径的仓库在 Sidebar 里混成一组；显示名去掉前缀并加 Mac 标签。

### 4.6 批准卡

- 载体：`create_internal_confirm`（标题「批准新 Mac 加入」，上下文含新 Mac 名称与安全码、提示「允许后它可以回答你所有 Mac 上 Agent 的提问，包括批准权限请求」，选项「允许 / 拒绝」，TTL = 邀请剩余时间）。
- 投递（P0 确认）：现有 `create_internal_confirm` 只面向「来源渠道」，调用方各自 `start_feishu` / `start_dingtalk`…；批准卡需要一个**多面投递**入口——本机作答窗口、所有已配对手机（`confirm::start_ios`）、所有在线对端（摘要先行 + 加载门，沿用确认路径）。不看活跃槽，也不改活跃槽。
- 首个决定生效，其余面收到「已在别处处理」；结果回到邀请方状态机（`peer.accept` / `peer.reject`）。没有任何可用面（无头、无手机）时，卡片只在设置页 / `channel ios mac approve` 里处理。
- 预期手机端不需要改 App：卡片就是一张普通的确认卡（P0 验证 `ConfirmView` 能完整表达）。
- **手机推送立即**（需求 §13 第 18 条）：批准卡对手机不等「Mac 空闲 D」；需要 `ios-alert-timing` 的 `deliver()` / `flush_due_notifications` 支持按请求覆盖（例如请求带「立即推」标记），P0.9 确认。

### 4.7 弹窗前端与 IPC

- `ShowPayload` 增可选 `remote: { hostId, hostName, state, summary{preview, questions, attachments, size}, … }`（`#[serde(default)]`，不 +1）。
- 需要新增的消息（P0 列全并按 overview「IPC 协议号」判断）：弹窗 → daemon 的「加载远端内容」「取远端附件」，daemon → 弹窗的「远端状态更新」（已加载 / 失败 / 提交中 / 已结束）。**新增枚举变体预计需要 `PROTOCOL_VERSION` 4 → 5**；若 P0 能证明可用现有消息 + 可选字段表达则不 +1。+1 时：改常量、overview 历史补一行、`daemon-graceful-drain.md` 记过渡期影响。
- 前端（Vitest）：Sidebar 的 Mac 标签与「Mac + 项目」分组；摘要卡与「查看并回答」；未加载 / 加载中 / 已加载 / 失败 / 提交中 / 已结束状态；确认卡顶部「此操作将在 X 上执行」；隐藏聚焦终端、打开项目目录、Git 信息、「在访达中显示」；远端附件经 daemon 拉到临时路径后复用现有预览面板（只读）；中英文案。
- 到达提醒沿用现有气泡 / 提示音，文字取摘要；唤醒批量只提醒一次。

### 4.8 设置页与 CLI

- `IosOp` 增量：`MacInviteStart` / `MacInviteCancel` / `MacJoin{code}` / `MacJoinCancel` / `MacApprove{ticket_id, allow}` / `MacPause{id, paused}` / `MacPreview{id, on}` / `MacRevoke{id}` / `MacRename{id, name}` / `MeshSettings{auto_load}`；`IosResult` 增对应返回；`GatewayState` 见 §4.2。（与 §4.7 同一次 IPC 变更。）
- `IosChannelCard.vue`：设备列表分「手机」「Mac」两栏；「添加 Mac」两个入口；待批准列表；对端行的开关（暂停接收、摘要不带预览）；全局「自动加载」「暂停接收所有 Mac」；Daemon 常驻提示；流量显示含互通。
- CLI（`cli/ios_cmd.rs`）：`channel ios mac invite | join <码|-> | approve <编号> [--deny] | pause|resume <名称>`；`ios devices` 加类型列；`ios revoke` 接受 Mac。邀请码经 stdin / 文件读入，不进命令行参数（避免进 shell 历史）。
- 配置：`channels.ios.mesh.autoLoad`（默认 `false`）。

### 4.9 生命周期、健康与触点

- **Daemon 常驻**：按需求 M13 实现（见 §4.5「闲退」）。
- **健康**：`channel test ios` 与设置页展示对端状态、最后在线、待发队列长度、最近一次错误；`channel ios status` 同。
- **Dev Instance**：两个 Dev Instance（各自 `ASKHUMAN_HOME`、各自 Mac 标识）可在一台机器上模拟「家」与「公司」。
- **唤醒**：对端醒来 hello 后由快照补齐；批量到达只提醒一次。
- **卸载 / 关闭渠道**：关闭 iOS 渠道即离线；对端看到离线；本机的远端条目被取消。

## 5. 阶段与验收

每个阶段结束前：跑自动测试；Mac 端改动按 AGENTS.md 先 `./scripts/install.sh`，再用新装的 `AskHuman` 验证。涉及真实 Agent 的启动 / 发送 prompt 的测试必须先经 AskHuman 获批。每个阶段的提交遵循 §9。

### P0 预研门（不写产品代码；产出 `docs/investigations/mac-mesh-p0.md`）

- 0.1 **Actor 同体**：读 Gateway Actor（`mod.rs` 事件循环、`on_presence` / `on_up` / `dispatch`、`PhoneRt`、`Outbox`），确认「答题端作为同一个 Actor 的新角色」可行并估规模；不行再评估事件分流。
- 0.2 **RemoteEntry**：读 `daemon/request.rs`（`RequestRegistry::create`、`RequestEntry` / `ConfirmEntry`）、`daemon/runtime/mod.rs` 的 Popup 提交分发与关窗路径、`Coordinator`；确认「`remote` 链接 + 终态由对端裁决」的分流点，列出需要改的 `match` / 调用点。
- 0.3 **Popup IPC 面**：读 `ipc/mod.rs`、Popup Host 与前端 store；列出所有需要新增的消息 / 字段；按 overview 规则判断 `PROTOCOL_VERSION` 是否 +1，并尽量用可选字段规避。
- 0.4 **批准卡载体**：确认 `create_internal_confirm` 配多面投递（作答窗口 / `confirm::start_ios` / 对端确认路径）的可行性与改动点。
- 0.5 **保留消息**：①`LoopbackBroker` 是否支持保留消息的覆盖、清除、晚订阅补收，不足则扩展并加测试；②**真实 EMQX Serverless** 上的行为（保留消息发布 → 晚订阅者补收 → 空载荷清除；TLS 8883 与 WSS 8084 都试；数量与大小限制）。②沿用 iOS 渠道「端到端测试可对真实服务运行」的授权方式：一个 `#[ignore]` 的 Rust 测试经 `AH_E2E_REAL_FROM` 在内存里取连接串，Agent 不读取密码，消耗的是几 KB 流量；运行前经 AskHuman 请用户授权并给出已配置好 iOS 渠道的实例。
- 0.6 **Swift 与夹具容忍度**：确认 `AHProtocol` 对 `RequestNew.summary`、`RequestSummary.questions` / `attachments`、`Hello.paused` 等多出的字段与未知消息类型的解码行为；确认 Swift 测试按文件名读取 `wire-v1`，不会扫描整个夹具目录。
- 0.7 **测试台**：设计 `TestMesh`（多个 Actor 共用一个 `LoopbackBroker`，每个有自己的 `TestHost` 与虚拟时钟）；确定端到端驱动的形态（两台一次性 Mac + `ios/Tools/local-broker`；用 `AskHuman` CLI 在 A 提问；在 B 侧用模拟的 Popup Host IPC 扮演作答窗口，优先复用现有协议而不新增产品面 CLI）。
- 0.8 **远端附件预览**：读弹窗附件预览链路对「本地路径」的假设，定远端附件的临时目录、TTL 与清理策略。
- 0.9 **与 `ios-alert-timing` 对接**：确认它已合并，读合并后的 `Gateway::deliver` / `flush_due_notifications` / `on_request_seen`、`Host::mac_idle_secs` 与候选筛选（`with_ios`）；落实需求 §13 第 17、18 条的挂接点（对端 `request.get` → 已看过；批准卡的「立即推」覆盖）；需求 M16 已定 v1 不做「在对端工作时抑制手机推送」，所以不需要评估在线状态携带空闲时长（留给后置的「在席过滤」）。
- **门禁**：任一项暴露硬伤（保留消息在真实 EMQX 上不可用、Coordinator 无法承载「终态由对端裁决」、必须改变已确认决策……），立即停下，经 AskHuman 把证据与选项交给用户，不自行换方案。

### P1 配对内核（Rust，无 UI）

- 交付：`derive_mesh_keys`（互惠派生）与向量夹具；`ahmac1` 邀请码；`peer.*` 消息；邀请 / 加入 / 批准的异步状态机（保留消息、落盘、竞争保护、时间戳窗口、清理）；记录与密钥存储（含 `kind`、`hosts`）；`TestMesh`；批准决定由测试注入（真批准卡在 P5）。
- 验收：多 Actor 回环测试——双方不同时在线完成配对（H 先离线 / O 先离线 / 批准时 O 离线）；过期、重复、拒绝、竞争（`contested`）、broker 指纹不一致、满员；重启后恢复（存储重载）；篡改与重放被拒；四把方向密钥互不相同且双方一致，安全码一致；日志与命令输出无秘密。

### P2 提问方扩展

- 交付：§4.2 全部；§3.6、§3.7 的线增量与夹具；`GatewayState` 增量。
- 验收：回环测试里「Mac 型客户端」收摘要 → `request.get` → `request.full` → `answer.submit` 全链路；未加载的 `confirm.decide` 被拒；暂停 / 恢复；白名单外消息逐一被拒；条目上限；**手机路径回归**（既有 `ios::` 测试全部通过，手机收到的 `request.new` 与现状逐字节一致）。

### P3 答题端引擎

- 交付：§4.3 全部（无 UI）；`Host` 回调与 `GatewayHandle` 命令；`seen` 去重；唤醒批量。
- 验收：两个真实 Actor（提问方 + 答题方）的回环端到端——提问 → 摘要 → 加载 → 作答 → 采纳 → 其余端清理；对端离线 / 睡眠 / 重启 / 回执丢失 / 重复 / 乱序旧快照 / rev 断档 resync；首答胜出与 `answeredElsewhere`；附件拉取与回复上传；确认流程与加载门；答题端不转发（三 Actor 下无成环）。

### P4 Daemon 接入与弹窗（第一个用户可见版本）

- 交付：§4.5、§4.7 全部；弹窗前端；IPC 变更（若 +1 则同步 overview 与 drain 文档）；隔离清单逐项单测。
- 验收：两个 Dev Instance + 本地 broker：用 `AskHuman` CLI 在 A 提问 → B 的作答窗口出现摘要并有到达提醒 → 查看 → 作答 → A 的 CLI 输出与本机作答一致；自动激活开 / 关两种配置；本机无弹窗兜底；`daemon stop` / 睡眠恢复；Vitest；安装后用真实窗口截图核对。
- **用户 gate（评审时已确认保留）**：交付两个 Dev Instance 的演示截图 / 录屏，用户看过最大的 UI 变更后再继续 P5–P7。

### P5 批准卡与圈子引荐

- 交付：§4.6 批准卡（本机 + 手机 + 对端，首个决定生效）；§4.4 引荐、撤销、发件箱；被引荐设备通知。
- 验收：三个 Actor / 三个实例：加入圈子一次批准；离线成员上线后补齐；撤销后各端通道失效；**引荐人无法解密 P↔X 的流量**（用 M 持有的全部材料尝试派生，单测失败即通过）；批准卡在 H 本机与手机任一处决定、另一处消失；iOS App 对批准卡的呈现（模拟器截图，P0.4 的预期在此验证）。

### P6 设置页与 CLI

- 交付：§4.8 全部；中英文案；`docs/wiki/ios-setup.md` 的「添加另一台 Mac」草稿。
- 验收：Vitest；安装后在 Dev Instance 走通邀请 / 加入 / 批准 / 暂停 / 预览开关 / 撤销；CLI 全流程（一次性 Mac）；邀请码不进命令行参数与日志。

### P7 加固与验收

- 故障矩阵（§6）逐项自动化 / 手工；流量统计含互通；安全自检（日志无秘密、跨配对访问被拒、已撤销设备被拒、白名单、邀请码不入日志）；延迟分段测量（摘要出现、点查看到可作答、提交到回执），报告原始分布与全部超时；真实 EMQX 上的端到端（两台一次性 Mac，沿用 `AH_E2E_REAL_FROM` 授权方式）；真实 Agent 的启动 / 发 prompt 测试经用户批准后进行。
- 文档收尾：`docs/overview-ios.md`（Mac 互通的实现地图）、`docs/overview.md`（仅当仓库级地图或不变量变化：IPC 历史、iOS 渠道小节加一行指针）、`docs/wiki/ios-setup.md`（含英文）、需求状态、`docs/PROGRESS.md`（删本节，登记遗留项）。
- **用户验收**：用户在家 / 公司两台真实 Mac 上走一遍 §5.2 的流程与日常提问。

## 6. 测试矩阵

- **自动化（Rust，纯逻辑）**：互惠密钥向量（双方一致、四把互异）；邀请码编解码与篡改；`bf` 指纹；时间戳窗口（保留消息放宽、正常通道不放宽）；摘要 / 桩视图构造（预览开关、截断、计数）；白名单；加载门；`MAX_*` 上限。
- **自动化（Rust，多 Actor 回环）**：配对状态机（异步、离线、过期、拒绝、竞争、重复、满员、重启）；引荐（成员离线、被引荐方离线、M 离线、撤销、乱序、重复投递幂等）；提问 / 摘要 / 加载 / 作答 / 抢答 / 清理；`LoopbackTransport` 注入丢包 / 乱序 / 断线 / 重复；首答胜出与三方并发；睡眠恢复与快照对齐；`answer_outbox` 重发幂等；三 Actor 不成环。
- **自动化（daemon）**：`create_remote*`；弹窗提交分流（不触发 `try_submit`）；`select_im_delivery_candidates` 兜底条件；赢家 `mac` → 活跃槽 `popup`；隔离清单（历史、待办、收敛、Git、注册表、hook）；idle 守卫。
- **自动化（与 `ios-alert-timing` 衔接）**：对端点开（`request.get`）后手机不再待通知；对端没点开的不影响手机的待通知；提问被对端答 → 沿用「推前不发、推后替换」；批准卡对手机立即推（不受 D 影响）；对端没有推送。
- **自动化（前端）**：Sidebar 标签与分组；摘要卡与状态机；确认卡提示；隐藏的本机入口；设置页。
- **端到端（两台一次性 Mac + 本地 broker）**：配对（CLI）、提问、作答、抢答、撤销；再对真实 EMQX 跑一遍（授权后）。
- **手工**：两个 Dev Instance 的真实窗口（截图）；睡眠恢复；Wi-Fi / 切网；公司网络（用户已口头确认）；家 / 公司两台真实 Mac。
- **安全**：日志 / 命令输出 / 提交中无 broker 密码、邀请码、票据秘密、方向密钥（沿用 iOS 渠道的自检测试并扩展）。

## 7. 用户手动步骤（我不接触任何密钥）

1. 评审本计划与需求并回答需求 §14 的问题——已于 2026-10-27 完成（结论见需求 §17）。
2. 开工前确认 `ios-alert-timing` 已合并，并确认 worktree 与是否挂渠道预设（经 AskHuman）。
3. P0.5：授权对真实 EMQX 跑保留消息检查，并指定已配置好 iOS 渠道的实例（密码只在测试进程内存里，我不读取）。
4. P7：授权用一次性 Mac 对真实 EMQX 跑端到端；真实 Agent 测试逐次批准。
5. P7：在家 / 公司两台真实 Mac 上升级、导入连接串（两台已有同一个 EMQX 账号则省略）、按需求 §5.2 配对并日常使用。

## 8. 风险与开放验证项

| 风险 / 待验证 | 处理 |
|---|---|
| 保留消息在 EMQX Serverless 上的行为与限额 | P0.5 实测；不可用则停下来请用户决定（备选：邀请方在线才能完成配对，或改用其它异步载体） |
| Coordinator 难以承载「终态由对端裁决」 | P0.2 评估；备选：远端条目用独立的轻量终态机，只复用登记表与弹窗通道 |
| 弹窗 IPC 面比预想的大 | P0.3 列全；尽量加可选字段；确需新增变体则 +1 并按 overview 同步 |
| 批准卡的多面投递需要改动较多渠道代码 | P0.4 评估；备选：批准卡先只发本机作答窗口 + 手机，对端 Mac 的批准后置（需先经用户确认，因为需求写了「+ 圈内对端」） |
| Gateway Actor 因新增两个角色而过于庞大 | 按功能拆文件（§1）；P0.1 若估算不可接受，评估事件分流 |
| 手机路径被改坏 | P2 的「手机路径逐字节一致」回归；`wire-v1` 夹具不变 |
| 邀请 / 引荐的持久化状态在崩溃时不一致 | 先落盘后发送；消息幂等（`introId` / `ticketId`）；重启恢复测试 |
| 对端数量增长造成订阅 / 在线状态风暴 | 上限 8；在线状态消息本来就小；P7 量一次 |
| 远端内容渲染安全 | 沿用本机清洗；远端附件只读；路径不外泄到本机动作 |
| 无推送唤醒 + 公司设备休眠 | Daemon 常驻（需求 M13）；文档与设置页说明 |
| 开发期间主工作区有其他会话的未提交改动 | 用独立 worktree；不改动他们的文件；`docs/PROGRESS.md` 只做局部编辑 |
| 与 `ios-alert-timing`（改同一批代码、语义上也依赖它）冲突 | 它先合并；开工前核对；P0.9 确认接口；本计划不重复实现它的任何规则 |

## 9. 提交与文档规范

- 提交信息遵循 Conventional Commits；本功能的所有提交（Mac 端、前端、文档）带 `Release-Note: skip` 尾注，直到用户决定对外发布；类型按改动性质选 `feat` / `fix` / `test` / `docs` / `chore`，范围用 `ios` / `mesh` / `popup` / `ipc` / `settings`。
- 代码注释用英文；本计划与需求文档用中文。
- 改 `ipc/mod.rs` 前再读一遍 overview 的「IPC 协议号」；+1 时同步 overview 历史与 `daemon-graceful-drain.md`。
- 改线协议先改夹具（`mesh-v1`；`wire-v1` 只在确需时改，并同步 Swift）。
- 每阶段完成后检查 `docs/overview.md`、本计划与需求是否已不准确，只改对应文档；overview 只放架构 / 不变量，Mac 互通的实现地图写进 `docs/overview-ios.md`。
- 阶段记录与差异写在本计划 §10，不写进 overview；任务进度只放 `docs/PROGRESS.md`，完成后删除对应 section。
- 开发开始后，计划与需求不得擅自变更；需要调整先经 AskHuman 确认，并把结论追加到需求文档「反馈意见」。

## 10. 实施记录（与计划的差异）

（开工后填写）
