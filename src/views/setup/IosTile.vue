<script setup lang="ts">
// The iOS App tile of the wizard's remote page (docs/specs/setup-ios-guidance.md §3). iOS is more
// work than an IM channel (a broker account, the app from TestFlight, a pairing), so the tile lays
// the whole road out: three steps with where the person stands, what to prepare, the setup guide,
// and a prompt to hand the job to an Agent. It only reads. Everything is edited in the iOS card in
// Settings, which the main button opens (D1); what the tile shows is decided by the same facts and the
// same function as that card (`deriveIosSetup`), so the two never say different things.
import { computed, nextTick, ref } from "vue";
import { useI18n } from "vue-i18n";
import { openPath, openSettings } from "../../lib/ipc";
import IosAgentBox from "../settings/IosAgentBox.vue";
import { detectBroker } from "../settings/iosBrokers";
import { iosSetupGuideUrl } from "../settings/iosLinks";
import { deriveIosSetup, stepStatus, type IosStepNumber } from "../settings/useIosSetup";
import BrandBadge from "./BrandBadge.vue";
import { iosTileKind, iosTileStartsSetup, iosTileTone } from "./iosTile";
import { useSetupContext } from "./useSetup";

const { t, locale } = useI18n();
const { config, secretsPresent, iosGateway, iosPhones } = useSetupContext();

const ios = computed(() => config.value!.channels.ios);

const setup = computed(() =>
  deriveIosSetup({
    enabled: ios.value.enabled,
    host: ios.value.host,
    username: ios.value.username,
    // The password is typed in Settings only; here it is known only as "stored".
    passwordFilled: secretsPresent.value.iosBrokerPassword,
    gateway: iosGateway.value,
    phones: iosPhones.value.length,
    // "I've installed it" is a fact of the card's window; a paired phone is the only proof here.
    appInstalled: false,
  }),
);
const kind = computed(() => iosTileKind(setup.value));
const tone = computed(() => iosTileTone(kind.value));

const label = computed(() => {
  switch (kind.value) {
    case "off":
      return t("setup.remote.state.unconfigured");
    case "incomplete":
      return t("setup.remote.state.incomplete");
    case "stopped":
      return t("setup.remote.state.configuredOff");
    case "issue":
      return t("setup.remote.state.issue");
    case "paired":
      return t("setup.remote.iosTile.paired", { n: iosPhones.value.length }, iosPhones.value.length);
    case "await":
      return t("setup.remote.iosTile.await");
    default:
      return t("setup.remote.state.ready");
  }
});

const buttonLabel = computed(() => {
  switch (kind.value) {
    case "off":
      return t("settings.channels.ios.start");
    case "paired":
      return t("setup.remote.iosTile.manage");
    case "issue":
      return t("setup.remote.iosTile.view");
    default:
      return t("setup.remote.iosTile.continue");
  }
});

/** Nothing set up yet: the tile says what to prepare and where the guide is. */
const fresh = computed(() => kind.value === "off" || kind.value === "incomplete");
/** A finished setup is one line (the phones); the steps are for getting there. */
const showSteps = computed(() => kind.value !== "paired");

const providerName = computed(() => {
  const id = detectBroker(ios.value.host);
  if (id === "custom") return ios.value.host.trim() || t("settings.channels.ios.brokerFallback");
  return t(`settings.channels.ios.tile.${id}.name`);
});

const STEPS: { n: IosStepNumber; key: "broker" | "app" | "pair" }[] = [
  { n: 1, key: "broker" },
  { n: 2, key: "app" },
  { n: 3, key: "pair" },
];

/** A finished step says what came of it; one still to do says what it is. */
function caption(n: IosStepNumber, key: string): string {
  if (setup.value.done[n - 1]) {
    if (n === 1) return t("settings.channels.ios.stepSummary.connected", { name: providerName.value });
    if (n === 2) return t("settings.channels.ios.stepSummary.installed");
    const phones = iosPhones.value.length;
    return t("settings.channels.ios.stepSummary.phones", { n: phones }, phones);
  }
  return t(`setup.remote.iosTile.cap.${key}`);
}

const issueText = computed(() =>
  kind.value === "issue"
    ? `${t("settings.channels.ios.statusDisconnected")}: ${iosGateway.value?.lastError ?? ""}`
    : "",
);
const phoneNames = computed(() =>
  iosPhones.value.map((phone) => phone.aliasName || phone.name).join(" · "),
);

// ——— the prompt for an Agent ———

const agentOpen = ref(false);
const agentBox = ref<InstanceType<typeof IosAgentBox> | null>(null);

async function toggleAgent() {
  agentOpen.value = !agentOpen.value;
  if (!agentOpen.value) return;
  // The box opens under the steps, which in a short window is below the footer: bring it into view.
  await nextTick();
  agentBox.value?.$el.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

// ——— the buttons ———

/** The card in Settings; a card that is still off starts its setup as it opens (`#start`, D7). */
function openCard() {
  const target = "channel#channel-ios";
  void openSettings(iosTileStartsSetup(kind.value) ? `${target}#start` : target);
}

function openGuide() {
  void openPath(iosSetupGuideUrl(String(locale.value).startsWith("zh")));
}
</script>

<template>
  <div
    class="channel-tile setup-ios-tile"
    :class="tone"
    data-testid="channel-row-ios"
    :data-kind="kind"
  >
    <div class="setup-ios-head">
      <BrandBadge class="setup-ios-badge" id="ios" :label="t('setup.remote.ios')" />
      <div class="setup-ios-main">
        <!-- Name and buttons share a line while they fit; the buttons drop under the name when not. -->
        <div class="setup-ios-line">
          <span class="setup-ios-name">
            <span class="tile-name">{{ t("setup.remote.ios") }}</span>
            <span class="status-tag" :class="tone" data-testid="channel-state-ios">{{ label }}</span>
          </span>
          <span class="setup-ios-actions">
            <button
              v-if="kind !== 'paired'"
              class="btn"
              type="button"
              data-testid="ios-tile-agent"
              @click="toggleAgent"
            >
              {{ t("settings.channels.ios.agentButton") }}
            </button>
            <button class="btn" type="button" data-testid="channel-open-ios" @click="openCard">
              {{ buttonLabel }}
            </button>
          </span>
        </div>
        <span v-if="fresh" class="tile-sub" data-testid="ios-tile-needs">
          {{ t("setup.remote.needs", { items: t("setup.remote.iosTile.needsItems") }) }} ·
          <a class="link guide-link" href="#" data-testid="ios-tile-guide" @click.prevent="openGuide">
            {{ t("settings.channels.setupGuide") }} ↗
          </a>
        </span>
        <span v-else-if="kind === 'paired'" class="tile-sub" data-testid="ios-tile-phones">{{ phoneNames }}</span>
      </div>
    </div>

    <ol v-if="showSteps" class="setup-ios-steps" data-testid="ios-tile-steps">
      <li
        v-for="step in STEPS"
        :key="step.n"
        class="setup-ios-step"
        :class="stepStatus(setup, step.n)"
        :data-testid="`ios-tile-step-${step.n}`"
      >
        <span class="setup-ios-num">
          <svg v-if="setup.done[step.n - 1]" viewBox="0 0 12 12" aria-hidden="true">
            <path
              d="M2.6 6.5l2.2 2.2 4.6-5"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
          <template v-else>{{ step.n }}</template>
        </span>
        <span class="setup-ios-text">
          <span class="setup-ios-title">{{ t(`settings.channels.ios.step.${step.key}`) }}</span>
          <span class="setup-ios-cap">{{ caption(step.n, step.key) }}</span>
        </span>
      </li>
    </ol>

    <p v-if="issueText" class="tile-note warn" data-testid="ios-tile-issue">{{ issueText }}</p>
    <IosAgentBox v-if="agentOpen && kind !== 'paired'" ref="agentBox" />
  </div>
</template>
