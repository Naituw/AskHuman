# Slack 渠道配置

简体中文 | [English](./slack-setup.en.md)

本文说明如何创建并配置一个 Slack App，使 AskHuman 的「Slack Channel」可用。该渠道采用 **Slack App + Socket Mode 长连接（WebSocket）+ 机器人 + 单聊（DM）** 形态，**无需公网**即可收发消息与卡片交互。

## 一、创建应用

1. 打开 [Slack API → Your Apps](https://api.slack.com/apps) → **Create New App** → **From scratch**。
2. 填写 App 名称并选择目标 Workspace → **Create App**。

> 头像：可在 **Settings → Basic Information → Display Information** 设置 App 图标。仓库内置两张可作头像的图片（深色 / 浅色背景），也可用于其它场景：[深色](../../assets/avatars/bot-avatar-dark.jpg) · [浅色](../../assets/avatars/bot-avatar-light.jpg)。

## 二、开启 Socket Mode 并获取 App-Level Token

1. 左侧 **Settings → Socket Mode** → 打开 **Enable Socket Mode** 开关。
2. 开启时会提示创建 **App-Level Token**：命名任意，勾选 scope **`connections:write`** → **Generate**。
3. 记录生成的 **App-Level Token**（以 `xapp-` 开头，填入 AskHuman 设置页）。

## 三、配置 Bot Token 权限（Scopes）

1. 左侧 **Features → OAuth & Permissions** → 找到 **Scopes → Bot Token Scopes** → **Add an OAuth Scope**，添加：

| Scope | 用途 |
| --- | --- |
| `chat:write` | 发送 / 更新消息与互动卡片 |
| `im:write` | 打开与目标用户的单聊（DM）频道 |
| `im:history` | 接收用户在 DM 中发来的消息事件 |
| `files:read` | 下载用户在 DM 里发的图片 / 文件（人 → AI 回传） |
| `files:write` | 上传 `-f` 附件（AI → 人发文件） |

2. 回到该页顶部 **OAuth Tokens → Install to Workspace**（每次增删 scope 后都需重新安装）。
3. 安装后记录 **Bot User OAuth Token**（以 `xoxb-` 开头，填入 AskHuman 设置页）。

## 四、订阅事件（Event Subscriptions）

1. 左侧 **Features → Event Subscriptions** → 打开 **Enable Events**。
   > Socket Mode 下无需填写 Request URL，事件直接经长连接下发。
2. 展开 **Subscribe to bot events** → **Add Bot User Event** → 添加 **`message.im`**：用于接收用户在单聊中发送的文字 / 图片 / 文件。
3. 保存。若提示需重新安装应用，按提示重装。

## 五、开启交互（Interactivity）

1. 左侧 **Features → Interactivity & Shortcuts** → 打开 **Interactivity** 开关。
   > Socket Mode 下同样无需填写 Request URL；用户点卡片「提交」按钮产生的 `block_actions` 交互经长连接回传。
2. 保存。

## 六、启用 App Home 私聊（必做）

Slack 自 2021 年起默认**禁止用户主动给机器人发私聊**（改为按需开启）。不开这一步，你在与机器人的 DM 里会看到「Sending messages to this app has been turned off」、输入框置灰发不出消息——「自动识别」的 4 位码、作答期间回传图片 / 文件都会被挡住。

1. 左侧 **Features → App Home** → 找到 **Show Tabs**。
2. 打开 **Messages Tab** 开关。
3. 勾选其下的 **Allow users to send Slash commands and messages from the messages tab**。
4. 保存，回 Slack 客户端刷新（必要时重新安装应用）；之后 DM 输入框即可正常发送。

## 七、在 AskHuman 中填写

打开 AskHuman 设置页 → 「通信渠道」→「Slack」，开启开关后填写：

| 字段 | 说明 |
| --- | --- |
| Bot Token | Bot User OAuth Token（`xoxb-…`，用于所有 Web API 调用） |
| App-Level Token | App-Level Token（`xapp-…`，scope `connections:write`，用于建立 Socket Mode 长连接） |
| User ID | 接收 / 作答用户的 Slack User ID（`U…`，单聊）。可点「自动识别」：先校验两个 Token，再提示你用目标账号私聊机器人发送一个 4 位数字以精确回填 |

填好后点「测试连接」：会校验 Bot Token（`auth.test` 并向该 User 的单聊发一条测试消息）与 App Token（`apps.connections.open` 能拿到长连接地址），都通过即配置成功。

> 找不到 User ID 时，建议直接用「自动识别」。手动获取方式：在 Slack 中点开目标用户头像 → 资料 → 更多（···）→ Copy member ID。

## 八、交互与回退行为

- 提问以 **Block Kit 消息内表单**逐题发送：消息内放复选框（多选预定义选项）+ 多行输入框（补充文字）+「提交」按钮，点「提交」完成该题（交互经 Socket Mode 回传；底层收帧即应答，满足 Slack 3 秒确认要求，已自动处理）。
- 提交 / 被抢答收尾时，卡片用 `chat.update` 替换为**静态终态**：保留题目，回显已选项（✓）与补充文字（💬），并加一行状态（「已提交」/「已在 X 回答」/「已取消」），移除所有控件。
- 作答期间在 DM 里发送的**图片 / 文件**会被累积进该题答案；**纯文字会被忽略**（请用卡片输入框补充文字）。
- 若卡片投放失败，会自动**回退**为「纯文本 + 编号选项」：回复编号（多选用逗号，如 `1,3`）/ 直接输入文字 / 发送图片 / 文件均可作答。
- 多渠道同启时以「整个会话」为粒度**抢答**：哪端先答完全部题即采用该端结果，其余自动收尾（Slack 卡片会被更新成「已在 X 回答」终态）。

## 九、常见问题

| 现象 | 可能原因 |
| --- | --- |
| DM 输入框置灰 / 提示「Sending messages to this app has been turned off」 | 未启用 App Home 的 Messages Tab 私聊（见第六步），改后刷新或重装应用 |
| 测试连接报 `invalid_auth` / `not_authed` | Bot Token 错误，或改了 scope 后未重新 Install to Workspace |
| 测试连接报 App Token 相关错误 | App-Level Token 错误，或未给它勾选 `connections:write` scope |
| 收不到用户消息 / 自动识别一直等待 | 未订阅 `message.im` 事件，或未开启 Socket Mode，或未启用 App Home 私聊（第六步）；改动后需重新安装应用 |
| 点卡片「提交」无反应 | 未开启 **Interactivity** 开关 |
| 发送 / 下载文件报权限错误 | 缺少 `files:write` / `files:read` scope（重新安装后生效） |
| 发消息报 `channel_not_found` | `im:write` 未授予，无法打开 DM 频道 |
| 想确认长连接事件是否到达 | 设环境变量 `ASKHUMAN_SLACK_DEBUG=1` 运行，查看 `~/.askhuman/slack-debug.log` 中的帧日志 |
