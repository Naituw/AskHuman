import { describe, expect, it, vi } from "vitest";
import type { PopupInboxRequest } from "../../lib/types";
import { acceptsFiles, DRAG, dropPoints, dwellTarget, edgeSpeed, sameHit, serialDrag, type DragPayload, type DropHit } from "./inboxDrop";

describe("where a native drop position lands on the page", () => {
  // The small window of a fixed canvas: the canvas keeps the expanded size, the window is much smaller.
  const small = { sidebarWidth: 240, canvas: { left: 606, height: 620 }, frame: { height: 179 } };
  const settled = { sidebarWidth: 240, canvas: { left: 606, height: 620 }, frame: { height: 620 } };

  it("corrects y by how far the canvas sticks out below the window, and x by the canvas origin", () => {
    // wry measures y from the top of the canvas-sized frame, so a point 40 down in a 179pt window reads 40 + 441.
    expect(dropPoints({ x: 100, y: 481 }, { layout: small, scale: 2, windows: false })).toEqual([[460, 40]]);
  });

  it("needs no correction once the window and the canvas are the same height", () => {
    expect(dropPoints({ x: 100, y: 300 }, { layout: settled, scale: 2, windows: false })).toEqual([[460, 300]]);
  });

  it("never corrects upwards when the canvas is the smaller one", () => {
    const odd = { ...settled, canvas: { left: 606, height: 500 } };
    expect(dropPoints({ x: 0, y: 300 }, { layout: odd, scale: 1, windows: false })[0][1]).toBe(300);
  });

  it("keeps the same answer location before and after the Sidebar is revealed (the canvas origin does not move)", () => {
    const lone = { sidebarWidth: 0, canvas: { left: 606, height: 620 }, frame: { height: 620 } };
    expect(dropPoints({ x: 30, y: 200 }, { layout: lone, scale: 1, windows: false })).toEqual([[636, 200]]);
    expect(dropPoints({ x: 276, y: 200 }, { layout: settled, scale: 1, windows: false })).toEqual([[636, 200]]);
  });

  it("without a canvas tries one reading of the units first and the other as the fallback, by platform", () => {
    const none = { layout: null, scale: 2 };
    expect(dropPoints({ x: 400, y: 200 }, { ...none, windows: true })).toEqual([[200, 100], [400, 200]]);
    expect(dropPoints({ x: 400, y: 200 }, { ...none, windows: false })).toEqual([[400, 200], [200, 100]]);
    expect(dropPoints({ x: 400, y: 200 }, { layout: { sidebarWidth: 240 }, scale: 0, windows: false })).toEqual([[400, 200], [400, 200]]);
  });
});

describe("what a rest over the window waits for", () => {
  const row = (id: string): DropHit => ({ kind: "row", id });
  it("opens the small window onto the row under the pointer, and anywhere else onto the default request", () => {
    expect(dwellTarget({ collapsed: true, hit: row("b"), active: "a", fallback: "a" })).toBe("b");
    expect(dwellTarget({ collapsed: true, hit: null, active: "a", fallback: "c" })).toBe("c");
    expect(dwellTarget({ collapsed: true, hit: null, active: null, fallback: null })).toBeNull();
  });

  it("opens another request's body in an open window, and nothing for the open one or anything else", () => {
    expect(dwellTarget({ collapsed: false, hit: row("b"), active: "a", fallback: "a" })).toBe("b");
    expect(dwellTarget({ collapsed: false, hit: row("a"), active: "a", fallback: "a" })).toBeNull();
    expect(dwellTarget({ collapsed: false, hit: { kind: "body", id: "a" }, active: "a", fallback: "a" })).toBeNull();
    expect(dwellTarget({ collapsed: false, hit: { kind: "card", id: "a", question: 2 }, active: "a", fallback: "a" })).toBeNull();
    expect(dwellTarget({ collapsed: false, hit: null, active: "a", fallback: "a" })).toBeNull();
  });
});

describe("edge scrolling", () => {
  const box = { top: 100, bottom: 500 };
  const middle = { top: 300, height: 2000, client: 400 };

  it("scrolls up in the top band, down in the bottom band, and not at all in between", () => {
    expect(edgeSpeed(box, 120, middle)).toBeLessThan(0);
    expect(edgeSpeed(box, 480, middle)).toBeGreaterThan(0);
    expect(edgeSpeed(box, 300, middle)).toBe(0);
    expect(edgeSpeed(box, box.top + DRAG.edgePx, middle)).toBe(0);
    expect(edgeSpeed(box, box.bottom - DRAG.edgePx, middle)).toBe(0);
  });

  it("goes faster the closer the pointer is to the edge, with the square of the depth, up to the limit", () => {
    const half = edgeSpeed(box, box.bottom - DRAG.edgePx / 2, middle);
    const full = edgeSpeed(box, box.bottom, middle);
    expect(half).toBeCloseTo(DRAG.edgeSpeed / 4);
    expect(full).toBeCloseTo(DRAG.edgeSpeed);
    expect(edgeSpeed(box, box.bottom + 300, middle)).toBeCloseTo(DRAG.edgeSpeed);
  });

  it("pushes as hard as the edge itself from anywhere above the top band (the heading, the navigation bar)", () => {
    expect(edgeSpeed(box, 20, middle)).toBeCloseTo(-DRAG.edgeSpeed);
    expect(edgeSpeed(box, box.top, middle)).toBeCloseTo(-DRAG.edgeSpeed);
  });

  it("does not push where the area cannot scroll any further that way", () => {
    expect(edgeSpeed(box, 120, { ...middle, top: 0 })).toBe(0);
    expect(edgeSpeed(box, 480, { ...middle, top: 1600 })).toBe(0);
    expect(edgeSpeed(box, 480, { ...middle, top: 1599 })).toBeGreaterThan(0);
    expect(edgeSpeed(box, 480, { top: 0, height: 400, client: 400 })).toBe(0);
  });
});

describe("hits and requests", () => {
  it("tells one hit from another, a card by its question", () => {
    expect(sameHit(null, null)).toBe(true);
    expect(sameHit({ kind: "row", id: "a" }, { kind: "row", id: "a" })).toBe(true);
    expect(sameHit({ kind: "row", id: "a" }, { kind: "row", id: "b" })).toBe(false);
    expect(sameHit({ kind: "row", id: "a" }, { kind: "body", id: "a" })).toBe(false);
    expect(sameHit({ kind: "card", id: "a", question: 1 }, { kind: "card", id: "a", question: 1 })).toBe(true);
    expect(sameHit({ kind: "card", id: "a", question: 1 }, { kind: "card", id: "a", question: 2 })).toBe(false);
    expect(sameHit(null, { kind: "body", id: "a" })).toBe(false);
  });

  const ask = (selectOnly: boolean, questions: number): PopupInboxRequest => ({ requestId: "r", sequence: 1, project: "/p", source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id: "r", isMarkdown: true, selectOnly, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: Array.from({ length: questions }, () => ({ message: "Q", predefinedOptions: [] })) } } });
  it("takes files only where an answer editor exists", () => {
    expect(acceptsFiles(ask(false, 1))).toBe(true);
    expect(acceptsFiles(ask(false, 3))).toBe(true);
    expect(acceptsFiles(ask(true, 1))).toBe(false);
    expect(acceptsFiles(ask(false, 0))).toBe(false);
    expect(acceptsFiles({ ...ask(false, 1), interaction: { type: "confirm" } as PopupInboxRequest["interaction"] })).toBe(false);
  });
});

describe("one drag event at a time", () => {
  const at = (type: DragPayload["type"], x = 0): DragPayload => ({ type, position: { x, y: 0 } });
  const label = (payload: DragPayload) => `${payload.type}${payload.position?.x || ""}`;
  /** A handler that keeps its `enter` busy until `release()`, as opening the window does. */
  function slowEnter() {
    const seen: string[] = [];
    const failures: unknown[] = [];
    let release!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    const feed = serialDrag(async payload => { seen.push(label(payload)); if (payload.type === "enter") await held; }, e => failures.push(e));
    return { feed, seen, failures, release };
  }

  it("handles the events in the order they arrived, none overtaking another", async () => {
    const { feed, seen, release } = slowEnter();
    const all = [feed(at("enter", 1)), feed(at("leave", 2)), feed(at("enter", 3)), feed(at("drop", 4))];
    release();
    await Promise.all(all);
    expect(seen).toEqual(["enter1", "leave2", "enter3", "drop4"]);
  });

  it("collapses the moves that pile up behind a slow step into the newest", async () => {
    const { feed, seen, release } = slowEnter();
    const all = [feed(at("enter", 1)), feed(at("over", 2)), feed(at("over", 3)), feed(at("over", 4))];
    release();
    await Promise.all(all);
    expect(seen).toEqual(["enter1", "over4"]);
  });

  it("keeps handling moves one by one when each is handled before the next arrives", async () => {
    const handled = vi.fn(async () => {});
    const feed = serialDrag(handled, () => {});
    await feed(at("over", 1));
    await feed(at("over", 2));
    expect(handled).toHaveBeenCalledTimes(2);
  });

  it("never drops a leave or a drop, and forgets a move that was queued before them", async () => {
    const { feed, seen, release } = slowEnter();
    const all = [feed(at("enter")), feed(at("over", 2)), feed(at("leave")), feed(at("over", 5)), feed(at("drop"))];
    release();
    await Promise.all(all);
    expect(seen).toEqual(["enter", "leave", "drop"]);
  });

  it("carries on with the next event after one fails, and reports the failure", async () => {
    const seen: string[] = [];
    const failures: unknown[] = [];
    const feed = serialDrag(async payload => { seen.push(payload.type); if (payload.type === "enter") throw new Error("boom"); }, e => failures.push(e));
    await Promise.all([feed(at("enter")), feed(at("leave")), feed(at("drop"))]);
    expect(seen).toEqual(["enter", "leave", "drop"]);
    expect(failures).toHaveLength(1);
  });

  it("reports a failing move too, and still handles the one after it", async () => {
    const seen: number[] = [];
    const failures: unknown[] = [];
    const feed = serialDrag(async payload => { seen.push(payload.position?.x ?? -1); if (payload.position?.x === 1) throw new Error("boom"); }, e => failures.push(e));
    await feed(at("over", 1));
    await feed(at("over", 2));
    expect(seen).toEqual([1, 2]);
    expect(failures).toHaveLength(1);
  });
});
