<script setup lang="ts">
// The add row at the end of a todo list (docs/specs/todo-list-actions.md D6): a dashed "＋ Add a todo" row that turns
// into a multi-line input in place. The input stays open after an add so several todos can be typed in a row; the new
// row is lit for a moment by the list. The keys (send, newline, Esc) are handled in `useTodoActions`, which sees them
// before the popup's own shortcuts do.
import { nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";
import { resizeTextareaToContent } from "./textareaAutosize";

const { t } = useI18n();
const {
  todoAddOpen,
  todoDraft,
  todoAddError,
  todoAddNote,
  todoAddInputEl,
  submitKeyLabel,
  submitting,
  openTodoAdd,
} = usePopupContext();

/** The input stops growing at about five lines. */
const INPUT_MAX_HEIGHT = 112;

const inputEl = ref<HTMLTextAreaElement | null>(null);

function resizeInput() {
  const el = inputEl.value;
  resizeTextareaToContent(el, true, true, INPUT_MAX_HEIGHT);
  // A growing input must not slide under the footer: keep the whole row on the page while the caret is in it.
  if (el && document.activeElement === el) {
    el.closest<HTMLElement>(".todo-add-wrap")?.scrollIntoView?.({ block: "nearest" });
  }
}

// The composable puts the caret in the input through this reference when the row opens (and `null` comes back when
// the input goes away).
function setInput(el: unknown) {
  const textarea = el instanceof HTMLTextAreaElement ? el : null;
  inputEl.value = textarea;
  todoAddInputEl.value = textarea;
  if (textarea) void nextTick(resizeInput);
}

// Typing, a restored draft and a cleared draft (after an add) all change the height.
watch(todoDraft, () => void nextTick(resizeInput));
</script>

<template>
  <div class="todo-add-wrap" data-find-skip>
    <button
      v-if="!todoAddOpen"
      class="todo-add"
      type="button"
      :disabled="submitting"
      @click="openTodoAdd()"
    >
      <span class="todo-add-ico">
        <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true">
          <path d="M6 2v8M2 6h8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </span>
      <span class="todo-add-label">{{ t("popup.todos.add") }}</span>
    </button>
    <!-- A paste here is text for the todo: the page's own paste handler would take clipboard images into the answer. -->
    <div v-else class="todo-add editing" @paste.stop>
      <span class="todo-add-ico">
        <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true">
          <path d="M6 2v8M2 6h8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </span>
      <span class="todo-add-field">
        <textarea
          :ref="setInput"
          v-model="todoDraft"
          class="todo-add-input"
          rows="1"
          :placeholder="t('popup.todos.addPlaceholder')"
          :aria-label="t('popup.todos.add')"
        ></textarea>
      </span>
      <span class="todo-add-hint"><kbd>{{ submitKeyLabel }}</kbd> {{ t("popup.todos.addHint") }}</span>
    </div>
    <p v-if="todoAddError" class="todo-add-error" role="alert">{{ todoAddError }}</p>
    <p v-else-if="todoAddNote" class="todo-add-note" role="status">{{ todoAddNote }}</p>
  </div>
</template>
