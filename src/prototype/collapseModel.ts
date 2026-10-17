// Pure state machine for the "temporarily put away" (secondary) marking and the derived
// expanded/collapsed window view. Rules R1–R10 live in docs/specs/popup-inbox-collapse.md §5.1.
// Kept free of Vue/Tauri so the same logic can move into the product unchanged.

export type Kind = "ask" | "permission" | "stop";
export type View = "expanded" | "collapsed";

/** One question of a multi-question request: its own options, answer and attachments. */
export interface CQuestion {
  text: string;
  options: string[];
}

export interface CRequest {
  id: number;
  kind: Kind;
  project: string;
  path: string;
  agent: string;
  session: string;
  title: string;
  message: string;
  options: string[];
  /** C32: several questions in one request (a single question keeps using `message` / `options`). */
  questions?: CQuestion[];
  /** R1: secondary = "not answering this right now". Display-only; never cancels or answers. */
  secondary: boolean;
  /** The body has been shown at least once, which clears the unread dot and ripple (R7). */
  visited: boolean;
}

export interface InboxState {
  requests: CRequest[];
  groupOrder: string[];
  /** The request whose body is (or will be) shown. null only for an empty round. */
  active: number | null;
  view: View;
  /** D5: the Sidebar appears with the 2nd request and stays until the queue clears. */
  sidebarSticky: boolean;
  /** Last request the user opened; the default target when expanding from the small window. */
  lastBrowsed: number | null;
}

/** What the view layer has to animate after a state change. */
export type Effect = "collapse" | "expand" | "switch" | "sidebar" | "mini-resize" | null;

export interface Outcome {
  state: InboxState;
  effect: Effect;
  /** The round is over (queue cleared): the window closes, marks and position reset (R9). */
  closed: boolean;
}

export function emptyState(): InboxState {
  return { requests: [], groupOrder: [], active: null, view: "expanded", sidebarSticky: false, lastBrowsed: null };
}

/** Project groups in first-seen order, requests in arrival order inside a group (production inboxGroups). */
export function orderedRequests(state: Pick<InboxState, "requests" | "groupOrder">): CRequest[] {
  return state.groupOrder.flatMap(path => state.requests.filter(r => r.path === path));
}

/**
 * D6 next-request rule with a skip set: continue inside the current project, then advance through
 * the stable project order, wrapping at the end. `excluded` carries removed and secondary ids.
 */
export function nextPrimary(ordered: CRequest[], from: number, excluded: Set<number>): number | null {
  const index = ordered.findIndex(r => r.id === from);
  const project = ordered[index]?.path;
  const cycle = index < 0 ? ordered : [...ordered.slice(index + 1), ...ordered.slice(0, index)];
  const candidates = cycle.filter(r => !excluded.has(r.id));
  return (candidates.find(r => r.path === project) ?? candidates[0])?.id ?? null;
}

export function hasPrimary(state: InboxState): boolean {
  return state.requests.some(r => !r.secondary);
}

/** The Sidebar column exists in the expanded layout (D5). */
export function sidebarVisible(state: InboxState): boolean {
  return state.sidebarSticky || state.requests.length > 1;
}

/** Target when the small window is clicked: last opened → first unread → first in list order. */
export function defaultOpenTarget(state: InboxState): number | null {
  const ordered = orderedRequests(state);
  const remembered = ordered.find(r => r.id === state.lastBrowsed);
  return remembered?.id ?? ordered.find(r => !r.visited)?.id ?? ordered[0]?.id ?? null;
}

function patch(state: InboxState, changes: Partial<InboxState>): InboxState {
  return { ...state, ...changes };
}

function setFlags(state: InboxState, id: number, changes: Partial<CRequest>): InboxState {
  return patch(state, { requests: state.requests.map(r => (r.id === id ? { ...r, ...changes } : r)) });
}

/** Show a request's body. Showing it clears its unread state; its secondary mark is kept (C9). */
function show(state: InboxState, id: number): InboxState {
  return patch(setFlags(state, id, { visited: true }), { active: id, lastBrowsed: id });
}

/**
 * R5: a new request is primary. A collapsed window expands onto it (C4) — or, with `showNew: false`, onto the body
 * the small window would open by default while the newcomer stays unread; an expanded window keeps its body (D7).
 */
export function arrive(state: InboxState, request: CRequest, options: { showNew?: boolean } = {}): Outcome {
  const first = state.requests.length === 0;
  const groupOrder = state.groupOrder.includes(request.path) ? state.groupOrder : [...state.groupOrder, request.path];
  const requests = [...state.requests, { ...request, secondary: false }];
  const next = patch(state, { requests, groupOrder, sidebarSticky: state.sidebarSticky || requests.length > 1 });
  if (first) return { state: show(next, request.id), effect: null, closed: false };
  if (state.view === "collapsed") {
    const target = options.showNew === false ? defaultOpenTarget(state) ?? request.id : request.id;
    return { state: patch(show(next, target), { view: "expanded" }), effect: "expand", closed: false };
  }
  return { state: next, effect: sidebarVisible(next) && !sidebarVisible(state) ? "sidebar" : null, closed: false };
}

/**
 * The footer "稍后回答" action on the current primary request (C10, C21) and the Sidebar hover icon on the active row.
 * R3: mark it secondary, then continue with the next primary request, or collapse when none remains.
 * (For a request that is already secondary the footer button is "取消稍后答" = `unmark`, C24.)
 */
export function putAway(state: InboxState): Outcome {
  const id = state.active;
  if (id === null || state.view === "collapsed") return { state, effect: null, closed: false };
  const marked = setFlags(state, id, { secondary: true });
  const excluded = new Set(marked.requests.filter(r => r.secondary).map(r => r.id));
  const next = nextPrimary(orderedRequests(marked), id, excluded);
  if (next !== null) return { state: show(marked, next), effect: "switch", closed: false };
  return { state: patch(marked, { view: "collapsed" }), effect: "collapse", closed: false };
}

/**
 * Sidebar hover icon on any row. Marking the active row is the footer action; marking another row only
 * changes its mark — it never collapses the window under a user who is reading something else.
 */
export function markSecondary(state: InboxState, id: number): Outcome {
  if (state.view === "expanded" && id === state.active) return putAway(state);
  return { state: setFlags(state, id, { secondary: true }), effect: null, closed: false };
}

/** Restore a request to primary. R2: from the small window this makes the window expand onto it. */
export function unmark(state: InboxState, id: number): Outcome {
  const restored = setFlags(state, id, { secondary: false });
  if (state.view === "collapsed") {
    return { state: patch(show(restored, id), { view: "expanded" }), effect: "expand", closed: false };
  }
  return { state: restored, effect: null, closed: false };
}

/** Open a request (Sidebar / small-window row / tray focus). The mark stays; a collapsed window expands (R6, C9). */
export function browse(state: InboxState, id: number): Outcome {
  if (!state.requests.some(r => r.id === id)) return { state, effect: null, closed: false };
  const shown = show(state, id);
  if (state.view === "collapsed") return { state: patch(shown, { view: "expanded" }), effect: "expand", closed: false };
  return { state: shown, effect: id === state.active ? null : "switch", closed: false };
}

/**
 * The window goes back to the small state without marking anything (C23): the user left AskHuman while the expanded
 * window shows nothing but secondary requests (a "peek"). A primary request keeps the window open.
 */
export function collapseWindow(state: InboxState): Outcome {
  if (state.view === "collapsed" || hasPrimary(state)) return { state, effect: null, closed: false };
  return { state: patch(state, { view: "collapsed" }), effect: "collapse", closed: false };
}

/**
 * An answer was sent, the request was cancelled, or a remote channel ended it (R4 / R6).
 * With the open request gone: the next primary request; with only secondary ones left (C22) an engaged user — the one
 * who just answered here, or an app that is currently active — is taken to the next secondary request in D6 order
 * instead of being left with a collapsing window. Nobody is looking (`engaged: false`) or the window is already small:
 * the window collapses / stays small.
 */
export function finish(state: InboxState, id: number, options: { engaged?: boolean } = {}): Outcome {
  if (!state.requests.some(r => r.id === id)) return { state, effect: null, closed: false };
  const before = orderedRequests(state);
  const survivors = state.requests.filter(r => r.id !== id);
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
  const excluded = new Set([id, ...survivors.filter(r => r.secondary).map(r => r.id)]);
  const next = nextPrimary(before, id, excluded);
  if (next !== null) {
    const shown = show(removed, next);
    return { state: state.view === "collapsed" ? patch(shown, { view: "expanded" }) : shown,
      effect: state.view === "collapsed" ? "expand" : "switch", closed: false };
  }
  if (state.view === "expanded" && options.engaged !== false) {
    // C22: only secondary requests remain; stay with the user and open the next one (the window stays expanded).
    const following = nextPrimary(before, id, new Set([id]));
    if (following !== null) return { state: show(removed, following), effect: "switch", closed: false };
  }
  // Nobody is looking, or the window is small already: keep it small instead of surfacing one of them.
  const resting = patch(removed, { active: before.find(r => r.id !== id)?.id ?? null, view: "collapsed" });
  return { state: resting, effect: state.view === "collapsed" ? "mini-resize" : "collapse", closed: false };
}
