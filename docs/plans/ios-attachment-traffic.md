# 计划：iOS 附件流量收敛（预览优先、二进制传输、内容去重）

> 需求：`docs/specs/ios-attachment-traffic.md`（决策编号 AT1–AT20 均指其 §3）。状态：**2026-11-21 起草，未开工**。规格里写规格时补的 AT14–AT20（AT20 动到 `Cargo.toml` 的编译配置）已由 Owner 确认；规格与本计划一并发给他看过，没有提出修改。下文各阶段的文件名与函数名是起草时按现有代码读出来的设想，**开工后实际以文末「实施记录」为准**。
> 工作方式：**开工前**按 `AGENTS.md` 问一次工作位置（主工作树与单独的 Dev Worktree 同列、一轮问完）。那一问里要说清：本功能会改 `Cargo.toml` 的依赖与编译配置，在主工作树里装机会替换 Owner 日常用的二进制。写计划时主工作树里有别的会话**未提交**的改动（「弹窗文件链接」一类：`src-tauri/src/app/*`、`commands.rs`、`attachment_markdown.rs`、`src/` 下的前端文件与几份弹窗规格）：本功能只提交自己的文件与自己的段落，不碰它们；要改同一份文档时只用精确替换，提交前逐个 hunk 核对。

## 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 规格、本计划、`docs/PROGRESS.md` 登记 | 链接与锚点检查，提交 |
| P1 | **D 二进制传输**：a) Rust 与 Swift 线协议（二进制信封、blob 帧、协商字段、夹具）**同一提交**；b) 手机引擎收发 | a) `cargo test ios::`、`swift test --filter AHProtocolTests`；b) `swift test`、真实 daemon 端到端 |
| P2 | **A+B 预览**：先做编译配置的底座（预览 crate + 两档 opt 3，先量耗时）；a) Mac 预览生成与取图 + 线协议字段（Rust 与 Swift 协议同一提交）；b) 手机引擎；c) 查看器界面 | 预览耗时对照规格 §10.3 的表；`cargo test`；`swift test`；UI 测试 |
| P3 | **C+G**：手机按内容哈希缓存、Mac 本地计数 | `cargo test`、`swift test` |
| P4 | 验证：全量测试、装机、真实 Mac + 模拟器、文档同步；出 TestFlight 另问 | 见「验证清单」 |

按 AT8，三步（P1 → P2 → P3）合成**一个 Mac 版本和一个 TestFlight 构建**：每一步都能单独装机验证、单独回退，但不单独发布。

## P1 细节（D：二进制传输）

### P1a 线协议（Rust 与 Swift 同一提交，共用夹具）

**Rust（`src-tauri/src/ios/`）**

- `crypto.rs`：加 `base32_decode_lower(&str) -> Option<Vec<u8>>`（小写字母表，长度与末尾补位都严格）与 `id_to_bytes(&str) -> Option<[u8; 16]>`（只认 `new_id()` 的规范形式，AT16）。测试：与 `new_id()` 往返；末尾补位非零、长度不对、大写、字母表外都是 `None`。
- `envelope.rs`：
  - 解码后的字节形态 `Frame { sender, kid, nonce, ciphertext, compressed }`。`parse_frame(bytes)`：首字节 `{` 走现有 JSON 解析再解 base64，`0xA1` 走二进制，其余畸形；flags 的第 1–7 位非零、总长 < 48 都畸形；仍受 `MAX_ENVELOPE_BYTES`。`parse_frame_from_phone` 在此之上拒收压缩位（沿用 `parse_from_phone` 的语义）。
  - `seal_binary_with_nonce` / `seal_binary`：AAD 仍用现有 `aad()`，发送方还原成 26 字符 base32，所以 AAD 与 JSON 信封逐字节相同；`open_frame` 与现有 `open` 共用解密与解压的实现；`can_use_binary(sender, kid) -> bool`（AT16）。
  - blob：`encode_blob(head: &Plain, bytes: &[u8])`；`decode_plaintext(&[u8]) -> Decoded::{Json(Plain), Blob { head, bytes }}`，首字节 `{` 或 `0x01`，其余畸形，`header_len ≤ BLOB_HEADER_MAX_BYTES`（4096）且不超过剩余长度。解压路径得到的明文以 `0x01` 开头一律拒收（AT15）。
  - 测试：二进制往返与固定向量；AAD 绑定主题 / 发送方 / `kid` / 压缩位（改任何一项都解不开）；未知 flags 位；手机方向的压缩位被拒；长度边界（47 / 48 / 上限）；非规范 id 与 `kid > 65535` 回退 JSON；blob 往返与各类畸形（首字节、`header_len` 越界 / 超限、头部不是合法 JSON、被压缩过的 blob）。
- `wire.rs`：`Hello.binary`、`Caps.binary`（缺省 false、false 时不发，`#[serde(default, skip_serializing_if = "is_false")]`）；`BLOB_HEADER_MAX_BYTES`；`AssetChunk.data`、`UploadChunk.data` 加 `#[serde(default)]`（只影响解码，blob 头部里没有 `data`；序列化规则不动，旧路径逐字节不变，blob 头部由序列化结果去掉 `data` 键得到）。所有字面构造补字段（`Hello` 与 `Caps` 在 `wire.rs` 内测试、`gateway/testkit.rs`、`tests_agents.rs`、`tests_traffic.rs` 等各处）。
- `gateway/mod.rs`：
  - `PhoneRt.binary`：`on_hello` 里取自 `hello.binary`，以最近一次 `hello` 为准，随 `hello_at` 清空（下线）而失效，同 `compression`。`Caps.binary` 由 Host 的 `caps()` 声明（`daemon/runtime/ios.rs` 的 `DaemonHost::caps` 恒为 true，测试宿主 `testkit.rs` 可开关）。
  - `send_value`：手机声明了 `binary` 且 `can_use_binary` 时发二进制信封，否则照旧；`hello.ack` 起的一切都可以是二进制（`hello` 自己恒为 JSON，配对、presence、push 不动）。新增 `send_blob(phone_id, msg_type, head, bytes, kind)`。
  - `open_up`：改用 `parse_frame_from_phone` + `open_frame`，不管手机有没有声明 `binary`，两种格式都收；配对主题上的二进制帧直接丢弃。`dispatch` 之前先 `decode_plaintext`：blob 只接受 `upload.chunk`，别的类型的 blob 丢弃并记一条日志。
  - 本地计数沿用 `count_message(.., bytes)`，字节数按线上实际字节。
- `gateway/assets.rs`：`serve_asset` 把每块的头部与字节交给一个 `send_chunk`（按协商选 `send_blob` 或旧的 JSON + base64）；`on_upload_chunk` 同时接受 JSON 的 `data` 与 blob 的字节（先统一成「头 + 字节」再进现有逻辑）；窗口、`notFound` / `tooLarge` / `internal` 规则一字不动。
- 夹具（`ios/fixtures.rs` 与 `ios/Fixtures/wire-v1/`）：**只追加**。`crypto.json` 加 `binaryEnvelopes`（固定密钥、nonce、发送方、`kid`、明文 → 期望字节）、`blobFrames`、畸形向量；`messages.json` 加带 `binary` 的 `hello` 与 `hello.ack`。既有样本原样，证明缺省字段不进 JSON。

**Swift（`ios/Packages/AskHumanKit/Sources/AHProtocol`）**

- `Base.swift`：`Bytes.base32Decode`、`Bytes.idToBytes`（规范性检查）。
- `Envelope.swift`：同样的 `Frame`；`AHEnvelope.parseFrame(_:)`（主题消息用；配对与推送继续用只认 JSON 的 `parse`）、`sealBinary`、`open(frame:)`；blob 的 `Plain.sealedBlob(...)` 与解码。
- `Wire.swift`、`Messages.swift`：`Hello.binary`、`Caps.binary`（`@OmitFalse`）、`Wire.blobHeaderMax`；`AssetChunk.data` 与 `UploadChunk.data` 解码时缺省为空。
- 测试：`CryptoFixtureTests`（新向量与畸形）、`MessageFixtureTests`（新样本）；两边对**同一份**夹具通过。
- **不碰** `ipc/mod.rs`；`PROTO` 与 `PROTOCOL_VERSION` 不动（AT9）。

### P1b 手机引擎（单独一个提交）

- `AHCore/PhoneEngine.swift`：`openFromMac` 改走 `parseFrame` + `open(frame:)`，返回 `.json(Plain)` 或 `.blob(head: Plain, bytes: Data)`；`onMessage` 只把 blob 交给 `asset.chunk` 的处理，别的类型丢弃；`send` 按该 Mac 的 `caps.binary` 选格式（握手前、断线后 JSON，`hello` 恒 JSON）。
- `AHCore/PhoneEngine+Sync.swift`：`hello` 带 `binary: true`；`hello.ack` 的 `caps.binary` 随握手重置并保存。
- `AHCore/PhoneEngine+Transfers.swift`：`onAssetChunk(macId, chunk, bytes)` 直接收字节（旧路径从 `chunk.data` 解 base64 后进同一个函数）；`performUpload` 协商后用 blob 帧发 `upload.chunk`。
- 测试：`AHCoreTests/TestMac.swift` 加 `binary` 开关（假 Mac 能收二进制、能回二进制与 blob 块）；`TransferTests.swift`、`TrafficReductionTests.swift` 加协商时机（握手前 JSON、`hello.ack` 后二进制）、`upload.chunk` 与 `asset.chunk` 的 blob、旧 Mac 回退（`caps.binary` 缺省）、两种格式都收、畸形帧被忽略且不影响后续。
- 真实 daemon 端到端（`AHE2ETests`）：二进制往返、一份多块文件下载后哈希一致；`TrafficReductionE2ETests.swift` 里按字节断言的预算要重量（同步类约少 25%）并改，改预算要写依据，不是放宽。

## P2 细节（A+B：预览）

### P2-0 编译配置的底座（AT20，先做、先量）

1. 新建预览 crate（路径与名字暂定 `src-tauri/crates/ah-preview`，主 crate 以路径依赖引用）。依赖：`image`（`png`、`jpeg`、`webp`）、`ravif`（`default-features = false`、`threading`，不开汇编）、`rgb`、`imgref`、`moxcms`。对外只有一个非泛型函数，进出的是字节与普通结构体。
2. `src-tauri/Cargo.toml`：对这个 crate 与它依赖的编解码 crate，在 `[profile.release.package.<crate>]` 与 `[profile.local-install.package.<crate>]` 里各设 `opt-level = 3`（清单用 `cargo tree -p ah-preview` 定，去掉只在构建期运行的过程宏与构建脚本依赖）；写一段英文注释说明为什么（泛型实例化在调用方 crate、`z` / 0 下慢 4–20 倍，见规格 §10.3）。
3. 先只放最小实现（读字节 → 解码 → 缩放 → 编码），按规格 §10.3 的方法（同一批图、各配置交替运行）在 `release` 与 `local-install` 两档量耗时，对照表里的约 0.31 s；通过了再把完整的资格检查补进去。
4. 量真实工程里的干净构建时间（`local-install`、`release`）、二进制大小，以及 `scripts/install.sh` 里各档 target 目录的体积预算（`BUDGET_*_MB`，超了会 `cargo clean`，下次构建变慢）会不会触线（规格 §15「没验证的」第 6 条）。记进实施记录。
5. 三个平台都要能编：纯 Rust、不需要 nasm；本地只验 macOS，CI 发布矩阵的结果要看。

### P2a Mac 与线协议（Rust 与 Swift 协议同一提交）

- `ios/preview.rs`（Mac 侧的薄封装，常量集中放一处：`PREVIEW_MAX_EDGE = 1600`、`PREVIEW_QUALITY = 76.0`、`PREVIEW_SPEED = 6`、`PREVIEW_MIN_ORIGINAL_BYTES = 400 * 1024`）：`eligible(name, size)`（按 `views::mime_for` 判 PNG / JPEG / WebP 加大小，AT10）；`PreviewKey`（路径 + 大小 + 修改时间）；缓存条目 `Ready { bytes: Arc<[u8]>, sha256 }` 或 `Unavailable`；把 crate 的结果映射成「预览 / 原图（原因）」。
- `wire.rs`：`Caps.asset_preview`、`AssetRef.preview`、`AssetRef.sha256`、`AssetGet.variant`、`AssetChunk.variant`，都是缺省不进 JSON 的可选字段（AT14）；`variant` 取值常量 `"preview"` 与 `"original"`；字面构造补字段（`AssetRef` 在 `views.rs`、`history_view.rs`、`testkit.rs` 等处）。
- `views.rs`：`asset_for` 对非 `tooLarge` 的文件读一次，算 `sha256` 记进 `AssetSource.sha256`，图片按 `eligible` 设 `preview`；读不到就不带。`history_view.rs` 的 `picture_ref` 同样设 `preview`，不算哈希（AT13、AT17）。
- `gateway/assets.rs`：
  - `on_asset_get`：解析顺序不变；只有 `variant == "preview"`、Host 声明了 `asset_preview`、且 `eligible` 才走预览，其余一字不变。
  - 预览路径经 `spawn_then` 在阻塞线程里：读文件 → 算 SHA-256 并与 `AssetSource.sha256` 比对（AT19）→ 调预览 crate → 结果进缓存。
  - 缓存与并发（AT12）：键 `PreviewKey`；几十个条目、几十 MiB 封顶（暂定 32 条 / 32 MiB，满了丢最旧）；同键并发共享一次生成（`pending: HashMap<PreviewKey, Vec<等待者>>`）；同时生成数有上限（暂定 3，对照规格 §10.3 的三并发数据）。
  - 预览按字节分块发送，`index` / `total` / `size` / 首块 `sha256` 都按预览字节，`asset.chunk` 回显 `variant: preview`；回退到原图时回显 `original`。
- Host：`DaemonHost::caps`（`daemon/runtime/ios.rs`）置 `asset_preview: true`；`gateway/testkit.rs` 的测试宿主可开关，并能注入「建请求之后原文件被改」的场景。
- 夹具：`messages.json` 追加带 `preview` / `sha256` 的请求、带 `variant` 的 `asset.get` 与 `asset.chunk`、带 `assetPreview` 的 `hello.ack`；既有样本原样。
- Swift `AHProtocol`：同名字段（`AssetRef.preview` 与 `sha256`、`AssetGet.variant`、`AssetChunk.variant`、`Caps.assetPreview`）与夹具测试。
- Rust 测试（`gateway/tests_assets.rs` 等）：规格 §14 的 Rust 一条，预览用例用内存里造的小图，避免在 `dev` 档里变慢。

### P2b 手机引擎（单独一个提交）

- `AHCore/Backend.swift`：`RemoteBackend` 加 `downloadForViewing(macId:asset:progress:) async throws -> ViewableAsset`（`{ url, isPreview }`），与已有的 `download`（原图）、`downloadForExport`（原图）并列；conformer 是 `PhoneEngine`、`MockBackend`，`AppStore` 转发。
- `AHCore/PhoneEngine+Transfers.swift`：`downloads` 的键由 `assetId` 改为 `DownloadKey(assetId, variant)`；`asset.get` 带 `variant`（Mac 没声明 `assetPreview` 或资产没有 `preview` 时不带）；`onAssetChunk` 按回显的 `variant` 归位，回 `original` 的预览请求改按原图收，并入已在途的原图下载，原图到了同时满足等预览的一方（规格 §7.1）；缓存文件 `<assetId>`（原图）与 `<assetId>.preview`；已有原图就直接用原图。
- 测试（`TransferTests.swift`）：预览下载与缓存、预览到原图、Mac 回 `original`、原图已在途时并入、旧 Mac 回退（没有 `assetPreview`）；分块校验失败重试沿用现有。

### P2c 查看器界面（单独一个提交）

- `ios/AskHuman/Views/Attachments/ImageGallery.swift`（`ImageGalleryViewer`）：当前页是预览时，工具栏出现「原图 · <大小>」按钮（大小取 `AssetRef.size`）；点击下载并原位替换，进度、失败重试沿用查看器现有的错误提示；相邻页只取预览。
- `AssetViews.swift`（`AssetPage`、`named()`）：预览的临时拷贝用 `.avif` 扩展名并与原图的拷贝路径区分；分享在原图未下载时先下载再打开面板（带进度）；预览解不开（`UIImage` 为空）时自动改取原图（AT18）。
- `ImageViewers.swift`（`ZoomableImage`）按内容重绘；`AttachmentDrag.swift` 不变（拖出走 `downloadForExport`，始终是原图）。
- `AHMock`：`MockBackend.downloadForViewing` 返回缩小的生成图，部分 Mock 附件带 `preview`；`MockData` 加预览场景（含不做预览的小图与动图）。
- 本地化：新增文案以英文为源语言，`xcodebuild build` → `ios/Tools/make-catalog.py`（缺中文会失败）；新增 Swift 文件后在 `ios/` 下 `xcodegen generate`。无障碍标识：`gallery-original` 一类。
- UI 测试（Mock 场景，en + zh-Hans）：预览先行、原图按钮与进度、原位替换后按钮消失、分享先取原图、相邻页只取预览、历史里的图、无预览的图没有按钮、深色、最大字号、iPad。

## P3 细节（C+G：内容去重与本地计数）

- 手机（`AHCore`）：按 `AssetRef.sha256` 的哈希缓存（索引的存法实现时定，规格 §15）：命中就不发 `asset.get`，预览也按原文件哈希；诚实规则按 AT19——原图验过整文件哈希且与 `sha256` 一致才进哈希索引，预览的可信由 Mac 保证；已有的按 `assetId` 命名的缓存文件继续有效。测试：命中顺序（`assetId` 再哈希）、`sha256` 不符不进索引、预览与原图各一份、同内容两条请求第二次不发 `asset.get`。
- Mac（`ios/gateway/traffic.rs`、`ios/store.rs`）：`TrafficBreakdown` 里加预览 / 原图分块的条数与字节、重复取的条数与字节、`asset.get` 按来源的条数；键名实现时定，`kind_of` 认新的固定名；要求同一字节只算进一个键、各键之和等于现有的 `out:asset.chunk`。重复取的记忆（手机 + 内容键）只在内存、有上限、重启即清。`AskHuman channel ios status --json` 读得到。测试：分区性质（各键之和）、重复判断、上限。
- 不上线、不新增消息（与 T6 一致）。

## 验证清单

1. `node scripts/machine-health.mjs`；退出码 2 时先用 AskHuman 把报告告诉 Owner，长任务放后台、短间隔轮询并说明在等什么。
2. Rust：定向（`ios::`、`daemon::runtime`、预览 crate）→ 全量 `cargo test`；`cargo clippy --all-targets -- -D warnings`；`cargo fmt --check`。
3. `./scripts/install.sh`（`local-install` 档）；装好后按规格验收 15 量一次预览耗时，再单独编一个 `release` 档量一次。之后的提问走新装的 `AskHuman`。
4. **真实 daemon 端到端**（`AH_E2E_BIN=<已安装的 AskHuman> AH_E2E_BROKER_DIR=<ios/Tools/local-broker> swift test --filter AHE2ETests`）：二进制往返；预览下载、解码、长边与大小；原图哈希一致；新旧互配四格（用能力位开关模拟旧端）；计数。**不在真实 EMQX 上跑大附件用例**（会消耗额度）。
5. **iOS**：`swift test`（`ios/Packages/AskHumanKit`）；`xcodebuild test`（模拟器，en + zh-Hans，含 iPad、深色、最大字号）：新用例加与附件、查看器、历史里的图、拖出有关的既有用例。别的会话的未提交改动可能让整套编译或个别用例失败：先判断是不是它们造成的，写进实施记录，不替它们修。
6. **真实 Mac + 模拟器**：用新装的 `AskHuman` 带几张真实截图问问题；模拟器配对到这台 Mac，打开一张约 3 MB 的图：看到预览、点「原图」、分享拿到原文件；`trafficBreakdown` 里的增量对照规格 §10.1。真机与 TestFlight **不出**，做完后单独问 Owner（出包按 `ios-testflight` skill，默认 Internal 与 Friends 两组）。
7. **文档同步**：见下一节。

## 文档同步（P4）

- `docs/specs/ios-channel.md`：§6.5 附件（`variant`、预览、哈希）与 §8 协议与安全（二进制信封、blob 帧、协商字段、AAD 不变）。
- `docs/overview-ios.md`：线协议与附件的新职责（`preview.rs`、预览 crate、二进制信封、计数）；「成本红线」一条补一句预览与二进制。
- `docs/specs/ios-traffic-reduction.md`：「二进制信封」不再是「不做」，改成已由附件流量收敛完成，附实测。
- `docs/specs/ios-attachment-traffic.md`：状态、实施记录、与实现的偏差。
- `docs/PROGRESS.md`：「待办」一节按完成情况改写，实测之后换成「待核验」。
- 手机端的真实数字（`docs/wiki/ios-setup*.md` 里的月用量说法）：等真机跑几天、读到实测之后再改，不在这一轮。
- `docs/overview.md`：新增的预览 crate 若改变了整个仓库的地图才改，否则不动。不涉及 `ipc/mod.rs`、`recovery.rs`。

## 提交（Conventional Commits，英文、祈使句；用户可见的只有第 6 条）

1. `docs(ios): spec and plan for attachment traffic reduction`（P0）
2. `feat(ios): add binary envelopes and blob frames to the wire protocol`（P1a，Rust + Swift + 夹具，`Release-Note: skip`）
3. `feat(ios): send and receive binary frames in the phone engine`（P1b，`Release-Note: skip`）
4. `build(ios): add the preview crate and optimize its codecs in release and local builds`（P2-0，Cargo 配置加最小实现；`build` 不进更新说明）
5. `feat(ios): serve downscaled AVIF previews of image attachments`（P2a，Rust + Swift 协议 + 夹具，`Release-Note: skip`）
6. `feat(ios): show previews first in the image viewer and fetch originals on demand`（P2b–P2c，带 `Release-Note:`，一句话写成用户语言，数字等实测后再写）
7. `feat(ios): reuse attachments by content hash and count preview and original traffic`（P3，`Release-Note: skip`）
8. `docs(ios): sync overview and specs with attachment traffic reduction`（P4）

写计划之前已有、尚未提交的四份文档改动（流量核对的实测：`docs/PROGRESS.md`、`docs/specs/ios-traffic-reduction.md`、`docs/wiki/ios-setup.md`、`docs/wiki/ios-setup.en.md`）与 P0 的文档是同一批未提交的文件，`docs/PROGRESS.md` 里两件事的段落交织在一起；要分开提交就得用 `git add -p` 逐 hunk 选，合成一个 `docs(ios)` 提交也可以，提交前问 Owner。

## 风险

1. **编译配置（AT20）**：漏设任何一处（预览 crate、它的依赖、任一档）预览就会慢 3–20 倍而且没有任何报错；靠 P2-0 先量、验收 15、`Cargo.toml` 里写明原因的注释来防。`ravif`、`rav1e` 及其传递依赖会进 `Cargo.lock`，要看一眼；真实工程里的编译时间、二进制大小、target 目录体积预算未量（规格 §15「没验证的」第 6 条）。
2. **成帧与加密的细节错误**（二进制信封、blob 帧、AAD）：夹具加畸形向量、两边互验；`PROTO` 不变、两种格式永远都收，出问题时手机不声明 `binary` 就回到 JSON。
3. **预览的 CPU 与内存突发**：同时生成上限、解码内存上限、缓存封顶；笔记本上短暂占核是已知局限（规格 §11）。
4. **流量预算类测试会变**：同步消息约少四分之一，`TrafficReductionE2ETests` 等按字节断言的用例要重量。
5. **哈希缓存的诚实性（AT19）**：「键说是 A、内容其实是 B」会让手机显示错的图；手机验哈希、Mac 的哈希守卫两层，各有用例。
6. **建视图时的哈希开销**：`asset_for` 对每个附件多读一次整文件；附件很多的请求实测一次，太慢就只给前 N 个附件带 `sha256`（规格 §15「实现时再定」第 6 条）。
7. **`AssetPage.named` 的临时拷贝**：预览与原图是同一个 `assetId`，拷贝路径要区分，否则查看器会读到旧的那份。
8. **别的会话的改动**：见开头；每个阶段提交前 `git status` / `git diff` 核对，只 `git add` 自己的文件；`Cargo.toml` 与 `Cargo.lock` 若别的会话也改了，先停下来看。
9. **机器很忙**：全量编译（新增 `rav1e`）与模拟器测试可能要十几分钟；开发中只跑定向测试，阶段结束跑完整的。

## 实施记录

（尚未开工。）
