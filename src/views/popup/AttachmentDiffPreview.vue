<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import type { ParsedDiff } from "./useAttachmentContent";
import { DIFF_ROW_HEIGHT, diffFindLines, type DiffFindMatch } from "./attachmentFind";
import { findAllRanges } from "../../lib/findInDom";
const props = defineProps<{ parsed: ParsedDiff; viewport: HTMLElement | null; top: number; query?: string; caseSensitive?: boolean; current?: DiffFindMatch | null }>();
const ROW = DIFF_ROW_HEIGHT;
const height = ref(620);
const container = ref<HTMLElement | null>(null);
let observer: ResizeObserver | undefined;
const lines = computed(() => diffFindLines(props.parsed));
function parts(text: string, line: number) {
  const ranges = findAllRanges(text, props.query ?? "", props.caseSensitive ?? false);
  const result: { text: string; hit: boolean; current: boolean }[] = [];
  let offset = 0;
  ranges.forEach((range, occurrence) => {
    if (range.start > offset) result.push({ text: text.slice(offset, range.start), hit: false, current: false });
    result.push({ text: text.slice(range.start, range.end), hit: true, current: props.current?.line === line && props.current.occurrence === occurrence });
    offset = range.end;
  });
  if (offset < text.length) result.push({ text: text.slice(offset), hit: false, current: false });
  return result;
}
const start = computed(() => Math.max(0, Math.floor((props.top - (container.value?.offsetTop ?? 0)) / ROW) - 12));
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
  <div ref="container" class="attachment-diff" :style="{ height: `${lines.length * ROW}px`, minWidth: `${width}px` }">
    <div v-for="(line, i) in visible" :key="start + i" class="attachment-diff-line"
      :class="line.row?.kind ?? 'file-header'" :data-find-line="start + i" :style="{ top: `${(start + i) * ROW}px` }">
      <template v-if="line.row">
        <span class="attachment-diff-number">{{ line.row.old }}</span><span class="attachment-diff-number">{{ line.row.new }}</span>
        <code><span v-for="(part, pi) in parts(line.row.text, start + i)" :key="pi" :class="{ 'popup-find-hit': part.hit, 'popup-find-hit-current': part.current }">{{ part.text }}</span></code>
      </template>
      <strong v-else :title="line.title ?? undefined"><span v-for="(part, pi) in parts(line.title ?? '', start + i)" :key="pi" :class="{ 'popup-find-hit': part.hit, 'popup-find-hit-current': part.current }">{{ part.text }}</span></strong>
    </div>
  </div>
</template>
