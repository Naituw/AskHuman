# iOS 渠道免费开放：开发与交付计划

日期：2026-11-21。状态：P1、P2 已完成；P3 起要改线上中继，需要用户在腾讯云控制台配合（中继子域名待定，P3 开始前再问用户）。需求与决策见 [`docs/specs/ios-open-access.md`](../specs/ios-open-access.md)。

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

**3a 预发验证**：同一个 zip 部署到一个**临时函数**，不动生产；验证完删除。逐项验证，任何一项不成立就停下来问用户：

1. Web 函数是否支持「最大独占配额」；超限时的返回码；超限请求是否不计费。
2. 自定义域名能否映射到「别名」上的函数 URL；HTTPS 证书与 HTTP→HTTPS。
3. 请求里客户端真实 IP 在哪个头里（决定注册限流怎么按 IP 分桶）。
4. 默认域名「仅供测试」的限制，是否也适用于自定义域名。
5. 单实例处理「拒绝类」请求的实际吞吐（把规格里「每实例 100 次/秒」的假设换成实测）。
6. 函数 URL / 自定义域名有没有可配置的限流（QPS）。

**3b 中继代码**（`relay/src/core.js`、`adapters/scf.js`、`relay/test/*`）：注册限流现在是一个全局共享桶（键恒为 `'all'`），有人刷注册就会把所有新用户挤掉。改为**按客户端 IP 分桶 + 全局兜底**；推送限流（按句柄编号）不动。桶仍是单实例内存、各实例各算一份，这是公平性手段，不是成本手段，文档里明说。单元测试覆盖 IP 解析、分桶、兜底；API 不变，旧客户端照常工作。提交 `feat(relay): rate-limit registration per client ip`（`Release-Note: skip`）。

**3c 生产部署**（用户在控制台点，我给逐步清单并在外部验证，不需要任何凭据）：

1. 发布版本 + 别名 `live`；在别名上开函数 URL（现有 URL 保持不动）。
2. 自定义域名：已备案域名的子域名（单域名，不支持泛域名），CNAME 到控制台给出的目标，路径 `/*` 映射到 `live`，HTTPS。
3. 最大独占配额 512 MB（4 × 128 MB）。
4. 告警与预算（规格 4.3）。
5. 熄火 / 恢复各演练一次。

**3d 自动熄火：不做**（OA14）。

验证：外部 `curl`（`/v1/health`、`/v1/register` 错误路径、`/v1/push` 无效句柄）；小规模并发压测（几十并发、十几秒，费用可忽略）观察超限返回；日志仍只含方法、状态、耗时。

## P4：向导与默认中继（Mac 设置页与 Rust）

- **4a 服务商目录与向导**：`src/views/settings/iosBrokers.ts`（纯数据）；`IosChannelCard.vue` 顶部加下拉与要点块；i18n 中英文；组件测试。不新增配置字段。
- **4b 默认中继**：Rust 新增 `effective_relay_url(cfg)`；`pairing.rs`（二维码里的 `r`）、`requests.rs`（推送）、`connstr.rs`、`daemon/runtime/ios.rs`、`cli/channel_cmd.rs`、`commands.rs`（健康检查）全部改用它；界面「使用公共中继 / 使用自己的中继」开关；设置里写明公共中继的数据与可用性说明。**默认地址要等 3c 域名生效后才合入。**
- **4c 隔离自检**：见规格 6.3。`ProbeReport` 新增 `isolation`；`ios_test` 的返回改为「消息 + 警告列表」；设置页与 CLI 都按结构显示；用 `local-broker` 的两种 ACL 模式做集成测试。
- **4d 错误提示**：认证失败 / 超时按服务商给针对性提示。

验证：vitest、`vue-tsc`、`cargo test`；`node scripts/machine-health.mjs` → `./scripts/install.sh`；在 Dev 实例里打开设置页把向导、测试连接、公共 / 自己的中继开关实际点一遍并存截图。配置不新增字段，不改 IPC，不需要 recovery 迁移。提交 `feat(ios): …`（`Release-Note: skip`）。

## P5：收尾

- 文档同步：`docs/overview-ios.md`（只改变了的部分）、`docs/wiki/ios-setup*.md` 的中继一节改为「默认使用公共中继；自编译 App 才需要自己部署」、本计划与规格的状态。
- `docs/PROGRESS.md`：删除「函数 URL 被标为仅供测试」那条待办（P3 解决）；记下未完成项（出包、复测、是否公开渠道）。
- 清理：`brew uninstall mosquitto cjson libevent libwebsockets`（用户同意后）；`/tmp` 里的草稿；预发临时函数。
- 三条路径用最终版套件再跑一遍，结果表落进 `ios-broker.md`；端到端套件对 flespi 与 Mosquitto 各再跑一遍最终代码。
- 隐私政策页核对中继的描述（默认中继后，设备 token 与 IP 会经过中继）。
- 随下一次出包改 `docs/release/ios-external-beta/*.txt` 与 `ios-testflight` 技能里的 EMQX 字样（出包由用户另行指示，OA13）。

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
