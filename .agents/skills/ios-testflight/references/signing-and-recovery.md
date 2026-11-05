# Signing, Keychain authorization, and recovery

Routine updates reuse the existing setup. Initial TestFlight delivery and Keychain migration were verified on 2026-10-30. Read this reference when the helper, credentials, or signing identity need repair.

## Credential boundary

The login Keychain holds two generic-password items under service `com.naituw.askhuman.testflight`:

- `app-store-connect.api-private-key`: the API signing key.
- `signing-keychain.password`: the password of the dedicated distribution keychain.

Their access rules trust the Developer ID-signed executable with identifier `com.naituw.askhuman.testflight-helper`, team `DMJXDB9H6Q`. The helper exposes fixed status, local export, upload, one-time migration, and TestFlight-notes operations; it has no credential-dump command. Status requests sign JWTs in memory and use an ephemeral HTTP session. The upload operation is restricted to the existing AskHuman bundle/team and an internal-only export template. Explicit external delivery changes only a temporary copy of the audience flag.

The `notes` operation (modes `read`, `write`, `probe`) acts only on this app and on one VALID, unexpired build number, and only on `zh-Hans` and `en-US` text of 1–4000 UTF-16 units. It sends a fixed set of requests: find the build, then list, create, and modify `betaBuildLocalizations`. A gate rejects every other method or path; a `DELETE` is allowed only in `probe` mode and only for an entry that same run created, and a `PATCH` only for an entry listed for the target build. Writes are read back and compared. The request arrives as an owner-only `0600` file in the launcher's throwaway directory, never in argv or the environment, and the output never contains tokens or credentials. Modifying modes share the export lock. The rules are implemented twice, in `NotesRules.swift` (inside the helper) and in `helper.py`; `scripts/notes-rules-cases.json` is the shared contract and `scripts/ios-testflight-changes.node-test.mjs` runs both implementations against it.

External operations have no arbitrary endpoint/app argument and no deletion. They validate the request before reading credentials and share the export lock. Other apps are read only through explicit `contact-sources`; contact reports exclude demo passwords. See [external-testing.md](external-testing.md) for the fixed app/group, external-eligible build and five-tester limits. Contact and invitation request files contain personal information and belong outside git, with mode `0600`.

Project Python/Node scripts invoke this executable through a one-shot job in the user's GUI launchd domain. That security session is necessary for background agents to access the login Keychain reliably. No foreground app or screen unlock is required when the login Keychain itself remains unlocked. The helper disables credential UI: if access is denied or the login Keychain is locked, it returns an error instead of waiting for an invisible dialog. Ask the user to restore access; do not retrieve their Mac login password or search for plaintext substitutes.

The user authorized unattended use of this specific helper. Publishing still requires a task that requests delivery; permission to inspect code does not request an upload. This protects against accidental credential exposure and broad CLI access; it is not a sandbox against a hostile process running as the same macOS user with code-signing capabilities.

## Local state and helper updates

`~/.config/askhuman-release/` is owner-only (`0700`) and contains:

- `app-store-connect.json`: non-secret `keyId`, `issuerId`, `credentialBackend=keychain`; no private-key path or value.
- `testflight-export-options.plist`: manual signing for team `DMJXDB9H6Q`, app-store-connect upload, internal-only testing, and both provisioning-profile mappings.
- `testflight-signing.keychain-db`: the encrypted local distribution identity.
- `testflight-helper`: the signed operation helper.
- `testflight-helper.previous`: the helper that the latest `install` replaced. To undo an update, rename it back to `testflight-helper`.

After a reviewed helper source change, run `python3 .agents/skills/ios-testflight/scripts/helper.py install`. The installer compiles `TestFlightHelper.swift`, `NotesRules.swift`, `ExternalStatus.swift`, `DistributionRules.swift` and `ExternalTesting.swift` together and requires the established Developer ID identity; it verifies the identifier and team before replacing the binary, and first keeps the binary being replaced as `testflight-helper.previous` (also verified). Stable signing requirements let a rebuilt helper use its existing Keychain authorization. Do not use an ad-hoc signature or another team's certificate as a workaround.

`python3 .agents/skills/ios-testflight/scripts/upload.py <archive> --check-signing` exercises Keychain reads, dedicated-keychain unlock, temporary-file handling, and local IPA signing **without uploading**. Use it when repairing credentials or changing the helper; routine updates need not sign twice.

After a helper change, also check the notes path: `helper.py notes --build <N> --mode read`, and, if the notes write path changed, once `--mode probe` against a delivered VALID build that has no notes. The probe creates a two-line marker (with a bullet and Chinese text, so newline and non-ASCII handling are covered) for each locale without an entry, reads it back, deletes it, and confirms the deletion. It refuses when both locales already have entries and never touches an existing one. The marker can be visible on a phone for a short time. This was done on build 6 on 2026-11-04.

## Temporary material and cancellation

Xcode's upload API requires `-authenticationKeyPath`. Only export/upload materializes that key, under `~/.config/askhuman-release/temporary-keys/upload-<uuid>/` (`0700` directory, `0600` file, excluded from normal system backup). The signing-keychain password stays in memory and is passed to Security.framework, not a command line or environment variable.

The helper serializes exports, starts Xcode in its own process group, and restores its keychain-search-list addition and deletes temporary files on success, failure, SIGTERM/SIGINT, or timeout. A force kill cannot execute cleanup; the next exclusive export removes stale directories in this dedicated namespace. Do not broaden cleanup to unrelated paths. The default keychain and existing identities are preserved.

## Missing credentials or a new Mac

Do not silently create replacement keys, revoke certificates, or raise privileges. Establish the specific repair with the user. Account login, legal terms, and one-time downloads remain user steps.

1. An App Store Connect team API key with App Manager role is sufficient for the established local-signing path. Team keys cover all apps in that account and cannot be restricted to AskHuman alone; explain that scope before requesting a new one.
2. The user supplies any required bootstrap private material through a secure file transfer outside the repository. Never paste private-key bodies into chat or public links. macOS denied Terminal access to iCloud Drive during initial setup; an AskHuman attachment was used instead, then its local copy was deleted after migration.
3. The signed helper must be installed before storing credentials. For an explicitly authorized initial migration, the legacy metadata can temporarily name `privateKeyPath` and a private `signing-keychain-password` bootstrap file; `helper.py migrate` stores both in the login Keychain with helper-specific access and verifies readback. It refuses to overwrite a different existing credential. A later rotation needs an explicit implementation/review, not an automatic overwrite.
4. Set `credentialBackend=keychain` and remove `privateKeyPath`, then verify status and local signing through the helper. Only after verification, remove the supplied local `.p8`, signing-password, raw certificate-private-key, P12, and transfer copies that the user authorized cleaning. The user's original iCloud download was not accessible to this migration; ask them to remove it on their phone if they want no original download retained.

The controlled credential directory and AskHuman transfer copy no longer retain plaintext credentials. The user explicitly chose to keep the original iCloud download as a backup; ordinary agents must use Keychain instead of reading that backup. Do not restore the old file-based uploader from git history to bypass a Keychain error.

## Distribution certificate and provisioning repair

Cloud signing rejected the App Manager key, so the user chose a local Apple Distribution certificate. Its first certificate expires on 2027-10-30. Certificate/profile expiration is independent from the API key.

- A renewal/setup request may need a fresh RSA key and CSR. Only the CSR goes to `POST /v1/certificates` with `certificateType=DISTRIBUTION`; private material stays local and is imported into the dedicated signing keychain.
- `POST /v1/profiles` uses `profileType=IOS_APP_STORE`, the matching bundle ID, and that certificate, separately for the app and `.notification` extension. Resolve resource IDs from Apple.
- Install profiles in `~/Library/Developer/Xcode/UserData/Provisioning Profiles/<uuid>.mobileprovision`, maintain both manual export mappings, and use the dedicated keychain's code-signing partition access (`apple-tool:,apple:,codesign:`). Do not change ACLs on unrelated keychains or grant generic-password access to all applications.
- If permissions or certificate quotas block renewal, report the exact failure and ask for a specific repair. Existing unrelated certificates must remain intact.

## Packaging and delivery failures

- **iPad orientations error:** the first upload validation failed because XcodeGen's target default (`TARGETED_DEVICE_FAMILY=1,2`) overrode the project-level iPhone-only setting and silently brought iPad in; Apple rejects an iPad build that does not declare all four orientations. The project now ships to iPhone and iPad on purpose: both targets set `1,2`, and the app declares four orientations for iPad in `UISupportedInterfaceOrientations~ipad` (iPhone stays portrait-only). Preflight requires exactly `1,2` for the app and the extension (`upload.py`, and the helper again before exporting), and `upload.py` also requires the four iPad orientations or `UIRequiresFullScreen`. Regenerate before building and inspect the archive. If the device families change on purpose, update `upload.py` and `TestFlightHelper.swift` together and reinstall the helper. Apple's own validation of an iPad build (icons, launch screen and so on) only runs at upload, so read the error it reports before trying again.
- **Wrong version/build:** Both Info.plists reference `MARKETING_VERSION` / `CURRENT_PROJECT_VERSION`; check archive values rather than trusting command-line overrides alone.
- **Privacy/export declarations:** Current encryption uses system CryptoKit. The app declares its exemption and app-owned UserDefaults/container metadata uses. Reassess if actual encryption/API usage changes.
- **Unknown upload result:** Check the exact build's Apple state before retrying. Transfer completion alone is not delivery.
- **VALID but unavailable:** Check the internal group and tester invitation state. New testers accept Apple's email; later builds appear in TestFlight. A new audience requires a request covering those people.
- **Notes write failed after a successful upload:** Do not upload again. A 401/403 means the API key's role or access changed: ask the user, and do not raise privileges. Other errors carry Apple's own `errors[]` text; retry only `notes` (it is idempotent and verified by read-back) once the cause is understood.
- **No baseline tag, or the numbers disagree:** `changes.mjs` stops on purpose. Ask the user which commit the last build came from rather than guessing; `docs/plans/ios-channel.md` names the source commit of each delivered build.
- **Probe leftover:** A failed probe tries to clean up its own entries and reports any resource ID it could not delete. The helper never deletes an entry it did not create, so ask the user to remove a leftover in App Store Connect.

## Sources and project context

- [Apple: upload builds](https://developer.apple.com/help/app-store-connect/manage-builds/upload-builds/)
- [Apple: API keys and scope](https://developer.apple.com/help/app-store-connect/get-started/app-store-connect-api/)
- [Apple: internal testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/add-internal-testers/)
- Project `docs/specs/secret-storage-keychain.md` explains stable signing requirements, GUI security sessions, and screen lock versus Keychain lock.
- `docs/plans/ios-channel.md`, “2026-10-30：TestFlight 内部分发”, records this app's verified setup.
