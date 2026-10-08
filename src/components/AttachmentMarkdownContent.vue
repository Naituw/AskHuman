<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { enhanceAttachmentMermaid } from "../lib/attachmentMermaid";
import { effectiveColorScheme } from "../lib/theme";

const props = defineProps<{ html: string }>();
const root = ref<HTMLElement | null>(null);
const { t, locale } = useI18n();
let generation = 0;
let mounted = false;
let dispose: (() => void) | undefined;
async function hydrate() {
  const token = ++generation;
  dispose?.();
  await nextTick();
  if (!mounted || token !== generation || !root.value) return;
  root.value.innerHTML = props.html;
  dispose = enhanceAttachmentMermaid(root.value, {
    theme: effectiveColorScheme.value,
    labels: {
      copy: t("common.copyCode"), copied: t("common.copied"),
      diagram: t("common.mermaid.diagram"), showSource: t("common.mermaid.showSource"),
      showDiagram: t("common.mermaid.showDiagram"), rendering: t("common.mermaid.rendering"),
      failed: t("common.mermaid.failed"), tooLarge: t("common.mermaid.tooLarge"), tooMany: t("common.mermaid.tooMany"),
    },
  }).dispose;
}
onMounted(() => { mounted = true; void hydrate(); });
watch([() => props.html, effectiveColorScheme, locale], () => { if (mounted) void hydrate(); }, { flush: "post" });
onBeforeUnmount(() => { mounted = false; generation++; dispose?.(); });
</script>

<template><article ref="root" v-html="html"></article></template>
