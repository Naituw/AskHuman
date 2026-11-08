// Fixtures shared by the Agents-tab tests: a fully integrated Agent that needs nothing, to which a
// test adds only the one thing it is about.
import type { AgentModeStatus } from "../../lib/types";

export function modeStatus(overrides: Partial<AgentModeStatus> = {}): AgentModeStatus {
  return {
    mode: "cli",
    needsUpdate: false,
    ruleNeedsUpdate: false,
    hookNeedsUpdate: false,
    mcpNeedsUpdate: false,
    rulePath: "~/.cursor/rules/askhuman.mdc",
    ruleInstalled: true,
    timeoutHookSupported: true,
    timeoutHookInstalled: true,
    timeoutHookNeedsUpdate: false,
    recoveryHookInstalled: false,
    permission: {
      supported: false,
      unsupportedReason: "native_permission_request_unsupported",
      enabled: false,
      configured: false,
      outdated: false,
      needsUpdate: false,
      knownBlockedReason: null,
      otherHandlersDetected: false,
    },
    permissionNeedsUpdate: false,
    lifecycle: {
      enabled: true,
      preferenceConfigured: true,
      installed: true,
      outdated: false,
      supported: true,
      needsUpdate: false,
      cleanupRequired: false,
    },
    stop: {
      supported: true,
      enabled: true,
      installed: true,
      outdated: false,
      otherHandlersDetected: false,
    },
    delegated: { supported: false, enabled: false },
    askQuestion: {
      supported: false,
      enabled: false,
      installed: false,
      outdated: false,
    },
    mcpSupported: true,
    mcpConfigPath: "~/.cursor/mcp.json",
    mcpConfigInstalled: false,
    runtimeArtifactKind: "hook",
    agentVersion: null,
    minimumVersion: null,
    versionSupported: true,
    ...overrides,
  };
}
