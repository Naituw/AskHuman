<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyTheme } from "../lib/theme";
import {
  clearHistory,
  getHistory,
  getHistoryProjects,
  historyInit,
} from "../lib/ipc";
import type { HistoryEntry, ProjectInfo } from "../lib/types";
import HistoryDetail from "../components/HistoryDetail.vue";

const { t, locale } = useI18n();

const ALL = "__all__";

const currentProject = ref("");
const currentProjectName = ref("");
const projects = ref<ProjectInfo[]>([]);
const selected = ref<string>(ALL); // ALL or a project key ("" = unknown project)
const entries = ref<HistoryEntry[]>([]);
const activeId = ref<string | null>(null);
const loading = ref(false);

// Clear confirmation: null | "current" | "all".
const confirmKind = ref<null | "current" | "all">(null);
const menuOpen = ref(false);

const activeEntry = computed(
  () => entries.value.find((e) => e.id === activeId.value) ?? null
);

interface Opt {
  token: string;
  label: string;
}

const projectOptions = computed<Opt[]>(() => {
  const opts: Opt[] = [{ token: ALL, label: t("history.allProjects") }];
  let hasCurrent = false;
  for (const p of projects.value) {
    if (p.key === currentProject.value) hasCurrent = true;
    const name = p.key ? p.name : t("history.unknownProject");
    opts.push({ token: p.key, label: `${name} (${p.count})` });
  }
  // Always offer the current project even if it has no history yet.
  if (!hasCurrent && currentProject.value) {
    opts.push({ token: currentProject.value, label: `${currentProjectName.value} (0)` });
  }
  return opts;
});

function channelName(id: string): string {
  const key = `history.channel.${id}`;
  const name = t(key);
  return name === key ? t("history.channel.unknown") : name;
}

function summaryOf(e: HistoryEntry): string {
  const msg = e.message.text.trim();
  if (msg) return firstLine(msg);
  const q = e.questions.find((x) => x.message.trim());
  return q ? firstLine(q.message) : "";
}

function firstLine(s: string): string {
  const line = s.split("\n").find((l) => l.trim()) ?? "";
  return line.replace(/^#+\s*/, "").trim();
}

function relativeTime(ms: number): string {
  const now = Date.now();
  const diff = Math.max(0, now - ms);
  const min = Math.floor(diff / 60000);
  if (min < 1) return t("history.time.justNow");
  if (min < 60) return t("history.time.minutesAgo", { n: min });
  const hr = Math.floor(min / 60);
  if (hr < 24) return t("history.time.hoursAgo", { n: hr });
  const d = new Date(ms);
  const yd = new Date(now - 86400000);
  if (
    d.getFullYear() === yd.getFullYear() &&
    d.getMonth() === yd.getMonth() &&
    d.getDate() === yd.getDate()
  ) {
    return t("history.time.yesterday");
  }
  try {
    return new Intl.DateTimeFormat(locale.value, { dateStyle: "short" }).format(d);
  } catch {
    return d.toLocaleDateString();
  }
}

async function reload() {
  loading.value = true;
  try {
    const list =
      selected.value === ALL
        ? await getHistory(null, true)
        : await getHistory(selected.value, false);
    entries.value = list;
    // Preserve the entry the user is currently viewing; only fall back to the
    // first one when the previous selection no longer exists (or none was set).
    if (!list.some((e) => e.id === activeId.value)) {
      activeId.value = list.length ? list[0].id : null;
    }
  } finally {
    loading.value = false;
  }
}

async function onSelectProject(token: string) {
  selected.value = token;
  activeId.value = null;
  await reload();
}

function askClear(kind: "current" | "all") {
  menuOpen.value = false;
  confirmKind.value = kind;
}

async function doClear() {
  const kind = confirmKind.value;
  confirmKind.value = null;
  if (!kind) return;
  if (kind === "all") {
    await clearHistory(true, null);
  } else {
    const proj = selected.value === ALL ? currentProject.value : selected.value;
    await clearHistory(false, proj);
  }
  projects.value = await getHistoryProjects();
  await reload();
}

let unlistenUpdated: UnlistenFn | null = null;

onMounted(async () => {
  const init = await historyInit();
  applyTheme(init.theme);
  currentProject.value = init.project;
  currentProjectName.value = init.projectName;
  projects.value = await getHistoryProjects();

  const params = new URLSearchParams(window.location.search);
  // Default to current project; `--history --all` opens with everything.
  selected.value = params.get("all") === "1" ? ALL : init.project;
  await reload();

  // Live update: the backend watches history.jsonl and emits this when a new
  // reply (from any process) is recorded. reload() keeps the current selection.
  unlistenUpdated = await listen("history-updated", async () => {
    projects.value = await getHistoryProjects();
    await reload();
  });
});

onBeforeUnmount(() => unlistenUpdated?.());
</script>

<template>
  <div class="history">
    <header class="hist-header" data-tauri-drag-region>
      <span class="hist-title" data-tauri-drag-region>{{ t("history.title") }}</span>
      <div class="hist-tools">
        <select
          class="project-select"
          :value="selected"
          @change="onSelectProject(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="o in projectOptions" :key="o.token" :value="o.token">
            {{ o.label }}
          </option>
        </select>
        <div class="clear-wrap">
          <button class="clear-btn" type="button" @click="menuOpen = !menuOpen">
            {{ t("history.clear") }}
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6" /></svg>
          </button>
          <div v-if="menuOpen" class="clear-menu">
            <button
              type="button"
              :disabled="selected === ALL && !currentProject"
              @click="askClear('current')"
            >
              {{ t("history.clearCurrent") }}
            </button>
            <button type="button" @click="askClear('all')">
              {{ t("history.clearAll") }}
            </button>
          </div>
        </div>
      </div>
    </header>

    <div class="hist-body">
      <!-- Left list -->
      <ul v-if="entries.length" class="entry-list">
        <li
          v-for="e in entries"
          :key="e.id"
          class="entry"
          :class="{ active: e.id === activeId }"
          @click="activeId = e.id"
        >
          <div class="entry-top">
            <span class="badge" :class="e.action">{{ channelName(e.channel) }}</span>
            <span class="entry-time">{{ relativeTime(e.timestampMs) }}</span>
          </div>
          <div class="entry-summary">{{ summaryOf(e) || t("history.noReply") }}</div>
        </li>
      </ul>
      <div v-else class="empty">
        <p class="empty-title">{{ t("history.empty") }}</p>
        <p class="empty-hint">{{ t("history.emptyHint") }}</p>
      </div>

      <!-- Right detail -->
      <div class="detail-pane">
        <HistoryDetail v-if="activeEntry" :key="activeEntry.id" :entry="activeEntry" />
        <div v-else class="select-hint">{{ t("history.selectHint") }}</div>
      </div>
    </div>

    <!-- Clear confirmation -->
    <div v-if="confirmKind" class="overlay" @click.self="confirmKind = null">
      <div class="dialog">
        <h3>{{ confirmKind === "all" ? t("history.confirmClearAllTitle") : t("history.confirmClearCurrentTitle") }}</h3>
        <p>{{ confirmKind === "all" ? t("history.confirmClearAllDesc") : t("history.confirmClearCurrentDesc") }}</p>
        <div class="dialog-actions">
          <button class="btn-ghost" type="button" @click="confirmKind = null">{{ t("history.confirmCancel") }}</button>
          <button class="btn-danger" type="button" @click="doClear">{{ t("history.confirmOk") }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.history {
  display: flex;
  flex-direction: column;
  height: 100%;
  color: var(--text-primary);
}
/* Header */
.hist-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
}
.vibrancy .hist-header {
  padding-top: 30px;
}
.hist-title {
  font-size: 14px;
  font-weight: 600;
  flex: 1 1 auto;
}
.hist-tools {
  display: flex;
  align-items: center;
  gap: 8px;
}
.project-select {
  height: 30px;
  max-width: 240px;
  padding: 0 28px 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm, 8px);
  background: var(--bg-elevated);
  color: var(--text-primary);
  font-size: 12px;
}
.clear-wrap {
  position: relative;
}
.clear-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm, 8px);
  background: var(--bg-elevated);
  color: var(--text-primary);
  font-size: 12px;
  cursor: pointer;
}
.clear-btn svg {
  width: 13px;
  height: 13px;
}
.clear-menu {
  position: absolute;
  right: 0;
  top: calc(100% + 4px);
  z-index: 10;
  min-width: 170px;
  display: flex;
  flex-direction: column;
  padding: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm, 8px);
  background: var(--card-bg, var(--bg-elevated));
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
}
.clear-menu button {
  text-align: left;
  padding: 8px 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-primary);
  font-size: 13px;
  cursor: pointer;
}
.clear-menu button:hover:not(:disabled) {
  background: color-mix(in srgb, var(--text-primary) 8%, transparent);
}
.clear-menu button:disabled {
  opacity: 0.4;
  cursor: default;
}
/* Body split */
.hist-body {
  flex: 1 1 auto;
  display: flex;
  min-height: 0;
}
.entry-list {
  flex: 0 0 264px;
  margin: 0;
  padding: 6px;
  list-style: none;
  overflow-y: auto;
  border-right: 1px solid var(--border);
}
.entry {
  padding: 9px 10px;
  border-radius: var(--radius-sm, 8px);
  cursor: pointer;
}
.entry:hover {
  background: color-mix(in srgb, var(--text-primary) 6%, transparent);
}
.entry.active {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}
.entry-top {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}
.badge {
  display: inline-flex;
  align-items: center;
  padding: 1px 7px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
}
.badge.cancel {
  background: color-mix(in srgb, #ff453a 16%, transparent);
  color: #ff453a;
}
.entry-time {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}
.entry-summary {
  font-size: 13px;
  color: var(--text-primary);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
/* Empty list */
.empty {
  flex: 0 0 264px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 24px;
  border-right: 1px solid var(--border);
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
}
/* Detail pane */
.detail-pane {
  flex: 1 1 auto;
  min-width: 0;
  overflow-y: auto;
}
.select-hint {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-secondary);
  font-size: 13px;
}
/* Confirm dialog */
.overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.32);
}
.dialog {
  width: 320px;
  padding: 20px;
  border-radius: var(--radius, 12px);
  background: var(--card-bg, var(--bg-elevated));
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.3);
}
.dialog h3 {
  margin: 0 0 8px;
  font-size: 15px;
}
.dialog p {
  margin: 0 0 18px;
  font-size: 13px;
  color: var(--text-secondary);
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.btn-ghost,
.btn-danger {
  height: 32px;
  padding: 0 16px;
  border-radius: var(--radius-sm, 8px);
  font-size: 13px;
  cursor: pointer;
}
.btn-ghost {
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-primary);
}
.btn-danger {
  border: none;
  background: #ff453a;
  color: #fff;
}
</style>
