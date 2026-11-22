---
title: AskHuman Mobile Beta
lang: en
---
# AskHuman Mobile Beta

[简体中文](../beta-zh/) · [Privacy policy](../privacy/)

Answer questions from agents on your own Mac using your iPhone. Follow these five steps; the optional self-hosted route is at the end.

**Before you start:** have your Mac and an iPhone running iOS 17 or later.

## 1. Install AskHuman on your Mac

If Node.js is not installed, install [Node.js LTS](https://nodejs.org/en/download) first. Then run in Terminal:

```bash
npm install -g askhuman@latest
AskHuman setup
```

Connect the agents you use in the wizard; you can skip other messaging tools. Restart any open agent sessions afterward to load the integration.

## 2. Connect a broker

The broker is the relay station between your Mac and your phone. You bring it, and AskHuman is not tied to any one service.

Run `AskHuman --settings`, open **Channels → iOS App** and click **Start setup**. Step 1 of the card covers EMQX Cloud Serverless (hosted in China), flespi (hosted in Europe) and self-hosted brokers such as Mosquitto, with the setup steps for each. The same facts, with each service's free allowance and drawbacks, are in **[Brokers: requirements, options and verification](https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.en.md)**.

Enter the host, username and password, then click **Test connection**. The account must only be able to read and write `ah/<username>/#`, so it cannot see anyone else's topics. Optionally turn on **Keep this Mac reachable**, so the phone never shows this Mac as offline; keep the Mac online and awake while you use it.

Prefer to let your agent do it? Click **Set up with an Agent** on the card and send it the text it copies.

## 3. Install the iPhone app

Step 2 of the card shows a QR code for [the join page](https://naituw.github.io/AskHuman/). On your iPhone, install [TestFlight](https://apps.apple.com/app/testflight/id899247664), open that page, join the beta, install AskHuman and allow notifications.

## 4. Pair the phone

1. In step 3 of the card, click **Pair a new phone**. On the phone, open **Macs → Add Mac** and scan the QR code.
2. Compare the **six-digit codes** and click **Allow** on the Mac. QR codes expire after five minutes; generate a new one if needed.

## 5. Test answers and notifications

When setup is complete, the card shows **Ready**. Click **Send test question** and answer it on your phone; this tests answering without making a model call.

To test the notification, send it again while AskHuman is closed on the phone. A test question is pushed at once; other questions are pushed after your Mac has been idle for 15 minutes by default (**Push timing** on the card). Inbox delivery never waits.

If something fails, send the maintainer a screenshot and both app versions. Hide passwords, connection strings and pairing codes.

## Optional: host your own push relay

Choose this route if you have **an Apple Developer account and Xcode** and want to use your own service:

1. Build and install the iOS app with your own team and bundle IDs, and create a matching APNs key.
2. Deploy the repository's `relay/` to your Tencent Cloud function, configure that key and check `/v1/health`.
3. On the card, open **More settings**, choose **Use my own relay**, enter your relay URL, test the connection and pair again with your own signed app.

See the **[complete self-hosting guide](https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-setup.en.md)** for commands and settings.
This route uses your own signed iOS app. The TestFlight route above uses the maintainer's app and the public relay.
