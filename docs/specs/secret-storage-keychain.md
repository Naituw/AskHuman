# 需求：渠道密钥安全存储（系统钥匙串 + 构建签名）

> 状态：已实现并验证——B1–B4 + C1 + C2 全部完成；C2 经 v0.4.0 release CI 端到端验证（两 macOS target 以 Developer ID 签名通过）。
> 关联计划：`docs/plans/secret-storage-keychain.md`
> 影响面：配置读写、设置页、daemon 配置热重载、构建/发布签名。不改对外任务契约与渠道协议。

## 1. 背景与动机

`AskHuman` 把渠道凭据保存在 `~/.askhuman/config.json`。其中三项是**真正的密钥**：

- 钉钉 `channels.dingding.clientSecret`（AppSecret）
- 飞书 `channels.feishu.appSecret`（App Secret）
- Telegram `channels.telegram.botToken`（Bot Token）

问题：这些密钥以**明文**写在固定路径。任何能读到该文件的程序即可直接拿到密钥。最典型的威胁是「用户自己安装的第三方/恶意程序，直接读固定配置路径」。

参考做法（OpenClaw 钉钉插件）：插件本身不存密钥，而是用 **SecretRef 间接层**（`env`/`file`/`exec`）把「密钥存哪、怎么保护」交给宿主/用户，并对字段做 UI/日志脱敏。其核心思想是**不要把密钥明文写进配置**。

本需求采用对 GUI 用户更友好的等价方案：把三项密钥迁入**操作系统密钥库**（macOS 钥匙串 / Windows 凭据管理器 / Linux Secret Service），配置文件不再保存密钥真值。

## 2. 已完成的前置项（阶段 A，本需求的一部分）

> 这部分已在本次工作中实现并验证，列于此以完整记录需求。

- `~/.askhuman/config.json` 权限收紧到 **0600**、目录 `~/.askhuman` 收紧到 **0700**（Unix）；保存时显式设置，读取时对历史 0644 文件自愈。
- 设置页 Telegram `botToken` 输入框改为密码框（与钉钉/飞书一致）。

## 3. 目标

- 三项密钥（钉钉 AppSecret、飞书 App Secret、Telegram Bot Token）迁入系统密钥库；`config.json` 在密钥库模式下这些字段**留空**。
- 现有读取密钥的代码（渠道连接、配置热重载比对等）**无需大改**：内存中的 `AppConfig` 始终携带「解析后的密钥真值」。
- **自动迁移**：启动时若发现 `config.json` 里还有明文密钥，搬进密钥库并清空字段。
- **优雅回退**：密钥库不可用（如无 Secret Service 的 Linux/无头环境）时，密钥写回 `config.json`（0600 明文）并告警，保证功能可用。
- 设置页**不把密钥读进界面**：以「已保存」占位表示已配置；留空＝不改、输入＝覆盖、另给「清除」。
- 构建签名：让本地开发与正式发布的二进制具备**稳定签名身份**，使密钥库读取「受信任、免弹框」，且不削弱「第三方程序读不到」的安全收益。

## 4. 非目标

- **公证（notarization）**：暂不做。仅 `codesign` 即满足「钥匙串免弹框」与 npm 分发（npm/curl 安装通常不带 quarantine，Gatekeeper 多半不触发）。后续如遇 Gatekeeper 问题再单独评估。
- **加密 config.json 其余内容**：非密钥项（clientId/appId/chatId/openId/userId、各类开关）仍明文存配置。
- **防御「同用户身份、主动调用密钥库 API」的高级恶意程序**：这是「daemon 必须无人值守读密钥」带来的固有上限，任何方案都无法根除；本需求只把门槛提到「第三方无法静默读固定文件」。
- 不引入 `env`/`file`/`exec` 形式的 SecretRef（评估后选择钥匙串方案，对 GUI 用户更友好）。

## 5. 密钥库存储与解析模型（关键设计）

### 5.1 范围
仅三项密钥进密钥库；ID 类（clientId/appId/chatId/openId/userId）与 base_url/api_base_url、各开关仍留 `config.json`。

### 5.2 条目命名
- 服务名（service）：`com.naituw.humaninloop`（与 bundle identifier 一致）。
- 账户名（account）逐项固定：
  - `channels.dingding.clientSecret`
  - `channels.feishu.appSecret`
  - `channels.telegram.botToken`

### 5.3 解析优先级（读取）
对每个密钥，按以下顺序取「有效值」：
1. 若 `config.json` 对应字段**非空** → 用它（代表「尚未迁移」或「回退明文模式」）。
2. 否则查密钥库该 account：命中 → 用之；`NoEntry` → 视为未配置（空）；其它错误 → 视为密钥库不可用（空，触发回退判断）。

内存中的 `AppConfig` 在 `load()` 后，这些字段填入「解析后的真值」。**因此所有现有 `config.channels.*.clientSecret` 等读取点行为不变**。

### 5.4 写入（保存）
保存时对每个密钥：
- 用户提供了新值：尝试写入密钥库；成功 → 该字段在写盘的 JSON 中**留空**；失败（密钥库不可用）→ 该字段以**明文**写入 JSON（回退）。
- 写盘始终是「临时文件 + rename」原子写，并保持 0600。

内存 `AppConfig` 仍保留解析后的真值（不被清空）。

### 5.5 自动迁移
`AppConfig::load()`（真实运行入口）执行一次性迁移：若某密钥字段非空且密钥库可用 → 写入密钥库 → 清空字段 → 原子重写 `config.json`。迁移为尽力而为：失败则保持明文（回退），不阻断启动。

### 5.6 平台后端
使用 `keyring` crate，按平台启用后端：macOS `apple-native`、Windows `windows-native`、Linux Secret Service（D-Bus，需 gnome-keyring/kwallet）。Linux 无 Secret Service 的无头环境 → `get/set` 报错 → 走 5.3/5.4 的回退路径。

## 6. 设置页交互

- 加载设置时，后端返回的配置中三项密钥**为空**，另返回「是否已配置」标志（每项一个布尔）。前端据此显示占位「已保存 / Saved」。
- 保存时区分三种意图：**不变**（用户没动，留占位/空）、**覆盖**（用户输入了新值）、**清除**（用户点「清除」按钮）。
  - 不变：不动密钥库与字段。
  - 覆盖：按 5.4 写入。
  - 清除：删除密钥库该 account，且字段留空。
- 密钥真值**不经由设置加载流出密钥库**（仅在保存时由设置进程写入；daemon 运行时自行从密钥库读取）。

## 7. 与 daemon 配置热重载的衔接

- daemon 通过 `notify` 监听 `config.json` 变更。设置页改密钥时即使 JSON 内容相同（密钥字段都空），原子写 rename 仍会触发监听脉冲。
- daemon 收到脉冲 → `AppConfig::load()` **重新从密钥库解析**密钥 → `invalidate_changed_routers` 比对的是**解析后的真值**（内存旧值 vs 新值）→ 能正确识别「仅密钥变化」并失效对应缓存 Router（惰性失效，行为与现状一致）。
- 因此密钥从配置迁到密钥库后，热重载/惰性失效逻辑**仍然成立**，无需新增 IPC。

## 8. 构建签名（消除开发期弹框 + 保障发布免弹）

### 8.1 为什么需要
macOS 密钥库按「指定要求（DR）」判断调用方可信。用稳定证书 + **固定 identifier** 签名后，DR 形如
`identifier "com.naituw.humaninloop" and anchor apple generic and certificate leaf = "<证书 CN>"`，**不含 cdhash** → 每次重新编译（cdhash 变）仍满足同一 DR → 读取免弹框。
- 安全收益不变：没有该证书 + 该 identifier 的第三方程序仍无法静默读取（会弹框/失败）。
- 实测确认：ad-hoc（无 Team、按 cdhash 绑定）每次重装必弹；改用 Apple Development 证书 + 固定 identifier 后，不同 cdhash 读取静默成功。

### 8.2 本地开发（`scripts/install.sh`）
- 签名命令：`codesign -i com.naituw.humaninloop --force --sign "<identity>" <bin>`。
- identity 选取：优先环境变量 `CODESIGN_IDENTITY`；未设则**自动探测**本机首个有效 codesigning 证书；都没有则**回退 ad-hoc**（仅导致开发期偶尔弹框，不影响安全/发布）。
- 固定 identifier 始终为 `com.naituw.humaninloop`。

### 8.3 正式发布（`.github/workflows/release.yml`，仅 macOS 两个 target）
- **显式、固定**：从 GitHub Secrets 导入 **Developer ID Application** 证书（`.p12`）到临时钥匙串，用确切 identity 签名，固定 identifier `com.naituw.humaninloop`。**不自动探测**。
- 所需 Secrets：① Developer ID `.p12` 的 base64；② 该 `.p12` 导出密码；③ 临时钥匙串密码。
- 暂不公证（见非目标）。
- 缘由：未签名发布会让终端用户每次更新都弹钥匙串框，故 **CI 签名是「发布本特性」的前置条件**。v0.4.0 release 已落地并验证（`codesign -dvv` 显示 Developer ID + 固定 identifier，`--verify --strict` 通过）。
- 运行环境：macOS 构建必须用 `macos-26` runner（提供 macOS 26 SDK）——Swift 语音桥用到 `SpeechAnalyzer` 等 26 专有 API，`macos-14`(Sonoma SDK) 编译失败；部署目标仍 11.0 + `if #available` 弱链接，产物可在旧系统运行。`build.yml` 同步改 `macos-26`。

### 8.4 证书准备（供用户操作，非代码）
- 发布证书复用账号级的 **Developer ID Application**（需付费 Apple Developer Program；CLI 与 .app 用同一种证书，可复用现有）。
- 本地开发证书用 **Apple Development**（本机已有 `Apple Development: Wu Tian`）。
- 两类证书用途不同、互不影响；密钥库条目由谁创建即绑谁的 DR（开发机用开发证书，终端用户用发布证书，各自自洽）。

## 9. 锁屏 / 离开 / daemon 回收 场景（确认无影响）

- macOS **锁屏 ≠ 锁钥匙串**；登录钥匙串默认 `no-timeout`、无睡眠锁定 → 整会话保持解锁（已在目标机器实测 `no-timeout`）。
- 「吃饭+锁屏」期间 Agent 提问：即便 daemon 已空闲回收，CLI 会重起新 daemon，其读密钥库**静默成功**（钥匙串解锁 + 二进制受信任）→ IM 正常发出。
- daemon 首次读后把密钥**缓存在内存**，已建连接不受后续钥匙串状态影响。
- 仅当钥匙串真被锁（注销 / 睡眠且开了睡眠锁定）且此时需新建连接才受影响 → 走优雅回退；且弹窗不需密钥仍可用。
- 正交前提（与密钥存哪无关）：离开时要**收到** IM，Mac 需保持唤醒。

## 10. 验收标准

- 三项密钥保存后，`config.json` 中对应字段为空，密钥库中存在对应条目；功能（钉钉/飞书/Telegram 收发）正常。
- 历史明文配置启动后被自动迁移（字段清空、条目就位）。
- 临时禁用/移除密钥库（模拟不可用）→ 回退明文 0600 + 告警，功能仍可用。
- 设置页：已配置显示「已保存」；留空保存不改密钥；输入覆盖；清除删除条目。
- 改密钥后 daemon 能识别变化并对下一个请求用新密钥（热重载/惰性失效）。
- 本地 `install.sh` 用证书签名后，重装后 daemon/设置读密钥库**不再弹框**；发布工作流对 macOS 二进制完成签名。
