// The services the iOS channel's connection form can help with (docs/specs/ios-open-access.md OA11,
// §6.4). Pure data: no config field records the choice, so a service is recognized by the host name,
// and anything unknown is "custom" (self-hosted or another provider). The wording of every entry —
// name, key points, hints — lives in the i18n files under `settings.experimental.ios.broker.<id>`.
// Facts here mirror docs/wiki/ios-broker.md, which stays the place that carries their dates.

export type IosBrokerId = "emqx" | "flespi" | "custom";

export interface IosBrokerPreset {
  id: IosBrokerId;
  /** Lower-case domain suffixes of the service's host names; empty for "custom". */
  hostSuffixes: readonly string[];
  /** The host when the service has one for everybody; other services give each account its own. */
  host?: string;
  tlsPort: number;
  wssPort: number;
  /** Keys of the short points shown under the choice, in order (`…broker.<id>.<key>`). */
  points: readonly string[];
  /** Heading anchors of the service's section in docs/wiki/ios-broker.md, per language. */
  anchor: { zh: string; en: string };
}

export const IOS_BROKERS: readonly IosBrokerPreset[] = [
  {
    id: "emqx",
    hostSuffixes: [".emqxsl.cn", ".emqxsl.com"],
    tlsPort: 8883,
    wssPort: 8084,
    points: ["host", "credentials", "acl", "limits"],
    anchor: { zh: "路径一emqx-cloud-serverless托管", en: "path-1-emqx-cloud-serverless-hosted" },
  },
  {
    id: "flespi",
    hostSuffixes: [".flespi.io"],
    host: "mqtt.flespi.io",
    tlsPort: 8883,
    wssPort: 443,
    points: ["host", "credentials", "acl", "limits"],
    anchor: { zh: "路径二flespi托管欧洲", en: "path-2-flespi-hosted-europe" },
  },
  {
    id: "custom",
    hostSuffixes: [],
    tlsPort: 8883,
    wssPort: 8084,
    points: ["requirements", "tls", "acl", "limits"],
    anchor: { zh: "对-broker-的要求", en: "what-the-broker-must-do" },
  },
];

export function brokerPreset(id: IosBrokerId): IosBrokerPreset {
  return IOS_BROKERS.find((b) => b.id === id) ?? IOS_BROKERS[IOS_BROKERS.length - 1]!;
}

/** The service a host name belongs to; "custom" when it is not one the form knows. */
export function detectBroker(host: string): IosBrokerId {
  const name = host.trim().toLowerCase().replace(/\.$/, "");
  if (!name) return "custom";
  for (const preset of IOS_BROKERS) {
    if (preset.hostSuffixes.some((suffix) => name.endsWith(suffix))) return preset.id;
  }
  return "custom";
}

/**
 * What choosing a service fills in: its ports, and its host when it has one for everybody. A choice
 * never clears what the user typed, so "custom" fills nothing at all.
 */
export function brokerFill(id: IosBrokerId): { host?: string; port: number; wsPort: number } | null {
  if (id === "custom") return null;
  const preset = brokerPreset(id);
  return {
    ...(preset.host ? { host: preset.host } : {}),
    port: preset.tlsPort,
    wsPort: preset.wssPort,
  };
}

/**
 * The ways "Test connection" fails that a wrong setting at the service causes, and for which every
 * service words a hint (`…broker.<id>.hint.<kind>`): the login was refused, logged in but the access
 * rules swallowed the test message, or no answer in time.
 */
export const HINT_KINDS = ["rejected", "noEcho", "timeout"] as const;
export type IosTestHintKind = (typeof HINT_KINDS)[number];

export function isHintKind(kind: string): kind is IosTestHintKind {
  return (HINT_KINDS as readonly string[]).includes(kind);
}

/** The wiki page of the services, in the reader's language. */
export function brokerGuideUrl(id: IosBrokerId, zh: boolean): string {
  const anchor = brokerPreset(id).anchor;
  const file = zh ? "ios-broker.md" : "ios-broker.en.md";
  return `https://github.com/Naituw/AskHuman/blob/main/docs/wiki/${file}#${zh ? anchor.zh : anchor.en}`;
}
