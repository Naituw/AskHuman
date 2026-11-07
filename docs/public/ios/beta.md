---
title: AskHuman Mobile Friends Beta
lang: en
---
# AskHuman Mobile Friends Beta

[简体中文](../beta-zh/) · [Privacy policy](../privacy/)

Answer questions from AI agents running on your Mac, handle confirmations, inspect sessions, and send instructions from your iPhone.

This is an invitation-only external TestFlight beta. You use your own Mac, agents and EMQX deployment. The maintainer provides the push relay. The service is experimental and may be interrupted for maintenance.

## Requirements

- An iPhone running iOS 17 or later, with Apple TestFlight installed.
- Your own Mac with the AskHuman version specified in your invitation and a working agent integration.
- Your own EMQX Serverless deployment. Keep its monthly spending limit at zero; exhausted allowances can stop service.
- The push relay URL supplied with your invitation. You do not receive an APNs private key.

You do not need Xcode or a paid Apple Developer account. Use the maintainer's TestFlight app without changing its bundle identifier. Agent operations use your own model account and allowance.

## Set up your broker

Create a Serverless deployment and use the TLS/WSS hostnames and ports shown in its console. Create an independent password and a username containing only letters and digits. Allow that user to publish and subscribe to ah/username/#, with a deny-all fallback for #. Verify the allowance and restart rules in your provider's console.

The [repository setup guide](https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-setup.en.md) has detailed broker instructions. For this beta, skip its Apple developer-resource and self-hosted push-relay sections.

## Connect your Mac and phone

1. In AskHuman on your Mac, enable experimental features in General settings.
2. Open Experimental → iOS App channel. Enable it, enter your own broker credentials and the relay URL from your invitation, and test the connection.
3. Accept the TestFlight email invitation on your iPhone, install AskHuman, and allow notifications.
4. On the Mac, choose to pair a new phone. On the phone's Macs tab, add a Mac and scan the QR code.
5. Pairing codes expire in five minutes and work once. Compare the six-digit confirmation codes on both devices and approve on your Mac.

You can import a connection string into another Mac you own. It includes your broker password; do not share it with other people or attach it to feedback.

## Check the basic workflow

Run this on your own Mac; it does not start an AI model request:

    AskHuman -q "Phone beta: please reply received"

Answer with the app in the foreground and check the terminal result. Temporarily set the Mac's push timing to “Immediately”, lock the phone, and send another question. Check notification delivery, opening the request and answering. Restore your preferred timing afterward.

The default push delay waits for 15 minutes without Mac keyboard or mouse activity. Inbox delivery does not wait for this delay. No system notification while you are working at the Mac can be expected behavior.

During normal use, observe Wi-Fi/cellular changes, Mac sleep and wake, and answering the same question on multiple surfaces. Only the first answer should take effect.

## Limits and feedback

The Mac must be online with the AskHuman daemon running. Use the daemon's keep-alive setting if you need continuous availability; it does not prevent computer sleep. Live observation works only while the app is in the foreground. Creating remote tasks requires the Mac's remote-task setting and a ready agent. Some features require the newer Mac version specified in your invitation.

Report issues through TestFlight or the [project issue tracker](https://github.com/Naituw/AskHuman/issues). Include the iOS build, Mac app and OS versions, phone model, time/time zone, network and proxy state, steps, expected result and actual result. Say whether opening or refreshing the app restores it. Redact screenshots. Never send broker passwords, connection strings, pairing codes, push tokens, private keys or confidential transcripts.
