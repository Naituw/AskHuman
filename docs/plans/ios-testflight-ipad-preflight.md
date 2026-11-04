# iOS 出包预检接受 iPad 实施计划

需求与决策见 [规格](../specs/ios-testflight-ipad-preflight.md)。状态：2026-11-04 起草；同日用户批准按计划实施，实施中。

本计划自成一体：先复述要做什么与全部规则，再给改动清单、各部分设计、验证与提交。**任何与本计划不一致的做法，先经 AskHuman 问用户。**

## 0. 要做什么

main 上的 iOS 工程已经把主 App 与通知扩展的设备族都改成 `1,2`（iPhone＋iPad），iPad 四向，iPhone 仍只有竖屏；出包预检却还写死只认 iPhone（`UIDeviceFamily == [1]`），所以下一次出包会被拦下。要做的是：

1. 把 `upload.py` 的预检与签名 helper 里的归档检查改成「设备族必须恰好是 iPhone＋iPad」。
2. 在 `upload.py` 预检里新增 iPad 方向检查，把「iPad 必须声明四个方向」这条 Apple 上传校验规则提前到本地。
3. 在真实归档上证明新检查成立，并证明旧的仅 iPhone 归档现在会被拒绝；全程不上传、不占用 build 7 的号码。

### 全部规则

- **设备族**（主 App、通知扩展各自的归档 `Info.plist`）：`UIDeviceFamily` 是整数数组（布尔值不算整数），排序后恰好 `[1, 2]`；顺序无关；缺失、含非整数、含重复、只含一项都拒绝。Python 与 Swift 两处规则相同。
- **iPad 方向**（只查主 App，只在 Python 预检里，helper 不查）：四个方向是 `UIInterfaceOrientationPortrait`、`UIInterfaceOrientationPortraitUpsideDown`、`UIInterfaceOrientationLandscapeLeft`、`UIInterfaceOrientationLandscapeRight`。iPad 上生效的方向列表＝`UISupportedInterfaceOrientations~ipad`（该键存在时，不看通用键），没有则取 `UISupportedInterfaceOrientations`（没有就当空列表）。列表包含全部四个，或 `UIRequiresFullScreen` 为 true，才通过。通知扩展不声明方向，不查。
- **其余既有检查不变**：Bundle ID（主 App 与 `.notification`）、通知扩展的版本号／构建号与主 App 一致、生产推送环境 `prod`、`ITSAppUsesNonExemptEncryption=false`、隐私清单、版本与构建号非空、仅内部测试的导出选项、helper 签名。
- **错误信息**写明：项目现在面向 iPhone 与 iPad（设备族 `1,2`）；若是有意改动，要同时修改 `upload.py` 与 `TestFlightHelper.swift` 并重装 helper，否则去修 `TARGETED_DEVICE_FAMILY`、重新生成并重新构建。
- **不做**：不上传；不做 Apple 服务端校验；不检查图标、启动屏等其他 iPad 项；不改 App 与 `ios/project.yml`；helper 的凭据、上传范围、`notes` 等行为一概不变。

## 1. 改动清单

| 文件 | 改动 |
| --- | --- |
| `.agents/skills/ios-testflight/scripts/upload.py` | 把归档检查抽成可测试的函数，新规则见 §2 |
| `.agents/skills/ios-testflight/scripts/TestFlightHelper.swift` | 导出前的两处设备族检查改成严格的 iPhone＋iPad，见 §3 |
| `scripts/ios-testflight-preflight.node-test.mjs`（新） | 用合成归档测归档检查（`npm test` 已会运行 `scripts/*.node-test.mjs`） |
| `.agents/skills/ios-testflight/SKILL.md`、`references/signing-and-recovery.md` | 文字更新，见 §4 |
| `docs/PROGRESS.md`、`docs/plans/ios-channel.md`、`docs/plans/ios-testflight-build-notes.md` | 把「出包会被预检拦下」的说法更新为已处理，见 §4 |

在本 worktree 基于当前 main 新开分支 `chore/ios-testflight-ipad` 实施；完成后问用户再并回 main。

## 2. `upload.py`

`prepare()` 现在把配置检查、helper 签名、导出选项和归档检查混在一起。把**归档检查**（`Info.plist` 与隐私清单的那一段）原样抽成函数 `inspect_archive(archive)`：只读归档里的文件、不碰配置与 helper，通过返回版本号与构建号，不通过抛 `RuntimeError`。`prepare()` 先做配置与 helper 检查，再调用它，输出的 JSON 摘要不变。

`inspect_archive` 的规则按 §0：

1. 主 App 的 Bundle ID 与设备族（设备族不通过时用 §0 的错误信息）；
2. 生产推送环境、加密声明、隐私清单、版本号与构建号非空（沿用原有检查与原有错误信息）；
3. iPad 方向（新）：错误信息写明「iPad 构建必须在 `UISupportedInterfaceOrientations~ipad` 里声明四个方向，或设置 `UIRequiresFullScreen`；否则 Apple 会在上传校验时拒收」；
4. 通知扩展：Bundle ID，版本号与构建号与主 App 一致，设备族同样严格（不再写成「等于主 App 的设备族」，而是和主 App 一样必须恰好是 `[1, 2]`）。

## 3. `TestFlightHelper.swift`

导出前检查里有两处 `UIDeviceFamily as? [Int] == [1]`（主 App 一处、通知扩展一处）。改成：取出整数数组，排序后恰好是 `[1, 2]`（所以顺序无关、重复与缺失都不通过）。两处错误信息里的「iPhone release」「device family」相应改为 iPhone 与 iPad。helper 的其他部分（凭据、锁、上传范围、`notes`）一行不动。改完用 `helper.py install` 重装：安装器会先把换掉的旧 helper 备份为 `testflight-helper.previous`，回滚就是把它改回原名。

## 4. 文档

- `SKILL.md` 第 5 步：预检清单里的「iPhone device family」改为「iPhone and iPad device family (`1,2`) and, for iPad, its orientation declaration」。
- `references/signing-and-recovery.md`：把「iPad orientations error」一条改写成现状：历史上 XcodeGen 的 target 默认值意外带进 iPad，当时显式设为 `1`；现在项目有意面向 iPhone 与 iPad，两个 target 都设为 `1,2`，iPad 声明四个方向，预检严格检查这两点；有意改变设备族时要同步改 `upload.py` 与 `TestFlightHelper.swift` 并重装 helper。
- `docs/PROGRESS.md`：「iOS 出包前的变更清单…」一节里「下一次出包会被现有预检拦下」那一条删除；build 7 首次真用那一条补一句「这也是 Apple 服务端对 iPad 构建的校验第一次生效，若报新错误，停下报告」。
- `docs/plans/ios-channel.md` 最后一节里「出包预检仍只认 iPhone，下一次出包会被拦下」改成已放宽并指向本计划；`docs/plans/ios-testflight-build-notes.md` §10 的「发现」补一句已由本计划处理。

## 5. 测试

新增 `scripts/ios-testflight-preflight.node-test.mjs`：每个用例在临时目录里按真实归档的目录结构造一个合成归档（`Products/Applications/AskHuman.app/Info.plist`、`PrivacyInfo.xcprivacy`、`PlugIns/AskHumanNotification.appex/Info.plist`），再用 Python 子进程导入 `upload.py` 调 `inspect_archive`，断言通过或拒绝（并断言拒绝时的错误信息指向正确的原因）。覆盖：

- 通过：设备族 `[1, 2]` 加四向；顺序 `[2, 1]`；`UIRequiresFullScreen=true` 且方向不全；`~ipad` 键有四向而通用键只有竖屏（`~ipad` 优先）；
- 设备族拒绝：主 App 为 `[1]`、`[2]`、`[1, 2, 2]`、缺失、`["1", "2"]`、含布尔值；通知扩展为 `[1]` 或缺失；
- 方向拒绝：`~ipad` 只有三个方向；`~ipad` 只有竖屏而通用键有四向（`~ipad` 优先，所以拒绝）；两个键都没有；`UIRequiresFullScreen=false`；
- 既有规则不被破坏：Bundle ID 不对、推送环境不是 `prod`、缺隐私清单、扩展版本号与主 App 不一致各拒绝一次。

故意改坏规则（例如把设备族比较放宽、去掉 `~ipad` 优先）做变异检查，确认测试会变红。helper 里的 Swift 检查是内联在导出流程里的，不单独抽出测试，由 §6 的真实归档验证（正向与负向都用真实归档）。

## 6. 验证（逐步进行，任一步与预期不符就停下、记录、报告，不换参数重试；全程绝不 `--upload`）

1. **准备**：运行 `node scripts/machine-health.mjs`（退出码 2 表示机器在换页，告诉用户并后台跑、分段轮询）；确认磁盘剩余足够。
2. **本地归档带 iPad 的包**（当前 main，构建号写 7，输出放在单独的路径 `ios/build/AskHuman-iPad-Check.xcarchive`，不覆盖 build 6 的归档，也不放在 skill 约定的标准路径）：`xcodegen generate` 后用 skill 里的 `xcodebuild archive` 命令，加 `-jobs 3`、降低优先级，作为后台任务运行并分段轮询，磁盘剩余低于 2 GiB 就中止。要求 `ARCHIVE SUCCEEDED`。
3. **核实实际值**：读这个归档里主 App 与通知扩展的 `Info.plist`，确认设备族是 `[1, 2]`、`UISupportedInterfaceOrientations~ipad` 有四个方向；顺带记录 iPad 图标声明（`CFBundleIcons~ipad`）是否存在，写进实施记录，供判断 Apple 服务端校验的风险。
4. **改代码并跑离线测试**（可与第 2 步的归档并行），测试全绿后做变异检查。
5. **新规则下的预检**：`upload.py <iPad 归档>` 预检通过（不读凭据，旧 helper 也能跑）。
6. **重装 helper**：`helper.py install`（自动备份旧版）；再跑 `status.mjs`，与换前一致；Keychain 仍静默可用。
7. **正向**：`upload.py <iPad 归档> --check-signing`，要求 `EXPORT SUCCEEDED`（本地签名，不上传）。
8. **负向**：对 build 6 的仅 iPhone 归档，`upload.py` 预检必须以设备族原因失败；用 `helper.py export` 直接调用 helper 的导出（只做本地导出，不上传；设备族检查发生在读取任何密钥之前）也必须被拒绝并给出归档预检错误。
9. **清理**：删除 `ios/build/AskHuman-iPad-Check.xcarchive` 以及这次产生的 `ios/build/TestFlightSigningCheck` 内容（避免带「7」的本地归档被误当成真正的 build 7）；build 6 的归档与 `ios/build` 的其余部分按用户此前的选择保留。
10. 跑全部脚本测试，确认没有互相影响。

## 7. 提交与文档

- 分支 `chore/ios-testflight-ipad`；提交类型用 `docs`／`chore`（scope `skills`），都不进面向用户的发行说明：
  1. `docs: add spec and plan for the ios testflight ipad preflight`；
  2. `chore(skills): accept iPad in the ios-testflight preflight and check its orientations`（`upload.py`、`TestFlightHelper.swift`、新测试）；
  3. `docs(skills): describe the iPhone and iPad preflight`（`SKILL.md`、参考文档、`docs/PROGRESS.md` 与两份记录）。
- 实施记录追加到本计划末尾（与计划的差异、各步验证的实际结果、没验证到的部分）。
- 完成后核对：是否有 overview 或其他 spec 因此失真；`docs/overview*.md` 不描述出包流程，预计无需改动。

## 8. 风险与对策

| 风险 | 对策 |
| --- | --- |
| Apple 服务端对 iPad 构建的校验（图标、方向、启动屏等）无法本地确认 | 本地检查与实际 `Info.plist` 核实只能降低概率；build 7 上传时才第一次生效，出现新错误就停下并如实报告，不换参数重试 |
| 重装 helper 后 Keychain 授权失效 | 稳定签名让重编译的 helper 沿用授权（上一次重装已验证）；换后立刻 `status` 与 `--check-signing`；旧版留作 `testflight-helper.previous`。若新 helper 拒绝了一个本应通过的 iPad 归档，旧 helper 同样会拒绝，回滚没有帮助：停下并报告 |
| 归档在机器紧张时很慢或占盘 | 先做健康检查，`-jobs 3`、降低优先级、后台运行并分段轮询、磁盘低于 2 GiB 中止；机器在换页就先告诉用户 |
| 严格规则在以后有意改设备族时会再次拦住出包 | 这是选定的取舍；错误信息写明要同步修改的位置 |
| 带「7」的本地验证归档被误当成真正的 build 7 | 放在非标准路径，验证后删除；真正出 build 7 时 skill 会从确认过的提交重新归档 |
| 方向规则误拒绝合法配置 | 全屏应用用 `UIRequiresFullScreen` 放行；`~ipad` 键优先于通用键，与系统取值一致 |
