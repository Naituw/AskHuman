<script setup lang="ts">
// 待办下拉区（spec todo-whats-next D7，第 11 轮改版；docs/specs/todo-list-actions.md）：跟在最后一个问题后面。
// 收起时只显示数量；展开后是与预选答案同构的可选行（选中＝提交时其文本加前缀并入最后一题回答、后端按 id 出队）
// + 行内完成 / 删除 + 末尾的「添加待办」行。项目没有待办时只显示一个「＋ 添加待办」入口。
// whats-next / Stop 弹窗不渲染本区（待办已是问题选项本体，动作在选项行上）；严格选择下行不可选，动作照常。
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";
import TodoAddRow from "./TodoAddRow.vue";
import TodoRowActions from "./TodoRowActions.vue";

const { t } = useI18n();
const {
  todos,
  todosOpen,
  todoChosenIds,
  todoChipsEnabled,
  todoSectionVisible,
  toggleTodo,
  completingTodoIds,
  addedTodoId,
  todoAddOpen,
  openTodoAdd,
  submitting,
} = usePopupContext();

function onToggleOpen() {
  todosOpen.value = !todosOpen.value;
}
</script>

<template>
  <div v-if="todoSectionVisible" class="todo-section">
    <!-- A project without a todo: the way to add one is all there is. -->
    <template v-if="!todos.length">
      <button
        v-if="!todoAddOpen"
        class="todo-toggle todo-entry"
        type="button"
        :disabled="submitting"
        @click="openTodoAdd()"
      >
        <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true">
          <path d="M6 2v8M2 6h8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
        <span class="todo-title">{{ t("popup.todos.add") }}</span>
      </button>
      <TodoAddRow v-else />
    </template>
    <template v-else>
      <button
        class="todo-toggle"
        type="button"
        :aria-expanded="todosOpen"
        @click="onToggleOpen"
      >
        <svg
          class="todo-caret"
          :class="{ open: todosOpen }"
          viewBox="0 0 12 12"
          width="10"
          height="10"
          aria-hidden="true"
        >
          <path
            d="M4 2.5 8 6l-4 3.5"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
        <span class="todo-title">{{ t("popup.todos.title") }}</span>
        <span class="todo-count">{{ todos.length }}</span>
        <span v-if="!todosOpen && todoChosenIds.length" class="todo-picked">
          {{ t("popup.todos.picked", { n: todoChosenIds.length }) }}
        </span>
      </button>

      <div v-if="todosOpen" class="options todo-options">
        <div
          v-for="td in todos"
          :key="td.id"
          class="option todo-option"
          :class="{
            selected: todoChosenIds.includes(td.id),
            static: !todoChipsEnabled,
            'todo-completing': completingTodoIds.has(td.id),
            'todo-added': addedTodoId === td.id,
          }"
          :data-todo-id="td.id"
          :title="todoChipsEnabled ? t('popup.todos.chipHint') : undefined"
          @click="!submitting && toggleTodo(td.id)"
        >
          <span v-if="todoChipsEnabled" class="check">{{
            todoChosenIds.includes(td.id) || completingTodoIds.has(td.id) ? "✓" : ""
          }}</span>
          <span class="label">
            {{ td.text }}
            <span v-if="td.attachments?.length" class="todo-file-count">
              {{ t("common.attachmentBadge", { n: td.attachments.length }) }}
            </span>
          </span>
          <TodoRowActions :todo-id="td.id" />
        </div>
        <TodoAddRow />
      </div>
    </template>
  </div>
</template>

<style scoped>
.todo-file-count {
  margin-left: 6px;
  color: var(--text-secondary);
  font-size: inherit;
  white-space: nowrap;
}
</style>
