<script setup lang="ts">
// The navbar todo button's popover (docs/specs/todo-quick-add.md): add a todo to this request's
// project, see the project's queue, delete a row (two steps), or open the todos window. It stays open
// after an add so several todos can be typed in a row; the keyboard rules live in useTodoPopover.
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";
import { resizeTextareaToContent } from "./textareaAutosize";

const { t } = useI18n();
const {
  projectName,
  todos,
  todoDraft,
  todoAdding,
  todoAddError,
  todoJustAddedId,
  todoInputEl,
  submitKeyLabel,
  addTodoFromPopover,
  removeTodoFromPopover,
  openTodosWindowFromPopover,
  closeTodoPopover,
} = usePopupContext();

/** The input stops growing at about four lines; the list below keeps its own scroll. */
const INPUT_MAX_HEIGHT = 96;

const rootEl = ref<HTMLElement | null>(null);
const inputEl = ref<HTMLTextAreaElement | null>(null);
const listEl = ref<HTMLElement | null>(null);

function resizeInput() {
  resizeTextareaToContent(inputEl.value, true, true, INPUT_MAX_HEIGHT);
}

onMounted(() => {
  // The composable focuses the input from here when the popover opens.
  todoInputEl.value = inputEl.value;
  resizeInput();
});
onBeforeUnmount(() => {
  if (todoInputEl.value === inputEl.value) todoInputEl.value = null;
});

// Typing, a restored draft and a cleared draft (after an add) all change the height.
watch(todoDraft, () => void nextTick(resizeInput));

// A row that was just added is highlighted and kept in view; it appears once the list reloads.
watch([todoJustAddedId, todos], () => {
  const id = todoJustAddedId.value;
  if (!id) return;
  void nextTick(() => {
    const row = Array.from(listEl.value?.children ?? []).find((el) => (el as HTMLElement).dataset.todoId === id);
    row?.scrollIntoView?.({ block: "nearest" });
  });
});

// Deleting needs two clicks: the first turns the ✕ into a confirm, a click anywhere else cancels.
const confirmDeleteId = ref<string | null>(null);

function onDelete(id: string) {
  if (confirmDeleteId.value !== id) {
    confirmDeleteId.value = id;
    return;
  }
  confirmDeleteId.value = null;
  void removeTodoFromPopover(id);
}

// Tab (or a click on another control) moving the focus out of the popover closes it; a click on blank
// popover space and the window losing focus (relatedTarget null) do not.
function onFocusOut(event: FocusEvent) {
  const next = event.relatedTarget;
  if (next instanceof Node && rootEl.value && !rootEl.value.contains(next)) closeTodoPopover(false);
}
</script>

<template>
  <div
    ref="rootEl"
    class="todo-popover"
    role="dialog"
    :aria-label="t('popup.todoPopover.title', { project: projectName })"
    @click.stop="confirmDeleteId = null"
    @paste.stop
    @focusout="onFocusOut"
  >
    <p class="tp-title">{{ t("popup.todoPopover.title", { project: projectName }) }}</p>
    <textarea
      ref="inputEl"
      v-model="todoDraft"
      class="tp-input"
      rows="1"
      :placeholder="t('popup.todoPopover.placeholder')"
      :aria-label="t('popup.todoPopover.placeholder')"
    ></textarea>
    <p v-if="todoAddError" class="tp-error" role="alert">{{ todoAddError }}</p>
    <div class="tp-compose">
      <span class="tp-hint">{{ t("popup.todoPopover.submitHint", { key: submitKeyLabel }) }}</span>
      <button
        class="btn btn-primary tp-add"
        type="button"
        :disabled="!todoDraft.trim() || todoAdding"
        @click="addTodoFromPopover"
      >
        {{ todoAdding ? t("popup.todoPopover.adding") : t("popup.todoPopover.add") }}
      </button>
    </div>
    <div v-if="todos.length" ref="listEl" class="tp-list" role="list">
      <div
        v-for="td in todos"
        :key="td.id"
        class="tp-row"
        :class="{ added: todoJustAddedId === td.id }"
        role="listitem"
        :data-todo-id="td.id"
      >
        <span class="tp-text" :title="td.text">
          {{ td.text }}
          <span v-if="td.attachments?.length" class="tp-badge">
            {{ t("common.attachmentBadge", { n: td.attachments.length }) }}
          </span>
        </span>
        <button
          v-if="confirmDeleteId === td.id"
          class="tp-del-confirm"
          type="button"
          @click.stop="onDelete(td.id)"
        >
          {{ t("popup.todos.deleteConfirm") }}
        </button>
        <button
          v-else
          class="tp-del"
          type="button"
          :title="t('popup.todos.delete')"
          :aria-label="t('popup.todos.delete')"
          @click.stop="onDelete(td.id)"
        >
          <svg viewBox="0 0 12 12" width="9" height="9" aria-hidden="true">
            <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
            <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </div>
    <p v-else class="tp-empty">{{ t("popup.todoPopover.empty") }}</p>
    <button class="tp-open" type="button" @click="openTodosWindowFromPopover">
      {{ t("popup.todoPopover.openWindow") }}
      <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
        <path d="M4.5 2.5H9.5V7.5M9.5 2.5L3 9" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
  </div>
</template>
