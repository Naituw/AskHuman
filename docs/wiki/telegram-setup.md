# Telegram 渠道配置

简体中文 | [English](./telegram-setup.en.md)

通过 Telegram Bot 收发提问，无需公网回调（采用长轮询）。配置完成后，提问会以带 inline 按钮的消息发到你的单聊，你可点选项或直接回文字作答。

## 一、创建 Bot

1. 在 Telegram 里与 [@BotFather](https://t.me/BotFather) 对话，发送 `/newbot`，按提示设置名称与用户名。
2. 记下返回的 **Bot Token**（形如 `123456:ABC-DEF...`）。

> 头像：可向 BotFather 发送 `/setuserpic` 设置 Bot 头像。仓库内置两张可作头像的图片（深色 / 浅色背景），也可用于其它场景：[深色](../../assets/avatars/bot-avatar-dark.jpg) · [浅色](../../assets/avatars/bot-avatar-light.jpg)。

## 二、获取 Chat ID

1. 先在 Telegram 中**主动给你的 Bot 发一条任意消息**（否则 Bot 无法主动联系你）。
2. 获取你的数字 **Chat ID**：可与 [@userinfobot](https://t.me/userinfobot) 对话获取，或访问 `https://api.telegram.org/bot<Token>/getUpdates` 在返回里查看 `chat.id`。

## 三、在 AskHuman 中填写

打开设置页 → 「通信渠道」→「Telegram」，开启开关后填写：

| 字段 | 说明 |
| --- | --- |
| Bot Token | BotFather 返回的令牌 |
| Chat ID | 你的数字 Chat ID（单聊） |
| API Base URL | 默认 `https://api.telegram.org`；如需自建反代可改 |

填好后点「测试连接」，Telegram 内收到测试消息即配置成功。

## 四、交互与限制

- 提问以消息下发，预定义选项为 inline 按钮；可点按钮、也可直接回复文字，再点「发送」完成作答。
- **不接收图片**（人 → AI 的图片回传请使用钉钉 / 飞书渠道）。
- 来源名「Question from {名称}」由环境变量 `ASKHUMAN_ENV_SOURCE_NAME` 定制（见 [配置](./configuration.md)）。
- Message 的附件（`-f`）会随 Message 一并发送：图片用 `sendPhoto`，其它用 `sendDocument`。
