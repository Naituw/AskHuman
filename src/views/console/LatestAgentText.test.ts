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

function mountPane(value = frame("First thought"), attachTo?: HTMLElement) {
  return mount(WatchPane, { props: { frame: value }, attachTo, global: { plugins: [i18n],
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

  // Opening the text only makes it taller (2026-11-15; it used to hold the words until it was closed, which made "New
  // content" the only way to see what the Agent said next). The words are held for the moment after the click, as after a
  // pointer, and then the next text replaces them by itself like it does a closed one.
  it("replaces an opened text by itself after the moment that follows opening it, and keeps it open", async () => {
    const pane = mountPane();
    await vi.advanceTimersByTimeAsync(11000);
    await pane.get(".text-actions button").trigger("click");
    const next = frame("Next thought");
    next.steps = [{ kind: "read", object: "camera.swift", state: "running" }];
    await pane.setProps({ frame: next });
    expect(pane.get(".thought").text()).toBe("First thought");
    expect(pane.get(".steps").text()).toContain("camera.swift");
    await vi.advanceTimersByTimeAsync(2900);
    expect(pane.get(".thought").text()).toBe("First thought");
    await vi.advanceTimersByTimeAsync(200);
    expect(pane.get(".thought").text()).toBe("Next thought");
    expect(pane.get(".text-actions button").attributes("aria-expanded")).toBe("true");
    expect(pane.find(".new-content").exists()).toBe(false);
    pane.unmount();
  });

  it("waits no longer than the replacement interval for an opened text that nothing touches", async () => {
    const pane = mountPane();
    await vi.advanceTimersByTimeAsync(1000);
    await pane.get(".text-actions button").trigger("click");
    await vi.advanceTimersByTimeAsync(4000);
    await pane.setProps({ frame: frame("Next thought") });
    expect(pane.find(".new-content").exists()).toBe(false);
    await vi.advanceTimersByTimeAsync(4900);
    expect(pane.get(".thought").text()).toBe("First thought");
    await vi.advanceTimersByTimeAsync(200);
    expect(pane.get(".thought").text()).toBe("Next thought");
    pane.unmount();
  });

  it("holds an opened text while a pointer is on it, and a tap on \"New content\" keeps it open", async () => {
    const pane = mountPane();
    await pane.get(".text-actions button").trigger("click");
    await pane.get(".text-body").trigger("pointerdown");
    await pane.setProps({ frame: frame("Next thought") });
    await vi.advanceTimersByTimeAsync(30000);
    expect(pane.get(".thought").text()).toBe("First thought");
    await pane.get(".new-content").trigger("click");
    expect(pane.get(".thought").text()).toBe("Next thought");
    expect(pane.get(".text-actions button").attributes("aria-expanded")).toBe("true");
    pane.unmount();
  });

  it("shows the newest text at once when an opened text is closed", async () => {
    const pane = mountPane();
    await pane.get(".text-actions button").trigger("click");
    await pane.setProps({ frame: frame("Next thought") });
    expect(pane.get(".thought").text()).toBe("First thought");
    await pane.get(".text-actions button").trigger("click");
    expect(pane.get(".thought").text()).toBe("Next thought");
    expect(pane.get(".text-actions button").attributes("aria-expanded")).toBe("false");
    expect(pane.find(".new-content").exists()).toBe(false);
    await pane.setProps({ frame: frame("New prose", "assistant") });
    await vi.advanceTimersByTimeAsync(9900);
    expect(pane.get(".thought").text()).toBe("Next thought");
    await vi.advanceTimersByTimeAsync(200);
    expect(pane.get(".markdown").text()).toBe("New prose");
    pane.unmount();
  });

  it("holds the text while a pointer is on it and resets immediately on a session switch", async () => {
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

  const pointerUp = () => document.dispatchEvent(new Event("pointerup"));

  it("holds the text for a moment after the pointer leaves it, then replaces it by itself", async () => {
    const pane = mountPane();
    await vi.advanceTimersByTimeAsync(11000);
    await pane.get(".text-body").trigger("pointerdown");
    await pane.setProps({ frame: frame("Next thought") });
    expect(pane.get(".thought").text()).toBe("First thought");
    expect(pane.get(".new-content").text()).toBe("New content");
    await vi.advanceTimersByTimeAsync(30000);
    expect(pane.get(".thought").text()).toBe("First thought");

    pointerUp();
    await vi.advanceTimersByTimeAsync(2900);
    expect(pane.get(".thought").text()).toBe("First thought");
    expect(pane.find(".new-content").exists()).toBe(true);
    await vi.advanceTimersByTimeAsync(200);
    expect(pane.get(".thought").text()).toBe("Next thought");
    expect(pane.find(".new-content").exists()).toBe(false);
    pane.unmount();
  });

  it("renews the lease when the pointer comes back and never shortens the replacement interval", async () => {
    const pane = mountPane();
    await vi.advanceTimersByTimeAsync(11000);
    await pane.get(".text-body").trigger("pointerdown");
    pointerUp();
    await pane.setProps({ frame: frame("Next thought") });
    await vi.advanceTimersByTimeAsync(2000);
    await pane.get(".text-body").trigger("pointerdown");
    await vi.advanceTimersByTimeAsync(5000);
    pointerUp();
    await vi.advanceTimersByTimeAsync(2900);
    expect(pane.get(".thought").text()).toBe("First thought");
    await vi.advanceTimersByTimeAsync(200);
    expect(pane.get(".thought").text()).toBe("Next thought");
    pane.unmount();

    // A lease that ends before the interval does is not what the new text waits for.
    const fresh = mountPane(frame("Fresh thought"));
    await vi.advanceTimersByTimeAsync(1000);
    await fresh.get(".text-body").trigger("pointerdown");
    pointerUp();
    await fresh.setProps({ frame: frame("Later thought") });
    await vi.advanceTimersByTimeAsync(5000);
    expect(fresh.get(".thought").text()).toBe("Fresh thought");
    expect(fresh.find(".new-content").exists()).toBe(false);
    await vi.advanceTimersByTimeAsync(4000);
    expect(fresh.get(".thought").text()).toBe("Later thought");
    fresh.unmount();
  });

  it("holds the text for a moment after a wheel turn over it", async () => {
    const pane = mountPane();
    await vi.advanceTimersByTimeAsync(11000);
    await pane.get(".text-body").trigger("wheel", { deltaY: 20 });
    await pane.setProps({ frame: frame("Next thought") });
    await vi.advanceTimersByTimeAsync(2900);
    expect(pane.get(".thought").text()).toBe("First thought");
    await vi.advanceTimersByTimeAsync(200);
    expect(pane.get(".thought").text()).toBe("Next thought");
    pane.unmount();
  });

  it("holds the text for a moment after its selection changed, and not for a selection elsewhere", async () => {
    const elsewhere = document.createElement("p");
    elsewhere.textContent = "Not the text";
    document.body.append(elsewhere);
    const pane = mountPane(frame("First thought"), document.body);
    await vi.advanceTimersByTimeAsync(11000);
    window.getSelection()?.selectAllChildren(elsewhere);
    document.dispatchEvent(new Event("selectionchange"));
    await pane.setProps({ frame: frame("Next thought") });
    expect(pane.get(".thought").text()).toBe("Next thought");

    await vi.advanceTimersByTimeAsync(11000);
    window.getSelection()?.selectAllChildren(pane.get(".thought").element);
    document.dispatchEvent(new Event("selectionchange"));
    await pane.setProps({ frame: frame("Last thought") });
    await vi.advanceTimersByTimeAsync(2900);
    expect(pane.get(".thought").text()).toBe("Next thought");
    await vi.advanceTimersByTimeAsync(200);
    expect(pane.get(".thought").text()).toBe("Last thought");
    window.getSelection()?.removeAllRanges();
    pane.unmount();
    elsewhere.remove();
  });

  it("is not held by what happens elsewhere on the page", async () => {
    const pane = mount({ components: { WatchPane }, props: ["frame"],
      template: '<div class="scroll-host" style="overflow-y: auto"><WatchPane :frame="frame" /></div>' },
    { props: { frame: frame("First thought") }, global: { plugins: [i18n] } });
    await vi.advanceTimersByTimeAsync(11000);
    await pane.setProps({ frame: frame("Second thought") });
    expect(pane.get(".thought").text()).toBe("Second thought");
    await pane.get(".scroll-host").trigger("pointerdown");
    await pane.get(".scroll-host").trigger("scroll");
    await pane.get(".scroll-host").trigger("wheel", { deltaY: -20 });
    await pane.get(".scroll-host").trigger("keydown", { key: "ArrowUp" });
    await vi.advanceTimersByTimeAsync(10000);
    await pane.setProps({ frame: frame("Third thought") });
    expect(pane.get(".thought").text()).toBe("Third thought");
    expect(pane.find(".new-content").exists()).toBe(false);
    pane.unmount();
  });
});
