<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { ReadingState } from "./useAttachmentContent";
const props = defineProps<{ url: string; width: number; height: number; name: string; state: ReadingState['image'] }>();
const emit = defineEmits<{ drag: [event: DragEvent]; error: [] }>();
const { t } = useI18n();
const viewport = ref<HTMLElement | null>(null);
const box = ref({ width: 1, height: 1 });
const overflowing = ref(false);
const panning = ref(false);
let observer: ResizeObserver | undefined;
const factor = computed(() => props.state.zoom === "fit"
  ? Math.min(1, Math.max(1, box.value.width - 32) / props.width, Math.max(1, box.value.height - 32) / props.height)
  : props.state.zoom);
const imageStyle = computed(() => ({ width: `${Math.max(1, props.width * factor.value)}px`, height: `${Math.max(1, props.height * factor.value)}px` }));
async function measure(restore = false) {
  await nextTick(); const el = viewport.value; if (!el) return;
  overflowing.value = el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1;
  if (restore) { el.scrollLeft = props.state.left; el.scrollTop = props.state.top; }
  if (props.state.zoom === "fit") { el.scrollLeft = 0; el.scrollTop = 0; }
}
async function zoom(value: "fit" | number) {
  const el = viewport.value; if (!el) return;
  const center = {
    x: (el.scrollLeft + el.clientWidth / 2 - Math.max(16, (el.clientWidth - props.width * factor.value) / 2)) / factor.value,
    y: (el.scrollTop + el.clientHeight / 2 - Math.max(16, (el.clientHeight - props.height * factor.value) / 2)) / factor.value,
  };
  props.state.zoom = typeof value === "number" ? Math.min(8, Math.max(0.05, value)) : value;
  await measure();
  if (value !== "fit") { el.scrollLeft = Math.max(16, (el.clientWidth - props.width * factor.value) / 2) + center.x * factor.value - el.clientWidth / 2; el.scrollTop = Math.max(16, (el.clientHeight - props.height * factor.value) / 2) + center.y * factor.value - el.clientHeight / 2; }
  saveScroll();
}
function saveScroll() { if (viewport.value) { props.state.left = viewport.value.scrollLeft; props.state.top = viewport.value.scrollTop; } }
function pan(event: PointerEvent) {
  const el = viewport.value; if (!el || !overflowing.value || event.button !== 0) return;
  event.preventDefault(); const target = event.currentTarget as HTMLElement;
  const { clientX, clientY } = event, left = el.scrollLeft, top = el.scrollTop;
  target.setPointerCapture(event.pointerId); panning.value = true;
  const move = (e: PointerEvent) => { el.scrollLeft = left + clientX - e.clientX; el.scrollTop = top + clientY - e.clientY; saveScroll(); };
  const end = () => { panning.value = false; target.removeEventListener("pointermove", move); target.removeEventListener("lostpointercapture", end); };
  target.addEventListener("pointermove", move); target.addEventListener("lostpointercapture", end);
}
function drag(event: DragEvent) { if (overflowing.value || panning.value) event.preventDefault(); else emit("drag", event); }
onMounted(() => {
  const el = viewport.value; if (!el) return;
  observer = new ResizeObserver(() => { box.value = { width: el.clientWidth, height: el.clientHeight }; void measure(); });
  observer.observe(el); box.value = { width: el.clientWidth, height: el.clientHeight }; void measure(true);
});
onBeforeUnmount(() => { saveScroll(); observer?.disconnect(); });
</script>
<template>
  <div class="attachment-image-preview">
    <div class="attachment-image-controls" data-tauri-drag-region>
      <button type="button" :aria-pressed="state.zoom === 'fit'" @click="zoom('fit')">{{ t('popup.preview.fit') }}</button>
      <button type="button" :aria-pressed="state.zoom === 1" @click="zoom(1)">100%</button>
      <button type="button" :aria-label="t('popup.preview.zoomOut')" @click="zoom(factor / 1.25)">−</button>
      <span data-tauri-drag-region>{{ Math.round(factor * 100) }}%</span>
      <button type="button" :aria-label="t('popup.preview.zoomIn')" @click="zoom(factor * 1.25)">+</button>
    </div>
    <div ref="viewport" class="attachment-image-viewport" tabindex="0" @scroll.passive="saveScroll">
      <div class="attachment-image-stage" data-tauri-drag-region :style="{ minWidth: `${width * factor + 32}px`, minHeight: `${height * factor + 32}px` }">
        <img :src="url" :alt="name" :style="imageStyle" :draggable="!overflowing && !panning" :class="{ pannable: overflowing, panning }"
          @pointerdown="pan" @dragstart="drag" @load="measure(true)" @error="emit('error')" />
      </div>
    </div>
  </div>
</template>
