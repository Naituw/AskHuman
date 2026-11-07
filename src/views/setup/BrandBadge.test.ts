// The wizard's logo badges: every Agent and channel it can show has one, and an id it has never
// heard of still renders something sensible instead of a blank square.
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { AgentId } from "../../lib/types";
import BrandBadge from "./BrandBadge.vue";
import { BADGES } from "./brandBadges";
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
  it("has a badge for every Agent, every guided channel and iOS", () => {
    const ids = [...Object.keys(AGENT_IDS), ...CHANNEL_DEFS.map((def) => def.id), "ios"];
    for (const id of ids) expect(BADGES[id], id).toBeDefined();
  });

  it("gives each badge a drawing or a glyph, and two gradient stops", () => {
    for (const [id, spec] of Object.entries(BADGES)) {
      expect(Boolean(spec.paths?.length) || Boolean(spec.glyph), id).toBe(true);
      expect(spec.from, id).toMatch(/^#[0-9a-f]{6}$/i);
      expect(spec.to, id).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });
});

describe("BrandBadge", () => {
  it("draws the strokes of a badge that has them", () => {
    const badge = mount(BrandBadge, { props: { id: "claude", label: "Claude Code" } });
    expect(badge.findAll("svg path")).toHaveLength(BADGES.claude.paths!.length);
    expect(badge.find(".brand-glyph").exists()).toBe(false);
    expect(badge.attributes("data-badge")).toBe("claude");
  });

  it("writes the glyph of a badge that has one", () => {
    const badge = mount(BrandBadge, { props: { id: "grok", label: "Grok" } });
    expect(badge.get(".brand-glyph").text()).toBe("G");
    expect(badge.find("svg").exists()).toBe(false);
  });

  it("falls back to the first letter of the name, in a neutral grey, for an id it does not know", () => {
    const badge = mount(BrandBadge, { props: { id: "newcomer", label: "newcomer agent" } });
    expect(badge.get(".brand-glyph").text()).toBe("N");
    expect(badge.attributes("style")).toContain("--badge-from: #6e6e73");
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
