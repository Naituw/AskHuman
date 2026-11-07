<script setup lang="ts">
// Environment findings of the silent check, shown as red cards at the top of the Agent page.
// A clean machine gets no card and no empty space (docs/specs/onboarding.md §3.4).
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useSetupContext } from "./useSetup";

const { t } = useI18n();
const { env, showPathCard } = useSetupContext();

const copied = ref(false);

async function copyFix(command: string) {
  try {
    await navigator.clipboard.writeText(command);
  } catch {
    /* clipboard unavailable: the command stays selectable on screen */
  }
  copied.value = true;
  setTimeout(() => (copied.value = false), 1500);
}
</script>

<template>
  <div v-if="env && (showPathCard || env.transient || env.installs)" class="setup-env">
    <div v-if="showPathCard" class="setup-env-card" data-testid="env-path">
      <p class="setup-env-title">{{ t("setup.env.pathTitle") }}</p>
      <p class="setup-env-desc">{{ t("setup.env.pathDesc") }}</p>
      <template v-if="env.pathFix">
        <div class="setup-env-fix">
          <code class="setup-env-code" data-testid="env-path-command">{{ env.pathFix.command }}</code>
          <button class="btn" type="button" @click="copyFix(env.pathFix.command)">
            {{ copied ? t("common.copied") : t("setup.env.copy") }}
          </button>
        </div>
        <p v-if="env.pathFix.needsPathEdit" class="setup-env-desc" data-testid="env-path-edit">
          {{ t("setup.env.pathEdit") }}
        </p>
        <p class="setup-env-desc">{{ t("setup.env.pathReopen") }}</p>
      </template>
    </div>

    <div v-if="env.transient" class="setup-env-card" data-testid="env-transient">
      <p class="setup-env-title">{{ t("setup.env.transientTitle") }}</p>
      <p class="setup-env-desc">{{ t("setup.env.transientDesc") }}</p>
      <code class="setup-env-code">{{ env.transient.path }}</code>
    </div>

    <div v-if="env.installs" class="setup-env-card" data-testid="env-installs">
      <p class="setup-env-title">{{ t("setup.env.installsTitle") }}</p>
      <p class="setup-env-desc">{{ env.installs.message }}</p>
    </div>
  </div>
</template>
