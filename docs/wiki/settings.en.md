# General Settings

[简体中文](./settings.md) | English

Run `AskHuman --settings` (or click the gear in the popup's top-right) to open the settings UI. It has three tabs: **General**, **Agents**, and **Channels**. This page covers the **General** preferences; for Agents integration see the "Integrate with your Agent" section in the README, and for channels see "Set up communication channels".

## General

iCloud todo sync stays under Experimental and requires enabling experimental features first.
See [iCloud todo sync](todo-sync.en.md) for requirements, setup, and limitations.

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

## Quick replies

The **Quick reply** button in the popup footer (⌘K / Ctrl+K) lists your preset replies: click one, or press the digits 1–9 (0 for the tenth), and it is **sent at once**. Questions you have already answered keep your answers; the ones without an answer all receive that sentence; once every question is answered the presets grey out. What's next and choose-only questions have no presets. The phone's answer screen has the same menu in its bottom bar — one tap sends; the presets are those of the Mac the question came from, and the phone does not edit them.

Edit them under **Settings → General → Quick replies** (or "Manage quick replies…" at the bottom of the menu):

- Five come built in ("Continue", "Use your own judgement, don't wait for me", …) in the interface language; changing any one of them makes the list yours.
- One input per row, saved when it loses focus; clearing a row deletes it; drag the handle on the left (or press ↑ / ↓ on it) to reorder; up to 10 replies of at most 200 characters; "Restore defaults" returns to the built-in list.

## Away mode and "stop using AskHuman in this session"

The other two entries in the menu are **instructions**: they do not send anything yet, they wait above the answer area and take effect with this answer when you press "Send and …".

- **Away mode**: AskHuman automatically answers Agent questions and What's next, telling the Agent to keep working toward your original objective and make ordinary decisions while respecting explicit task limits. When that objective is complete or no remaining work can proceed independently, the Agent contacts you with `--escalate "reason"` (MCP: `escalate: "reason"`). Its reason appears before the report in the popup, phone, IM and history. Old booleans and empty reasons receive corrective guidance; after three requests without a valid reason, the fourth is delivered as a clearly labeled system fallback without an Agent reason. Escalating does not end away mode. Choose "All Agents" or "This session only" and optionally add a note that stays attached to automatic replies. Every automatic reply is recorded in history.
  - **Visible**: the tray icon carries a moon, the console shows a banner, on macOS a desktop capsule sits at the top centre of the screen (hover or click to expand, draggable, can be turned off in Settings), and the phone app shows a teal strip above the tab bar.
  - **Turn on**: "Away mode…" in the menu, the tray menu, the "AskHuman" menu in a console session header, the moon button on the phone's Agents page, `AskHuman away on [note] [--session]`, or `/away [note]` in IM.
  - **Turn off**: only you can — "End away mode" at the top of an escalated popup, the tray, the console, the capsule, "End" on the phone, `AskHuman away off` (refused inside an Agent process), or `/back` in IM. On macOS, when you return to the computer after five idle minutes, a "Welcome back" summary appears; it only reminds, it never ends away mode.
  - **Settings → General → Away mode**: the desktop capsule, the welcome-back reminder, and "Wait before pushing to the phone" (default: Mac idle for 1 minute; the shorter of this and the iOS channel's own push timing applies).
- **Stop using AskHuman in this session**: this answer tells the Agent "the user chose not to use AskHuman, don't use it again"; afterwards any call from that session (including `--whats-next`) gets the same reminder immediately, and the Stop confirmation stops interrupting. Permission approvals and error notifications still work. To undo it, switch "Stop using" off for that session in the tray's Agent submenu, the console session header's "AskHuman" menu, or the phone's session row: at its next tool call the Agent reads "you may use AskHuman again" (delivered through the interject queue, so nothing is sent to the original chat and no new billed turn starts).

Stop confirmations, permission approvals, error cards and Codex native questions have no such button and are not affected by away mode.

## Cursor Desktop session protection

On macOS, **Agents → Cursor → Cursor session protection (experimental)** is off by default and
available while CLI/MCP integration is active. It applies to every local Cursor Desktop chat,
including chats that have not called AskHuman, and covers two things under one switch:

- **Background completions no longer continue on their own.** Background commands and subagents keep
  running and retain their results; completion notifications no longer start a model turn
  automatically, so send a message to continue. Active Await calls still receive results. Goals,
  subscriptions and scheduled automation are separate.
- **Running chats keep their messages in memory (Agents window only).** When a long first turn finally
  fails, Cursor judges from the messages it still has loaded whether the submission produced anything,
  and can wrongly archive the chat. While a chat is generating, the protection keeps its loaded
  messages, lowering that risk. It costs more memory for running chats, does not fix Cursor's own
  error handling, and cannot protect a turn that already had gaps before the protection was active.

Terminal Cursor CLI is not covered.

While enabled, AskHuman keeps its daemon running and automatically attaches to existing and newly
opened Cursor windows. Settings distinguish waiting, connecting, active, partly active (the part that
is not active is named, and the other keeps working) and failed states. A compatible Cursor version
must pass checks before each part becomes active; failed checks leave native behavior in place.
Startup and reload require a short attachment period. No special Cursor launch command is needed, and
compatibility with future upgrades is not guaranteed.
Disabling the switch or integration restores native behavior without replaying already settled
notifications or archiving/hiding chats. Stopping the daemon manually can leave existing window guards
active; disable this switch or integration to restore native behavior. `AskHuman doctor --json`
includes `cursorDesktop` diagnostics (counts of ignored unloads only; no chat IDs or message content).

## Creating Cursor tasks

**Where tasks run.** The "Agents → Cursor" integration card lets you choose where new tasks run: the CLI (default) or the
Agents window (experimental, macOS only). This only decides which route is tried first: when it is not usable and the other one is,
the other one is used automatically and the card's status line says why. So with only Cursor Desktop installed and no CLI, the phone,
IM and GUI can still create Cursor tasks (through the Agents window). On the Agents window route, Cursor tasks created from the GUI,
IM `/new` or the phone start a chat in your already-open Cursor Agents window and send the task automatically. Cursor must be
running with exactly one Agents window, and the project must already be added there. Once a task has started in the Agents window,
a failure never falls back to the CLI.

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
