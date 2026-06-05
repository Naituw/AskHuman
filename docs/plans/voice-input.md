# 开发计划：弹窗语音输入（macOS 26 SpeechAnalyzer，Swift 链入）

> 关联需求：`docs/specs/voice-input.md`
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

```
前端(PopupView.vue)
  ⌘D / 麦克风按钮  ──invoke──▶ start_speech / stop_speech / flush_speech (Rust 命令)
        ▲                                   │
        │ listen 事件                        ▼
   speech-* 事件  ◀──app.emit──  Rust 桥(objc2 + block2)
        │                                   │ msg_send! / 传 block 回调(捕获 AppHandle)
   增量插入/就地替换/移动光标→flush          ▼
                                   Swift @objc 桥 (AHSpeechBridge: NSObject)
                                        │ 持有
                                        ▼
                                   SpeechEngine(整理自 demo)
                                        └─ SpeechAnalyzer / SpeechTranscriber
                                           AVAudioEngine 采集 + AssetInventory 下载
build.rs(仅 macOS 目标)
  swiftc 编 src-tauri/swift/*.swift ─▶ 静态库(libahspeech.a, 按 $TARGET 架构)
  链接: -force_load + Swift 运行时/框架 search path + autolink + rpath /usr/lib/swift
```

要点：识别全部在 Swift 内（含 async/AsyncStream）；ObjC 暴露面只用 `NSString`/`NSInteger`/block 等 ObjC 类型；回调闭包在 Rust 端捕获 `AppHandle` 转成 Tauri 事件。

---

## 1. Swift 层（`src-tauri/swift/`）

### 1.1 `SpeechEngine.swift`（整理自 `speech-demo`）
- 复用 demo 的核心逻辑：新 API 全流程（`SpeechTranscriber.supportedLocale` 校验 → `AssetInventory` 下载安装 → `bestAvailableAudioFormat` → `AsyncStream<AnalyzerInput>` 喂音频 → 结果 `AsyncSequence` → committed/volatile 回调）、`AVAudioEngine` 采集（开 Voice Processing、取 ch0 做单声道、转换到分析器格式）、`flush()`（递增 `sessionGen` 失效旧回调 + `buildNewSession` 重建会话，不动音频引擎）、`stop()`。
- **正式化整理**（去 demo 痕迹）：删除 `/tmp/speechdemo.log` 文件日志与诊断计数、删除 `SPEECH_NO_VP` 等 env 开关、仅保留新 API（移除 `SFSpeechRecognizer` 旧路径与 `SpeechAPIKind`）、错误信息规整为可直接展示的中文文案。
- 回调保持：`onCommitted(String)`、`onVolatile(String)`、`onLevel(Float)`、`onStatus(String)`、`onError(String)`，新增 `onDownloadProgress(Double)`（见 1.3）。

### 1.2 `SpeechBridge.swift`（新增 `@objc` 桥）
- `@objc(AHSpeechBridge) final class AHSpeechBridge: NSObject`（显式稳定 ObjC 类名，供 Rust 运行时按名查找）。
- 持有一个 `SpeechEngine` 实例与会话状态；所有方法在内部调度到合适线程，回调通过保存的 block 触发。
- 暴露方法（仅 ObjC 兼容类型）：
  - `@objc func start(_ localeID: NSString)`（`localeID` 为 BCP-47 或空串=跟随系统）
  - `@objc func stop()`
  - `@objc func flush()`
  - `@objc static func requestAuth(_ completion: @convention(block) (Bool, Bool) -> Void)`（语音+麦克风授权）
  - `@objc static func isAvailable() -> Bool`（运行期 `if #available(macOS 26, *)` 判定）
- 暴露 block 回调属性（`@convention(block)`，参数用 `NSString`/`Double`/`Float`）：
  `onCommitted` / `onVolatile` / `onLevel` / `onStatus` / `onError` / `onDownloadProgress`。
- 版本门：所有触达 `SpeechAnalyzer` 的代码用 `if #available(macOS 26, *)` 包裹；低于 26 时 `start` 直接走 `onError("需要 macOS 26")`（前端实际会先用 `isAvailable` 隐藏按钮，这里兜底）。

### 1.3 模型下载进度
- `AssetInventory.assetInstallationRequest(...)` 返回的请求带 `Progress`；KVO/轮询 `fractionCompleted` → `onStatus("下载语言模型… N%")` 或 `onDownloadProgress(fraction)`。
- 下载期间标记 busy，拒绝重复 `start`；失败 → `onError(可读文案)` 并复位。

---

## 2. `build.rs`：编译并链接 Swift 静态库（仅 macOS 目标）

在现有 macOS 分支（Quartz 链接 + Info.plist 段嵌入）之后新增：

1. **架构映射**：读 `CARGO_CFG_TARGET_ARCH`（`aarch64`→`arm64`，`x86_64`→`x86_64`）拼 `-target {arch}-apple-macosx26.0`。
2. **取 SDK**：`xcrun --sdk macosx --show-sdk-path`；`xcrun -f swiftc` 取编译器。
3. **编译**：`swiftc -O -static -emit-library` 或 `-emit-object` 后 `libtool/ar` 归档为 `libahspeech.a`（产物落 `OUT_DIR`），输入 `src-tauri/swift/*.swift`，带 `-target`、`-sdk`、`-module-name ahspeech`、`-parse-as-library`。
4. **链接参数**（`cargo:rustc-link-search` / `cargo:rustc-link-arg`）：
   - `-L {OUT_DIR}` 且 `-Wl,-force_load,{OUT_DIR}/libahspeech.a`（**关键**：保证 `@objc` 类被注册、不被裁剪）。
   - Swift 运行时/标准库 search path：`{TOOLCHAIN}/usr/lib/swift/macosx` 与 SDK 内 swift 库路径；优先依赖 Swift 对象内嵌的 autolink 指令（`LC_LINKER_OPTION`）自动带出 `swiftCore` 等。
   - 框架：`Speech`、`AVFAudio`、`AVFoundation`、`Foundation`（autolink 通常已含，缺失再显式 `-framework`）。
   - `-Wl,-rpath,/usr/lib/swift`（运行时从系统解析 Swift 运行时）。
5. **重编触发**：`cargo:rerun-if-changed=src-tauri/swift`。
6. 非 macOS 目标完全跳过本节。

> 风险点（最先打通）：force_load + Swift 运行时 search path + x64 交叉链接。**M1 里程碑**先在本机分别验证 arm64 与 x86_64 链接通过。

---

## 3. Rust 层

### 3.1 删除与依赖清理
- 删除 `src-tauri/src/macos_speech.rs` 与 `main.rs` 的 `mod macos_speech;`。
- `Cargo.toml` 移除 `objc2-speech`、`objc2-avf-audio`；保留 `objc2`/`objc2-foundation`/`objc2-app-kit`/`block2`。

### 3.2 新增桥模块 `src-tauri/src/speech.rs`（仅 macOS 编译）
- 用 `objc2` 运行时按名取类：`AnyClass::get(c"AHSpeechBridge")`，`msg_send!`[cls, new] 实例化（`Retained<AnyObject>`）。
- 为每个回调用 `block2::RcBlock::new(move |arg| { app.emit("speech-…", payload) })` 创建 block，`msg_send!` 设到桥对象属性；**RcBlock 与桥对象一并存入会话结构体保活**（参照原 `macos_speech.rs` 保活 block 的做法）。
- 提供 `start(app, locale)` / `stop()` / `flush()` / `is_available()`；会话状态用 `Mutex`/`OnceLock` 持有当前桥对象与 blocks，stop 时释放。
- 线程：block 可能在 Swift 音频/识别线程回调，闭包内只做 `app.emit`（线程安全）。

### 3.3 命令（`commands.rs` + `app/mod.rs` 注册）
- 改造现有 `start_speech`/`stop_speech` 调到 `speech::`；新增 `flush_speech`、`speech_available`（前端据此 gating）。
- 在 `generate_handler!` 注册 `flush_speech`、`speech_available`。

### 3.4 事件契约（Rust→前端）
| 事件 | payload | 含义 |
|---|---|---|
| `speech-committed` | `string`（增量） | 已最终化片段，插入光标处 |
| `speech-volatile` | `string` | 当前实时片段，就地替换 |
| `speech-level` | `number` | 输入电平峰值（动效） |
| `speech-status` | `string` | 人类可读状态（聆听中/区域/下载中…） |
| `speech-download` | `number`（0–1） | 模型下载进度 |
| `speech-error` | `string` | 错误文案（不支持/下载失败等） |
| `speech-stopped` | `void` | 会话结束（前端复位 UI） |

---

## 4. 前端

### 4.1 `src/lib/ipc.ts`
- 新增 `flushSpeech()`、`speechAvailable()`；保留 `startSpeech`/`stopSpeech`。

### 4.2 `src/views/PopupView.vue`（重写语音逻辑，复刻 demo 插入模型）
- **可用性 gating**：挂载时 `speechAvailable()`，false → 隐藏/禁用麦克风按钮并 tooltip「需 macOS 26」。
- **触发**：麦克风按钮 + ⌘D 开始/停止；录音态高亮呼吸。
- **插入模型**（对 `<textarea>`）：
  - 记录 `regionStart`（开始/每次 flush 后的光标位置）与当前 volatile 区间 `[interimStart, interimLen]`。
  - `speech-committed`：在 volatile 区起点插入增量、固定下来、前移基点。
  - `speech-volatile`：替换 `[interimStart, interimLen]` 区间为最新片段。
  - 监听 textarea 的 `selectionchange`/光标变动：若**用户在会话中移动光标** → 固定当前内容、`invoke flush_speech()`，新基点=新光标。
- **状态/下载**：`speech-status`/`speech-download`/`speech-error` 显示在麦克风旁状态/错误行；下载中禁用重复触发。
- **结束**：`speech-stopped` 复位录音 UI。

### 4.3 设置页
- 「语音识别语言」下拉沿用现状（`speechLanguage`，`config.rs::speech_language`），开始识别时读取传给 `start_speech`。无需改动数据结构。

---

## 5. CI（`.github/workflows/build.yml` + `release.yml`）
- 两个 Apple 目标 `os: macos-14` → **`macos-26`**（含 26 SDK；x64 仍在该 arm64 runner 上交叉编译）。
- （可选，保可复现）用 `sudo xcode-select -s /Applications/Xcode_26.x.app` 或 `maxim-lobanov/setup-xcode` 固定 Xcode 版本。
- Windows/Linux job 不变。

---

## 6. 里程碑

- **M1 链接打通**：最小 Swift 桩 + build.rs，本机 `cargo build` 对 `aarch64-apple-darwin` 与 `x86_64-apple-darwin` 均链接通过（force_load/运行时/交叉链接）。
- **M2 识别跑通**：整理 `SpeechEngine` + `AHSpeechBridge`，Rust 桥转发事件，弹窗能实时出字（先不管插入精细度）。
- **M3 插入模型**：committed/volatile/移动光标 flush 在 textarea 完整复刻 demo 行为。
- **M4 下载交互**：模型下载中/进度/失败 UI 完整。
- **M5 收尾**：删旧实现与依赖、macOS<26 gating、CI 升 macos-26、双架构验证、文档/`--help` 同步（如涉及）。

## 7. 风险与回退
- **链接（最高）**：force_load / Swift 运行时 search path / x64 交叉链接。M1 先行验证；失败回退到方案 B（sidecar 子进程，JSON over stdio）——但需另行确认。
- **CI runner**：`macos-26` 默认 Xcode 将于 2026-06-08 切到 26.5，可固定版本规避。
- **下载体验**：首次下载耗时不可控，UI 须明确「下载中」并禁重复触发。
