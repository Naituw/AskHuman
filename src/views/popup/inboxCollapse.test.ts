import { describe, expect, it } from "vitest";
import {
  arrive, browse, collapseWindow, defaultOpenTarget, emptyState, finish, markSecondary, orderedRequests, putAway, unmark,
  type CollapseRequest, type InboxState, type Outcome,
} from "./inboxCollapse";

let seq = 0;
function req(project = "/p/a", extra: Partial<CollapseRequest> = {}): CollapseRequest {
  const sequence = ++seq;
  return { requestId: `r${sequence}`, project, sequence, secondary: false, visited: false, ...extra };
}
function build(...requests: CollapseRequest[]): InboxState {
  return requests.reduce((state, r) => arrive(state, r).state, emptyState());
}
const secondaryIds = (s: InboxState) => s.requests.filter(r => r.secondary).map(r => r.requestId);

describe("arrival (R5)", () => {
  it("shows the first request and marks it visited without any transition", () => {
    const a = req();
    const out = arrive(emptyState(), a);
    expect(out.state.active).toBe(a.requestId);
    expect(out.state.requests[0].visited).toBe(true);
    expect(out.effect).toBeNull();
  });

  it("keeps the current body when the window is already expanded (D7)", () => {
    const [a, b] = [req(), req()];
    const out = arrive(build(a), b);
    expect(out.state.active).toBe(a.requestId);
    expect(out.state.requests.find(r => r.requestId === b.requestId)!.visited).toBe(false);
    expect(out.effect).toBeNull();
  });

  it("forces a collapsed window to expand onto the new primary request", () => {
    const [a, b] = [req(), req()];
    const collapsed = putAway(build(a)).state;
    expect(collapsed.view).toBe("collapsed");
    const out = arrive(collapsed, b);
    expect(out.state.view).toBe("expanded");
    expect(out.state.active).toBe(b.requestId);
    expect(out.state.requests.find(r => r.requestId === b.requestId)!.secondary).toBe(false);
    expect(out.effect).toBe("expand");
  });

  it("never marks a new request secondary, even when every other request is", () => {
    const a = req();
    const out = arrive(putAway(build(a)).state, req());
    expect(out.state.requests.find(r => r.requestId !== a.requestId)!.secondary).toBe(false);
  });

  it("can expand onto the body the small window would open while the newcomer stays unread", () => {
    const [a, b, c] = [req(), req(), req()];
    const collapsed = putAway(markSecondary(build(a, b), b.requestId).state).state;
    expect(collapsed.view).toBe("collapsed");
    const out = arrive(collapsed, c, { showNew: false });
    expect(out.effect).toBe("expand");
    expect(out.state.view).toBe("expanded");
    expect(out.state.active).toBe(a.requestId);
    const added = out.state.requests.find(r => r.requestId === c.requestId)!;
    expect(added.visited).toBe(false);
    expect(added.secondary).toBe(false);
  });

  it("a lone collapsed request is still the body when the newcomer is not shown", () => {
    const [a, b] = [req(), req()];
    const out = arrive(putAway(build(a)).state, b, { showNew: false });
    expect(out.state.active).toBe(a.requestId);
    expect(out.state.requests.find(r => r.requestId === b.requestId)!.visited).toBe(false);
  });
});

describe("putting away the current request (R3, C10)", () => {
  it("collapses a lone request", () => {
    const a = req();
    const out = putAway(build(a));
    expect(out.effect).toBe("collapse");
    expect(out.state.view).toBe("collapsed");
    expect(secondaryIds(out.state)).toEqual([a.requestId]);
  });

  it("moves on to the next primary request when there is one", () => {
    const [a, b] = [req(), req()];
    const out = putAway(build(a, b));
    expect(out.effect).toBe("switch");
    expect(out.state.view).toBe("expanded");
    expect(out.state.active).toBe(b.requestId);
    expect(out.state.requests.find(r => r.requestId === b.requestId)!.visited).toBe(true);
  });

  it("skips secondary requests and wraps around", () => {
    const [a, b, c] = [req(), req(), req()];
    let state = build(a, b, c);
    state = markSecondary(state, b.requestId).state;
    const out = putAway(state);
    expect(out.state.active).toBe(c.requestId);
    const wrapped = putAway(out.state);
    expect(wrapped.effect).toBe("collapse");
    expect(secondaryIds(wrapped.state).sort()).toEqual([a.requestId, b.requestId, c.requestId]);
  });

  it("prefers the current project before advancing to another project", () => {
    const a = req("/p/a");
    const b = req("/p/b");
    const c = req("/p/a");
    const out = putAway(build(a, b, c));
    expect(out.state.active).toBe(c.requestId);
  });

  it("does nothing while the window is already collapsed", () => {
    const collapsed = putAway(build(req())).state;
    const out = putAway(collapsed);
    expect(out.effect).toBeNull();
    expect(out.state).toBe(collapsed);
  });

  it("keeps drafts-bearing requests intact: marking changes nothing but the flag", () => {
    const [a, b] = [req(), req()];
    const out = putAway(build(a, b));
    expect(out.state.requests.map(r => r.requestId)).toEqual([a.requestId, b.requestId]);
    expect(out.state.requests.find(r => r.requestId === a.requestId)).toEqual({ ...build(a, b).requests[0], secondary: true });
  });
});

describe("marking other rows from the Sidebar (C11)", () => {
  it("only sets the flag and never collapses under a user reading another request", () => {
    const [a, b] = [req(), req()];
    const out = markSecondary(build(a, b), b.requestId);
    expect(out.effect).toBeNull();
    expect(out.state.view).toBe("expanded");
    expect(out.state.active).toBe(a.requestId);
    expect(secondaryIds(out.state)).toEqual([b.requestId]);
  });

  it("does not auto-collapse when the last other primary is marked while reading a secondary request", () => {
    const [a, b] = [req(), req()];
    let state = build(a, b);
    state = browse(markSecondary(state, a.requestId).state, a.requestId).state;
    expect(state.active).toBe(a.requestId);
    const out = markSecondary(state, b.requestId);
    expect(out.state.view).toBe("expanded");
    expect(out.effect).toBeNull();
  });

  it("marking the active row behaves like the footer action", () => {
    const [a, b] = [req(), req()];
    const viaRow = markSecondary(build(a, b), a.requestId);
    const viaFooter = putAway(build(a, b));
    expect(viaRow.state).toEqual(viaFooter.state);
    expect(viaRow.effect).toBe(viaFooter.effect);
  });
});

describe("restoring and browsing (R2, R6, R7, C9)", () => {
  it("expands from the small window when a row is restored to primary", () => {
    const [a, b] = [req(), req()];
    let state = build(a, b);
    state = markSecondary(state, b.requestId).state;
    state = putAway(state).state;
    expect(state.view).toBe("collapsed");
    const out = unmark(state, b.requestId);
    expect(out.state.view).toBe("expanded");
    expect(out.state.active).toBe(b.requestId);
    expect(out.effect).toBe("expand");
    expect(out.state.requests.find(r => r.requestId === b.requestId)!.secondary).toBe(false);
  });

  it("restoring while expanded only flips the mark", () => {
    const [a, b] = [req(), req()];
    const marked = markSecondary(build(a, b), b.requestId).state;
    const out = unmark(marked, b.requestId);
    expect(out.effect).toBeNull();
    expect(secondaryIds(out.state)).toEqual([]);
  });

  it("browsing a secondary request from the small window expands but keeps the mark", () => {
    const [a, b] = [req(), req()];
    let state = build(a, b);
    state = putAway(putAway(state).state).state;
    expect(state.view).toBe("collapsed");
    const out = browse(state, a.requestId);
    expect(out.state.view).toBe("expanded");
    expect(out.effect).toBe("expand");
    expect(secondaryIds(out.state).sort()).toEqual([a.requestId, b.requestId]);
    expect(out.state.active).toBe(a.requestId);
  });

  it("keeps the unread state of a secondary request until its body is shown", () => {
    const [a, b] = [req(), req()];
    const marked = markSecondary(build(a, b), b.requestId).state;
    expect(marked.requests.find(r => r.requestId === b.requestId)!.visited).toBe(false);
    const opened = browse(marked, b.requestId).state;
    expect(opened.requests.find(r => r.requestId === b.requestId)!.visited).toBe(true);
  });

  it("switching between requests while expanded reports a body swap, re-opening the same one does not", () => {
    const [a, b] = [req(), req()];
    const state = build(a, b);
    expect(browse(state, b.requestId).effect).toBe("switch");
    expect(browse(state, a.requestId).effect).toBeNull();
  });

  it("ignores an unknown request id", () => {
    const state = build(req());
    const out = browse(state, "missing");
    expect(out.state).toBe(state);
    expect(out.effect).toBeNull();
  });

  it("restoring the open secondary request keeps the window and the body, and a primary request keeps it open (C24)", () => {
    const [a, b] = [req(), req()];
    let state = putAway(markSecondary(build(a, b), b.requestId).state).state; // a, b secondary → small
    state = browse(state, a.requestId).state; // peek: expanded on a
    expect(collapseWindow(state).effect).toBe("collapse");
    const out = unmark(state, a.requestId);
    expect(out.effect).toBeNull();
    expect(out.state.view).toBe("expanded");
    expect(out.state.active).toBe(a.requestId);
    expect(out.state.requests.find(r => r.requestId === a.requestId)!.secondary).toBe(false);
    expect(collapseWindow(out.state).effect).toBeNull();
  });

  it("can collapse a browsed secondary request only when no primary remains", () => {
    const [a, b] = [req(), req()];
    let state = markSecondary(build(a, b), a.requestId).state; // a secondary, b becomes active
    state = browse(state, a.requestId).state;
    expect(collapseWindow(state).effect).toBeNull(); // b is still primary
    state = putAway(markSecondary(state, b.requestId).state).state; // everything secondary, active a → collapse
    expect(state.view).toBe("collapsed");
    const reopened = browse(state, a.requestId).state;
    expect(collapseWindow(reopened).effect).toBe("collapse");
    expect(collapseWindow(reopened).state.view).toBe("collapsed");
  });
});

describe("answering and remote termination (R4, R6)", () => {
  it("continues with the next primary request after an answer", () => {
    const [a, b, c] = [req(), req(), req()];
    let state = markSecondary(build(a, b, c), b.requestId).state;
    const out = finish(state, a.requestId);
    expect(out.effect).toBe("switch");
    expect(out.state.active).toBe(c.requestId);
    expect(out.state.requests.map(r => r.requestId)).toEqual([b.requestId, c.requestId]);
  });

  it("moves on to a secondary request when only secondary ones remain, keeping the window expanded (C22)", () => {
    const [a, b] = [req(), req()];
    const state = markSecondary(build(a, b), b.requestId).state;
    const out = finish(state, a.requestId);
    expect(out.effect).toBe("switch");
    expect(out.state.view).toBe("expanded");
    expect(out.state.active).toBe(b.requestId);
    expect(out.state.requests.find(r => r.requestId === b.requestId)!.secondary).toBe(true); // the mark stays (C9)
    expect(out.state.requests.find(r => r.requestId === b.requestId)!.visited).toBe(true);
    expect(out.closed).toBe(false);
    expect(out.state.requests.map(r => r.requestId)).toEqual([b.requestId]);
  });

  it("answering the newcomer of a small window leads to the secondary requests, not to a collapse (C22)", () => {
    const [a, b, c] = [req(), req(), req()];
    let state = putAway(markSecondary(build(a, b), b.requestId).state).state; // a, b secondary → small window
    state = arrive(state, c).state; // expands onto c
    expect(state.active).toBe(c.requestId);
    const out = finish(state, c.requestId);
    expect(out.effect).toBe("switch");
    expect(out.state.view).toBe("expanded");
    expect([a.requestId, b.requestId]).toContain(out.state.active);
    expect(out.state.requests.find(r => r.requestId === out.state.active)!.secondary).toBe(true);
    expect(collapseWindow(out.state).effect).toBe("collapse"); // leaving AskHuman now shrinks the peek (C23)
  });

  it("picks the next secondary request in D6 order: same project first, then wrap around", () => {
    const a = req("/p/a");
    const b = req("/p/b");
    const c = req("/p/a");
    let state = markSecondary(markSecondary(build(a, b, c), b.requestId).state, c.requestId).state;
    expect(finish(state, a.requestId).state.active).toBe(c.requestId); // same project as the answered one
    state = markSecondary(markSecondary(build(a, b, c), b.requestId).state, a.requestId).state; // a, b secondary → active c
    expect(state.active).toBe(c.requestId);
    expect(finish(state, c.requestId).state.active).toBe(a.requestId); // wraps to the start of the list
  });

  it("a remote end while nobody is looking collapses instead of jumping (C22)", () => {
    const [a, b] = [req(), req()];
    const state = markSecondary(build(a, b), b.requestId).state;
    const out = finish(state, a.requestId, { engaged: false });
    expect(out.effect).toBe("collapse");
    expect(out.state.view).toBe("collapsed");
    expect(out.closed).toBe(false);
    expect(out.state.requests.map(r => r.requestId)).toEqual([b.requestId]);
    expect(out.state.active).toBe(b.requestId);
  });

  it("an engaged remote end still prefers the next primary request", () => {
    const [a, b, c] = [req(), req(), req()];
    const state = markSecondary(build(a, b, c), b.requestId).state;
    expect(finish(state, a.requestId, { engaged: false }).state.active).toBe(c.requestId);
    expect(finish(state, a.requestId, { engaged: false }).effect).toBe("switch");
  });

  it("closes the round when the queue empties and resets the view", () => {
    const a = req();
    const out = finish(build(a), a.requestId);
    expect(out.closed).toBe(true);
    expect(out.state.requests).toEqual([]);
    expect(out.state.active).toBeNull();
    expect(out.state.view).toBe("expanded");
  });

  it("closes the round after the last request ends while the window is small", () => {
    const a = req();
    const collapsed = putAway(build(a)).state;
    const out = finish(collapsed, a.requestId);
    expect(out.closed).toBe(true);
  });

  it("removing another request while small only resizes the small window", () => {
    const [a, b, c] = [req(), req(), req()];
    let state = build(a, b, c);
    state = putAway(markSecondary(markSecondary(state, b.requestId).state, c.requestId).state).state;
    expect(state.view).toBe("collapsed");
    const out = finish(state, c.requestId);
    expect(out.effect).toBe("mini-resize");
    expect(out.state.view).toBe("collapsed");
  });

  it("removing the remembered active request while small stays small", () => {
    const [a, b] = [req(), req()];
    const state = putAway(putAway(build(a, b)).state).state;
    expect(state.active).toBe(b.requestId);
    const out = finish(state, b.requestId);
    expect(out.effect).toBe("mini-resize");
    expect(out.state.view).toBe("collapsed");
    expect(out.state.active).toBe(a.requestId);
  });

  it("a remote end of a non-active request never changes the body", () => {
    const [a, b] = [req(), req()];
    const out = finish(build(a, b), b.requestId);
    expect(out.state.active).toBe(a.requestId);
    expect(out.effect).toBeNull();
  });

  it("ignores an id that is already gone", () => {
    const state = build(req());
    const out = finish(state, "missing");
    expect(out.state).toBe(state);
    expect(out.closed).toBe(false);
  });
});

describe("ordering helpers", () => {
  it("groups by first-seen project and keeps arrival order inside a group", () => {
    const a = req("/p/a");
    const b = req("/p/b");
    const c = req("/p/a");
    expect(orderedRequests(build(a, b, c)).map(r => r.requestId)).toEqual([a.requestId, c.requestId, b.requestId]);
  });

  it("orders by sequence inside a group, like the Sidebar does", () => {
    const late = req("/p/a");
    const early = req("/p/a");
    const reversed = { ...build(), requests: [late, early], groupOrder: ["/p/a"] };
    expect(orderedRequests(reversed).map(r => r.requestId)).toEqual([late.requestId, early.requestId]);
    const swapped = { ...reversed, requests: [{ ...late, sequence: 99 }, early] };
    expect(orderedRequests(swapped).map(r => r.requestId)).toEqual([early.requestId, late.requestId]);
  });

  it("ignores marks and restores of requests that are not in the queue", () => {
    const state = build(req());
    expect(markSecondary(state, "missing").state).toBe(state);
    expect(unmark(state, "missing").state).toBe(state);
    expect(unmark(state, "missing").effect).toBeNull();
  });

  it("opens last browsed, then first unread, then the first request", () => {
    const [a, b, c] = [req(), req(), req()];
    let state = build(a, b, c);
    expect(defaultOpenTarget(state)).toBe(a.requestId); // last browsed is the first (active) request
    state = { ...state, lastBrowsed: null };
    expect(defaultOpenTarget(state)).toBe(b.requestId); // first unread after a was visited
    state = { ...state, requests: state.requests.map(r => ({ ...r, visited: true })) };
    expect(defaultOpenTarget(state)).toBe(a.requestId);
    expect(defaultOpenTarget(emptyState())).toBeNull();
  });

  it("reports pure outcomes: input state is never mutated", () => {
    const [a, b] = [req(), req()];
    const state = build(a, b);
    const snapshot = JSON.stringify(state);
    const outcomes: Outcome[] = [putAway(state), markSecondary(state, b.requestId), browse(state, b.requestId), finish(state, a.requestId), arrive(state, req())];
    expect(outcomes.length).toBe(5);
    expect(JSON.stringify(state)).toBe(snapshot);
  });
});
