# askhuman

English | [简体中文](https://github.com/Naituw/AskHuman/blob/main/README.zh-CN.md)

Cross-platform "human-in-the-loop" interaction tool. When an AI assistant is about to end a conversation or needs confirmation, the `AskHuman` CLI pops up a window (or uses Telegram / Slack / DingTalk / Feishu) so you can ask follow-ups, pick options, add text, or attach images — and the result is returned to the AI.

跨平台「Human-in-the-loop」交互工具。当 AI 助手准备结束对话或需要确认时，命令行 `AskHuman` 弹出窗口（或经 Telegram / Slack / 钉钉 / 飞书），让你继续提问、勾选选项、补充文字或附带图片，并把结果回传给 AI。

<p align="center">
  <img src="https://raw.githubusercontent.com/Naituw/AskHuman/main/assets/channels.webp" alt="Reply to your agent from the local popup, Telegram, Slack, DingTalk, or Feishu" width="900">
</p>

Under the hood it's a single executable (Tauri 2 / Rust). This npm package distributes it via per-platform subpackages: installing fetches only the one binary matching your current platform.

## Standalone use

```bash
npm i -g askhuman
AskHuman "Continue?" -o "Continue" -o "Stop"
```

## As a dependency (programmatic use)

```bash
npm i askhuman
```

```js
import { getBinaryPath, isAvailable } from "askhuman";
import { spawnSync } from "node:child_process";

if (!isAvailable()) {
  // Binary not in place: skip the human-confirmation step to avoid blocking the flow
} else {
  const r = spawnSync(getBinaryPath(), ["Continue?", "-o", "Continue", "-o", "Stop"], {
    encoding: "utf8",
  });
  if (r.status === 3) {
    // No usable channel in this environment (GUI can't open and no session channel configured): degrade gracefully
  } else if (r.status === 0) {
    // Success: parse the result blocks from r.stdout
    console.log(r.stdout);
  }
}
```

`getBinaryPath()` resolution order: env var `ASKHUMAN_BINARY` (legacy `HUMANINLOOP_BINARY` still works) → platform subpackage → system `PATH`.

## Exit code contract

| Exit code | Meaning |
|---|---|
| `0` | Got a result, or the user cancelled (emits `[Status]`) |
| `3` | No usable channel (local popup can't open and no session channel configured) — downstream should degrade |
| `1` | Other error |

stdout contains only the result blocks (`[Selected options]` / `[User input]` / `[Images]` / `[Files]` / `[Status]`); all logs and errors go to stderr.

## Platforms and system dependencies

Supports macOS (arm64/x64) and Linux (x64).

> Running the GUI popup on Linux needs system WebKitGTK (e.g. `libwebkit2gtk-4.1`). If it's missing and a session-based channel is configured (Telegram / Slack / DingTalk / Feishu), AskHuman uses that channel automatically; if none is available, it signals degradation with exit code `3`.

More info in the project repo: <https://github.com/Naituw/AskHuman>
