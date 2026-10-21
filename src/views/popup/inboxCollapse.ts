// Pure decisions for "answer later" (secondary) requests and the window form they imply. The rules are
// R1–R12 of docs/specs/popup-inbox-collapse.md §5.1; the page applies the outcomes to its own state.
// Kept free of Vue and Tauri so it can be tested without a window. Ordering reuses inboxQueue (D6).
import { inboxGroups, nextInboxRequest } from "./inboxQueue";

export type View = "expanded" | "collapsed";

/** What the model needs to know about one request. Marks live here only for the length of a decision. */
export interface CollapseRequest {
  requestId: string;
  project: string;
  /** Arrival order, as `PopupInboxRequest.sequence`. */
  sequence: number;
  /** R1: secondary = "not answering this right now". Display only; never cancels or answers. */
  secondary: boolean;
  /** The body has been shown at least once, which clears the unread dot and ripple (R7). */
  visited: boolean;
}

export interface InboxState {
  requests: CollapseRequest[];
  groupOrder: string[];
  /** The request whose body is (or will be) shown. Null only for an empty round. */
  active: string | null;
  view: View;
  /** Last request the user opened; the default target when expanding from the small window. */
  lastBrowsed: string | null;
}

/** What the page has to do to the window after a decision. */
export type Effect = "collapse" | "expand" | "switch" | "mini-resize" | null;

export interface Outcome {
  state: InboxState;
  effect: Effect;
  /** The round is over (queue cleared): the window closes, marks and position reset (R9). */
  closed: boolean;
}

export function emptyState(): InboxState {
  return { requests: [], groupOrder: [], active: null, view: "expanded", lastBrowsed: null };
}

/** Project groups in first-seen order, requests in arrival order inside a group: what the Sidebar lists (D6). */
export function orderedRequests(state: Pick<InboxState, "requests" | "groupOrder">): CollapseRequest[] {
  return inboxGroups(state.requests, state.groupOrder).flatMap(group => group.requests);
}

export function hasPrimary(state: InboxState): boolean {
  return state.requests.some(r => !r.secondary);
}

/** Target when the small window is clicked: last opened → first unread → first in list order. */
export function defaultOpenTarget(state: InboxState): string | null {
  const ordered = orderedRequests(state);
  const remembered = ordered.find(r => r.requestId === state.lastBrowsed);
  return remembered?.requestId ?? ordered.find(r => !r.visited)?.requestId ?? ordered[0]?.requestId ?? null;
}

function patch(state: InboxState, changes: Partial<InboxState>): InboxState {
  return { ...state, ...changes };
}

function setFlags(state: InboxState, id: string, changes: Partial<CollapseRequest>): InboxState {
  return patch(state, { requests: state.requests.map(r => (r.requestId === id ? { ...r, ...changes } : r)) });
}

/** Show a request's body. Showing it clears its unread state; its secondary mark is kept (C9). */
function show(state: InboxState, id: string): InboxState {
  return patch(setFlags(state, id, { visited: true }), { active: id, lastBrowsed: id });
}

const none = (state: InboxState): Outcome => ({ state, effect: null, closed: false });

/**
 * R5: a new request is primary. A collapsed window expands onto it (C4) — or, with `showNew: false`, onto the body
 * the small window would open by default while the newcomer stays unread; an expanded window keeps its body (D7).
 * `request` is the newcomer; `state` does not contain it yet.
 */
export function arrive(state: InboxState, request: CollapseRequest, options: { showNew?: boolean } = {}): Outcome {
  const first = state.requests.length === 0;
  const groupOrder = state.groupOrder.includes(request.project) ? state.groupOrder : [...state.groupOrder, request.project];
  const next = patch(state, { requests: [...state.requests, { ...request, secondary: false }], groupOrder });
  if (first) return { state: show(next, request.requestId), effect: null, closed: false };
  if (state.view === "collapsed") {
    const target = options.showNew === false ? defaultOpenTarget(state) ?? request.requestId : request.requestId;
    return { state: patch(show(next, target), { view: "expanded" }), effect: "expand", closed: false };
  }
  return none(next);
}

/**
 * The footer "稍后回答" action on the current primary request (C10, C21) and the Sidebar hover icon on the active row.
 * R3: mark it secondary, then continue with the next primary request, or collapse when none remains.
 * (For a request that is already secondary the footer button is "取消稍后答" = `unmark`, C24.)
 */
export function putAway(state: InboxState): Outcome {
  const id = state.active;
  if (id === null || state.view === "collapsed") return none(state);
  const marked = setFlags(state, id, { secondary: true });
  const excluded = new Set(marked.requests.filter(r => r.secondary).map(r => r.requestId));
  const next = nextInboxRequest(orderedRequests(marked), id, excluded);
  if (next !== null) return { state: show(marked, next), effect: "switch", closed: false };
  return { state: patch(marked, { view: "collapsed" }), effect: "collapse", closed: false };
}

/**
 * Sidebar hover icon on any row. Marking the active row is the footer action; marking another row only
 * changes its mark — it never collapses the window under a user who is reading something else.
 */
export function markSecondary(state: InboxState, id: string): Outcome {
  if (!state.requests.some(r => r.requestId === id)) return none(state);
  if (state.view === "expanded" && id === state.active) return putAway(state);
  return { state: setFlags(state, id, { secondary: true }), effect: null, closed: false };
}

/** Restore a request to primary. R2: from the small window this makes the window expand onto it. */
export function unmark(state: InboxState, id: string): Outcome {
  if (!state.requests.some(r => r.requestId === id)) return none(state);
  const restored = setFlags(state, id, { secondary: false });
  if (state.view === "collapsed") {
    return { state: patch(show(restored, id), { view: "expanded" }), effect: "expand", closed: false };
  }
  return { state: restored, effect: null, closed: false };
}

/** Open a request (Sidebar / small-window row / tray focus). The mark stays; a collapsed window expands (R6, C9). */
export function browse(state: InboxState, id: string): Outcome {
  if (!state.requests.some(r => r.requestId === id)) return none(state);
  const shown = show(state, id);
  if (state.view === "collapsed") return { state: patch(shown, { view: "expanded" }), effect: "expand", closed: false };
  return { state: shown, effect: id === state.active ? null : "switch", closed: false };
}

/**
 * The window goes back to the small state without marking anything (C23, C27): the user left AskHuman, or asked
 * for it, while the expanded window shows nothing but secondary requests (a "peek"). A primary request keeps it open.
 */
export function collapseWindow(state: InboxState): Outcome {
  if (state.view === "collapsed" || hasPrimary(state)) return none(state);
  return { state: patch(state, { view: "collapsed" }), effect: "collapse", closed: false };
}

/**
 * An answer was sent, the request was cancelled, or a remote channel ended it (R4 / R6).
 * With the open request gone: the next primary request; with only secondary ones left (C22) an engaged user — the one
 * who just answered here, or an app that is currently active — is taken to the next secondary request in D6 order
 * instead of being left with a collapsing window. Nobody is looking (`engaged: false`) or the window is already small:
 * the window collapses / stays small. `state` still contains `id`.
 */
export function finish(state: InboxState, id: string, options: { engaged?: boolean } = {}): Outcome {
  if (!state.requests.some(r => r.requestId === id)) return none(state);
  const before = orderedRequests(state);
  const survivors = state.requests.filter(r => r.requestId !== id);
  const removed = patch(state, {
    requests: survivors,
    lastBrowsed: state.lastBrowsed === id ? null : state.lastBrowsed,
  });
  if (!survivors.length) {
    return { state: patch(removed, { active: null, view: "expanded" }), effect: null, closed: true };
  }
  if (state.active !== id) {
    return { state: removed, effect: state.view === "collapsed" ? "mini-resize" : null, closed: false };
  }
  const excluded = new Set([id, ...survivors.filter(r => r.secondary).map(r => r.requestId)]);
  const next = nextInboxRequest(before, id, excluded);
  if (next !== null) {
    const shown = show(removed, next);
    return {
      state: state.view === "collapsed" ? patch(shown, { view: "expanded" }) : shown,
      effect: state.view === "collapsed" ? "expand" : "switch",
      closed: false,
    };
  }
  if (state.view === "expanded" && options.engaged !== false) {
    // C22: only secondary requests remain; stay with the user and open the next one (the window stays expanded).
    const following = nextInboxRequest(before, id, new Set([id]));
    if (following !== null) return { state: show(removed, following), effect: "switch", closed: false };
  }
  // Nobody is looking, or the window is small already: keep it small instead of surfacing one of them.
  const resting = patch(removed, { active: before.find(r => r.requestId !== id)?.requestId ?? null, view: "collapsed" });
  return { state: resting, effect: state.view === "collapsed" ? "mini-resize" : "collapse", closed: false };
}
