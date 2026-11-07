// Data builders for the setup wizard's tests: an Agent row, the settings payload the wizard loads,
// the setup status and the per-channel status rows. Plain data only; each test file owns its mocks.
import type {
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
      ios: { password: "" },
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
