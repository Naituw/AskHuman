# 计划：iOS 附件流量收敛（预览优先、二进制传输、内容去重）

> 需求：`docs/specs/ios-attachment-traffic.md`（决策编号 AT1–AT20 均指其 §3）。状态：**2026-11-21 起草并开工，进度见文末「实施记录」**。规格里写规格时补的 AT14–AT20（AT20 动到 `Cargo.toml` 的编译配置）已由 Owner 确认；规格与本计划一并发给他看过，没有提出修改。下文各阶段的文件名与函数名是起草时按现有代码读出来的设想，**开工后实际以文末「实施记录」为准**。
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

### 工作位置

2026-11-21 Owner 选了**主工作树**开发（不开 Dev Instance）。主工作树里别的会话的未提交改动（弹窗文件链接）原样留着，本功能只提交自己的文件。

### P1a 线协议（已完成）

按计划做完，与计划设想不同的地方（以实现为准）：

- **名字**：Rust 是 `Frame`（解码后的字段）、`Format`（`Json` / `Binary`）、`Opened`（`Message` / `Blob { head, bytes }`）、`parse_frame`、`parse_frame_from_phone`、`open_frame`、`open_message`、`seal_plain_as`、`seal_blob`；没有单独的 `can_use_binary`——「发送方是不是规范 id、`kid` 是否放得下」就是 `Frame::encode_binary()` 返回 `None`，调用方据此回退 JSON（AT16）。Swift 是 `EnvelopeFrame`、`WireFormat`、`OpenedMessage`、`AHEnvelope.parseFrame` / `openFrame` / `openMessage` / `sealBlob`、`Plain.sealed(…, format:)`（缺省 `.json`，旧调用点不变）、`Bytes.base32LowerDecode` / `idToBytes`、`Wire.blobHeaderMaxBytes`。
- **`AssetChunk.data`、`UploadChunk.data`**：解码时缺省为空（Rust `#[serde(default)]`，Swift 新的 `@DefaultEmptyString`），序列化规则不动，JSON 旧路径逐字节与以前相同；blob 的头部是「消息去掉 `data` 键」（Rust 在 `send_chunk` 里去掉，Swift 是 `Plain(blobHeader:nowMs:)`）。**更正**：第一版提交（`6493250c`）给这两个字段加了 `skip_serializing_if = "String::is_empty"`，想让头部直接由结构体序列化得到；但 0 字节的文件会发出唯一的一块空数据，JSON 消息里没了 `data`，读不到这个键的旧手机会解不开它。后一个提交改回「序列化不动、头部去键」，并为空文件补了两条测试（JSON 路径保留空的 `data`，blob 路径是零字节的帧）。同一原因，`messages.json` 里原先追加的三条「blob 头部」样本（没有 `data` 的 `asset.chunk` / `upload.chunk`）撤掉了：它们过不了严格的类型往返；头部由 `crypto.json` 的 `blobFrames` 覆盖（两边都用类型模型读一遍，`data` 取缺省）。
- **协商**：Mac 向手机发二进制的条件是 `rt.binary && rt.hello_at.is_some()`（`hello.binary && caps.binary`，以最近一次 `hello` 为准，随下线的 presence 一起清掉）；手机向 Mac 发二进制只在本次握手的 `hello.ack` 带 `caps.binary` 之后（Swift 一侧在 P1b）。`hello`、配对主题、presence、push 恒为 JSON；配对主题上的二进制帧不收。
- **夹具（只追加）**：`crypto.json` 加 `binaryEnvelopes`（3 条，含 `kid` 大于 255）、`compressedBinaryEnvelopes`（1 条，只开不比，同 `compressedEnvelopes`）、`blobFrames`（2 条，字节里有 0、`{`、大于 0x7f 的值）、`malformedFrames`（23 条：解析层、认证层、blob 读取层各一批）；`messages.json` 加 2 条样本（带 `binary` 的 `hello` 与 `hello.ack`）。两边对同一份文件通过：Rust 字节对字节比对，Swift 打开 Rust 封的帧并用同样的 nonce 封出同样的字节。
- **节省**：一块满的 192 KiB 分块，blob 帧实测约为 JSON 信封的 56%（JSON 里是 base64 套两层；测试断言 < 60%）；一条小消息的二进制信封测试断言 < JSON 信封的 75%。
- **验证**：`cargo clippy --all-targets -- -D warnings` 干净；全量 `cargo test` 3560 通过 / 0 失败 / 12 忽略（含下一条更正提交的两条测试后是 3562）；`swift test --filter AHProtocolTests` 78 通过；`./scripts/install.sh` 装好（`local-install` 档）。
- **一次格式化的副作用**：开发中跑过一次整包的 `cargo fmt`，顺带重排了另一个会话的两个未提交文件（`src-tauri/src/attachment_markdown.rs`、`src-tauri/src/app/linked_files.rs`），只改了排版、没有改逻辑。之后只对自己的文件用 `rustfmt`，整包只用 `cargo fmt -- --check`。

### P1b 手机引擎（已完成）

按计划做完，与计划设想不同的地方（以实现为准）：

- **收**（`PhoneEngine.openFromMac`）：改走 `AHEnvelope.parseFrame` + `openMessage`，两种格式任何时候都收；返回 `(macId, plain, blob)`。blob 只认 `asset.chunk` 且头部里没有 `data`（头部自己也带 `data` 是「两条消息塞在一条里」），别的一律丢，跟 Mac 一侧 `open_up` 对称；丢弃不影响后续。`dispatch` 多一个 `blob` 参数，`onAssetChunk(macId, chunk, blob:)` 优先用 blob 的字节，没有再解 `chunk.data`（旧路径一字不动）。
- **发**（`send`、新增 `sendUploadChunk`）：是否发二进制只由 `MacRuntime.takesBinary` 决定，它是 `sync.isHandshaken && caps?.binary == true`。握手前是 `.none` / `.helloSent`，所以 `hello` 恒为 JSON、握手前的一切也是 JSON；`resetSession()` 把 `sync` 置回 `.none`，所以断线、Mac 下线、Mac 重启后的新 `hello`（新 epoch）都会让手机回到 JSON，直到本次会话的 `hello.ack`，旧会话的 `caps` 不会带过来（不需要为此另写重置）。`upload.chunk` 协商后走 blob 帧（`Plain(blobHeader:)` 去掉 `data`，字节原样跟在后面），封不出来（id 不规范、超长）就回退成带 base64 `data` 的 JSON 消息，与以前逐字节相同。
- **hello**：`binary: true`（`PhoneEngine+Sync.swift`）。`PhoneEngine+Pairing.swift` 与 `AHStore/PushDecryptor.swift` 不动，仍只认 JSON（配对主题、push 恒为 JSON）。
- **测试夹具**（`TestMac`）：`caps.binary` **缺省开**，与真实 Mac 一致，所以 `AHCoreTests` 里原有的三百多条用例现在都是走二进制路径的回归；关掉它就是旧版 Mac——只读 JSON 信封，二进制到它那里等于乱码（所以手机若误发二进制，用例会失败而不是悄悄通过）。新增 `blobChunks`（关掉时块是二进制信封里的 JSON 消息）、`Arrival`（每条消息到达的格式）、`sendBytes` / `downKeys`（手工伪造帧）、`inboxRevision`（让伪造的 `request.resolved` 是手机下一条会采用的那个修订号，真漏进去就看得见）。
- **新增用例**（没有按计划放进 `TransferTests` / `TrafficReductionTests`，单独一个 `AHCoreTests/BinaryTransportTests.swift`，9 条）：`hello` 是 JSON 且带 `binary`；握手前 JSON、本次 `hello.ack` 后二进制、Mac 重启后在新 ack 之前又回到 JSON；Mac 回来时换成旧版（`caps.binary` 没了）则又发 JSON；旧版 Mac 全程 JSON（上传、下载都成）；上传的每块是 blob 帧；下载的 19 块是 blob 帧；块是二进制信封里的 JSON 消息也收；空文件的那一块在 blob、二进制里的 JSON、旧版 JSON 三种下都下载成功；七种伪造帧（别的消息当 blob、头部带 `data`、别的主题、别的密钥、翻一位、截断、只有魔数）全部丢弃，不改收件箱、不让在等的下载提前完成，之后两种格式的消息仍照常处理。
- **对照实验**（用完即还原）：三处各改一遍再跑这九条，失败的恰好是预期的那几条。`takesBinary` 忽略 `caps`：「旧版 Mac 全程 JSON」与「Mac 回来时换成旧版」两条失败（这两条在失败前先发一条 `ping` 并确认旧版 Mac 读到了，所以是立刻失败而不是等超时挂住）；`takesBinary` 恒 `false`：协商时机、「Mac 回来时」、上传 blob 三条失败；去掉 blob 头部的校验：伪造帧一条失败。第一次做最后一个实验时伪造帧用例没有逮到「别的消息当 blob」，原因是探针里的修订号不连续、手机本来就会因此重新同步，改成下一个修订号之后才逮得到。
- **真实 daemon 端到端**：新增 `AHE2ETests/BinaryTransportE2ETests.swift`（一个一次性的 Mac daemon + 本地 broker，不碰真实 EMQX）：600 000 字节的文件下载 4 块，线上 600 872 字节（比文件多 0.15%）；400 000 字节上传 3 块，线上 400 531 字节；Mac 自己的 `trafficBreakdown`（`out:asset.chunk`、`in:upload.chunk`）按线上字节数记，两个方向的文件都逐字节一致；JSON + base64 会是文件的约 1.33 倍。整套 `AHE2ETests` 72 条：64 通过、8 跳过（真实服务、模拟器、付费 Codex 要另外的环境变量）、0 失败。
- **预算重量**（`TrafficReductionE2ETests`，AT 里说的「改预算要写依据」）：同一个用例，手机声明 `binary` 时前台进入一次 2 236 字节（Agent 快照 526），不声明（Mac 回 JSON）时 3 223 字节（742），少 31%（快照少 29%）；各只跑了一次。上限从 6 000 降到 4 000，保持原来对 JSON 数字大约 1.9 倍的余量，理由写在用例的注释里。这是收紧，不是放宽。
- **验证**：整包 `swift test`（不含 E2E）通过：Transport 14、Store 39、Protocol 78、Markdown 16、Core 313、App 556，共 1 016 条，0 失败；E2E 用的是更正提交之后 `./scripts/install.sh` 装好的二进制（此后 Rust 没有再改）。

### P2-0 编译配置的底座（已完成，提交 `89490560`；工作区与测量记录随 P2a 提交补齐）

按计划做完，与计划设想不同的地方（以实现为准）：

- **crate**：`src-tauri/crates/ah-preview`，主 crate 以路径依赖引用。对外只有一个非泛型函数 `make_preview(&[u8], &PreviewParams) -> Result<Preview, Skip>`，进出的都是字节与普通结构体（长边、质量、速度、源图像素上限由调用方传，常量放在 Mac 一侧）。依赖 `image`（`png`、`jpeg`、`webp`）、`ravif`（`default-features = false`、`threading`，不开汇编）、`rgb`、`imgref`、`moxcms`。
- **opt 3 的清单只写一份，两档都生效**：计划是 `[profile.release.package.<crate>]` 与 `[profile.local-install.package.<crate>]` 各一份；实际只有 `[profile.release.package]` 一份（51 项，从 `cargo tree -p ah-preview` 里去掉只在构建期运行的过程宏与构建脚本依赖），因为 `local-install` 是 `inherits = "release"`，**包级覆盖一并继承**，实测 `local-install` 下预览同样是 0.32 s。效果与计划相同，少一份要同步维护的清单；`Cargo.toml` 的注释写明了原因，也写明「只设 `ah-preview` 不够（0.8 s 与 2.5 s）」。
- **耗时实测**（规格 §10.3 的方法：同一批 12 张图，每张每个配置各起一个进程、配置之间交替运行）：设了覆盖的 `release` 0.32 s、`local-install` 0.32 s，全程 opt 3 的对照 0.32 s；不设覆盖时 `release` 1.19 s（×3.7）、`local-install` 6.9 s（×21.6）。与规格 §10.3 的小工程数据一致。装机版与 `release` 版的端到端耗时（验收 15）在 P4 再量。
- **工作区**（计划没写，P2a 提交里加）：`src-tauri/Cargo.toml` 加了 `[workspace]`：`members = ["crates/ah-preview"]`、`default-members = [".", "crates/ah-preview"]`，`vendor/drag` 与 `vendor/tao` 放进 `exclude`，和以前一样留在工作区外。原因：不是成员的路径 crate 带着 dev-dependencies 时，Cargo 不让在这里测它，CI 与 `AGENTS.md` 里的 `cargo test`、`cargo clippy --all-targets`、`cargo fmt --check`（都在 `src-tauri/` 下、或用 `--manifest-path src-tauri/Cargo.toml`、不带 `-p`）也就覆盖不到预览 crate。设为默认成员后命令一字不改就覆盖到；`./scripts/install.sh` 照常用（已验证：装机成功、`cargo-artifact.mjs` 找得到 `AskHuman`）。
- **干净构建、体积、target 预算**（计划 P2-0 第 4 条，规格 §15「没验证的」第 6 条）：同一台机器上，各档各干净构建一次，源码分别是预览 crate 之前（`76d85f78`）与 P2a 完成后的树；`nice` 后台跑，CPU = user + sys。后台还有别的会话的负载（四次开始时负载 4、11、13、18），墙钟只作参考，CPU 时间更可信：

  | 档位 | | 墙钟 | CPU | 二进制 | target 的 profile 目录 |
  |---|---|---|---|---|---|
  | `local-install` | 之前 | 66.9 s | 329 s | 59.7 MiB | 2416 MiB |
  | | 之后 | 79.8 s（+19%） | 429 s（+30%） | 60.2 MiB（+0.5，+0.9%） | 2515 MiB（+99） |
  | `release` | 之前 | 94.0 s | 659 s | 20.4 MiB | 2416 MiB |
  | | 之后 | 96.8 s（+3%） | 695 s（+5.5%） | 21.8 MiB（+1.4，+7%） | 2524 MiB（+108） |

  干净构建只在依赖第一次编译或被清理后发生，增量构建不重编依赖。`release` 二进制多约 1.4 MiB，比规格 AT2 估的「约 0.9 MB」多一些（没有再追查构成）。
  - `scripts/install.sh` 的预算（主工作树：`local-install` 4096、`dev` 6144、`full-debug` 6144、`release` 4096 MB）：这次装机真的触发了清理——`local-install` 5866 MB → 整个 profile 清空，`dev` 11709 MB → 3406 MB（只清本项目产物就够了）。前者是一次性的：自上次装机（P1b）以来，几乎所有三方依赖都重编了，新旧两套产物叠在一起超了预算（我没有分开验证成因：这期间 `Cargo.toml` 改了两处——`[profile.release.package]` 里有 `libc`、`log`、`cfg-if`、`memchr` 这类处在依赖图底部的 crate，改它们的优化等级会让依赖它们的产物都重编；以及 `[workspace]`）。清空之后下一次装机整个重编（本机约 1 分 14 秒），此后日常目录约 2.5–3 GB，在预算内。`dev` 档里本项目产物 3.1 GB + 增量 5.1 GB 是平日 `cargo test` / `clippy` 攒的，三方依赖 3.2 GB 其中新增的 392 MiB（`ravif` 137、`ah-preview` 62、`rav1e` 60 等）；清完本项目产物约 3.4 GB。**没有任何一档在清完之后仍超预算。**
- **依赖审计**：`Cargo.lock` 新增 31 个外部 crate（`rav1e`、`ravif`、`av1-grain`、`v_frame`、`rayon`、`rayon-core`、`crossbeam-deque`、`crossbeam-epoch`、`nom`、`paste`、`rgb` 等），`itertools` 另多一个 0.12.1。本机没装 `cargo audit`（CI 的 `audit` 任务会跑），改为把 RustSec 公告库（2026-11-21 的快照）按 crate 名逐个比对：命中 4 条，其中三条的版本都在已修复范围内（`crossbeam-deque 0.8.8`、`crossbeam-epoch 0.9.21`、`rgb 0.8.53`），另一条是 `paste 1.0.15` 的「不再维护」（RUSTSEC-2024-0436，informational，没有修复版本，由 `rav1e` 引入）。`cargo audit` 对这类只给警告、不让 CI 失败（工作流里没有 `--deny warnings`）。没有未修复的漏洞；CI 上真正的 `cargo audit` 结果要看。
- **三个平台**：只在 macOS 上编过；`rav1e` 不用汇编、没有 `nasm` 依赖，Windows 与 Linux 要看 CI 发布矩阵的结果。

### P2a Mac 与线协议（已完成）

按计划做完（Rust、Swift 协议、夹具同一提交），与计划设想不同的地方（以实现为准）：

- **预览 crate 补全**：资格检查全在 crate 里——解码上限（源图 5000 万像素、解码分配 256 MiB、边长 32 768 px）；带 alpha 通道的图要扫一遍是不是真有透明像素（macOS 截图带着全不透明的 alpha 通道，不能因此放弃）；EXIF 方向转正；Lanczos3 缩到长边 ≤ 1600、不放大；带 ICC 的 RGB 图经 `moxcms` 转 sRGB；`ravif` 4:4:4、质量 76、速度 6、`rayon` 多线程、不用汇编；不比原图小就放弃。放弃的原因是 `Skip` 的七个取值（`Undecodable`、`TooLarge`、`Animated`、`Transparent`、`UnsupportedColor`、`EncodeFailed`、`NotSmaller`），Mac 一侧一律发原图。规格 §15「实现时再定」第 4 条（非 sRGB 转换做不到）定为**发原图**（`UnsupportedColor`），不接受色偏照发：预览只在「同一幅画面」时才替换原图。crate 自带 15 条测试，用真实编码器：输出能被 `image` 重新解码、尺寸与长边、各类放弃原因、方向、ICC。
- **`ios/preview.rs`**（Mac 侧）：常量集中在这一个文件（`PREVIEW_MAX_EDGE = 1600`、`PREVIEW_QUALITY = 76.0`、`PREVIEW_SPEED = 6`、`PREVIEW_MIN_ORIGINAL_BYTES = 400 * 1024`、`PREVIEW_MAX_SOURCE_PIXELS = 50_000_000`）；`eligible(name, size)` 按类型（PNG / JPEG / WebP）与大小（400 KiB 到 10 MiB）；`PreviewMaker` 特征让网关测试注入假预览器；缓存键 `PreviewKey { path, size, modified }`，有界先进先出（32 条 / 32 MiB，「做不了预览」的结论每条按 256 B 计，单条比总量上限还大的不留）；同键并发共用一次生成（`pending` 里挂等待者）；同时生成数 3（`Semaphore`）；生成在 `spawn_blocking` 里：读文件（≤ 10 MiB）、算 SHA-256、调 crate。规格 §15 第 2 条的取值就是这些。
- **AT19 的核对挪到「用的时候、按请求」**（计划写的是生成时比对 `AssetSource.sha256`）：缓存与合并按文件身份（路径 + 大小 + 修改时间），同一个身份下可以有记着不同 `sha256` 的请求（一个在文件被改之前建的，另一个在之后建的），而生成只做一次，所以比对放在每个等待者拿结果的那一刻：`Made::preview_for(source)` 用该请求记的 `sha256` 与生成时读到的原文件哈希比较，不符就发原图、回显 `original`，这份预览也不用来满足那个请求。可观察的行为与规格 §6.4 第 0 步相同；用例：建请求之后文件被改，送出的是原图，之后哈希相符的请求仍复用同一次生成。「做不了预览」的结论不记原因（没有读者，记了还会触发死代码告警）。
- **`views.rs`、`history_view.rs`**：`file_sha256` 以 64 KiB 为块流式读，超过 `MAX_ASSET_BYTES` 返回 `None`；`asset_for` 对非 `tooLarge` 的文件算哈希，记进 `AssetRef.sha256` 与 `AssetSource.sha256`；读不到（文件已经不在）就不带，手机来取时得到 `notFound`；`preview` 只按类型与大小判断，不读文件。历史图的 `picture_ref` 设 `preview`、不设 `sha256`（AT13、AT17）。
- **建视图时哈希的开销**（计划风险 6，规格 §15「实现时再定」第 6 条）：一个带 20 个、每个 10 MiB（`MAX_ASSET_BYTES`）文件的请求，页缓存是热的，只量哈希：`release` 档共 0.74 s（每个 37 ms），`local-install` 档共 4.9 s（每个 246 ms，约 40 MB/s）；`ask_view` 整个与哈希几乎相同（测试用的是非图片文件，没有缩略图）。典型的请求（几张 1–3 MB 的截图，合计 5 MB 上下）在 `local-install` 档约 25 ms/MB，即几十到一百多毫秒，`release` 档几毫秒。`ask_view` 在 `spawn_blocking` 里、与取会话标题并行，不占网关，只推迟这条请求送到手机的时间。**没有加上限**：计划的备选是「只给前 N 个附件带 `sha256`」，N 取 20（与缩略图一样）时最坏情况不变，按字节限则要多一套规则，先按设计全部带上。慢的成因是 `sha2` 及它的依赖在 `local-install` 档按 opt 0 编译，**不是这次才有的**：网关每次取图本来就对整个文件算一次哈希（从第 0 块起的那次 `asset.get`，给首块的 `sha256`）。临时副本里的试验（未提交）：把 `sha2`、`digest`、`block-buffer`、`crypto-common`、`generic-array`、`cpufeatures` 也设成 opt 3，`local-install` 档 4.9 s → 0.38 s（约 13 倍）。这超出 AT20 写明的范围（「其它依赖的配置不动」），所以**没有做**，留给 Owner 决定（见最后的汇报）。
- **取图**（`gateway/assets.rs`）：`serve_asset` 先判 `tooLarge`；`variant` 缺省走原来的原图路径（逐字节与以前相同，块里没有 `variant` 键）；`variant: preview` 且该 Host 声明了 `asset_preview`、资产满足 `eligible` 才走预览：先 `stat` 取 `PreviewKey`，命中缓存就直接发，否则登记等待者、必要时起一次生成；别的 `variant` 值回原图并回显 `original`。预览按预览字节分块，`index` / `total` / `size` / 首块 `sha256` 都是预览的，回显 `variant: preview`；预览生成期间网关照常处理别的消息（用例：把预览器挂住，`ping` 与别的 `asset.get` 照答）。
- **Host 与线协议**：`DaemonHost::caps` 置 `asset_preview: true`；测试宿主有开关，预览器可注入 `FakePreviews`（可挂起、数调用次数与最大并发、脚本化「预览 / 放弃原因」）。`Caps.assetPreview`、`AssetRef.preview` / `sha256`、`AssetGet.variant`、`AssetChunk.variant` 全是缺省不进 JSON 的可选字段，Rust 与 Swift 同名；`AssetChunk.variant` 只在 `asset.get` 带了 `variant` 时回显。`PROTO`、`PROTOCOL_VERSION` 不变，没动 `ipc/mod.rs` 与 `recovery.rs`。夹具只追加 5 条（带预览位的 `hello.ack` 与 `request.new` 各一条、`asset.get` 一条、`asset.chunk` 两条：`preview-first`、`original-instead`），两边通过。
- **测试**：预览 crate 15；主 crate 32（`ios::preview` 12、网关预览 17、视图 / 历史视图 / 线协议各 1）；Swift `AssetPreviewWireTests` 6。网关的预览用例用 `FakePreviews`，不在 `dev` 档里跑真实编码（那一档的依赖没有 opt 3，一张图要好几秒）；真实编码由 crate 自己的 15 条与之后 P4 的端到端覆盖。网关预览用例用真实时钟的 `#[tokio::test]`，不用 `start_paused`：暂停的时钟在运行时空闲、等 `spawn_blocking` 的线程完成时会自动拨到下一个定时器，让带超时的断言误触发。
- **验证**：`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings` 干净；全量 `cargo test`：主 crate 3594 通过 / 0 失败 / 12 忽略，预览 crate 15 通过；`swift test --filter AHProtocolTests` 84 通过；`./scripts/install.sh` 装机成功（`local-install` 档，见 P2-0 里的预算清理）。

### P2b 手机引擎（已完成）

按计划做完，与计划设想不同的地方（以实现为准）：

- **取哪种变体只有一个入口**（`PhoneEngine+Transfers.swift` 的 `fetch(macId:asset:preferPreview:progress:)`）：`download` 与 `downloadForExport` 永远是原图（`preferPreview: false`），`downloadForViewing` 是 `preferPreview: true`。顺序：已有原图 → 直接用；`preferPreview` 且 `asset.preview` 且已有预览缓存 → 直接用（不看链路状态：Mac 离线时已经取过的预览照样看）；`preferPreview` 且 `asset.preview` 且本次会话里 Mac 声明了 `assetPreview`（读的是 `macs[macId]?.caps`，每次会话的 `hello.ack` 重置，不用另写重置）→ 发 `variant: preview`；其余一律原图。`ViewableAsset { url, isPreview }` 在 `AHCore/ViewableAsset.swift`（单独一个小文件）。`Backend.swift` 里 `RemoteBackend` 的协议要求之外，在 `public extension RemoteBackend` 里给了默认实现（返回原图），所以只有真正会做预览的 `PhoneEngine` 要重写；`MockBackend` 到 P2c 再重写。
- **按「`assetId` + 变体」分键**：`DownloadKey` 与 `AssetVariant`（`original` / `preview`，原始值就是线上的字符串）是 `PhoneEngine+Transfers.swift` 里的内部类型；定时器键、窗口重发、超时、`resumeTransfers`（重连后补发）都按这个键。每个 `asset.chunk` 靠回显的 `variant` 归位：没有 → 原图（旧 Mac 全是这样）；`preview` → 预览；`original` → `adoptOriginal`；别的值 → **丢弃**（手机没要过的东西，不猜）。
- **Mac 回原图**（`adoptOriginal`）：预览下载还没收到任何块时，把它改成原图下载——已有同一资产的原图下载在途就并进去（等待者与进度回调合并，在途的那份保留），没有就以它为原图下载（沿用它的重试状态与窗口，换一个超时定时器）；之后各窗口的 `asset.get` 都不带 `variant`（只有预览是点名要的）。如果预览的块已经收到了一部分，随后又来了 `original`，说明文件在请求之间变了，按 `invalid("The file changed.")` 失败，不把两份拼在一起。同一个窗口的后几块到达时预览下载已经搬走，靠「原图下载在不在」认出来。原图下载完成时（`satisfyPreviewWaiters`）同一资产还在等预览的人拿到原图，那次还没回来的预览请求不再等。
- **错误与取消**：`asset.error` 针对资产，把它名下两种变体的下载都失败掉；取消按等待者（`cancelDownloadWaiter(_ waiter:)` 在所有下载里找，因为等待者可能已经随 `adoptOriginal` 搬了家），最后一个等待者走了就删下载与定时器。预览的 `total`、`size` 要等首块来才知道，原图的 `size` 一开始就用 `AssetRef.size`。
- **缓存文件**：`<assetId>`（原图，已有的缓存继续有效）与 `<assetId>.preview`，都经 `Data.write(options: .atomic)`；`asset.sha256` 与按内容命名的缓存是 P3。预览校验用它自己的首块 `sha256` 与 `size`，与原图的校验同一段代码。
- **测试**（计划放进 `TransferTests.swift`，实际单独一个 `AHCoreTests/AssetPreviewTests.swift`，20 条，因为那个文件已经很长）：请求带 `variant: preview` 并把预览与原图分开存、第二次看走缓存；`download` 与导出仍是原图、之后查看器看原图且不再发 `asset.get`；没有 `assetPreview` 的 Mac、没有 `preview` 位的图都不带 `variant`；Mac 离线时已有的预览照样看、没有的报离线；预览与原图同时在途（确定性的一条：把原图的首个请求丢掉，让预览先到；另一条是两个一起跑）；两个查看器合用一次下载；Mac 回原图（文件 10 块，第二个窗口不再带 `variant`、不留 `.preview` 文件、进度不倒退）、原图在途时并入、原图先到时满足等预览的人；预览校验失败不留文件；`asset.error`；`tooLarge` 不发请求；窗口丢了按预览重发；取消后再看重新开始；预览块之后跟原图块失败；陌生 `variant` 的块不影响两个在途的下载（它「恰好完整」，任何一边认了都会提前完成）；没人等的块丢弃。`TestMac` 加了 `register(preview:data:)`、`answerPreviewsWithFiles`，`caps.assetPreview` 缺省开（与真实 Mac 一致，所以原有的用例都是新 caps 下的回归），关掉就是旧 Mac：它不认 `variant`、发的是不带 `variant` 的原图，所以手机若误发 `variant`，用例看得到。等结果的地方都经 `finish(_:)`：5 秒（真实时间）后取消任务，再等 1 秒仍没结束就报「还在等」——引擎自己的定时器走手动时钟，这里没人拨，没有它出了回归就是整个测试进程挂住。
- **对照实验**（在 `/tmp` 里的一份包副本上做，不碰工作树，因为另一个会话也在编这个包；每次只改一处，跑这 20 条）：十处里每处都至少被一条用例逮到。`asset.get` 恒不带 `variant`：12 条失败；已到的原图不满足等预览的人：1 条；`asset.error` 只失败原图那一份：1 条；允许预览块与原图块混在一起：1 条；忽略 `caps.assetPreview`：1 条；陌生 `variant` 当原图收：第一版用例**逮不到**（它只在预览在途时喂块，原图那边没有下载可被误伤），改成两种下载同时在途、块「恰好完整」之后 1 条失败；`download()` 也可以返回预览缓存：6 条；预览存成原图的文件名：4 条；最后一个等待者取消时不删下载：1 条；并入时把在途的原图下载整个换掉（等待者丢了）：1 条，第一版的 `finish` 只取消任务、而丢了的等待者取消不到，这条一直挂住（7 分钟后我手工杀掉了测试进程）；改成取消后再等 1 秒就报「还在等」之后，约 6 秒内失败。
- **验证**：整包 `swift test`（不含需要 `AH_E2E_BIN` 的 E2E 用例）566 条通过、0 失败（其中包含另一个会话未提交改动带来的用例）；`PhoneEngine`、`AppStore`、`Backend` 是和另一个会话共改的文件，只暂存了我的几处。

**没有处理（留给 P2c / 之后）**：预览解不开自动取原图（AT18）在界面一侧；相邻页「只取预览」时 Mac 若回的是原图，引擎仍会把它收完（与改动前每页都取原图的行为相同，只在 Mac 做不了预览的少数情形发生，P2c 决定相邻页要不要因此跳过）；原图下载在途时同一资产再来一次查看（预览请求被 Mac 回成原图的那种图）会让 Mac 把第一个窗口再发一遍，并成一次下载，只多一个窗口的流量，没有处理。
