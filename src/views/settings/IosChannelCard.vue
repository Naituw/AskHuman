<script setup lang="ts">
// The iOS App channel card (docs/specs/ios-setup-experience.md): one card from "turn it on" to a
// working phone. What it shows follows facts the Mac already knows (useIosSetup.ts), in three forms:
//   off     one line about the channel and the two ways to begin
//   setup   a three-step guide: connect a broker, install the app, pair a phone
//   ready   connected with a phone paired: the guide is one line plus a test question, and the
//           usual sections follow
// The state of the gateway, the pairing and the form helpers live in useIosChannel.ts; the parts
// of the card read it through iosChannelContext.ts.
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { openPath } from "../../lib/ipc";
import ChannelCardHead from "./ChannelCardHead.vue";
import AgentTasksSwitchRow from "./AgentTasksSwitchRow.vue";
import IosAgentBox from "./IosAgentBox.vue";
import IosConnectionForm from "./IosConnectionForm.vue";
import IosJoinApp from "./IosJoinApp.vue";
import IosKeepAlive from "./IosKeepAlive.vue";
import IosPairing from "./IosPairing.vue";
import IosProviderGuide from "./IosProviderGuide.vue";
import IosPushTiming from "./IosPushTiming.vue";
import IosReadyBox from "./IosReadyBox.vue";
import IosTrafficInfo from "./IosTrafficInfo.vue";
import { useSettingsContext } from "./context";
import { provideIosChannel } from "./iosChannelContext";
import { iosSetupGuideUrl } from "./iosLinks";
import { deriveIosSetup, stepStatus, type IosStepNumber } from "./useIosSetup";
import { useIosChannel } from "./useIosChannel";

const { t, locale } = useI18n();
const ctx = useSettingsContext();
const { persist } = ctx;
const channel = useIosChannel();
provideIosChannel(channel);
const {
  ios,
  provider,
  passwordFilled,
  defaultRelayUrl,
  state,
  polls,
  activePhones,
  pairBusy,
  pairing,
  startPair,
  revokeTarget,
  confirmRevoke,
} = channel;

const STEPS = [
  { n: 1, key: "broker" },
  { n: 2, key: "app" },
  { n: 3, key: "pair" },
] as const;

// ——— what the Mac knows ———

// "I've installed it" is only a fact of this window: the Mac cannot see an install, and a phone that
// is paired proves one anyway.
const appInstalled = ref(false);

const setup = computed(() =>
  deriveIosSetup({
    enabled: ios.value.enabled,
    host: ios.value.host,
    username: ios.value.username,
    passwordFilled: passwordFilled.value,
    gateway: state.value ? { connected: state.value.connected, lastError: state.value.lastError } : null,
    phones: activePhones.value.length,
    appInstalled: appInstalled.value,
  }),
);

// The broker by name; a self-hosted one by its host, which is all that tells it apart.
const providerName = computed(() => {
  const id = provider.value;
  if (!id) return t("settings.channels.ios.brokerFallback");
  if (id === "custom" && ios.value.host.trim()) return ios.value.host.trim();
  return t(`settings.channels.ios.tile.${id}.name`);
});

// The prompt for an Agent is for getting started: a finished setup has no use for it.
const showAgent = computed(() => agentOpen.value && !(ios.value.enabled && setup.value.ready));

const issueText = computed(() => {
  if (setup.value.tag !== "issue") return "";
  return `${t("settings.channels.ios.statusDisconnected")}: ${state.value?.lastError ?? ""}`;
});

// ——— the guide ———

// Which step is open: by default the first one that is not done; a click picks any step (0: none).
const open = ref<number | null>(null);
const stepsOpen = ref(false);
const agentOpen = ref(false);

// The provider's instructions fold away only when the broker already worked as the card first
// appeared, never while the person is still in step 1 (the page would move under their hands).
const foldGuide = ref(false);
watch(polls, (count) => {
  if (count === 1) foldGuide.value = setup.value.connected;
});

// A connection that comes up while the person is in step 1 does not move them on: they read the
// result and go on with "Next".
watch(
  () => setup.value.connected,
  (now, before) => {
    if (now && !before && polls.value > 1 && open.value === null) open.value = 1;
  },
);

function isOpen(n: IosStepNumber): boolean {
  return open.value === null ? n === setup.value.current : open.value === n;
}

function toggle(n: IosStepNumber) {
  open.value = isOpen(n) ? 0 : n;
}

function toggleSteps() {
  stepsOpen.value = !stepsOpen.value;
  open.value = null;
}

function summary(n: IosStepNumber): string {
  if (!setup.value.done[n - 1]) return "";
  if (n === 1) return t("settings.channels.ios.stepSummary.connected", { name: providerName.value });
  if (n === 2) return t("settings.channels.ios.stepSummary.installed");
  const phones = activePhones.value.length;
  return t("settings.channels.ios.stepSummary.phones", { n: phones }, phones);
}

function installed() {
  appInstalled.value = true;
  open.value = null;
}

// ——— the header ———

async function toggleEnabled(on: boolean) {
  ios.value.enabled = on;
  await persist();
}

function openGuide() {
  void openPath(iosSetupGuideUrl(String(locale.value).startsWith("zh")));
}

const highlighted = computed(() => ctx.settingsTargetHighlight?.value === "channel-ios");
</script>

<template>
  <div
    id="channel-ios"
    class="card channel-card ios-card"
    :class="{ 'settings-target-highlight': highlighted }"
    data-testid="ios-card"
  >
    <ChannelCardHead
      id="ios"
      :title="t('settings.channels.ios.title')"
      :enabled="ios.enabled"
      :state="setup.tag"
      guide
      @toggle="toggleEnabled"
      @guide="openGuide"
    />
    <p class="card-desc ios-desc">
      {{ t(defaultRelayUrl ? "settings.channels.ios.descPublicRelay" : "settings.channels.ios.desc") }}
    </p>
    <p v-if="issueText" class="card-desc warn ios-issue" data-testid="ios-issue">{{ issueText }}</p>

    <!-- Off: nothing to fill in yet, so only the two ways to begin. -->
    <div v-if="!ios.enabled" class="ios-actions">
      <button class="btn btn-primary" type="button" data-testid="ios-start" @click="toggleEnabled(true)">
        {{ t("settings.channels.ios.start") }}
      </button>
      <button class="btn" type="button" data-testid="ios-agent" @click="agentOpen = !agentOpen">
        {{ t("settings.channels.ios.agentButton") }}
      </button>
    </div>
    <IosAgentBox v-if="showAgent" />

    <template v-if="ios.enabled && (!setup.ready || stepsOpen)">
      <div class="ios-guide-head">
        <span class="ios-guide-title">{{ t("settings.channels.ios.stepsTitle") }}</span>
        <span class="ios-progress" data-testid="ios-progress">{{
          setup.ready ? t("settings.channels.ios.stepsDone") : `${setup.current} / ${STEPS.length}`
        }}</span>
        <span class="spacer"></span>
        <button
          v-if="!agentOpen && !setup.ready"
          class="btn"
          type="button"
          data-testid="ios-agent"
          @click="agentOpen = true"
        >
          {{ t("settings.channels.ios.agentButton") }}
        </button>
      </div>
      <ol class="ios-steps" data-testid="ios-steps">
        <li
          v-for="step in STEPS"
          :key="step.n"
          class="ios-step"
          :class="[stepStatus(setup, step.n), { open: isOpen(step.n) }]"
          :data-testid="`ios-step-${step.n}`"
        >
          <button type="button" class="ios-step-head" :aria-expanded="isOpen(step.n)" @click="toggle(step.n)">
            <span class="ios-num">
              <svg
                v-if="stepStatus(setup, step.n) === 'done'"
                viewBox="0 0 12 12"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M2.5 6.5 5 9l4.5-6" />
              </svg>
              <template v-else>{{ step.n }}</template>
            </span>
            <span class="ios-step-title">{{ t(`settings.channels.ios.step.${step.key}`) }}</span>
            <span class="ios-step-sum">{{ summary(step.n) }}</span>
            <svg
              class="ios-chevron"
              viewBox="0 0 10 10"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M1.5 3.5 5 7l3.5-3.5" />
            </svg>
          </button>

          <div v-if="isOpen(step.n)" class="ios-step-body">
            <!-- 1 · Broker -->
            <template v-if="step.n === 1">
              <IosProviderGuide :folded="foldGuide" />
              <IosConnectionForm v-if="provider" />
              <div v-if="provider" class="ios-keep">
                <IosKeepAlive />
              </div>
              <div v-if="setup.connected" class="ios-next">
                <button class="btn btn-primary" type="button" data-testid="ios-next-1" @click="open = null">
                  {{ t("settings.channels.ios.stepsNext") }}
                </button>
              </div>
            </template>

            <!-- 2 · The app, through TestFlight -->
            <IosJoinApp v-else-if="step.n === 2" @done="installed" />

            <!-- 3 · Pairing -->
            <IosPairing v-else help />
          </div>
        </li>
      </ol>
    </template>

    <!-- Ready: the guide is one box; the sections below are what the card was before, plus a few rows. -->
    <template v-if="ios.enabled && setup.ready">
      <IosReadyBox :name="providerName" :steps-open="stepsOpen" @toggle-steps="toggleSteps" />

      <hr class="divider" />
      <div class="row">
        <p class="ios-section">{{ t("settings.channels.ios.devicesTitle") }}</p>
        <span class="spacer"></span>
        <button
          class="btn"
          type="button"
          :disabled="pairBusy || !!pairing || !state"
          data-testid="ios-pair-ready"
          @click="startPair"
        >
          {{ pairBusy ? t("settings.channels.ios.pairStarting") : t("settings.channels.ios.pairButton") }}
        </button>
      </div>
      <IosPairing :with-button="false" />

      <hr class="divider" />
      <IosKeepAlive />

      <hr class="divider" />
      <IosPushTiming />

      <!-- The app's "New task" needs the Agent-task switch on this Mac: it is right here, where the phone is set up. -->
      <hr class="divider" />
      <AgentTasksSwitchRow />

      <hr class="divider" />
      <details class="ios-conn" data-testid="ios-conn">
        <summary class="ios-conn-summary">
          {{ t("settings.channels.ios.connectionTitle")
          }}<span class="ios-subdesc">{{ providerName }} · {{ ios.host }} · {{ ios.username }}</span>
        </summary>
        <div class="ios-conn-body"><IosConnectionForm show-export /></div>
      </details>

      <hr class="divider" />
      <IosTrafficInfo />
      <p v-if="provider === 'emqx'" class="ios-notice info" data-testid="ios-emqx-notice">
        {{ t("settings.channels.ios.guide.emqx.notice") }}
      </p>
    </template>
  </div>

  <div v-if="revokeTarget" class="workspace-panel-backdrop">
    <section
      class="confirm-dialog"
      role="alertdialog"
      aria-modal="true"
      :aria-label="t('settings.channels.ios.revokeTitle', { name: revokeTarget.aliasName || revokeTarget.name })"
    >
      <h2>{{ t("settings.channels.ios.revokeTitle", { name: revokeTarget.aliasName || revokeTarget.name }) }}</h2>
      <p class="confirm-note">{{ t("settings.channels.ios.revokeDesc") }}</p>
      <div class="confirm-actions">
        <button class="btn" type="button" @click="revokeTarget = null">
          {{ t("settings.channels.ios.cancel") }}
        </button>
        <button class="btn btn-primary" type="button" @click="confirmRevoke">
          {{ t("settings.channels.ios.revokeConfirm") }}
        </button>
      </div>
    </section>
  </div>
</template>
