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
- Baselines: every delivered build N has a local annotated tag `ios-testflight-N` on its source commit (build 2 predates the tags). Tags are not pushed. They are what lets `changes.mjs` list what a new build adds.
- Local non-secret configuration: `~/.config/askhuman-release/app-store-connect.json` (`keyId`, `issuerId`, `credentialBackend=keychain`) and `testflight-export-options.plist`.
- Credentials live in the login Keychain under service `com.naituw.askhuman.testflight`. The fixed, Developer ID-signed `testflight-helper` is authorized to use them. The distribution signing key stays in `testflight-signing.keychain-db`; its unlock password also lives in the login Keychain.
- The API key has **App Manager** access. This setup uses a local Apple Distribution certificate because cloud signing rejected that role. It already works; no new key or certificate is needed for routine updates.

Read the repository's `AGENTS.md` and the relevant iOS guidance in `docs/overview-ios.md`. Historical implementation details are in `docs/plans/ios-channel.md`, “2026-10-30：TestFlight 内部分发”.

## Build and deliver

Paths below are relative to the repository root. Set `SKILL_DIR=.agents/skills/ios-testflight` for convenience.

1. Establish the revision to ship as a definite commit: by default `HEAD` with a clean working tree. If the tree has uncommitted changes, stop and ask the user; the tag in step 9 must point at the source that was packaged. Step 3 is where the user decides what the package contains. Run the checks appropriate to the changes; existing Swift/Mock and simulator tests verify app logic without starting paid Agent sessions. Use a dedicated simulator rather than another task's device.
2. Read the current Apple state with `node "$SKILL_DIR/scripts/status.mjs"`. If the requested revision is already available, help install that build and skip rebuilding. An uncertain upload result must be checked here before trying another upload. Otherwise list what the new build would add:

   ```sh
   node "$SKILL_DIR/scripts/changes.mjs"
   ```

   It prints every non-merge commit under `ios/` since the baseline: the local annotated tag `ios-testflight-N` with the largest N, which points at the source commit of build N. It also suggests build N+1 (use it, so the number is above everything Apple has published). **Apple's largest build number must equal N.** If the tag is missing, is not an ancestor of the build point, or the numbers differ (tags are local; another clone may lack them), stop and ask the user; do not guess. If there are no iOS commits, tell the user nothing is new and let them decide whether to build anyway.
3. **Confirm the contents with the user before archiving.** Draft the test notes (rules below) and send the user, through AskHuman: the version and build number; the baseline tag and the build-point commit; every commit from the list (short hash, date, subject; you may group them by feature); and both drafts in full. Offer: build it as listed, edit the text, change the build point, cancel. Nothing is archived or uploaded until the user has answered. After "build it as listed", steps 4–10 run without further questions unless something unexpected happens.
   - To leave a change out, move the build point to an earlier commit of the same history: `changes.mjs --point <commit> --expect-dropped <hash,...>`. It fails unless the point drops exactly the commits the user chose to exclude. If a change the user wants to keep comes after one they want out, stop and ask the user how to proceed (for example reverting on the main line first). Never cherry-pick or assemble a package from pieces. For a point other than `HEAD`, archive from a clean temporary worktree at that commit (`git worktree add --detach <dir> <commit>`, regenerate the project there, remove the worktree afterwards) instead of the current tree.
4. Regenerate the ignored Xcode project **before building**:

   ```sh
   (cd ios && xcodegen generate)
   xcodebuild archive -project ios/AskHuman.xcodeproj -scheme AskHuman \
     -configuration Release -destination 'generic/platform=iOS' \
     -derivedDataPath ios/build/DerivedDataDevice \
     -archivePath ios/build/AskHuman-TestFlight.xcarchive \
     CURRENT_PROJECT_VERSION=<next-build-number>
   ```

   Use separate build/archive paths if another task is using these outputs. Require `ARCHIVE SUCCEEDED`. The project controls the marketing version; do not bump it just to upload an internal update. Record `git rev-parse HEAD` of the tree you archive from; step 9 verifies it did not move.
5. Inspect the actual archive and signing inputs without uploading:

   ```sh
   python3 "$SKILL_DIR/scripts/upload.py" ios/build/AskHuman-TestFlight.xcarchive
   ```

   Preflight checks both bundle IDs and versions, that the app and its extension ship to exactly iPhone and iPad (device family `1,2`), the iPad orientation declaration (all four, or `UIRequiresFullScreen`), production push environment, privacy manifest, internal-only export options, and the installed helper's signature. It does not read credentials. Fix an unexpected value in source configuration, regenerate, and rebuild; do not edit a signed archive. If the shipped device families change on purpose, update `upload.py` and `TestFlightHelper.swift` together and reinstall the helper.
6. For an authorized delivery, upload that archive once:

   ```sh
   python3 "$SKILL_DIR/scripts/upload.py" ios/build/AskHuman-TestFlight.xcarchive --upload
   ```

   The signed helper reads Keychain items in the user's GUI security session, unlocks the dedicated signing keychain in memory, and permits one export at a time. Xcode requires a key-file path: the helper creates a transient `0600` file outside the project and removes it on completion, failure, or interruption. It also restores its signing-keychain search-list addition. Require `EXPORT SUCCEEDED`; preserve the output log when diagnosing failures.
7. Check `status.mjs` after upload. Allow Apple time to process; check again after 30–60 seconds initially and back off if unchanged. Completion requires the build to be **VALID**, present in the internal group, and the intended tester invited/accepted/installed. Upload progress at 100% alone is insufficient. If Apple reports INVALID/FAILED or the state does not advance, report the concrete state and investigate it before another upload; write no notes and create no tag for such a build.
8. Once the build is VALID, write the confirmed test notes (private temporary files; delete them afterwards) and let the helper read them back:

   ```sh
   python3 "$SKILL_DIR/scripts/helper.py" notes --build <N> --mode write --zh <zh.txt> --en <en.txt>
   ```

   `--dry-run` validates the files and prints what would be sent, without starting the helper. The write is idempotent and verified by reading the text back. If it fails after a successful upload, **do not upload again**: the build is already delivered. A 401/403 is an authorization problem for the user; for anything else report Apple's message, and retry only this step once the cause is understood.
9. Tag the source commit locally: `git tag -a ios-testflight-<N> <source-commit> -m "TestFlight <version> (<N>)" -m "Apple build <id>" -m "Source commit <full hash>"`. First check that `HEAD` and the working tree are still what step 4 recorded, otherwise ask the user before tagging. Never overwrite an existing tag, and do not push tags: the tag is the next build's baseline on this machine.
10. Record the delivery in `docs/plans/ios-channel.md`: source commit, baseline tag, the change list in a few lines, the Chinese test notes in full, the Apple build ID and its status. Tell the user the exact version/build. Existing testers can update in TestFlight; a first-time tester must accept Apple's invitation email. Ask them to install and open AskHuman once, confirming their existing Mac connection remains available. This also lets the app refresh its production push registration. Do not uninstall the app as an update step.

### Test notes

TestFlight shows them under the build, in Simplified Chinese (`zh-Hans`) and English (`en-US`). They are plain text: TestFlight does not render Markdown.

- First line: `构建 N · 源码 <7-character commit>` and `Build N · source <7-character commit>`.
- Then one sentence per user-visible change (what it does and where to see it), writing a feature's commits as one entry; mention test, refactor and chore commits only when they change behavior; no commit hashes in the body; bullets use `•`.
- Then 1–3 items under `请重点看` / `Please check` that need a real device, taken from the pending checks in `docs/PROGRESS.md` where they apply.
- Chinese and English entries correspond one to one. If the build contains a change the user earlier chose to hold back, say so plainly.
- At most 4000 characters (aim for 1500). Every internal tester can read them: no secrets, local paths or unreleased information.

## Boundaries and recovery

- Keychain access is granted to the stable signed helper, not to Python, Node, or a general credential-reading CLI. Its public operations return status, perform an export/upload, or read and write the TestFlight notes of one build (`notes`: this app only, `zh-Hans` and `en-US` only, fixed endpoints; deletion exists only in `probe` mode and only for an entry that same run created); no operation returns private-key contents. Scripts never read a persistent `.p8` or password file, nor pass a password in argv/environment.
- When the login Keychain is unlocked, the approved helper can work while the screen is locked. A locked login Keychain, missing item, or changed authorization must fail promptly and be resolved through the user. Do not enable access for all applications, use ad-hoc signing, or fall back to plaintext files.
- Rebuilding the helper with its established identifier and Developer ID preserves Keychain authorization. Use `python3 "$SKILL_DIR/scripts/helper.py" install` when a reviewed helper update is needed; it keeps the replaced binary as `testflight-helper.previous`, and renaming that file back undoes the update. Ordinary uploads reuse the installed binary. After a helper change, check `status.mjs`, `upload.py <archive> --check-signing`, and `helper.py notes --mode read`; to exercise the write path run `notes --mode probe` once against an already delivered VALID build that has no notes (it writes a marker text, reads it back, and deletes it). Never write real text to any build except the one just delivered. For credential repair, read the recovery reference first.
- A force-killed process cannot run cleanup; the next exclusive export removes stale helper-owned temporary directories. Keep the credential directory `0700` and temporary key files `0600`. The helper never writes temporary credentials into the project.
- Preserve the existing bundle ID, team, and shared keychain group so an update retains pairing identity. Do not create an external/public group or add other testers without a request covering that audience.
- For missing/expired credentials, certificate/profile failures, or initial setup on another Mac, read [references/signing-and-recovery.md](references/signing-and-recovery.md). Do not revoke unrelated certificates or increase API privileges to get past a failure.
- Release builds use production APNs. A successful foreground connection is useful evidence, but does not replace the separate real-device push/network matrix tracked in `docs/PROGRESS.md`.
- After delivery, update only the appropriate implementation record and genuinely completed progress items. Keep secrets and machine-specific configuration outside the repository.
