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
- 手机能做的事：回答问题 / 确认（含多题、推荐、附件）、看 Agent 列表、关注（Follow，≤5）、实时观察（Watch，≤2 路、仅前台）、给 Agent 插话、新建 Agent 任务（Face ID 确认）。
- **投递与通知分开**（`docs/specs/ios-alert-timing.md`）：iOS 不参与 IM 的活跃槽，`attach_im_channels` / `confirm_im_candidates` 在选完 IM 候选后用 `with_ios` 把它并进来，每个提问 / 确认都挂给所有已配对手机，手机一连上就从 hello 快照（`request.get`）拉到。手机在看（在线 + 前台 + 已 hello）时经 MQTT 实时送达、不推送；不在看时只有系统推送能打扰人，而它要等 Mac 键盘 / 鼠标空闲满 `channels.ios.alertAfterSecs`（默认 900 秒，`0` = 立即）才发，每题每手机最多一条，没有提醒。

## Mac 端（`src-tauri/src/`）

| 位置 | 职责 |
|---|---|
| `ios/crypto.rs` · `envelope.rs` · `wire.rs` | 密钥派生、信封、全部线协议消息与限额常量；与 Swift `AHProtocol` 共用 `ios/Fixtures/wire-v1/` 夹具，两边测试必须同时通过 |
| `ios/transport.rs` | `rumqttc` 传输（TLS 优先、WSS 备用、退避重连）、`LoopbackTransport`（测试：丢包 / 乱序 / 断线注入）、`probe`（设置页「测试连接」）。明文 TCP 只对回环地址生效（`first_via`） |
| `ios/store.rs` | `~/.askhuman/ios/` 下的 `state.json`（手机、关注集、推送句柄、本月流量）与 `journal.json`（状态变更消息的幂等日志）；配对密钥在系统钥匙串，缺钥匙串时回退 `keys.json` 并在设置页提示 |
| `ios/views.rs` | 把请求投影成手机渲染的视图（选项原文、TODO、确认变体） |
| `ios/connstr.rs` · `qr.rs` | `ahconn1.` 连接串（含 broker 密码，只在显式请求时产出）、配对二维码 SVG |
| `ios/push.rs` | 中继客户端（`/v1/push`），只传信封与句柄；`check_health`（`/v1/health`，设置页 / `channel test ios` 的中继检查） |
| `ios/gateway/` | Gateway actor：`mod.rs`（事件循环、在线状态、hello 快照）· `pairing.rs`（配对状态机）· `requests.rs`（问答 / 确认闭环、答案校验、按 Mac 空闲时长触发推送）· `agents.rs`（Agent 列表、Follow；被关注 Agent 第一次「做完 / 出错」时：手机在看发应用内横幅，否则排队等 Mac 空闲满 D 才推，你回到 Mac 就作废，出结果后移出关注集）· `watch.rs`（实时帧）· `tasks.rs`（新任务、状态机 accepted→launching→launched→watching…）· `assets.rs`（附件双向分块）· `hostapi.rs`（Gateway 向 Daemon 提问的 `Host` trait）；`testkit.rs` + `tests*.rs` 是进程内的完整测试台 |
| `daemon/runtime/ios.rs` · `ios_agents.rs` · `ios_tasks.rs` | `Host` 的实现：配置变化时重启 Gateway、读 registry / Watch / 插话、启动任务；`mac_idle_secs()` 与 `alert_after_secs()` 把空闲时长与配置接给推送判断 |
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
  | `AHCore` | `PhoneEngine`（actor：连接、配对、同步、发件箱重试、推送登记、Agents / Watch / 插话 / 任务）与 `AppStore`（`@MainActor @Observable`，UI 的唯一入口）及各 Composer |
  | `AHMarkdown` | 请求正文的 Markdown 块解析 |
  | `AHMock` | `MockBackend`：演示数据与可编程故障，UI 测试和截图都跑在它上面 |

- 通知扩展 `AskHumanNotification` 只解密预览（`NotificationText`），不连网络。点击通知经 `AppStore.openNotification` 路由：提问类打开请求，`follow.*` 打开 Agent 页。
- 本地化：`Localizable.xcstrings`（en + zh-Hans）由 `ios/Tools/make-catalog.py` 从编译器产出的 `.stringsdata` 生成——先 `xcodebuild build -derivedDataPath build/DerivedData …`，再跑脚本；缺中文翻译会让脚本失败。
- Mock 模式启动参数：`-AHMock YES -AHScenario demo|empty|offline|quota|syncing -AHLatency <x> -AHTab inbox|agents|history|macs -AHOpen <id> -AHAutoAdvance YES|NO -AHOpenAgent <key> -AHNewTask YES -AHHoldInterjects YES -AHFollowEvent done|failed|waiting`；`syncing` 场景让首次同步迟迟不完成，配 `-AHSyncDelay <秒>`（多久后完成）与 `-AHCached YES`（同步前就带着上次缓存的请求）；`-AHAdvanceHold <秒>` / `-AHAdvanceSettle <秒>` 拉长回执与换页的时间，方便截图 / 录屏。
- XCUITest 经验：容器 view 上的 `accessibilityIdentifier` 需要 `.accessibilityElement(children: .contain/.combine)` 才找得到子元素；Form / List 的懒加载行需要滚动到附近才存在；浮动 TabBar 会挡住详情页底部，详情页用 `.toolbar(.hidden, for: .tabBar)`。挂在容器上的 `accessibilityIdentifier` 会盖住子元素自己的标识，要查子元素就把标识挂在叶子视图上；判断「被盖住」用 `isHittable`（被 `.accessibilityHidden` 的元素仍然 `exists`）。推送横幅是 springboard 里的 `BannerNotification`（标识 `NotificationShortLookView`），权限弹窗按钮是 `springboard.alerts.buttons["Allow"]`；合并后的 `push-status` 行标签形如 `Notifications, On`，判断用 `hasSuffix`。

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

模拟器场景（`SimulatorLiveTests` + `AskHumanLiveUITests`）把一次性 Mac 留在宿主进程里，手机侧是 `xcodebuild test-without-building` 启动的 XCUITest，两边用 `ui/` `host/` 目录里的「会合点」文件互相等待。它走完：配对与校验码 → 通知权限 → token → 中继登记 → Mac 持有句柄 → 前台单题 / 多题 → 权限确认（允许；带理由拒绝）→ 一个 `__agent-hook` 会话：列表、实时观看、关注、插话与读取回执、应用内「Agent 已完成」横幅 → 任务被关闭时新任务表单的提示 → App 离开前台 → 一次真 APNs 沙盒推送（模拟器收得到，扩展会运行；`simctl push` 不会运行扩展）→ 点击横幅 → 被关注的 Agent 结束回合的推送 → 点击打开 Agent 页 → 历史 → 解除配对。配对码含 broker 密码，只经命名管道从宿主交给 UI 测试（不落文件、不进环境变量 / 剪贴板，码在屏幕上时不截图），`xcresult` 结束时删除，并在日志与模拟器键盘日志里扫描配对码。模拟器默认只卸载 App + `simctl keychain reset` + 等 CPU 静下来；`AH_E2E_SIM_ERASE=1` 才抹掉设备（首次开机要占满一个核好几分钟）。`AH_E2E_SHOTS=<目录>` 保留截图。

## 不变量与注意点

- **隔离**：任何隔离运行的 Mac / daemon 必须设置 `ASKHUMAN_HOME`（只改 `HOME` 时 daemon 标签虽已独立，但 GUI Host 登录项与 Agent hook 文件仍会碰到真实环境）；测试台在启动前断言它位于临时目录。
- **IPC 协议号**：当前为 4，由 `ClientMsg::Ios` / `ServerMsg::Ios` 带来。给 `IosOp` / `IosResult` 加带 `#[serde(default)]` 的字段不 +1；规则见 `docs/overview.md` 的「IPC 协议号」。
- **线协议**：`ios/wire.rs` 与 `AHProtocol` 的限额常量（信封 ≤ 900,000 B、分块 192 KB、附件 ≤ 10 MB、每次回复 ≤ 5 个附件、令牌桶 6/s 突发 10、Follow ≤ 5、任务正文 ≤ 3000 字、插话 ≤ 20,000 字、Watch 间隔 ≥ 2 s 且每手机 ≤ 2 路）两边必须一致；改任何一边先改夹具。
- **幂等**：状态变更消息（回答、插话、`task.create` 等）按 `msgId` 进幂等日志，重放返回同一结果、同 ID 不同内容被拒绝。
- **「活着」的手机**：Gateway 只向「在线 + 前台 + 已 hello」的手机实时投递。App 在首次同步之后才发布前台状态，所以 hello 与前台状态之间的变化（新请求、Agent、Follow）不会被实时发出；`Actor::on_presence` 在手机由此变为「活着」时补发四个快照。改动这一段先看 `tests_requests` / `tests_agents` 里 `…between_hello_and_the_foreground_presence…` 两个用例。
- **「待通知」是推导出来的，不存标志位**：一道题对某部手机是否还在等推送 = `!seen && !pushed && !(is_live && connected) && can_push`，由 `flush_due_notifications`（`deliver()` 末尾与每 250 ms 的 `tick_requests`）判断；手机上下线、broker 抖动、`request.seen` 都不必逐个维护状态，推过一次 `pushed` 置位后条件不再成立，所以天然没有提醒。3 秒补推（前台手机没确认）不受等待时长约束；读不到空闲时长按「够久了」处理；`alert_after_secs` 不在 `ConnKey` 里，改设置不重连网关。改这一段先看 `tests_requests.rs` 的「push timing」一节。
- **Follow 的「做完 / 出错」走同一条等待规则，但是显式排队**（`docs/specs/ios-follow-narrowing.md`）：结果没有长期存在的对象可推导，所以 `announce` 把它排进 `Live.follow_notices`（内存，每部手机每个 Agent 至多一条），由 `flush_follow_notices`（`announce` 末尾与每 250 ms 的 `tick`）判断：Agent 又开始工作 / 会话结束 → 作废并继续关注；手机变成在看、或结果之后 Mac 有了新输入 → 作废；空闲满 D（或 `D = 0`、读不到空闲）→ 推一次。除「Agent 又开始工作」外，推出或作废都会把该 Agent 移出关注集（一次性），所以重启 daemon 会丢掉还没推出的通知。改这一段先看 `tests_agents.rs` 里 “A result (done / failed)” 那一节。
- **推送信封没有 `v`**：中继转给 APNs 的 `ah` 只有 `{f,k,n,c}`（Mac 封装时的 `v` 不转发），`PushDecryptor` 缺 `v` 时按本协议处理、带了别的版本则拒绝。改推送载荷或中继的白名单时，同步改 `PushDecryptorTests` 里「中继的形状」，否则单元测试与真实推送会各说各话。
- **通知代理方法必须在主 actor 上**：`AppDelegate` 的 `UNUserNotificationCenterDelegate` 方法是 `@MainActor`（一致性标 `@preconcurrency`）；写成 `nonisolated async` 时系统的完成回调落在协作线程池，iOS 27 上点推送横幅会崩溃。
- **每次握手都把推送句柄再给 Mac**：`PhoneEngine` 在每次 `hello.ack` 清空 `pushSent` 并重发，发送失败要回滚标记，token 在 ack 与首个快照之间到达也立即发；Mac 只会回 `push.state` 的 `ok` / `gone`，不会反过来索要句柄。
- **离开 / 返回前台的竞态**：`setForeground(false)` 在 `goodbye()` 处挂起，goodbye 之后若应在前台必须重连（`ConnectionTests.testComingBackWhileTheGoodbyeIsOnItsWayStillReconnects`）。`scenePhase == .inactive`（控制中心、通知中心、多任务界面、系统弹窗——App 仍在屏幕上）**不算离开**：`PhoneEngine.setInactive()` 给 `inactiveGrace`（10 s）的宽限期，超过才告别，期间回到前台则取消计时、保留连接；`.background`（锁屏、回主屏）仍立即告别；`AskHumanApp` 只做 `scenePhase` → `setInactive` / `setForeground` 的映射（`ConnectionTests` 里 5 个 inactive 用例，每处保护去掉都会变红）。强退没有任何 `scenePhase` 回调，靠 Last Will 兜底。真机上只验证了「停留超过 10 s 时离线窗口远短于停留」，短停留不离线、锁屏 / 回主屏与强退后 Mac 多快看到离线都**没在真机上验证**（见 P0 记录 §3.8）。
- **`AppStore` 的状态流与事件流没有先后**：`AppStore` 在两个独立任务里分别消费 `states` 与 `events`，同一次变化的两者谁先到不确定。`AHAppTests` 里不要假设顺序，每个条件各自用 `eventually` 等。模拟「别处已作答」用 `MockBackend.simulateFinishedElsewhere`（结束事件带 `by`，页面保留请求并显示提示）；不要用自己提交的回答代替——它的结束事件没有 `by`，App 会前进到下一个请求并忘掉提示，结果取决于哪条流先到（曾因此在 CI 上偶发失败三个用例）。
- **iOS 26 的顶部滚动边缘效果**：别用 `safeAreaInset(edge: .top)` 在列表上方固定自绘的栏——它会落进导航栏下的模糊带，让导航栏高到约三倍（曾经的筛选条）。需要的控件放进 toolbar（筛选是 `Menu`），状态文字放 `.principal` 的标题位。
- **「没有数据」与「还没加载」是两回事**：`EngineState.listReadiness`（`ready` / `loading` / `unreachable`）与 `syncStatus`（`upToDate` / `updating` / `reconnecting` / `refused`）由 `EngineState`（`AHCore/Models.swift`）从连接阶段与各 Mac 的 `link` 推导，`MacLink.connecting` 是唯一的「还不知道」状态。UI 只有在 `ready` 时才显示「已全部处理」类的空状态，`loading` 显示缓存或骨架行（15 秒仍无应答才显示「等待 Mac 响应」）；同步提示放标题位且延迟约 0.6 秒才出现，只有需要用户处理的问题（Broker 拒绝、时钟偏差、流量、Mac 离线）才在列表顶部出现提示卡。新增列表页时沿用这套，别自己写 `isEmpty → 空状态`。
- **多题表单的「发送」要等看过最后一题**：`AnswerComposer` 记录每题是否「看过」（题卡底部进入过可视区，已作答的算看过），多题时 `hasReachedEnd` 之前底部按钮是「下一题」，之后才是「发送」；与弹窗的 V1 / V2（`docs/specs/multi-question-vertical.md`）一致，改一边要同步改另一边。
- **提交后的换页是一个小状态机**（`AppStore.advance` → `moveOn`）：收到 Mac 的确认后先设 `receipt`（`RequestScreen` 盖上全页「已发送」回执）；`AdvanceTiming.hold` 之后把下一条**压在**已发送页上（`inboxPath = [a, b]`，得到系统自带的推入动画）并出「已发送 · 还剩 N 条」提示条；等推入真正停稳（`settle`，1.5 秒）再把 `a` 从栈里**无动画**地拿掉（`InboxScreen` 的 `.transaction(value:)`，因为栈顶页没变，带动画的话系统会把它「弹出」去显示一个一模一样的页面）。直接 `inboxPath = [b]` 只会原地交叉淡入，没有位移；`settle` 太短会让旧页面闪一帧。回执期间用户自己离开页面则 `cancelAdvance()`。UI 测试用 `AdvanceTiming.immediate` / `-AHAdvanceHold`；看这个动画要用不带 XCUITest 的 `simctl io recordVideo`（XCUITest 的无障碍快照会让过渡画面出现重影，录屏也是变帧率，只有帧的先后顺序可信）。
- **成本红线**：EMQX 消费上限保持 0（额度用尽即停，不自动付费）；不创建管理 API Key；Mac 与手机本机统计流量，80% 免费额度时预警。
- **保密**：broker 密码、配对码、方向密钥不得出现在日志、命令输出、文档与提交里；`E2ETests.testSecretsNeverReachTheLogsOrTheCommandOutput` 守着这条。
