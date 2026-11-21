# iOS 渠道同步流量收敛

2026-11-17 草案；同日经 Owner 评审，T2 → (b)、T4 → 不显示、T5 → 只压列表类、T6 → 加，并决定「全部一次做完」。
**已实现**（R1 + L1 + C + T2 b + T6），实现记录、实测数字与和草案的差异见 §8。下文 §1–§7 保留评审时的草案文字，
T 表（§7）已按最终决定更新状态。**2026-11-21 已用 EMQX 控制台与两台 Mac 的 `trafficBreakdown` 核对落地效果，见 §9；
§1.1 的收益数字是评审时的推算，与实测的对照见 §9.4，以实测为准。**

起因：Owner 自己的 EMQX 用量超出免费额度（按三台 Mac 的发送量折算，本月到 11-17 已约 1.5 GB，约 87 MB/天，整月约 2.6 GB；
口径与推算见 §1.1，2026-11-21 用 EMQX 自己的报文流量核对过，11-02 至 11-16 日均 92 MiB，见 §9.1），而 [iOS 渠道](ios-channel.md)定的预算目标是
「典型使用（每天 100 个请求、实时观察 1 小时）每月 ≤ 300 MB」。
2026-11-17 量过流量花在哪（§1）后，Owner 选定三件事：进前台只补变化（R1）、消息压缩（C）、Agents 流按需推送（L1），
并补充：「Agents 页面也不用全量推吧？我经常七八个 Agent 一起跑，实际 Watch 的同时最多一个」。据此 L1 不做成
「按页面开关」，而是**按 Agent 的关注度分级**（§3.2）。

关联：[多设备报文流量统计](ios-traffic.md)（台账）、[Watch 瘦帧与看门狗](agent-observation-latest-text.md)（§7.3）、
[Follow 收窄](ios-follow-narrowing.md)、[Mac 互通](mac-mesh.md)（未实现，依赖 `s/+`）、
[iOS 渠道总览](../overview-ios.md)（「活着」的手机）、
[附件流量收敛](ios-attachment-traffic.md)（接手 §9.3 的附件流量和本稿不做的二进制信封；2026-11-21 已出规格与计划，未开工）。

## 1. 测得什么

2026-11-17 在 Owner 的真实 broker 上做过一次只读的被动观察（只数主题类别、大小、时间，不解密、不落盘内容），
并用本机 broker + 真实手机引擎搭了一次性测试台（1 台 Mac、17 个合成会话；未入库）。

**已测**

- **后台为零**：手机在后台时，三台 Mac 一条消息都不发（真实 broker 上连续 10.5 分钟）。
- **进前台一次是整套重来两遍**：手机重连 → `hello` → Mac 回 `hello.ack` + 五个快照（inbox / agents / follow / slot / away）
  + `traffic.usage`（测试台 12.0 KB，其中 agents 快照 9.6 KB）→ 手机报「前台」presence → Mac **把五个快照再整套发一遍**
  （10.6 KB，占 Mac→手机 22.6 KB 的 47%）。真实 broker 上一次三台 Mac 的进前台，30 秒内共 140 KB，每台的快照都成对出现、
  大小相同。原因在 `gateway/mod.rs` 的 `on_presence`：手机「变成活着」时无条件 `send_snapshots`。它的本意是补上 hello 与
  前台 presence 之间漏掉的变化（App 在第一次同步完才报前台，这段时间的新请求不会实时推给它），但没有判断这段时间到底变没变。
- **前台期间按每一步推整条 Agent**：`agent_sig` 把除 `activeElapsedSecs` 以外的字段全算进签名，`currentTool` / `turnSteps` /
  `lastActivityAt` 随每个工具调用变；每变一次，Mac 就给**每台前台手机**补发这个 Agent 的整条 `AgentView`（信封约 0.8–1.0 KB）。
  Mac 只知道手机在前台，不知道它停在哪一页，停在收件箱也照推。测试台：一个 Agent 每分钟变 12 次 ≈ 9.8 KB/min，30 次 ≈ 23 KB/min
  （每条增量 797 字节）；空闲 90 秒只有一条 `traffic.usage`；真机前台 2 分钟实测 7–20 KB/min。
- **Watch 期间一半是列表增量**：测试台 Watch 一个每分钟变 12 次的 Agent，一分钟 18.5 KB = 12 帧 7.9 KB（约 660 字节/帧）+ 被 Watch 的那个
  Agent 的 12 条列表增量 9.6 KB。这些增量同时是手机看门狗的信号源（§3.2），本稿不动，记在 §2。
- **压缩有效**：用这台 Mac 上 14 个真实会话的元数据按线上格式重建报文（含 base64url 与信封开销）：agents 快照 10.2 KB → 2.7 KB
  （−74%）；8 个会话 5.4 KB → 1.7 KB（−70%）；单个 Agent 增量 −29%…−34%。
- **台账（负载口径，两端各记一次）**：三台 Mac 本月到 11-17 发 697.8 MB（约 41 MB/天）、收 38.7 MB（约 2.3 MB/天）；手机一侧自 11-12
  才开始记（收 201.8 MB、发 14.0 MB），所以把全部记录相加（0.89 GiB）会少算，折算见 §1.1。
- **每次连接都会收到整个账号的 retained presence**：手机订阅 `s/+`，而它把收到的每条消息（含 presence）都计入台账。清理前每次连接灌入
  301 条（约 20 KB，与一个 Mac 的 hello 一轮同量级）；2026-11-17 清理后降到 19 条。

**当时没测到**（落地后已用 T6 的计数测出，见 §9.2）：Watch 帧的真实大小与占比、进前台与前台期间各占多少。进前台次数当时只有间接依据
（§1.1，约 100 次/天，误差大），实测约 66–70 次/天。下面 §1.1 的收益数字来自测试台、重建报文与台账总量的推算，实测对照见 §9.4。

### 1.1 预计收益（2026-11-17 的推算；实测对照见 §9.4）

**基线（台账实测折算）**：每条消息在 EMQX 按「发布」与「投递」各计一次。Mac→手机一天约 41 MB，手机→Mac 约 2.3 MB，合计
约 2 × (41 + 2.3) ≈ 87 MB/天，整月约 **2.6 GB**（超出 1 GB 免费额度约 1.6 GB，按 ¥1.5/GB 只是每月约 ¥2.4；真正的代价是要先充 ¥100）。
到 11-17 已约 1.5 GB，对得上 Owner 在 EMQX 控制台看到的 1–2 GB。

**拆分（推断，误差大）**

- 手机收到 19.2 万条，同期 Mac 只发了约 3.3 万条（Mac 月计数 10.95 万条 ÷ 17 天 × 5.2 天）；多出的约 16 万条是每次连接灌入的 retained
  presence，按 301 条/次折算约 525 次连接 / 5.2 天 ≈ **100 次/天**（含网络波动造成的重连；若当时幽灵 presence 更少，次数还会更多）。
- 一次三台 Mac 的进前台约 140 KB → 进前台约 14 MB/天，约占 Mac 发送量的三分之一。
- 其余约 27 MB/天：Agents 列表增量约 22 MB，Watch 帧、请求与其它约 5 MB（这 5 MB 是猜的）。Mac 月均每条 6.4 KB，比测试台的单条增量（0.8 KB）
  大得多；我推断是同一个 1 秒节拍里变的好几个 Agent 被合进一条增量，也可能有一部分是大的 Watch 帧或请求正文，T6 的计数会分清。

**各措施的预计效果（中心情形；一天的 Mac→手机量，整月为两端合计）**

| | Mac→手机 一天 | 整月 |
|---|---:|---:|
| 现在 | 41 MB | ≈ 2.6 GB |
| R1 | −6.6 MB（−16%） | ≈ 2.2 GB |
| R1 + L1 | −26 MB（−63%） | ≈ 1.04 GB |
| R1 + L1 + C | −31 MB（−76%） | **≈ 0.73 GB** |

推算：R1 去掉第二轮（−47% 的进前台量）；L1 之后剩下的增量约占现在的 12%（被 Watch 的那个，加状态切换等稳定字段变化）；
C 把 hello 一轮里的 agents 快照压 74%、剩余增量压 30%。

**敏感性**（逐项单独变化，其余取中心值，整月）：进前台 50 / 150 次/天 → 约 0.67 / 0.78 GB；L1 之后剩下的增量占比 5% / 25% → 约 0.66 / 0.85 GB；
不被这三项减少的那部分（Watch 帧、请求、其它，我猜 5 MB/天）为 3 / 10 MB → 约 0.62 / 1.0 GB。所以区间约 **0.65–0.85 GB/月，最坏约 1.0 GB**；
单靠 R1 或 R1 + L1 都没有稳稳回到 1 GB 以内，全做才有余量。

**不在估算内**：Watch 用得比预想多、新增设备或更多并行 Agent、手机→Mac 方向（本稿不动）。已做的 presence 清理省的约 2 MB/天只发生在
手机收到的那一条腿上，不在上表里（基线按清理后的 0.1 MB/天计）。

**2026-11-21 实测**：落地后除附件外两端合计约 13 MB/天，整月约 0.4 GB，低于上表的 0.73 GB；但附件（§2 明确不做）被手机拉取一次就是
几 MB，占落地后总字节的 92%，本月总流量因此仍到 1.67 GB。拆分与对照见 §9。

## 2. 范围

**目标**：Owner 本人的用法（七八个 Agent 并行、一天上百次进前台）整月回到 EMQX 的 1 GB 免费额度以内，当时的估算见 §1.1（约 0.65–0.85 GB/月）。
实测（§9）：除附件外的同步已经达到（约 0.4 GB/月），附件不在本稿范围，整月是否回到 1 GB 以内取决于附件用量；
附件另见 [附件流量收敛](ios-attachment-traffic.md)。

**做**：R1 进前台只补变化（§3.1）、L1 Agents 列表按关注度分级（§3.2）、C 消息压缩（§3.3）、T6 本地诊断计数（§7）。

**不做**

- 后台流量（已是 0）、Watch 帧本身、请求正文、附件、推送。
- 被 Watch 的 Agent 的列表增量（§1：与帧重复，约占 Watch 总量一半）：它是看门狗判断「列表说它变了、却没有帧」的信号源，
  要省得先改看门狗的设计，本稿不动；落地后若 Watch 占比确实高再单独立项。
- R2 会话续传（`hello` 带上次的 epoch 与各流 `rev`，只补差；手机现在在 `hello.ack` 时清空缓存，改动大）：本稿落地后按实测再议。
- R4 只订阅对端 presence 以取代 `s/+`：要先和 [Mac 互通](mac-mesh.md)对齐（它依赖 `s/+`）。真实 broker 上测试遗留的 retained
  presence 已在 2026-11-17 一次性清理（301 → 19 条；这是症状层面的数据清理，根治靠 R4 与测试自己清理自己的 presence）。
- 手机→Mac 方向的压缩（量小）、二进制信封（2026-11-21 起由 [附件流量收敛](ios-attachment-traffic.md) 接手）、AgentView 字段级补丁。
- 页面级开关（原 L1 设想）：理由见 §3.2。

## 3. 设计

### 3.1 R1：进前台只补变化

**规则**

- Mac 记下本会话已发给该手机的每个流的内容（基线）。手机「变成活着」时，只补发与基线不同的流，全部相同就什么也不发。
  基线随每次 `hello` 重置（手机在 `hello.ack` 时清空缓存，一切从 hello 的快照重新算起）；没有基线（例如 hello 时收件箱还没准备好、
  快照被推迟）一律按「不同」处理。
- 补发什么：
  - inbox / follow / slot / away：内容（含 `rev`）与基线不同就整份重发（量小）。比较 `rev` 是必须的：hello 与前台 presence 之间
    漏掉的新请求会让 `rev` 前进。
  - **agents：发增量，不发快照**。`send_agents_delta` 本来就是「只发与该手机基线（`agents_sent`）不同的 Agent」，catch-up 直接走这条路径
    （先刷新列表）。这点对你的用法很关键：七八个 Agent 并行时，hello 与前台 presence 之间几乎总有 Agent 动一下，补整份快照
    （约 10 KB）等于白省；补增量只补那几条。手机若对不上增量的 `rev`，现有的断档检测会自己要一份快照（`Resync`）。
- 不动：hello 本身仍回完整的五个快照。

**对现有守护用例的影响**：「变化必须补到」的语义不变，断言按新形态改写。

- `tests_requests.rs` 的 `a_request_that_came_between_hello_and_the_foreground_presence_is_caught_up`：inbox 有变 → 仍补快照，不用改。
- `tests_agents.rs` 的 `an_agent_change_between_hello_and_the_foreground_presence_is_caught_up`：由「补 `agents.snapshot`」改为「补 `agents.delta`，
  `rev == 基线 + 1`，之后增量接着 `rev` 往下走」。
- `a_phone_that_says_foreground_again_without_a_change_is_not_sent_snapshots` 保持；新增「hello 之后什么都没变 → 前台 presence 不补发任何东西」
  与「只有 agents 变了 → 只补 agents 增量」。
- [overview-ios](../overview-ios.md)「活着的手机」的不变量要同步改（它写「补发四个快照」，代码实际是五个，落地时一并更正）。

**Watch 重开（已核对读代码）**：`on_watch_open` 不要求手机已是前台，`rt.watches` 原先只在「收到非前台 presence」或 Mac 自己断开 broker 时清空，
所以正常进前台流程里 hello 之后登记的 Watch，在前台 presence 到达后仍然有效，第二轮快照没有「重开 Watch」的必要作用。
还有一个顺带的好处（按代码推断，未单独实测）：原先第二轮 agents 快照会让手机的 `reopenWatches` 再发一次 `watch.open`，Mac 对重复的
`watch.open` 会从头重发一帧完整文字；去掉第二轮后只发一次。
**唯一的耦合**：若一条「非前台」presence 夹在 `watch.open` 与前台 presence 之间到达（如上一个会话的遗嘱消息晚到），Mac 会清掉这个 Watch，
原先恰好由第二轮快照触发的 `reopenWatches` 补回；去掉第二轮后只剩手机的看门狗兜底（列表里该 Agent 状态变了却没有帧才会重开；
静止的 Agent 不会触发）。时序极窄，处理方式见 T2。

**T2 的最终设计（b，已实现）**：Mac 把 `rt.watches` 的清空从「收到非前台 presence」改为「收到新的 `hello`」（以及 Mac 自己断开 broker 时），
不再依赖时序不确定的 presence。离开前台的手机不会再收到帧（`tick_watch` 只服务「活着」的手机、`live_watch_total` 只数在线且前台的手机），
但登记保留到它的下一次 `hello`；`hello` 是手机新会话的开始，手机在 `hello.ack` 时本来就清空缓存、重新登记 Watch，所以旧登记此时失效是对的。
Mac 单边改动，旧手机也受益。

**兼容**：Mac 单边改动，不改线协议；已装的手机构建不用动，Mac 升级就生效。

### 3.2 L1：Agents 列表按关注度分级

**规则（对在 `hello` 里声明 `leanAgents` 的手机）**

- **活动字段**：`lastActivityAt`、`turnSteps`、`currentTool`（`activeElapsedSecs` 本来就不触发更新，手机自己推算）。其余是**稳定字段**
  （新会话、状态、标题、等待预览、排队数、结果、能否插话、关注标记、移除……）。
- 对这台手机，Mac 的 Agents 流：
  - 手机正在 Watch 的 Agent（Mac 已接受的 `watch.open`，每台手机最多 2 个）：**行为不变**，任一字段变化都发增量。Watch 看门狗要靠它判断
    「列表说它变了、却没有帧」（[agent-observation-latest-text.md](agent-observation-latest-text.md) §7.3）。
  - 状态为 `working` 且**没有被 Watch** 的 Agent：活动字段的变化**不触发**增量，只有稳定字段变化才发；发出的增量带发送那一刻的完整 `AgentView`，
    活动字段只用于排序，不保证新鲜。
  - 其余状态（等待 / 空闲 / 已结束）：不变。它们的活动字段只在状态切换时才动，本来就不频繁。
  - 手机开始或结束 Watch 某个 Agent 时，下一个 tick 按新的规则补发（开始 Watch → 立即带上最新活动字段）。
- 实现位置：比较签名的那一步（`send_agents_delta` 里的 `agent_sig`）对分级的 Agent 先去掉活动字段；基线仍存实际发出的内容。

**手机（`hello.ack` 的 `caps.leanAgents` 为真时）**

- 对「工作中且没有被 Watch」的行（T4）：不显示工具 chip、步数 chip，也不显示右上的相对「最后活动」时间（这几项不再实时，显示陈旧值比不显示更误导）。
  状态 chip、标题、项目、等待预览、排队数、结果照旧；已运行时长由手机按 `activeElapsedSecs` 自己推算，不受影响。
- 列表排序规则不变；工作中分组内的先后只在稳定字段变化时重排，不再随每一步抖动。
- Agent 详情页（正在 Watch）与现在一样实时。
- Mac 不声明 `caps.leanAgents`（旧 Mac）时，手机保持现状。

**为什么不做页面级开关**：分级之后，列表流只剩稳定字段，量已经很小，没必要再为它引入「当前在哪一页」的消息；而且 Todos 页的项目 Agent 点
读的是同一条流，按页面开关会让它过期。

**示意（假设，不是实测）**：8 个 Agent 并行、每个每分钟 6 次活动字段变化，现在前台约 43 KB/min（8 × 6 × 0.9 KB）；分级后这部分归零，
只剩状态切换等稳定字段变化。

**备选（不推荐）**：保留工具与步数 chip，由 Mac 每 15 秒给「工作中且没 Watch」的 Agent 发一条摘要消息（约 0.8–1 KB，≈ 4 KB/min）。
代价：新消息类型、新能力位、手机端合并逻辑和测试；换来的是行上的 chip 至多晚 15 秒。你说「Watch 的同时最多一个」，我认为不值。

### 3.3 C：消息压缩（Mac→手机）

**规则**

- 手机在 `hello` 里声明 `compression`（缺省 false）= 我能解开带 `z` 的信封。对这样的手机，Mac→手机方向、明文 JSON
  （`{"i","t","ts","b"}`）≥ 512 字节的**列表类消息**（范围见 T5）先 raw DEFLATE（RFC 1951，无 zlib 头尾）再加密；信封加 `"z":1`。
  压缩后没变小就不压。
- 信封：现有「出现 `z` 即拒收」对没声明的一方保持（旧端永远收不到 `z`）；新端只接受 `z == 1`，其他值一律拒收（给将来别的算法留号）。
  **推送信封不变，继续拒收 `z`**（通知扩展不解压、保持小）。
- AAD：信封带 `z` 时，在原 AAD（`"ah1|"+topic+"|"+f+"|"+k`）末尾追加 `|z1`；没有 `z` 的信封 AAD 不变，现有夹具与旧端不受影响。
  这样压缩标记也受认证，改 `z` 即认证失败。
- 解压上限：解压后明文不得超过 `MAX_PLAIN_BYTES`（512 KiB），超限或流损坏即拒收（防解压炸弹）。
- 标志以**最近一次 `hello`** 为准，不持久化：手机降级后下一次 hello 不带标志，Mac 立刻不再压缩。
- 流量台账按实际发布的信封字节计，自动受益。
- 库：Rust 用依赖树里已有的 `flate2`（现经 `zip` 间接引入）；Swift 用系统 `Compression` 框架的 `COMPRESSION_ZLIB`（即 raw DEFLATE）。

**侧信道（要 Owner 定，T5）**：先压缩后加密会让密文长度反映内容的冗余度（CRIME / BREACH 类）。可利用的前提是：攻击者能看到密文长度
（broker 与网络观察者本来就看得到长度），并且能把自选文字塞进一条**同时含秘密**的消息里反复观察。Agent 的提问与历史正文可能同时含两者
（例如被注入的仓库内容 + 凭据），所以草案只压**列表类**消息（`agents.snapshot`、`agents.delta`、`inbox.snapshot`）：它们占进前台流量的
绝大部分，内容是标题、项目、状态、预览，没有正文。

## 4. 协议与兼容

新增字段全部可缺省，缺省即旧行为（沿用 `lean_watch` 的做法：`#[serde(default, skip_serializing_if = "is_false")]`，
`wire.rs` 里有 `the_lean_watch_bit_of_a_hello_is_additive_in_both_directions` 这类测试可照抄）。
iOS 线协议 `PROTO` 仍为 1；本机 IPC 的 `PROTOCOL_VERSION` 与恢复存储（`daemon/recovery.rs`）不动。

| 位置 | 字段 | 语义 |
|---|---|---|
| `hello`（手机→Mac） | `leanAgents: bool` | 手机按 §3.2 显示，Mac 可对没被 Watch 的工作中 Agent 只推稳定字段 |
| `hello`（手机→Mac） | `compression: bool` | 手机能解开带 `z` 的信封 |
| `hello.ack` 的 `caps`（Mac→手机） | `leanAgents: bool` | Mac 会按 §3.2 对该手机分级；手机据此才隐藏活动字段 |
| 信封 | `z: 1` | 明文是 raw DEFLATE；只出现在协商后的 Mac→手机方向 |

| | 旧 Mac | 新 Mac |
|---|---|---|
| **旧手机**（现装构建） | 现状 | R1 生效；不分级、不压缩 |
| **新手机** | 现状：收不到 `caps.leanAgents`，界面照旧；Mac 不认识的 `hello` 字段被忽略 | 全部生效 |

夹具（`ios/Fixtures/wire-v1/`，Rust 与 Swift 共用）：`messages.json` 增加带新字段的 `hello` / `hello.ack`；`crypto.json` 增加一条带 `z` 的信封
（Rust 压缩+加密产生，Swift 解密+解压须得到同样明文）；Mac 从不需要解手机发来的压缩消息，所以 Swift 压缩方向只需单测。

## 5. 验收

测试台沿用 2026-11-17 的做法（本机 broker + 真实手机引擎 + 合成 Mac 会话），其中「进前台一次 + 30 次活动变化」转成常驻回归用例
（约 1 分钟，断言字节预算并留余量），不带 90 秒空闲段。

1. **R1**：进前台一次（1 台 Mac、17 个会话、之间没变化）Mac→手机从约 22.6 KB 降到约 12 KB，第二轮为 0；hello 与前台 presence 之间
   只有 agents 变 → 只多一条增量；只有 inbox 变 → 只补 inbox；既有守护用例按 §3.1 改写后全绿。详情页打开 → 退后台 → 回前台，
   Watch 帧恢复。
2. **L1**：没被 Watch 的工作中 Agent 的工具 / 步数 / 最后活动变化 30 次 → 0 条增量；状态切换 → 1 条；被 Watch 的 Agent 行为不变
   （含看门狗用例）；开始 / 结束 Watch 的切换时刻补发。手机在 `caps.leanAgents` 为真 / 假时的行展示各一例。
3. **C**：协商后列表类消息带 `z`、其余不带；未协商的手机永远收不到 `z`；往返夹具；解压超限与损坏流拒收；`z` 的 AAD 绑定；
   推送信封拒收 `z`。
4. 兼容矩阵四格各一条端到端用例。
5. **数字**（测试台，同样的 17 个会话）：进前台一次 Mac→手机约 22.6 KB → ≈ 12 KB（R1）→ ≈ 5 KB（R1 + C，评审时的估计；
   2026-11-21 真实账号实测：每次进前台 5.3 KB / 7.4 KB，见 §9.2）；
   一个 Agent 每分钟 30 次变化的前台流量约 23 KB/min → 0（没被 Watch 时）。真实占比以落地后的台账为准，不拿 300 MB/月当验收线
   （那是「典型」用法，七八个 Agent 并行属于重度）。

## 6. 落地顺序、验证与文档

1. **R1 先做**：只改 Mac、无协议变化，装机验证后，已装的手机构建立刻受益。
2. **L1 + C 同批**：都要手机新构建声明能力；Mac 先行（旧手机不受影响），再出 TestFlight（另行授权）。
   （Owner 最终决定一次做完：R1、L1、C、T2 b、T6 同一批实现，测试全绿后装机，随后发版与 TestFlight，见 §8。）
3. 每步之后用台账（`AskHuman channel ios status --json` 的 traffic）与一次只读被动观察核对「进前台」与「前台期间」的真实占比，
   [PROGRESS.md](../PROGRESS.md) 里「流量预算分段实测」那一条随之推进或收尾。
4. 落地时要同步的文档：[overview-ios](../overview-ios.md)（「活着的手机」不变量、Agents 分级与压缩各一条）、
   [plans/ios-channel.md](../plans/ios-channel.md) 的信封一节（「`z` 字段保留给未来压缩，v1 出现即拒绝」改为「协商后只在 Mac→手机方向启用」）、
   [ios-traffic.md](ios-traffic.md)（压缩后台账口径不变，一句话说明）、
   [wiki/ios-setup](../wiki/ios-setup.md) 与 `ios-setup.en.md` 里的「典型使用每月远低于 300 MB」（与实测对不上：先按 §1.1 改成估算值，2026-11-21 又按 §9 的实测改写）；
   主 overview 的全局地图与不变量不变，不改。

## 7. 决定

| # | 决定 | 状态 |
|---|---|---|
| T1 | R1：补发只补变化；inbox / follow / slot / away 内容有变整份重发，**agents 走现有增量路径**；基线随 hello 重置，缺基线按「不同」处理；不改线协议 | **已实现** |
| T2 | R1 去掉第二轮后 Watch 重开的极端时序（§3.1「唯一的耦合」）：(a) 接受，靠看门狗兜底；(b) Mac 把「清空 Watch」从「收到非前台 presence」改为「收到新 hello」（Mac 单边，旧手机也受益）；(c) 手机在发出前台 presence 后再 `reopenWatches`（要新构建，且会抵消「只发一次 `watch.open`」的好处） | **选 (b)，已实现**（Owner 2026-11-17：从根上让 Watch 的生命周期跟随会话，而不是跟随时序不确定的 presence） |
| T3 | L1：按 Agent 关注度分级；Watch 中的 Agent 不变，没被 Watch 的工作中 Agent 只推稳定字段；不做页面级开关；能力位 `Hello.leanAgents` + `caps.leanAgents` | **已实现** |
| T4 | 没被 Watch 的工作中行：不显示工具 / 步数 / 相对时间（§3.2）；备选是每 15 秒一条摘要消息，保留 chip | **选「不显示」，已实现**（Owner 2026-11-17） |
| T5 | C：raw DEFLATE，`z:1`，AAD 追加 `\|z1`，解压上限 512 KiB，推送信封继续拒收 `z`，只做 Mac→手机；压缩范围只含 `agents.snapshot` / `agents.delta` / `inbox.snapshot`；备选：所有 ≥ 512 B 的 Mac→手机消息（收益略多，侧信道暴露面含 Agent 正文，见 §3.3） | **选「只压列表类」，已实现**（Owner 2026-11-17） |
| T6 | 本地诊断计数：Mac 按「消息类型 × 方向」累计字节，只存本机、只进 `channel ios status --json`，不改线协议，用来回答「进前台 / 前台变化 / Watch 帧各占多少」 | **选「加」，已实现**（Owner 2026-11-17） |
| T7 | 落地顺序：R1 单独先上，L1 + C 同批随新手机构建 | 被取代：Owner 决定「全部一次做完」（§6） |

## 8. 实现记录（2026-11-17）

R1、L1、C、T2 (b)、T6 同一批实现。线协议 `PROTO` 仍为 1；本机 IPC 的 `PROTOCOL_VERSION` 与恢复存储未动
（`GatewayState.traffic_breakdown` 是带 `#[serde(default)]` 的新增字段）。§6.4 列的文档已同步。

| 项 | Mac（`src-tauri/src/ios/`） | 手机（`ios/Packages/AskHumanKit` 与 App） |
|---|---|---|
| R1 | `gateway/mod.rs`：`PhoneRt.sent_streams`（每台手机每个流的基线，`hello` 时清空）、`catch_up` / `sync_stream` / `send_baselined`；agents 走 `send_agents_delta` | 不变 |
| L1 | `gateway/agents.rs`：`lean_signature`、`differs_for_the_phone`；`PhoneRt.lean_agents = hello.leanAgents && caps.leanAgents`；`wire::Caps.lean_agents`（`daemon/runtime/ios.rs` 声明） | `Hello.leanAgents`；`AppStore.showsLiveActivity(of:)`；`AgentRow` 据此隐藏工具 / 步数 / 相对时间 |
| C | `envelope.rs`（`compress` / `inflate` / `seal_plain_with` / `open` / `parse_from_phone`）、`wire::is_compressible` 白名单与 `MIN_COMPRESS_BYTES`；`flate2` 用 `zlib-rs` 后端（依赖树里本来就有，没有新 crate） | `AHProtocol/Envelope.swift`（`Compression` 框架的 `COMPRESSION_ZLIB`，逐块解压、超过 512 KiB 即拒）；`Hello.compression` |
| T2 (b) | `gateway/watch.rs` 与 `mod.rs`：登记在 `on_hello` 清空，不再随非前台 presence；`tick_watch` 只服务活着的手机 | 不变 |
| T6 | `store.rs`：`TrafficBreakdown` / `KindCount`，`gateway/traffic.rs`：`count_message`（入站在收到时、出站在入队时计；未知类型归 `other`，不会长出新的键） | 不变 |

**实现中定下的细节（草案没写）**

- R1 的比较连同 `rev` 一起比，宁可多发一次也不漏：`rev` 前进说明 hello 与前台 presence 之间有变化。
- L1：被跳过的 Agent 在下一次比较时仍对着手机**实际持有**的那一版（`send_agents_delta` 为跳过的视图保留 `held`），所以活动字段不会被「悄悄吃掉」，
  只在稳定字段变化、开始 Watch 或状态切换时随增量带出。
- 手机一侧「正在被 Watch」用「详情页开着」代替：详情页打开即登记 Watch、离开即取消（`agentPath` 的 setter 调 `setWatch`），两者同步。
- 压缩要同时满足：手机最近一次 `hello` 声明了 `compression`、类型在白名单里、明文 ≥ 512 B、压缩后确实更小。任何一项不满足都照旧明文。
- 手机→Mac 的信封带 `z`、推送信封带 `z`、`z` 不是数字 1：一律拒收（Rust 的 `parse_from_phone` / `open`，Swift 的 `parse` / `open`）。
- 夹具：`messages.json` 增加带 `leanAgents` / `compression` 的 `hello` 与 `caps.leanAgents` 的 `hello.ack`；`crypto.json` 增加 `compressedEnvelopes`
  （Rust 压缩并加密，Swift 解密并解压，得到同样的明文，含中文标题）。原先断言「`z` 出现即拒收」的 Swift 用例改为「只认数字 1」。

**验证**

- Rust：全量测试、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check` 通过；新增 `gateway/tests_reduction.rs`（R1 / L1 / C / T2 (b) / T6 与字节预算），
  按 §3.1 改写了两条既有守护用例。Swift 包全量测试通过（含解压炸弹、截断流、`z` 取值、推送拒收 `z`、引擎收压缩消息、`showsLiveActivity`）。
- 真实 daemon + 真实手机引擎 + 本地 broker 的测试台（同 §1 的 17 个合成会话，只重跑了 Mac→手机方向的字节，`envelope` 口径）：

  | 状态 | 之前 | 现在 |
  |---|---:|---:|
  | 进前台一次，Mac→手机 | 22,638 B | **3,356 B**（−85%） |
  | 前台 90 秒、没有变化 | 558 B/min | 557 B/min（只有 `traffic.usage`，未动） |
  | 没被 Watch 的工作中 Agent，每分钟 12 次活动变化 | 9,985 B/min | **0** |
  | 同上，每分钟 30 次 | 23,588 B/min | **0** |
  | Watch 一个每分钟 12 次变化的 Agent | 18,915 B/min | 16,323 B/min（−14%） |
  | 后台 | 0 | 0 |

  Watch 那一行只少了其它 Agent 的增量：被 Watch 的那个 Agent 的 12 条列表增量（6.9 KB）按 §2 保留，它们是看门狗的信号源。
  测试台的合成会话重复度比真实会话高，agents 快照压到 0.9 KB（−91%）偏乐观；用真实会话元数据重建的报文是 −74%，
  所以真实的进前台一次预计在 5 KB 附近，仍是 §5 估的量级。进前台里现在最大的一块是两条 `traffic.usage`（每条约 0.83 KB，一来一回），
  占约一半，是下一个可压的目标（不在本稿范围）。手机台账里进前台仍有约 12 KB，是测试台为模拟清理前的账号注入的 300 条幽灵 retained presence；
  真实账号清理后只有 19 条。

**没验证的**：当时是真机、真实 EMQX 与 TestFlight 构建上的行为。2026-11-21 已用两台 Mac 的 `trafficBreakdown` 与 EMQX 控制台核对，
§1.1 的 0.65–0.85 GB/月估算由 §9 的实测取代；手机一侧新构建上的前台 / 后台行为仍待 Owner 核验（见 [PROGRESS.md](../PROGRESS.md)）。

## 9. 实测（2026-11-21）

落地三天后的核对：读 Mac mini 与 Mac Studio 的 `AskHuman channel ios status --json`（`trafficBreakdown` 与台账），对照 EMQX 控制台的
报文流量（按小时与按日）、消息数、计费流量与账单。MacBook Pro 没读（台账里它几乎不活动）。控制台只读，没改任何设置。
计数窗口（北京时间）：Mac mini 11-18 01:19 → 11-21 17:16（3.665 天）；Mac Studio 约 11-18 11:05 → 11-21 17:07（3.252 天）。
起点是该 Mac 换上带 T6 的构建、计数从 0 开始的时刻（Mac Studio 的起点是估的）。

### 9.1 与 EMQX 对账

- **台账对得上 EMQX**。UTC 11-18 00:00 → 11-21 09:00（北京时间 11-18 08:00 → 11-21 17:00）：EMQX 报文流量 607.7 MiB，同期台账两端合计
  608.5 MiB，差 0.1%。窗口边界、MacBook Pro 以及 Mac Studio 窗口之前那几个小时是按 11-17 夜里的台账快照估的，合计约 ±1%。
  [ios-traffic.md](ios-traffic.md) 记过 11-12 的旧算法比 EMQX 高 24%，改成设备端点计量之后已经对上。
- **计费流量比报文流量少**。11 月 1–20 日（UTC）报文流量 1.94 GiB，计费流量 1.63 GB，少 16%。EMQX 把每个整点小时的报文量先四舍五入到
  0.01 GiB 再累加成计费流量（日值就是 24 个小时值之和，11-18 至 11-20 逐日核对），不足约 5 MiB 的小时记 0：最近 71 个小时桶逐个核对，无一例外。
  落地前的流量是全天散布的小报文，被舍入吃掉的多；落地后流量集中在少数爆发的小时，舍入几乎不起作用（这 71 小时只吃掉 1.5%）。
  1 GB 即 1 GiB；免费 1 GiB，超出部分 ¥1.5/GiB。官方文档没有写这条舍入规则，它是对照小时序列得出的。
- **评审时的基线成立**。EMQX 报文流量 11-02 至 11-16 日均 92.0 MiB（31–180 MiB），对得上 §1.1 的 87 MB/天；到 11-17 累计 1.39 GiB，
  对得上「已约 1.5 GB」。

### 9.2 Mac 发出的字节拆成什么

`trafficBreakdown` 按「方向:类型」累计 PUBLISH 负载字节（出站在入队时数，已压缩、已加密）。下表是 Mac→手机方向，字节为十进制 MB：

| 类别（Mac→手机） | Mac mini | 占非附件 | Mac Studio | 占非附件 |
|---|---:|---:|---:|---:|
| 附件分块 `asset.chunk` | 291.9（878 块） | 占总量 97.4% | 3.0（10 块） | 占总量 26.9% |
| **非附件合计** | **7.94**（2.17/天） | 100% | **8.07**（2.48/天） | 100% |
| 进前台一轮（`hello.ack`、`inbox.snapshot`、`agents.snapshot`、`follow.state`、`slot.state`、`away.state`、`push.state`） | 1.29 | 16.3% | 1.68 | 20.8% |
| `agents.delta` | 0.76（989 条） | 9.5% | 2.94（2,923 条） | 36.5% |
| Watch（`watch.frame`、`watch.closed`） | 1.62（731 条） | 20.4% | 0.10（79 条） | 1.2% |
| 请求与回答（`request.*`、`answer.ack`） | 1.78 | 22.4% | 1.00 | 12.4% |
| 待办与任务（`todo.*`、`task.*`） | 1.78 | 22.4% | 1.44 | 17.8% |
| `traffic.usage`（计入 `other`） | 0.71（415 条） | 8.9% | 0.77（449 条） | 9.5% |
| 历史 | 0.01 | 0.1% | 0.14 | 1.8% |

手机→Mac 方向每台只有 0.46 / 0.53 MB/天，其中 `traffic.usage` 占 65% / 63%。

- **进前台一轮**每次 5.3 KB（Mac mini）/ 7.4 KB（Mac Studio），两台各约 67 / 70 次/天（每台数自己收到的 `hello`）。评审时估的是 ≈ 5 KB 与
  ≈ 100 次/天。Mac Studio 的 `agents.snapshot` 平均 4.8 KB（Mac mini 3.2 KB），所以它更大。
- **`traffic.usage`** 没登记进 `wire::registry!`，T6 因此把它归在 `other`（每条固定 1,706 B）。平均每次 `hello` 对应来回约 4.4 条（Mac mini）/
  4.8 条（Mac Studio），每次 7.4 / 8.2 KB，比进前台那一轮本身还大；两个方向合计占各 Mac 非附件流量的 19%。§8 说「最大的一块是两条
  `traffic.usage`」，真实账号上来回有 4–5 条。它不在本稿范围，是下一个可压的目标。
- **`agents.delta`** 每条平均 764 B / 1,006 B，和测试台的单条增量（0.8 KB）同量级。§1 推断「Mac 月均每条 6.4 KB」主要是同一个节拍里多个
  Agent 合并的增量，也可能是大的 Watch 帧或请求正文；前一个推断不成立，拉高均值的是附件分块（每条 332 KB）、请求正文（5.6–24 KB）与
  待办列表（2.4–12.7 KB）。落地前没有按类型的计数，无法逐类证明落地前也是这样。Mac Studio 上 `agents.delta` 仍是最大的非附件项
  （约 900 条/天），原因没查：§2 保留了被 Watch 的 Agent 的增量，状态与标题等稳定字段的变化也会触发。
- **Watch 帧**每帧 2,293 B（Mac mini，701 帧）/ 1,309 B（Mac Studio，74 帧），在 Mac mini 上占非附件流量的 20%，与请求、待办、进前台同量级。
  草案当时没有数据（§2 把 Watch 帧本身列为不做）。
- **后台没有业务流量**：夜里手机与 Mac 都空闲的小时，EMQX 报文流量只有约 1.4 KiB/小时（心跳）。

### 9.3 附件

- 附件分块占 Mac mini 出站字节的 97.4%，占两台 Mac 与手机的台账两端合计的 92.4%（589.8 / 638.1 MB）。
- 每块平均 332,512 B，折回约 187 KB 原始字节：每个文件的末块不满，所以比满块小。满块（192 KiB）在线上是 350 KB，即 ×1.78：字节先 base64
  放进 JSON，加密后再 base64 放进信封（`envelope.rs`）。一次 `asset.get`（最多取 8 块）平均 6.2 块，Mac 出站 2.06 MB，EMQX 上两端合计约
  4.1 MB（3.9 MiB）。一张约 2 MB 的截图要两次 `asset.get`，打开一次约 7 MB。
- 时间分布：最近 71 个小时里，12 个超过 10 MiB 的小时占 91%（548.5 MiB），最大的一个小时 129 MiB。非附件流量一天才约 12.5 MiB，撑不起这样的
  单小时，这 12 个小时的总量与附件的总量（约 563 MiB）吻合。
- 手机只在大图查看器里下载：页面在屏幕上或相邻（`AssetPage.near`）才取，列表只用缩略图；下载过的文件按 `assetId` 缓存在手机上，再次打开不再取。
  所以这部分与「看了多少张大图」成正比，不是后台预取。窗口里约 167 MiB/天，三天的数据波动很大，只当量级看。
- EMQX 日均报文流量：落地前 92.0 MiB（11-02 至 11-16），落地后 188.2 MiB（11-18 至 11-20）；平均每条消息（收到的发布加投递出去的）
  从 1.7 KiB 变成 13.5 KiB。落地前每条消息很小，附件应该很少，所以 §9.4 可以拿落地前的总量当同步部分的上界。
- 收敛办法见 [附件流量收敛](ios-attachment-traffic.md)：手机先看预览、点「原图」才取原文件，去掉两遍 base64，按内容去重。

### 9.4 与 §1.1 的对照

| | 2026-11-17 的推算 | 2026-11-21 实测 |
|---|---|---|
| 落地前日均 | 87 MB/天（台账折算） | 92.0 MiB/天（EMQX，11-02 至 11-16），约 96 MB |
| 进前台一次 Mac→手机 | ≈ 5 KB | 5.3 KB / 7.4 KB |
| 进前台次数 | ≈ 100 次/天 | 66.6 / 69.5 次/天（每台 Mac） |
| 除附件外，整月两端合计 | ≈ 0.73 GB（0.65–0.85） | ≈ 0.4 GB（约 13 MB/天）；MacBook Pro 若和 Mac Studio 一样活跃约 0.5 GB |
| EMQX 投递的消息数 | — | 日均 46,800 → 8,700（−81%）；收到的发布 8,850 → 5,600（−37%）。手机与 Mac 的更新同期到位，分不开各自的贡献 |
| 11 月总计费流量 | 整月 ≈ 0.73 GB | 到 11-21 08:00（UTC）已 1.67 GB；落地后总字节的 92% 是附件 |

同步部分（不含附件）从约 92 MiB/天降到约 12.5 MiB/天，降幅最多 86%；落地前的 92 MiB 还含当时的附件，同步部分本身略小，所以实际降幅略低于这个数。
**结论**：评审时的目标（同步部分整月回到 1 GB 以内）已经达到并有余量，但整月总量取决于附件：一次 `asset.get` 约 4.1 MB，
整月 ≈ 0.4 GB + 附件次数 × 4.1 MB。

### 9.5 限制与没查清的

- 窗口只有 3.3–3.7 天，用量按天波动很大（EMQX 日值 31–311 MiB），月值是外推，不是整月的实测。
- MacBook Pro 没读，约 1.8 MB/天（两端合计）是按台账增量估的；Mac Studio 窗口之前的几个小时也是估的。
- 压缩（C）没有单独测：`agents.snapshot` 平均 3.2 / 4.8 KB 已是压缩后的字节，没有对照组，§8 的 −74% 仍是用真实元数据重建的数字。
- 手机一侧没有单独的计数（T6 只记在 Mac），新构建上的前台 / 后台行为仍待 Owner 核验（见 [PROGRESS.md](../PROGRESS.md)）。
- 按日值累计（UTC），到 11-14 为 0.96、到 11-15 为 1.06，免费 1 GiB 应在 11-15 用完，而首张流量账单是 UTC 11-16 那天的 ¥0.01
  （北京时间 11-17 08:00 出账），差约 0.06 GiB。原因没查，不影响本节结论。
