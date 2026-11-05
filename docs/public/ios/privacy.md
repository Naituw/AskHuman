---
title: AskHuman Mobile Privacy Policy
lang: en
---
# AskHuman Mobile Privacy Policy

Effective date: November 5, 2026

[简体中文](../privacy-zh/) · [Beta setup guide](../beta/)

AskHuman Mobile is a companion to AskHuman on your own Mac. This policy describes the iOS app and the push relay provided by the AskHuman project maintainer for the invitation-only beta. Your chosen MQTT provider, Apple, your agent provider and any service you operate have their own privacy terms.

## What stays on your devices

The app processes questions, answers, confirmations, agent information, task instructions, and files you choose to send. These may contain personal or confidential information. Paired devices exchange this content with end-to-end encryption; the MQTT broker and our push relay do not hold the pairing keys needed to decrypt it.

Your iPhone stores pairing credentials and keys in the system Keychain. It also stores request caches, drafts, outgoing messages, local answer history, attachments and usage counters. Local history is pruned when entries are added, keeping at most 500 entries from the preceding 90 days; this is not a timed deletion guarantee for every cached file. iOS protects application files according to its device data-protection facilities. Your Mac separately keeps its own AskHuman records.

The mobile todo view reads and changes todos on a paired Mac. Optional iCloud synchronization is performed by the Mac; the iOS app does not directly access the project's CloudKit container. Starting or messaging an agent causes your own Mac and agent provider to process your instructions and attachments under their respective settings and terms.

## What services receive

- **Your MQTT broker:** encrypted messages and attachments, the broker username, connection identifiers, topic names, traffic and connection timing, and network information needed to accept a connection. You choose and administer this service.
- **Our push relay:** your Apple push device token, application identifier, push environment, an opaque device handle and encrypted notification content. The service turns these into Apple push requests. Handles contain encrypted routing information and expire after 180 days; the app can register again. The relay does not maintain a user-account or message-content database.
- **Apple Push Notification service:** routing information and encrypted notification content. The notification extension decrypts a preview on the receiving device; previews may be visible on the lock screen according to your iOS settings.
- **Infrastructure providers:** hosting and network providers can process IP addresses, request timing and operational metadata. The relay application's own log records request method, status and duration, not message bodies, tokens or handles. Provider access logs and retention depend on the hosting configuration; end-to-end encryption does not hide all metadata.
- **TestFlight and feedback:** Apple automatically collects beta crash logs and usage information and shares them with the developer; feedback you submit is also shared, as described in [TestFlight & Privacy](https://www.apple.com/legal/privacy/data/en/test-flight/). This can include your invitation email and test device/version information. If you email us or submit a screenshot or issue, we receive what you send and use it to investigate and respond.

AskHuman Mobile does not contain advertising or an added third-party analytics SDK. We do not sell your questions, answers or attachments. End-to-end encryption does not protect content once displayed on an unlocked device or sent to an agent/provider you have chosen.

## Your choices and deletion

You can decline notification or camera access, change permissions in iOS Settings, and choose which files to attach. Camera access is used for pairing QR codes or photos you explicitly take. Device authentication protects starting a new agent task where the app requests it.

You can delete answer-history entries in the app, unpair a Mac, or use “Forget all pairings” in the Macs tab. Revoke the phone on the Mac as well, especially if either device is offline. These actions do not erase records already kept by your Mac, agent provider, MQTT provider or Apple. Deleting a history entry is not a promise to remove every attachment cache. Removing the app removes its application-container data according to iOS behavior, but Keychain pairing information can survive reinstallation; forget pairings before uninstalling if you do not want to restore them.

Stop using our relay by changing or removing its address in your Mac's iOS channel settings and unpairing the phone. A stateless routing handle is not a centrally managed user account. Contact us for help with feedback you have sent us; we may need enough information to identify it. Do not include passwords, pairing codes, private keys or confidential agent transcripts in public issues.

## Contact and changes

Contact the AskHuman project maintainer through the feedback email displayed in your TestFlight invitation, or [the project's issue tracker](https://github.com/Naituw/AskHuman/issues). Public issues are visible to others. We will update this page when the app's data practices change and show the effective date above.
