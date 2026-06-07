# Configuration

[简体中文](./configuration.md) | English

This page covers AskHuman's config file, settings UI, and environment variables. For per-channel onboarding, see the dedicated guides: [Telegram](./telegram-setup.en.md) · [DingTalk](./dingtalk-setup.en.md) · [Feishu / Lark](./feishu-setup.en.md).

## Settings UI

Run `AskHuman --settings` (or click the gear in the popup's top-right) to open settings. Three tabs:

- **General** — theme (system / light / dark), always-on-top, appear animation, glass effect, speech-input language and shortcut, reply-history retention limit (offers "Clean up now" when it's below the existing count).
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
    "speechShortcut": "cmd+d",  // empty string disables it
    "historyLimit": 200         // global reply-history retention; 0 = stop recording but keep old entries
  },
  "channels": {
    "popup":    { "enabled": true, "width": 560, "height": 620, "rememberSize": true },
    "telegram": { "enabled": false, "botToken": "", "chatId": "", "apiBaseUrl": "https://api.telegram.org" },
    "dingding": { "enabled": false, "clientId": "", "clientSecret": "", "userId": "", "cardTemplateId": "" },
    "feishu":   { "enabled": false, "appId": "", "appSecret": "", "openId": "", "baseUrl": "https://open.feishu.cn" }
  }
}
```

## Reply history

Every reply (a "send" completed in the popup or any IM channel, plus a cancel you trigger yourself) is recorded to `~/.askhuman/history.jsonl` (one JSON entry per line, storing only local paths of images / files). System-triggered cancellations (timeout, disconnect, daemon stop) are not recorded.

- Open it with `AskHuman --history` (current project only by default), or click the "History" button in the popup's top-right. Add `--all` to view every project; the window also has a top dropdown to switch projects.
- Project identification: walk up from the command's working directory to the first `.git` repository root; if there's no `.git`, the working directory is used.
- Retention: controlled by `general.historyLimit` (default 200). Setting it to `0` stops recording new entries, but existing entries remain viewable. Lowering it below the existing count in settings shows a notice with a "Clean up now" button to trim to the new limit.
- Clear: the "Clear" menu in the history window can clear the "current project" or "all projects".

## Environment variables

| Variable | Purpose | Legacy alias |
| --- | --- | --- |
| `ASKHUMAN_ENV_SOURCE_NAME` | Custom "source name": the popup title and channel message headers change from the default `the Loop` to your value (e.g. `Question from Agent`) | — |
| `ASKHUMAN_BINARY` | Absolute path to a binary that program integrations (the npm package) should prefer, handy for custom / test builds | `HUMANINLOOP_BINARY` |
| `ASKHUMAN_FEISHU_DEBUG` | When set to a non-empty value other than `0`, writes Feishu long-connection diagnostics to `~/.askhuman/feishu-debug.log` | `HUMANINLOOP_FEISHU_DEBUG` |

> The legacy variable names in parentheses are still recognized for a smooth migration.
