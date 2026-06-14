<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyTheme } from "../lib/theme";
import { applyLanguage } from "../i18n";
import { agentsInit, agentsStartSubscription } from "../lib/ipc";
import type { AgentKind, AgentRecord, AgentRunState } from "../lib/types";

const { t } = useI18n();

const agents = ref<AgentRecord[]>([]);
// 是否已收到首帧快照（在此之前显示 Loading，而非"暂无 Agent"，避免误导）。
const loaded = ref(false);
// 每秒重算一次相对时间（与数据推送解耦）。
const nowMs = ref(Date.now());

// 查看维度：状态（默认，运行中置顶）/ 类型 / 项目。
type ViewMode = "status" | "type" | "project";
const VIEW_MODES: ViewMode[] = ["status", "type", "project"];
const STORAGE_KEY = "askhuman.agents.viewMode";

function loadMode(): ViewMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "status" || v === "type" || v === "project") return v;
  } catch {
    /* localStorage 不可用：用默认 */
  }
  return "status";
}

const mode = ref<ViewMode>(loadMode());
watch(mode, (v) => {
  try {
    localStorage.setItem(STORAGE_KEY, v);
  } catch {
    /* 忽略持久化失败 */
  }
});

// 分组折叠：按 `<mode>:<groupKey>` 记忆，跨维度互不影响，持久化到 localStorage。
const COLLAPSE_KEY = "askhuman.agents.collapsed";
function loadCollapsed(): Set<string> {
  try {
    const v = localStorage.getItem(COLLAPSE_KEY);
    if (v) return new Set(JSON.parse(v) as string[]);
  } catch {
    /* 忽略 */
  }
  return new Set();
}
const collapsed = ref<Set<string>>(loadCollapsed());
function collapseId(g: Group): string {
  return `${mode.value}:${g.key}`;
}
function isCollapsed(g: Group): boolean {
  return collapsed.value.has(collapseId(g));
}
function toggleCollapse(g: Group): void {
  const id = collapseId(g);
  const next = new Set(collapsed.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  collapsed.value = next; // 替换整集合以触发响应式更新
  try {
    localStorage.setItem(COLLAPSE_KEY, JSON.stringify([...next]));
  } catch {
    /* 忽略持久化失败 */
  }
}

// 类型分组顺序。
const KIND_ORDER: AgentKind[] = ["claude", "codex", "cursor"];
// 状态分组顺序（运行中置顶）。
const STATE_ORDER: AgentRunState[] = ["working", "idle", "ended"];

interface Group {
  key: string;
  label: string;
  // 是否高亮（状态视图的「工作中」组用绿色强调）。
  accent: boolean;
  items: AgentRecord[];
}

// 用于排序/相对时间的「该记录的时间锚点」（秒）。
function anchor(a: AgentRecord): number {
  if (a.state === "ended") return a.endedAt ?? a.lastActivity;
  return a.lastActivity;
}

// 任一分类下，组内一律按时间倒序（新→旧）。
function byTimeDesc(x: AgentRecord, y: AgentRecord): number {
  return anchor(y) - anchor(x);
}

const groups = computed<Group[]>(() => {
  const list = agents.value;

  if (mode.value === "type") {
    return KIND_ORDER.map((k) => ({
      key: k,
      label: kindLabel(k),
      accent: false,
      items: list.filter((a) => a.kind === k).sort(byTimeDesc),
    })).filter((g) => g.items.length > 0);
  }

  if (mode.value === "project") {
    const map = new Map<string, AgentRecord[]>();
    for (const a of list) {
      const key = a.cwd ? projectName(a.cwd) : "";
      const arr = map.get(key) ?? [];
      arr.push(a);
      map.set(key, arr);
    }
    const result: Group[] = [];
    for (const [key, items] of map) {
      result.push({
        key: key || "__unknown__",
        label: key || t("agents.unknownProject"),
        accent: false,
        items: items.sort(byTimeDesc),
      });
    }
    // 组按「该组最近活动」倒序；未知项目排最后。
    result.sort((x, y) => {
      const xu = x.key === "__unknown__";
      const yu = y.key === "__unknown__";
      if (xu !== yu) return xu ? 1 : -1;
      return anchor(y.items[0]) - anchor(x.items[0]);
    });
    return result;
  }

  // 默认：按状态（运行中置顶）。
  return STATE_ORDER.map((s) => ({
    key: s,
    label: stateLabel(s),
    accent: s === "working",
    items: list.filter((a) => a.state === s).sort(byTimeDesc),
  })).filter((g) => g.items.length > 0);
});

const isEmpty = computed(() => agents.value.length === 0);
const isLoading = computed(() => !loaded.value);

function viewLabel(m: ViewMode): string {
  return t(`agents.view.${m}`);
}

function kindLabel(kind: AgentKind): string {
  return t(`agents.kind.${kind}`);
}

function stateLabel(s: AgentRunState): string {
  return t(`agents.state.${s}`);
}

function projectName(cwd?: string | null): string {
  if (!cwd) return "";
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || cwd;
}

// 卡片是否展示某维度信息：分组所依据的维度在卡片上省略（组标题已表达）。
function showKind(): boolean {
  return mode.value !== "type";
}
function showState(): boolean {
  return mode.value !== "status";
}
function showProject(a: AgentRecord): boolean {
  return !!a.cwd && mode.value !== "project";
}

// 绝对时间（hover 提示用）：保留简洁的相对显示，同时可悬停看到精确时间。
function absoluteTime(secs?: number | null): string {
  if (!secs) return "";
  return new Date(secs * 1000).toLocaleString();
}

// 后端时间戳为 unix 秒。
function relativeTime(secs?: number | null): string {
  if (!secs) return "";
  const diff = Math.max(0, Math.floor(nowMs.value / 1000) - secs);
  if (diff < 5) return t("agents.time.justNow");
  if (diff < 60) return t("agents.time.secondsAgo", { n: diff });
  const min = Math.floor(diff / 60);
  if (min < 60) return t("agents.time.minutesAgo", { n: min });
  const hr = Math.floor(min / 60);
  if (hr < 24) return t("agents.time.hoursAgo", { n: hr });
  const d = Math.floor(hr / 24);
  return t("agents.time.daysAgo", { n: d });
}

let unlisten: UnlistenFn | null = null;
let ticker: number | undefined;

onMounted(async () => {
  // 先注册监听，再触发后端订阅：daemon 一连上就推首帧立即快照，监听必须先就绪才不丢帧。
  unlisten = await listen<AgentRecord[]>("agents-updated", (e) => {
    agents.value = Array.isArray(e.payload) ? e.payload : [];
    loaded.value = true;
  });
  try {
    const init = await agentsInit();
    applyTheme(init.theme);
    applyLanguage(init.lang);
  } catch {
    /* 读取失败：保持兜底外观 */
  }
  // 监听已就绪，启动到 daemon 的快照订阅。
  try {
    await agentsStartSubscription();
  } catch {
    /* 订阅启动失败：窗口停留在 Loading，由后端重连逻辑兜底 */
  }
  ticker = window.setInterval(() => {
    nowMs.value = Date.now();
  }, 1000);
});

onBeforeUnmount(() => {
  unlisten?.();
  if (ticker) window.clearInterval(ticker);
});
</script>

<template>
  <div class="agents">
    <header class="ag-header" data-tauri-drag-region>
      <span class="ag-title" data-tauri-drag-region>{{ t("agents.title") }}</span>
      <div v-if="!isLoading && !isEmpty" class="seg" role="tablist">
        <button
          v-for="m in VIEW_MODES"
          :key="m"
          class="seg-btn"
          :class="{ active: mode === m }"
          role="tab"
          :aria-selected="mode === m"
          @click="mode = m"
        >
          {{ viewLabel(m) }}
        </button>
      </div>
    </header>

    <div class="ag-body">
      <div v-if="isLoading" class="empty">
        <span class="spinner" />
        <p class="empty-hint">{{ t("agents.loading") }}</p>
      </div>

      <div v-else-if="isEmpty" class="empty">
        <p class="empty-title">{{ t("agents.empty") }}</p>
        <p class="empty-hint">{{ t("agents.emptyHint") }}</p>
      </div>

      <template v-else>
        <section v-for="g in groups" :key="g.key" class="group">
          <button
            type="button"
            class="group-title"
            :class="{ accent: g.accent, collapsed: isCollapsed(g) }"
            :aria-expanded="!isCollapsed(g)"
            @click="toggleCollapse(g)"
          >
            <svg class="chevron" viewBox="0 0 12 12" aria-hidden="true">
              <path d="M4 2.5 L8 6 L4 9.5" fill="none" stroke="currentColor" stroke-width="1.6"
                stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            <span class="group-label">{{ g.label }}</span>
            <span class="group-count">{{ g.items.length }}</span>
          </button>
          <ul v-show="!isCollapsed(g)" class="card-list">
            <li
              v-for="a in g.items"
              :key="a.sessionId"
              class="card"
              :class="a.state"
            >
              <div class="card-top">
                <span class="dot" :class="a.state" />
                <span v-if="showKind()" class="kind-badge">{{ kindLabel(a.kind) }}</span>
                <span class="card-title">{{ a.title || t("agents.untitled") }}</span>
                <span v-if="showState()" class="status-badge" :class="a.state">
                  {{ stateLabel(a.state) }}
                </span>
              </div>

              <div v-if="showProject(a) || a.pid" class="meta">
                <span v-if="showProject(a)" class="meta-item" :title="a.cwd ?? ''">
                  <span class="meta-k">{{ t("agents.field.project") }}</span>
                  <span class="meta-v">{{ projectName(a.cwd) }}</span>
                </span>
                <span v-if="a.pid" class="meta-item">
                  <span class="meta-k">{{ t("agents.field.pid") }}</span>
                  <span class="meta-v mono">{{ a.pid }}</span>
                </span>
              </div>

              <div class="meta">
                <span class="meta-item full">
                  <span class="meta-k">{{ t("agents.field.session") }}</span>
                  <span class="meta-v mono sid">{{ a.sessionId }}</span>
                </span>
              </div>

              <div class="meta times">
                <span class="meta-item">
                  <span class="meta-k">{{ t("agents.field.started") }}</span>
                  <span class="meta-v" :title="absoluteTime(a.startedAt)">
                    {{ relativeTime(a.startedAt) }}
                  </span>
                </span>
                <span class="meta-item">
                  <span class="meta-k">{{
                    a.state === "ended"
                      ? t("agents.state.ended")
                      : t("agents.field.lastActivity")
                  }}</span>
                  <span
                    class="meta-v"
                    :title="absoluteTime(a.state === 'ended' ? a.endedAt : a.lastActivity)"
                  >
                    {{ relativeTime(a.state === "ended" ? a.endedAt : a.lastActivity) }}
                  </span>
                </span>
              </div>
            </li>
          </ul>
        </section>
      </template>
    </div>
  </div>
</template>

<style scoped>
.agents {
  display: flex;
  flex-direction: column;
  height: 100%;
  color: var(--text-primary);
}
.ag-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
}
.vibrancy .ag-header {
  padding-top: 30px;
}
.ag-title {
  font-size: 14px;
  font-weight: 600;
  white-space: nowrap;
}
.seg {
  display: inline-flex;
  padding: 2px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--text-primary) 8%, transparent);
}
.seg-btn {
  appearance: none;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 500;
  padding: 3px 12px;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
  white-space: nowrap;
}
.seg-btn:hover {
  color: var(--text-primary);
}
.seg-btn.active {
  background: var(--bg-elevated);
  color: var(--text-primary);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
}
.ag-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px 18px;
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
  animation: ag-spin 0.7s linear infinite;
}
@keyframes ag-spin {
  to {
    transform: rotate(360deg);
  }
}
.group {
  margin-bottom: 18px;
}
.group-title {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  margin: 0 0 8px;
  padding: 2px 4px;
  border: none;
  background: transparent;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-secondary);
  cursor: pointer;
  user-select: none;
  transition: background 0.12s ease;
}
.group-title:hover {
  background: color-mix(in srgb, var(--text-primary) 6%, transparent);
}
.group-title.accent {
  color: #248a3d;
}
.group-label {
  flex: 0 0 auto;
}
.chevron {
  flex: 0 0 auto;
  width: 12px;
  height: 12px;
  color: var(--text-secondary);
  transition: transform 0.15s ease;
  transform: rotate(90deg);
}
.group-title.collapsed .chevron {
  transform: rotate(0deg);
}
.group-title.accent .chevron {
  color: #248a3d;
}
.group-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-primary) 10%, transparent);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
}
.group-title.accent .group-count {
  background: color-mix(in srgb, #30d158 20%, transparent);
  color: #248a3d;
}
.card-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.card {
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm, 8px);
  background: var(--bg-elevated);
}
.card.ended {
  opacity: 0.6;
}
.card-top {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}
.dot {
  flex: 0 0 auto;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-secondary);
}
.dot.working {
  background: #30d158;
  box-shadow: 0 0 0 3px color-mix(in srgb, #30d158 22%, transparent);
}
.dot.idle {
  background: #ff9f0a;
}
.dot.ended {
  background: var(--text-secondary);
}
.kind-badge {
  flex: 0 0 auto;
  padding: 1px 7px;
  border-radius: 5px;
  font-size: 10px;
  font-weight: 600;
  background: color-mix(in srgb, var(--text-primary) 9%, transparent);
  color: var(--text-secondary);
  white-space: nowrap;
}
.card-title {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.status-badge {
  flex: 0 0 auto;
  padding: 1px 8px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
}
.status-badge.working {
  background: color-mix(in srgb, #30d158 18%, transparent);
  color: #248a3d;
}
.status-badge.idle {
  background: color-mix(in srgb, #ff9f0a 18%, transparent);
  color: #c77700;
}
.status-badge.ended {
  background: color-mix(in srgb, var(--text-primary) 10%, transparent);
  color: var(--text-secondary);
}
.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  font-size: 11px;
  margin-top: 3px;
}
.meta.times {
  color: var(--text-secondary);
}
.meta-item {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  min-width: 0;
}
.meta-item.full {
  width: 100%;
}
.meta-k {
  flex: 0 0 auto;
  color: var(--text-secondary);
}
.meta-v {
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.meta-v.sid {
  white-space: normal;
  overflow: visible;
  word-break: break-all;
}
.mono {
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-variant-numeric: tabular-nums;
}
</style>
