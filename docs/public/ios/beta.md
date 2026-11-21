---
title: AskHuman Mobile Friends Beta
lang: en
---
# AskHuman Mobile Friends Beta

[简体中文](../beta-zh/) · [Privacy policy](../privacy/)

Answer questions from agents on your own Mac using your iPhone. Follow these five steps for TestFlight; the optional self-hosted route is at the end.

**Before you start (TestFlight):** have your Mac, an iPhone running iOS 17 or later, and the **TestFlight email invitation and push relay URL** supplied by the maintainer.

## 1. Install AskHuman on your Mac

If Node.js is not installed, install [Node.js LTS](https://nodejs.org/en/download) first. Then run in Terminal:

```bash
npm install -g askhuman@latest
AskHuman setup
```

Connect the agents you use in the wizard; you can skip other messaging tools. Restart any open agent sessions afterward to load the integration.

## 2. Prepare your own broker

The broker is the relay station between your Mac and your phone. You bring it, and AskHuman is not tied to any one service. Choose a path in **[Brokers: requirements, options and verification](https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.en.md)** and set it up: EMQX Cloud Serverless (hosted in China), flespi (hosted in Europe) or self-hosted Mosquitto. All three are tested, and each has step-by-step setup, its free allowance and its drawbacks.

Write down the **host name**, the TLS port (default `8883`), the WSS port (default `8084`; flespi uses `443`), the **username** and the **password**. The account must only be able to read and write `ah/<username>/#`, so it cannot see anyone else's topics.

## 3. Enter the Mac connection settings

Run `AskHuman --settings` to open Settings:

1. In **General**, enable experimental features.
2. In **Experimental → iOS App channel**, enable the channel and enter:

   | Field | Value |
   |---|---|
   | Broker host | The host name from the previous step, without a scheme, port or path |
   | TLS port | The TLS port you wrote down, usually `8883` |
   | WSS port | The WSS port you wrote down, usually `8084` (`443` for flespi) |
   | Username | The username from the previous step (the token ID for flespi) |
   | Password | The password from the previous step (the full token for flespi) |
   | Push relay URL | The complete URL supplied separately by the maintainer |

3. Click **Test connection**. Both the **broker round trip and relay health check must pass**.
4. In **Advanced → Daemon lifecycle**, choose **Keep alive**. Keep your Mac online and awake during use.

## 4. Install and pair the iPhone app

1. Install [TestFlight](https://apps.apple.com/app/testflight/id899247664) on your iPhone. Open your email invitation, install the latest AskHuman build and allow notifications.
2. In the Mac's iOS channel settings, choose **Pair new phone**. On the phone, open **Macs → Add Mac** and scan the QR code.
3. Compare the **six-digit codes** and approve on the Mac. QR codes expire after five minutes; generate a new one if needed.

## 5. Test answers and notifications

1. Keep AskHuman open on your phone and run this in your Mac terminal:

   ```bash
   AskHuman -q "Phone test: please reply received"
   ```

2. Answer on the phone and check that the terminal receives it. This tests answering without making a model call.
3. Temporarily set the Mac iOS channel's **Push timing** to **Immediately**. Lock your phone, run the command again, then open the notification and answer.

If both tests pass, setup is complete. Restore your preferred push timing afterward. By default, push waits until the Mac has been idle for 15 minutes; inbox delivery does not wait.

If something fails, send the maintainer a screenshot and both app versions. Hide passwords, connection strings and pairing codes.

## Optional: host your own push relay

Choose this route if you have **an Apple Developer account and Xcode** and want to use your own service:

1. Build and install the iOS app with your own team and bundle IDs, and create a matching APNs key.
2. Deploy the repository's `relay/` to your Tencent Cloud function, configure that key and check `/v1/health`.
3. Enter your relay URL on the Mac, test the connection and pair again with your own signed app.

See the **[complete self-hosting guide](https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-setup.en.md)** for commands and settings.
This route uses your own signed iOS app. The TestFlight route above uses the maintainer's app and relay.
