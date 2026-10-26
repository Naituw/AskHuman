# iOS 原生移动渠道实现交接

更新日期：2026-10-24。接手对象：继续实现 HumanInLoop 原生 iOS 渠道的 Agent。

**已有可运行的 SwiftUI Mac 与 iOS 交互原型，使用用户的 EMQX 杭州 Serverless 实例真实收发。正式 AskHuman 渠道、真实 Agent 控制、真机 APNs 和产品级配对尚未实现。**用户本轮要求保存交接，后续由其他 Agent 实现；本文件是实现起点和证据索引，不是已经获确认的完整产品规格。

## 用户目标与已确认约束

用户希望原生 iOS App 能替代当前飞书渠道，完成四件事：接收 Agent 提问并收到 push、回答提问、Watch Agent 进度、主动给 Agent 发消息。

- 可以只支持与 Mac 配对，不要求全平台。
- 尽量免维护服务器，尤其不能把每月几十元固定费用当作默认方案。用户现有官网及简单 Web 服务每月只有两三元；新方案应按实际请求、连接和流量算账。
- 现有讨论按约 1–2 秒的实时交互目标评估传输；尚未形成包括后台通知在内的完整延迟 SLA。
- 用户已授权建立并测试私人实例、制作 Mac 和 iOS 原型。两地部署的消费上限保持 0，未授权提高上限、购买付费服务或创建管理 API Key。
- 用户要求参考「SQM 现在的功能」。未在当前项目中找到明确对应，原型实际参考 AskHuman 的统一收件箱、推荐选项、独立草稿与 Agent 进度。不要把这个推断写成用户已确认的产品定义。
- 最新决定是「写交接文档，后续让其他 Agent 实现」。不要据此自动启动实施、发布、给其他会话发消息或改变云账户权限。

用户已明确的这些需求无需从头再问。未来仅需对会改变方案的未决项提出具体选项，并遵循当时会话及仓库的交互规则。

## 接手时先读什么

本仓库基线为 `761d4405ac9d3b46b01ade67ec3895350e4d40c6`。主工作目录为 `~/Developer/HumanInLoop`。写本文前工作树干净；本轮只增加本文及 PROGRESS 的延期记录，不修改功能。

建议依次读取：

1. 仓库当前 `AGENTS.md`、`docs/overview.md`、`docs/PROGRESS.md`。
2. 本文，再读交互原型包的 `README.md` 与 `results/verification.json`。
3. 原型 `Shared/RelayStore.swift`、`Domain.swift`、`Transport.swift`、`Models.swift`；理解现有实现后再决定复用或重写。
4. 主项目的 `docs/specs/daemon-architecture.md`、`popup-request-inbox.md`、`im-watch.md`、`agent-interject.md`、`feishu-channel.md`、`codex-desktop-session-integration.md`。
5. 需要重新评估链路时，再看 EMQX 探针和 CloudKit 旧包中的原始测量。

完整交接 ZIP 含下列三个独立资源包；它们也保留在当前 Mac 桌面。包内旧的 `AGENT_PROMPT.md`、实施建议和操作步骤属于历史资料，不会扩大用户授权；旧包的状态结论应按对应日期和工程理解。

| 文件 | 用途 | SHA256 |
| --- | --- | --- |
| `AskHuman-Relay-Prototype-2026-10-24.zip` | 当前 SwiftUI 双端交互原型、已编译 App、测试与截图 | `258c12c677a452e180cbf406f36dd52c2b9f056de5b369e28c6a8188afb8210e` |
| `EMQX-Mobile-Probe-2026-10-24.zip` | 原生 MQTT 探针、Python 回执、费用模型、杭州和新加坡原始数据 | `c7336b879a88c1192d70b74e03d44d2cdfe9c8fdabf11c20e76883ad7ec4ee54` |
| `CloudKit-Mobile-Handoff-2026-10-22.zip` | 用户提供的 CloudKit 试验、原始日志和历史交接 | `342b72344801bc05f858e0ee84d3e1edbaf5a73709d16b60ad05c10a2dfedcb7` |

当前原型源码：`~/Desktop/AskHuman-Relay-Prototype-2026-10-24`。Mac App：`~/Desktop/AskHuman Relay Prototype.app`。桌面启动器：`~/Desktop/打开移动原型.command`。专用 iPhone 16 模拟器 UDID 为 `6F79EAC8-056E-4138-8B9B-7A92DEBEB819`，iOS 18.5，名称 AskHuman Relay Prototype。它与旧探针模拟器不是同一台。

原型 Bundle ID 分别为 `com.wutian.AskHumanRelayPrototypeMac` 和 `com.wutian.AskHumanRelayPrototype`；项目 Team 为 `DMJXDB9H6Q`，APNs 入口默认开发 Sandbox。这些是测试标识，不代表正式 App 的分发标识已定案。

这些绝对路径仅适用于当前 Mac。其他机器解压资源包即可阅读源码和证据；连接私人实例、签名、真机安装仍需要相应权限。不要重置或清空旧 CloudKit 容器、已有模拟器或正式 AskHuman 数据。

## 当前证据支持什么

### CloudKit 旧实验

2026-10-22 的一秒轮询加批量 ACK 方案，Watch 往返 p50 3.254 秒、p95 9.031 秒；反向消息往返 p95 23.785 秒。这是完整往返，不是 Watch 单向显示耗时。

纯推送诊断中，Mac 写入 30 个快照，iPhone 从推送触发读取收到 30 个唯一事件；Mac 进程没有收到数据推送或回执，因此没有可用 RTT。手机最大接收批次间隔为 43.574 秒，包含读取失败、退避和积压，不能直接叫 APNs 延迟。标准 App 启动后的 Mac APNs 注册成功，但没有完成新的反向验证。

**不能据此说 CloudKit 不支持推送或双向通信。**现有证据不足以把它作为满足本需求的实时主通道；也没有证明所有延迟都由 CloudKit 服务端造成。旧包 `RESULTS.md` 明确记录这些边界。

### EMQX 杭州与新加坡

2026-10-24 杭州测试的 RTT 是「发送 → Mac 写入 SQLite → 应用层 ACK 返回」，不是 MQTT PUBACK，也不是 Agent 已消费。

| 杭州场景 | 收到与计划 | p95 | 最慢 |
| --- | ---: | ---: | ---: |
| 原生 iOS TLS → Mac TLS，5 KiB | 50/50 | 0.259 秒 | 0.286 秒 |
| 原生 iOS WSS → Mac TLS，5 KiB | 50/50 | 0.268 秒 | 0.395 秒 |
| 原生 iOS WSS → Mac WSS，5 KiB | 50/50 | 0.174 秒 | 0.257 秒 |
| 五路合成 Watch，每路每 2 秒 5 KiB，单向 | 100/100 | 0.119 秒 | 0.131 秒 |
| Watch 同时发送的命令，业务 ACK 往返 | 20/20 | 0.154 秒 | 0.167 秒 |

七组主请求共 300/300，主请求最慢 0.614 秒；33 次重复请求与七组显式重连后的回执检查完成。两地各 11 项认证和主题隔离检查通过，包含匿名、错误密码、未知用户和跨配对访问拒绝。杭州接收进程重启后同 ID 只有一行记录。

这些 iOS 测量均使用同一 Mac 上的模拟器。域名解析为 Fake IP，路由先进入 `utun10`；没有确认底层代理或直连。没有真机 Wi-Fi、蜂窝、切网或长时间数据。**杭州结果是继续采用该候选的依据，不是生产稳定性承诺。**

新加坡历史数据必须保留：多组 p95 达数秒且有超过 10 秒的确认超时；五路 Watch 单向 p95 8.01 秒，同时命令 RTT p95 14.42 秒。两地时段和路径不同，不能认定区域是唯一原因。不要把「全部收到」等同「实时达标」，也不要只统计成功样本而隐藏超时。

主要证据在探针包 `results/ios-china-tls-5kb.json`、`ios-china-wss-5kb.json`、`ios-china-both-wss.json`、`china-watch-streams.json`、`china-responder-restart.json`、`china-access-control.json`、`china-network-context.json`。

### 当前交互原型

原型使用标准 TLS 8883、CocoaMQTT 2.4.1、SwiftUI、CryptoKit。实际界面操作验证了：

- Mac 创建问题，iOS 选择选项回复；iOS 发送 `Hello mac`，Mac 收到；Mac 追加文字，iOS 收到。
- 一个会话的文字草稿与另一会话的选项草稿分别保存，手机重启后恢复。
- Mac 退出后，手机发送 `Queue`，再退出并重启手机；Mac 恢复后两端各一条、待发队列清空。证据为 `results/offline-before-restart.json`、`offline-after-phone-restart.json`、`offline-recovered.json`。
- 演示 Agent 进度在手机观察到 83% 到 100%，Agent 页可进入详情。**演示并未连接真实 Agent，也不执行命令。**
- 七项核心单元测试通过；Mac、iOS Simulator、未签名 iPhoneOS 设备架构构建通过。未安装真机；设备架构编译不能替代设备运行。
- 最终 Mac 和模拟器包通过代码签名校验。截图、验证汇总与构建日志均在原型包 `results/`。

原型有在线本地通知、APNs Token 注册、点击通知路由、Mac APNs 提供方及设置界面，但没有配置 `.p8`、实际发送 APNs 或验收通知权限展示。二维码和扫描入口已实现，物理相机扫码未验收。

原型每次发送整个会话快照，正文限 64,000 字节，加密包限 900,000 字节。它没有完整多题表单、权限审批、Stop 确认、附件、分页、撤回/过期和跨渠道抢答。其 JSON 原子替换及去重验证覆盖进程重启，没有断电或任意业务副作用的 exactly-once 保证。

## 费用与云资源约束

当前国区项目 AskHumanMobile 的 AskHumanCN 实例为 `<instance-id>`，阿里云杭州；TLS 地址 `<instance-id>.ala.cn-hangzhou.emqxsl.cn:8883`，WSS 同域名 `8084/mqtt`。消费上限 **¥0**。旧国际站 AskHumanDemo 为 `<instance-id-intl>`，Google Cloud 新加坡，上限 **$0**，不要把它当成已经删除的资源。

国区每月免费 100 万连接分钟、1 GB 入站加出站流量、100 万规则动作；超额单价分别 ¥8/百万分钟、¥1.5/GB、¥1.8/百万动作。官网的 GB 按 1024³ 字节计算，日结可能舍入。上限 0 意味着免费额度用完停止，不能一边保持 0 上限一边自动支付超额。[EMQX 国内计费](https://docs.emqx.com/zh/cloud/latest/price/pricing.html)

30 天、Mac 每天在线 24 小时、手机前台 1 小时，连接量为 `30 × 25 × 60 = 45,000` 分钟，在未占用的免费额度内连接费为 0。一个手机订阅者的消息体双向流量估算为：

```text
GB = 天数 × 每天观看秒数 ÷ 更新间隔秒数 × 每包字节 × 同时流数 × 2 ÷ 1024³
流量费 = max(0, GB - 尚余免费 GB) × 1.5 元
```

| 30 天使用假设 | 消息体双向量 | 免费额度完整可用时的理论流量费 |
| --- | ---: | ---: |
| 合计 5 KiB，每 2 秒一次，每天 1 小时 | 0.515 GB | ¥0 |
| 5 路各 5 KiB，每 2 秒一次，每天 1 小时 | 2.575 GB | 约 ¥2.36 |
| 已计所有开销的月总量 1.5 GB | 1.5 GB | ¥0.75 |
| 已计所有开销的月总量 3 GB | 3 GB | ¥3.00 |

前两行不含协议、ACK、心跳、重试、加密封装和不断增长的会话快照。本原型 Base64 约放大三分之一；五路示例仅加入这一项即约 3.43 GB、¥3.65，仍非实际账单。也不包含 Apple 开发者资格、附件存储、可能新增的推送/配对服务。原型未使用云规则动作。可复算脚本在探针包 `scripts/cost_model.py`。

后续应先记录真实包大小、观看时长和连接数，再做预算；不要笼统报「几十元/月」。若需要极小的受信任服务，应先核实能否复用用户已有的按量 Web 服务及其单价；目前不知道该服务的平台、运行限制和计费明细。

## 后续架构建议

以下是依据当前证据提出的实现方向；未决产品选择仍见下一节。

```text
真实 Agent 与 AskHuman CLI
             │
      Mac AskHuman Daemon
      ├─ 请求协调与首个终态
      ├─ Agent registry / Watch / Interject
      ├─ 持久状态 / outbox / 去重
      ├──────── EMQX TLS 或 WSS ──────── iOS 前台与恢复同步
      └──────── APNs 提供方 ─────────── 系统通知与打开入口
```

主项目已由 Daemon 独占渠道与长连接。建议正式移动渠道复用该权威状态和现有业务入口，SwiftUI Mac 原型用作交互与通信参考。不要简单让两个独立业务数据库同时决定哪个答案有效；Mac 端最终是集成到现有应用、配一个辅助进程，还是保留独立原生 App，需要在实施规格中定清。

### 业务接入

| 功能 | 当前仓库入口 | 接入要求 |
| --- | --- | --- |
| 提问与跨渠道抢答 | `src-tauri/src/channels/mod.rs` 的 `Channel`，`src-tauri/src/app/coordinator.rs`，`channels/feishu.rs` | 复用 start/interrupt/result sink；首个终态获胜，其他设备收到终态 |
| 多请求表单 | `docs/specs/popup-request-inbox.md`，`src/views/PopupInboxView.vue` 与 `src/views/popup/` | 一次请求可有多题；独立草稿；类型、选项、取消和结果契约保持一致 |
| Agent 列表与状态 | `src-tauri/src/agents/registry.rs`，`agents/activity.rs`，`daemon/runtime/mod.rs` | 使用可信会话身份和能力，不以列表序号作为持久身份 |
| Watch | `src-tauri/src/watch.rs`，`daemon/runtime/watch.rs` | 复用状态生成；工作态约 2 秒 tick，也可由事件唤醒；无观看者时避免无意义流量 |
| 给 Agent 发消息 | `src-tauri/src/agents/interject.rs`，`daemon/runtime/subs.rs`，`docs/specs/agent-interject.md` | 复用实际支持家族和送达机制；排队与 hook 消费必须分别展示 |
| 原生桌面会话 | `docs/specs/codex-desktop-session-integration.md` | 使用已有适配能力，不能假定所有 Agent 都支持相同控制 |
| 设置与协议 | `src-tauri/src/config.rs`，`src-tauri/src/ipc/mod.rs` | 按配置迁移与 IPC 兼容规则实施，不破坏当前安装与 Dev Instance 隔离 |

需要明确区分：手机本机已保存、Mac 已持久接收、请求答案已被 Coordinator 接受、Agent 真正已消费。MQTT PUBACK 和 APNs 200 都不能代表最后一项。现有插话可能等下一次 hook，网络变快不会消除这段等待。

### 可靠性与同步

正式协议应定义协议版本、配对身份、消息 ID、请求/会话 ID、顺序或修订号、终态与有效期。相同 ID 的重试须绑定相同内容，不能让同 ID 不同 payload 静默复用旧结果。

建议把接收去重、业务状态转换和待发回执纳入明确的持久化边界。重启、断线、ACK 丢失、乱序和跨渠道抢答后，都要能重建正确状态。iOS 应能快速补一个权威快照，再接增量；Watch 中间帧可以合并，答案和控制消息不能因合并而丢弃。

当前原型使用共享配对密钥、ChaChaPoly 加密与 Keychain、每端约 220 ms 发布间隔、5 秒 outbox 重试、15 秒 presence。可复用基本思路，但整个会话快照、固定心跳、单个 JSON 文件和单一配对不等于正式规模设计。消耗上限、过大的消息和失效凭据需要清晰错误状态。

### APNs 与使用范围

**个人自用阶段：**自己的 Mac 可以持有自己的 APNs 提供方密钥，直接调用 Apple。iOS 将 Token 通过加密 MQTT 回传，Mac 发不带问题正文的通用提醒，点击后补同步。这不要求再租常驻云主机，但 Mac 必须运行联网。

**向其他用户发行：**不能把应用运营方的 APNs `.p8` 私钥或 EMQX 管理密钥嵌入每台用户 Mac。Apple 要求 APNs 签名密钥保持私密；是否采用受信任的按量推送服务，或让每个高级用户自备开发者与实例配置，必须先定使用范围。[Apple APNs 提供方认证](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns)

这是一条需要补充的产品边界，不是对现有个人原型的否定。已有 Mac APNs 代码可作私人测试基线，但不能由此宣称「公开发行也完全不需要可信服务端」。

iOS 后台不能靠 MQTT 常驻保证收信。应实测前台、后台、锁屏、被系统终止及用户强制退出的差异，并分别标记失败重试、通知展示与点击恢复。不要用模拟器注入通知、本地通知或 Apple 接受回执代替真正 APNs 送达证据。

### 配对与密钥

原型复用已创建的两个测试身份。当前 ACL 限制在随机配对前缀内，并有全局拒绝兜底；没有实现该配对内部按 Mac/iOS 方向隔离的权限。二维码直接带手机身份和配对密钥，是私人可重复导入信息，不是短期挑战或一次性配对票据。

正式方案需要定义授权来源、短期配对确认、设备移除、密钥轮换、重装恢复和已撤销设备拒绝。至少验证旧二维码/旧设备在撤销后不能继续使用。公开发行前核实 EMQX Serverless 的账号与 ACL 自动化接口、账户权限和价格；已有 API 目录只验证到客户端、订阅和发布，不能从中推断自动发号已具备。[Serverless API](https://docs.emqx.com/en/cloud/latest/api/serverless.html)

当前 Mac 的受限配置路径：探针 `Config/china-secrets.json`、`Config/private-secrets.json`，交互原型 `LocalConfig/`。这些只在本机，ZIP 不包含。读取必须局限于被授权的测试；不要在终端输出、命令参数、文档、截图或提交中暴露值。不要重新生成配对密钥后仍期待旧手机能继续解密，也不要把管理密钥作为设备凭据。

## 实施前需要定清的范围

下列问题尚无用户定案，应随实施规格处理，不要把原型的临时选择当成要求：

1. 首期只服务用户本人，还是面向其他人发行；这影响 APNs、MQTT 身份与可信服务端。
2. Mac 侧最终形态、iOS 分发方式、Apple Team 与正式 Bundle ID。原型使用 macOS 14+/iOS 17+，只是实现基线。
3. 飞书替代的首期范围：普通问答之外，多题、权限、Stop、文件图片、历史和多 Mac 是否必须首发。四项原始能力必须保留，新增范围不能自行当作已确认。
4. 1–2 秒目标具体适用于前台命令 ACK、Watch 新内容还是通知；按 p95、最大值还是另一标准验收。推送受平台与网络影响，不应承诺每次锁屏严格 1–2 秒。
5. 「SQM」的具体指代，以及现有官网 Web 服务是否适合承载可能必要的极小推送/配对职责。

## 建议实施顺序与完成条件

| 阶段 | 应产出什么 | 完成条件 |
| --- | --- | --- |
| 1 明确首期使用范围 | 一份短的移动渠道 spec，定义 Mac 形态、身份、APNs 和首期表单 | 关键选择有依据，费用按具体用量估算 |
| 2 验证真机通知与链路 | 正式签名的真机 App、APNs 提供方、Wi-Fi/蜂窝测量 | 真通知可展示、点击回到对应请求；数据重连恢复；有超时原始记录 |
| 3 接入真实问答 | Daemon 移动渠道与 iOS 请求模型 | Agent 提问到手机，回复返回对应 CLI；取消、过期与别处已答正确收敛 |
| 4 接入 Watch 与 Interject | 真实 Agent 列表、订阅、消息和状态回执 | 多 Agent 可同时看，命令不中断；Mac 收到与 Agent 消费区分清楚 |
| 5 完善配对与恢复 | 已定义范围内的配对、撤销、重装、密钥与持久化 | 旧设备撤销有效；Mac/iPhone 重启无重复答案或静默丢失 |
| 6 产品验收 | 原生 UI 与完整链路报告、流量预算、运行说明 | 满足已定范围；已知限制明确；主项目安装后真实渠道可用 |

先验证真机 APNs，避免把 UI 做完才发现身份或分发方式不支持预期。原型 UI 和协议可以复用，但正式 Agent 接入应以主项目现有状态机为准。

测试至少覆盖：跨渠道同时回答、重复及同 ID 不同内容、乱序旧快照、提交后断网、ACK 丢失、双端分别重启、Mac 睡眠恢复、不同网络、通知权限拒绝、Token 更新、跨配对访问拒绝和设备撤销。延迟应拆成状态产生到发布、网络/持久 ACK、UI 展示、Agent 消费和 APNs 展示，不能把不同起终点混成一个数字。两台物理设备未校时的单向时间不要当精确网络延迟。

公开服务、自动开户、提高消费上限和真机账户能力变更都不是本交接已执行的动作；后续在具体范围获得授权后再做。

## 复现原型与开发注意

在本机，双击桌面启动器即可打开已配对原型。开发时打开 `AskHumanRelay.xcodeproj`，选择 RelayMac 或 RelayPhone。依赖版本已锁定。修改 `project.yml` 后再运行 XcodeGen。

```sh
cd ~/Desktop/AskHuman-Relay-Prototype-2026-10-24
./scripts/build.sh
# Quit the running Mac prototype before replacing its bundle.
./scripts/install_mac.sh
```

签名与调试中已解决的具体问题：

- iOS Simulator 必须保留 Xcode 默认签名；禁用签名曾导致 Keychain bootstrap 未导入，不能因为 App 能启动就判配对成功。
- Mac 使用现有个人 Developer ID，Keychain 服务项为独立的 `.signed-v1`。不要反复改成 ad-hoc 签名；旧临时签名项仍保留，未改权限。
- 安装 Mac 必须替换完整 app bundle；目录叠加复制曾留下旧 `RelayTests.xctest`，导致签名验证失败。`install_mac.sh` 已避免此问题。
- CUA 对模拟器文本区域的直接粘贴曾只改变视觉值，没有触发 SwiftUI 草稿绑定；实际 UI 验证改用屏幕键盘并核对持久草稿。不要把自动化操作返回成功当成已发送。
- Device Hub 是当前 Xcode 的模拟器界面。只操作本次专用 UDID，不使用其他正在工作的模拟器。原型的 Capture Keyboard 测试开关已恢复关闭。
- 原型不支持可靠的解绑或换 Mac 流程。不要把重新安装 iOS App 当作清除 Keychain 配对项的可靠办法；正式实现应有明确的解绑、清除范围与恢复路径。

本机原型已经准备两条测试会话及一条待用户体验的「试试你的移动收件箱」。测试内容是合成消息。CloudKit、EMQX 探针和当前交互原型分别是三个工程；不要拿探针 README 中的「尚无收件箱/APNs 代码」覆盖后来交互原型的实现状态。

正式仓库功能修改前，如需独立 worktree，先按 `docs/agent-worktree-setup.md` 配置隔离环境。IPC 变化遵循 overview 的协议号规则；功能变更完成后按 AGENTS 执行安装脚本，再用新安装的 AskHuman 验证。本次仅写交接文档，不执行正式功能安装。
