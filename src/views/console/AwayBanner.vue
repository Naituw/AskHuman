<script setup lang="ts">
// The console's global away-mode banner (docs/specs/special-replies.md D25): shown between the
// header and the two panes while away mode is on, with the way out and a link to the record.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { AwayGlobalView } from "../../lib/types";
import { awayDuration } from "./away";

const { t } = useI18n();

const props = defineProps<{
  global: AwayGlobalView;
  /** Escalated questions waiting for an answer. */
  waiting: number;
  /** Clock the duration is told against (the console's 1 s ticker). */
  nowMs: number;
}>();

const emit = defineEmits<{
  viewRecord: [];
  end: [];
}>();

const duration = computed(() => {
  const d = awayDuration(props.global.sinceMs, props.nowMs);
  if (d.kind === "now") return t("console.away.durationNow");
  if (d.kind === "m") return t("console.away.durationM", { m: d.m });
  return t("console.away.durationHM", { h: d.h, m: d.m });
});

const description = computed(() => {
  const parts = [
    t("console.away.bannerScopeAll"),
    t("console.away.bannerFor", { d: duration.value }),
    t("console.away.bannerReplies", { n: props.global.replies }),
  ];
  if (props.waiting > 0) parts.push(t("console.away.bannerWaiting", { n: props.waiting }));
  return parts.join(" · ");
});
</script>

<template>
  <div class="away-banner" role="status" data-testid="away-banner">
    <svg class="away-banner-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg>
    <span class="away-banner-title">{{ t("console.away.bannerTitle") }}</span>
    <span class="away-banner-desc">{{ description }}</span>
    <span class="away-banner-spacer" />
    <button class="away-banner-btn" type="button" data-testid="away-banner-record" @click="emit('viewRecord')">
      {{ t("console.away.viewRecord") }}
    </button>
    <button class="away-banner-btn strong" type="button" data-testid="away-banner-end" @click="emit('end')">
      {{ t("console.away.end") }}
    </button>
  </div>
</template>

<style scoped>
.away-banner {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 14px;
  font-size: 12.5px;
  border-bottom: var(--hairline) solid var(--border);
  background: color-mix(in srgb, var(--accent-orange, #ff9f0a) 9%, var(--control-bg, transparent));
}
.away-banner-icon {
  width: 14px;
  height: 14px;
  flex: none;
  color: var(--accent-orange, #ff9f0a);
}
.away-banner-title {
  font-weight: 600;
  white-space: nowrap;
}
.away-banner-desc {
  min-width: 0;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.away-banner-spacer {
  flex: 1;
}
.away-banner-btn {
  flex: none;
  appearance: none;
  border: none;
  background: transparent;
  padding: 2px 8px;
  border-radius: 6px;
  font: inherit;
  color: var(--accent);
  cursor: pointer;
  white-space: nowrap;
}
.away-banner-btn:hover {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}
.away-banner-btn.strong {
  font-weight: 500;
}
</style>
