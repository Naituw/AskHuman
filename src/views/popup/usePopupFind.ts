// Popup in-page find (spec docs/specs/popup-find.md): ⌘/Ctrl+F bar, highlight, next/prev.
import {
  computed,
  nextTick,
  onBeforeUnmount,
  ref,
  shallowRef,
  watch,
  type Ref,
} from "vue";
import {
  applyFindMarks,
  clearFindMarks,
  findAllRanges,
  findHighlightableRanges,
  setCurrentFindMark,
  type TextRange,
} from "../../lib/findInDom";
import { isMac } from "../../lib/platform";
import type {
  ConfirmRequest,
  FileAttachment,
  OptionItem,
  Question,
} from "../../lib/types";
import { optionDisplayText } from "./optionDisplay";
import type { AttachmentFindAdapter, FindScope } from "./attachmentFind";

export interface FindSegment {
  /** Stable id for DOM roots: data-find-seg */
  id: string;
  text: string;
  /** Sequential/vertical question index to reveal, if any. */
  qIndex: number | null;
}

export interface FindMatch {
  segmentId: string;
  /** 0-based index among matches in this segment. */
  occurrence: number;
  start: number;
  end: number;
  qIndex: number | null;
}

const MAX_PREFILL = 200;

export function usePopupFind(deps: {
  contentRef: Ref<HTMLElement | null>;
  isConfirm: Ref<boolean>;
  confirmRequest: Ref<ConfirmRequest | null>;
  messageText: Ref<string>;
  viewSource: Ref<boolean>;
  attachments: Ref<FileAttachment[]>;
  questions: Ref<Question[]>;
  whatsNext: Ref<boolean>;
  todoPrefix: Ref<string>;
  /** Confirm choice rows as currently displayed (label + description). */
  confirmChoiceTexts: Ref<string[]>;
  confirmTitle: Ref<string>;
  confirmSummary: Ref<string>;
  confirmToolName: Ref<string>;
  confirmBodyText: Ref<string>;
  currentQ: Ref<number>;
  verticalMode: Ref<boolean>;
  /** Reveal a question without focusing the answer composer (find navigation). */
  revealQuestion: (index: number) => void | Promise<void>;
  previewOpen?: Readonly<Ref<boolean>>;
  previewIndex?: Ref<number | null>;
  active?: Readonly<Ref<boolean>>;
}) {
  const findActive = ref(false);
  const findQuery = ref("");
  const findCaseSensitive = ref(false);
  /** 0-based into matches; -1 when none. */
  const findCurrent = ref(-1);
  const findInputEl = ref<HTMLInputElement | null>(null);
  const matches = ref<FindMatch[]>([]);
  const findScope = ref<FindScope>("question");
  const lastRegion = ref<FindScope>("question");
  const attachment = shallowRef<AttachmentFindAdapter | null>(null);
  let restoreAttachmentFocus = false;
  let markedRoot: HTMLElement | null = null;
  let restoreFocusEl: HTMLElement | null = null;
  let applyToken = 0;
  const isActive = () => deps.active?.value !== false;

  const findTotal = computed(() => findScope.value === "attachment" ? attachment.value?.state.value.total ?? 0 : matches.value.length);
  const current = computed(() => findScope.value === "attachment" ? attachment.value?.state.value.current ?? -1 : findCurrent.value);
  const findStatus = computed(() => findScope.value === "attachment" ? attachment.value?.state.value.status ?? "loading" : "ready");
  const findCountLabel = computed(() => {
    const n = findTotal.value;
    if (!findQuery.value || findStatus.value !== "ready") return "";
    if (n === 0) return "0/0";
    return `${current.value + 1}/${n}`;
  });
  const findNoMatch = computed(
    () =>
      findActive.value && findStatus.value === "ready" && findQuery.value.length > 0 && findTotal.value === 0,
  );

  function buildSegments(): FindSegment[] {
    const segs: FindSegment[] = [];
    if (deps.isConfirm.value) {
      const cr = deps.confirmRequest.value;
      if (!cr) return segs;
      if (deps.confirmTitle.value.trim()) {
        segs.push({
          id: "confirm-title",
          text: deps.confirmTitle.value,
          qIndex: null,
        });
      }
      if (deps.confirmSummary.value.trim()) {
        segs.push({
          id: "confirm-summary",
          text: deps.confirmSummary.value,
          qIndex: null,
        });
      }
      if (deps.confirmToolName.value.trim()) {
        segs.push({
          id: "confirm-tool",
          text: deps.confirmToolName.value,
          qIndex: null,
        });
      }
      if (deps.confirmBodyText.value.trim()) {
        segs.push({
          id: "confirm-body",
          text: deps.confirmBodyText.value,
          qIndex: null,
        });
      }
      deps.confirmChoiceTexts.value.forEach((text, i) => {
        if (text.trim()) {
          segs.push({ id: `confirm-choice-${i}`, text, qIndex: null });
        }
      });
      return segs;
    }

    const msg = deps.messageText.value;
    if (msg) segs.push({ id: "message", text: msg, qIndex: null });

    deps.attachments.value.forEach((a, i) => {
      if (a.name) segs.push({ id: `att-${i}`, text: a.name, qIndex: null });
    });

    const wn = deps.whatsNext.value;
    const prefix = deps.todoPrefix.value;
    deps.questions.value.forEach((q, qi) => {
      if (q.message) {
        segs.push({ id: `q-${qi}-msg`, text: q.message, qIndex: qi });
      }
      q.predefinedOptions.forEach((opt: OptionItem, oi: number) => {
        const text = optionDisplayText(opt, wn, prefix);
        if (text) {
          segs.push({ id: `q-${qi}-opt-${oi}`, text, qIndex: qi });
        }
      });
    });
    return segs;
  }

  function segmentRanges(seg: FindSegment, query: string): TextRange[] {
    const root = deps.contentRef.value;
    const element = root?.querySelector(
      `[data-find-seg="${CSS.escape(seg.id)}"]`,
    ) as HTMLElement | null;
    return element
      ? findHighlightableRanges(element, query, findCaseSensitive.value)
      : findAllRanges(seg.text, query, findCaseSensitive.value);
  }

  function rebuildMatches(): void {
    const q = findQuery.value;
    const segs = buildSegments();
    const next: FindMatch[] = [];
    if (q) {
      for (const seg of segs) {
        const ranges = segmentRanges(seg, q);
        ranges.forEach((r, occurrence) => {
          next.push({
            segmentId: seg.id,
            occurrence,
            start: r.start,
            end: r.end,
            qIndex: seg.qIndex,
          });
        });
      }
    }
    matches.value = next;
    if (next.length === 0) {
      findCurrent.value = -1;
    } else if (findCurrent.value < 0 || findCurrent.value >= next.length) {
      findCurrent.value = 0;
    }
  }

  /**
   * Rebuild after the DOM (not the query) changed, keeping the same logical match current.
   * Segment texts are re-read from whatever is mounted, so indices may shift (e.g. a
   * sequential question mounting swaps raw Markdown for rendered text); the current match is
   * therefore re-identified by segment + occurrence rather than by position.
   */
  function rebuildMatchesKeepingCurrent(): void {
    const anchor = matches.value[findCurrent.value] ?? null;
    rebuildMatches();
    if (!anchor) return;
    const idx = matches.value.findIndex(
      (m) =>
        m.segmentId === anchor.segmentId && m.occurrence === anchor.occurrence,
    );
    if (idx >= 0) findCurrent.value = idx;
    else if (matches.value.length > 0) findCurrent.value = 0;
    else findCurrent.value = -1;
  }

  async function ensureQuestionVisible(qIndex: number | null): Promise<void> {
    if (qIndex === null) return;
    if (deps.currentQ.value === qIndex && !deps.verticalMode.value) {
      // Sequential already on target — still may need scroll only.
      return;
    }
    if (deps.currentQ.value === qIndex && deps.verticalMode.value) {
      // Vertical: still ask reveal to scroll the card into view.
      await deps.revealQuestion(qIndex);
      return;
    }
    await deps.revealQuestion(qIndex);
  }

  function scrollMarkIntoView(el: HTMLElement): void {
    const content = deps.contentRef.value;
    if (!content) {
      el.scrollIntoView({ block: "center", inline: "nearest" });
      return;
    }
    const cRect = content.getBoundingClientRect();
    const mRect = el.getBoundingClientRect();
    const margin = 48;
    if (
      mRect.top >= cRect.top + margin &&
      mRect.bottom <= cRect.bottom - margin
    ) {
      return;
    }
    const delta =
      mRect.top - cRect.top - (cRect.height / 2 - mRect.height / 2);
    content.scrollTop += delta;
  }

  /** Re-wrap marks on the mounted DOM and style the current one; returns that mark, if mounted. */
  function paintMarks(root: HTMLElement): HTMLElement | null {
    const query = findQuery.value;
    const caseSensitive = findCaseSensitive.value;
    clearFindMarks(root);

    const segs = buildSegments();
    const allMarks: HTMLElement[] = [];
    for (const seg of segs) {
      const el = root.querySelector(
        `[data-find-seg="${CSS.escape(seg.id)}"]`,
      ) as HTMLElement | null;
      if (!el) continue;
      allMarks.push(...applyFindMarks(el, query, caseSensitive));
    }

    const mi = findCurrent.value;
    if (mi < 0 || mi >= matches.value.length) {
      setCurrentFindMark(allMarks, -1);
      return null;
    }
    const m = matches.value[mi]!;
    const segEl = root.querySelector(
      `[data-find-seg="${CSS.escape(m.segmentId)}"]`,
    ) as HTMLElement | null;
    let currentEl: HTMLElement | null = null;
    if (segEl) {
      const segMarks = Array.from(
        segEl.querySelectorAll<HTMLElement>("[data-popup-find]"),
      );
      currentEl = segMarks[m.occurrence] ?? null;
    }
    setCurrentFindMark(allMarks, currentEl ? allMarks.indexOf(currentEl) : -1);
    return currentEl;
  }

  /**
   * Bring the current match on screen (user navigation: open / typing / next-prev / Aa):
   * reveal its question when hidden, repaint marks, then scroll the mark into view. Only this
   * path moves the viewport; passive repaints never do, otherwise every scroll-spy tick in
   * vertical mode (which rewrites `currentQ`) would drag the user back to the match.
   */
  async function navigateToCurrent(): Promise<void> {
    const token = ++applyToken;
    const root = deps.contentRef.value;
    if (markedRoot && markedRoot !== root) {
      clearFindMarks(markedRoot);
    }
    markedRoot = root;

    if (!findActive.value || findScope.value !== "question" || deps.active?.value === false || !root || !findQuery.value) {
      if (root) clearFindMarks(root);
      return;
    }

    const anchor = matches.value[findCurrent.value] ?? null;
    if (anchor) {
      await ensureQuestionVisible(anchor.qIndex);
      if (token !== applyToken || findScope.value !== "question" || !isActive()) return;
      // After a sequential switch, re-read DOM-backed segment texts.
      rebuildMatchesKeepingCurrent();
    }

    await nextTick();
    if (token !== applyToken || findScope.value !== "question" || !isActive()) return;
    const currentEl = paintMarks(root);
    if (currentEl) scrollMarkIntoView(currentEl);
  }

  /**
   * Repaint after the mounted DOM changed underneath an open session (Markdown re-render,
   * sequential question mounted, source toggle, scroll-spy in sequential mode). Keeps the same
   * logical match current but does not reveal or scroll: the user may have deliberately moved
   * elsewhere, and a match on an unmounted question simply has no styled mark until the next
   * Enter / ⌘G navigates back to it. Never cancels an in-flight navigation.
   */
  function repaintHighlights(): void {
    const root = deps.contentRef.value;
    if (markedRoot && markedRoot !== root) {
      clearFindMarks(markedRoot);
    }
    markedRoot = root;
    if (!findActive.value || findScope.value !== "question" || deps.active?.value === false || !root || !findQuery.value) {
      if (root) clearFindMarks(root);
      return;
    }
    rebuildMatchesKeepingCurrent();
    paintMarks(root);
  }

  function openFind(prefillFromSelection = true, target: FindScope = lastRegion.value, nativeSelection?: string): void {
    if (target === "attachment" && !deps.previewOpen?.value) target = "question";
    const wasActive = findActive.value;
    const changed = findScope.value !== target;
    const active = document.activeElement;
    if (!wasActive && active instanceof HTMLElement && !active.closest(".popup-find-bar")) {
      restoreFocusEl = active;
      restoreAttachmentFocus = target === "attachment";
    }

    let prefill = "";
    if (prefillFromSelection && !wasActive) {
      const sel = window.getSelection();
      const region = sel?.anchorNode?.parentElement?.closest(target === "attachment" ? ".attachment-preview-body" : ".content");
      const validSelection = region && deps.contentRef.value?.contains(region) && sel?.focusNode
        && deps.contentRef.value.contains(sel.focusNode) && !sel.anchorNode?.parentElement?.closest("textarea, input");
      const t = target === "attachment" ? nativeSelection ?? attachment.value?.selection() ?? ""
        : validSelection ? sel?.toString().trim() ?? "" : "";
      if (t) prefill = t.slice(0, MAX_PREFILL);
    }

    // Already open in this scope: focus the existing search input.
    if (wasActive && !changed) {
      findInputEl.value?.focus({ preventScroll: true });
      findInputEl.value?.select();
      return;
    }

    findActive.value = true;
    findScope.value = target;
    applyToken++;
    if (markedRoot) clearFindMarks(markedRoot);
    attachment.value?.clear();
    if (target === "attachment") {
      if (prefill) findQuery.value = prefill;
      attachment.value?.search(findQuery.value, findCaseSensitive.value, true);
      void nextTick(() => { findInputEl.value?.focus({ preventScroll: true }); findInputEl.value?.select(); });
      return;
    }
    if (prefill) {
      findQuery.value = prefill;
      rebuildMatches();
      findCurrent.value = matches.value.length > 0 ? 0 : -1;
    } else if (findQuery.value) {
      rebuildMatches(); findCurrent.value = matches.value.length ? 0 : -1;
    } else {
      matches.value = [];
      findCurrent.value = -1;
    }

    // FindBar focuses its input on mount; navigation waits for the searchable DOM.
    void nextTick(async () => {
      await navigateToCurrent();
    });
  }

  function closeFind(): void {
    findActive.value = false;
    findQuery.value = "";
    findCurrent.value = -1;
    matches.value = [];
    findCaseSensitive.value = false;
    attachment.value?.clear();
    applyToken++;
    if (markedRoot) {
      clearFindMarks(markedRoot);
      markedRoot = null;
    }
    const restore = restoreFocusEl;
    const nativeRestore = restoreAttachmentFocus;
    restoreAttachmentFocus = false;
    restoreFocusEl = null;
    void nextTick(() => {
      if (deps.active?.value === false) return;
      if (nativeRestore && deps.previewOpen?.value) attachment.value?.restoreFocus();
      else if (restore && document.contains(restore)) {
        try {
          restore.focus({ preventScroll: true });
        } catch {
          /* ignore */
        }
      }
    });
  }

  async function goFind(delta: number): Promise<void> {
    if (!findActive.value) return;
    if (findScope.value === "attachment") { await attachment.value?.go(delta); return; }
    const n = matches.value.length;
    if (n === 0) return;
    const cur = findCurrent.value < 0 ? 0 : findCurrent.value;
    findCurrent.value = (cur + delta + n * 10) % n;
    await navigateToCurrent();
  }

  function onFindQueryInput(value: string): void {
    findQuery.value = value;
    if (findScope.value === "attachment") { attachment.value?.search(value, findCaseSensitive.value, true); return; }
    rebuildMatches();
    findCurrent.value = matches.value.length > 0 ? 0 : -1;
    void navigateToCurrent();
  }

  function toggleFindCase(): void {
    findCaseSensitive.value = !findCaseSensitive.value;
    if (findScope.value === "attachment") { attachment.value?.search(findQuery.value, findCaseSensitive.value, true, false); return; }
    const prev = matches.value[findCurrent.value];
    rebuildMatches();
    if (prev) {
      const idx = matches.value.findIndex(
        (m) =>
          m.segmentId === prev.segmentId &&
          m.start === prev.start &&
          m.end === prev.end,
      );
      findCurrent.value =
        idx >= 0 ? idx : matches.value.length > 0 ? 0 : -1;
    } else {
      findCurrent.value = matches.value.length > 0 ? 0 : -1;
    }
    void navigateToCurrent();
  }

  /** Returns true if the event was handled. */
  function handleFindKeydown(e: KeyboardEvent): boolean {
    if (e.isComposing || e.keyCode === 229) return false;
    const mod = isMac ? e.metaKey : e.ctrlKey;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;

    // ⌘/Ctrl+F — open or refocus (no alt/shift).
    if (mod && !e.altKey && !e.shiftKey && key === "f") {
      e.preventDefault();
      const inBar = e.target instanceof Element && !!e.target.closest(".popup-find-bar");
      openFind(true, inBar && findActive.value ? findScope.value : lastRegion.value);
      return true;
    }

    if (!findActive.value) return false;

    // ⌘/Ctrl+G next, ⌘/Ctrl+Shift+G prev
    if (mod && !e.altKey && (key === "g" || e.key === "g" || e.key === "G")) {
      e.preventDefault();
      void goFind(e.shiftKey ? -1 : 1);
      return true;
    }

    if (e.key === "Escape") {
      e.preventDefault();
      closeFind();
      return true;
    }

    const inFindInput =
      e.target instanceof HTMLElement &&
      e.target.closest(".popup-find-bar") !== null;

    if (inFindInput && e.key === "Enter" && !mod && !e.altKey) {
      e.preventDefault();
      void goFind(e.shiftKey ? -1 : 1);
      return true;
    }

    return false;
  }

  // Repaint when the searchable DOM changes underneath an open session. `currentQ` only matters
  // in sequential mode, where it decides which question is mounted; in vertical mode every card
  // is mounted and scroll-spy rewrites `currentQ` on each scroll, so it must not retrigger here.
  watch(
    () =>
      [
        deps.messageText.value,
        deps.viewSource.value,
        deps.verticalMode.value ? -1 : deps.currentQ.value,
        deps.verticalMode.value,
        deps.isConfirm.value,
        deps.confirmBodyText.value,
        deps.confirmChoiceTexts.value.join("\0"),
        deps.questions.value.length,
      ] as const,
    () => {
      if (!findActive.value || findScope.value !== "question" || !findQuery.value) return;
      repaintHighlights();
    },
  );

  function noteFindRegion(region: FindScope): void { lastRegion.value = region; }
  let findTabFocus = false;
  function noteFindInteraction(event: Event): void {
    if (!isActive()) return;
    const element = event.target instanceof Element ? event.target : null;
    if (!element || element.closest(".popup-find-bar")) return;
    if (event instanceof KeyboardEvent) {
      // Pressing a shortcut modifier must not let a stale DOM focus reclaim the region.
      if (["Meta", "Control", "Alt", "Shift"].includes(event.key)) return;
      findTabFocus = event.key === "Tab";
      const mod = isMac ? event.metaKey : event.ctrlKey;
      if (mod && ["f", "g"].includes(event.key.toLowerCase())) return;
    } else if (event.type === "focusin") {
      if (!findTabFocus) return;
      findTabFocus = false;
    }
    if (element.closest(".attachment-preview")) noteFindRegion("attachment");
    else if (element.closest(".popup-main") && !element.closest(".navbar")) noteFindRegion("question");
  }
  function registerAttachmentFind(adapter: AttachmentFindAdapter | null): void {
    attachment.value?.clear(); attachment.value = adapter;
    if (adapter && findActive.value && findScope.value === "attachment") adapter.search(findQuery.value, findCaseSensitive.value, false);
  }
  if (deps.previewIndex) watch(deps.previewIndex, (index, old) => {
    if (index !== null && index !== old && deps.active?.value !== false) lastRegion.value = "attachment";
  }, { flush: "sync" });
  if (deps.previewOpen) watch(deps.previewOpen, open => {
    if (!open) {
      if (findActive.value && findScope.value === "attachment") closeFind();
      lastRegion.value = "question";
    }
  }, { flush: "sync" });
  if (deps.active) watch(deps.active, active => {
    applyToken++;
    if (!active) { if (markedRoot) clearFindMarks(markedRoot); attachment.value?.clear(); }
    else if (findActive.value) {
      if (findScope.value === "attachment") attachment.value?.search(findQuery.value, findCaseSensitive.value, false, false);
      else void nextTick(repaintHighlights);
    }
  }, { flush: "sync" });

  onBeforeUnmount(() => {
    if (markedRoot) clearFindMarks(markedRoot);
  });

  return {
    findActive,
    findScope,
    noteFindInteraction,
    findStatus,
    noteFindRegion,
    registerAttachmentFind,
    findQuery,
    findCaseSensitive,
    findCurrent: current,
    findTotal,
    findCountLabel,
    findNoMatch,
    findInputEl,
    openFind,
    closeFind,
    goFind,
    onFindQueryInput,
    toggleFindCase,
    handleFindKeydown,
    /** Repaint after the DOM settled (sequential transition, Markdown update); never scrolls. */
    refreshFind: () => {
      if (!findActive.value) return;
      if (findScope.value === "attachment") attachment.value?.refresh();
      else repaintHighlights();
    },
  };
}
