import { describe, expect, it } from "vitest";
import { i18n } from "../../i18n";
import {
  HINT_KINDS,
  IOS_BROKERS,
  brokerChips,
  brokerFill,
  brokerLinks,
  brokerPreset,
  detectBroker,
  guideKeys,
  isHintKind,
} from "./iosBrokers";
import { parseRich } from "./iosRich";

const LOCALES = ["zh", "en"] as const;

function message(path: string, locale: (typeof LOCALES)[number]): string {
  return String(i18n.global.t(path, { acl: "ah/<x>/#" }, { locale }));
}

describe("detectBroker", () => {
  it("recognizes the hosted services by the end of the host name", () => {
    expect(detectBroker("abc123.ala.cn-hangzhou.emqxsl.cn")).toBe("emqx");
    expect(detectBroker("abc123.ala.eu-central-1.emqxsl.com")).toBe("emqx");
    expect(detectBroker("mqtt.flespi.io")).toBe("flespi");
  });

  it("ignores case, spaces and a trailing dot", () => {
    expect(detectBroker("  MQTT.Flespi.IO. ")).toBe("flespi");
    expect(detectBroker("ABC.EMQXSL.CN")).toBe("emqx");
  });

  it("only matches whole domain labels, so look-alike names are not taken for a service", () => {
    expect(detectBroker("evilemqxsl.cn")).toBe("custom");
    expect(detectBroker("emqxsl.cn.evil.example")).toBe("custom");
    expect(detectBroker("flespi.io.example.com")).toBe("custom");
  });

  it("treats everything else, including nothing, as self-hosted or another broker", () => {
    expect(detectBroker("")).toBe("custom");
    expect(detectBroker("   ")).toBe("custom");
    expect(detectBroker("broker.example.com")).toBe("custom");
    expect(detectBroker("localhost")).toBe("custom");
  });
});

describe("brokerFill", () => {
  it("fills the ports of a service, and the host only when the service has one for everybody", () => {
    expect(brokerFill("emqx")).toEqual({ port: 8883, wsPort: 8084 });
    expect(brokerFill("flespi")).toEqual({ host: "mqtt.flespi.io", port: 8883, wsPort: 443 });
  });

  it("fills nothing for a broker the form knows nothing about", () => {
    expect(brokerFill("custom")).toBeNull();
  });
});

describe("brokerLinks", () => {
  it("points the hosted services' steps at their consoles", () => {
    expect(brokerLinks("emqx", true)).toEqual({ console: "https://cloud.emqx.com/" });
    expect(brokerLinks("flespi", false)).toEqual({ console: "https://flespi.io/" });
  });

  it("points the self-hosted steps at sections of the wiki page in the reader's language", () => {
    expect(brokerLinks("custom", false)).toEqual({
      mosquitto:
        "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.en.md#path-3-self-hosted-mosquitto",
      suite: "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.en.md#verify-a-broker-yourself",
    });
    expect(brokerLinks("custom", true).suite).toBe(
      "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.md#怎样自己验证",
    );
  });
});

describe("brokerChips", () => {
  it("copies fixed strings as they are", () => {
    expect(brokerChips("emqx", "")).toMatchObject({ all: "#" });
    expect(brokerChips("flespi", "")).toMatchObject({ wild: "ah/#" });
    expect(brokerChips("custom", "")).toEqual({ mosquitto: "pattern readwrite ah/%u/#" });
  });

  it("has nothing to copy for an account's topic until a user name is typed", () => {
    expect(brokerChips("emqx", "")).toMatchObject({ acl: null });
    expect(brokerChips("emqx", "   ")).toMatchObject({ acl: null });
    expect(brokerChips("emqx", " ah1 ")).toMatchObject({ acl: "ah/ah1/#" });
    expect(brokerChips("flespi", "7364410")).toMatchObject({ acl: "ah/7364410/#" });
  });
});

describe("the catalog", () => {
  it("has one entry per service, with 'custom' last as the fallback", () => {
    expect(IOS_BROKERS.map((b) => b.id)).toEqual(["emqx", "flespi", "custom"]);
    expect(brokerPreset("custom")).toBe(IOS_BROKERS[IOS_BROKERS.length - 1]);
  });

  it("never lists the same host suffix twice", () => {
    const suffixes = IOS_BROKERS.flatMap((b) => b.hostSuffixes);
    expect(new Set(suffixes).size).toBe(suffixes.length);
    for (const suffix of suffixes) expect(suffix).toMatch(/^\.[a-z0-9.-]+$/);
  });

  it("has every tile, step, notice and hint in both interface languages", () => {
    for (const locale of LOCALES) {
      for (const preset of IOS_BROKERS) {
        // A name can be short ("flespi"); a step or a hint is a sentence.
        const entries = [
          ["tile.%.name", 3] as const,
          ["tile.%.sub", 3] as const,
          ...guideKeys(preset.id).map((key) => [`guide.%.${key}`, 8] as const),
          ...HINT_KINDS.map((kind) => [`broker.%.hint.${kind}`, 12] as const),
        ];
        for (const [template, shortest] of entries) {
          const path = `settings.channels.ios.${template.replace("%", preset.id)}`;
          expect(i18n.global.te(path, locale), `${locale}: ${path}`).toBe(true);
          const text = message(path, locale);
          // A message that fails to compile (a stray brace or bar) comes back as its own key.
          expect(text, `${locale}: ${path}`).not.toBe(path);
          expect(text.length, `${locale}: ${path}`).toBeGreaterThanOrEqual(shortest);
        }
      }
    }
  });

  it("only uses links and chips a service defines, and leaves no markup unread", () => {
    for (const locale of LOCALES) {
      for (const preset of IOS_BROKERS) {
        for (const key of guideKeys(preset.id)) {
          const path = `settings.channels.ios.guide.${preset.id}.${key}`;
          const pieces = parseRich(message(path, locale));
          for (const piece of pieces) {
            if (piece.kind === "link") expect(preset.links, `${locale}: ${path}`).toHaveProperty(piece.id);
            if (piece.kind === "chip") expect(preset.chips, `${locale}: ${path}`).toHaveProperty(piece.id);
            if (piece.kind === "text") expect(piece.text, `${locale}: ${path}`).not.toMatch(/\*\*|`|\[\[|\]\]/);
          }
        }
      }
    }
  });

  it("uses every link and chip a service defines", () => {
    for (const preset of IOS_BROKERS) {
      const used = new Set<string>();
      for (const key of guideKeys(preset.id)) {
        for (const piece of parseRich(message(`settings.channels.ios.guide.${preset.id}.${key}`, "en"))) {
          if (piece.kind === "link" || piece.kind === "chip") used.add(piece.id);
        }
      }
      for (const id of [...Object.keys(preset.links), ...Object.keys(preset.chips)]) {
        expect(used.has(id), `${preset.id}: ${id}`).toBe(true);
      }
    }
  });

  it("says the same in both languages about what is copied, linked and numbered", () => {
    for (const preset of IOS_BROKERS) {
      for (const key of guideKeys(preset.id)) {
        const path = `settings.channels.ios.guide.${preset.id}.${key}`;
        const refs = (locale: (typeof LOCALES)[number]) =>
          parseRich(message(path, locale))
            .filter((p) => p.kind === "link" || p.kind === "chip")
            .map((p) => (p.kind === "link" || p.kind === "chip" ? `${p.kind}:${p.id}` : ""))
            .sort();
        expect(refs("en"), path).toEqual(refs("zh"));
      }
    }
  });
});

describe("isHintKind", () => {
  it("is true for the failures a wrong setting causes, and false for the rest", () => {
    for (const kind of ["rejected", "noEcho", "timeout"]) expect(isHintKind(kind)).toBe(true);
    // Not the service's doing: a missing field, a broken relay, anything unexpected.
    for (const kind of ["input", "relay", "other", ""]) expect(isHintKind(kind)).toBe(false);
  });
});
