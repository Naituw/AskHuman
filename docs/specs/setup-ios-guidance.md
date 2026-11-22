# 需求：安装向导正式引导 iOS App 渠道（`AskHuman setup` 的 GUI 与终端版）

> 状态：已实现（第 3 轮）。方向由用户定下（2026-11-22：「让向导正式引导 iOS」）；第 1 轮已裁决 D1 = A、D3 = ①、D4 = A、D6 = B（先做原型再确认），D5 是用户的一句要求：「先做一版给我看效果，考虑到 iOS 配置较麻烦，你需要合理配置」；第 2 轮用户确认原型并裁决 D7 = B、D8 = A（「可以，按此实现」）。产品代码、测试与文档已按本稿落地；实现与草案的两处差别在 §3.2（磁贴怎样读写向导的配置）与 §4 末尾（终端版补的几句话、保存配置到配对之间的时序结论）。原型（`docs/assets/setup-ios-guidance/`；真实 `SetupView` + mock 后端，代码在 `src/prototype/` 的 `setup-ios.html` 一组文件，不进产品，看法见 §10）仍保留作视觉参照。
> 依赖 / 复用：[`onboarding.md`](onboarding.md)（D3、D12、D16、D20、D22、D25）、[`ios-setup-experience.md`](ios-setup-experience.md)（卡片三步、`deriveIosSetup`、「让 Agent 帮我配置」）、[`cli-config.md`](cli-config.md)（`channel set|test ios`、`channel ios pair`）。
> 平台：三端同一份界面与终端流程（iOS 渠道代码平台无关，只在 macOS 实机验证过）。
> 影响面：`src/views/setup/`（`RemotePage.vue`、`useSetup.ts`、新增 `IosTile.vue`）、`src/i18n/`、`src-tauri/src/onboarding/`（`state.rs`、`text.rs`、`channels.rs` 的注释）、弹窗页脚提示条文案；D7 选 B 时还有设置页 iOS 卡片的一个深链动作；文档 `onboarding.md`、`docs/overview.md`。
> **不改**：iOS 卡片的界面与逻辑（D7 选 B 时只多一个深链动作）；配置字段；IPC 与 `PROTOCOL_VERSION`；`channels::ORDER`（它驱动 IM 渠道探测，iOS 不进）；允许页与完成页对 iOS 的处理（已经正确）；IM 渠道的任何行为。

## 1. 背景与目标

### 1.1 改前的现状（已对照代码核实）

| 位置 | 今天的行为 |
|---|---|
| GUI 第 2 页「远程回答」 | 四个 IM 磁贴，后面跟一块 iOS App 磁贴：只有名称和「在设置页打开」（深链 `channel#channel-ios`），**没有状态、没有准备项**；页首文案只写「手机的 IM」。 |
| GUI「继续 / 跳过」 | 只看 IM 是否已启用（`hasEnabledChannel`）。只配了 iOS 的人在这里仍看到「跳过」。 |
| 终端版 `setup --text` 第 2 步 | 列表只有四个 IM（`channels::ORDER`），**没有 iOS 选项**；iOS 只在第 3 步的两行摘要、末尾汇总里出现。 |
| 弹窗页脚提示条 | 「渠道域是否已决定」不算 iOS（`state.rs::any_channel_enabled`；来自 onboarding.md D3 / D22 的「iOS v1 不做引导」，第 9 轮沿用并标了 †「留待回看」；测试 `channel_counting_excludes_popup_and_ios` 固定了它）。 |
| iOS 卡片（设置 → 渠道） | 已有三步引导和状态判定 `deriveIosSetup`：未配置 / 配置中 / 已就绪 / 异常；未启用时有两个按钮「开始设置」「让 Agent 帮我配置」。 |
| iOS 的命令行路径 | 已完整：`channel set ios`（交互向导）、`channel test ios`、`channel ios pair`（终端二维码 + 配对码 + 确认码比对）、`channel ios status`。 |

> 更正：我在上一条回答里把「iOS 不算已决定」说成「小缺口」，不准确。查 `onboarding.md` 之后：它来自 D3 / D22 的「iOS v1 不做引导」（当时 iOS 还是实验功能）；第 9 轮增补 D25 时沿用，并被标成 †（实现侧取舍、用户没有逐项评审、「留待回看」）；2026-11-22 iOS 移出实验时也没有回看。所以它既不是疏漏，也不是你明确拍板过的决定，而是一条一直在等回看的 † 项。本稿就是这次回看。

### 1.2 目标

1. 新用户跑 `AskHuman setup` 时，在第 2 步（GUI 和终端）就能看出 iOS App 是一个正式的远程渠道：它现在是什么状态、要准备什么、从哪里继续。
2. iOS 比 IM 配置麻烦（要注册 Broker 账号、装 App、扫码配对），所以向导要把这条路径摆出来，并给到捷径（指南、让 Agent 代劳），而不是只放一个按钮。
3. 配好 iOS 的人，和配好 IM 的人一样被视为「远程已决定」：步骤打勾、「继续」代替「跳过」、弹窗页脚的提示条不再出现。

## 2. 决策

| # | 问题 | 方案 | 结果 / 设计者建议 |
|---|---|---|---|
| D1 | GUI 里做多深 | **A. 富磁贴 + 跳转设置页**：磁贴显示状态与路径，按钮打开设置里的 iOS 卡片继续（卡片是唯一的编辑入口）。B. 把卡片的三步嵌进向导。 | **用户选 A。** 卡片有十余个子组件，依赖设置窗口的 context 和「整份写入」式保存；嵌进来后向导和卡片两处写同一份配置，`onboarding.md` D25 ⑨ 已经记过这类窗口竞争。 |
| D2 | 磁贴显示哪些状态 | 和卡片同一套判定（`deriveIosSetup`），七种显示，见 §3.1。 | 随 D1。向导和卡片永远不会各说各话。 |
| D3 | iOS 算不算「远程已决定」 | ①**已启用就算**（和 IM 同一规则）。②配好并至少配对一台手机才算。③维持现状不算。 | **用户选 ①。** 代价：启用了但还没配对的人，提示条会消失；磁贴（重跑向导时）和卡片仍会显示「待配对手机」。 |
| D4 | 终端版怎么引导 | **A. 完整流程**：选 iOS → 现有 `channel set ios` 向导 → 「测试连接？」→「现在配对手机？」→ 终端二维码 + 确认码。B. 精简提示项。C. 只打印一段指向设置页和指南的提示。 | **用户选 A。** 完全复用现有函数（`wizard_for_setup("ios")` 已支持 iOS）。 |
| D5 | iOS 配置麻烦，怎么「合理配置」 | 用户原话：「考虑到 iOS 配置较麻烦，你需要合理配置」。 | 设计者的理解与作答见 §3.0：磁贴把整条路径摆出来（三步 + 准备项 + 指南 + 让 Agent 代劳），随时看得到走到哪、下一步是什么。**这是设计者的理解，请确认。** |
| D6 | 实现与验证节奏 | A. 通过后直接实现。B. 先做原型截图再确认。 | **用户选 B**（推翻了设计者建议的 A）：本稿的原型即此。确认之后才改产品代码。 |
| D7 | 磁贴主按钮打开卡片后，要不要直接替用户开始（第 2 轮新增） | A. 只打开卡片；未启用时卡片里还要再点一次「开始设置」。B. 打开卡片并带一个 `start` 动作：卡片处于未启用状态时，等同于用户点了它的「开始设置」（启用渠道、展开第 1 步）；动作在设置窗口里完成。 | **用户选 B**（第 2 轮：「替我点掉：打开卡片时直接启用渠道、展开第 1 步」）。流程本来就长，少一次点击；代价是深链多一个动作、卡片多一处启动逻辑与测试。已启用的卡片不受影响。 |
| D8 | 向导里要不要提供「让 Agent 帮我配置」（第 2 轮新增；第 1 轮 §6 写过「不做」，所以要用户点头） | A. 两处都给：GUI 磁贴多一个按钮，展开与卡片相同的提示词框；终端选 iOS 后多一个选项，打印同一段提示词。B. 只给 GUI。C. 都不给。 | **用户选 A**（第 2 轮：「两处都给」）。配 iOS 最费事的部分（注册服务商账号、填 host、装 App）正适合交给 Agent；卡片里已是现成做法（[`IosAgentBox.vue`](../../src/views/settings/IosAgentBox.vue)，提示词让 Agent 先运行 `AskHuman channel ios --agent-guide`）。 |

顺序不变：iOS 仍排在四个 IM 之后，不加「推荐」徽标（onboarding.md D22 的 2026-11-22 修订）。

## 3. GUI

### 3.0 「配置较麻烦」的对策（D5）

IM 渠道是一个填表的事；iOS 要三件事：先去服务商注册 Broker 账号，再在 iPhone 上装 App，最后扫码配对。所以磁贴在 IM 磁贴的「需要什么 + 配置按钮」之上，多给四样东西：

![未配置](../assets/setup-ios-guidance/01-off.jpg)

1. **整条路径**：三步「连接 Broker / 安装 iPhone App / 配对手机」，每步一行说明；做完的步骤打勾并写结果（如「EMQX Cloud · 已连接」），当前步骤高亮。词汇和判定与设置里的卡片完全一致。
2. **准备项和指南**：没配好时，副标题写清要准备什么，行尾是「配置指南 ↗」（打开 wiki，按界面语言）。
3. **让 Agent 代劳**（D8）：「让 Agent 帮我配置」展开与卡片相同的提示词，可复制（[02](../assets/setup-ios-guidance/02-agent.jpg)）。
4. **进度和异常看得见**：橙色「待配对手机」（[03](../assets/setup-ios-guidance/03-await.jpg)）和「异常」+ 错误行（[05](../assets/setup-ios-guidance/05-issue.jpg)）说明卡在哪；配好后收成一行：手机名 + 「管理」（[04](../assets/setup-ios-guidance/04-ready.jpg)）。

不做更多：Broker 表单、服务商对照、配对二维码都在卡片里，向导不再做一份（D1）。

### 3.1 iOS 磁贴

磁贴占满两列，放在四个 IM 磁贴下面。头部一行：图标、名称、状态标签，右边是「让 Agent 帮我配置」和主按钮；名称与按钮放不下时（英文、最小窗口）按钮折到名称下面，不压缩、不重叠（[11](../assets/setup-ios-guidance/11-en-await-min.jpg)）。其下是三步条。状态由卡片的 `deriveIosSetup` 给出，再按连接与手机细分：

| 显示（标签，色） | 条件 | 三步条 | 主按钮 | 截图 |
|---|---|---|---|---|
| 未配置（灰） | `tag = unconfigured` | 第 1 步为当前；有准备项行 | 开始设置 | [01](../assets/setup-ios-guidance/01-off.jpg) [02](../assets/setup-ios-guidance/02-agent.jpg) |
| 配置不完整（橙） | `incomplete`：已启用但 host / 用户名 / 密码不全 | 同上 | 继续设置 | [09](../assets/setup-ios-guidance/09-gallery.jpg) |
| 已配置（未启用）（灰） | `configuredOff` | 第 1 步为当前 | 继续设置 | [09](../assets/setup-ios-guidance/09-gallery.jpg) |
| 异常（橙）+ 错误行 | `issue`：已启用、配置齐全、网关连不上 Broker 且带错误 | 第 1 步为当前 | 查看 | [05](../assets/setup-ios-guidance/05-issue.jpg) |
| 待配对手机（橙） | 非 `issue`，没有已配对手机，网关已连接 | 第 1 步 ✓（写「EMQX Cloud · 已连接」），第 2 步为当前 | 继续设置 | [03](../assets/setup-ios-guidance/03-await.jpg) [06](../assets/setup-ios-guidance/06-dark-await.jpg) |
| 已配置（灰） | 非 `issue`，没有手机，网关未连接且无错误（含读不到网关） | 第 1 步为当前 | 继续设置 | — |
| 已就绪 · N 台手机（绿） | 非 `issue`，至少一台手机 | 隐藏，改显示手机名 | 管理 | [04](../assets/setup-ios-guidance/04-ready.jpg) |

六种状态并排：[中文](../assets/setup-ios-guidance/09-gallery.jpg)、[English](../assets/setup-ios-guidance/12-gallery-en.jpg)。所有主按钮都打开设置里的 iOS 卡片（深链 `channel#channel-ios`）；渠道还没启用的两种状态（「未配置」「已配置（未启用）」）多带一个 `start` 动作（`channel#channel-ios#start`），设置窗口把卡片带到眼前之后，等同于用户点了它的「开始设置」（启用渠道、展开第 1 步，D7 = B）；已启用的状态只打开卡片。「让 Agent 帮我配置」在已就绪时不显示。

### 3.2 状态来源与刷新

- 配置（host、用户名、是否已启用、密码是否已存）取自已保存的配置（`getSettings`），网关状态与手机数经 `iosOp({ op: "state" })` 读取，都是卡片已在用的读取方式，不新增命令。`appInstalled`（卡片里「我已安装」这个界面事实，不存盘）恒为 `false`：配对之前第 2 步一直是当前步。
- **与草案的差别**（实现时的取舍；草案写的是「磁贴只读自己的一份快照，不写回向导的 `config`」）：读到的已保存配置只替换向导 `config` 里的 `channels.ios` 一个子树（连同「iOS 密码已存」这一个标记），其余字段一概不动。原因：D7 = B 让「向导 → 设置窗口 → 启用 iOS → 回到向导」成了主路径，而向导之后任何一次保存（例如接着配一个 IM 渠道）写的是它那份整份配置副本，副本里的 iOS 还是旧的，会把用户刚启用的 iOS 渠道静默写回关闭（`onboarding.md` D25 ⑨ 记过的同类整份写入竞争，这一块被 D7 = B 放大了）。只换 iOS 子树之后，向导里 IM 表单尚未保存的输入照样不会被覆盖（草案要保护的正是这个），iOS 也不会被写回旧值；此外页脚的「继续 / 跳过」读的也是这份子树，不必另存一份快照。IM 一侧的整份写入竞争依旧存在（改前就有，不在本稿范围）。另：`start` 动作由设置窗口里的 iOS 卡片执行，若那个窗口在向导之前就已开着、且持有旧的配置副本，卡片的保存同样是整份写入——也是这同一类竞争，本稿不处理。
- 进入第 2 页、以及窗口重新获得焦点（用户从设置窗口回来）时各读一次；渠道未启用时不问网关；连续的读取只认最新一次的结果。**不轮询。**
- 读不到网关（daemon 没运行 / 渠道未启用）时，只按配置显示，不显示手机数。读网关与设置卡片一样，会在 daemon 没运行时把它拉起来（经 `client::ios_op`）。

### 3.3 文案（新增键放在 `setup.remote.iosTile.*`；`setup.remote.ios` 已是字符串，不能再嵌套）

| 位置 | 中文 | English |
|---|---|---|
| 页首文案（改写 `setup.remote.lead`） | 离开电脑时，也能在手机上收到 Agent 的提问并回复，IM 或 iOS App 均可。选一个你常用的配置，也可以先跳过。 | When you're away, answer an Agent's question on your phone, in an IM app or the iOS app. Set up the one you use most, or skip for now. |
| 准备项（`needs` 的 items） | MQTT Broker 账号 · iPhone（iOS 17+，通过 TestFlight 安装） | an MQTT broker account · an iPhone (iOS 17+, installed via TestFlight) |
| 三步标题 | 复用卡片：连接 Broker / 安装 iPhone App / 配对手机 | Connect a broker / Install the iPhone app / Pair a phone |
| 三步说明（未完成） | 注册服务商账号并填入 / 通过 TestFlight 安装 / 扫码并核对确认码 | Create an account / Install via TestFlight / Scan, compare codes |
| 三步说明（已完成） | 复用卡片的摘要（「EMQX Cloud · 已连接」等） | 同 |
| 标签（新） | 待配对手机 / 已就绪 · {n} 台手机 | Waiting for a phone / Ready · {n} phone(s) |
| 按钮（新） | 继续设置 / 管理 / 查看；「开始设置」「让 Agent 帮我配置」「配置指南」复用卡片 | Continue setup / Manage / View |
| 跳过说明 | 不变（「之后随时在「设置 → 渠道」里添加」；iOS 卡片也在该页） | 不变 |

### 3.4 页面的其余部分

- 页脚「继续 / 跳过」的规则见 §5。
- 展开「让 Agent 帮我配置」后，提示词框在窄窗里会落到固定页脚下面；产品里展开后 `scrollIntoView`（原型要手动滚，[02](../assets/setup-ios-guidance/02-agent.jpg) 是滚到底之后的样子）。
- 窗口 640 × 720 时，中文各状态（展开提示词除外）一屏放得下；最小窗口 560 × 600 时内容区滚动，头部仍是一行（[07](../assets/setup-ios-guidance/07-min-size.jpg)）。英文更长：默认窗口下头部仍是一行（[08](../assets/setup-ios-guidance/08-en-off.jpg)、[10](../assets/setup-ios-guidance/10-en-await.jpg)，六种状态见 [12](../assets/setup-ios-guidance/12-gallery-en.jpg)），最小窗口时按钮折到名称下面、三步说明折成两行，不重叠。
- 深色主题见 [06](../assets/setup-ios-guidance/06-dark-await.jpg)。

## 4. 终端版（`setup --text`）

第 2 步在四个 IM 之后加第 5 项（iOS 不进 `channels::ORDER`，列表另加一行；状态文字沿用现有 `channel_state` 的几种，再加两种）。选了 iOS 之后，先把 GUI 磁贴里的东西（三步、准备项、指南）打出来，再问怎么配（D8）：

```
[2/3] 远程回答（可选，可跳过）
离开电脑时，也能在手机上收到 Agent 的提问并回复，IM 或 iOS App 均可。
  1) Telegram  未配置
  2) Slack     未配置
  3) 钉钉      未配置
  4) 飞书      未配置
  5) iOS App   未配置
  s) 跳过
请选择 [s]: 5
iOS App 分三步：① 连接 Broker  ② 在 iPhone 上安装 App  ③ 配对手机
需要：MQTT Broker 账号 · iPhone（iOS 17+，通过 TestFlight 安装）
指南：https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-setup.md
  1) 在这里配置
  2) 让 Agent 帮我配置（打印一段提示词，发给你的 Agent）
  b) 返回
请选择 [1]: 1
配置渠道: ios（回车保留当前值）
启用该渠道? [Y/n]  → host / port / wsPort / username / password / relayUrl / macName（现有向导，回车保留默认值）
ios 已保存
测试连接？[y/N] y
  ✓ 测试通过。
下一步：在 iPhone 上安装 AskHuman（用手机打开 https://naituw.github.io/AskHuman/，经 TestFlight 安装），再配对。
现在配对手机？[y/N] y
（终端二维码；用 AskHuman App 扫描二维码，或粘贴配对码）
「iPhone」请求配对。请确认 App 上显示的确认码一致：123456
允许这台设备？[y/N] y
配对成功：iPhone
  5) iOS App   已启用 · 1 台手机
```

选 2 时打印与 GUI 相同的提示词（「请帮我配置 AskHuman 的 iOS App 渠道。先运行：`AskHuman channel ios --agent-guide`，然后按输出的步骤做。」），并说明之后可在「设置 → 渠道」看进度，回到列表，iOS 一行不变。

- iOS 一行的状态文字：未配置 / 已配置，未启用 / 已启用，配置不完整（沿用）；**已启用，待配对手机**、**已启用 · N 台手机**（新）；读不到网关时只写「已启用」。
- 「下一步：在 iPhone 上安装…」是新增的一句：配对前要先装 App，终端里没有卡片的第 2 步提醒它。链接是卡片里同一个页面（`IOS_JOIN_URL` / `ios_guide.rs` 的 `JOIN_URL`）。
- 「配对」步骤沿用 `channel ios pair` 的现有实现，本稿不改配对协议。保存配置之后到网关起来之间若有时间差，**实现时先核实 daemon 应用配置的时序**；有时差就复用已有的显式重载机制，不加轮询或延时重试。
  - **核实结论（第 3 轮）**：配置变更经文件监视 + 300 ms 防抖才到 daemon（`daemon/config_watch.rs` → `on_config_changed` → `ensure_ios_gateway`），**没有**显式的重载 IPC 可复用；所以刚存完配置的头 300 ms 内跑 `channel ios pair`，可能读到「iOS 渠道未运行」。向导里「保存」与「配对」之间隔着至少两个由人回答的提示（测试连接？现在配对手机？），实际不会撞上；真撞上了（例如脚本喂答案）会打印这条原因并指向 `AskHuman channel ios pair` 重试，**没有**加轮询或延时重试。这是一个已知的、被时序规避而不是被机制消除的窗口，记在这里。
- 配对失败、超时或用户拒绝：打印原因并回到选择列表，与「测试失败」同样不阻塞向导。「拒绝」是 `channel ios pair` 自己打印「已拒绝」后正常返回；失败与超时是它返回的错误，向导在其下补一句 `之后可用 AskHuman channel ios pair 重试`。
- 英文版同位置各一句，措辞对应；`setup --text` 的入口与非交互环境下的行为不变。
- **实现时补的几句**（草案没写，都是同一流程里「没有它会卡住或不知道往哪走」的提示，措辞见代码与单测）：选「在这里配置」之后若配置没填全，沿用 IM 的那句「还没有配置完整；可用 `AskHuman channel set ios` 补全」，不再问测试与配对；向导里回答了「不启用」（配置存了但渠道是关的）时，仍问测试，但不问配对，改为提示 `AskHuman channel enable ios`；选择「现在配对手机？」答否时，提示之后可用 `AskHuman channel ios pair` 或在「设置 → 渠道」里操作；页首那句话与 GUI 相同（第 2 步原来的「手机 / IM」改写为「IM 或 iOS App」）。
- **读网关的方式**：列表里 iOS 一行的网关状态只在渠道已启用、配置齐全、**且 daemon 已经在运行**时才问（与 `channel list` 同样「不为了列一行字把 daemon 拉起来」）；否则按配置写，不带手机数。手机数只算 `active` 的。英文里 iOS 一行与摘要、第 3 步沿用已有的写法「iOS app」。
- **保存后的那句提示**（验收时发现、用户裁决）：`channel set` 向导保存后原本固定打印「提示：可运行 `channel detect` 自动识别用户 id……」，对 iOS 没有意义（`detect` 只支持钉钉、飞书、Slack），在这条流程里还正好夹在「ios 已保存」和「测试连接？」之间。现在只对 iOS 不再打印（`channel_cmd.rs::saved_tip`），其他渠道不动；所以单独运行 `AskHuman channel set ios` 也不再有这句。

## 5. 「远程已决定」与弹窗提示条（D3 的落点）

- `state.rs::any_channel_enabled` 改为 `any_remote_channel_enabled`：任一 IM 渠道或 iOS 渠道已启用（弹窗不算；注释同步改）；`derive` 的 `remote` 随之变化：`remote = 任一远程渠道已启用 || skipped || 旧提示已关`。
- GUI：页脚「继续 / 跳过」改为「有 IM 渠道已启用，或 iOS 渠道已启用」（`hasRemoteChannel`）。向导里的配置副本在用户去设置窗口配完 iOS 回来时还是旧的，所以 iOS 这一半读的是随聚焦刷新的 `channels.ios` 子树（§3.2）；真正离开页面时仍走现有的 `leaveRemote()`（重读已保存的配置，D25 ⑨）。只配了 iOS 的人从此看到「继续」，并且不再被记为 `remote` 已跳过。异常状态（[05](../assets/setup-ios-guidance/05-issue.jpg)）同样是「继续」：和 IM 一样，已启用就算，不看是否真能收发。
- 终端版：结束第 2 步时，`mark_skipped(REMOTE)` 的条件改为「没有任何渠道（含 iOS）已启用」。
- 弹窗页脚提示条文案：

| | 中文 | English |
|---|---|---|
| 现在 | 配置 IM 渠道后，离开电脑也能在手机上收到并回复提问。 | Set up an IM channel to receive and answer prompts on your phone when away. |
| 改为 | 配置 IM 渠道或 iOS App 后，离开电脑也能在手机上收到并回复提问。 | Set up an IM channel or the iOS app to receive and answer prompts on your phone when away. |

## 6. 不做什么

- 不把 iOS 卡片嵌进向导；不在向导里新写 Broker 表单、配对界面。
- 「让 Agent 帮我配置」只提供提示词，向导和 AskHuman 都不替 Agent 执行任何配置。
- 不新增 IPC、配置字段、命令；不改 `PROTOCOL_VERSION`。
- 不给磁贴加轮询或延时重试；刷新只在进入页面和窗口重新聚焦时发生。
- 不动允许页（iOS 已排第一、未启用时置灰）与完成页（已启用才显示 iOS 芯片）。

## 7. 影响面与测试

**前端**：`IosTile.vue`（新；复用 `deriveIosSetup`、`stepStatus`、`detectBroker`、`IosAgentBox.vue`，样式词汇同原型）、`RemotePage.vue`（替换现有 iOS 块；页脚按 §5）、`useSetup.ts`（iOS 快照、聚焦刷新）、`i18n/zh.ts` 与 `en.ts`（新键、改写 `lead` 与提示条）。D7 选 B 时：`open_settings` 的深链多一个动作 token，`IosChannelCard.vue` 在未启用时响应它。`channelDefs.ts` 不动：iOS 仍不在 `CHANNEL_DEFS` 里，`channelDef("ios")` 仍为 `undefined`（现有测试保持）。

**Rust**：`state.rs`（`any_remote_channel_enabled` 与 `derive`）、`text.rs`（第 2 步的 iOS 行与流程；`TextDeps` 增加 `ios_row`、`pair_ios` 两个入口——配置与测试复用已经认识 iOS 的 `configure_channel` / `test_channel`——测试用假实现）、`ios_guide.rs`（`JOIN_URL`、指南地址 `setup_guide_url`、Agent 提示词 `agent_prompt` 以 `pub(crate)` 开放给终端流程，并有单测把提示词与指南地址钉在设置卡片 / `iosLinks.ts` 的同一份文字上）、`channels.rs` 头部注释（「iOS is not part of the wizard」已经过时）。

**测试**
- `state.rs`：原 `channel_counting_excludes_popup_and_ios` 改为「iOS 已启用计入、弹窗不计入」；`derive` 随之的用例。
- 前端：iOS 磁贴的七种显示、按钮、深链（及 D7 的动作）；Agent 提示词展开 / 收起；已就绪时「继续」代替「跳过」；聚焦时重读快照、且不改写 IM 表单的未保存输入；`zh` / `en` 键对齐。
- 终端：选 iOS → 配置 → 测试 → 配对（成功、拒绝、超时、跳过配对）；选「让 Agent 帮我配置」只打印不改配置；iOS 已启用时第 2 步不记为「跳过」；列表里 iOS 一行的各种状态文字。
- 提示条：iOS 已启用时不出现（`hint.rs`）。

**验证**：先 `node scripts/machine-health.mjs`，再 `./scripts/install.sh`；在隔离环境（临时 HOME / Dev 实例）里跑真实的向导窗口和 `AskHuman setup --text`，逐一核对磁贴各状态与终端流程；窗口取 640 × 720 与 560 × 600、中英文、浅深色，对照 `docs/assets/setup-ios-guidance/`。不抬协议号，无需更新恢复存储。

**验证结果（第 3 轮，装机的二进制，一次性 `HOME` + `ASKHUMAN_HOME` + `ASKHUMAN_NO_KEYCHAIN=1`，用后清理）**
- 真实向导窗口（640 × 720）：「未配置」「配置不完整」「已配置（未启用）」「异常」（Broker 拒绝连接）「待配对手机」（连上回环 Mosquitto）五种显示，加 Agent 提示词展开；英文深色一张（待配对手机 + 提示词）。版式、文案与原型截图一致。进入第 2 页时的读取、同一窗口里语言与主题随配置实时切换都正常。
- 「开始设置」（D7 = B）：点磁贴主按钮后，设置窗口落在「通信渠道」页、滚到 iOS 卡片，渠道开关已打开、「设置步骤 1 / 3」且第 1 步已展开；回到向导后磁贴变为「配置不完整 · 继续设置」，页脚由「跳过」变为「继续」。
- 终端 `AskHuman setup --text`（中文、英文各一遍，跑到第 3 步入口为止）：第 2 步列表第 5 行；选 5 后三步说明、需要、指南地址、两个选项；`b` 返回、选 2 打印提示词且不改配置；选 1 走完 `channel set ios` 向导 → 保存 → 「测试连接？」（失败时打印原因并继续）→ 「下一步：在 iPhone 上安装……」→ 「现在配对手机？」选否 → 之后配对的提示行，列表里 iOS 一行变为「已启用」（daemon 没运行，读不到网关，按设计只写「已启用」）。
- 没有在真实环境验证的：「已就绪」（有已配对手机；没有手机，也不再测配对流程，由前端单测与原型截图覆盖）；560 × 600 的真实窗口（自动化里调不了窗口尺寸，原型 07、11 覆盖）；窗口重新获得焦点时的重读（真实窗口里只验证了进入页面时的读取，聚焦重读由 `SetupView.test.ts` 覆盖）。

## 8. 文档同步

- `onboarding.md`：D3（「iOS 渠道不做配置引导」）、D16（渠道域「任一渠道已启用」的口径）、D22（iOS 磁贴，及「仍不算渠道域已决定」）、D25 ②（「只有 iOS 渠道时是『跳过』」）与 ⑦（iOS 不计入已决定）、非目标一节「不做 iOS 渠道的配置引导」、第 9 轮日志里标 † 的那一条（注明已在本稿回看），以及新增一轮的轮次记录。
- `docs/overview.md`：「新用户从 `AskHuman setup` 起步」那一条补一句 iOS App 也在第 2 步。
- 不改：`ios-setup-experience.md`（卡片本身不变）、`docs/wiki/ios-setup*.md`（可选：「需要准备」前加一句从 `AskHuman setup` 进入，不属于本稿）。

## 9. 轮次记录

- **第 1 轮**：用户选定方向「让向导正式引导 iOS：GUI 磁贴显示状态与准备项、终端版加 iOS 选项、页首文案提 iOS App」，先出方案、不动代码。方案里的问题经用户裁决：D1 = A、D3 = ①、D4 = A、D6 = B；D5 用户回复「你先做一版给我看效果，考虑到 iOS 配置较麻烦，你需要合理配置」。
- **第 2 轮**：按 D6 = B 做了原型并截图（十二张）；按对 D5 的理解把磁贴做成「整条路径」（§3.0）；原型过程中发现并修正了头部一行在英文下重叠的版式问题（改为放不下就折行）；新增两个待裁决项 D7、D8（D8 改动了第 1 轮 §6 的「不做」）。状态映射比第 1 轮细：「待配对手机」只在网关已连接时出现，其余读不到的情况归「已配置」。用户的回答：「可以，按此实现（含下面两题的选择）」；D8 = 两处都给；D7 = B（替我点掉：打开卡片时直接启用渠道、展开第 1 步）。
- **第 3 轮（本稿，实现）**：按第 2 轮确认的设计与文案落地：GUI 的 `IosTile`（七种显示、三步条、Agent 提示词、指南链接、按钮与 `start` 动作）、`useSetup` 的聚焦刷新、页脚按 iOS 是否启用、设置页深链的 `#start` 动作；终端版第 5 项与完整流程；`any_remote_channel_enabled`、弹窗提示条文案；单测与前端测试；`onboarding.md` 与 `overview.md` 的同步。实现时与草案不一致的地方都写在正文里：§3.2（只替换 `channels.ios` 子树，而不是「不写回」）、§4（时序核实结论；补的几句话；读网关的方式）；`TextDeps` 新增的入口比草案少两个。验收时用户另行裁决：去掉 `channel set ios` 保存后那句不适用的 `channel detect` 提示（只对 iOS，见 §4）。

## 10. 原型怎么看

`pnpm dev`（端口 5180）后打开 `http://localhost:5180/prototype/setup-ios.html?scene=<场景>`：真实的 `SetupView` 配 mock 后端，只有 iOS 磁贴是原型版（`IosTileProto.vue`，按钮不动作）。

- `scene`：`off`（未配置）、`agent`（展开提示词）、`incomplete`、`stopped`、`connecting`、`await`、`ready`、`ready-two`、`issue`、`im-ready`，或 `all`（六种状态并排的图库）。
- 其他参数：`theme=dark`、`lang=en`、`w=560&h=600`（窗口尺寸）。
- 场景数据在 `src/prototype/setupIosScenes.ts`（假 Broker 与手机名，不含真实凭据）。
