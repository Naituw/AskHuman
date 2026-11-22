<script setup lang="ts">
// Push timing (docs/specs/ios-alert-timing.md): the wait before an unseen question is pushed to
// the phone, and how long this Mac has been idle so far.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useSettingsContext } from "./context";
import { injectIosChannel } from "./iosChannelContext";
import {
  ALERT_AFTER_PRESETS,
  alertAfterChoices,
  idleDuration,
  splitDuration,
  type SplitDuration,
} from "./iosAlertTiming";

const { t } = useI18n();
const { persist } = useSettingsContext();
const { ios, state } = injectIosChannel();

function durationText(d: SplitDuration): string {
  return t(`settings.channels.ios.duration.${d.unit}`, { n: d.n });
}

const alertAfterOptions = computed(() =>
  alertAfterChoices(ios.value.alertAfterSecs).map((secs) => {
    if (secs === 0) return { secs, label: t("settings.channels.ios.alertAfterNow") };
    const text = durationText(splitDuration(secs));
    return {
      secs,
      label: ALERT_AFTER_PRESETS.includes(secs)
        ? text
        : t("settings.channels.ios.alertAfterCustom", { value: text }),
    };
  }),
);

const macIdleText = computed(() => {
  const secs = state.value?.macIdleSecs;
  if (secs === undefined || secs === null) return "";
  return t("settings.channels.ios.macIdle", { time: durationText(idleDuration(secs)) });
});
</script>

<template>
  <p class="ios-section">{{ t("settings.channels.ios.alertTimingTitle") }}</p>
  <div class="row ios-alert-after">
    <span class="label">{{ t("settings.channels.ios.alertAfterLabel") }}</span>
    <span class="spacer"></span>
    <select class="select" v-model.number="ios.alertAfterSecs" @change="persist">
      <option v-for="option in alertAfterOptions" :key="option.secs" :value="option.secs">
        {{ option.label }}
      </option>
    </select>
  </div>
  <p class="ios-subdesc">{{ t("settings.channels.ios.alertAfterHint") }}</p>
  <p v-if="macIdleText" class="ios-subdesc ios-idle">{{ macIdleText }}</p>
</template>
