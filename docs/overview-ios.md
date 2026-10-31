# iOS 渠道概览

> 本文是 `docs/overview.md` 的专题补充，记录 iOS 渠道（实验功能）当前的实现地图与不变量；需求决策见 `docs/specs/ios-channel.md`，实现步骤见 `docs/plans/ios-channel.md`，用户使用说明见 `docs/wiki/ios-setup.md`。

## 形态

```
Agent / CLI ─► Daemon ── IosGateway（一个 actor）──► MQTT（TLS 8883 / WSS 8084）──► EMQX Serverless ──► iOS App
                                │                                                                  ▲
                                └────── 推送中继（无状态云函数，持有 APNs 密钥）──► APNs ───────────┘
```

- iOS 是 Daemon 里的又一个渠道（`channels/ios.rs`），受 `experimental.enabled` 隐藏；一个 Mac 一份 broker 凭据，多台 Mac 共用同一份（连接串可导入导出），手机经二维码配对。
- **端到端加密**：每个配对一对方向密钥（ChaCha20-Poly1305），broker 只见密文；配对用一次性票据 + 6 位校验码，由 Mac 用户在设置页或 CLI 上放行。推送预览同样加密，由手机上的通知扩展解密。
- **Mac 不持有 APNs 密钥**：Mac 把加密后的信封和手机的不透明句柄交给中继，中继按固定白名单拼出提醒并转给 APNs。
- 手机能做的事：回答问题 / 确认（含多题、推荐、附件）、看 Agent 列表（App 里没有手动关注：手机新建任务或原会话接续的新一轮出结果时 Mac 会通知，列表行与详情页只读地标出）、实时观察（Watch，≤2 路、仅前台）、给 Agent 插话、新建 Agent 任务（Face ID 确认）、管理待办（「待办」标签：来源是某台 Mac 或「iCloud」，浏览项目、增 / 改 / 勾 / 删 / 排序、⚡ 自动执行、看执行历史；`docs/specs/ios-todos.md`）。
- **投递与通知分开**（`docs/specs/ios-alert-timing.md`）：iOS 不参与 IM 的活跃槽，`attach_im_channels` / `confirm_im_candidates` 在选完 IM 候选后用 `with_ios` 把它并进来，每个提问 / 确认都挂给所有已配对手机，手机一连上就从 hello 快照（`request.get`）拉到。手机在看（在线 + 前台 + 已 hello）时经 MQTT 实时送达、不推送；不在看时只有系统推送能打扰人，而它要等 Mac 键盘 / 鼠标空闲满 `channels.ios.alertAfterSecs`（默认 900 秒，`0` = 立即）才发，每题每手机最多一条，没有提醒。


生命周期结果与原会话控制见 [统一结果与接续](specs/agent-lifecycle-results-and-recovery.md)：
`AgentView.lastTurnResult` 独立于空闲／运行／结束状态；手机增加「需要处理」与两个接续入口。
`daemon/runtime/agent_control.rs` 将原生新回合证明关联到手机；`PhoneRecord.followTurns` 将通知绑定该轮，
`lifecycleResults` 能力声明避免旧手机收到不认识的未完成推送。用户中断静默，结果随 `agents.json` 保留。

## Mac 端（`src-tauri/src/`）

| 位置 | 职责 |
|---|---|
| `ios/crypto.rs` · `envelope.rs` · `wire.rs` | 密钥派生、信封、全部线协议消息与限额常量；与 Swift `AHProtocol` 共用 `ios/Fixtures/wire-v1/` 夹具，两边测试必须同时通过 |
| `ios/transport.rs` | `rumqttc` 传输（TLS 优先、WSS 备用、退避重连）、`LoopbackTransport`（测试：丢包 / 乱序 / 断线注入）、`probe`（设置页「测试连接」）。明文 TCP 只对回环地址生效（`first_via`） |
| `ios/store.rs` | `~/.askhuman/ios/` 下的 `state.json`（手机、关注集、推送句柄、本月流量）与 `journal.json`（状态变更消息的幂等日志）；配对密钥在系统钥匙串，缺钥匙串时回退 `keys.json` 并在设置页提示 |
| `ios/views.rs` | 把请求投影成手机渲染的视图（选项原文、TODO、确认变体） |
| `ios/todos_view.rs` | 待办「项目列表」的纯函数：候选 + 同步元数据 → 带不透明 id、同名后缀、分段的行（与桌面端侧栏的 `mergeProjects` / `labelProjects` 同一算法，两边各有镜像用例）；只有哈希 id 与最少的上层目录名上线，路径不出 Mac |
| `ios/connstr.rs` · `qr.rs` | `ahconn1.` 连接串（含 broker 密码，只在显式请求时产出）、配对二维码 SVG |
| `ios/push.rs` | 中继客户端（`/v1/push`），只传信封与句柄；`check_health`（`/v1/health`，设置页 / `channel test ios` 的中继检查） |
| `ios/gateway/` | Gateway actor：`mod.rs`（事件循环、在线状态、hello 快照）· `pairing.rs`（配对状态机）· `requests.rs`（问答 / 确认闭环、答案校验、按 Mac 空闲时长触发推送）· `agents.rs`（Agent 列表、Follow = 通知集：App 里没有手动入口，手机新建任务及手机原会话消息真正启动的新一轮会进来；通知集里的 Agent 第一次「完成／失败／未完成」时：手机在看发应用内横幅，否则排队等 Mac 空闲满 D 才推，你回到 Mac 就作废，出结果后移出通知集）· `watch.rs`（实时帧）· `tasks.rs`（新任务、状态机 accepted→launching→launched→watching…）· `assets.rs`（附件双向分块）· `todos.rs`（待办：项目列表 / 单个项目 / 七种写入 / 订阅与变更通知，`docs/specs/ios-todos.md`）· `hostapi.rs`（Gateway 向 Daemon 提问的 `Host` trait）；`testkit.rs` + `tests*.rs` 是进程内的完整测试台 |
| `daemon/runtime/ios.rs` · `ios_agents.rs` · `ios_tasks.rs` · `ios_todos.rs` | `Host` 的实现：配置变化时重启 Gateway、读 registry / Watch / 插话、启动任务、读写 `todos.json`（`ios_todos.rs`：与待办窗口 / CLI 同一把锁、同一套 `todos.rs` 函数）；`mac_idle_secs()` 与 `alert_after_secs()` 把空闲时长与配置接给推送判断 |
| `idle.rs` | `secs_since_last_input()`：macOS 用 CoreGraphics `CGEventSourceSecondsSinceLastEventType`（合并会话状态、任意输入事件，无需任何权限）；其它平台返回 `None`（Gateway 把「读不到」当作「够久了」，照旧立即推） |
| `daemon/runtime/task_launch.rs` | IM `/new` 与 iOS 共用的任务启动（门槛、冲突、终端 / Codex App 路径） |
| `channels/ios.rs` | `Channel` 实现：请求交给 Gateway，首个终态获胜后撤销手机上的请求 |
| `cli/ios_cmd.rs` | `AskHuman channel ios status / pair / allow / devices / revoke …` 与 `channel set ios …`（密码经 stdin / 文件 / 环境变量，从不进 argv） |
| `commands.rs` | 设置页用的 Tauri 命令：`ios_op`（经 IPC 的 `ClientMsg::Ios`）、`ios_test`、`ios_pair_qr`、`ios_connection_import` / `export` |

前端：`src/views/settings/IosChannelCard.vue` + `useIosChannel.ts`（状态轮询、配对流程）、`iosTraffic.ts`（本月流量与免费额度预警）、`iosAlertTiming.ts`（「推送时机」下拉的预设与时长显示）。

## 推送中继（`relay/`）

零依赖 Node.js（`http` / `http2` / `crypto`），为腾讯云云函数 Web 函数而写：`src/core.js`（句柄编解码、请求校验、载荷白名单、ES256 JWT、错误映射、令牌桶）与 `src/adapters/scf.js`（HTTP 服务、环境变量、HTTP/2 到 APNs）。接口 `GET /v1/health`、`POST /v1/register`、`POST /v1/push`。密钥只在云函数环境变量里（`APNS_KEY_P8` 等），仓库与日志里没有任何密钥。`scripts/build-zip.sh` 产出上传用的 zip，`npm test` 跑 `node --test`。

## iOS App（`ios/`）

- 工程用 XcodeGen（`ios/project.yml`），`*.xcodeproj` 不入库；新增文件后 `xcodegen generate`。iOS 17+，SwiftUI，Swift 5 语言模式 + 严格并发。Team `DMJXDB9H6Q`，Bundle ID `com.naituw.humaninloop.ios`，通知扩展 `…ios.notification`，共享钥匙串组 `…ios.shared`。
- 业务全部在本地 Swift Package `ios/Packages/AskHumanKit`，UI 只是它的薄壳：

  | Target | 内容 |
  |---|---|
  | `AHProtocol` | 与 Rust 对应的线协议、加解密、信封（通知扩展也链接它，所以保持极小） |
  | `AHStore` | 文件型 JSON 存储（历史、草稿、发件箱、流量）、钥匙串 `Vault`、`PushDecryptor`（通知扩展用） |
  | `AHTransport` | `BrokerClient` 抽象、`MQTTBrokerClient`（CocoaMQTT）、进程内 `LoopbackBroker` |
  | `AHCore` | `PhoneEngine`（actor：连接、配对、同步、发件箱重试、推送登记、Agents / Watch / 插话 / 任务 / 待办请求）、`TodosStore`（待办标签的状态：来源与 iCloud 接入 Mac、多台 Mac 开着同步时合并成 iCloud 一个来源、项目行上取自 Agents 页状态的点、「Mac 最后的回答 + 用户未被应答的写入」叠加出屏幕上的列表、写入队列）与 `AppStore`（`@MainActor @Observable`，UI 的唯一入口）及各 Composer |
  | `AHMarkdown` | Markdown 块解析，供原生渲染（题干、列表摘要、Watch 卡片、附件预览，以及网页渲染不可用时的回落）；请求正文平时由网页渲染（见下） |
  | `AHMock` | `MockBackend`：演示数据与可编程故障，UI 测试和截图都跑在它上面 |

- 新建任务的 `TaskComposer` 在点击「开始」时，将模型 / 强度按 Agent 存为这部手机的本地偏好，所有 Mac / 工作区共用；取消不更新。
  打开表单或切换 Mac / Agent 时恢复，缺少已保存模型的 Mac 临时回默认并保留原偏好。完整约定见 `docs/specs/ios-task-model.md` M3。
- Agent 消息对 Codex Desktop 走原生 `Operation::Send`（运行中补充／空闲时继续，继承线程设置）；其余走原有工作中 Hook 队列。
  Desktop 断开绝不回退队列；Gateway 后台发送并去重，成功只代表 App 接受。手机对不明结果保留消息 ID 与草稿，经用户检查原聊天后才能解除。见 `ios-channel.md` §6.7。
- Codex 新任务可逐次开启 Fast（桌面 App／CLI 均支持），默认关闭且关闭明确传标准速度；速度不保存，模型／强度的偏好规则不变。
  能力取对应 Codex 目录的逐模型声明，旧 Mac 隐藏开关；进度显示所选速度，详情仅显示会话有证据的速度配置。见 `docs/specs/ios-task-speed.md`。
- 回复、插话与新任务共用 `Views/Attachments/ReplyAttachments.swift` 的附件菜单：相册允许照片 / 视频混选，视频通过文件型 `Transferable` 读取。
  `AHCore/VideoAttachmentPreparer.swift` 保留限额内原视频，超限时先预估可用压缩档位，只有预估 ≤8 MB 才尝试；编码一次后核对实际体积与完整时长。回答区、插话栏与新任务表单直接显示压缩进度及取消按钮，由用户决定是否继续等待。
  导出器的状态与取消约束在 MainActor，原文件 / 导出临时目录按生命周期清理；三个 Composer 用准备任务的 token 集合阻止处理期间发送。验证办法见 `docs/plans/ios-video-attachments.md`。
- 新任务的手机附件最多 5 个、单个 10 MiB，必须有文字或待办，全部上传成功才创建。`TaskComposer` 持有本机临时文件与进度，
  `upload.begin.scope.taskId` 与 `task.create.files` 将上传绑定至手机和任务；Mac 在共享启动管线合并待办附件。能力缺失时不允许带文件启动；规格见 `docs/specs/ios-task-attachments.md`。
- 通知扩展 `AskHumanNotification` 只解密预览（`NotificationText`），不连网络。点击通知经 `AppStore.openNotification` 路由：提问类打开请求（请求还没到手机时页面用通知原文占位、转圈等待，只有 Mac 已答复且没有它才说「不可用」），`follow.*` 打开 Agent 页。
- **请求正文由网页渲染，其余 Markdown 仍是原生**：提问页的 Message、确认页 / 历史详情的正文用 `WebMarkdownView`（`Views/Markdown/`）——一个 `WKWebView` 加载随 App 打包的 `Resources/WebMarkdown`（`project.yml` 里 `type: folder` 的文件夹引用；与 Mac 弹窗同一套 markdown-it + Mermaid，版本以根 `package.json` 为准，升级后跑 `ios/Tools/web-markdown/sync.sh` 重新拷贝并写 `VERSIONS.txt`）。一页至多一个（每个 `WKWebView` 各占一个 WebContent 进程）；页面无特权、禁网（CSP）、锁死导航，只有 `ready / height / rendered / copy / failed` 五个消息能回到 App，链接交给 `LinkPresenter.tapped`（http(s) 先确认域名，`mailto` / `tel` 交系统，其余忽略）。题干、列表卡片、Watch 卡片、附件预览仍走 `AHMarkdown` 解析 + `MarkdownView` 原生渲染（软换行换行、行内代码有底、可选字）。页面进程被回收超过 2 次 / 加载失败 / 5 秒（`-AHWebTimeout`；模拟器默认 30）没有高度会自动回落原生；Message 排好版之前多题表单不统计「看过」（`ReadingTracker.begin(awaitingMessage:patience:)` 等 `onSettled`，最多等看门狗时间 + 1 秒）。设计、限额与取舍见 `docs/specs/ios-web-markdown.md`。
- 本地化：`Localizable.xcstrings`（en + zh-Hans）由 `ios/Tools/make-catalog.py` 从App 与通知扩展编译器产出的 `.stringsdata` 生成——先 `xcodebuild build -derivedDataPath build/DerivedData …`，再跑脚本；缺中文翻译会让脚本失败。
- Mock 模式启动参数：`-AHMock YES -AHScenario demo|agentResults|empty|offline|quota|syncing|single|longs|markdown|strict|tall|outdated|bothSync -AHLatency <x> -AHTab inbox|agents|todos|history|macs -AHOpen <id> -AHAutoAdvance YES|NO -AHOpenAgent <key> -AHNewTask YES -AHHoldInterjects YES -AHFollowEvent done|failed|waiting -AHFollowAgent <agentKey 或会话 id>`；`-AHFollowAgent` 只在真实后端下生效（给模拟器真服务场景用）：等该会话出现在列表里就经引擎 `setFollow` 关注它——App 里已没有手动入口，场景又需要一个「会通知你」的 Agent；`syncing` 场景让首次同步迟迟不完成，配 `-AHSyncDelay <秒>`（多久后完成）与 `-AHCached YES`（同步前就带着上次缓存的请求）；`single` 只剩一条短提问（答完回到空收件箱），`longs` 是两条长表单（测换页后新页从顶部开始）；`markdown` 只有一条 Markdown 展示请求（`req-markdown`：软换行、行内样式、内联图片、长单元格表格、Mermaid 流程图、代码、引用、嵌套列表），`strict` 只有一条要滚动才看得完的三题纯选择表单（`req-strict`，测「发送」的提示与跳转），`tall` 是同一份很高的 Markdown Message 配两道很短的纯选择题（`req-tall`，测 Message 排版前不统计「看过」）；`-AHWebMarkdown NO` 让所有 Markdown 正文走原生渲染（等同网页渲染不可用时的回落路径），`-AHWebTimeout <秒>` 改网页渲染的看门狗时间（默认 5，模拟器 30：冷启动的模拟器第一次起 WebContent 要十来秒；网页的 UI 测试传 60，其余 UI 测试用 `-AHWebMarkdown NO`，因为网页晚到会把下面的版面整体推走）；`-AHAdvanceHold <秒>` / `-AHAdvanceSettle <秒>` 拉长回执与换页的时间，方便截图 / 录屏；`-AHNotify <请求 id>` 模拟点开该请求的推送（页面立刻打开，不等同步，带一段虚构的通知文字）；`-AHArrival <会话标题|none>` 启动约 2 秒后来一条新提问（应用内横幅），`none` 表示该会话没有标题。
- XCUITest 经验：**`WKWebView` 里的内容在无障碍树里是一串平铺的普通元素**（StaticText / Link / Image / Button，Link 的标签是链接文字，**没有 WebView 节点**，`app.webViews` 永远找不到），所以测网页渲染要靠只有网页才有的标志（复制按钮标签「Copy」、Mermaid 图是带图中文字的 `Image`），原生渲染的对应物是「Copy code」与源码文字；链接文字折行时 Link 的 frame 是整块包围盒，点它的中心可能点到别的链接，测试挑单行的那个。容器 view 上的 `accessibilityIdentifier` 需要 `.accessibilityElement(children: .contain/.combine)` 才找得到子元素；Form / List 的懒加载行需要滚动到附近才存在；浮动 TabBar 会挡住详情页底部，详情页用 `.toolbar(.hidden, for: .tabBar)`。挂在容器上的 `accessibilityIdentifier` 会盖住子元素自己的标识，要查子元素就把标识挂在叶子视图上；判断「被盖住」用 `isHittable`（被 `.accessibilityHidden` 的元素仍然 `exists`）。推送横幅是 springboard 里的 `BannerNotification`（标识 `NotificationShortLookView`），权限弹窗按钮是 `springboard.alerts.buttons["Allow"]`；合并后的 `push-status` 行标签形如 `Notifications, On`，判断用 `hasSuffix`。 `AskHumanUITests` 里按序号点底部标签的 `tab(_:)`：收件箱 0、Agents 1、待办 2、历史 3、Mac 4（加标签时要回头改这里的序号）。待办的 UI 测试在 iOS 18.5 的模拟器上跑 `testAddingFinishingEditingAndRestoringATodo` 会卡住：拖动待办页把键盘收起（`dismissKeyboard()`）之后，每一个动作都要等满 60 秒的「App animations complete notification not received」才继续，整个类跑上 20 分钟也不结束，而此时 App 空闲（CPU 约 0、主线程停在 run loop）；同一份构建在 iOS 27.0 的模拟器上约 1.5 分钟通过。所以跑待办的 UI 测试用 iOS 27.0 的模拟器；这只是在一个环境里的观察，没有查到 18.5 上的根因。

## 验证

| 范围 | 命令 |
|---|---|
| Rust（含 Gateway 测试台与跨语言夹具） | `cargo test --manifest-path src-tauri/Cargo.toml ios::` |
| Swift 包（单元 + 进程内集成） | `cd ios/Packages/AskHumanKit && swift test` |
| 中继 | `cd relay && npm test` |
| 设置页 | `npx vitest run src/views/settings` |
| App UI（模拟器） | `xcodebuild test -project ios/AskHuman.xcodeproj -scheme AskHuman -destination 'platform=iOS Simulator,id=<UDID>' -derivedDataPath ios/build/DerivedData CODE_SIGNING_ALLOWED=NO -parallel-testing-enabled NO -collect-test-diagnostics never`（`TEST_RUNNER_AH_UI_LANG=zh-Hans` 切中文，`TEST_RUNNER_AH_UI_SHOTS=<dir>` 保存截图）。必须前台跑；后台会被工具杀掉 |
| 真实 daemon 端到端（本地 broker） | `cd ios/Packages/AskHumanKit && AH_E2E_BIN=<已安装的 AskHuman> AH_E2E_BROKER_DIR=<ios/Tools/local-broker> swift test --filter AHE2ETests`（缺环境变量时自动跳过） |
| 真实 daemon 端到端（真实 EMQX + 中继） | 同上，把 `AH_E2E_BROKER_DIR` 换成 `AH_E2E_REAL_FROM=<已配置好 ios 渠道的实例的 ASKHUMAN_HOME>` 与 `AH_E2E_BUNDLE_ID=<App 的 Bundle ID>`（中继只收这个 ID）；多出 3 项只在真实服务上跑的检查（真中继 + 真 APNs 沙盒、延迟目标） |
| 真实服务 + 模拟器上的真 App | 先 `xcodegen generate`，在 `ios/` 下 `xcodebuild build-for-testing -project AskHuman.xcodeproj -scheme AskHuman -destination 'platform=iOS Simulator,id=<UDID>' -derivedDataPath build/DerivedDataLive -allowProvisioningUpdates`（要签名：推送需要真 token）；再在 `ios/Packages/AskHumanKit` 下 `AH_E2E_BIN=… AH_E2E_REAL_FROM=… AH_E2E_BUNDLE_ID=… AH_E2E_SIM_UDID=<UDID> swift test --filter SimulatorLiveTests`（缺 `AH_E2E_SIM_UDID` 或构建产物时跳过）。要一台专用模拟器，必须前台跑，约 2 分钟，机器忙时成倍 |

CI（`.github/workflows/build.yml`）只跑上表的「Swift 包」与「中继」两行，各占一个 job（`ios-kit`：`macos-26` 上 `swift test`；`relay`：`npm test`），与四平台构建矩阵并行；不跑 `xcodebuild`（App 本体与 UI 测试要模拟器）、真实 daemon 端到端和真实服务场景（端到端缺环境变量会自动跳过）。Rust 侧的 `ios::` 测试已包含在矩阵里的 `cargo test`。

端到端测试台会起**一台一次性的 Mac**：独立 `ASKHUMAN_HOME`、`HOME`、`ASKHUMAN_NO_KEYCHAIN=1`，加本地 Node broker；`agentTasks.enabled` 保持关闭，所以不会真的启动任何终端。伪造的 Agent 会话通过隐藏命令 `AskHuman __agent-hook <agent> <event>` 注入。

真实服务模式不起本地 broker：一次性 Mac 用 `channel ios export` 取出实例的连接串（只在测试进程的内存里，经 stdin 交给 `channel set ios --connection-stdin`，不显示、不落盘），手机侧是 macOS 上真正的 `PhoneEngine` + `MQTTBrokerClient`。每个用例的 Mac / 手机 ID 都是新生成的，话题互不重叠（共用同一个 EMQX 账号），但会消耗真实 EMQX 的流量与中继调用。`AH_E2E_REPORT=<json>` 写入延迟统计，`AH_E2E_KEEP_LOGS=<目录>` 让失败的用例留下 Mac 的 daemon 日志与手机侧摘要（都不含秘密）。

模拟器场景（`SimulatorLiveTests` + `AskHumanLiveUITests`）把一次性 Mac 留在宿主进程里，手机侧是 `xcodebuild test-without-building` 启动的 XCUITest，两边用 `ui/` `host/` 目录里的「会合点」文件互相等待。它走完：配对与校验码 → 通知权限 → token → 中继登记 → Mac 持有句柄 → 前台单题 / 多题 → 权限确认（允许；带理由拒绝）→ 一个 `__agent-hook` 会话（App 以 `-AHFollowAgent` 启动，所以这个会话「会通知你」）：列表、实时观看、详情页的那行说明与列表行上的铃铛（没有关注按钮）、插话与读取回执 → 任务被关闭时新任务表单的提示 → App 离开前台 → 一次真 APNs 沙盒推送（模拟器收得到，扩展会运行；`simctl push` 不会运行扩展）→ 点击横幅 → 该 Agent 第一次结束回合的推送（一次性：之后会话移出通知集）→ 点击打开 Agent 页 → 历史 → 解除配对。一次性 Mac 的 `alertAfterSecs` 播种为 `0`（真实服务模式的 `RealBroker.settings()`），推送不依赖这台机器此刻是否空闲。配对码含 broker 密码，只经命名管道从宿主交给 UI 测试（不落文件、不进环境变量 / 剪贴板，码在屏幕上时不截图），`xcresult` 结束时删除，并在日志与模拟器键盘日志里扫描配对码。模拟器默认只卸载 App + `simctl keychain reset` + 等 CPU 静下来；`AH_E2E_SIM_ERASE=1` 才抹掉设备（首次开机要占满一个核好几分钟）。`AH_E2E_SHOTS=<目录>` 保留截图。

## 不变量与注意点

- **请求详情不能把重连误报为 Mac 离线**：作答／确认页共用 `EngineState.requestConnection(macId:)` 与 `RequestConnectionNotice`，先判断手机连接阶段，再判断当前请求的目标 Mac。连接／重试中显示中性进度；仅在手机已连通且目标 `MacLink.macOffline` 时显示 Mac 离线。连接恢复保留页面和草稿，发送继续受就绪状态约束；见 `docs/specs/ios-answer-flow.md` §3.6。

- **新请求的前台动效**：`AppStore.requestArrivals` 保留逐请求事件，状态／事件乱序时等请求进入 inbox 后各自释放，不生成旧的大横幅；打开、处理、失活时清理，动画完成不改未读。Agent 完成等其他横幅保留。
  `Views/Arrival/ArrivalSceneHost.swift` 通过根视图的零尺寸 attachment 绑定真实 `UIWindowScene`，以不可成为 key、`hitTest` 恒为空且禁用交互的透明装饰窗口覆盖 App 页面／弹层。它不操作真实输入框；只有 Debug + mock 专属组合态测试 fixture 会准备 marked text。
  `ArrivalLayout.swift` 从真实底栏、导航栏、安全区和键盘测量锚点；底栏可用飞收件箱，否则落顶部短暂提示，移动锚点平滑跟随。`AHCore/ArrivalMotion.swift` 与 Mac 共用 `ios/Fixtures/arrival-motion.json` 数值参考，使用原轨迹；SVG／PDF 位于 `ios/Tools/arrival-motion/` 与 assets。减少动态效果只显示静态提示，后台停止 display link。规格见 `docs/specs/ios-arrival-motion.md`。

- **隔离**：任何隔离运行的 Mac / daemon 必须设置 `ASKHUMAN_HOME`（只改 `HOME` 时 daemon 标签虽已独立，但 GUI Host 登录项与 Agent hook 文件仍会碰到真实环境）；测试台在启动前断言它位于临时目录。
- **IPC 协议号**：当前为 4，由 `ClientMsg::Ios` / `ServerMsg::Ios` 带来。给 `IosOp` / `IosResult` 加带 `#[serde(default)]` 的字段不 +1；规则见 `docs/overview.md` 的「IPC 协议号」。
- **线协议**：`ios/wire.rs` 与 `AHProtocol` 的限额常量（信封 ≤ 900,000 B、分块 192 KB、附件 ≤ 10 MB、每次回复 ≤ 5 个附件、令牌桶 6/s 突发 10、Follow ≤ 5、任务正文 ≤ 3000 字、插话 ≤ 20,000 字、Watch 间隔 ≥ 2 s 且每手机 ≤ 2 路）两边必须一致；改任何一边先改夹具。
- **幂等**：状态变更消息（回答、插话、`task.create` 等）按 `msgId` 进幂等日志，重放返回同一结果、同 ID 不同内容被拒绝。
- **「活着」的手机**：Gateway 只向「在线 + 前台 + 已 hello」的手机实时投递。App 在首次同步之后才发布前台状态，所以 hello 与前台状态之间的变化（新请求、Agent、Follow）不会被实时发出；`Actor::on_presence` 在手机由此变为「活着」时补发四个快照。改动这一段先看 `tests_requests` / `tests_agents` 里 `…between_hello_and_the_foreground_presence…` 两个用例。
- **「待通知」是推导出来的，不存标志位**：一道题对某部手机是否还在等推送 = `!seen && !pushed && !(is_live && connected) && can_push`，由 `flush_due_notifications`（`deliver()` 末尾与每 250 ms 的 `tick_requests`）判断；手机上下线、broker 抖动、`request.seen` 都不必逐个维护状态，推过一次 `pushed` 置位后条件不再成立，所以天然没有提醒。3 秒补推（前台手机没确认）不受等待时长约束；读不到空闲时长按「够久了」处理；`alert_after_secs` 不在 `ConnKey` 里，改设置不重连网关。改这一段先看 `tests_requests.rs` 的「push timing」一节。
- **Follow 的「做完 / 出错」走同一条等待规则，但是显式排队**（`docs/specs/ios-follow-narrowing.md`）：结果没有长期存在的对象可推导，所以 `announce` 把它排进 `Live.follow_notices`（内存，每部手机每个 Agent 至多一条），由 `flush_follow_notices`（`announce` 末尾与每 250 ms 的 `tick`）判断：Agent 又开始工作 / 会话结束 → 作废并继续关注；手机变成在看、或结果之后 Mac 有了新输入 → 作废；空闲满 D（或 `D = 0`、读不到空闲）→ 推一次。除「Agent 又开始工作」外，推出或作废都会把该 Agent 移出关注集（一次性），所以重启 daemon 会丢掉还没推出的通知。改这一段先看 `tests_agents.rs` 里 “A result (done / failed)” 那一节。
- **App 没有关注入口，但 `setFollow` / `FollowSet` / `FollowState` / `FollowEvent` / `Wire.maxFollow` 都留着**：旧版 Mac 与旧版 App 还在用它们（旧版 App 的星标连新版 Mac 照常能加，只是同样一次性、同样遵守 D），`-AHFollowAgent` 也靠 `setFollow`。把它们当死代码清掉之前，先看 `docs/specs/ios-follow-narrowing.md` 的「兼容」。App 只读地展示 `AgentView.followed`：列表行铃铛、详情页 `follow-note`；Mac 拒绝（名额满）时静默，不弹提示。
- **推送信封没有 `v`**：中继转给 APNs 的 `ah` 只有 `{f,k,n,c}`（Mac 封装时的 `v` 不转发），`PushDecryptor` 缺 `v` 时按本协议处理、带了别的版本则拒绝。改推送载荷或中继的白名单时，同步改 `PushDecryptorTests` 里「中继的形状」，否则单元测试与真实推送会各说各话。
- **通知代理方法必须在主 actor 上**：`AppDelegate` 的 `UNUserNotificationCenterDelegate` 方法是 `@MainActor`（一致性标 `@preconcurrency`）；写成 `nonisolated async` 时系统的完成回调落在协作线程池，iOS 27 上点推送横幅会崩溃。
- **每次握手都把推送句柄再给 Mac**：`PhoneEngine` 在每次 `hello.ack` 清空 `pushSent` 并重发，发送失败要回滚标记，token 在 ack 与首个快照之间到达也立即发；Mac 只会回 `push.state` 的 `ok` / `gone`，不会反过来索要句柄。
- **手机自己能从「握手卡住 / 连接半死」里恢复，不靠 Mac**（`docs/specs/ios-alert-timing.md` D12）：`hello` 一轮 3 次（各 5 秒）没人应后不放弃，按 `helloRetryDelays`（30 / 60 / 120 秒封顶）继续问；`hello.ack` 认本会话最近 8 条 `hello`（`MacRuntime.helloNonces`）中的任何一条，`resetSession` 清空名单，所以旧会话的随机数没用，握手完成后晚到的 ack 也被忽略；下拉刷新对没握手的 Mac 重新握手（`restartHello`），不再只发会被忽略的 `Resync`。App 在前台收到收件箱里没有的请求的推送时（`AppDelegate.willPresent` → `AppStore.noteUnknownRequest`，`resolved` 与 `follow.*` 不算）走 `PhoneEngine.checkLink`：向手机**自己的下行主题**发一个随机令牌，**只有这条回声**算连接健康（PUBACK 不证明下行，Mac 的流量不证明上行）；`probeTimeout`（4 秒）内没回来或发布直接失败就 `reconnectNow()`（不等退避），回来了就重新握手或 `Resync("inbox")`；同一台 Mac 10 秒内只处理一次。探测发布不能 `await` 在看门狗之前——`MQTTBrokerClient.publish` 最多等 10 秒 PUBACK。改这一段先看 `SessionTests` 里 hello 的重试 / 迟到确认用例与 `ConnectionTests` 的 “link check” 一节（`LoopbackClient.dropsInbound` 模拟看着活着却什么都收不到的连接）；`AHE2ETests/LinkCheckE2ETests` 在真实 daemon + 真实 MQTT 客户端 + 本地 broker 上证明回声走得通（自发自收经 CocoaMQTT 与 Aedes 都成立；EMQX 的自发自收见 P0 记录），以及「聋」连接（`ObservedBroker.goDeaf()`）会被换掉、漏掉的请求随后补上。
- **离开 / 返回前台的竞态**：`setForeground(false)` 在 `goodbye()` 处挂起，goodbye 之后若应在前台必须重连（`ConnectionTests.testComingBackWhileTheGoodbyeIsOnItsWayStillReconnects`）。`scenePhase == .inactive`（控制中心、通知中心、多任务界面、系统弹窗——App 仍在屏幕上）**不算离开**：`PhoneEngine.setInactive()` 给 `inactiveGrace`（10 s）的宽限期，超过才告别，期间回到前台则取消计时、保留连接；`.background`（锁屏、回主屏）仍立即告别；`AskHumanApp` 只做 `scenePhase` → `setInactive` / `setForeground` 的映射（`ConnectionTests` 里 5 个 inactive 用例，每处保护去掉都会变红）。强退没有任何 `scenePhase` 回调，靠 Last Will 兜底。真机上只验证了「停留超过 10 s 时离线窗口远短于停留」，短停留不离线、锁屏 / 回主屏与强退后 Mac 多快看到离线都**没在真机上验证**（见 P0 记录 §3.8）。
- **`AppStore` 的状态流与事件流没有先后**：`AppStore` 在两个独立任务里分别消费 `states` 与 `events`，同一次变化的两者谁先到不确定。`AHAppTests` 里不要假设顺序，每个条件各自用 `eventually` 等。模拟「别处已作答」用 `MockBackend.simulateFinishedElsewhere`（结束事件带 `by`，页面保留请求并显示提示）；不要用自己提交的回答代替——它的结束事件没有 `by`，App 会前进到下一个请求并忘掉提示，结果取决于哪条流先到（曾因此在 CI 上偶发失败三个用例）。
- **iOS 26 的顶部滚动边缘效果**：别用 `safeAreaInset(edge: .top)` 在列表上方固定自绘的栏——它会落进导航栏下的模糊带，让导航栏高到约三倍（曾经的筛选条）。需要的控件放进 toolbar（筛选是 `Menu`），状态文字放 `.principal` 的标题位。
- **「没有数据」与「还没加载」是两回事**：`EngineState.listReadiness`（`ready` / `loading` / `unreachable`）与 `syncStatus`（`upToDate` / `updating` / `reconnecting` / `refused`）由 `EngineState`（`AHCore/Models.swift`）从连接阶段与各 Mac 的 `link` 推导，`MacLink.connecting` 是唯一的「还不知道」状态。UI 只有在 `ready` 时才显示「已全部处理」类的空状态，`loading` 显示缓存或骨架行（15 秒仍无应答才显示「等待 Mac 响应」）；同步提示放标题位且延迟约 0.6 秒才出现，只有需要用户处理的问题（Broker 拒绝、时钟偏差、流量、Mac 离线）才在列表顶部出现提示卡。新增列表页时沿用这套，别自己写 `isEmpty → 空状态`。
- **多题表单的「发送」要等看过最后一题，而「下一题」只看屏幕位置**：`AnswerComposer` 记录每题是否「看过」（题卡底边进入过可视区，已作答的算看过；一旦看过最后一题就不再收回），多题时 `hasReachedEnd` 之前底部是「下一题」，之后是「发送」（屏幕下方还有题时，「发送」旁多一个「下一题」图标按钮；纯选择题表单没选全时，计数下的提示可点，跳到第一道没选的题）；与弹窗的 V1 / V2（`docs/specs/multi-question-vertical.md`）一致，改一边要同步改另一边。「下一题」落到哪张卡由 `QuestionNavigator`（AHCore，纯函数）按题卡与可视区的**当前**位置算——第一张底边还在可视区下方的卡，放得下就整张露出，放不下先对齐顶部、再按一次看底部——**不看「看过」的历史**，所以往回滑后再点不会跳过看过但没答的题（同弹窗的 `goNext`）。选项行点按即时选中，不做渐变（`OptionRow`）。规格与弹窗对照见 `docs/specs/ios-answer-flow.md`。
- **提问页 / 确认页的底栏：iOS 26 起是浮动的 Liquid Glass 栏，之前是经典栏**（`Views/Common/BottomBar.swift`，规格 `docs/specs/ios-answer-flow.md` §3.4）：`.actionBar { … }` 在 iOS 26+ 用 `scrollEdgeEffectStyle(.soft, for: .bottom)` + `safeAreaBar`——栏自己**没有底板也没有分隔线**，内容滚到它后面，可读性靠系统的柔和渐隐（`.soft` 要显式写，自动样式在有的系统版本上是带硬边的色带）；iOS 17–25 回退到 `safeAreaInset` + `classicBarBacking()`（`.bar` 材质 + 分隔线）。栏里不是控件的文字（计数、提示、上传进度、确认页的状态行）一律过 `barStatusChip()` 放进玻璃小块——直接放在渐隐上，滚过来的内容会和它叠在一起读不清；**一行里的计数块与按钮同高**（行 `fixedSize(horizontal: false, vertical: true)`，两边的标签 `maxHeight: .infinity`，行高由最高的那块定；只在浮动栏、非辅助字号下），所以提示（它会折成两三行）不放进计数块，单独成行上方的一小条（`barStatusChip(compact: true)`），行的高度不随提示变——左右两块高度不一样会很难看，用户反馈过；**可点的小块（会跳去未答题的提示）要把玻璃包在按钮外面、内边距留在标签里**（`barChipButtonLabel()` + `barChipButtonGlass()`）：玻璃放进按钮的标签里，iOS 26.2 上点它没有反应（27.0 上有）；按钮用 `barPrimaryButton()` / `barSecondaryButton()`（iOS 26 起 `.glassProminent` / `.glass`，之前 `borderedProminent` / `bordered`）；一栏里的玻璃放进同一个 `GlassGroup`（`GlassEffectContainer`），栏内各块的间距要大于它的 `spacing`，否则相邻的玻璃会融成一块。**「下一题 → 发送」是同一个 `Button`**（`SendBar.primaryButton`，只换文字与标识 `next-button` ↔ `send-button`）：换成两个按钮时，玻璃样式下换身份的那几帧会闪出一个空胶囊。新加底栏沿用 `.actionBar`，不要自己写 `safeAreaInset` + `.background(.bar)`；带输入框的栏见下一条。
- **输入栏（Agent 页的插话栏 `InterjectBar`、待办页的添加栏 `TodoAddBar`）同样是浮动玻璃栏**（规格 `docs/specs/ios-answer-flow.md` §3.4 G11）：同一套 `.actionBar` + `GlassGroup(spacing: 4)`，经典栏（iOS 17–25）一点没变，输入栏本来就没有分隔线，所以用 `classicBarBacking(hairline: false)`。**一排里的块同高**（`BottomBar.rowHeight` = 44 pt）：输入框 `barFieldBox(focus:)`（玻璃胶囊，折行后变圆角矩形，向上长高、圆钮留在底部；**整个框都是点按区域**——SwiftUI 的 `TextField` 只有那一行字能点，框比它高，空白处点了不弹键盘，所以框自己接点按手势去设 `@FocusState`，要把焦点传进来）；附件 / 自动执行是 `barRoundButton()`（系统的 `.glass` 圆钮），图标过 `barRoundGlyph()`——固定 17 pt、外框 30 pt，**圆钮的直径来自系统给玻璃按钮的内边距（`.regular` 每边 7 pt，`.large` 每边 15 pt）加上标签，不是自己画的**，所以标签框要按「44 − 14」算；发送是 `BarSendButton`（着色玻璃圆钮，发送中转圈）；附件列表与上传进度合成一块 `barPanel()`；字数与「不能发」的说明是 `barStatusChip(compact: true)` 的小条。iOS 26.2 上系统玻璃按钮在水平方向两侧各多出约 5 pt 的布局空隙（27.0 没有），圆钮离屏幕边是 21 pt 而不是 16 pt，没去补。
- **详情的手动快切与自动前进是两条路径**（`docs/specs/ios-request-switching.md`）：`AppStore.requestNavigation` 捕获首次列表入栈时的筛选与展开顺序，显式 `open`／通知和自动前进入口用全部范围；`RequestNavigation` 只追加新 ID，以结束项作位置锚点并跳过它们。`RequestScreen` 共用右侧箭头组与最右更多菜单，手动替换唯一详情页、提交组合文字并收键盘。`requestNavigationEpoch` 随页面变化递增，发送记录代次，切走后即使再回来也不被旧回执推进；原 `nextPending` 的全收件箱选择不变。
- **阅读和附件准备归请求拥有**：`RequestScrollPosition` 观察原生滚动几何，用 `RequestReadingAnchor` 记录题卡／正文锚点与相对偏移，Markdown 就绪后恢复；每请求位置保存在 AppStore 的非观察缓存，用户新滚动会结束恢复。它不替换 scroll delegate、不设置输入焦点。`AnswerComposer` 按题保留 `AttachmentPreparationState`，请求换页不取消准备，其他 AttachmentMenu 用法仍在离开时取消；结束草稿会取消其准备。发送中的原表单不可编辑，其他请求不受影响；失败说明按请求 ID 保存，不在别条表单上弹错。
- **提交后的换页是一个小状态机**（`AppStore.advance` → `moveOn`）：收到 Mac 的确认后先设 `receipt`（`RequestScreen` 盖上全页「已发送」回执；它带着 `summary`——请求一关闭页面就没有详情了，回执自己记着是谁问的）；`AdvanceTiming.hold` 之后栈**一步**从 `[a]` 换成 `[b]`（栈里始终只有一页），回执记下 `next = b`，并出「已发送 · 还剩 N 条」提示条；位移由页面自己做（`RequestScreen.swift` 里的 `Arrival`，只有 `next` 指向它的页面才接手回执：回执的一份副本留在原地并略向左让，新页从右边滑进来盖住它，约 0.45 秒；新页的版面第一次排好后才开始动，否则长表单首次排版的卡顿会吃掉动画；「减少动态效果」下改成淡入）；`receipt` 一直留到 `settle`（0.8 秒）之后才清，回到列表时被弹出的那页也还在用它。**不用系统的推入**：它要把已发送页留在栈里、推完再拿掉，而 `NavigationStack` 按位置认页面，拿掉一页等于把第 0 位那页「改成」下一条——那页最后一帧（回执）会再闪两帧，它的滚动位置与 `@State`（阅读进度，决定底栏写「下一题」还是「发送」）也被沿用；而原地替换栈顶（`[a] → [b]`）在 iOS 27.0 上只有交叉淡入、在 18.5 上没有动画，两边都没有位移，所以位移放在页面里，并且每条请求一个独立页面（`RequestScreen` 上 `.id(id)`）。回执期间用户自己离开页面则 `cancelAdvance()`。UI 测试用 `AdvanceTiming.immediate` / `-AHAdvanceHold`；看这个动画要用不带 XCUITest 的 `simctl io recordVideo`（XCUITest 的无障碍快照会让过渡画面出现重影，长表单上一次查询要几秒，期间应用主线程被占住、动画被吞掉；录屏也是变帧率，只有帧的先后顺序可信）；模拟器场景 `single`（只剩一条）与 `longs`（两条长表单）就是为这条流程准备的。
- **回答与创建任务的多行输入**共用 `Views/Common/CaretTextView.swift`：原生输入保留选区与中文组合态，在输入、选区、布局或键盘变化后只滚动露出光标所需的距离，视口扣除键盘和操作栏；行数上限后框内滚动。原生 UI 测试只在 Debug mock 的 `-AHInputCaretDiagnostics YES` 下读取真实光标几何，规格见 `docs/specs/ios-answer-flow.md` §3.5。
- **空状态要按「TabBar 总在」居中**：请求页与 Agent 详情页隐藏 TabBar，返回时列表先出来、TabBar 约 0.5 秒后才回来，底部安全区增高约 48 pt，居中的空状态会上跳约 24 pt。列表页的空状态 / 骨架提示一律经 `.emptyState { … }`（`Views/Common/ListChrome.swift`，记住这个宽度下见过的最大底部边距）摆放，不要直接 `.overlay { ContentUnavailableView }`。
- **请求附带的图片：缩略图是 1:1 方块，查看器一条消息一个**（`Views/Attachments/`）：`AssetList` 里图片缩略图的格子大小由一块 `Color` + `.aspectRatio(1, contentMode: .fit)` 决定，图片放进它的 `overlay` 里 `scaledToFill` 再裁切；不要把 `scaledToFill` 的图片直接放进 `.frame(height:)`——它的布局宽度按图片比例撑大，宽图会溢出网格格子、叠到邻格上。点任一张图都打开 `ImageGalleryViewer`（这条消息的全部图片，左右滑动；非图片文件仍由 `AssetViewer` 单独打开），它用 iOS 17 的分页 `ScrollView`（非惰性 `HStack` + `scrollTargetLayout` + `.paging` + `scrollPosition(id:)`），不用 `TabView(.page)`（UI 测试里第一次滑动常被吞，疑似邻页刚被创建）；`scrollPosition(id:)` 的初值不会把非惰性的栈滚到那一页，所以开页时另用 `ScrollViewReader.scrollTo`。只有当前页与左右相邻页会下载并打开图片（`AssetPage(near:)`）。规格与取舍见 `docs/specs/ios-channel.md` §15（2026-10-29）；模拟器场景 `gallery` 与 `testPictureThumbnails…` / `testThePictureViewerPages…` 等四个 UI 用例守着它。
- **待办只是 Mac 的遥控与展示**（`docs/specs/ios-todos.md`）：权威状态仍是 Mac 的 `todos.json`；线上项目只有不透明 id（key 的 SHA-256 前 8 字节十六进制）与最少的上层目录名，没有路径；手机不把待办内容落盘（只在内存里缓存），也不进推送；写操作按待办 id 定位、带 `clientMsgId` 进幂等日志，应答附带该项目最新的整表，所以手机屏幕上的列表 = Mac 最后一次的回答 + 用户还没被应答的写入。「iCloud」只是一个**来源**（由一台开着同步且在线的 Mac 代答，掉线自动换），手机不直连 CloudKit（那要用户为 iOS App ID 开 iCloud 容器，见该 spec §11）。配对了多台 Mac 时，开着同步的 Mac 并进「iCloud」、不再单独出现在来源菜单里；它们各自「仅本机」的项目在 iCloud 视图里按 Mac 分段，点进去在那台 Mac 上读写。项目行上的 Agent 点不是 Mac 算的：手机拿 Agents 页已有的实时状态，按 Agent 项目信息里可选的 `repo`（仓库地址，Mac 开着同步时才带）或同一台 Mac 上的目录哈希对到行，所以跨 Mac 的点要那台 Mac 也装新版。项目列表的合并 / 分段 / 同名后缀在 Rust（`ios/todos_view.rs`）里有一份镜像桌面端 TS 的实现，改一边先看另一边的镜像用例；`todo.watch on` 会立刻回一条 `todo.changed` 作为基线。
- **成本红线**：EMQX 消费上限保持 0（额度用尽即停，不自动付费）；不创建管理 API Key；Mac 与手机本机统计流量，80% 免费额度时预警。
- **保密**：broker 密码、配对码、方向密钥不得出现在日志、命令输出、文档与提交里；`E2ETests.testSecretsNeverReachTheLogsOrTheCommandOutput` 守着这条。
