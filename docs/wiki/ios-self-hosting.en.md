# Advanced: your own build and push relay

[简体中文](./ios-self-hosting.md) | English

The default path, installing the author's app from TestFlight and using the public relay, does not need this page; see [iOS app channel setup](./ios-setup.en.md).

You only need it if you **build and install the app with your own Apple developer account** (your own bundle ID). The public relay serves only the author's app and refuses registrations from any other bundle ID, so an app you build yourself needs your own push relay and APNs key.

**What you need**: an Apple developer account, Xcode, and a Tencent Cloud account (for the relay's cloud function).

## 1. Prepare the Apple developer resources

1. In Apple Developer → Identifiers register two App IDs: the app (enable **Push Notifications**) and the notification extension (no capabilities). The author's app uses `com.naituw.humaninloop.ios` and `com.naituw.humaninloop.ios.notification`; use your own, and change the team and the bundle IDs in `ios/project.yml` to match.
2. Keys → create a key with **Apple Push Notifications service (APNs)** enabled. The team and environment must match your app: Sandbox for development builds, Production for TestFlight / distribution. Download the `.p8` file (**only once**) and note the **Key ID** and your **Team ID**. New keys can be scoped to a team or to specific apps and to an environment; existing keys that support both environments remain valid. Each relay instance configures one key; deploy one relay per environment when you need both. Follow [Apple's current APNs authentication guide](https://developer.apple.com/documentation/UserNotifications/establishing-a-token-based-connection-to-apns) for the available key settings.

## 2. Deploy the push relay

1. Run `relay/scripts/build-zip.sh` in the repository to get `relay/dist/askhuman-relay.zip`.
2. Tencent Cloud console → Cloud Function → create a **Web function**: runtime Node.js 20, region Hong Kong (no ICP filing needed, reaches APNs), 128 MB of memory, 15 s timeout, upload the zip.
3. Set the environment variables on the function:

   | Variable | Content |
   |---|---|
   | `APNS_KEY_P8` | the whole `.p8` file (paste it; line breaks may be written as a literal `\n`) |
   | `APNS_KEY_ID` | the key's Key ID |
   | `APNS_TEAM_ID` | your Team ID |
   | `APNS_TOPIC` | the app's bundle ID (the same as in `ios/project.yml`) |
   | `RELAY_HANDLE_KEY` | 32 random bytes in base64 (for example `openssl rand -base64 32`) |
   | `REVOKED_RIDS` | (optional) handle ids to revoke by hand, comma separated |

4. Create the **function URL** (public access, authorization "open"). `<function URL>/v1/health` should answer `{"ok":true,"v":1,"apns":true}`.

> Keep the `.p8` and `RELAY_HANDLE_KEY` in the function's environment variables only: not in the repository, a chat or a screenshot.

## 3. Build and install the app

1. Open `ios/AskHuman.xcodeproj` in Xcode (run `cd ios && xcodegen generate` first) and check that the team and the bundle IDs are yours.
2. Pick your device and Run. Allow notifications on first launch.

## 4. Point the Mac at your relay

In AskHuman Settings → **Channels** → **iOS App**, open "More settings" on the card, choose "Use my own relay", enter the function URL and click **Test connection**, then pair your own signed app with this Mac again (see [step 3 of the setup guide](./ios-setup.en.md#3-pair-the-phone)). On the command line: add `--relay-url <function URL>` to `AskHuman channel set ios …`.

## Cost

Using the public relay costs you nothing. If you deploy your own, the cloud function sees very few calls (roughly 1,000 to 20,000 a month); check your account's billing rules in the cost center before deploying so there is no surprise fixed fee.
