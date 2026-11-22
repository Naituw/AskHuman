<script setup lang="ts">
// Prototype of the iOS App tile on the wizard's remote page (docs/specs/setup-ios-guidance.md §3).
// What it shows is decided by the same facts and the same function as the card in Settings
// (`deriveIosSetup`); the real tile would read the facts from the saved config and `ios_op`.
// The buttons do nothing here: in the product they open the card in Settings ("channel#channel-ios").
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import BrandBadge from "../views/setup/BrandBadge.vue";
import IosAgentBox from "../views/settings/IosAgentBox.vue";
import { detectBroker } from "../views/settings/iosBrokers";
import { deriveIosSetup, stepStatus, type IosStepNumber } from "../views/settings/useIosSetup";
import type { TileFacts } from "./setupIosScenes";

const props = defineProps<{ facts: TileFacts; agentOpen?: boolean }>();

const { t } = useI18n();
const agentOpen = ref(props.agentOpen === true);

const setup = computed(() =>
  deriveIosSetup({
    enabled: props.facts.enabled,
    host: props.facts.host,
    username: props.facts.username,
    passwordFilled: props.facts.passwordFilled,
    gateway: props.facts.gateway,
    phones: props.facts.phones.length,
    appInstalled: false,
  }),
);

type Kind = "off" | "incomplete" | "stopped" | "issue" | "paired" | "await" | "configured";

// One display per situation; the card's tag is the first cut, the connection and the phones the rest.
const kind = computed<Kind>(() => {
  const s = setup.value;
  if (s.tag === "unconfigured") return "off";
  if (s.tag === "incomplete") return "incomplete";
  if (s.tag === "configuredOff") return "stopped";
  if (s.tag === "issue") return "issue";
  if (props.facts.phones.length > 0) return "paired";
  return s.connected ? "await" : "configured";
});

const TONE: Record<Kind, "ok" | "warn" | "muted"> = {
  off: "muted",
  incomplete: "warn",
  stopped: "muted",
  issue: "warn",
  paired: "ok",
  await: "warn",
  configured: "muted",
};
const tone = computed(() => TONE[kind.value]);

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
      return t("setup.remote.iosTile.paired", { n: props.facts.phones.length }, props.facts.phones.length);
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

const fresh = computed(() => kind.value === "off" || kind.value === "incomplete");
const showSteps = computed(() => kind.value !== "paired");

const providerName = computed(() => {
  const id = detectBroker(props.facts.host);
  if (id === "custom") return props.facts.host.trim() || t("settings.channels.ios.brokerFallback");
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
    const phones = props.facts.phones.length;
    return t("settings.channels.ios.stepSummary.phones", { n: phones }, phones);
  }
  return t(`setup.remote.iosTile.cap.${key}`);
}

const issueText = computed(() =>
  kind.value === "issue"
    ? `${t("settings.channels.ios.statusDisconnected")}: ${props.facts.gateway?.lastError ?? ""}`
    : "",
);
const phoneNames = computed(() => props.facts.phones.map((phone) => phone.name).join("、"));
</script>

<template>
  <div class="channel-tile iost" :class="tone" data-testid="channel-row-ios" :data-kind="kind">
    <div class="iost-head">
      <BrandBadge class="iost-badge" id="ios" :label="t('setup.remote.ios')" />
      <div class="iost-main">
        <!-- Name and buttons share a line while they fit; the buttons drop under the name when not. -->
        <div class="iost-line">
          <span class="iost-name">
            <span class="tile-name">{{ t("setup.remote.ios") }}</span>
            <span class="status-tag" :class="tone" data-testid="channel-state-ios">{{ label }}</span>
          </span>
          <span class="iost-actions">
            <button
              v-if="kind !== 'paired'"
              class="btn"
              type="button"
              data-testid="ios-tile-agent"
              @click="agentOpen = !agentOpen"
            >
              {{ t("settings.channels.ios.agentButton") }}
            </button>
            <button class="btn" type="button" data-testid="channel-open-ios">{{ buttonLabel }}</button>
          </span>
        </div>
        <span v-if="fresh" class="tile-sub iost-sub" data-testid="ios-tile-needs">
          {{ t("setup.remote.needs", { items: t("setup.remote.iosTile.needsItems") }) }} ·
          <a class="link guide-link" href="#" @click.prevent>{{ t("settings.channels.setupGuide") }} ↗</a>
        </span>
        <span v-else-if="kind === 'paired'" class="tile-sub iost-sub">{{ phoneNames }}</span>
      </div>
    </div>

    <ol v-if="showSteps" class="iost-steps" data-testid="ios-tile-steps">
      <li
        v-for="step in STEPS"
        :key="step.n"
        class="iost-step"
        :class="stepStatus(setup, step.n)"
      >
        <span class="iost-num">
          <svg v-if="setup.done[step.n - 1]" viewBox="0 0 12 12" aria-hidden="true">
            <path d="M2.6 6.5l2.2 2.2 4.6-5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          <template v-else>{{ step.n }}</template>
        </span>
        <span class="iost-text">
          <span class="iost-title">{{ t(`settings.channels.ios.step.${step.key}`) }}</span>
          <span class="iost-cap">{{ caption(step.n, step.key) }}</span>
        </span>
      </li>
    </ol>

    <p v-if="issueText" class="tile-note warn">{{ issueText }}</p>
    <IosAgentBox v-if="agentOpen && kind !== 'paired'" />
  </div>
</template>
