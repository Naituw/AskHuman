# Cursor 断线排查与处理

2026 年 11 月 7 日 19:40 前后有一次约 86 秒的联通宽带中断。同一天的单条 Cursor 断流还出现了三种不同的顺序。部分样本在代理路径先关闭，部分从 Cursor 方向收到关闭包，另有样本在下行已积压时由客户端主动取消。时间均为北京时间。

## 样本与分工

| 调查 | 进度 | 证据范围 |
| --- | --- | --- |
| 宽带 | 已核对 19:40 接入中断，联合轮无同类直连成簇失败 | Cursor 日志、Mac 与 HouseMini 抓包、DMIT 网卡抓包、联通光猫状态 |
| Snell | 90 分钟续采结束。四条已建连接为 Cursor 方向先关闭，另有入口握手异常 | Snell 入口与出口、Surge 连接事件、connector 和 Snell syscall |
| Stall Detector | 已三端复现，Snell 下行积压先于客户端取消 | Cursor 计时与重试日志、Mac 与 HouseMini 的窗口及 ACK、DMIT TCP 队列 |

截至 17:04 的 Cursor 日志记录 11 个回合里的 85 条传输失败，其中 55 条为已建立 WebSocket 的异常，30 条为 HTTP 或 SSE 失败，5 个回合重试用尽。这个数字按传输失败计，一次掉线可以导致多条记录。另有两次 `resource_exhausted`，单独归类。

此前的调查在 Cursor 聊天「Cursor billing analysis」。2026 年 10 月 30 日在相同 Surge 策略组运行的四条合成长连接均持续完整三小时。该对照连接到自有 VPS，只覆盖到代理出口的路径与指定流量模式。等待 AskHuman 时的自动恢复行为见[此前的等待期调查](cursor-idle-wait-keepalive.md)。

## 宽带

HouseMini 的默认网关是联通中兴 ZXHN G7615V2。19:39:37.314 到 19:41:03，HouseMini 未收到 DMIT 的 Snell 数据或 TCP 握手回包。同期 Surge 记录了 560 条 `DIRECT` 失败，其中 220 条是直接访问公网 IP 的失败，涉及 52 个不同地址。

| 时间 | 证据 |
| --- | --- |
| 19:40:00–19:40:59 | HouseMini 向 DMIT 发出 5,777 个 SYN，覆盖 2,136 个 TCP 四元组，DMIT 网卡记录到 0 个对应 SYN。同期 DMIT 向 Cursor 的 20 个 TCP SYN 均收到 SYN-ACK，Snell 进程无重启。 |
| 约 19:40:50 | 光猫状态页在 21:15:53 显示 EPON 已注册已认证，连接 5,704 秒。按计时反推，EPON 在故障末尾重新注册。 |
| 19:41:03 | HouseMini 再次收到 DMIT 回包。19:41:05–19:41:27 的对照窗中，199 个 HouseMini SYN 均可在 DMIT 网卡对应。 |
| 约 19:41:05 | 光猫在 21:18:32 显示上网 PPPoE 已连接 5,848 秒，上次失败原因是「连接 ISP 服务器超时」。管理用途的 TR-069 PPPoE 会话也在同一分钟重连。 |

多目标同时失败，且 EPON 与 PPPoE 重连和抓包时间吻合，足以将这一批中断定位到宽带接入。事后光功率正常。普通账号的管理与诊断页面未提供历史告警或整机运行时长，光路中断、光猫重启和联通局端异常仍需用局端记录区分。

向联通报障时可直接提供以下内容。

> 2026 年 11 月 7 日北京时间 19:39:37–19:41:03，宽带中断约 86 秒。中兴 ZXHN G7615V2 光猫状态页推算 EPON 约 19:40:50 重新注册，上网 PPPoE 约 19:41:05 重连，上次拨号失败显示「连接 ISP 服务器超时」。请查询 19:39–19:42 局端 OLT 的 ONU 离线原因、光信号丢失或掉电告警，以及 PPPoE 会话断开原因，把结果写入工单。

19:40 事件的本机证据保存在临时目录 `/tmp/cursor-snell-trigger.AajCYp/`。`house.jsonl`、`dmit-outage-window.jsonl` 和 `mac.jsonl` 分别是 HouseMini、DMIT 和 Mac 的采集。上表保留了日后复核所需的时间和计数。

## Snell

三端证据按连接分别判断。Mac 和 DMIT 出口使用同一 TLS ClientHello 随机数的哈希配对。HouseMini 和 DMIT 的 Snell 外层使用 TCP 报文头摘要配对，Snell 入口与出口的关联仍需靠双向字节走势辅助判断。

- 19:00:12–17，DMIT 向三个 `api2` 目标 IP 发出六条已建立至少两分钟的连接的首个 FIN。其中两条之后由 Mac 收到首个 FIN，另两条约 15 秒、34 秒后才由 Mac 发出首个 FIN。剩余两条的 Mac FIN 先于或接近 DMIT 的 FIN。19:00:10–19 的抓包里，Mac 侧 24 个新 SYN 全部收到 SYN-ACK。关闭集中于已有连接，新连接仍能建立。
- 其中报错的 SSE（Mac 端口 62667，请求 `0518ec98-c0c9-4940-8696-bb88c29b4bbf`）在 DMIT 出口的最早关闭包为 19:00:12.960 发向 `api2` 的 FIN。DMIT 在 19:00:14.852 收到对端 FIN，Mac 到 19:01:12.649 才收到 FIN，Cursor 随后记为 `socket_reset`。当时已抓到 Mac 和 DMIT 出口，Snell 入口采集约 19:14 才启动。
- 19:23:12 和 19:26:14，DMIT 在两条 WebSocket 对应的 Cursor 出口流分别先收到 RST 和 FIN，Mac 约 2–3 秒后收到 FIN。这两条流在第二轮采集前就已建立，回查第一轮的 TLS 握手后可精确配对。19:23 的 Snell 入口候选隧道与该出口的双向字节走势吻合，DMIT 出口先收到 RST，随后 DMIT 向 HouseMini 发出 RST。包的关闭方向无法单独区分 Cursor 应用、入口负载均衡和中间网络。
- 18:17 用户将 Cursor 切到 DMIT 单跳，18:35 仍出现一次独立 WebSocket 中断。该次中断无需经过原来的 QQ.PW 一跳即可发生。

19:00 事件最早能定位到 DMIT 主动关闭出口 TCP。Surge 的主日志和事件库在该窗口没有相关记录，DMIT 的 `snell.service` journal 没有条目，Snell 服务进程保持同一个 PID 且重启计数为零。网关运行 Surge 6.10.0（12350），DMIT 运行 Snell 5.0.1，代理策略启用了 `reuse=true`。厂商关于终止连接被放回复用池的修复早在 Surge 6.8.0 发布。当前证据仍无法区分 Surge 发出的隧道结束、Snell 转发端自身的关闭条件和入口链路异常。[Surge 版本说明](https://nssurge.com/support/mac/release-notes/) [Snell 复用参数](https://manual.nssurge.com/policies/snell.html)

本轮在当前单跳线路被动记录 21:35–22:22，三端采集已停止，抓包均报告内核丢包为零。期间 Cursor 写入 7 条失败诊断。复发时先定位对应 Cursor 请求并用 TLS 指纹确认出口连接，再核对 Snell 入口的首个 FIN/RST、重传、ACK 和收发队列。入口先停止推进就查 HouseMini 到 DMIT 这一段。如果出口先收到 FIN/RST 且入口随后关闭，则归为 Cursor 方向发起的 TCP 关闭。若像 22:11 一样 Mac 首先收到 Surge 的 FIN，下一次需在 Surge 的明细滚动前立即保留该请求的连接事件，用它定位代理侧发出逻辑结束的原因。

本轮 21:42:28 捕获一条 Stall Detector 样本（请求 `8e9c6c0e-c962-497b-b67f-0417c179e491`，21:06:49 已发起）。最后一条入站心跳约在 21:40:58.38，90 秒后客户端中止，21:42:30 发起重试。该空窗中 HouseMini 每秒都有从 DMIT 收到的 Snell 数据，合计约 38.6 MB。此请求早于本轮采集，握手无法精确配对。[Stall Detector 的补充采集](#2142-三端复现)通过 TCP 字节走势和报文头关联了该请求，确认对应单条隧道存在下行积压。事件窗口保存在本轮目录的 `event-1794058948/`。

21:47:31 的 SSE（请求 `76b0fd53-ab4c-4b89-a05a-77c0f39a1eba`）也早于本轮采集，但可用结束时的字节数强关联。Mac 端口 59071 的内核 `tcp_connection_summary` 显示 Cursor Helper 已连接约 47 分钟，收发计数分别为 10,631,482 和 2,313 字节，含 30,608 字节重复入站数据。去重后与 DMIT 的 10,600,875 和 2,314 字节双向各差 1 字节。该 DMIT 出口在 21:47:29.270 先收到 FIN，到 21:47:30.632 才发 FIN。时间顺序取同一 DMIT 网卡上的记录。Snell 入口的字节走势候选在同窗没有外层 FIN/RST。这个样本与 19:23、19:26 一样，最早关闭包来自 Cursor 方向。事件窗口在 `event-1794059251/`。

21:57:27 的 WebSocket（请求 `e9cd17f8-cf3a-4e9e-9c5d-d944e0880966`，Mac 端口 54501）有完整握手，可以精确配对 Mac 与 DMIT 出口。DMIT 的同一网卡先在 21:57:25.370161 收到 Cursor 方向的 RST，58 微秒后向 HouseMini 发出 Snell 外层 FIN。HouseMini 抓到同一 FIN 的报文头摘要。Snell 隧道是通过双向字节走势关联到出口的候选，关闭前出口 socket 的收发队列为零，最近仍有数据和 ACK。Mac 收到 FIN 后，Cursor 日志记为 `socket_reset`。这个样本的出口关闭先于 Snell 外层收尾。事件窗口在 `event-1794059847/`。

22:07:12 的 SSE（请求 `e795ecac-ece6-4b0b-b800-8b4122c8ee14`，Mac 端口 54283）同样有完整握手。DMIT 出口先在 22:07:11.862 收到 FIN，到 22:07:12.871 才发 FIN。Mac 收到 FIN 后报 `socket_reset`。事件窗口在 `event-1794060432/`。

22:08 的 WebSocket 和 22:10 的 SSE 附近，各有一条采集前已建立的连接，通过 Mac 内核去重后的字节数和 Surge 计数与 DMIT 出口强关联。22:08:36.157 的出口先收到 RST，22:10:19.243 的出口先收到 FIN。没有用这两条候选推断同时关闭的其他连接。原始窗口分别在 `event-1794060518/` 和 `event-1794060709/`。

22:11:49 的 WebSocket（请求 `ff13368b-33a4-4816-804a-86d2d7db6e2d`，Mac 端口 58645）复现了代理方向先关，Mac 与 DMIT 出口已按 TLS 握手精确配对。

| 时间 | 证据 |
| --- | --- |
| 22:11:49.596 | Mac 先收到 FIN，约 31 毫秒后 Cursor 记录 `socket_reset`。Mac 内核确认这是 Cursor Helper 的连接。 |
| 22:11:53.130 | DMIT 出口仍为 `ESTAB`。相较 22:11:51 的采样，出口又收到 73 字节，已确认的上行字节也增加，发送队列归零。 |
| 22:11:54.973346 | DMIT 收到 HouseMini 发来的 Snell 外层 RST。该外层隧道是按双向字节走势和收尾时刻关联的候选，HouseMini 有相同的报文头摘要。 |
| 22:11:54.973409 | 同一 DMIT 网卡在 63 微秒后首次向 Cursor 发 FIN。出口稍后才收到对端 FIN。 |

22:11 最早可见的关闭在 Surge 到 Mac 的一侧，出口收尾发生在 HouseMini 方向的 RST 之后。当时 HouseMini 仍和 DMIT 收发其他数据，Surge 主日志与 `snell.service` journal 都没有相应记录，Snell 进程未重启。此例把关闭缩小到代理转发路径，但 Snell 加密通道中的逻辑关闭无法从 TCP 标志判定。Surge 为何先结束 Mac 连接，以及是否由 DMIT 发来的逻辑结束触发，仍需同一请求的 Surge 连接事件或代理侧诊断。事件窗口在 `event-1794060709/`。

19:00 与 19:23/26 的本机证据分别在 `/tmp/cursor-close-observe.6zc2ei/` 和 `/tmp/cursor-snell-trigger.AajCYp/`，本轮的本机目录为 `/private/tmp/snell-closure.r9Y2JV/`。记录保存 TCP 头部摘要、握手哈希和 socket 状态。

22:30 后在 HouseMini 加了两类持续日志。定期保存 Surge 已完成的 Cursor 连接备注，在最近记录滚动前留下本机端口、Record ID 和双方关闭事件。同时从现有内存日志保留 `SGUnifiedConnection` 和 `SNConnector` 的关联、复用池状态和 EOF 或错误回调。22:34 已验证可从实际 Record ID 关联到 Snell connector，并读到 `Received server EOF`、`socketDidReadEOF` 和复用池回收事件。若下一次故障先出现 `Received server EOF` 或解码后的零长结束块，即可确认 Surge 在转发 DMIT 发来的逻辑结束。若先出现 Surge 的本地错误或主动断开回调，则沿该错误查关闭触发。此轮采集正在等待自然故障。作用域仅为本机 Cursor 流和相关 connector 诊断，日志在 HouseMini 的 `/tmp/snell-request-events.C3ZeUQ/`。

22:39 复核两路记录均在写入，Cursor 侧尚无新失败诊断。下一次检查约 22:45。后台采集最晚于 11 月 8 日 04:30 自动停止，若在此之前捕获到同一故障的客户端报错、Surge Record ID、Snell connector 和首个逻辑关闭或错误回调，则先核对并保存该窗口。

22:40、22:43 复发时，两组 Surge 内部回调先记录 `Received server EOF without data`，再触发 `connectorDidReadEOF`，最后才读到 Mac 的 EOF。22:43 的 `SNConnectorV4-227853` 在 HouseMini 时钟 22:43:31.110508 记录服务端 EOF，63 微秒后 `SGUnifiedConnection-8209964` 开始处理；校正约 0.697 秒的主机时钟偏差后，与 Mac 端口 62656 的关闭相差约 11 毫秒。端口对应依据是结束时间和连接活动记录，连接建立早于内部日志采集。已确认 Surge 能看见 DMIT 的逻辑 EOF，仍缺同一事件的 DMIT 出口记录来区分代理自产和远端转发。新加的关闭字节数记录用于核对这种预先建立的连接。事件片段在本机 `/private/tmp/snell-request-events.C3ZeUQ/first-failures.jsonl`。

### 联合采集中的 Surge 与 DMIT

- 沿用一个共同 run ID。Surge 保存本机源端口、Record ID、SGUnifiedConnection、Snell connector 及其复用、逻辑 EOF、错误和前端关闭事件。长连接可能早于日志开始，也可能在结束时已超出 Surge 最近列表的 200 条范围。此时用打开时间、内核去重后的双向字节数和关闭事件关联，并标明关联方法。
- DMIT 用与 HouseMini 相同的 TCP 头部采集器记录 Snell 外层 ACK、序号、接收窗口、SYN 窗口缩放和报文摘要，同时保存 Cursor 出口握手指纹、FIN/RST 和 socket 计数。socket 快照补进程 FD、cookie、重传、RTT 与收发队列，主机网卡计数用于核对是否有共同的吞吐变化。采集文件在各主机有界落地，事件前至少 180 秒、后 60 秒按统一窗口拉回。
- 先按相同 TLS 握手配对 Mac 与 DMIT 出口，再用 Surge 日志确认哪个逻辑流收到或发出 EOF。若匹配的出口先收到对端 FIN/RST，随后 Surge 才收到 Snell EOF，按出口关闭传回处理。若先有已解码的 Snell 服务端 EOF，而同一 DMIT 出口之后仍收到新数据且没有更早的对端 FIN/RST，优先追 DMIT 的逻辑收尾。若 Surge 本地先报错，则查对应的回调与读写状态。落在时钟误差内或关联不足的样本保持未决。

联合 run `cursor-20261107T150300Z` 的各端在 23:03 就绪，标记为用户报告配额重置后。HouseMini 和 DMIT 的采集最晚 23:52:30 自动停止。本机 `/private/tmp/cursor-20261107T150300Z/snell/clock.jsonl` 每 30 秒记录时钟偏差上下界。DMIT 的独立 tracefs 记录自 23:06:30 起有效，只保留 Snell 线程的 `shutdown` 和 `close` 及 FD；旧解析器的首次短窗未保存 syscall。它作为关闭顺序的补证，和 `ss` 的 socket FD 对齐。

23:05:30 的正常对照切片已拉回，共配对 Mac 172/172 个 TLS 握手及 HouseMini 和 DMIT 的 2,112 个 Snell 报文摘要。故障触发器只读取本轮内的 Cursor 失败事件，自动保留前 180 秒和后 60 秒，并将同窗的两端头部、Surge 逐连接与 connector 日志、DMIT socket、网卡和 syscall 记录写入本机 `/private/tmp/cursor-20261107T150300Z/events/`。

23:26–23:28 的长流对照仍有实际 Cursor 数据。Surge 端口 60612 的累计收发为 27,116,234 和 6,808,362 字节，DMIT 出口快照双向相差 0 和 1 字节。约两分钟内该出口新增约 860 KB 下行，Snell 外层候选的双向字节走势相关系数约 0.999。候选发送队列最高 37,742 字节，RTO 的中位数和最高值均为 360 毫秒。对比配额重置前 21:42 的停滞样本，发送队列曾达 391 KB，计时窗内的 RTO 中位数约 3.8 秒。这是不同时间和流量负载下的观测对照，仍需按具体故障区分限速、逻辑 EOF 和 Cursor 方向关闭。对照切片在 `events/1794065290/`。

联合轮于 23:52:30 收尾。自 23:03 三端就绪后，Cursor 事件记录有一次成功、零条新失败，23:00 的失败是启动时回读的旧事件。Mac 与 DMIT 共有 6,576 个 TLS 指纹，Mac 共记录 6,582 个。Mac 包头在 23:46:11.680 因磁盘空间不足暂停，于 23:48:09.725 恢复，最后记录到 23:52:29。HouseMini 和 DMIT 的包头均记录到 23:52:29，HouseMini 报告 9,857 个内核采集丢包，DMIT 为零。tracefs 无丢失标记且已移除，DMIT 临时 SSH 会话已关闭。原始包头在各主机的私有目录，摘要在 `snell/coverage.json`，两段正常对照已留在本机。HouseMini 上的低负担 Surge 逻辑事件记录继续到 11 月 8 日 04:30。未捕获到配额重置后的同类关闭，因此此次联合轮仍无法把早先 Snell EOF 归因到 DMIT 的具体触发条件。

### Cursor 方向先关闭的规律

五个关联证据较强的样本都在流建立后十几分钟到约 47 分钟出现，关闭前几秒仍能看到 DMIT 出口接收数据或确认上行。在已观察的样本里，关闭时间没有集中在固定连接寿命，也看不到统一的长空闲期。三条 WebSocket 既有 RST 也有 FIN，两条 SSE 都先收到 FIN。Cursor 最后统一记作 `socket_reset`。这组样本可以定位 TCP 关闭方向，触发方究竟是 Cursor 应用、边缘设备还是中间网络仍需对端日志。

| Cursor 报错时间 | 传输 | DMIT 出口先收到 | 连接已持续约 |
| --- | --- | --- | --- |
| 19:23:12 | WebSocket | RST | 21 分 19 秒 |
| 19:26:14 | WebSocket | FIN | 25 分 54 秒 |
| 21:47:31 | SSE | FIN | 47 分钟 |
| 21:57:27 | WebSocket | RST | 14 分 18 秒 |
| 22:07:12 | SSE | FIN | 24 分 41 秒 |

连接时长按 TLS 握手到首次关闭估算，21:47 采用 Mac 内核连接摘要。19:26 的出口在收到 FIN 前约 1.5 秒内还有下行，21:57 在 RST 前约 1.4 秒的采样里收发队列均为零，最近一个 ACK 约 40 毫秒前到达。这两条尤其适合交给 Cursor 支持按请求 ID 对照。

11 月 8 日的续采 run `cursor-20261107T160300Z` 于 00:03:01 就绪，计划持续到 01:33。除同样的三端包头与代理逻辑日志外，HouseMini 每 15 秒保存 Cursor 路由的 Record ID、真实起点和双向字节增量。区分单条 TCP 连接的实际寿命与同一 Agent 回合相邻失败的间隔。采集前已建立的连接按进入观察时的年龄计入，结束时仍存活的连接保留为右删失。00:05 的首次核对已见 12 条连接超过 60 分钟后仍有新增字节，其中端口 55207 的连接当时约 83 分 40 秒，初始约 106 秒新增 1.26 MB。同一 DMIT 出口 socket 在续采中持续接收数据。

| 续采中报错 | 实际连接寿命 | 同一回合距上次失败 | 首个可见关闭 |
| --- | --- | --- | --- |
| 00:09:51.665 WebSocket，`a626b8bf`，Mac 端口 60022 | 约 12 分 47.3 秒 | 约 12 分 47.6 秒 | DMIT 00:09:51.355891 先从 Cursor 方向收到 FIN，00:09:51.516593 发 FIN，Snell 进程同刻关闭对应 FD 131。 |
| 00:13:21.564 SSE，`82970221`，Mac 端口 55207 | 约 91 分 39.8 秒 | 约 92 分 51.4 秒 | DMIT 00:13:19.983713 先从 Cursor 方向收到 FIN，00:13:20.139927 发 FIN，Snell 进程同刻关闭对应 FD 51。 |
| 00:46:19.230 WebSocket，`04fcb5df`，Mac 端口 63978 | 约 36 分 26.5 秒 | 约 36 分 27.6 秒 | DMIT 00:46:18.878721 先从 Cursor 方向收到 RST，171 微秒后在匹配的 Snell 外层发 FIN。 |
| 01:32:05.760 WebSocket，`e3f0660c`，Mac 端口 62182 | 约 40 分 30.9 秒 | 约 34 分 51.6 秒 | DMIT 01:32:04.369751 先从 Cursor 方向收到 RST，209 微秒后在匹配的 Snell 外层发 FIN。 |

两条均早于续采开始，由 Surge 源端口与 Record ID 定位，DMIT 出口由双向字节计数和时序配对。00:09 的 Surge Record 8571579 映射到 `SNConnectorV4-229098`。出口收发与 Surge 关闭统计双向只差约 24 字节，另有相同的出站分桶计数。00:13 的 Record 8542405 映射到 `SNConnectorV4-228384`，关闭时 Surge 的 29,664,660 / 2,313 字节与 DMIT 的 29,664,661 / 2,314 字节对应。该出口 socket 从前一轮一直留到关闭。Surge 两条都记录了服务端零长结束块，之后才关闭 Mac 流。用同窗 Snell 两端的共同 TCP 控制包约束时钟偏差，出口 FIN 在两条逻辑 EOF 之前，保守区间分别约 6–153 毫秒和 1–151 毫秒。两条为出口关闭传回的样本，连接寿命覆盖十几分钟和一小时半。证据在续采的 `events/1794067791/` 与 `events/1794068001/`。

00:46 的第三条在续采内建立，Mac 和 DMIT 用 TLS 握手哈希精确配对。DMIT 出口在关前仍收到数据，发送队列为零。DMIT 网卡先收 RST，约 100 微秒后 Snell 关闭出口 FD 20，再关闭外层 FD 258，随后发出的外层 FIN 在 HouseMini 上有同一个 TCP 指纹。外层通过约 0.9998 的双向字节走势关联。Surge 对应的 Record 8577234 映射到 `SNConnectorV4-229148`，首先记录外层 socket 的 EOF，随后关闭 Mac 流。此例捕获了从出口 RST 到代理传回的完整关闭顺序。证据在 `events/1794069979/`。

00:57:14–16 另有两个 Agent 回合报告 `ConnectError / Unavailable`，底层原因为 `ETIMEDOUT`。以 SYN 序号与 SYN-ACK 确认号配对，00:57:07–21 的 Mac 侧 27 个独立新握手中有 6 个未收到回复，同窗 DMIT 出口的 27 个握手都成功，HouseMini 的既有 `DIRECT` 日志在事件前后 30 秒无建连失败。该组按 Mac 到 Surge 入口的建连异常单独统计，现有记录尚不能区分网关虚拟网络、Surge 接受连接的路径与本机。证据在 `events/1794070634/` 和 `events/1794070636/`。

01:32 的续采尾声样本也有完整 TLS 握手配对。DMIT 收到出口 RST 后约 159 微秒关闭出口 FD 275，约 201 微秒关闭 Snell 外层 FD 265，外层的双向字节走势相关约 0.9978。HouseMini 也记录了随后相同的外层 RST 报文，Surge Record 8605746 对应 `SNConnectorV4-229408` 并记为 `Closed by remote`。Mac 先发 FIN 的单端记录在这里容易误判方向，以 DMIT 同一网卡的事件顺序为准。故障后因计划截止保存约 54 秒，事件切片在 `events/1794072725/`。

续采从 00:03 到 01:33 约 90 分钟。三端包头均结束于计划时刻，均报告零内核采集丢包，Mac 的 14,767 个 TLS 指纹有 14,758 个可在 DMIT 对应。Cursor 记录去重后有六条失败，四条已建立连接均为 Cursor 方向先关，另两条属于 00:57 的同一组入口握手超时。356 次 Surge 活跃快照无漏轮，观察到 15 条连接在年龄超过 60 分钟后仍有新字节，其中 13 条超过 90 分钟仍有新字节。超过阈值的这些连接均早于续采开始，这组年龄统计包含 Cursor 后台连接。四条能定位的异常里有一次确实在约一小时半出现，另三次在约 13、36、41 分钟出现；同一回合的相邻失败间隔也分散。结果支持连接会在不同年龄异常关闭，未观察到固定的一小时级定时关闭。续采的汇总保存在 `snell/coverage.json` 和 `snell/age-coverage.json`，DMIT tracefs 与临时 SSH 会话已收尾。

## Stall Detector

本机 Cursor 3.27.22 实际在 **30 秒没有入站 Agent 消息时告警，90 秒时取消当前尝试**。服务端心跳和已解析的业务消息重置计时。客户端发送心跳只记录时间。11 月 7 日截至约 21:00 的 Agent Host 日志有 66 条 30 秒告警和两次 90 秒取消。两次均归为 `stall` 类传输失败，随后约 70 秒发起下一次重试。

| 时间 | 请求与日志 | 结果 |
| --- | --- | --- |
| 12:35:32.712 | `92b7e087-f79a-405e-a0c2-b6f120af3fb5`。最后一条入站消息是约 90 秒前的服务端心跳。该时段缺少同步传输采样。 | 12:36:42 开始重试。同一回合之后还有数次独立的连接重置，13:57:22 达到传输重试上限。 |
| 19:32:33.181 | `af80ce64-f531-49bb-920d-bca71b1889a2`。WebSocket 池未就绪，该次尝试走 HTTP，最后一条已读出的入站消息是约 19:31:03 的 checkpoint。 | 19:33:43 开始重试。同一回合此后还遭遇宽带中断与独立的连接重置，20:59:03 达到传输重试上限。 |

19:32 的请求与 Surge 的本机端口 `50575` 在打开和关闭时刻吻合。Surge 与 DMIT 出口的字节计数在停滞前反复精确对应。DMIT 上高度吻合的 Snell 下行流在约 19:31:01 开始出现发送队列，约 19:32:22 增至 **562 KB**。这期间 DMIT 的 Cursor 出口继续收到约 **1 MB**，HouseMini 包头与 Surge 计数各看到约 **0.5 MB** 下行。Snell 流用逐时刻字节增量关联，两端的五个抽样 TCP 包头指纹全部匹配。到 19:32:33.181 客户端日志记录 `stall_detector`，Mac 在 19:32:33.182 发 FIN，DMIT 出口随后发 FIN。队列积压先于客户端主动取消。证据来自 Cursor 的 Agent Host 和网络调试日志，以及 `/tmp/cursor-snell-trigger.AajCYp/` 内的 `mac.jsonl`、`dmit.jsonl`、`sockets.jsonl`、`ingress.jsonl`、`house.jsonl` 和 `surgeactive.jsonl`。此处发送队列按 Linux 的已写入字节与已确认字节之差统计。[Linux TCP 队列定义](https://github.com/torvalds/linux/blob/master/net/ipv4/tcp_diag.c)

19:32 的最早可见异常是 Snell 下行积压。该次记录缺少对端接收窗口与 Mac 的数据 ACK。12:35 仅能确定客户端主动取消，起因仍未知。

### 21:42 三端复现

请求 `8e9c6c0e-c962-497b-b67f-0417c179e491` 在 21:06 开始。21:40:58 左右 Cursor 读到最后一条服务端心跳，21:41:28 预警，21:42:28.385 主动取消，21:42:30.563 开始自动重试。Mac 端口 `59072` 的 5 秒心跳节拍及最后一个 73 字节的下行包与应用日志吻合。多段双向 TCP 字节增量把它与 DMIT 的 Cursor 出口和 Snell 连接配上。Snell 连接有 21 个抽样包头指纹只命中同一条 HouseMini 流。连接早于抓包开始，Mac 到 DMIT 的配对依据是时间与字节走势。

| 时间 | 三端观测 |
| --- | --- |
| 21:37:48 | 匹配的 Snell 下行发送队列已达 132 KB，早于最后一条入站心跳。21:39:28–21:40:58 的窗口内队列最高约 391 KB，DMIT 统计重传约 381 KB。 |
| 21:40:58–21:42:28 | DMIT 的 Cursor 出口又收到约 559 KB。HouseMini 的 Snell ACK 推进约 373 KB，Mac 在 21:42:28 前确认收到约 297 KB。两端记录到的接收窗口始终大于零，窗口原始值也未持续收缩。 |
| 21:42:28.146 | Snell 在 DMIT 侧的发送队列约 265 KB。计时窗口内的 RTO 中位数约 3.8 秒、最高约 5.3 秒，另有约 16 KB 重传。 |
| 21:42:28.385 后 | Stall Detector 取消该次尝试。下行随后加快，Mac 的 ACK 在发 FIN 前又推进约 637 KB，耗时约 10 秒。DMIT 出口随后发 FIN。 |

这一样本同时观察到了 Snell 下行交付受限和 Mac 已收到部分 TCP 字节。窗口与 ACK 记录将重点放在 DMIT 到 HouseMini 这段的延迟、重传与发送进度。TCP 字节是否组成完整的 Agent 消息仍未知。现有证据无法继续区分 Snell 实现、TCP 发送侧和底层网络条件各自的影响，也无法证明延长超时一定能让该请求恢复。新样本存于 `/tmp/cursor-backlog.X7HQWN/`，其中 `mac.jsonl` 和 `house.jsonl` 含窗口与 ACK，`dmit-event/` 为 Snell 分支同步记录的出口、入口和队列切片。

复核多个 Snell 连接在 DMIT 与 HouseMini 两端都看到的 TCP 包头指纹，以 15 秒分桶，将两个方向的跨端时间中位数相加抵消固定时钟偏差。21:39:15–21:40:30 的各桶约为 0.7–1.3 秒，21:40:45–21:42:15 约为 1.2–2.4 秒。这是两处网卡之间的传输时间估计，覆盖两方向的不同报文。结合重传和始终开放的接收窗口，节点与家之间的传输是这次优先排查的环节，具体排队或丢包发生在哪一跳仍未知。超时后 ACK 加快说明延长等待有机会覆盖这类短时恶化。应用已在 90 秒取消，无法验证原请求能否完成。

### 45–90 秒内自行恢复的样本

在同一个请求 ID 的两次 30 秒告警之间，可以用活动历史估计没有入站消息的时长。安装包的 NAL 客户端每 5 秒安排一次心跳，在将心跳交给传输前记录活动。若两组入站事件之间有连续 16 次客户端心跳，则两组至少间隔约 75 秒。第二次告警的时间减去日志记录的无活动时长，给出该段最后一条新入站消息的时间上界。

| 请求 | 最后一条入站消息 | 恢复窗口 | 随后读到的消息 |
| --- | --- | --- | --- |
| `af80ce64` | 19:25:26 左右 | 约 75–78 秒 | KV、读取工具完成和模型 token |
| `af80ce64` | 19:26:44 左右 | 约 60–69 秒 | KV |
| `bdd17e69` | 19:44:37 左右 | 约 75–82 秒 | KV、读取工具完成和模型 token |

这三段都在原请求上恢复读取入站消息，未发生重试。这些样本本身无法估计 90 秒之后的恢复率。

复发时同步对比 Snell 两端和 Mac 的 TCP 序号、ACK、接收窗口及 DMIT 发送队列。若窗口持续收缩且 ACK 停滞，沿 HouseMini、网关和 Mac 追读端背压。若窗口充足但 RTT 和重传上升，追 DMIT 到 HouseMini 的传输。若 Mac 已确认持续收到数据而 Cursor 仍报告无入站消息，转查 HTTP 流的分帧和客户端处理。

超时来源已静态核对。Cursor 通过实验配置读取阈值，安装包的内部启动参数 `--smoke-test-nal-stream-timeout-ms=180000` 可指定 180 秒并关闭 30 秒预警，需重新启动 Cursor，尚未做运行验证。用户选择保留 90 秒，先查积压。本轮已补到一例自然复现，Mac 和 HouseMini 的补充采集已结束。

### DMIT 月流量配额

用户在 11 月 7 日查看 DMIT 面板，实例的本月配额已经用完，距自动重置约两天。当晚用户随后付费重置了部分用量，预计限速解除。DMIT 的 LAX EB 产品说明写明，配额耗尽后会限制 VirtIO 端口速率。实例实际开始和解除限速的时刻、档位与方向尚待核实。[DMIT LAX EB 说明](https://www.dmit.io/pages/lax-eyeball) · [DMIT 实例配额规则](https://docs.dmit.io/guide/faq/instance#what-are-the-transfer-overload-rule)

端口限速与 19:32、21:42 观察到的模式吻合。DMIT 仍能从 Cursor 收数据，Snell 送往 HouseMini 的数据积压，其他连接也同时传输。19:30–19:33 与 21:40:58–21:42:28，HouseMini 收到的 Snell TCP 载荷总速率按秒统计，中位数均约 3.5 Mbps，九十分位约 5 Mbps。这些是实际流量，不能用作厂商限速档位。需要对齐限速开始时间和积压窗口，再用相同单跳线路比较重置前后的重传、队列和停滞，才能判断限速贡献。19:40 的 EPON 和 PPPoE 重连及直连失败仍按宽带事件处理。Cursor 方向发来的 FIN/RST 目前只定位到包的方向。

## 处理顺序

| 现场现象 | 已有样本 | 下一步 |
| --- | --- | --- |
| 直连和代理同时大量失败，EPON 或 PPPoE 连接时长重置 | 19:40 | 优先报联通，按[宽带](#宽带)一节的时间窗查局端离线和拨号原因。根据记录处理光路、光猫或局端端口，再核对后续同类故障是否还伴随重新注册。 |
| 某条代理流先关闭，DMIT 到 Cursor 的出口还活着，或 DMIT 先发 FIN | 22:11、19:00 | 保留 Surge 逐连接事件，与 DMIT 的入口、出口和队列同步。22:11 已缩到代理转发路径，仍需确认哪一端发出逻辑结束。只有同类样本指向某个条件后，再针对该条件做单变量对照。 |
| DMIT 先从 Cursor 方向收到 FIN 或 RST，Mac 随后断流 | 19:23、19:26、21:47、21:57、22:07 | 保留 Cursor 请求 ID、时间、传输类型和 DMIT 出口的首个关闭包方向。若同类持续出现，把这些样本交给 Cursor 支持对照服务端或边缘日志。 |
| 客户端报告 90 秒无入站消息，代理的发送队列此前已经积压 | 19:32、21:42 | 先核对 DMIT 配额和限速生效时间。优先用现有单跳线路做配额重置前后对照，比较下行队列、重传、ACK 和停滞次数。需要换线对照时先用相同的合成长连接和可控流量。 |

延长 Stall Detector 的等待时间属于缓解措施。已有请求曾在约 60–82 秒后自行恢复。要验证更长阈值是否有效，需要看到同一请求跨过原 90 秒阈值后继续读到业务消息并成功完成，同时记录真正挂死的请求多久才进入重试。超时方案由对应调查另行评估，本次样本按原阈值分析。

## 复发时怎么查

1. 记下弹错时间和自然运行的 Cursor 回合标识。从 `~/Library/Application Support/Cursor/logs/` 下的 Agent Host 和结构化日志提取请求 ID、传输类型、首次错误、重试、终态和 Stall Detector 时间。保留故障前后的窗口，区分传输失败、模型容量错误和重试用尽。
2. 先检查同一窗口是否有多个独立公网目标或直连流量也失败。若有，尽快看主网关 EPON、WAN PPPoE 的连接时长和最后失败原因，记下读取时间，按计时倒推。用局域网、直连和代理出口各自的证据缩小范围，再向联通提交精确故障窗口。
3. 如果只有一条或一组 Cursor 流失败，先保存 Surge 的连接详情，再按 TCP 四元组及可用的握手或字节计数，把 Mac、HouseMini/Surge、DMIT 的入口和出口对应起来。优先比较同一网卡记录到的事件顺序，跨主机对比前校准时钟。记录首个 FIN/RST、重传、ACK 进度和收发队列，找出哪一段先停止推进。传输仍然建立时，继续核对 Cursor 的计时与恢复日志。
4. 优先复用自然故障和已有记录。对照时同步各端时钟，保留故障前后的成功样本。当前调查只使用被动日志、包头和合成流量，避免触发真实 Cursor Agent 请求。

## 配额恢复后的联合采集

本轮 ID 为 `cursor-20261107T150300Z`，本机证据目录为 `/private/tmp/cursor-20261107T150300Z/`。11 月 7 日 23:03 左右，Mac、HouseMini、DMIT 均已就绪。本轮对照沿用 DMIT 单跳，取自然流量。三端联合窗口截止 23:52:30，宽带统计已停止。

| 分工 | 唯一来源 | 对账字段 |
| --- | --- | --- |
| 宽带 | HouseMini 的已有 Surge 日志增量，必要时补查现有系统日志与网关状态 | 按秒聚合 DIRECT 建连失败和不同公网 IP 数，保留时钟偏差区间。出现多目标失败时再检查 en9 和 WAN 状态。 |
| Snell | HouseMini 的单份外层抓包、Surge 逐连接事件与 DMIT 的入口、出口和 socket 记录 | Surge Record ID、Snell connector、逻辑 EOF、TCP 首个关闭包、发送队列、ACK、重传和窗口。 |
| Stall Detector | Mac 的单份 `en0` 包头与 Cursor 已有日志 | 请求 ID、连接尝试、TLS 指纹、本机端口、业务消息时间、30 秒告警、90 秒取消与重试。 |

每个自然故障保留前 180 秒、后 60 秒。先比较负载相近的重置前后窗口，分别计入下行积压与停滞、来自 Cursor 方向的 TCP 关闭、代理逻辑关闭，以及直连与代理共同失败。宽带事件单独标记。若本轮没有足够的自然活跃连接，只报告观测到的队列和事件范围。已有采集只覆盖 TCP 元数据、计数及代理事件。

配额重置后的第一段定量对照来自 HouseMini。23:04:38–23:07:37，按秒统计同一节点的 Snell 下行 TCP 载荷速率，中位数约 110 Mbps，九十分位约 242 Mbps。重置前两个积压窗口对应约 3.5 Mbps 和 5 Mbps。恢复后的负载更高，但路线已经明显突破先前的实测速率，实际 Cursor 长流的队列和 RTO 也下降。约 50 分钟的联合轮记录了一次 Agent 尝试成功、零条新失败。这支持配额限速是早先积压的一个重要因素，仍需在后续正常使用中观察独立关闭是否复发。

联合轮的宽带对照在 `broadband-direct.jsonl`。HouseMini 已有 Surge 日志记录 39 条 `DIRECT` 建连失败，分布于 30 个秒窗，每分钟最多 4 条，只有 1 条为直接访问公网 IP 的失败。一次只读提取超时后从原日志位置继续，文件增量没有跳过字节。联合窗口未见 19:40 那种直连多目标与代理同时成片失败。下次采集前先检查各端磁盘余量，运行中保留记录器退出和内核丢包计数。

### Mac 与 Stall Detector

- 共用同一个 run ID、UTC epoch 和截止时间。开始与结束各记一组跨端时钟偏差上下界。不同主机的事件间隔落在时钟误差内时，以同一网卡上的包方向和顺序为准。每个自然事件至少保留前 180 秒、后 60 秒。
- Mac 只开一份面向 Cursor 目标的 `en0` 采集。用本机源端口区分连接，用 TLS ClientHello 随机值的哈希与 DMIT 出口精确配对。记录 SYN、FIN、RST、TCP 序号和 ACK 的进度、载荷长度、接收窗口最小值与零窗口次数。SYN 时记录窗口缩放。早于采集的连接若缺少握手，只按关闭时点和双向字节走势给出候选，并注明关联强度。采集不保留载荷内容。
- Mac 同时只读追踪 Cursor Agent Host 和结构化日志。从相同请求 ID 提取尝试开始、30 秒预警、90 秒停滞取消、最后一条入站消息类型与距今时间、连接错误、重试和终态。自然 `socket_reset` 或 90 秒 `stall_detector` 触发保留事件窗。另保留同期成功的 Cursor 握手和直接公网连接作为对照。
- HouseMini 的 Snell 下行窗口与 ACK 复用联合采集中的同一份包头记录。用两端相同的 TCP 指纹跨过 NAT 对齐。Surge 的 Record ID 与本机源端口匹配时校验开始、结束时间和双向字节增量。加密 Snell 外层与 DMIT 出口之间仍以时间和字节走势关联。
- Mac 的包头采集和事件过滤已在 21:42 的自然样本上验证。联合采集于 23:03 就绪，文件为 `mac/packets.jsonl` 和 `mac/events.jsonl`。HouseMini 与 DMIT 的远端文件各自在 `/tmp/cursor-20261107T150300Z/` 下的 `house/`、`dmit/`，跨端时钟校准保存在本机的 `snell/clock.jsonl`。

### 11 月 8 日 00:03 起的连接年龄观察

联合轮 `cursor-20261107T160300Z` 覆盖北京时间 00:03–01:33。Mac 的应用事件过滤器在 00:03:00.863 就绪，包头记录在 00:03:01.020 就绪，最后一条包头时间为 01:32:58.746。共捕获 925,608 个包，内核报告丢包 0，产生 312,414 条聚合记录。包头分段压缩，与应用日志独立写入。

Mac 的临时采集目录在 07:32 复核时已不存在。上文的对账数字及事件顺序来自 01:33–01:36 读回的完整记录。

在 14,766 条同时捕获到 SYN、SYN-ACK 和 TLS 握手的新连接中，13 条达到 30 分钟，0 条达到 60 分钟。30–60 分钟区间累计观察约 1.71 连接小时，其中 3 条出现首个关闭包，10 条在截止时仍在连接。该统计覆盖 Cursor 目标上的 TLS 流，包括常规短连接，不能用作 Agent 故障率。采集开始前已建立的长连接单独分析。

00:09 和 00:13 两条均由应用日志记为 `socket_reset`，`stallAborted=false`。Mac 分别在 00:09:51.656 和 00:13:20.284 收到该流的首个入站 FIN。01:32:05.760 的应用错误同样未触发停滞。对应的 Mac 端口 `62182` 从 SYN 到首个关闭包约 40 分 30.9 秒。同一 TLS 握手匹配的 DMIT 流在 01:32:04.370 先从 Cursor 方向收到 RST。Mac 侧首个可见关闭包为 01:32:04.499 出站 FIN，同秒已从上游收到最后 24 字节。故障距采集截止约 54 秒。其它三端配对与代理关闭时序见上文续采对照。

## 终端 Cursor CLI 与计费

11 月 8 日静态核查，本机 `cursor-agent` 指向厂商发行版 `2026.10.01-e373342`。AskHuman 的「Cursor 会话防护（实验）」挂接 macOS Cursor Desktop 的 Agents 窗口和 IDE 路径。「CLI/MCP」是 AskHuman 的提问接入方式。终端 Cursor CLI 的行为单独判断。三项防护和目前的接入状态见[会话防护规格](../specs/cursor-desktop-background-followup.md)。

| 防护 | 终端 Cursor CLI 的情况 |
| --- | --- |
| 后台完成续跑拦截 | 当前 `9969.index.js` 的交互路径在空闲时会把后台 Shell 或子代理完成转成 `backgroundTaskCompletionAction`，调用下一轮 Agent。显式中断会标记 `user_aborted`，随后处理完成通知时丢弃而不重新入队。自然结束和 AskHuman 回答「结束」后正常收尾，仍有完成通知唤起模型的路径。headless 默认也处理后台完成，隐藏的 `--single-turn` 只接受 `--print` 模式。AskHuman 的 CLI 拦截原型已延期。 |
| 运行中会话保护 | CLI 交互会话通过 `store.db` 保存，与 Desktop 窗口中的消息卸载路径分开。CLI 在终态错误时按收到的 checkpoint 决定回退。若还没有 checkpoint，`stopUiConversation` 可从聊天视图截掉未保存的轮次，并尝试把原提示词放回输入框。是否仍列在 `resume` 取决于是否已保存可用的会话状态。静态核查未找到与 Desktop 相同的卸载后误归档链路。 |
| 延长停滞等待和重试窗口 | CLI `index.js` 已包含 Stall Detector 与自动续跑。当前包内无覆盖值时单次无入站活动的回退阈值为 30 秒，连续停滞重试窗口为 180 秒。普通传输重试有上限 10，两个已连上却没有新 checkpoint 的续跑失败可提前终止。单次 30 秒是静态回退值。公开 CLI 配置未列对应覆盖，AskHuman 的 5 分钟和 10 分钟方案面向 Desktop。 |

自动续跑沿用原请求 ID 关联，给新尝试分配新请求 ID，并在有已保存状态时从 checkpoint 恢复。旧计划的本机 Desktop 账单里，曾对上 30 次有模型活动的尝试，其中 24 次是自动续跑，每条 `Included` 用量记 `Requests=1`。另有两次只见心跳的续跑没找到对应事件。这个对照支持按是否实际发生模型调用理解扣量，不能用重连次数或错误条数推算费用。CLI 在服务端已经执行推理后断线，已发生的用量仍可能入账。续跑若继续产生推理，也可能再消耗额度。`Included` 计入套餐，用量事件本身不代表账单上出现额外付款。[Cursor 对续跑与中断收费的说明](https://forum.cursor.com/t/resumable-agent-sessions-and-usage-protection-after-network-interruptions/165914/7) · [旧计划账单对账](cursor-idle-wait-keepalive.md#24-计费)

本机找到 18 个现存 CLI 临时日志，未找到可对账的断线续跑记录。现有 235 条 Dashboard 导出缺少客户端类型和请求 ID，历史对账样本来自 Desktop。当前能确认 CLI 同样存在断线后续跑和后台续跑调用路径，单次 CLI 自动续跑在此账户上实际扣多少仍需自然失败的 CLI 日志与同一时窗的 Dashboard 明细。复发时先保存 `/logs` 指向的 CLI 日志，按原请求 ID、各次尝试 ID 和准确时间记录，同时保留模型、Max 状态和账单的 `Kind`、`Requests`、Token。若同窗有 Desktop 请求，不能只按时间认定来源。[CLI 日志入口](https://cursor.com/docs/cli/reference/slash-commands) · [用量明细说明](https://prod.cursor.com/help/models-and-usage/usage-limits)

终端和 Desktop 如果都经过同一个 HouseMini 和 DMIT，仍共享联通接入中断和 DMIT 限速。CLI 有独立的网络配置，本机 `network.useHttp1ForAgent` 为 `false`，该配置可切换到 HTTP/1.1 SSE。连接协议可能影响独立断流的表现。AskHuman 另有新建 Cursor CLI 任务时的 Max 闸门，控制旧按次计划是否以 Max 的费率运行。本机 CLI 顶层 `maxMode` 当前为 `false`，这套闸门独立于三项会话防护。[CLI 网络配置](https://cursor.com/docs/cli/reference/configuration#http11-fallback) · [AskHuman Max 控制规格](../specs/cursor-max-control.md)

### 唤醒拦截与超时的 CLI 接入方案

首版候选是 macOS 的交互式 Cursor CLI，两项能力共用一个受管启动入口，分别校验和报告状态。AskHuman 新建终端任务可以在现有 `__agent-launch` 里完成 Max 检查后，给核验过的厂商 `cursor-agent` 进程加私有 Node preload。手动启动可通过一个明确的入口，如 `AskHuman cursor -- <原参数>`。两条路径共用启动代码，保留原参数、工作目录、TTY、信号和退出码。原厂命令和安装目录继续由 Cursor 安装器管理。`agent` 在本机还可能解析到 Grok，受管入口应解析并核验 `cursor-agent` 的真实路径。[既有 CLI 接入评估](../plans/cursor-cli-durable-control.md) · [Node preload](https://nodejs.org/api/cli.html#-r---require-module)

**后台完成续跑。** 历史原型 `233b1a59` 已在本机同版 `2026.10.01-e373342` 验证 Shell、子代理结果、原生 ACK、Shell Await 和手动续聊。可复用它的队列核查。接入应限定在交互式后台完成分发，在调用 `backgroundTaskCompletionAction` 前只消费已排除活跃 Await 的 Shell 和子代理完成通知。继续用原生 ACK 结算 pending、in-flight 与去重，任务结果仍由原生路径保存。这个版本可在 `9969.index.js` 的 `Ka` 处理器内，完成 Await 排除后、建立新 Agent turn 前接入。历史原型包装 `2320.index.js` 导出的 registry `pull`，范围更广，使用时必须证明不会截到 Await 或其他消费者。升级后还要验证 ACK 的同步性和失败语义，避免异步 ACK 或空批次被误判为生效。现有 `stop` hook 只能控制它自己请求的后续消息，CLI 内部直接构造完成动作的路径需要单独接入。[Cursor Hooks](https://cursor.com/docs/hooks)

**延长停滞。** 本机包的主 `index.js` 中，`../agent-client/dist/index.js` 的 `run` 将同一份 `runOptions` 交给 `runInternal` 的 Stall Detector 和重试分类器。进入新回合时可复制并注入 `stallDetectorConfig.failTimeoutMs=300000`，保留原生预警或禁用设置，也保留已经更长的原生阈值。连续停滞分类的 `180000` 在同一模块里，需要配套变成 `600000`，按该回合取到的策略快照处理。可借用已做过预检的 Desktop 思路，把改动限定在原生 `hooks.classify` 的停滞分支，保留 Ctrl+C、普通 reset、次数上限和无 checkpoint 的保护。10 分钟从第一次停滞失败且决定续跑时起算，连续三次各等 5 分钟时，整体约 15 分钟加退避。

离线提取原生函数后已核对 180 秒和 600 秒边界、10 次传输重试上限、用户取消与普通连接错误。单次阈值解析也确认能接受 300 秒并保留原预警或显式禁用。厂商自带 Node `v26.8.1` 提供同步 `registerHooks()`。可在已验证的主包加载时做限定范围的进程内适配。实际 CLI 进程的加载、策略生效与还原还需隔离预检。[Node 同步加载钩子](https://nodejs.org/api/module.html#moduleregisterhooksoptions) · [Desktop 停滞预检](../specs/cursor-desktop-background-followup.md#81-原生预检记录)

验收先跑无模型的原生队列和模拟传输。校验厂商构建和目标文件指纹，确认两处超时改动同时通过后才启用超时能力。CLI 启动阶段恢复原有 `NODE_OPTIONS`，清理 AskHuman 预加载标记，避免子进程继承。未知版本或非交互式模式沿用厂商行为并报告未受保护。首版可在新进程启动时读取开关，给每个已开始的回合固定计时策略。既有原型对 Shell Await 有真实证据，重新接入仍需验证用户的 AskHuman「结束」、子代理结果、原生 ACK 和原始命令恢复。唤醒拦截直接减少非预期模型调用，延长等待只减少某些过早取消，实际额度变化要用自然日志和 Dashboard 对账。

### 旧版兼容性与快捷入口

| 已检查的构建 | 后台完成唤醒 | 停滞计时 |
| --- | --- | --- |
| 7 月 20 日、7 月 23 日 | 同一 registry 模块与后台完成动作。`pull` 和 `ack` 由基类继承，已做过旧版无模型队列探针。 | 两版都可把 `stallDetectorConfig.failTimeoutMs` 传到单次计时器，原生回退值是 30 秒。未找到 10 月构建所用的 `stallRetryElapsedMs` 连续窗口实现。 |
| 10 月 1 日 `e373342` | 模块入口保留，`pull` 和 `ack` 变为派生类方法。旧原型曾验证后台工作结果、原生 ACK 和手动续聊。 | 单次配置入口保留，存在连续停滞 180 秒判定。本轮离线验证了 300 秒配置与 600 秒窗口的边界。 |
| 10 月 14 日的两个样本 | `941492e` 的静态记录仍有原生 pull/ACK，但 in-flight 统计实现换过。 | 本机留存的 `0f9edde` 仍有单次配置和 180 秒连续窗口。它和 `941492e` 是不同构建。 |

初次对照覆盖约 86 天的后台完成机制和单次超时入口，连续重试窗口覆盖 10 月两个日期之间的 13 天。模块组织和队列语义在这段时间内发生过变化，后续用更新发行包和同一套行为验收扩大了样本，见下一节。[CLI 持久性评估](cursor-background-reliability.md#desktop-与-cli-的持久性比较)

首版将 `AskHuman cursor` 作为显式入口。`AskHuman cursor shortcut cursor2` 在 `~/.local/bin` 创建 `cursor2`，使用安装入口原样转发参数。已有同名命令或文件时拒绝覆盖。`cursor2 --resume <会话 ID>` 与 `AskHuman cursor -- --resume <会话 ID>` 使用同一路径。交互终端可启用防护，`--print`、ACP、版本检查等原生模式继续转发。AskHuman 现有新建任务路径未改动。

### 适配如何跨版本维护

补查 11 月 5 日官方 macOS arm64 发行包 `2026.11.05-86047df`。包里出现了内嵌脚本的 `cursor-agent-sea`，同时仍提供 `node` 路径使用的顶层 `index.js` 和 `cursor-agent` 启动器。后者和 10 月 1 日的启动脚本字节完全一致。新主包仍将 `stallDetectorConfig` 交给单次计时器，并在重试状态中保留 `stallRetryElapsedMs`，原生连续停滞边界仍为 180 秒。后台完成 registry 仍用 `pull`、`ack` 和 `forgetInFlight`，in-flight 统计采用 10 月后期已出现的按任务类型统计。此次只读归档，没有执行厂商程序。[Cursor 官方安装脚本](https://cursor.com/install)

据此，单次超时与后台完成机制的静态样本跨度扩大到 **108 天**，连续停滞窗口扩大到 **35 天**。10 月 1 日的重试分类器内联在回合启动函数里，10 月 14 日把它拆成了独立函数。接口形态改变时，继续依赖固定压缩变量名会增加逐版维护成本。公开 CLI 配置中仍未发现这两处停滞阈值的覆盖方式，SDK 的公开重试开关只控制本地 SDK 回合是否自动重试。[CLI 配置](https://cursor.com/docs/cli/reference/configuration) · [SDK 本地重试配置](https://cursor.com/docs/sdk/typescript)

11 月 8 日确认采用原生分类器包装，在重试入口按回合设置 `hooks.classify`。5 分钟通过 `runOptions.stallDetectorConfig` 提供。已确认原生分类器将 `stallRetryElapsedMs` 用作停滞窗口的构建，在目标 600 秒前让该字段缺省。到目标时间后传入真实时长，交由原生分类器处理。这保留厂商原本更长的策略，接入也不依赖厂商的 180 秒数值。同一包装已在 10 月 1 日、10 月 14 日、11 月 5 日的提取函数上验证，300 秒时可继续停滞重试，600 秒到边界，用户取消、普通连接错误和重试次数上限保持原判定。三份构建的主包和后台分发包均通过变换后的语法检查。本机 10 月 1 日的 bundled Node 实际加载两份变换，测试加载钩子随后用夹具取代厂商代码执行。

唤醒和停滞防护新增构建时先验证启动路径、队列 ACK 和两处计时的行为，再记录该构建的指纹。唤醒拦截单独验收，超时的 5 分钟与 10 分钟作为一组通过。如果新构建只有一处超时接入点通过，该构建仍可保留独立验证过的唤醒拦截。两项防护不可用时分别提示，是否能从快捷命令启动由下文的 Max 恢复接口检查决定。

本机可用 `ASKHUMAN_CURSOR_CLI_PACKAGE=<发行包目录> node --test scripts/cursor-cli.node-test.mjs` 复核加载路径。测试用夹具检查原生模块接入、结算、计时和 Max 恢复判定，无需登录或调用模型。

11 月 8 日 01:39，主工作树 `./scripts/install.sh` 成功更新本机日常版。使用已安装的 `AskHuman` 在临时 HOME 生成 `cursor2`，从 `PATH` 调用该快捷命令，核对了包含中文、空字符串和 shell 字符的参数以及替身退出码。再次生成同名入口会失败，原文件字节保留。上述验证使用 Cursor 替身和已安装 Node 的隔离夹具。真实交互的断线率与额度仍从自然样本对账。

### CLI 恢复会话自动打开 Max

本机升级到 `2026.11.05-86047df` 后，CLI 仍会用旧会话的模型 ID 和全局保存的模型参数重新选变体。全局 Max 为关时，若恢复到仅限 Max 的变体，CLI 可把 `maxMode` 写回 `true`，并将 Max 带入下一次请求。用本机模型目录和只读提取的会话模型 ID 离线回放，20 个有模型 ID 的会话都能触发，当前使用的 Claude Sonnet 5.5 300K High 作为非 Max 对照保持关闭。10 月 1 日版结果一致。没有发模型请求。

`AskHuman cursor` 的新增保护在恢复时优先尝试同模型的非 Max 变体，否则保留当前非 Max 模型并提示。启动前按已安装包中的实际接口启用。历史样本 `2026.07.23-e383d2b`、`2026.10.01-e373342`、`2026.10.14-0f9edde`、`2026.11.05-86047df` 横跨 105 天，四版的原生模型选择函数均通过非 Max 回退、仅限 Max 回退、同模型恢复和手动开启 Max 的隔离夹具。三份可运行的厂商 Node 另通过实际加载检查，10 月 14 日暂存包的 Node 无法运行，保留该版的源级和模型选择函数验证。行为边界见 [Cursor Max 模式控制](../specs/cursor-max-control.md#手动恢复-cli-会话)。

11 月 8 日 08:12，主工作树 `./scripts/install.sh` 安装完成。用实际已创建的 `cursor2`、伪终端、临时 HOME 和改名的发行文件夹具验证新版接入。兼容接口下加载 Max 保护，保留字面参数和替身退出码。删去恢复接口后在运行 CLI 入口前以状态码 1 退出。快捷脚本字节未变，测试期间发出 0 次模型请求。
