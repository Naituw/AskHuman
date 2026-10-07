<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { rippleFrame, rippleRadius } from "./unreadRipple";

const props = defineProps<{ active: boolean }>();
const circle = ref<HTMLElement | null>(null);
let row: HTMLElement | null = null;
let radius = 0;
let epoch = 0;
let mounted = false;
let visible = false;
let running = false;
let frame: number | null = null;
let rest: ReturnType<typeof setTimeout> | null = null;
let intersection: IntersectionObserver | undefined;
let resize: ResizeObserver | undefined;
let reduced: MediaQueryList | undefined;

function canRun() {
  return mounted && props.active && visible && radius > 0 && !document.hidden && !reduced?.matches;
}

function stop() {
  running = false;
  if (frame !== null) cancelAnimationFrame(frame);
  if (rest !== null) clearTimeout(rest);
  frame = null;
  rest = null;
  if (circle.value) circle.value.style.opacity = "0";
}

function paint(now: number) {
  const state = rippleFrame(now - epoch, radius);
  const style = circle.value!.style;
  style.transform = `scale(${state.scale})`;
  style.opacity = String(state.opacity);
  style.backgroundImage = `radial-gradient(circle closest-side, var(--inbox-ripple-color) 0%, var(--inbox-ripple-color) ${state.opaquePercent}%, transparent 100%)`;
  return state.sleepMs;
}

function tick(now: number) {
  frame = null;
  if (!canRun()) { stop(); return; }
  const sleepMs = paint(now);
  if (sleepMs) {
    // No frames are needed during the fully transparent rest interval.
    rest = setTimeout(() => {
      rest = null;
      if (canRun()) frame = requestAnimationFrame(tick);
      else stop();
    }, sleepMs);
  } else frame = requestAnimationFrame(tick);
}

function sync() {
  if (!canRun()) { stop(); return; }
  if (running) return;
  running = true;
  epoch = performance.now();
  tick(epoch);
}

function measure() {
  const dot = row?.querySelector<HTMLElement>(".inbox-dot");
  if (!row || !dot || !circle.value) return;
  const bounds = row.getBoundingClientRect();
  const point = dot.getBoundingClientRect();
  if (!bounds.width || !bounds.height) { radius = 0; sync(); return; }
  const x = point.left + point.width / 2 - bounds.left;
  const y = point.top + point.height / 2 - bounds.top;
  radius = rippleRadius(bounds.width, bounds.height, x, y);
  Object.assign(circle.value.style, {
    width: `${radius * 2}px`, height: `${radius * 2}px`,
    left: `${x - radius}px`, top: `${y - radius}px`,
  });
  if (running) paint(performance.now());
  sync();
}

watch(() => props.active, sync);
onMounted(() => {
  mounted = true;
  row = circle.value!.parentElement;
  reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)");
  if (reduced?.addEventListener) reduced.addEventListener("change", sync);
  else reduced?.addListener(sync);
  document.addEventListener("visibilitychange", sync);
  if (typeof IntersectionObserver !== "undefined") {
    intersection = new IntersectionObserver(entries => {
      visible = entries.some(entry => entry.isIntersecting);
      sync();
    });
    intersection.observe(row!);
  } else visible = true;
  if (typeof ResizeObserver !== "undefined") {
    resize = new ResizeObserver(measure);
    resize.observe(row!);
  } else window.addEventListener("resize", measure);
  measure();
});

onBeforeUnmount(() => {
  mounted = false;
  stop();
  intersection?.disconnect();
  resize?.disconnect();
  document.removeEventListener("visibilitychange", sync);
  window.removeEventListener("resize", measure);
  if (reduced?.removeEventListener) reduced.removeEventListener("change", sync);
  else reduced?.removeListener(sync);
});
</script>

<template><span ref="circle" class="inbox-unread-ripple" aria-hidden="true" /></template>

<style scoped>
.inbox-unread-ripple {
  position: absolute;
  z-index: 0;
  pointer-events: none;
  border-radius: 50%;
  opacity: 0;
}
</style>
