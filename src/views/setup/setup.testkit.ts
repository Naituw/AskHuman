// Data builders for the setup wizard's tests: an Agent row, the settings payload the wizard loads,
// the setup status and the per-channel status rows. Plain data only; each test file owns its mocks.
import type {
  AgentKind,
  AgentTaskReadiness,
  AgentTaskWorkspace,
  AppConfig,
  SecretsPresent,
  SettingsPayload,
  SetupAgentRow,
  SetupChannelStatus,
  SetupEnvReport,
  SetupStatus,
} from "../../lib/types";

export function row(over: Partial<SetupAgentRow> = {}): SetupAgentRow {
  return {
    id: "claude",
    title: "Claude Code",
    signals: { cli: true, configDir: true, app: false },
    detected: true,
    mode: "none",
    needsUpdate: false,
    statusError: null,
    supportsCli: true,
    supportsMcp: true,
    recommended: "cli",
    delivery: null,
    planned: { cli: [], mcp: [] },
    ...over,
  };
}

export function status(over: Partial<SetupStatus> = {}): SetupStatus {
  return {
    agentsDecided: false,
    remoteDecided: false,
    backgroundDecided: false,
    loginItemInstalled: false,
    loginItemAvailable: true,
    trayAvailable: true,
    everydayIntegrationsReadOnly: false,
    phoneTasksDeclined: false,
    ...over,
  };
}

/** A saved project the phone could pick; the label is the folder's name. */
export function workspace(
  path: string,
  over: Partial<AgentTaskWorkspace> = {},
): AgentTaskWorkspace {
  return {
    path,
    label: path.split("/").pop() ?? path,
    lastUsedAt: 1_700_000_000,
    agents: ["claude"],
    pinned: false,
    hidden: false,
    ...over,
  };
}

/** An Agent that can start a task (CLI found, lifecycle and integration in place). */
export function readiness(
  kind: AgentKind,
  over: Partial<AgentTaskReadiness> = {},
): AgentTaskReadiness {
  return {
    kind,
    label: kind === "claude" ? "Claude Code" : kind[0].toUpperCase() + kind.slice(1),
    command: kind,
    executable: `/usr/local/bin/${kind}`,
    version: null,
    binaryReady: true,
    lifecycleReady: true,
    integrationReady: true,
    integrationMode: "cli",
    ready: true,
    launchSupported: true,
    diagnostics: [],
    ...over,
  };
}

export function env(over: Partial<SetupEnvReport> = {}): SetupEnvReport {
  return { askhumanOnPath: "found", pathFix: null, transient: null, installs: null, ...over };
}

export function channelRow(
  id: string,
  over: Partial<SetupChannelStatus> = {},
): SetupChannelStatus {
  return { id, enabled: false, configured: false, connected: null, ...over };
}

/** Every channel off and nothing configured. */
export function noChannels(): SetupChannelStatus[] {
  return ["telegram", "slack", "dingding", "feishu"].map((id) => channelRow(id));
}

export function secrets(over: Partial<SecretsPresent> = {}): SecretsPresent {
  return {
    dingdingSecret: false,
    feishuSecret: false,
    telegramToken: false,
    slackBotToken: false,
    slackAppToken: false,
    iosBrokerPassword: false,
    ...over,
  };
}

interface ConfigOptions {
  style?: string;
  menuBarIcon?: string;
  daemonLifecycle?: string;
  experimental?: boolean;
  agentTasks?: boolean;
  /** The iOS app channel is on (it is set up in Settings, not in the wizard). */
  ios?: boolean;
}

/** Just the config fields the wizard reads or writes, typed as the full config. */
export function config(opts: ConfigOptions = {}): AppConfig {
  return {
    general: {
      theme: "light",
      language: "en",
      collaborationStyle: opts.style ?? "aligned",
      menuBarIcon: opts.menuBarIcon ?? "active",
      daemonLifecycle: opts.daemonLifecycle ?? "activity",
    },
    channels: {
      telegram: { enabled: false, botToken: "", chatId: "", apiBaseUrl: "" },
      slack: { enabled: false, botToken: "", appToken: "", userId: "" },
      dingding: { enabled: false, clientId: "", clientSecret: "", userId: "" },
      feishu: { enabled: false, appId: "", appSecret: "", openId: "" },
      ios: { enabled: opts.ios ?? false, password: "" },
    },
    agentTasks: { enabled: opts.agentTasks ?? false },
    experimental: { enabled: opts.experimental ?? false },
  } as unknown as AppConfig;
}

export function settings(
  opts: ConfigOptions & { secretsPresent?: Partial<SecretsPresent>; readOnly?: boolean } = {},
): SettingsPayload {
  return {
    config: config(opts),
    secretsPresent: secrets(opts.secretsPresent),
    everydayIntegrationsReadOnly: opts.readOnly === true,
  };
}
