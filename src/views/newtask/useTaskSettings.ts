import { computed, ref, watch, type Ref } from "vue";
import { newTaskOptions } from "../../lib/ipc";
import type { AgentKind, NewTaskOptions } from "../../lib/types";

const preferenceKey = (kind: string) => `askhuman.task.model-choice.${kind}`;
interface Preference { model: string | null; effort: string | null }

/** Desktop equivalent of iOS TaskComposer's model preferences and per-form speed / Max choices. */
export function useTaskSettings(kind: Ref<AgentKind | null>) {
  const options = ref<NewTaskOptions | null>(null);
  const loading = ref(false);
  const failed = ref(false);
  const modelId = ref<string | null>(null);
  const effortId = ref<string | null>(null);
  const fast = ref(false);
  const fastWasReset = ref(false);
  const allowMax = ref(false);
  let sequence = 0;
  let needsRestore = true;
  let preserveUnavailablePreference = false;

  const model = computed(() => options.value?.models.find(m => m.id === modelId.value));
  const effortChoices = computed(() => (model.value ?? options.value?.default)?.efforts ?? []);
  const defaultEffort = computed(() => (model.value ?? options.value?.default)?.defaultEffort);
  const offersModels = computed(() => !!options.value?.models.length);
  const speedControl = computed(() => kind.value === "codex" && options.value?.speedControl === true);
  const supportsFast = computed(() => speedControl.value && model.value?.fastSupported === true);
  const maxControl = computed(() => kind.value === "cursor" && options.value?.maxControl === true);
  const maxRequired = computed(() => maxControl.value ? (model.value ?? options.value?.default)?.maxRequired : null);
  const maxBlocked = computed(() => maxControl.value && !allowMax.value && maxRequired.value === true);
  const speed = computed(() => speedControl.value ? (fast.value && supportsFast.value ? "fast" : "standard") : undefined);

  function prune() {
    if (modelId.value && !model.value) {
      modelId.value = null;
      effortId.value = null;
      preserveUnavailablePreference = true;
    }
    if (effortId.value && !effortChoices.value.includes(effortId.value)) effortId.value = null;
    if (fast.value && !supportsFast.value) {
      fast.value = false;
      fastWasReset.value = true;
    }
    if (!maxControl.value) allowMax.value = false;
  }

  function restore() {
    if (!kind.value || !needsRestore) return;
    needsRestore = false;
    preserveUnavailablePreference = false;
    try {
      const saved = JSON.parse(localStorage.getItem(preferenceKey(kind.value)) ?? "null") as Preference | null;
      modelId.value = typeof saved?.model === "string" ? saved.model : null;
      effortId.value = typeof saved?.effort === "string" ? saved.effort : null;
    } catch { modelId.value = null; effortId.value = null; }
  }

  async function reload() {
    const current = ++sequence;
    const agent = kind.value;
    if (!agent) { options.value = null; loading.value = false; return; }
    loading.value = true;
    failed.value = false;
    try {
      const result = await newTaskOptions(agent);
      if (sequence !== current) return;
      options.value = result;
      restore();
      prune();
    } catch {
      if (sequence !== current) return;
      failed.value = true;
    } finally {
      if (sequence === current) loading.value = false;
    }
  }

  function selectModel(id: string | null) {
    modelId.value = id;
    preserveUnavailablePreference = false;
    prune();
  }

  function selectEffort(id: string | null) {
    effortId.value = id;
    preserveUnavailablePreference = false;
    prune();
  }

  function setFast(enabled: boolean) {
    fast.value = enabled && supportsFast.value;
    fastWasReset.value = false;
  }

  function remember() {
    if (!kind.value || !offersModels.value || preserveUnavailablePreference) return;
    try {
      localStorage.setItem(preferenceKey(kind.value), JSON.stringify({
        model: model.value?.id ?? null,
        effort: effortChoices.value.includes(effortId.value ?? "") ? effortId.value : null,
      }));
    } catch { /* A storage failure must not prevent an otherwise valid launch. */ }
  }

  function resetTask() {
    fast.value = false;
    fastWasReset.value = false;
    allowMax.value = false;
    modelId.value = null;
    effortId.value = null;
    needsRestore = true;
    preserveUnavailablePreference = false;
    if (options.value) { restore(); prune(); }
  }

  watch(kind, () => {
    options.value = null;
    resetTask();
    void reload();
  }, { flush: "sync" });

  return { options, loading, failed, modelId, effortId, model, effortChoices, defaultEffort,
    offersModels, speedControl, supportsFast, fast, fastWasReset, speed, maxControl,
    maxRequired, maxBlocked, allowMax, reload, selectModel, selectEffort, setFast, remember, resetTask };
}
