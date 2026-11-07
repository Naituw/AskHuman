# 计划：iCloud 待办同步的正式版分发

> 需求与用户决定见 [规格](../specs/todo-sync-release.md)。在当前工作树实施。

## 实施顺序

1. 补齐本地组件管理：macOS 14 门控、随包资源定位、签名与环境校验、
   安装 / 注册 / 替换 / 回滚、防降级、首次启用与修复入口、已开启用户的升级。
2. 补齐资源生产与分发：helper 双架构 Release 构建、签名与公证脚本、
   npm 平台子包与 GitHub 资产、自更新与本地源码安装的资源传播。
3. 补齐用户状态与文案：首次安装、批准登录项、组件缺失与修复、系统要求；
   保留实验入口和默认关闭。新增中英文用户文档。
4. 自动验证核心安装事务、包内容与更新路径；跑相关 Rust / Swift / Vue / Node 回归。
   每次长构建前运行 machine-health；安装新程序并用装出的 AskHuman 验证。
5. 用可审阅的候选资源安排签名 / 公证和真实首次安装；朋友账号、稳定系统、
   实际运行的验收如实记录；用户后续明确排除断网 / 睡眠测试与 24 小时等待门槛。
   最后再决定发布版本与公开发布。

## 发布外部条件

- CI 需要现有 Developer ID 证书、对应 Production provisioning profile 与公证凭据。
  不把私钥或凭据写入仓库；配置缺失时发布应失败，不能发出缺组件或未公证的包。
- 新增 Apple 账号资源、导出或配置凭据、上传公证与正式发版在候选结果可审阅后单独确认。
- 不复用旧的 Mac B SSH / 网络变更授权；需要真实双机验收时再安排。
- 独立账号首次使用由朋友验收；用户 2026-11-07 确认保留未验边界，授权作为默认关闭的实验性功能随 0.17.0 公开发布。

## 实施记录

- 2026-11-06：范围已通过 AskHuman 确认，按用户要求在主工作树实施。
- 工程已接入 macOS 14 门控、启用前安装、签名 / Production 权限校验、注册与批准提示、
  后台升级、版本防降级、失败回滚和可重试错误状态。关闭同步时不安装资源。
- 资源采用用户追加确认的内嵌方案：`build.rs` 用 `ASKHUMAN_TODO_SYNC_ARCHIVE` 嵌入，
  `resource-info --require-bundled` 检查；npm 与旧 Direct 更新器无须额外的资源下载路径。
- Release 流水线先构建、公证并 staple universal helper，再构建两种 Mac 主程序。
  凭据 / 资源缺失时阻止发布。用户已手动配置 GitHub encrypted secrets：
  `MACOS_TODO_SYNC_PROFILE_BASE64`、`MACOS_NOTARY_APPLE_ID`、`MACOS_NOTARY_PASSWORD`；
  已只读核对名称与更新时间，没有读取 secret 值。可选 Team API key 认证仍保留。
- 本地 helper 候选：`todo-sync-helper/build/release-candidate/AskHumanTodoSync.app`，
  同时含 arm64 / x86_64，版本暂为当前源码的 0.16.0（正式发版版本尚未定），
  使用已有 Developer ID 与原自用 helper 的有效 Production profile；签名、权限、
  bundle ID、容器、版本、RPC 协议与两种架构均已读回验证。候选随后已通过公证（见下）。
  `release.py` 先无签名编译，再嵌入已有 profile 并明确签名，避免 Xcode 拒绝在
  manual-signing 模式使用已有 Xcode-managed profile；不创建新的 Apple 账号资源。
- 自动回归：同步 Rust 测试 138 通过、2 个性能测试忽略；待办相关 Vue 测试 125 通过；
  发布脚本 Python 校验 7 通过（由 Node 测试入口纳入 `pnpm test`）。
  全量 TypeScript 检查已在共享树的新任务表单类型修复后通过。
- 本机 Gatekeeper 可以接受仅有 Developer ID 的未公证候选（`source=Developer ID`），
  因此安装验证额外要求 codesign 的 `notarized` 条件，而不把 spctl 成功当作公证证据。
  同一候选满足 Developer ID / Production 校验，但 `notarized` 条件明确失败，符合预期。
- 中英文用户文档已写入 `docs/wiki/todo-sync(.en).md`；原需求的 5 秒门槛按本轮确认撤下，
  同时澄清 20 秒只是刷新阈值，1.5 秒超时可能继续使用本机数据。
- 首轮 `./scripts/install.sh` 已成功编译、Developer ID 签名并装到日常入口；安装检查无告警。
  装出的程序支持 `resource-info` 和新增组件状态。当时资源尚未公证，首轮源码安装未内嵌它，
  继续兼容正在运行的 0.1.0 自用 Production helper；状态为同步开启、0 未上传、无组件安装错误。
- 离开模式后继续完成的独立验证：全量 Rust 2614 通过、11 忽略；Clippy
  `--all-targets -- -D warnings` 通过（已有 vendor/tao 告警仍由依赖自身报告）；
  全量 Vue 104 文件 / 1296 测试通过，Node 140 通过 / 19 平台跳过；Swift 43 通过。
  独立构建期资源回归 5 项通过，覆盖缺资源阻止发布、非 Mac 不嵌入、源码构建省略、
  坏路径失败和真实 rustc 编译后的字节完整性。
- 已用真正的 ARM / Intel 主程序验证两种 npm 平台包的 pack / 解包及旧版仅复制主程序的
  Direct 更新路径；两边资源 SHA-256 均与输入一致，未公证资源均明确拒绝，测试没有安装或
  执行 helper、没有写入用户同步状态。Intel 构建使用已有 rustup stable 1.97.1 + x86_64 target，
  运行通过本机 Rosetta；默认 Homebrew Rust 1.94.1 没有 x86_64 标准库，不能拿它交叉编译。
- 增加 `resource-verify`：对实际内嵌内容做完整信任与版本验证，不触碰 iCloud / 登录项；
  发布流水线会执行它和真实 npm / Direct 分发 smoke。workflow YAML 已解析通过。
- 公证命令先持久化 submission ID，再有界等待 10 分钟；中断后重跑同一签名候选会复用
  原提交，离线回归覆盖该流程。CI 保存 recovery artifact（receipt 与原输入 zip），
  维护者操作说明见 `todo-sync-helper/RELEASING.md`。
- 压力窗口测试已完成（200 项目 / 4000 条目）：热前端 p90 227 ms、端到端 p90 498 ms；
  冷前端 p90 969.6 ms、端到端 p90 4435.6 ms。机器当时负载 369 → 116.8，不能据此建立
  正式基线或关闭原 A/B 门槛；测试没有写 baseline。已定位原 P0 参考提交 `9f5bab2b`
  （旧下拉框 + 性能埋点），只读归档在 `todo-sync-helper/build/perf-reference-source`，
  没有创建 worktree。构建前 machine-health 持续退出 2；已告知用户并请其释放内存，
  用户明确选择保留当前环境、单任务后台继续，后续长任务按此执行并短间隔检查。
- 待完成：独立账号与稳定系统上的真实首次安装 / 批准，
  以及安静机器上的原 P0 对比与正式性能基线。断网和睡眠测试按用户后续明确要求不执行。
  工程测试通过不代表正式发布准备完成；未创建发布标签或公开发布。

### 本机公证已通过

用户在 AskHuman 选择「没有现成配置，带我配置」，随后自行在可信终端完成
`AskHumanTodoSync-Notary` 的钥匙串配置，并明确选择「已配置完成，继续上传候选公证」。
凭据验证通过后，候选已上传 Apple；submission `83904ad5-dbe7-405b-8db6-894857140dcb`
返回 **Accepted**，staple、ticket、代码身份 / Production 权限、双架构校验均通过。
receipt 和已公证 zip 在同一 `build/release-candidate` 目录。未读取或记录应用专用密码。
最终 zip 的 SHA-256 为 `c52d402611b0046de0d3437ec446977a36dc1c540231ae8a4190e5166e7e1d0b`。
另在临时副本加入 quarantine 标记，Developer ID / 明确的 notarized requirement、stapled ticket、
Production 权限、双架构及 Gatekeeper 评估均通过；没有执行组件，不能代替真实下载后的首次启动。

为沿用用户选择的认证方式，CI 增加 Apple ID + app-specific password 的临时钥匙串路径，
保留已有 Team API key 路径；手动 workflow 默认 `verify_only=true`，可以仅构建 / 验证而不发布。
用户已通过 GitHub CLI 完成三个 encrypted secrets（Apple ID、应用专用密码、
本组件公开分发的 Production profile）的配置，并在 AskHuman 确认；引导脚本不读取密码，
密码由 GitHub CLI 交互输入。已只读核对 secret 名称。用户随后批准本次改动单独提交并推送 main，
运行 `verify_only=true` 的 release CI，并明确授权该 CI 候选的 Apple 公证上传。

公证后重查机器仍为 machine-health 退出 2，用户确认「保持当前环境，单任务后台继续」。
2026-11-07 首次重查为 12.3 GiB swap / 16 GiB RAM / 一台模拟器、负载 238；
以 `CARGO_BUILD_JOBS=1` 后台构建的 ARM 主程序已通过真实 npm pack / 解包、旧 Direct
仅复制主程序路径与资源信任检查，哈希与已公证 zip 一致。随后 `./scripts/install.sh` 成功，
日常入口携带同一资源，`resource-verify` 通过。

现有 daemon 正在等待其他会话的在途请求排空，没有强制重启；通过新装程序的 `todo-sync repair`
验证了已有自用组件升级：0.1.0 → 0.16.0，登录项 enabled、Production、0 未上传，
无暂停 / 组件安装错误，原 0.1.0 app 保留为 `AskHumanTodoSync.previous.app`。
这次验证覆盖显式修复 / 升级。daemon 随后已自然接班，指纹与日常入口一致，
维护后仍无组件安装错误；`doctor` 安装检查无告警。本机未触发需要系统批准的登录项形态，
没有把 enabled 的结果当作批准路径已验收。
Intel 带同一已公证资源的主程序也已构建成功；通过 Rosetta 实际执行的 npm pack / 解包、
旧 Direct 复制和资源信任检查全部通过，与 ARM 使用相同 zip / SHA-256，未安装组件或写入用户状态。

### 仅验证 CI 已通过

`1808f269` 已按用户授权单独提交并推送 main，未包含并行 iOS 界面改动。用户确认这轮保留
「每次 release 都构建 / 公证一份 universal helper」的策略；普通 push / PR 的测试 CI 不上传公证。

[release run 42251370975](https://github.com/Naituw/AskHuman/actions/runs/42251370975)
以 `workflow_dispatch`、`verify_only=true` 执行并成功：

- 开始至结束 11 分 12 秒（2026-11-07 00:19:57–00:31:09，UTC+8）。
- helper job 1 分 59 秒，其中构建、签名、公证、staple 与校验步骤合计 1 分 41 秒。
- Linux 4 分 01 秒、Intel Mac 4 分 05 秒、ARM Mac 7 分 01 秒、Windows 9 分 03 秒。
- `publish npm + GitHub Release` 为 skipped；没有创建 release、发布 npm 或打新标签。
- CI 生成的资源 SHA-256 为 `c60f409277507043a5fdf25ca5153b085cbd81164e2729a666976b4ede3bf18a`，
  与本机候选不同（独立构建与签名）。已下载 CI 资源和 Mac 主程序用于额外本机核验；
  ARM、Intel 的签名身份与实际 npm / Direct 资源信任检查均通过。

同一源码提交的[普通测试 CI](https://github.com/Naituw/AskHuman/actions/runs/42251327937)也已全绿：
四平台、iOS Swift package、cargo audit、relay 共 7 jobs 成功。第一次运行的 Mac jobs
未分配到 runner，GitHub annotations 明确报告 arm64 容量不足；只重试失败 / 取消项后通过。
Linux 首轮系统依赖安装耗时较长，但最终成功；不把 runner 排队取消记为代码测试失败。

参考提交 `9f5bab2b` 的前端与 P0 原始 Rust 源码已构建成功（Rust 2 分 08 秒）；
借用本机依赖编译缓存的 APFS clone，不安装参考程序，也不覆盖日常入口。
已用日常新装程序与该参考启动隔离交错 A/B，覆盖 real / stress 两个数据层级。

### 本轮不执行的真机测试

2026-11-07 用户在 CI 报告后明确要求「不要做断网相关的测试，还有睡眠的也不要做，
其他的你继续做完」。本轮不执行网络切换、断网编辑 / 恢复、睡眠 / 唤醒操作；
这些场景保留为未验证边界，不用单测、联网状态或静态检查冒充通过。
随后用户补充「不需要等一天，这本来就是实验性功能」：取消新组件满 24 小时的等待门槛，
不为该项建立回查或延期待办。组件继续日常运行，已有自然运行观察如实保留。

### 其他本机验证与性能限制

- 从 CI 下载的 helper 解包到临时目录后，手动加 quarantine 标记；身份 / 公证条件、
  stapled ticket、Gatekeeper、Production 权限均通过，ARM 与 Rosetta x86_64 的 `--version`
  均返回 0.16.0。该命令在进入 NSApplication / CloudKit 前退出，没有注册或创建 helper 状态；
  这是带 quarantine 的受控执行检查，不冒充朋友账号上的真实首次安装。
- 新装程序实际执行停用、核对、再启用：停用时 `todos.json` 和同步 `state.json` 均保留且
  SHA-256 未变；恢复启用后 Production / 0.16.0 / 0 未上传 / 无暂停或安装错误。
- 只读核对升级前后最近 24 小时自然运行日志：234 次 `tick_done` 全部 exit 0，无 `rpc_failed`
  或 timeout / failed 事件；新版当时仅运行约 38 分钟，空闲 CPU 0.0%、RSS 约 7 MiB。
  历史观察覆盖旧版与新版；稳定系统和独立账号仍待验。新候选必须满 24 小时的门槛已由用户取消。
- 原 P0 `9f5bab2b` 对日常候选的完整交错 A/B 已完成，负载 210，按脚本的繁忙环境规则用
  median 比较：real / hot 端到端 +23.6%；stress / cold 端到端 +14.6%、前端 +24.2%；
  stress / hot 端到端 +14.7%、前端 +33.5%，脚本返回 1。压力热前端 p90 192.4 ms，
  满足 300 ms 绝对预算；相对 P0 的非回归门槛未通过，没有写 baseline。
  P0 与本轮前的主线之间还包含其他桌面功能变化，因此继续归档本轮前的 `980349aa`
  作定位对比，先区分本轮增量与此前变化，不凭繁忙机器数据直接修改界面。
- 定位对比 `980349aa`（本轮前的主线）已完成并通过脚本门槛：stress / cold 端到端
  median 553 vs 549 ms、前端 166 vs 174 ms；stress / hot 端到端 662.5 vs 679 ms、
  前端 241 vs 236.5 ms。两边都有 8 cold / 24 hot 有效样本，负载 22.9 → 203.9；
  热前端 p90 301.8 vs 310 ms。该繁忙环境下的对比未检出本轮明显增量回归，
  不等于原 P0 门槛或 300 ms 绝对预算已在安静环境通过；没有生成正式 baseline。
- 构建大小辅助定位：本轮前后 `TodosView` 均为 65,534 bytes，入口从 580,868 增至
  581,231 bytes；P0 的对应大小为 37,758 / 524,968 bytes。窗口自身前端源码没有在本轮改变。
  该证据只区分版本变化范围，不用包大小代替性能计时或确定因果。
- 早晨等待负载降至 4.49 后又完整重测原 P0；过程中负载再升到 117.12，脚本仍按繁忙环境
  比较 median 并返回 1：real / hot 端到端 +18.0%，stress / hot 端到端 +21.1%、前端 +26.4%。
  压力热前端 p90 为 283.7 ms，但该轮也不能作为安静环境达标证据，没有生成正式 baseline。
  原规格 §9.9 的目标值注明待 P0 基线校准，继续保留原性能记录；本轮分发改动的增量比较
  与该遗留项分开记录，不把实验性组件交付误写成全部原性能目标已通过。

### 0.17.0 发布授权与安全检查

2026-11-07 用户要求先检查凭据再发布，审阅英文发布说明并批准 0.17.0。Gitleaks 8.30.1 使用默认完整规则、关闭仓库忽略项，扫描全部受版本管理文件与 main 可达的 1099 个提交。16 处告警已逐项核实：7 个固定加密测试向量、8 个假的 CloudKit 游标、1 个 Swift 类型声明；未发现真实密钥。仓库未跟踪私钥、证书导出、环境凭据文件。实际安装包另行检查；不读取或导出 Keychain 凭据。独立账号、稳定系统与首次批准形态保持待验，不宣称已通过。

### 2026-11-07：0.17.0 已正式发布

- 用户审阅并批准英文 Release notes 后，最终版本由 0.16.1 改为 0.17.0。固定发布源码为
  `4da9a915195f7a3a5aedcb186dffd5c79ce6ea68`；`v0.17.0` 标签只指向该提交，不包含发布后并行的工作区改动。
- [候选验证 42334807543](https://github.com/Naituw/AskHuman/actions/runs/42334807543) 四平台成功、发布 skipped；
  [常规 CI 42334803246](https://github.com/Naituw/AskHuman/actions/runs/42334803246) 七项全部成功。
- [正式发布 42336271991](https://github.com/Naituw/AskHuman/actions/runs/42336271991) 全部成功；
  [GitHub v0.17.0](https://github.com/Naituw/AskHuman/releases/tag/v0.17.0) 已为公开稳定版本，包含四平台原版资产和 SHA256SUMS。
  npm 主包 `askhuman` 与四个 `@humaninloop/*` 平台包的 0.17.0 / latest 均已在官方源读回。
  Windows 元数据曾短暂不可见，刷新后恢复；发布日志确认五包均成功，未重发或改版本。
- 正式组件 ZIP SHA-256 为 `7bb3cbaf44ff698d9aba15bad7195592ccfd500b0481a0434d47b45a0492e29a`，
  两架构、版本 0.17.0、Production、固定 Team / bundle / 容器、显式 notarized 要求、staple 与 Gatekeeper 均通过。
- 四平台正式直接包 SHA-256 与清单相符；Mac ARM / Intel 实物的 npm pack / 旧式直接复制 smoke 通过，
  仅在临时目录验证资源，不注册 helper、不写用户状态。隔离前缀从官方 npm 安装后，CLI 版本、
  wrapper 与内嵌资源信任均通过，没有创建应用状态。
- 长构建前 machine-health 为 tight（非退出 2），按已授权的单任务后台方式完成 install.sh；
  日常二进制为 0.17.0，helper 自动升级至 0.17.0，Production / 0 未上传 / 无安装错误，随后正常 AskHuman 提问成功。
- 发布前后均做密钥检查：Gitleaks 默认规则、不采用仓库忽略，16 个源码 / 历史告警均为已核实的测试数据或类型误报；
  实际候选与正式四平台二进制的 ASCII / UTF-16 字符串扫描无告警，私钥与个人渠道配置未进入安装包。
- [中文接入页面](https://naituw.github.io/AskHuman/beta-zh/) 已通过
  [Pages 42337415283](https://github.com/Naituw/AskHuman/actions/runs/42337415283) 部署并 HTTP 200 读回，包含 0.17.0 安装入口、
  EMQX 官网 / 国内控制台及逐步配置、可选自部署。另备中文离线 HTML / Markdown 与原版安装文件合集；
  中继入口仅在私有交付资料中提供，没有复制维护者的 broker 密码或任何签名凭据。
- 独立账号、稳定系统与首次批准形态继续待验；不宣称补齐，不等待 24 小时，不执行断网或睡眠测试。

正式发布后的五个 npm tarball 另已完整下载：文件清单与版本正确、官方 dist.integrity 相符；四个平台的二进制均与对应 GitHub 原版资产逐字节一致，GitHub 发布正文也与用户批准的英文文件完全一致。
