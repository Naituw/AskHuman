<script setup lang="ts">
// Away mode card (general tab, docs/specs/special-replies.md D26): the desktop capsule and the
// welcome-back reminder exist on macOS only, and the wait before a question reaches the phone only
// matters when the iOS channel is visible (experimental).
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { isMac } from "../../lib/platform";
import { useSettingsContext } from "./context";
import { AWAY_PUSH_PRESETS, alertAfterChoices, splitDuration } from "./iosAlertTiming";

const { t } = useI18n();
const ctx = useSettingsContext();
const { persist } = ctx;
// The parent only renders this card once the config has loaded.
const config = computed(() => ctx.config.value!);

const visible = computed(() => isMac || config.value.experimental.enabled);
const pushOptions = computed(() =>
  alertAfterChoices(config.value.general.away.pushAfterSecs, AWAY_PUSH_PRESETS).map((secs) => {
    if (secs === 0) return { secs, label: t("settings.away.pushNow") };
    const d = splitDuration(secs);
    const text = t("settings.away.pushIdle", {
      time: t(`settings.experimental.ios.duration.${d.unit}`, { n: d.n }),
    });
    return {
      secs,
      label: AWAY_PUSH_PRESETS.includes(secs) ? text : t("settings.away.pushCustom", { value: text }),
    };
  }),
);
</script>

<template>
  <div v-if="visible" class="card" data-testid="away-card">
    <p class="card-title">{{ t("settings.away.title") }}</p>
    <template v-if="isMac">
      <div class="row">
        <span class="label">{{ t("settings.away.capsule") }}</span>
        <span class="spacer"></span>
        <label class="switch">
          <input
            type="checkbox"
            data-testid="away-capsule"
            v-model="config.general.away.capsule"
            @change="persist"
          />
          <span class="track"></span>
        </label>
      </div>
      <p class="card-desc">{{ t("settings.away.capsuleHint") }}</p>
      <hr class="divider" />
      <div class="row">
        <span class="label">{{ t("settings.away.welcomeBack") }}</span>
        <span class="spacer"></span>
        <label class="switch">
          <input
            type="checkbox"
            data-testid="away-welcome-back"
            v-model="config.general.away.welcomeBack"
            @change="persist"
          />
          <span class="track"></span>
        </label>
      </div>
      <p class="card-desc">{{ t("settings.away.welcomeBackHint") }}</p>
    </template>
    <template v-if="config.experimental.enabled">
      <hr v-if="isMac" class="divider" />
      <div class="row">
        <span class="label">{{ t("settings.away.pushAfter") }}</span>
        <span class="spacer"></span>
        <select
          class="select"
          data-testid="away-push-after"
          v-model.number="config.general.away.pushAfterSecs"
          @change="persist"
        >
          <option v-for="option in pushOptions" :key="option.secs" :value="option.secs">
            {{ option.label }}
          </option>
        </select>
      </div>
      <p class="card-desc">{{ t("settings.away.pushHint") }}</p>
    </template>
  </div>
</template>
