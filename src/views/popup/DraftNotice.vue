<script setup lang="ts">
// Under the navbar of a card that came up with what an earlier ask of the same question was holding when its caller went
// away (docs/specs/popup-draft-on-cancel.md D7): one line that says so, and can be dismissed. It informs and never
// blocks; the content is the user's own from then on.
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const { t } = useI18n();
const { draftNotice, dismissDraftNotice } = usePopupContext();
</script>

<template>
  <div v-if="draftNotice" class="qr-banner draft-banner" role="status">
    <svg class="qr-bar-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
      <path d="M12 7v5l4 2" />
    </svg>
    <span class="qr-banner-text">
      {{ draftNotice.missingImages > 0
        ? t("popup.draftRestored.missingImages", { n: draftNotice.missingImages }, draftNotice.missingImages)
        : t("popup.draftRestored.text") }}
    </span>
    <button
      class="qr-bar-close"
      type="button"
      :title="t('popup.draftRestored.dismiss')"
      :aria-label="t('popup.draftRestored.dismiss')"
      @click="dismissDraftNotice()"
    >
      <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
        <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </button>
  </div>
</template>
