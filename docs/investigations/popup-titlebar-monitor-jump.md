# macOS 点击标题栏后跳到主屏的原因与修复

2026-10-30，反馈用户在副屏点击顶部导航区域时，AskHuman 窗口有时瞬间移到主屏。
视频中窗口从左侧外接屏移到右侧笔记本屏，内容及尺寸基本保持；用户的版本、屏幕逻辑排列未知。
用户要求自行确认代码原因，不向反馈用户索取诊断，并批准按下述方案修复、在主 worktree 开发。

## 原因

`PopupNavbar.vue` 把导航背景标为 `data-tauri-drag-region`。Tauri 2.11.5 在 `mousedown` 就调用
`start_dragging`，所以单击也会进入拖动代码。JS / IPC / Rust 调度后，Tao 0.35.3 的
`drag_window()` 再读取 `NSApp.currentEvent`，它不保证还是最初那次鼠标按下事件。

旧实现遇到特殊事件类型 `0x15` 时，使用 `NSEvent.mouseLocation` 重建左键事件，却沿用该事件
的 `windowNumber`。前者是全局屏幕坐标；只要窗口号关联具体窗口，构造函数的 `location`
就应当是窗口内坐标。把屏幕点 C 当作窗口内点击偏移，会使拖动所需原点变为 `C - C = (0, 0)`，
而不是原窗口位置。全局原点属于主显示器，因此这处错误可以解释特定的回主屏方向。

这不是仅凭代码推测的坐标差异：独立 AppKit 检查通过真实的 `performWindowDragWithEvent:`
计算窗口内偏移，并在进入 WindowServer 拖动之前截取该偏移。

| 输入 / 输出 | 原始事件 | 旧回退事件，非零窗口号 | 屏幕事件，窗口号为零 | 正确转换后的回退事件 |
| --- | --- | --- | --- | --- |
| 窗口原点 | (-2000, 200) | (-2000, 200) | (-2000, 200) | (-2000, 200) |
| 鼠标屏幕点 | (-1880, 610) | (-1880, 610) | (-1880, 610) | (-1880, 610) |
| 原生窗口内偏移 | (120, 410) | **(-1880, 610)** | (120, 410) | (120, 410) |
| 鼠标点减偏移 | (-2000, 200) | **(0, 0)** | (-2000, 200) | (-2000, 200) |

检查覆盖 6 个原点（左、右、上、负 X/Y、主屏原点）、3 个点击点、4 种事件构造，72 项通过。
这是事件转换的证据，不是真实双屏硬件上的鼠标拖动：本机只有一屏，运行时锁屏，没有向全局
输入队列发送鼠标事件。视频中实际经过的事件类型不能回溯，因此不把它说成已记录到的现场调用栈。

焦点回调会采样窗口几何，但只有工作区变化才请求重排；重排等待左键松开，没有“激活就搬到主屏”
的显式分支。原生布局中的 `CGMainDisplayID` 用来翻转 Y 坐标，不等于选择主屏作为窗口目的地。

## 修复

在现有 `[patch.crates-io]` 下加入 Tao 0.35.3 本地副本，保留许可证。只改 macOS
`window.rs` 与 `window_delegate.rs`；其他平台、前端拖动区域、双击最大化及 IPC 协议保持原实现。

- 在 Tao 既有的原生 `NSWindow.sendEvent:` 入口、WebKit 接收事件之前，保留原始左键按下事件，
  检查其原生窗口与接收者一致。全进程同一时刻只有一条主键手势，所以 UI 线程只保留最新一条，
  并绑定原生窗口身份；另一窗口的新按下覆盖旧手势。
- `drag_window()` 仅在左键仍按住且事件属于目标窗口时一次性取走事件；错误窗口的请求不消费它。
  不再读取 `NSApp.currentEvent`，不再构造 `0x15` 回退事件。先释放缓存借用，再调用 AppKit，
  避免它重入事件循环时触发 `RefCell` 冲突。
- 松键、原生窗口关闭时清除。若 AppKit 内部拖动循环吞掉 mouse-up，下一次请求发现按键已松开时
  同样清除。原生 `movableByWindowBackground` 已经开始的拖动也消费事件，避免 IPC 再开始一次。

事件是 UI 线程上的 `Retained<NSEvent>`，没有跨线程传原生对象。保留原始事件同时保留坐标、窗口号、
事件号、时间戳及修饰键；屏幕缩放和 Space 切换继续由 AppKit 处理。补丁维护与删除条件见
`src-tauri/vendor/tao/ASKHUMAN-PATCH.md`。

## 原生回归

```bash
# 真实 AppKit 坐标转换，72 项；私有截取方法只存在于此测试程序。
clang -fobjc-arc -framework AppKit scripts/popup-drag-offset-regression.m \
  -o /tmp/askhuman-drag-offset-regression
/tmp/askhuman-drag-offset-regression

# 真实 Tauri -> vendored Tao -> 原生 sendEvent / drag_window 路径。
cargo run --manifest-path src-tauri/Cargo.toml \
  --example popup_window_drag_regression --features custom-protocol
```

第二个检查在独立进程的隐藏窗口运行，只替换本进程的按键读数、currentEvent 和最后的原生拖动调用，
不连接 daemon，不改配置，不产生真实鼠标输入。36 组位置 / 点击点 / currentEvent 为空或已被替换
的组合都要求最后收到**同一个原始 NSEvent 对象**。创建方释放其引用后也必须通过，以检查保留所有权。
另验重复请求、跨窗口拒绝与保留、新点击覆盖旧点击、mouse-up、右键、原生循环吞掉 mouse-up、关闭清理。

把 `drag_window()` 临时恢复为上游旧实现后，同一检查在“使用 currentEvent 而非原始事件”断言失败；
恢复补丁后通过。补丁之外没有保留运行时方法替换或诊断开关。

2026-10-30 自动检查：Rust `--bin AskHuman` 全套 2253 passed / 0 failed / 11 ignored；
Rust 1.99.0 `cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --all --check` 通过。
vendored 源码逐文件核对，除上述两个 macOS 文件和新增补丁说明外与 crates.io 0.35.3 一致；
Cargo.lock 只移除了 Tao 的 registry source / checksum，依赖版本未变化。
随后 `./scripts/install.sh` 成功构建、签名并安装到主环境 `~/.local/bin/AskHuman`。

上述自动检查不代替真实双屏、混合缩放、鼠标拖动与 macOS 不同版本的硬件观感验证。

## 一手参考

- [锁定版本的 Tao drag_window](https://github.com/tauri-apps/tao/blob/tao-v0.35.3/src/platform_impl/macos/window.rs#L931-L957)
- [Tauri 的 mousedown 拖动入口](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/window/scripts/drag.js#L78-L104)
- [Apple mouseLocation 屏幕坐标](https://developer.apple.com/documentation/appkit/nsevent/mouselocation)
- [Apple 鼠标事件 location 参数](https://developer.apple.com/documentation/appkit/nsevent/mouseevent(with:location:modifierflags:timestamp:windownumber:context:eventnumber:clickcount:pressure:))
- [Apple performDrag 要求原始鼠标按下事件](https://developer.apple.com/documentation/appkit/nswindow/performdrag(with:))
- [Tao 加入 0x15 回退的原始 PR](https://github.com/tauri-apps/tao/pull/743)
- [上游对 IPC 后 currentEvent 消失的报告](https://github.com/tauri-apps/tao/issues/1371)
