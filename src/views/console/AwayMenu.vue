<script setup lang="ts">
// The per-session "AskHuman" pull-down in the console's detail header (docs/specs/special-replies.md
// D25): normal / away for this session / stop using AskHuman, or, while away mode is on for everyone,
// follow the global switch / always ask normally / stop using AskHuman.
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { AwaySessionMode, AwaySessionSetting } from "../../lib/types";
import { sessionChoices, sessionTone, sessionValue } from "./away";

const { t } = useI18n();

const props = defineProps<{
  /** The session's own setting; `null` = it follows the global switch. */
  own: AwaySessionMode | null;
  /** Away mode is on for every Agent. */
  globalOn: boolean;
}>();

const emit = defineEmits<{
  set: [setting: AwaySessionSetting];
}>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);

const choices = computed(() => sessionChoices(props.own, props.globalOn));
const value = computed(() => t(`console.away.${sessionValue(props.own, props.globalOn)}`));
const tone = computed(() => sessionTone(props.own, props.globalOn));

function pick(setting: AwaySessionSetting) {
  open.value = false;
  emit("set", setting);
}

function onPointerDown(e: PointerEvent) {
  if (open.value && !(e.target instanceof Node && root.value?.contains(e.target))) open.value = false;
}
function onKeydown(e: KeyboardEvent) {
  if (open.value && e.key === "Escape") {
    e.stopPropagation();
    open.value = false;
  }
}
onMounted(() => {
  window.addEventListener("pointerdown", onPointerDown);
  window.addEventListener("keydown", onKeydown, true);
});
onBeforeUnmount(() => {
  window.removeEventListener("pointerdown", onPointerDown);
  window.removeEventListener("keydown", onKeydown, true);
});
</script>

<template>
  <div ref="root" class="am-wrap" data-testid="away-menu">
    <button
      class="am-btn"
      :class="tone"
      type="button"
      aria-haspopup="menu"
      :aria-expanded="open"
      :aria-label="t('console.away.menuLabel')"
      data-testid="away-menu-button"
      @click="open = !open"
    >
      <span class="am-key">{{ t("console.away.menuKey") }}</span>
      <span class="am-val" data-testid="away-menu-value">{{ value }}</span>
      <svg class="am-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
    </button>
    <div v-if="open" class="am-pop" role="menu">
      <button
        v-for="c in choices"
        :key="c.setting"
        class="am-item"
        type="button"
        role="menuitemradio"
        :aria-checked="c.checked"
        :data-setting="c.setting"
        @click="pick(c.setting)"
      >
        <span class="am-check">
          <svg v-if="c.checked" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5" /></svg>
        </span>
        <span class="am-label">{{ t(`console.away.${c.label}`) }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.am-wrap {
  position: relative;
  margin-left: 6px;
  display: inline-flex;
}
.am-btn {
  appearance: none;
  border: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 20px;
  padding: 0 6px 0 8px;
  border-radius: 6px;
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--text-primary) 6%, transparent);
}
.am-btn:hover {
  background: color-mix(in srgb, var(--text-primary) 10%, transparent);
}
.am-key {
  color: var(--text-tertiary);
}
.am-chev {
  width: 11px;
  height: 11px;
  opacity: 0.7;
}
.am-btn.away {
  color: color-mix(in srgb, var(--accent-orange, #ff9f0a) 85%, var(--text-primary));
  background: color-mix(in srgb, var(--accent-orange, #ff9f0a) 14%, transparent);
}
.am-btn.away .am-key {
  color: inherit;
  opacity: 0.75;
}
.am-btn.mute .am-val {
  color: var(--text-tertiary);
}
.am-pop {
  position: absolute;
  top: 25px;
  left: 0;
  z-index: 20;
  min-width: 232px;
  padding: 4px;
  border-radius: 10px;
  background: var(--surface-overlay, var(--bg));
  border: var(--hairline) solid var(--control-border, var(--border));
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.28);
  font-size: 13px;
}
.am-item {
  appearance: none;
  border: none;
  background: transparent;
  width: 100%;
  display: flex;
  align-items: center;
  height: 26px;
  padding: 0 10px 0 4px;
  border-radius: 6px;
  font: inherit;
  color: var(--text-primary);
  text-align: left;
  cursor: pointer;
}
.am-item:hover {
  background: color-mix(in srgb, var(--text-primary) 7%, transparent);
}
.am-check {
  width: 20px;
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--accent);
}
.am-check svg {
  width: 13px;
  height: 13px;
}
.am-label {
  flex: 1;
  white-space: nowrap;
}
</style>
