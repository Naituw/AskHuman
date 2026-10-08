import { computed, nextTick, onBeforeUnmount, ref, watch, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isMac } from "../../lib/platform";
import { applyFindMarks, clearFindMarks, findHighlightableRanges, setCurrentFindMark } from "../../lib/findInDom";
import { DIFF_ROW_HEIGHT, diffFindMatches, type AttachmentFindAdapter, type AttachmentFindState, type DiffFindMatch } from "./attachmentFind";
import type { PreviewContent } from "./useAttachmentContent";

let nativeGeneration = 0;
export function useAttachmentFind(deps: {
  requestId: Ref<string>; index: Ref<number | null>; content: Ref<PreviewContent | null>;
  loading: Ref<boolean>; raw: Ref<boolean>; active: Ref<boolean>; enabled: Ref<boolean>;
  body: Ref<HTMLElement | null>; nativeBody: Ref<HTMLElement | null>; failed: Ref<string | null>;
  restoreScroll: () => Promise<void>; syncNative: () => Promise<void>; setTop: (top: number) => void;
}) {
  const state = ref<AttachmentFindState>({ total: 0, current: -1, status: "ready" });
  const query = ref("");
  const caseSensitive = ref(false);
  const diffMatches = ref<DiffFindMatch[]>([]);
  let version = 0, disposed = false, pendingNavigation = false, nativeToken = 0;
  let nativeSelection = "", marked: HTMLElement | null = null;
  let rememberedCurrent = -1;
  let unlisten: (() => void) | undefined;
  let listenerReady: Promise<void> | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const live = () => !disposed && deps.active.value && deps.enabled.value;
  const domRoot = () => deps.body.value?.querySelector<HTMLElement>(".attachment-preview-text, .attachment-preview-markdown") ?? null;
  async function nativeAction(action: string, delta = 0) {
    if (!isMac || deps.index.value === null) return;
    const generation = action === "search" || action === "cancel" ? ++nativeGeneration : nativeToken;
    if (action === "search" || action === "cancel") nativeToken = generation;
    if (action === "search") {
      await ensureNativeListener();
      if (generation !== nativeToken || !live()) return;
    }
    return invoke<void>("popup_preview_find", {
      search: { requestId: deps.requestId.value, index: deps.index.value, generation,
      action, query: query.value, caseSensitive: caseSensitive.value, navigate: pendingNavigation,
      delta: action === "search" ? rememberedCurrent : delta,
      },
    });
  }
  function clearMarks() { if (marked) clearFindMarks(marked); marked = null; }
  function clear() {
    version++; clearTimeout(timer); pendingNavigation = false; clearMarks();
    if (deps.content.value?.kind === "native") void nativeAction("cancel").catch(() => {});
  }
  function scrollTo(el: HTMLElement) {
    const viewport = deps.body.value;
    if (!viewport) return;
    const rect = el.getBoundingClientRect(), bounds = viewport.getBoundingClientRect();
    if (rect.top < bounds.top + 16 || rect.bottom > bounds.bottom - 16)
      viewport.scrollTop += rect.top - bounds.top - bounds.height / 2 + rect.height / 2;
    // Nested code blocks own their horizontal scrolling.
    const scroller = el.closest("pre") ?? viewport;
    const horizontal = scroller.getBoundingClientRect();
    if (rect.left < horizontal.left + 12 || rect.right > horizontal.right - 12)
      scroller.scrollLeft += rect.left - horizontal.left - 12;
  }
  async function paint(navigate: boolean) {
    const token = version;
    await nextTick();
    if (!live() || token !== version) return;
    if (deps.content.value?.kind === "diff" && !deps.raw.value) {
      const hit = diffMatches.value[state.value.current];
      if (!navigate || !hit || !deps.body.value) return;
      const offset = deps.body.value.querySelector<HTMLElement>(".attachment-diff")?.offsetTop ?? 0;
      deps.body.value.scrollTop = Math.max(0, offset + hit.line * DIFF_ROW_HEIGHT - deps.body.value.clientHeight / 2);
      deps.setTop(deps.body.value.scrollTop);
      await nextTick();
      if (!live() || token !== version) return;
      const line = deps.body.value.querySelector(`[data-find-line="${hit.line}"]`);
      const mark = line?.querySelector<HTMLElement>(".popup-find-hit-current");
      if (mark) scrollTo(mark);
      return;
    }
    clearMarks(); const root = domRoot();
    if (!root || !query.value) return;
    marked = root;
    const marks = applyFindMarks(root, query.value, caseSensitive.value);
    setCurrentFindMark(marks, state.value.current);
    if (navigate && marks[state.value.current]) scrollTo(marks[state.value.current]!);
  }
  async function rebuild(navigate: boolean, reset: boolean, restore = false) {
    const token = ++version;
    clearTimeout(timer);
    if (!live()) return;
    if (reset) rememberedCurrent = 0;
    pendingNavigation ||= navigate;
    if (deps.loading.value || !deps.content.value) { state.value = { total: 0, current: -1, status: "loading" }; return; }
    clearMarks();
    const content = deps.content.value;
    const failure = deps.failed.value ?? (content.kind === "unavailable" ? content.reason : null);
    if (failure || content.kind === "image") {
      state.value = { total: 0, current: -1, status: failure === "readFailed" ? "readFailed" : failure === "limit" ? "limit" : "unsupported" };
      pendingNavigation = false; return;
    }
    if (restore) await deps.restoreScroll();
    await nextTick();
    if (token !== version || !live()) return;
    if (content.kind === "native") {
      state.value = { total: 0, current: -1, status: query.value ? "searching" : "loading" };
      // Coalesce typing before loading a distinct, cancellable PDF search document.
      timer = setTimeout(async () => {
        if (token !== version || !live()) return;
        try {
          await deps.syncNative();
          if (token !== version || !live()) return;
          await nativeAction("search");
        } catch { if (token === version && live()) state.value = { total: 0, current: -1, status: "error" }; }
      }, 100);
      return;
    }
    const old = diffMatches.value[state.value.current];
    let total = 0;
    if (content.kind === "diff" && !deps.raw.value) {
      diffMatches.value = diffFindMatches(content.parsed, query.value, caseSensitive.value);
      total = diffMatches.value.length;
    } else { diffMatches.value = []; total = domRoot() ? findHighlightableRanges(domRoot()!, query.value, caseSensitive.value).length : 0; }
    let current = reset ? 0 : rememberedCurrent;
    if (!reset && old && diffMatches.value.length) {
      const same = diffMatches.value.findIndex(m => m.line === old.line && m.start === old.start);
      if (same >= 0) current = same;
    }
    state.value = { total, current: total ? Math.max(0, Math.min(current, total - 1)) : -1, status: "ready" };
    rememberedCurrent = state.value.current;
    const move = pendingNavigation; pendingNavigation = false;
    await paint(move);
  }
  const adapter: AttachmentFindAdapter = {
    state,
    search(value, sensitive, navigate, reset = true) {
      query.value = value; caseSensitive.value = sensitive;
      void rebuild(navigate, reset);
    },
    async go(delta) {
      if (!live() || state.value.status !== "ready" || !state.value.total) return;
      if (deps.content.value?.kind === "native") { await nativeAction("go", delta).catch(() => { state.value.status = "error"; }); return; }
      state.value.current = (state.value.current + delta + state.value.total) % state.value.total;
      rememberedCurrent = state.value.current;
      await paint(true);
    },
    refresh() { void rebuild(false, false); },
    clear,
    selection() {
      const selection = window.getSelection();
      const root = deps.body.value;
      return root && selection?.anchorNode && root.contains(selection.anchorNode) && selection.focusNode && root.contains(selection.focusNode)
        ? selection.toString().trim() : nativeSelection;
    },
    restoreFocus() {
      if (deps.content.value?.kind === "native") void nativeAction("focus").catch(() => {});
      else deps.body.value?.focus({ preventScroll: true });
    },
  };
  watch([deps.index, deps.content, deps.raw, deps.loading, deps.failed], (values, old) => {
    if (!live()) return;
    const changedFile = values[0] !== old[0];
    nativeSelection = "";
    void rebuild(changedFile, changedFile, true);
  }, { flush: "post" });
  watch(deps.active, active => { if (!active) clear(); }, { flush: "sync" });
  function ensureNativeListener(): Promise<void> {
    if (listenerReady) return listenerReady;
    listenerReady = listen<AttachmentFindState & { requestId: string; index: number; generation: number; selection?: string }>("popup-preview-find", event => {
    const result = event.payload;
    if (!live() || result.requestId !== deps.requestId.value || result.index !== deps.index.value || result.generation !== nativeToken) return;
    state.value = { total: result.total, current: result.current, status: result.status };
    if (result.status === "ready" || result.status === "noText") rememberedCurrent = result.current;
    nativeSelection = result.selection ?? "";
    if (result.status !== "loading" && result.status !== "searching") pendingNavigation = false;
    }).then(off => { if (disposed) off(); else unlisten = off; });
    return listenerReady;
  }
  onBeforeUnmount(() => { clear(); disposed = true; unlisten?.(); });
  return { adapter, diffQuery: computed(() => deps.enabled.value ? query.value : ""), diffCaseSensitive: caseSensitive,
    diffCurrent: computed(() => deps.enabled.value ? diffMatches.value[state.value.current] ?? null : null) };
}
