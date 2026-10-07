import { describe, expect, it, vi } from "vitest";
import { arrivalAnimator, arrivalFrame, type ArrivalFrame } from "./inboxArrival";

const geometry = { origin: { x: 780, y: 310 }, target: { x: 383, y: 151 } };
describe("arrival motion", () => {
  it("lands exactly on the live dot as an opaque 10px circle", () => {
    const moved = { ...geometry, target: { x: 405, y: 228 } };
    const frame = arrivalFrame(1280, moved);
    expect(frame).toMatchObject({ ...moved.target, opacity: 1, morph: 1, plateOpacity: 0, haloOpacity: 0, done: true });
    expect(frame.scale * 88 * 20 / 24).toBeCloseTo(10);
    expect(arrivalFrame(500, geometry)).toMatchObject({ ...geometry.origin, scale: 1, opacity: 1, morph: 0 });
  });
  it("runs concurrent timelines and cancels one without changing the other", () => {
    let now = 0, sequence = 0;
    const callbacks = new Map<number, FrameRequestCallback>();
    const painted = new Map<string, ArrivalFrame>();
    const settle = vi.fn();
    const animator = arrivalAnimator({ measure: () => geometry, paint: (id, state) => painted.set(id, state), settle,
      now: () => now, schedule: cb => { callbacks.set(++sequence, cb); return sequence; }, cancel: id => { callbacks.delete(id); } });
    function step(time: number) { now = time; const batch = [...callbacks.values()]; callbacks.clear(); batch.forEach(cb => cb(time)); }
    animator.play("a"); step(500); animator.play("b"); step(900);
    expect(painted.get("a")!.x).toBeLessThan(geometry.origin.x);
    expect(painted.get("b")!.x).toBe(geometry.origin.x);
    animator.cancel("a"); step(1800);
    expect(settle.mock.calls).toEqual([["a", "cancelled"], ["b", "landed"]]);
    expect(callbacks.size).toBe(0);
    step(2000); expect(settle).toHaveBeenCalledTimes(2);
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
