<script setup lang="ts">
import { computed, markRaw, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "vue-i18n";
import PopupView from "./PopupView.vue";
import UnreadRipple from "./popup/UnreadRipple.vue";
import type { PopupScope, InboxLayout } from "./popup/usePopupCore";
import type { PopupInboxRequest } from "../lib/types";
import { cancelPopup, confirmPopupReady, popupInboxActivate, popupInboxIdle, popupInboxInit, popupShowWindow } from "../lib/ipc";
import { inboxGroups, inboxKind, inboxTitle, nextInboxRequest } from "./popup/inboxQueue";
import { answerTransition, transitionClock, type CompletionFeedback } from "./popup/inboxTransition";

const { t } = useI18n();
const layoutReview = new URLSearchParams(location.search).get("layoutReview") === "1";
const allocation = ref<InboxLayout | null>(null);
const nativeCanvas = computed(() => !!allocation.value?.canvas);
const frozenCanvas = computed(() => !!allocation.value?.canvas && allocation.value.canvas.frozen !== false);
const rootStyle = computed(() => {
  const a = allocation.value;
  const c = a?.canvas;
  const left = a ? a.sidebarWidth + (a.sidebarWidth > 0 ? 6 : 0) : 0;
  return { '--inbox-sidebar': `${sidebarWidth.value}px`,
    '--inbox-canvas-left': `${c?.left ?? 0}px`,
    '--inbox-main': c && !frozenCanvas.value && !a?.previewWidth ? 'calc(100vw - var(--inbox-canvas-left))' : `${a?.mainWidth ?? 560}px`,
    '--inbox-body-width': `${a ? a.mainWidth + (a.previewWidth > 0 ? 6 + a.previewWidth : 0) : 560}px`,
    '--inbox-visible-left': `${c ? c.left - left : 0}px`,
    '--inbox-visible-width': c && !frozenCanvas.value ? 'calc(100vw - var(--inbox-visible-left))' : `${a?.frame?.width ?? 560}px`,
  };
});
const requests = ref<PopupInboxRequest[]>([]);
const active = ref<string | null>(null);
const groupOrder = ref<string[]>([]);
const sidebar = ref(false);
const sidebarWidth = ref(0);
const pin = ref<boolean | null>(null);
const renderedSidebar = computed(() => sidebarWidth.value > 0);
const previews = new Map<string, boolean>();
const nativePreviewSync = new Map<string, () => Promise<void>>();
const visited = ref(new Set<string>());
const drafts = ref(new Set<string>());
const flashed = ref(new Set<string>());
const recovered = ref(false);
const error = ref("");
const notice = ref("");
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
const motion = ref<{ from: string; completion: CompletionFeedback; phase: "waiting" | "out" | "prepare" | "in";
  incoming: string | null; reduced: boolean; rowHeight: number; group: string | null; groupHeight: number } | null>(null);
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
const blocked = computed(() => !!modal.value || advancing.value);
const selectedId = computed(() => motion.value?.phase === "prepare" ? motion.value.from : active.value);
const snapshot = computed(() => requests.value.filter(r => modal.value?.ids.includes(r.requestId)));
const newAfterModal = computed(() => requests.value.filter(r => !modal.value?.ids.includes(r.requestId)).length);
const currentSnapshot = computed(() => snapshot.value.filter(r => r.requestId === modal.value?.current));
function later(fn: () => void, ms: number) {
  const timer = setTimeout(() => { timers.delete(timer); if (!disposed) fn(); }, ms);
  timers.add(timer);
}
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
function scopeFor(id: string): PopupScope {
  let scope = scopes.get(id);
  if (!scope) {
    scope = markRaw<PopupScope>({ layout: allocation, requestId: id, active: computed(() => active.value === id), blocked, pin,
      completion: computed(() => motion.value?.from === id ? motion.value.completion : null),
      nativePreviewBlocked: computed(() => blocked.value && !(motion.value?.incoming === id && ["prepare", "in"].includes(motion.value.phase))),
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
async function geometry(arrival = false, width?: number, mainExtent?: number) {
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
      sidebar: sidebar.value, sidebarWidth: width ?? null, preview: previews.get(active.value ?? "") ?? false, mainExtent: mainExtent ?? null,
    });
    allocation.value = prepared;
    sidebarWidth.value = prepared.sidebarWidth;
    await nextTick();
    // Native PDF/Quick Look shares the fixed canvas and is placed before the window reveals it.
    if (active.value) await nativePreviewSync.get(active.value)?.();
    if (disposed) throw new Error("popup inbox disposed during layout preparation");
    await invoke("popup_inbox_commit", { revision: prepared.revision });
    const finished = await invoke<InboxLayout | undefined>("popup_inbox_finish", { revision: prepared.revision, arrival });
    if (finished) allocation.value = finished;
    return prepared;
  } catch (e) {
    if (prepared) await invoke("popup_inbox_finish", { revision: prepared.revision, arrival: false }).catch(() => {});
    throw e;
  }
}
async function select(id: string, explicit = false) {
  if (!requests.value.some(r => r.requestId === id)) return;
  rememberFocus();
  await popupInboxActivate(id);
  active.value = id;
  visited.value.add(id);
  await nextTick();
  await geometry();
  if (readyIds.has(id)) await popupShowWindow(id);
  restoreFocus(id);
  if (explicit) await getCurrentWindow().setFocus();
  if (!modal.value) error.value = "";
}
function choose(id: string) {
  if (blocked.value) return;
  void enqueue(() => select(id));
}
function toggleReviewSidebar() {
  void enqueue(async () => { sidebar.value = !sidebar.value; await geometry(); });
}
function append(show: PopupInboxRequest) {
  if (!groupOrder.value.includes(show.project)) groupOrder.value.push(show.project);
  requests.value.push(show);
  if (requests.value.length > 1) sidebar.value = true;
}
async function add(show: PopupInboxRequest, arrival = true) {
  if (terminals.has(show.requestId)) return;
  const existing = requests.value.findIndex(r => r.requestId === show.requestId);
  if (existing >= 0) {
    requests.value[existing] = show;
    if (!earlyArrivals.delete(show.requestId)) return;
  } else append(show);
  if (active.value === null) await select(show.requestId);
  await nextTick();
  // Ready means that an answer surface exists, including navigation to unopened confirmations.
  if (show.interaction.type === "confirm") await confirmPopupReady(show.requestId);
  if (show.requestId !== active.value) await popupShowWindow(show.requestId);
  if (arrival) {
    flashed.value.add(show.requestId);
    later(() => flashed.value.delete(show.requestId), 2000);
    await geometry(true);
  } else await geometry();
}
function removeLocal(id: string) {
  requests.value = requests.value.filter(r => r.requestId !== id);
  visited.value.delete(id); drafts.value.delete(id); flashed.value.delete(id);
  readyIds.delete(id); scopes.delete(id); remembered.delete(id); previews.delete(id); nativePreviewSync.delete(id);
  failedIds.delete(id); preparedIds.value.delete(id); focusRestorers.delete(id); earlyArrivals.delete(id); settleReady(id);
}
async function finishRound() {
  active.value = null; sidebar.value = false; sidebarWidth.value = 0; groupOrder.value = [];
  recovered.value = false; modal.value = null; error.value = ""; notice.value = "";
  pin.value = null;
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
  const timer = transitionClock(); clock = timer;
  const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  const row = root.value?.querySelector<HTMLElement>(`[data-inbox-row="${CSS.escape(id)}"]`);
  const group = row?.closest<HTMLElement>(".inbox-group");
  const from = requests.value.find(r => r.requestId === id)!;
  motion.value = { from: id, completion, phase: "waiting", incoming: null, reduced,
    rowHeight: row?.getBoundingClientRect().height ?? 0, group: from.project,
    groupHeight: group?.getBoundingClientRect().height ?? 0 };
  advancing.value = true;
  const candidate = () => nextInboxRequest(ordered.value, id, terminals);
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
      active.value = next;
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
  const next = nextInboxRequest(ordered.value, active.value ?? "", new Set([id]));
  if (wasActive && winner === "popup" && completion && !modal.value) {
    await completed(id, completion); return;
  }
  if (wasActive && winner !== "popup") {
    advancing.value = true;
    notice.value = t("popup.inbox.external", { source: winner });
    clock = transitionClock(); await clock.wait(600); clock.cancel(); clock = null;
    if (disposed) return;
    advancing.value = false;
    notice.value = "";
  }
  removeLocal(id);
  if (modal.value && !snapshot.value.length) modal.value = null;
  if (!requests.value.length) await finishRound();
  else if (wasActive || !requests.value.some(r => r.requestId === active.value)) await advance(next);
}
function projectName(path: string) { return path.split(/[\\/]/).filter(Boolean).slice(-1)[0] || t("popup.inbox.unknownProject"); }
function panePhase(id: string) {
  if (!motion.value) return undefined;
  if (id !== motion.value.from && motion.value.phase === "prepare") return "prepare";
  if (motion.value.reduced) return undefined;
  if (motion.value.incoming === id) return motion.value.phase;
  if (motion.value.from === id && motion.value.phase !== "waiting") return "out";
  return undefined;
}
function collapseGroup(path: string) {
  return motion.value?.group === path && !motion.value.reduced && motion.value.phase !== "waiting"
    && !requests.value.some(r => r.project === path && !terminals.has(r.requestId));
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
  listeners.push(await listen<PopupInboxRequest>("popup-inbox-show", event => {
    const show = event.payload;
    if (motion.value && !terminals.has(show.requestId) && !requests.value.some(r => r.requestId === show.requestId)) {
      append(show); earlyArrivals.add(show.requestId);
    }
    void enqueue(() => add(show));
  }));
  listeners.push(await listen<{ requestId: string; winner: string; completion?: CompletionFeedback }>("popup-inbox-terminal", event => {
    const { requestId, winner, completion } = event.payload;
    if (terminals.has(requestId)) return;
    terminals.add(requestId); settleReady(requestId);
    if (motion.value?.incoming === requestId) { motion.value.incoming = null; motion.value.phase = "prepare"; }
    void enqueue(() => terminal(requestId, winner, completion));
  }));
  listeners.push(await listen("popup-inbox-close", openClose));
  listeners.push(await listen<string>("popup-inbox-focus", event => { void enqueue(() => select(event.payload, true)); }));
  listeners.push(await listen<InboxLayout>("popup-inbox-layout", event => { allocation.value = event.payload; sidebarWidth.value = event.payload.sidebarWidth; }));
  listeners.push(await listen("popup-inbox-reconcile", () => { void enqueue(() => geometry()); }));
  const snapshot = await popupInboxInit();
  recovered.value = snapshot.recovered;
  await enqueue(async () => {
    for (const request of snapshot.requests) await add(request, false);
    if (snapshot.focusedRequestId) await select(snapshot.focusedRequestId, true);
  });
});
onBeforeUnmount(() => {
  disposed = true; window.removeEventListener("keydown", keydown, true);
  clock?.cancel(); readyWaiters.forEach(waiters => waiters.forEach(resolve => resolve())); readyWaiters.clear();
  listeners.forEach(off => off()); timers.forEach(clearTimeout);
});
</script>

<template>
  <div ref="root" class="inbox-root" :class="{ 'inbox-expanded': renderedSidebar, 'inbox-native': nativeCanvas, 'inbox-frozen': frozenCanvas }" :style="rootStyle" :data-completion-phase="motion?.phase">
    <aside v-if="renderedSidebar" class="inbox-sidebar" :inert="blocked" :aria-label="t('popup.inbox.pending')">
      <header class="inbox-heading" data-tauri-drag-region>{{ t('popup.inbox.pending') }} <span>{{ requests.length }}</span></header>
      <nav class="inbox-navigation" @pointerdown.capture="rememberFocus">
        <section v-for="group in groups" :key="group.path" class="inbox-group" :class="{ 'inbox-group-collapse': collapseGroup(group.path) }" :style="motion?.group === group.path ? { '--completion-group-height': `${motion.groupHeight}px` } : undefined">
          <h2 :title="group.path">{{ projectName(group.path) }} <span>{{ group.requests.length }}</span></h2>
          <div v-for="request in group.requests" :key="request.requestId" class="inbox-entry" :class="{ 'inbox-entry-collapse': motion?.from === request.requestId && motion.phase !== 'waiting' && !motion.reduced }" :style="motion?.from === request.requestId ? { '--completion-row-height': `${motion.rowHeight}px` } : undefined">
          <button class="inbox-row" :data-inbox-row="request.requestId" :class="{ selected: request.requestId === selectedId, flash: flashed.has(request.requestId) && motion?.from !== request.requestId, unread: !visited.has(request.requestId) }" :aria-current="request.requestId === selectedId ? 'true' : undefined" :title="inboxTitle(request)" @click="choose(request.requestId)">
            <UnreadRipple :active="!visited.has(request.requestId) && !flashed.has(request.requestId)" />
            <span class="inbox-dot" :class="{ unread: !visited.has(request.requestId) }" :aria-label="!visited.has(request.requestId) ? t('popup.inbox.unread') : undefined"></span>
            <span class="inbox-row-content"><strong>{{ inboxTitle(request) || t('popup.inbox.untitled') }}</strong><span class="inbox-row-meta">{{ request.agentKind || request.source }} · {{ t(`popup.inbox.kind.${inboxKind(request)}`) }}<em v-if="drafts.has(request.requestId)">{{ t('popup.inbox.draft') }}</em></span></span>
          </button>
          </div>
        </section>
      </nav>
    </aside>
    <div v-if="renderedSidebar" class="inbox-divider" role="separator" aria-orientation="vertical" :aria-label="t('popup.inbox.resize')" @pointerdown="beginResize"></div>
    <main class="inbox-body" :inert="blocked">
      <button v-if="layoutReview" class="inbox-layout-review" @click="toggleReviewSidebar">{{ renderedSidebar ? '隐藏 Sidebar' : '显示 Sidebar' }} · 几何测试</button>
      <div v-if="recovered" class="inbox-recovery" role="alert">{{ t('popup.inbox.recovered') }}<button @click="recovered = false" :aria-label="t('popup.inbox.dismiss')">×</button></div>
      <div v-for="request in mounted" v-show="(request.requestId === active && (!terminals.has(request.requestId) || motion?.from === request.requestId)) || (motion?.phase === 'prepare' && request.requestId === motion.from)" :key="request.requestId" class="inbox-request" :class="{ 'inbox-preparing': motion?.incoming === request.requestId && motion.phase === 'prepare' }" :data-answer-phase="panePhase(request.requestId)" :data-inbox-request="request.requestId" :inert="request.requestId !== active">
        <PopupView :scope="scopeFor(request.requestId)" />
      </div>
    </main>
    <div v-if="notice" class="inbox-notice" role="status">{{ notice }}</div>
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
.inbox-native .inbox-sidebar { position: absolute; left: calc(var(--inbox-canvas-left) - var(--inbox-sidebar) - 6px); top: 0; bottom: 0; width: var(--inbox-sidebar); }
.inbox-native .inbox-divider { position: absolute; left: calc(var(--inbox-canvas-left) - 6px); top: 0; bottom: 0; width: 6px; }
.inbox-native .inbox-body { position: absolute; left: var(--inbox-canvas-left); top: 0; bottom: 0; right: 0; width: auto; }
.inbox-native.inbox-frozen .inbox-body { right: auto; width: var(--inbox-body-width); }
.inbox-native .inbox-close-backdrop { left: var(--inbox-visible-left); width: var(--inbox-visible-width); right: auto; }
.inbox-native .inbox-notice, .inbox-native .inbox-error { left: calc(var(--inbox-canvas-left) + var(--inbox-main) / 2); }
.inbox-native .inbox-recovery { width: var(--inbox-main); box-sizing: border-box; }
.inbox-root { width: 100vw; height: 100vh; display: grid; grid-template-columns: minmax(0, 1fr); overflow: hidden; color: var(--text-primary); }
.inbox-root.inbox-expanded { grid-template-columns: var(--inbox-sidebar) 6px minmax(0, 1fr); }
.inbox-sidebar { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: color-mix(in srgb, var(--bg, #f5f5f5) 72%, transparent); }
.inbox-heading { display: flex; align-items: center; justify-content: space-between; padding: 30px 16px 14px; font-size: 13px; font-weight: 600; user-select: none; }
.inbox-heading span, .inbox-group h2 span { font-variant-numeric: tabular-nums; opacity: .6; }
.inbox-navigation { flex: 1; overflow: auto; padding: 0 8px 16px; }
.inbox-group h2 { display: flex; justify-content: space-between; gap: 8px; font-size: 11px; font-weight: 600; opacity: .6; padding: 16px 12px 7px; margin: 0; overflow: hidden; }
.inbox-entry-collapse { overflow: hidden; animation: inbox-entry-complete 300ms cubic-bezier(.2, 0, .2, 1) both; }
.inbox-group-collapse { overflow: hidden; animation: inbox-group-complete 300ms cubic-bezier(.2, 0, .2, 1) both; }
/* Only the answer body moves; native preview, navbar and footer retain their positions. */
.inbox-request[data-answer-phase="out"] .content, .inbox-request[data-answer-phase="out"] .composer-dock { animation: inbox-answer-out 126ms cubic-bezier(.4, 0, 1, 1) both; }
.inbox-request[data-answer-phase="prepare"] .content, .inbox-request[data-answer-phase="prepare"] .composer-dock { opacity: 0; transform: translateY(14px); }
.inbox-request[data-answer-phase="in"] .content, .inbox-request[data-answer-phase="in"] .composer-dock { animation: inbox-answer-in 174ms cubic-bezier(0, 0, .2, 1) both; }
@keyframes inbox-answer-out { to { opacity: 0; transform: translateY(-14px); } }
@keyframes inbox-answer-in { from { opacity: 0; transform: translateY(14px); } to { opacity: 1; transform: translateY(0); } }
@keyframes inbox-entry-complete { from { height: var(--completion-row-height); opacity: 1; } to { height: 0; opacity: 0; } }
@keyframes inbox-group-complete { from { height: var(--completion-group-height); opacity: 1; } to { height: 0; opacity: 0; } }
.inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .18); position: relative; isolation: isolate; overflow: hidden; display: flex; align-items: flex-start; gap: 7px; width: 100%; padding: 10px 8px; border: 0; border-radius: 8px; text-align: left; background: transparent; color: inherit; cursor: pointer; }
.theme-dark .inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .252); }
@media (prefers-color-scheme: dark) { :root:not(.theme-light) .inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .252); } }
.inbox-row.selected { background: color-mix(in srgb, #2685e8 12%, transparent); }
.inbox-row:hover { background: color-mix(in srgb, #2685e8 8%, transparent); }
.inbox-row.flash { animation: inbox-row-arrival .4s ease-in-out 5; }
.inbox-dot { position: relative; z-index: 1; flex: 0 0 6px; height: 6px; margin-top: 5px; }
.inbox-dot.unread::before { content: ''; position: absolute; left: -2px; top: -2px; width: 10px; height: 10px; border-radius: 50%; background: #2685e8; animation: inbox-unread-dot 2.8s cubic-bezier(.4, 0, .2, 1) infinite; }
.inbox-row-content { position: relative; z-index: 1; min-width: 0; flex: 1; }
.inbox-row strong { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; font-size: 12px; line-height: 1.45; font-weight: 500; }
.inbox-row-meta { display: flex; gap: 4px; font-size: 10px; opacity: .6; margin-top: 6px; }
.inbox-row-meta em { font-style: normal; margin-left: auto; }
.inbox-divider { cursor: col-resize; background: color-mix(in srgb, currentColor 8%, transparent); touch-action: none; }
.inbox-body { min-width: 0; min-height: 0; position: relative; display: flex; flex-direction: column; }
.inbox-body[inert], .inbox-sidebar[inert] { pointer-events: none; }
.inbox-request { flex: 1; min-height: 0; height: 100%; }
.inbox-request.inbox-preparing { position: absolute; inset: 0; visibility: hidden; }
.inbox-request .popup-shell, .inbox-request .popup-status { height: 100%; width: 100%; }
.inbox-recovery { padding: 10px 14px; font-size: 12px; background: #ffe7a880; display: flex; gap: 12px; }
.inbox-recovery button { margin-left: auto; background: transparent; border: none; color: inherit; cursor: pointer; }
.inbox-notice, .inbox-error { position: absolute; z-index: 110; bottom: 30px; left: 50%; transform: translateX(-50%); padding: 10px 18px; border-radius: 12px; background: var(--surface-overlay, #fff); box-shadow: 0 4px 24px #0002; font-size: 13px; }
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
@keyframes inbox-row-arrival {
  0%, 70%, 100% { box-shadow: inset 0 0 0 100px #2685e800; }
  20%, 35% { box-shadow: inset 0 0 0 100px #2685e840; }
}
@keyframes inbox-unread-dot { 0%, 100% { background: #2685e8; } 45% { background: #006dff; } 70% { background: #2685e8; } }
@media (prefers-reduced-motion: reduce) {
  .inbox-row.flash, .inbox-dot.unread::before { animation: none; }
  .inbox-row.unread { box-shadow: inset 0 0 0 100px rgba(38, 133, 232, .08); }
}
</style>
