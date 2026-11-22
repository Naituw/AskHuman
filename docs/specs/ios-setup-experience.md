# 需求：iOS App 渠道的设置体验（设置进度、Broker 与 TestFlight 引导、Agent 代配置、移出「实验」）

> 状态：设计已通过（2026-11-22，第 3 轮确认）。D1–D5 是用户经 AskHuman 第 1 轮定案的；D13、D18–D21 是第 2 轮定案的；D6–D12、D14–D17 是设计者做原型时的派生决定，随整体设计在第 3 轮通过（§12）。
> 原型已做完并截图（`docs/assets/ios-setup-experience/`；真实 `SettingsView` + mock 后端，代码在 `src/prototype/` 的 `settings-ios.html` 一组文件，不进产品）；**产品代码尚未改动**（除了把 EMQX 提醒里「作者所知」的措辞改掉：`src/i18n/zh.ts`、`en.ts` 各一行）。
> 依赖 / 复用：[`ios-channel.md`](ios-channel.md)（D22：内测期放「实验」，对外发行后转正）、[`ios-open-access.md`](ios-open-access.md)（OA2–OA6 Broker 并列与中立措辞、OA11 服务商目录、OA12 本文接手的可见性）、
> [`docs/wiki/ios-broker.md`](../wiki/ios-broker.md)（Broker 事实的唯一出处）、[`onboarding.md`](onboarding.md)（`setup-probe` 探测）、
> [`settings-agents-channels-cards.md`](settings-agents-channels-cards.md)（渠道卡片的徽标、状态标签与版式）、[`ios-external-testflight.md`](../plans/ios-external-testflight.md)（外部测试的资料与 Apple 侧约束）。
> 平台：Mac 端界面三端同一份（iOS 渠道代码平台无关，只在 macOS 实机验证过，见 §10.6）；手机端 iPhone，iOS 17+。
> 影响面：`src/views/settings/`（`IosChannelCard.vue` 重做并拆分、`iosBrokers.ts` 沿用、`ChannelsTab.vue`、`ExperimentalTab.vue`、`AwayCard.vue`）、`src/views/setup/RemotePage.vue`、i18n；
> `src-tauri/src/cli/`（`ios_cmd.rs`、`channel_cmd.rs`、新增 `ios_guide.rs`）、`src-tauri/src/onboarding/`（探测支持 iOS）、`src-tauri/src/daemon/runtime/mod.rs` 与 `src-tauri/src/ios/gateway/requests.rs`（探测的投递）；
> `scripts/build-ios-beta-site.mjs`（Pages 落地页）、`docs/public/ios/`、`docs/wiki/ios-setup*.md`、README；iOS App 的两处路径文案。
> **不改**：IPC 消息与 `PROTOCOL_VERSION`（§9）；配置字段与 recovery 存储；配对协议与端到端加密；`测试连接`（连接 + 往返 + 隔离自检）与中继的行为；`docs/wiki/ios-broker.md` 里的 Broker 事实。

## 1. 背景与目标

今天一个新用户要配好 iOS 渠道，要在五个地方来回：读朋友内测指南（[`docs/public/ios/beta-zh.md`](../public/ios/beta-zh.md)）→ 去 Broker 控制台建账号和访问规则 → 回 Mac 打开隐蔽的「实验性功能」开关再进「实验」页填表 → 等 TestFlight 邮件邀请 → 手机配对 → 终端里跑一条命令才知道通不通。设置页本身只有一张平铺的表单：服务商下拉和几条要点，没有先后顺序，看不出做到哪一步，也没有任何关于「手机上装什么、怎么装」的内容。

目标：

1. **用户一打开这张卡就能一路配完**：Broker 怎么配、TestFlight 怎么加入、手机怎么配对、怎么确认通了，都在卡片里，不需要先读外部文档。
2. **也能交给 Agent 配**：Agent 读一份随版本更新、带当前状态的说明，完成它能做的部分；必须本人做的事（注册、输密码、装 App、扫码确认）由 Agent 通过 AskHuman 叫用户做。**密码与配对码永远不经过 Agent。**
3. **iOS 渠道对所有用户可见**，不再藏在「实验」后面。

![整体位置](../assets/ios-setup-experience/00-in-context.jpg)

## 2. 已确认决策

### 2.1 用户经 AskHuman 定案（2026-11-22，第 1、2 轮）

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | TestFlight 加入方式 | 开 TestFlight **公共链接**；卡片里放按钮和二维码，手机扫码就能加入。链接放在 Pages 站点（`https://naituw.github.io/AskHuman/`）的一个落地页上，今后换链接不用发 Mac 新版（§5）。**否决**继续用邮件邀请：每个新用户都要作者手动加，开放不起来。 |
| D2 | 卡片形态 | 卡片顶部做「设置步骤」清单：完成的步骤折叠成一行，当前步展开；配好后收成一行，下面保留现有的管理项（§3）。**否决**单独的向导窗口 / 弹窗：设置页是日后回来管理的地方，向导和管理拆成两处会各自过时。 |
| D3 | Agent 协助 | 新增 `AskHuman channel ios --agent-guide`（随版本更新、带当前状态、中英文），卡片按钮复制一句提示词让 Agent 去运行它（§7）；**Broker 密码永远由用户在本机输入，不经过 Agent**。指南里要有我们验证过的服务（EMQX、flespi、自建 Mosquitto），并对 EMQX 特意提醒：**充值最低 100 元，建议关注流量使用，尽量保持在免费额度内**（§4.4）。 |
| D4 | 两个附加项 | 保活一键开关（§8）+ 卡片里「发一条测试问题」按钮（§9）。 |
| D5 | 可见性 | 把卡片整体移出「实验」（对外承诺稳定性），即 `ios-channel.md` D22 的「转正」。**实施可以先做；放给用户的时间（Mac 新版取消「实验」、打开公共链接）要用户另行拍板**（§10.4）。 |
| D13 | 卡片位置（第 2 轮） | 渠道页**最上面**：「本地弹窗」卡之下、「建议开启「IM 渠道按需发送」」提示之上（[00](../assets/ios-setup-experience/00-in-context.jpg)）。 |
| D18 | 文案原则（第 2 轮） | 这是 App 界面，不是攻略：每句话只说用户要做或要知道的事，越短越好；**不写防御性说明**（给 Agent 的提示词里不写「不要索取密码」，这些规则在 `--agent-guide` 的输出里）；**不用作者口吻**（「据作者所知」「已实测」「下面三条路」），拿不准的事用「可能」。全部新增 / 改写的文案见 §14。 |
| D19 | 公共链接的组与上限（第 2 轮） | 新建外部组「AskHuman Public」，测试员上限 **1,000**；与 Friends 的 1–5 人邀请分开管理（§5.3）。 |
| D20 | 放出顺序（第 2 轮） | 先 Pages 落地页 + 公共链接 + iOS 新构建，最后发 Mac 新版；具体放出时间由用户另行决定（§10.4）。 |
| D21 | README（第 2 轮） | 加一节（中英）介绍 iOS App 并链到落地页（§10.5）。 |

### 2.2 设计者的派生决定（第 3 轮随整体设计通过）

| 编号 | 决策项 | 结论 |
|---|---|---|
| D6 | 步骤数 | **3 步**：连接 Broker → 安装 iPhone App → 配对手机。「试一条」不是第 4 步：配对完成就已就绪，没有「走到」第 4 步的时刻，它做成「已就绪」框里随时可点的一行（§6.2）。**否决** 4 步清单：试一条一直亮着「未完成」，或者要靠人为的「下一步」才出现。 |
| D7 | 完成的判定 | 用「事实」而不是「点过按钮」：Broker 已连接、至少一台已配对手机；唯一的例外是「安装 App」，它没有可检测的事实，用一个只存在于界面里的「我已装好」（§3.3）。 |
| D8 | Broker 的选择 | 三个**中立**磁贴：EMQX Cloud / flespi / 自建或其它；不预选、不加「推荐」（OA3）。选了只填默认端口，flespi 再填主机；不会清掉用户已经填的内容（沿用 `iosBrokers.ts: brokerFill`）。 |
| D9 | EMQX 的额度提醒 | 选了 EMQX 后，**先于步骤**显示一条橙色提示（§4.4），让人在去注册之前就看到；配好之后在卡片底部以中性色常驻一条。不可关闭、不折叠。flespi 有一条中性提示；自建没有。 |
| D10 | 二维码指向 | 指向 Pages 落地页，不直接指向 TestFlight 链接：换链接只需重新发布 Pages（D1）。**否决**直接编码 TestFlight 链接：它写进 Mac 二进制，换链接要发新版。 |
| D11 | 保活开关 | 就是「高级 → 守护进程生命周期」那个**全局**设置（`general.daemonLifecycle`：`keepalive` / `activity`）的快捷开关，不新增配置；默认保持现状（关）；文案如实写「常驻并开机自启」；被「允许从手机 / IM 创建 Agent 任务」锁定时只读并写明原因（§8）。 |
| D12 | 测试问题的实现 | 复用设置向导的探测（`setup-probe`），给它增加 `ios` 目标；**不新增 IPC 变体，不升 `PROTOCOL_VERSION`**（§9）。 |
| D14 | 折叠规则 | 当前步展开，同时只展开一步；已完成的步骤一行；Broker 一旦连上，服务商的配置步骤折叠成「配置步骤（EMQX Cloud）」，表单留着；连上后**停在原步**，由「下一步」切到下一步，不自动跳（避免页面在用户眼前跳动）。 |
| D15 | Agent 指南的边界 | Agent 只能做**不含机密**的部分：读状态、写主机 / 端口 / 用户名 / 启用、跑 `channel test ios`、（先问）改守护进程生命周期；`pair`、`allow`、`deny`、`export`、`import` 与任何要密码、配对码、连接串的事一律只能用户做（§7.4）。 |
| D16 | 实验机制本身 | 隐蔽开关、「实验」Tab 与 `experimental.enabled` 字段**保留**，现在没有内容时显示一行「目前没有实验中的功能」；不删、不迁移配置。**否决**顺手删掉：以后的实验功能还要用，与本次的范围无关。 |
| D17 | 平台范围 | 卡片对三个平台都显示（与其它 IM 渠道一致）；Windows / Linux 沿用「只在 macOS 验证」的口径（`ios-channel.md` 平台基线），不另做平台分支（§10.6）。 |

## 3. 卡片的结构与状态

### 3.1 三种形态

| 形态 | 何时 | 内容 | 截图 |
|---|---|---|---|
| 未启用 | 渠道开关关着 | 头部 + 一句简介 + 两个按钮「开始设置」「让 Agent 帮我配置」 | [01](../assets/ios-setup-experience/01-off.jpg)、[02](../assets/ios-setup-experience/02-agent.jpg) |
| 设置中 | 已启用，三步里还有没完成的 | 头部 + 「设置步骤 n / 3」清单（手风琴）；清单右上角是「让 Agent 帮我配置」 | [03](../assets/ios-setup-experience/03-choose.jpg)–[11](../assets/ios-setup-experience/11-pair-confirm.jpg) |
| 已就绪 | Broker 已连接且至少一台手机已配对 | 头部 + 绿色「已就绪」框（状态行 + 测试问题行）+ 管理项；「查看设置步骤」可把清单重新展开 | [12](../assets/ios-setup-experience/12-ready-first.jpg)–[14](../assets/ios-setup-experience/14-steps-reopened.jpg) |

头部沿用渠道卡片的写法（`ChannelCardHead`）：徽标 + 名称 + 状态标签、右侧「配置指南 ↗」与启用开关。「配置指南」指向 `docs/wiki/ios-setup*.md`（随界面语言）。简介沿用现有的一段（`descPublicRelay`），不改：
「在 iPhone 上用 AskHuman App 回答问题。消息端到端加密，Broker 只能看到密文。需要自备 MQTT Broker；推送默认走公共中继，也可以换成自己的。」

头部的开关就是 `channels.ios.enabled`。「开始设置」只是开关的快捷方式（打开它，卡片进入「设置中」或「已就绪」，由 §3.3 的事实决定）。

### 3.2 状态标签

沿用 `channelState` / `channelTone` 的词汇，不新增状态：

| 条件 | 标签（语气，`channelTone`） |
|---|---|
| 未启用且什么都没填 | 未配置（灰） |
| 未启用但已填过 | 未启用（灰） |
| 已启用，主机 / 用户名 / 密码有缺 | 配置不完整（橙） |
| 已启用且填全，Broker 还没连上、也没有报错 | 已启用（绿） |
| Broker 已连接 | 已连接（绿） |
| 已启用且填全，网关报错（`last_error`） | 异常（橙） |

### 3.3 三个步骤与完成判定

| 步骤 | 完成条件 | 折叠行右侧的摘要 |
|---|---|---|
| 1 连接 Broker | 网关在运行且 `connected` | 「EMQX Cloud · 已连接」（服务商名由 `detectBroker(host)` 识别，认不出的写主机名） |
| 2 安装 iPhone App | 已有已配对手机，**或**用户点过「我已装好，下一步」（只存在于界面里，不落盘；重新打开设置、仍没有手机时回到这一步） | 「已安装」 |
| 3 配对手机 | 至少一台 `state = active` 的手机 | 「N 台手机」 |

- **当前步** = 第一个未完成的步骤；用户可以点任何步骤的标题展开它（手风琴，同时只展开一步）：已完成的步骤展开后仍是可用的（步骤 1 是可编辑的连接表单，步骤 2 是二维码，步骤 3 是配对）。
- **已就绪** = 步骤 1 ∧ 步骤 3 完成（步骤 2 的「已装好」不参与：有手机就一定装过）。
- 这是**纯函数**（输入：配置、网关状态、界面里的「已装好」），放在 `useIosSetup.ts`，写单测。

### 3.4 与现有卡片的对应（不丢任何能力）

| 现有 | 新的位置 |
|---|---|
| 启用开关 | 头部，不变 |
| Broker 服务商下拉 + 要点 | 步骤 1 的三个磁贴 + 编号步骤 |
| 主机、TLS 端口、WSS 端口、用户名、密码 | 步骤 1 的表单；就绪后收进「连接设置」一行，点开就是同一份表单 |
| 测试连接、按服务商的失败提示 | 步骤 1，不变 |
| 沿用另一台 Mac 的设置（导入连接串） | 步骤 1 的「从另一台 Mac 导入…」 |
| 复制连接串（导出） | 就绪后「连接设置」展开里 |
| 推送中继（公共 / 自己的）、本机显示名 | 步骤 1 与「连接设置」里的「更多设置」，不变 |
| 配对新手机、已配对手机列表（重命名、撤销） | 步骤 3 与就绪后的「已配对的手机」 |
| 推送时机 | 就绪后保留 |
| 本月流量与提醒 | 就绪后保留 |
| 允许从手机 / IM 创建 Agent 任务 | 就绪后保留（`AgentTasksSwitchRow`，同一个设置、同一个确认弹窗） |

## 4. 步骤 1：连接 Broker

![选择 Broker](../assets/ios-setup-experience/03-choose.jpg)

### 4.1 结构

引导一句 + 三个磁贴（`radiogroup`）+ 所选服务商的提示与编号步骤 + 连接表单 + 保活行 +（已连接时）「下一步」。

引导句：「Broker 在 Mac 和手机之间转发消息，选一个服务商：」（卡片简介里已经写了「需要自备 MQTT Broker」，这里不重复）。

### 4.2 磁贴

| 磁贴 | 副标题 | 选中后填入 |
|---|---|---|
| EMQX Cloud | 国内节点 · 免费额度 | TLS 8883、WSS 8084 |
| flespi | 欧洲节点 · 免费套餐 | 主机 `mqtt.flespi.io`、TLS 8883、WSS 443 |
| 自建或其它 | 如 Mosquitto | 不填 |

已填了主机的卡片（老用户、重新打开设置）按主机名后缀识别服务商并选中对应磁贴（`detectBroker`，OA11）；认不出就选「自建或其它」。

### 4.3 各服务商的编号步骤

事实只在 [`docs/wiki/ios-broker.md`](../wiki/ios-broker.md) 里维护，卡片和 Agent 指南是它的摘要；下表是**必须出现**的事实（措辞以原型截图为准：[EMQX](../assets/ios-setup-experience/04-emqx.jpg)、[flespi](../assets/ios-setup-experience/06-flespi.jpg)、[自建](../assets/ios-setup-experience/07-custom.jpg)）：

| 服务商 | 步骤要点 |
|---|---|
| EMQX Cloud | ① 打开 EMQX Cloud（链接），注册并新建 **Serverless** 部署，**消费限额保持 0**；② 把部署概览里的「连接地址」填到「主机」；③ 「访问控制 → 客户端认证」添加用户名和密码（用户名只用字母和数字）；④ 「访问控制 → 客户端授权」（默认放行所有话题，**必须**设置）：「全部用户」标签加话题 `#`、「发布 & 订阅」、**不允许**；「用户名」标签加话题 `ah/<用户名>/#`、「发布 & 订阅」、**允许**。话题可点击复制。 |
| flespi | ① 打开 flespi，注册并保持 Free 套餐；② 「Tokens」新建 token：类型 **ACL**，有效期（TTL）填最大值 8760 小时；ACL 只留一条：`mqtt` 模块，publish 与 subscribe，主题 `ah/#`；③ 记下 token 左上角 `#` 后的**编号**，把 ACL 主题改成 `ah/<编号>/#`；④ 「用户名」填编号，「密码」填完整 token（只有新建时能复制）。 |
| 自建或其它 | MQTT 3.1.1，QoS 1，保留消息和遗嘱，单条消息至少约 1 MB，用户名只含字母和数字；TLS 证书由公共 CA 签发（不接受自签），默认 8883，WSS 可选（路径 `/mqtt`，默认 8084）；每个账号只能读写 `ah/<用户名>/#`（Mosquitto：`pattern readwrite ah/%u/#`）；链接：Mosquitto 参考配置、兼容套件。 |

「打开 …」类链接用系统浏览器打开（`openUrl`），地址取 `ios-broker.md` 里写的官方入口；链接文案和地址放在 `iosBrokers.ts`（纯数据），不散落在组件里。
新增 i18n 键放在 `settings.channels.ios.*`（§10.2），原有的 `…broker.<id>.hint.*`（测试失败时按服务商的提示）原样保留。

### 4.4 额度提醒（EMQX）

![EMQX](../assets/ios-setup-experience/04-emqx.jpg)

用户 2026-11-22 的要求：EMQX 要特意提醒，充值最低 100 元，建议关注流量使用，尽量保持在免费额度内；措辞由用户在第 2 轮改定（D18）。

- **位置**：选了 EMQX 后，在磁贴下面、编号步骤**上面**（先看到再去注册）。橙色提示框。
- **文案**（zh）：「需要注意：如果免费额度（每月 100 万连接分钟、1 GB 流量）用完，最低充值金额可能需要 ¥100。建议关注流量使用，尽量保持在免费额度内。」官方文档没写最低充值额（见 `ios-broker.md`），所以用「可能」，不用作者口吻的限定。
- **常驻**：配好之后，卡片底部（流量行下面）以中性色保留同一句（[13](../assets/ios-setup-experience/13-ready.jpg)）；识别出不是 EMQX 时不显示。
- flespi 的中性提示：「需要注意：免费套餐面向测试与开发；服务器在欧洲，大陆访问可能有延迟；账号 60 天没有活动会被删除。」

### 4.5 表单与测试连接

字段、校验、保存方式不变：改动即保存（`persist`），「测试连接」用保存后的配置。成功行「已连接（tls）· 往返 142 ms · 账号已隔离」；有警告（隔离自检）时按现有的警告样式显示，**不挡**保存与配对（OA10）；失败时附按服务商的提示（认证被拒 / 登录了但没回显 / 超时，各服务商一份）。

「更多设置」折叠：推送中继（使用公共中继 / 自己的中继，含现有的说明与空地址提示）、本机显示名。

### 4.6 连上之后（D14）

[05](../assets/ios-setup-experience/05-emqx-connected.jpg)：标题行变成 ✓ + 「连接 Broker」+ 右侧「EMQX Cloud · 已连接」；服务商步骤折叠成一行「配置步骤（EMQX Cloud）」，表单仍在，「下一步」出现（保活行从选定服务商起就在表单下面）。用户点「下一步」才进步骤 2。

## 5. 步骤 2：安装 iPhone App

![TestFlight](../assets/ios-setup-experience/08-testflight.jpg)

### 5.1 内容

左边二维码（编码落地页地址，D10），右边三句：①「在 iPhone 上安装 **TestFlight**。」②「扫描二维码，在 TestFlight 里接受邀请并安装 AskHuman。」③「打开 AskHuman，允许通知。」下面一行「需要 iOS 17 或更高版本。」；按钮：「复制链接」「在浏览器中打开 ↗」「我已装好，下一步」（后者是主按钮，只改界面状态）。窄窗口（480）下右侧按钮纵向排列（[17](../assets/ios-setup-experience/17-narrow-testflight.jpg)）。

### 5.2 落地页

地址 `https://naituw.github.io/AskHuman/`（现在的 Pages 根页），由 `scripts/build-ios-beta-site.mjs` 生成，只能通过手动工作流 `ios-beta-pages` 发布。内容（中英双语，上下两段，沿用站点现在「不用脚本」的做法）：

1. 一句话说明 AskHuman Mobile 做什么；
2. 一个醒目的按钮「在 TestFlight 中加入测试」，指向 TestFlight 公共链接；
3. 三步：装 TestFlight → 加入 → 回到 Mac 的设置里配对（链到 `ios-setup`）；需要 iOS 17+；
4. 链接：设置指南、隐私政策；
5. 不放统计脚本，不收集任何信息。

**换链接的流程**：TestFlight 链接只写在一个地方（`docs/public/ios/testflight.json` 的 `url`），构建脚本读取并校验：存在但不是 `https://testflight.apple.com/join/…` 就让构建失败；**文件不存在时落地页不显示按钮，保持现在「目前仅限邀请」的文案**（所以在公共链接开通之前，Pages 随时可以重新发布，比如发布新的隐私政策，不会露出一个没有去处的按钮）。改完重新发布 Pages 即生效；**不需要发 Mac 版，也不需要发 iOS 版**。Mac 卡片从不内嵌 TestFlight 链接。

### 5.3 Apple 侧的事实与承诺

- 公共链接挂在 TestFlight **外部组**上；该组的构建要过 Beta App Review（已有组与资料见 [`ios-external-testflight.md`](../plans/ios-external-testflight.md)）。
- 公共链接可以设测试员上限，上限只挡新加入的人，不影响已加入的人；随时可以关掉链接。
- **TestFlight 构建 90 天后过期**（Apple 文档所述，本账号未实测）：公共链接要一直可用，就得**至少每 80 天出一次包**。这是个长期的维护承诺，随放出一起记入 `docs/PROGRESS.md`（§10.4）。
- 链接挂在新建的外部组「AskHuman Public」上，上限 1,000（D19）；中继成本按 `ios-open-access.md` §4 估算约每用户每月 ¥0.004，1,000 人约 ¥4。上限随时可调。
- 现在的签名 helper 只支持 Friends 组（1–5 人、公共链接关）；公共链接需要给它新增「建组 / 开关公共链接 / 设上限」这一项**有界**操作（固定到本应用与这个组），且**只在用户明确同意之后**才对 App Store Connect 执行。

## 6. 步骤 3：配对手机与「已就绪」

### 6.1 配对

![开始配对](../assets/ios-setup-experience/09-pair-start.jpg)

配对流程本身（二维码、配对码、5 分钟有效、两边 6 位确认码一致后点「允许」、被拒绝 / 过期 / 取消）**完全沿用现状**，只改摆放：

- 步骤 3 展开时，先一句手机端路径：「点「配对新手机」，在手机上打开 AskHuman →「Mac」页 →「添加 Mac」，扫描二维码；两边的确认码一致后，点「允许」。」，再是**主按钮**「配对新手机」。扫码不方便时的「复制配对码」沿用二维码块里现有的说明。
- 点了之后按钮让位给二维码（[10](../assets/ios-setup-experience/10-pairing.jpg)，二维码块自带「取消配对」）；手机扫码后按钮区变成确认码与「允许 / 拒绝」（[11](../assets/ios-setup-experience/11-pair-confirm.jpg)），这时那句手机端路径隐去。
- 手机端路径文案必须与 iOS App 里的实际界面一致；这条路径本来在 `docs/wiki/ios-setup.md` 里有，卡片里是同一句。

### 6.2 已就绪

![已就绪](../assets/ios-setup-experience/12-ready-first.jpg)

配对成功后卡片收成一个绿色「已就绪」框，两行：

- 状态行：「已就绪 · EMQX Cloud · 1 台手机在线」（0 台在线写「手机当前不在线」），右侧「查看设置步骤 / 收起设置步骤」；
- 测试行：「发一条测试问题，确认手机能收到并回答。」+ 按钮「发一条测试问题」（§9）。

框下面是管理项，顺序：已配对的手机（「配对新手机」按钮 + 列表）→ 保持这台 Mac 随时可连（§8）→ 推送时机 → 允许从手机 / IM 创建 Agent 任务 → 连接设置（折叠，一行摘要「EMQX Cloud · 主机 · 用户名」）→ 本月流量 → EMQX 提醒（识别为 EMQX 时）。除保活行和测试行外，都是现有的行，文案与行为不变。

「查看设置步骤」把三步清单重新展开在已就绪框**上面**（[14](../assets/ios-setup-experience/14-steps-reopened.jpg)），三步都是 ✓，用户可以点开任何一步重看（比如给第二台手机再看一遍 TestFlight 二维码）。

深色主题的两张见 [15](../assets/ios-setup-experience/15-dark-testflight.jpg)、[16](../assets/ios-setup-experience/16-dark-ready.jpg)，只用设计令牌，不写死颜色。

## 7. 让 Agent 帮我配置

![Agent 提示词](../assets/ios-setup-experience/02-agent.jpg)

### 7.1 卡片上的入口

两处：未启用状态的按钮，和「设置中」清单右上角的按钮；已就绪后不再显示（管理项不需要 Agent 带着走）。点击展开 / 收起一个说明框，只有两样东西：

- 一行「把下面这段发给你的 Agent：」+ 右侧「复制」（复制后显示「已复制」）；
- **完整显示**的提示词（等宽、可选择）——用户复制的东西必须是他看过的。

不写别的说明（D18）：不经 Agent 的规则不是写给用户看的，写在指南的输出里（§7.4）。

### 7.2 提示词

中文：

```
请帮我配置 AskHuman 的 iOS App 渠道。先运行：
AskHuman channel ios --agent-guide
然后按输出的步骤做。
```

英文：

```
Please set up AskHuman's iOS App channel. First run:
AskHuman channel ios --agent-guide
Then follow the steps it prints.
```

提示词随界面语言。命令放在单独一行，避免被换行折断。提示词里**不写规则**（D18）：「谁来输密码」「哪些命令只能用户运行」「要用户动手时用 AskHuman 问」都在指南的输出里，Agent 读得到，用户不必看。

### 7.3 `AskHuman channel ios --agent-guide`

- **输出**：stdout，退出码 0；语言随 `general.language`（同 `--agent-help`）。是**随版本编译进二进制**的文本，所以版本升级后说明自动更新。
- **状态感知**：读取本机状态后生成，**不含任何凭据**，也**不读钥匙串**：
  - 渠道是否启用、主机、用户名（不是机密）、识别出的服务商；
  - 密码「已填 / 未知」：网关在运行就一定有；没在运行时写「未知」，不去读它；
  - 网关状态：已连接（传输方式）/ 未运行 / 断开（`last_error`）；
  - 手机：已配对 N 台（在线 M 台）；
  - 守护进程生命周期：保活 / 按活动（是否被 Agent 任务锁定）；
  - 据此给出「下一步：第 n 步」。
  - 读状态**不应拉起守护进程**（和探测的「观察 Agent」一样用不启动的连接）；连不上就写「守护进程未运行」。
- **分节**：
  1. **角色与硬性规则**（§7.4 的规则原文）；
  2. **当前状态**（上面那块）；
  3. **步骤**：已完成的步骤一行 `✓`，当前步完整展开，后面的步骤只有标题；
  4. **各服务商要点**：EMQX / flespi / 自建，摘要同 §4.3，**EMQX 的额度与充值提示原样带上**，并要求 Agent 在用户选 EMQX 前先转告；
  5. **故障对照**：现象 → 可能原因 → 做法（认证被拒、登录了但没回显、超时、隔离警告、配对码过期、手机收不到通知，各服务商的提示沿用 `…broker.<id>.hint.*` 的事实）；
  6. **命令速查**（§7.4 的三张清单）。
- **入口**：`channel ios` 的子命令之一（`channel ios --agent-guide`，`channel ios help` 与 `channel --help` 的 ios 一节各提一行）。**不**加进全局 `--agent-help`：它在每个 Agent 会话里都要被读，不该为一个一次性的配置任务占 token。
- **门控**：放出时随其它 `channel ios` 命令一起解除（§10.1）；放出之前和它们一样在隐蔽开关后面，开发与验证在打开开关的环境里做。

### 7.4 规则与命令分级

**硬性规则**（指南第一节原文，中英文）：

1. 不索取、不读取、不打印、不记录：Broker 密码、flespi token、配对码、连接串（`ahconn1.` 开头）；不读 `~/.askhuman/` 下的文件与系统钥匙串；不要让用户把它们贴进对话。
2. 凭据只走两条路：设置里的卡片（首选），或用户**自己**在终端里执行的 `AskHuman channel set ios --password-stdin`（无回显读入；给无图形界面的 Mac，如 SSH 上去的机器）。
3. 注册账号、付款、在服务商控制台里创建账号与访问规则，由用户本人完成；Agent 可以逐步指导、核对用户读出的**非机密**项（主机、端口、用户名），不要替用户登录，不要碰付款页面。
4. 用户选 EMQX 前，先**原样**转告额度与充值提示（§4.4）。
5. 每个要用户动手的点，用 AskHuman 提问并等答复，不假设已完成。
6. 修改 `general.daemonLifecycle` 前先问（它是全局设置，开机自启）。

**命令分级**：

| 级别 | 命令 |
|---|---|
| Agent 可以直接运行 | `channel ios status [--json]`、`channel ios devices [--json]`（都不含机密，`--json` 已隐去配对码）；`channel set ios --host H --username U [--port N] [--ws-port N] [--mac-name 名称] --enable`（只含非机密字段）；`channel enable ios` / `channel disable ios`；`channel test ios`（用已存凭据；macOS 可能弹出钥匙串授权，提醒用户点允许） |
| 先问用户 | `config set general.daemonLifecycle keepalive`；`channel ios rename`、`channel ios revoke`（只在用户明确要求时） |
| **只能用户自己运行** | `channel ios pair`（打印配对码 = Broker 凭据）、`channel ios allow` / `deny`（确认码必须由人核对两台设备上的 6 位数）、`channel ios export` / `import`（连接串含密码）、`channel set ios --password-*`（密码） |

**无图形界面的 Mac**（SSH）：Agent 先做非机密部分，再让用户**在自己的终端**里：设密码（`--password-stdin`）→ `AskHuman channel ios pair`（终端里打印二维码，并带确认码核对）。

### 7.5 事实一致

卡片、Agent 指南和 `ios-broker.md` 是三份摘要，容易各自过时。办法：事实只在 `ios-broker.md` 维护；卡片文案与指南用同一组「事实键」；加一个一致性测试，检查关键数字（每月 100 万连接分钟、1 GB、¥100、8760 小时、8883 / 8084 / 443、`ah/<用户名>/#`）同时出现在 zh / en 的卡片文案和 zh / en 的指南里。

## 8. 保持 Mac 随时可连

保活行在**步骤 1**（表单下面，选定服务商后就出现，连上时用户正好看到）和**已就绪的管理项**里各出现一次，是同一个设置。

| 状态 | 文案 |
|---|---|
| 平时（开或关，两处相同） | 标题「保持这台 Mac 随时可连」；说明「开启后，后台服务常驻并开机自启，手机上不会显示这台 Mac 离线。」 |
| 被「允许从手机 / IM 创建 Agent 任务」锁定 | 开关只读，说明「已被「允许从手机 / IM 创建 Agent 任务」固定为常驻。」（沿用 `daemonLifecycleLockedByTasks` 的逻辑） |

- 它读写 `general.daemonLifecycle`（`keepalive` ↔ `activity`），与「高级」页那一行互相同步；不新增配置，不改守护进程的行为。
- **默认不替用户打开**：回答 Agent 的提问不需要常驻（Agent 一发问，守护进程就会被拉起并连上 Broker）；常驻解决的是「手机上看不到这台 Mac」和「从手机新建任务」。因为它是**全局**设置（也影响 IM 渠道），所以文案如实写「开机自启」。

## 9. 发一条测试问题

![测试问题](../assets/ios-setup-experience/13-ready.jpg)

### 9.1 行为

按钮在「已就绪」框的第二行，四种状态：

| 状态 | 文案 |
|---|---|
| 未发 | 「发一条测试问题，确认手机能收到并回答。」 + 按钮「发一条测试问题」 |
| 已发出 | 「已发送，请在手机上回答…」（按钮禁用） |
| 收到回答 | 「已收到回答，一切正常。」 + 按钮「再发一条」 |
| 超时 / 取消 / 失败 | 「120 秒内没有收到回答。请确认手机在线并已允许通知。」/「测试已取消。」/「测试问题没能发出：…」 + 按钮「再发一条」 |

发出的是手机上的一条普通提问：说明「这是 AskHuman 设置向导发来的测试提问。在这里点击「收到」，说明这个渠道既能收到提问、也能把回答送回来。」，问题「能看到这条测试提问吗？」，唯一选项「收到」。**和设置向导里各 IM 渠道的测试是同一条**（文案、选项都复用 `probe::build_task` 的渠道版本）。

### 9.2 实现：复用向导的探测，不新增 IPC（D12）

设置向导的可选检查已经是一条条普通的 `TaskRequest`：`native_request_id = "setup-probe:<target>:<nonce>"`，守护进程认出标记后按 `ProbePolicy` 处理（不弹窗、只发给指定渠道、不触发 `ask-received` 钩子、不改当前渠道），`AskRequest.setup_probe` 也已在线上（additive）。iOS 现在被刻意排除在外（有测试断言 `parse_probe_id("setup-probe:ios:…") == None`）。要做的是：

1. `ProbeTarget::parse` 接受 `ios`（不要把 `ios` 加进 `channels::ORDER`：它驱动向导的 IM 磁贴列表）；`probe_run::Check::parse("channel", Some("ios"))` 放行；
2. `daemon/runtime/mod.rs` 里直接挂载 iOS 的那段（`ios_can_deliver` 为真才挂），条件从「`probe_policy.im == ImScope::All`」扩成「`All`，或 `Only("ios")`」；`attach_im_channels` 对 `Only("ios")` 本来就返回空（`scoped_candidates` 只认 IM 渠道，iOS 不在其中），不用改；
3. `ios/gateway/requests.rs::flush_due_notifications` 现在只对「开发中继」与「确认类」提问立即通知，其余要等 Mac 空闲满「推送时机」：给 `ask.setup_probe` 加一条立即通知（否则测试要等 15 分钟，而测试的目的之一就是验证推送）；
4. 卡片按钮调用现有的 Tauri 命令 `setup_probe_start("channel", "ios")`，监听 `setup-probe-result`（超时 120 s，即 `probe_run::ASK_TIMEOUT`，每种检查同时只有一个）；窗口关闭时调 `setup_probe_cancel`。

**为什么不用升协议号**：没有新的消息变体，也没有新字段；探测由设置窗口进程自己发起，走同一套握手，后台换新机制保证它连上同构建的守护进程。唯一的过渡期风险：多安装 hold 的场景里旧守护进程不认识 `ios` 目标，会把它当普通提问（弹窗与各渠道都会收到）。极少见，写进风险（§11）。

测试问题**不进回复历史**（守护进程对探测的既有处理）。

## 10. 移出「实验」

### 10.1 清单

| 位置 | 改动 |
|---|---|
| `src/views/settings/ChannelsTab.vue`、`settings.css` | 放进渠道页最上面（D13）：在模板里紧跟「本地弹窗」卡、排在「按需发送」提示卡之前，按 DOM 顺序即可，不用 CSS `order`（IM 卡片的 `order` 是 10 起）；加 `id="channel-ios"`，深链与搜索沿用 `channel-<id>` 约定 |
| `src/views/settings/ExperimentalTab.vue`、`SettingsView.vue`、`useGeneralSettings.ts`、`GeneralTab.vue` | 不再渲染 iOS 卡；实验机制保留，空时显示「目前没有实验中的功能」（D16） |
| `src/views/settings/AwayCard.vue`（通用页「离开模式」卡） | 「离开后多久推送到手机」一行不再依赖 `experimental.enabled`：整张卡对所有人显示，Mac 专属的两行仍只在 macOS 显示；文件头注释同步 |
| `src/views/setup/RemotePage.vue` | 向导里的 iOS 磁贴对所有人显示（去掉 `showIos` 门控），「在设置中打开」改成 `openSettings("channel#channel-ios")` |
| `src-tauri/src/cli/ios_cmd.rs`、`channel_cmd.rs` | 去掉 `available()` 门控：`channel ios …`、`channel set/enable/test ios`、`channel list`、`channel help` 对所有人可用并列出 `ios` |
| `docs/specs/ios-channel.md` | D22 标「已转正（日期）」；L97 / L282 / L284 的「实验」路径改成渠道页；L284 与代码对不上的「命令始终可用」改成事实 |
| `docs/specs/ios-open-access.md` | OA12 标「已由本文接手并决定」 |
| i18n | 命名空间从 `settings.experimental.ios.*` 迁到 `settings.channels.ios.*`；旧键删除；中英同步 |
| iOS App | `OnboardingView.swift:24`、`PairingSheet.swift:80` 与字符串目录（`Localizable.xcstrings`、`make-catalog.py`）里的「设置 → 实验 → iOS App 渠道」改成「设置 → 通信渠道 → iOS App」；随下一次 TestFlight 构建发布 |

### 10.2 配置与兼容

`channels.ios.*` 本来就在 `experimental` 之外，所以**没有配置迁移**；`experimental.enabled` 保留（老配置不受影响，`config get/set experimental.enabled` 照常）。守护进程本来就不看这个开关，所以没有后台行为变化。

### 10.3 老用户

现在能看到卡片的人只有打开了隐蔽开关的内测用户（作者与首批朋友）。他们升级后卡片出现在渠道页，已填的配置原样在；「实验」页为空。**不做**专门的迁移提示（人很少，且他们本来就在接收作者的更新说明）。

### 10.4 放出顺序（D5 的「另行拍板」落在这里）

卡片一旦对所有人可见，步骤 2 的二维码就必须能用。所以顺序是：

1. 落地页改好并发布（Pages），TestFlight 公共链接开通并通过 Beta 审核；
2. 出带新路径文案的 iOS 新构建，进入公共组；
3. 最后发 Mac 新版（卡片可见、CLI 解除门控）。

任何一步没准备好，Mac 新版不出。具体放出时间由用户另行决定（D20）；每一步里碰 Apple 与 Pages 的动作都要用户逐项同意。**构建 90 天的过期承诺**（§5.3）与放出一起写进 `docs/PROGRESS.md`。

### 10.5 文档同步

`docs/public/ios/beta*.md` 改成公共链接版（去掉「邀请人发来的邮件邀请与中继地址」「打开实验性功能」两步，路径改渠道页，配置步骤改指向卡片）；`docs/wiki/ios-setup*.md` 的入口；`README` 中英各加一节介绍 iOS App 并链到落地页（D21）；`docs/overview-ios.md` 只改变了的部分；隐私政策页随 Pages 一起发布（现在只写好未发布）。

### 10.6 平台

iOS 渠道代码平台无关，但只在 macOS 验证过。非 macOS 读不到 Mac 空闲时间（`idle::secs_since_last_input` 返回 `None`），手机通知一律立即发，「推送时机」选项实际不起作用；本次**不改**（沿用现状），在 `ios-channel.md` 的平台基线里留着。

## 11. 分期、验证与风险

分期与实施步骤见 [`docs/plans/ios-setup-experience.md`](../plans/ios-setup-experience.md)：P1 卡片（前端，仍在「实验」里）→ P2 后端小改（探测、Agent 指南）→ P3 文档与落地页 → P4 iOS 文案与出包 → P5 放出（一个提交翻可见性：卡片移位、门控解除；随后发 Mac 新版）。

验证（详细见计划）：步骤判定的纯函数单测；卡片组件测试（三种形态、手风琴、磁贴、复制、保活锁定、测试问题四种结局）；Rust：探测对 iOS 的策略与立即推送、`--agent-guide` 的各状态输出（含「输出里没有任何凭据」的断言）、门控解除后的 `channel` 帮助；前后端的事实一致性测试（§7.5）；`./scripts/install.sh` 装机后在真实窗口里走一遍；最后由用户在真机上从零走完整条链路（新 Broker 账号 → TestFlight → 配对 → 测试问题）。

风险：

- **TestFlight 的长期维护**（每 80 天出包、审核被拒或测试员涌入）：在放出时一并记入 `PROGRESS.md`，用上限和随时可关的链接兜底。
- **Broker 事实过期**（服务商改价、改界面）：事实只有 `ios-broker.md` 一份并带日期，拿不准的数字（最低充值额）在卡片与指南里用「可能」；一致性测试防的是三份摘要互相不一致，不是防服务商变化。
- **Agent 越界**：靠指南规则与「命令分级」，但这是文字约束，不是技术约束；真正的防线是机密本来就不进 Agent 能读到的任何输出（`status --json` 已隐去配对码，指南输出不含凭据，由测试保证）。
- **旧守护进程遇上新探测**（§9.2）：仅多安装 hold 过渡期；表现是测试问题以普通提问出现，无数据风险。

## 12. 确认记录

- 第 1 轮（2026-11-22）：D1–D5。
- 第 2 轮（2026-11-22）：D13、D18–D21，并要求按反馈重改设计与文案。
- 第 3 轮（2026-11-22）：用户通过按第 2 轮反馈改过的设计，含派生决定 D6–D12、D14–D17 与 §14 的全部文案；允许开工，但放出时间、碰 Apple / Pages / 公共链接 / 发版的动作仍逐项另问（D20）。

## 13. 不做

- 代管 Broker、账号体系、用户配额；EMQX 等服务商的自动开户；在卡片里嵌入服务商控制台或自动化注册。
- 让 Agent 读取、保存或代填密码 / token / 配对码 / 连接串。
- App Store 上架（只做 TestFlight 公共测试）。
- 修改配对协议、IPC、配置字段、端到端加密；改「测试连接」的探测内容。
- iOS App 内置引导页的重做（只改两处路径文案）。
- 非 macOS 的推送时机适配（§10.6）。
- 把保活做成 iOS 专属设置。

## 14. 文案

D18 的原则落到字上：App 里的话，短，不写作者口吻，拿不准就用「可能」。下面是新增与改写的全部文案，其余沿用现有的键；中文与原型截图一致，English 同一语气。键放在 `settings.channels.ios.*`（§10.2），实现时以本节为准，§7.5 的一致性测试覆盖其中的数字。

### 14.1 卡片、清单与 Agent

| 位置 | 中文 | English |
|---|---|---|
| 简介 | 沿用 `descPublicRelay` | unchanged |
| 未启用的两个按钮 | 开始设置 · 让 Agent 帮我配置 | Start setup · Set up with an Agent |
| Agent 说明框 | 把下面这段发给你的 Agent： · 复制 / 已复制 | Send this to your Agent: · Copy / Copied |
| 清单标题与进度 | 设置步骤 · 2 / 3（全部完成：已完成） | Setup steps · 2 / 3 (all done: Done) |
| 步骤标题 | 连接 Broker · 安装 iPhone App · 配对手机 | Connect a broker · Install the iPhone app · Pair a phone |
| 折叠行摘要 | EMQX Cloud · 已连接 · 已安装 · 1 台手机 | EMQX Cloud · Connected · Installed · 1 phone |
| 步骤 1 的按钮 | 下一步 | Next |

### 14.2 步骤 1：Broker

| 位置 | 中文 | English |
|---|---|---|
| 引导句 | Broker 在 Mac 和手机之间转发消息，选一个服务商： | The broker relays messages between your Mac and phone. Choose a provider: |
| 磁贴 | EMQX Cloud · 国内节点 · 免费额度 / flespi · 欧洲节点 · 免费套餐 / 自建或其它 · 如 Mosquitto | EMQX Cloud · China region · free allowance / flespi · EU region · free plan / Self-hosted or other · e.g. Mosquitto |
| EMQX 提示（步骤上方；配好后卡片底部保留同一句） | 需要注意：如果免费额度（每月 100 万连接分钟、1 GB 流量）用完，最低充值金额可能需要 ¥100。建议关注流量使用，尽量保持在免费额度内。 | Note: if the free allowance (1 million connection minutes and 1 GB of traffic per month) runs out, the minimum top-up may be ¥100. Keep an eye on traffic and try to stay within the free allowance. |
| EMQX ① | 打开 EMQX Cloud ↗，注册并新建 Serverless 部署，消费限额保持 0。 | Open EMQX Cloud ↗, sign up and create a Serverless deployment. Keep the spending limit at 0. |
| EMQX ② | 把部署概览里的「连接地址」填到下面的「主机」。 | Enter the "connection address" from the deployment overview as Broker host below. |
| EMQX ③ | 「访问控制 → 客户端认证」：添加用户名和密码（用户名只用字母和数字），填到下面。 | "Access Control → Authentication": add a username and password (letters and digits only) and enter them below. |
| EMQX ④ | 「访问控制 → 客户端授权」：默认放行所有话题，必须按下面设置。<br>「全部用户」标签：话题 `#`，「发布 & 订阅」，选不允许。<br>「用户名」标签：话题 `ah/<用户名>/#`，「发布 & 订阅」，选允许。 | "Access Control → Authorization": everything is allowed by default, so you must set it up as follows.<br>"All users" tab: topic `#`, Publish & Subscribe, Deny.<br>"Username" tab: topic `ah/<username>/#`, Publish & Subscribe, Allow. |
| flespi 提示 | 需要注意：免费套餐面向测试与开发；服务器在欧洲，大陆访问可能有延迟；账号 60 天没有活动会被删除。 | Note: the free plan is meant for testing and development; the servers are in Europe, so access from mainland China may be slow; an account with no activity for 60 days is deleted. |
| flespi ① | 打开 flespi ↗，注册并保持 Free 套餐。 | Open flespi ↗, sign up and keep the Free plan. |
| flespi ② | 「Tokens」→ 新建 token：类型选 ACL，有效期（TTL）填最大值 8760 小时；ACL 只留一条：`mqtt` 模块，publish 和 subscribe，主题 `ah/#`。 | "Tokens" → create a token: type ACL, validity (TTL) at the maximum of 8760 hours; keep one ACL entry: the `mqtt` module, publish and subscribe, topic `ah/#`. |
| flespi ③ | 记下 token 左上角 `#` 后面的编号，打开这个 token，把 ACL 主题改成 `ah/<编号>/#`。 | Note the number after `#` at the top left of the token, open the token and change the ACL topic to `ah/<number>/#`. |
| flespi ④ | 「用户名」填编号，「密码」填完整的 token（只有新建时能复制）。 | Username: the number. Password: the full token (it can only be copied when it is created). |
| 自建 ① | MQTT 3.1.1，QoS 1，保留消息和遗嘱，单条消息至少约 1 MB；用户名只含字母和数字。 | MQTT 3.1.1, QoS 1, retained messages and last will, messages of at least about 1 MB; the username contains letters and digits only. |
| 自建 ② | TLS 证书由公共 CA 签发（App 不接受自签），默认端口 8883；WSS 可选，路径 `/mqtt`，默认 8084。 | The TLS certificate must come from a public CA (the app does not accept self-signed ones), default port 8883; WSS is optional, path `/mqtt`, default 8084. |
| 自建 ③ | 每个账号只能读写 `ah/<用户名>/#`（Mosquitto：`pattern readwrite ah/%u/#`）。 | Each account may only read and write `ah/<username>/#` (Mosquitto: `pattern readwrite ah/%u/#`). |
| 自建的链接 | Mosquitto 参考配置 ↗ · 兼容套件 ↗ | Mosquitto reference config ↗ · Compatibility suite ↗ |
| 导入入口 | 从另一台 Mac 导入…（点开后才出现「沿用另一台 Mac 的设置」输入框，沿用现有文案） | Import from another Mac… (the "Reuse another Mac's settings" field only appears once it is opened, existing text) |
| 保活行 | 保持这台 Mac 随时可连：开启后，后台服务常驻并开机自启，手机上不会显示这台 Mac 离线。<br>被锁定时：已被「允许从手机 / IM 创建 Agent 任务」固定为常驻。 | Keep this Mac reachable: when on, the background service stays running and starts at login, so the phone never shows this Mac as offline.<br>When locked: kept running by "Allow creating Agent tasks from phone or IM". |

### 14.3 步骤 2、3 与已就绪

| 位置 | 中文 | English |
|---|---|---|
| 步骤 2 | ① 在 iPhone 上安装 TestFlight。<br>② 扫描二维码，在 TestFlight 里接受邀请并安装 AskHuman。<br>③ 打开 AskHuman，允许通知。<br>需要 iOS 17 或更高版本。 | ① Install TestFlight on your iPhone.<br>② Scan the QR code, accept the invitation in TestFlight and install AskHuman.<br>③ Open AskHuman and allow notifications.<br>Requires iOS 17 or later. |
| 步骤 2 的按钮 | 复制链接 · 在浏览器中打开 ↗ · 我已装好，下一步 | Copy link · Open in browser ↗ · I've installed it, next |
| 步骤 3 | 点「配对新手机」，在手机上打开 AskHuman →「Mac」页 →「添加 Mac」，扫描二维码；两边的确认码一致后，点「允许」。 | Click "Pair a new phone", then on the phone open AskHuman → Macs → Add a Mac and scan the QR code. When the confirmation codes match, click "Allow". |
| 已就绪 · 状态行 | 已就绪 · EMQX Cloud · 1 台手机在线（0 台：手机当前不在线）<br>查看设置步骤 / 收起设置步骤 | Ready · EMQX Cloud · 1 phone online (none: No phone is online)<br>Show setup steps / Hide setup steps |
| 测试问题 · 未发 | 发一条测试问题，确认手机能收到并回答。 · 发一条测试问题 | Send a test question to check that your phone receives it and can answer. · Send test question |
| 测试问题 · 已发出 | 已发送，请在手机上回答… | Sent. Answer it on your phone… |
| 测试问题 · 收到回答 | 已收到回答，一切正常。 · 再发一条 | Answer received. Everything works. · Send another |
| 测试问题 · 超时 | 120 秒内没有收到回答。请确认手机在线并已允许通知。 | No answer within 120 seconds. Make sure the phone is online and notifications are allowed. |
| 测试问题 · 取消 / 失败 | 测试已取消。 / 测试问题没能发出：…（没有原因时：测试问题没能发出。） | Test cancelled. / The test question could not be sent: … (without a reason: The test question could not be sent.) |
| 推送时机说明（改写 `alertAfterHint`） | 问题一出现就会同步到手机 App。手机通知只在 Mac 键盘和鼠标空闲满这个时长后发送，每个问题最多通知一次；选「立即」则立刻通知。 | Questions reach the phone app as soon as they are asked. The phone is only notified after the Mac's keyboard and mouse have been idle for this long, at most once per question; "Immediately" notifies right away. |

### 14.4 提示词

见 §7.2（zh / en 各三行）。
