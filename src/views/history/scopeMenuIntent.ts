/**
 * Pointer-intent guard for the two-level history scope menu (project column | session column).
 *
 * Going from the hovered project row to a session row is a diagonal path that usually crosses
 * other project rows on the way. Switching the session column on every crossed row makes the
 * session being aimed at jump away. This guard defers a project switch while the pointer is
 * plainly heading for the session column, i.e. while it stays inside the "safe triangle" spanned
 * by the last point on the active row and the near edge of the session column (the same idea
 * native submenus use). It is framework agnostic; HistoryView wires it to the menu.
 */

export interface Point {
  x: number;
  y: number;
}

/** A rectangle in viewport coordinates (a `DOMRect` fits). */
export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface PointerSample extends Point {
  /** `PointerEvent.pointerType`; touch has no hover and always switches at once. */
  pointerType?: string;
}

/** A pointer that holds still this long over another row has chosen that row. */
export const STALL_MS = 150;
/** The guard never defers a switch longer than this, however the pointer keeps moving. */
export const MAX_GRACE_MS = 1000;
/** Movement shorter than this is jitter and does not count as "still moving". */
export const MIN_MOVE_PX = 3;
/** Backing off this far from the furthest point reached toward the session column cancels the protection. */
export const RETREAT_PX = 8;
/** The triangle apex sits this far behind the last point on the active row so early samples are not clipped. */
export const APEX_PAD = 6;

/** Whether `p` lies inside (or on the border of) the triangle `a`, `b`, `c`. */
export function pointInTriangle(p: Point, a: Point, b: Point, c: Point): boolean {
  const side = (o: Point, u: Point, v: Point) => (u.x - o.x) * (v.y - o.y) - (u.y - o.y) * (v.x - o.x);
  const d1 = side(a, b, p);
  const d2 = side(b, c, p);
  const d3 = side(c, a, p);
  const hasNegative = d1 < 0 || d2 < 0 || d3 < 0;
  const hasPositive = d1 > 0 || d2 > 0 || d3 > 0;
  return !(hasNegative && hasPositive);
}

export interface ScopeIntentOptions {
  /** Viewport rectangle of the session column, or null while it is not rendered. */
  target: () => Rect | null;
  /** The project whose sessions are currently shown. */
  active: () => string;
  /** Show the sessions of another project. */
  activate: (token: string) => void;
  /** Called when the guard starts or stops deferring a switch. */
  onGuardChange?: (guarding: boolean) => void;
}

export interface ScopeIntent {
  /** The pointer is over (or moving across) a project row. */
  pointer(token: string, sample: PointerSample): void;
  /** The pointer left a project row. */
  leave(token: string): void;
  /** An explicit choice (click, keyboard focus): switch at once. */
  choose(token: string): void;
  /** Stop guarding without switching: the pointer reached the session column or left the menu. */
  settle(): void;
  /** Release timers. */
  dispose(): void;
}

export function createScopeIntent(options: ScopeIntentOptions): ScopeIntent {
  /** Last pointer position seen over the active row: where the pointer set off from. */
  let origin: Point | null = null;
  /** Furthest progress toward the session column since `origin`, in px. */
  let progress = 0;
  /** The row under the pointer whose switch is being deferred. */
  let pending: string | null = null;
  let guarding = false;
  /** Pointer position when the stall timer was last armed. */
  let armedAt: Point | null = null;
  let stallTimer: ReturnType<typeof setTimeout> | null = null;
  let capTimer: ReturnType<typeof setTimeout> | null = null;

  function setGuarding(next: boolean): void {
    if (guarding === next) return;
    guarding = next;
    options.onGuardChange?.(next);
  }

  /** Forget everything: no deferred switch, no departure point. */
  function end(): void {
    if (stallTimer !== null) clearTimeout(stallTimer);
    if (capTimer !== null) clearTimeout(capTimer);
    stallTimer = null;
    capTimer = null;
    pending = null;
    armedAt = null;
    origin = null;
    progress = 0;
    setGuarding(false);
  }

  function commit(token: string): void {
    end();
    options.activate(token);
  }

  /** A timer ran out: switch to the row the pointer still rests on, if any. */
  function expire(): void {
    const token = pending;
    end();
    if (token !== null) options.activate(token);
  }

  function armStall(at: Point): void {
    if (stallTimer !== null) clearTimeout(stallTimer);
    stallTimer = setTimeout(expire, STALL_MS);
    armedAt = at;
  }

  function pointer(token: string, sample: PointerSample): void {
    // Hover intent is a mouse/pen notion; a touch tap is an explicit choice.
    if (sample.pointerType === "touch") {
      commit(token);
      return;
    }
    const at: Point = { x: sample.x, y: sample.y };

    if (token === options.active()) {
      // Back on (or still on) the active row: a deferred switch is moot and this is the new departure point.
      end();
      origin = at;
      return;
    }

    const rect = options.target();
    if (origin === null || rect === null) {
      // First contact, or nothing to aim at: there is no intent to protect.
      commit(token);
      return;
    }

    // The session column normally sits to the right of the departure point; stay correct if it does not.
    const dir = origin.x <= rect.left ? 1 : -1;
    const edge = dir === 1 ? rect.left : rect.right;
    const along = (at.x - origin.x) * dir;
    progress = Math.max(progress, along);
    const backingOff = along < progress - RETREAT_PX;
    const apex: Point = { x: origin.x - dir * APEX_PAD, y: origin.y };
    const heading =
      !backingOff &&
      pointInTriangle(at, apex, { x: edge, y: rect.top }, { x: edge, y: rect.bottom });
    if (!heading) {
      commit(token);
      return;
    }

    // Heading for the session column: keep showing the active project's sessions for now.
    pending = token;
    if (!guarding) {
      setGuarding(true);
      capTimer = setTimeout(expire, MAX_GRACE_MS);
      armStall(at);
    } else if (armedAt === null || Math.hypot(at.x - armedAt.x, at.y - armedAt.y) >= MIN_MOVE_PX) {
      armStall(at);
    }
  }

  function leave(token: string): void {
    // The pointer is no longer on the row a deferred switch was waiting for.
    if (pending === token) pending = null;
  }

  return {
    pointer,
    leave,
    choose: commit,
    settle: end,
    dispose: end,
  };
}
