// The remote-answering channels the wizard can set up, as data: which config fields are
// required, which one the auto-detect fills in, and the key points of the platform-side steps.
// The rendering and the save logic are generic over this table, so a further channel is one more
// entry (docs/specs/onboarding.md D22).
import type { Ref } from "vue";
import type { AppConfig, SecretsPresent } from "../../lib/types";
import type { useChannels } from "../settings/useChannels";

export type ChannelId = "telegram" | "slack" | "dingding" | "feishu";

/** The secret behind a field: which "saved" flag it has and which clear intent it uses. */
export interface SecretSlot {
  present: keyof SecretsPresent;
  clear: "telegram" | "dingding" | "feishu" | "slackBot" | "slackApp";
}

export interface ChannelField {
  /** Key inside `config.channels.<id>`. */
  key: string;
  /** i18n key of the label. */
  labelKey: string;
  secret?: SecretSlot;
  /** The auto-detect fills this field in. */
  detected?: boolean;
}

export interface ChannelDef {
  id: ChannelId;
  /** i18n key of the channel's name. */
  titleKey: string;
  /** Required fields, in the order they are asked. */
  fields: ChannelField[];
  /** How many platform-side key points `setup.channels.<id>.points.<n>` holds. */
  points: number;
  /** i18n key of the "send this code to the bot" hint shown while detecting. */
  detectHintKey: string | null;
}

export const CHANNEL_DEFS: readonly ChannelDef[] = [
  {
    id: "telegram",
    titleKey: "settings.channels.telegramTitle",
    fields: [
      {
        key: "botToken",
        labelKey: "settings.channels.botToken",
        secret: { present: "telegramToken", clear: "telegram" },
      },
      { key: "chatId", labelKey: "settings.channels.chatId" },
    ],
    points: 3,
    detectHintKey: null,
  },
  {
    id: "slack",
    titleKey: "settings.channels.slackTitle",
    fields: [
      {
        key: "botToken",
        labelKey: "settings.channels.slackBotToken",
        secret: { present: "slackBotToken", clear: "slackBot" },
      },
      {
        key: "appToken",
        labelKey: "settings.channels.slackAppToken",
        secret: { present: "slackAppToken", clear: "slackApp" },
      },
      { key: "userId", labelKey: "settings.channels.slackUserId", detected: true },
    ],
    points: 5,
    detectHintKey: "settings.channels.slackDetectHint",
  },
  {
    id: "dingding",
    titleKey: "settings.channels.dingtalkTitle",
    fields: [
      { key: "clientId", labelKey: "settings.channels.clientId" },
      {
        key: "clientSecret",
        labelKey: "settings.channels.clientSecret",
        secret: { present: "dingdingSecret", clear: "dingding" },
      },
      { key: "userId", labelKey: "settings.channels.userId", detected: true },
    ],
    points: 4,
    detectHintKey: "settings.channels.detectHint",
  },
  {
    id: "feishu",
    titleKey: "settings.channels.feishuTitle",
    fields: [
      { key: "appId", labelKey: "settings.channels.appId" },
      {
        key: "appSecret",
        labelKey: "settings.channels.appSecret",
        secret: { present: "feishuSecret", clear: "feishu" },
      },
      { key: "openId", labelKey: "settings.channels.openId", detected: true },
    ],
    points: 4,
    detectHintKey: "settings.channels.feishuDetectHint",
  },
];

export function channelDef(id: string): ChannelDef | undefined {
  return CHANNEL_DEFS.find((def) => def.id === id);
}

/** The channel's config section as plain string fields (every required field is a string). */
export function channelSection(
  config: AppConfig,
  id: ChannelId,
): Record<string, string | boolean> {
  return config.channels[id] as unknown as Record<string, string | boolean>;
}

/**
 * Whether every required field has a value. A secret counts as filled when it is typed now or
 * already stored (the settings page shows the same "saved" placeholder for it).
 */
export function requiredFieldsFilled(
  def: ChannelDef,
  config: AppConfig,
  secretsPresent: SecretsPresent,
): boolean {
  const section = channelSection(config, def.id);
  return def.fields.every((field) => {
    const typed = String(section[field.key] ?? "").trim() !== "";
    if (typed) return true;
    return field.secret ? secretsPresent[field.secret.present] === true : false;
  });
}

/** What the remote-answering list says about one channel. */
export type ChannelState =
  | "unconfigured"
  | "configuredOff"
  | "incomplete"
  | "ready"
  | "connected"
  | "issue";

export function channelState(
  row: { enabled: boolean; configured: boolean; connected: boolean | null },
  hasIssue: boolean,
): ChannelState {
  if (row.enabled && hasIssue) return "issue";
  if (row.enabled && row.configured) return row.connected === true ? "connected" : "ready";
  if (row.enabled) return "incomplete";
  return row.configured ? "configuredOff" : "unconfigured";
}

/** What the generic form needs from the settings page's channel logic, per channel. */
export interface ChannelOps {
  testing: Ref<boolean>;
  message: Ref<string | null>;
  error: Ref<boolean>;
  runTest: () => Promise<void>;
  /** `null` for a channel without auto-detect (Telegram). */
  detecting: Ref<boolean> | null;
  detectCode: Ref<string | null> | null;
  runDetect: (() => Promise<void>) | null;
}

type ChannelsApi = ReturnType<typeof useChannels>;

export function channelOps(id: ChannelId, api: ChannelsApi): ChannelOps {
  switch (id) {
    case "telegram":
      return {
        testing: api.telegramTesting,
        message: api.telegramMessage,
        error: api.telegramError,
        runTest: api.runTelegramTest,
        detecting: null,
        detectCode: null,
        runDetect: null,
      };
    case "slack":
      return {
        testing: api.slackTesting,
        message: api.slackMessage,
        error: api.slackError,
        runTest: api.runSlackTest,
        detecting: api.slackDetecting,
        detectCode: api.slackDetectCode,
        runDetect: api.runSlackDetect,
      };
    case "dingding":
      return {
        testing: api.dingtalkTesting,
        message: api.dingtalkMessage,
        error: api.dingtalkError,
        runTest: api.runDingtalkTest,
        detecting: api.dingtalkDetecting,
        detectCode: api.dingtalkDetectCode,
        runDetect: api.runDingtalkDetect,
      };
    case "feishu":
      return {
        testing: api.feishuTesting,
        message: api.feishuMessage,
        error: api.feishuError,
        runTest: api.runFeishuTest,
        detecting: api.feishuDetecting,
        detectCode: api.feishuDetectCode,
        runDetect: api.runFeishuDetect,
      };
  }
}
