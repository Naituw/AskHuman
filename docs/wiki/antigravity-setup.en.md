# Antigravity integration

[简体中文](./antigravity-setup.md) | English

AskHuman can integrate with Antigravity on Unix hosts. Open **Settings → Agents → Antigravity** and select one of three mutually exclusive modes:

| Mode | Result |
| --- | --- |
| **CLI** (recommended) | Installs the shared Antigravity rule for calls to the AskHuman command line. Antigravity has no managed timeout Hook. |
| **MCP** | Installs the shared rule and registers AskHuman as an MCP server. |
| **None** | Removes the shared rule owned by this AskHuman instance and the `mcpServers.askhuman` entry. |

You can use the same modes from the CLI:

```bash
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity none
AskHuman agents mode antigravity
AskHuman agents update antigravity
```

## Shared rule and CLI mode

Both CLI and MCP modes install:

```text
~/.gemini/config/rules/askhuman-antigravity.md
```

The file contains an always-on rule shared by Antigravity clients. In CLI mode, the rule tells the agent to invoke AskHuman with a family hint:

```bash
AskHuman --caller-agent antigravity "Please confirm the next step"
```

The hint identifies only the Antigravity family. It does not authenticate a conversation or grant permissions. If another agent family's inherited marker is present, AskHuman rejects the conflicting identity.

CLI mode does not install a timeout Hook. Waiting depends on the command tool exposed by the selected Antigravity client. An asynchronous yield or background-task transition is not a human answer. The generated prompt includes conditional guidance for `run_command` and `manage_task` only when a host provides those tools; it does not assume that IDE or extension tools have the same names, task IDs, parameters, or timeout controls.

## MCP mode

MCP mode writes the AskHuman server to:

```text
~/.gemini/config/mcp_config.json
```

The managed entry uses the absolute AskHuman executable path, the `mcp` argument, and a 24-hour timeout budget:

```json
{
  "mcpServers": {
    "askhuman": {
      "command": "/absolute/path/to/AskHuman",
      "args": ["mcp"],
      "timeoutSeconds": 86400
    }
  }
}
```

`timeoutSeconds` is a configuration budget for the AskHuman server and also applies to connection and tool-list operations. It does not mean AskHuman has verified a real 24-hour human wait.

AskHuman edits only the `askhuman` entry and preserves unrelated JSONC entries and comments. If the file is malformed, or `mcpServers` is not an object, the operation stops without replacing the file.

## Session and history limits

AskHuman does not currently receive a trusted native Antigravity session ID.

- In CLI mode, `AskHuman --caller-agent antigravity --show-last` refuses a project-wide fallback because it cannot distinguish conversations. Ask the human to confirm the earlier exchange again.
- In MCP mode, history recovery falls back to the MCP server instance plus the current project. If one MCP server instance serves multiple conversations, recovery may be ambiguous. Restarting the server starts a new history scope. Ask the human to reconfirm rather than assuming which conversation is current.

These limits apply to history lookup only; normal questions and completion handoffs still work through the selected transport.

## Platform and client scope

- Antigravity rule and configuration changes are supported by the Unix integration path. Windows returns an explicit unsupported error instead of writing a partial integration.
- The shared rule does not guarantee that every Antigravity client reads it, exposes a command tool, or supports MCP. Check the behavior in the CLI, IDE, or extension you actually use.
- Current runtime verification is limited to Linux/NixOS. This documentation does not claim hosted CI, macOS, Windows, IDE, extension, or Antigravity 2.0 acceptance.

## Unsupported capabilities

Antigravity currently has no AskHuman integration for:

- lifecycle tracking or Agent Console status
- Stop confirmation
- native permission interception
- context recovery hooks
- interject or Watch
- task launch or Fork

The integration can ask, recover only when a safe history scope exists, and hand off the next task. Features that depend on a trusted session or native lifecycle hook remain unavailable.

## Switching modes safely

Antigravity prepares the MCP configuration before reconciling its rule. The rule uses strict ownership checks and a recovery journal, and the MCP entry is restored conditionally if the rule operation fails. This is not a single atomic transaction across both files.

If a mode switch reports an interrupted or incomplete update, inspect the reported state and retry the explicit target mode:

```bash
AskHuman agents mode antigravity cli
AskHuman agents mode antigravity mcp
AskHuman agents mode antigravity none
```

AskHuman refuses to overwrite a rule that is a link, was edited outside AskHuman, no longer matches the instance ledger, or cannot be reconciled. See the [Antigravity behavior spec](../specs/antigravity.md) for the ownership and transaction contract.
