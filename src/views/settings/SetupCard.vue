<script setup lang="ts">
// The always-present entry to the setup wizard at the top of Settings → General, for anyone who
// wants to run it again (docs/specs/onboarding.md D2). It does not depend on any state.
import { useI18n } from "vue-i18n";
import { openSetup } from "../../lib/ipc";

const { t } = useI18n();
</script>

<template>
  <div class="card setup-card" data-testid="setup-card">
    <div class="row">
      <div class="col">
        <p class="card-title">{{ t("settings.setup.cardTitle") }}</p>
        <p class="card-desc">{{ t("settings.setup.cardDesc") }}</p>
      </div>
      <span class="spacer"></span>
      <button class="btn" type="button" data-testid="setup-card-open" @click="openSetup()">
        {{ t("settings.setup.cardOpen") }}
      </button>
    </div>
  </div>
</template>
