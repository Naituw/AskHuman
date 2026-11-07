<script setup lang="ts">
// One optional check: a button that starts it (and cancels it while it runs) and a line that says
// how it went (docs/specs/onboarding.md D11). The backend does the work and `useProbe` follows it;
// nothing here ever blocks finishing the wizard.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { ProbeState } from "./useProbe";

const props = withDefaults(
  defineProps<{
    /** The messages' namespace under `setup.probe`. */
    kind: "popup" | "agent" | "channel";
    state: ProbeState;
    /** Values for `{agent}` / `{channel}` in the messages. */
    params?: Record<string, string>;
    /** Show the check's name above its description (the channel form has its own heading). */
    withLabel?: boolean;
  }>(),
  { params: () => ({}), withLabel: true },
);
const emit = defineEmits<{ start: []; cancel: [] }>();

const { t } = useI18n();

const message = computed(() => {
  const { phase, outcome, error } = props.state;
  if (phase === "running") return t(`setup.probe.${props.kind}.running`, props.params);
  if (phase !== "done") return "";
  if (error) return t("setup.probe.startFailed", { error });
  return t(`setup.probe.${props.kind}.${outcome ?? "failed"}`, props.params);
});

const tone = computed(() => {
  if (props.state.phase !== "done") return "";
  return props.state.outcome === "ok" ? "ok" : "err";
});

/** What to look at when no Agent asked anything. */
const CHECKLIST = ["restart", "path", "codex", "cursor", "mcp"] as const;
const showChecklist = computed(
  () =>
    props.kind === "agent" &&
    props.state.phase === "done" &&
    props.state.outcome === "timeout" &&
    !props.state.error,
);
</script>

<template>
  <div class="setup-probe" :data-testid="`probe-${kind}`">
    <div class="setup-probe-head">
      <div class="setup-probe-text">
        <p v-if="withLabel" class="setup-probe-label">{{ t(`setup.probe.${kind}.label`) }}</p>
        <p class="card-desc">{{ t(`setup.probe.${kind}.desc`) }}</p>
      </div>
      <button
        v-if="state.phase === 'running'"
        class="btn"
        type="button"
        :data-testid="`probe-${kind}-cancel`"
        @click="emit('cancel')"
      >
        {{ t("setup.probe.cancel") }}
      </button>
      <button
        v-else
        class="btn"
        type="button"
        :data-testid="`probe-${kind}-start`"
        @click="emit('start')"
      >
        {{ state.phase === "done" ? t("setup.probe.retry") : t(`setup.probe.${kind}.button`) }}
      </button>
    </div>
    <p v-if="message" class="result" :class="tone" :data-testid="`probe-${kind}-status`">
      {{ message }}
    </p>
    <ul v-if="showChecklist" class="setup-probe-checklist" :data-testid="`probe-${kind}-checklist`">
      <li v-for="item in CHECKLIST" :key="item">{{ t(`setup.probe.agent.checklist.${item}`) }}</li>
    </ul>
  </div>
</template>
