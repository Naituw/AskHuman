<script setup lang="ts">
// The Antigravity card's task-creation part (docs/specs/antigravity-deep-integration.md §12.10):
// the one-time allowance for AskHuman to manage sidecars, which route a new task tries first, and
// the sidecars AskHuman keeps for the App (remove them, edit what a project's sidecar may do).
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import {
  antigravityDesktop,
  antigravityLaunchStatus,
  type AntigravityLaunchStatus,
  type AntigravityRefusal,
  type AntigravitySidecar,
  type AntigravitySidecarList,
} from "../../lib/ipc";
import { antigravityLaunchDescription } from "../../lib/antigravityLaunch";
import { isMac } from "../../lib/platform";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const { config, persist, modeBusy } = useSettingsContext();

const managing = computed(() => config.value?.antigravityDesktop?.manageSidecars ?? false);
const launchPreference = computed(() => config.value?.antigravityDesktop?.launchPreference ?? "desktop");

let disposed = false;
let unlisten: UnlistenFn | undefined;

// ===== The allowance =====
const manageSaving = ref(false);
const manageError = ref("");

async function chooseManaging(value: boolean) {
  if (!config.value || manageSaving.value || !isMac) return;
  const previous = managing.value;
  config.value.antigravityDesktop ??= {};
  config.value.antigravityDesktop.manageSidecars = value;
  manageSaving.value = true;
  manageError.value = "";
  try {
    await persist();
  } catch (e) {
    config.value.antigravityDesktop.manageSidecars = previous;
    manageError.value = String(e);
    manageSaving.value = false;
    return;
  }
  manageSaving.value = false;
  await Promise.all([refreshLaunch(), refreshList()]);
}

// ===== Which route a new task tries first =====
const launchStatus = ref<AntigravityLaunchStatus | null>(null);
const launchSaving = ref(false);
const launchChecking = ref(false);
const launchResolving = ref(false);
const launchError = ref("");
const launchMismatch = computed(() => !!launchStatus.value && launchStatus.value.target !== launchStatus.value.preference);
let launchGeneration = 0;

async function refreshLaunch() {
  if (disposed || launchSaving.value) return;
  const current = ++launchGeneration;
  launchChecking.value = true;
  try {
    const result = await antigravityLaunchStatus(true);
    if (disposed || current !== launchGeneration) return;
    launchStatus.value = result;
    // Reconcile only this preference; other settings may contain unsaved edits.
    if (config.value?.antigravityDesktop) config.value.antigravityDesktop.launchPreference = result.preference;
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
  config.value.antigravityDesktop ??= {};
  config.value.antigravityDesktop.launchPreference = value;
  try {
    await persist();
  } catch (e) {
    config.value.antigravityDesktop.launchPreference = previous;
    launchError.value = String(e);
    launchSaving.value = false;
    launchResolving.value = false;
    return;
  }
  launchSaving.value = false;
  await refreshLaunch();
}

// ===== The sidecars =====
const list = ref<AntigravitySidecarList | null>(null);
const listLoading = ref(false);
const listError = ref("");
let listGeneration = 0;

async function refreshList() {
  if (disposed || !isMac) return;
  const current = ++listGeneration;
  listLoading.value = true;
  try {
    const answer = await antigravityDesktop<AntigravitySidecarList>({ op: "sidecars" });
    if (disposed || current !== listGeneration) return;
    list.value = answer;
    listError.value = "";
  } catch (e) {
    if (!disposed && current === listGeneration) listError.value = String(e);
  } finally {
    if (!disposed && current === listGeneration) listLoading.value = false;
  }
}

const sidecars = computed<AntigravitySidecar[]>(() => list.value?.sidecars ?? []);
const actionError = ref("");
const busyId = ref<string | null>(null);

/** A refusal on purpose arrives as an answer; say its reason instead of a failure. */
function refusalOf(answer: unknown): string | null {
  const refusal = answer as Partial<AntigravityRefusal> | null;
  return refusal?.status === "refused" ? refusal.message ?? refusal.reason ?? "refused" : null;
}

async function remove(id: string) {
  if (busyId.value) return;
  busyId.value = id;
  actionError.value = "";
  try {
    const answer = await antigravityDesktop({ op: "remove", id });
    actionError.value = refusalOf(answer) ?? "";
    if (editingId.value === id) editingId.value = null;
  } catch (e) {
    actionError.value = String(e);
  } finally {
    busyId.value = null;
  }
  await refreshList();
}

const confirmingRemoveAll = ref(false);
async function removeAll() {
  if (busyId.value) return;
  busyId.value = "*";
  actionError.value = "";
  confirmingRemoveAll.value = false;
  try {
    const answer = await antigravityDesktop<{ failed?: { id: string; message: string }[] }>({ op: "removeAll" });
    const failed = answer.failed ?? [];
    if (failed.length) actionError.value = failed.map((item) => `${item.id}: ${item.message}`).join("\n");
    editingId.value = null;
  } catch (e) {
    actionError.value = String(e);
  } finally {
    busyId.value = null;
  }
  await refreshList();
}

// ===== Per-folder rules =====
/** The sidecar whose rules are being edited, and the text of the edit (one rule per line). */
const editingId = ref<string | null>(null);
const rulesDraft = ref("");
const rulesSaving = ref(false);
const rulesError = ref("");

function toggleRules(item: AntigravitySidecar) {
  if (editingId.value === item.id) {
    editingId.value = null;
    return;
  }
  editingId.value = item.id;
  rulesDraft.value = item.rules.join("\n");
  rulesError.value = "";
}

async function saveRules() {
  const id = editingId.value;
  if (!id || rulesSaving.value) return;
  const rules = rulesDraft.value.split("\n").map((line) => line.trim()).filter(Boolean);
  rulesSaving.value = true;
  rulesError.value = "";
  try {
    const answer = await antigravityDesktop({ op: "setRules", id, rules });
    const refusal = refusalOf(answer);
    if (refusal) {
      rulesError.value = refusal;
      return;
    }
    editingId.value = null;
  } catch (e) {
    rulesError.value = String(e);
    return;
  } finally {
    rulesSaving.value = false;
  }
  await refreshList();
}

// ===== What a row says =====
function folderName(item: AntigravitySidecar): string {
  if (item.kind === "control") return t("antigravityDesktop.sidecars.controlName");
  const parts = item.directory.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || item.directory;
}
function lastUsed(item: AntigravitySidecar): string {
  return item.lastUsedAt
    ? t("antigravityDesktop.sidecars.lastUsed", { time: new Date(item.lastUsedAt).toLocaleString() })
    : t("antigravityDesktop.sidecars.neverUsed");
}
/** The problems a row shows besides its state, each in plain words. */
function problems(item: AntigravitySidecar): string[] {
  const found: string[] = [];
  if (item.kind !== "control" && !item.directoryExists) found.push(t("antigravityDesktop.sidecars.folderMissing"));
  if (item.files === "missing") found.push(t("antigravityDesktop.sidecars.filesMissing"));
  if (item.files === "conflict") found.push(t("antigravityDesktop.sidecars.filesForeign"));
  if (item.files === "present" && !item.enabled) found.push(t("antigravityDesktop.sidecars.notEnabled"));
  return found;
}

const hasWide = computed(() => sidecars.value.some((item) => item.kind === "yolo"));

const refreshAll = () => {
  void refreshLaunch();
  void refreshList();
};
onMounted(async () => {
  window.addEventListener("focus", refreshAll);
  unlisten = await listen("settings-updated", refreshAll);
  if (disposed) {
    unlisten();
    return;
  }
  await Promise.all([refreshLaunch(), refreshList()]);
});
onBeforeUnmount(() => {
  disposed = true;
  ++launchGeneration;
  ++listGeneration;
  unlisten?.();
  window.removeEventListener("focus", refreshAll);
});
</script>

<template>
  <hr class="divider" />
  <div class="row agent-row">
    <span class="label">{{ t("antigravityDesktop.manage.title") }}</span>
    <span class="spacer" />
    <label class="switch">
      <input type="checkbox" :checked="managing" :aria-label="t('antigravityDesktop.manage.title')"
        data-testid="antigravity-manage"
        :disabled="manageSaving || modeBusy.antigravity || !isMac"
        @change="chooseManaging(($event.target as HTMLInputElement).checked)" />
      <span class="track"></span>
    </label>
  </div>
  <p class="card-desc agent-hint">{{ t("antigravityDesktop.manage.hint") }}</p>
  <p v-if="!isMac" class="card-desc agent-hint">{{ t("antigravityDesktop.manage.macOnly") }}</p>
  <p v-if="manageError" class="result err" role="alert">{{ manageError }}</p>

  <hr class="divider" />
  <div class="row agent-row">
    <span class="label">{{ t("antigravityDesktop.launch.preference") }}</span>
    <span class="spacer" />
    <div class="segmented" :aria-label="t('antigravityDesktop.launch.preference')">
      <button v-for="value in (['desktop', 'terminal'] as const)" :key="value"
        type="button" class="seg" :class="{ active: launchPreference === value }"
        :aria-pressed="launchPreference === value"
        :disabled="launchSaving || modeBusy.antigravity || (value === 'desktop' && !isMac)"
        @click="chooseLaunch(value)">{{ t(value === 'desktop' ? 'antigravityDesktop.launch.preferApp' : 'antigravityDesktop.launch.preferCli') }}</button>
    </div>
  </div>
  <div class="row launch-result">
    <p class="card-desc agent-hint" role="status" data-testid="antigravity-launch-status">
      {{ launchSaving || launchResolving || (!launchStatus && launchChecking) ? t('desktop.checking') : launchStatus ? antigravityLaunchDescription(launchStatus, t) : launchError ? t('desktop.detectionFailed') : t('desktop.checking') }}
    </p>
    <button v-if="launchMismatch && !launchResolving" type="button" class="btn refresh-runtime"
      :disabled="launchChecking || launchSaving" @click="refreshLaunch">{{ t(launchChecking ? 'desktop.checking' : 'desktop.refresh') }}</button>
  </div>
  <p class="card-desc agent-hint">{{ t("antigravityDesktop.launch.hint") }}</p>
  <p v-if="launchError" class="result err" role="alert">{{ launchError }}</p>

  <template v-if="isMac">
    <hr class="divider" />
    <div class="row agent-row">
      <span class="label">{{ t("antigravityDesktop.sidecars.title", { n: sidecars.length, limit: list?.limit ?? 20 }) }}</span>
      <span class="spacer" />
      <button type="button" class="btn" :disabled="listLoading" @click="refreshList">{{ t("desktop.refresh") }}</button>
      <template v-if="sidecars.length">
        <button v-if="!confirmingRemoveAll" type="button" class="btn" data-testid="antigravity-remove-all"
          :disabled="!!busyId" @click="confirmingRemoveAll = true">{{ t("antigravityDesktop.sidecars.removeAll") }}</button>
        <template v-else>
          <button type="button" class="btn btn-danger" data-testid="antigravity-remove-all-confirm"
            :disabled="!!busyId" @click="removeAll">{{ t("antigravityDesktop.sidecars.removeAllConfirm", { n: sidecars.length }) }}</button>
          <button type="button" class="btn" @click="confirmingRemoveAll = false">{{ t("antigravityDesktop.sidecars.cancel") }}</button>
        </template>
      </template>
    </div>
    <p class="card-desc agent-hint">{{ t("antigravityDesktop.sidecars.hint") }}</p>
    <p v-if="hasWide" class="card-desc agent-hint warn" data-testid="antigravity-wide-note">{{ t("antigravityDesktop.sidecars.wideNote") }}</p>
    <p v-if="listError" class="result err" role="alert">{{ listError }}</p>
    <p v-else-if="list && !sidecars.length" class="card-desc agent-hint" data-testid="antigravity-no-sidecars">{{ t("antigravityDesktop.sidecars.none") }}</p>
    <ul v-if="sidecars.length" class="sidecar-list">
      <li v-for="item in sidecars" :key="item.id" class="sidecar-item" :data-testid="`antigravity-sidecar-${item.id}`">
        <div class="sidecar-head">
          <span class="sidecar-name" :title="item.directory">{{ folderName(item) }}</span>
          <span class="sidecar-kind" :class="{ wide: item.kind === 'yolo' }">{{ t(`antigravityDesktop.sidecars.kinds.${item.kind}`) }}</span>
          <span class="sidecar-state" :class="{ on: item.online }">
            <span class="dot" :class="item.online ? 'on' : 'off'"></span>{{ t(item.online ? 'antigravityDesktop.sidecars.online' : 'antigravityDesktop.sidecars.offline') }}
          </span>
          <span class="spacer" />
          <button v-if="item.kind === 'workspace'" type="button" class="btn" :disabled="!managing || !!busyId"
            :title="managing ? '' : t('antigravityDesktop.sidecars.rulesNeedAllowance')"
            @click="toggleRules(item)">{{ t("antigravityDesktop.sidecars.rules", { n: item.rules.length }) }}</button>
          <button type="button" class="btn" :disabled="!!busyId" @click="remove(item.id)">{{ t("antigravityDesktop.sidecars.remove") }}</button>
        </div>
        <p v-if="item.kind !== 'control'" class="sidecar-path" :title="item.directory">{{ item.directory }}</p>
        <p class="sidecar-meta">{{ lastUsed(item) }}<template v-for="problem in problems(item)" :key="problem"> · <span class="sidecar-problem">{{ problem }}</span></template></p>
        <div v-if="editingId === item.id" class="sidecar-rules">
          <p class="card-desc agent-hint">{{ t("antigravityDesktop.sidecars.rulesHint") }}</p>
          <textarea v-model="rulesDraft" class="sidecar-rules-input" rows="4" spellcheck="false"
            :aria-label="t('antigravityDesktop.sidecars.rulesLabel')" placeholder="read_file(~/notes/**)"></textarea>
          <p v-if="rulesError" class="result err" role="alert">{{ rulesError }}</p>
          <div class="row">
            <span class="spacer" />
            <button type="button" class="btn" :disabled="rulesSaving" @click="editingId = null">{{ t("antigravityDesktop.sidecars.cancel") }}</button>
            <button type="button" class="btn btn-primary" :disabled="rulesSaving" @click="saveRules">{{ t("antigravityDesktop.sidecars.saveRules") }}</button>
          </div>
        </div>
      </li>
    </ul>
    <p v-if="actionError" class="result err" role="alert">{{ actionError }}</p>
  </template>
</template>

<style scoped>
.launch-result { align-items: center; gap: 12px; margin-top: 4px; }
.launch-result .card-desc { flex: 1; min-width: 0; margin: 0; }
.refresh-runtime { flex: none; }
.warn { color: var(--accent-orange); }
.sidecar-list { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
.sidecar-item { border: var(--hairline) solid var(--border); border-radius: 8px; padding: 8px 10px; }
.sidecar-head { display: flex; align-items: center; gap: 8px; min-width: 0; }
.sidecar-name { font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sidecar-kind { font-size: 11px; padding: 1px 6px; border-radius: 999px; background: var(--control-bg); color: var(--text-secondary); flex: none; }
.sidecar-kind.wide { background: color-mix(in srgb, var(--accent-orange) 18%, transparent); color: var(--accent-orange); }
.sidecar-state { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; color: var(--text-secondary); flex: none; }
.sidecar-path { margin: 4px 0 0; font-size: 11px; color: var(--text-tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sidecar-meta { margin: 2px 0 0; font-size: 11px; color: var(--text-secondary); }
.sidecar-problem { color: var(--accent-orange); }
.sidecar-rules { margin-top: 8px; display: flex; flex-direction: column; gap: 6px; }
.sidecar-rules-input { width: 100%; box-sizing: border-box; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; border: var(--hairline) solid var(--border); border-radius: 7px; background: var(--control-bg); color: var(--text-primary); padding: 6px 8px; resize: vertical; }
</style>
