// The inbox window's drop controller (docs/specs/popup-inbox-collapse.md C28, C31, C32; R13, R15, R16). The page registers
// ONE listener for the window's native drag-drop events (the WebView cannot see file paths in HTML drag events) and routes
// them here, so that a single drag can reach any pending request and any of its questions, the way a drag in Finder
// opens folders ("spring-loading"):
//  - resting over the small window opens it, onto the row under the pointer (anywhere else: the default request);
//  - resting on another Sidebar row opens that request; the row fills a bar while it waits;
//  - the top and bottom edge of the list and of the body scroll, so that every row and every question card can be reached;
//  - a drop goes to whatever is under the pointer at that moment, without waiting for any of that.
// Pure decisions live in inboxDrop.ts; this applies them to the window the page owns.
import { ref } from "vue";
import { isWindows } from "../../lib/platform";
import { DRAG, dropPoints, dwellTarget, edgeSpeed, sameHit, serialDrag, type DragPayload, type DropHit } from "./inboxDrop";
import type { DropReceiver, InboxLayout } from "./usePopupCore";

type Point = [number, number];
type Probe = DropHit | "preview" | null;

export interface InboxDropEnv {
  root: () => HTMLElement | null;
  layout: () => InboxLayout | null;
  view: () => "expanded" | "collapsed";
  /** The request whose body is open. */
  active: () => string | null;
  /** The window is changing form, swapping bodies or showing a dialog: not a time to take files. */
  blocked: () => boolean;
  /** The request the small window would open on its own. */
  fallback: () => string | null;
  /** Whether a request can take files at all (a confirmation or a select-only request cannot). */
  accepts: (id: string) => boolean;
  receiver: (id: string) => DropReceiver | undefined;
  /** Open a request: the small window expands onto it, an open window switches its body. Resolves once the window has settled. */
  open: (id: string) => Promise<void>;
  /** Hand the files to a request (question null: the one it is on) and show it. */
  deliver: (id: string, paths: string[], question: number | null) => Promise<void>;
  /** The drag opened the window and gave up: shrink it back (only a window with nothing left to answer does). */
  springBack: () => void;
  fail: (error: unknown) => void;
}

export function useInboxDrop(env: InboxDropEnv) {
  /** Files are being dragged over the window. */
  const dragging = ref(false);
  /** What a drop would land on right now, when that request can take files: a row, a question card, or the open body. */
  const aim = ref<DropHit | null>(null);
  /** The request a running wait will open, and how long the wait is. */
  const dwell = ref<{ id: string; ms: number } | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let springTimer: ReturnType<typeof setTimeout> | null = null;
  /** Page coordinates of the pointer at the last event: edge scrolling and re-aiming after a layout change use it. */
  let pointer: Point | null = null;
  /** Something opened under a pointer that did not move: the next wait only starts once it has moved. */
  let hold: Point | null = null;
  /** This drag opened the small window, so giving up shrinks it back. */
  let openedByDrag = false;
  /** A wait ran out and the window is changing form for it. */
  let opening: Promise<void> | null = null;
  let disposed = false;

  function clearDwell() {
    if (timer) clearTimeout(timer);
    timer = null;
    dwell.value = null;
  }
  function clearSpringBack() {
    if (springTimer) clearTimeout(springTimer);
    springTimer = null;
  }

  /** What is at a page point. The Sidebar's rows (the pointer over a row's own button counts) come first. */
  function hitAt([x, y]: Point): Probe {
    const element = document.elementFromPoint(x, y);
    if (!element) return null;
    const row = element.closest(".inbox-entry")?.querySelector<HTMLElement>("[data-inbox-row]");
    if (row?.dataset.inboxRow) return { kind: "row", id: row.dataset.inboxRow };
    // An attachment preview never takes files (as before).
    if (element.closest(".attachment-preview, .attachment-preview-divider")) return "preview";
    const id = env.active();
    if (!id || !element.closest(".inbox-body")) return null;
    // The vertical layout makes every question a card: the one under the pointer is the target.
    const card = element.closest<HTMLElement>(".q-card");
    const question = card?.dataset.qIndex === undefined ? NaN : Number(card.dataset.qIndex);
    return Number.isNaN(question) ? { kind: "body", id } : { kind: "card", id, question };
  }
  /** A native position, taken to the page, and what is there. */
  function probe(position: { x: number; y: number }): { hit: Probe; point: Point } {
    const points = dropPoints(position, { layout: env.layout(), scale: window.devicePixelRatio, windows: isWindows });
    for (const point of points) {
      const hit = hitAt(point);
      if (hit) return { hit, point };
    }
    return { hit: null, point: points[0] };
  }
  /** Aim at what is lit for `hit`. Pointer moves come every few ms; only a change of target reaches the page's state. */
  function lightUp(hit: Probe) {
    const next = hit && hit !== "preview" && env.accepts(hit.id) ? hit : null;
    if (!sameHit(aim.value, next)) aim.value = next;
  }

  /**
   * Aim at what is under the pointer and, unless that is already what is open, start the wait that opens it. Nothing
   * starts while the window is changing: the layout under the pointer is not the final one.
   */
  function moved(point: Point, hit: Probe) {
    pointer = point;
    lightUp(hit);
    if (hold) {
      if (Math.hypot(point[0] - hold[0], point[1] - hold[1]) < DRAG.holdPx) return;
      hold = null;
    }
    const collapsed = env.view() === "collapsed";
    const wanted = dwellTarget({ collapsed, hit: hit === "preview" ? null : hit, active: env.active(), fallback: env.fallback() });
    if (wanted === null || env.blocked() || opening) { clearDwell(); return; }
    if (dwell.value?.id === wanted) return;
    clearDwell();
    const ms = collapsed ? DRAG.expandMs : DRAG.switchMs;
    dwell.value = { id: wanted, ms };
    timer = setTimeout(() => { timer = null; dwell.value = null; void openByDrag(wanted); }, ms);
  }

  /** A wait ran out: open the small window onto `id`, or switch the open window's body to it. */
  async function openByDrag(id: string) {
    if (disposed || !dragging.value) return;
    if (env.view() === "collapsed") openedByDrag = true;
    // The layout is about to move under a pointer that will not: aim again afterwards, and wait for it to move.
    hold = pointer ? [...pointer] : null;
    const run = env.open(id).catch(env.fail);
    opening = run;
    await run;
    if (opening === run) opening = null;
    if (!disposed && dragging.value && pointer) lightUp(hitAt(pointer));
  }

  /** The drag ended, with a drop or otherwise: nothing of it is left. */
  function finish() {
    dragging.value = false;
    aim.value = null;
    pointer = null;
    hold = null;
    openedByDrag = false;
    clearDwell();
    clearSpringBack();
    stopEdge();
  }
  /** The drag left the window without dropping. One that opened the window and gave up closes it again, unless it comes back in time. */
  function left() {
    dragging.value = false;
    aim.value = null;
    pointer = null;
    hold = null;
    clearDwell();
    stopEdge();
    if (!openedByDrag) return;
    clearSpringBack();
    springTimer = setTimeout(() => {
      springTimer = null;
      if (dragging.value || disposed) return;
      openedByDrag = false;
      env.springBack();
    }, DRAG.springBackMs);
  }

  async function drop(paths: string[], position: { x: number; y: number }) {
    const { hit } = probe(position);
    const aimed = aim.value;
    const collapsed = env.view() === "collapsed";
    finish();
    if (hit === "preview") return;
    // Not on a row or a card: the open request (for the small window, the one it would open).
    const target = hit ?? aimed;
    const id = target?.id ?? (collapsed ? env.fallback() : env.active());
    if (!id || !paths.length || !env.accepts(id)) return;
    await env.deliver(id, paths, target?.kind === "card" ? target.question : null);
  }

  async function handle(payload: DragPayload) {
    if (disposed) return;
    // A window change that a wait started finishes first, so that the layout under the pointer is the final one.
    if (opening) await opening;
    if (payload.type === "leave") { left(); return; }
    const id = env.active();
    const own = id ? env.receiver(id) : undefined;
    if (own?.draggingOut()) {
      // One of this request's own attachments is on its way out of the window: that is not files coming in.
      aim.value = null;
      clearDwell();
      stopEdge();
      if (payload.type === "drop") own.endDragOut();
      return;
    }
    if (env.blocked()) {
      // A dialog or a transition has the window: it takes no files now.
      aim.value = null;
      clearDwell();
      if (payload.type === "drop") finish();
      return;
    }
    const position = payload.position ?? { x: 0, y: 0 };
    if (payload.type === "drop") { await drop(payload.paths ?? [], position); return; }
    // `enter` starts a drag, but so does the first move should the `enter` ever be missed.
    if (!dragging.value) { dragging.value = true; clearSpringBack(); }
    const { hit, point } = probe(position);
    moved(point, hit);
    startEdge();
  }

  // Edge scrolling: the list and the body scroll while the pointer rests near their top or bottom edge, faster the closer
  // it gets, so that any row and any question card can be reached without letting go.
  let frame = 0;
  let last = 0;
  let edgeElement: HTMLElement | null = null;
  let edgePosition = 0;
  /** The list or body under the pointer, and the signed speed (px/s) it should scroll at; null when it should stay put. */
  function edgePush([x, y]: Point): { element: HTMLElement; speed: number } | null {
    const root = env.root();
    if (!root) return null;
    const side = root.querySelector<HTMLElement>(".inbox-sidebar")?.getBoundingClientRect();
    const inSide = !!side && side.width > 0 && x >= side.left && x < side.right;
    const id = env.active();
    const element = inSide ? root.querySelector<HTMLElement>(".inbox-navigation")
      : id ? root.querySelector<HTMLElement>(`[data-inbox-request="${CSS.escape(id)}"] .content`) : null;
    if (!element) return null;
    const speed = edgeSpeed(element.getBoundingClientRect(), y, { top: element.scrollTop, height: element.scrollHeight, client: element.clientHeight });
    return speed ? { element, speed } : null;
  }
  function tick(now: number) {
    frame = 0;
    if (disposed || !dragging.value || !pointer) { edgeElement = null; return; }
    const dt = last ? Math.min(now - last, 50) : 16;
    last = now;
    const push = env.blocked() || opening ? null : edgePush(pointer);
    if (push) {
      // Fractions of a pixel add up here: the element's own scrollTop would round them away at slow speeds.
      if (push.element !== edgeElement || Math.abs(push.element.scrollTop - edgePosition) > 2) {
        edgeElement = push.element;
        edgePosition = push.element.scrollTop;
      }
      edgePosition += (push.speed * dt) / 1000;
      push.element.scrollTop = edgePosition;
      // Whatever slid under the pointer is the target now.
      moved(pointer, hitAt(pointer));
    } else edgeElement = null;
    frame = requestAnimationFrame(tick);
  }
  function startEdge() {
    if (frame) return;
    last = 0;
    frame = requestAnimationFrame(tick);
  }
  function stopEdge() {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    edgeElement = null;
  }

  return {
    dragging, aim, dwell,
    /** Feed the window's native drag-drop events here, in the order they arrive. */
    onEvent: serialDrag(handle, env.fail),
    /** A new round starts with no drag in it. */
    reset: finish,
    dispose() { disposed = true; finish(); },
  };
}
