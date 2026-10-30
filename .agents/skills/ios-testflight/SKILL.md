---
name: ios-testflight
description: Build, upload, and deliver AskHuman Mobile updates through internal TestFlight. Use when the user asks to install or update the iOS app remotely, publish a new internal build, or troubleshoot this project's TestFlight delivery.
---

# iOS TestFlight

Deliver the requested iOS revision to the user's phone through the existing internal TestFlight setup. An explicit request to install or deliver a new version authorizes the upload to this existing audience; a code review or build-only request does not. Reuse the configured credentials and signing identity.

## Established setup

- App Store Connect: **AskHuman Mobile**, App ID **6828712684**.
- Installed display name: **AskHuman**; bundle ID **com.naituw.humaninloop.ios**; team **DMJXDB9H6Q**.
- Internal group: **AskHuman Internal**, automatically receiving subsequent builds. Uploads are internal-only; no external review or App Store submission is needed.
- Local configuration: `~/.config/askhuman-release/app-store-connect.json`, `testflight-export-options.plist`, `testflight-signing.keychain-db`, and `signing-keychain-password`.
- The API key has **App Manager** access. This setup uses a local Apple Distribution certificate because cloud signing rejected that role. It already works; no new key or certificate is needed for routine updates.

Read the repository's `AGENTS.md` and the relevant iOS guidance in `docs/overview-ios.md`. Historical implementation details are in `docs/plans/ios-channel.md`, “2026-10-30：TestFlight 内部分发”.

## Build and deliver

Paths below are relative to the repository root. Set `SKILL_DIR=.agents/skills/ios-testflight` for convenience.

1. Establish the requested revision and inspect local changes. Include only the changes the user intends to install. Run the checks appropriate to those changes; existing Swift/Mock and simulator tests verify app logic without starting paid Agent sessions. Use a dedicated simulator rather than another task's device.
2. Read the current Apple state with `node "$SKILL_DIR/scripts/status.mjs"`. If the requested revision is already available, help install that build and skip rebuilding. Otherwise choose a new build number above the published numbers. An uncertain upload result must be checked here before trying another upload.
3. Regenerate the ignored Xcode project **before building**:

   ```sh
   (cd ios && xcodegen generate)
   xcodebuild archive -project ios/AskHuman.xcodeproj -scheme AskHuman \
     -configuration Release -destination 'generic/platform=iOS' \
     -derivedDataPath ios/build/DerivedDataDevice \
     -archivePath ios/build/AskHuman-TestFlight.xcarchive \
     CURRENT_PROJECT_VERSION=<next-build-number>
   ```

   Use separate build/archive paths if another task is using these outputs. Require `ARCHIVE SUCCEEDED`. The project controls the marketing version; do not bump it just to upload an internal update.
4. Inspect the actual archive and signing inputs without uploading:

   ```sh
   python3 "$SKILL_DIR/scripts/upload.py" ios/build/AskHuman-TestFlight.xcarchive
   ```

   The helper checks the bundle ID, build/version, iPhone device family, production push environment, privacy manifest, internal-only export options, and local credential permissions. Fix an unexpected value in source configuration, regenerate, and rebuild; do not edit a signed archive.
5. For an authorized delivery, upload that archive once:

   ```sh
   python3 "$SKILL_DIR/scripts/upload.py" ios/build/AskHuman-TestFlight.xcarchive --upload
   ```

   The helper permits one upload at a time per signing configuration. It temporarily adds the dedicated signing keychain and removes its addition on completion, failure, or interruption. It does not change the default keychain. Require `EXPORT SUCCEEDED`; preserve the output log when diagnosing failures.
6. Check `status.mjs` after upload. Allow Apple time to process; check again after 30–60 seconds initially and back off if unchanged. Completion requires the build to be **VALID**, present in the internal group, and the intended tester invited/accepted/installed. Upload progress at 100% alone is insufficient. If Apple reports INVALID/FAILED or the state does not advance, report the concrete state and investigate it before another upload.
7. Tell the user the exact version/build. Existing testers can update in TestFlight; a first-time tester must accept Apple's invitation email. Ask them to install and open AskHuman once, confirming their existing Mac connection remains available. This also lets the app refresh its production push registration. Do not uninstall the app as an update step.

## Boundaries and recovery

- Keep credential contents out of replies, logs, source files, and git. The helpers read only the configured key file. The directory is private (`0700`); API and signing secrets are owner-only (`0600`).
- Preserve the existing bundle ID, team, and shared keychain group so an update retains pairing identity. Do not create an external/public group or add other testers without a request covering that audience.
- For missing/expired credentials, certificate/profile failures, or initial setup on another Mac, read [references/signing-and-recovery.md](references/signing-and-recovery.md). Do not revoke unrelated certificates or increase API privileges to get past a failure.
- Release builds use production APNs. A successful foreground connection is useful evidence, but does not replace the separate real-device push/network matrix tracked in `docs/PROGRESS.md`.
- After delivery, update only the appropriate implementation record and genuinely completed progress items. Keep secrets and machine-specific configuration outside the repository.
