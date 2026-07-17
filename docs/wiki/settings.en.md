# General Settings

[简体中文](./settings.md) | English

Run `AskHuman --settings` (or click the gear in the popup's top-right) to open the settings UI. It has three tabs: **General**, **Agents**, and **Channels**. This page covers the **General** preferences; for Agents integration see the "Integrate with your Agent" section in the README, and for channels see "Set up communication channels".

## General

- **Theme** — system / light / dark.
- **Always on top** — keep the popup above other windows.
- **Appear animation** (macOS only) — None / Document / Alert.
- **Window material** (macOS only) — macOS 11–25 offers solid / blur, while macOS 26+ also offers glass. The default is blur; solid is fully opaque; older systems resolve an existing glass preference to blur without rewriting the config.
- **Speech input** (macOS only) — recognition language and trigger shortcut.
- **Reply-history retention** — defaults to 200; set it to `0` to stop recording and clear existing entries. When the existing count exceeds the limit, a "Clean up now" button appears to trim immediately.

## Reply history

Every reply (a "send" completed in the popup or any channel, plus a cancel you trigger yourself) is recorded locally so you can refer back to it while answering new questions. System-triggered cancellations (timeout, disconnect, daemon stop) are not recorded.

- **Open** it with `AskHuman --history` (current project only by default; add `--all` to view every project), or click the "History" button in the popup's top-right. The window also has a top dropdown to switch projects.
- **Project identification** — walk up from the command's working directory to the first `.git` repository root; if there's no `.git`, the working directory is used.
- **Clear** — the "Clear" menu in the history window can clear the "current project" or "all projects".
