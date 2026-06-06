# Telegram channel setup

[简体中文](./telegram-setup.md) | English

Send and receive questions through a Telegram bot — no public callback needed (uses long polling). Once configured, questions arrive in your direct chat as messages with inline buttons; you can tap options or reply with text.

## 1. Create a bot

1. In Telegram, chat with [@BotFather](https://t.me/BotFather), send `/newbot`, and follow the prompts to set a name and username.
2. Note the returned **Bot Token** (looks like `123456:ABC-DEF...`).

## 2. Get your Chat ID

1. First **send any message to your bot** in Telegram (otherwise the bot can't initiate contact with you).
2. Find your numeric **Chat ID**: chat with [@userinfobot](https://t.me/userinfobot), or open `https://api.telegram.org/bot<Token>/getUpdates` and read `chat.id` from the response.

## 3. Fill in AskHuman

Open Settings → Channels → Telegram, enable it, and fill in:

| Field | Notes |
| --- | --- |
| Bot Token | The token returned by BotFather |
| Chat ID | Your numeric Chat ID (direct chat) |
| API Base URL | Defaults to `https://api.telegram.org`; change it if you run a reverse proxy |

Click "Test connection" — receiving the test message in Telegram means you're set.

## 4. Behavior and limits

- Questions are delivered as messages; predefined options become inline buttons. You can tap buttons or reply with text, then tap "Send" to submit.
- **Images are not received** (for human → AI image upload, use the DingTalk / Feishu channels).
- The source name "Question from {name}" is customizable via the `ASKHUMAN_ENV_SOURCE_NAME` environment variable (see [Configuration](./configuration.en.md)).
- Message attachments (`-f`) are sent alongside the Message: images via `sendPhoto`, others via `sendDocument`.
