<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const { t } = useI18n();
const {
  isMulti,
  total,
  composerOwnerQ,
  dockedComposerQ,
  setComposerDockRef,
  composerDockTargetId,
  returnComposerHome,
} = usePopupContext();
</script>

<template>
  <section
    :ref="(el) => setComposerDockRef(el as HTMLElement | null)"
    v-show="dockedComposerQ !== null"
    class="composer-dock"
    role="region"
    :aria-label="t('popup.composer.dockedLabel')"
  >
    <button
      v-if="isMulti && composerOwnerQ !== null"
      class="composer-dock-home"
      type="button"
      :title="t('popup.composer.returnToQuestion')"
      :aria-label="t('popup.composer.returnToQuestion')"
      @click="returnComposerHome"
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="m15 18-6-6 6-6" />
        <path d="M9 12h10" />
      </svg>
      <span>{{ t("popup.question.indexed", { i: composerOwnerQ + 1, n: total }) }}</span>
    </button>
    <div :id="composerDockTargetId" class="composer-dock-target"></div>
  </section>
</template>
