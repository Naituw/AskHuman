# PROGRESS

记录需要跨会话保留的未完成 / 延期事项和明确下一步。任务 / 需求完成后删除其 section
（历史留在 git）。

## 进行中：iOS 原生渠道（原型已完成，待真实服务与真机验收）

2026-10-24 用户要求整理交接（[移动渠道交接](investigations/native-ios-channel-handoff.md)：杭州 EMQX 传输验证包、
Mac / iOS 交互原型、双向消息与进程重启补发已验证）。2026-10-25 经多轮问答定稿设计：需求
[`docs/specs/ios-channel.md`](specs/ios-channel.md)，计划 [`docs/plans/ios-channel.md`](plans/ios-channel.md)
（作为新渠道 `ios`；EMQX 杭州 + 腾讯云云函数推送中继 + 端到端加密；多 Mac 配对同一部手机；回答 / 确认 / 附件 /
Watch / Follow / 插话 / 新任务），实现地图见 [`docs/overview-ios.md`](overview-ios.md)，用户说明见
[`docs/wiki/ios-setup.md`](wiki/ios-setup.md)。实现在 worktree `HumanInLoop-ios`（分支 `feat/ios-channel`），2026-10-26 已快进合并进 `main`（渠道默认关闭：受 `experimental.enabled` 隐藏，`channels.ios.enabled` 默认 `false`；所有 `feat` / `fix` 提交都带 `Release-Note: skip`，不进发布说明）。代码已随 v0.16.0 发出，发布说明未提及该渠道。

**状态（2026-10-25）**：Mac Gateway / IPC 3→4 / 设置页 / CLI、推送中继 `relay/`、iOS App（收件箱 · 作答 · 确认 ·
附件 · Agents · 关注 · 实时观察 · 插话 · 新任务 · 历史 · 配对 · 通知扩展）全部实现，通过 Rust / Swift / 中继 / 前端测试、
模拟器 UI 测试（en + zh-Hans）和「真实 daemon + 本地 broker」端到端测试。真实服务接入进度（见 [P0 记录](investigations/ios-channel-p0.md)）：
**EMQX 国区实例已接通并验证**（Mac 连得上、账号互相隔离、延迟已量），**APNs 密钥已创建**，**腾讯云中继已部署并验证**（香港 Web 函数；
`/v1/health` 从 Mac 量 100 / 100 成功、p95 1.4 s，沙盒与生产的 APNs 凭据都被接受；函数 URL 只存在 Mac 的本机配置里，不入库），
腾讯云计费已由用户接受（第 0.5 项完成），真机签名构建已通过；同一套端到端对真实 EMQX + 中继跑了一遍，33 项全过（前台延迟：回答 → Mac 确认 p95 195 ms、AskHuman 调用 → 手机显示 p95 254 ms，达标），
顺带修了一个 Mac 端竞态（先 hello、后上报前台的手机漏收中间的变化，计划 §13）。
**模拟器上的真 App 对真实服务的完整场景已通过**（配对 + 校验码、通知权限 → token → 中继登记 → Mac 持有句柄、前台单题 / 多题、权限确认允许 / 带理由拒绝、
Agents 列表 + 实时观看 + 关注 + 插话读取回执 + 完成横幅、任务关闭时的表单提示、真 APNs 沙盒推送 → 通知扩展解密 → 点击打开请求、被关注 Agent 的 `follow.done` 推送、历史、解除配对；
基础场景连续三次、完整场景两次，[P0 记录 §3.7](investigations/ios-channel-p0.md)），并找出四个真实缺陷已修复（点击横幅崩溃、扩展解不开中继的信封、快速 inactive → active 后永久断线、推送句柄没可靠送到 Mac，计划 §13）；
**真机（iPhone Air，2026-10-26，手工，单样本）**：前台提问、锁屏真 APNs 推送（横幅带解密后的问题文字，点横幅直达详情页）、强退后真 APNs 推送（冷启动并直达该问题）各通过一次，答案逐字一致；
真机上又找到第五个缺陷并修复：`inactive` 也告别——控制中心 / 通知中心 / 多任务界面盖在 App 上会让 Mac 看到手机离线 5.9–6.9 秒（现在 `inactive` 超过 10 秒才告别，提交 `fix(ios): keep the connection while the app is only inactive`，
[P0 记录 §3.8](investigations/ios-channel-p0.md)）。**剩下的是真机矩阵的其余项与手机侧测量。**

约束沿用交接：两个 EMQX 实例的消费上限保持 0，不创建管理 API Key，不泄露任何密钥或本地私有配置；不得把模拟器 /
演示 Agent 验证当作完整渠道验收；真实 Agent 的启动 / 发 prompt 测试必须先经用户批准；密码 / `.p8` 只由用户自己输入，Agent 不读取。

下一步（需要用户账号 / 真机，一次一步经 AskHuman 引导）：
- [ ] P0 门禁剩余项（记在 `docs/investigations/ios-channel-p0.md`）：手机在 Wi-Fi / 蜂窝（关闭代理）上调 `/v1/health` 100 次、手机到 EMQX 的 TLS 与 WSS 往返各 100 次、
  真机 APNs（≥ 30 次送达耗时分布、空闲 ≥ 30 分钟后第一条推送的耗时；加密预览、扩展解密、点击回到 App 已在真机各看过一次；Mac 侧空闲 3 小时后的 `/v1/health` 首次请求已量：1305 ms）、
  公司网络端口（用户在那个网络运行 `ios/Tools/broker-probe/probe.cjs`）；任一项不达标先停下问用户，不自行换方案。
  **测量方式**（独立探针 App，还是产品 App 内只在 DEBUG 构建里存在的诊断与时间线）须先经 AskHuman 与用户确认；倾向后者——量的就是产品实际走的 TLS / WSS / 中继客户端与通知扩展，不需要另一个 Bundle ID 与签名，Release 构建不含
  （DEBUG 里记录 `scenePhase` 与连接事件的手机侧时间线，也能补上下面 `inactive` 三项的精确核对）
- [ ] 中继的函数 URL 被腾讯云标为「仅供测试使用」（限速 / 下线条件未公开）：原型期不阻塞，给朋友长期用之前建议绑自定义域名（香港地域不需要备案），由用户决定
- [ ] 真机矩阵其余项（计划 §9 手工矩阵；前台 / 锁屏 / 强退后的真 APNs 推送已各做过一次）：Wi-Fi / 蜂窝 / 切网、通知权限被拒、token 刷新、跨渠道同时回答、Mac 睡眠恢复、设备撤销、重装遗留配对。
  **做法**：用户 2026-10-26 明确反馈「已经测了很多遍，再测很浪费时间」——一次设计好、合并成尽量少的几轮，能用 `devicectl` 自己做的自己做，只有必须用手的动作才叫用户（备忘见 P0 记录末尾「真机手测备忘」）
- [ ] `inactive` 修复在真机上**未验证**的三项（用户决定不再手测，只在文档里记为未验证）：停留短于 10 秒不离线；锁屏 / 回主屏后 Mac 多快看到离线（告别从 `inactive` 挪到 `background`，估计晚 0.1–0.3 秒；最坏有「3 秒无确认补发推送」兜底）；
  强退后 Mac 多久看到离线（靠 Last Will）。有手机侧时间线（见上面 P0 门禁的测量方式）或做真机矩阵其余项时顺带量，不单独叫用户测
- [ ] 性能口径（需求 D10 四条）与流量预算（典型场景 ≤ 300 MB / 月）分段实测，报告原始分布与全部超时
- [ ] 合并进 main 之后仍待处理：
  - `.cargo/audit.toml` 里忽略了 5 条 `rustls-webpki 0.102.8` 的公告（`rumqttc 0.25.1` 带入；它只用这个 crate 的错误类型，证书校验走已修复的 0.103.16，理由与删除条件写在文件注释里）。
    `rumqttc` 换到用 webpki 0.103 的版本、或换掉该客户端后，**删掉这 5 条**并重跑 `cargo audit --file src-tauri/Cargo.lock`。合并后 `cargo audit` 比之前多一条 `unmaintained` 警告：`rustls-pemfile 2.2.0`（RUSTSEC-2025-0134，同样由 `rumqttc` 带入），只是警告，不影响 CI
  - 本机默认的 `cargo` 是 Homebrew 的 1.94.1，比 CI 的 `stable`（现为 1.99.0）旧，clippy 的结论会不同（新 clippy 对 `async_trait` 生成的方法报了 `double_must_use`，已在三个 trait 上 `allow`）。
    提交前要对齐 CI 时：`PATH=~/.rustup/toolchains/1.99.0-aarch64-apple-darwin/bin:$PATH CARGO_TARGET_DIR=/tmp/<独立目录> cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
    （在持久 shell 里用 `export` 会把工具链环境泄漏给后面的命令，写成行内前缀）
  - 决定是否公开该渠道（README 链接、去掉提交的 `Release-Note: skip`）
  - 这条分支的历史已在公开仓库（`origin/feat/ios-channel` 与 `origin/main`）：推送前从历史里清掉了真实 EMQX 实例 ID 与 APNs Key ID（提交哈希因此改写过一次，文档里改用提交标题指代）。
    **不得再改写**（不要 rebase，也不要 force-push）；分支继续开发时同步 main 用合并（2026-10-26 合过一次：`chore(ios): merge main into feat/ios-channel`，合并时解了 `agents/stop.rs` 的冲突）

已知延期（不阻塞原型）：
- Codex 桌面 App 的会话暂不支持手机插话（返回 `unsupported`）；Pi 的扩展只有在 Stop 确认开着时才会随 daemon 启动自动迁移（2026-10-26 实测：日志出现 `migrated outdated Stop confirmation hooks: pi`），只开了生命周期跟踪的用户仍要在设置里重新保存 / 刷新才会上报失败的回合；已在运行的 Pi 会话要重启后才加载新扩展。
- IM `/new` 与 iOS 新任务共用的启动流水线 `daemon/runtime/task_launch.rs::start_task` 没有自动化测试（该文件只有门槛顺序的 1 个测试）。2026-10-26 在真实飞书上走通过一轮 Codex 终端路径；TODO 出队、终端打不开时的清理、Codex App 路径只靠对照新旧实现读代码确认。要补自动测试，得先把 `open_terminal` / `todos` 等副作用抽成可注入的接缝。
- 提问正文里的本地图片尚未改写为 `ahasset://`，列表项的墓碑展示也未做；撤回插话只取回文字、不取回附件。
- 发行给朋友之前：EMQX 自动开户与额度共享、TestFlight 外部测试说明、隐私政策网址与联系邮箱。

## 待验收：iOS 通知时机（已实现并装机，真机一轮只验了一部分）

2026-10-26 实现完成：iOS 不再受活跃槽 / `channels.autoActivation` 约束（每个提问和确认都挂到手机收件箱），手机没在看时的推送要等 Mac 键盘 / 鼠标空闲满
`channels.ios.alertAfterSecs`（默认 900 秒，`0` = 立即）才发，每题每手机最多一条、不重复提醒。规格 [`docs/specs/ios-alert-timing.md`](specs/ios-alert-timing.md)，
计划 [`docs/plans/ios-alert-timing.md`](plans/ios-alert-timing.md)。自动化（Rust 全量、前端、`vue-tsc`、clippy 1.99 `-D warnings`、fmt）全绿，已 `./scripts/install.sh` 装机；
在用户的 Mac 上确认过：活跃槽为 `popup`、`autoActivation` 开时，提问也会挂到手机（`AskHuman channel ios status` 的「待回答请求」≥ 1）；daemon 读到的「Mac 已空闲」与独立读数一致，
并随着停手而增长（0 → 5 秒）。iOS App、线协议、中继都没改。

2026-10-27 真机一轮（`alertAfterSecs` 临时设为 60 秒；Mac「在用」用 `CGEventPost` 的 mouse-moved 事件模拟；两个测试提问用 `AskHuman -q … &` 发出、`kill` 取消；
期间另有两个 Agent 会话的真实提问按同一规则被推送；配置已逐字节还原）：
- 已验证：Mac 空闲越过 60 秒的时刻前后，手机收到两条通知（测试提问一 + 另一个会话的真实提问）；取消提问一后，已送达的那条被改写成「已取消 / 已处理」类文字 ——
  **第一次真机证据：`collapse-id` 替换可用**。daemon 读到的空闲值无人操作时正常增长（到近 5 分钟），合成输入会把它清零。
- 没观察到（用户没盯着看或没回答）：Mac 在用期间手机**没有**通知；推送之后**不重复**；提问到达时 Mac 已空闲则**几秒内**推。这三条目前只有自动化测试（虚拟时钟）覆盖。

下一步：
- [ ] （可选）补测上面三条没观察到的：这次要盯着手机，只看这三点，约 3 分钟。做法：把配置里的 `channels.ios.alertAfterSecs` 临时改成 60（设置页会显示成「1 分钟（自定义）」）；
  「Mac 在用」用 `CGEventPost` 发 mouse-moved 事件模拟（实测会把空闲清零，不需要额外权限）；用 `AskHuman -q … &` 在后台发测试提问、`kill` 取消。时间线：Mac 在用时发一个提问（手机**不应**有通知）→
  停止输入，空闲满 60 秒时来**一条**且不重复 → 再发一个（此时 Mac 已空闲，应几秒内就推）。手机要关专注模式、App 在后台，Mac 上别有别的待答提问（否则它也会被推）。
  做完删掉配置里的键（原文件逐字节还原，可用 `shasum` 核对）。**不要用 `AskHuman help` 之类的词试命令**——它会被当成提问弹窗。
- [ ] 设置页（设置 → 实验 → iOS App 渠道 → 推送时机）的实际外观：下拉、说明、「Mac 已空闲 N 分钟」。组件测试只覆盖结构与文案，没有看过截图（这台机器没给命令行 / Computer Use 屏幕录制权限，没法自己截）。
- [ ] 规格「已知局限」9 里没验证的：远程桌面 / 屏幕共享的输入是否计入空闲；daemon 由 SSH 等非图形会话拉起时读到的空闲值是否可靠。有结果后写回规格，全部验完删掉本节。

后续（本期不做，细节在规格「后续」一节；Follow 的去留已另立一节，见下）：
- [ ] IM 统一（每个渠道各自的「提醒时机」）；保底封顶；有待答题时保持唤醒；D 到点时合并成一条「N 个提问在等你」；Time Sensitive、通知扩展送达回执、划掉追踪、手机上展示「Mac 在用」

## 待验收：iOS「做完 / 出错」通知收窄（Mac 端与 App 端都已实现，真机一轮未做）

2026-10-27 用户决定：App 里去掉手动「关注」星标（Agents 页本来就能看到所有 Agent 的状态），只保留「从手机创建的任务做完 / 出错」的自动通知，
并且和提问一致——Mac 键鼠空闲满 D（默认 15 分钟）才推，你在电脑前不打扰；你回到 Mac 前没推出去的通知作废；任务第一次出结果后就不再通知该会话（一次性）；
App 里用只读小铃铛标出「会通知你」。需求 [`docs/specs/ios-follow-narrowing.md`](specs/ios-follow-narrowing.md)、计划 [`docs/plans/ios-follow-narrowing.md`](plans/ios-follow-narrowing.md)，
同日全部批准。

已完成：Mac 端（P1–P2，已 `./scripts/install.sh` 装机）、App 端（P3，提交 `feat(ios): drop the follow star and mark notified Agents with a bell`）、文档回写（P4）。
自动化全绿：Rust 全量与 clippy / fmt、Swift 包全量、对真实 daemon + 本地 broker 的端到端、模拟器 UI 测试（en + zh-Hans）、
模拟器上真 App 对真实 EMQX + 中继 + APNs 沙盒的完整场景（含被通知的 Agent 第一次结果的推送，之后铃铛消失）。线协议、中继、配置项都没改；
旧版 App 的星标连新版 Mac 照常能加，同样一次性、同样遵守 D。**新版 App 没有装到用户的真机上，Mac 端的新规则也没有在真机上验证过。**

下一步：
- [ ] （可选，需要用户在场）真机一轮：把新版 App 装到手机（Xcode / `devicectl`），`alertAfterSecs` 临时设成 60 秒，从手机「新任务」发一个很快做完的任务
  （Face ID；真实 Agent 的启动要用户批准）——列表行出现铃铛、详情页有一行说明；Mac 在用时做完应**不**推；停手满 60 秒才来一条；
  做完之前回到 Mac 敲键盘则不应补推；做完后铃铛消失（`state.json` 的 `followSet` 为空）。配置用后还原；不要用 `AskHuman help` 之类的词试命令。

## 待验收：iOS 请求正文网页渲染、选项手感、「下一题」逻辑、图片查看与底栏（已实现并过模拟器测试，真机一轮待用户看）

2026-10-28 用户在真机上提了三点（提问页 Markdown 渲染不如 Mac 弹窗且流程图画不出来、不能选字；选项选中是渐变的、手感软；「下一题」会跳过看过没答的题），
选定「正式实现 WebView」后去休息，实现与取舍由我自行决定，记在需求 [`docs/specs/ios-web-markdown.md`](specs/ios-web-markdown.md)（W1–W17）、
[`docs/specs/ios-answer-flow.md`](specs/ios-answer-flow.md)（O1–O6、F1–F11）与 `docs/specs/ios-channel.md` §15 的 2026-10-28 一节，计划在 `docs/plans/ios-web-markdown.md`、`docs/plans/ios-answer-flow.md`。
已通过 Swift 包测试与模拟器 UI 测试；**没有在真机上看过**：凌晨手机能无线连上，Debug 构建已装到 iPhone Air，但启动不了（应是锁屏），我没有去点亮它。

下一步：
- [ ] 用户在真机上验收：Message 的渲染（表格、流程图、选字、链接）、选项点按手感、「下一题」往回滑后不跳题、辅助字号下的底栏；有意见记进 `ios-channel.md` §15
- [ ] 真机冒烟：新版已装在 iPhone Air 上（凌晨 03:40 前后装的 Debug 构建），**没启动验证**；解锁后直接打开 App 就是新版。想在真机上看 Markdown 展示页（Mock，不碰真实数据）：
  `xcrun devicectl device process launch --device 00008150-00043D6A2E12401C --terminate-existing com.naituw.humaninloop.ios -- -AHMock YES -AHScenario markdown -AHOpen req-markdown`，从主屏再打开 App 就回到真实模式（见 `docs/plans/ios-web-markdown.md` §1.6）
- [ ] 图片缩略图与查看器（2026-10-29 用户在真机上反馈：图片缩略图不是 1:1、溢出格子，多张会叠在一起；查看图片时不能横滑到下一张）：已改成 1:1 方块缩略图与左右滑动的图片查看器（`ios-channel.md` §15 的 2026-10-29 一节），只过了模拟器测试（iOS 27.0 / 18.5），**真机上没滑过**。
  **这一版还没装到 iPhone Air**：装机时手机连不上（`devicectl list devices` 里是 unavailable，安装报 CoreDevice 4016；应是睡着了或不在同一网络，我没去唤醒它），Debug 构建物已备好（`ios/build/DerivedDataDevice/Build/Products/Debug-iphoneos/AskHuman.app`，本地、被忽略；若之后又改了代码，先 `xcodebuild build -destination 'generic/platform=iOS' -derivedDataPath build/DerivedDataDevice -allowProvisioningUpdates` 重新构建）；手机回来后 `xcrun devicectl device install app --device 00008150-00043D6A2E12401C <上面的路径>`。
  想看 Mock（四张不同比例的图 + 一份说明）：`xcrun devicectl device process launch --device 00008150-00043D6A2E12401C --terminate-existing com.naituw.humaninloop.ios -- -AHMock YES -AHScenario gallery -AHOpen req-gallery`。已知没做：放大后拖到边缘不会接着翻页（要先缩回原大小）；用户自己回复时附的图片仍不能点开查看
- [ ] 提问页 / 确认页的底栏（2026-10-29 用户：底栏不符合 Liquid Glass、不该有背后的蒙层）：iOS 26 起改成浮动的玻璃栏，栏自己没有底板与分隔线，内容滚到它后面（需求 `ios-answer-flow.md` §3.4 的 G1–G9，`ios-channel.md` §15 的 2026-10-29 底栏一节），iOS 17–25 仍是经典栏。只过了模拟器测试，**真机没看过，新版也没装到 iPhone Air**（当时 `devicectl` 里是 unavailable）。真机上请看：栏下面系统的柔和渐隐是否可以接受（我只去掉了自画的底板，保留了渐隐，因为关掉后文字读不清；若连它也不要，见 §3.4 G2）、「下一题 → 发送」换身份时的过渡、辅助字号下的叠放。Agent 页的插话栏与待办页的底栏没改，仍是经典样式
- [ ] iOS 26.2 模拟器上 `testCreatingATaskWalksThroughTheFormAndOpensTheNewAgent`（新建任务页）稳定失败：一打开「开始」按钮（`task-start`）就是可点的，测试期望不可点（`AskHumanUITests.swift:1020`）。27.0 上通过；改动底栏之前的提交上也是这样，与底栏无关。原因没查（可能是 26.x 上工具栏按钮的 `isEnabled` 报法不同，也可能新建任务页在 26.x 上真的一打开就可开始），要先在 26.2 上看一眼页面再定
- [ ] 若真机上首屏仍觉得跳（页面报出第一个高度前是占位，真机约 0.1–0.2 秒）：做预渲染（请求到达时在后台先算好高度），见需求 §4
- [ ] 后置（需求 §4）：Mermaid 对齐 Mac 的 `sandbox`、`ahasset://` 本地图片、Markdown 附件预览改网页渲染、页内查找
- [ ] 仓库外的原型目录 `/Users/wutian/Developer/HumanInLoop-ios-web-md-proto/`（约 399 MB，正式实现已从中移植）可以删除，删之前请用户确认

## 进行中：跨设备待办同步（多台 Mac，iCloud）——已合并进 main（本地，未推送），两台 Mac 已装机并对真实数据启用，剩下的真机项要用户在场

需求 `docs/specs/todo-sync.md`、计划 `docs/plans/todo-sync.md`（实施记录在其 §10）、实现地图 `docs/overview-todo-sync.md`、界面稿 `docs/assets/todo-sync-ui/`。
2026-10-27 用户评审通过并说「你自己全部搞定，我要上班了」；分支 `feat/todo-sync`，worktree `../HumanInLoop-todosync`（Dev Instance，popup-only）；两者 2026-10-29 已清理（提交都在 main 里）。
S0 可行性 spike 的结果摘要在需求附录 A；spike 工程在仓库外：`~/Developer/AskHuman-TodoSync-Spike/`（复现方法见其中的 `RUNBOOK.txt`）。

已定（用户 AskHuman 八轮，详见需求 §3、§17）：同一 Apple ID 的自己的多台 Mac；按 git remote 自动合并，每个目录 / worktree 是同一份待办的副本；无 remote 不同步；⚡ 不跨机；
v1 不同步附件、同步执行历史；**清空历史是全局的**（清空时提示）；iCloud 模式下可见所有项目；架构 = Rust 同步核心 + 薄 CloudKit 适配器（不用 `CKSyncEngine`，推送只当提示）；
whats-next 前「过期才拉」（> 20 秒，最多等 1.5 秒）；待办窗口统一改侧栏且打开不得卡顿；先自用，之后另行评审随正式版发布。
容器 / bundle id：`iCloud.com.naituw.humaninloop.todosync` / `com.naituw.humaninloop.todosync`。

进度（分支 `feat/todo-sync`，提交即进度）：P0 性能埋点与脚本 ✓；P1 侧栏 ✓（**已合并进 main：`abdfa3b`，未推送**）；P2 同步核心 ✓（模拟与崩溃枚举全过）；
P3 CloudKit helper ✓（`swift test` 过；Xcode 自动签名的 Development 构建对真实 iCloud 跑通）；P4 集成 ✓（读取钩子、`AskHuman todo-sync`、设置卡、侧栏底部状态；Dev Instance 恒关闭所以只能用模拟验证）；
P5 iCloud 模式界面 ✓（虚拟键 `icloud:<身份>`、合并行、仅云端 / 仅本机、移除；与界面稿的对照用浏览器预览目测）；P6 双机验收 ✓（下面两段；睡眠唤醒、联网恢复、登录项形态、24 小时浸泡除外）。各阶段与计划的差异见计划 §10。
**已合并进 main（本地，未推送）**：用户 2026-10-28 看了两台真机验收的报告后选了「现在就合并」（功能默认关、helper 不随版本发布，回来直接用 main 的构建装机验收），推翻了 2026-10-27 22:05「真机验收后再合并」的决定。合并前先把 main（含 iOS 选模型的 4 个提交）合进分支，只有 `docs/PROGRESS.md` 一处冲突；合并后全量测试：Rust 2040 通过、0 失败（10 忽略）；vue-tsc 与 vitest 未重跑（main 这 4 个提交没动 `src/`）。
**真实 iCloud（Development）验证 ✓**（用户 2026-10-27 22:05 授权，Xcode 自动签名创建 App ID / 容器 / 开发描述文件）：两份假 HOME 模拟两台 Mac、共用一个 helper，`scripts/todo-sync-e2e.py` 13 组用例 59 项断言全过（互传 / 并发 / 全局清空历史 / 虚拟键与接管 / worktree / helper 缺席 / 账号变化 / 重建 / 清云与恢复 / 轮询追平 / `kill -9` 循环 / 空闲资源）；真实延迟：`fetch` 中位 0.48 s、`save` 0.70 s、无改动 tick 中位 0.45 s（冷连接 2.45 s），whats-next 的 1.5 s 预算够用；helper 空闲 0.0 % CPU / 9 MiB。验收中发现并修了一个真实缺陷（只剩 `enabled` 标记、`state.json` 丢了时 tick 空转而设置显示已开启，现在自愈重建）。结果与限制见计划 §10，用法见 `docs/overview-todo-sync.md`。
新容器在大陆 iCloud 账号上首次访问会被拒（CKError 15），用户已在 CloudKit Dashboard 打开过 Development 环境；测试数据已用 `clear-cloud` 清掉，假 HOME 与临时文件已删，helper 已停。
**两台真 Mac 对 Production ✓**（用户 2026-10-28 授权 Developer ID 构建 + SSH 到 Mac B，只用假 HOME 与测试数据；Production 的 schema 由用户部署；不含公证）：`scripts/todo-sync-e2e-twomac.py` 8 组用例 22 项断言全过（双向互传、设备列表、并发新增 4.7 秒一致、删除 / 清空互传、B 的 helper 被杀后启动 3.4 秒追平、空闲 10 分钟后再传）。**推送路径是通的，延迟与 S0 同量级**：两台都无人操作时一台保存到另一台收到推送 142 / 107 / 273 秒（10 分钟轮询封顶），刚有往来时 12 / 8.5 秒；helper 空闲 0.0 % CPU、5–34 MiB。需求 §16 第 1 条「两台活跃时 5 秒内」要有人在用的机器才能判定，这次无人值守没有结论。测试区已用 `clear-cloud` 从 Production 删除，两台的 helper 已停、假 HOME 与日志已删，没碰任何一台的真实 `~/.askhuman`、B 的网络与代理。结果与发现（含 `open` 会把 `ASKHUMAN_HOME` 传给 helper 与 tick 的坑）见计划 §10，用法见 `docs/overview-todo-sync.md`。
**两台 Mac 已装机并对真实数据启用 ✓**（用户 2026-10-28 夜在场授权，A = MacBook Pro，B = Mac Studio）：A 装了 main 的构建与 helper 并启用（登录项直接 `enabled`，没弹批准），首次同步 93 条；B 经 SSH 装（AskHuman 在 B 上用 B 自己的 Developer ID 重签，A 与 B 的签名团队不同，直接拷会让 B 的钥匙串不再信任；SSH 里 `codesign` 拿不到私钥，改走 GUI 会话的一次性 launchd 任务），helper 用 zip + `ditto` 拷过去、同样注册成登录项。首次合并后两台各显示 **40 待办 / 86 历史、0 未上传**，设备列表有两台；同一仓库的多个目录互为副本，所以本机目录数与条数比装前多（A：8 → 10 个项目、21 → 34 条；B：8 → 14、23 → 44）。A 约 3.5 分钟后才由 10 分钟轮询追平 B 的首次上传（没收到推送，与无人值守的 107–273 秒同量级），之后一次上传 0.4 秒内推到了 B。对真实数据第一次跑还发现并修了一个真实缺陷：同一条待办在 A 的两个目录里附件数不同（只有一处有文件），附件提示被两个目录轮流改写，A 每个 tick 都白上传一条记录（修法与测试见计划 §10；B 不受这个缺陷影响，之后也已更新到修复后的构建——第一次因 B 的磁盘被 B 上别的项目的测试写满而中止，用户清理后重来）。装机与重签的细节、坑见计划 §10 与 `docs/overview-todo-sync.md`「装到另一台 Mac」。

下一步：
- [ ] **看着用一阵**：两台的真实数据已在同步（见上一段）。有异常（重复、缺失、迟迟不到）先看 `AskHuman todo-sync status` 与 `~/Library/Logs/AskHumanTodoSync/helper.log`；要退回就 `AskHuman todo-sync disable`，装前备份在两台的 `~/.askhuman/state/todos.json.before-todo-sync-2026-10-28`（满意后可删）。B 的旧二进制备份在 B 的 `~/.askhuman/backups/pre-todo-sync-install-*/`。
- [ ] **推送延迟没达到需求 §16 第 1 条的 5 秒，等用户定怎么办**（2026-10-29 凌晨用户在场，两台真 Mac、真实区、探针待办，19 个计入样本）：落地时间最快 3.1 秒、中位 22.7 秒、p90 47 秒、最大 139 秒，5 秒内的 2/17，另有 2 个 240 秒内没落地；其中我们自己的固定部分中位约 3.4 秒（源端去抖 + 先拉后推约 2.2 秒、对端拉取约 1.2 秒），推送到达 0.1–52.6 秒（中位 15.7 秒）才是大头，两台各开 `caffeinate -u` 没有改善。规格与计划已改成实测描述，§16 第 1 条的标准本身没动。出路：① 改标准；② 固定部分压到约 2 秒（去抖 1 秒 → 0.3 秒、刚拉过时跳过「先拉」）；③ 待办窗口可见时 helper 每 3 秒快速轮询；④ 维持现状。数据、方法与局限见计划 §10，工具 `python3 scripts/todo-sync-latency-probe.py run --peer <B>`。
- [ ] **需要用户在场的真机项**（计划 §5 P6、§6 的 E8 / E9，远程与无人值守做不了）：睡眠唤醒与联网恢复触发（用户 2026-10-28 23:29 选了「先不做，稍后再说」；工具已备好：`python3 scripts/todo-sync-trigger-check.py sleep` 让本机睡眠、你按键唤醒，`… network` 把 Wi-Fi 关 15 秒再开，各自核对 helper 日志里 `reason=wake|unlock` / `network` 的 tick，Cursor 会断线几十秒；`selftest` / `status` 不碰系统）；登录项的批准形态（A、B 注册后都直接是 `enabled`，没弹过批准，所以那条路径没见过）与 helper 缺失 / 过期时的安装体验；24 小时浸泡；公证（`scripts/notarize.sh`，需用户授权）。SSH 到 Mac B 的同意只覆盖了装机与 2026-10-29 凌晨的延迟测量（都已用完），再用要重新同意；不改 B 的网络 / 代理设置。
- [ ] **发布**：main 还没推送；P3–P5 的提交带 `Release-Note: skip`（helper 暂不随版本发布）；决定随版本发布时补一条带用户可见说明的 `feat(todo-sync)` 提交
- [ ] P0 的绝对基线 `docs/perf/todos-window-baseline.json` 还没生成（机器常被别的任务占满，负载 5–230，计时不可信）；P1 的性能门用 `--ref <P0 参考二进制>` A/B（参考二进制在 worktree 的 `.askhuman-dev/perf-ref/AskHuman-p0`，被 gitignore）。安静且屏幕亮着时（窗口得真的绘制，否则脚本报「the window never painted」）跑 `scripts/perf-todos-window.mjs` 出基线，并按它校准需求 §9.9 的目标；iCloud 模式的首屏数字要真机开了同步后补（Dev Instance 开不了）
- [ ] 用户文档 `docs/wiki/todo-sync.md`（含英文版）等 helper 能随版本发布时再写
- [ ] 分发给别人前：用另一个大陆 iCloud 账号复测（容器 / 用户层初始化）；helper 打进 `AskHuman.app` 的 `Contents/Library/LoginItems`、公证、自更新下发；附件同步、手机直连 CloudKit 等另行评审（iOS App 读写待办已经由 Mac 网关代答做了，见下一节）
- 构建小贴士（shell 的 HOME 被指向 `/tmp/ph`）：`export CARGO_HOME=/Users/wutian/.cargo CARGO_INCREMENTAL=0 GIT_CONFIG_GLOBAL=/Users/wutian/.gitconfig npm_config_store_dir=/Users/wutian/Library/pnpm/store`，否则 cargo 会重新下载依赖、`./scripts/install.sh` 会卡在 pnpm 的交互提示

## 待验收：iOS App 管理待办（已合并进 main，两台 Mac 与 iPhone Air 都已换上新版，决策全部是 agent 在用户睡觉时代定的，等用户审阅与真机验收）

需求 `docs/specs/ios-todos.md`（决策 I1–I16 在 §3，可整条否决）、计划 `docs/plans/ios-todos.md`；分支 `feat/ios-todos`（worktree `../HumanInLoop-todosync`，两者 2026-10-29 已清理）已于 2026-10-29 早上经用户同意快进合并进 main（本地，**未推送**），并用 `./scripts/install.sh` 装到了这台 Mac（装机后一次真实同步 tick 正常）；合并前已把 main 合进分支并在合并后的树上复测过。同日早上又应用户要求装到了 Mac B 与 iPhone Air：B 的二进制在 B 上用 B 自己的 Developer ID 重签后换入（指定要求与旧版逐字相同，旧版备份在 B 的 `~/.askhuman/backups/pre-ios-todos-install-20261029-080650/`），手机上是 Debug 构建（开发签名，用现成的描述文件，没有改 Apple 账号；没启动、没点亮手机，直接打开 App 就是新版，配对与数据不变）。用户在手机上打开待办页后发现状态行一直停在「正在同步…」：它取自 Mac 回答那一刻的快照，而打开页面时的订阅会让 Mac 立刻拉一次 iCloud（约 1 秒），第一份回答恰好撞上，同步结束后又没有任何通知；已修（`3e52cbb`，分支 `fix/ios-todos-sync-line`，已快进合并进 main，未推送）：手机见到 `syncing` 为真就静默复查，约 35 秒，见规格 §7；手机已换成修复后的构建（在干净 worktree 里构建）。又按用户的选择改了 iCloud 来源的失败表现（`56948da`，分支 `fix/ios-todos-icloud-failure`，已快进合并进 main，未推送）：一次请求没得到回答时不再清空列表，保留上次的行、打开着的项目和同步状态行，只在页面顶部提示一句；手机也已换成这个构建（同样没启动手机）。随后又按用户的选择改了决策 I3 的代价一栏（`8955ce8`，分支 `fix/ios-todos-icloud-offline`，已快进合并进 main，未推送）：iCloud 接入的 Mac 离线、又没有别的 Mac 能代答时，已加载的行与打开着的项目也保留（只读，页面顶部说它暂时连不上），与选某台 Mac 做来源时一致；同步被关掉或那台 Mac 被取消配对时仍然清空；这些行只在这次启动的内存里（I13），重启后没有行可留；手机已换成这个构建（同样没启动手机）。
用户 2026-10-29 夜里说「手机 App 想支持待办：能选设备、能选 iCloud 模式、还有项目来源的问题——你都设计，实现验证，我明天早上验收」，所以没有经过 AskHuman 问答。
做法：手机经已有的 iOS 网关（MQTT + 端到端加密）读写 Mac 的 `todos.json`，新增 `todo.*` 线消息；「iCloud」是一个**来源**，由开着同步且在线的 Mac 代答、掉线自动换，手机不直连 CloudKit（那要给 iOS App ID 开 iCloud 容器，是 Apple 账号上的改动，要用户授权）；项目列表由 Mac 按桌面侧栏同一算法算好，手机只看到不透明 id 与同名后缀，不暴露路径；手机不落盘待办。

自测（2026-10-29，都在这台机器上、没碰真实的 `~/.askhuman`）：Rust `cargo test ios::` 252 条 + `ios_todos` 25 条，clippy / fmt 无告警；Swift 包全套（合并 main 之后的树上）431 条（协议 27、存储 21、传输 14、Markdown 10、核心 144、应用 174、端到端 41，其中 4 条要真实服务、已跳过）无失败；模拟器 UI 测试 `AskHumanTodosUITests` 7 条，英文与中文各跑一遍并看过截图；合并后整个 `AskHumanUITests` 目标 90 条（待办 7、模型 15、主体 56、网页 11）全过，只有要真实服务的 `AskHumanLiveUITests` 1 条跳过；真实 daemon 端到端（本地 broker）5 条；另把真实 `todos.json` 的一份拷贝放进一次性 Mac 对账（15 行与文件逐项一致，最大一份整表 17 KB，拷贝已删）。UI 测试里发现并修复了两个缺陷：待办页先于 Mac 列表上屏时，旧 Mac 会让页面一直转圈（需求 §13 第 7 条）；合并 main 后，字符串目录脚本里「Copy」被本分支的「拷贝」盖过了 main 的「复制」，main 的网页渲染 UI 测试因此失败，已删掉重复项，脚本现在遇到同一个键两种译法会直接报错。**老 Mac 要先装带这个功能的版本**，否则手机上会显示「这台 Mac 需要更新」。

下一步：
- [ ] **用户验收**：看 `docs/specs/ios-todos.md` §3 的 16 条决策（尤其 I2 不直连 CloudKit、I4 设备模式不列仅云端项目、I5 手机不能新建项目、I13 手机不落盘），不同意的整条否决；真机上装一遍（`ios/` 里 `xcodegen generate`，配对后进「待办」标签）；多台 Mac、iCloud 来源自动换机、Mac 离线时的表现要真机才看得全；打开待办页后状态行应当在几秒内从「正在同步…」变成「已同步 · 刚刚」，请确认上面的修复有效。
- [ ] 后置（需求 §11）：手机直连 CloudKit（Mac 都不在线也能用，需用户授权开 iCloud 容器 + Swift 重实现同步核心）、附件的查看与添加、「全部待办」聚合页、清空历史、从 iCloud 移除项目、待办推送、离线编辑队列。
- [ ] 已知局限（需求 §10）：Mac 都不在线时手机看不到待办；线协议里 `helper` 没有「未安装」状态，手机上「没装」与「没在跑」显示成同一句；单条超大待办（桌面端 / CLI 不限长度，手机端限 4000 字）可能让那个项目的 `todo.list` 装不进一条消息而超时。

## 进行中：iOS 新建任务时选模型与强度——Mac 端与 iOS App（P1–P3）已完成，下一步 P4（文档回写与真机一轮）

2026-10-27 用户提出「iOS 版创建任务时可以选择模型」，经 6 题问答定下范围：模型 + 强度一起做；清单以动态发现为主（Claude 用固定别名）；每次都从「默认」开始、不记忆；
详情页显示实际模型、进度页显示所选；先只做 iOS，启动管线放共享层。用户同时强调**严禁触发计费**，并且**Cursor 严禁实测**。同日做了只读探测（没有起任何 Agent、没有发提示词、
没有带真实凭据调模型），写成需求 [`docs/specs/ios-task-model.md`](specs/ios-task-model.md) 与计划 [`docs/plans/ios-task-model.md`](plans/ios-task-model.md)，用户评审批准，
并对六条待确认点作了选择（Grok 用 `grok models`、Codex 一律走 `app-server` 的 `model/list`、Pi 三档强度、做 M10、实现完做一轮真实验证但每次要批准）。

已完成：
- P1（Mac 启动管线：`ModelChoice` 贯穿 `TaskSpec` → `LaunchRecord` → argv 与 Codex 桌面应用；没选时对外行为与以前逐字节相同）。
- P2（Mac 清单 + 线协议 + 网关 + 详情页实际模型，已装机）：`task.options` 多了可选的 `models` / `defaultModel`，`task.create` 多了可选的 `model` / `effort`，
  `watch.frame` 多了可选的 `model` / `reasoning`；旧版 App 连新 Mac 与以前完全一样。实施记录与偏差见计划「P2 实施记录」。
- P3（iOS App，在 `/tmp` 隔离副本里做、对着新 HEAD 重新验证后合入）：新建任务表单在 Agent 与权限之间多了「模型」行（进选择页，超过 12 项有搜索）与「强度」行（菜单），
  都从「默认」开始、不记忆，Mac 没给 `models`（旧 Mac / 没拿到清单）时两行都不出现；进度页有一行所选，Agent 详情页有一行它实际用的模型 / 强度；中文文案齐全；共享夹具有了新用例。
  实施记录、验证结果与已知局限见计划「P3 实施记录」。**App 还没装到真机**（P4 的真机一轮会用到）。

下一步：
- [ ] P4：文档回写（`docs/specs/ios-channel.md` §6.8 与「不能选模型」的局限、`docs/overview-ios.md` 的线消息 / 模块 / 启动参数 `-AHNoModels` `-AHFailTask`、`docs/wiki/ios-setup.md` 与 `.en.md`、
  `docs/overview.md` 的 `agents/` 清单行）；Mac 端已装机，App 的新界面要装到真机才看得到。
- [ ] （实现完做，**每次执行前**经用户批准，会产生模型费用；永远不含 Cursor）真机一轮：见计划 P4。同时验证两件我在本机看不到的事：已登录账号下 `grok models` 的输出
  是否带逐模型档位（带了才补 Grok 的强度行与 M10）、带真实凭据的 `codex app-server` 的 `model/list`。
- [ ] `ios_tasks::create()` 里「读就绪 → 取清单来源 → `resolve_pick` → `TaskSpec.choice`」这段胶水没有单测（`create()` 会真的起任务）；各部件各有测试，整条由上面那一轮覆盖。
- [ ] （可选，等用户决定）强度行的「默认（中）」提示：手机上取的是所选模型自己的默认档位；但 Mac 配置里固定了强度、且该模型支持它时（目前只会出现在 Codex），Agent 实际用的是配置的档位（M10），
  两者可能不同。要精确得让 Mac 在 `task.options` 里按模型各算一个「默认时实际用的档位」。本期按需求原样实现。

## 待观察：拖入文件失败（`daemon.log` 的 `askhuman_drop` 行）

2026-10-17 用户在日常版（v0.14.5）遇到几次「拖图片进窗口没加附件」（来源访达的下载 / 桌面 / 文稿，第二次拖就可以）。已加每种情形的可见提示和不含路径的诊断日志，
决策与读日志的办法见 `docs/specs/popup-inbox-collapse.md` §6.9。
2026-10-22 查到并修复了主因：拖拽开始时 AskHuman 不是前台，wry 在 `draggingEntered:` 里读文件被系统拒绝（`permission_denied`）；macOS 上现在带文件的拖拽进入窗口时先取前台、
拖走不松手再还回（`popup_drag_front.rs` + `swift/PopupDragFront.swift`，根因与验证见 spec §6.10）。Dev Instance 里用自动化的 Dock 堆栈拖拽验证过；同日日常版覆盖安装含修复的构建后，用户亲自把 Dock「下载」堆栈里的图片拖进弹窗成功，`askhuman_drop` 记 `added`。

真实窗口验证（2026-10-17，隔离安装，用户随手拖了 4 次「下载」里的文件）：落到普通请求正文 / 多问题卡片各一次 → `added`，落到「只能选择」的请求两次 →
`not_accepting` 并确认看到橙边提示「…不接收附件」；日志的位置类别、落点、请求编号都对得上，没有读取失败。用户决定不再补测，下面这些**没在真窗口里看过**，只有自动测试覆盖：
桌面文件与系统授权弹窗 / 拒绝后的提示、网页里的图片（应提示改按粘贴）、拖到附件预览区、读取失败 / 部分成功 / 全部重复的提示、成功提示条「已添加 N 个文件到「…」」的外观
（用户没答）、独立弹窗标题栏下的一行。等日常版里真遇到时再对号。

待做：
- 日常版已装含修复的构建，继续观察还有没有「拖不进去」，有就先 `rg askhuman_drop ~/.askhuman/daemon.log` 对号入座（这时再出现 `permission_denied` 才是真的权限问题）。
- 修复没在真机上验证过的部分：Finder 窗口 / 桌面作为拖拽来源（只测了 Dock「下载」堆栈）、其他 macOS 版本。钩子靠 wry 的类名与方法名，升级 wry 后要确认它还装得上
  （`daemon.log` 里出现 `askhuman_drag_front` 行、`installed` 为 false 就是没装上）。
- **日志出来再定**（目前都不做）：图片读取改成异步（现在是一次同步整文件读取，大图 / 网络盘会卡界面）；预览区接收文件（现在落上去只提示、不收）；拖入过程中对预览区显示「不可放」的视觉。

## 延期：Cursor CLI 后台完成控制持久性补强

2026-10-15 用户在入口取舍讨论后决定先不做 CLI 接管。计划与审核见
`docs/plans/cursor-cli-durable-control.md`；本轮未恢复功能代码，独立入口也未获确认。
恢复时先重新确认接入方式，再补构建校验、原生队列预检与跨构建验收。Desktop 已恢复为默认
关闭、带原生行为预检的实验功能，见 `docs/plans/cursor-desktop-restoration.md`；不覆盖终端 CLI。

## 待外部验收：统一作答窗口的真实渠道与其他平台

macOS 合并/可选独立模式、原生画布过渡、普通外缘缩放修复、10pt 未查看蓝点和持续圆形扩散已接入并安装。
行为规格见 `docs/specs/popup-request-inbox.md`；实施与验证记录见
`docs/plans/popup-request-inbox.md` §10–24。最近 Rust 1256 passed / 3 ignored、
最新前端 252 项通过；原生几何 harness 84 项通过，真实 Sidebar 拖动及宽度恢复已验证。
自动化外缘拖动未成功改变窗口尺寸，
程序化几何证据不代替物理拖动观感验收。

2026-10-08 已在 Windows 11 24H2 VM 的交互式桌面验证合并窗口核心流程、到达气泡、草稿与选区、
最小化恢复、Sidebar 拖动、附件三栏和混合类型取消，并复现修复 #15 的空白窗口残留；见计划 §27。
用户已同意延期的真实 IM 终态、Linux 实机、DPI/多屏矩阵仍待补齐（AskHuman 项目
todo #4：a7d02da0-72d5-4191-954d-d0bcb77d322e）。上述 Windows popup-only VM 证据
不能代替这些外部 gate；补齐后删除本节。

2026-10-09 macOS 又完成气泡两次弹跳 / 750ms 勾线飞行和正文本地 Markdown 图片的安装与实窗
验收，见 `docs/plans/popup-arrival-notice.md` 和 `docs/plans/popup-markdown-images.md`。
2026-10-10 正文本地图片又改为有界原文件 URL / 懒加载 / 超限回退，并经安装和用户实窗验收。
其他平台补验应使用最新动效参数，覆盖正文图片的本地 / 项目相对路径、专用协议 URL、
超限 / 解码失败提示和点击打开原图。Windows / Linux 已按用户要求核对官方资料及依赖源码；
本轮没有对应实机证据，详见 `docs/investigations/base64-usage-audit.md`。
后续平台补验还应覆盖历史、待办草稿与 Interject 的纯显示图片 URL、移除 / 切换 / 卸载清理、
超限文件胶囊与提交原路径；本机自动验证记录见 `docs/plans/display-image-urls.md`。
右侧附件大图也应补验原文件 URL、特殊格式临时 PNG、缓存淘汰 / 过期读取释放和 GIF / WebP 动画；
历史继续保存原路径，记录见 `docs/plans/attachment-image-urls.md`。

## 待外部验收：Windows / Linux Popup 附件预览面板

实现、macOS 安装及用户体验验收已完成，自动回归和真实 WKWebView / AppKit 验证记录见
`docs/plans/popup-attachment-preview-panel.md` §9；行为规格见
`docs/specs/popup-attachment-preview-panel.md`。已验证固定右侧、分隔线、外缘缩窄后的主区恢复、
最大化后恢复、阅读状态、图片动画 / 平移、原文件打开及多题草稿提交。展开总尺寸未污染主区偏好。

2026-10-08 已补齐 Windows 11 VM 的安装与 Markdown 三栏展开、到达动效排除预览区、Sidebar
分隔线和草稿保留，见 `docs/plans/popup-request-inbox.md` §27；未据此验收全部附件格式。
剩余为 Windows 完整附件 / 浏览器 / 拖放矩阵、Linux 实机安装、DPI / 多屏、X11 / Wayland：保留已确认的外部
gate（AskHuman 项目 todo #3），完成后删除本节。Markdown 浏览器打开也应在这两平台补验系统默认浏览器、HTML 文件关联不同、启动失败和真实菜单；本地记录见 `docs/plans/markdown-browser-open.md`。不得以 macOS 或纯计算测试代替这些平台实机证据。

2026-10-09 分区搜索已实现并通过 macOS 安装/交互验收，见 `docs/specs/popup-find-scopes.md` §9。
这两平台补验附件列表打开后 Ctrl+F 的区域路由、文本/Markdown/长 diff 的计数与定位、
窄区布局及输入法；PDF 沿用明确的不支持提示。当前自动测试不能代替这些平台实机证据。

2026-10-22 历史详情接入同一块面板（规格 §10，实施记录见 `docs/plans/history-attachment-preview.md` §6）。
这两平台还需补验历史窗口：向右展开与窗口内分栏回落（含条目列表折叠）、关闭恢复、用户移动后保持、
DPI / 多屏、X11 / Wayland 的定位限制，以及右键 / 更多菜单（`popup_preview_actions.rs` 现按所有者 ID 找窗口；
本机没有非 macOS 编译目标，这部分只由 CI 编译）。同样不得以 macOS 证据代替。

## 待外部验收：Windows / Linux 合并窗口的稍后回答与小窗

macOS 的「稍后回答」标记、小窗「待答 N」、拖入文件的停留展开 / 切换、切走缩回已实现，并由用户在真实窗口验收；需求决策见
`docs/specs/popup-inbox-collapse.md`，实施与验证记录见 `docs/plans/popup-inbox-collapse.md` §8。Windows / Linux 的代码路径
（窗口即时变形 `popup_canvas::instant`、以窗口 `Focused` 事件作「用户在 AskHuman」信号、CSS grid 布局、拖放落点单位候选）已写好，
在 macOS 上通过类型检查和自动测试，并在 headless Chromium 里量过：单请求的列表不再挤占正文的网格列。但**没有在任何一台 Windows / Linux
机器上运行过**；用户已同意延期（计划 §1、§3 P7，AskHuman 项目 todo #1）。实机补验清单：

- 窗口变形：缩小 / 展开 / 小窗里来新请求时，小窗内容区是否恰好是页面量出的宽高（Windows：`set_resizable(false)` 去掉粗边框后客户区有无变化；
  GTK：先改尺寸再锁定的顺序，窗管会不会把尺寸钉住或还原）；展开后位置是否仍以小窗左上角为锚、最小尺寸是否恢复；拖动小窗后再展开的位置。
  Wayland 合成器不接受应用设定的位置（代码忽略被拒的位置），看窗口落在哪里是否可接受。
- DPI 与多屏：125% / 150% / 200% 缩放；在缩放不同的显示器之间拖动小窗再展开，尺寸有无取整偏差。
- 页面布局（WebView2 / WebKitGTK）：小窗的列表宽度、3 行高度与滚动，「待答 N」标题和按钮；这两个平台没有红绿灯，顶部留白与展开的 Sidebar 相同；
  系统标题栏下的小窗是否被截断或难看；单请求缩小 / 展开的瞬间正文不闪、不被挤。
- 切走缩回（窗口 `Focused` 事件）：peek 时切到别的应用 → 200ms 后缩回，回来则取消；系统文件选择框 / 确认层打开时是否被误判为离开
  （Windows 的文件对话框会让 popup 失去焦点）；弹窗不抢焦点地弹出时起始状态是否为「不在场」。
- 拖入文件：Windows（物理像素）与 GTK（CSS 像素）的落点单位是否都选对（`dropPoints` 先试一种、失败再退另一种）；停留展开 / 切换 / 边缘滚动的手感。
- 自家附件拖给另一个请求（R17，spec §6.11）：窗口自己发起的拖拽在 WebView2（OLE）和 WebKitGTK 里是否也把 `enter` / `over` / `drop` 送回本窗口（macOS 上会，旧日志里有 `own_drag_out`）；
  送到的话，别的请求行的亮起、停留切换、松手落点、来源请求不收、来源正文不自滚是否和 macOS 一致。事件不送回本窗口的话，这条在该平台上等于没有，要另想办法（例如拖拽开始时就让页面进入拖拽态）。

不得以 macOS 或纯计算测试代替以上实机证据；完成后删除本节。

## 待外部验收：Windows 发布候选

功能与架构实现已在 `codex/windows-platform-parity` 完成；设计、实施记录和 Win11 证据见
`docs/specs/windows-platform-parity.md`、`docs/plans/windows-platform-parity.md` §17.9 与
`docs/plans/windows-unsigned-update-policy.md` §12。当前 Win11 24H2 VM 已通过 PS5/PS7 install、
named-pipe daemon、完整 Rust tests（1127 passed / 2 ignored）、Windows update 专属测试（19/19）、
Clippy、165 Vitest + 5 Node tests、production/release build、真实 authenticated Codex 0.147 E2E、
卸载维护链和未签名 binary fail-closed / `update prepare` 文件锁闭环。交互式桌面已覆盖统一图标、Ctrl
快捷键、Advanced、设置稳定性、真实飞书取消、Windows Terminal 精确 focus、Dev Instance 以及无闪窗
login/logout。外部平台逻辑 Review 的 correctness 项也已收口：daemon metadata watcher、共享 Windows
path identity、launchId 平台标记和 rename 覆盖语义均有 Windows 原生回归证据。

发布认证仍依赖仓库外状态：

- 准备干净 Windows 10 22H2 x64 VM 并复跑核心矩阵；
- 生产 Authenticode、timestamp、publisher identity pinning 与 SmartScreen 认证已决定暂缓，后续作为
  独立签名项目恢复；当前 unsigned Windows 已固定关闭自动 apply，并提供 `update prepare` 手动闭环；
- 补齐真实交互式 Windows 桌面的 DPI/多屏/输入法/文件选择/声音完整矩阵；
- 如发布认证要求覆盖每个 IM provider，以真实凭据补跑飞书以外渠道；当前飞书真实取消链路已通过，
  其余渠道为 deterministic mock 覆盖。

以上 gate 完成后删除本节。Windows ARM64、原生 installer、Windows Server/RDS 多会话是已确认后置
项目，不属于当前 release candidate blocker。

## 待验收：本地 Markdown Mermaid 图表的跨平台实机运行

完整实现、自动测试、本地浏览器 sandbox / 布局验证与 bundle spike 已完成，详见
`docs/plans/mermaid-rendering.md` 的实施记录。仍需在 Catalina 级 WKWebView、Windows WebView2 与
Linux WebKitGTK 分别跑一次计划 §8.3 的图型、错误、主题、Find 和回答流程矩阵；当前 macOS Tauri
Popup 已完成本机验收。2026-10-09 又补齐 Markdown 附件预览与离线浏览器快照，用户确认两入口与
深色背景正常；实现、Safari Find / 打印和背景修复记录见 `docs/plans/attachment-mermaid-rendering.md`。
跨平台补验应包含新增附件入口。实机 gate 未齐前不降低安全等级或提高系统要求。

## 定期同步：Codex Shell 判定复刻（codex-permission-remember §6.4）

权限记忆功能复刻了 Codex 的 Shell 判定逻辑（`src-tauri/src/shell_safety.rs` +
`permission_shell.rs`）。版本门控只设下限（`VERIFIED_CODEX_VERSION_FLOOR` = **0.122**，
hook 引入版）；`VERIFIED_CODEX_VERSION_CEILING`（当前 **0.146**，对拍来源 Codex upstream
main `1a817bb95d`，2026-07-24；同批吸收 0.145.0 的 #34271 禁选前缀扩容与 #32232
hook-before-guardian 语义，见 spec D50-D53）是最近一次逐行对拍的版本，用户装机超出它时
功能**保持启用**、worker stderr 记一条日志。因此同步不再是紧急事项，但仍需**定期**
（Codex 新 minor 发布后）对拍以下上游文件并抬升已审计版本（相对 codex-rs/）：

- `shell-command/src/bash.rs`（`bash -lc` 脚本拆分）
- `shell-command/src/command_safety/is_safe_command.rs`、`is_dangerous_command.rs`（heuristics）
- `core/src/exec_policy.rs`（fallback 判定 / amendment 派生 / `BANNED_PREFIX_SUGGESTIONS`）
- `config/src/loader/`（配置层叠与项目信任，影响 rules 文件发现与 managed 检测）
- `codex execpolicy check` 的 CLI 契约（参数与 JSON 输出；有 ignored 集成测试
  `permission_shell::tests::real_codex_cli_contract_when_available` 可拿真机验证）

无差异则只改常量 + 记录新 commit；有差异先改 port 再抬已审计版本。若上游出现我方未携带的
**放宽**类变更（新增 safe 命令等），只影响覆盖率；出现语义级破坏（拆分格式、hook 契约）时
fail-closed 机制会自动降级为基础弹窗，届时按 D35 修订的证据链重新评估。

## 待办：Cursor 全局 Rules 迁移为用户级 always-on Skill

调查与候选设计见 `docs/investigations/cursor-global-rule-user-skill.md`。无 workspace folder 的 Cursor IDE
不创建项目 Rules 加载器，因此不会读取 `~/.cursor/rules/askhuman.mdc`。未来改为用户级
`~/.cursor/skills/askhuman/SKILL.md`，旧安装显示“需更新”，迁移时先写新 Skill、再清理旧托管 MDC。
Grok 默认会扫描 Cursor Skills，候选 frontmatter 已设计为对 Cursor 常驻、对 Grok 不可调用。

## 待办：Cursor 等待期保活（避免“无进展”规则中止长等待，待讨论）

调查与候选设计见 `docs/investigations/cursor-idle-wait-keepalive.md`。AskHuman 长时间阻塞时回合内没有新
checkpoint；网络中途断开后，Cursor 连续两次“流已建立却无 checkpoint”的自动续跑会被它自己中止
（`Agent stopped retrying …`），实测 5 个长回合里 4 个因此终止。候选：① 规则层改为后台 AskHuman + `AwaitShell`
分段等待；② 程序层让 AskHuman 每 N 分钟返回“仍在等待，请重跑”，daemon 保持问题挂起。2026-10-26 用户决定先记录、
后续讨论；尚未修改任何规则 / 代码，也未做需计费的真实验证（需先经用户批准）。

下一步：与用户确认方案取向和周期 N；`transportCap=10` 仍是硬上限，要延长还需降低断线频率；方案 ② 可与
“daemon 即时换新与在途提问恢复”共用请求身份 / 重连基础设施。已登记 AskHuman 项目 todo #8。

断线来源（本机链路 vs Cursor 服务端）仍未定：2026-10-26 的统计显示失败率随流的年龄上升、不同流不成簇
（见调查文档 §2.3）。待用户有空时，在另一台机器上用同一份只读统计做对照分析（交接包在用户手里，不在仓库内）。

计费口径待核对：Dashboard 事件对应“带模型调用的请求流”，10-19 有 2 次只有心跳的续跑没有事件，所以“空等中的续跑
不计费”是个待验证的假设；若成立，保活会把免费的空等续跑变成计费请求（调查文档 §2.4）。零成本验证：重新导出
Dashboard CSV（覆盖 10-26 00:47 之后）再对账。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。

## 待办：daemon 即时换新与在途提问恢复（延期）

用户希望评估：本地安装或版本升级时，无需等现有提问答完即可更新 daemon，换新后继续原提问。
2026-10-07 已完成初步可行性调查；用户决定暂不处理，记录为 AskHuman 项目 todo #7
（`416b9760-cf6e-4cf1-bed1-22a04fab1df5`）。尚未修改功能，也未确认具体恢复体验或实施方案。

当前请求/抢答状态只在 daemon 内存中，关停会取消在途请求；已受理请求的 CLI 在连接断开时返回
退出码 3，Popup Host 在 EOF 时退出，前端作答草稿也未持久化。可行方向是增加可靠的请求/终态存储、
稳定请求身份与恢复凭证、CLI/Popup 重连，以及答案确认和副作用去重。CLI 进程可在 daemon 重启期间
保持存活和原 stdout 管道，但现有旧客户端没有恢复能力，首次上线及不兼容协议仍需安全回退。

下一步先确认保留旧 Popup Host 还是重建新版弹窗并恢复草稿，再设计跨版本兼容、多等待者、IM 卡片
接管和 Windows exe 文件锁处理；以上均为待评估事项，现有 graceful drain 策略继续有效。

## 待办：多份安装互顶刹车的收尾（Windows 与配置扫描）

2026-10-22 落地了 Daemon 的 hold（同一对二进制 10 分钟内互顶 ≥2 次后不再让位）、换新原因记录和
`AskHuman doctor install`，决策与验收见 `docs/specs/daemon-graceful-drain.md` D10–D12、§6。macOS 上做过
隔离 e2e 与真机 PATH 扫描。下面两项用户决定之后再做（AskHuman 项目 todo #2、#3）：

- `scripts/install-windows.ps1` 装完后调用 `AskHuman doctor install --warn-only`。Rust 侧已跨平台编译，
  但 Windows 上的 PATH 扫描（只查 `AskHuman.exe`，不解析 npm 的 `.cmd` shim；`canonicalize` 会带
  `\\?\` 前缀）与 `--version` 探测没在 Windows 实机验过。
- 扫描各 Agent 的 hooks / MCP 配置里写死的 AskHuman 路径，与 Daemon 实际运行的二进制对比。
  现有补救是装完用 `AskHuman agents update` 重写。
