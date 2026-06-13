<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyTheme } from "../lib/theme";
import { applyLanguage } from "../i18n";
import { agentsInit } from "../lib/ipc";
import type { AgentKind, AgentRecord, AgentRunState } from "../lib/types";

const { t } = useI18n();

const agents = ref<AgentRecord[]>([]);
// 每秒重算一次相对时间（与数据推送解耦）。
const nowMs = ref(Date.now());

// 分组顺序（D14：按类型分组）。
const KIND_ORDER: AgentKind[] = ["claude", "codex", "cursor"];
// 状态优先级（D9：先按状态再按时间）。
const STATE_RANK: Record<AgentRunState, number> = {
  working: 0,
  idle: 1,
  ended: 2,
};

interface Group {
  kind: AgentKind;
  items: AgentRecord[];
}

// 用于排序/相对时间的「该记录的时间锚点」（秒）。
function anchor(a: AgentRecord): number {
  if (a.state === "ended") return a.endedAt ?? a.lastActivity;
  return a.lastActivity;
}

const groups = computed<Group[]>(() => {
  const byKind = new Map<AgentKind, AgentRecord[]>();
  for (const a of agents.value) {
    const arr = byKind.get(a.kind) ?? [];
    arr.push(a);
    byKind.set(a.kind, arr);
  }
  const result: Group[] = [];
  for (const kind of KIND_ORDER) {
    const items = byKind.get(kind);
    if (!items || items.length === 0) continue;
    items.sort((x, y) => {
      const r = STATE_RANK[x.state] - STATE_RANK[y.state];
      if (r !== 0) return r;
      return anchor(y) - anchor(x); // 同状态按时间倒序（新→旧）。
    });
    result.push({ kind, items });
  }
  return result;
});

const isEmpty = computed(() => agents.value.length === 0);

function kindLabel(kind: AgentKind): string {
  return t(`agents.kind.${kind}`);
}

function stateLabel(s: AgentRunState): string {
  return t(`agents.state.${s}`);
}

function shortSession(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 8)}…${id.slice(-4)}`;
}

function projectName(cwd?: string | null): string {
  if (!cwd) return "";
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || cwd;
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
  try {
    const init = await agentsInit();
    applyTheme(init.theme);
    applyLanguage(init.lang);
  } catch {
    /* 读取失败：保持兜底外观 */
  }
  unlisten = await listen<AgentRecord[]>("agents-updated", (e) => {
    agents.value = Array.isArray(e.payload) ? e.payload : [];
  });
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
    </header>

    <div class="ag-body">
      <div v-if="isEmpty" class="empty">
        <p class="empty-title">{{ t("agents.empty") }}</p>
        <p class="empty-hint">{{ t("agents.emptyHint") }}</p>
      </div>

      <template v-else>
        <section v-for="g in groups" :key="g.kind" class="group">
          <h2 class="group-title">
            {{ kindLabel(g.kind) }}
            <span class="group-count">{{ g.items.length }}</span>
          </h2>
          <ul class="card-list">
            <li
              v-for="a in g.items"
              :key="a.sessionId"
              class="card"
              :class="a.state"
            >
              <div class="card-top">
                <span class="dot" :class="a.state" />
                <span class="card-title">{{ a.title || t("agents.untitled") }}</span>
                <span class="status-badge" :class="a.state">{{ stateLabel(a.state) }}</span>
              </div>
              <div class="meta">
                <span v-if="a.cwd" class="meta-item" :title="a.cwd ?? ''">
                  <span class="meta-k">{{ t("agents.field.project") }}</span>
                  <span class="meta-v">{{ projectName(a.cwd) }}</span>
                </span>
                <span class="meta-item">
                  <span class="meta-k">{{ t("agents.field.session") }}</span>
                  <span class="meta-v mono" :title="a.sessionId">{{ shortSession(a.sessionId) }}</span>
                </span>
                <span v-if="a.pid" class="meta-item">
                  <span class="meta-k">{{ t("agents.field.pid") }}</span>
                  <span class="meta-v mono">{{ a.pid }}</span>
                </span>
              </div>
              <div class="meta times">
                <span class="meta-item">
                  <span class="meta-k">{{ t("agents.field.started") }}</span>
                  <span class="meta-v">{{ relativeTime(a.startedAt) }}</span>
                </span>
                <span class="meta-item">
                  <span class="meta-k">{{
                    a.state === "ended"
                      ? t("agents.state.ended")
                      : t("agents.field.lastActivity")
                  }}</span>
                  <span class="meta-v">{{
                    relativeTime(a.state === "ended" ? a.endedAt : a.lastActivity)
                  }}</span>
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
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
}
.vibrancy .ag-header {
  padding-top: 30px;
}
.ag-title {
  font-size: 14px;
  font-weight: 600;
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
.group {
  margin-bottom: 18px;
}
.group-title {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 0 8px;
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-secondary);
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
}
.meta.times {
  margin-top: 3px;
  color: var(--text-secondary);
}
.meta-item {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  min-width: 0;
}
.meta-k {
  color: var(--text-secondary);
}
.meta-v {
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mono {
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-variant-numeric: tabular-nums;
}
</style>
