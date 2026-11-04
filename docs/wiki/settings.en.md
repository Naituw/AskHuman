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

## Answer windows

Open Settings, then **General → Popup behavior → Question window mode**, the first option in that card. **Merged** is the default.
Search for **merged**, **independent**, **window mode**, or **Sidebar** to jump directly to it.
Requests from multiple agents appear in one window, grouped by project in the sidebar. Each request retains its own draft; the window hides after all requests finish.

Choose **Independent** to use one window per request. Changes apply to new requests. Open requests and their drafts stay in their current windows. Both modes use the Popup prewarm setting.

## Reply history

Every reply (a "send" completed in the popup or any channel, plus a cancel you trigger yourself) is recorded locally so you can refer back to it while answering new questions. System-triggered cancellations (timeout, disconnect, daemon stop) are not recorded.

- **Open** it with `AskHuman --history` for the current project or add `--all` for every project. You can also click "History" in the popup. When the current Agent session already has history, the popup entry shows that session across projects; otherwise it falls back to the current project.
- **Project identification** — walk up from the command's working directory to the first `.git` repository root; if there's no `.git`, the working directory is used.
- **Filter** — use one two-level scope menu to pick a project, then "Everything in this project" / "All sessions" or one session from its submenu; space-separated keywords can be layered on top. Native Agent sessions show a best-effort title and short ID; MCP fallbacks are explicitly labeled as approximate sessions.
- **Clean up** — the context action reads "Delete current search results", "Clear selected session history", or "Clear selected project history" and deletes only the entries frozen when confirmation opens. New matching entries that arrive afterward are retained. "Clear all history" remains separate and is the only item shown for the unfiltered global scope.

## Cursor Desktop background follow-ups

On macOS, **Agents → Cursor → Prevent background completion follow-ups (experimental)** is off by default and
available while CLI/MCP integration is active. It applies to every local Cursor Desktop chat,
including chats that have not called AskHuman. Background commands and subagents keep running and
retain their results; completion notifications no longer start a model turn automatically. Send a
message to continue. Active Await calls still receive results. Goals, subscriptions and scheduled
automation are separate. Terminal Cursor CLI is not covered.

While enabled, AskHuman keeps its daemon running and automatically attaches to existing and newly
opened Cursor windows. Settings distinguish waiting, connecting, active and failed states. A compatible
Cursor version must pass checks before protection becomes active; failed checks leave native behavior
in place. Startup and reload require a short attachment period. No special Cursor launch command is
needed, and compatibility with future upgrades is not guaranteed.
Disabling the switch or integration restores native behavior without replaying already settled
notifications or archiving/hiding chats. Stopping the daemon manually can leave existing window guards
active; disable this switch or integration to restore native follow-ups. `AskHuman doctor --json`
includes `cursorDesktop` diagnostics.

## Creating Cursor tasks

**Where tasks run.** The "Agents → Cursor" integration card lets you choose where new tasks run: the CLI (default) or the
Agents window (experimental, macOS only). With the Agents window selected, Cursor tasks created from the GUI, IM `/new` or the
phone start a chat in your already-open Cursor Agents window and send the task automatically. Cursor must be running with exactly
one Agents window, and the project must already be added there. If the window is not available before creation, the task
uses the CLI and the card's status line says why; once a task has started in the Agents window, a failure never falls back to the CLI.

**Max mode.** On the old request-based plan, Max mode is billed at API rates and burns through credits quickly, so AskHuman
turns Max off by default when it creates a Cursor task (Advanced → "Max mode for Cursor tasks"):

- The "Allow Max" switch in the new-task window and on the phone is off by default, is chosen per task, and is never saved; turning it on runs with Cursor's default behavior. IM `/new` has no switch and follows the default policy.
- The Cursor model list marks which models need Max. A model that needs Max does not start while Max is not allowed, and you are told to pick another model or allow Max for this task. When AskHuman cannot tell, the task still starts with a notice and is re-checked after launch.
- The CLI route turns off the CLI's global Max switch, which is a sticky global setting: manual CLI runs afterwards stay off until you turn it back on with `/max-mode`. The Agents window route only affects the chat it creates.
- If Max is found switched on again after launch, an ordinary question card alerts you.
- The policy can be changed to "Follow Cursor's current setting", which means no intervention, no blocking and no notices. The new token-based plans have no Max mode; the plan type is detected automatically and can be set manually, and on a new plan all related UI is hidden and Cursor's settings are left untouched.

## Codex MCP environment overrides

`AskHuman agents mode codex mcp --env DISPLAY=:1` explicitly adds an environment variable to Codex's AskHuman MCP server. Repeat `--env NAME=VALUE` to merge multiple keys; the last repeated key wins. Values may be empty or contain `=`. Names must be nonempty and contain neither `=` nor NUL; values must not contain NUL. Other agents and modes reject this option before changing integration files.

Unspecified variables survive installs and updates, including existing TOML table or inline-table `env` entries. Claude and Cursor updates also preserve an existing JSON `env` object, while rebuilding other managed entry fields. Malformed environment entries abort the configuration write. This is opt-in: AskHuman does not inject desktop environment variables automatically.
