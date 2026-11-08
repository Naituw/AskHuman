<script setup lang="ts">
// A small rounded tile with the mark of an Agent or channel on it (see brandBadges.ts).
import { computed } from "vue";
import { BADGES, FALLBACK_TILE, svgUrl } from "./brandBadges";
import "./brandBadge.css";

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
// A one-colour mark is a mask over a painted layer; a mark in its own colours is the image itself.
const ownColours = computed(() => spec.value !== undefined && spec.value.mark.paint === undefined);

const vars = computed<Record<string, string>>(() => {
  const badge = spec.value;
  if (!badge) return { "--badge-tile": FALLBACK_TILE };
  const out: Record<string, string> = {
    "--badge-tile": badge.tile,
    "--badge-hairline": badge.hairline,
    "--mark-url": svgUrl(badge.mark.svg),
    "--mark-scale": String(badge.mark.scale),
  };
  if (badge.mark.paint !== undefined) out["--mark-paint"] = badge.mark.paint;
  return out;
});
</script>

<template>
  <span
    class="brand-badge"
    :class="[size, { 'own-colours': ownColours }]"
    :style="vars"
    :data-badge="id"
    aria-hidden="true"
  >
    <span v-if="spec" class="brand-mark" />
    <span v-else class="brand-glyph">{{ fallbackGlyph }}</span>
  </span>
</template>
