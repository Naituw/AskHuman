# Antigravity integration

[简体中文](./antigravity-setup.md) | English

AskHuman can integrate with Antigravity on Unix hosts as **one Antigravity plugin**. The Antigravity app and the `agy` command line share it, and it carries the interaction rules, AskHuman's MCP server and the lifecycle hooks.
Open **Settings → Agents → Antigravity** and select one of three mutually exclusive modes:

| Mode | Result |
| --- | --- |
| **MCP** (recommended) | Installs the plugin: the rules, the MCP server (`askhuman_mcp`) and the hooks. Works in the app and in `agy`. |
| **CLI** | Installs the plugin: the rules (telling the agent to call the AskHuman command line) and the hooks, without MCP. The app's default sandbox may block command-line calls, so it is not the recommended mode. |
| **None** | Removes the plugin AskHuman manages and cleans up the shared rule and the `mcpServers.askhuman` entry that earlier versions left behind. |

You can use the same modes from the CLI:

```bash
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity none
AskHuman agents mode antigravity
AskHuman agents update antigravity
```

## The plugin

The plugin directory is:

```text
~/.gemini/config/plugins/askhuman/
```

It holds `plugin.json`, `rules/AGENTS.md` (the interaction rules), `hooks.json` (when lifecycle tracking is on), `mcp_config.json` in MCP mode, and AskHuman's ownership marker `.askhuman-managed`.
Antigravity hot-loads a plugin placed there with no registration, and `agy plugin list` shows it.

- **Lifecycle tracking**: the hooks report Antigravity sessions to AskHuman. They appear in the Agent console with a "working / idle" state and a title (the first user request). You can switch this off on its own in Settings.
- **Switching the plugin off inside Antigravity**: Antigravity records that in its own configuration. AskHuman only reads it and shows "the plugin is switched off in Antigravity" in Settings, `AskHuman agents` and `AskHuman doctor`; it never changes Antigravity's configuration.
- **Only AskHuman's own files**: plugin files are rewritten only when their content changes, and a file you edited counts as a conflict and stays as it is. Rewriting files does not restart a running MCP server or interrupt a pending question, but a changed `mcp_config.json` only reaches a server that starts afterwards, so restart Antigravity for that.

## First-use permission (MCP mode, app)

In the Antigravity app, MCP tools default to "ask": the first AskHuman call shows a native prompt, so choose "Always allow", or add `mcp(askhuman_mcp/*)` in the app's permission settings by hand.
While AskHuman has not seen that grant, Settings and `AskHuman agents` remind you of this step. AskHuman only reads the grant and never writes it.

## CLI mode and the rules

The rules tell the agent to invoke AskHuman with a family hint:

```bash
AskHuman --caller-agent antigravity "Please confirm the next step"
```

The hint identifies only the Antigravity family and grants no permissions. If another agent family's inherited marker is present, AskHuman drops this invocation's identity binding and continues; history recovery that requires a trusted caller still refuses.

CLI mode does not install a timeout Hook. Waiting depends on the command tool exposed by the selected Antigravity client. An asynchronous yield or background-task transition is not a human answer. The generated prompt includes conditional guidance for `run_command` and `manage_task` only when a host provides those tools; it does not assume that IDE or extension tools have the same names, task IDs, parameters, or timeout controls.

## MCP mode

The plugin's `mcp_config.json`:

```json
{
  "mcpServers": {
    "mcp": {
      "command": "/absolute/path/to/AskHuman",
      "args": ["mcp"],
      "timeoutSeconds": 86400
    }
  }
}
```

The host names it `askhuman_mcp`. `timeoutSeconds` is a configuration budget for the AskHuman server and also applies to connection and tool-list operations. It does not mean AskHuman has verified a real 24-hour human wait.

The plugin directory is AskHuman-managed content. Symlinks, hard links and files owned by another user are not supported: AskHuman reports a conflict and leaves them as they are. See [linked configuration and declarative management](../specs/antigravity.md#31-链接与声明式配置管理).

## Sessions and history

AskHuman tells sessions apart by the **root conversation ID** the Antigravity host provides:

- In MCP mode every tool call carries the conversation the host injects, so `show_last` and history are scoped to the conversation, and a sub-agent belongs to its root.
- In CLI mode the conversation ID in the environment is only a candidate. AskHuman trusts it only for a conversation a lifecycle hook has registered as live; otherwise `AskHuman --caller-agent antigravity --show-last` refuses a project-wide fallback, so ask the human to confirm the earlier exchange again.
- Sub-agents currently appear as sessions of their own, without a title. How to group them is still being decided.

## Platform and client scope

- Antigravity plugin and configuration changes are supported by the Unix integration path. Windows returns an explicit unsupported error instead of writing a partial integration.
- The host contract (hook payloads, MCP metadata, plugin loading) was observed on macOS with Antigravity app 2.32.1 and `agy` 1.5.9. Linux shares the implementation but has not been tested. This documentation does not claim hosted CI, Windows, IDE, or extension acceptance.
- The plugin does not guarantee that every Antigravity client reads the rules, exposes a command tool, or supports MCP. Check the behavior in the app or `agy` you actually use.

## Unsupported capabilities

Antigravity currently has no AskHuman integration for:

- Stop confirmation
- interjection and idle messages
- Watch
- native permission interception
- context recovery hooks
- task creation from the phone or IM, and Fork

The integration can ask, recover history by conversation, track lifecycle (console status and title), and hand off the next task. Stop confirmation, interjection, Watch and task creation are designed and will arrive in phases.

## Switching modes safely, and migration

When you switch modes AskHuman first publishes (or removes) the whole plugin, then cleans up the shared rule `~/.gemini/config/rules/askhuman-antigravity.md` and the `askhuman` entry in the global `~/.gemini/config/mcp_config.json` that earlier versions left behind, so there is never a moment without the interaction rules, at most a short duplication.
If you edited those old files AskHuman keeps them and reports it; the plugin is in place either way. Several files are not one atomic transaction.

If a mode switch reports an interrupted or incomplete update, inspect the reported state and retry the explicit target mode:

```bash
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity none
```

AskHuman refuses to overwrite a file that is a link, was edited outside AskHuman, or no longer matches the instance ledger's content or owner. See the [Antigravity behavior spec](../specs/antigravity.md) for the ownership and transaction contract.
