<script setup lang="ts">
// A small rounded badge that stands in for an Agent's or channel's logo (see brandBadges.ts).
import { computed } from "vue";
import { BADGES } from "./brandBadges";

const props = withDefaults(
  defineProps<{
    /** Agent or channel id; unknown ids fall back to the first letter of `label`. */
    id: string;
    label?: string;
    size?: "md" | "sm";
  }>(),
  { label: "", size: "md" },
);

const spec = computed(() => BADGES[props.id]);
const fallbackGlyph = computed(() => (props.label || props.id).trim().charAt(0).toUpperCase());
</script>

<template>
  <span
    class="brand-badge"
    :class="size"
    :style="{
      '--badge-from': spec?.from ?? '#6e6e73',
      '--badge-to': spec?.to ?? '#48484a',
    }"
    :data-badge="id"
    aria-hidden="true"
  >
    <svg v-if="spec?.paths" viewBox="0 0 24 24" focusable="false">
      <path v-for="d in spec.paths" :key="d" :d="d" />
    </svg>
    <span v-else class="brand-glyph">{{ spec?.glyph ?? fallbackGlyph }}</span>
  </span>
</template>
