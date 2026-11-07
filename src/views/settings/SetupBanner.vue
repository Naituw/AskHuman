<script setup lang="ts">
// Settings banner while the Agent question of the setup wizard is open (docs/specs/onboarding.md
// D2, D15, D17): "Open wizard" starts it, the close button counts as skipping the Agent step.
import { useI18n } from "vue-i18n";
import { openSetup } from "../../lib/ipc";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const { setupState, dismissSetupBanner } = useSettingsContext();
</script>

<template>
  <div v-if="setupState && !setupState.agentsDecided" class="setup-banner" data-testid="setup-banner">
    <span class="setup-banner-text">{{ t("settings.setup.bannerText") }}</span>
    <button
      class="btn btn-primary"
      type="button"
      data-testid="setup-banner-open"
      @click="openSetup('agents')"
    >
      {{ t("settings.setup.bannerOpen") }}
    </button>
    <button
      class="setup-banner-close"
      type="button"
      :title="t('settings.setup.bannerDismiss')"
      :aria-label="t('settings.setup.bannerDismiss')"
      data-testid="setup-banner-dismiss"
      @click="dismissSetupBanner"
    >
      ✕
    </button>
  </div>
</template>
