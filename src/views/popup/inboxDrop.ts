// Files dragged over the inbox window (docs/specs/popup-inbox-collapse.md C28, C31, C32; R13, R15, R16). Like dragging
// onto folders in Finder, one drag has to reach any pending request and any of its questions: resting over the small
// window opens it, resting on another Sidebar row opens that request, the edges of the list and the body scroll, and a
// drop goes to whatever is under the pointer at that moment. This file holds the pure decisions; useInboxDrop.ts
// applies them to the window. Kept free of Vue and Tauri so it can be tested without a window.
import type { PopupInboxRequest } from "../../lib/types";
import { inboxDropPoint, type CanvasPosition } from "./inboxCoordinates";

/** The prototype's defaults, which the user confirmed by dragging real files (spec §6.8 ④). Not settings. */
export const DRAG = {
  /** Resting over the small window this long opens it. */
  expandMs: 500,
  /** Resting on another Sidebar row this long opens that request. */
  switchMs: 250,
  /** After a wait opened something, the pointer has to move this far before the next wait starts. */
  holdPx: 4,
  /** A drag that opened the window and then gave up shrinks it back after this. */
  springBackMs: 600,
  /** The band at the top and bottom of the list and of the body that scrolls them, and the fastest it goes (px/s). */
  edgePx: 56,
  edgeSpeed: 900,
  /** The longest a drop waits for a request that was never opened to be ready for files. */
  mountMs: 3000,
} as const;

/** Tauri's native drag-drop events as the page receives them. */
export interface DragPayload {
  type: "enter" | "over" | "drop" | "leave";
  paths?: string[];
  position?: { x: number; y: number };
}

/** What is under the pointer. A card is only ever a target in the vertical layout, where every question is one. */
export type DropHit =
  | { kind: "row"; id: string }
  | { kind: "card"; id: string; question: number }
  | { kind: "body"; id: string };

export function sameHit(a: DropHit | null, b: DropHit | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.kind !== b.kind || a.id !== b.id) return false;
  return a.kind !== "card" || (b.kind === "card" && a.question === b.question);
}

/** Whether a request has anywhere to put files: confirmations have no answer editor, select-only requests turn it off. */
export function acceptsFiles(request: PopupInboxRequest): boolean {
  const interaction = request.interaction;
  return interaction.type === "ask" && !interaction.request.selectOnly && interaction.request.questions.length > 0;
}

export interface DropGeometry {
  layout: (CanvasPosition & { canvas?: { left: number; height: number } | null; frame?: { height: number } }) | null;
  /** Window.devicePixelRatio. */
  scale: number;
  windows: boolean;
}

/**
 * Where a native drop position lands on the page: one point, or the candidates in order of likelihood.
 *
 * With the fixed canvas (macOS) wry reports x from the window's left edge, and y down from the top of the webview
 * *frame*. That frame is the canvas, which sticks out below a window whose viewport is pinned larger (the small
 * window), so y comes out too large by the difference. A settled expanded window has a responsive canvas and no
 * difference. Without the canvas the units differ by platform: Tauri labels the position physical, but only Windows
 * reports physical pixels; macOS and GTK report points, the scale of CSS pixels. One reading is tried first and the
 * other is the fallback, so a platform that changes its mind does not break drops.
 */
export function dropPoints(position: { x: number; y: number }, geometry: DropGeometry): [number, number][] {
  const { layout } = geometry;
  if (layout?.canvas) {
    const excess = Math.max(0, layout.canvas.height - (layout.frame?.height ?? layout.canvas.height));
    return [inboxDropPoint(position.x, position.y - excess, layout)];
  }
  const scale = geometry.scale || 1;
  const scaled: [number, number] = [position.x / scale, position.y / scale];
  const raw: [number, number] = [position.x, position.y];
  return geometry.windows ? [scaled, raw] : [raw, scaled];
}

/**
 * Which request a wait should open for a pointer over `hit`, or null for none. The small window opens onto the row
 * under the pointer, anywhere else on it onto the default request; an open window only switches to another row's request.
 */
export function dwellTarget(options: { collapsed: boolean; hit: DropHit | null; active: string | null; fallback: string | null }): string | null {
  const row = options.hit?.kind === "row" ? options.hit.id : null;
  if (options.collapsed) return row ?? options.fallback;
  return row !== null && row !== options.active ? row : null;
}

/**
 * The signed speed (px/s) a scroll area should move at while the pointer is at `y`, or 0 to stay put. Anything above the
 * top band (the heading, the navigation bar) pushes as hard as the edge itself. The speed grows with the square of how far
 * into the band the pointer is, and there is none where the area cannot scroll any further that way.
 */
export function edgeSpeed(box: { top: number; bottom: number }, y: number, scroll: { top: number; height: number; client: number }): number {
  let push = 0;
  if (y < box.top + DRAG.edgePx) push = -Math.min(1, (box.top + DRAG.edgePx - y) / DRAG.edgePx);
  else if (y > box.bottom - DRAG.edgePx) push = Math.min(1, (y - (box.bottom - DRAG.edgePx)) / DRAG.edgePx);
  if (!push) return 0;
  const room = push < 0 ? scroll.top > 0 : scroll.top < scroll.height - scroll.client - 0.5;
  return room ? Math.sign(push) * DRAG.edgeSpeed * push * push : 0;
}

/**
 * Native drag events arrive in bursts: `enter`, `over` a millisecond later, then another every few milliseconds. Handled
 * concurrently, a later `over` overtook the `enter` that was about to open the small window and the window stayed shut.
 * So the events are handled one at a time, in order; pointer moves that pile up behind a slow step (the window opening)
 * collapse into the newest one, since only the latest position matters. `leave` and `drop` are never dropped, and a move
 * that was queued before them describes a situation that is already over.
 */
export function serialDrag(handle: (payload: DragPayload) => Promise<void>, fail: (error: unknown) => void) {
  let chain: Promise<void> = Promise.resolve();
  let queued: DragPayload | null = null;
  return (payload: DragPayload): Promise<void> => {
    if (payload.type === "over") {
      const waiting = queued !== null;
      queued = payload;
      if (waiting) return chain;
      chain = chain.then(() => {
        const latest = queued;
        queued = null;
        return latest ? handle(latest) : undefined;
      }).catch(fail);
      return chain;
    }
    queued = null;
    chain = chain.then(() => handle(payload)).catch(fail);
    return chain;
  };
}
