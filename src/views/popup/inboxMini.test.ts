import { describe, expect, it } from "vitest";
import { MINI_ROWS, formAnimation, measureMini, miniMotion } from "./inboxMini";

function rect(top: number, height: number): DOMRect {
  return { top, bottom: top + height, left: 0, right: 100, width: 100, height, x: 0, y: top, toJSON: () => ({}) } as DOMRect;
}

/** A list whose rows start at `first` and are `row` tall with `gap` between them, inside a root at y = 0. */
function list(count: number, { first = 70, row = 40, gap = 4, scrollTop = 0 } = {}) {
  const root = document.createElement("div");
  root.innerHTML = `<div class="inbox-navigation">${Array.from({ length: count }, () => '<div class="inbox-entry"></div>').join("")}</div>`;
  const nav = root.querySelector<HTMLElement>(".inbox-navigation")!;
  const seen: { measure: boolean; still: boolean }[] = [];
  const watch = () => seen.push({ measure: root.hasAttribute("data-measure"), still: root.hasAttribute("data-still") });
  root.getBoundingClientRect = () => rect(0, 600);
  // The list itself stays where it is when it scrolls; its rows move.
  nav.getBoundingClientRect = () => { watch(); return rect(first, 500); };
  nav.scrollTop = scrollTop;
  root.querySelectorAll<HTMLElement>(".inbox-entry").forEach((entry, index) => {
    entry.getBoundingClientRect = () => rect(first - scrollTop + index * (row + gap), row);
  });
  return { root, seen };
}

describe("measureMini", () => {
  it("fits the window to the last visible row plus the bottom space", () => {
    const { root } = list(2);
    // navTop 70; the second row ends 70 + (40 + 4) + 40 = 154; the list itself is 84 + 16 tall.
    expect(measureMini(root)).toEqual({ height: 70 + 84 + 16, navHeight: 84 + 16 });
  });

  it("stops at the row limit; the rest of the list scrolls", () => {
    const { root } = list(8);
    const rows = MINI_ROWS;
    const last = (rows - 1) * 44 + 40;
    expect(measureMini(root)).toEqual({ height: 70 + last + 16, navHeight: last + 16 });
    expect(measureMini(root, 2)).toEqual({ height: 70 + 44 + 40 + 16, navHeight: 44 + 40 + 16 });
  });

  it("reads rows in content coordinates when the list is already scrolled", () => {
    const { root } = list(8, { scrollTop: 120 });
    const last = (MINI_ROWS - 1) * 44 + 40;
    expect(measureMini(root)).toEqual({ height: 70 + last + 16, navHeight: last + 16 });
  });

  it("measures an empty list as just its padding", () => {
    const { root } = list(0);
    expect(measureMini(root)).toEqual({ height: 70 + 16, navHeight: 16 });
  });

  it("has nothing to measure without a list", () => {
    expect(measureMini(document.createElement("div"))).toBeNull();
  });

  it("puts the root in the small layout without motion only while measuring", () => {
    const { root, seen } = list(2);
    measureMini(root);
    expect(seen.length).toBeGreaterThan(0);
    expect(seen.every(s => s.measure && s.still)).toBe(true);
    expect(root.hasAttribute("data-measure")).toBe(false);
    expect(root.hasAttribute("data-still")).toBe(false);
  });

  it("restores the root even when reading the layout throws", () => {
    const { root } = list(2);
    root.querySelector<HTMLElement>(".inbox-navigation")!.getBoundingClientRect = () => { throw new Error("detached"); };
    expect(() => measureMini(root)).toThrow("detached");
    expect(root.hasAttribute("data-measure")).toBe(false);
    expect(root.hasAttribute("data-still")).toBe(false);
  });
});

describe("formAnimation", () => {
  it("uses the approved durations and the emphasized curve", () => {
    expect(formAnimation("collapse", false)).toEqual({ durationMs: 440, curve: [0.2, 0, 0, 1] });
    expect(formAnimation("expand", false)).toEqual({ durationMs: 480, curve: [0.2, 0, 0, 1] });
    expect(formAnimation("resize", false)).toEqual({ durationMs: 240, curve: [0.2, 0, 0, 1] });
  });

  it("is instant for reduced motion", () => {
    for (const kind of ["collapse", "expand", "resize"] as const) expect(formAnimation(kind, true)).toBeUndefined();
  });

  it("hands out its own copy of the curve", () => {
    const curve = formAnimation("collapse", false)!.curve;
    curve[0] = 9;
    expect(miniMotion.curve[0]).toBe(0.2);
  });
});
