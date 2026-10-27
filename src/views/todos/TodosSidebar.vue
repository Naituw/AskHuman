<script setup lang="ts">
// Project navigation of the todos window (spec todo-sync §9.2): search, the "All todos" row and
// the project sections. Presentational: the parent owns the data (`SidebarModel`) and the
// selection; this component only renders it, reports clicks / keys and manages focus.
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ALL_KEY, stepSelection, type SidebarModel } from "./todoProjects";

const props = defineProps<{
  model: SidebarModel;
  /** Selected project key, or `ALL_KEY` for the aggregate row. */
  selectedKey: string;
  query: string;
}>();

const emit = defineEmits<{
  select: [key: string];
  "update:query": [value: string];
}>();

defineSlots<{
  /** Bottom strip (sync state); the strip is not rendered without it. */
  footer?: () => unknown;
}>();

const { t } = useI18n();

const listRef = ref<HTMLElement | null>(null);
const searchRef = ref<HTMLInputElement | null>(null);

/** Roving tab stop: the selected row, or the first row when the selection is filtered out. */
const tabStop = computed(() =>
  props.model.navKeys.includes(props.selectedKey) ? props.selectedKey : (props.model.navKeys[0] ?? "")
);

function sectionLabel(id: "withTodos" | "recent"): string {
  return t(id === "withTodos" ? "todosWin.sectionWithTodos" : "todosWin.sectionRecent");
}

function rows(): HTMLElement[] {
  return listRef.value ? Array.from(listRef.value.querySelectorAll<HTMLElement>("[data-key]")) : [];
}

async function focusRow(key: string): Promise<void> {
  await nextTick();
  const row = rows().find((el) => el.dataset.key === key);
  if (!row) return;
  row.focus({ preventScroll: true });
  row.scrollIntoView({ block: "nearest" });
}

// A long list opens scrolled to the selected row (once; later selections are driven by the user,
// who already sees where they clicked). The data arrives after mount, so wait until the selected
// row actually exists in the DOM.
let revealed = false;
watch(
  () => [props.selectedKey, props.model.rowCount] as const,
  async () => {
    if (revealed || props.model.rowCount === 0) return;
    await nextTick();
    const row = rows().find((el) => el.dataset.key === props.selectedKey);
    if (!row) return;
    revealed = true;
    row.scrollIntoView?.({ block: "center" });
  },
  { immediate: true },
);

function onListKeydown(event: KeyboardEvent): void {
  if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
  const keys = props.model.navKeys;
  const here =
    (event.target instanceof HTMLElement
      ? event.target.closest<HTMLElement>("[data-key]")?.dataset.key
      : undefined) ?? props.selectedKey;
  let next: string | null;
  switch (event.key) {
    case "ArrowDown":
      next = stepSelection(keys, here, 1);
      break;
    case "ArrowUp":
      next = stepSelection(keys, here, -1);
      break;
    case "Home":
      next = keys[0] ?? null;
      break;
    case "End":
      next = keys[keys.length - 1] ?? null;
      break;
    default:
      return;
  }
  // The window also steps through todo rows on ↑/↓; inside the sidebar those keys are ours.
  event.preventDefault();
  event.stopPropagation();
  if (!next) return;
  if (next !== props.selectedKey) emit("select", next);
  void focusRow(next);
}

function onSearchInput(event: Event): void {
  emit("update:query", (event.target as HTMLInputElement).value);
}

function onSearchKeydown(event: KeyboardEvent): void {
  if (event.isComposing || event.keyCode === 229) return;
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    if (props.query) emit("update:query", "");
    else searchRef.value?.blur();
    return;
  }
  const keys = props.model.navKeys;
  if (event.key === "ArrowDown") {
    event.preventDefault();
    event.stopPropagation();
    // Into the list, on the first match when searching, otherwise on the current row.
    const target = props.query ? (keys[1] ?? keys[0]) : tabStop.value;
    if (target) void focusRow(target);
    return;
  }
  if (event.key === "Enter" && props.query) {
    const first = keys[1];
    if (!first) return;
    event.preventDefault();
    emit("select", first);
    void focusRow(first);
  }
}

defineExpose({
  focusSearch(): void {
    searchRef.value?.focus();
    searchRef.value?.select();
  },
});
</script>

<template>
  <aside class="td-sidebar" :aria-label="t('todosWin.projectLabel')">
    <!-- macOS: the traffic lights float over this strip; it doubles as a window drag handle. -->
    <div class="td-side-drag" data-tauri-drag-region />
    <div class="td-side-search">
      <svg class="td-side-search-icon" viewBox="0 0 16 16" aria-hidden="true">
        <circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.5" />
        <path d="M10.5 10.5 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
      <input
        ref="searchRef"
        class="td-side-input"
        type="text"
        :value="query"
        :placeholder="t('todosWin.searchProjects')"
        :aria-label="t('todosWin.searchProjects')"
        spellcheck="false"
        autocomplete="off"
        @input="onSearchInput"
        @keydown="onSearchKeydown"
      />
    </div>

    <div
      ref="listRef"
      class="td-side-list"
      role="listbox"
      :aria-label="t('todosWin.projectLabel')"
      @keydown="onListKeydown"
    >
      <button
        v-if="model.navKeys.length"
        type="button"
        class="td-side-row"
        :class="{ selected: selectedKey === ALL_KEY }"
        role="option"
        :aria-selected="selectedKey === ALL_KEY"
        :tabindex="tabStop === ALL_KEY ? 0 : -1"
        :data-key="ALL_KEY"
        @click="emit('select', ALL_KEY)"
      >
        <svg class="td-side-icon" viewBox="0 0 16 16" aria-hidden="true">
          <path
            d="M5.5 4h8M5.5 8h8M5.5 12h8"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
          />
          <circle cx="2.4" cy="4" r="0.9" fill="currentColor" />
          <circle cx="2.4" cy="8" r="0.9" fill="currentColor" />
          <circle cx="2.4" cy="12" r="0.9" fill="currentColor" />
        </svg>
        <span class="td-side-name">{{ t("todosWin.allTodos") }}</span>
        <span class="td-side-count">{{ model.allCount }}</span>
      </button>

      <div
        v-for="section in model.sections"
        :key="section.id"
        class="td-side-section"
        role="group"
        :aria-label="sectionLabel(section.id)"
      >
        <div class="td-side-section-title" aria-hidden="true">{{ sectionLabel(section.id) }}</div>
        <button
          v-for="row in section.rows"
          :key="row.key"
          type="button"
          class="td-side-row"
          :class="{ selected: selectedKey === row.key }"
          role="option"
          :aria-selected="selectedKey === row.key"
          :tabindex="tabStop === row.key ? 0 : -1"
          :data-key="row.key"
          :title="row.path"
          @click="emit('select', row.key)"
        >
          <svg class="td-side-icon" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="M1.8 4.6c0-.8.6-1.4 1.4-1.4h2.9l1.5 1.6h5.2c.8 0 1.4.6 1.4 1.4v5.6c0 .8-.6 1.4-1.4 1.4H3.2c-.8 0-1.4-.6-1.4-1.4z"
              fill="none"
              stroke="currentColor"
              stroke-width="1.3"
              stroke-linejoin="round"
            />
          </svg>
          <span class="td-side-name">
            {{ row.name }}<span v-if="row.hint" class="td-side-hint"> · {{ row.hint }}</span>
          </span>
          <span v-if="row.count" class="td-side-count">{{ row.count }}</span>
        </button>
      </div>

      <p v-if="query && model.rowCount === 0" class="td-side-empty">{{ t("todosWin.noMatch") }}</p>
    </div>

    <div v-if="$slots.footer" class="td-side-foot">
      <slot name="footer" />
    </div>
  </aside>
</template>

<style scoped>
.td-sidebar {
  flex: 0 0 220px;
  width: 220px;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border-right: var(--hairline) solid var(--border);
  background: color-mix(in srgb, var(--text-primary) 3%, transparent);
}
.td-side-drag {
  flex: 0 0 auto;
  height: 0;
}
.macos .td-side-drag {
  height: 30px;
}
.td-side-search {
  position: relative;
  flex: 0 0 auto;
  padding: 8px 10px 6px;
}
.td-side-search-icon {
  position: absolute;
  left: 19px;
  top: 50%;
  width: 12px;
  height: 12px;
  margin-top: -3px;
  color: var(--text-tertiary);
  pointer-events: none;
}
.td-side-input {
  box-sizing: border-box;
  width: 100%;
  height: 28px;
  padding: 0 8px 0 27px;
  border: var(--hairline) solid var(--control-border);
  border-radius: 7px;
  background: var(--control-bg);
  color: var(--text-primary);
  font: inherit;
  font-size: 12px;
  outline: none;
}
.td-side-input::placeholder {
  color: var(--text-tertiary);
}
.td-side-input:focus-visible {
  box-shadow: var(--focus-ring);
}
.td-side-list {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 2px 8px 10px;
}
.td-side-section-title {
  padding: 12px 8px 4px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-tertiary);
}
/* Fixed row height: counts arriving later never move the rows (spec todo-sync §9.9). */
.td-side-row {
  box-sizing: border-box;
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 28px;
  padding: 0 8px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-primary);
  font: inherit;
  font-size: 12.5px;
  text-align: left;
  cursor: pointer;
}
.td-side-row:hover {
  background: color-mix(in srgb, var(--text-primary) 6%, transparent);
}
.td-side-row.selected {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}
.td-side-row:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.td-side-icon {
  flex: 0 0 auto;
  width: 14px;
  height: 14px;
  color: var(--text-secondary);
}
.td-side-row.selected .td-side-icon {
  color: var(--accent);
}
.td-side-name {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.td-side-hint {
  color: var(--text-tertiary);
}
.td-side-count {
  flex: 0 0 auto;
  min-width: 18px;
  text-align: right;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  color: var(--text-tertiary);
}
.td-side-row.selected .td-side-count {
  color: var(--text-secondary);
}
.td-side-empty {
  margin: 12px 8px;
  font-size: 12px;
  color: var(--text-tertiary);
}
.td-side-foot {
  flex: 0 0 auto;
  padding: 8px 12px;
  border-top: var(--hairline) solid var(--border);
  font-size: 11.5px;
  color: var(--text-secondary);
}
</style>
