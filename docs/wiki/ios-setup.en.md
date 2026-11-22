# iOS app channel setup

[简体中文](./ios-setup.md) | English

Your Agent asks on the Mac, you answer on your iPhone. Several Macs can pair with one phone, and all their questions land in one inbox. For what you can do once it is set up, see [Using the iOS app](./ios-usage.en.md).

**What you need**:

- a Mac with AskHuman installed, and an iPhone running iOS 17 or later;
- an MQTT broker you bring yourself: the free tier of a hosted service or one you run, see [Brokers: requirements, options and verification](./ios-broker.en.md);
- no Apple developer account: pushes go through the public push relay by default.

In AskHuman Settings → **Channels** → **iOS App**, click "Start setup". The card walks you through three steps: connect a broker, install the iPhone app and pair a phone. You can also click "Set up with an Agent" and send the text it copies to your Agent. The steps below are the same.

## 1. Connect a broker

1. Pick one path in the [broker guide](./ios-broker.en.md) (EMQX Cloud Serverless, flespi or self-hosted Mosquitto) and set it up. When you are done you have:
   - the **host**, a TLS port (default `8883`) and a WSS port (default `8084`; it differs per service, flespi uses `443`);
   - a **username and password**: use letters and digits only for the username; it is also the topic namespace `ah/<username>/`;
   - **access control**: the account may only read and write `ah/<username>/#`. It is required when several people share one broker; the broker guide has the exact steps for each service.
2. In step 1 of the card pick your broker service, fill in the above and click **Test connection**. It connects to the broker and sends itself a message (reporting whether TLS or WSS was used and the round-trip time), asks the push relay for its health, and checks whether this account can read or write topics outside its own. If it can, you get a warning (it never blocks saving or pairing); limit the account to `ah/<username>/#` in the broker's access rules.

The same from the command line:

```bash
AskHuman channel set ios --enable --host <host> --username <username> \
  --password-stdin                              # the password is read from stdin
AskHuman channel test ios
```

"More settings" also holds the push relay (default "Use the public relay") and the name this Mac shows on the phone (empty uses the computer name). You rarely need to change either.

## 2. Install the iPhone app

Install [TestFlight](https://apps.apple.com/app/testflight/id899247664) on the iPhone, then open the [join page](https://naituw.github.io/AskHuman/) that the QR code or link in step 2 of the card points to: join the beta, install AskHuman and allow notifications on first launch.

## 3. Pair the phone

1. On the Mac: step 3 of the card, click **Pair a new phone** (command line: `AskHuman channel ios pair`). A QR code and a pairing code appear, **valid for 5 minutes and for one use**.
2. On the phone: the **Macs** tab in AskHuman → **Add a Mac** → **Scan QR code** (or "Paste the pairing code" when scanning is awkward).
3. The phone and the Mac each show a **6-digit check code**. Compare them and click **Allow** on the Mac. Pairing is done and the phone immediately receives the Mac's open requests.

To undo it: click **Revoke** next to the phone in the Mac settings (`AskHuman channel ios revoke <name>`); on the phone use "Unpair this Mac" in the Mac's details, or "Forget all pairings" at the bottom of the Macs tab. After reinstalling the app the phone asks whether to keep the pairings left in the keychain.

## 4. Send a test question

When setup is complete the card shows "Ready". Click **Send test question** and answer it on the phone; this tests answering without making a model call, and a test question is pushed at once. To test the notification, close AskHuman on the phone and send it again.

Other questions are pushed only after your Mac has been idle for 15 minutes by default; **Push timing** on the card changes it to 5 minutes or "Immediately". The phone's inbox never waits: a question shows up as soon as it is asked.

## Several Macs

A second Mac does not need the broker set up again. On the first one, open "Connection" on the card and click **Copy connection string** (it contains the broker password: pass it only between your own devices); on the second, pick a broker service in step 1, click "Import from another Mac…", paste it and click Import, then pair the phone with it as in step 3. On the command line: `AskHuman channel ios export` and `AskHuman channel ios import -`.

## Keeping the Mac visible on the phone

The iOS channel does not force the background service to stay up: it is running while your Agents work, and when it has stopped the phone shows that Mac as offline. To keep the Mac visible at all times turn on "Keep this Mac reachable" on the card (the same as setting the daemon lifecycle to keepalive in Settings → Advanced; turning on "Allow creating Agent tasks from phone or IM" keeps it on).

## How it works, and who can see what

```
Agent ─► AskHuman on the Mac ──MQTT (TLS)──► your broker ──► AskHuman on the iPhone
                 └── push relay (cloud function) ──► APNs ──► iPhone notification
```

- **End-to-end encrypted.** Every pairing creates a pair of keys only that Mac and that phone know. The broker and the push relay see ciphertext and metadata such as "who is online", never your questions or answers. The question summary on the lock screen is encrypted too: the notification extension on the phone decrypts it, and if that fails you only see a generic line.
- **The public push relay only forwards pushes and stores nothing.** It sees the device token and the app's bundle ID when the phone registers for pushes, the encrypted envelope and the handle when a push is sent, and the network address of each request (used only in memory to limit abuse, never written to a log; Tencent Cloud's own access logs are separate). The Apple push key lives only in the relay's cloud function; the Mac holds no Apple key. See the [privacy policy](https://naituw.github.io/AskHuman/privacy/).

## Troubleshooting

| Symptom | What to do |
|---|---|
| Settings say "Broker not connected" | Click "Test connection" for the reason. When the office network blocks 8883 it falls back to WSS 8084 by itself; an authentication failure is usually a wrong password or access control |
| Questions arrive while the app is open, but no pushes | Check the system notification permission; look for "push is no longer valid" under the phone in the Mac settings; open `https://askhuman-relay.maipo.me/v1/health`, which should return `"apns":true` |
| A question was asked long ago and the phone never notified you | The notification waits until the Mac's keyboard and mouse have been idle for the "Push timing" setting (15 minutes by default) and is never sent while you are using the Mac. Check that "The Mac has been idle for …" on the card keeps growing (stuck at 0 means some program keeps generating input), or set the push timing to "Immediately". Opening the app always shows every open question |
| Pushes arrive with a generic text only | The notification extension could not decrypt: make sure both sides share the same pairing; after reinstalling the app pair again or keep the old pairings |
| "This phone's clock differs too much from a Mac's" | Messages have a ±15 minute replay window; fix Date & Time on the phone |
| That Mac shows offline on the phone | Its background service is not running (it exits when idle without Agent activity). Have an Agent ask once, or turn on "Keep this Mac reachable" on the card |
| You want a Mac gone from the phone | Unpair it in the phone's Mac details and revoke the phone on the Mac |

## Advanced

Building the app yourself (with your own Apple developer account and bundle ID) and running your own push relay: see [Advanced: your own build and push relay](./ios-self-hosting.en.md).
