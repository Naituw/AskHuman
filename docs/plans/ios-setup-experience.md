# iOS App 渠道设置体验：开发与交付计划

日期：2026-11-22。状态：**设计已通过（第 3 轮）**；P0（原型与规格）、P1（卡片）、P2（后端）、P3（文档与落地页）、P4（iOS 文案与出包）已完成并提交（Apple 侧动作逐项取得了用户同意：helper 的公共链接操作、0.1.0 (29)、公共组与公共链接，2026-11-22）；P5 放出还没开始，碰 Pages / 发版的步骤逐项另问用户。需求与决策见 [`docs/specs/ios-setup-experience.md`](../specs/ios-setup-experience.md)（全部文案在其 §14，实现以它为准），原型与截图在 `src/prototype/settings-ios.html` 一组文件与 `docs/assets/ios-setup-experience/`。

**工作位置：主工作区**（用户第 2 轮选定，沿用现有安装流程）。同一棵树里常有别的会话的未提交改动（现在有 `docs/PROGRESS.md`、`docs/specs/ios-traffic-reduction.md`、`docs/wiki/ios-setup*.md`、`src-tauri/src/app/attachment_owner.rs` 等），提交时只暂存自己的文件或自己的改动块（`git apply --cached` 一个只含自己改动块的补丁），不碰别人的。

顺序：P0 设计 → P1 卡片（前端，仍在「实验」里）→ P2 后端小改（探测、Agent 指南）→ P3 文档与落地页 → P4 iOS 文案与出包 → P5 放出（翻可见性 + 发版）。
**可见性只在 P5 的一个提交里翻**：P1–P4 做的所有东西仍在隐蔽开关后面，随时推送到 `main` 也不会被别的发布顺带带出去（卡片一旦可见，步骤 2 的二维码就必须能用，见规格 §10.4）。

提交约定：P1–P4 的提交带 `Release-Note: skip`；P5 的放出提交是唯一一个面向用户的 `feat(ios)`，写好 release note（见 P5）。

## P0：设计（已完成）

- 原型：`src/prototype/settings-ios.html`、`IosSetupProto.vue`、`IosProviderGuide.vue`、`IosConnForm.vue`、`IosPairBlock.vue`、`iosSetupState.ts`、`settings-ios-main.ts`、`settingsIos.css`、`assets/ios-*-qr.svg`；真实 `SettingsView` + mock 后端，`pnpm dev` 后打开 `http://localhost:5180/prototype/settings-ios.html?scene=<场景>`（场景 `off|agent|broker|emqx|emqx-filled|flespi|custom|app|pair-start|pair|confirm|try|try-sent|steps|ready`，另有 `theme=dark`、`w=480|560`、`focus=1`、`fold=0`）。
- 截图 18 张在 `docs/assets/ios-setup-experience/`（浅色、深色、窄窗口）。
- 规格与本计划。第 3 轮用户已通过（规格 §12），分两个提交：① `docs(ios)`：原型、截图、规格、计划、`PROGRESS.md` 里自己那一块（`Release-Note: skip`）；② `fix(ios)`：第 2 轮顺手改掉的措辞——`src/i18n/zh.ts`、`en.ts` 里 EMQX 的 `limits`、`docs/wiki/ios-broker*.md`、`docs/specs/ios-open-access.md` 的 OA6（只有隐蔽开关后面看得到，带 `Release-Note: skip`）。

## P1：卡片（前端；仍放在「实验」里）（已完成）

目标：把 `IosChannelCard.vue`（473 行）重做成规格 §3–§9 的样子，行为沿用现有的配置读写、测试连接与配对，不改后端。

1. **步骤判定**：新增 `src/views/settings/useIosSetup.ts`，纯函数 `deriveIosSetup({ enabled, host, username, passwordFilled, gateway, phones, appInstalled })` → `{ configured, connected, tag, tone, done, current, ready }`（规格 §3.2–§3.3），不碰 Vue 响应式，写单测：全空、已启用未填全、填全未连接、已连接无手机、`pendingConfirm` 的手机不算、有手机、`last_error` → 异常、`appAcked` 只影响步骤 2、已就绪的判定。
2. **组件拆分**（原型 → 产品）。卡片壳 `IosChannelCard.vue` 持有手风琴与三种形态；它的状态来自 `useIosChannel.ts`，各部件通过 `iosChannelContext.ts` 的 provide / inject 读同一份：
   - `IosAgentBox.vue`：提示词框与复制；
   - `IosProviderGuide.vue` + `IosRich.vue` / `iosRich.ts`：三个磁贴 + 编号步骤 + 提示；**数据进 `iosBrokers.ts`**（每家的步骤键、官方入口链接、提示种类、可复制的「片段」），文案进 i18n，`**粗体**`、`` `代码` ``、链接与可复制片段由 `iosRich.ts` 解析；`detectBroker` / `brokerFill` / 失败提示分类沿用；
   - `IosConnectionForm.vue`：字段、测试连接、导入、「更多设置」（就绪后同一份表单收进「连接设置」，多一个「复制连接串」）；
   - `IosKeepAlive.vue`：保活行，读写 `general.daemonLifecycle`，锁定判定沿用 `AgentTasksSwitchRow` 现有的逻辑，不新增配置；
   - `IosJoinApp.vue`：步骤 2（二维码、复制链接、在浏览器中打开、「我已装好」）；
   - `IosPairing.vue`：配对块与设备列表（沿用 `useIosChannel.ts` 的逻辑）；
   - `IosReadyBox.vue` + `useIosTestQuestion.ts`：已就绪框与测试问题行；后者封装 `setup_probe_start("channel","ios")` / `setup_probe_cancel` 与 `setup-probe-result` 事件、结局、窗口关闭时取消（120 s 超时由后端管）；
   - `IosPushTiming.vue`、`IosTrafficInfo.vue`：就绪后保留的两块，从旧卡原样搬出；
3. **步骤 2**：二维码用现有 Tauri 命令 `ios_pair_qr(text)`（名字是配对专用，实际接受任意文本）渲染落地页地址；落地页地址是一个常量（`iosBrokers.ts` 旁的 `iosLinks.ts`），**不放 TestFlight 链接**。「复制链接」「在浏览器中打开」用现有的 `openUrl`。
4. **样式**：原型用 `ios2-` 前缀避免冲突；产品并回 `settings.css` 的 `ios-` 一组，删掉旧样式；只用设计令牌，深色、480 / 560 宽度都要看。
5. **i18n**：新键放 `settings.channels.ios.*`（中英），文案逐字取自规格 §14（含提示词中英）。旧卡整个被换掉，所以 `settings.experimental.ios.*` 在 P1 就整体迁到 `settings.channels.ios.*`（`AwayCard.vue` 的时长文案一并改引用），没有留着不用的旧键；P5 只剩位置与门控。
6. **事实一致性测试**（规格 §7.5）：前端 `iosFacts.test.ts` 把卡片文案与 `ios-broker*.md`（zh / en）对照：关键数字（100 万连接分钟、1 GB、¥100、8760 小时、60 天、8883 / 8084 / 443、`ah/<用户名>/#`）、`iOS 17` 对 `ios/project.yml`、卡片指向的 wiki 小节锚点确实存在；P2 的 Rust 测试用同一组数字断言 `--agent-guide` 的输出。
7. **测试**：`useIosSetup` 单测；`IosChannelCard.test.ts` 重写（三种形态、手风琴、磁贴选择与填充、复制、保活锁定、测试问题四种结局、「我已装好」只改界面、已就绪后「查看设置步骤」）；`iosBrokers.test.ts` 补新数据。

验证：`npx vitest run src/views/settings src/views/setup src/i18n`、`vue-tsc --noEmit`；把原型页的挂载换成真实的 `IosChannelCard` 在浏览器里用 mock 后端点一遍所有场景（浅 / 深、480 / 560），与规格截图逐张对照；`./scripts/install.sh` 前先 `node scripts/machine-health.mjs`。不改 IPC，不改配置字段，不需要 recovery 迁移。

## P2：后端小改（探测、Agent 指南）（已完成）

1. **探测支持 iOS**（规格 §9.2）：
   - `onboarding/probe.rs`：`ProbeTarget::parse` 接受 `ios`（不改 `channels::ORDER`）；把「`parse_probe_id("setup-probe:ios:abc") == None`」与「`Check::parse("channel", Some("ios")).is_err()`」两个测试改成相反的断言，并补 `policy(Some(ProbeTarget::Channel("ios")))` = 不弹窗 / `Only("ios")` / 无钩子 / 不改当前渠道；
   - `onboarding/probe_run.rs`：`Check::parse` 放行 `ios`；
   - `daemon/runtime/mod.rs`（约 3414 行）：直接挂载 iOS 的条件扩成 `All` 或 `Only("ios")`；顺带把 iOS 的挂载计入 `im_attached`（只影响日志 / 埋点）；
   - `ios/gateway/requests.rs::flush_due_notifications`（约 634 行）：`immediate` 再加 `ask.setup_probe`；
   - 测试：`daemon/request.rs` 里 `setup_probe` 对 `ios` 的断言；`ios/gateway/tests_requests.rs` 里「探测提问在 Mac 刚有输入（`idle < alert_after`）时也立即通知」；`ImScope::reaches_ios` 的纯函数测试。`daemon/runtime` 里没有现成的渠道探测运行时测试可以仿，没有补 `handle_submit` 级的测试：「只挂 iOS、不弹窗、不触发钩子、不进回复历史」由 `daemon/request` 的探测循环与这几处纯函数一起守着，真机一轮再确认。
2. **`channel ios --agent-guide`**（规格 §7.3–§7.4）：
   - 新增 `src-tauri/src/cli/ios_guide.rs`：`render(&GuideState, Lang) -> String` 是纯函数，`GuideState` 只含**已去敏**的状态（启用、主机、用户名、服务商、密码「已填 / 未知」、网关状态、手机数与在线数、生命周期与是否被锁定）；因为输入里根本没有凭据，输出不可能带出凭据；
   - 取状态：新增一个**不拉起守护进程**的读取（实现为 `client::ios_op_if_running`，守护进程没在运行返回 `Ok(None)`），连不上就写「守护进程未运行」；配置只用 `AppConfig::load_without_secrets()`，不读钥匙串；
   - `ios_cmd::dispatch` 增加 `--agent-guide` 分支；`channel ios help` 与 `channel --help` 的 ios 一节各提一行；
   - 文本中英各一份，内容见规格 §7.3 的分节与 §7.4 的规则、命令分级；**所有给 Agent 的规则只写在这里**，卡片复制出去的提示词只有三行、不写规则（规格 D18）；EMQX 的额度与充值提示原样带上；
   - 测试：每个状态 × 两种语言一份快照（未启用 / 已启用未填全 / 已连接无手机 / 有手机 / 守护进程未运行 / 断开带错误）；断言输出里**有**硬性规则与 EMQX 提示、**没有**命令分级里「只能用户运行」的命令被写成「可以直接运行」、**没有**任何哨兵凭据；P1 的一致性测试把本文本纳入。
3. 保持 `ios_cmd::available()` 门控**不变**（P5 才去掉）：P2 的验证要在打开隐蔽开关的环境里做。

验证：`node scripts/machine-health.mjs` 通过后跑 `cargo test` 里 `onboarding::`、`ios::`、`cli::`、`daemon::request`、`daemon::runtime` 相关项与 `cargo clippy -D warnings`；`./scripts/install.sh` 装机后，在隐蔽开关打开的机器上：卡片的「发一条测试问题」对真实手机（用户拿手机）跑一次，另用仓库的 iOS 网关测试夹具验证立即推送；`AskHuman channel ios --agent-guide` 在各状态下读一遍输出；按指南走一遍（真实走查见 P2b：第一次用 `codex exec` 走，暴露出指南只让 Agent 念步骤的问题）。IPC 与 `PROTOCOL_VERSION` 不变（规格 §9.2 的论证）。

## P2b：Agent 代办（规格 D24、D25，已完成）

起因：P2 之后第一次真实走查（隔离实例、EMQX、`codex exec`）里，Agent 只是把步骤发给用户，操作仍由用户做完；用户要求「能让 Agent 做的都由它做，用户能少做就少做」。

1. **指南改写**（`cli/ios_guide.rs`，中英各一份）：规则改成「用户只做只有人能做的事」、第一次提问说清用户要做的事、Agent 生成并保存 Broker 密码、控制台只新增 / 付款页必须停、没有浏览器工具时的做法、确认码比对后才 `allow`、不改保活；步骤 1 用 `printf … | channel set ios … --password-stdin --enable`；步骤 3 改成 Agent 发起配对；命令速查改成两张（直接运行 / 只在用户要求时）；服务商一节面向「谁在操作控制台」；故障对照补粘贴按钮、确认码不一致。
2. **步骤全部展开**：原来「只展开当前步」会让第 3 步永远不显示（只有配对完成才同时完成第 2、3 步），改成已完成的一行、其余全部展开、第一个标「（当前）」。
3. **两条新命令**：`channel ios pair --no-wait`（只打印配对码就返回，配合 `| pbcopy`；票据在守护进程里，返回后仍有效到过期）；`channel ios test-question`（复用探测 `build_task` + `internal_ask`，120 s 超时，文案同卡片，没有已配对手机时直接报错）。不碰 IPC，不升 `PROTOCOL_VERSION`。
4. 测试：`cli::ios_guide`（步骤全部展开、规则编号与步骤里的引用一致、命令速查里的命令都存在于 `channel ios help`、配对步骤含 `pair --no-wait | pbcopy` / `pairing.sas` / `allow` / `deny`、状态块仍无任何凭据）与 `cli::ios_cmd`（`--no-wait` 解析、帮助、测试问题各结局的文案）。
5. 走查（真实，交互式）：隔离实例 + 在 Terminal 窗口里用 Codex CLI（不用 `codex exec`），只给卡片上的提示词；Agent 在用户登录的 Edge 里操作 EMQX 控制台，用户登录并做手机上的几步。已走通（2026-11-22，约 7 分钟），发现的两处多余提问已改指南，已有部署改为由用户选（规格 §12 第 7 轮）；用户决定不再重走一轮。规格 §11 的「用户在真机上从零走完整条链路」随之改成：Agent 按提示词配，用户只做 D24 里只有人能做的事。
6. 第二台 Mac（规格 D25，走查里配对码被手机拒绝暴露出来）：指南第一问加「这部手机已经配对过另一台 Mac」，选了就用连接串沿用第一台的 Broker 设置（用户在那台 Mac 上复制，Agent `pbpaste | channel ios import -` 再 `channel enable ios`，跳过建账号与第 2 步）；第 2 步开头一句说明已装好就直接配对；故障对照补手机的「Broker 账号不同」提示（不建议「忘记所有配对」）；规则 1、3 把连接串列为人做的事和要保密的内容；`channel ios import` 升到「Agent 可以直接运行」。测试 `a_second_mac_reuses_the_broker_account_of_the_first` 把引用的手机提示、「忘记所有配对」对着 `Localizable.xcstrings`，按钮路径对着卡片的 `zh.ts` / `en.ts`；用隔离实例对一对 `export` / `import -` 管道跑过（假凭据，指南状态块里只有主机与用户名）。没有再让真实 Agent 走第二台 Mac 的路径。

## P3：文档与落地页（已完成）

1. **落地页**（规格 §5.2）：`scripts/build-ios-beta-site.mjs` 的根页改成中英双语的 TestFlight 落地页；`docs/public/ios/testflight.json`（`{"url": "…"}`）由 P4 拿到真实公共链接后才新增，构建脚本读取并校验（存在但不是 `https://testflight.apple.com/join/…` 就失败）；**文件不存在时不显示按钮，保持现在「目前仅限邀请」的文案**，所以 P3 的改动不改变 Pages 现状，Pages 也可以先为别的原因（新的隐私政策）重新发布。给脚本加 `node:test` 校验（缺失、域名不对、通过三种）。本地 `node scripts/build-ios-beta-site.mjs` 后在浏览器里看 `site-dist/index.html`（有链接与没有链接两种、浅 / 深、窄屏）并留截图。
2. **朋友内测指南** `docs/public/ios/beta.md`、`beta-zh.md`：改成公共链接版（去掉「邀请人发来的邮件邀请与中继地址」「打开实验性功能」，配置步骤改指向卡片，保留自部署一节）。
3. **Wiki**：`docs/wiki/ios-setup.md`、`ios-setup.en.md` 的入口路径与步骤改成卡片的三步；`ios-broker.md` 的事实不动，只在开头加一句「卡片里的步骤是这里的摘要」。
4. **规格与概览**（P3 只改不依赖可见性的部分）：`docs/specs/ios-open-access.md` 的 OA12 标「已由 ios-setup-experience 接手」；`docs/overview-ios.md` 只改变了的部分；`docs/specs/ios-channel.md` 的 D22 等放到 P5 与可见性同一提交。
5. **README**（用户第 2 轮已同意，规格 D21）：中英各加一节 iOS App，保持简短：一句话介绍、落地页链接、设置指南链接。
6. **`docs/PROGRESS.md`**：开工时加「进行中：iOS 设置体验」一节（下一步、卡点、等用户的事）；P5 放出时把 90 天构建刷新的承诺写进去；与本任务重叠的旧条目（「发行给朋友之前…」「带默认中继的 Mac 版本发布后改朋友内测指南」等）随各自完成删除。

验证：构建脚本的测试；`site-dist` 里所有站内链接可达；文案与代码事实（路径、按钮名）逐条对一遍。

## P4：iOS 文案与出包（已完成；Apple 侧动作逐项取得了用户同意）

1. **App 文案**：`ios/AskHuman/Views/Inbox/OnboardingView.swift:24`、`ios/AskHuman/Views/Pairing/PairingSheet.swift:80`，以及 `ios/Tools/make-catalog.py`（424、453 行）与 `Localizable.xcstrings`（用脚本重新生成）里的「设置 → 实验 → iOS App 渠道」改成「设置 → 通信渠道 → iOS App」。`xcodebuild` 模拟器编译；检查没有别的地方引用旧字符串（快照、测试）。
2. **审核与测试材料**：`docs/release/ios-external-beta/*.txt`（审核备注、Beta 描述、What to Test）与 `ios-testflight` 技能里仍写「自备 EMQX」「实验」的字样，一并改成中立的新说法（`ios-open-access.md` P2 当时刻意留到这次出包）。
3. **公共链接**（规格 §5.3）：给签名 helper（`ExternalTesting.swift`，现在封闭到 Friends 组 1–5 人、公共链接关）新增**有界**的「开 / 关公共链接、设测试员上限」操作，固定到本应用与新建的外部组「AskHuman Public」（上限 1,000，规格 D19），写入前先读现状并计算差异，不增加任意 API 调用能力；Friends 组保持原样。**只有用户明确同意后才对 App Store Connect 执行**，同意只对所申请的那一项有效。
4. **出包**：走 `ios-testflight` 技能，先列变更清单；新构建（仍是 0.1.0，沿用先前的 Beta 审核，规格 D23）进入公共组，审核状态读回应直接是 `APPROVED`，不是的话停下来问用户（那时才需要给审核员附 Mac 包，见 `docs/release/ios-external-beta/README.md`）；拿到公共链接，新增 `docs/public/ios/testflight.json`（P3 的脚本已能读它）。出包与送审另行取得用户指示（OA13）。

验证：模拟器编译与 `swift test --filter AHStoreTests`；helper 的模拟 HTTP 测试覆盖「固定应用 / 组边界、请求字段、幂等、失败」；真实后台只做本次目标的必要变更并读回。

实施记录（2026-11-22）：第 3 项在 `d90c25dc` 里给 helper 加了 `public` / `public-off`（模拟 Apple 的检查 25 条通过；`helper.py install` 装到本机后 `status`、`--check-signing`、`notes --mode read` 正常）。
第 4 项出了 0.1.0 (29)（源码 `d90c25dc`，Internal 与 Friends，`submit` 读回直接 `APPROVED`，交付记录见 [ios-channel.md](ios-channel.md#2026-11-22testflight-内部与外部构建-29)）；
公共组经 `public` 的 `dryRun`（只读现状，只有创建这一类差异）后写入，读回为：外部组「AskHuman Public」、测试员上限 1,000、构建 29、链接已开；匿名打开链接，页面是「Join the AskHuman Mobile beta」；
Friends 与 Internal 读回没变。`docs/public/ios/testflight.json` 已加（当时只是本地提交；之后已推送、Pages 已发布，见 P5 的状态）。

## P5：放出（翻可见性 + 发版）

顺序固定（规格 §10.4），**具体放出时间由用户另行决定**（规格 D20）；每一步都先确认上一步已真实生效：

1. 发布 Pages（手动工作流 `ios-beta-pages`），打开落地页与 TestFlight 公共链接，**用一台没加过测试的手机从落地页走到 TestFlight 安装成功**；
2. 带新文案的 iOS 构建（0.1.0 的新 Build）已在公共组，审核状态读回 `APPROVED`（沿用先前的审核，规格 D23）；
3. **一个提交**翻可见性（规格 §10.1）：
   - 卡片移到 `ChannelsTab.vue` 的最上面（规格 D13）：模板里紧跟「本地弹窗」卡、排在「按需发送」提示之前，按 DOM 顺序，不用 CSS `order`；`id="channel-ios"`、`useSearch.ts` 与深链；`ExperimentalTab.vue` 不再渲染 iOS 卡（页里还有 iCloud 待办同步卡，所以空状态那行用不上，没有加；`settings.experimental.ios.*` 已在 P1 迁完）；
   - `AwayCard.vue`、`RemotePage.vue`（`showIos` 与 `openSettings("channel#channel-ios")`；磁贴名「iOS（实验）」改成「iOS App」，即 `setup.remote.ios`）；
   - `ios_cmd::available()` 与 `channel_cmd.rs` 里四处门控去掉，`channel list` / `help` 与测试更新，`ios_cmd.rs` 帮助里的「(experimental)」去掉；
   - 文档：`ios-channel.md`（D22、L97、L282、L284）、`overview-ios.md`、`docs/overview.md` 与 `overview-configuration.md` 里提到「实验」的地方；顺带 `ios-open-access.md` OA12、`onboarding.md` D22、`mac-mesh.md`、`todo-sync.md`、`ios-alert-timing.md` 里写着旧路径的几处；
   - 提交信息 `feat(ios): …`，**不带** `Release-Note: skip`，并写 `Release-Note:` 一行：「iOS App 渠道不再是实验功能：设置里有分步引导（Broker、TestFlight、配对）、一键测试，也可以让 Agent 帮你配置。」（先给用户看措辞）；
4. 装机验证后发 Mac 新版（发布流程另行取得用户指示）；
5. 把「每 80 天出一次 TestFlight 包」「公共链接与上限的现状」「一周后复核中继与链接的用量」记进 `docs/PROGRESS.md`。

**状态（2026-11-22）**：第 3 项已做成一个提交（`e9c565b6`），`Release-Note` 措辞经用户确认。随后用户同意推送并发布 Pages：23 个本地提交已推送，工作流成功，线上五个页面与本地构建逐字节相同，落地页上的公共链接可打开——第 1 项的发布与读回完成；其中「用一台没加过测试的手机从落地页走到 TestFlight 安装成功」没做：用户 2026-11-22 决定不做（现在没有合适的手机，也不想再重测配对流程）。第 2 项早已满足（公共组里是 Build 29，带新路径文案）。第 5 项的记录已写进 `docs/PROGRESS.md`。**第 4 项（Mac 新版）已做**：v0.18.0 于同日发布，卡片对用户可见，见下面的「发布记录」。

验证：`node scripts/machine-health.mjs` → `./scripts/install.sh`；真实窗口里 设置 → 通信渠道 看卡片三种形态、深链、搜索；`AskHuman channel list` 与 `channel ios --agent-guide` 对没打开隐蔽开关的环境可用；**用户在真机上从零走完整条链路**（新 Broker 账号 → 落地页 → TestFlight → 配对 → 测试问题）并让一个 Agent 按提示词配一遍（密码由 Agent 生成并保存，用户只做 D24 里只有人能做的事）。

### 发布记录（2026-11-22：Mac v0.18.0）

用户选了「开始 Mac 新版的发布流程」，版本号与发布说明由用户定（规格 §12 第 11 轮）：**v0.18.0**，发布说明选手写，英文，
随版本提交为 `docs/release-notes/v0.18.0.md`，用户看过全文（特别是「没填中继地址就走公共中继」那一句，它是行为变化）；Release 页的正文与该文件逐字一致。
发布顺序（规格 §10.4）的前两步早已完成，这是第三步。

- **先修 CI**：`cargo audit` 的红灯（`url` 2.5.8，RUSTSEC-2026-0592）升到 2.5.9，`99ee7957`，只改 `Cargo.lock`（连带 `form_urlencoded` 1.2.3）。
  本地 `cargo audit` 对旧锁文件退出码 1、新锁文件 0；`clippy -D warnings` 干净；`cargo test --bin AskHuman` 3649 通过、0 失败、12 忽略。
- **流程**：主工作树（干净）里 `bump-version` 到 0.18.0；`cargo update --workspace --locked --offline` 确认 `Cargo.lock` 与清单一致；
  十二处版本（含 npm 主包对四个子包的依赖）核对一致；提交 `30e56ec7 chore(release): prepare v0.18.0`（连同发布说明），与 `99ee7957` 一起推送。
  该提交的 [CI 45013844912](https://github.com/Naituw/AskHuman/actions/runs/45013844912) 8 项全绿（约 22 分钟，含 `cargo audit`）。
  用户在 CI 全绿后点头；注解 tag `v0.18.0` 指向 `30e56ec7`，只推了这一个 tag：没有用 `--tags`（本地的 `ios-testflight-*` 标签没有出去），
  也没有用 `scripts/publish.sh`（它是裸 `git push`）。推送前确认过远端没有 `v0.18*` 标签、npm 上最新是 0.17.3、GitHub 上签名与 npm 所需的 Secrets 都在（只看名字）。
- **发布结果**：[Release 45015764037](https://github.com/Naituw/AskHuman/actions/runs/45015764037) 全部成功，从打 tag 到结束约 12 分钟（没有排队），
  2026-11-22 19:19（北京时间）发布；Release 为非草稿、非预发布，四个二进制归档和 `SHA256SUMS` 都在。辅助组件的公证与四个平台的构建、两个 macOS 的 Developer ID 签名一次通过。
- **核验**：四个归档都通过 `SHA256SUMS`。Mac ARM64：严格签名验证通过（Developer ID Application，TeamID `DMJXDB9H6Q`，hardened runtime，带时间戳），
  `--version` 为 v0.18.0，`todo-sync resource-info --require-bundled` 给出内嵌组件版本 0.18.0、协议 1，`todo-sync resource-verify` 通过（都在隔离的 `ASKHUMAN_HOME` 里运行，没有留下任何状态）。
  Mac Intel：签名验证通过、TeamID 相同，没有运行（本机没有 Rosetta）。Linux 与 Windows：只用 `file` 认出是 ELF / PE，没有运行。
  ARM64 二进制 24.2 MB，v0.17.3 是 22.4 MB（多约 1.7 MB；这一版新增了 iPhone 图片预览的编码器，没有单独量它占多少）。
  本机日常版用 `./scripts/install.sh` 对齐到 v0.18.0。
- **npm**：发布步骤的日志里五个包都打印了 `+ …@0.18.0`（四个平台包在前，主包最后），`latest` 指向 0.18.0，主包把四个平台包固定在 0.18.0。
  隔离目录、全新缓存里 `npm install askhuman@0.18.0` 装上了主包与 `darwin-arm64`，能运行 `AskHuman --version`（v0.18.0），其 native binary 的 SHA-256 与 GitHub 下载包一致。
  注册表有延迟：发布后约 3 分钟平台包才能下载；主包的 tarball 在我过早请求（发布后约 2.5 分钟）之后被 CDN 缓存了一份 5 分钟的 404（`cache-control: public, max-age=300`，加个查询串就能取到 200，说明源站早有文件），
  约 8 分钟后我才装成功。刚发布的几分钟里，用户可能遇到同样的 404。
- **与 v0.17.3 的对照**：那次首发死在两个 macOS 构建的签名一步；这次读了当时的日志，原因是 `codesign --timestamp` 联不上 Apple 的时间戳服务
  （`A timestamp was expected but was not found.`，同日 Apple 开发者服务宕机），不是证书或密钥的问题。这次同一步一次通过，`PROGRESS.md` 里的说法已改。
- **没验证**：Windows、Linux、Intel Mac 的发布二进制没有在本机运行（CI 只覆盖编译与测试）；没有实际走一遍应用内自更新到 0.18.0；
  没有用一台没加过测试的手机从落地页走到安装成功，用户决定不做（见上面的状态）；整条链路里各段分别验过
  （测试问题在真实手机上跑通，Agent 代办走通，Pages 与公共链接只读回过页面），没人验过的是「全新 Apple ID 从落地页经公共链接装上 TestFlight 构建」。
- **对外动作**（2026-11-22，措辞逐条给用户看过、原样发布，线上正文读回一致）：在已合并的 PR
  [#29](https://github.com/Naituw/AskHuman/pull/29#issuecomment-7078581812)、[#31](https://github.com/Naituw/AskHuman/pull/31#issuecomment-7078582371) 与 issue
  [#30](https://github.com/Naituw/AskHuman/issues/30#issuecomment-7078582134)、[#32](https://github.com/Naituw/AskHuman/issues/32#issuecomment-7078582598) 各回复一条，告知修复 / 诊断已随 v0.18.0 发布。
  #29 提醒升级后要在「设置 → Agents」对 Claude Code 点一次「更新」；#31 如实写明我们的 Cmd+Tab 对照看不出补丁的差异、Dock 点击仍靠作者在合并窗口的手动验证，并请他再验独立窗口；
  #30 请反馈者用原复现步骤确认；#32 请报告者升级后贴新日志里的那一行，并补上 launchd 下的 curl 结果。后续在 `PROGRESS.md` 的 #30 与 #32 两处。
- **发布说明修订**：写 #29 的回复时核对发现，说明里 Windows 一条漏了必要的步骤：这次修复只改了 Windows 的 hook 脚本，脚本只在「更新」或 `agents update claude` 时重写
  （`claude_hook::install()` 只由切换模式和显式更新调用，升级二进制不会换），只升级的 Windows 用户拿不到修复。用户同意补这一句：`docs/release-notes/v0.18.0.md` 与 Release 页正文
  （`gh release edit`，名称、标签、发布时间不变）同步，读回逐字一致（2983 字符）；tag 内容不变。
  同时发现 Dock 一条在说明里写成了既成事实，而 [`popup-request-inbox.md`](../specs/popup-request-inbox.md) §4.2 仍标「待验收」（Dock 点击没测过，Cmd+Tab 对照里补丁与基线没有差异）；
  用户只选了补 Windows 一句，这一条的措辞没改，验证状态只在 #31 的回复里说明。

## 风险与回退

- 步骤 2 依赖 Apple 与 Pages：任何一步没准备好，P5 不进行；P1–P4 的内容在隐蔽开关后面，不影响现有用户。
- 回退：P5 的翻转是一个提交，`git revert` 即回到「实验」；Pages 与公共链接可以单独关闭。
- 测试问题的过渡期风险与 Agent 越界风险见规格 §11。

## 不在本计划

规格 §13 全部；以及：自动化 Broker 开户、非 macOS 的推送时机适配、iOS 内置引导页重做。
