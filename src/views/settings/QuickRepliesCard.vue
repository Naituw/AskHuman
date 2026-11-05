<script setup lang="ts">
// Quick replies card (general tab, docs/specs/special-replies.md D27): the list the popup's Quick reply menu and the
// phone's reply menu offer. `null` in the config means "never edited": the card then shows the factory replies in the
// interface language, and the first edit turns them into the user's own list. Rows are plain text inputs that save when
// they lose focus; the handle at the left drags a row (or moves it with ↑ / ↓) and the cross deletes it.
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { quickReplyDefaults } from "../../lib/ipc";
import type { QuickReplyDefaults } from "../../lib/types";
import { useSettingsContext } from "./context";
import { QUICK_REPLIES_MAX, QUICK_REPLY_MAX_CHARS, sanitizeQuickReplies } from "./quickReplies";

interface Row {
  /** Stable across reordering, so each input keeps its own focus and caret. */
  id: number;
  text: string;
}

const { t, locale } = useI18n();
const ctx = useSettingsContext();
const { persist } = ctx;
// The parent only renders this card once the config has loaded.
const config = computed(() => ctx.config.value!);

const root = ref<HTMLElement | null>(null);
const factory = ref<QuickReplyDefaults>({ en: [], zh: [] });
const factoryList = computed(() => (String(locale.value).startsWith("zh") ? factory.value.zh : factory.value.en));
const usingDefaults = computed(() => config.value.general.quickReplies === null);

let nextId = 1;
const toRows = (texts: readonly string[]): Row[] => texts.map((text) => ({ id: nextId++, text }));
const rows = ref<Row[]>(toRows(config.value.general.quickReplies ?? []));
const full = computed(() => rows.value.length >= QUICK_REPLIES_MAX);

// The factory list arrives from the daemon (and follows the interface language) while nothing was edited.
watch(factoryList, (list) => {
  if (usingDefaults.value) rows.value = toRows(list);
});
onMounted(async () => {
  try {
    factory.value = await quickReplyDefaults();
  } catch {
    // Without the factory list the card simply starts empty; editing still works.
  }
});
if (usingDefaults.value) rows.value = toRows(factoryList.value);

/** Stores what the rows hold (blank ones are left out) and saves the settings. */
function commit() {
  config.value.general.quickReplies = sanitizeQuickReplies(rows.value.map((row) => row.text));
  void persist();
}

function onChange(index: number) {
  const row = rows.value[index];
  if (!row) return;
  // A row that was emptied is a deleted row.
  if (!row.text.trim()) rows.value.splice(index, 1);
  commit();
}

/** Leaving a row blank (a fresh one nobody typed in) drops it; nothing was stored for it. */
function dropIfBlank(id: number) {
  const index = rows.value.findIndex((row) => row.id === id);
  if (index !== -1 && !rows.value[index].text.trim()) rows.value.splice(index, 1);
}

function remove(index: number) {
  rows.value.splice(index, 1);
  commit();
}

async function add() {
  if (full.value) return;
  const row: Row = { id: nextId++, text: "" };
  rows.value.push(row);
  await nextTick();
  root.value?.querySelector<HTMLInputElement>(`[data-row-id="${row.id}"] input`)?.focus();
}

function restore() {
  config.value.general.quickReplies = null;
  rows.value = toRows(factoryList.value);
  void persist();
}

function moveRow(from: number, to: number) {
  if (from === to || to < 0 || to >= rows.value.length) return;
  const [row] = rows.value.splice(from, 1);
  rows.value.splice(to, 0, row);
}

function focusHandle(id: number) {
  root.value?.querySelector<HTMLElement>(`[data-row-id="${id}"] .qrc-handle`)?.focus();
}

/** ↑ / ↓ on a handle moves its row, the way a drag would. */
async function onHandleKey(event: KeyboardEvent, index: number) {
  const delta = event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0;
  if (!delta) return;
  event.preventDefault();
  const target = index + delta;
  if (target < 0 || target >= rows.value.length) return;
  const id = rows.value[index].id;
  moveRow(index, target);
  commit();
  await nextTick();
  focusHandle(id);
}

// Dragging: the row under the pointer takes the dragged row's place while the pointer moves; the order is saved on release.
const dragId = ref<number | null>(null);

function onHandleDown(event: PointerEvent, index: number) {
  if (event.button !== 0) return;
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  dragId.value = rows.value[index].id;
}

function rowIndexAt(y: number): number {
  const items = Array.from(root.value?.querySelectorAll<HTMLElement>(".qrc-row") ?? []);
  for (let i = 0; i < items.length; i++) {
    const rect = items[i].getBoundingClientRect();
    if (y < rect.top + rect.height / 2) return i;
  }
  return items.length - 1;
}

function onHandleMove(event: PointerEvent) {
  if (dragId.value === null) return;
  const from = rows.value.findIndex((row) => row.id === dragId.value);
  const to = rowIndexAt(event.clientY);
  if (from !== -1 && to !== -1) moveRow(from, to);
}

function onHandleUp() {
  if (dragId.value === null) return;
  dragId.value = null;
  commit();
}

const highlighted = computed(() => ctx.settingsTargetHighlight?.value === "quick-replies");
</script>

<template>
  <div
    id="quick-replies"
    ref="root"
    class="card quick-replies-card"
    :class="{ 'settings-target-highlight': highlighted }"
    data-testid="quick-replies-card"
  >
    <p class="card-title">{{ t("settings.quickReplies.title") }}</p>
    <p class="card-desc">{{ t("settings.quickReplies.hint", { max: QUICK_REPLIES_MAX }) }}</p>
    <ul v-if="rows.length" class="qrc-list">
      <li
        v-for="(row, i) in rows"
        :key="row.id"
        class="qrc-row"
        :class="{ dragging: dragId === row.id }"
        :data-row-id="row.id"
        data-testid="quick-reply-row"
      >
        <button
          class="qrc-handle"
          type="button"
          data-testid="quick-reply-handle"
          :title="t('settings.quickReplies.drag')"
          :aria-label="t('settings.quickReplies.drag')"
          @pointerdown="onHandleDown($event, i)"
          @pointermove="onHandleMove"
          @pointerup="onHandleUp"
          @pointercancel="onHandleUp"
          @keydown="onHandleKey($event, i)"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
            <circle cx="9" cy="6" r="1.6" />
            <circle cx="15" cy="6" r="1.6" />
            <circle cx="9" cy="12" r="1.6" />
            <circle cx="15" cy="12" r="1.6" />
            <circle cx="9" cy="18" r="1.6" />
            <circle cx="15" cy="18" r="1.6" />
          </svg>
        </button>
        <input
          v-model="row.text"
          class="input qrc-input"
          type="text"
          data-testid="quick-reply-input"
          :maxlength="QUICK_REPLY_MAX_CHARS"
          :placeholder="t('settings.quickReplies.placeholder')"
          @change="onChange(i)"
          @blur="dropIfBlank(row.id)"
          @keydown.enter.prevent="($event.target as HTMLInputElement).blur()"
        />
        <button
          class="qrc-remove"
          type="button"
          data-testid="quick-reply-remove"
          :title="t('settings.quickReplies.remove')"
          :aria-label="t('settings.quickReplies.remove')"
          @click="remove(i)"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
            <path d="M6 6l12 12M18 6 6 18" />
          </svg>
        </button>
      </li>
    </ul>
    <p v-if="usingDefaults" class="card-desc" data-testid="quick-reply-defaults-hint">
      {{ t("settings.quickReplies.usingDefaults") }}
    </p>
    <div class="row qrc-actions">
      <button class="btn" type="button" data-testid="quick-reply-add" :disabled="full" @click="add">
        {{ t("settings.quickReplies.add") }}
      </button>
      <span v-if="full" class="card-desc qrc-full">{{ t("settings.quickReplies.full", { max: QUICK_REPLIES_MAX }) }}</span>
      <span class="spacer"></span>
      <button v-if="!usingDefaults" class="btn" type="button" data-testid="quick-reply-restore" @click="restore">
        {{ t("settings.quickReplies.restore") }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.qrc-list {
  list-style: none;
  margin: var(--space-3) 0 var(--space-2);
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.qrc-row {
  display: flex;
  align-items: center;
  gap: 6px;
  border-radius: var(--radius-sm);
}
.qrc-row.dragging {
  opacity: 0.7;
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}
.qrc-input {
  flex: 1 1 auto;
  min-width: 0;
}
.qrc-handle,
.qrc-remove {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 28px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-tertiary);
  cursor: default;
}
.qrc-handle {
  cursor: grab;
  touch-action: none;
}
.qrc-handle:hover,
.qrc-remove:hover {
  color: var(--text-primary);
  background: var(--control-hover-bg);
}
.qrc-handle:focus-visible,
.qrc-remove:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.qrc-actions {
  gap: var(--space-3);
  margin-top: var(--space-2);
}
.qrc-full {
  margin: 0;
}
</style>
