<script setup lang="ts">
import CollapseStage from "./CollapseStage.vue";
import CollapseControls from "./CollapseControls.vue";

const controls = new URLSearchParams(location.search).get("surface") === "controls";
</script>

<template>
  <CollapseControls v-if="controls" />
  <CollapseStage v-else />
</template>
