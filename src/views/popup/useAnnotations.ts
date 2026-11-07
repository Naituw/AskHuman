// Popup notes on passages of the Agent's text (docs/specs/message-annotations.md): the state and the actions behind
// the annotation layer, the footer chip and its list, and the keyboard rules of the open editor and list.
//
// The list is KEYBOARD-MODAL like the todo popover: while it is open `handleAnnotationKeydown` swallows every shortcut
// the popup owns except ⌘W. The editor is modal only while the focus is inside it; the Enter, Esc and the send shortcut
// it takes can therefore never submit the answer or close the window by accident (spec D11), and a click into the
// answer box hands the keyboard back.
import { computed, nextTick, ref, watch, type Ref } from "vue";
import * as engine from "../../lib/annotations";
import { primaryModifierPressed } from "../../lib/platform";
import type { UserAnnotation } from "../../lib/types";

/** Where the notes go: the Agent's Message, or the lone question when there is no Message (spec D6). */
export type AnnotationTarget = "message" | "question";

export interface AnnotationNote extends engine.PlacedNote {
  note: string;
  /** False while the passage cannot be found in the text on screen; such a note stays in the list and is still sent. */
  anchored: boolean;
}

export interface AnnotationEditor {
  /** The note being edited, or null for a new one. */
  id: number | null;
  /** The passage of a new note; an edited note keeps its own. */
  anchor: engine.Anchor | null;
  quote: string;
  draft: string;
}

export interface PendingSelection {
  anchor: engine.Anchor;
  /** The note whose passage the selection touches; the button then edits it instead of adding another. */
  overlap: number | null;
}

export interface AnnotationDeps {
  /** What the popup lets the user annotate right now; null = nothing, no root is rendered. */
  target: Ref<AnnotationTarget | null>;
  /** The scrolling content area that holds the annotation root. */
  contentRef: Ref<HTMLElement | null>;
  /** Enter sends (any modifier + Enter breaks the line) instead of ⌘/Ctrl+Enter sending. */
  submitWithBareEnter: Ref<boolean>;
  /** Insert a newline at the caret of the focused textarea. */
  insertNewline: () => void;
  /** The popup cannot take input right now (hidden, behind another dialog, a confirm modal or popover is up). */
  suspended: () => boolean;
  /** Written here, read by the todo popover and the quick-reply menu: they stay shut while an editor or list is open. */
  modalOpen: Ref<boolean>;
}

export function useAnnotations(deps: AnnotationDeps) {
  const notes = ref<AnnotationNote[]>([]);
  const pending = ref<PendingSelection | null>(null);
  const editor = ref<AnnotationEditor | null>(null);
  const listOpen = ref(false);
  const listIndex = ref(0);
  /** The editor card and its textarea; the layer registers them. */
  const editorEl = ref<HTMLElement | null>(null);
  const editorInputEl = ref<HTMLTextAreaElement | null>(null);

  let nextId = 1;
  let restoreFocusEl: HTMLElement | null = null;

  const enabled = computed(() => deps.target.value !== null);
  const hasAnnotations = computed(() => notes.value.length > 0);
  const limitReached = computed(() => notes.value.length >= engine.MAX_ANNOTATIONS);

  watch(() => !!editor.value || listOpen.value, open => { deps.modalOpen.value = open; }, { immediate: true, flush: "sync" });

  /** The element whose text the notes refer to (`data-annotation-root` on the Message or the lone question). */
  function root(): HTMLElement | null {
    return deps.contentRef.value?.querySelector<HTMLElement>("[data-annotation-root]") ?? null;
  }

  function anchoredNotes(): AnnotationNote[] {
    return notes.value.filter(n => n.anchored);
  }

  /** Reading order: placed notes by position, then the ones whose passage is gone (spec N3). */
  function sortNotes(): void {
    const placed = anchoredNotes().sort((a, b) => a.start - b.start);
    const lost = notes.value.filter(n => !n.anchored);
    notes.value = [...placed, ...lost];
  }

  /**
   * Find every note in the text on screen and draw the marks again. Call it whenever that text may have been redrawn
   * (Markdown update, source view, a note added or removed).
   */
  function reapply(): void {
    const el = root();
    if (!el) return;
    const draft = editor.value?.anchor ?? null;
    const drawn = `mark[${engine.ID_ATTR}], mark[${engine.DRAFT_ATTR}]`;
    if (notes.value.length === 0 && !draft && !el.querySelector(drawn)) return;
    if (notes.value.length > 0) {
      for (const found of engine.relocate(el, notes.value)) {
        const hit = notes.value.find(n => n.id === found.id);
        if (!hit) continue;
        hit.start = found.start;
        hit.end = found.end;
        hit.anchored = found.located;
      }
      sortNotes();
    }
    engine.highlight(el, notes.value.flatMap((n, i) => (n.anchored ? [{ id: n.id, start: n.start, end: n.end, n: i + 1 }] : [])));
    if (draft) engine.markDraft(el, draft);
  }

  function rememberFocus(): void {
    const active = document.activeElement;
    restoreFocusEl = active instanceof HTMLElement && active !== document.body ? active : null;
  }

  /** Opening the editor from the open list keeps what the list remembered: its rows are gone by the time the editor closes. */
  function rememberOpeningFocus(): void {
    if (!listOpen.value) rememberFocus();
  }

  function restoreFocus(): void {
    const target = restoreFocusEl;
    restoreFocusEl = null;
    if (target?.isConnected) void nextTick(() => target.focus({ preventScroll: true }));
  }

  // ===== Selection → the "add annotation" button =====

  /** Look at the current selection and decide what the floating button offers (nothing, add, or edit). */
  function refreshSelection(): void {
    const el = root();
    if (!el || !enabled.value || editor.value || listOpen.value || deps.suspended()) {
      pending.value = null;
      return;
    }
    const anchor = engine.currentAnchor(el);
    pending.value = anchor ? { anchor, overlap: engine.overlapping(anchoredNotes(), anchor) } : null;
  }

  // ===== The editor =====

  /** The floating button was pressed: edit the note the selection touches, or start a new one. */
  function startFromSelection(): void {
    const hit = pending.value;
    if (!hit || editor.value || deps.suspended()) return;
    if (hit.overlap !== null) {
      startEdit(hit.overlap);
      return;
    }
    const el = root();
    if (!el || limitReached.value) return;
    rememberOpeningFocus();
    listOpen.value = false;
    editor.value = { id: null, anchor: hit.anchor, quote: hit.anchor.quote, draft: "" };
    pending.value = null;
    window.getSelection()?.removeAllRanges();
    engine.markDraft(el, hit.anchor);
    void nextTick(() => editorInputEl.value?.focus({ preventScroll: true }));
  }

  /** Open the editor on an existing note (its highlight, the list, or the button over a touched passage). */
  function startEdit(id: number): void {
    const hit = notes.value.find(n => n.id === id);
    if (!hit || editor.value || deps.suspended()) return;
    rememberOpeningFocus();
    listOpen.value = false;
    editor.value = { id, anchor: null, quote: hit.quote, draft: hit.note };
    pending.value = null;
    window.getSelection()?.removeAllRanges();
    const el = root();
    if (el && hit.anchored) engine.scrollToNote(el, id, false);
    void nextTick(() => editorInputEl.value?.focus({ preventScroll: true }));
  }

  /** Close the editor without keeping anything; the passage outline goes away with it. */
  function closeEditor(restore = true): void {
    if (!editor.value) return;
    editor.value = null;
    const el = root();
    if (el) engine.markDraft(el, null);
    if (restore) restoreFocus();
    else restoreFocusEl = null;
  }

  function cancelEditor(): void {
    closeEditor();
  }

  /** Keep the text as a note (a new one, or the new text of an edited one). False when there is nothing to keep. */
  function commitEditor(): boolean {
    const current = editor.value;
    if (!current) return false;
    const text = engine.cutChars(current.draft.trim(), engine.MAX_NOTE_CHARS);
    if (!text) return false;
    if (current.id === null) {
      if (!current.anchor || limitReached.value) return false;
      const { start, end, quote } = current.anchor;
      notes.value.push({ id: nextId++, start, end, quote, note: text, anchored: true });
    } else {
      const hit = notes.value.find(n => n.id === current.id);
      if (hit) hit.note = text;
    }
    closeEditor();
    sortNotes();
    reapply();
    return true;
  }

  /** Sending while the editor is open keeps what was typed rather than losing it; an empty editor just closes. */
  function commitOpenDraft(): void {
    const current = editor.value;
    if (!current) return;
    if (current.draft.trim() && !(current.id === null && limitReached.value)) commitEditor();
    else closeEditor(false);
  }

  // ===== Notes =====

  function removeNote(id: number): void {
    notes.value = notes.value.filter(n => n.id !== id);
    if (editor.value?.id === id) closeEditor(false);
    if (notes.value.length === 0) {
      listOpen.value = false;
      restoreFocus();
    } else {
      listIndex.value = Math.min(listIndex.value, notes.value.length - 1);
    }
    reapply();
  }

  /** Scroll a note's passage into view and pulse it; a note whose passage is gone has nowhere to go. */
  function locateNote(id: number): void {
    const el = root();
    const hit = notes.value.find(n => n.id === id);
    if (!el || !hit?.anchored) return;
    engine.scrollToNote(el, id, true);
    engine.flash(el, id);
  }

  /** A click on a highlight edits its note; true when it was one. */
  function onHighlightClick(target: Element): boolean {
    const el = root();
    if (!el || !el.contains(target) || editor.value || deps.suspended()) return false;
    const mark = target.closest(`mark[${engine.ID_ATTR}]`);
    // A link or a button inside a highlighted passage keeps its own click.
    if (!mark || target.closest("a, button")) return false;
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed) return false;
    startEdit(Number(mark.getAttribute(engine.ID_ATTR)));
    return true;
  }

  // ===== The list =====

  function openList(): void {
    if (listOpen.value || editor.value || notes.value.length === 0 || deps.suspended()) return;
    rememberFocus();
    pending.value = null;
    listIndex.value = 0;
    listOpen.value = true;
  }

  function closeList(restore = true): void {
    if (!listOpen.value) return;
    listOpen.value = false;
    if (restore) restoreFocus();
    else restoreFocusEl = null;
  }

  function toggleList(): void {
    if (listOpen.value) closeList();
    else openList();
  }

  // ===== Per request =====

  /** What goes out with the answer, in reading order. */
  function payload(): UserAnnotation[] {
    return notes.value.map(n => ({ quote: n.quote, note: n.note }));
  }

  /** Forget everything about the previous request (the core is initialised once per request). */
  function reset(): void {
    const el = root();
    if (el) engine.clearHighlights(el);
    notes.value = [];
    pending.value = null;
    editor.value = null;
    listOpen.value = false;
    listIndex.value = 0;
    restoreFocusEl = null;
    nextId = 1;
  }

  // Something took the window over (a dialog, a confirmation, another request): the overlays give way.
  watch(deps.suspended, suspended => {
    if (!suspended) return;
    pending.value = null;
    closeEditor(false);
    closeList(false);
  });

  // ===== Keyboard =====

  /**
   * Returns true when the event was consumed (the caller must stop handling it). Call it before any other popup
   * shortcut.
   */
  function handleAnnotationKeydown(event: KeyboardEvent): boolean {
    const composing = event.isComposing || event.keyCode === 229;
    const mod = primaryModifierPressed(event);
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;

    if (editor.value) {
      // Typing in the answer box while a note is still open is ordinary typing there.
      const inside = !!editorEl.value && event.target instanceof Node && editorEl.value.contains(event.target);
      if (!inside) return false;
      if (composing) return true;
      // Closing the window stays available; the popup's own ⌘W handler takes it from here.
      if (mod && key === "w") return false;
      if (key === "Escape") {
        event.preventDefault();
        cancelEditor();
        return true;
      }
      if (key === "Enter") {
        // Enter on a button keeps its native click; anything outside the input does nothing else.
        if (event.target !== editorInputEl.value) return true;
        const anyModifier = event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;
        const primaryOnly = mod && !event.shiftKey && !event.altKey;
        // Same rules as the answer box: with bare-Enter sending, any modifier + Enter breaks the line; with
        // ⌘/Ctrl+Enter sending, every other modifier + Enter does.
        const bare = deps.submitWithBareEnter.value;
        const newline = bare ? anyModifier : anyModifier && !primaryOnly;
        const send = bare ? !anyModifier : primaryOnly;
        if (newline) {
          event.preventDefault();
          deps.insertNewline();
        } else if (send) {
          event.preventDefault();
          commitEditor();
        }
        // Otherwise (bare Enter while ⌘/Ctrl+Enter adds the note): the browser inserts the newline itself.
        return true;
      }
      return true;
    }

    if (listOpen.value) {
      // Open: nothing below may reach the rest of the popup, whatever the focus is on.
      if (composing) return true;
      if (mod && key === "w") return false;
      if (key === "Escape") {
        event.preventDefault();
        closeList();
        return true;
      }
      if (mod || event.altKey) return true;
      const count = notes.value.length;
      const row = notes.value[listIndex.value];
      if (key === "ArrowDown" || key === "ArrowUp") {
        event.preventDefault();
        if (count > 0) listIndex.value = (listIndex.value + (key === "ArrowDown" ? 1 : count - 1)) % count;
      } else if (key === "Enter") {
        // Enter on a row's own button keeps its native click.
        if (event.target instanceof Element && event.target.closest("button")) return true;
        event.preventDefault();
        if (row) locateNote(row.id);
      } else if (key === "e" && !event.shiftKey) {
        event.preventDefault();
        if (row) startEdit(row.id);
      } else if (key === "Delete" || key === "Backspace") {
        event.preventDefault();
        if (row) removeNote(row.id);
      }
      return true;
    }

    return false;
  }

  return {
    annotationTarget: deps.target,
    annotationNotes: notes,
    annotationPending: pending,
    annotationEditor: editor,
    annotationListOpen: listOpen,
    annotationListIndex: listIndex,
    annotationLimitReached: limitReached,
    annotationEditorEl: editorEl,
    annotationEditorInputEl: editorInputEl,
    hasAnnotations,
    annotationRoot: root,
    refreshAnnotationSelection: refreshSelection,
    startAnnotation: startFromSelection,
    editAnnotation: startEdit,
    commitAnnotation: commitEditor,
    cancelAnnotation: cancelEditor,
    removeAnnotation: removeNote,
    locateAnnotation: locateNote,
    openAnnotationList: openList,
    closeAnnotationList: closeList,
    toggleAnnotationList: toggleList,
    reapplyAnnotations: reapply,
    onAnnotationHighlightClick: onHighlightClick,
    annotationPayload: payload,
    commitAnnotationDraft: commitOpenDraft,
    resetAnnotations: reset,
    handleAnnotationKeydown,
  };
}
