<script setup lang="ts">
// Under the navbar of a question that reached the user although away mode is on (docs/specs/special-replies.md D21):
// why it came through, and a one-click "end away mode".
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const { t } = useI18n();
const { quickReplyEscalated, quickReplyEnding, quickReplyAwayError, endAwayMode } = usePopupContext();
</script>

<template>
  <div v-if="quickReplyEscalated" class="qr-banner" role="status">
    <svg class="qr-bar-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
    </svg>
    <span class="qr-banner-text">
      <b>{{ t("popup.escalated.title") }}</b>{{ t(`popup.escalated.${quickReplyEscalated}`) }}
    </span>
    <button class="qr-banner-action" type="button" :disabled="quickReplyEnding" @click="endAwayMode()">
      {{ t("popup.escalated.end") }}
    </button>
  </div>
  <p v-if="quickReplyAwayError" class="status-error qr-banner-error" role="alert">
    {{ t("popup.escalated.endFailed", { message: quickReplyAwayError }) }}
  </p>
</template>
