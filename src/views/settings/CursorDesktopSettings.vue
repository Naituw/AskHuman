<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { cursorDesktopStatus, type CursorDesktopStatus } from "../../lib/ipc";
import { isMac } from "../../lib/platform";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const { config, persist, modes, modeBusy } = useSettingsContext();
const status = ref<CursorDesktopStatus | null>(null);
const saving = ref(false);
const error = ref("");
const enabled = computed(() => config.value?.cursorDesktop?.suppressBackgroundCompletionFollowUp ?? false);
const phase = computed(() => !isMac ? "unsupported" : status.value?.phase ?? (enabled.value ? "attaching" : "disabled"));
const detail = computed(() => error.value || status.value?.error || status.value?.windows.find(w => w.error)?.error || "");
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

watch(() => modes.value.cursor.mode, () => { void refresh(); });
onMounted(async () => {
  window.addEventListener("focus", refresh);
  timer = setInterval(() => {
    if (enabled.value || ["detaching", "error", "offline"].includes(phase.value)) void refresh();
  }, 3000);
  unlisten = await listen("settings-updated", () => { void refresh(); });
  if (disposed) { unlisten(); return; }
  await refresh();
});
onBeforeUnmount(() => {
  disposed = true;
  ++generation;
  if (timer) clearInterval(timer);
  unlisten?.();
  window.removeEventListener("focus", refresh);
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
  <p class="card-desc agent-hint" role="status">
    {{ saving ? t("cursorDesktop.saving") : detail ? t("cursorDesktop.error") : t(`cursorDesktop.${phase}`) }}
  </p>
  <p v-if="detail" class="result err" role="alert">{{ detail }}</p>
</template>
