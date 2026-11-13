<script setup lang="ts">
// The actions at the end of a todo row (docs/specs/todo-list-actions.md D3–D5): ✓ marks it done, ✕ deletes it.
// They show while the pointer is on the row or the keyboard focus is in it, so a list stays clean and the row's own
// click (picking it) is never taken. ✓ strikes the row through for a second with "Undo" left; ✕ asks a second click
// ("Confirm delete") and a click anywhere else gives it up. Shared by the folded section of an ordinary question and
// the todo options of What's next and of a Stop confirmation.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const props = defineProps<{ todoId: string }>();

const { t } = useI18n();
const {
  completingTodoIds,
  confirmDeleteTodoId,
  submitting,
  completeTodo,
  undoCompleteTodo,
  askDeleteTodo,
  removeTodo,
} = usePopupContext();

const completing = computed(() => completingTodoIds.value.has(props.todoId));
const confirming = computed(() => confirmDeleteTodoId.value === props.todoId);
</script>

<template>
  <!-- The row's own click picks it; none of these buttons may. -->
  <span class="todo-acts" :class="{ on: completing || confirming }" @click.stop>
    <button
      v-if="completing"
      class="todo-undo"
      type="button"
      @click.stop="undoCompleteTodo(todoId)"
    >
      {{ t("popup.todos.undo") }}
    </button>
    <button
      v-else-if="confirming"
      class="todo-del-confirm"
      type="button"
      :disabled="submitting"
      @click.stop="removeTodo(todoId)"
    >
      {{ t("popup.todos.deleteConfirm") }}
    </button>
    <template v-else>
      <button
        class="todo-act todo-act-done"
        type="button"
        :disabled="submitting"
        :title="t('popup.todos.complete')"
        :aria-label="t('popup.todos.complete')"
        @click.stop="completeTodo(todoId)"
      >
        <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true">
          <path d="M2.4 6.5 4.9 9 9.7 3.3" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      <button
        class="todo-act todo-act-del"
        type="button"
        :disabled="submitting"
        :title="t('popup.todos.delete')"
        :aria-label="t('popup.todos.delete')"
        @click.stop="askDeleteTodo(todoId)"
      >
        <svg viewBox="0 0 12 12" width="9" height="9" aria-hidden="true">
          <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        </svg>
      </button>
    </template>
  </span>
</template>
