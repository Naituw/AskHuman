<script setup lang="ts">
// The floating part of the message annotations (docs/specs/message-annotations.md D10–D12): the "add annotation" button
// beside a text selection and the editor card. Both are placed from the live DOM (the selection's rectangle, the marks of
// the passage) and follow it while the content scrolls. The state and the keyboard rules live in useAnnotations.
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { MAX_ANNOTATIONS, MAX_NOTE_CHARS, draftMarks, marksOf } from "../../lib/annotations";
import { usePopupContext } from "./context";

const { t } = useI18n();
const {
  contentRef,
  annotationPending,
  annotationEditor,
  annotationListOpen,
  annotationLimitReached,
  annotationEditorEl,
  annotationEditorInputEl,
  annotationRoot,
  submitKeyLabel,
  refreshAnnotationSelection,
  startAnnotation,
  commitAnnotation,
  cancelAnnotation,
  removeAnnotation,
  onAnnotationHighlightClick,
} = usePopupContext();

const FAB_WIDTH = 112;
const FAB_HEIGHT = 26;
const EDITOR_WIDTH = 320;
const EDITOR_FALLBACK_HEIGHT = 190;
const MARGIN = 12;
const GAP = 8;

const layerEl = ref<HTMLElement | null>(null);
const cardEl = ref<HTMLElement | null>(null);
const inputEl = ref<HTMLTextAreaElement | null>(null);
const fab = ref<{ x: number; y: number } | null>(null);
const editorPos = ref<{ x: number; y: number } | null>(null);

// The composable focuses the textarea and judges where the keys come from through these two.
watch(cardEl, (el) => { annotationEditorEl.value = el; }, { flush: "post" });
watch(inputEl, (el) => { annotationEditorInputEl.value = el; }, { flush: "post" });

const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(value, Math.max(low, high)));

/** The scroll area's box when it has one (a zero box means the layout is not measurable, as in a test DOM). */
function contentBox(): DOMRect | null {
  const box = contentRef.value?.getBoundingClientRect();
  return box && box.height > 0 ? box : null;
}

// ===== The button beside a selection =====

function placeFab() {
  const layer = layerEl.value?.getBoundingClientRect();
  const selection = window.getSelection();
  if (!annotationPending.value || !layer || !selection || selection.rangeCount === 0) {
    fab.value = null;
    return;
  }
  const rect = selection.getRangeAt(0).getBoundingClientRect?.();
  if (!rect || (rect.width === 0 && rect.height === 0)) {
    fab.value = null;
    return;
  }
  const content = contentBox();
  // The selection scrolled out of the visible text: nothing to point at.
  if (content && (rect.bottom < content.top || rect.top > content.bottom)) {
    fab.value = null;
    return;
  }
  const x = clamp(rect.left + rect.width / 2 - FAB_WIDTH / 2, layer.left + MARGIN, layer.right - FAB_WIDTH - MARGIN);
  const above = rect.top - FAB_HEIGHT - GAP;
  let y = above;
  if (content && above < content.top + 2) y = rect.bottom + GAP;
  if (content) y = clamp(y, content.top + 2, content.bottom - FAB_HEIGHT - 2);
  fab.value = { x: x - layer.left, y: y - layer.top };
}

let pointerDown = false;
// From the mouse-up until the evaluation it schedules: the selectionchange that trails the release belongs to the same
// gesture and must not push the button back by the keyboard's debounce.
let settling = false;
let selectionTimer: number | undefined;

function evaluateSelection() {
  selectionTimer = undefined;
  settling = false;
  refreshAnnotationSelection();
  placeFab();
}

function scheduleSelection(delay: number) {
  window.clearTimeout(selectionTimer);
  selectionTimer = window.setTimeout(evaluateSelection, delay);
}

function onSelectionChange() {
  if (pointerDown || settling) return;
  const selection = window.getSelection();
  // A collapsed selection takes the button away at once; a growing one (keyboard, double click) settles first.
  if (!selection || selection.isCollapsed) {
    window.clearTimeout(selectionTimer);
    evaluateSelection();
  } else {
    scheduleSelection(180);
  }
}

function onPointerDown(event: PointerEvent) {
  const target = event.target instanceof Element ? event.target : null;
  if (target?.closest(".an-fab")) return;
  pointerDown = true;
  settling = false;
  window.clearTimeout(selectionTimer);
  fab.value = null;
  annotationPending.value = null;
  // A press anywhere outside an editor that is still empty closes it; a typed note stays until it is added or cancelled.
  const editor = annotationEditor.value;
  if (editor && !(target && cardEl.value?.contains(target)) && !editor.draft.trim()) cancelAnnotation();
}

function onPointerUp() {
  if (!pointerDown) return;
  pointerDown = false;
  // The selection is final once the press is over.
  settling = true;
  scheduleSelection(0);
}

function onDocumentClick(event: MouseEvent) {
  if (event.target instanceof Element) onAnnotationHighlightClick(event.target);
}

// ===== The editor card =====

function passageRects(): DOMRect[] {
  const root = annotationRoot();
  const editor = annotationEditor.value;
  if (!root || !editor) return [];
  const marks = editor.id === null ? draftMarks(root) : marksOf(root, editor.id);
  return marks.flatMap((mark) => Array.from(mark.getClientRects()));
}

function placeEditor() {
  const card = cardEl.value;
  const layer = layerEl.value?.getBoundingClientRect();
  if (!card || !layer) {
    editorPos.value = null;
    return;
  }
  const content = contentBox();
  const height = card.offsetHeight || EDITOR_FALLBACK_HEIGHT;
  const width = card.offsetWidth || EDITOR_WIDTH;
  const rects = passageRects();
  const first = rects[0];
  const last = rects[rects.length - 1];
  let left: number;
  let top: number;
  if (first && last) {
    // Below the passage, or above it when the text runs too close to the bottom of the visible area.
    left = first.left;
    top = last.bottom + GAP;
    if (content && top + height > content.bottom - 4) top = first.top - GAP - height;
  } else if (content) {
    // The passage is not on screen (a note whose text is gone, or not drawn yet): park the card at the bottom right.
    left = content.right - width - 16;
    top = content.bottom - height - 8;
  } else {
    left = layer.left + MARGIN;
    top = layer.top + MARGIN;
  }
  if (content) top = clamp(top, content.top + 4, content.bottom - height - 4);
  left = clamp(left, layer.left + MARGIN, layer.right - width - MARGIN);
  editorPos.value = { x: left - layer.left, y: top - layer.top };
}

// Until it is placed the card is only transparent, not `visibility: hidden`: the input takes the focus as the card opens,
// before the first placement, and a hidden element cannot be focused.
watch(annotationEditor, () => {
  editorPos.value = null;
  void nextTick(placeEditor);
});

/** A press on the card's own text keeps the focus in the textarea instead of dropping it. */
function onCardMouseDown(event: MouseEvent) {
  const target = event.target instanceof Element ? event.target : null;
  if (target?.closest("textarea, button")) return;
  event.preventDefault();
  inputEl.value?.focus({ preventScroll: true });
}

// ===== Following the content =====

let frame = 0;
function reposition() {
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    placeFab();
    if (annotationEditor.value) placeEditor();
  });
}

let resizeObserver: ResizeObserver | undefined;
let observed: HTMLElement | null = null;
watch(contentRef, (el) => {
  if (observed) observed.removeEventListener("markdown-content-updated", reposition);
  observed = el;
  if (el) el.addEventListener("markdown-content-updated", reposition);
  resizeObserver?.disconnect();
  if (el) resizeObserver?.observe(el);
}, { immediate: true });

onMounted(() => {
  if (typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(reposition);
    if (observed) resizeObserver.observe(observed);
  }
  document.addEventListener("selectionchange", onSelectionChange);
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("pointerup", onPointerUp, true);
  document.addEventListener("pointercancel", onPointerUp, true);
  document.addEventListener("click", onDocumentClick);
  window.addEventListener("scroll", reposition, true);
  window.addEventListener("resize", reposition);
  window.addEventListener("blur", onPointerUp);
});

onBeforeUnmount(() => {
  window.clearTimeout(selectionTimer);
  if (frame) cancelAnimationFrame(frame);
  resizeObserver?.disconnect();
  observed?.removeEventListener("markdown-content-updated", reposition);
  document.removeEventListener("selectionchange", onSelectionChange);
  document.removeEventListener("pointerdown", onPointerDown, true);
  document.removeEventListener("pointerup", onPointerUp, true);
  document.removeEventListener("pointercancel", onPointerUp, true);
  document.removeEventListener("click", onDocumentClick);
  window.removeEventListener("scroll", reposition, true);
  window.removeEventListener("resize", reposition);
  window.removeEventListener("blur", onPointerUp);
});

/** The quote can run to 2000 characters; the card only has room for two lines of it. */
function excerpt(text: string, max = 90): string {
  const chars = Array.from(text);
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : text;
}
</script>

<template>
  <div ref="layerEl" class="an-layer">
    <!-- The press must not collapse the selection first, so the button keeps the mouse down to itself. -->
    <button
      v-if="fab && annotationPending && !annotationEditor && !annotationListOpen"
      class="an-fab"
      type="button"
      :disabled="annotationLimitReached && annotationPending.overlap === null"
      :style="{ left: `${fab.x}px`, top: `${fab.y}px` }"
      @mousedown.prevent
      @click.stop="startAnnotation()"
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M21 11.5a8.4 8.4 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.4 8.4 0 0 1-3.8-.9L3 21l1.9-5.7a8.4 8.4 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.4 8.4 0 0 1 3.8-.9h.5a8.5 8.5 0 0 1 8 8z" />
        <path d="M12 8.5v6M9 11.5h6" />
      </svg>
      {{
        annotationPending.overlap !== null
          ? t("popup.annotations.edit")
          : annotationLimitReached
            ? t("popup.annotations.full", { max: MAX_ANNOTATIONS })
            : t("popup.annotations.add")
      }}
    </button>

    <div
      v-if="annotationEditor"
      ref="cardEl"
      class="an-editor"
      role="dialog"
      :aria-label="t('popup.annotations.dialog')"
      :style="editorPos ? { left: `${editorPos.x}px`, top: `${editorPos.y}px` } : { opacity: 0, pointerEvents: 'none' }"
      @mousedown="onCardMouseDown"
      @click.stop
      @paste.stop
    >
      <div class="an-quote">{{ excerpt(annotationEditor.quote) }}</div>
      <textarea
        ref="inputEl"
        v-model="annotationEditor.draft"
        class="an-textarea"
        rows="3"
        :maxlength="MAX_NOTE_CHARS"
        :placeholder="t('popup.annotations.placeholder')"
        :aria-label="t('popup.annotations.placeholder')"
      ></textarea>
      <div class="an-editor-foot">
        <button v-if="annotationEditor.id !== null" class="an-delete" type="button" @click="removeAnnotation(annotationEditor.id)">
          {{ t("popup.annotations.delete") }}
        </button>
        <span v-else class="an-hint"><kbd class="an-kbd">{{ submitKeyLabel }}</kbd> {{ t("popup.annotations.hintAdd") }}</span>
        <span class="an-spacer"></span>
        <button class="btn" type="button" @click="cancelAnnotation()">{{ t("common.cancel") }}</button>
        <button class="btn btn-primary" type="button" :disabled="!annotationEditor.draft.trim()" @click="commitAnnotation()">
          {{ annotationEditor.id === null ? t("popup.annotations.addNote") : t("popup.annotations.save") }}
        </button>
      </div>
    </div>
  </div>
</template>
