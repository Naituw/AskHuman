# Configuration

[简体中文](./configuration.md) | English

This page covers AskHuman's config file, settings UI, and environment variables. For per-channel onboarding, see the dedicated guides: [Telegram](./telegram-setup.en.md) · [DingTalk](./dingtalk-setup.en.md) · [Feishu / Lark](./feishu-setup.en.md).

## Settings UI

Run `AskHuman --settings` (or click the gear in the popup's top-right) to open settings. Three tabs:

- **General** — theme (system / light / dark), always-on-top, appear animation, glass effect, speech-input language and shortcut.
- **Integrations** — copyable reference prompt, and Cursor Hook install / remove (macOS / Linux only; see the "Pairing with an AI Agent" section in the [README](../../README.en.md)).
- **Channels** — toggles and parameters for the local popup, Telegram, DingTalk, and Feishu. Each channel has a "Test connection" button.

## Config file

Configuration is stored at `~/.askhuman/config.json`, read and written by the settings UI (atomic writes, tolerant decoding: missing fields fall back to defaults, unknown fields are ignored).

> Backward compatibility: if `~/.askhuman/config.json` does not exist but a legacy `~/.humaninloop/config.json` does, the legacy file is read automatically.

Shape overview:

```jsonc
{
  "general": {
    "theme": "system",          // system | light | dark
    "language": "auto",         // auto | en | zh
    "alwaysOnTop": true,
    "appearAnimation": "alert", // none | document | alert
    "windowEffect": "glass",    // glass | blur
    "speechLanguage": "auto",   // BCP-47, e.g. zh-CN / en-US
    "speechShortcut": "cmd+d"   // empty string disables it
  },
  "channels": {
    "popup":    { "enabled": true, "width": 560, "height": 620, "rememberSize": true },
    "telegram": { "enabled": false, "botToken": "", "chatId": "", "apiBaseUrl": "https://api.telegram.org" },
    "dingding": { "enabled": false, "clientId": "", "clientSecret": "", "userId": "", "cardTemplateId": "" },
    "feishu":   { "enabled": false, "appId": "", "appSecret": "", "openId": "", "baseUrl": "https://open.feishu.cn" }
  }
}
```

## Environment variables

| Variable | Purpose | Legacy alias |
| --- | --- | --- |
| `ASKHUMAN_ENV_SOURCE_NAME` | Custom "source name": the popup title and channel message headers change from the default `the Loop` to your value (e.g. `Question from Agent`) | — |
| `ASKHUMAN_BINARY` | Absolute path to a binary that program integrations (the npm package) should prefer, handy for custom / test builds | `HUMANINLOOP_BINARY` |
| `ASKHUMAN_FEISHU_DEBUG` | When set to a non-empty value other than `0`, writes Feishu long-connection diagnostics to `~/.askhuman/feishu-debug.log` | `HUMANINLOOP_FEISHU_DEBUG` |

> The legacy variable names in parentheses are still recognized for a smooth migration.
