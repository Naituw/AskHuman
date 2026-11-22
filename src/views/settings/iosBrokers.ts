// The services the iOS channel's connection form can help with (docs/specs/ios-open-access.md OA11,
// docs/specs/ios-setup-experience.md §4). Pure data: no config field records the choice, so a service
// is recognized by the host name, and anything unknown is "custom" (self-hosted or another provider).
// The wording of every entry — tile, notice, steps, hints — lives in the i18n files under
// `settings.channels.ios.tile.<id>`, `…guide.<id>` and `…broker.<id>.hint`.
// Facts here mirror docs/wiki/ios-broker.md, which stays the place that carries their dates.

export type IosBrokerId = "emqx" | "flespi" | "custom";

/** One line of a service's instructions: `…guide.<id>.<key>`, with optional sub-lines under it. */
export interface IosGuideStep {
  key: string;
  sub?: readonly string[];
}

/** A link a step's text points at (`[label](id)`): an address, or a section of the wiki page. */
export type IosGuideLink = { url: string } | { anchor: { zh: string; en: string } };

/**
 * A string a step has the person type somewhere else (`[[id]]`), shown as a chip that copies it.
 * `{ acl }` is a topic for the account's user name (`{}` stands for it): until one is typed the chip
 * shows a placeholder and cannot be copied.
 */
export type IosGuideChip = { value: string } | { acl: string };

export interface IosBrokerPreset {
  id: IosBrokerId;
  /** Lower-case domain suffixes of the service's host names; empty for "custom". */
  hostSuffixes: readonly string[];
  /** The host when the service has one for everybody; other services give each account its own. */
  host?: string;
  tlsPort: number;
  wssPort: number;
  /** `…guide.<id>.notice`, a line shown above the instructions: a warning, a plain note, or none. */
  notice: "warn" | "info" | null;
  /** What goes between `ah/<` and `>/#` where a step names the topic of one account. */
  aclWord: "username" | "number";
  /** Whether `…guide.<id>.links`, a line of further reading, is shown below them. */
  footer: boolean;
  /** Numbered steps for a service the person signs up to; bullet requirements for one they run. */
  numbered: boolean;
  steps: readonly IosGuideStep[];
  links: Readonly<Record<string, IosGuideLink>>;
  chips: Readonly<Record<string, IosGuideChip>>;
}

/** Sections of docs/wiki/ios-broker.md the self-hosted instructions point at. */
const WIKI_MOSQUITTO = { zh: "路径三自建-mosquitto", en: "path-3-self-hosted-mosquitto" };
const WIKI_VERIFY = { zh: "怎样自己验证", en: "verify-a-broker-yourself" };

export const IOS_BROKERS: readonly IosBrokerPreset[] = [
  {
    id: "emqx",
    hostSuffixes: [".emqxsl.cn", ".emqxsl.com"],
    tlsPort: 8883,
    wssPort: 8084,
    notice: "warn",
    aclWord: "username",
    footer: false,
    numbered: true,
    steps: [
      { key: "step1" },
      { key: "step2" },
      { key: "step3" },
      { key: "step4", sub: ["step4a", "step4b"] },
    ],
    links: { console: { url: "https://cloud.emqx.com/" } },
    // The catch-all deny every account of a shared deployment needs, then the allow for this one.
    chips: { all: { value: "#" }, acl: { acl: "ah/{}/#" } },
  },
  {
    id: "flespi",
    hostSuffixes: [".flespi.io"],
    host: "mqtt.flespi.io",
    tlsPort: 8883,
    wssPort: 443,
    notice: "info",
    aclWord: "number",
    footer: false,
    numbered: true,
    steps: [{ key: "step1" }, { key: "step2" }, { key: "step3" }, { key: "step4" }],
    links: { console: { url: "https://flespi.io/" } },
    chips: { wild: { value: "ah/#" }, acl: { acl: "ah/{}/#" } },
  },
  {
    id: "custom",
    hostSuffixes: [],
    tlsPort: 8883,
    wssPort: 8084,
    notice: null,
    aclWord: "username",
    footer: true,
    numbered: false,
    steps: [{ key: "step1" }, { key: "step2" }, { key: "step3" }],
    links: { mosquitto: { anchor: WIKI_MOSQUITTO }, suite: { anchor: WIKI_VERIFY } },
    chips: { mosquitto: { value: "pattern readwrite ah/%u/#" } },
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

const WIKI = "https://github.com/Naituw/AskHuman/blob/main/docs/wiki";

/** The addresses a service's steps link to, in the reader's language. */
export function brokerLinks(id: IosBrokerId, zh: boolean): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [name, link] of Object.entries(brokerPreset(id).links)) {
    out[name] =
      "url" in link
        ? link.url
        : `${WIKI}/${zh ? "ios-broker.md" : "ios-broker.en.md"}#${zh ? link.anchor.zh : link.anchor.en}`;
  }
  return out;
}

/**
 * What a step's chips copy, for the account's user name. A chip that needs the user name has no
 * value until one is typed (`null`: shown as a placeholder, not copyable).
 */
export function brokerChips(id: IosBrokerId, username: string): Record<string, string | null> {
  const name = username.trim();
  const out: Record<string, string | null> = {};
  for (const [chip, spec] of Object.entries(brokerPreset(id).chips)) {
    out[chip] = "value" in spec ? spec.value : name ? spec.acl.replace("{}", name) : null;
  }
  return out;
}

/** Every key a service's text needs below `settings.channels.ios.guide.<id>`. */
export function guideKeys(id: IosBrokerId): string[] {
  const preset = brokerPreset(id);
  const keys = preset.steps.flatMap((step) => [step.key, ...(step.sub ?? [])]);
  if (preset.notice) keys.push("notice");
  if (preset.footer) keys.push("links");
  return keys;
}
