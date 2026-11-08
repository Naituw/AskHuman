import { mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { AgentDetailFrame, LatestAgentText as Candidate } from "../../lib/types";
import WatchPane from "./WatchPane.vue";

function frame(text: string, kind: Candidate["kind"] = "thinking"): AgentDetailFrame {
  return { type: "watchFrame", sessionId: "one", seq: 1, kindLabel: "Cursor", phase: "working",
    text: "Old prose", latestText: { id: text, revision: text, kind, text, truncated: false },
    steps: [], stepsOmitted: 0, todos: [] };
}

function mountPane(value = frame("First thought")) {
  return mount(WatchPane, { props: { frame: value }, global: { plugins: [i18n],
    stubs: { MarkdownContent: { props: ["source"], template: '<div class="markdown">{{ source }}</div>' } } } });
}

describe("stable latest Agent text", () => {
  beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(0); i18n.global.locale.value = "en"; });
  afterEach(() => { vi.useRealTimers(); });

  it("shows thoughts immediately and merges dense updates against a fixed deadline", async () => {
    const pane = mountPane();
    expect(pane.get(".thought").text()).toBe("First thought");
    await vi.advanceTimersByTimeAsync(1000);
    await pane.setProps({ frame: frame("Second thought") });
    await vi.advanceTimersByTimeAsync(8000);
    await pane.setProps({ frame: frame("Newest thought") });
    expect(pane.get(".thought").text()).toBe("First thought");
    await vi.advanceTimersByTimeAsync(1000);
    expect(pane.get(".thought").text()).toBe("Newest thought");
    pane.unmount();
  });

  it("keeps expanded reading stable while tools refresh and updates on request", async () => {
    const pane = mountPane();
    await pane.get(".text-actions button").trigger("click");
    const next = frame("Next thought");
    next.steps = [{ kind: "read", object: "camera.swift", state: "running" }];
    await pane.setProps({ frame: next });
    await vi.advanceTimersByTimeAsync(20000);
    expect(pane.get(".thought").text()).toBe("First thought");
    expect(pane.get(".steps").text()).toContain("camera.swift");
    await pane.get(".new-content").trigger("click");
    expect(pane.get(".thought").text()).toBe("Next thought");
    expect(pane.get(".text-actions button").attributes("aria-expanded")).toBe("true");
    await pane.setProps({ frame: frame("New prose", "assistant") });
    await vi.advanceTimersByTimeAsync(10000);
    expect(pane.get(".thought").text()).toBe("Next thought");
    await pane.get(".text-actions button").trigger("click");
    expect(pane.get(".markdown").text()).toBe("New prose");
    pane.unmount();
  });

  it("locks text selection and resets immediately on a session switch", async () => {
    const pane = mountPane();
    await pane.get(".text-body").trigger("pointerdown");
    await pane.setProps({ frame: frame("Pending thought") });
    await vi.advanceTimersByTimeAsync(20000);
    expect(pane.get(".thought").text()).toBe("First thought");
    const next = frame("Another session"); next.sessionId = "two";
    await pane.setProps({ frame: next });
    expect(pane.get(".thought").text()).toBe("Another session");
    expect(pane.find(".new-content").exists()).toBe(false);
    pane.unmount();
  });

  it("uses legacy prose and ignores duplicate frames and temporary missing candidates", async () => {
    const initial = frame("Prose", "assistant"); delete initial.latestText; initial.text = "Legacy text";
    const pane = mountPane(initial);
    expect(pane.get(".markdown").text()).toBe("Legacy text");
    await pane.setProps({ frame: { ...initial } });
    await pane.setProps({ frame: { ...initial, text: null } });
    await vi.advanceTimersByTimeAsync(20000);
    expect(pane.get(".markdown").text()).toBe("Legacy text");
    pane.unmount();
  });

  it("does not fall back to streaming prose when a new Mac has no completed candidate yet", async () => {
    const initial = frame("Growing prose", "assistant"); initial.latestText = null;
    const pane = mountPane(initial);
    expect(pane.find(".markdown").exists()).toBe(false);
    await pane.setProps({ frame: frame("Completed thought") });
    expect(pane.get(".thought").text()).toBe("Completed thought");
    await pane.setProps({ frame: initial });
    expect(pane.get(".thought").text()).toBe("Completed thought");
    pane.unmount();
  });

  it("locks upward reading gestures but ignores layout-induced scroll events", async () => {
    const pane = mount({ components: { WatchPane }, props: ["frame"],
      template: '<div class="scroll-host" style="overflow-y: auto"><WatchPane :frame="frame" /></div>' },
    { props: { frame: frame("First thought") }, global: { plugins: [i18n] } });
    await pane.setProps({ frame: frame("Second thought") });
    await pane.get(".scroll-host").trigger("scroll");
    await vi.advanceTimersByTimeAsync(10000);
    expect(pane.get(".thought").text()).toBe("Second thought");
    await pane.get(".scroll-host").trigger("wheel", { deltaY: -20 });
    await pane.setProps({ frame: frame("Third thought") });
    await vi.advanceTimersByTimeAsync(10000);
    expect(pane.get(".thought").text()).toBe("Second thought");
    expect(pane.get(".new-content").text()).toBe("New content");
    pane.unmount();
  });
});
