<script setup lang="ts">
// Prototype overlay on the Agent console baseline: global away banner, per-session glyphs in the
// sidebar and the per-session "AskHuman" pull-down in the detail header. The baseline component
// is rendered untouched; extra nodes are teleported into its DOM and ordered with CSS `order`.
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import AgentConsoleProto from "./AgentConsoleProto.vue";

const q = new URLSearchParams(location.search);
const variant = (q.get("variant") ?? "banner") as "banner" | "menu";
const globalAway = variant === "banner";
const menuOpen = variant === "menu";

const consoleEl = ref<Element | null>(null);
const metaRow = ref<Element | null>(null);
const sessRows = ref<Element[]>([]);

function probe() {
  const c = document.querySelector(".console");
  if (c !== consoleEl.value) consoleEl.value = c;
  const m = document.querySelector(".dt-meta");
  if (m !== metaRow.value) metaRow.value = m;
  const rows = Array.from(document.querySelectorAll(".sess"));
  if (rows.length !== sessRows.value.length || rows.some((r, i) => r !== sessRows.value[i])) {
    sessRows.value = rows;
  }
}

let mo: MutationObserver | null = null;
onMounted(async () => {
  probe();
  mo = new MutationObserver(probe);
  mo.observe(document.body, { childList: true, subtree: true });
  // The baseline follows the OS scheme; default these renders to light unless asked otherwise.
  await nextTick();
  if (q.get("theme") !== "dark") {
    document.querySelector<HTMLButtonElement>('button[title="切浅色"]')?.click();
  }
});
onBeforeUnmount(() => mo?.disconnect());

// Row indexes in the baseline sidebar: 0 selected session, 4 the idle WebApp session.
const glyphFor = (i: number): "moon" | "bell-off" | null => {
  if (i === 4) return "bell-off";
  if (i === 0 && !globalAway) return "moon";
  return null;
};
</script>

<template>
  <AgentConsoleProto />

  <!-- Global away banner (sits between the top bar and the two panes) -->
  <Teleport v-if="consoleEl && globalAway" :to="consoleEl">
    <div class="ca-banner">
      <svg class="ca-banner-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg>
      <span class="ca-banner-title">离开模式开启中</span>
      <span class="ca-banner-desc">所有 Agent · 已 1 小时 23 分 · 自动回复 6 次</span>
      <span class="ca-spacer"></span>
      <button class="ca-banner-btn" type="button">查看记录</button>
      <button class="ca-banner-btn strong" type="button">结束离开模式</button>
    </div>
  </Teleport>

  <!-- Per-session state glyph in the sidebar -->
  <template v-for="(row, i) in sessRows" :key="i">
    <Teleport v-if="glyphFor(i)" :to="row">
      <span class="ca-glyph" :class="glyphFor(i)">
        <svg v-if="glyphFor(i) === 'moon'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg>
        <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5" /><path d="M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7" /><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" /><path d="m2 2 20 20" /></svg>
      </span>
    </Teleport>
  </template>

  <!-- Per-session pull-down in the detail header meta row -->
  <Teleport v-if="metaRow" :to="metaRow">
    <div class="ca-ctl-wrap">
      <button class="ca-ctl" :class="globalAway ? 'inherit' : 'away'" type="button">
        <span class="ca-ctl-key">AskHuman</span>
        <span class="ca-ctl-val">{{ globalAway ? "跟随全局" : "离开中" }}</span>
        <svg class="ca-ctl-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6" /></svg>
      </button>
      <div v-if="menuOpen" class="ca-pop">
        <div class="ca-pop-item">
          <span class="ca-pop-check"></span>
          <span class="ca-pop-label">正常</span>
        </div>
        <div class="ca-pop-item on">
          <span class="ca-pop-check"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5" /></svg></span>
          <span class="ca-pop-label">离开模式（此会话）</span>
        </div>
        <div class="ca-pop-item">
          <span class="ca-pop-check"></span>
          <span class="ca-pop-label">不再使用 AskHuman（此会话）</span>
        </div>
      </div>
    </div>
  </Teleport>
</template>
