// The iOS App card, the Agent guide and docs/wiki/ios-broker.md are three summaries of the same
// facts about the brokers (docs/specs/ios-setup-experience.md §7.5). The wiki is where a fact is kept
// and dated; this test catches the card drifting from it, in both languages. (The guide's own text is
// held to the same figures by a Rust test.)
import { describe, expect, it } from "vitest";
import wikiEn from "../../../docs/wiki/ios-broker.en.md?raw";
import wikiZh from "../../../docs/wiki/ios-broker.md?raw";
import iosProject from "../../../ios/project.yml?raw";
import { i18n } from "../../i18n";
import { IOS_BROKERS, brokerPreset } from "./iosBrokers";
import { IOS_JOIN_URL } from "./iosLinks";

const WIKI = { zh: wikiZh, en: wikiEn };
const LOCALES = ["zh", "en"] as const;

function text(path: string, locale: (typeof LOCALES)[number]): string {
  return String(i18n.global.t(`settings.channels.ios.${path}`, { acl: "" }, { locale }));
}

/** GitHub's anchor for a heading: lower case, punctuation dropped, spaces to hyphens. */
function slug(heading: string): string {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-");
}

function anchors(markdown: string): string[] {
  return markdown
    .split("\n")
    .filter((line) => /^#{1,6} /.test(line))
    .map((line) => slug(line.replace(/^#+ /, "")));
}

describe("the figures of the hosted services", () => {
  // Each: where the card says it, and what the wiki says, per language.
  const FIGURES = [
    {
      what: "the free allowance of EMQX Cloud and what exceeding it costs",
      path: "guide.emqx.notice",
      zh: ["100 万连接分钟", "1 GB", "¥100"],
      en: ["1 million connection minutes", "1 GB", "¥100"],
    },
    {
      what: "the longest validity of a flespi token",
      path: "guide.flespi.step2",
      zh: ["8760 小时"],
      en: ["8760 hours"],
    },
    {
      what: "how long flespi keeps an idle account",
      path: "guide.flespi.notice",
      zh: ["60 天"],
      en: ["60 days"],
    },
  ] as const;

  for (const figure of FIGURES) {
    it(`agrees with the wiki on ${figure.what}`, () => {
      for (const locale of LOCALES) {
        for (const part of figure[locale]) {
          expect(text(figure.path, locale), `${locale} card: ${part}`).toContain(part);
          expect(WIKI[locale], `${locale} wiki: ${part}`).toContain(part);
        }
      }
    });
  }

  it("names the same ports as the wiki, and the same as the form fills in", () => {
    expect([brokerPreset("emqx").tlsPort, brokerPreset("emqx").wssPort]).toEqual([8883, 8084]);
    expect([brokerPreset("flespi").tlsPort, brokerPreset("flespi").wssPort]).toEqual([8883, 443]);
    expect([brokerPreset("custom").tlsPort, brokerPreset("custom").wssPort]).toEqual([8883, 8084]);
    for (const locale of LOCALES) {
      expect(text("guide.custom.step2", locale)).toContain("8883");
      expect(text("guide.custom.step2", locale)).toContain("8084");
      expect(text("broker.emqx.hint.timeout", locale)).toContain("8883");
      expect(text("broker.flespi.hint.timeout", locale)).toContain("443");
      for (const port of ["8883", "8084", "443"]) expect(WIKI[locale], `${locale} wiki: ${port}`).toContain(port);
    }
  });

  it("writes an account's topic the way the wiki does, in both languages", () => {
    const words = {
      zh: { username: "用户名", number: "编号" },
      en: { username: "username", number: "number" },
    };
    for (const locale of LOCALES) {
      for (const preset of IOS_BROKERS.filter((p) => p.id !== "custom")) {
        const topic = `ah/<${text(`guide.word.${preset.aclWord}`, locale)}>/#`;
        expect(topic).toBe(`ah/<${words[locale][preset.aclWord]}>/#`);
        expect(WIKI[locale], `${locale} wiki: ${topic}`).toContain(topic);
      }
      expect(WIKI[locale]).toContain("pattern readwrite ah/%u/#");
    }
    // The chip copies exactly the line the wiki and the reference configuration give.
    expect(brokerPreset("custom").chips.mosquitto).toEqual({ value: "pattern readwrite ah/%u/#" });
  });

  it("points the self-hosted steps at sections that exist in the wiki", () => {
    for (const locale of LOCALES) {
      for (const link of Object.values(brokerPreset("custom").links)) {
        if (!("anchor" in link)) continue;
        expect(anchors(WIKI[locale]), `${locale}: ${link.anchor[locale]}`).toContain(link.anchor[locale]);
      }
    }
  });
});

describe("what the card asks of the phone and the app", () => {
  it("names the iOS version the app supports", () => {
    expect(iosProject).toMatch(/deploymentTarget:\s*\n\s*iOS: "17\.0"/);
    expect(text("app.requires", "en")).toContain("iOS 17");
    expect(text("app.requires", "zh")).toContain("iOS 17");
  });

  it("sends the phone to the landing page of the Pages site, which carries the invitation link", () => {
    expect(IOS_JOIN_URL).toBe("https://naituw.github.io/AskHuman/");
  });

  it("uses the commands the Agent prompt names", () => {
    for (const locale of LOCALES) {
      expect(text("agentPrompt", locale)).toContain("AskHuman channel ios --agent-guide");
    }
  });
});
