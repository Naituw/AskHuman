# DingTalk channel setup

[简体中文](./dingtalk-setup.md) | English

Send and receive questions through DingTalk using an "internal enterprise app + bot + Stream mode (WebSocket) + direct chat" — **no public endpoint, domain, or certificate required**. Questions are delivered as interactive cards one at a time; you check options, add text, and submit to answer.

> Prerequisite: you have developer access in your DingTalk organization and can create an internal app.

## 1. Create an internal app and enable the bot

1. Go to the [DingTalk Open Platform](https://open-dev.dingtalk.com) and create an **internal enterprise app**.
2. Record the credentials **ClientId (AppKey)** and **ClientSecret (AppSecret)**.
3. Under "App capabilities", add and enable the **Bot**. The bot's `robotCode` equals the app's AppKey, so no separate configuration is needed.
4. In "Permissions", grant the API scopes needed for sending messages, interactive cards, and media files.
5. Set the bot's message-receiving mode to **Stream mode** (pushed over a local long connection, no public callback).
6. Create and publish a version, and make sure the bot is available to your target user.

> Avatar: the repo ships two images you can use as the bot avatar (dark / light background), or for other uses: [dark](../../assets/avatars/bot-avatar-dark.jpg) · [light](../../assets/avatars/bot-avatar-light.jpg).

## 2. (Optional) Custom card template

AskHuman ships with a **built-in advanced interactive-card template** that works out of the box — no setup required. To customize the card, build and publish an **advanced** interactive-card template (same app) on the DingTalk card platform, then enter its template ID in the settings "Card template ID" field. Leave it blank to use the built-in default.

## 3. Fill in AskHuman

Open Settings → Channels → DingTalk, enable it, and fill in:

| Field | Notes |
| --- | --- |
| ClientId | App AppKey |
| ClientSecret | App AppSecret |
| UserId | The userId of the receiving / answering user (direct chat). Click "Auto-detect": it first validates ClientId/ClientSecret, then asks you to DM the bot a 4-digit code to accurately fill it in |
| Card template ID | Blank uses the built-in default; fill in to use your own advanced card template |

Click "Test connection": it exchanges a token and sends a test message to that userId's direct chat. Receiving it in DingTalk means you're set.

## 4. Behavior and fallback

- Questions are delivered as **advanced interactive cards**, one per question: check predefined options (multi-select), optionally add text, then tap "Submit" to finish that question (callbacks go over Stream, no public endpoint).
- **Images / files** sent in the chat while answering are accumulated into that question's answer (images into `[Images]`, files into `[Files]`); use the card's text field for plain text.
- If card delivery fails, it automatically **falls back** to "plain text + numbered options": reply with numbers (comma-separated for multi-select, e.g. `1,3`), type text, or send images / files to answer.
- Message attachments (`-f`) are uploaded as media and sent alongside the Message.
- With multiple channels enabled, racing happens at the granularity of the whole session: whichever side finishes all questions first wins, and the others wrap up.

## 5. Known limitation

Only one Stream connection is allowed per ClientId at a time. This is fine for the normal "one question at a time" flow; with **rapid / concurrent** questions, multiple Streams may compete for messages and deliver a reply to the wrong connection. Avoid launching concurrent questions against the same app.
