# Popup 原生几何与并发 Review（2026-10-07）

## 结论边界

0.14.0 发布前，用户报告同屏拖动标题栏时窗口反复变小、放大，要求全面 review 此轮 Popup 改动。
临时诊断版仅增加本机事件记录，没有先修改行为。随后用户分别确认单条窗口和 Sidebar + PDF
三栏窗口的实际拖动大小稳定。原始的持续缩放现象尚未在这两轮实测中复现，不能宣布其唯一根因。

Review 找到并实际复现了独立的提醒动画重叠缺陷，也确认了原生几何提交中的竞态风险。
它们均已修正。修复不改变默认合并、可选独立窗口、同轮保留 Sidebar 或 300ms 换题的决策。

## 已确认缺陷与修复

| 优先级 | 缺陷 | 证据与修复 |
| --- | --- | --- |
| P1 | 新提醒先读取 transform，再取消旧提醒；可能把旧提醒正在使用的缩放作为新基准 | 真实 WindowServer harness 在旧代码上以 30ms 间隔稳定失败，最终 transform 偏差 1.9554443359375，线性比例 0.9986998438835144。改为在同一所有权锁中先恢复旧基准、再采样并登记新动画。 |
| P1 | 动画任务释放所有权后仍重复恢复 original，可能覆盖另一段动画或用户移动后的映射 | 删除脱离所有权的最后一次写入；取消、逐帧回调和 DisplayLink guard 都只处理当前 ID，最终读回也在持锁期间完成。 |
| P2 | 提醒没有在原生鼠标操作开始前取消，捕获的屏幕映射可能与用户移动竞争 | 为共享窗口安装鼠标按下和原生 willMove/liveResize/minimize/fullscreen/close 边界，事件继续正常分派；有按钮按住时不启动缩放。 |
| P2 | 几何提交将分开读取的旧 Tauri size 与稍后读取的 NSWindow frame 相加，且计算与应用位于不同回调 | 正式 pane 切换已是即时提交，删除不再使用的逐帧路径。在一次原生回调中采样、设置绝对尺寸、补偿画布并读取实际结果；仅按准备后的真实移动增量更新位置。 |
| P2 | 监视器工作区变化会在标题栏拖动中立即排队重设尺寸 | 拖动期间暂缓 pane 准备/提交及工作区 reconcile，松开后合并为一次 reconcile；几何超时回收为真实拖动留出结束宽限。该路径不是本次同屏现象的已证实原因。 |

## 核查范围

| 区域 | 核查内容 |
| --- | --- |
| Daemon / IPC | 单宿主租约和握手、generation、逐 ID Ack/Cancel、first-winner、恢复预算、空闲回收、模式切换只作用于新请求 |
| 共享作答容器 | 串行事件队列、终态 tombstone、后继就绪、迟到终态/新到达、300ms 过渡、关闭快照、草稿/焦点、卸载资源释放 |
| 原生几何 / Canvas | prepare/commit/finish、可见 outer extent 与离屏 WebView 区分、移动/缩放/Zoom/DPI、主区/Sidebar/Preview 偏好、单次 AppKit 事务 |
| 提醒合成 | CVDisplayLink 生命周期、重叠和取消、基准恢复、鼠标/窗口事件边界、减少动态效果 |
| 附件预览 | 请求/文件索引校验、异步读回 generation、DOM/native 布局版本、原生 PDF/QL 生命周期与 responder、实时缩放不被旧 DOM 测量覆盖 |
| 设置 | 缺失值默认 merged、通用页入口与搜索、独立模式的旧请求保留和预热回收 |

未发现需要改变请求路由、Sidebar 生命周期或设置模式语义的新缺陷。此结论来自上述变化路径和
现有回归用例的复核，不等同于所有操作系统/真实 IM 场景均已实机验收。

## 验证与剩余边界

- 临时诊断记录只包含本机几何，不包含提问内容。单条窗口最初 41 次真实移动始终为 612×920；
  两个窗口共 173 次移动记录的工作区变化为 0。后续记录也包含用户主动外缘缩放，不能把其尺寸
  变化算作标题栏反馈循环。三栏窗口实际拖动由用户确认稳定。
- `cargo run --manifest-path src-tauri/Cargo.toml --example popup_pulse_regression --features custom-protocol`
  使用正式 pulse 源码和真实 WindowServer，不连接 Daemon/IM。旧代码在 30ms 重叠时失败；
  修正后单次、30/50/80/100/140/180/220/280/360/440ms 十组重叠和中途取消均通过，最终 transform
  drift 为 0，NSWindow frame 保持稳定。这是动画恢复的原生证据，不代替真实拖动验收。
- 新增绝对尺寸和准备后移动位置的计算回归。临时事件记录代码及启用开关已移除，不进入发布。
- 本地完整 Rust 1256 passed / 3 ignored；252 Vitest + 5 Node tests、production build、全 targets
  Clippy 通过，安装及签名成功。新安装的实际 Dev 窗口完成 8 次 Sidebar/PDF 开合和上下文
  切换，模型和 WindowServer 正文 X 偏差均为 0，PDF X 保持 1720。独立草稿、E 成功提交后
  D 的 PDF/草稿及剩一条 Sidebar 均恢复正确。
- 修正版新安装后，用户通过 AskHuman #61 确认单条及三栏窗口的实际拖动体验通过。
  隔离 Dev 测试窗口已清理，原配置及旧测试 bundle binary 已恢复，Dev daemon 已停止。
- 最终修复提交 `363e475` 的 [四平台 CI 与依赖审计](https://github.com/Naituw/AskHuman/actions/runs/37580173657)
  全部通过。用户另通过 AskHuman #62 批准更新未发布的标签；发布与产物核验已完成。
- 用户已接受延期的真实 IM、Windows/Linux 视觉、DPI/多屏矩阵继续保留在 PROGRESS，不用本机证据替代。

## 正式发布结果

`v0.14.0` 已经用户批准，通过 lease 保护将标签从 `e959bad` 更新到最终修复提交
`363e4758e8a454d9d101b7f4e74b52a11e2ec8d7`。[正式发布流水线](https://github.com/Naituw/AskHuman/actions/runs/37581187277)
的四平台构建、macOS 签名和 npm/GitHub 发布全部成功。
[GitHub Release](https://github.com/Naituw/AskHuman/releases/tag/v0.14.0) 于 2026-10-07
14:33:06（Asia/Shanghai）发布，已确认是最新正式版，四份归档和 SHA256SUMS 均可下载，
发布正文与仓库的人工 release notes 一致。

发布后实际下载并核验：

- 四份归档通过发布的 SHA256SUMS；解包后的 Mach-O ARM64 / x64、Windows PE x64 和 Linux ELF x64 架构正确。
- 两份 macOS binary 均通过严格签名校验，identifier 为 `com.naituw.humaninloop`，Developer ID team 为 `DMJXDB9H6Q`。
- `askhuman` 和 `@humaninloop/{darwin-arm64,darwin-x64,win32-x64,linux-x64}` 均为 `0.14.0`，`latest` 全部指向该版本。
- 五份 npm tarball 均通过 registry 的 SHA512 integrity；四个平台 npm 包的 binary 与对应 GitHub 归档逐字节一致。
- 隔离目录实际执行 `npm install askhuman@0.14.0`，确认解析的是包内 ARM64 binary，CLI 返回 `AskHuman v0.14.0`，退出码 0。

以上确认正式产物及本机可执行性，不扩展已延期的其他平台视觉和真实 IM 验收结论。

## 一手参考

- 本机锁定 Tao 0.35.3 的 `macos/util/mod.rs`：顶部坐标使用 `CGDisplay::main().pixels_high()`，
  native 提交沿用同一坐标基准。
- [Apple NSEvent 本地事件监视器](https://developer.apple.com/documentation/appkit/nsevent/addlocalmonitorforevents(matching:handler:))：
  监视器在分派前处理初始事件，AppKit 的嵌套窗口拖动循环内事件不会进入该监视器。
- [Apple NSWindow](https://developer.apple.com/documentation/appkit/nswindow)：原生移动和 live resize 通知边界。
