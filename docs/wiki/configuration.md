# 配置

简体中文 | [English](./configuration.en.md)

本文说明 AskHuman 的配置文件、设置界面与环境变量。各通信渠道的接入步骤见独立文档：[Telegram](./telegram-setup.md) · [钉钉](./dingtalk-setup.md) · [飞书 / Lark](./feishu-setup.md)。

## 设置界面

运行 `AskHuman --settings`（或在弹窗右上角点齿轮）打开设置，含三个 Tab：

- **通用**：主题（跟随系统 / 浅色 / 深色）、窗口置顶、出现动画、毛玻璃效果、语音输入语言与快捷键。
- **集成**：可复制的参考提示词、Cursor Hook 的安装 / 移除（仅 macOS / Linux，详见 [README](../../README.md) 的「与 AI Agent 搭配」一节）。
- **通信渠道**：本地弹窗、Telegram、钉钉、飞书的开关与参数，每个渠道都带「测试连接」。

## 配置文件

配置保存在 `~/.askhuman/config.json`，由设置界面读写（原子写入、容错解码：缺失字段走默认值、未知字段忽略）。

> 向后兼容：若 `~/.askhuman/config.json` 不存在但旧版 `~/.humaninloop/config.json` 存在，会自动读取旧文件。

结构概览：

```jsonc
{
  "general": {
    "theme": "system",          // system | light | dark
    "language": "auto",         // auto | en | zh
    "alwaysOnTop": true,
    "appearAnimation": "alert", // none | document | alert
    "windowEffect": "glass",    // glass | blur
    "speechLanguage": "auto",   // BCP-47，如 zh-CN / en-US
    "speechShortcut": "cmd+d"   // 空串表示关闭
  },
  "channels": {
    "popup":    { "enabled": true, "width": 560, "height": 620, "rememberSize": true },
    "telegram": { "enabled": false, "botToken": "", "chatId": "", "apiBaseUrl": "https://api.telegram.org" },
    "dingding": { "enabled": false, "clientId": "", "clientSecret": "", "userId": "", "cardTemplateId": "" },
    "feishu":   { "enabled": false, "appId": "", "appSecret": "", "openId": "", "baseUrl": "https://open.feishu.cn" }
  }
}
```

## 环境变量

| 变量 | 作用 | 兼容旧名 |
| --- | --- | --- |
| `ASKHUMAN_ENV_SOURCE_NAME` | 自定义「来源名」：弹窗标题与各渠道消息头由默认的 `the Loop` 改为指定名称（如 `Question from Agent`） | — |
| `ASKHUMAN_BINARY` | 程序集成（npm 包）时优先使用的二进制绝对路径，便于自定义 / 测试 | `HUMANINLOOP_BINARY` |
| `ASKHUMAN_FEISHU_DEBUG` | 设为非空且非 `0` 时，写飞书长连接诊断日志到 `~/.askhuman/feishu-debug.log` | `HUMANINLOOP_FEISHU_DEBUG` |

> 括号中的旧变量名仍被识别，方便平滑迁移。
