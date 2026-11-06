# iOS 朋友外部测试：开发与交付计划

日期：2026-11-05，跟进于 2026-11-06。状态：0.1.0 (10) 已获批并开始外测，首位朋友已获邀请，等待接受、安装与接入验收。工作树不挂渠道预设；GitHub Pages 已获授权启用，审核联系人已按用户选择复用 Spatialify。

## 已确认的目标

首批 1–5 位开发者朋友，各用自己的 Mac、AskHuman、Agent 与 EMQX。开发者提供现有 APNs 推送中继。采用 TestFlight 外部邮件邀请，不公开邀请链接，不给朋友 App Store Connect 后台权限。先不增加演示模式，直接尝试 Beta App Review；审核要求补充资源时，按实际反馈再与用户决定，不擅自扩大范围。首批中继不迁移、不加自动开户或用户配额系统。

用户已要求写开发计划并由 agent 推进全部流程，包括实现、核验、准备资料、上传及送审。联系人、隐私页面发布位置和首批邀请名单须由用户提供或确认；不猜测身份和公开联系方式。

## 起始证据与缺口（2026-11-05 实施前只读核查）

- AskHuman Mobile：App ID 6828712684，Bundle ID com.naituw.humaninloop.ios，沿用 Team、通知扩展和共享钥匙串组。
- 最新 Apple 构建为 **0.1.0 (9)**，VALID、INTERNAL_ONLY，外测状态 NOT_APPLICABLE；本地 `ios-testflight-9` 指向 fc87415d。它不能转成外测构建；实际出包前再次查询最高号。
- 仅有 AskHuman Internal 组，Beta 应用级本地化为空，审核联系人 / 备注为空。build 9 的中英文 What to Test 已存在，与应用级 Beta 材料分开。
- 已从其他两个应用读取到可复用联系人。两者反馈邮箱相同，审核姓名 / 邮箱不同；数据保存在仓库外的私有目录，等待用户选来源，不在本文公开。
- GitHub Pages 尚未启用。用户明确要求配置准备好之后再确认启用；工作流已准备为仅手动发布。
- 最新公开 Mac 发布仍为 v0.16.0。送审 / 邀请材料需要附明确可用的兼容 Mac 包与固定源码点，不能只写“装最新版”。
- 工具、公开文案、App 链接与审核备注草稿已实现；真实 Apple 写入、外测上传、送审和邀请尚未执行。

## P0：工作树与只读外测核对

1. 从当前 main 创建 codex/ios-external-testflight 工作树；按 AGENTS 询问是否挂渠道预设，默认不挂，通过正式端转送 Dev 验收。
2. dev enable，先跑 machine-health，再 install.sh，并以项目 dev_ask / dev.mjs ask 验证本树环境。
3. 扩充签名 helper 的只读状态操作：查询本应用的 Beta 描述/隐私 URL/反馈邮箱、审核资料完整性、内部与外部组、构建 internal-only 标记、外测审核状态和出口合规状态。报告联系方式是否缺失，避免无必要打印个人信息。
4. 保持固定 App ID 和已签名 helper 的凭据边界，不提供任意 URL、通用 API 代理或导出密钥操作。读回真实后台，再确定尚缺材料。

验收：原内部 status/notes 仍工作；新增结果可明确区分 processingState 与 external testing/review state；已有后台内容不改动。

## P1：审核材料与朋友接入资料

1. 在 docs 下新增中英文朋友内测指南：兼容的 Mac 与 iOS 版本、安装入口、自备 EMQX 的账号/权限、开发者中继、实验功能开关、配对、通知时机、故障反馈。
2. 起草隐私政策与可访问的公开页面：端到端密文、broker/中继/APNs 的职责与元数据、设备 token、手机历史及附件、测试反馈、删除/撤销方法、联系方式。最终内容按实际代码与服务日志核对；不笼统声称没有数据存储。
3. 按用户确定的位置发布隐私页面；App 内提供可见的隐私链接，复用现有设置/关于界面，保持最小改动。PrivacyInfo.xcprivacy 与公开隐私政策分别处理。
4. 准备中英文 Beta 描述、首轮 What to Test，以及英文审核备注。明确 Mac 配套端、自备 broker、核心功能的完整接入步骤；不伪造演示账号，不提交会过期的一次性配对码，不暗示已有演示模式。
5. 收集反馈邮箱、审核联系人姓名/邮箱/电话及首批朋友邮箱；联系人资料只进私有临时文件和 Apple，不写入公开 git。

验收：公开链接可访问且内容真实；新装 App 可进入隐私页面；审阅材料完整，安装步骤可复现。无演示模式作为已接受的审核风险记录。

## P2：内部/外部明确区分的上传支持

1. 默认沿用 internal；新增显式 external 选择，在 Python 预检、请求编码与签名 helper 中一致校验。
2. 外部模式生成临时导出选项，使新构建可以外测；不覆盖日常内部上传配置、不更改 Bundle ID/Team/profiles。
3. 保留 archive、版本、设备族、iPad 方向、生产 APNs、privacy manifest、密钥临时文件清理、独占锁与中断恢复检查。
4. 更新 ios-testflight skill 与必要规格，明确内部交付与外部审核/交付完成标准。既有 changes 与标签继续按统一构建号记录。

验收：默认内部仍是内部；显式外部才解除 internal-only；不合法的受众、配置或构建在获取凭据/上传前拒绝；测试不上传真实构建。

## P3：受限的外测资料、送审与邀请操作

1. 以 Apple 官方 API schema 为依据，实现本应用所需的资料写入、外部组管理、指定构建提交审核、指定邮箱邀请和读回。后台可用时亦可走可审查的 UI；不为此增加任意 API 调用能力。
2. 分离“上传”“提交 Beta 审核”“向朋友发邀请”，每步可单独核验。组关闭公开链接，首轮采用人工指定构建。
3. 所有变更先读现状并计算差异；已存在的资料/组不重复创建；提交结果不明时先查状态，不重复上传/送审/邀请。
4. API 权限不足、条款或身份核验交回用户处理，不自行提权、创建新密钥或接受法律协议。

验收：模拟 HTTP 测试覆盖固定应用/构建/组边界、请求字段、失败与幂等路径；真实后台只执行本次目标的必要变更并读回。

## P4：验证、明确源码点与首个外测构建

1. 测试 Python/Swift 协议与边界、原有 changes/notes 回归；检查英文注释、格式、类型与涉及的 iOS 构建/UI。
2. 所有长构建前运行 machine-health。退出 2 时按项目规则通知用户，在后台运行并用短轮询汇报。
3. 安装稳定签名 helper（保留 previous），用只读状态/notes 和本地签名检查验证 Keychain 边界；不把未授权写入当作测试。
4. 功能改动按项目规则 install.sh 到本树并 Dev 验收；提交 Conventional Commits，确保归档源码点干净且可追溯。
5. 出包前重新查询最高构建号与本地基线；核对自上次以来全部 iOS 改动，以及用户已明确不应交付的内容。保持同一条提交历史，不拼装发布源。
6. 归档 Release，确认 ARCHIVE SUCCEEDED；预检通过后单次外部模式上传，等待 Apple VALID。
7. 写入并读回中英文测试说明，设置外部组、审核资料，提交 Beta App Review。上传成功与审核通过分别记录。

验收：新构建允许外测，Apple 接受送审；源码 commit、版本/build、Apple build ID、外测状态与材料版本可追溯。若 Apple 拒绝，先向用户解释具体原因及建议，再决定补充方案。

## P5：批准后邀请与交付

1. 观察审核状态；若等待时间跨会话，在 PROGRESS 记录准确状态、构建及下一步，不宣称完成，也不擅自创建自动化。
2. 审核通过后，仅邀请已确认的首批邮箱；检查邀请状态，提供 TestFlight 与接入指南。
3. 先引导第一位朋友验证配置、配对、前台回答、锁屏生产推送、切网恢复与首答生效；真实 Agent 的计费调用由朋友自行决定。
4. 区分“已邀请”“已接受”“已安装”“实际接入通过”，不以上传 100% 或后台 VALID 代替交付。
5. 记录构建源码标签和交付结果；更新适当的专题文档与进度条目。全局架构未变时不修改主 overview。

最终完成条件：可外测构建获批，确认名单中的朋友已获邀请，并有首位朋友成功安装/配对/问答与推送的证据；其他人的未完成动作明确列出。若用户只要求推进到送审/邀请，则按其明确调整的边界交付。

## 输入处理结果

- 已批准并启用 GitHub Pages 的 Actions 发布，隐私 / 接入页面已上线。
- 用户选定 Spatialify 的审核联系人，反馈邮箱复用两个已有应用一致的值；均只存私有输入并提交 Apple。
- 兼容 Mac 包的公开下载地址与既有中继 URL 已填入最终私有审核材料；朋友邮箱尚未提供，可以等获批后再提供。

## 执行记录

- 工作树：codex/ios-external-testflight，起点 fc87415d；不挂渠道预设。Dev 已启用，首次 install.sh 及正式转送已通过（收到“收到”）。
- 官方 API 依据：Apple OpenAPI 4.6，2026-11-05 下载核对；不将完整 schema 入库。
- 完成 `external-status` / 显式 `contact-sources`、外部导出选择、封闭 metadata/group/submit/invite 操作；真实 Apple 只读已验证，未触发写入。
- 工具回归 **24/24** 通过，包含旧 notes / changes / archive 规则、新 audience、资料读回、提交幂等、外测状态与邀请边界。
- iOS 27.0 专用 iPhone 18 Pro 模拟器：`testMacsTabListsTheMacsAndOpensOne` **1/1** 通过，App 与扩展完成编译。未动日常 Duo 模拟器。
- 静态站生成 5 页，桌面与 390px 手机宽度浏览器预览通过；后续公开部署见下文。
- 新 helper 已按既有 Developer ID 重装，保留 previous；通过真实 external-status、build 9 notes read。以现有 build 9 archive 做外部模式本地导出，`EXPORT SUCCEEDED`（仅签名验证，没有上传或改变 build 9 的 Apple 受众）。
- 最后 install.sh 已成功装到本树 Dev 目录；最终转送收到“收到”（verification 2d687f06-9693-45db-9553-1a35130802f8）。未覆盖日常二进制。
- 构建前健康检查曾退出 2（交换 9.6 GiB、负载 215）；按 AGENTS 报告后用户回复“继续”。后续负载回落，检查为 tight，长任务后台运行。

P0–P4 已完成。P5 于 2026-11-06 完成获批、开始测试和首位朋友邀请，安装、配对与推送真机验收仍未完成；最新证据见文末。

### 候选产物与固定源码点

- 源码点 `d3bb8336e38ff27d01ca0d97d80f3c561e5b1847`，包含实现提交 `8398a3e3`，已合入 main 的 build 9 / 真实 Codex 验收文档。
- `changes.mjs`：基线 `ios-testflight-9`，一个 iOS 增量提交（关于页两处链接），建议下一号 10。
- `0.1.0 (10)` 归档：`ios/build/AskHuman-External-Candidate.xcarchive`，Release `ARCHIVE SUCCEEDED`，external preflight 通过；选择号码及上传前均核对 Apple 最大构建为 9。
- 对应中英文 What to Test 已生成并 dry-run 通过（英文 746、中文 265 UTF-16 units）；保存在仓库外。
- Apple Silicon Mac 配套包从同一源码点用普通 `custom-protocol` / `local-install` 构建，不带 development-ui；通过既有 Developer ID 签名校验和 `--help` 启动。没有替换日常安装。
  私有产物目录 `~/.config/askhuman-release/friends-candidate-d3bb833/`，zip 16.7 MiB，含二进制、安装说明和 SHA256SUMS；binary SHA256 为 `3af24ef66f606a5df56846f9b2446817bf5ec7ae2190341341043ded38505357`。
  它已发布，尚未公证、未在朋友的新 Mac 验证；不是 Intel / universal 包。版本字符串仍为 0.16.0，以源码点 / 校验值区别旧公开发布。
- 已读现有中继 URL并放入私有英文审核材料；`GET /v1/health` 返回 HTTP 200、ok=true。没有复制 broker 凭据。最终 metadata JSON 包含公开 Mac 下载 URL 与选定联系人，已提交并读回。
- 经用户确认：代码合并后手动发布 Pages；Mac zip 作为单独的 GitHub prerelease 资源（非 `v*` 标签，不触发正式桌面发版流水线），保留现有稳定版本。

### 发布授权与版本冻结

用户已批准启用 Pages、发布独立 Mac 预览包并继续上传 / 送审；审核联系人沿用 Spatialify。
发布仍采用已验收的 d3bb833 候选。等待期间 main 增加的真实 Codex 测试及文档已合入发布工具分支；
它们不进入现有候选归档。主工作树另有进行中的未提交 iOS 改动，本次在隔离树推送已提交历史，不操作那些文件。
最终 Dev 已完成；Apple 重查最新仍为 9、INTERNAL_ONLY，尚无外部组。

### 已发布与首次上传结果

- 远端 main / codex/ios-external-testflight 已推至 e22cb150；完整 CI（run 42040697614）通过。
- Pages 已启用 workflow 模式、HTTPS 强制；部署 run 42040761031 通过。四个公开页面均无需登录 HTTP 200，内容与候选一致。
- Mac 预览版：<https://github.com/Naituw/AskHuman/releases/tag/friends-beta-20261105>；tag 固定 d3bb833，prerelease、非 latest，稳定版仍为 v0.16.0。公开 ZIP 下载 SHA256 与本地一致。
- Apple 应用级中英文 Beta 资料及 Spatialify 审核联系人已写入并完整读回。PATCH 曾超时，但读回证明全部字段已保存，没有重复写入。
- 用户释放资源后磁盘从约 277 MiB 恢复到约 50 GiB；本树 Cargo local-install 缓存已用官方 clean 命令清理，归档 / 安装包仍保留。
- 2026-11-05 23:03 开始第一次 build 10 外部上传。签名通过，Xcode 的 swinfo / xcrun 工具发现阶段约 14 分钟；23:18 文件传输到 100%，23:19 最后的 CHANGE UPLOAD STATE TO COMPLETE 返回 401 / reauthenticationNotSupported，进程退出 70（EXPORT FAILED）。
- 首次失败时，上传会话 81b235eb-4db3-46ee-8372-e74ca5bcbe0a 最后读到 AWAITING_UPLOAD；新 token 的只读查询仍成功，Apple 构建列表最高仍为 9。当时没有把传输 100% 当作上传完成，也没有创建标签、外测组或审核提交。

### 受控重试成功与 Beta 审核提交

- 资源平稳后再次确认 Apple 未出现 build 10，使用同一归档与新上传认证重试；没有更换 API key、证书或候选源码。2026-11-05 **23:34:13（北京时间）** 返回 `Upload succeeded` / `EXPORT SUCCEEDED`。
- 随后 Apple 处理完成：build `81b235eb-4db3-46ee-8372-e74ca5bcbe0a`，版本 0.1.0 (10)，`VALID`、`APP_STORE_ELIGIBLE`、未过期、非豁免加密为 false。到期时间 2027-02-03 23:35:46（北京时间）。持久化内部导出配置仍为 internal-only，临时凭据已清理。
- 中英文 What to Test 写入并逐字读回，中文 265、英文 746 UTF-16 units；中文全文记录于 [iOS 交付记录](ios-channel.md#2026-11-05testflight-外部候选构建-10)，英文与中文对应。
- 建立 `AskHuman Friends`（组 ID `168413b8-7d93-45ab-9ca1-8810aea0a186`）：外部组，公开链接关闭，手动指定 build 10，自动通知关闭，测试员 0。原内部组也已自动收到 build 10。
- **23:45:26** 提交并读回：审核提交 `WAITING_FOR_REVIEW`，构建外测状态 `WAITING_FOR_BETA_REVIEW`。这表示等待 Beta App Review，尚未获批，也没有邀请朋友。
- 本地附注标签 `ios-testflight-10` 指向明确批准并冻结的源码 `d3bb8336e38ff27d01ca0d97d80f3c561e5b1847`，未推送标签。后续合入 main 的“已结束问题页面”及测试 / 文档不在该归档，不能把当前 HEAD 当作这个包的源码。
- 后续只查询审核进展；审核通过后接收明确邮箱名单并执行邀请、首位朋友验收。没有创建定时监控；若 Apple 要求演示资源，携实际反馈向用户确认，不自行扩展范围。
- 用户收到完整交付报告及中英文测试说明后，明确选择“后续记为待办，拿到审核结果和邮箱后继续”。已创建项目待办 `7ceb1fca-b979-4508-8ebd-1ad240058f8f`；本次工作按此边界完成，P5 延后，不建立定时监控。

### 2026-11-06：获批与首位朋友邀请

- 跟进查询确认 build 10 为 `BETA_APPROVED`、VALID、未过期。用户提供一个确切邮箱，仅邀请这一位，暂时没有其他人的计划；邮箱保存在仓库外 0600 输入文件，不写入 git。对方为 Apple Silicon Mac，Mac 安装改用用户即将发布的新版本，实际版本待记录。
- 首次邀请在 `GET betaTesters` 返回 HTTP 400 / `PARAMETER_ERROR.INVALID`，只读复查 Friends 仍 0 人。经用户批准在既有工作树修复、不挂渠道预设：成员关系改从本应用已验证的 Friends 组端点读取，再按邮箱忽略大小写精确匹配；账号范围复用仍限确切邮箱。
- 修复查询后，成员已加入，但 `POST betaTesterInvitations` 返回 HTTP 409 / `STATE_ERROR.TESTER_INVITE.NO_INSTALLABLE_BUILDS`；复查 Friends 为 1 位 `NOT_INVITED`，构建仍 `BETA_APPROVED`。自动通知关闭时，还需手动开始测试，不能把获批当作已经可安装。
- 按 Apple OpenAPI 4.6 和官方 Build Beta Notifications 文档补上封闭的 `start` 操作。该操作会通知构建所有已分配测试员，用户已另行明确批准：原有 2 位内部测试员和这一位朋友。执行前验证只有 Friends / AskHuman Internal 两组、确切外部邮箱表、批准的内部人数，以及无超出两组的独立测试员；不改变自动通知或公开链接。
- 新邀请路径在 `BETA_APPROVED` 只准备成员并返回 `requiresStartTesting: true`，不尝试发出不可安装构建的邀请；开始测试独立授权，ready/testing 后才继续确切收件人的邀请。未知结果不自动重发。
- 工具回归 **24/24** 通过：先复现旧查询失败，再验证修复，并覆盖准备成员、开始测试去重、受众变更拒绝、分页拒绝和结果不明停止。稳定签名 helper 已重装；原 status、build 10 中英文 notes read 和本地外部签名 `EXPORT SUCCEEDED` 均通过，没有新上传或修改 build 10 内容。工作树 install.sh 成功，初次 Dev 正式转送收到“收到”；最终转送的回复待补。
- **22:35（北京时间）**：`start` 读回 `IN_BETA_TESTING`、外部 1 人、内部 2 人；随后 `invite` 读回 `INVITED: 1`、`requiresStartTesting: false`、`changed: 0`，未重复创建成员。尚无接受邀请、安装、配对或生产推送通过的证据。
- 接入指南、隐私页面、预览包下载和现有中继 `/v1/health` 均实际检查成功；后续 Mac 按用户选择的新发布版安装。首位朋友需完成前台回答、锁屏通知与点击回答、Wi-Fi/蜂窝切换和首答生效，才能完成 P5。原项目待办仍保留，无定时监控。
- 用户再次明确先等自己发布新 Mac 版本；收到版本号或链接后再继续朋友安装验收，不在发布前催促操作。工具修复留在外测工作树，主工作树正有其他发布改动，未混入或覆盖。
