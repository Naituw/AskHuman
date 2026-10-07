import { defineComponent, h, ref } from "vue";
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import UnreadRipple from "./UnreadRipple.vue";

describe("unread ripple lifecycle", () => {
  let wrapper: VueWrapper;
  let now: number;
  let width: number;
  let hidden: boolean;
  let visible: (value: boolean) => void;
  let resize: () => void;
  let motionChange: () => void;
  let media: { matches: boolean; addListener: ReturnType<typeof vi.fn>; removeListener: ReturnType<typeof vi.fn> };
  let frames: Map<number, FrameRequestCallback>;
  const active = ref(true);
  const disconnectIntersection = vi.fn();
  const disconnectResize = vi.fn();

  beforeEach(() => {
    now = 0; width = 224; hidden = false; active.value = true;
    frames = new Map();
    let frameId = 0;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.spyOn(performance, "now").mockImplementation(() => now);
    vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("inbox-row")) return { left: 20, top: 40, width, height: 57 } as DOMRect;
      if (this.classList.contains("inbox-dot")) return { left: 28, top: 55, width: 6, height: 6 } as DOMRect;
      return {} as DOMRect;
    });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.set(++frameId, callback); return frameId; });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    media = { matches: false, addListener: vi.fn(callback => { motionChange = callback; }), removeListener: vi.fn() };
    vi.stubGlobal("matchMedia", () => media);
    disconnectIntersection.mockClear(); disconnectResize.mockClear();
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: IntersectionObserverCallback) {
        visible = value => callback([{ isIntersecting: value } as IntersectionObserverEntry], {} as IntersectionObserver);
      }
      observe() {}
      disconnect = disconnectIntersection;
    });
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: () => void) { resize = callback; }
      observe() {}
      disconnect = disconnectResize;
    });
  });
  afterEach(() => {
    wrapper?.unmount(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  });
  function start() {
    wrapper = mount(defineComponent({ setup: () => () => h("button", { class: "inbox-row" }, [
      h(UnreadRipple, { active: active.value }), h("span", { class: "inbox-dot" }),
    ]) }));
  }
  function advance(time: number) {
    now = time;
    const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(now));
  }
  const style = () => (wrapper.find(".inbox-unread-ripple").element as HTMLElement).style;

  it("anchors to the dot, retains its phase on resize, and stops as soon as viewed", async () => {
    start(); visible(true);
    const initialRadius = Number.parseFloat(style().width) / 2;
    expect(Number.parseFloat(style().left) + initialRadius).toBeCloseTo(11);
    expect(Number.parseFloat(style().top) + initialRadius).toBeCloseTo(18);
    expect(style().backgroundImage).toContain("radial-gradient");
    advance(1550); expect(Number(style().opacity)).toBe(.5);
    width = 320; resize();
    expect(Number.parseFloat(style().width)).toBeGreaterThan(initialRadius * 2);
    expect(Number(style().opacity)).toBe(.5);
    active.value = false; await wrapper.vm.$nextTick();
    expect(frames.size).toBe(0);
    expect(style().opacity).toBe("0");
  });

  it("sleeps during rest and cancels the sleeping timer when removed", async () => {
    start(); visible(true); advance(2300);
    expect(style().opacity).toBe("0"); expect(frames.size).toBe(0);
    expect(vi.getTimerCount()).toBe(1);
    now = 3300; await vi.advanceTimersByTimeAsync(1000); advance(3300);
    expect(style().opacity).toBe("1"); expect(frames.size).toBe(1);
    advance(5600); expect(vi.getTimerCount()).toBe(1);
    wrapper.unmount();
    expect(frames.size).toBe(0); expect(vi.getTimerCount()).toBe(0);
    expect(disconnectIntersection).toHaveBeenCalledOnce();
    expect(disconnectResize).toHaveBeenCalledOnce();
    expect(media.removeListener).toHaveBeenCalledWith(motionChange);
  });

  it("pauses outside the visible list or while the document is hidden", () => {
    start(); expect(frames.size).toBe(0);
    visible(true); expect(frames.size).toBe(1);
    visible(false); expect(frames.size).toBe(0); expect(style().opacity).toBe("0");
    visible(true); hidden = true; document.dispatchEvent(new Event("visibilitychange"));
    expect(frames.size).toBe(0);
    hidden = false; document.dispatchEvent(new Event("visibilitychange"));
    expect(frames.size).toBe(1);
  });

  it("respects reduced motion, including Safari's legacy media listeners", () => {
    media.matches = true; start(); visible(true);
    expect(frames.size).toBe(0); expect(style().opacity).toBe("0");
    media.matches = false; motionChange(); expect(frames.size).toBe(1);
    media.matches = true; motionChange(); expect(frames.size).toBe(0);
  });
});
