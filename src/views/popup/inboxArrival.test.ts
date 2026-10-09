import { describe, expect, it, vi } from "vitest";
import { arrivalAnimator, arrivalFrame, arrivalTiming, type ArrivalFrame } from "./inboxArrival";

const geometry = { origin: { x: 780, y: 310 }, target: { x: 383, y: 151 } };
const flyAt = arrivalTiming.appear + arrivalTiming.first + arrivalTiming.second;
const landedAt = flyAt + arrivalTiming.fly;
describe("arrival motion", () => {
  it("lands exactly on the live dot as an opaque 10px circle", () => {
    const moved = { ...geometry, target: { x: 405, y: 228 } };
    const frame = arrivalFrame(landedAt, moved);
    expect(frame).toMatchObject({ ...moved.target, opacity: 1, morph: 1, plateOpacity: 0, haloOpacity: 0, done: true });
    expect(frame.scale * 88 * 20 / 24).toBeCloseTo(10);
    expect(arrivalFrame(arrivalTiming.appear, geometry)).toMatchObject({ ...geometry.origin, scale: 1, opacity: 1, morph: 0 });
  });
  it("bounces twice before curling downward into the flight path", () => {
    expect(arrivalFrame(710, geometry).y).toBeCloseTo(geometry.origin.y - 52);
    expect(arrivalFrame(1180, geometry).stretchY).toBeCloseTo(.93);
    expect(arrivalFrame(1616, geometry).y).toBeCloseTo(geometry.origin.y - 32);
    expect(arrivalFrame(flyAt + 300, geometry).y).toBeGreaterThan(geometry.origin.y);
    expect(arrivalFrame(flyAt + 600, geometry).x).toBeLessThan(geometry.origin.x);
  });
  it("keeps position, shape and falling velocity continuous at both handoffs", () => {
    for (const time of [arrivalTiming.appear + arrivalTiming.first, flyAt]) {
      const before = arrivalFrame(time - .01, geometry), at = arrivalFrame(time, geometry), after = arrivalFrame(time + .01, geometry);
      expect(before.y).toBeCloseTo(at.y, 2); expect(after.y).toBeCloseTo(at.y, 2);
      expect((at.y - before.y) / .01).toBeCloseTo((after.y - at.y) / .01, 3);
      expect(before.stretchY).toBeCloseTo(after.stretchY, 5);
      expect((after.x - at.x) / .01).toBeCloseTo(0, 4);
    }
  });
  it("keeps the hook finite and descending when the live target moves far below the origin", () => {
    const lower = { ...geometry, target: { x: 150, y: 900 } };
    expect(arrivalFrame(flyAt + 1, lower).y).toBeGreaterThan(arrivalFrame(flyAt, lower).y);
    for (let time = flyAt; time <= landedAt; time += 10) {
      const frame = arrivalFrame(time, lower);
      expect([frame.x, frame.y, frame.scale].every(Number.isFinite)).toBe(true);
      expect(frame.x).toBeGreaterThanOrEqual(lower.target.x);
      expect(frame.x).toBeLessThanOrEqual(lower.origin.x);
    }
    expect(arrivalFrame(landedAt, lower)).toMatchObject({ ...lower.target, done: true });
  });
  it("runs concurrent timelines and cancels one without changing the other", () => {
    let now = 0, sequence = 0;
    const callbacks = new Map<number, FrameRequestCallback>();
    const painted = new Map<string, ArrivalFrame>();
    const settle = vi.fn();
    const animator = arrivalAnimator({ measure: () => geometry, paint: (id, state) => painted.set(id, state), settle,
      now: () => now, schedule: cb => { callbacks.set(++sequence, cb); return sequence; }, cancel: id => { callbacks.delete(id); } });
    function step(time: number) { now = time; const batch = [...callbacks.values()]; callbacks.clear(); batch.forEach(cb => cb(time)); }
    animator.play("a"); step(500); animator.play("b"); step(flyAt + 100);
    expect(painted.get("a")!.x).toBeLessThan(geometry.origin.x);
    expect(painted.get("b")!.x).toBe(geometry.origin.x);
    animator.cancel("a"); step(landedAt + 500);
    expect(settle.mock.calls).toEqual([["a", "cancelled"], ["b", "landed"]]);
    expect(callbacks.size).toBe(0);
    step(landedAt + 1000); expect(settle).toHaveBeenCalledTimes(2);
  });
  it("settles unavailable targets and releases all pending callbacks", () => {
    const schedule = vi.fn((_callback: FrameRequestCallback) => 9), cancel = vi.fn(), settle = vi.fn();
    const animator = arrivalAnimator({ measure: () => null, paint: vi.fn(), settle, now: () => 0, schedule, cancel });
    animator.play("removed"); schedule.mock.calls[0][0](100);
    expect(settle).toHaveBeenCalledWith("removed", "unavailable");
    animator.play("c"); animator.play("d"); animator.clear();
    expect(cancel).toHaveBeenCalledWith(9);
    expect(settle.mock.calls.slice(1)).toEqual([["c", "cancelled"], ["d", "cancelled"]]);
  });
});
