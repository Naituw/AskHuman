// The wizard's logo badges: every Agent and channel it can show has one, the Agents are drawn from
// the iOS app's own logo files, and an id it has never heard of still renders something sensible
// instead of a blank square.
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import antigravitySvg from "../../../ios/Tools/agent-logos/antigravity.svg?raw";
import claudeSvg from "../../../ios/Tools/agent-logos/claude.svg?raw";
import codexSvg from "../../../ios/Tools/agent-logos/codex.svg?raw";
import cursorSvg from "../../../ios/Tools/agent-logos/cursor.svg?raw";
import grokSvg from "../../../ios/Tools/agent-logos/grok.svg?raw";
import piSvg from "../../../ios/Tools/agent-logos/pi.svg?raw";
import type { AgentId } from "../../lib/types";
import BrandBadge from "./BrandBadge.vue";
import { BADGES, svgUrl } from "./brandBadges";
import { CHANNEL_DEFS } from "./channelDefs";

// Spelling every id out as a record key makes the compiler flag a new Agent that has no badge yet.
const AGENT_IDS: Record<AgentId, true> = {
  cursor: true,
  claude: true,
  codex: true,
  grok: true,
  pi: true,
  antigravity: true,
};

describe("badge registry", () => {
  it("has a badge for every Agent, every guided channel, iOS and the local popup", () => {
    const ids = [...Object.keys(AGENT_IDS), ...CHANNEL_DEFS.map((def) => def.id), "ios", "popup"];
    for (const id of ids) expect(BADGES[id], id).toBeDefined();
  });

  it("gives each badge a tile, a hairline and a mark that is an SVG with a grid", () => {
    for (const [id, spec] of Object.entries(BADGES)) {
      expect(spec.tile, id).toMatch(/^(#[0-9a-f]{3,6}|linear-gradient\(.+\))$/i);
      expect(spec.hairline, id).toMatch(/^rgba\(/);
      expect(spec.mark.svg, id).toMatch(/^<svg [^>]*viewBox="[^"]+"/);
      expect(spec.mark.scale, id).toBeGreaterThan(0.3);
      expect(spec.mark.scale, id).toBeLessThanOrEqual(0.85);
      if (spec.mark.paint !== undefined) {
        expect(spec.mark.paint, id).toMatch(/^(#[0-9a-f]{3,6}|linear-gradient\(.+\))$/i);
      }
    }
  });

  it("draws the Agents from the iOS app's logo files, so a logo exists once", () => {
    expect(BADGES.cursor.mark.svg).toBe(cursorSvg);
    expect(BADGES.claude.mark.svg).toBe(claudeSvg);
    expect(BADGES.codex.mark.svg).toBe(codexSvg);
    expect(BADGES.grok.mark.svg).toBe(grokSvg);
    expect(BADGES.pi.mark.svg).toBe(piSvg);
    expect(BADGES.antigravity.mark.svg).toBe(antigravitySvg);
  });

  it("paints Codex as its app icon does: a white tile, the cloud in a violet-to-blue gradient", () => {
    expect(BADGES.codex.tile).toBe("#fff");
    expect(BADGES.codex.mark.paint).toContain("linear-gradient");
    expect(BADGES.codex.mark.paint).toContain("#3941ff");
    // A white tile needs a dark hairline to stand apart from a light page.
    expect(BADGES.codex.hairline).toContain("rgba(0, 0, 0");
  });

  it("paints a one-colour mark white on a coloured tile, and leaves a mark in its own colours alone on white", () => {
    for (const id of ["cursor", "claude", "grok", "dingding", "ios", "popup"]) {
      expect(BADGES[id].mark.paint, id).toBe("#fff");
      expect(BADGES[id].tile, id).toContain("linear-gradient");
    }
    // Pi stays on a dark tile; the others show the product's own colours on a white one.
    expect(BADGES.pi.mark.paint).toBeUndefined();
    for (const id of ["antigravity", "telegram", "slack", "feishu"]) {
      expect(BADGES[id].mark.paint, id).toBeUndefined();
      expect(BADGES[id].tile, id).toBe("#fff");
      expect(BADGES[id].hairline, id).toContain("rgba(0, 0, 0");
    }
  });

  it("draws Feishu as the Lark bird in its three brand colours", () => {
    const colours = new Set(BADGES.feishu.mark.svg.match(/fill="#[0-9a-f]{6}"/gi));
    expect([...colours].sort()).toEqual(['fill="#00d6b9"', 'fill="#163c9a"', 'fill="#3370ff"']);
  });

  it("draws Slack in its four colours and Antigravity as a blue arch warmed towards its top", () => {
    const slack = new Set(BADGES.slack.mark.svg.match(/fill="#[0-9a-f]{6}"/gi));
    expect(slack.size).toBe(4);
    const arch = BADGES.antigravity.mark.svg;
    expect(arch).toContain('fill="#3186ff"');
    // Drawn from gradients only: no filter, so it renders alike in every web view and as a vector PDF.
    expect(arch).not.toContain("<filter");
    expect(arch).toContain("linearGradient");
  });
});

describe("svgUrl", () => {
  it("carries the SVG itself, URL-encoded, and gives the same string again", () => {
    const url = svgUrl(cursorSvg);
    expect(url.startsWith('url("data:image/svg+xml,')).toBe(true);
    expect(url.endsWith('")')).toBe(true);
    expect(url).not.toContain("<");
    const encoded = url.slice('url("data:image/svg+xml,'.length, -2);
    expect(decodeURIComponent(encoded)).toBe(cursorSvg.trim());
    expect(svgUrl(cursorSvg)).toBe(url);
  });
});

describe("BrandBadge", () => {
  it("paints a one-colour mark through the shape of its SVG", () => {
    const badge = mount(BrandBadge, { props: { id: "claude", label: "Claude Code" } });
    expect(badge.find(".brand-mark").exists()).toBe(true);
    expect(badge.find(".brand-glyph").exists()).toBe(false);
    expect(badge.classes()).not.toContain("own-colours");
    expect(badge.attributes("data-badge")).toBe("claude");
    const style = badge.attributes("style") ?? "";
    expect(style).toContain("--mark-paint");
    expect(style).toContain("--mark-url");
    expect(style).toContain("--badge-tile");
  });

  it("shows a mark in its own colours as the image it is, with nothing painted over it", () => {
    const badge = mount(BrandBadge, { props: { id: "pi", label: "Pi" } });
    expect(badge.classes()).toContain("own-colours");
    expect(badge.attributes("style") ?? "").not.toContain("--mark-paint");
  });

  it("falls back to the first letter of the name, on a neutral grey tile, for an id it does not know", () => {
    const badge = mount(BrandBadge, { props: { id: "newcomer", label: "newcomer agent" } });
    expect(badge.get(".brand-glyph").text()).toBe("N");
    expect(badge.find(".brand-mark").exists()).toBe(false);
    expect(badge.attributes("style")).toContain("#6e6e73");
  });

  it("falls back to the id itself when there is no name", () => {
    const badge = mount(BrandBadge, { props: { id: "zed" } });
    expect(badge.get(".brand-glyph").text()).toBe("Z");
  });

  it("is decorative: hidden from assistive tech, in the requested size", () => {
    const medium = mount(BrandBadge, { props: { id: "cursor" } });
    expect(medium.attributes("aria-hidden")).toBe("true");
    expect(medium.classes()).toContain("md");
    const small = mount(BrandBadge, { props: { id: "cursor", size: "sm" } });
    expect(small.classes()).toContain("sm");
  });
});
