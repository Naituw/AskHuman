# 计划：iOS 历史里的图

> 需求：`docs/specs/ios-history-pictures.md`（决策编号 HP1–HP12 均指其 §3）。状态：2026-11-12 用户批准需求（AskHuman 三问全选推荐项），开工并已实现；提交、偏差与验证见文末「实施记录」。下文各阶段的文件名是起草时的设想，**实际以「实施记录」为准**。
> 工作方式：**在主工作树开发**（用户沿用今天会话里已选的位置，不再问）。主工作树里有别的会话**未提交**的改动（开工时是 iOS 页面容器 `Views/Common/AppTabs.swift`、`ListDetail.swift`、`RootView.swift` 等与一份新的 `AskHumanPersistentTabsUITests.swift`，及几份 iOS 规格 / overview 的段落）：本功能只提交自己的文件与自己的段落，不碰它们；要改同一份文档时只用精确替换，提交前逐个 hunk 核对。

## 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 需求、本计划 | 提交 |
| P1 | 线协议：Rust 与 Swift **同一提交**（共用夹具） | `cargo test ios::fixtures`、`swift test --filter AHProtocolTests` |
| P2 | Mac：图的投影与缩略图、`shrink`、`history.exchange.get`、历史图取图、Host 与能力位 | `cargo test`（`ios::`、`daemon::runtime`）、`cargo clippy -D warnings`、`cargo fmt --check` |
| P3 | 手机引擎：`Backend.sessionExchange`、`pictures` 自动带、Mock、单测 | `cd ios/Packages/AskHumanKit && swift test` |
| P4 | 手机界面：折叠排 / 展开网格 / 占位 / 回复图 / 查看器、「历史」标签详情页、本地化、UI 测试 | `xcodebuild build` → `make-catalog.py` → `xcodebuild test`（模拟器，en + zh-Hans） |
| P5 | 验证：真实 daemon 端到端、装机、文档同步 | 见「验证清单」 |

## P1 细节（线协议，Rust 与 Swift 同一提交）

- **Rust `ios/wire.rs`**：
  - 常量：`HISTORY_PICTURES_MAX = 12`、`HISTORY_THUMB_MAX_BYTES = 6 * 1024`；`t::HISTORY_EXCHANGE_GET = "history.exchange.get"`、`t::HISTORY_EXCHANGE = "history.exchange"`，`registry!` 与 `msg!` 各加一项。
  - 类型：`AssetRef.gone: bool`（`#[serde(default, skip_serializing_if = "is_false")]`）；`SessionExchange.pictures: Vec<AssetRef>`、`SessionQuestion.pictures: Vec<AssetRef>`（缺省空、空时不发）；`SessionHistoryGet.pictures: bool`（缺省 false、false 时不发）；`HistoryExchangeGet { req_id, id }`、`HistoryExchange { req_id, id, item: Option<SessionExchange> }`；`Caps.history_pictures: bool`（缺省 false、false 时不发）。所有字面构造要补字段（`Caps`、`AssetRef`、`SessionExchange`、`SessionQuestion`、`SessionHistoryGet` 在 `views.rs`、`history_view.rs`、`testkit.rs`、`wire.rs` 内测试各处）。
  - `SessionHistory::shrink` 与新的 `HistoryExchange::shrink`：第一步改成「去掉最旧的、仍带缩略图的往来的缩略图（返回真）」，没有缩略图可去再走原来的丢旧往来 / 减半文字。
- **Swift `AHProtocol`**：同名类型与常量；`Caps.historyPictures`（`@OmitFalse`）、`AssetRef.gone`（`@OmitFalse`）；`SessionExchange.pictures` / `SessionQuestion.pictures`（缺省 `nil`，`nil` 不编码）；注册表加两项。
- **夹具 `ios/Fixtures/wire-v1/messages.json`**：只**追加**；`history.session.get` 带 `pictures` 一条；`history.session` 带 `pictures`（含 `thumb`、`gone`、`tooLarge`、回复图）一条；`history.exchange.get`、`history.exchange`（有 / 无 `item`）；一条带 `historyPictures` 的 `hello.ack`。既有样本原样，证明缺省字段不进 JSON。
- **不碰** `ipc/mod.rs`；`PROTO` 与 `PROTOCOL_VERSION` 不动。

## P2 细节（Mac）

1. **槽与 id**（`ios/history_view.rs`，纯函数）：`PictureSlot { Message(usize), Reply { question, index } }`；`picture_id(exchange_id, slot) -> String`（`base32_lower(sha256(...)[..16])`，沿用 `crypto` 里现有的编码；测试：确定、格式合法、不同槽 / 不同往来不同）。
2. **探测接口与投影**：`trait PictureProbe { fn probe(&self, path: &Path, name: &str) -> Option<PictureFile> }`（`None` = 文件不在；`PictureFile { size, thumb: Option<Vec<u8>> }`）；`with_pictures(exchange, entry, &probe) -> SessionExchange`：消息的 `is_image` 文件各一条（记录顺序、最多 12）、各题 `answers[q].images` 各一条（最多 12，名字取文件名）；不在 → `gone`、`size` 为 0、无缩略图；`size > MAX_ASSET_BYTES` → `tooLarge`、无缩略图；`mime` 由 `views::mime_for` 推出。`exchange_of` 保持纯计数。测试：以上全部，加序列化里没有路径、非图片只计数。
3. **缩略图**（`ios/views.rs`）：`history_thumbnail(path) -> Option<Vec<u8>>`：解码，按阶梯 [(200,60),(160,50),(120,40),(96,35)] 缩成 JPEG，取第一张 ≤ `HISTORY_THUMB_MAX_BYTES` 的；**缓存**（`(路径, 修改时间, 大小)` 为键，上限 256 张，满了整体清空即可）。测试用真实小图（`image` crate 在内存里造），断言字节上限与方向。生产探测 `DiskPictures` 放在 `daemon/runtime/ios_history.rs`（它是唯一读磁盘的地方）。
4. **取图解析**（纯函数，`ios/history_view.rs`）：`find_picture(entries, asset_id) -> Option<PictureLocation { path, name }>`：逐往来逐槽算 id 比对。`Host::history_picture(&self, asset_id: &str) -> Option<AssetSource>`（默认 `None`）；Daemon 实现：`spawn_blocking` 读最近历史（`history.rs` 现有的读取，条数同历史上限），命中且文件仍是普通文件才返回 `AssetSource`（与请求附件同型）。测试：命中、不在、不是文件、历史关着、未知 id。
5. **网关取图**（`ios/gateway/assets.rs`）：把「按 `AssetSource` 读文件并发块」抽成一个私有函数；`on_asset_get` 顺序：待答请求 → 网关的历史图缓存（`HashMap<assetId, AssetSource>`，≤ 256 条，满了丢最旧）→ `spawn_then(host.history_picture(id))` 命中写缓存并发送、未命中 `notFound`。测试：历史图分块与校验、第二次取图走缓存（Host 只被问一次）、换新网关实例仍可解析（Host 里仍有）、未知 / 非法 id `notFound`、请求附件优先、窗口 / 上限规则不变。
6. **单条往来**（`ios/gateway/history.rs`）：`on_history_exchange_get`，同 `on_session_history_get` 的 `spawn_then` + `shrink` 到发得出去，回显 `reqId` 与 `id`；`Host::history_exchange(&get) -> HistoryExchange`（默认无 `item`）；`gateway/mod.rs` 分发加一支；老手机从不发这条。
7. **页**：`Host::session_history` 在 `get.pictures` 为真时带图（Daemon 实现把探测接进 `page_with`）；Daemon 的 `caps()` 置 `history_pictures: true`；`gateway/testkit.rs` 的测试宿主支持注入图、历史图与 `history_exchange`。测试：不带 `pictures` 时页面与现在逐字相同；带时有引用；`shrink` 先去缩略图；能力位只在 Host 声明时发。
8. **不碰** `ipc/mod.rs`、恢复库；`history.jsonl` 的格式与保留策略不变。

## P3 细节（手机引擎）

1. `AHProtocol`：见 P1。
2. `AHCore/Backend.swift`：`RemoteBackend` 加 `sessionExchange(macId:id:) async throws -> SessionExchange?`；conformer 只有 `PhoneEngine` 与 `MockBackend`。
3. `AHCore/PhoneEngine+SessionHistory.swift`：`sessionHistory` 在 `caps?.historyPictures == true` 时带 `pictures: true`；`sessionExchange` 与它同一套信箱（`TodoReply` 加 `.exchange`，键 `"x|" + macId + "|" + reqId`，断线 / Mac 掉线 / 删除 Mac 的统一失败自动覆盖），超时 10 秒，`macOffline` / `.rejected("unsupported")`；分发加 `case HistoryExchange.type`。
4. `AHMock`：`sessionHistory` 场景给 Mac 加 `historyPictures`；`MockData` 造带图的往来（0、1、2、5、7 张，含已不在的、含回复图）；`MockBackend.download` 对历史图 `assetId` 返回生成的图；`sessionExchange` 按往来 id 返回。「历史」标签的 `demo` 数据里有一条请求 id 与 Mock 往来对得上。
5. 测试：`AHCoreTests/TestMac+SessionHistory.swift` 扩展（假 Mac 回带图的页与单条往来）；引擎：`pictures` 随能力位自动带 / 不带、`sessionExchange` 路由、超时、离线、不支持、断线失败；下载一张历史图走既有分块。

## P4 细节（手机界面）

1. **缩略图格**：把 `AssetViews.swift` 里私有的 `ImageThumbnail` 提成可复用的 `PictureCell`（尺寸参数、`gone` 占位、`tooLarge` 角标、`+N` 蒙层、点击回调），请求页的使用方式不变（像素级回归由既有 UI 测试保证）。
2. **会话历史卡片**（`SessionThreadView.swift`）：`SessionExchangeCard` 收一个 `macId`；折叠时消息下面一排 `PictureCell`（≤ 4，`@ScaledMetric` 56 pt）；展开时网格（`GridItem(.adaptive(minimum: 96, maximum: 140), spacing: 8)`）与每题答案下的回复图；末尾「附件 N 个」= `attachments − pictures.count`（小于等于 0 不写），回复里剩余的「N 张图」同理；点缩略图 → `ImageGalleryViewer`（本往来所有非 `gone` 的图，消息图在前、回复图依次在后，起点是被点的）；点占位 → 弹说明；点卡片其它处展开 / 收起；读屏：卡片保持合并元素，加自定义动作「查看图片」，点缩略图不触发展开。
3. **「历史」标签详情页**（`HistoryDetailScreen.swift`）：`task` 里在 Mac 在线且有能力时读一次 `sessionExchange(macId:id: entry.request.id)`，本地 `@State` 保存；有 `item` 时「消息文件」里图片附件换成网格（其余文件仍是文件名行），回答里该题回复图排在答案下面；失败 / 没有 `item` 保持现状，不显示错误；页面离开即丢。
4. **文案**：新增「已不在 Mac 上」「这张图已不在这台 Mac 上（文件被移动或被系统清理了）。」「查看图片」「{n} 张图未显示」之类，英文为源语言；`xcodebuild build` → `ios/Tools/make-catalog.py`（缺中文会失败）；新增 Swift 文件后在 `ios/` 下 `xcodegen generate`。
5. **无障碍标识**：`session-picture`（每个缩略图格，值为文件名）、`session-picture-gone`、`session-picture-more`（`+N` 格）、`history-picture`（详情页缩略图）。
6. **UI 测试** `AskHumanHistoryPicturesUITests.swift`（Mock `sessionHistory` 场景，en + zh-Hans）：折叠排与「+N」；点缩略图开查看器且**不展开**卡片；展开网格、回复图、占位格与说明；没有图的卡片不变；「历史」标签详情页出现缩略图并可打开；老 Mac（无能力）不显示、不报错；最大字号、深色、iPad 各一条。

## 验证清单

1. `node scripts/machine-health.mjs`；退出码 2 时先用 AskHuman 把报告告诉用户并请他腾内存（用户已说过：继续放后台跑、短间隔轮询、期间只写文档，不用管他），长任务放后台，短间隔轮询并说明在等什么。
2. `cargo test`（相关模块）→ 全量一遍；`cargo clippy --all-targets -- -D warnings`；`cargo fmt --check`。
3. `./scripts/install.sh`；之后的提问走新装的 `AskHuman`。
4. **真实 daemon 端到端**（`AH_E2E_BIN=<已安装的 AskHuman> AH_E2E_BROKER_DIR=<ios/Tools/local-broker> swift test --filter AHE2ETests`）：在一次性 Mac 的 `configDir` 里写 `history.jsonl`（同一会话几条往来，带真实的小 PNG 附件与一条回复图、一条文件已删除的），手机引擎取该会话：引用与缩略图在、已删除的 `gone`；`download` 一张图 SHA-256 与原文件一致；`sessionExchange` 取单条；重启 daemon 后同一 `assetId` 仍能下载。
5. **iOS**：`swift test`；模拟器 `xcodebuild test`：`AskHumanHistoryPicturesUITests`，加上与会话历史、历史标签、请求页附件（缩略图 / 查看器 / 拖出）有关的既有用例（`AskHumanSessionHistoryUITests`、`AskHumanHistoryUITests` 一类、`AskHumanAttachmentUITests` 一类），en + zh-Hans。别的会话的未提交改动可能让整套编译或个别用例失败：失败时先判断是不是它们造成的，写进实施记录，不替它们修。
6. **真实 Mac + 模拟器**：用新装的 `AskHuman` 带一两张图问一个问题、答掉，再发下一个；模拟器配对到这台 Mac，请求页下拉看见带图的往来；点开看大图；「历史」标签详情页同样；把其中一张原文件删掉再看占位。真机与 TestFlight：**不出**，做完后单独问用户（出包按 `ios-testflight` skill）。
7. **文档同步**：`docs/overview-ios.md`（能力位 `historyPictures`、两条新消息、`history_view.rs` / `gateway/assets.rs` / `gateway/history.rs` 的新职责、「历史图 id 确定、解析靠历史、Mac 存储不动」进不变量）；`docs/specs/ios-session-history.md` 已加交叉引用；本计划的「实施记录」；`docs/PROGRESS.md`（待真机验收一条）。主 `docs/overview.md` 不改。

## 提交（Conventional Commits，英文、祈使句；用户可见的只有第 4 条）

1. `docs(ios): spec and plan for pictures in the history`（P0）
2. `feat(ios): add picture fields and exchange messages to the wire protocol`（P1，Rust + Swift + 夹具，`Release-Note: skip`）
3. `feat(ios): describe and serve the pictures in a session's history`（P2，`Release-Note: skip`）
4. `feat(ios): show the pictures of past exchanges in session history and the History tab`（P3–P4，带 `Release-Note:`，一句话写成用户语言）
5. `docs(ios): sync overview and specs with history pictures`（P5）

## 风险

1. **页大小**：靠缩略图 6 KiB 上限、整页预算与 `shrink` 先去缩略图保证；单测里造极端往来（每题 12 张图、20 条往来）。
2. **整页缩略图的解码耗时**：一页最多 8（至多 20）条往来 × 数张图；解码在 `spawn_blocking`、有缓存、> 10 MB 的不解码；实测慢再限并发。
3. **读历史取图**：每次首次取原图要整份读 `history.jsonl`（默认 200 条）；网关缓存挡掉同一张图的重复扫描，实测慢再考虑索引。
4. **`AssetRef` 的复用**：`gone` 是新字段，老手机只会在不带 `pictures` 时见不到它；夹具证明缺省不进 JSON。
5. **请求页缩略图格的回归**：`ImageThumbnail` 改成共用组件，要把请求页的既有 UI 用例（缩略图、查看器、拖出）全跑一遍。
6. **别的会话的改动**：见开头；每个阶段提交前 `git status` / `git diff` 核对，只 `git add` 自己的文件，`docs/overview-ios.md` 等共用文件用精确替换并逐 hunk 核对。
7. **机器很忙**：全量编译与模拟器测试可能要十几分钟；开发中只跑定向测试，阶段结束跑完整的。

## 实施记录

### 与起草时的偏差

1. **提交粒度**：起草时 P1（线协议）与 P2（Mac）分两个提交，实际合成一个。Rust crate 一旦给 `AssetRef` 加了 `gone`、给 `Caps` 加了 `history_pictures`，所有字面构造（`views.rs`、`history_view.rs`、`testkit.rs` 等）都得同时改，P1 单独提交编译不过；AGENTS.md 又要求 Rust 与 Swift 线类型同一提交，所以「线协议 + 夹具 + Mac 逻辑」是一个提交，手机引擎一个，手机界面一个。
2. **网关的历史图缓存有 10 分钟过期**（起草只写了 ≤ 256 条）：缓存的只是「id → 路径」的解析结果，每个窗口仍重新读文件，所以期间文件被清掉照样回 `notFound`、被换成同名新图则按新内容发（块数与 SHA-256 以实际读到的字节为准）；过期只是让很久以后的取图重新解析一遍历史。规格 HP7 已补。
3. **折叠一排把回答里贴的图也算进去**（消息图在前、回复图在后，与查看器翻页的顺序一致），而不只是消息的图；规格 HP3 的括号已补。
4. **读屏做成分段而不是「合并元素 + 读屏动作」**：没有图的卡片仍是一个合并元素；有图的卡片是容器，头部与每题各一个合并停靠点，每张图是单独的按钮。合并元素上的自定义动作藏在转子里不好发现，而图可能有几张，逐张聚焦更直接。规格 §7、§12 已改。
5. **Mock 场景**：没有改 `sessionHistory` 场景（它的数据与大量既有测试按原样依赖），而是新增独立的 `historyPictures` 场景（`MockData+HistoryPictures.swift`）；图 id 里编着尺寸与色相，`MockBackend.download` 据此生成图。`PictureCell` 没有另起组件：`ImageThumbnail` 提成可复用（圆角、紧凑形态两个参数），占位与「+N」蒙层在新文件 `HistoryPictureViews.swift` 里。
6. **文案**比起草时少一条：没有「查看图片」读屏动作，也没有「{n} 张图未显示」。新增的是「已不在 Mac 上」、弹窗说明、两条读屏标签（「名字，已不在 Mac 上」「名字，后面还有 N 张」）。

### 提交（2026-11-12，主工作树，未推送）

| 提交 | 内容 |
|---|---|
| `a10f546b` | `docs(ios): spec and plan for pictures in the history` |
| `09862243` | `feat(ios): describe and serve the pictures of a session's history`：线协议（Rust + Swift + 夹具）与 Mac 一起，`Release-Note: skip` |
| `e7dde0de` | `feat(ios): read the pictures of a session's history in the phone engine`：引擎、Mock、单测与真实 daemon 端到端测试，`Release-Note: skip` |
| `2256fab9` | `feat(ios): show the pictures of past exchanges in session history and the History tab`：界面、本地化、UI 测试；唯一带用户可见 `Release-Note:` 的一条 |
| 文档同步提交 | `docs(ios): sync overview and specs with history pictures`（`docs/overview-ios.md`、规格、本计划、`docs/PROGRESS.md`；主 `docs/overview.md` 没改） |

### 验证结果

机器一直偏紧（交换区 11–18 GiB，期间磁盘到过 99%，删掉了可再生的 `src-tauri/target/debug/incremental` 腾出空间），长任务都放后台、短间隔轮询。

- **Rust**：`cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --check` 干净；`ios::`、`daemon::runtime` 的定向测试通过；全量 `cargo test`：3005 通过、0 失败、11 个忽略（在机器换页最厉害的时候跑完，214 秒）。
- **Swift 包**（`ios/Packages/AskHumanKit`，`swift test`）：542 个测试、0 失败，含新增的 `HistoryPicturesWireTests`、`HistoryPicturesEngineTests`、`HistoryPicturesStoreTests`。
- **真实 daemon 端到端**（`AHE2ETests`，用刚 `./scripts/install.sh` 装好的 0.17.1）：10 个测试、0 失败，其中 5 个是新增的 `HistoryPicturesE2ETests`：页里的图没有路径、缩略图 ≤ 6 KiB、600×500 的噪点 PNG 多窗口下载与原文件逐字节一致、回复图可下载；页取到之后文件被删 → `notFound`；11 MB 的文件 → `tooLarge` 且不发下载请求；`sessionExchange` 与页内同一条相等、未知 id 回空；`daemon restart --force` 之后同一 `assetId` 仍可下载。
- **模拟器 UI**（`xcodebuild test`）：
    - iPhone 16 / iOS 18.5：`AskHumanHistoryPicturesUITests` 12/12（英文）与 12/12（`zh-Hans`）；请求页附件与查看器的回归 8 个通过；`AskHumanSessionHistoryUITests` 35 个通过（其中 `testPullingTheHistoryOnAnEndedPageStopsItsCountdownToo` 在忙碌的机器上失败过一次、单独重跑通过，是时序抖动，与本功能无关）、3 个按设计跳过。
    - iPad Pro 11 英寸（M4）/ iOS 18.5：新用例 11 个通过、1 个按设计跳过，加既有的 `AskHumanLargeScreenUITests/testTheHistoryOpensAnEntryBesideTheList` 通过；详情页在列表右侧显示 7 张图的网格与「checklist.md」文件行。iPad 上第一次跑暴露了两处**测试**的假设（不是产品问题）：iPad 的顶部标签栏不在 `tabBars` 里（按钮是普通 `Button`），所以「历史」标签改用启动参数 `-AHTab history` 进入；最大字号下 iPad 的工具栏把请求菜单折进系统的「更多」按钮、没有标识，所以那条「最大字号一排不出卡片」的用例在 iPad 上跳过（iPhone 上更窄、一排会折行，是更严的场景，那里通过）。改完在 iPad 与 iPhone 上各重跑通过。
- **本地化**：`make-catalog.py` 重新生成 667 条，没有缺中文；新增的只有 4 条（见偏差 6）。

### 没有验证的

1. **真机与 TestFlight**：没出包，手感、手指好不好点、真实 VoiceOver 的朗读（现在只有 XCUITest 对标签与停靠点的断言）都没在真机上看过。
2. **带签名的 App 连真实服务**（`SimulatorLiveTests` 需要真实服务与钥匙串重置）没跑；引擎到真实 daemon 这一段有端到端测试，界面这一段只在 Mock Mac 上验过。
3. **附件拖出的 UI 测试**（`AskHumanAttachmentDragUITests`）需要宿主端的接收器，在这台机器上按设计跳过；`ImageThumbnail` 的改动（圆角、紧凑形态两个参数）由请求页的其它附件 / 查看器用例覆盖，拖出本身没跑。
4. **iOS 26.2 / 27.2 模拟器与折叠屏（Duo）**没跑，只在 iOS 18.5 的 iPhone 16 与 iPad Pro 上验过。
5. `AgentE2ETests` 等与本功能无关的端到端套件没有跑基线。
