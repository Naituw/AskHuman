import { describe, expect, it } from "vitest";
import { rippleFrame, rippleRadius } from "./unreadRipple";

describe("accepted unread ripple", () => {
  it("covers the farthest row corner at any Sidebar width", () => {
    expect(rippleRadius(224, 57, 11, 18)).toBeCloseTo(Math.hypot(213, 39) * 1.05);
    expect(rippleRadius(320, 74, 11, 18)).toBeCloseTo(Math.hypot(309, 56) * 1.05);
  });

  it("overlaps expansion and linear fade, then rests for one second", () => {
    expect(rippleFrame(0, 250)).toMatchObject({ scale: .02, opacity: 1, opaquePercent: 100 });
    expect(rippleFrame(800, 250).opacity).toBe(1);
    expect(rippleFrame(1550, 250).opacity).toBe(.5);
    expect(rippleFrame(2000, 250)).toMatchObject({ scale: 1, opaquePercent: 60 });
    expect(rippleFrame(2000, 250).opacity).toBeCloseTo(.2);
    expect(rippleFrame(2300, 250)).toMatchObject({ opacity: 0, sleepMs: 1000 });
    expect(rippleFrame(3299, 250).sleepMs).toBe(1);
    expect(rippleFrame(3300, 250)).toEqual(rippleFrame(0, 250));
  });

  it("uses the saved slow curve and quadratic physical feather, including on resize", () => {
    // Bezier parameter .5 gives time .659 and expansion .125 for the v2 curve.
    for (const radius of [180, 250, 400]) {
      const result = rippleFrame(1318, radius);
      const start = 5 / radius;
      expect(result.scale).toBeCloseTo(start + (1 - start) * .125, 4);
      const physicalFeather = (1 - result.opaquePercent / 100) * radius * result.scale;
      expect(physicalFeather).toBeCloseTo(100 * .125 ** 2, 3);
    }
  });
});
