<script setup lang="ts">
// The attachment conditions in effect, as removable tags under the search row (docs/specs/reply-history.md §3.7 A2).
import { useI18n } from "vue-i18n";
import { NO_EXTENSION, type AttachmentChip } from "../../lib/historyAttachments";
import type { AttachmentChipItem } from "./useAttachmentFilter";

defineProps<{ items: AttachmentChipItem[] }>();
defineEmits<{ remove: [chip: AttachmentChip]; clear: [] }>();

const { t } = useI18n();
</script>

<template>
  <div class="attach-chips" role="group" :aria-label="t('history.attach.active')">
    <TransitionGroup name="attach-chip" tag="div" class="attach-chip-list">
      <span
        v-for="item in items"
        :key="item.key"
        class="attach-chip"
        :class="[item.chip.kind, { plain: item.chip.kind === 'ext' && item.chip.ext === NO_EXTENSION }]"
      >
        <svg
          v-if="item.chip.kind !== 'ext'"
          class="attach-chip-icon"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        ><path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" /></svg>
        <span class="attach-chip-text">{{ item.label }}</span>
        <button
          type="button"
          class="attach-chip-x"
          :aria-label="t('history.attach.remove', { label: item.label })"
          :title="t('history.attach.remove', { label: item.label })"
          @click="$emit('remove', item.chip)"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
        </button>
      </span>
    </TransitionGroup>
    <button type="button" class="attach-chip-clear" @click="$emit('clear')">
      {{ t("history.attach.clear") }}
    </button>
  </div>
</template>

<style scoped>
.attach-chips {
  flex: 0 0 auto;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 0 14px 9px;
}
.attach-chip-list {
  position: relative;
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.attach-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 100%;
  height: 22px;
  padding: 0 3px 0 8px;
  border: var(--hairline) solid color-mix(in srgb, var(--accent) 32%, transparent);
  border-radius: 999px;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--text-primary);
  font-size: 12px;
}
.attach-chip-icon {
  flex: 0 0 auto;
  width: 11px;
  height: 11px;
  color: var(--accent);
}
.attach-chip-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.attach-chip.ext .attach-chip-text {
  font-family: var(--font-mono);
  font-size: 11.5px;
}
/* "No extension" is a phrase, not a suffix. */
.attach-chip.ext.plain .attach-chip-text {
  font-family: inherit;
  font-size: 12px;
}
.attach-chip-x {
  flex: 0 0 auto;
  display: inline-grid;
  place-items: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}
.attach-chip-x:hover {
  background: color-mix(in srgb, var(--text-primary) 14%, transparent);
  color: var(--text-primary);
}
.attach-chip-x:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.attach-chip-x svg {
  width: 10px;
  height: 10px;
}
.attach-chip-clear {
  flex: 0 0 auto;
  height: 22px;
  padding: 0 6px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  white-space: nowrap;
  cursor: pointer;
}
.attach-chip-clear:hover {
  background: color-mix(in srgb, var(--text-primary) 8%, transparent);
  color: var(--text-primary);
}
.attach-chip-clear:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

.attach-chip-enter-active,
.attach-chip-leave-active,
.attach-chip-move {
  transition: opacity 0.12s ease, transform 0.12s ease;
}
.attach-chip-enter-from,
.attach-chip-leave-to {
  opacity: 0;
  transform: scale(0.9);
}
.attach-chip-leave-active {
  position: absolute;
}
@media (prefers-reduced-motion: reduce) {
  .attach-chip-enter-active,
  .attach-chip-leave-active,
  .attach-chip-move {
    transition: none;
  }
}
</style>
