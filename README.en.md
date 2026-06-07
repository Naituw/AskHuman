<p align="center">
  <img src="assets/banner.jpg" alt="AskHuman" width="800">
</p>

<p align="center"><a href="./README.md">简体中文</a> | English</p>

# AskHuman

A cross-platform human-in-the-loop tool. When an AI agent is about to end a conversation or needs confirmation, it runs the `AskHuman` CLI to pop up a window so you can ask follow-ups, pick options, add text, or attach images — and the result is returned to the agent.

- A single executable `AskHuman` that is both a CLI and an on-demand GUI popup
- Built on **Tauri 2 (Rust + Vue 3)**, supports **macOS / Windows / Linux**
- Multiple channels: local popup + Telegram + DingTalk + Feishu, independently toggleable and racing in parallel when several are on
- Reply history: review recent messages and answers per project (including the reply channel), recorded uniformly across all channels
- Built-in settings UI, Cursor Hook, reference prompt; native macOS glass look

## Install

```bash
# npm (recommended): downloads only the one binary matching your platform
npm i -g askhuman
```

You can also download a platform archive from [GitHub Releases](https://github.com/Naituw/AskHuman/releases), extract it, and put `AskHuman` on your `PATH`. To build from source, see the [development guide](docs/development.md).

> Running the GUI popup on Linux needs system WebKitGTK (e.g. `libwebkit2gtk-4.1`); if it's missing and a session-based channel is configured, AskHuman uses that channel automatically.

## Usage

### 1. The AskHuman command

```bash
# Ask (result goes to stdout). Without -q, the first argument is the question
AskHuman "Continue?" -o "Continue" -o "Stop"

# Multiple questions: the first argument is the shared Message; each -q is a question, -o attaches to the nearest preceding question
AskHuman "Please confirm a few things:" -q "Keep logs?" -o "Keep" -o "Clear" -q "Enable cache?" -o "On" -o "Off"

# Attach files / images for display (apply to the Message, repeatable; absolute / relative / ~ paths)
AskHuman "Take a look?" -f ~/Documents/spec.md -f ./diagram.png

# Others
AskHuman "Plain text" --no-markdown   # disable Markdown rendering
AskHuman --settings                   # open the settings UI
AskHuman --history                    # open reply history (current project; add --all for every project)
AskHuman --help                       # help
AskHuman --version                    # version
```

Results are written to stdout in `[Selected options]` / `[User input]` / `[Images]` / `[Files]` / `[Status]` blocks; logs go to stderr. For the full invocation and output format, see `AskHuman --agent-help`.

### 2. Pairing with an AI Agent

To make an agent "ask the human before finishing", there are a few ways to use it:

- **Put the prompt in rules**: the settings "Integrations" tab provides a copyable reference prompt. Add it to your agent's rules (e.g. Cursor rules / `AGENTS.md` / `CLAUDE.md`) to guide the agent to call `AskHuman` when finishing or needing confirmation.
- **Cursor Hook** (macOS / Linux only): install it with one click from settings. It registers a script in `~/.cursor/hooks.json` that, when it detects a Shell call to `AskHuman`, extends the tool-call timeout to 24 hours so it isn't force-canceled while waiting for your reply.
- **Program integration**: add `askhuman` to your project (`npm i askhuman`); `npm install` pulls the current platform's binary, and at runtime you resolve the path and call it:

```js
import { getBinaryPath, isAvailable } from "askhuman";
import { spawnSync } from "node:child_process";

if (isAvailable()) {
  const r = spawnSync(getBinaryPath(), ["Continue?", "-o", "Continue", "-o", "Stop"], { encoding: "utf8" });
  if (r.status === 3) { /* no available channel: degrade without blocking */ }
  else if (r.status === 0) { /* parse the result blocks from r.stdout */ }
}
```

> Exit codes: success / cancel is `0`; no available channel is `3`; other errors are `1`.
> Custom source name: set `ASKHUMAN_ENV_SOURCE_NAME=Agent`, and the popup title and channel message headers become `Question from Agent`.

## Configuration

Configuration is stored at `~/.askhuman/config.json` and managed by the settings UI. For general config and environment variables, see the [configuration guide](docs/wiki/configuration.en.md); for channel onboarding, see [Telegram](docs/wiki/telegram-setup.en.md) · [DingTalk](docs/wiki/dingtalk-setup.en.md) · [Feishu / Lark](docs/wiki/feishu-setup.en.md).

## Development

For local build, test, and release workflow, see the [development guide](docs/development.md).

## License

[MIT](LICENSE) © Naituw
