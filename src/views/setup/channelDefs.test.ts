// The wizard's channel registry: its state rules, required-field check, the settings-page wiring,
// and that every text it points at exists in both languages.
import { ref } from "vue";
import { describe, expect, it } from "vitest";
import en from "../../i18n/en";
import zh from "../../i18n/zh";
import {
  CHANNEL_DEFS,
  channelDef,
  channelOps,
  channelSection,
  channelState,
  requiredFieldsFilled,
} from "./channelDefs";
import { config, secrets } from "./setup.testkit";

type Messages = { [key: string]: string | Messages };

/** Resolve a dotted i18n key in a message tree; the segments here never contain dots. */
function message(tree: Messages, key: string): string | undefined {
  let node: string | Messages | undefined = tree;
  for (const part of key.split(".")) {
    if (node === undefined || typeof node === "string") return undefined;
    node = node[part];
  }
  return typeof node === "string" ? node : undefined;
}

describe("channelDef", () => {
  it("covers the four IM channels once each, and looks them up by id", () => {
    expect(CHANNEL_DEFS.map((d) => d.id).sort()).toEqual(["dingding", "feishu", "slack", "telegram"]);
    expect(channelDef("slack")?.id).toBe("slack");
    expect(channelDef("ios")).toBeUndefined();
  });

  it("points only at texts that exist in both languages", () => {
    for (const tree of [en as unknown as Messages, zh as unknown as Messages]) {
      for (const def of CHANNEL_DEFS) {
        expect(message(tree, def.titleKey), def.titleKey).toBeTruthy();
        for (const field of def.fields) expect(message(tree, field.labelKey), field.labelKey).toBeTruthy();
        if (def.detectHintKey) expect(message(tree, def.detectHintKey), def.detectHintKey).toBeTruthy();
        for (let n = 1; n <= def.points; n++) {
          const key = `setup.channels.${def.id}.points.p${n}`;
          expect(message(tree, key), key).toBeTruthy();
        }
        // `points` is the exact count: one more would render an empty bullet.
        expect(message(tree, `setup.channels.${def.id}.points.p${def.points + 1}`)).toBeUndefined();
      }
    }
  });

  it("only asks for fields the config section has, and flags a secret by its saved slot", () => {
    const cfg = config();
    for (const def of CHANNEL_DEFS) {
      const section = channelSection(cfg, def.id);
      for (const field of def.fields) {
        expect(field.key in section, `${def.id}.${field.key}`).toBe(true);
        if (field.secret) expect(field.secret.present in secrets()).toBe(true);
      }
    }
  });

  it("gives every channel but Telegram an auto-detect that fills one field", () => {
    for (const def of CHANNEL_DEFS) {
      const detected = def.fields.filter((f) => f.detected);
      expect(detected.length).toBe(def.id === "telegram" ? 0 : 1);
      expect(def.detectHintKey === null).toBe(def.id === "telegram");
    }
  });
});

describe("requiredFieldsFilled", () => {
  const telegram = channelDef("telegram")!;

  it("is false while a plain field is empty or blank", () => {
    const cfg = config();
    cfg.channels.telegram.botToken = "123:abc";
    expect(requiredFieldsFilled(telegram, cfg, secrets())).toBe(false);
    cfg.channels.telegram.chatId = "   ";
    expect(requiredFieldsFilled(telegram, cfg, secrets())).toBe(false);
    cfg.channels.telegram.chatId = "42";
    expect(requiredFieldsFilled(telegram, cfg, secrets())).toBe(true);
  });

  it("counts a stored secret as filled even though the field is empty", () => {
    const cfg = config();
    cfg.channels.telegram.chatId = "42";
    expect(requiredFieldsFilled(telegram, cfg, secrets())).toBe(false);
    expect(requiredFieldsFilled(telegram, cfg, secrets({ telegramToken: true }))).toBe(true);
  });

  it("needs both of Slack's tokens", () => {
    const slack = channelDef("slack")!;
    const cfg = config();
    cfg.channels.slack.userId = "U1";
    expect(requiredFieldsFilled(slack, cfg, secrets({ slackBotToken: true }))).toBe(false);
    expect(requiredFieldsFilled(slack, cfg, secrets({ slackBotToken: true, slackAppToken: true }))).toBe(true);
  });
});

describe("channelState", () => {
  const row = (enabled: boolean, configured: boolean, connected: boolean | null = null) => ({
    enabled,
    configured,
    connected,
  });

  it("walks from unconfigured to connected", () => {
    expect(channelState(row(false, false), false)).toBe("unconfigured");
    expect(channelState(row(false, true), false)).toBe("configuredOff");
    expect(channelState(row(true, false), false)).toBe("incomplete");
    expect(channelState(row(true, true, null), false)).toBe("ready");
    expect(channelState(row(true, true, false), false)).toBe("ready");
    expect(channelState(row(true, true, true), false)).toBe("connected");
  });

  it("a known fault on an enabled channel wins over everything", () => {
    expect(channelState(row(true, true, true), true)).toBe("issue");
    expect(channelState(row(true, false), true)).toBe("issue");
  });

  it("a fault on a channel that is off is not shown", () => {
    expect(channelState(row(false, true), true)).toBe("configuredOff");
  });
});

describe("channelOps", () => {
  const fake = () => {
    const f = (name: string) => ({
      [`${name}Testing`]: ref(false),
      [`${name}Message`]: ref<string | null>(null),
      [`${name}Error`]: ref(false),
      [`run${name[0].toUpperCase()}${name.slice(1)}Test`]: async () => {},
    });
    const detect = (name: string) => ({
      [`${name}Detecting`]: ref(false),
      [`${name}DetectCode`]: ref<string | null>(null),
      [`run${name[0].toUpperCase()}${name.slice(1)}Detect`]: async () => {},
    });
    return {
      ...f("telegram"),
      ...f("slack"),
      ...detect("slack"),
      ...f("dingtalk"),
      ...detect("dingtalk"),
      ...f("feishu"),
      ...detect("feishu"),
    } as unknown as Parameters<typeof channelOps>[1];
  };

  it("maps each channel to the settings page's own state", () => {
    const api = fake() as unknown as Record<string, unknown>;
    expect(channelOps("telegram", api as never).testing).toBe(api.telegramTesting);
    expect(channelOps("slack", api as never).detectCode).toBe(api.slackDetectCode);
    expect(channelOps("dingding", api as never).message).toBe(api.dingtalkMessage);
    expect(channelOps("feishu", api as never).runDetect).toBe(api.runFeishuDetect);
  });

  it("Telegram has no auto-detect", () => {
    const ops = channelOps("telegram", fake());
    expect(ops.detecting).toBeNull();
    expect(ops.detectCode).toBeNull();
    expect(ops.runDetect).toBeNull();
  });
});
