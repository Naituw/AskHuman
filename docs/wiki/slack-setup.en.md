# Slack channel setup

[简体中文](./slack-setup.md) | English

This guide explains how to create and configure a Slack App so AskHuman's "Slack channel" works. The channel uses a **Slack App + Socket Mode long connection (WebSocket) + bot + direct message (DM)** model, and needs **no public network** to send/receive messages and card interactions.

## 1. Create the app

1. Open [Slack API → Your Apps](https://api.slack.com/apps) → **Create New App** → **From scratch**.
2. Enter an app name, pick the target workspace → **Create App**.

> Avatar: set the app icon under **Settings → Basic Information → Display Information**. The repo ships two images you can use as the avatar (dark / light background), or for other uses: [dark](../../assets/avatars/bot-avatar-dark.jpg) · [light](../../assets/avatars/bot-avatar-light.jpg).

## 2. Enable Socket Mode and get the App-Level Token

1. In the sidebar, **Settings → Socket Mode** → turn on **Enable Socket Mode**.
2. You'll be prompted to create an **App-Level Token**: name it anything, add the scope **`connections:write`** → **Generate**.
3. Record the generated **App-Level Token** (starts with `xapp-`; enter it in AskHuman settings).

## 3. Configure Bot Token scopes

1. In the sidebar, **Features → OAuth & Permissions** → under **Scopes → Bot Token Scopes**, **Add an OAuth Scope** and add:

| Scope | Purpose |
| --- | --- |
| `chat:write` | Send / update messages and interactive cards |
| `im:write` | Open a DM channel with the target user |
| `im:history` | Receive message events the user sends in the DM |
| `files:read` | Download images / files the user sends in the DM (human → AI) |
| `files:write` | Upload `-f` attachments (AI → human file sending) |

2. Scroll up to **OAuth Tokens → Install to Workspace** (re-install after any scope change).
3. After installing, record the **Bot User OAuth Token** (starts with `xoxb-`; enter it in AskHuman settings).

## 4. Subscribe to events

1. In the sidebar, **Features → Event Subscriptions** → turn on **Enable Events**.
   > Under Socket Mode no Request URL is needed; events arrive over the long connection.
2. Expand **Subscribe to bot events** → **Add Bot User Event** → add **`message.im`**: used to receive text / images / files the user sends in the DM.
3. Save. If prompted to reinstall the app, do so.

## 5. Enable interactivity

1. In the sidebar, **Features → Interactivity & Shortcuts** → turn on **Interactivity**.
   > Under Socket Mode no Request URL is needed either; the `block_actions` interaction from tapping the card's "Submit" button is delivered over the long connection.
2. Save.

## 6. Enable App Home messaging (required)

Since 2021 Slack **disables users from messaging a bot by default** (it's now opt-in). Skip this and, in the DM with the bot, you'll see "Sending messages to this app has been turned off" with a greyed-out input box — which blocks the "Auto-detect" 4-digit code and any images / files you try to send back while answering.

1. In the sidebar, **Features → App Home** → find **Show Tabs**.
2. Turn on the **Messages Tab** toggle.
3. Check **Allow users to send Slash commands and messages from the messages tab**.
4. Save, then refresh your Slack client (reinstall the app if needed); the DM input box will then work.

## 7. Fill in AskHuman

Open AskHuman settings → "Channels" → "Slack", enable the toggle, then fill in:

| Field | Description |
| --- | --- |
| Bot Token | Bot User OAuth Token (`xoxb-…`, used for all Web API calls) |
| App-Level Token | App-Level Token (`xapp-…`, scope `connections:write`, used to open the Socket Mode connection) |
| User ID | The recipient/answering user's Slack User ID (`U…`, DM). Click "Auto-detect": it validates both tokens, then asks you to DM the bot a 4-digit code from the target account to fill it in precisely |

Then click "Test connection": it validates the Bot Token (`auth.test` plus sending a test DM to that user) and the App Token (`apps.connections.open` returns a connection URL). If both pass, you're set.

> If you can't find the User ID, use "Auto-detect". Manually: in Slack, open the target user's profile → More (···) → Copy member ID.

## 8. Interaction & fallback behavior

- Questions are sent per-question as a **Block Kit in-message form**: checkboxes (multi-select predefined options) + a multiline input (free text) + a "Submit" button. Tapping "Submit" completes the question (the interaction comes over Socket Mode; the transport acks each frame on receipt to satisfy Slack's 3-second requirement, handled automatically).
- On submit / when preempted, the card is replaced via `chat.update` with a **static terminal state**: the question is kept, selected options (✓) and the note (💬) are echoed back, and a status line is added ("Submitted" / "Answered via X" / "Cancelled"), with all controls removed.
- Images / files sent in the DM during answering are accumulated into that question's answer; **plain text is ignored** (use the card's input field for free text).
- If the card fails to deliver, it automatically **falls back** to "plain text + numbered options": reply with option numbers (comma-separated for multiple, e.g. `1,3`), type free text, or send images / files.
- With multiple channels enabled, the whole conversation is a single race: whichever side finishes answering all questions first wins, and the others finalize automatically (the Slack card updates to an "Answered via X" terminal state).

## 9. Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| DM input box is greyed out / "Sending messages to this app has been turned off" | The App Home Messages Tab isn't enabled for user messaging (see step 6); refresh or reinstall the app afterwards |
| Test connection reports `invalid_auth` / `not_authed` | Wrong Bot Token, or you changed scopes without re-installing to the workspace |
| Test connection reports an App Token error | Wrong App-Level Token, or it lacks the `connections:write` scope |
| No incoming user messages / auto-detect keeps waiting | The `message.im` event isn't subscribed, or Socket Mode isn't enabled, or App Home messaging (step 6) isn't enabled; reinstall the app after changes |
| Tapping the card's "Submit" does nothing | The **Interactivity** toggle is off |
| File send / download permission errors | Missing `files:write` / `files:read` scope (effective after reinstall) |
| Sending reports `channel_not_found` | `im:write` not granted, so the DM channel can't be opened |
| Want to confirm long-connection events arrive | Run with `ASKHUMAN_SLACK_DEBUG=1` and inspect the frame logs in `~/.askhuman/slack-debug.log` |
