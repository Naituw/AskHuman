# Duo 折叠后右侧导航按钮消失

2026-11-08，接续 `PROGRESS.md` 中 11-07 的侧栏按钮消失排查。

## 实施与回归状态

**本地修复与适用布局回归已完成，真机待验收。** 尚未上传 TestFlight，真机 iOS 27.3 Beta 未复验。
仅替换原生标签容器的原型并不能解决问题；同二进制产品对照进一步定位到根标签容器外的全局横幅 `.overlay`。
裸根、到达动效背景、配对 sheet、触觉反馈对照均通过三次开合；只恢复横幅 overlay 就在首次折叠丢整组导航按钮。
当前实现把横幅放入与 tabs 相同的 UIKit 父容器，作为同级的稳定 UIHostingController，以 intrinsicContentSize 更新高度；
空时不占高度，只有可交互横幅存在时接收触摸，发送提示保持透传。
没有按尺寸重建、强制刷新、手动生命周期调用或延迟修复。

`product-native-banner-request.xcresult` 请求单独重跑通过三次开合、切题、草稿、前后台及实际返回；
`product-native-banner-full.xcresult` 六条全部通过（新任务、Agent、历史、Mac、待办、请求），包括最后的请求再跑三次。
请求四张外屏图及各类详情 / 表单的外屏图已核对。`product-native-root-cover.xcresult` 另验证外屏横幅显示及实际点击跳转通过。
`install.sh` 已再次成功完成，Mac 二进制内容无变化。适用的大屏与窄内屏回归已完成；结果及瞬时横幅用例的补验见下表。

最终本地验证：

| 范围 | 结果与证据 |
|---|---|
| 原始请求单独重复 | `product-native-banner-request.xcresult`，三次开合、四按钮、草稿、切题、前后台、实际返回通过 |
| 六条真实开合整组 | `product-native-banner-full.xcresult`，6 通过 / 0 失败 / 0 跳过；请求在其余五条后再次完成三次 |
| 大屏其余流程与到达动效 | `product-native-root-wide.xcresult`，17 通过 / 3 预期跳过；AX 和瞬时横幅的失败分别以下两组补验 |
| 辅助字号返回 | `product-native-root-ax-banner-retest.xcresult` 的 AX 用例通过；同组横幅在启动前终止旧 App 失败，无界面结论 |
| 窄内屏竖屏 | `product-native-root-narrow-portrait.xcresult`，实际 669×951，4 通过 / 0 失败 / 0 跳过；含浮层呼出、重选、草稿与发送后返回 |
| 外屏默认五秒横幅 | `product-native-root-cover.xcresult`，真实显示、点击和跳转通过，截图完整 |
| 宽屏横幅布局 / 点击 | `persistent-wide-banner-touch.log` 与 `native-root-persistent-wide-banner*.png`，相同产品动态库的完整 RootView 在独立 SwiftUI App 中用 Mock 持续横幅夹具实际点击，打开 Agent 且关闭横幅；显示区域 520×76 |

宽屏补验夹具只在本机日志目录，保留正常 SwiftUI App / AppDelegate / 环境与完整 RootView，直接复用 SHA-256 一致的产品动态库。
夹具以现有 `AppStore.show(_:for:)` 放一条 120 秒 Mock 横幅，验证布局和触摸路由；它不验证默认五秒自动消失，后者仍由外屏原用例覆盖。
没有修改、重新编译产品代码或延长产品默认计时。早先 debugger 注入因内部类型解析失败没有建立夹具，不计结果。
夹具 App 已停止，源码、截图、日志保留。产品和夹具的 debug dylib 均为
`3de30b482148988dc43527d6c523f975d73d1991f45e570c4cdbf2d4ec8802af`。

窄内屏准备中，宿主的短暂转屏等待曾追不上重载下的实际界面；最终只发送一个姿态，再连续三次核对 activeScreen / sizePoints，稳定为内屏竖屏后才开始四条测试。这是测试宿主的准备，不是 App 的延迟修复。

以下保留各阶段的原型和失败证据；当前结论以本节的最新验证状态为准。
五个标签原有的导航、列表选择样式与窄浮层规则均保留。早先单一 split / 原生 List selection
的产品重构已全部撤回：重复开合中仍丢 back，不能解决根问题。

### 原生标签容器基础（单独使用仍不足）

坏状态的页面路径、UIKit 导航栈、back item 与 toolbar groups 均在，丢失的是最外层标签容器
承载的共享竖向栏。独立程序保留同一份 SwiftUI split → stack 页面与相同的标签栏显隐策略，
改由原生 UITabBarController 承载后，十次有效开合均保留 back、前后切题和 More；
SwiftUI TabView 对照会复现。但截图显示该原生探针的标签栏在 compact 下仍可见，SwiftUI toolbar 偏好没有驱动原生 UITabBarController 隐藏；它并未覆盖产品主动隐藏标签栏的条件。证据将问题缩到标签容器和 SwiftUI 导航工具栏之间的适配边界，
不能据此宣称已经证明 Apple 内部某个私有回调的错误。

产品使用 `AppTabs`，每个 UITab / UIHostingController 只创建一次。普通 UIViewController
按公开的 addChild / 子视图约束 / didMove 生命周期承载 UITabBarController，并通过
`childForPreferredVerticalBarBehavior` 把共享栏决策交给标签容器。整个容器填满窗口，
UIKit 自行处理安全区；没有按尺寸改 identity、重建页面、延迟刷新或私有 API。
RootView 同步模型选择、未读徽标、紧凑屏标签栏显隐、收件箱重选与待办订阅可见性；
程序同步期间暂停 delegate，用户选择只回写实际变化，避免首次标签回调覆盖 AHTab 或重复触发收件箱重选。
More 使用完整 `Label("More", systemImage: "ellipsis")`。

### 产品集成对照

直接从 UIViewControllerRepresentable 返回 UITabBarController 的第一版发生了独立的集成回归：
点 Agent 行后 AppStore.agentPath 从 0 变为 1，但 AgentsColumns.body 没有重新计算，页面仍为占位。
同一 store 的读取与 setter 地址相同，实际面板触摸也复现，故不能解释为测试没有点到。
重复 rootView 赋值、AnyView、创建时机、额外根修饰器均已排除；相应临时诊断代码全部移除。
最小计数探针的 State / Observable / AppStore 都能正常更新，故也不能泛化为所有 UIHosting 页面失效。

同一份临时产品二进制的开关对照：

| 承载方式 | 选择结果 | 本机证据 |
|---|---|---|
| 直接 UITabBarController，程序同步暂停 delegate | Agent 路径变更，详情不显示 | `pick-variant-guard_raw.png` |
| 普通 UIKit 父容器，程序同步暂停 delegate | Agent 详情与选中行正常 | `pick-variant-guard_plain.png` |
| 同上，推迟到 viewDidAppear 安装标签 | 正常，两次重复 | `pick-variant-guard_late.png`、`guard_late_2.png` |

因此最终采用正常父子控制器关系，不保留延迟安装。未隔离程序 delegate 时，首次默认标签的
回调曾把模型选中的 Agents 改回 Inbox；这属于集成反馈问题，不能靠反复重设页面掩盖。
`product-contained-tabs-fold.xcresult` 中 Agent 从列表进入详情后，首次折叠仍缺 back；History / Inbox 的列表行存在但不可点（标签栏 AX 几何落在左上）。Mac、待办与新任务 sheet 共三条通过，另外三条失败。

随后把原生标签栏显隐从 SwiftUI 尺寸类回传移到 UIKit trait 生命周期：RootView 无条件传当前页的打开状态，容器在尺寸类／辅助功能字号 trait 变化及 viewIsAppearing 时套用显隐规则。相同产品测试的 Agent、History、Request 三条已全部通过（`product-uikit-traits-three.xcresult`），每条三次开合、实际返回，请求另含切题与草稿。后续 `product-uikit-traits-full.xcresult` 五条通过，Mac 在折叠前的列表行 exists 但 not hittable。相同产品不改代码重跑 Agent + Mac 两条全部通过（`product-uikit-traits-agent-mac.xcresult`）；直接从外屏启动 Mac 页再展开，用 duoctl 的活动面板触摸也正常进入详情，截图与元素记录为 `macs-cold-cover-open-*`。这次失败不是按钮消失的证据，暂不归因于产品逻辑或确定为 XCTest 缺陷。

测试准备调整为先终止 App、展开内屏，再启动 App；每条均从用户给出的内屏起点开始，避免上一条留下的外屏混入下一条的启动过程。首版等待 SpringBoard 首个窗口宽度的 `product-final-inner-first-fold.xcresult` 无效：首个窗口是状态栏而非整屏，测试未启动 App 即超时，该轮已停止。最终改为每次开合携带独立 nonce；宿主读到 Duo 实际 activeScreen 后，向测试 runner 的 Documents 原子写入确认，测试读到对应 nonce 才继续。App 运行中的开合还必须等 App 窗口宽度匹配。所有 exists / isHittable 断言保留，宿主在失败时另采活动面板截图。请求用例增加前后台返回后的工具栏与草稿检查。干净重编译成功，`product-final-handshake-fold.xcresult` 五条完整通过；请求在第一次折叠后仍丢返回、前后切题与 More。宿主捕获的活动外屏 `product-final-handshake-fold-missing-1.png` 显示右侧空、底部空胶囊，确认是真实产品失败。不能把该轮或原生根容器方案标为修复完成。继续在独立程序补齐 safeAreaBar 底栏条件，比较 UIKit 原生窗口与 SwiftUI 包装根容器、固定 split 与 split → stack 的组合。

补齐显隐条件的原生 UIWindow 探针也完成两组各三次开合：有语义工具栏（`Probe/native-window-hybrid-dynamic-*`）与仅 back（`Probe/native-window-hybrid-dynamic-back-only-*`），截图中标签栏确实隐藏、按钮均在。它说明原生容器与 SwiftUI 导航内容可在这一条件下配合；产品结论仍以产品用例为准。

### 后续原生窗口与外围承载对照（尚无产品结论）

`Probe/native-window-bottom-keyboard-switch-*` 在原生 UIWindow 根、同一 split → stack 内容、主动隐藏标签栏、safeAreaBar 输入栏及首次键盘条件下三次保留按钮与草稿。`wrapped-parent-bottom-keyboard-switch-*` 也完成三次；这些简化程序仍不足以证明产品修复。

进一步使用实际五个页面、AppEnvironment / Mock，排除额外全局修饰器：

- 产品临时 `AH_NAV_BARE_ROOT=YES`（仅 Mock 生效）只移除 RootView 外围修饰器；标记须在交付前移除。
- `product-bare-root-request.xcresult` 在 App 启动前的宿主 `duoctl state` 超时，未产生有效 UI 结论。
- 独立 `NativeProductProbe` 的全量构建先因 Info.plist 重复（已修）及空间耗尽失败，未运行。其构建缓存已通过 Xcode clean 清理，源码保留。
- 改用 `@testable import AskHuman` 复用产品的 `AskHuman.debug.dylib`，只链接独立 UIKit 窗口入口与原生标签控制器，不重编或修改页面；位于本机 `NativeProductProbe/LinkedApp/NativeProductProbe.app`。Mach-O 已确认同为 simulator platform 7、minOS 17.0、SDK 27.2。其五个页面没有全局横幅／配对等修饰器，不能把它直接作为完整产品。
- 宿主关闭 Xcode 的 UITargetAppPerformanceAntipatternChecker（界面导航测试不检查性能），动画与可点断言保留。本机 root 的 PerfPowerServicesSignpostReader 已占约 27 GiB，不能无管理员权限停止；未操作其他设备或进程。本次 Duo 关机保留数据后空间恢复约 7 GiB，正在重新启动。

### 根视图外围修饰器的同二进制对照

2026-11-08 22:24，同一产品二进制、同一测试与起始内屏，`AH_NAV_BARE_ROOT=YES` 的
`product-bare-root-request-valid.xcresult` 完成三次开合、切题、草稿、前后台和返回，全通过。
紧接着恢复完整 RootView 的 `product-full-root-request-control.xcresult` 首次折叠再次缺 BackButton；
活动截图 `product-full-root-request-control-missing-1.png` 确认右侧整组为空，底部还有空胶囊。
正在逐组恢复背景、横幅和呈现修饰器；尚不能把裸根通过等同于完整产品修复。

根修饰器矩阵（同二进制）：`arrival` / `sheet` / `feedback` 三条都完成三次开合、前后台、草稿和返回；
`banner` 仅恢复全局横幅 `.overlay` 就在首次折叠缺 BackButton，活动外屏截图确认。
证据为 `product-root-probe-{arrival,banner,sheet,feedback}.xcresult`。
候选修复把横幅作为普通 UIKit 父容器中的独立 UIHostingController，与 tabs 同级；通过
intrinsicContentSize 让其高度随内容更新，空时零高，仅存在可交互横幅时接收触摸。
标签容器继续负责竖向栏，不在其外面叠 SwiftUI 横幅。所有根诊断开关已移除，完整回归正在进行。
这定位了产品中可控制的承载边界，不能据此命名 Apple 内部清空条目的私有实现。

独立链接的 UIKit 窗口对照存在独立的无效条件：直接仅 UIHostingController（没有标签）与跳过
原生 updateProperties 的版本，实际点击后模型路径都为 1、页面仍占位。根层新增只读深度文字
则能更新为 1，实际产品的 InboxColumns 仍停在占位。因此这组链接入口的页面观察关系尚未解释，
不能据此断定正常 UIKit 窗口根会怎样；暂时以原产品 SwiftUI App 的同二进制根修饰器对照为准。
`UIObservationTrackingEnabled=false` 对这组失效无改善，且不是已证明的关闭自动跟踪方法。

### 扩大布局回归与测试定位

`product-native-root-wide.xcresult` 共 22 条，17 通过、3 条按设备 / 宽度预期跳过，另有两项待复验：
AX 返回用例只查询 navigationBars，Duo 实际返回键在浮动栏；失败附件的 UI 树明确含 BackButton，
frame `(884,129,38,38)`。测试改为先检查系统 BackButton 的 exists / isHittable 并点击，旧平台继续走原导航栏定位，
回到列表的断言保留。`product-native-root-ax-banner-retest.xcresult` 中 AX 已通过，耗时 193 秒（严重换页）。
本次只重新编译 XCTest 定位，产品 debug dylib SHA-256 前后一致：
`3de30b482148988dc43527d6c523f975d73d1991f45e570c4cdbf2d4ec8802af`。

大屏横幅那次在 t=34 秒找到元素，截图之后 t=36 秒读 label 时已经消失；模型横幅默认只保留五秒。
复跑第二次在 App 启动前终止旧进程即失败，没有界面结论；不能把这两次算作点击通过。
外屏横幅显示与实际点击的 `product-native-root-cover.xcresult` 已通过，截图已核对完整排版。
其余大屏流程（选择 / 切题 / 发送 / 三个标签详情 / 两类表单 / 配对 / 解除配对 / 未配对引导）及到达动效均通过。
窄内屏首轮在宿主旋转准备失败，未运行测试；正在补齐有效的竖屏与重选回归。

### 已撤回与无效结果

- 单一 split / List selection 初轮部分通过，扩展与重复回归仍缺 back；不是最终方案。
- 禁止动画、延迟隐藏标签栏无效；重建 TabView 外壳被用户明确否决，已撤回。
- `product-native-tabs-fold.xcresult` 与 `product-native-tabs-clean-fold.xcresult` 的新任务草稿通过，
  但列表选择在折叠前失败，不能算产品开合通过。
- `product-native-tabs-stable-pages-two.xcresult` 的调试器附加与测试关闭旧进程冲突，没有有效页面断言。
- 初次计数探针点在 plain 文字按钮的文字外，得到的全零结果作废；正确文字坐标下三个计数均更新。
- 系统切屏一度持续黑帧；独立十次开合脚本已拒收黑帧。XCUIScreen.main 在内屏可能截到黑色外屏，
  布局截图用 duoctl 的活动面板采集，不把黑图记为通过。

本次 Duo 曾因迁移、磁盘及严重换页停滞。只重启本次设备，保留数据；未停止其他模拟器，
后期未重启全局 CoreSimulator 服务。旧构建缓存通过 Xcode / Swift Package 官方 clean 清理，
保留源码、测试结果与配对。bootstatus 报 Data Migration Failed 后实际 App 与开合已恢复，
不能只据这一状态判断不可测试。

## 复现与证据

用户补齐的路径：**内屏横屏 → 从收件箱列表点开提问 → 切到外屏**。
消失的是右侧整组按钮（返回、前后切题、更多），不是只有返回键。
用户环境是 TestFlight / iOS 27.3 Beta；本机在 Duo / iOS 27.1 模拟器、
Xcode 27.2 / iOS 27.2 SDK 的 Debug 构建上复现同样现象。

用已有 Mock 从列表点开 `req-migration` 可以复现；
以 `-AHOpen req-migration` 在启动时直接打开详情的一次对照没有复现。
此前只在外屏尝试前后台切换没有复现，不能据此排除折叠路径。

坏状态下的「复制诊断信息」：

- `AppStore.inboxPath` 长度 1，UIKit 导航栈长度 2，有 back item，`hidesBackButton = false`。
- 顶层 navigation item 仍有 4 个 trailing groups。
- 主窗口已经是外屏 `466 × 678`，compact × regular，右安全区仍是 84。
- 导航控制器与标签控制器的 floating bar 都是 `buttons 0 shown of 0`。
- 标签控制器里的 `VerticalBarPocketView` 隐藏，仍留着 `(586, 0, 83×951)` 的旧几何。

这证明页面与导航项目仍在、系统右侧栏未承载它们；**还不能仅凭这份报告断定 UIKit
内部是哪一个回调清掉了按钮**。代码排查重点是 `ColumnsOrStack` 在 regular → compact 时
将 `NavigationSplitView` 换成 `NavigationStack`，同时详情将 tab bar 从 automatic 改为 hidden。

## 验证入口

`AskHumanFoldNavigationUITests` 必须由宿主驱动真实开合，普通 UI 测试运行时跳过。
它从列表打开页面，不能用 `-AHOpen` 代替；覆盖五个标签各三次开合后的返回、提问前后切换与草稿，以及新任务 sheet 的草稿与退出。

先按 AGENTS 运行 `node scripts/machine-health.mjs`，再构建：

```sh
xcodegen generate --spec ios/project.yml
xcodebuild build-for-testing -project ios/AskHuman.xcodeproj -scheme AskHuman \
  -destination 'platform=iOS Simulator,id=<Duo UDID>' \
  -derivedDataPath ios/build/DerivedDataDuoNavigation CODE_SIGNING_ALLOWED=NO
```

让 Duo 内屏保持横屏（≥700pt），执行：

```sh
python3 ios/Tools/test-fold-navigation.py \
  ios/build/DerivedDataDuoNavigation/Build/Products/AskHuman_iphonesimulator27.2-arm64.xctestrun \
  '<Duo UDID>' --result '<尚不存在的绝对路径>.xcresult'
```

宿主识别测试输出中的开合指令，调用 `duoctl`；测试等待实际窗口尺寸变化，
再断言按钮可点和草稿仍在。测试包先编译成功不等于开合回归通过。

## 第一轮环境阻塞（随后已恢复）

已复现并取到坏状态报告。第一个实验只在 `ColumnsOrStack` 的 `usesColumns` 变化事务上设置
`transaction.disablesAnimations = true`，已编译；未得到有效运行结果，**功能实验已从主树撤回**。
它不是已经验证的修法，后续也不能未经验证恢复后出包。

验证状态：

- Debug App 和包含新增用例的 `build-for-testing` 均成功；两条开合用例未运行通过，宿主驱动也尚未完成实际往返。
- `./scripts/install.sh` 成功，日常二进制内容未变；本次没有 Mac 逻辑修改。
- 原 Duo 随后开合／启动不响应。按用户选择等待另一轮 UI 测试结束后重启 Simulator，
  再连续等待约 10 分钟；经用户同意重启 CoreSimulator 服务后，设备启动返回 `Data Migration Failed`，
  主屏幕显示占位图标，AskHuman 启动未取得进程句柄。没有清除设备数据。
- 用户批准保留原设备、将其关闭释放内存，新建临时 iOS 27.2 Duo：
  `AH-Duo-Navigation-27.2` / `6782E649-6297-4D3E-9A20-9CB126876102`。
  新设备停在首次 `AddressBookLegacy.migrator`，安装 App 的命令也未完成。
- 原 Duo 在关闭后又被启动；用户决定**让另一会话先使用模拟器，本次保留结果、后续继续**。
  本次临时设备已关闭，保留以便后续使用；不再操作另一会话的设备或服务。
- 未上传 TestFlight，未修改配对或协议。

本机证据保存在 `~/Library/Logs/AskHumanDuoNavigation/`：
`askhuman-duo-broken.png` / `.txt`（坏状态截图与报告）、`askhuman-duo-before.png`（一次正常对照）、
`askhuman-duo-tests-build.log`、`askhuman-duo-install.log`、
`disable-transition-animation-experiment.patch`（已撤回的未验证实验）。证据来自 Mock，无真实消息内容。

后续：先协调一段只运行一台 Duo 的测试时段并确认设备能够启动 App，
再从列表点开详情比较修法、运行上述开合回归。已记项目待办 #10
（`636dd45b-d512-4946-aa35-6db52918adc6`）。模拟器修复验证后先汇报，再由用户决定是否出 TestFlight。

## 恢复排查与根因约束（同日）

用户随后要求检查是否可以继续，原 Duo 恢复到能运行 App 和开合。
对照结果：禁止交接动画、下一轮任务再隐藏 tab bar 均无效；始终保留 tab bar 的一次对照也未恢复导航按钮。
重建 `TabView` 外壳的一次实验恢复了按钮，但用户明确否决把这种绕过作为修复：
必须定位根因，不能以多个补丁叠加换取表面恢复。`ListDetail.swift` 与 `RootView.swift` 的业务实验均已撤回。

独立最小程序放在本机证据目录的 `Probe/`（`project.yml`、`NavigationProbe.swift`），不属于产品 target，
只含两个 Tab、三行列表、一页 Form 和工具栏。`-AutoPick YES` 在根视图出现三秒后把空选中改为 `[1]`，
并记录控制器身份，避免 `-AHOpen` 那种在容器建立前就已选中的不同路径；截图中要确认 `Message 1` 已打开。

已取得的证据（Duo / iOS 27.1，Xcode 27.2 / SDK 27.2）：

| 对照 | 实际结果 | 证据 |
|---|---|---|
| `-Mode switch`：regular 用 split，compact 用 stack | 最小程序复现整组导航按钮消失；`path=[1]` 保留，旧 split 被新 stack 替换，Tab 控制器身份不变 | `Probe/switch.log`、`switch-folded.png` |
| `-Mode split`：始终使用同一个 split，系统自行折叠 | 第一次折叠正常；第二、三次开合后返回键保留，但自定义工具栏三项消失、留下空胶囊，静置后仍未恢复；split 和内容宿主身份保持不变 | `Probe/split.log`、`split-folded.png`、`split-folded-2.png`、`split-folded-3.png`、`split-folded-settled.png` |

两种对照都有 `Unbalanced calls to begin/end appearance transitions` 警告，
所以**不能仅用这个警告证明是哪一层造成按钮消失，也不能宣称单一 split 已经解决问题**。
这组反例表明问题可以脱离 AskHuman 的业务代码出现；系统 / SwiftUI 导航工具栏适配仍需继续隔离。

随后对照旧 SDK（按命令设置 `DEVELOPER_DIR`，不改变全局 Xcode 选择）；结果与归档核对见下文。

Apple 的 [NavigationSplitView 文档](https://developer.apple.com/documentation/swiftui/navigationsplitview)
说明它在 compact 尺寸类会自行折叠，可用 `preferredCompactColumn` 管理显示列；
最小程序的 `split` 对照使用这些公开 API，没有 `.id`、定时刷新或私有 API。

### 真机报告与进一步对照

用户提供 2026-11-08 13:05:39 的完整报告（AskHuman 对话中保留）：build 13、iPhone19,4、
iOS 27.3 `24C5037i`，Xcode `2720 (27B52)`、SDK `iphoneos27.2 (24B8118)`。
当时 inbox 栈 1、UIKit 栈 2、有 back item、不隐藏 back、trailing groups 4，
但两个 floating bar 都是 0 个按钮，`VerticalBarPocketView` 隐藏。
这次侧栏几何已经是外屏 `(387, 0, 79×678)`，所以**旧尺寸不是充分解释**。
`UIPlatformGlassInteractionView` 下还有三个 `UIKitBarItemHost`，却位于底部 `y=606` 一带，
而非右侧栏；NavigationController 的 bottom safe area 仍是 82，主窗口为 34。
报告确认真机也存在「声明仍在、承载与位置不对」的状态，不把模拟器暂时恢复作为真机修复证据。

源码不变的 SDK 26.2 对照已编译、运行，但进入兼容布局：inner 与 cover 的 SwiftUI 宽度类始终 compact，
按钮留在顶部；它没有经过相同的分栏／竖向栏适配，**不作为排除或证实 SDK 27 回归的同条件对照**。
实际读取外测 worktree 的 build 10 归档 `AskHuman-External-Candidate.xcarchive`：
`DTSDKName=iphoneos27.1`、`DTXcode=2710`，不是 SDK 26。先前「build 11 首次使用 SDK 27」的宽泛说法不准确，
目前可确认的变化是 27.1 → 27.2。

SDK 27.2 下 `-Mode stack` 始终使用同一 UINavigationController，三次开合后也出现自定义按钮部分／全部不画，
返回键还在；没有 appearance 不平衡警告（`Probe/stack.log`、`stack-folded*.png`）。
因此不能把导航控制器替换或该警告当作完整根因。

进一步隔离（均为独立 probe，业务代码保持无实验改动）：

| 对照 | 实际结果 | 证据 |
|---|---|---|
| 把 tab bar 可见性声明从详情移到标签固定根部 | 第一次折叠仍丢失整组按钮，不能采用 | `Probe/owner-root.log`、`owner-root-folded.png` |
| 新版 `Tab` API 替代旧 `.tabItem` | 仍复现整组丢失与底部空胶囊，不能归因于旧 Tab API | `Probe/modern.log`、`modern-folded-1.png` |
| `switchNoTab`：保留相同 split/stack 切换、详情与工具栏，只移除 TabView | 三次开合均保留返回与工具栏按钮 | `Probe/no-tab.log`、`no-tab-folded-{1,2,3}.png` |
| 纯 UIKit：原生 UITabBarController + UINavigationController + 图片 UIBarButtonItem，详情始终隐藏 tab bar | 三次开合均正常 | `Probe/uikit.log`、`uikit-folded*.png` |
| 纯 UIKit：改为 regular 显示、compact 隐藏 tab bar（公开 `setTabBarHidden`，宽度 trait 变化时更新） | 三次开合均正常；补齐了显隐条件，而不只是始终隐藏的不同布局 | `Probe/uikit-dynamic.log`、`uikit-dynamic-folded-{1,2,3}.png` |
| 固定 NavigationStack，详情始终 `.toolbar(.hidden, for: .tabBar)` | 第二次仍出现空工具栏胶囊，back 保留，`path=[1]`；不只由显隐切换触发 | `Probe/stack-hidden.log`、`stack-hidden-folded-2.png` |
| 纯 UIKit：图片 bar item 改成 `customView: UIButton`，明确 `.axisBehavior(.verticalPreferred)` | 三次正常，原生自定义 UIView 的搬迁本身不足以触发 | `Probe/uikit-custom.log`、`uikit-custom-folded-{1,2,3}.png` |
| UIKit 标签容器，选中页为 `UIHostingController` 内的相同 SwiftUI NavigationStack / 图片按钮（原生 tab bar 始终隐藏） | 三次正常，`path=[1]`；在这组控制条件下，保留 SwiftUI 导航与工具栏、移除 SwiftUI 标签桥接也能正常 | `Probe/hybrid.log`、`hybrid-folded-{1,2,3}.png` |
| 单一 split，工具栏改成带标题与符号的标准 Button / Menu，并把 More 拆为单独 ToolbarItem（对齐产品分组） | 三次自定义按钮都画出，但第一次漏 back，第二、三次 back 恢复；仍不能作为修复 | `Probe/semantic-split.log`、`semantic-split-folded-{1,2,3}.png` |
| 固定 stack、带标题与符号的工具栏，完全不声明 tab bar 显隐 | 三次正常；标签栏保留可见 | `Probe/semantic-stack-unmanaged.log`、`semantic-stack-unmanaged-folded-{1,2,3}.png` |
| 固定 stack、带标题与符号的工具栏，regular 自动显示 / compact 隐藏标签栏 | 三次正常；说明此前纯图片 stack 的空胶囊不能直接当作产品整组按钮丢失的同机制证据 | `Probe/semantic-stack-compact.log`、`semantic-stack-compact-folded-{1,2,3}.png` |
| 相同语义工具栏、显隐规则，改回 regular split / compact stack | 三次均丢整组按钮与 back，底部留下空胶囊，`path=[1]` 仍在 | `Probe/semantic-switch.log`、`semantic-switch-folded-{1,2,3}.png` |
| UIKit 标签容器承载共用的 `NavigationContent.swift`，保留 regular split / compact stack，regular 显示 / compact 隐藏标签栏 | 三次保留全部按钮、`path=[1]`；未出现 appearance 不平衡警告。由纯 UIKit 标签容器管理同一显隐策略，SwiftUI 页面源码共用 | `Probe/hybrid-switch.log`、`hybrid-switch-folded-{1,2,3}.png` |
| 保留 SwiftUI TabView；同一个 split 用原生 `List(selection:)` / `NavigationLink(value:)` 驱动详情，不切换容器或手动指定 compact column | 三次保留全部按钮、`path=[1]`，无 appearance 不平衡警告；是可继续验证的导航建模修正方向，尚非产品验证通过 | `Probe/native-selection.log`、`native-selection-folded-{1,2,3}.png` |
| 上一项的工具栏换成产品的 Label 前后箭头 + 固定 spacer + Image-only More Menu / accessibilityLabel | 三次 back 与箭头保留，但 More 的胶囊可变空，仍有 appearance 警告；因此需要把导航交接与 Image-only Menu 的承载分开处理。带标题与符号的 Menu 对照三次完整 | `Probe/product-selection.log`、`product-selection-folded-{1,2,3}.png` |

`switch` 模式下始终隐藏 tab bar 的另一次运行在折叠时被清空为 `path=[]`、回到列表，
不作为「详情按钮是否保留」的有效对照（`Probe/always-hidden.log`）。

用户补充真机恢复路径：**滑动返回列表，再次打开详情即可恢复**，不必杀 App 或重建整个 TabView。
纯图片标签与带语义标签的结果有区别，不能把所有「空胶囊」与「整组丢失」都假定为同一内部机制。
目前证据把故障缩到 TabView 与 SwiftUI 导航工具栏交接，尚未确定是框架桥接的哪一个机制，
不能把「移除标签栏」或「改用 UIKit」这类改变当作已完成的产品根因修复。

原生 selection 模型的返回往返也已验证：内屏打开 → 折叠 → 点系统 back，
SwiftUI selection / `path` 自动清空为 `[]`，UIKit 栈从 2 回到 1；再点列表第一项可重新进入。
见 `Probe/check-native-return.py`、`native-return.log`、`native-return-{before,after,reopened}.png`。
这是实施前的探针记录；实施与产品回归状态见本文开头。

### SDK 行为门槛诊断

`git diff d3bb833 38d0ff0 -- RootView.swift ListDetail.swift AgentDetailScreen.swift`
（各自完整仓库路径）为空：build 10 与 11 的这三处源码完全相同。
用户批准只在独立探针中调整 linked-SDK 声明，按 27.2 → 27.1 → 27.2 重复同一路径。
这是观察框架兼容路径的诊断，**不等同于原版 Xcode / SDK 27.1 编译，不能作为出包或产品修复**；
阴性结果也不能排除编译器、SDK 头文件或其他链接差异。

变体在 `Probe/SDKMarkerComparison/`：两份都经 `vtool` 重写同一 `LC_BUILD_VERSION`、相同临时签名，
保留 minOS 17.0 与 LD 27106.1，只把主可执行文件的 SDK 字段设为 27.2 或 27.1。
`NavigationProbe.debug.dylib` 相同（SHA256 `47f006c346f6f9be7a09d708ab92b73cd7e8d73f43c54915e09dcdaac87dcb3a`），
`metadata.json` 与冻结源码保存在该目录，`run-comparison.py` 记录三个变体的精确运行参数。

第一轮直接覆盖安装有问题：27.1 变体安装后读出的可执行文件仍是上一份 27.2 内容，
所以 `sdk271-marker*` 的第一轮结果无效，不能用于 SDK 推断。
重跑改为每轮卸载**独立 probe**、再安装，逐份断言主程序与 debug dylib 的 SHA256 与输入相同，
运行中再次确认 27.1 主程序字段没有被替换（`sdk-fresh271-while-running.json`）。
`run-fresh-comparison.py` 的 27.2 → 27.1 → 27.2 三组各三次开合，均仍出现整组按钮消失。
证据是 `Probe/sdk-fresh272-a-*`、`sdk-fresh271-marker-*`、`sdk-fresh272-b-*` 与三份 installed JSON。
**仅更改主程序 SDK 声明没有改变现象，不能据此宣称 SDK 27.2 的运行时门槛是根因。**
debug dylib 仍由 27.2 编译且声明 27.2；这组阴性结果不替代原版工具链对照。
