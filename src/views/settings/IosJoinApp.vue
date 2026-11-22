<script setup lang="ts">
// Step 2: getting the AskHuman app onto the phone through TestFlight's public link
// (docs/specs/ios-setup-experience.md §5). The QR code and the link point at the landing page,
// which holds the current invitation link, so this address never has to change with it.
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { iosPairQr, openPath } from "../../lib/ipc";
import IosRich from "./IosRich.vue";
import { IOS_JOIN_URL } from "./iosLinks";

const emit = defineEmits<{ done: [] }>();

const { t } = useI18n();

// The code never changes, so it is drawn once per window.
let cachedQr: string | null = null;
const qrSvg = ref(cachedQr ?? "");
let disposed = false;
onMounted(async () => {
  if (cachedQr) return;
  try {
    const svg = await iosPairQr(IOS_JOIN_URL);
    cachedQr = svg;
    if (!disposed) qrSvg.value = svg;
  } catch {
    /* no code: the link below still gets the phone there */
  }
});

const copied = ref(false);
let timer: ReturnType<typeof setTimeout> | undefined;
async function copyLink() {
  try {
    await navigator.clipboard.writeText(IOS_JOIN_URL);
  } catch {
    /* clipboard unavailable: the QR code and the browser link still work */
  }
  copied.value = true;
  clearTimeout(timer);
  timer = setTimeout(() => (copied.value = false), 1500);
}
function openJoin() {
  void openPath(IOS_JOIN_URL);
}
onBeforeUnmount(() => {
  disposed = true;
  clearTimeout(timer);
});
</script>

<template>
  <div class="ios-join" data-testid="ios-join">
    <div v-if="qrSvg" class="ios-qr" data-testid="ios-join-qr" v-html="qrSvg"></div>
    <div class="ios-side">
      <ol class="ios-list">
        <li><IosRich :text="t('settings.channels.ios.app.install')" /></li>
        <li>{{ t("settings.channels.ios.app.join") }}</li>
        <li>{{ t("settings.channels.ios.app.open") }}</li>
      </ol>
      <p class="ios-subdesc">{{ t("settings.channels.ios.app.requires") }}</p>
      <div class="ios-links">
        <button class="btn" type="button" data-testid="ios-join-copy" @click="copyLink">
          {{ copied ? t("settings.channels.ios.app.linkCopied") : t("settings.channels.ios.app.copyLink") }}
        </button>
        <button class="btn" type="button" data-testid="ios-join-open" @click="openJoin">
          {{ t("settings.channels.ios.app.openLink") }}
        </button>
        <span class="spacer"></span>
        <button class="btn btn-primary" type="button" data-testid="ios-join-done" @click="emit('done')">
          {{ t("settings.channels.ios.app.done") }}
        </button>
      </div>
    </div>
  </div>
</template>
