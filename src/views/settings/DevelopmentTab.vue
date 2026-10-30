<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "vue-i18n";
import type { DevWorkbenchStatus, DevMigration } from "../../lib/types";

const { t } = useI18n();
const status = ref<DevWorkbenchStatus | null>(null);
const busy = ref(false);
const error = ref("");
const notice = ref("");
const addPath = ref("");
const selected = ref("");
const agent = ref("codex");
const task = ref("");
const migration = ref<DevMigration | null>(null);
const worktree = computed(() => status.value?.worktrees.find((item) => item.root === selected.value));
const isSystem = computed(() => status.value?.instanceTarget?.kind === "system");
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

onMounted(() => { void run(async () => {
  await refresh();
  if (isSystem.value) migration.value = await invoke<DevMigration | null>("development_migration", { action: "status", id: null });
}); });

function migrate(action: "plan" | "apply" | "rollback") {
  void run(async () => {
    try {
      migration.value = await invoke<DevMigration | null>("development_migration", { action, id: migration.value?.id ?? null });
    } catch (cause) {
      migration.value = await invoke<DevMigration | null>("development_migration", { action: "status", id: null }).catch(() => migration.value);
      throw cause;
    } finally { await refresh(); }
  });
}

function register() {
  void run(async () => {
    await invoke("development_register", { worktree: addPath.value.trim() });
    selected.value = addPath.value.trim(); addPath.value = "";
    await refresh();
  });
}

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
  <div class="card development-intro">
    <div class="dev-heading"><p class="card-title">{{ t("development.title") }}</p><button type="button" :disabled="busy" @click="run(refresh)">{{ t("development.refresh") }}</button></div>
    <p class="card-desc">{{ t("development.description") }}</p>
    <p v-if="status" class="dev-mode">{{ t(status.mode === 'explicit' ? "development.explicitMode" : "development.legacyMode") }}</p>
    <p v-if="status && !isSystem" class="card-desc">{{ t("development.systemSettingsHint") }}</p>
    <button v-if="status && !isSystem" type="button" :disabled="busy" @click="run(async () => { await invoke('development_open_system_settings'); })">{{ t("development.openSystem") }}</button>
  </div>

  <p v-if="busy" role="status" class="dev-feedback">{{ t("development.working") }}</p>
  <p v-if="error" role="alert" class="dev-feedback dev-error">{{ error }}</p>
  <p v-if="notice" role="status" class="dev-feedback">{{ notice }}</p>

  <div v-if="isSystem" class="card">
    <p class="card-title">{{ t("development.migrationTitle") }}</p>
    <p class="card-desc">{{ t("development.migrationHint") }}</p>
    <p class="card-desc">{{ t("development.migrationRestart") }}</p>
    <div class="dev-actions"><button type="button" :disabled="busy" @click="migrate('plan')">{{ t("development.migrationInspect") }}</button></div>
    <template v-if="migration">
      <p class="dev-mode">{{ t(`development.phase_${migration.phase}`) }}</p>
      <ul v-if="migration.conflicts.length" class="dev-error"><li v-for="conflict in migration.conflicts" :key="conflict">{{ conflict }}</li></ul>
      <p v-if="migration.error" class="dev-error" role="alert">{{ migration.error }}</p>
      <ul class="dev-migration-actions"><li v-for="item in migration.actions" :key="item.agent">{{ item.agent }} · {{ item.mode }} · {{ item.status }}<p v-if="item.error" class="dev-error">{{ item.error }}</p></li></ul>
      <details class="dev-diagnostics"><summary>{{ t("development.migrationFiles") }}</summary><ul><li v-for="item in migration.artifacts" :key="item.path">{{ item.path }}</li></ul></details>
      <div class="dev-actions">
        <button v-if="['planned', 'partial'].includes(migration.phase)" type="button" :disabled="busy || !!migration.conflicts.length" @click="migrate('apply')">{{ t(migration.phase === 'partial' ? "development.migrationRetry" : "development.migrationApply") }}</button>
        <button v-if="['partial', 'complete', 'rollingBack'].includes(migration.phase)" type="button" :disabled="busy" @click="migrate('rollback')">{{ t("development.migrationRollback") }}</button>
      </div>
    </template>
  </div>

  <div class="card">
    <p class="card-title">{{ t("development.worktrees") }}</p>
    <div class="dev-add"><input v-model="addPath" :aria-label="t('development.path')" :placeholder="t('development.path')" :disabled="busy" @keydown.enter="register" /><button type="button" :disabled="busy || !addPath.trim()" @click="register">{{ t("development.add") }}</button></div>
    <p v-if="status && !status.worktrees.length" class="card-desc">{{ t("development.empty") }}</p>
    <label v-if="status?.worktrees.length" class="dev-label">{{ t("development.worktree") }}<select v-model="selected" :disabled="busy"><option v-for="item in status.worktrees" :key="item.root" :value="item.root">{{ item.name }} · {{ item.root }}</option></select></label>
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
</template>

<style scoped>
button { border: 1px solid var(--border); background: var(--bg-input, transparent); color: var(--text-primary); border-radius: 6px; padding: 6px 10px; font: inherit; cursor: pointer; }
button:disabled { opacity: .45; cursor: default; }
.dev-migration-actions, .dev-diagnostics ul { padding-inline-start: 18px; font-size: 12px; overflow-wrap: anywhere; }
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
