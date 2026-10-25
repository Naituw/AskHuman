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

## Mac 端（`src-tauri/src/`）

| 位置 | 职责 |
|---|---|
| `ios/crypto.rs` · `envelope.rs` · `wire.rs` | 密钥派生、信封、全部线协议消息与限额常量；与 Swift `AHProtocol` 共用 `ios/Fixtures/wire-v1/` 夹具，两边测试必须同时通过 |
| `ios/transport.rs` | `rumqttc` 传输（TLS 优先、WSS 备用、退避重连）、`LoopbackTransport`（测试：丢包 / 乱序 / 断线注入）、`probe`（设置页「测试连接」）。明文 TCP 只对回环地址生效（`first_via`） |
| `ios/store.rs` | `~/.askhuman/ios/` 下的 `state.json`（手机、关注集、推送句柄、本月流量）与 `journal.json`（状态变更消息的幂等日志）；配对密钥在系统钥匙串，缺钥匙串时回退 `keys.json` 并在设置页提示 |
| `ios/views.rs` | 把请求投影成手机渲染的视图（选项原文、TODO、确认变体） |
| `ios/connstr.rs` · `qr.rs` | `ahconn1.` 连接串（含 broker 密码，只在显式请求时产出）、配对二维码 SVG |
| `ios/push.rs` | 中继客户端（`/v1/push`），只传信封与句柄 |
| `ios/gateway/` | Gateway actor：`mod.rs`（事件循环、在线状态、hello 快照）· `pairing.rs`（配对状态机）· `requests.rs`（问答 / 确认闭环、答案校验、通知触发）· `agents.rs`（Agent 列表、Follow、`AgentEvent` 三类通知）· `watch.rs`（实时帧）· `tasks.rs`（新任务、状态机 accepted→launching→launched→watching…）· `assets.rs`（附件双向分块）· `hostapi.rs`（Gateway 向 Daemon 提问的 `Host` trait）；`testkit.rs` + `tests*.rs` 是进程内的完整测试台 |
| `daemon/runtime/ios.rs` · `ios_agents.rs` · `ios_tasks.rs` | `Host` 的实现：配置变化时重启 Gateway、读 registry / Watch / 插话、启动任务 |
| `daemon/runtime/task_launch.rs` | IM `/new` 与 iOS 共用的任务启动（门槛、冲突、终端 / Codex App 路径） |
| `channels/ios.rs` | `Channel` 实现：请求交给 Gateway，首个终态获胜后撤销手机上的请求 |
| `cli/ios_cmd.rs` | `AskHuman channel ios status / pair / allow / devices / revoke …` 与 `channel set ios …`（密码经 stdin / 文件 / 环境变量，从不进 argv） |
| `commands.rs` | 设置页用的 Tauri 命令：`ios_op`（经 IPC 的 `ClientMsg::Ios`）、`ios_test`、`ios_pair_qr`、`ios_connection_import` / `export` |

前端：`src/views/settings/IosChannelCard.vue` + `useIosChannel.ts`（状态轮询、配对流程）、`iosTraffic.ts`（本月流量与免费额度预警）。

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
- Mock 模式启动参数：`-AHMock YES -AHScenario demo|empty|offline|quota -AHLatency <x> -AHTab inbox|agents|history|macs -AHOpen <id> -AHAutoAdvance YES|NO -AHOpenAgent <key> -AHNewTask YES -AHHoldInterjects YES -AHFollowEvent done|failed|waiting`。
- XCUITest 经验：容器 view 上的 `accessibilityIdentifier` 需要 `.accessibilityElement(children: .contain/.combine)` 才找得到子元素；Form / List 的懒加载行需要滚动到附近才存在；浮动 TabBar 会挡住详情页底部，详情页用 `.toolbar(.hidden, for: .tabBar)`。

## 验证

| 范围 | 命令 |
|---|---|
| Rust（含 Gateway 测试台与跨语言夹具） | `cargo test --manifest-path src-tauri/Cargo.toml ios::` |
| Swift 包（单元 + 进程内集成） | `cd ios/Packages/AskHumanKit && swift test` |
| 中继 | `cd relay && npm test` |
| 设置页 | `npx vitest run src/views/settings` |
| App UI（模拟器） | `xcodebuild test -project ios/AskHuman.xcodeproj -scheme AskHuman -destination 'platform=iOS Simulator,id=<UDID>' -derivedDataPath ios/build/DerivedData CODE_SIGNING_ALLOWED=NO -parallel-testing-enabled NO -collect-test-diagnostics never`（`TEST_RUNNER_AH_UI_LANG=zh-Hans` 切中文，`TEST_RUNNER_AH_UI_SHOTS=<dir>` 保存截图）。必须前台跑；后台会被工具杀掉 |
| 真实 daemon 端到端 | `cd ios/Packages/AskHumanKit && AH_E2E_BIN=<已安装的 AskHuman> AH_E2E_BROKER_DIR=<ios/Tools/local-broker> swift test --filter AHE2ETests`（缺环境变量时自动跳过） |

端到端测试台会起**一台一次性的 Mac**：独立 `ASKHUMAN_HOME`、`HOME`、`ASKHUMAN_NO_KEYCHAIN=1`，加本地 Node broker；`agentTasks.enabled` 保持关闭，所以不会真的启动任何终端。伪造的 Agent 会话通过隐藏命令 `AskHuman __agent-hook <agent> <event>` 注入。

## 不变量与注意点

- **隔离**：任何隔离运行的 Mac / daemon 必须设置 `ASKHUMAN_HOME`（只改 `HOME` 时 daemon 标签虽已独立，但 GUI Host 登录项与 Agent hook 文件仍会碰到真实环境）；测试台在启动前断言它位于临时目录。
- **IPC 协议号**：当前为 4，由 `ClientMsg::Ios` / `ServerMsg::Ios` 带来。给 `IosOp` / `IosResult` 加带 `#[serde(default)]` 的字段不 +1；规则见 `docs/overview.md` 的「IPC 协议号」。
- **线协议**：`ios/wire.rs` 与 `AHProtocol` 的限额常量（信封 ≤ 900,000 B、分块 192 KB、附件 ≤ 10 MB、每次回复 ≤ 5 个附件、令牌桶 6/s 突发 10、Follow ≤ 5、任务正文 ≤ 3000 字、插话 ≤ 20,000 字、Watch 间隔 ≥ 2 s 且每手机 ≤ 2 路）两边必须一致；改任何一边先改夹具。
- **幂等**：状态变更消息（回答、插话、`task.create` 等）按 `msgId` 进幂等日志，重放返回同一结果、同 ID 不同内容被拒绝。
- **成本红线**：EMQX 消费上限保持 0（额度用尽即停，不自动付费）；不创建管理 API Key；Mac 与手机本机统计流量，80% 免费额度时预警。
- **保密**：broker 密码、配对码、方向密钥不得出现在日志、命令输出、文档与提交里；`E2ETests.testSecretsNeverReachTheLogsOrTheCommandOutput` 守着这条。
