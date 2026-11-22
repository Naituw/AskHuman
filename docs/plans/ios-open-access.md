# iOS 渠道免费开放：开发与交付计划

日期：2026-11-21。状态：P1–P4 已完成（含 3c 验证后填入的内置默认中继地址）：P3 的预发验证（3a）、注册限流（3b）、域名与证书、告警与预算，以及 2026-11-22 生产函数的切换（3c）都做完并外部验证过；P5 收尾只剩随下一次出包的文档与几项清理，见 `docs/PROGRESS.md`。需求与决策见 [`docs/specs/ios-open-access.md`](../specs/ios-open-access.md)。

在主工作树做；同一棵树里常有别的会话的未提交改动，提交时只暂存自己的文件或自己的改动块（`git apply --cached` 一个只含自己改动块的补丁）。

顺序：P1 文档与工具 → P2 去绑定 → P3 公共中继 → P4 向导与默认中继 → P5 收尾。P4 里「默认中继地址」要等 P3 的域名生效后才合入。

## P1：兼容套件与 Broker 文档（已完成）

- 兼容套件 `ios/Tools/broker-probe/`（`compat.cjs`、`lib/`、离线自测 `test/`，27 项；CI 任务 `broker-probe`）；`acl-check.cjs` 改成薄包装；Mosquitto 参考配置在 `examples/mosquitto/`。提交 `1b372ef5`。
- 文档：`docs/wiki/ios-broker.md`（+ `.en.md`）、`ios-setup`（中英）的 broker 一节、`docs/public/ios/beta*.md` 的第 2 步、`docs/overview-ios.md` 的验证表。提交 `dff0fe7d`。
- 对三个真实 broker 跑过套件（2026-11-21）：EMQX Cloud Serverless（杭州）、flespi Free（欧洲）、Mosquitto 2.1.2（本机，自签 CA），结果和各服务的免费额度、缺点、大陆访问的单次抽样都在 `ios-broker.md`。

遗留（小）：

- flespi 的账号隔离与跨用户互验只在 TLS 上跑过；真实 App 在 EMQX 的 WSS 上断开时是否多发一条遗嘱没有实测。
- 仓库自带的端到端套件（真实 Mac 守护进程 + 手机引擎）对 flespi 与 Mosquitto 还没各跑一遍（P5 做）。

## P2：去 EMQX 绑定（已完成）

- Mac 设置卡与手机 Macs 页去掉三处与 EMQX 绑定的东西（写死 1 GiB 的百分比、EMQX 控制台链接、点名 EMQX 的文案），保留 GB 步进提醒；不加配置字段，不改协议。提交 `b02e2af2`（Mac）、`6142a6cc`（手机，含字符串目录）。
- 规格与概览：`ios-channel.md` 的 D3、D23 标「2026-11-21 修订」，`ios-traffic.md` 新增「与具体服务商解绑」，`overview*.md` 改成「自备 MQTT broker」。提交 `e51df398`。
- 验证：`npx vitest run src/views/settings src/i18n`、`vue-tsc --noEmit`、`cargo test ios::`（482 项）、`swift test --filter AHStoreTests`、`xcodebuild build`（模拟器）、`./scripts/install.sh`。
- 刻意没改：`docs/release/ios-external-beta/*.txt` 与 `ios-testflight` 技能里的 EMQX 字样（Apple 审核材料，随下一次出包一起改）；历史计划与调查记录。

## P3：公共中继（线上，需要用户的腾讯云控制台）

**3a 预发验证（已完成，结果见规格 §7）**：同一个 zip 部署到一个**临时函数**，不动生产；验证完删除。逐项验证，任何一项不成立就停下来问用户：

1. Web 函数是否支持「最大独占配额」；超限时的返回码；超限请求是否不计费。
2. 自定义域名能否映射到「别名」上的函数 URL；HTTPS 证书与 HTTP→HTTPS。
3. 请求里客户端真实 IP 在哪个头里（决定注册限流怎么按 IP 分桶）。
4. 默认域名「仅供测试」的限制，是否也适用于自定义域名。
5. 单实例处理「拒绝类」请求的实际吞吐（把规格里「每实例 100 次/秒」的假设换成实测）。
6. 函数 URL / 自定义域名有没有可配置的限流（QPS）。

**3b 中继代码（已完成，提交 `545f5c5f`）**（`relay/src/core.js`、`adapters/scf.js`、`relay/test/*`）：注册限流原来是一个全局共享桶（键恒为 `'all'`），有人刷注册就会把所有新用户挤掉。改为**按客户端 IP 分桶 + 全局兜底**；推送限流（按句柄编号）不动。桶仍是单实例内存、各实例各算一份，这是公平性手段，不是成本手段，文档里明说。客户端 IP 取 `X-Forwarded-For` 从右数第三项（3a 实测），`TRUSTED_PROXY_HOPS` 可调。单元测试覆盖 IP 解析、分桶、兜底（relay 共 43 项）；API 不变，旧客户端照常工作。提交 `feat(relay): rate-limit registration per client ip`（`Release-Note: skip`）。

**3c 生产部署（已完成，2026-11-22）**（不需要任何凭据：Owner 登录控制台，新建的资源由我在浏览器里操作并从外部验证，Owner 只做登录与文件选择框；**动生产函数前要 Owner 明确批准**，批准只对下面八步有效）。

已做（2026-11-22，都挂在 Owner 的腾讯云账号上，没有碰任何已有的函数、域名、记录与证书）：

- 域名 `askhuman-relay.maipo.me`（中继留在香港；Owner 已有的 `api.maipo.me` 是按地域分流的，不动）：DNSPod 的 CNAME 指向 SCF 的自定义域名入口，免费 TrustAsia 证书托管并打开自动续期，强制 HTTPS；映射 `/*` 在 3c 之前指向预发函数 `askhuman-relay-verify` 的别名 `live`，3c 第 4 步改到了生产函数的 `live`。
- 告警两条与月预算 ¥30（规格 4.3）。
- 上传包的本机预检（2026-11-22，上传前）：`relay/dist/askhuman-relay.zip`（8,579 字节）与当前 `relay/src` 逐文件一致；用 zip 解出来的 `scf_bootstrap` 在本机起服务（一次性密钥），验过 health、注册与推送的错误路径、同一客户端第 31 次注册 → 429、伪造的 `X-Forwarded-For` 前缀绕不过、日志只有方法 / 状态 / 耗时。这只证明包与代码本身；真实 SCF 上的 `X-Forwarded-For` 层数、端口与运行时仍靠下面第 5 步的外部验证。

切换生产（蓝绿、随时可回滚），Owner 批准后在他的 Mac 前做完；八步都做了，结果见下：

1. 给生产函数 `askhuman-relay` 现有代码发布版本 1（回滚用），把 `$DEFAULT` 别名固定到版本 1：旧函数 URL 继续跑旧代码，不受后面的操作影响。
2. 把 `relay/dist/askhuman-relay.zip`（`relay/scripts/build-zip.sh` 生成）上传到 `$LATEST`（Owner 在文件选择框里选一次），发布版本 2。
3. 建别名 `live` → 版本 2，并给它建自己的函数 URL（取消「参数兼容」）；给函数设最大独占配额 512 MB（4 × 128 MB，**不要用 128 MB / 1**）。
4. 把自定义域名的 `/*` 映射从预发函数改到 `default/askhuman-relay/live`。
5. 外部验证：`/v1/health`；`/v1/register` 错误路径；`/v1/push` 无效句柄 → 401；连续 31 次注册 → 第 31 次起 429；伪造的 `X-Forwarded-For` 前缀绕不过限流；几十并发、十几秒的小压测；超限时返回 432、恢复后立即可用。压测会触发两条告警，先知会 Owner。
6. 把 `$DEFAULT` 指向版本 2（旧 URL 也走新代码）。回滚 = 把别名 / `$DEFAULT` 指回版本 1、或把配额设回去，生效约几秒。
7. 删除预发函数 `askhuman-relay-verify`（它的调试端点回显请求头）。
8. 熄火 / 恢复在生产函数上各演练一次。

注意：发布版本会固化当时的环境变量（APNs 密钥等）；以后换密钥要重新发布版本并把 `live` 指过去。

**结果（2026-11-22）**：1–8 步都完成，具体数字在规格 §7「生产切换与验证结果」。几处与计划不同或值得记的：

- 第 5 步外部验证用的是自己写的几个一次性脚本（一律只打印状态码与响应头名，不回显任何账号相关的头）；压测量控制在两条告警的阈值之下，没有为了「触发告警」而压。
- 量 `X-Forwarded-For` 层数时临时加过一条 `/zz-probe/*` 映射（测完已删）：自定义域名的路径映射**按列表顺序匹配、先匹配者生效**，`/*` 排在前面会盖住更具体的路径；要调顺序只能删掉重加。
- 第 6 步之后核对：控制台里 `$DEFAULT`（旧函数 URL 绑定的别名）指向版本 2，旧函数 URL 仍然可用（health 200）。黑盒测不出旧 URL 跑的是哪一版：两版的注册限流参数相同（旧版是全局一个桶，新版按 IP），从一个出口 IP 打，行为一样。
- 第 7 步删除预发函数时，控制台先弹出一个与删除无关的「部分功能下线」通知（2027-01-01 起停止新建 Custom Runtime 等，不含 Node.js 20.19 的 Web 函数），点「我知道了」后才是真正的删除确认框（列出要连带删除的函数 URL、别名、版本；自定义域名 0 个，所以没有牵连到现在的映射）。
- 第 8 步演练的总中断约 20 秒，远低于「并发触顶」告警要求的连续 5 个 1 分钟周期。

**3d 自动熄火：不做**（OA14）。

验证：外部 `curl`（`/v1/health`、`/v1/register` 错误路径、`/v1/push` 无效句柄）；小规模并发压测（几十并发、十几秒，费用可忽略）观察超限返回；日志仍只含方法、状态、耗时。

## P4：向导与默认中继（Mac 设置页与 Rust）（已完成，默认地址除外）

- **4a 服务商目录与向导**：`src/views/settings/iosBrokers.ts`（纯数据）；`IosChannelCard.vue` 顶部加下拉与要点块；i18n 中英文；组件测试。不新增配置字段。
- **4b 默认中继**：Rust 新增 `effective_relay_url(cfg)`；`pairing.rs`（二维码里的 `r`）、`requests.rs`（推送）、`connstr.rs`、`daemon/runtime/ios.rs`、`cli/channel_cmd.rs`、`commands.rs`（健康检查）全部改用它；界面「使用公共中继 / 使用自己的中继」开关；设置里写明公共中继的数据与可用性说明。默认地址等 3c 验证通过后才合入（见下方「结果」）。
- **4c 隔离自检**：见规格 6.3。`ProbeReport` 新增 `isolation`；`ios_test` 的返回改为「消息 + 警告列表」；设置页与 CLI 都按结构显示；用 `local-broker` 的两种 ACL 模式做集成测试。
- **4d 错误提示**：认证失败 / 超时按服务商给针对性提示。

验证：vitest、`vue-tsc`、`cargo test`；`node scripts/machine-health.mjs` → `./scripts/install.sh`；在 Dev 实例里打开设置页把向导、测试连接、公共 / 自己的中继开关实际点一遍并存截图。配置不新增字段，不改 IPC，不需要 recovery 迁移。提交 `feat(ios): …`（`Release-Note: skip`）。

**结果（2026-11-22，`e2c37a8f`，后续小修 `30471705`）**：4a–4d 都做完了，`./scripts/install.sh` 已装机。验证：前端全量 142 个文件、2078 项，`vue-tsc` 干净；Rust 的 `ios::transport`、`config::`、`cli::channel`、`i18n` 共 121 项，clippy `-D warnings` 干净。隔离自检用装机的真实二进制跑过：对 Aedes 夹具 broker 的三种 ACL（`suback`、`forward`、`none`）和**真实 Mosquitto 2.1.2**（`pattern readwrite ah/%u/#` → 没有警告；没有 ACL → 给出警告），错误路径（密码错 → 拒绝、端口不通 → 超时）的提示文案也看过。设置页没有用 Dev 实例的真实窗口点（被遮挡的窗口内容截不到），改在浏览器里用**真实的 `SettingsView` + mock 后端**点了一遍并看过截图：服务商识别与要点、公共 / 自己的中继开关、测试成功 / 警告 / 失败 + 按服务商的提示（中英文各看过）。顺手修了两处：卡片简介随「有没有公共中继」换说法；端口两栏 4px 错位（既有问题）。

默认中继地址常量 `IOS_DEFAULT_RELAY_URL` 当时仍为空，3c 验证通过后另起一个小提交填入 `https://askhuman-relay.maipo.me`，同一次提交改了 CLI `--relay-url` 帮助、`ios-setup*.md` 的中继一节、隐私政策页（中英；Owner 看过措辞并批准，生效日期改为 2026-11-22）与 `overview-ios.md`。没做：隔离自检在真实 EMQX、flespi 上的运行（要凭据，见 `docs/PROGRESS.md` 悬而未决项）。

## P5：收尾

- 文档同步（已做）：`docs/overview-ios.md`（只改变了的部分）、`docs/wiki/ios-setup*.md` 的中继一节改为「默认使用公共中继；自编译 App 才需要自己部署」、本计划与规格的状态。
- `docs/PROGRESS.md`（已做）：删除「函数 URL 被标为仅供测试」那条待办（P3 解决，老配对的旧 URL 写进了规格 §6.2 与 §7）；记下未完成项（出包、复测、是否公开渠道）。
- 清理：预发临时函数已删，`/tmp` 里自己的草稿已清；Homebrew 的 `mosquitto cjson libevent libwebsockets` Owner 选了保留（2026-11-22）。
- 三条路径用最终版套件再跑一遍，结果表落进 `ios-broker.md`；端到端套件对 flespi 与 Mosquitto 各再跑一遍最终代码。**结果（2026-11-22）**：兼容套件自 P1 的存档运行后没有改过，存档结果仍有效；端到端套件的本地 broker 模式用装机的最终二进制跑过（`AHE2ETests`，80 项、9 项按设计跳过——Codex 付费 4、真实服务 3、模拟器 2、0 失败，约 7.6 分钟），覆盖 P4 对中继地址取值的改动在真实 daemon + 手机引擎上的配对、请求与推送路径。真实服务模式没有跑：Mosquitto 过不了（一次性 Mac 只带 `alertAfterSecs`，不带 `insecureLocal`，回环明文 TCP 会被拒，要用公共 CA 的 TLS），flespi / EMQX 要凭据，EMQX 还会吃 Owner 的流量额度。
- 隐私政策页核对中继的描述（默认中继后，设备 token 与 IP 会经过中继）（已做：中英各加了「只在内存里按网络地址做短期计数」一句，「停止使用我们的中继」改成选「使用自己的中继」，生效日期 2026-11-22）。
- 随下一次出包改 `docs/release/ios-external-beta/*.txt` 与 `ios-testflight` 技能里的 EMQX 字样，以及朋友内测指南 `docs/public/ios/beta*.md` 里「中继地址由邀请人单独提供」的步骤（带默认中继的 Mac 版本发布后，改成保持「使用公共中继」）（出包由用户另行指示，OA13）。

## 验证总表

| 项 | 方法 |
| --- | --- |
| 兼容套件 | 离线 `node --test`；对 Mosquitto / EMQX / flespi 各跑一遍存档 |
| Mac 前端 | `npx vitest run`、`vue-tsc` |
| Mac Rust | `cargo test`（含 ios 模块）；clippy 对齐 CI 工具链（见 PROGRESS） |
| 手机 | `swift test`（AHStore / AHCore）；`xcodebuild` 模拟器编译 |
| 中继 | `npm test`（relay）；预发函数小规模压测；熄火 / 恢复演练 |
| 整体 | `node scripts/machine-health.mjs` → `./scripts/install.sh`；Dev 实例里实际点向导与测试连接 |
| 端到端 | 仓库自带的端到端套件（真实 Mac 守护进程 + 手机引擎）对 flespi 与 Mosquitto 各再跑一遍最终代码 |
