<script setup lang="ts">
import { computed, markRaw, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "vue-i18n";
import PopupView from "./PopupView.vue";
import InboxSidebar from "./popup/InboxSidebar.vue";
import InboxArrivalNotice from "./popup/InboxArrivalNotice.vue";
import { arrivalTiming, type ArrivalEnd, type ArrivalGeometry } from "./popup/inboxArrival";
import type { DropAim, DropReceiver, PopupScope, InboxLayout } from "./popup/usePopupCore";
import type { DropLogEntry, DropLogTarget, PopupInboxRequest } from "../lib/types";
import { cancelPopup, confirmPopupReady, popupDropLog, popupInboxActivate, popupInboxIdle, popupInboxInit, popupShowWindow } from "../lib/ipc";
import { isMac, primaryShortcutLabel } from "../lib/platform";
import { inboxGroups, inboxKind, inboxTitle, nextInboxRequest } from "./popup/inboxQueue";
import * as model from "./popup/inboxCollapse";
import type { InboxState, View } from "./popup/inboxCollapse";
import { answerTransition, transitionClock, type CompletionFeedback, type InboxMotion } from "./popup/inboxTransition";
import { formAnimation, measureMini, type FormAnimation, type FormChange } from "./popup/inboxMini";
import { acceptsFiles, DRAG, type DropReport } from "./popup/inboxDrop";
import { addedCount, createDragLog, describeIgnored, describeResult, NOTICE_MS, resultLog, type DropWords } from "./popup/dropFeedback";
import { useInboxDrop } from "./popup/useInboxDrop";

const { t } = useI18n();
const layoutReview = new URLSearchParams(location.search).get("layoutReview") === "1";
const allocation = ref<InboxLayout | null>(null);
const nativeCanvas = computed(() => !!allocation.value?.canvas);
const frozenCanvas = computed(() => !!allocation.value?.canvas && allocation.value.canvas.frozen !== false);
const rootStyle = computed(() => {
  const a = allocation.value;
  const c = a?.canvas;
  const left = a ? a.sidebarWidth + (a.sidebarWidth > 0 ? 6 : 0) : 0;
  return { '--inbox-sidebar': `${listWidth.value}px`,
    '--ms': `${motionMs.value}ms`, '--mini-w': `${mini.value.width}px`, '--mini-h': `${mini.value.height}px`, '--mini-nav-h': `${mini.value.navHeight}px`,
    '--inbox-canvas-left': `${c?.left ?? 0}px`,
    '--inbox-main': c && !frozenCanvas.value && !a?.previewWidth ? 'calc(100vw - var(--inbox-canvas-left))' : `${a?.mainWidth ?? 560}px`,
    '--inbox-body-width': `${a ? a.mainWidth + (a.previewWidth > 0 ? 6 + a.previewWidth : 0) : 560}px`,
    '--inbox-visible-left': `${c ? c.left - left : 0}px`,
    '--inbox-visible-width': c && !frozenCanvas.value ? 'calc(100vw - var(--inbox-visible-left))' : `${a?.frame?.width ?? 560}px`,
  };
});
const requests = ref<PopupInboxRequest[]>([]);
const sessionTitles = new Map<string, string>();
const active = ref<string | null>(null);
const groupOrder = ref<string[]>([]);
const sidebar = ref(false);
const sidebarWidth = ref(0);
const pin = ref<boolean | null>(null);
// A single request has no Sidebar column, but its small window is the list: before it shrinks the list is laid out
// hidden (to be measured) and after it expanded the list fades out, both at this width.
const loneList = ref(0);
const listWidth = computed(() => sidebarWidth.value || loneList.value);
const renderedSidebar = computed(() => listWidth.value > 0);
const previews = new Map<string, boolean>();
const nativePreviewSync = new Map<string, () => Promise<void>>();
const visited = ref(new Set<string>());
// "Answer later" (secondary) marks, and whether the window is the small one (docs/specs/popup-inbox-collapse.md).
const secondary = ref(new Set<string>());
const view = ref<View>("expanded");
const shifting = ref(false);
// What is painted. `view` is what the page asks of the window; `look` follows it when the native change starts, so
// the cross-fade runs with the window instead of ahead of it. The rest describes the small window's layout.
const look = ref<View>("expanded");
const settled = ref(false);
const mini = ref({ width: 0, height: 0, navHeight: 0 });
// How long the page's cross-fade takes, for the duration of a window form change only: anything else that changes the
// layout (a Sidebar appearing, a resize) happens at once, together with the native commit.
const motionMs = ref(0);
// Whether the user is in AskHuman right now; the native app-activation event keeps it current.
const appActive = ref(true);
let lastBrowsed: string | null = null;
const drafts = ref(new Set<string>());
const flashed = ref(new Set<string>());
const arriving = ref(new Set<string>());
const arrivalNotice = ref<InstanceType<typeof InboxArrivalNotice> | null>(null);
const pendingArrivals = new Set<string>();
// Newcomers whose bubble plays although their body is already open: the small window opened onto them (C4).
const shownArrivals = new Set<string>();
let hostPresented = false;
const recovered = ref(false);
const error = ref("");
const notice = ref("");
// A warning (files that did not get attached, and why) stays longer and is marked; a plain notice just informs.
const noticeLevel = ref<"info" | "warn">("info");
const modal = ref<{ ids: string[]; current: string | null } | null>(null);
const cancelBusy = ref(false);
const advancing = ref(false);
const continueButton = ref<HTMLButtonElement | null>(null);
const root = ref<HTMLElement | null>(null);
const scopes = new Map<string, PopupScope>();
const readyIds = new Set<string>();
const failedIds = new Set<string>();
const preparedIds = ref(new Set<string>());
const readyWaiters = new Map<string, Set<() => void>>();
const focusRestorers = new Map<string, () => void>();
const earlyArrivals = new Set<string>();
const motion = ref<InboxMotion | null>(null);
// Switching between requests in an expanded window (C17): the body being left fades out, then the next one fades in.
const swap = ref<{ from: string; incoming: string; phase: "out" | "prepare" | "in" } | null>(null);
let clock: ReturnType<typeof transitionClock> | null = null;
const terminals = new Set<string>();
const listeners: UnlistenFn[] = [];
const timers = new Set<ReturnType<typeof setTimeout>>();
const remembered = new Map<string, { element: HTMLElement; start?: number; end?: number }>();
let disposed = false;
let tail = Promise.resolve();
const groups = computed(() => inboxGroups(requests.value, groupOrder.value));
const ordered = computed(() => groups.value.flatMap(group => group.requests));
const mounted = computed(() => requests.value.filter(r => visited.value.has(r.requestId) || preparedIds.value.has(r.requestId)));
const blocked = computed(() => !!modal.value || advancing.value || shifting.value || (!!swap.value && swap.value.phase !== "in"));
// Whether any request is still waiting; when none is, the expanded window is a "peek" (C27).
const hasPrimary = computed(() => requests.value.some(r => !secondary.value.has(r.requestId)));
// The row follows the click at once, while the body it will show is still fading out.
const selectedId = computed(() => swap.value?.phase === "out" ? swap.value.incoming : motion.value?.phase === "prepare" ? motion.value.from : active.value);
const snapshot = computed(() => requests.value.filter(r => modal.value?.ids.includes(r.requestId)));
const newAfterModal = computed(() => requests.value.filter(r => !modal.value?.ids.includes(r.requestId)).length);
const currentSnapshot = computed(() => snapshot.value.filter(r => r.requestId === modal.value?.current));
function later(fn: () => void, ms: number) {
  const timer = setTimeout(() => { timers.delete(timer); if (!disposed) fn(); }, ms);
  timers.add(timer);
}
function unreadArrival(id: string) {
  return (!visited.value.has(id) || shownArrivals.has(id)) && !terminals.has(id) && requests.value.some(request => request.requestId === id);
}
function cancelArrival(id: string) {
  pendingArrivals.delete(id); arriving.value.delete(id); flashed.value.delete(id); shownArrivals.delete(id);
  arrivalNotice.value?.cancel(id);
}
function arrivalSettled(id: string, reason: ArrivalEnd) {
  pendingArrivals.delete(id); arriving.value.delete(id);
  const due = unreadArrival(id);
  shownArrivals.delete(id);
  if (reason !== "landed" || !due) return;
  flashed.value.add(id);
  later(() => flashed.value.delete(id), arrivalTiming.land);
}
function arrivalRow(id: string) {
  return root.value?.querySelector<HTMLElement>(`[data-inbox-row="${CSS.escape(id)}"]`);
}
function measureArrival(id: string): ArrivalGeometry | null {
  if (!unreadArrival(id) || !root.value) return null;
  const row = arrivalRow(id);
  const nav = row?.closest(".inbox-navigation")?.getBoundingClientRect();
  const dot = row?.querySelector(".inbox-dot")?.getBoundingClientRect();
  const main = root.value.querySelector<HTMLElement>(`[data-inbox-request="${CSS.escape(active.value ?? "")}"] .popup-main`)?.getBoundingClientRect();
  const sidebarBounds = root.value.querySelector(".inbox-sidebar")?.getBoundingClientRect();
  if (!nav || !dot || !main || !sidebarBounds || !dot.width || !main.width) return null;
  const x = dot.left + dot.width / 2, y = dot.top + dot.height / 2;
  // Follow layout changes without scrolling again during a flight.
  if (x - 5 < nav.left || x + 5 > nav.right || y - 5 < nav.top || y + 5 > nav.bottom) return null;
  const bounds = root.value.getBoundingClientRect();
  return {
    origin: { x: (sidebarBounds.left + main.right) / 2 - bounds.left, y: (main.top + main.bottom) / 2 - bounds.top - 10 },
    target: { x: x - bounds.left, y: y - bounds.top },
  };
}
function flushArrivals() {
  if (!hostPresented || document.hidden || advancing.value || !arrivalNotice.value) return;
  for (const id of [...pendingArrivals]) {
    pendingArrivals.delete(id);
    if (!unreadArrival(id)) { cancelArrival(id); continue; }
    const row = arrivalRow(id), nav = row?.closest<HTMLElement>(".inbox-navigation");
    if (row && nav) {
      const r = row.getBoundingClientRect(), n = nav.getBoundingClientRect();
      // Change only the navigation scroll offset; scrollIntoView can move the answer.
      if (r.top < n.top) nav.scrollTop += r.top - n.top;
      else if (r.bottom > n.bottom) nav.scrollTop += r.bottom - n.bottom;
    }
    void arrivalNotice.value.play(id);
  }
}
watch(advancing, () => { void nextTick(flushArrivals); });
function enqueue(fn: () => Promise<unknown>) {
  tail = tail.then(async () => { if (!disposed) await fn(); }).catch(e => {
    if (disposed) return;
    // Superseded or expired geometry is an internal coordination result. Native
    // diagnostics retain its cause; it does not tell the user what action to take.
    if (String(e) === "preview geometry changed") return;
    error.value = String(e);
  });
  return tail;
}
function rememberFocus() {
  if (!active.value) return;
  const element = document.activeElement;
  if (!(element instanceof HTMLElement) || !element.closest(`[data-inbox-request="${CSS.escape(active.value)}"]`)) return;
  const editor = element instanceof HTMLTextAreaElement || element instanceof HTMLInputElement ? element : null;
  remembered.set(active.value, { element, start: editor?.selectionStart ?? undefined, end: editor?.selectionEnd ?? undefined });
}
function settleReady(id: string) { readyWaiters.get(id)?.forEach(resolve => resolve()); readyWaiters.delete(id); }
async function prepareRequest(id: string, timer: ReturnType<typeof transitionClock>) {
  if (terminals.has(id) || timer.cancelled) return;
  preparedIds.value.add(id);
  await nextTick();
  if (readyIds.has(id) || failedIds.has(id) || terminals.has(id) || timer.cancelled) return;
  await new Promise<void>(resolve => {
    const finish = () => { off(); readyWaiters.get(id)?.delete(finish); resolve(); };
    const off = timer.onCancel(finish);
    const waiters = readyWaiters.get(id) ?? new Set<() => void>();
    waiters.add(finish); readyWaiters.set(id, waiters);
  });
}
function restoreFocus(id: string) {
  const saved = remembered.get(id);
  if (saved?.element.isConnected) {
    saved.element.focus({ preventScroll: true });
    if (saved.element instanceof HTMLTextAreaElement || saved.element instanceof HTMLInputElement) {
      if (saved.start !== undefined) saved.element.setSelectionRange(saved.start, saved.end ?? saved.start);
    }
  } else focusRestorers.get(id)?.();
}
// Files dragged over the window (docs/specs/popup-inbox-collapse.md C28–C32): one window-level controller routes them to any
// request. Each request's core lends it a receiver, and shows what the pointer would drop onto in the request.
const receivers = new Map<string, DropReceiver>();
/** A window change the drag controller started after its drag had left the window (the button is still down). */
let carrying = false;
const droppable = computed(() => new Set(requests.value.filter(r => acceptsFiles(r) && !terminals.has(r.requestId)).map(r => r.requestId)));
// Every drag of the window leaves a line in the diagnostic log, and a drop that adds nothing says why (dropFeedback.ts).
const dragLog = createDragLog(entry => { void popupDropLog(entry); }, () => ({ view: view.value, requestId: active.value }));
const dropWords = (): DropWords => ({ t, mac: isMac, paste: primaryShortcutLabel("v") });
function shortTitle(request: PopupInboxRequest) {
  const full = inboxTitle(request) || t("popup.inbox.untitled");
  return full.length > 28 ? `${full.slice(0, 28)}…` : full;
}
/** The controller's word on a drag: its start and end go to the log; a drop turned away also goes to the user. */
function reportDrop(report: DropReport) {
  if (report.kind === "seen") { dragLog.seen(report.from, report.paths, report.ignored ? { outcome: report.ignored } : {}); return; }
  if (report.kind === "leave") { dragLog.left(); return; }
  dragLog.end();
  if (report.reason !== "own_drag_out" && report.reason !== "no_target") {
    const request = requests.value.find(r => r.requestId === report.id);
    const told = describeIgnored(report.reason, dropWords(), request ? shortTitle(request) : undefined);
    say(told.text, told.level);
  }
  dragLog.send({ action: "drop", paths: report.paths, outcome: report.reason, target: report.target, requestId: report.id });
}
const drop = useInboxDrop({
  root: () => root.value, layout: () => allocation.value, view: () => view.value, active: () => active.value,
  blocked: () => blocked.value, fallback: () => model.defaultOpenTarget(collapseState()),
  accepts: id => !terminals.has(id) && droppable.value.has(id),
  receiver: id => receivers.get(id),
  open: id => enqueue(() => openForDrag(id)),
  deliver: deliverDrop, report: reportDrop,
  // The drag has left the window but still holds the button: the shrink must not wait for it to come up.
  springBack: () => { void enqueue(async () => { carrying = true; try { await peekShrink(); } finally { carrying = false; } }); },
  fail: e => { error.value = String(e); dragLog.send({ action: "error" }); },
});
function scopeFor(id: string): PopupScope {
  let scope = scopes.get(id);
  if (!scope) {
    scope = markRaw<PopupScope>({ layout: allocation, requestId: id, active: computed(() => active.value === id), blocked, pin,
      dropReceiver: receiver => { if (receiver) receivers.set(id, receiver); else receivers.delete(id); },
      // Only the open request shows an aim, and only inside the request: a row is the Sidebar's to light.
      dropAim: computed<DropAim | null>(() => {
        const hit = drop.aim.value;
        return hit && hit.id === id && hit.kind !== "row" ? { question: hit.kind === "card" ? hit.question : null } : null;
      }),
      completion: computed(() => motion.value?.from === id ? motion.value.completion : null),
      nativePreviewBlocked: computed(() => (blocked.value || view.value === "collapsed")
        && !(motion.value?.incoming === id && ["prepare", "in"].includes(motion.value.phase))
        && !(swap.value?.incoming === id && swap.value.phase !== "out")),
      secondary: computed(() => secondary.value.has(id)),
      markLater: () => markLater(id),
      unmarkLater: () => unmarkLater(id),
      shrinkWindow,
      restoreFocus: restore => { if (restore) focusRestorers.set(id, restore); else focusRestorers.delete(id); },
      failed: () => { failedIds.add(id); settleReady(id); },
      sending: () => {
        const next = nextInboxRequest(ordered.value, id, terminals);
        if (next) preparedIds.value.add(next);
      },
      ready: () => {
        readyIds.add(id); settleReady(id);
        void enqueue(async () => {
        if (!requests.value.some(r => r.requestId === id)) return;
        if (active.value === id) await popupShowWindow(id);
      }); },
      close: openClose,
      preview: async (open: boolean, extent?: number, finished = true) => {
        previews.set(id, open);
        let result: { revision: number; mainWidth: number; mainHeight: number; previewWidth: number; limited: boolean } | undefined;
        await enqueue(async () => {
          if (active.value !== id) return;
          if (extent === undefined) result = await geometry();
          else {
            const resized = await invoke<InboxLayout>("popup_inbox_resize_preview", { requestId: id, mainExtent: extent, finished });
            if (active.value === id) allocation.value = resized;
            result = resized;
          }
        });
        if (!result) throw new Error("request is no longer active");
        return { revision: result.revision, mainWidth: result.mainWidth, mainHeight: result.mainHeight, side: result.previewWidth > 0 ? result.limited ? "inside" : "right" : "closed" };
      },
      draft: (_: string, hasDraft: boolean) => { if (hasDraft) drafts.value.add(id); else drafts.value.delete(id); },
      nativePreviewSync: sync => { if (sync) nativePreviewSync.set(id, sync); else nativePreviewSync.delete(id); },
    });
    scopes.set(id, scope);
  }
  return scope;
}
// What a layout transaction changes about the window form: the small window's size, how the change is animated, and
// what to do the moment native starts it (the page's cross-fade starts with the window, not ahead of it).
interface Form { collapsed?: { width: number; height: number }; animation?: FormAnimation; started?: () => void }
async function geometry(arrival = false, form: Form = {}) {
  let prepared: InboxLayout | undefined;
  try {
    const current = allocation.value;
    if (current?.canvas && !frozenCanvas.value) {
      // Pin the currently used widths before native expands the offscreen viewport. The
      // main canvas origin remains identical in both layouts, including the handoff frame.
      const main = root.value?.querySelector<HTMLElement>(`[data-inbox-request="${CSS.escape(active.value ?? "")}"] .popup-main`);
      const body = root.value?.querySelector<HTMLElement>(".inbox-body");
      const bodyWidth = body?.getBoundingClientRect().width || current.mainWidth + (current.previewWidth > 0 ? 6 + current.previewWidth : 0);
      const mainWidth = main?.getBoundingClientRect().width || (current.previewWidth > 0 ? current.mainWidth : bodyWidth);
      allocation.value = { ...current, mainWidth,
        previewWidth: current.previewWidth > 0 ? Math.max(1, bodyWidth - mainWidth - 6) : 0,
        canvas: { ...current.canvas, frozen: true } };
      await nextTick();
      // Force the pin to be applied before the native command changes WebKit's viewport.
      body?.getBoundingClientRect();
    }
    prepared = await invoke<InboxLayout>("popup_inbox_layout", {
      // The small window is always a list column, even for a single request that has no Sidebar when expanded.
      sidebar: sidebar.value || view.value === "collapsed", sidebarWidth: null, preview: previews.get(active.value ?? "") ?? false, mainExtent: null,
      // The page states the window form every time; native treats a missing size as the expanded window.
      ...(view.value === "collapsed" ? { collapsed: form.collapsed ?? { width: mini.value.width, height: mini.value.height } } : {}),
      // Native holds a window change back while the primary button is down (an edge drag in progress). Files dragged over the
      // window hold it on purpose, so a change they set off must not wait for the button to come up.
      ...(drop.dragging.value || carrying ? { dragging: true } : {}),
    });
    allocation.value = prepared;
    sidebarWidth.value = prepared.sidebarWidth;
    await nextTick();
    // Native PDF/Quick Look shares the fixed canvas and is placed before the window reveals it.
    if (active.value) await nativePreviewSync.get(active.value)?.();
    if (disposed) throw new Error("popup inbox disposed during layout preparation");
    form.started?.();
    await invoke("popup_inbox_commit", { revision: prepared.revision, ...(form.animation ? { animation: form.animation } : {}) });
    const finished = await invoke<InboxLayout | undefined>("popup_inbox_finish", { revision: prepared.revision, arrival });
    if (finished) allocation.value = finished;
    return prepared;
  } catch (e) {
    if (prepared) await invoke("popup_inbox_finish", { revision: prepared.revision, arrival: false }).catch(() => {});
    throw e;
  }
}
// The page's requests as the pure model sees them. Requests whose end is already known (queued behind the current
// work) are left out, except the one a decision is about (`keep`) or one that is about to be added (`without`).
function collapseState(options: { keep?: string; without?: string } = {}): InboxState {
  return {
    requests: requests.value
      .filter(r => r.requestId !== options.without && (!terminals.has(r.requestId) || r.requestId === options.keep))
      .map(r => ({ requestId: r.requestId, project: r.project, sequence: r.sequence,
        secondary: secondary.value.has(r.requestId), visited: visited.value.has(r.requestId) })),
    groupOrder: groupOrder.value, active: active.value, view: view.value, lastBrowsed,
  };
}
function newcomer(show: PopupInboxRequest) {
  return { requestId: show.requestId, project: show.project, sequence: show.sequence, secondary: false, visited: false };
}
const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
// The painted side of a window form change: the cross-fade runs for as long as the window animates.
function change(kind: FormChange, to: View, collapsed?: { width: number; height: number }): Form {
  const animation = formAnimation(kind, reducedMotion());
  return { collapsed, animation, started: () => { motionMs.value = animation?.durationMs ?? 0; look.value = to; } };
}
// The small window's size: the list's width, and the height measured from the list in its small layout.
function smallWindow() {
  const height = root.value && measureMini(root.value);
  if (!height) throw new Error("The small window cannot be measured");
  return { width: listWidth.value, ...height };
}
// Native refuses to shrink a maximized or full-screen window; the request stays marked and the window stays open.
const shrinkRefused = (e: unknown) => String(e).includes("cannot shrink");
// Go to (or stay in) the small window form; `resting` is the request it remembers as its own.
async function shrink(resting: string | null) {
  // A close confirmation is on screen, so the user is here: the small window has no room for it.
  if (modal.value) return;
  const from = view.value;
  rememberFocus();
  if (resting !== null) active.value = resting;
  // Arrival bubbles belong to the expanded window.
  for (const id of [...pendingArrivals, ...arriving.value]) cancelArrival(id);
  view.value = "collapsed"; shifting.value = true;
  try {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    // A single request has no Sidebar column yet: lay its list out hidden, at the width it will have, to measure it.
    if (!renderedSidebar.value) loneList.value = allocation.value?.sidebarPreferred ?? 240;
    await nextTick();
    const size = smallWindow();
    settled.value = false;
    mini.value = size;
    await geometry(false, change(from === "expanded" ? "collapse" : "resize", "collapsed", { width: size.width, height: size.height }));
    settled.value = true;
  } catch (e) {
    view.value = from; look.value = from; settled.value = from === "collapsed";
    if (!shrinkRefused(e)) throw e;
  } finally { shifting.value = false; loneList.value = 0; motionMs.value = 0; }
}
// The heading's icons: the small window opens (what the user was last on), a peek goes back to the small window.
function expandFromList() {
  if (blocked.value) return;
  void enqueue(async () => {
    const target = model.defaultOpenTarget(collapseState());
    if (target) await select(target, true);
  });
}
// A peek (expanded, nothing left to answer) goes back to the small window; a request that is still waiting keeps the
// window open, and marks and drafts are untouched.
async function peekShrink() {
  const outcome = model.collapseWindow(collapseState());
  if (outcome.effect === "collapse") await shrink(outcome.state.active);
}
function shrinkWindow() {
  if (blocked.value) return;
  void enqueue(peekShrink);
}
// C23: the user left AskHuman while the window is a peek: it shrinks back. Checked inside the queue, so a window change
// in flight finishes first and the user having come back meanwhile cancels it. Files being dragged over the window are
// the user working with it, wherever the focus is (spec §2.5): the drag itself shrinks the window back if it gives up.
function collapseOnLeave() {
  void enqueue(async () => {
    if (appActive.value || drop.dragging.value) return;
    await peekShrink();
  });
}
// Focus can hop between two windows of the process in a blink; only leaving for longer than this counts as leaving.
const LEAVE_GRACE_MS = 200;
let leaving: ReturnType<typeof setTimeout> | null = null;
function appActivity(active: boolean) {
  appActive.value = active;
  if (leaving) { clearTimeout(leaving); timers.delete(leaving); leaving = null; }
  if (active) return;
  const timer = leaving = setTimeout(() => {
    timers.delete(timer); leaving = null;
    if (!disposed) collapseOnLeave();
  }, LEAVE_GRACE_MS);
  timers.add(timer);
}
// Mount the newcomer hidden, wait until it can be answered, then open it: the small window must not reveal an empty body.
async function openArrival(id: string, arrival: boolean) {
  const timer = transitionClock();
  try { await prepareRequest(id, timer); } finally { timer.cancel(); }
  if (disposed || terminals.has(id) || !requests.value.some(r => r.requestId === id)) return;
  await select(id, false, arrival);
}
// The footer / Sidebar "answer later" actions (C10, C11, C24). Marking only changes a display mark: nothing is
// answered or cancelled, and a mark never survives the round.
function markLater(id: string) { if (!blocked.value) void enqueue(() => applyMark(id, true)); }
function unmarkLater(id: string) { if (!blocked.value) void enqueue(() => applyMark(id, false)); }
async function applyMark(id: string, mark: boolean) {
  if (terminals.has(id) || !requests.value.some(r => r.requestId === id)) return;
  const state = collapseState();
  const outcome = mark ? model.markSecondary(state, id) : model.unmark(state, id);
  if (mark) secondary.value.add(id); else secondary.value.delete(id);
  const target = outcome.state.active;
  if (outcome.effect === "collapse" || outcome.effect === "mini-resize") await shrink(target);
  // Jumping to the next request after putting one away swaps the body like a click on its row does (C17).
  else if ((outcome.effect === "switch" || outcome.effect === "expand") && target) await select(target, false, false, true);
}
// C17: the body being left fades out before the next request takes its place. Nothing to wait for with reduced motion,
// without a body on screen, or while a body that is itself still fading in is replaced. "fade" means the incoming
// body fades in; "ended" means the request ended during the wait, so the body that was fading out stays.
async function leaveBody(id: string): Promise<"none" | "fade" | "ended"> {
  const from = active.value;
  if (!from || from === id || reducedMotion() || view.value !== "expanded" || motion.value) return "none";
  if (!requests.value.some(r => r.requestId === from) || terminals.has(from)) return "none";
  if (swap.value?.phase === "in") return "fade";
  const timer = transitionClock(); clock = timer;
  swap.value = { from, incoming: id, phase: "out" };
  // Mount the next body while the current one fades out.
  preparedIds.value.add(id);
  try {
    await nextTick();
    // The animation starts from a laid-out frame.
    root.value?.querySelector<HTMLElement>(`[data-inbox-request="${CSS.escape(from)}"] .content`)?.getBoundingClientRect();
    if (!await timer.wait(answerTransition.exit)) { swap.value = null; return "none"; }
  } finally { timer.cancel(); if (clock === timer) clock = null; }
  if (terminals.has(id) || !requests.value.some(r => r.requestId === id)) { swap.value = null; return "ended"; }
  return "fade";
}
// The incoming body fades in once the window has its final geometry; the swap ends with the fade.
function swapIn() {
  const current = swap.value;
  if (!current) return;
  current.phase = "in";
  later(() => { if (swap.value === current) swap.value = null; }, answerTransition.enter);
}
async function select(id: string, explicit = false, arrival = false, animate = false) {
  if (!requests.value.some(r => r.requestId === id)) return;
  rememberFocus();
  // R6/C9: opening a request from the small window expands it; the request keeps its mark.
  const expanding = model.browse(collapseState(), id).effect === "expand";
  await popupInboxActivate(id);
  const left = animate && !expanding ? await leaveBody(id) : "none";
  if (left === "ended") return;
  const swapping = left === "fade";
  if (swapping) swap.value = { from: active.value!, incoming: id, phase: "prepare" };
  active.value = id;
  visited.value.add(id);
  lastBrowsed = id;
  cancelArrival(id);
  // The list the small window shows, in case the expansion has to be undone.
  const list = sidebarWidth.value;
  if (expanding) {
    view.value = "expanded"; shifting.value = true; settled.value = false;
    // A single request has no Sidebar column once expanded: its list stays, fading out, until the window has grown.
    if (!sidebar.value) loneList.value = list;
  }
  try {
    await nextTick();
    await geometry(arrival, expanding ? change("expand", "expanded") : {});
  } catch (e) {
    if (expanding) { view.value = "collapsed"; look.value = "collapsed"; settled.value = true; sidebarWidth.value = list; }
    throw e;
  } finally {
    if (expanding) { shifting.value = false; loneList.value = 0; motionMs.value = 0; }
    if (swapping) swapIn();
  }
  // The body is inert while the window changes form; focus can only return once Vue has lifted that.
  if (expanding) await nextTick();
  if (readyIds.has(id)) await popupShowWindow(id);
  restoreFocus(id);
  if (explicit) await getCurrentWindow().setFocus();
  if (!modal.value) error.value = "";
}
function choose(id: string) {
  if (blocked.value) return;
  void enqueue(() => select(id, false, false, true));
}
// A drag rested on a row (C31): the small window opens onto its request, an open window switches its body, like a click
// (the mark stays, and no focus is taken: the user is still carrying files). Runs in the queue.
async function openForDrag(id: string) {
  if (terminals.has(id) || !requests.value.some(r => r.requestId === id) || blocked.value) return;
  await select(id, false, false, true);
}
// A request that has never been opened has no core yet: mount it hidden and wait until it can take files.
async function receiverFor(id: string): Promise<DropReceiver | undefined> {
  if (!readyIds.has(id) || !receivers.has(id)) {
    const timer = transitionClock();
    void timer.wait(DRAG.mountMs).then(waited => { if (waited) timer.cancel(); });
    try { await prepareRequest(id, timer); } finally { timer.cancel(); }
  }
  return readyIds.has(id) ? receivers.get(id) : undefined;
}
function say(message: string, level: "info" | "warn" = "info") {
  notice.value = message;
  noticeLevel.value = level;
  later(() => { if (notice.value === message) notice.value = ""; }, NOTICE_MS[level]);
}
// A drop (C32): the files go to the question the pointer was on (a card), else the one the request is on; then the request
// that received them is shown, with the window in front and focused, so that leaving AskHuman afterwards shrinks a peek (C23).
// Whatever comes of the files is told to the user and logged (§6.9), including when nothing could be attached.
async function deliverDrop(id: string, paths: string[], question: number | null, landed: DropLogTarget) {
  dragLog.end();
  const log = (fields: Omit<DropLogEntry, "action">) => dragLog.send({ ...fields, action: "drop", paths, target: landed, requestId: id });
  const request = requests.value.find(r => r.requestId === id);
  if (!request || terminals.has(id)) { log({ outcome: "request_gone" }); return; }
  const title = shortTitle(request);
  const receiver = await receiverFor(id);
  if (!receiver) {
    const gone = terminals.has(id);
    if (!gone) error.value = t("popup.inbox.dropFailed", { title });
    log({ outcome: gone ? "request_gone" : "no_receiver" });
    return;
  }
  const target = question ?? receiver.currentQuestion();
  const result = await receiver.acceptDrop(paths, target);
  log(resultLog(result));
  if (terminals.has(id)) return;
  const several = request.interaction.type === "ask" && request.interaction.request.questions.length > 1;
  const told = describeResult(result, dropWords(), { title, question: several ? target : null, announce: addedCount(result) > 0 });
  if (told) say(told.text, told.level);
  await enqueue(() => revealDropped(id));
}
async function revealDropped(id: string) {
  if (terminals.has(id) || !requests.value.some(r => r.requestId === id) || blocked.value) return;
  // Already open: nothing to switch to, only the focus to take.
  if (active.value === id && view.value === "expanded") await getCurrentWindow().setFocus();
  else await select(id, true, false, true);
}
function toggleReviewSidebar() {
  void enqueue(async () => { sidebar.value = !sidebar.value; await geometry(); });
}
function append(show: PopupInboxRequest) {
  if (!groupOrder.value.includes(show.project)) groupOrder.value.push(show.project);
  requests.value.push({ ...show, agentSessionTitle: sessionTitles.get(show.requestId) ?? show.agentSessionTitle });
  if (requests.value.length > 1) sidebar.value = true;
}
async function add(show: PopupInboxRequest, arrival = true) {
  if (terminals.has(show.requestId)) return;
  const existing = requests.value.findIndex(r => r.requestId === show.requestId);
  if (existing >= 0) {
    requests.value[existing] = { ...show, agentSessionTitle: sessionTitles.get(show.requestId)
      ?? show.agentSessionTitle ?? requests.value[existing].agentSessionTitle };
    if (!earlyArrivals.delete(show.requestId)) return;
  } else append(show);
  if (active.value === null) await select(show.requestId);
  // C4: the small window opens onto the newcomer (a primary request); an expanded one keeps its body (D7).
  const target = view.value === "collapsed" ? model.arrive(collapseState({ without: show.requestId }), newcomer(show)).state.active : null;
  const opened = target !== null;
  if (opened) await openArrival(target, arrival);
  // C4: with the window opened onto the newcomer there is no unread row to point at; the bubble lands on the open one.
  if (opened && arrival && active.value === show.requestId) shownArrivals.add(show.requestId);
  const animate = arrival && unreadArrival(show.requestId);
  if (animate) arriving.value.add(show.requestId);
  await nextTick();
  // Ready means that an answer surface exists, including navigation to unopened confirmations.
  if (show.interaction.type === "confirm") await confirmPopupReady(show.requestId);
  if (show.requestId !== active.value) await popupShowWindow(show.requestId);
  if (opened) {
    if (animate) { pendingArrivals.add(show.requestId); flushArrivals(); }
    return;
  }
  if (arrival) {
    try {
      await geometry(true);
      await nextTick();
      if (animate && unreadArrival(show.requestId)) { pendingArrivals.add(show.requestId); flushArrivals(); }
      else cancelArrival(show.requestId);
    } catch (e) { cancelArrival(show.requestId); throw e; }
  } else await geometry();
}
function removeLocal(id: string) {
  cancelArrival(id);
  sessionTitles.delete(id);
  requests.value = requests.value.filter(r => r.requestId !== id);
  visited.value.delete(id); drafts.value.delete(id); flashed.value.delete(id);
  secondary.value.delete(id);
  if (lastBrowsed === id) lastBrowsed = null;
  readyIds.delete(id); scopes.delete(id); receivers.delete(id); remembered.delete(id); previews.delete(id); nativePreviewSync.delete(id);
  failedIds.delete(id); preparedIds.value.delete(id); focusRestorers.delete(id); earlyArrivals.delete(id); settleReady(id);
}
async function finishRound() {
  pendingArrivals.clear(); shownArrivals.clear(); arriving.value.clear(); arrivalNotice.value?.clear(); hostPresented = false;
  active.value = null; sidebar.value = false; sidebarWidth.value = 0; groupOrder.value = [];
  recovered.value = false; modal.value = null; error.value = ""; notice.value = "";
  pin.value = null;
  // R9: the next round starts expanded and unmarked; native resets the window position (C30).
  secondary.value.clear(); view.value = "expanded"; look.value = "expanded"; shifting.value = false; lastBrowsed = null;
  settled.value = false; loneList.value = 0; motionMs.value = 0; swap.value = null;
  // A drag that was over the window when the round ended does not carry into the next one.
  drop.reset();
  await popupInboxIdle();
}
async function advance(preferred: string | null) {
  active.value = null;
  const candidates = [preferred, ...ordered.value.map(r => r.requestId)].filter((id): id is string => !!id);
  for (const id of new Set(candidates)) {
    if (!requests.value.some(r => r.requestId === id)) continue;
    try { await select(id); error.value = ""; return; }
    catch (e) {
      if (!String(e).includes("no longer pending")) throw e;
      // A batched cancellation or remote answer may finish the chosen successor before its
      // terminal event reaches this queue. Skip it using the backend's authoritative result.
      terminals.add(id); removeLocal(id);
    }
  }
  if (!requests.value.length) await finishRound();
}
async function completed(id: string, completion: CompletionFeedback) {
  // The answer's own transition takes over from a body swap that is still fading in.
  swap.value = null;
  const timer = transitionClock(); clock = timer;
  const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  const row = root.value?.querySelector<HTMLElement>(`[data-inbox-row="${CSS.escape(id)}"]`);
  const group = row?.closest<HTMLElement>(".inbox-group");
  const from = requests.value.find(r => r.requestId === id)!;
  motion.value = { from: id, completion, phase: "waiting", incoming: null, reduced,
    rowHeight: row?.getBoundingClientRect().height ?? 0, group: from.project,
    groupHeight: group?.getBoundingClientRect().height ?? 0 };
  advancing.value = true;
  // R4/C22: the next primary request; with only secondary ones left, the user who just answered is taken to the next of them.
  const candidate = () => {
    // The answered request was the open one in an expanded window, even after the body has moved on to a successor.
    const outcome = model.finish({ ...collapseState({ keep: id }), active: id, view: "expanded" }, id, { engaged: true });
    return outcome.effect === "switch" || outcome.effect === "expand" ? outcome.state.active : null;
  };
  const first = candidate();
  const preparation = first ? prepareRequest(first, timer) : Promise.resolve();
  try {
    await preparation;
    if (disposed) return;
    // Readiness is the only gate; there is no success message or deliberate pause.
    let next = candidate();
    if (!next) { removeLocal(id); await finishRound(); return; }
    await prepareRequest(next, timer);
    if (disposed) return;
    motion.value!.phase = reduced ? "prepare" : "out";
    await nextTick();
    root.value?.querySelector<HTMLElement>(`[data-inbox-request="${CSS.escape(id)}"] .content`)?.getBoundingClientRect();
    const collapse = reduced ? Promise.resolve(true) : timer.wait(answerTransition.queue);
    if (!reduced && !await timer.wait(answerTransition.exit)) return;
    while (!disposed && !timer.cancelled) {
      next = candidate();
      if (!next) break;
      await prepareRequest(next, timer);
      if (disposed || timer.cancelled) return;
      if (terminals.has(next)) { removeLocal(next); continue; }
      try { await popupInboxActivate(next); }
      catch (e) {
        if (!String(e).includes("no longer pending")) throw e;
        terminals.add(next); removeLocal(next); continue;
      }
      if (terminals.has(next)) { removeLocal(next); continue; }
      motion.value!.incoming = next; motion.value!.phase = "prepare";
      active.value = next; lastBrowsed = next; view.value = "expanded";
      await nextTick();
      await geometry();
      if (disposed) return;
      if (terminals.has(next)) { removeLocal(next); continue; }
      // Visibility, unread state and native preview ownership change together.
      motion.value!.phase = "in";
      if (readyIds.has(next)) visited.value.add(next);
      await nextTick();
      root.value?.querySelector<HTMLElement>(`[data-inbox-request="${CSS.escape(next)}"] .content`)?.getBoundingClientRect();
      if (readyIds.has(next)) await popupShowWindow(next);
      if (!reduced && !await timer.wait(answerTransition.enter)) return;
      if (!terminals.has(next)) break;
      removeLocal(next);
    }
    await collapse;
    if (disposed) return;
    removeLocal(id);
    if (!next) await finishRound();
  } catch (e) {
    // A committed answer cannot be reopened even if preparing its successor fails.
    const preferred = candidate();
    removeLocal(id);
    if (active.value === id) active.value = null;
    if (!disposed && (!active.value || terminals.has(active.value))) await advance(preferred);
    throw e;
  } finally {
    timer.cancel(); if (clock === timer) clock = null;
    motion.value = null; advancing.value = false;
    if (!disposed && active.value && !terminals.has(active.value)) {
      await nextTick(); restoreFocus(active.value);
    }
  }
}
async function terminal(id: string, winner: string, completion?: CompletionFeedback) {
  terminals.add(id);
  if (!requests.value.some(r => r.requestId === id)) return;
  const wasActive = active.value === id;
  if (wasActive && winner === "popup" && completion && !modal.value) {
    await completed(id, completion); return;
  }
  if (wasActive && winner !== "popup") {
    advancing.value = true;
    noticeLevel.value = "info";
    notice.value = t("popup.inbox.external", { source: winner });
    clock = transitionClock(); await clock.wait(600); clock.cancel(); clock = null;
    if (disposed) return;
    advancing.value = false;
    notice.value = "";
  }
  // R4/R6/C22: decide with the queue as it is now. A user who is here (this window, or the app is active) is taken
  // to the next request; when nobody is looking and only secondary requests remain, the window goes small.
  const engaged = winner === "popup" || appActive.value || !!modal.value;
  const outcome = model.finish(collapseState({ keep: id }), id, { engaged });
  removeLocal(id);
  if (modal.value && !snapshot.value.length) modal.value = null;
  if (!requests.value.length) await finishRound();
  else if (wasActive || !requests.value.some(r => r.requestId === active.value)) {
    if (outcome.effect === "collapse" || outcome.effect === "mini-resize") await shrink(outcome.state.active);
    else await advance(outcome.state.active);
  } else if (outcome.effect === "mini-resize") await shrink(null);
}
function panePhase(id: string) {
  if (swap.value) {
    const { from, incoming, phase } = swap.value;
    return phase === "out" ? (id === from ? "out" : undefined) : id === incoming ? phase : undefined;
  }
  if (!motion.value) return undefined;
  if (id !== motion.value.from && motion.value.phase === "prepare") return "prepare";
  if (motion.value.reduced) return undefined;
  if (motion.value.incoming === id) return motion.value.phase;
  if (motion.value.from === id && motion.value.phase !== "waiting") return "out";
  return undefined;
}
// The project group that folds away with the request that just ended (its last one), if any.
function collapsingGroup() {
  const path = motion.value?.group;
  if (!path || motion.value!.reduced || motion.value!.phase === "waiting") return null;
  return requests.value.some(r => r.project === path && !terminals.has(r.requestId)) ? null : path;
}
function consequences(items: PopupInboxRequest[]) {
  const parts = (["ask", "permission", "stop"] as const).map(kind => {
    const n = items.filter(r => inboxKind(r) === kind).length;
    return n ? t(`popup.inbox.consequence.${kind}`, { n }, n) : "";
  }).filter(Boolean);
  const n = items.filter(r => drafts.value.has(r.requestId)).length;
  if (n) parts.push(t("popup.inbox.draftsLost", { n }, n));
  return parts.join(t("popup.inbox.separator"));
}
function openClose() {
  if (!requests.value.length || modal.value || advancing.value) return;
  // The confirmation has no room in the small window: expand onto the request it would open first (spec §2.3).
  if (view.value === "collapsed") {
    void enqueue(async () => {
      const target = model.defaultOpenTarget(collapseState());
      if (target) await select(target, true);
      showCloseDialog();
    });
    return;
  }
  showCloseDialog();
}
function showCloseDialog() {
  if (!requests.value.length || modal.value || advancing.value) return;
  rememberFocus();
  modal.value = { ids: requests.value.map(r => r.requestId), current: active.value };
  error.value = "";
  void nextTick(() => continueButton.value?.focus());
}
function continueAnswering() {
  if (cancelBusy.value) return;
  modal.value = null; error.value = "";
  const saved = active.value && remembered.get(active.value);
  if (saved) saved.element.focus({ preventScroll: true });
}
async function cancelSnapshot(all: boolean) {
  if (!modal.value || cancelBusy.value) return;
  const ids = (all ? snapshot.value : currentSnapshot.value).map(r => r.requestId);
  cancelBusy.value = true; error.value = "";
  try {
    // Fixed IDs exclude later arrivals, even when they appear while acknowledgements are pending.
    for (const id of ids) {
      if (requests.value.some(r => r.requestId === id)) await cancelPopup(id);
    }
    modal.value = null;
  } catch (e) { error.value = String(e); }
  finally { cancelBusy.value = false; }
}
function keydown(event: KeyboardEvent) {
  if (!modal.value || event.isComposing) return;
  if (event.key === "Escape") { event.preventDefault(); continueAnswering(); }
  if (event.key === "Tab") {
    const buttons = [...(root.value?.querySelectorAll<HTMLButtonElement>(".inbox-close-dialog button:not(:disabled)") ?? [])];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    event.preventDefault(); buttons[(index + (event.shiftKey ? -1 : 1) + buttons.length) % buttons.length]?.focus();
  }
}
function beginResize(event: PointerEvent) {
  if (event.button !== 0 || blocked.value) return;
  event.preventDefault();
  const divider = event.currentTarget as HTMLElement;
  const start = event.clientX, initial = sidebarWidth.value;
  divider.setPointerCapture(event.pointerId);
  let width = initial, frame = 0;
  let pending = false, ending = false, stopped = false, lastApplied = initial, finishSent = false;
  const apply = () => {
    frame = 0;
    if (pending || stopped || disposed) return;
    pending = true;
    void enqueue(async () => {
      const requested = width, finished = ending;
      try {
        const result = await invoke<InboxLayout>("popup_inbox_resize_sidebar", { width: requested, finished });
        allocation.value = result;
        sidebarWidth.value = result.sidebarWidth;
        lastApplied = requested; finishSent = finished;
      } catch (e) { stopped = true; throw e; }
    }).finally(() => {
      pending = false;
      if (!disposed && !stopped && !finishSent && (ending || width !== lastApplied) && !frame) frame = requestAnimationFrame(apply);
    });
  };
  const move = (e: PointerEvent) => { width = Math.max(180, initial + e.clientX - start); if (!frame) frame = requestAnimationFrame(apply); };
  const end = () => { ending = true; cancelAnimationFrame(frame); frame = 0; apply(); divider.removeEventListener("pointermove", move); divider.removeEventListener("lostpointercapture", end); };
  divider.addEventListener("pointermove", move); divider.addEventListener("lostpointercapture", end);
}
onMounted(async () => {
  window.addEventListener("keydown", keydown, true);
  document.addEventListener("visibilitychange", flushArrivals);
  listeners.push(await listen("popup-inbox-presented", () => { hostPresented = true; void nextTick(flushArrivals); }));
  listeners.push(await listen<{ requestId: string; title: string }>("popup-session-title", event => {
    const { requestId, title } = event.payload;
    if (terminals.has(requestId) || !title.trim()) return;
    // Keep enrichment that arrives while init or another request's layout is still pending.
    sessionTitles.set(requestId, title);
    const request = requests.value.find(request => request.requestId === requestId);
    if (request) request.agentSessionTitle = title;
  }));
  listeners.push(await listen<PopupInboxRequest>("popup-inbox-show", event => {
    const show = event.payload;
    if (motion.value && !terminals.has(show.requestId) && !requests.value.some(r => r.requestId === show.requestId)) {
      append(show); earlyArrivals.add(show.requestId);
      arriving.value.add(show.requestId);
    }
    void enqueue(() => add(show));
  }));
  listeners.push(await listen<{ requestId: string; winner: string; completion?: CompletionFeedback }>("popup-inbox-terminal", event => {
    const { requestId, winner, completion } = event.payload;
    if (terminals.has(requestId)) return;
    terminals.add(requestId); settleReady(requestId); cancelArrival(requestId);
    if (motion.value?.incoming === requestId) { motion.value.incoming = null; motion.value.phase = "prepare"; }
    void enqueue(() => terminal(requestId, winner, completion));
  }));
  // A popup can be shown without activating the app, so the starting state is asked of native; an event that arrives
  // before the answer is newer than it.
  let activityHeard = false;
  listeners.push(await listen<boolean>("popup-app-active", event => { activityHeard = true; appActivity(!!event.payload); }));
  void invoke<boolean>("popup_app_active")
    .then(active => { if (!activityHeard && typeof active === "boolean") appActive.value = active; })
    .catch(() => {});
  listeners.push(await listen("popup-inbox-close", openClose));
  listeners.push(await listen<string>("popup-inbox-focus", event => { void enqueue(() => select(event.payload, true)); }));
  listeners.push(await listen<InboxLayout>("popup-inbox-layout", event => { allocation.value = event.payload; sidebarWidth.value = event.payload.sidebarWidth; }));
  listeners.push(await listen("popup-inbox-reconcile", () => { void enqueue(() => geometry()); }));
  // The window's own drag-drop events (C28): one listener routes a drag to any request, instead of each request's core.
  listeners.push(await getCurrentWebview().onDragDropEvent(event => { void drop.onEvent(event.payload); }));
  const snapshot = await popupInboxInit();
  hostPresented ||= snapshot.presented ?? false;
  recovered.value = snapshot.recovered;
  await enqueue(async () => {
    // A fresh cold host may receive a burst before its WebView subscribes. Restored
    // or already-presented snapshots keep their unread state without replaying it.
    const initialArrival = !snapshot.recovered && !hostPresented;
    for (const request of snapshot.requests) await add(request, initialArrival);
    if (snapshot.focusedRequestId) await select(snapshot.focusedRequestId, true);
  });
});
onBeforeUnmount(() => {
  disposed = true; drop.dispose(); window.removeEventListener("keydown", keydown, true);
  document.removeEventListener("visibilitychange", flushArrivals);
  pendingArrivals.clear(); arrivalNotice.value?.clear();
  clock?.cancel(); readyWaiters.forEach(waiters => waiters.forEach(resolve => resolve())); readyWaiters.clear();
  listeners.forEach(off => off()); timers.forEach(clearTimeout);
});
</script>

<template>
  <div ref="root" class="inbox-root" :class="{ 'inbox-expanded': renderedSidebar, 'inbox-native': nativeCanvas, 'inbox-frozen': frozenCanvas }" :style="rootStyle" :data-completion-phase="motion?.phase"
    :data-view="look" :data-primary="hasPrimary ? 'yes' : 'no'" :data-sidebar="sidebar ? 'on' : 'off'" :data-settled="settled ? 'yes' : 'no'" :data-lights="nativeCanvas ? 'hide' : 'keep'" :data-shifting="shifting ? 'yes' : 'no'"
    :data-dragging="drop.dragging.value ? 'yes' : 'no'">
    <InboxSidebar v-if="renderedSidebar" :groups="groups" :total="requests.length" :selected-id="look === 'collapsed' ? null : selectedId" :visited="visited" :arriving="arriving" :flashed="flashed" :drafts="drafts" :secondary="secondary" :blocked="blocked" :motion="motion" :collapsing-group="collapsingGroup()"
      :droppable="droppable" :drop-target="drop.aim.value?.kind === 'row' ? drop.aim.value.id : null" :dwell="drop.dwell.value"
      @choose="choose" @remember="rememberFocus" @mark="markLater" @unmark="unmarkLater" @expand="expandFromList" @shrink="shrinkWindow" />
    <div v-if="renderedSidebar" class="inbox-divider" role="separator" aria-orientation="vertical" :aria-label="t('popup.inbox.resize')" @pointerdown="beginResize"></div>
    <main class="inbox-body" :inert="blocked || view === 'collapsed'">
      <button v-if="layoutReview" class="inbox-layout-review" @click="toggleReviewSidebar">{{ renderedSidebar ? '隐藏 Sidebar' : '显示 Sidebar' }} · 几何测试</button>
      <div v-if="recovered" class="inbox-recovery" role="alert">{{ t('popup.inbox.recovered') }}<button @click="recovered = false" :aria-label="t('popup.inbox.dismiss')">×</button></div>
      <div v-for="request in mounted" v-show="(request.requestId === active && (!terminals.has(request.requestId) || motion?.from === request.requestId)) || (motion?.phase === 'prepare' && request.requestId === motion.from)" :key="request.requestId" class="inbox-request" :class="{ 'inbox-preparing': motion?.incoming === request.requestId && motion.phase === 'prepare' }" :data-answer-phase="panePhase(request.requestId)" :data-inbox-request="request.requestId" :inert="request.requestId !== active">
        <PopupView :scope="scopeFor(request.requestId)" />
      </div>
    </main>
    <!-- The small window's accent frame; it fades in once the window has gathered. -->
    <div class="mini-ring" aria-hidden="true"></div>
    <InboxArrivalNotice ref="arrivalNotice" :measure="measureArrival" @settled="arrivalSettled" />
    <div v-if="notice" class="inbox-notice" :data-level="noticeLevel" :role="noticeLevel === 'warn' ? 'alert' : 'status'">{{ notice }}</div>
    <div v-if="modal" class="inbox-close-backdrop" @click.self="continueAnswering">
      <section class="inbox-close-dialog" role="dialog" aria-modal="true" aria-labelledby="inbox-close-title">
        <h2 id="inbox-close-title">{{ t('popup.inbox.closeTitle') }}</h2>
        <p>{{ consequences(snapshot) }}</p>
        <p v-if="newAfterModal" class="inbox-close-new">{{ t('popup.inbox.newExcluded', { n: newAfterModal }, newAfterModal) }}</p>
        <p v-if="error" class="status-error" role="alert">{{ error }}</p>
        <button ref="continueButton" class="inbox-continue" :disabled="cancelBusy" @click="continueAnswering">{{ t('popup.inbox.continue') }}</button>
        <button v-if="snapshot.length > 1" :disabled="cancelBusy || !currentSnapshot.length" @click="cancelSnapshot(false)">{{ t('popup.inbox.cancelCurrent') }}<small>{{ consequences(currentSnapshot) }}</small></button>
        <button class="inbox-cancel" :disabled="cancelBusy || !snapshot.length" @click="cancelSnapshot(true)">{{ t(snapshot.length > 1 ? 'popup.inbox.cancelAll' : 'popup.inbox.cancelOne') }}</button>
      </section>
    </div>
    <p v-else-if="error" class="inbox-error" role="alert">{{ error }}</p>
  </div>
</template>

<style>
.inbox-layout-review { position: absolute; z-index: 90; top: 4px; left: 190px; padding: 4px 12px; border: 1px solid #8884; border-radius: 6px; background: var(--surface-overlay); color: var(--text-primary); font-size: 11px; cursor: pointer; }
/* Both layouts keep the same main origin. Only the transition pins the body's extent. */
.inbox-root.inbox-native { display: block; }
/* The Sidebar column sits at the window's left edge (`--inbox-visible-left`), which is also where a single request's list is. */
.inbox-native .inbox-sidebar { position: absolute; left: var(--inbox-visible-left); top: 0; bottom: 0; width: var(--inbox-sidebar); }
.inbox-native .inbox-divider { position: absolute; left: calc(var(--inbox-canvas-left) - 6px); top: 0; bottom: 0; width: 6px; }
.inbox-native .inbox-body { position: absolute; left: var(--inbox-canvas-left); top: 0; bottom: 0; right: 0; width: auto; }
.inbox-native.inbox-frozen .inbox-body { right: auto; width: var(--inbox-body-width); }
.inbox-native .inbox-close-backdrop { left: var(--inbox-visible-left); width: var(--inbox-visible-width); right: auto; }
.inbox-native .inbox-notice, .inbox-native .inbox-error { left: calc(var(--inbox-canvas-left) + var(--inbox-main) / 2); }
.inbox-native .inbox-recovery { width: var(--inbox-main); box-sizing: border-box; }
.inbox-root {
  --ms: 0ms; --inbox-ease: cubic-bezier(.2, 0, 0, 1); --inbox-ease-in: cubic-bezier(.4, 0, 1, 1); --inbox-ease-out: cubic-bezier(0, 0, .2, 1);
  --inbox-lift: 16px; --inbox-win-radius: 16px;
  position: relative; width: 100vw; height: 100vh; display: grid; grid-template-columns: minmax(0, 1fr); overflow: hidden; color: var(--text-primary);
}
/* Measuring the small layout must neither animate nor paint anything. */
.inbox-root[data-still], .inbox-root[data-still] *, .inbox-root[data-still] *::before, .inbox-root[data-still] *::after { transition: none !important; animation: none !important; }
/* The answer body and the list cross-fade while the window changes form. */
.inbox-body { transition: opacity calc(var(--ms) * .5) var(--inbox-ease-out) calc(var(--ms) * .4), visibility 0s linear calc(var(--ms) * .4); }
.inbox-root[data-view="collapsed"] .inbox-body { opacity: 0; visibility: hidden; pointer-events: none; transition: opacity calc(var(--ms) * .42) var(--inbox-ease-in) 0ms, visibility 0s linear calc(var(--ms) * .42); }
.inbox-root[data-shifting="yes"] :is(.inbox-body, .inbox-sidebar) { will-change: opacity; }
.inbox-root[data-shifting="yes"] .sb-lift { will-change: transform; }
.inbox-divider { transition: opacity calc(var(--ms) * .3) ease; }
.inbox-root[data-sidebar="off"] .inbox-divider, .inbox-root[data-view="collapsed"] .inbox-divider { opacity: 0; }
.mini-ring {
  position: absolute; z-index: 60; left: var(--inbox-visible-left); top: 0; width: var(--mini-w); height: var(--mini-h); border-radius: var(--inbox-win-radius);
  pointer-events: none; opacity: 0; box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 70%, transparent);
  transition: opacity calc(var(--ms) * .25) ease 0ms;
}
.inbox-root[data-view="collapsed"] .mini-ring { opacity: 1; transition: opacity calc(var(--ms) * .4) ease calc(var(--ms) * .6); }
/* Files over the small window: the accent frame turns solid and bold, so it is clear that resting here opens the window. */
.inbox-root[data-dragging="yes"][data-view="collapsed"] .mini-ring {
  opacity: 1; box-shadow: inset 0 0 0 3px var(--accent), inset 0 0 26px color-mix(in srgb, var(--accent) 35%, transparent);
  transition-delay: 0ms;
}
.inbox-root.inbox-expanded { grid-template-columns: var(--inbox-sidebar) 6px minmax(0, 1fr); }
/* Only the answer body moves; native preview, navbar and footer retain their positions. */
.inbox-request[data-answer-phase="out"] .content, .inbox-request[data-answer-phase="out"] .composer-dock { animation: inbox-answer-out 126ms cubic-bezier(.4, 0, 1, 1) both; }
.inbox-request[data-answer-phase="prepare"] .content, .inbox-request[data-answer-phase="prepare"] .composer-dock { opacity: 0; transform: translateY(14px); }
.inbox-request[data-answer-phase="in"] .content, .inbox-request[data-answer-phase="in"] .composer-dock { animation: inbox-answer-in 174ms cubic-bezier(0, 0, .2, 1) both; }
@keyframes inbox-answer-out { to { opacity: 0; transform: translateY(-14px); } }
@keyframes inbox-answer-in { from { opacity: 0; transform: translateY(14px); } to { opacity: 1; transform: translateY(0); } }
.inbox-divider { cursor: col-resize; background: color-mix(in srgb, currentColor 8%, transparent); touch-action: none; }
.inbox-body { min-width: 0; min-height: 0; position: relative; display: flex; flex-direction: column; }
.inbox-body[inert] { pointer-events: none; }
.inbox-request { flex: 1; min-height: 0; height: 100%; }
.inbox-request.inbox-preparing { position: absolute; inset: 0; visibility: hidden; }
.inbox-request .popup-shell, .inbox-request .popup-status { height: 100%; width: 100%; }
.inbox-recovery { padding: 10px 14px; font-size: 12px; background: #ffe7a880; display: flex; gap: 12px; }
.inbox-recovery button { margin-left: auto; background: transparent; border: none; color: inherit; cursor: pointer; }
.inbox-notice, .inbox-error { position: absolute; z-index: 110; bottom: 30px; left: 50%; transform: translateX(-50%); padding: 10px 18px; border-radius: 12px; background: var(--surface-overlay, #fff); box-shadow: 0 4px 24px #0002; font-size: 13px; }
/* A notice only informs: it must not catch clicks meant for the footer under it, nor wrap at half the window's width. */
.inbox-notice { pointer-events: none; width: max-content; max-width: min(420px, 80%); }
/* A warning says what went wrong and what to do about it: wider, and marked with the warning color. */
.inbox-notice[data-level="warn"] { max-width: min(520px, 92%); line-height: 1.5; box-shadow: 0 4px 24px #0002, inset 3px 0 0 var(--accent-orange, #ff9500); }
.inbox-close-backdrop { position: absolute; inset: 0; z-index: 100; background: #0003; display: grid; place-items: center; }
.inbox-close-dialog { background: var(--surface-overlay, #fff); border-radius: 16px; padding: 24px; width: min(390px, calc(100% - 48px)); box-shadow: 0 12px 60px #0003; color: var(--text-primary, #222); }
.inbox-close-dialog h2 { font-size: 16px; margin: 0 0 14px; }
.inbox-close-dialog button small { display: block; font-size: 11px; font-weight: 400; opacity: .7; margin-top: 4px; }
.inbox-close-dialog p { font-size: 13px; line-height: 1.6; }
.inbox-close-dialog button { display: block; width: 100%; padding: 10px; margin-top: 9px; border: 1px solid #8883; border-radius: 8px; color: inherit; background: transparent; cursor: pointer; }
.inbox-close-dialog button:disabled { opacity: .4; cursor: default; }
.inbox-close-dialog .inbox-continue { color: white; background: #2685e8; border-color: #2685e8; }
.inbox-close-dialog .inbox-cancel { color: #d74343; }
.inbox-close-new { opacity: .6; }
.popup-submission-error { margin: 0; padding: 8px 14px; font-size: 12px; }
</style>
