# iOS 出包预检：接受 iPad（设备族 1,2）

> 状态：2026-11-04 起草；同日用户批准按计划实施，已实施并验证。实施计划：[ios-testflight-ipad-preflight](../plans/ios-testflight-ipad-preflight.md)。
> 关联：项目 skill [`ios-testflight`](../../.agents/skills/ios-testflight/SKILL.md)；iPad 来自 [ios-large-screen](ios-large-screen.md)；
> 这个问题是 [ios-testflight-build-notes](ios-testflight-build-notes.md) 的 `changes.mjs` 在第一份真实输出里发现的。

## 1. 背景与目标

2026-11-04，main 合入 `build(ios): ship the app to iPad as well as iPhone`（`30d9559e`）：主 App 与通知扩展的设备族都改成 `1,2`，并为 iPad 单独声明四个方向（竖、倒竖、左横、右横），iPhone 仍只有竖屏。这是 [ios-large-screen](ios-large-screen.md) 定下的方向（iPad mini、折叠屏 iPhone 的内屏），它取代了 `ios-channel.md` 里「iPad 适配暂缓」那条。

出包预检里却还写死着「只认 iPhone」，共三处：

- `upload.py` 的 `prepare()`：主 App 的 `UIDeviceFamily` 必须是 `[1]`，通知扩展必须与主 App 一致；
- `TestFlightHelper.swift` 导出前的归档检查：主 App 与通知扩展的 `UIDeviceFamily` 都必须是 `[1]`；
- 文档：`SKILL.md` 第 5 步写着「iPhone device family」，参考文档里「iPad orientations error」一条把「两个 target 都设为 `1`」当作现状。

所以现在出包会在预检就被拦下（`Unexpected bundle ID or device family`）。

这条检查的来历：2026-10-30 首次上传校验就失败过——XcodeGen 在 target 层默认写入 `TARGETED_DEVICE_FAMILY=1,2`，把 iPad 意外带进了「仅 iPhone」的包，Apple 因 iPad 方向声明不全拒收；当时的修法是把两个 target 显式设成 `1`，首次有效构建是 `0.1.0 (2)`。预检的用意是拦住「意外」的设备族变化，不是禁止 iPad。现在 iPad 是有意加入的，检查要跟着改。

目标：

1. 预检接受并**只**接受「iPhone＋iPad」：意外变回只有 iPhone（或只有 iPad）仍会被拦下。
2. 设备族含 iPad 时，在本地检查 iPad 的方向声明，不要等导出、上传一整轮（约 7 分钟）之后才由 Apple 拒收。
3. 在真实归档上证明新检查成立，并证明旧的仅 iPhone 归档现在会被拒绝。

非目标：不上传任何东西，也不占用 build 7 的号码；不做 Apple 服务端校验（`altool` 一类）；不检查图标、启动屏等其他 iPad 项；不改 App 与 `ios/project.yml`；不改 helper 的其他行为（凭据、上传范围、`notes` 等都不变）；出 build 7 本身是另一件事，等用户另行要求。

## 2. 已确认决策

| 编号 | 决策 |
| --- | --- |
| P1 | 设备族规则：主 App 与通知扩展的 `UIDeviceFamily` **都**必须恰好是 iPhone(1)＋iPad(2)。按集合比较，不看顺序；缺失、含非整数、含重复、只含其中一项，一律拒绝 |
| P2 | 签名 helper 里的同一道检查也改成同样的严格规则；helper 的其余检查与行为不变。改动后用 `helper.py install` 重装，沿用上一次的备份与回滚做法（旧版留作 `testflight-helper.previous`） |
| P3 | iPad 方向检查加进 Python 预检（helper 不检查）：设备族含 iPad 时，iPad 上生效的方向列表必须同时含四个方向，或 `UIRequiresFullScreen` 为 true。「iPad 上生效的方向列表」取 `UISupportedInterfaceOrientations~ipad`，没有该键时取 `UISupportedInterfaceOrientations` |
| P4 | 验证：用当前 main 在本地归档一个带 iPad 的包（构建号写 7，只在本地，绝不上传，验证完删除）。在它上面：预检通过、新 helper 的本地签名检查通过；再用 build 6 的仅 iPhone 归档确认 Python 预检与 helper 都会拒绝 |
| P5 | 错误信息写明：项目现在面向 iPhone 与 iPad（设备族 `1,2`）；若是有意改动，要同时修改 `upload.py` 与 `TestFlightHelper.swift` 并重装 helper，否则去修 `TARGETED_DEVICE_FAMILY`、重新生成并重新构建 |
| P6 | 验证时 Apple 服务端对 iPad 构建的校验（图标、方向、启动屏等）无法本地确认，只有真的上传 build 7 才知道；出现新的 Apple 校验错误时，停下并如实报告，不换参数重试 |

## 3. 规则（精确）

- **设备族**（主 App、通知扩展各自的归档 `Info.plist`）：`UIDeviceFamily` 必须是整数数组（布尔值不算整数），排序后恰好 `[1, 2]`。
- **iPad 方向**（只查主 App，只在 Python 预检里）：四个方向是 `UIInterfaceOrientationPortrait`、`UIInterfaceOrientationPortraitUpsideDown`、`UIInterfaceOrientationLandscapeLeft`、`UIInterfaceOrientationLandscapeRight`。iPad 上生效的方向列表（见 P3）包含全部四个，或 `UIRequiresFullScreen` 为 true，才通过。通知扩展不声明方向，不查。
- **其余既有检查不变**：Bundle ID、两者的版本号与构建号一致、生产推送环境、`ITSAppUsesNonExemptEncryption=false`、隐私清单、仅内部测试的导出选项、helper 签名。

## 4. 验证

- 离线：用合成归档测 `upload.py` 的归档检查——设备族各种组合与顺序、通知扩展不一致、方向声明各种情形（含 `~ipad` 键覆盖通用键、`UIRequiresFullScreen`），以及既有规则不被破坏。
- 真实归档（均已由用户批准，逐步进行，任一步与预期不符即停）：本地归档带 iPad 的包并核实 `Info.plist` 的实际值；预检通过；新 helper 的 `--check-signing` 通过（本地签名，不上传）；用 build 6 的仅 iPhone 归档确认 `upload.py` 预检与 helper 的导出都拒绝它。
- 首次真用在 build 7（用户要求出包时）：Apple 服务端校验在那时才第一次对 iPad 构建生效。

## 5. 用户原话与选择（2026-11-04）

- 上一项任务（出包前清单与测试内容）完成后，用户在「下一任务」里选：「放宽 ios-testflight 的出包预检，让它接受 iPad（设备族 1,2）：build 7 前必须先解决，要改 helper，先给你方案再动」。
- 对四个设计问题的选择：设备族「严格等于 iPhone＋iPad（1,2）：app 和通知扩展都必须是」；helper「与 Python 一致，也改成严格的 iPhone＋iPad」；方向检查「加：设备族含 iPad 时，要求声明四个方向，或设置 UIRequiresFullScreen」；验证「做：本地归档、预检、本地签名检查，再用 build 6 归档确认拒绝」。

## 6. 反馈意见

- 2026-11-04：用户审阅规格与计划后选择「批准，按计划实施」，没有调整。
