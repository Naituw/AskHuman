<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { codexLaunchStatus, type CodexLaunchStatus } from "../../lib/ipc";
import { codexLaunchDescription } from "../../lib/codexLaunch";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const { config, persist, modes, modeBusy } = useSettingsContext();
const status = ref<CodexLaunchStatus | null>(null);
const saving = ref(false);
const checking = ref(false);
const resolvingPreference = ref(false);
const mismatch = computed(() => !!status.value && status.value.target !== status.value.preference);
const error = ref("");
const preference = computed(() => config.value?.codexDesktop?.launchPreference ?? "desktop");
let generation = 0;
let disposed = false;
let unlisten: UnlistenFn | undefined;

async function refresh() {
  if (saving.value) return;
  const current = ++generation;
  checking.value = true;
  try {
    const result = await codexLaunchStatus(true);
    if (disposed || current !== generation) return;
    status.value = result;
    // Reconcile only this preference; other settings may contain unsaved edits.
    if (config.value?.codexDesktop) config.value.codexDesktop.launchPreference = result.preference;
    error.value = "";
  } catch (e) {
    if (!disposed && current === generation) {
      if (resolvingPreference.value) status.value = null;
      error.value = String(e);
    }
  } finally {
    if (!disposed && current === generation) {
      checking.value = false;
      resolvingPreference.value = false;
    }
  }
}
async function choose(value: "desktop" | "terminal") {
  if (saving.value || !config.value?.codexDesktop || value === preference.value) return;
  ++generation;
  checking.value = false;
  const previous = preference.value;
  saving.value = true;
  resolvingPreference.value = true;
  error.value = "";
  config.value.codexDesktop.launchPreference = value;
  try {
    await persist();
  } catch (e) {
    config.value.codexDesktop.launchPreference = previous;
    error.value = String(e);
    saving.value = false;
    resolvingPreference.value = false;
    return;
  }
  saving.value = false;
  await refresh();
}
watch(() => modes.value.codex.mode, refresh);
onMounted(async () => {
  window.addEventListener("focus", refresh);
  unlisten = await listen("settings-updated", refresh);
  if (disposed) { unlisten(); return; }
  await refresh();
});
onBeforeUnmount(() => {
  disposed = true;
  ++generation;
  unlisten?.();
  window.removeEventListener("focus", refresh);
});
</script>

<template>
  <hr class="divider" />
  <div class="row agent-row">
    <span class="label">{{ t("desktop.preference") }}</span>
    <span class="spacer" />
    <div class="segmented" :aria-label="t('desktop.preference')">
      <button v-for="value in (['desktop', 'terminal'] as const)" :key="value"
        type="button" class="seg" :class="{ active: preference === value }"
        :aria-pressed="preference === value" :disabled="saving || modeBusy.codex"
        @click="choose(value)">{{ t(value === 'desktop' ? 'desktop.preferApp' : 'desktop.preferCli') }}</button>
    </div>
  </div>
  <div class="row launch-result">
    <p class="card-desc agent-hint" role="status">
      {{ saving || resolvingPreference || (!status && checking) ? t('desktop.checking') : status ? codexLaunchDescription(status, t) : error ? t('desktop.detectionFailed') : t('desktop.checking') }}
    </p>
    <button v-if="mismatch && !resolvingPreference" type="button" class="btn refresh-runtime"
      :disabled="checking || saving" @click="refresh">{{ t(checking ? 'desktop.checking' : 'desktop.refresh') }}</button>
  </div>
  <p v-if="error" class="result err" role="alert">{{ error }}</p>
</template>

<style scoped>
.launch-result { align-items: center; gap: 12px; margin-top: 4px; }
.launch-result .card-desc { flex: 1; min-width: 0; margin: 0; }
.refresh-runtime { flex: none; }
</style>
