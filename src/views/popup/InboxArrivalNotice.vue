<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { arrivalAnimator, type ArrivalEnd, type ArrivalFrame, type ArrivalGeometry } from "./inboxArrival";
import lucideLicense from "./lucide-license.txt?raw";

const props = defineProps<{ measure: (id: string) => ArrivalGeometry | null }>();
const emit = defineEmits<{ settled: [id: string, reason: ArrivalEnd] }>();
const notices = ref<{ id: string; key: number }[]>([]);
const elements = new Map<string, HTMLElement>();
let sequence = 0;
let reduced: MediaQueryList | undefined;
let disposed = false;
function settle(id: string, reason: ArrivalEnd) {
  if (!notices.value.some(item => item.id === id)) return;
  notices.value = notices.value.filter(item => item.id !== id);
  elements.delete(id);
  emit("settled", id, reason);
}
function paint(id: string, state: ArrivalFrame) {
  const element = elements.get(id);
  if (!element) return;
  element.style.transform = `translate3d(${state.x - 44}px, ${state.y - 44}px, 0) scale(${state.scale}) rotate(${state.angle}deg)`;
  element.style.opacity = String(state.opacity);
  element.style.setProperty("--arrival-morph", String(state.morph));
  element.style.setProperty("--arrival-hole", String(1 - state.morph));
  element.style.setProperty("--arrival-plate", String(state.plateOpacity));
  element.style.setProperty("--arrival-halo", String(state.haloOpacity));
  element.style.setProperty("--arrival-clip", `${68 + (100 * 10 / 24 - 68) * state.morph}%`);
  element.style.setProperty("--arrival-graphic-scale", String(1 + (88 / 52 - 1) * state.morph));
  // Precomputed colors avoid relying on color-mix() in older WebKit.
  element.querySelector("stop")?.setAttribute("stop-color", `rgb(${38 + 33 * (1 - state.morph)}, ${133 + 18 * (1 - state.morph)}, ${232 + 3 * (1 - state.morph)})`);
}
const animator = arrivalAnimator({ measure: id => props.measure(id), paint, settle });
async function play(id: string) {
  if (disposed || notices.value.some(item => item.id === id)) return;
  notices.value.push({ id, key: ++sequence });
  if (reduced?.matches || document.hidden) { settle(id, "unavailable"); return; }
  await nextTick();
  if (!disposed && elements.has(id)) animator.play(id);
}
function cancel(id: string) { animator.cancel(id); settle(id, "cancelled"); }
function clear() { animator.clear(); for (const item of [...notices.value]) settle(item.id, "cancelled"); }
function accessibilityChanged() { if (document.hidden || reduced?.matches) clear(); }
defineExpose({ play, cancel, clear });
onMounted(() => {
  reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)");
  if (reduced?.addEventListener) reduced.addEventListener("change", accessibilityChanged);
  else reduced?.addListener(accessibilityChanged);
  document.addEventListener("visibilitychange", accessibilityChanged);
});
onBeforeUnmount(() => {
  disposed = true; clear();
  document.removeEventListener("visibilitychange", accessibilityChanged);
  if (reduced?.removeEventListener) reduced.removeEventListener("change", accessibilityChanged);
  else reduced?.removeListener(accessibilityChanged);
});
</script>

<template>
  <div class="inbox-arrival-layer" aria-hidden="true">
    <div v-for="item in notices" :key="item.key" :ref="element => { if (element) elements.set(item.id, element as HTMLElement); }" class="inbox-arrival-floater" :data-arrival-id="item.id">
      <div class="inbox-arrival-halo" />
      <div class="inbox-arrival-face">
        <div class="inbox-arrival-plate" />
        <!-- MessageCircleQuestionMark geometry: Lucide, ISC license (see lucide-license.txt). -->
        <svg viewBox="0 0 24 24" class="inbox-arrival-graphic">
          <desc>{{ lucideLicense }}</desc>
          <defs>
            <mask :id="`arrival-hole-${item.key}`" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24" style="mask-type: luminance">
              <path fill="#fff" d="M2.992 16.342a2 2 0 0 1 .094 1.167l-1.065 3.29a1 1 0 0 0 1.236 1.168l3.413-.998a2 2 0 0 1 1.099.092 10 10 0 1 0-4.777-4.719" />
              <g fill="none" stroke="#000" stroke-width="1.85" stroke-linecap="round" class="inbox-arrival-hole">
                <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" />
                <path d="M12 17h.01" />
              </g>
            </mask>
            <linearGradient :id="`arrival-blue-${item.key}`" x1="0" y1="0" x2=".2" y2="1">
              <stop offset="0" stop-color="#4797eb" /><stop offset="1" stop-color="#2685e8" />
            </linearGradient>
          </defs>
          <path :fill="`url(#arrival-blue-${item.key})`" :mask="`url(#arrival-hole-${item.key})`" d="M2.992 16.342a2 2 0 0 1 .094 1.167l-1.065 3.29a1 1 0 0 0 1.236 1.168l3.413-.998a2 2 0 0 1 1.099.092 10 10 0 1 0-4.777-4.719" />
        </svg>
      </div>
    </div>
  </div>
</template>

<style scoped>
.inbox-arrival-layer { position: absolute; inset: 0; z-index: 80; pointer-events: none; }
.inbox-arrival-layer * { pointer-events: none; }
.inbox-arrival-floater { position: absolute; left: 0; top: 0; width: 88px; height: 88px; opacity: 0; transform-origin: center; will-change: transform, opacity; }
.inbox-arrival-halo { position: absolute; inset: -36px; border-radius: 50%; background: radial-gradient(circle, rgba(38,133,232,.18), transparent 68%); opacity: var(--arrival-halo, 1); }
.inbox-arrival-face { position: absolute; inset: 0; display: grid; place-items: center; }
.inbox-arrival-plate { position: absolute; inset: 0; border-radius: calc(24px + 20px * var(--arrival-morph, 0)); background: linear-gradient(145deg, #fff 10%, #f8fbff 47%, #eaf1fb); box-shadow: 0 1px 2px #17345214, 0 5px 10px #1b3d6324, 0 15px 28px #233e622b, 0 0 0 .5px #54729633, 0 1.2px 1px #fff inset, 0 -1px 1px #b5c7e044 inset; opacity: var(--arrival-plate, 1); transform: scale(calc(1 - .2 * var(--arrival-morph, 0))); }
.inbox-arrival-graphic { position: relative; width: 52px; height: 52px; clip-path: circle(var(--arrival-clip, 68%) at 50% 50%); transform: scale(var(--arrival-graphic-scale, 1)); }
.inbox-arrival-hole { opacity: var(--arrival-hole, 1); }
</style>
