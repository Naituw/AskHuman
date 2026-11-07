<script setup lang="ts">
// The big heading every wizard page opens with: an icon, the title (with an optional tag next to
// it) and one lead paragraph, all centred. `hero` is the first page's welcome variant.
withDefaults(
  defineProps<{
    title: string;
    lead: string;
    variant?: "hero" | "page";
    leadTestid?: string;
  }>(),
  { variant: "page", leadTestid: undefined },
);
</script>

<template>
  <header class="setup-header" :class="variant">
    <div v-if="$slots.icon" class="setup-header-icon">
      <slot name="icon" />
    </div>
    <h1 class="setup-title">
      {{ title }}
      <slot name="tag" />
    </h1>
    <p class="setup-lead" :data-testid="leadTestid">{{ lead }}</p>
  </header>
</template>
