<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import type { ParsedDiff, DiffRow } from "./useAttachmentContent";
const props = defineProps<{ parsed: ParsedDiff; viewport: HTMLElement | null; top: number }>();
const ROW = 22;
const height = ref(620);
let observer: ResizeObserver | undefined;
const lines = computed(() => props.parsed.sections.flatMap(section => [
  ...(section.title ? [{ title: section.title, row: null as DiffRow | null }] : []),
  ...section.rows.map(row => ({ title: null, row })),
]));
const start = computed(() => Math.max(0, Math.floor(props.top / ROW) - 12));
const visible = computed(() => lines.value.slice(start.value, start.value + Math.ceil(height.value / ROW) + 24));
const width = computed(() => {
  // Count tabs and wide Unicode conservatively; keep long source lines horizontally readable.
  let longest = 0;
  for (const line of lines.value) {
    let count = 0; for (const c of line.row?.text ?? line.title ?? "") count += c === "\t" ? 4 : c.codePointAt(0)! > 255 ? 2 : 1;
    longest = Math.max(longest, count);
  }
  return Math.max(300, longest * 8 + 112);
});
watch(() => props.viewport, viewport => {
  observer?.disconnect();
  if (viewport) { observer = new ResizeObserver(() => { height.value = viewport.clientHeight; }); observer.observe(viewport); height.value = viewport.clientHeight; }
}, { flush: "post", immediate: true });
onBeforeUnmount(() => observer?.disconnect());
</script>
<template>
  <div class="attachment-diff" :style="{ height: `${lines.length * ROW}px`, minWidth: `${width}px` }">
    <div v-for="(line, i) in visible" :key="start + i" class="attachment-diff-line"
      :class="line.row?.kind ?? 'file-header'" :style="{ top: `${(start + i) * ROW}px` }">
      <template v-if="line.row">
        <span class="attachment-diff-number">{{ line.row.old }}</span><span class="attachment-diff-number">{{ line.row.new }}</span><code>{{ line.row.text }}</code>
      </template>
      <strong v-else :title="line.title ?? undefined">{{ line.title }}</strong>
    </div>
  </div>
</template>
