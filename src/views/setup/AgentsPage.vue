<script setup lang="ts">
// Wizard page 1: connect the Agents and pick the collaboration style (docs/specs/onboarding.md D5-D9).
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { CollaborationStyle } from "../../lib/types";
import AgentRow from "./AgentRow.vue";
import EnvCards from "./EnvCards.vue";
import { useSetupContext } from "./useSetup";

const { t } = useI18n();
const {
  status,
  readOnly,
  rowsLoaded,
  rowsError,
  mainRows,
  otherRows,
  isRerun,
  style,
  styleBaseline,
  applying,
  applyError,
  appliedOnce,
  agentsPrimaryKind,
  applyAgents,
  skipAgents,
  scanAgents,
  goto,
} = useSetupContext();

const othersOpen = ref(false);

/** "Custom" is only an option while it is what is set: it is edited in Settings, never here. */
const styleOptions = computed<CollaborationStyle[]>(() =>
  styleBaseline.value === "custom" ? ["aligned", "autonomous", "custom"] : ["aligned", "autonomous"],
);

const primaryLabel = computed(() => {
  switch (agentsPrimaryKind.value) {
    case "apply":
      return t("setup.agents.applyContinue");
    case "continue":
      return t("setup.common.continue");
    default:
      return t("setup.common.next");
  }
});

async function onPrimary() {
  if (agentsPrimaryKind.value === "apply") await applyAgents();
  else goto("remote");
}
</script>

<template>
  <div class="settings-body setup-body">
    <h1 class="setup-title">{{ t("setup.agents.title") }}</h1>
    <p class="setup-lead" data-testid="agents-lead">
      {{ isRerun ? t("setup.agents.leadRerun") : t("setup.agents.lead") }}
    </p>

    <EnvCards />

    <p v-if="readOnly" class="setup-note" data-testid="agents-readonly">
      {{ t("setup.agents.readOnly") }}
    </p>

    <div v-if="!rowsLoaded" class="settings-section-loading" data-testid="agents-loading">
      <span class="permission-state-spinner"></span>
      <p>{{ t("setup.agents.scanning") }}</p>
    </div>
    <div v-else-if="rowsError" class="card" data-testid="agents-scan-error">
      <p class="error-text">{{ rowsError }}</p>
      <button class="btn" type="button" @click="scanAgents()">
        {{ t("settings.integration.retryStatus") }}
      </button>
    </div>
    <template v-else>
      <div v-if="mainRows.length > 0" class="card setup-agents" data-testid="agents-main">
        <AgentRow v-for="row in mainRows" :key="row.id" :row="row" />
      </div>
      <p v-else class="setup-note" data-testid="agents-none">
        {{ t("setup.agents.noneDetected") }}
      </p>

      <template v-if="otherRows.length > 0">
        <button
          class="link setup-others-toggle"
          type="button"
          data-testid="agents-others-toggle"
          @click="othersOpen = !othersOpen"
        >
          {{ othersOpen ? "▾" : "▸" }} {{ t("setup.agents.others", { n: otherRows.length }) }}
        </button>
        <div v-if="othersOpen" class="card setup-agents" data-testid="agents-others">
          <AgentRow v-for="row in otherRows" :key="row.id" :row="row" />
        </div>
      </template>

      <div class="card setup-style" data-testid="agents-style">
        <p class="card-title">{{ t("setup.agents.styleTitle") }}</p>
        <label v-for="option in styleOptions" :key="option" class="setup-radio">
          <input
            v-model="style"
            type="radio"
            name="collaboration-style"
            :value="option"
            :disabled="readOnly || applying"
            :data-testid="`style-${option}`"
          />
          <span>
            <b>{{
              option === "aligned"
                ? t("settings.integration.collabAligned")
                : option === "autonomous"
                  ? t("settings.integration.collabAutonomous")
                  : t("setup.agents.styleCustom")
            }}</b>
            <span class="setup-radio-desc">{{
              option === "aligned"
                ? t("settings.integration.collabAlignedDesc")
                : option === "autonomous"
                  ? t("settings.integration.collabAutonomousDesc")
                  : t("settings.integration.collabCustomDesc")
            }}</span>
          </span>
        </label>
      </div>
    </template>

    <p v-if="applyError" class="error-text" data-testid="agents-apply-error">{{ applyError }}</p>
    <p v-if="appliedOnce" class="setup-note" data-testid="agents-restart">
      {{ t("setup.agents.restartNote") }}
    </p>
  </div>

  <footer class="setup-footer">
    <button
      v-if="status && !status.agentsDecided"
      class="btn"
      type="button"
      data-testid="agents-skip"
      :disabled="applying"
      @click="skipAgents"
    >
      {{ t("setup.agents.skip") }}
    </button>
    <span class="spacer"></span>
    <button
      class="btn btn-primary"
      type="button"
      data-testid="agents-primary"
      :disabled="applying || !rowsLoaded"
      @click="onPrimary"
    >
      {{ applying ? t("setup.agents.applying") : primaryLabel }}
    </button>
  </footer>
</template>
