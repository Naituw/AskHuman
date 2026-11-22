<script setup lang="ts">
// Pairing a phone and the list of paired ones (docs/specs/ios-channel.md §3 / §7, wording and
// layout in docs/specs/ios-setup-experience.md §6). The same part serves step 3 of the guide and the
// "Paired phones" section of a finished setup.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { IosPhone } from "../../lib/types";
import { injectIosChannel } from "./iosChannelContext";

// `help`: step 3, with the instructions above the button. `withButton: false`: the parent has the
// "Pair a new phone" button in a header row of its own.
const props = withDefaults(defineProps<{ help?: boolean; withButton?: boolean }>(), { withButton: true });

const { t } = useI18n();
const {
  state,
  activePhones,
  pairing,
  pairBusy,
  pairNotice,
  codeCopied,
  livePairing,
  pairSecondsLeft,
  sasText,
  startPair,
  cancelPair,
  decidePair,
  copyPairCode,
  revokeTarget,
  renameTarget,
  renameText,
  beginRename,
  commitRename,
} = injectIosChannel();

// While a phone's request waits for an answer the instructions have done their job.
const showHelp = computed(
  () =>
    props.help &&
    !activePhones.value.length &&
    (!pairing.value || !livePairing.value || livePairing.value.stage === "waiting"),
);
// The code is only for scanning: once the phone has scanned it, it has nothing more to say.
const showQr = computed(() => !!pairing.value && (!livePairing.value || livePairing.value.stage === "waiting"));

const pairNoticeText = computed(() => {
  const n = pairNotice.value;
  if (!n) return "";
  if (n.kind === "done") return t("settings.channels.ios.pairDone", { name: n.name });
  if (n.kind === "ended") return t("settings.channels.ios.pairEnded");
  return n.text;
});

function phoneName(phone: IosPhone): string {
  return phone.aliasName || phone.name;
}

function lastSeenText(phone: IosPhone): string {
  if (phone.online) return t("settings.channels.ios.online");
  if (!phone.lastSeenAt) return t("settings.channels.ios.never");
  return t("settings.channels.ios.lastSeen", {
    time: new Date(phone.lastSeenAt).toLocaleString(),
  });
}

function pushWarning(phone: IosPhone): string {
  if (phone.push === "gone") return t("settings.channels.ios.pushGone");
  if (phone.push === "error") return t("settings.channels.ios.pushError");
  return "";
}
</script>

<template>
  <p v-if="showHelp" class="ios-lead" data-testid="ios-pair-help">{{ t("settings.channels.ios.pairHelp") }}</p>
  <!-- In step 3 the button gives way to the QR code (which has its own cancel) while pairing runs. -->
  <div v-if="withButton && !(help && pairing)" class="row">
    <button
      class="btn"
      :class="{ 'btn-primary': help && !activePhones.length }"
      type="button"
      :disabled="pairBusy || !!pairing || !state"
      data-testid="ios-pair"
      @click="startPair"
    >
      {{ pairBusy ? t("settings.channels.ios.pairStarting") : t("settings.channels.ios.pairButton") }}
    </button>
    <span class="spacer"></span>
  </div>

  <div v-if="pairing" class="ios-pair">
    <div v-if="showQr" class="ios-qr" v-html="pairing.qrSvg"></div>
    <div class="ios-side">
      <template v-if="!livePairing || livePairing.stage === 'waiting'">
        <p class="card-desc">{{ t("settings.channels.ios.pairScan") }}</p>
        <p class="card-desc hint">
          {{ t("settings.channels.ios.pairExpires", { seconds: pairSecondsLeft }) }}
        </p>
        <div class="row">
          <button class="btn" type="button" @click="copyPairCode">
            {{ codeCopied ? t("settings.channels.ios.pairCopied") : t("settings.channels.ios.pairCopy") }}
          </button>
          <button class="btn" type="button" @click="cancelPair">
            {{ t("settings.channels.ios.pairCancel") }}
          </button>
        </div>
      </template>
      <template v-else-if="livePairing.stage === 'pending'">
        <p class="ios-pair-title">
          {{ t("settings.channels.ios.pairConfirmTitle", { name: livePairing.phoneName || "iPhone" }) }}
        </p>
        <p class="card-desc">{{ t("settings.channels.ios.pairConfirmDesc") }}</p>
        <p class="ios-sas">{{ sasText }}</p>
        <div class="row">
          <button class="btn btn-primary" type="button" @click="decidePair(true)">
            {{ t("settings.channels.ios.pairAllow") }}
          </button>
          <button class="btn" type="button" @click="decidePair(false)">
            {{ t("settings.channels.ios.pairDeny") }}
          </button>
        </div>
      </template>
      <template v-else>
        <p class="card-desc">{{ t("settings.channels.ios.pairFinishing") }}</p>
      </template>
    </div>
  </div>
  <p v-if="pairNotice" class="result" :class="pairNotice.kind === 'error' ? 'err' : 'ok'">
    {{ pairNoticeText }}
  </p>

  <p v-if="!activePhones.length && !pairing && !help" class="ios-subdesc">
    {{ t("settings.channels.ios.noDevices") }}
  </p>
  <ul v-else-if="activePhones.length" class="ios-devices">
    <li v-for="phone in activePhones" :key="phone.phoneId" class="ios-device">
      <span class="ios-dot" :class="{ on: phone.online }"></span>
      <div class="ios-device-main">
        <template v-if="renameTarget === phone.phoneId">
          <input
            class="input"
            v-model="renameText"
            @keydown.enter.prevent="commitRename"
            @keydown.esc.prevent="renameTarget = null"
            @blur="commitRename"
          />
        </template>
        <p v-else class="ios-device-name">{{ phoneName(phone) }}</p>
        <p class="ios-device-meta">{{ lastSeenText(phone) }}</p>
        <p v-if="pushWarning(phone)" class="ios-device-meta warn">{{ pushWarning(phone) }}</p>
      </div>
      <button class="btn" type="button" @click="beginRename(phone)">
        {{ t("settings.channels.ios.rename") }}
      </button>
      <button class="btn" type="button" @click="revokeTarget = phone">
        {{ t("settings.channels.ios.revoke") }}
      </button>
    </li>
  </ul>
</template>
