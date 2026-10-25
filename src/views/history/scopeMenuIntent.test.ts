import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  MAX_GRACE_MS,
  MIN_MOVE_PX,
  STALL_MS,
  createScopeIntent,
  pointInTriangle,
  type Rect,
  type ScopeIntent,
} from "./scopeMenuIntent";

// The geometry mirrors the real menu: project rows are 32px tall in a column spanning x 176..384
// (row i covers y 69 + 32 i .. 101 + 32 i) and the session column spans x 384..734.
const ROWS = ["all", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"];
const PANE: Rect = { left: 384, top: 65, right: 734, bottom: 361 };
const COLUMN_X = 260;
const rowCenter = (i: number) => 85 + i * 32;
const rowAt = (y: number) => ROWS[Math.min(ROWS.length - 1, Math.max(0, Math.floor((y - 69) / 32)))];

interface Rig {
  intent: ScopeIntent;
  /** Every project the guard switched to, in order. */
  log: string[];
  /** Every guard state change, in order. */
  guard: boolean[];
  /** Whether the guard is deferring a switch right now. */
  isGuarding: () => boolean;
}

function setup(options: { active?: string; target?: Rect | null } = {}): Rig {
  let active = options.active ?? "hotel";
  const log: string[] = [];
  const guard: boolean[] = [];
  let guarding = false;
  const intent = createScopeIntent({
    target: () => (options.target === undefined ? PANE : options.target),
    active: () => active,
    activate: (token) => {
      active = token;
      log.push(token);
    },
    onGuardChange: (next) => {
      guarding = next;
      guard.push(next);
    },
  });
  return { intent, log, guard, isGuarding: () => guarding };
}

/** A mouse that fires leave/enter-style notifications for the row under it, like the real view does. */
function mouseOf(intent: ScopeIntent) {
  let on: string | null = null;
  let x = 0;
  let y = 0;
  const api = {
    move(nextX: number, nextY: number) {
      const row = rowAt(nextY);
      if (row !== on) {
        if (on !== null) intent.leave(on);
        on = row;
      }
      x = nextX;
      y = nextY;
      intent.pointer(row, { x, y, pointerType: "mouse" });
    },
    /** Straight line to (x1, y1) in `step` px samples; stops before the first sample at or past `stopAtX`. */
    walk(x1: number, y1: number, step = 6, stopAtX = Infinity) {
      const x0 = x;
      const y0 = y;
      const samples = Math.max(1, Math.ceil(Math.hypot(x1 - x0, y1 - y0) / step));
      for (let i = 1; i <= samples; i++) {
        const nx = x0 + ((x1 - x0) * i) / samples;
        if (nx >= stopAtX) return;
        api.move(nx, y0 + ((y1 - y0) * i) / samples);
      }
    },
    leaveAll() {
      if (on !== null) intent.leave(on);
      on = null;
    },
  };
  return api;
}

describe("pointInTriangle", () => {
  const a = { x: 0, y: 0 };
  const b = { x: 10, y: 0 };
  const c = { x: 0, y: 10 };

  it("accepts points inside and on the border", () => {
    expect(pointInTriangle({ x: 2, y: 2 }, a, b, c)).toBe(true);
    expect(pointInTriangle({ x: 5, y: 0 }, a, b, c)).toBe(true);
    expect(pointInTriangle({ x: 0, y: 0 }, a, b, c)).toBe(true);
    expect(pointInTriangle({ x: 5, y: 5 }, a, b, c)).toBe(true);
  });

  it("rejects points outside, whichever way the triangle is wound", () => {
    expect(pointInTriangle({ x: 8, y: 8 }, a, b, c)).toBe(false);
    expect(pointInTriangle({ x: -1, y: 1 }, a, b, c)).toBe(false);
    expect(pointInTriangle({ x: 1, y: -1 }, a, b, c)).toBe(false);
    expect(pointInTriangle({ x: 8, y: 8 }, a, c, b)).toBe(false);
    expect(pointInTriangle({ x: 2, y: 2 }, a, c, b)).toBe(true);
  });

  it("treats a degenerate triangle as the line it collapses to", () => {
    expect(pointInTriangle({ x: 3, y: 4 }, a, b, b)).toBe(false);
    expect(pointInTriangle({ x: 4, y: 0 }, a, b, b)).toBe(true);
  });
});

describe("scope menu pointer intent", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("switches at once on first contact, when the pointer has no departure point yet", () => {
    const rig = setup({ active: "hotel" });
    rig.intent.pointer("bravo", { x: COLUMN_X, y: rowCenter(2), pointerType: "mouse" });
    expect(rig.log).toEqual(["bravo"]);
    expect(rig.guard).toEqual([]);
  });

  it("keeps the active project while the pointer heads diagonally for the session column", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, rowCenter(8)); // hover hotel
    mouse.walk(PANE.left - 1, 236); // up and right: crosses golf, foxtrot and echo on the way
    expect(rig.log).toEqual([]);
    expect(rig.guard).toEqual([true]);

    rig.intent.settle(); // the pointer enters the session column
    expect(rig.guard).toEqual([true, false]);
    vi.advanceTimersByTime(5000);
    expect(rig.log).toEqual([]);
  });

  it("never switches on the way to any session row, from any project row", () => {
    for (let from = 1; from < ROWS.length; from++) {
      for (let targetY = 85; targetY <= 345; targetY += 20) {
        const rig = setup({ active: ROWS[from] });
        const mouse = mouseOf(rig.intent);
        mouse.move(COLUMN_X, rowCenter(from));
        mouse.walk(560, targetY, 6, PANE.left); // stop where the pointer would enter the session column
        rig.intent.settle();
        expect(rig.log, `from ${ROWS[from]} towards y=${targetY}`).toEqual([]);
      }
    }
  });

  it("follows a vertical scrub through the project column row by row", () => {
    const rig = setup({ active: "alpha" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, rowCenter(1));
    mouse.walk(COLUMN_X, rowCenter(8));
    expect(rig.log).toEqual(["bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"]);
    expect(rig.isGuarding()).toBe(false);
  });

  it("decides a vertical move within a few samples", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.walk(COLUMN_X, 280, 8); // straight up, 8px per sample, into golf and foxtrot
    expect(rig.log[0]).toBe("golf");
  });

  it("switches at once when the pointer backs away from the session column", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(300, 330);
    mouse.move(340, 320); // heading for the session column across golf: deferred
    expect(rig.log).toEqual([]);
    expect(rig.guard).toEqual([true]);
    mouse.move(310, 285); // 30px back from the furthest point reached, now over foxtrot
    expect(rig.log).toEqual(["foxtrot"]);
    expect(rig.guard).toEqual([true, false]);
  });

  it("switches to the row the pointer stops on once it holds still", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.move(300, 315); // golf, inside the safe triangle
    expect(rig.log).toEqual([]);
    vi.advanceTimersByTime(STALL_MS - 1);
    expect(rig.log).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(rig.log).toEqual(["golf"]);
    expect(rig.guard).toEqual([true, false]);
  });

  it("keeps deferring while the pointer keeps moving, but not through jitter", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    let x = 280;
    let y = 318;
    mouse.move(x, y); // first deferred sample
    for (let i = 0; i < 4; i++) {
      vi.advanceTimersByTime(100); // under STALL_MS apart: each real movement re-arms the timer
      x += 4;
      y -= 4;
      mouse.move(x, y);
    }
    expect(rig.log).toEqual([]);

    // Now only sub-threshold jitter: it must not postpone the switch.
    const jitter = MIN_MOVE_PX - 1;
    for (let i = 0; i < 5; i++) {
      vi.advanceTimersByTime(20);
      mouse.move(x + (i % 2 ? jitter : 0), y);
    }
    expect(rig.log).toEqual([]);
    vi.advanceTimersByTime(STALL_MS - 100);
    expect(rig.log).toHaveLength(1);
  });

  it("never defers longer than the hard limit, however the pointer keeps moving", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    let x = 266;
    let y = 320;
    mouse.move(x, y); // enters golf: the guard starts here, at t = 0
    expect(rig.guard).toEqual([true]);
    for (let t = 0; t < MAX_GRACE_MS - 100; t += 100) {
      vi.advanceTimersByTime(100); // always under STALL_MS apart, and always a real movement
      x += 3;
      y -= 3.5;
      mouse.move(x, y);
    }
    vi.advanceTimersByTime(99); // t = MAX_GRACE_MS - 1
    expect(rig.log).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(rig.log).toHaveLength(1);
    expect(rig.isGuarding()).toBe(false);
  });

  it("drops a deferred switch when the pointer returns to the active row", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.move(300, 315);
    expect(rig.guard).toEqual([true]);
    mouse.move(290, 335); // back on hotel
    expect(rig.guard).toEqual([true, false]);
    vi.advanceTimersByTime(5000);
    expect(rig.log).toEqual([]);
  });

  it("does not switch to a row the pointer already left", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.move(300, 315); // golf
    mouse.leaveAll(); // off the rows, e.g. into the column padding
    vi.advanceTimersByTime(5000);
    expect(rig.log).toEqual([]);
    expect(rig.guard).toEqual([true, false]);
  });

  it("treats a touch pointer as an explicit choice", () => {
    const rig = setup({ active: "hotel" });
    rig.intent.pointer("hotel", { x: COLUMN_X, y: 330, pointerType: "mouse" });
    rig.intent.pointer("golf", { x: 300, y: 315, pointerType: "touch" });
    expect(rig.log).toEqual(["golf"]);
    expect(rig.guard).toEqual([]);
  });

  it("switches at once when there is no session column to aim at", () => {
    const rig = setup({ active: "hotel", target: null });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.move(300, 315);
    expect(rig.log).toEqual(["golf"]);
  });

  it("works the same with the session column on the left", () => {
    const left: Rect = { left: 0, top: 65, right: 400, bottom: 361 };
    const rig = setup({ active: "hotel", target: left });
    const mouse = mouseOf(rig.intent);
    mouse.move(500, 335);
    mouse.walk(410, 235, 6); // up and to the left, towards the session column
    expect(rig.log).toEqual([]);
    expect(rig.guard).toEqual([true]);

    mouse.move(500, 335); // straight back onto hotel
    mouse.walk(500, 270, 6); // straight up: no intent to cross over
    expect(rig.log[0]).toBe("golf");
  });

  it("treats a click or keyboard focus as an immediate choice and clears the guard", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.move(300, 315);
    expect(rig.guard).toEqual([true]);
    rig.intent.choose("echo");
    expect(rig.log).toEqual(["echo"]);
    expect(rig.guard).toEqual([true, false]);
    vi.advanceTimersByTime(5000);
    expect(rig.log).toEqual(["echo"]);
  });

  it("behaves like first contact again after the pointer has been in the session column", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.move(300, 315);
    rig.intent.settle(); // pointer entered the session column
    mouse.move(300, 280); // and came back over another row
    expect(rig.log).toEqual(["foxtrot"]);
  });

  it("releases its timers on dispose", () => {
    const rig = setup({ active: "hotel" });
    const mouse = mouseOf(rig.intent);
    mouse.move(COLUMN_X, 330);
    mouse.move(300, 315);
    rig.intent.dispose();
    vi.advanceTimersByTime(5000);
    expect(rig.log).toEqual([]);
    expect(rig.guard).toEqual([true, false]);
  });
});
