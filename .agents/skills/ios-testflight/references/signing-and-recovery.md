# Signing and recovery

Use this only when the established update path fails or must be configured on a different Mac. The initial setup was completed and used successfully on 2026-10-30; routine updates should reuse it.

## Local inputs

`~/.config/askhuman-release/app-store-connect.json` contains `keyId`, `issuerId`, and an absolute `privateKeyPath`. Export options in the same directory contain:

- `method=app-store-connect`, `destination=upload`, `signingStyle=manual`, `teamID=DMJXDB9H6Q`;
- `testFlightInternalTestingOnly=true`, `manageAppVersionAndBuildNumber=true`, `uploadSymbols=true`;
- a signing certificate selector and `provisioningProfiles` entries for the main app and `.notification` extension.

The directory also holds the dedicated signing keychain and its password file. Private certificate material stays here. The original host's reusable uploader is `upload-testflight.py`; the project's `scripts/upload.py` is the maintained, inspectable equivalent.

Never print the JSON key file or secret file contents to diagnose authentication. Read only the needed metadata in memory. The API key is ES256; read-only `scripts/status.mjs` tests authentication without uploading. Certificate and profile expirations are independent from the API key.

## Authentication

- The API key has App Manager access. Apple team API keys apply to all apps in the account; they cannot be restricted to AskHuman alone. Explain that scope before asking a user to create a replacement.
- New/replacement keys are created by the user under App Store Connect → Users and Access → Integrations → App Store Connect API → Team Keys. The user handles account login, any terms, and the one-time private-key download. Never ask them to paste the key body into chat.
- A locked Mac prevented native Xcode login, and Terminal was denied access to iCloud Drive. The user transferred the downloaded `.p8` through an AskHuman attachment. If access is denied again, request an accessible file rather than attempting to bypass macOS permissions.
- The App record already exists as AskHuman Mobile. An empty result when querying its ID/bundle usually needs an account/team/access diagnosis; do not create a second app or change the bundle ID as a workaround.

## Distribution signing

Cloud signing returned “You haven't been given access to cloud-managed distribution certificates” with this key. The user chose to keep App Manager privileges and use a local Apple Distribution certificate. That worked through the same API:

1. Generate a new RSA private key and CSR locally only when renewal/setup is actually needed and authorized. Keep the private key owner-only; send only the CSR to Apple.
2. Use `POST /v1/certificates` with `certificateType=DISTRIBUTION` and `csrContent`. Do not create another certificate merely because an upload failed; check the current certificate, matching private key, and expiration first.
3. Use `POST /v1/profiles` with `profileType=IOS_APP_STORE`, the relevant bundle ID relationship, and the distribution certificate relationship, separately for the app and notification extension. Resolve resource IDs from Apple; do not assume another machine's IDs.
4. Install profiles under `~/Library/Developer/Xcode/UserData/Provisioning Profiles/<uuid>.mobileprovision` and import the local signing identity into a dedicated keychain. Configure its code-signing access with `apple-tool:,apple:,codesign:` using that keychain's own password. Preserve existing keychains and identities.
5. Use manual export with both profile mappings. The uploader serializes uploads per credential directory, temporarily exposes this keychain to the signing search list, and removes only its own addition afterward.

The first local distribution certificate expires on 2027-10-30; inspect actual certificate/profile state when renewing. If creation hits permissions or a quota, report it and ask for the specific repair. Do not revoke another certificate or replace the API key with an Admin key automatically.

## Packaging failures already encountered

- **iPad multitasking/orientations error:** XcodeGen's target-level `TARGETED_DEVICE_FAMILY=1,2` overrode the project-level `1`. The main app and extension now explicitly set target-level `1`, matching the project's iPhone-only scope. Regenerate before building; inspect `UIDeviceFamily` in the archive.
- **Wrong version/build:** XcodeGen's default Info.plist values were literals. `ios/project.yml` now references `MARKETING_VERSION` and `CURRENT_PROJECT_VERSION` for both bundles. Verify the archive rather than trusting command-line overrides alone.
- **Privacy/export declarations:** The app uses the operating system's CryptoKit; the current build declares `ITSAppUsesNonExemptEncryption=false`. Its privacy manifest declares app-owned UserDefaults (`CA92.1`) and app-container file metadata (`C617.1`). Reassess declarations if actual encryption or API usage changes.
- **Unknown upload outcome:** An uploader error after transfer can precede Apple processing. Check the exact build in App Store Connect before retrying; a second upload of the same number is not a recovery strategy.
- **VALID but unavailable to the user:** Check internal group membership and tester state. First-time testers need Apple's invitation email; subsequent updates appear in TestFlight. Adding new testers is a separate audience decision.

## Sources

- [Apple: upload builds](https://developer.apple.com/help/app-store-connect/manage-builds/upload-builds/)
- [Apple: App Store Connect API keys and scope](https://developer.apple.com/help/app-store-connect/get-started/app-store-connect-api/)
- [Apple: internal testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/add-internal-testers/)
- [Apple: required-reason API declarations](https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitype)
- Project record: `docs/plans/ios-channel.md`, “2026-10-30：TestFlight 内部分发”.
