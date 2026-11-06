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
   双机断网 / 唤醒与 24 小时运行验收如实记录。最后再决定发布版本与公开发布。

## 发布外部条件

- CI 需要现有 Developer ID 证书、对应 Production provisioning profile 与公证凭据。
  不把私钥或凭据写入仓库；配置缺失时发布应失败，不能发出缺组件或未公证的包。
- 新增 Apple 账号资源、导出或配置凭据、上传公证与正式发版在候选结果可审阅后单独确认。
- 不复用旧的 Mac B SSH / 网络变更授权；需要真实双机验收时再安排。
- 独立账号首次使用由朋友验收；没有真实记录前不标为发布准备完成。

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
- `./scripts/install.sh` 已成功编译、Developer ID 签名并装到日常入口；安装检查无告警。
  装出的程序支持 `resource-info` 和新增组件状态。由于资源尚未公证，本次源码安装未内嵌它，
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
- 待完成：携带 quarantine 的安装、内嵌候选安装 / 升级实测、CI 实际验证、
  独立账号 / 稳定系统、断网 / 唤醒 / 24 小时运行和安静机器上的性能基线。
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
