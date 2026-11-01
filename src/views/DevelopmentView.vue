<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "vue-i18n";
import type { DevWorkbenchStatus } from "../lib/types";

const { t } = useI18n();
const status = ref<DevWorkbenchStatus | null>(null);
const busy = ref(false);
const error = ref("");
const notice = ref("");
const selected = ref("");
const agent = ref("codex");
const task = ref("");
const worktree = computed(() => status.value?.worktrees.find((item) => item.root === selected.value));
const support = computed(() => status.value?.agents.find((item) => item.kind === agent.value));

async function run(action: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true; error.value = ""; notice.value = "";
  try { await action(); } catch (cause) { error.value = String(cause); }
  finally { busy.value = false; }
}

async function refresh() {
  status.value = await invoke<DevWorkbenchStatus>("development_status");
  if (!status.value.worktrees.some((item) => item.root === selected.value)) selected.value = status.value.worktrees[0]?.root ?? "";
}

onMounted(() => { void run(refresh); });

function changeDelivery(event: Event) {
  const delivery = (event.target as HTMLSelectElement).value;
  void run(async () => {
    try { await invoke("development_set_delivery", { worktree: selected.value, delivery }); }
    catch (cause) { (event.target as HTMLSelectElement).value = worktree.value?.preferences.delivery ?? "local"; throw cause; }
    await refresh();
    notice.value = t("development.saved");
  });
}

function openWindow(kind: string) {
  void run(async () => { await invoke("development_open_window", { worktree: selected.value, kind }); });
}

function prepare() {
  void run(async () => {
    await invoke("development_prepare_profile", { worktree: selected.value, agent: agent.value });
    notice.value = t("development.profilePrepared");
  });
}

function start() {
  void run(async () => {
    await invoke("development_start_session", { worktree: selected.value, agent: agent.value, task: task.value.trim() || t("development.defaultTask") });
    notice.value = t("development.sessionStarted");
  });
}
</script>

<template>
  <main class="dev-workbench">
  <div class="card development-intro">
    <div class="dev-heading"><p class="card-title">{{ t("development.title") }}</p><button type="button" :disabled="busy" @click="run(refresh)">{{ t("development.refresh") }}</button></div>
    <p class="card-desc">{{ t("development.description") }}</p>
    <p v-if="status" class="card-desc">{{ t("development.systemSettingsHint") }}</p>
    <button v-if="status" type="button" :disabled="busy" @click="run(async () => { await invoke('development_open_system_settings'); })">{{ t("development.openSystem") }}</button>
  </div>

  <p v-if="busy" role="status" class="dev-feedback">{{ t("development.working") }}</p>
  <p v-if="error" role="alert" class="dev-feedback dev-error">{{ error }}</p>
  <p v-if="notice" role="status" class="dev-feedback">{{ notice }}</p>

  <div class="card">
    <p class="card-title">{{ t("development.worktree") }}</p>
    <p class="dev-root">{{ selected }}</p>
    <template v-if="worktree">
      <p v-if="worktree.error" class="dev-error" role="status">{{ worktree.error }}</p>
      <p v-if="worktree.build" class="card-desc">{{ t("development.built", { time: new Date(worktree.build.createdAtMs).toLocaleString() }) }}</p>
      <details class="dev-diagnostics"><summary>{{ t("development.diagnostics") }}</summary><dl><dt>{{ t("development.binary") }}</dt><dd>{{ worktree.binary }}</dd><dt>{{ t("development.home") }}</dt><dd>{{ worktree.home }}</dd><dt>{{ t("development.build") }}</dt><dd>{{ worktree.build?.binaryFingerprint ?? '—' }}</dd></dl></details>
      <label class="dev-label">{{ t("development.delivery") }}<select :value="worktree.preferences.delivery" :disabled="busy || !worktree.enabled" @change="changeDelivery"><option value="local">{{ t("development.local") }}</option><option value="remote">{{ t("development.remote") }}</option><option value="channels">{{ t("development.channels") }}</option></select></label>
      <p class="card-desc">{{ t("development.deliveryHint") }}</p>
      <div class="dev-actions"><button type="button" :disabled="busy || !worktree.build" @click="openWindow('settings')">{{ t("development.configureChannels") }}</button><button type="button" :disabled="busy || !worktree.build" @click="openWindow('history')">{{ t("development.history") }}</button></div>
    </template>
  </div>

  <div v-if="worktree" class="card">
    <p class="card-title">{{ t("development.testSession") }}</p>
    <p class="card-desc">{{ t("development.sessionHint") }}</p>
    <label class="dev-label">Agent<select v-model="agent" :disabled="busy"><option v-for="item in status?.agents ?? []" :key="item.kind" :value="item.kind">{{ item.kind }}</option></select></label>
    <p v-if="support?.reason" class="card-desc" role="status">{{ support.reason }}</p>
    <p v-if="support?.launchReason" class="card-desc" role="status">{{ support.launchReason }}</p>
    <label class="dev-label">{{ t("development.testTask") }}<textarea v-model="task" rows="3" maxlength="2400" :placeholder="t('development.defaultTask')" :disabled="busy" /></label>
    <p class="card-desc">{{ t("development.loginHint") }}</p>
    <div class="dev-actions"><button type="button" :disabled="busy || !worktree.build || !support?.supported" @click="prepare">{{ t("development.prepare") }}</button><button type="button" :disabled="busy || !worktree.build || !support?.supported || support.terminalAvailable === false" @click="start">{{ t("development.start") }}</button></div>
  </div>
  </main>
</template>

<style scoped>
.dev-workbench { max-width: 820px; margin: 0 auto; padding: 36px 24px 24px; height: 100vh; overflow-y: auto; box-sizing: border-box; }
.card { padding: 18px; border: 1px solid var(--border); border-radius: 12px; margin-bottom: 16px; background: var(--bg-card); }
.card-title { font-size: 15px; font-weight: 600; margin: 0 0 10px; }
.card-desc { font-size: 13px; color: var(--text-secondary); line-height: 1.5; }
.dev-root { overflow-wrap: anywhere; user-select: text; font-size: 13px; }
button { border: 1px solid var(--border); background: var(--bg-input, transparent); color: var(--text-primary); border-radius: 6px; padding: 6px 10px; font: inherit; cursor: pointer; }
button:disabled { opacity: .45; cursor: default; }
.dev-diagnostics ul { padding-inline-start: 18px; font-size: 12px; overflow-wrap: anywhere; }
.dev-heading, .dev-add, .dev-actions { display: flex; align-items: center; gap: 10px; }
.dev-heading { justify-content: space-between; }
.dev-heading .card-title { margin: 0; }
.dev-add { margin: 12px 0; }
.dev-add input { flex: 1; min-width: 0; }
.dev-label { display: grid; gap: 6px; margin: 14px 0; font-size: 13px; }
input, select, textarea { border: 1px solid var(--border); border-radius: 6px; padding: 7px 9px; background: var(--bg-input, transparent); color: var(--text-primary); font: inherit; min-width: 0; width: 100%; box-sizing: border-box; }
textarea { resize: vertical; }
.dev-actions { flex-wrap: wrap; margin-top: 12px; }
.dev-feedback { font-size: 13px; overflow-wrap: anywhere; }
.dev-error { color: var(--danger, #c43d36); font-size: 12px; overflow-wrap: anywhere; }
.dev-mode { font-size: 13px; }
.dev-diagnostics { margin: 12px 0; font-size: 12px; }
.dev-diagnostics summary { cursor: pointer; color: var(--text-secondary); }
dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 6px 10px; }
dt { color: var(--text-secondary); }
dd { margin: 0; overflow-wrap: anywhere; user-select: text; }
</style>
