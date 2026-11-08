<script setup lang="ts">
// Wizard page 3: what was set up, the background choices (launch at login, keep running, start
// Agent tasks from the phone or IM), and how to use it from here on
// (docs/specs/onboarding.md D8, D12, D19, D25).
import { computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { openSettings } from "../../lib/ipc";
import { isMac } from "../../lib/platform";
import BrandBadge from "./BrandBadge.vue";
import ProbeRow from "./ProbeRow.vue";
import SetupHeader from "./SetupHeader.vue";
import { isIntegrated } from "./agentRows";
import { CHANNEL_DEFS } from "./channelDefs";
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
  backgroundBChecked,
  backgroundC,
  iosEnabled,
  hasRemoteChannel,
  hasEnabledChannel,
  showBackgroundB,
  showBackgroundC,
  backgroundBLocked,
  showTasksTip,
  adoptSavedConfig,
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

const integratedRows = computed(() => rows.value.filter(isIntegrated));

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

const enabledChannels = computed(() =>
  CHANNEL_DEFS.filter((def) => enabledChannelIds.value.includes(def.id)),
);

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
  // The iOS channel is set up in a Settings window while this one stays open: read what is saved.
  void adoptSavedConfig();
  // A direct link may land here before the scan finished: the summary needs the rows.
  if (!rowsLoaded.value) await scanAgents(true);
  await loadCapabilities();
});
</script>

<template>
  <div class="settings-body setup-body">
    <div class="setup-page">
      <SetupHeader :title="t('setup.done.title')" :lead="t('setup.done.lead')">
        <template #icon>
          <span class="setup-success" aria-hidden="true">
            <svg viewBox="0 0 24 24" focusable="false"><path d="M5.5 12.8l4.2 4.2 8.8-9.4" /></svg>
          </span>
        </template>
      </SetupHeader>

      <section class="setup-panel" data-testid="done-summary">
        <ul class="check-list">
          <li class="check-item" :class="{ none: rowsLoaded && integratedRows.length === 0 }">
            <span class="check-icon" aria-hidden="true"></span>
            <span class="check-label">{{ t("setup.done.agents") }}</span>
            <span class="check-value" data-testid="summary-agents">
              <template v-if="!rowsLoaded">…</template>
              <span v-else-if="integratedRows.length === 0" class="check-none">{{
                t("setup.done.noneYet")
              }}</span>
              <template v-else>
                <span
                  v-for="row in integratedRows"
                  :key="row.id"
                  class="check-chip"
                  :data-testid="`summary-agent-${row.id}`"
                >
                  <BrandBadge :id="row.id" :label="row.title" size="sm" />
                  {{ row.title }}
                  <em>{{ row.mode === "cli" ? "CLI" : "MCP" }}</em>
                </span>
              </template>
            </span>
          </li>

          <li class="check-item">
            <span class="check-icon" aria-hidden="true"></span>
            <span class="check-label">{{ t("setup.agents.styleTitle") }}</span>
            <span class="check-value" data-testid="summary-style">{{ styleText }}</span>
          </li>

          <li class="check-item" :class="{ none: !hasRemoteChannel }">
            <span class="check-icon" aria-hidden="true"></span>
            <span class="check-label">{{ t("setup.done.channels") }}</span>
            <span class="check-value" data-testid="summary-channels">
              <span v-if="!hasRemoteChannel" class="check-none">{{
                t("setup.done.channelsLater")
              }}</span>
              <template v-else>
                <span
                  v-for="def in enabledChannels"
                  :key="def.id"
                  class="check-chip"
                  :data-testid="`summary-channel-${def.id}`"
                >
                  <BrandBadge :id="def.id" :label="t(def.titleKey)" size="sm" />
                  {{ t(def.titleKey) }}
                </span>
                <!-- The iOS app is set up in Settings, not here, but it is a remote channel all the same. -->
                <span v-if="iosEnabled" class="check-chip" data-testid="summary-channel-ios">
                  <BrandBadge id="ios" :label="t('setup.remote.ios')" size="sm" />
                  {{ t("setup.remote.ios") }}
                </span>
              </template>
            </span>
          </li>

          <li v-if="capabilities.length > 0" class="check-item" data-testid="summary-capabilities">
            <span class="check-icon" aria-hidden="true"></span>
            <span class="check-label">{{ t("setup.done.capabilitiesTitle") }}</span>
            <span class="check-value">{{ capabilitiesText }}</span>
          </li>
        </ul>
        <p v-if="capabilities.length > 0" class="check-adjust">
          <a
            class="link"
            href="#"
            data-testid="summary-adjust"
            @click.prevent="openSettings('integration')"
          >
            {{ t("setup.done.adjust") }} ↗
          </a>
        </p>
      </section>

      <!-- Optional checks: whatever they find, Done stays available (D11). -->
      <section class="setup-panel" data-testid="done-verify">
        <h2 class="setup-section-title">{{ t("setup.probe.title") }}</h2>
        <p class="setup-section-lead">{{ t("setup.probe.lead") }}</p>
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
      </section>

      <section v-if="trayAvailable || showBackgroundB" class="setup-panel" data-testid="done-background">
        <h2 class="setup-section-title">{{ t("setup.done.backgroundTitle") }}</h2>
        <div v-if="trayAvailable" class="setup-switch-row">
          <div class="setup-switch-text">
            <p class="setup-switch-label">{{ t("setup.done.switchA") }}</p>
            <p class="setup-switch-desc">{{ t("setup.done.switchADesc") }}</p>
            <p v-if="showLoginItemNote" class="setup-switch-desc" data-testid="login-item-note">
              {{ t("setup.done.loginItemNote") }}
            </p>
          </div>
          <label class="switch">
            <input v-model="backgroundA" type="checkbox" data-testid="switch-a" />
            <span class="track"></span>
          </label>
        </div>
        <!-- C comes before B: turning it on pins B, which then reads as locked right below it. -->
        <div v-if="showBackgroundC" class="setup-switch-row">
          <div class="setup-switch-text">
            <p class="setup-switch-label">{{ t("setup.done.switchC") }}</p>
            <p class="setup-switch-desc">{{ t("setup.done.switchCDesc") }}</p>
            <!-- The IM bot note only applies when an IM bot is on; a phone-only setup has none. -->
            <p v-if="hasEnabledChannel" class="setup-switch-desc" data-testid="switch-c-bot">
              {{ t("setup.done.switchCBot") }}
            </p>
          </div>
          <label class="switch">
            <input v-model="backgroundC" type="checkbox" data-testid="switch-c" />
            <span class="track"></span>
          </label>
        </div>
        <div v-if="showBackgroundB" class="setup-switch-row">
          <div class="setup-switch-text">
            <p class="setup-switch-label">{{ t("setup.done.switchB") }}</p>
            <p class="setup-switch-desc">
              {{ backgroundBLocked ? t("setup.done.switchBLocked") : t("setup.done.switchBDesc") }}
            </p>
          </div>
          <label class="switch">
            <input
              v-model="backgroundBChecked"
              type="checkbox"
              :disabled="backgroundBLocked"
              data-testid="switch-b"
            />
            <span class="track"></span>
          </label>
        </div>
      </section>

      <section class="setup-tip" data-testid="done-next">
        <p class="setup-tip-title">{{ t("setup.done.nextTitle") }}</p>
        <p class="setup-tip-body">{{ t("setup.done.nextBody") }}</p>
        <!-- Where C is not offered yet, say where to turn the feature on later. -->
        <p v-if="showTasksTip" class="setup-tip-body" data-testid="done-tasks-tip">
          {{ t("setup.done.tasksTip") }}
          <a
            class="link"
            href="#"
            data-testid="done-tasks-tip-link"
            @click.prevent="openSettings('advanced')"
          >
            {{ t("setup.done.tasksTipLink") }} ↗
          </a>
        </p>
      </section>
    </div>
  </div>

  <footer class="setup-footer">
    <p v-if="finishError" class="error-text setup-footer-error" data-testid="finish-error">
      {{ finishError }}
    </p>
    <span class="spacer"></span>
    <button
      class="btn btn-primary btn-lg"
      type="button"
      data-testid="finish"
      :disabled="finishing"
      @click="finish"
    >
      {{ t("setup.done.finish") }}
    </button>
  </footer>
</template>
