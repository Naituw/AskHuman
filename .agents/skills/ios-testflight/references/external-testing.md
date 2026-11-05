# External TestFlight for friends

Use only when the task explicitly requests external delivery. The first cohort is one to five developer friends, each with their own Mac, agents and EMQX. The maintainer provides the existing relay. No public joining link, new account platform or demo mode was requested. This is Beta App Review, not an App Store release.

## Inspect before acting

```sh
python3 .agents/skills/ios-testflight/scripts/helper.py external-status
# Only when the owner asks to reuse contact information from their other apps:
python3 .agents/skills/ios-testflight/scripts/helper.py contact-sources
```

`external-status` covers this app's Beta localizations/review details, all groups, group tester-state counts and ten most recently uploaded builds. `status.mjs` remains the original internal-delivery view. `contact-sources` reads contact fields from other apps and projects a fixed output allowlist, excluding demo credentials and other apps' review notes. Contact output contains personal information: store it privately, never paste it into a public report or git. If sources disagree, obtain the owner's choice; do not invent contact details. Lists over 200 entries fail instead of silently treating a partial result as complete.

## Materials and local validation

- Public documents: `docs/public/ios/{privacy,privacy-zh,beta,beta-zh}.md`. Build with `node scripts/build-ios-beta-site.mjs`. This removes and recreates the dedicated `site-dist` output and publishes only those pages plus an index.
- Intended HTTPS base: `https://naituw.github.io/AskHuman/`. Verify privacy and guide URLs in both languages without authentication **before shipping**. GitHub Pages was disabled on 2026-11-05; the owner explicitly requires confirmation before enabling it. The workflow is manual, so committing it alone does not publish anything.
- Descriptions and review notes: `docs/release/ios-external-beta/`. They are drafts until the Mac download and relay information are attached. Describe the actual Mac/MQTT dependency and absence of an in-app demo honestly. Do not claim approval is guaranteed.
- Keep contact/recipient JSON outside the repository, mode `0600`. There is no reason to read a `.p8` file or increase API-key privileges.
- Run `node --test scripts/ios-testflight-changes.node-test.mjs scripts/ios-testflight-preflight.node-test.mjs scripts/ios-testflight-external.node-test.mjs`. The fake transport exercises the real Swift mutation rules with no credentials or network.

## Build and upload

Follow the skill's clean source commit, baseline, machine-health, archive, notes and local-tag rules. Re-query the latest Apple build immediately before choosing the next number. Never reuse an internal-only build: Apple marks it `INTERNAL_ONLY`, and it cannot be converted for external testing.

```sh
# Preflight only; credentials are not read:
python3 .agents/skills/ios-testflight/scripts/upload.py <archive> --audience external
# Local IPA signing only, when validating a helper change:
python3 .agents/skills/ios-testflight/scripts/upload.py <archive> --audience external --check-signing
# Authorized upload, once:
python3 .agents/skills/ios-testflight/scripts/upload.py <archive> --audience external --upload
```

Both Python and the signed helper validate the audience. Persistent export options must remain internal-only. The helper sets `testFlightInternalTestingOnly=false` in a temporary copy for external export/upload. Bundle IDs, team, profiles, encryption declaration, privacy manifest, iPhone/iPad and production APNs checks remain in force. After upload require `VALID`, unexpired, `APP_STORE_ELIGIBLE`, and resolved encryption before the external mutation steps. A successful local export proves signing, not Apple acceptance.

## Bounded operations

All operations use the same entry point:

```sh
python3 .agents/skills/ios-testflight/scripts/helper.py external --request /private/path/request.json
```

The request's `mode` selects one operation. Other fields or app IDs are rejected before credentials are read. The example build `10` is illustrative; use the actual just-uploaded build.

1. `metadata`: exact locales `en-US` and `zh-Hans`, each with `description`, `feedbackEmail`, `privacyPolicyUrl`, `marketingUrl`; `review` with `contactFirstName`, `contactLastName`, `contactPhone`, `contactEmail`, `notes`, `demoAccountRequired: false`. Plain text, at most 4000 UTF-16 units, HTTPS public URLs. Existing fields are compared; only changed fields are patched, missing locales created, and every supplied value read back. No demo password is supplied or overwritten.
2. `{"mode":"group","build":"10"}` creates or reuses **AskHuman Friends** in app **6828712684**. The group must be external, public links off, manual build access. It disables and reads back auto-notify before assigning the build. Adding a new build to a group that already has testers is refused: later cohort updates need a separately authorized workflow.
3. Write both build notes using the original `notes` command. Then `{"mode":"submit","build":"10"}` requires the build already in the group and complete Beta metadata. It submits only from `READY_FOR_BETA_SUBMISSION`. Waiting/in-review/approved/testing states are returned without another POST; submission is read back. Network errors stop; inspect the state before another attempt.
4. **Only after approval and an explicitly provided recipient list**, `{"mode":"invite","build":"10","emails":["friend@example.com"]}` adds exact authorized emails. Existing testers in other apps can be reused but only this group's membership changes. Total cohort size is capped at five. With auto-notify off, the helper sends a tester-specific invitation only from `NOT_INVITED`; `INVITED`, `ACCEPTED`, `INSTALLED` are preserved without re-sending. Unknown/revoked/unconfirmed states stop for inspection. The helper does not send general email or create App Store Connect users.

Do not run an example against Apple. Prepare the real private request, inspect it, then execute the authorized operation. Each modifying operation holds the same exclusive lock as upload/notes. Partial results are not rolled back or blindly retried: read current state first.

## Completion and recovery

Keep these milestones separate: uploaded/VALID; external eligible; submitted; approved; invited; accepted; installed; actual pairing/foreground answer/production push working. Do not mark delivery complete solely because Apple accepted an upload or a tester was added to a group. The first friend verifies their own setup; do not start paid model calls to simulate their acceptance.

A 401/403, Apple agreement, identity verification or reviewer request for a different review strategy goes to the owner. Do not broaden privileges, accept legal terms, provide a new demo mode or host a shared broker without agreement. Record waiting state and the exact next step in `docs/PROGRESS.md`; do not create a monitor unless requested.

Record the source tag after Apple accepts the external-eligible build so subsequent build-number selection remains correct even while review is pending. The tag means “uploaded source”; external approval and delivery are separate dated records in the plan. Never overwrite an existing tag.

Sources: [external testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/invite-external-testers/), [invitation behavior](https://developer.apple.com/documentation/appstoreconnectapi/beta-tester-invitations), [Apple OpenAPI](https://developer.apple.com/sample-code/app-store-connect/app-store-connect-openapi-specification.zip). The initial implementation was checked against official OpenAPI 4.6 on 2026-11-05.
