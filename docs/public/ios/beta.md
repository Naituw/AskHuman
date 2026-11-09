---
title: AskHuman Mobile Friends Beta
lang: en
---
# AskHuman Mobile Friends Beta

[简体中文](../beta-zh/) · [Privacy policy](../privacy/)

Answer questions from agents on your own Mac using your iPhone. Follow these five steps.

**Before you start:** have your Mac, an iPhone running iOS 17 or later, and the **TestFlight email invitation and push relay URL** supplied by the maintainer.

## 1. Install AskHuman on your Mac

If Node.js is not installed, install [Node.js LTS](https://nodejs.org/en/download) first. Then run in Terminal:

```bash
npm install -g askhuman@latest
AskHuman setup
```

Connect the agents you use in the wizard; you can skip other messaging tools. Restart any open agent sessions afterward to load the integration.

## 2. Create your own EMQX deployment

1. Register and sign in to the [EMQX China console](https://cloud.emqx.com/console/). Create a project named `AskHuman`. These steps use that console.
2. In the project, create a **Serverless** deployment named `askhuman`. Set the **spending limit to `0`**, leave other options at their defaults, and wait until it is running.
3. Open the deployment's **Access Control → Authentication**. Add username `ah01`, choose a password and save it.
4. In **Access Control → Authorization**, add these rules:
   - **Username** tab: username `ah01`, topic `ah/ah01/#`, action **Publish & Subscribe**, permission **Allow**.
   - **All Users** tab: topic `#`, action **Publish & Subscribe**, permission **Deny**. Remove any existing rule allowing `#`.
5. Return to **Overview** and copy the hostname under **Connection Information** for the next step.

A spending limit of `0` uses only the free allowance and stops the deployment when it is exhausted. See [EMQX billing](https://docs.emqx.com/en/cloud/latest/price/pricing.html).

## 3. Enter the Mac connection settings

Run `AskHuman --settings` to open Settings:

1. In **General**, enable experimental features.
2. In **Experimental → iOS App channel**, enable the channel and enter:

   | Field | Value |
   |---|---|
   | Broker host | The EMQX hostname, without a scheme, port or path |
   | TLS port | `8883` |
   | WSS port | `8084` |
   | Username | `ah01` |
   | Password | The client password you just created |
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
