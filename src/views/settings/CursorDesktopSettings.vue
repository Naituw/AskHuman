<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { cursorDesktopStatus, cursorLaunchStatus, type CursorDesktopStatus, type CursorLaunchStatus } from "../../lib/ipc";
import { cursorLaunchDescription } from "../../lib/cursorLaunch";
import { isMac } from "../../lib/platform";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const { config, persist, modes, modeBusy } = useSettingsContext();
const status = ref<CursorDesktopStatus | null>(null);
const saving = ref(false);
const error = ref("");
const enabled = computed(() => config.value?.cursorDesktop?.suppressBackgroundCompletionFollowUp ?? false);
const phase = computed(() => !isMac ? "unsupported" : status.value?.phase ?? (enabled.value ? "attaching" : "disabled"));
const detail = computed(() => error.value || status.value?.error
  || status.value?.windows.find(w => w.error)?.error
  || status.value?.windows.find(w => w.retention?.error)?.retention?.error || "");
// The two guards are reported apart, so a partly active state can name the one that is missing.
const missing = computed(() => {
  const windows = status.value?.windows ?? [];
  const names: ("completion" | "retention")[] = [];
  if (windows.some(w => w.phase === "error")) names.push("completion");
  if (windows.some(w => w.retention?.state === "unavailable")) names.push("retention");
  return names;
});
const statusText = computed(() => {
  if (saving.value) return t("cursorDesktop.saving");
  if (phase.value === "partial") {
    // Without a named guard there is nothing truthful to say beyond "still checking".
    if (!missing.value.length) return t("cursorDesktop.attaching");
    const what = missing.value.map(name => t(`cursorDesktop.guards.${name}`)).join(t("cursorDesktop.guards.separator"));
    return t("cursorDesktop.partial", { what }, missing.value.length);
  }
  return detail.value ? t("cursorDesktop.error") : t(`cursorDesktop.${phase.value}`);
});
// Where new Cursor tasks run: the CLI in a terminal (default) or Cursor's Agents window.
const launchStatus = ref<CursorLaunchStatus | null>(null);
const launchSaving = ref(false);
const launchChecking = ref(false);
const launchResolving = ref(false);
const launchError = ref("");
const launchPreference = computed(() => config.value?.cursorDesktop?.launchPreference ?? "terminal");
const launchMismatch = computed(() => !!launchStatus.value && launchStatus.value.target !== launchStatus.value.preference);
let launchGeneration = 0;
let generation = 0, disposed = false;
let refreshing: number | undefined;
let timer: ReturnType<typeof setInterval> | undefined;
let unlisten: UnlistenFn | undefined;

async function refresh() {
  if (disposed || saving.value || refreshing !== undefined) return;
  const current = ++generation;
  refreshing = current;
  try {
    const result = await cursorDesktopStatus();
    if (disposed || current !== generation) return;
    status.value = result;
    if (config.value) {
      config.value.cursorDesktop ??= { suppressBackgroundCompletionFollowUp: false };
      config.value.cursorDesktop.suppressBackgroundCompletionFollowUp = result.preferenceEnabled;
    }
    error.value = "";
  } catch (e) {
    if (!disposed && current === generation) {
      status.value = null;
      error.value = String(e);
    }
  } finally { if (refreshing === current) refreshing = undefined; }
}

async function choose(value: boolean) {
  if (!config.value || saving.value || !isMac) return;
  ++generation;
  refreshing = undefined;
  const previous = enabled.value;
  config.value.cursorDesktop ??= { suppressBackgroundCompletionFollowUp: false };
  config.value.cursorDesktop.suppressBackgroundCompletionFollowUp = value;
  status.value = null;
  error.value = "";
  saving.value = true;
  try { await persist(); }
  catch (e) {
    config.value.cursorDesktop.suppressBackgroundCompletionFollowUp = previous;
    error.value = String(e);
    saving.value = false;
    return;
  }
  saving.value = false;
  await refresh();
}

async function refreshLaunch() {
  if (disposed || launchSaving.value) return;
  const current = ++launchGeneration;
  launchChecking.value = true;
  try {
    const result = await cursorLaunchStatus(true);
    if (disposed || current !== launchGeneration) return;
    launchStatus.value = result;
    // Reconcile only this preference; other settings may contain unsaved edits.
    if (config.value?.cursorDesktop) config.value.cursorDesktop.launchPreference = result.preference;
    launchError.value = "";
  } catch (e) {
    if (!disposed && current === launchGeneration) {
      if (launchResolving.value) launchStatus.value = null;
      launchError.value = String(e);
    }
  } finally {
    if (!disposed && current === launchGeneration) {
      launchChecking.value = false;
      launchResolving.value = false;
    }
  }
}

async function chooseLaunch(value: "terminal" | "desktop") {
  if (!config.value || launchSaving.value || value === launchPreference.value) return;
  if (value === "desktop" && !isMac) return;
  ++launchGeneration;
  launchChecking.value = false;
  const previous = launchPreference.value;
  launchSaving.value = true;
  launchResolving.value = true;
  launchError.value = "";
  config.value.cursorDesktop ??= { suppressBackgroundCompletionFollowUp: false };
  config.value.cursorDesktop.launchPreference = value;
  try { await persist(); }
  catch (e) {
    config.value.cursorDesktop.launchPreference = previous;
    launchError.value = String(e);
    launchSaving.value = false;
    launchResolving.value = false;
    return;
  }
  launchSaving.value = false;
  await refreshLaunch();
}

const refreshAll = () => { void refresh(); void refreshLaunch(); };
watch(() => modes.value.cursor.mode, refreshAll);
onMounted(async () => {
  window.addEventListener("focus", refreshAll);
  timer = setInterval(() => {
    if (enabled.value || ["detaching", "error", "offline"].includes(phase.value)) void refresh();
  }, 3000);
  unlisten = await listen("settings-updated", refreshAll);
  if (disposed) { unlisten(); return; }
  await Promise.all([refresh(), refreshLaunch()]);
});
onBeforeUnmount(() => {
  disposed = true;
  ++generation;
  ++launchGeneration;
  if (timer) clearInterval(timer);
  unlisten?.();
  window.removeEventListener("focus", refreshAll);
});
</script>

<template>
  <hr class="divider" />
  <div class="row agent-row">
    <span class="label">{{ t("cursorDesktop.title") }}</span>
    <span class="spacer" />
    <label class="switch">
      <input type="checkbox" :checked="enabled" :aria-label="t('cursorDesktop.title')"
        :disabled="saving || modeBusy.cursor || !isMac"
        @change="choose(($event.target as HTMLInputElement).checked)" />
      <span class="track"></span>
    </label>
  </div>
  <p class="card-desc agent-hint">{{ t("cursorDesktop.hint") }}</p>
  <p class="card-desc agent-hint">{{ t("cursorDesktop.experimental") }}</p>
  <p class="card-desc agent-hint" role="status">{{ statusText }}</p>
  <p v-if="detail" class="result err" role="alert">{{ detail }}</p>
  <hr class="divider" />
  <div class="row agent-row">
    <span class="label">{{ t("cursorDesktop.launch.preference") }}</span>
    <span class="spacer" />
    <div class="segmented" :aria-label="t('cursorDesktop.launch.preference')">
      <button v-for="value in (['terminal', 'desktop'] as const)" :key="value"
        type="button" class="seg" :class="{ active: launchPreference === value }"
        :aria-pressed="launchPreference === value"
        :disabled="launchSaving || modeBusy.cursor || (value === 'desktop' && !isMac)"
        @click="chooseLaunch(value)">{{ t(value === 'desktop' ? 'cursorDesktop.launch.preferWindow' : 'cursorDesktop.launch.preferCli') }}</button>
    </div>
  </div>
  <div class="row launch-result">
    <p class="card-desc agent-hint" role="status">
      {{ launchSaving || launchResolving || (!launchStatus && launchChecking) ? t('desktop.checking') : launchStatus ? cursorLaunchDescription(launchStatus, t) : launchError ? t('desktop.detectionFailed') : t('desktop.checking') }}
    </p>
    <button v-if="launchMismatch && !launchResolving" type="button" class="btn refresh-runtime"
      :disabled="launchChecking || launchSaving" @click="refreshLaunch">{{ t(launchChecking ? 'desktop.checking' : 'desktop.refresh') }}</button>
  </div>
  <p class="card-desc agent-hint">{{ t("cursorDesktop.launch.hint") }}</p>
  <p v-if="launchError" class="result err" role="alert">{{ launchError }}</p>
</template>

<style scoped>
.launch-result { align-items: center; gap: 12px; margin-top: 4px; }
.launch-result .card-desc { flex: 1; min-width: 0; margin: 0; }
.refresh-runtime { flex: none; }
</style>
