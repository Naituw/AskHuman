<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyTheme } from "../lib/theme";
import { applyLanguage } from "../i18n";
import {
  todosAdd,
  todosClear,
  todosHistory,
  todosHistoryClear,
  todosInit,
  todosList,
  todosProjects,
  todosRemove,
  todosReorder,
  todosRestore,
  todosSetAuto,
} from "../lib/ipc";
import type {
  ThemeMode,
  TodoDoneEntry,
  TodoEntry,
  TodoProjectInfo,
} from "../lib/types";

const { t } = useI18n();

const projects = ref<TodoProjectInfo[]>([]);
const selected = ref<string>("");
const entries = ref<TodoEntry[]>([]);
// 首次加载完成前显示 Loading（避免空态闪现误导）。
const loaded = ref(false);
const newText = ref("");
// 清空确认改为模态 alert（第 18 轮定案：危险操作需要光标移动后再确认，且提示无法恢复）；
// 'todos'＝清空待办队列，'history'＝清空执行历史。
const confirmKind = ref<"todos" | "history" | null>(null);
// 防连点重复新增（Enter 连击）。
const adding = ref(false);
const addInputRef = ref<HTMLInputElement | null>(null);

// 带预选项目打开（托盘 agent「添加待办」/ AgentsView 入口）＝为该项目记想法而来 →
// 自动聚焦新增输入框（footer 在数据加载后才渲染，故 nextTick）。
async function focusAddInput(): Promise<void> {
  await nextTick();
  addInputRef.value?.focus();
}

function basename(key: string): string {
  const parts = key.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || key;
}

function agentLabel(kind?: string | null): string {
  if (!kind) return "";
  const key = `agents.kind.${kind}`;
  const label = t(key);
  return label === key ? kind : label;
}

async function reloadProjects(): Promise<void> {
  const list = await todosProjects();
  // 预选项目不在候选中（如 daemon 刚退、agent 项目未入 workspace 索引）→ 兜底追加，保持选中稳定。
  if (selected.value && !list.some((p) => p.key === selected.value)) {
    list.push({ key: selected.value, name: basename(selected.value), count: 0 });
  }
  projects.value = list;
  if (!selected.value) {
    selected.value = list[0]?.key ?? "";
  }
}

async function reloadEntries(): Promise<void> {
  if (!selected.value) {
    entries.value = [];
    history.value = [];
    return;
  }
  [entries.value, history.value] = await Promise.all([
    todosList(selected.value),
    todosHistory(selected.value),
  ]);
}

async function reloadAll(): Promise<void> {
  try {
    await reloadProjects();
    await reloadEntries();
  } catch (err) {
    console.warn("todos reload failed", err);
  } finally {
    loaded.value = true;
  }
}

async function onSelect(): Promise<void> {
  confirmKind.value = null;
  confirmDeleteId.value = null;
  histOpen.value = false;
  await reloadEntries();
}

// 添加行「自动执行」开关（第 17 轮定案）：勾选后新增的待办在 whats-next 时直接派发。
const newAuto = ref(false);
const newAutoHintHovered = ref(false);
const newAutoHintKeyboardFocused = ref(false);
const newAutoHintPinned = ref(false);
let newAutoHintTimer: number | undefined;

function onNewAutoFocus(e: FocusEvent): void {
  const target = e.currentTarget;
  newAutoHintKeyboardFocused.value =
    target instanceof HTMLElement && target.matches(":focus-visible");
}

function toggleNewAuto(): void {
  newAuto.value = !newAuto.value;
  newAutoHintPinned.value = true;
  if (newAutoHintTimer !== undefined) window.clearTimeout(newAutoHintTimer);
  newAutoHintTimer = window.setTimeout(() => {
    newAutoHintPinned.value = false;
    newAutoHintTimer = undefined;
  }, 500);
}

async function addEntry(): Promise<void> {
  const text = newText.value.trim();
  if (!text || !selected.value || adding.value) return;
  adding.value = true;
  try {
    await todosAdd(selected.value, text, newAuto.value);
    newText.value = "";
    await reloadAll();
  } catch (err) {
    console.warn("todo add failed", err);
  } finally {
    adding.value = false;
  }
}

// 行内 ⚡ 切换自动执行。
async function toggleAuto(e: TodoEntry): Promise<void> {
  if (!selected.value) return;
  try {
    await todosSetAuto(selected.value, e.id, !e.auto);
  } catch (err) {
    console.warn("todo set auto failed", err);
  }
  await reloadAll();
}

function onNewKeydown(e: KeyboardEvent): void {
  // isComposing：IME 组词中的 Enter 不当提交。
  if (e.key === "Enter" && !e.isComposing) {
    e.preventDefault();
    void addEntry();
  }
}

// 删除二次确认（第 13 轮定案，防误删）：首次点 ✕ 该行按钮变「确认删除」文字，再点才删；
// 点其它行的 ✕ 把确认焦点挪过去，点删除按钮之外的任意区域取消确认态（删除按钮 @click.stop，
// 其余点击冒泡到根容器复位），切项目/删除后复位。
const confirmDeleteId = ref<string | null>(null);

function onRootClick(): void {
  confirmDeleteId.value = null;
}

async function removeEntry(id: string): Promise<void> {
  if (!selected.value) return;
  if (confirmDeleteId.value !== id) {
    confirmDeleteId.value = id;
    return;
  }
  confirmDeleteId.value = null;
  try {
    await todosRemove(selected.value, id);
  } catch (err) {
    console.warn("todo remove failed", err);
  }
  await reloadAll();
}

async function doConfirmedClear(): Promise<void> {
  const kind = confirmKind.value;
  confirmKind.value = null;
  if (!selected.value || !kind) return;
  try {
    if (kind === "todos") {
      await todosClear(selected.value);
    } else {
      await todosHistoryClear(selected.value);
    }
  } catch (err) {
    console.warn("todo clear failed", err);
  }
  await reloadAll();
}

function absoluteTime(ms: number): string {
  return ms ? new Date(ms).toLocaleString() : "";
}

// ===== 执行历史（第 16 轮定案：仅执行出队进历史；可一键恢复回队列末尾）=====
const history = ref<TodoDoneEntry[]>([]);
const histOpen = ref(false);

function shortTime(ms: number): string {
  if (!ms) return "";
  return new Date(ms).toLocaleString(undefined, {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

async function restoreEntry(id: string): Promise<void> {
  if (!selected.value) return;
  try {
    await todosRestore(selected.value, id);
  } catch (err) {
    console.warn("todo restore failed", err);
  }
  await reloadAll();
}

// ===== 拖拽排序（第 14 轮定案，仅 GUI 窗口）=====
// 手柄 dragstart 记起点；经过其它行时本地 splice 实时预览；dragend 一次性持久化
// （todos.json 写入触发 todos-updated → reloadAll，与其它进程的并发增删自然合流）。
const dragIndex = ref<number | null>(null);

function onDragStart(i: number, e: DragEvent): void {
  dragIndex.value = i;
  confirmDeleteId.value = null;
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = "move";
    // Safari/WebKit 需要 setData 才启动拖拽。
    e.dataTransfer.setData("text/plain", String(i));
  }
}

function onDragOverRow(i: number, e: DragEvent): void {
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
  const from = dragIndex.value;
  if (from === null || from === i) return;
  const moved = entries.value.splice(from, 1)[0];
  entries.value.splice(i, 0, moved);
  dragIndex.value = i;
}

async function onDragEnd(): Promise<void> {
  if (dragIndex.value === null) return;
  dragIndex.value = null;
  if (!selected.value) return;
  try {
    await todosReorder(
      selected.value,
      entries.value.map((e) => e.id)
    );
  } catch (err) {
    console.warn("todo reorder failed", err);
    await reloadAll();
  }
}

let unlistenUpdated: UnlistenFn | null = null;
let unlistenGoto: UnlistenFn | null = null;
let unlistenSettings: UnlistenFn | null = null;

onMounted(async () => {
  try {
    const init = await todosInit();
    applyTheme(init.theme);
    applyLanguage(init.lang);
  } catch {
    /* 读取失败：保持兜底外观 */
  }
  const preselect = new URLSearchParams(window.location.search).get("project");
  if (preselect) selected.value = preselect;
  // 设置变更实时生效（主题/语言与设置窗口同宿主进程广播）。
  unlistenSettings = await listen<{ theme?: ThemeMode; language?: string }>(
    "settings-updated",
    (e) => {
      if (typeof e.payload.theme === "string") applyTheme(e.payload.theme);
      if (typeof e.payload.language === "string") applyLanguage(e.payload.language);
    }
  );
  // todos.json 被任意进程改写（CLI/弹窗/出队）→ 宿主文件监听推事件 → 重载。
  unlistenUpdated = await listen("todos-updated", () => {
    void reloadAll();
  });
  // 窗口已开时再次带预选项目打开（托盘 agent「添加待办」/ Agent 卡片入口）→ 切换选中项目。
  unlistenGoto = await listen<string>("todos-goto-project", async (e) => {
    if (typeof e.payload === "string" && e.payload) {
      selected.value = e.payload;
      confirmKind.value = null;
      confirmDeleteId.value = null;
      histOpen.value = false;
      await reloadAll();
      await focusAddInput();
    }
  });
  await reloadAll();
  if (preselect) await focusAddInput();
});

onBeforeUnmount(() => {
  if (newAutoHintTimer !== undefined) window.clearTimeout(newAutoHintTimer);
  unlistenUpdated?.();
  unlistenGoto?.();
  unlistenSettings?.();
});
</script>

<template>
  <div class="todos-win" @click="onRootClick">
    <header class="td-header" data-tauri-drag-region>
      <span class="td-title" data-tauri-drag-region>{{ t("todosWin.title") }}</span>
      <select
        v-if="projects.length"
        v-model="selected"
        class="td-select"
        :aria-label="t('todosWin.projectLabel')"
        @change="onSelect"
      >
        <option v-for="p in projects" :key="p.key" :value="p.key" :title="p.key">
          {{ p.name }}{{ p.count ? ` (${p.count})` : "" }}
        </option>
      </select>
    </header>

    <div class="td-body">
      <div v-if="!loaded" class="empty">
        <span class="spinner" />
      </div>

      <div v-else-if="!projects.length" class="empty">
        <p class="empty-title">{{ t("todosWin.noProjects") }}</p>
        <p class="empty-hint">{{ t("todosWin.noProjectsHint") }}</p>
      </div>

      <template v-else>
      <div v-if="!entries.length" class="empty" :class="{ compact: history.length }">
        <p class="empty-title">{{ t("todosWin.empty") }}</p>
        <p class="empty-hint">{{ t("todosWin.emptyHint") }}</p>
      </div>

      <ul v-else class="td-list">
        <li
          v-for="(e, i) in entries"
          :key="e.id"
          class="td-row"
          :class="{ dragging: dragIndex === i }"
          @dragover="onDragOverRow(i, $event)"
          @drop.prevent
        >
          <span
            v-if="entries.length > 1"
            class="td-handle"
            draggable="true"
            :title="t('todosWin.dragHint')"
            @dragstart="onDragStart(i, $event)"
            @dragend="onDragEnd"
          >
            <svg viewBox="0 0 10 14" aria-hidden="true">
              <circle cx="3" cy="3" r="1.1" fill="currentColor" />
              <circle cx="7" cy="3" r="1.1" fill="currentColor" />
              <circle cx="3" cy="7" r="1.1" fill="currentColor" />
              <circle cx="7" cy="7" r="1.1" fill="currentColor" />
              <circle cx="3" cy="11" r="1.1" fill="currentColor" />
              <circle cx="7" cy="11" r="1.1" fill="currentColor" />
            </svg>
          </span>
          <div class="td-content" :title="absoluteTime(e.createdAtMs)">
            <span class="td-text">{{ e.text }}</span>
            <span v-if="agentLabel(e.agentKind)" class="td-source">
              {{ t("todosWin.addedBy", { agent: agentLabel(e.agentKind) }) }}
            </span>
          </div>
          <button
            type="button"
            class="td-auto"
            :class="{ on: e.auto }"
            :title="e.auto ? t('todosWin.autoOff') : t('todosWin.autoOn')"
            :aria-label="e.auto ? t('todosWin.autoOff') : t('todosWin.autoOn')"
            @click="toggleAuto(e)"
          >
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path d="M6.8 1 L2.5 7 H5.6 L5.2 11 L9.5 5 H6.4 Z"
                :fill="e.auto ? 'currentColor' : 'none'"
                stroke="currentColor" stroke-width="1" stroke-linejoin="round" />
            </svg>
          </button>
          <button
            v-if="confirmDeleteId === e.id"
            type="button"
            class="td-del-confirm"
            @click.stop="removeEntry(e.id)"
          >
            {{ t("todosWin.deleteConfirm") }}
          </button>
          <button
            v-else
            type="button"
            class="td-del"
            :title="t('todosWin.delete')"
            :aria-label="t('todosWin.delete')"
            @click.stop="removeEntry(e.id)"
          >
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path d="M3 3 L9 9 M9 3 L3 9" stroke="currentColor" stroke-width="1.4"
                stroke-linecap="round" />
            </svg>
          </button>
        </li>
      </ul>

      <!-- 清空（第 18 轮定案：列表区末尾、历史区上方，紧邻其作用对象、远离新增行；
           点击弹模态确认，需要移动光标二次确认）。 -->
      <div v-if="entries.length" class="td-clear-row">
        <button type="button" class="td-clear" @click="confirmKind = 'todos'">
          {{ t("todosWin.clear") }}
        </button>
      </div>

      <!-- 执行历史折叠区：仅执行出队的待办；可一键恢复回队列末尾。 -->
      <section v-if="history.length" class="td-hist">
        <div class="td-hist-head">
          <button type="button" class="td-hist-toggle" @click="histOpen = !histOpen">
            <span class="td-hist-caret" :class="{ open: histOpen }">▸</span>
            <span>{{ t("todosWin.historyTitle") }}</span>
            <span class="td-hist-count">{{ history.length }}</span>
          </button>
          <button
            v-if="histOpen"
            type="button"
            class="td-clear"
            @click="confirmKind = 'history'"
          >
            {{ t("todosWin.clearHist") }}
          </button>
        </div>
        <ul v-if="histOpen" class="td-list td-hist-list">
          <li v-for="h in history" :key="h.id" class="td-row td-hist-row">
            <div class="td-content" :title="absoluteTime(h.doneAtMs)">
              <span class="td-text">{{ h.text }}</span>
              <span v-if="agentLabel(h.agentKind)" class="td-source">
                {{ t("todosWin.addedBy", { agent: agentLabel(h.agentKind) }) }}
              </span>
            </div>
            <span class="td-hist-time">{{ shortTime(h.doneAtMs) }}</span>
            <button
              type="button"
              class="td-restore"
              :title="t('todosWin.restore')"
              :aria-label="t('todosWin.restore')"
              @click="restoreEntry(h.id)"
            >
              <svg viewBox="0 0 12 12" aria-hidden="true">
                <path d="M4.5 2 L2 4.5 L4.5 7 M2 4.5 H8 A2.5 2.5 0 0 1 8 9.5 H5"
                  fill="none" stroke="currentColor" stroke-width="1.3"
                  stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
          </li>
        </ul>
      </section>
      </template>
    </div>

    <footer v-if="projects.length" class="td-footer">
      <div class="td-add">
        <input
          ref="addInputRef"
          v-model="newText"
          class="td-input"
          type="text"
          :placeholder="t('todosWin.addPlaceholder')"
          @keydown="onNewKeydown"
        />
        <div
          class="td-auto-hint-anchor"
          @mouseenter="newAutoHintHovered = true"
          @mouseleave="newAutoHintHovered = false"
        >
          <button
            type="button"
            class="td-auto td-auto-new"
            :class="{ on: newAuto }"
            :aria-label="newAuto ? t('todosWin.autoOff') : t('todosWin.autoOn')"
            aria-describedby="todo-auto-new-hint"
            :aria-pressed="newAuto"
            @focus="onNewAutoFocus"
            @blur="newAutoHintKeyboardFocused = false"
            @click="toggleNewAuto"
          >
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path d="M6.8 1 L2.5 7 H5.6 L5.2 11 L9.5 5 H6.4 Z"
                :fill="newAuto ? 'currentColor' : 'none'"
                stroke="currentColor" stroke-width="1" stroke-linejoin="round" />
            </svg>
            <span>{{ t("todosWin.autoLabel") }}</span>
          </button>
          <div
            v-show="newAutoHintHovered || newAutoHintKeyboardFocused || newAutoHintPinned"
            id="todo-auto-new-hint"
            class="td-auto-hint"
            role="tooltip"
            aria-live="polite"
          >
            {{ t("todosWin.autoNewHint") }}
          </div>
        </div>
        <button
          type="button"
          class="td-btn td-btn-add"
          :disabled="!newText.trim() || adding"
          @click="addEntry"
        >
          {{ t("todosWin.add") }}
        </button>
      </div>
    </footer>

    <!-- 清空确认（模态 alert）：清空不可恢复，强制光标移动后再确认。 -->
    <div v-if="confirmKind" class="td-overlay" @click.self="confirmKind = null">
      <div class="td-dialog" role="alertdialog">
        <h3>
          {{ confirmKind === "todos" ? t("todosWin.clearTitle") : t("todosWin.clearHistTitle") }}
        </h3>
        <p>
          {{
            confirmKind === "todos"
              ? t("todosWin.clearDesc", { n: entries.length })
              : t("todosWin.clearHistDesc", { n: history.length })
          }}
        </p>
        <div class="td-dialog-actions">
          <button type="button" class="td-btn" @click="confirmKind = null">
            {{ t("todosWin.confirmCancel") }}
          </button>
          <button type="button" class="td-btn td-btn-danger" @click="doConfirmedClear">
            {{ t("todosWin.clearOk") }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.todos-win {
  display: flex;
  flex-direction: column;
  height: 100%;
  color: var(--text-primary);
}
.td-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  border-bottom: var(--hairline) solid var(--border);
}
.macos .td-header {
  padding-top: 30px;
}
.td-title {
  font-size: 14px;
  font-weight: 600;
  white-space: nowrap;
}
.td-select {
  flex: 0 1 auto;
  min-width: 0;
  max-width: 60%;
  appearance: auto;
  border: var(--hairline) solid var(--border);
  border-radius: 7px;
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 12px;
  padding: 3px 8px;
  box-shadow: var(--clickable-shadow);
}
.td-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px;
}
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 100%;
  text-align: center;
}
.empty-title {
  font-size: 14px;
  font-weight: 600;
  margin: 0;
}
.empty-hint {
  font-size: 12px;
  color: var(--text-secondary);
  margin: 0;
  max-width: 320px;
}
.spinner {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: 2px solid color-mix(in srgb, var(--text-primary) 18%, transparent);
  border-top-color: var(--text-secondary);
  animation: td-spin 0.7s linear infinite;
}
@keyframes td-spin {
  to {
    transform: rotate(360deg);
  }
}
.td-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.td-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 9px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm, 8px);
  background: var(--bg-elevated);
}
.td-row.dragging {
  opacity: 0.55;
  border-style: dashed;
  border-color: var(--control-border);
}
.td-handle {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 20px;
  margin: 0 -2px 0 -4px;
  color: var(--text-secondary);
  opacity: 0.55;
  cursor: grab;
}
.td-handle:active {
  cursor: grabbing;
}
.td-row:hover .td-handle {
  opacity: 1;
}
.td-handle svg {
  width: 10px;
  height: 14px;
}
.td-content {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.td-text {
  font-size: 13px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
}
.td-source {
  color: var(--text-muted, #8e8e93);
  font-size: 10px;
  line-height: 1.25;
}
/* 自动执行 ⚡ 切换：未开启时随行 hover 淡入；开启后常显琥珀色实心。 */
.td-auto {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-top: 1px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  opacity: 0;
  cursor: pointer;
}
.td-row:hover .td-auto {
  opacity: 0.75;
}
.td-auto.on {
  opacity: 1;
  color: #ff9f0a;
}
.td-auto:hover {
  opacity: 1;
  background: color-mix(in srgb, #ff9f0a 14%, transparent);
  color: #ff9f0a;
}
.td-auto svg {
  width: 12px;
  height: 12px;
}
/* 添加行的 ⚡ 开关：常显（不依赖行 hover），带「自动」文字标签便于理解。 */
.td-auto-hint-anchor {
  position: relative;
  display: inline-flex;
  align-items: center;
  flex: 0 0 auto;
}
.td-auto-new {
  opacity: 0.75;
  align-self: center;
  border: var(--hairline) solid var(--border);
  width: auto;
  height: 28px;
  padding: 0 8px;
  gap: 3px;
  border-radius: 7px;
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
}
.td-auto-new.on {
  opacity: 1;
  border-color: color-mix(in srgb, #ff9f0a 55%, transparent);
  background: color-mix(in srgb, #ff9f0a 12%, transparent);
}
.td-auto-hint {
  position: absolute;
  right: 0;
  bottom: calc(100% + 8px);
  z-index: 20;
  width: max-content;
  max-width: min(280px, calc(100vw - 28px));
  padding: 7px 9px;
  border: var(--hairline) solid var(--border);
  border-radius: 7px;
  background: var(--bg);
  box-shadow: 0 5px 18px rgba(0, 0, 0, 0.22);
  color: var(--text-primary);
  font-size: 11px;
  font-weight: 400;
  line-height: 1.45;
  text-align: left;
  white-space: normal;
  pointer-events: none;
}
.td-auto-hint::after {
  content: "";
  position: absolute;
  right: 18px;
  bottom: -5px;
  width: 8px;
  height: 8px;
  border-right: var(--hairline) solid var(--border);
  border-bottom: var(--hairline) solid var(--border);
  background: var(--bg);
  transform: rotate(45deg);
}
.td-del {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-top: 1px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}
.td-del:hover {
  background: color-mix(in srgb, #ff453a 14%, transparent);
  color: #ff453a;
}
.td-del svg {
  width: 12px;
  height: 12px;
}
/* 二次确认态：✕ 变红色文字按钮，再点才真正删除。 */
.td-del-confirm {
  flex: 0 0 auto;
  margin-top: 1px;
  padding: 1px 8px;
  border: 1px solid color-mix(in srgb, #ff453a 45%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, #ff453a 12%, transparent);
  color: #ff453a;
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
  cursor: pointer;
}
.td-del-confirm:hover {
  background: color-mix(in srgb, #ff453a 22%, transparent);
}
.td-footer {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 14px 12px;
  border-top: var(--hairline) solid var(--border);
}
.td-add {
  display: flex;
  gap: 8px;
}
.td-input {
  flex: 1 1 auto;
  min-width: 0;
  border: var(--hairline) solid var(--control-border);
  border-radius: 7px;
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 12px;
  padding: 6px 9px;
  box-shadow: var(--clickable-shadow);
}
.td-input:focus {
  outline: none;
  border-color: color-mix(in srgb, #0a84ff 55%, transparent);
}
.td-btn {
  appearance: none;
  flex: 0 0 auto;
  border: var(--hairline) solid var(--control-border);
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 12px;
  font-weight: 600;
  padding: 5px 12px;
  border-radius: 7px;
  cursor: pointer;
  box-shadow: var(--clickable-shadow);
}
.td-btn:hover:not(:disabled) {
  background: var(--control-hover-bg);
}
.td-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.td-btn-add {
  border-color: transparent;
  background: #0a84ff;
  color: #fff;
}
.td-btn-add:hover:not(:disabled) {
  background: #0071e3;
}
.td-btn-danger {
  border-color: transparent;
  background: #ff453a;
  color: #fff;
}
.td-btn-danger:hover {
  background: #e0352b;
}
.td-clear-row {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 6px;
}
.td-clear {
  appearance: none;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  padding: 2px 6px;
  border-radius: 5px;
  cursor: pointer;
}
.td-clear:hover {
  background: color-mix(in srgb, #ff453a 12%, transparent);
  color: #ff453a;
}
/* 空态与历史并存时不再全高居中。 */
.empty.compact {
  height: auto;
  padding: 28px 0 20px;
}
/* ===== 执行历史折叠区 ===== */
.td-hist {
  margin-top: 14px;
}
.td-hist-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.td-hist-toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: none;
  background: transparent;
  padding: 2px 4px;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  border-radius: 5px;
}
.td-hist-toggle:hover {
  color: var(--text-primary);
}
.td-hist-caret {
  display: inline-block;
  font-size: 10px;
  transition: transform 0.15s ease;
}
.td-hist-caret.open {
  transform: rotate(90deg);
}
.td-hist-count {
  padding: 0 6px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--text-primary) 10%, transparent);
  font-size: 11px;
  font-weight: 600;
}
.td-hist-list {
  margin-top: 8px;
}
.td-hist-row {
  align-items: center;
}
.td-hist-row .td-text {
  color: var(--text-secondary);
}
.td-hist-time {
  flex: 0 0 auto;
  font-size: 11px;
  color: var(--text-muted, #8e8e93);
  white-space: nowrap;
}
.td-restore {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}
.td-restore:hover {
  background: color-mix(in srgb, #0a84ff 14%, transparent);
  color: #0a84ff;
}
.td-restore svg {
  width: 12px;
  height: 12px;
}
/* ===== 清空确认模态（与历史窗口 .overlay/.dialog 同构） ===== */
.td-overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.32);
}
.td-dialog {
  width: 300px;
  padding: 20px;
  border-radius: var(--radius, 12px);
  /* --card-bg 是近乎透明的叠色，会与底下文字混叠；模态框必须不透明底。 */
  background: var(--bg, #fff);
  border: var(--hairline) solid var(--border);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.3);
}
.td-dialog h3 {
  margin: 0 0 8px;
  font-size: 14px;
}
.td-dialog p {
  margin: 0 0 18px;
  font-size: 12px;
  color: var(--text-secondary);
}
.td-dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
