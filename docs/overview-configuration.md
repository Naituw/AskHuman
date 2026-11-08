# 配置结构概览

> 本文是 `docs/overview.md` 的专题补充，记录配置模型与读取边界。用户向说明见 `docs/wiki/`，字段真值以 `src-tauri/src/config.rs` 为准。

## 文件与兼容性

- 主配置文件为 `~/.askhuman/config.json`；新位置缺失时回退旧 `~/.humaninloop/config.json`。
- 配置缺字段时取默认值，未知字段忽略，文件缺失或损坏时回退完整默认配置。
- Unix 下配置目录收紧为 `0700`，配置文件收紧为 `0600`。

## 字段地图

- `cursorDesktop.suppressBackgroundCompletionFollowUp`：默认 `false`，字段名沿用；一个开关管 Cursor
  会话防护的两个独立守卫——后台完成续跑拦截，以及 Agents 窗口里运行中会话保护（不卸载生成中会话的消息）。
  macOS Cursor 的 CLI/MCP 自动集成有效时，由 Daemon 管理实验性的 Desktop 全局接入；逐窗口、逐守卫原生行为预检
  通过后才修改并报告 active，一个守卫不可用时另一个照常（状态 `partial`）。有效能力独立保活，不改写
  `general.daemonLifecycle`；None 停止接入但保留偏好，终端 Cursor CLI 不在范围内。
- `cursorDesktop.launchPreference`：`terminal|desktop`，默认 `terminal`；新 Cursor 任务的运行位置。`desktop` 在 macOS 上经 Agents 窗口创建（实验），不可用时落回终端并在状态行说明；偏好终端时终端不可用也不会改用 Agents 窗口（与 Codex 的双向回退不同）；任务开始后不再改走另一条路径。
- `general`
  - `theme`、`language`、`alwaysOnTop`、`appearAnimation`、`windowEffect`（`solid|blur|glass`；配置默认 `blur`，显式 `glass` 在 macOS 26 以下有效值自动解析为 `blur`）
  - `speechLanguage`、`speechShortcut`
  - `historyLimit`（默认 200）与 `popupSound`
  - `menuBarIcon`（`off|active|always`，默认 `always`，macOS/Linux/Windows 桌面；已有显式值保持不变）
  - `popupPrewarm`（默认 `true`，macOS/Linux/Windows 桌面）
  - `daemonLifecycle`（`activity|keepalive`，默认 `activity`，macOS/Linux/Windows）
  - `quickReplies`（`null` 或字符串数组，默认缺省 = `null`）：弹窗与手机「快捷回复」菜单里的预设。`null` = 从没编辑过，Daemon 在建请求时取出厂 5 条并跟随界面语言（`i18n.rs::quick_reply_factory`）；一旦在设置里编辑就存成用户自己的列表（可以是空数组 = 不要预设）。读取一律经 `sanitize_quick_replies`：去首尾空白、丢空行、每条最多 200 个字符、最多 10 条（`QUICK_REPLIES_MAX` / `QUICK_REPLY_MAX_CHARS`，前端 `settings/quickReplies.ts` 有同样的常量，改要两边一起改）；`save_settings` 落盘前也会再过一遍。每个请求在创建时把解出的列表放进 `AskRequest.quick_replies`（What's next、严格选择与没有菜单的卡片为空），手机从 `AskView.quickReplies` 读到的就是同一份，所以每台 Mac 各用各的配置、手机不编辑。设计见 `docs/specs/special-replies.md` D27
  - `away`（离开模式的桌面外观与推送节奏；模式本身不在配置里，在 `~/.askhuman/state/away.json`）：`capsule`（桌面胶囊，默认 `true`，仅 macOS 有效）、`welcomeBack`（回到电脑时的「欢迎回来」提醒，默认 `true`，仅 macOS）、`pushAfterSecs`（离开期间升级提问等 Mac 空闲多久才推到手机，默认 60，`0` = 立即，上限同 `alertAfterSecs`；取它与 iOS 渠道自己的 `alertAfterSecs` 的较小值，所以只会让推送更早、不会更晚；只在 iOS 渠道开启时有意义）
- `channels.popup`：`enabled`、`width`、`height`、`rememberSize`
- `channels.telegram`：`enabled`、`botToken`、`chatId`、`apiBaseUrl`
- `channels.dingding`：`enabled`、`clientId`、`clientSecret`、`userId`、普通提问/确认/权限卡片模板 ID，以及文本附件内联/转 docx 开关；Agent 任务输入复用普通提问模板
- `channels.feishu`：`enabled`、`appId`、`appSecret`、`openId`、`baseUrl`
- `channels.slack`：`enabled`、`botToken`、`appToken`、`userId`
- `channels.ios`（实验，设置「实验」Tab 的 iOS 渠道卡）：`enabled`、`host`、`port`（TLS，默认 8883）、`wsPort`（WSS，默认 8084）、`username`（只含字母数字，同时是话题命名空间 `ah/{username}/`）、`password`（进钥匙串）、`relayUrl`、`macName`（留空取电脑名）、`alertAfterSecs`（手机没在看某个提问时，Mac 键盘 / 鼠标空闲满多少秒才推送通知，默认 900，`0` = 立即，上限 86400；改它不会重连 broker）；隐藏的 `insecureLocal` 只给回环测试 broker 用（对非回环地址无效）。手机配对、关注集、推送句柄与流量计数不在 `AppConfig`，存于 `~/.askhuman/ios/`
- `channels.autoActivation`：IM 渠道按需发送，默认 `false`
- `channels.autoEndWatch`：切离活跃 IM 后自动结束该渠道 watch，默认 `true`，仅在 `autoActivation` 开启时生效
- `agentTasks.enabled`：从手机（iPhone App 的「新任务」）/ IM（`/new`）创建电脑端 Agent 任务，默认 `false`；开启会强制 `daemonLifecycle=keepalive` 并同步 daemon 登录项。入口在设置「高级」Tab（2026-07-25 从「实验」转正）；iOS 渠道卡片顶部的提示条与 `AskHuman setup` 向导（GUI 在远程回答之后有一屏确认页，开启后当场检查手机上能看到的项目与 Agent；文字版是同一个问题）也能开启，效果相同（`docs/specs/onboarding.md` D25）
- `agentTasks.permissionPrompt`：`ask|agent-default|yolo`，默认 `ask`
- `agentTasks.cursorMax.{policy, plan}`：Cursor Max 模式控制。`policy` 为 `enforce-off`（默认，创建任务时强制关闭 Max）或 `inherit`（沿用 Cursor 当前设置，不干预）；`plan` 为 `auto`（默认，读 Cursor 的本地数据判断）、`legacy` 或 `token-based`（新计划无 Max，全部隐藏且不干预）；每次启动读取，改后立即生效
- `experimental.enabled`：显示实验区，默认 `false`（当前实验区里是 iOS 渠道卡；`channel list` 与帮助也只在它开启时列出 `ios`）
- `experimental.verticalQuestions`：多问题纵向同时显示全部问题，默认 `true`（设置「通用」Tab 可关回旧版一次一题；配置键沿用 `experimental.` 前缀）。2026-10-17 起出厂默认由关改为开：新配置与缺该字段的旧配置跟随新默认，已落盘的显式值（含 `false`）不迁移

## 读取入口

- `AppConfig::load()` 解析完整配置和系统钥匙串中的密钥；Daemon、IM 渠道、设置页密钥状态等确需凭据的路径使用它。
- `AppConfig::load_without_secrets()` 只读非密钥配置；语言、主题、历史上限、纯信息命令及窗口创建等路径使用它，避免无关操作触发钥匙串访问。
- `AppConfig::save()` 负责原子写入，并将受管密钥迁移或写入系统钥匙串；只持有非密钥配置的调用不会清空既有密钥。

密钥存储和 macOS 会话边界见 `docs/specs/secret-storage-keychain.md`。

Agent 任务的 workspace 索引独立存于 `~/.askhuman/agent-workspaces.json`，一次性启动记录存于
`~/.askhuman/state/agent-launches/`；二者都不是 `AppConfig`。

轻量界面状态（设置向导里用户明确跳过的域（及拒绝「手机任务」邀请的记录）与对「后台」做过选择的标记，即 `onboarding` 字段；
旧弹窗「配置 IM 渠道」小条的已关闭标记 `imTipDismissed` 只读保留、仍算「远程回答已跳过」）独立存于
`~/.askhuman/ui-state.json`（`src-tauri/src/uistate.rs`）：读失败视为默认、写失败静默，
与用户配置分离以免互相触发保存/迁移逻辑。向导的其余状态都从机器现状推导，不落盘
（`docs/specs/onboarding.md` §4）。
各 Agent 的 capability 偏好也不在 `AppConfig` 里，各自独立成文件：权限审批
`~/.askhuman/permission-preferences.json`、结束确认 `~/.askhuman/stop-preferences.json`、
接管 Claude 内置提问 `~/.askhuman/ask-question-preferences.json`（默认开，见
`docs/specs/claude-ask-user-question.md`）。它们与 `agents mode` 正交：偏好长期保存，
只有 CLI/MCP mode 才把对应 Hook 落到磁盘。
