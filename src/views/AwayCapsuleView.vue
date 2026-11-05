<script setup lang="ts">
// The desktop capsule of away mode and its card (docs/specs/special-replies.md D22, D23; picture:
// docs/assets/special-replies/mac-hud.jpg). It lives in its own small transparent window that the GUI
// Host keeps above everything else; the Host sends the state and resizes the window to the size this
// page reports.
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "vue-i18n";
import { applyLanguage } from "../i18n";
import {
  awayApply,
  capsuleDismissWelcome,
  capsuleFocusWaiting,
  capsuleInit,
  capsuleResize,
  openHistoryChannel,
} from "../lib/ipc";
import { applyTheme } from "../lib/theme";
import type { CapsuleState, ThemeMode } from "../lib/types";
import { awayDuration } from "./console/away";
import {
  WELCOME_TIMEOUT_MS,
  endSteps,
  useCapsuleExpansion,
  useWindowDrag,
} from "./awayCapsule";

const { t, locale } = useI18n();

const state = ref<CapsuleState>({ view: null, welcome: false });
const view = computed(() => state.value.view);
const welcome = computed(() => state.value.welcome);
const nowMs = ref(Date.now());
const root = ref<HTMLElement | null>(null);

const { expanded, enter, leave, toggle, collapse } = useCapsuleExpansion(welcome);
const drag = useWindowDrag(
  () => getCurrentWindow().startDragging(),
  () => toggle(),
);

function duration(sinceMs: number): string {
  const d = awayDuration(sinceMs, nowMs.value);
  if (d.kind === "now") return t("console.away.durationNow");
  if (d.kind === "m") return t("console.away.durationM", { m: d.m });
  return t("console.away.durationHM", { h: d.h, m: d.m });
}

const elapsed = computed(() => (view.value ? duration(view.value.sinceMs) : ""));

const startedAt = computed(() => {
  if (!view.value) return "";
  return new Date(view.value.sinceMs).toLocaleTimeString(locale.value === "zh" ? "zh-CN" : "en-US", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
});

const scopeText = computed(() => {
  const v = view.value;
  if (!v) return "";
  return v.scope === "all"
    ? t("capsule.scopeAll")
    : t("capsule.scopeSessions", { n: v.sessions });
});

function kindLabel(kind: string): string {
  return t(`agents.kind.${kind}`);
}

async function end(): Promise<void> {
  const v = view.value;
  if (!v) return;
  try {
    for (const step of endSteps(v)) await awayApply(step);
  } catch (err) {
    console.warn("ending away mode failed", err);
  }
  collapse();
}

function viewRecord(): void {
  openHistoryChannel("away").catch((err) => console.warn("open away record failed", err));
}

function keepAway(): void {
  capsuleDismissWelcome().catch((err) => console.warn("dismiss welcome failed", err));
  collapse();
}

function viewWaiting(): void {
  capsuleFocusWaiting().catch((err) => console.warn("focus waiting question failed", err));
}

// The welcome-back card only reminds. Left alone it goes back to being a capsule.
let welcomeTimer: ReturnType<typeof setTimeout> | null = null;
watch(welcome, (on) => {
  if (welcomeTimer) clearTimeout(welcomeTimer);
  welcomeTimer = on ? setTimeout(keepAway, WELCOME_TIMEOUT_MS) : null;
});

// The window follows the page: report the natural size whenever it changes.
let observer: ResizeObserver | null = null;
let lastSize = "";
function report(): void {
  const el = root.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const width = Math.ceil(rect.width);
  const height = Math.ceil(rect.height);
  if (width < 1 || height < 1 || `${width}x${height}` === lastSize) return;
  lastSize = `${width}x${height}`;
  capsuleResize(width, height).catch(() => {});
}
watch(root, (el) => {
  observer?.disconnect();
  if (el && typeof ResizeObserver !== "undefined") {
    observer = new ResizeObserver(report);
    observer.observe(el);
  }
  void nextTick(report);
});

let unlistenState: UnlistenFn | null = null;
let unlistenSettings: UnlistenFn | null = null;
let ticker: ReturnType<typeof setInterval> | null = null;

onMounted(async () => {
  document.documentElement.classList.add("capsule-window");
  ticker = setInterval(() => {
    nowMs.value = Date.now();
  }, 15_000);
  unlistenState = await listen<CapsuleState>("capsule-state", (event) => {
    state.value = event.payload;
    nowMs.value = Date.now();
  });
  unlistenSettings = await listen<{ theme?: ThemeMode; language?: string }>(
    "settings-updated",
    (event) => {
      if (typeof event.payload.theme === "string") applyTheme(event.payload.theme);
      if (typeof event.payload.language === "string") applyLanguage(event.payload.language);
    },
  );
  try {
    const init = await capsuleInit();
    applyTheme(init.theme);
    applyLanguage(init.lang);
    state.value = init.state;
    nowMs.value = Date.now();
  } catch (err) {
    console.warn("capsule init failed", err);
  }
});

onBeforeUnmount(() => {
  unlistenState?.();
  unlistenSettings?.();
  observer?.disconnect();
  if (ticker) clearInterval(ticker);
  if (welcomeTimer) clearTimeout(welcomeTimer);
});
</script>

<template>
  <div
    v-if="view"
    ref="root"
    class="cap-root"
    data-testid="capsule"
    @pointerenter="enter"
    @pointerleave="leave"
  >
    <!-- Collapsed: always there while away mode is on. -->
    <div
      v-if="!expanded"
      class="cap-pill"
      role="button"
      tabindex="-1"
      data-testid="capsule-pill"
      @pointerdown="drag.down"
      @pointermove="drag.move"
      @pointerup="drag.up"
    >
      <svg class="cap-moon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg>
      <span class="cap-pill-title">{{ t("capsule.title") }}</span>
      <span class="cap-pill-time" data-testid="capsule-elapsed">{{ view.scope === "all" ? elapsed : t("capsule.scopeSessions", { n: view.sessions }) }}</span>
      <span v-if="view.waiting > 0" class="cap-badge" data-testid="capsule-badge">{{ view.waiting }}</span>
    </div>

    <!-- Expanded: the card, or "welcome back". -->
    <div v-else class="cap-card" :class="{ welcome }" data-testid="capsule-card">
      <div
        class="cap-head"
        @pointerdown="drag.down"
        @pointermove="drag.move"
        @pointerup="drag.up"
      >
        <span class="cap-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg>
        </span>
        <div class="cap-titles">
          <div class="cap-title" data-testid="capsule-title">
            {{ welcome ? t("capsule.welcomeTitle") : t("capsule.title") }}
          </div>
          <div class="cap-sub" data-testid="capsule-sub">
            <template v-if="welcome">{{ t("capsule.welcomeSub", { d: elapsed }) }}</template>
            <template v-else>{{ scopeText }} · {{ t("capsule.startedAt", { time: startedAt }) }}</template>
          </div>
        </div>
        <span v-if="!welcome" class="cap-elapsed">{{ t("capsule.awayFor", { d: elapsed }) }}</span>
      </div>

      <div v-if="view.note" class="cap-note" data-testid="capsule-note">{{ t("capsule.note", { note: view.note }) }}</div>

      <div class="cap-sec" data-testid="capsule-replies">{{ t("capsule.replies", { n: view.replies }) }}</div>
      <ul v-if="view.rows.length > 0" class="cap-rows">
        <li v-for="(row, i) in view.rows" :key="i" class="cap-row">
          <span class="cap-row-kind">{{ kindLabel(row.kind) }}</span>
          <span class="cap-row-title">{{ row.title }}</span>
          <span class="cap-row-count">{{ t("capsule.times", { n: row.replies }) }}</span>
        </li>
      </ul>
      <div v-if="view.more > 0" class="cap-more">{{ t("capsule.more", { n: view.more }) }}</div>

      <div v-if="view.waiting > 0" class="cap-waiting" data-testid="capsule-waiting">
        <span class="cap-waiting-dot" />
        <span class="cap-waiting-text">{{ t("capsule.waiting", { n: view.waiting }) }}</span>
        <button class="cap-link" type="button" data-testid="capsule-view-waiting" @click="viewWaiting">
          {{ t("capsule.view") }}
        </button>
      </div>

      <div class="cap-actions">
        <button v-if="welcome" class="btn" type="button" data-testid="capsule-keep" @click="keepAway">
          {{ t("capsule.keepAway") }}
        </button>
        <button v-else class="btn" type="button" data-testid="capsule-record" @click="viewRecord">
          {{ t("capsule.viewRecord") }}
        </button>
        <span class="cap-spacer" />
        <button class="btn btn-primary" type="button" data-testid="capsule-end" @click="end">
          {{ t("capsule.end") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style>
/* The window is transparent: only the capsule or card itself is painted. */
html.capsule-window,
html.capsule-window body,
html.capsule-window #app {
  background: transparent;
  overflow: hidden;
}
html.capsule-window body {
  /* The page is the size of its content; never stretch it to the window. */
  height: auto;
  -webkit-user-select: none;
  user-select: none;
  cursor: default;
}
</style>

<style scoped>
.cap-root {
  width: max-content;
  font-size: 12.5px;
}
.cap-pill,
.cap-card {
  background: var(--surface-overlay, var(--bg));
  color: var(--text-primary);
  border: 0.5px solid color-mix(in srgb, var(--text-primary) 14%, transparent);
}
.cap-pill {
  height: 28px;
  padding: 0 12px 0 10px;
  border-radius: 14px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
  cursor: default;
}
.cap-moon {
  width: 14px;
  height: 14px;
  flex: none;
  color: var(--accent-orange);
}
.cap-pill-title {
  font-weight: 500;
}
.cap-pill-time {
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}
.cap-badge {
  min-width: 16px;
  height: 16px;
  margin-left: 2px;
  padding: 0 4px;
  border-radius: 8px;
  box-sizing: border-box;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--accent);
  color: #fff;
  font-size: 11px;
  font-weight: 600;
  line-height: 1;
}

.cap-card {
  width: 332px;
  padding: 14px;
  border-radius: 14px;
  box-sizing: border-box;
}
.cap-head {
  display: flex;
  align-items: center;
  gap: 10px;
  cursor: default;
}
.cap-icon {
  flex: none;
  width: 30px;
  height: 30px;
  border-radius: 9px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--accent-orange);
  background: color-mix(in srgb, var(--accent-orange) 15%, transparent);
}
.cap-icon svg {
  width: 16px;
  height: 16px;
}
.cap-titles {
  min-width: 0;
  flex: 1;
}
.cap-title {
  font-size: 14px;
  font-weight: 600;
  line-height: 1.25;
}
.cap-sub {
  margin-top: 1px;
  font-size: 12px;
  color: var(--text-secondary);
}
.cap-elapsed {
  flex: none;
  align-self: flex-start;
  font-size: 12px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  padding-top: 1px;
}
.cap-note {
  margin-top: 10px;
  font-size: 12px;
  color: var(--text-secondary);
  overflow-wrap: anywhere;
}
.cap-sec {
  margin: 14px 0 4px;
  font-size: 11.5px;
  color: var(--text-tertiary);
}
.cap-rows {
  list-style: none;
  margin: 0;
  padding: 0;
}
.cap-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 4px 0;
}
.cap-row-kind {
  flex: none;
  font-weight: 500;
}
.cap-row-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
}
.cap-row-count {
  flex: none;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}
.cap-more {
  padding: 2px 0;
  font-size: 12px;
  color: var(--text-tertiary);
}
.cap-waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
  padding: 7px 10px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--accent) 11%, transparent);
}
.cap-waiting-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--accent);
  flex: none;
}
.cap-waiting-text {
  flex: 1;
}
.cap-link {
  appearance: none;
  border: none;
  background: none;
  padding: 0;
  font: inherit;
  color: var(--accent);
  cursor: pointer;
}
.cap-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 14px;
}
.cap-spacer {
  flex: 1;
}
</style>
