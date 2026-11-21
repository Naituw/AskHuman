import { describe, expect, it } from "vitest";
import { i18n } from "../../i18n";
import {
  HINT_KINDS,
  IOS_BROKERS,
  brokerFill,
  brokerGuideUrl,
  brokerPreset,
  detectBroker,
  isHintKind,
} from "./iosBrokers";

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

describe("brokerGuideUrl", () => {
  it("points at the service's section of the wiki page in the reader's language", () => {
    expect(brokerGuideUrl("emqx", false)).toBe(
      "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.en.md#path-1-emqx-cloud-serverless-hosted",
    );
    expect(brokerGuideUrl("flespi", true)).toBe(
      "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.md#路径二flespi托管欧洲",
    );
    expect(brokerGuideUrl("custom", false)).toContain("#what-the-broker-must-do");
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

  it("has every name, point and hint in both interface languages", () => {
    for (const locale of ["zh", "en"] as const) {
      for (const preset of IOS_BROKERS) {
        // A name can be short ("flespi"); a point or a hint is a sentence.
        const entries = [
          ["name", 3] as const,
          ...preset.points.map((p) => [p, 12] as const),
          ...HINT_KINDS.map((kind) => [`hint.${kind}`, 12] as const),
        ];
        for (const [key, shortest] of entries) {
          const path = `settings.experimental.ios.broker.${preset.id}.${key}`;
          expect(i18n.global.te(path, locale), `${locale}: ${path}`).toBe(true);
          const text = String(i18n.global.t(path, {}, { locale }));
          // A message that fails to compile (a stray brace or bar) comes back as its own key.
          expect(text, `${locale}: ${path}`).not.toBe(path);
          expect(text.length, `${locale}: ${path}`).toBeGreaterThanOrEqual(shortest);
        }
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
