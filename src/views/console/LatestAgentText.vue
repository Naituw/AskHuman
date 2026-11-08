<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import MarkdownContent from "../../components/MarkdownContent.vue";
import type { LatestAgentText } from "../../lib/types";
import { useLatestAgentText } from "./useLatestAgentText";

const props = defineProps<{ candidate?: LatestAgentText | null; legacyText?: string | null }>();
const { t } = useI18n();
const root = ref<HTMLElement | null>(null);
const body = ref<HTMLElement | null>(null);
const { displayed, expanded, reading, hasNewContent, receive, lockReading, toggleExpanded, updateNow } = useLatestAgentText();
const candidate = computed<LatestAgentText | null>(() => props.candidate !== undefined ? props.candidate : (props.legacyText ? {
  id: "legacy", revision: props.legacyText, kind: "assistant", text: props.legacyText, truncated: false,
} : null));
watch(candidate, receive, { immediate: true });
watch([displayed, expanded], async () => {
  await nextTick();
  if (body.value) body.value.scrollTop = !expanded.value && displayed.value?.kind === "thinking" ? body.value.scrollHeight : 0;
});

function selectionChanged() {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || !selection.rangeCount || !root.value) return;
  if (selection.getRangeAt(0).intersectsNode(root.value)) lockReading();
}

const scrollParents: HTMLElement[] = [];
function wheelReading(event: WheelEvent) {
  if (event.deltaY < 0) lockReading();
}
function keyboardReading(event: KeyboardEvent) {
  if (["ArrowUp", "PageUp", "Home"].includes(event.key) || (event.key === " " && event.shiftKey)) lockReading();
}
function removeScrollListeners() {
  scrollParents.forEach((node) => {
    node.removeEventListener("wheel", wheelReading);
    node.removeEventListener("pointerdown", lockReading);
    node.removeEventListener("keydown", keyboardReading);
  });
  scrollParents.length = 0;
}

onMounted(() => {
  document.addEventListener("selectionchange", selectionChanged);
});
watch(root, (element) => {
  removeScrollListeners();
  for (let node = element?.parentElement; node; node = node.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(node).overflowY)) {
      scrollParents.push(node);
      node.addEventListener("wheel", wheelReading, { passive: true });
      node.addEventListener("pointerdown", lockReading, { passive: true });
      node.addEventListener("keydown", keyboardReading);
    }
  }
});
onBeforeUnmount(() => {
  document.removeEventListener("selectionchange", selectionChanged);
  removeScrollListeners();
});
</script>

<template>
  <section v-if="displayed" ref="root" class="latest-text">
    <div v-if="displayed.kind === 'thinking'" class="text-kind">{{ t("console.observation.thinking") }}</div>
    <div ref="body" class="text-body" :class="{ preview: !expanded, thinking: displayed.kind === 'thinking' }"
      @pointerdown="lockReading" @wheel.passive="lockReading">
      <p v-if="displayed.kind === 'thinking'" class="thought">{{ displayed.text }}</p>
      <MarkdownContent v-else class="prose" :source="displayed.text" />
    </div>
    <div class="text-actions">
      <button type="button" :aria-expanded="expanded" @click="toggleExpanded">
        {{ t(expanded ? "console.observation.collapse" : "console.observation.expand") }}
      </button>
      <span v-if="displayed.truncated || (!expanded && displayed.kind === 'thinking')" class="fragment">
        {{ t("console.observation.fragment") }}
      </span>
      <span class="action-spacer" />
      <button v-if="hasNewContent && (reading || expanded)" type="button" class="new-content" @click="updateNow">
        {{ t("console.observation.newContent") }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.latest-text {
  margin-bottom: 12px;
}
.text-kind, .fragment {
  color: var(--text-tertiary);
  font-size: 11px;
}
.text-kind {
  margin-bottom: 6px;
}
.text-body {
  font-size: 13px;
  line-height: 1.6;
}
.text-body.preview {
  height: 6.4em;
  overflow: hidden;
}
.thought {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  margin: 0;
  color: var(--text-secondary);
}
.text-actions {
  display: flex;
  align-items: center;
  gap: 9px;
  min-height: 25px;
  margin-top: 5px;
}
.text-actions button {
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 11px;
  padding: 3px 0;
  cursor: pointer;
}
.text-actions button:hover, .text-actions .new-content {
  color: var(--accent);
}
.action-spacer {
  flex: 1;
}
</style>
