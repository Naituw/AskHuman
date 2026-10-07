<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { CompletionFeedback } from "./inboxTransition";
defineProps<{ completion: CompletionFeedback }>();
const { t } = useI18n();
</script>

<template>
  <span class="completion-label"><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="m3 8 3 3 7-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>{{ t(`popup.inbox.${completion}`) }}</span>
</template>

<style>
.completion-label { display: inline-flex; align-items: center; gap: 5px; height: 13px; vertical-align: top; }
.footer .btn.completion-success:disabled { opacity: 1; }
</style>
