<script setup lang="ts">
// The footer's "Annotations N" button and its list (docs/specs/message-annotations.md D13): it sits beside Send (after the
// quick-reply menu) and exists only while there are notes. A click on a row finds the passage and pulses it; hovering shows
// edit and delete. The keyboard rules (↑↓, Enter, E, Delete, Esc) live in useAnnotations.
import { computed, nextTick, ref, unref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const { t } = useI18n();
const {
  annotationNotes,
  annotationListOpen,
  annotationListIndex,
  toggleAnnotationList,
  closeAnnotationList,
  locateAnnotation,
  editAnnotation,
  removeAnnotation,
  submitting,
} = usePopupContext();

const menuEl = ref<HTMLElement | null>(null);
// `unref` tolerates a context without notes (the footer is also mounted on its own in tests).
const count = computed(() => unref(annotationNotes)?.length ?? 0);

// The list hangs from the button's right edge. With several questions Previous / Next / Send sit on that button's right,
// so the button is left of centre and a full-width list would run past the window's left edge: slide it back inside.
const WINDOW_MARGIN = 12;
const shiftX = ref(0);
async function keepMenuInside() {
  shiftX.value = 0;
  await nextTick();
  const box = menuEl.value?.getBoundingClientRect();
  if (!box || box.width === 0) return;
  shiftX.value = Math.max(0, WINDOW_MARGIN - box.left);
}
watch(annotationListOpen, (open) => {
  if (open) void keepMenuInside();
  else shiftX.value = 0;
});

// Roving focus: the highlighted row owns the focus, so Enter and the screen reader follow the arrows.
watch([annotationListOpen, annotationListIndex], async ([open, index]) => {
  if (!open) return;
  await nextTick();
  const row = menuEl.value?.querySelectorAll<HTMLElement>(".an-row")[index as number];
  row?.focus({ preventScroll: true });
  row?.scrollIntoView?.({ block: "nearest" });
});

// Tab (or a click on another control) moving the focus out of the list closes it; the window losing focus
// (relatedTarget null) does not.
function onFocusOut(event: FocusEvent) {
  const next = event.relatedTarget;
  if (next instanceof Node && menuEl.value && !menuEl.value.contains(next)) closeAnnotationList(false);
}

/** The quote can run to 2000 characters; a row shows one line of it. */
function excerpt(text: string, max = 60): string {
  const chars = Array.from(text);
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : text;
}
</script>

<template>
  <div v-if="count > 0" class="qr-anchor an-anchor" @click.stop>
    <div v-if="annotationListOpen" class="qr-backdrop" @click="closeAnnotationList()"></div>
    <button
      class="btn qr-trigger an-chip"
      :class="{ open: annotationListOpen }"
      type="button"
      aria-haspopup="menu"
      :aria-expanded="annotationListOpen"
      :aria-label="`${t('popup.annotations.chip')} ${count}`"
      :title="t('popup.annotations.chip')"
      :disabled="submitting"
      @click="toggleAnnotationList()"
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M21 11.5a8.4 8.4 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.4 8.4 0 0 1-3.8-.9L3 21l1.9-5.7a8.4 8.4 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.4 8.4 0 0 1 3.8-.9h.5a8.5 8.5 0 0 1 8 8z" />
      </svg>
      <!-- Hidden by the footer's narrow-window rule (popup.css); the button keeps its name through aria-label. -->
      <span class="an-chip-label">{{ t("popup.annotations.chip") }}</span>
      <span class="an-count">{{ count }}</span>
    </button>

    <div
      v-if="annotationListOpen"
      ref="menuEl"
      class="qr-menu an-list"
      :style="shiftX ? { transform: `translateX(${shiftX}px)` } : undefined"
      role="menu"
      :aria-label="t('popup.annotations.chip')"
      @paste.stop
      @focusout="onFocusOut"
    >
      <div class="qr-head">
        <span>{{ t("popup.annotations.listTitle", { n: count }) }}</span>
        <span class="qr-head-hint">{{ t("popup.annotations.listHint") }}</span>
      </div>
      <div
        v-for="(note, i) in annotationNotes"
        :key="note.id"
        class="an-row"
        :class="{ hover: i === annotationListIndex, lost: !note.anchored }"
        role="menuitem"
        tabindex="0"
        :data-annotation-row="note.id"
        @mousemove="annotationListIndex = i"
        @click="locateAnnotation(note.id)"
      >
        <span class="an-num">{{ i + 1 }}</span>
        <span class="an-row-body">
          <span class="an-row-quote">“{{ excerpt(note.quote) }}”</span>
          <span class="an-row-text">{{ note.note }}</span>
          <span v-if="!note.anchored" class="an-row-lost">{{ t("popup.annotations.unlocated") }}</span>
        </span>
        <span class="an-row-actions">
          <button class="an-icon-btn" type="button" :title="t('popup.annotations.editRow')" :aria-label="t('popup.annotations.editRow')" @click.stop="editAnnotation(note.id)">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" /></svg>
          </button>
          <button class="an-icon-btn" type="button" :title="t('popup.annotations.delete')" :aria-label="t('popup.annotations.delete')" @click.stop="removeAnnotation(note.id)">
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true"><line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /><line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg>
          </button>
        </span>
      </div>
    </div>
  </div>
</template>
