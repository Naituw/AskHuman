<script setup lang="ts">
import { computed } from "vue";
import PopupView from "./views/PopupView.vue";
import SettingsView from "./views/SettingsView.vue";
import HistoryView from "./views/HistoryView.vue";
import AgentsView from "./views/AgentsView.vue";

// 视图模式由 Rust 侧通过窗口 URL 的查询参数注入：?view=popup | settings | history | agents
const view = computed(() => {
  const params = new URLSearchParams(window.location.search);
  return params.get("view") ?? "popup";
});
</script>

<template>
  <SettingsView v-if="view === 'settings'" />
  <HistoryView v-else-if="view === 'history'" />
  <AgentsView v-else-if="view === 'agents'" />
  <PopupView v-else />
</template>
