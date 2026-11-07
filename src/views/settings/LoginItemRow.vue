<script setup lang="ts">
// Menu-bar card row: "Always" promises a login item, so while this instance can write one and it
// is missing (the user set the mode in Settings, or the item was removed), say so and offer to
// enable it. It disappears by itself once the item exists; a Dev Instance never sees it
// (`loginItemAvailable` is false there). docs/specs/onboarding.md D13.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const { config, setupState, enableLoginItem, loginItemBusy, loginItemError } = useSettingsContext();

const missing = computed(
  () =>
    config.value?.general.menuBarIcon === "always" &&
    setupState.value?.loginItemAvailable === true &&
    setupState.value.loginItemInstalled === false,
);
</script>

<template>
  <template v-if="missing">
    <hr class="divider" />
    <div class="row" data-testid="login-item-row">
      <span class="label">{{ t("settings.menuBar.loginItemMissing") }}</span>
      <span class="spacer"></span>
      <button
        class="btn"
        type="button"
        :disabled="loginItemBusy"
        data-testid="login-item-enable"
        @click="enableLoginItem"
      >
        {{ t("settings.menuBar.loginItemEnable") }}
      </button>
    </div>
    <p v-if="loginItemError" class="error-text" data-testid="login-item-error">
      {{ loginItemError }}
    </p>
  </template>
</template>
