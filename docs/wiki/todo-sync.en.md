# iCloud todo sync

> For a new release that includes the sync component. Published v0.16.0 does not include it.
> [简体中文](todo-sync.md)

Share project todos and run history between your Macs signed in to the same Apple ID. This is an
experimental feature, off by default, requiring **macOS 14 or later**. Other AskHuman features
continue to support macOS 11 or later.

## Enable

1. Install or update AskHuman on each Mac with `npm install -g askhuman`.
2. Enable Experimental features in Settings, then open Experimental → Todo storage.
3. Select iCloud sync. AskHuman installs the component included in the release; no separate download
   or Xcode installation is required.
4. If macOS asks you to approve the background item, open System Settings → General → Login Items
   & Extensions and allow **AskHuman Todo Sync**. Return to AskHuman and click Repair.
5. Enable sync separately on your other Macs using the same Apple ID, and wait for the first sync.

You do not need an Apple developer account, CloudKit Dashboard, or service credentials. Each Apple ID
uses its own iCloud private database. Todos are not shared between different Apple IDs.

## What syncs

- Projects match by their normalized Git remote. Directories and worktrees of one repository become
  copies of the same queue. Directories without a remote remain local only.
- Text, ordering, completion, restoration, deletion, and run history sync between Macs. You can also
  view and edit projects whose directory is not on this Mac yet.
- **Attachment files do not sync.** Other Macs show which device has them. The ⚡ automatic-run flag
  stays local too.
- **Clearing a project's run history clears that history on every Mac and in iCloud.** The confirmation
  explains this. Removing a project from iCloud also removes its synced todos and history everywhere.

## Timing and offline use

iCloud pushes and periodic polling bring background changes across. This can take tens of seconds
or longer; there is no seconds-level delivery guarantee. Opening or focusing the todo window refreshes
in the background. You can also click Sync now.

What's next tries to refresh when the last successful pull is older than 20 seconds, waiting at most
an extra 1.5 seconds. On timeout or offline it uses the local copy and catches up in the background.
An already-displayed card does not change its options when later data arrives.

You can read and edit local todos offline; they merge after reconnection. Wait for the other Mac to
finish syncing before assuming it has received a particular change.

## Stop sync and account changes

Stop syncing keeps both this Mac's copy and the cloud data. Enabling again reconciles the differences.
Clear todo data in iCloud deletes the entire cloud todo zone and pauses other Macs; it requires
explicit confirmation.

Changing this Mac's Apple ID pauses sync so the previous account's data is not automatically uploaded
to a different account. Check the account and intended data before proceeding; keep the local copy
if unsure.

## Troubleshooting

- Waiting for background approval: allow AskHuman Todo Sync in System Settings, then click Repair.
- Missing or incompatible component: update AskHuman and click Repair. npm updates carry the matching component.
- Signed out or iCloud storage full: resolve the account or storage problem and retry.
- Missing changes: check both Macs, click Sync now, and confirm the same Apple ID and Git remote,
  a working network, and awake devices.

Use `AskHuman todo-sync status --json` for diagnostics. Component logs are in
`~/Library/Logs/AskHumanTodoSync/helper.log`. Do not delete `todos.json` to repair syncing.
