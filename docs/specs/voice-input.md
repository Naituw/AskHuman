# 需求：弹窗语音输入（macOS 26 SpeechAnalyzer，Swift 链入）

> 状态：已确认（待按计划实现）
> 关联计划：`docs/plans/voice-input.md`

## 1. 背景

弹窗输入框需要「按住说话即转写为文字」的语音输入能力。此前尝试过两条路并放弃：

- **旧原生 `SFSpeechRecognizer`（objc2 调用，`src-tauri/src/macos_speech.rs`）**：可用但停顿/分段、状态处理别扭。
- **Web Speech（`webkitSpeechRecognition`）**：WKWebView(=WebKit) 上有 Apple 自身已知 bug（最终文本重复塞进临时结果、`isFinal` 常不触发），不可靠。

最终方案：改用 **macOS 26 新增的 `SpeechAnalyzer` / `SpeechTranscriber`**（识别质量好、离线、实时增量）。该 API 为 **Swift-only**，objc2 无法直接调用，因此需写 Swift 并链入现有 Rust(Tauri) 二进制。已用独立 `speech-demo`（CLI + SwiftUI GUI）验证：实时增量、光标处插入、移动光标即固定并重启会话、首次触发模型下载等行为均符合预期。

本需求接受 **仅支持 macOS 26+**。

## 2. 目标

- 弹窗输入框右下角的麦克风按钮 / 快捷键 **⌘D** 开始、再次触发停止；录音时麦克风按钮高亮（呼吸动效）。
- 识别文本**实时增量**写入输入框：
  - 已最终化片段（committed）→ 永久插入到当前光标处并前移光标。
  - 当前未最终化片段（volatile）→ 在原位就地替换显示。
  - 会话进行中**用户移动了光标** → 固定当前已写内容并 `flush`（重启识别会话），后续文本从新光标处继续。
- 识别语言沿用「设置 → 弹窗行为 → 语音识别语言」下拉（`speechLanguage`，默认跟随系统，含中/繁/英/日/韩）。
- 首次使用某语言会触发**模型下载**，需有「下载中 / 进度 / 失败」交互。
- 单进程、单文件分发；麦克风/语音识别权限沿用主程序身份（与现有一致）。

## 3. 已确认决策

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 识别引擎 | macOS 26 `SpeechAnalyzer` + `SpeechTranscriber`（新 API），离线、实时增量。**不再使用** `SFSpeechRecognizer` / Web Speech |
| D2 | 集成方式 | Swift 编为**静态库链入现有 Rust 二进制**（单进程，权限沿用主程序，单文件分发） |
| D3 | Rust↔Swift 桥接面 | **Objective-C 包装**：Swift 写 `@objc` 的 `NSObject` 子类（稳定 ObjC 类名），方法/回调走 ObjC。Rust 用 `objc2` `msg_send!` 调用、`block2` 传回调。**不**用 `@_cdecl`+C 函数指针 |
| D4 | 回调形式 | **block（`@convention(block)` 闭包属性）**，Rust 端 `block2::RcBlock` 闭包**直接捕获 `AppHandle`** 调 `app.emit`。不用 delegate（避免在 Rust 端 `define_class!` 实现协议的样板） |
| D5 | 系统版本支持 | **仅 macOS 26+**。macOS < 26：隐藏/禁用麦克风按钮并给出提示，**不**回退到旧 API |
| D6 | 旧实现清理 | 删除 `src-tauri/src/macos_speech.rs`；移除 `objc2-speech`、`objc2-avf-audio` 依赖（音频/识别全部由 Swift 承担）。保留 `objc2`/`objc2-foundation`/`objc2-app-kit`（QuickLook/菜单仍用）与 `block2`（桥回调用） |
| D7 | 语言不支持 | 选定语言机型/系统不支持时：**明确提示并中止**本次识别，不静默回退系统默认 |
| D8 | 模型下载交互 | 复用麦克风按钮旁的状态/错误行，显示「下载中 xx% / 完成 / 失败」；下载期间禁用重复触发 |
| D9 | 目标架构 | 同时支持 **arm64 + x86_64**。沿用现有 CI 矩阵「按架构分别交叉编译」（产出 `darwin-arm64`/`darwin-x64` 两个独立产物），**不**做 lipo 通用二进制。`build.rs` 按当前 `$TARGET` 架构编对应 Swift 静态库 |
| D10 | CI runner | 两个 Apple 目标从 `macos-14` 升级到 **`macos-26`**（自带 Xcode 26 / `macosx26.x` SDK，universal 含 x86_64 切片）；x64 仍在 `macos-26`(arm64) 上交叉编译 |
| D11 | 构建约束 | 今后所有 macOS 构建（含本地 `install.sh`/调试）**需本机装有 Xcode 26 SDK**，否则 `build.rs` 编 Swift 失败。已接受 |
| D12 | Swift 源码位置与质量 | 放 `src-tauri/swift/`；复用 demo 的 `SpeechEngine` 逻辑，但**按正式代码质量重新整理**（去掉 `/tmp` 文件日志、诊断噪音、env 开关等 demo 痕迹），新增 `@objc` 桥 |
| D13 | 前端插入模型 | 复刻 demo 的「增量提交 + 实时片段就地替换 + 移动光标即固定并 flush」到弹窗 `<textarea>` |
| D14 | 触发方式 | 麦克风按钮 + **⌘D**（与现状一致） |

## 4. 约束与既有规则（不可破坏）

- **单文件分发**：Swift 运行时在 macOS 上 ABI 稳定、随系统位于 `/usr/lib/swift`，**不额外打包 dylib**；产物仍是单个 Mach-O。
- **权限身份不变**：仍靠 `build.rs` 将 `Info.plist`（`NSMicrophoneUsageDescription` / `NSSpeechRecognitionUsageDescription`）嵌入 `__TEXT,__info_plist` 段；权限沿用主程序。
- **跨平台不受影响**：Swift 编译/链接仅在 macOS 目标触发；Windows/Linux job 不变。
- **release 构建模式**：生产构建仍须 `tauri/custom-protocol`，不回退。
- **stdout / 结果契约不变**：语音仅写入输入框，不改变任何 CLI 输出契约。

## 5. 验收标准

1. macOS 26 弹窗中按 ⌘D 或点麦克风开始，说话时文本实时增量出现；停止后文本保留、无报错。
2. 光标置于中段开始识别 → 文本插入到光标处；会话中移动光标 → 已写内容固定，新文本从新光标处继续（不串位/不重复）。
3. 首次使用某语言触发下载时，麦克风旁出现「下载中/进度/失败」提示；下载失败给出明确错误且不卡死。
4. 选「简体中文」能识别中文；机型不支持所选语言时明确提示并中止。
5. macOS < 26：麦克风按钮隐藏/禁用并提示「需 macOS 26」。
6. `cargo build --target aarch64-apple-darwin` 与 `--target x86_64-apple-darwin` 在 macOS 26 + Xcode 26 下均能编译链接通过；CI 两 Apple 目标产物正常。
7. 旧 `macos_speech.rs` 及 `objc2-speech`/`objc2-avf-audio` 依赖已移除，编译无警告残留。
