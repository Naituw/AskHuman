import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";
import { i18n } from "../../i18n";
import type { AgentRecord, AwaySnapshot } from "../../lib/types";
import {
  awayDuration,
  emptyAway,
  glyphModes,
  ownMode,
  sessionChoices,
  sessionTone,
  sessionValue,
} from "./away";
import AwayBanner from "./AwayBanner.vue";
import AwayMenu from "./AwayMenu.vue";
import Sidebar from "./Sidebar.vue";

const snapshot = (overrides: Partial<AwaySnapshot> = {}): AwaySnapshot => ({
  global: null,
  sessions: [],
  escalatedWaiting: 0,
  ...overrides,
});

describe("console away helpers", () => {
  it("only sessions set on their own to away or mute get a glyph", () => {
    const away = snapshot({
      sessions: [
        { sessionKey: "sid:a", mode: "away", sinceMs: 1, replies: 0 },
        { sessionKey: "sid:b", mode: "mute", sinceMs: 1, replies: 0 },
        { sessionKey: "sid:c", mode: "normal", sinceMs: 1, replies: 0 },
        { sessionKey: "mcp:inst", mode: "mute", sinceMs: 1, replies: 0 },
      ],
    });
    expect(glyphModes(away)).toEqual({ a: "away", b: "mute" });
    expect(glyphModes(emptyAway())).toEqual({});
    expect(ownMode(away, "b")).toBe("mute");
    expect(ownMode(away, "zzz")).toBeNull();
  });

  it("offers normal / away / mute while nobody is away", () => {
    expect(sessionChoices(null, false)).toEqual([
      { setting: "follow", label: "optNormal", checked: true },
      { setting: "away", label: "optAway", checked: false },
      { setting: "mute", label: "optMute", checked: false },
    ]);
    expect(sessionChoices("away", false).map((c) => c.checked)).toEqual([false, true, false]);
    expect(sessionChoices("mute", false).map((c) => c.checked)).toEqual([false, false, true]);
  });

  it("offers follow / always ask / mute while away mode is on for everyone", () => {
    expect(sessionChoices(null, true)).toEqual([
      { setting: "follow", label: "optFollow", checked: true },
      { setting: "normal", label: "optAlways", checked: false },
      { setting: "mute", label: "optMute", checked: false },
    ]);
    expect(sessionChoices("normal", true).map((c) => c.checked)).toEqual([false, true, false]);
    // A redundant own "away" reads as following the global switch.
    expect(sessionChoices("away", true).map((c) => c.checked)).toEqual([true, false, false]);
  });

  it("names the pill's value and tone", () => {
    expect(sessionValue(null, false)).toBe("valueNormal");
    expect(sessionValue(null, true)).toBe("valueFollow");
    expect(sessionValue("away", false)).toBe("valueAway");
    expect(sessionValue("normal", true)).toBe("valueAlways");
    expect(sessionValue("mute", true)).toBe("valueMute");
    expect(sessionTone("away", false)).toBe("away");
    expect(sessionTone("away", true)).toBe("plain");
    expect(sessionTone("mute", false)).toBe("mute");
    expect(sessionTone(null, true)).toBe("plain");
  });

  it("tells the duration to the minute", () => {
    const t0 = 1_700_000_000_000;
    expect(awayDuration(t0, t0 + 59_000)).toEqual({ kind: "now" });
    expect(awayDuration(t0, t0 + 5 * 60_000 + 10)).toEqual({ kind: "m", m: 5 });
    expect(awayDuration(t0, t0 + 83 * 60_000)).toEqual({ kind: "hm", h: 1, m: 23 });
    // A clock that went backwards never shows a negative time.
    expect(awayDuration(t0, t0 - 10_000)).toEqual({ kind: "now" });
  });
});

describe("AwayBanner", () => {
  beforeEach(() => {
    i18n.global.locale.value = "zh";
  });

  const mountBanner = (props: { replies?: number; waiting?: number; since?: number } = {}) =>
    mount(AwayBanner, {
      props: {
        global: { sinceMs: props.since ?? 0, replies: props.replies ?? 6 },
        waiting: props.waiting ?? 0,
        nowMs: 83 * 60_000,
      },
      global: { plugins: [i18n] },
    });

  it("says how long and how many auto-replies, and offers the record and the way out", async () => {
    const wrapper = mountBanner();
    expect(wrapper.text()).toContain("离开模式开启中");
    expect(wrapper.text()).toContain("所有 Agent · 已 1 小时 23 分 · 自动回复 6 次");
    expect(wrapper.text()).not.toContain("待答");
    await wrapper.get('[data-testid="away-banner-record"]').trigger("click");
    await wrapper.get('[data-testid="away-banner-end"]').trigger("click");
    expect(wrapper.emitted("viewRecord")).toHaveLength(1);
    expect(wrapper.emitted("end")).toHaveLength(1);
  });

  it("mentions escalated questions that are waiting", () => {
    const wrapper = mountBanner({ waiting: 2 });
    expect(wrapper.text()).toContain("2 个升级提问待答");
  });

  it("speaks English", () => {
    i18n.global.locale.value = "en";
    const wrapper = mountBanner({ replies: 1 });
    expect(wrapper.text()).toContain("Away mode is on");
    expect(wrapper.text()).toContain("All Agents · for 1 h 23 min · auto-replies: 1");
    expect(wrapper.text()).toContain("End away mode");
  });
});

describe("AwayMenu", () => {
  beforeEach(() => {
    i18n.global.locale.value = "zh";
  });

  const mountMenu = (own: "away" | "mute" | "normal" | null, globalOn: boolean) =>
    mount(AwayMenu, { props: { own, globalOn }, global: { plugins: [i18n] }, attachTo: document.body });

  it("shows the current value and opens the three choices", async () => {
    const wrapper = mountMenu("away", false);
    expect(wrapper.get('[data-testid="away-menu-value"]').text()).toBe("离开中");
    expect(wrapper.find('[role="menu"]').exists()).toBe(false);
    await wrapper.get('[data-testid="away-menu-button"]').trigger("click");
    const items = wrapper.findAll('[role="menuitemradio"]');
    expect(items.map((i) => i.text())).toEqual(["正常", "离开模式（此会话）", "不再使用 AskHuman（此会话）"]);
    expect(items.map((i) => i.attributes("aria-checked"))).toEqual(["false", "true", "false"]);
    wrapper.unmount();
  });

  it("picking a choice closes the menu and reports the setting", async () => {
    const wrapper = mountMenu(null, false);
    await wrapper.get('[data-testid="away-menu-button"]').trigger("click");
    await wrapper.get('[data-setting="mute"]').trigger("click");
    expect(wrapper.emitted("set")).toEqual([["mute"]]);
    expect(wrapper.find('[role="menu"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("while away mode is on for everyone the choices are about following it", async () => {
    const wrapper = mountMenu(null, true);
    expect(wrapper.get('[data-testid="away-menu-value"]').text()).toBe("跟随全局");
    await wrapper.get('[data-testid="away-menu-button"]').trigger("click");
    expect(wrapper.findAll('[role="menuitemradio"]').map((i) => i.text())).toEqual([
      "跟随全局",
      "始终正常提问",
      "不再使用 AskHuman（此会话）",
    ]);
    await wrapper.get('[data-setting="normal"]').trigger("click");
    expect(wrapper.emitted("set")).toEqual([["normal"]]);
    wrapper.unmount();
  });

  it("closes on Escape and on a click elsewhere", async () => {
    const wrapper = mountMenu(null, false);
    await wrapper.get('[data-testid="away-menu-button"]').trigger("click");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[role="menu"]').exists()).toBe(false);
    await wrapper.get('[data-testid="away-menu-button"]').trigger("click");
    expect(wrapper.find('[role="menu"]').exists()).toBe(true);
    document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[role="menu"]').exists()).toBe(false);
    wrapper.unmount();
  });
});

describe("Sidebar away glyphs", () => {
  beforeEach(() => {
    i18n.global.locale.value = "zh";
    localStorage.clear();
  });

  const record = (sessionId: string): AgentRecord => ({
    seq: 1,
    kind: "codex",
    sessionId,
    title: sessionId,
    cwd: "/tmp/project",
    startedAt: 1_700_000_000,
    lastActivity: 1_700_000_010,
    state: "idle",
  });

  it("marks only the sessions that were set on their own", () => {
    const wrapper = mount(Sidebar, {
      props: {
        groups: [
          {
            key: "/tmp/project",
            label: "project",
            path: "/tmp/project",
            items: [record("a"), record("b"), record("c")],
          },
        ],
        recent: [],
        selectedId: null,
        newTaskSupported: false,
        todoCounts: {},
        nowMs: 1_700_000_020_000,
        awayModes: { a: "away", b: "mute" },
      },
      global: { plugins: [i18n] },
    });
    const rows = wrapper.findAll(".sess");
    expect(rows[0].get("[data-glyph]").attributes("data-glyph")).toBe("away");
    expect(rows[0].get("[data-glyph]").attributes("title")).toBe("此会话处于离开模式");
    expect(rows[1].get("[data-glyph]").attributes("data-glyph")).toBe("mute");
    expect(rows[1].get("[data-glyph]").attributes("title")).toBe("此会话不再使用 AskHuman");
    expect(rows[2].find("[data-glyph]").exists()).toBe(false);
  });
});
