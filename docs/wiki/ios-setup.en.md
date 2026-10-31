# iOS app channel setup (experimental)

[简体中文](./ios-setup.md) | English

The AskHuman iOS app is a communication channel next to Feishu and Slack: your Agent asks on the Mac, you answer on your iPhone. You can also see what your Agents are doing, watch one live, send it a message and start new tasks. **Several Macs can pair with one phone** (home and office, say); all their questions land in one inbox.

> This is an **experimental feature** and you bring three things yourself: an MQTT broker (the free tier of EMQX Serverless is enough), a push relay (a Tencent Cloud function, free tier is enough) and an Apple developer account (to create a push key and install the app). The steps below cover all of it.

## How it works, and who can see what

```
Agent ─► AskHuman on the Mac ──MQTT (TLS)──► EMQX ──► AskHuman on the iPhone
                 └── push relay (cloud function) ──► APNs ──► iPhone notification
```

- **End-to-end encrypted.** Every pairing creates a pair of keys only that Mac and that phone know. EMQX and the push relay see ciphertext and metadata such as "who is online", never your questions or answers.
- **Your APNs key lives only in the cloud function.** The Mac holds no Apple key; the relay is stateless and forwards the encrypted preview to Apple from a fixed allowlist.
- **Push content is encrypted too.** The question summary on the lock screen is decrypted by the notification extension on the phone; if that fails you only see a generic line.
- **One broker account for all your Macs.** Set up the first Mac, then paste its "connection string" into the others.

## 1. Prepare EMQX (the broker)

1. Create a **Serverless** deployment on [EMQX Cloud](https://cloud.emqx.com/) (China site) or the [international site](https://cloud-intl.emqx.com/) (free tier: 1 million connection minutes and 1 GB of traffic a month).
2. Note the **connection address** on the deployment overview, and the ports: TLS `8883`, WSS `8084`.
3. **Authentication**: add a username and password. Use **letters and digits only** for the username (it is also the topic namespace `ah/<username>/` of the line).
4. **Authorization (ACL)** (console: Access Control → Client Authorization). The page defaults to **blacklist mode: anything not explicitly denied is allowed**, so rule 1 below, the catch-all, is **required**; without it every account can read and write everyone else's topics:
   1. Switch to the "All users" tab and add: topic `#`, action "Publish & Subscribe", **Deny** (catch-all).
   2. Switch to the "Username" tab and add: the username from the previous step, topic `ah/<username>/#`, action "Publish & Subscribe", **Allow**. EMQX checks the account's own rules first and the "All users" rules after, so only its own topics get through.

   Afterwards you can check the isolation with `ios/Tools/broker-probe/acl-check.cjs` (it needs one account's username and password; run `npm install` in `ios/Tools/broker-probe` first and pass the password in the `AH_PROBE_PASSWORD` environment variable, see the notes at the top of the script).
5. **Keep the spend cap at 0.** When the free quota is used up the service pauses; nothing is charged automatically. AskHuman warns you as you approach the quota (see "Traffic and cost").

> Sharing with a friend: create a separate username and password for each person and add one `ah/<username>/#` allow rule of their own (rule 2); leave the "All users" catch-all alone. They cannot see each other's topics.

## 2. Prepare the Apple developer resources

1. In Apple Developer → Identifiers register two App IDs: the app `com.naituw.humaninloop.ios` (enable **Push Notifications**) and the notification extension `com.naituw.humaninloop.ios.notification` (no capabilities). With your own team, change the team and the bundle IDs in `ios/project.yml` too, and make the relay's `APNS_TOPIC` equal the app's bundle ID.
2. Keys → create a key with **Apple Push Notifications service (APNs)** enabled. Click Configure on the APNs row and pick the environment **Sandbox & Production** (development builds use the sandbox, TestFlight and the App Store use production, and one key serves both; the restriction then has to be Team Scoped). Apple **does not let you change these two settings after saving**. After registering, download the `.p8` file (**you can only download it once**) and note the **Key ID** and your **Team ID**.

## 3. Deploy the push relay

1. Run `relay/scripts/build-zip.sh` in the repository to get `relay/dist/askhuman-relay.zip`.
2. Tencent Cloud console → Cloud Function → create a **Web function**: runtime Node.js 20, region Hong Kong (no ICP filing needed, reaches APNs), 128 MB of memory, 15 s timeout, upload the zip.
3. Set the environment variables on the function:

   | Variable | Content |
   |---|---|
   | `APNS_KEY_P8` | the whole `.p8` file (paste it; line breaks may be written as a literal `\n`) |
   | `APNS_KEY_ID` | the key's Key ID |
   | `APNS_TEAM_ID` | your Team ID |
   | `APNS_TOPIC` | `com.naituw.humaninloop.ios` (the app's bundle ID) |
   | `RELAY_HANDLE_KEY` | 32 random bytes in base64 (for example `openssl rand -base64 32`) |
   | `REVOKED_RIDS` | (optional) handle ids to revoke by hand, comma separated |

4. Create the **function URL** (public access, authorization "open"). `<function URL>/v1/health` should answer `{"ok":true,"v":1,"apns":true}`.

> Keep the `.p8` and `RELAY_HANDLE_KEY` in the function's environment variables only: not in the repository, a chat or a screenshot.

## 4. Turn it on on the Mac

1. AskHuman Settings → **General** → at the very bottom turn on "Experimental features"; an "Experimental" tab appears (or run `AskHuman config set experimental.enabled true`).
2. **Experimental** → **iOS App channel**, switch it on and fill in:
   - broker host, TLS port (8883), WSS port (8084), username, password;
   - the push relay URL (the function URL);
   - the name this Mac shows on the phone (empty uses the computer name).
3. Click **Test connection**: it connects to the broker and sends itself a message, reporting whether `tls` or `wss` was used and the round-trip time; then it asks the relay for its health and reports how fast it answered and whether it has an APNs key (without a relay address it warns that the phone gets no pushes).
4. The same from the command line:

   ```bash
   AskHuman channel set ios --enable --host <host> --username <username> \
     --relay-url <function URL> --password-stdin        # the password is read from stdin
   AskHuman channel test ios
   ```

5. **A second Mac**: on the first one click **Copy connection string** (it contains the broker password: pass it only between your own devices); on the second paste it under "Reuse another Mac's settings" and click Import. On the command line: `AskHuman channel ios export` and `AskHuman channel ios import -`.

> The iOS channel does not force the daemon to stay up: it is running while your Agents work, and when it has stopped the phone shows that Mac as offline. To keep the Mac reachable at all times set the daemon lifecycle to keepalive (Settings → Advanced; turning on "Create Agent tasks from IM" does it for you).

## 5. Install the app and pair

1. Open `ios/AskHuman.xcodeproj` in Xcode (run `cd ios && xcodegen generate` first), pick your device and Run; or install through TestFlight. Allow notifications on first launch.
2. On the Mac: Settings → Experimental → iOS App channel → **Pair a new phone** (command line: `AskHuman channel ios pair`). A QR code and a pairing code appear, **valid for 5 minutes and for one use**.
3. On the phone: the **Macs** tab → **Add a Mac** → **Scan QR code** (or "Paste the pairing code" when scanning is awkward).
4. The phone and the Mac each show a **6-digit check code**. Compare them and click **Allow** on the Mac. Pairing is done and the phone immediately receives the Mac's open requests.
5. To undo it: click **Revoke** next to the phone in the Mac settings (`AskHuman channel ios revoke <name>`); on the phone use "Unpair this Mac" in the Mac's details, or "Forget all pairings" at the bottom of the Macs tab. After reinstalling the app the phone asks whether to keep the pairings left in the keychain.

## 6. Everyday use

- **Inbox**: questions from all your Macs, grouped by project and filterable by Mac. Open one to pick one or several options, add a note, attach images and files; a multi-question request is one vertical page whose bottom button reads "Next" until you have seen the last question and only then turns into "Send" (as in the Mac popup, so nothing goes out half-read), recommended options carry a star (nothing is preselected). After an answer goes out the app slides to the next request and tells you how many are left. Actions that need your decision (permissions, Stop) have a dedicated confirmation page.
- **Switch between requests**: use the arrows at the top right to open the previous or next request; More stays at the far right. Drafts, attachments and reading positions are kept, and switching dismisses the keyboard. The queue keeps the list filter and appends new requests during this visit. Uploads and replies can finish in the background without taking you away from another request. Automatic advance after sending still uses the entire inbox.
- **When notifications arrive**: every question shows up in the phone's inbox, with nothing to "activate" or "take over"; open the app any time to see and answer it. The system notification only follows once you have been away from the Mac for a while: when the Mac's keyboard and mouse have been idle for the configured time (15 minutes by default) and you have not looked at the question on the phone, you get one notification and no reminders; while you are still working on the Mac nothing is pushed. To be told sooner, change **Push timing** in the Mac settings → Experimental → iOS app channel to 5 minutes or even "Immediately" (push as soon as a question is asked); the "The Mac has been idle for …" line there shows whether the detection works.
- **Answering across channels**: when the same question shows up in the Mac popup, Feishu and the phone, the first answer wins and the others are told it was handled.
- **Agents tab**: shows every Mac session and its last result. Failed and incomplete sessions appear under Needs attention; interrupted turns are silent. Tasks created from the phone and new rounds actually started by a phone message or Request continuation receive one completion, failure or incomplete-result notification (up to 5 per Mac). Notifications still wait for Mac idle time and are dropped when you return to the Mac first. A small read-only bell marks the rounds that will notify you. Other Mac turns are not automatically subscribed. Live watching is foreground-only, with up to two streams.
- **Continue an existing session**: send an idle Codex Desktop, Cursor Desktop or live Pi session a message from its detail page, or choose Request continuation after a failed or incomplete result. Continuation preserves your unsent draft. Pi needs an updated extension; unsupported sessions explain why. Existing working-session messages can still be queued and edited or withdrawn before consumption; accepted native messages cannot be withdrawn. If delivery is unknown, check the original chat before making a fresh submission. Both the App and Mac need this update.
- **New task**: the "New task" button on the Agents tab: pick the Mac, project, Agent kind, the permission (ask before acting / run automatically; you must choose one) and optionally a TODO, confirm with Face ID and the Mac launches it (Codex can use the desktop App or CLI; other Agents use a terminal). It needs "Create Agent tasks from IM" turned on on that Mac, with its prerequisites.
- **Task attachments**: add camera photos, library photos or videos, and files. You can add up to 5 files from the phone, each up to 10 MB. Enter instructions or select a TODO; its existing attachments are included too. Upload failures keep the draft and do not start the task. Older Macs need an AskHuman update.
- **New requests while using the app**: a blue question bubble bounces twice and flies into Inbox when a question, permission request or Stop confirmation arrives. With the tab bar hidden, it lands in a brief hint at the top. Each request plays independently; the animation passes through touches, keeps your page and keyboard in place, and opens nothing. Visit Inbox to answer. With Reduce Motion enabled, only the static hint is shown.
- **Codex Fast**: choose a supported model in a new task to enable Fast for either the desktop App or CLI. It starts off for each new task; off explicitly means Standard speed. Fast uses more of your allowance, and the speed choice is never remembered. AskHuman and Codex on the Mac must support this feature.
- **Todos tab**: manage the todos on your Macs from the phone: browse projects, add, edit (tap), finish (swipe right), delete (swipe left, then tap Delete), reorder ("Edit" at the top right), switch ⚡ auto-run, and look through the history and restore from it. The menu in the title picks whose todos you see: one Mac (only the projects that have a folder on it), or **iCloud** (when you use iCloud todo sync: every synced project of your Apple ID, including ones that have no folder on the Mac in front of you; a Mac that has sync on and is online answers for it, and another one takes over when it drops). The todos live on the Macs and the phone keeps no copy of them, so the Mac you are looking at has to be online (when it is not, the page says so and the input bar is disabled; nothing is queued). The phone cannot create projects (a project is a folder on a Mac: open it under "Recent projects" / "Without todos" to give it its first todo). A change on the Mac shows up on the phone within a few seconds; a change from the phone is written to the Mac it went through and, when iCloud todo sync is on, travels from there to your other Macs (iCloud's own delay applies, usually tens of seconds). A Mac whose AskHuman is too old to know about todos shows "Needs an update" in the menu: update AskHuman on that Mac.
- **History**: answered requests and attachments are kept on the phone.

## Traffic and cost

- EMQX China-site free tier: 1 million connection minutes and 1 GB of traffic (in plus out) a month; typical use (100 requests a day, one hour of live watching a day) stays far below 300 MB a month.
- The Mac settings and the phone's Macs tab both show this month's traffic (counted locally; a message counts twice, published and delivered). **At 80% of the free quota it turns amber**, and red when it is probably gone. With the spend cap at 0 the broker then refuses connections and the app says so: "the broker refused the connection, this month's free allowance may be used up".
- The cloud function sees very few calls (roughly 1,000 to 20,000 a month); check your account's billing rules in the cost center before deploying so there is no surprise fixed fee.

## Troubleshooting

| Symptom | What to do |
|---|---|
| Settings say "Broker not connected" | Click "Test connection" for the reason; when the office network blocks 8883 it falls back to WSS 8084 by itself; an authentication failure is usually a wrong password or ACL |
| No pushes, but requests arrive while the app is open | Check the system notification permission; look for "push is no longer valid" under the phone in the Mac settings; make sure `<function URL>/v1/health` says `apns:true` |
| A question was asked long ago and the phone never notified you | The notification waits until the Mac's keyboard and mouse have been idle for the "Push timing" setting (15 minutes by default) and is never sent while you are using the Mac; check that "The Mac has been idle for …" in the Mac settings keeps growing (stuck at 0 means some program keeps generating input), or set the push timing to "Immediately". Opening the app always shows every open question |
| Pushes arrive with a generic text only | The notification extension could not decrypt: make sure both sides share the same pairing; after reinstalling the app pair again or keep the old pairings |
| "This phone's clock differs too much from a Mac's" | Messages have a ±15 minute replay window; fix Date & Time on the phone |
| You want a Mac gone from the phone | Unpair it in the phone's Mac details and revoke the phone on the Mac |
| That Mac shows offline on the phone | Its daemon is not running (it exits when idle without Agent activity): have an Agent ask once, or set the daemon lifecycle to keepalive |
| Traffic warning | Use less live watching and fewer attachments, or check the usage in the EMQX console; the local count only covers this device's messages |

## Limits

- Experimental: the settings entry and the commands are hidden by default and details may change.
- Live watching only works while the app is in the foreground and stops in the background.
- Codex Desktop chats accept additions while running and new messages while idle, including images and files, with the chat's existing settings. Desktop must stay connected on the Mac. Success means the App accepted the message, not that the model read it; it cannot be withdrawn. If delivery is uncertain, inspect the original chat before choosing **I checked the original chat**; this prompt and recovery action require an updated phone app. Codex has no "failed" state, so there is no "failed" push for it.
- Withdrawing a message gives back the text only, not the attachments you had picked.
