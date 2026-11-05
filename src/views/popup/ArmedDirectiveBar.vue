<script setup lang="ts">
// The bar above the footer where a quick-reply directive waits until Send (docs/specs/special-replies.md D19):
// away mode with its scope and note, or "stop using AskHuman in this session" (scope fixed). × takes it back.
import { useI18n } from "vue-i18n";
import { QUICK_REPLY_NOTE_MAX } from "./useQuickReply";
import { usePopupContext } from "./context";

const { t } = useI18n();
const { armedDirective, disarmDirective, submitting } = usePopupContext();
</script>

<template>
  <div v-if="armedDirective" class="qr-bar" role="group" :aria-label="t('popup.quickReply.armed.group')">
    <template v-if="armedDirective.kind === 'away'">
      <svg class="qr-bar-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
      </svg>
      <span class="qr-bar-label">{{ t("popup.quickReply.armed.away") }}</span>
      <select
        v-model="armedDirective.scope"
        class="select qr-scope"
        :disabled="submitting"
        :aria-label="t('popup.quickReply.armed.scope')"
      >
        <option value="global">{{ t("popup.quickReply.armed.scopeAll") }}</option>
        <option value="session">{{ t("popup.quickReply.armed.scopeSession") }}</option>
      </select>
      <input
        v-model="armedDirective.note"
        class="qr-note"
        type="text"
        :maxlength="QUICK_REPLY_NOTE_MAX"
        :disabled="submitting"
        :placeholder="t('popup.quickReply.armed.notePlaceholder')"
        :aria-label="t('popup.quickReply.armed.note')"
        @paste.stop
      />
    </template>
    <template v-else>
      <svg class="qr-bar-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5" />
        <path d="M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7" />
        <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
        <path d="m2 2 20 20" />
      </svg>
      <span class="qr-bar-label">{{ t("popup.quickReply.armed.mute") }}</span>
      <span class="qr-bar-text">{{ t("popup.quickReply.armed.muteText") }}</span>
    </template>
    <button
      class="qr-bar-close"
      type="button"
      :disabled="submitting"
      :title="t('popup.quickReply.armed.undo')"
      :aria-label="t('popup.quickReply.armed.undo')"
      @click="disarmDirective()"
    >
      <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
        <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </button>
  </div>
</template>
