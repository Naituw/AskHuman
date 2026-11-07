<script setup lang="ts">
// Wizard page 3: what was set up, the two background choices, and how to use it from here on
// (docs/specs/onboarding.md D8, D12, D19).
import { computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { openSettings } from "../../lib/ipc";
import { isMac } from "../../lib/platform";
import { isIntegrated } from "./agentRows";
import { CHANNEL_DEFS } from "./channelDefs";
import ProbeRow from "./ProbeRow.vue";
import { useSetupContext } from "./useSetup";

const { t } = useI18n();
const ctx = useSetupContext();
const {
  status,
  rows,
  rowsLoaded,
  styleBaseline,
  enabledChannelIds,
  probes,
  agentProbeAvailable,
  capabilities,
  backgroundA,
  backgroundB,
  showBackgroundB,
  backgroundBLocked,
  finishing,
  finishError,
  scanAgents,
  refreshChannelStatus,
  loadCapabilities,
  finish,
} = ctx;

const trayAvailable = computed(() => status.value?.trayAvailable === true);
/** The login-item notice only applies where this instance really writes one. */
const showLoginItemNote = computed(() => status.value?.loginItemAvailable === true && isMac);

const agentsText = computed(() => {
  const integrated = rows.value.filter(isIntegrated);
  if (integrated.length === 0) return t("setup.done.noneYet");
  return integrated
    .map((row) => `${row.title} · ${row.mode === "cli" ? "CLI" : "MCP"}`)
    .join(" / ");
});

const styleText = computed(() => {
  switch (styleBaseline.value) {
    case "autonomous":
      return t("settings.integration.collabAutonomous");
    case "custom":
      return t("settings.integration.collabCustom");
    default:
      return t("settings.integration.collabAligned");
  }
});

const channelsText = computed(() => {
  const names = CHANNEL_DEFS.filter((def) => enabledChannelIds.value.includes(def.id)).map((def) =>
    t(def.titleKey),
  );
  return names.length > 0 ? names.join(" / ") : t("setup.done.noneYet");
});

const capabilitiesText = computed(() =>
  capabilities.value.map((key) => t(`setup.done.capabilities.${key}`)).join(" · "),
);

/** The Agent whose question the check saw, by the name the page shows elsewhere. */
const probedAgentName = computed(() => {
  const kind = probes.states.agent.agentKind;
  if (!kind) return "";
  return rows.value.find((row) => row.id === kind)?.title ?? kind;
});

onMounted(async () => {
  void refreshChannelStatus();
  // A direct link may land here before the scan finished: the summary needs the rows.
  if (!rowsLoaded.value) await scanAgents(true);
  await loadCapabilities();
});
</script>

<template>
  <div class="settings-body setup-body">
    <h1 class="setup-title">{{ t("setup.done.title") }}</h1>
    <p class="setup-lead">{{ t("setup.done.lead") }}</p>

    <div class="card" data-testid="done-summary">
      <p class="card-title">{{ t("setup.done.summaryTitle") }}</p>
      <div class="row">
        <span class="label">{{ t("setup.done.agents") }}</span>
        <span class="spacer"></span>
        <span class="setup-value" data-testid="summary-agents">{{
          rowsLoaded ? agentsText : "…"
        }}</span>
      </div>
      <div class="row">
        <span class="label">{{ t("setup.agents.styleTitle") }}</span>
        <span class="spacer"></span>
        <span class="setup-value" data-testid="summary-style">{{ styleText }}</span>
      </div>
      <div class="row">
        <span class="label">{{ t("setup.done.channels") }}</span>
        <span class="spacer"></span>
        <span class="setup-value" data-testid="summary-channels">{{ channelsText }}</span>
      </div>
      <div v-if="capabilities.length > 0" class="row" data-testid="summary-capabilities">
        <span class="label">{{ t("setup.done.capabilitiesTitle") }}</span>
        <span class="spacer"></span>
        <span class="setup-value">{{ capabilitiesText }}</span>
      </div>
      <p v-if="capabilities.length > 0" class="card-desc">
        <a
          class="link"
          href="#"
          data-testid="summary-adjust"
          @click.prevent="openSettings('integration')"
        >
          {{ t("setup.done.adjust") }} ↗
        </a>
      </p>
    </div>

    <!-- Optional checks: whatever they find, Done stays available (D11). -->
    <div class="card" data-testid="done-verify">
      <p class="card-title">{{ t("setup.probe.title") }}</p>
      <p class="card-desc">{{ t("setup.probe.lead") }}</p>
      <ProbeRow
        kind="popup"
        :state="probes.states.popup"
        @start="probes.start('popup')"
        @cancel="probes.cancel('popup')"
      />
      <ProbeRow
        v-if="agentProbeAvailable"
        kind="agent"
        :state="probes.states.agent"
        :params="{ agent: probedAgentName }"
        @start="probes.start('agent')"
        @cancel="probes.cancel('agent')"
      />
    </div>

    <div v-if="trayAvailable || showBackgroundB" class="card" data-testid="done-background">
      <p class="card-title">{{ t("setup.done.backgroundTitle") }}</p>
      <template v-if="trayAvailable">
        <div class="row">
          <span class="label">{{ t("setup.done.switchA") }}</span>
          <span class="spacer"></span>
          <label class="switch">
            <input v-model="backgroundA" type="checkbox" data-testid="switch-a" />
            <span class="track"></span>
          </label>
        </div>
        <p class="card-desc">{{ t("setup.done.switchADesc") }}</p>
        <p v-if="showLoginItemNote" class="card-desc" data-testid="login-item-note">
          {{ t("setup.done.loginItemNote") }}
        </p>
      </template>
      <template v-if="showBackgroundB">
        <div class="row">
          <span class="label">{{ t("setup.done.switchB") }}</span>
          <span class="spacer"></span>
          <label class="switch">
            <input
              v-model="backgroundB"
              type="checkbox"
              :disabled="backgroundBLocked"
              data-testid="switch-b"
            />
            <span class="track"></span>
          </label>
        </div>
        <p class="card-desc">
          {{ backgroundBLocked ? t("setup.done.switchBLocked") : t("setup.done.switchBDesc") }}
        </p>
      </template>
    </div>

    <div class="card" data-testid="done-next">
      <p class="card-title">{{ t("setup.done.nextTitle") }}</p>
      <p class="card-desc">{{ t("setup.done.nextBody") }}</p>
    </div>
  </div>

  <footer class="setup-footer">
    <p v-if="finishError" class="error-text setup-footer-error" data-testid="finish-error">
      {{ finishError }}
    </p>
    <span class="spacer"></span>
    <button
      class="btn btn-primary"
      type="button"
      data-testid="finish"
      :disabled="finishing"
      @click="finish"
    >
      {{ t("setup.done.finish") }}
    </button>
  </footer>
</template>
