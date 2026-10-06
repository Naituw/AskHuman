import { describe, expect, it } from "vitest";
import { inboxDropPoint } from "./inboxCoordinates";
describe("native drop coordinates in the fixed canvas", () => {
  it("hits the same answer location before and after the sidebar is revealed", () => {
    expect(inboxDropPoint(30, 200, { canvas: { left: 606 }, sidebarWidth: 0 })).toEqual([636, 200]);
    expect(inboxDropPoint(276, 200, { canvas: { left: 606 }, sidebarWidth: 240 })).toEqual([636, 200]);
  });
  it("translates right preview locations and respects constrained sidebar widths", () => {
    expect(inboxDropPoint(826, 50, { canvas: { left: 606 }, sidebarWidth: 240 })).toEqual([1186, 50]);
    expect(inboxDropPoint(216, 50, { canvas: { left: 606 }, sidebarWidth: 180 })).toEqual([636, 50]);
  });
});
