<script setup lang="ts">
// The history window's "Attachments" button and its filter panel (docs/specs/reply-history.md §3.7).
// Stateless about the filter itself: the window owns it and the panel edits a copy through v-model.
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  NO_EXTENSION,
  emptyAttachmentFilter,
  isAttachmentFilterActive,
  isAttachmentRequirementImplied,
  setAttachmentRequired,
  setAttachmentSource,
  toggleAttachmentExt,
  type AttachmentFacet,
  type AttachmentFilter,
  type AttachmentSource,
} from "../../lib/historyAttachments";

const props = defineProps<{
  modelValue: AttachmentFilter;
  open: boolean;
  /** Suffix choices for the entries in scope (only computed while the panel is open). */
  facet: AttachmentFacet;
  /** Entries that pass every condition right now. */
  matchCount: number;
  /** Number of removable conditions, shown on the button. */
  chipCount: number;
}>();
const emit = defineEmits<{
  "update:modelValue": [value: AttachmentFilter];
  "update:open": [value: boolean];
}>();

const { t } = useI18n();
const SOURCES: AttachmentSource[] = ["all", "question", "reply"];
const uid = useId();
const panelId = `attach-panel-${uid}`;
const radioName = `attach-source-${uid}`;

const root = ref<HTMLElement | null>(null);
const button = ref<HTMLButtonElement | null>(null);
const panel = ref<HTMLElement | null>(null);
const list = ref<HTMLElement | null>(null);

const active = computed(() => isAttachmentFilterActive(props.modelValue));
// A source other than "all" or any suffix already means "has attachments": the switch shows on and is locked.
const implied = computed(() => isAttachmentRequirementImplied(props.modelValue));
const requiredOn = computed(() => implied.value || props.modelValue.withAttachments);

const maxCount = computed(() => props.facet.options.reduce((max, option) => Math.max(max, option.count), 0));
/** Share of the busiest suffix, with a floor so a rare one still draws a sliver. */
function share(count: number): number {
  if (!count || !maxCount.value) return 0;
  return Math.max(count / maxCount.value, 0.05);
}

function extLabel(ext: string): string {
  return ext === NO_EXTENSION ? t("history.attach.noExt") : `.${ext}`;
}

function update(next: AttachmentFilter) {
  emit("update:modelValue", next);
}
function setSource(source: AttachmentSource) {
  update(setAttachmentSource(props.modelValue, source));
}
function setRequired(event: Event) {
  update(setAttachmentRequired(props.modelValue, (event.target as HTMLInputElement).checked));
}
function toggleExt(ext: string) {
  update(toggleAttachmentExt(props.modelValue, ext));
}
function reset() {
  update(emptyAttachmentFilter());
}

function setOpen(open: boolean) {
  if (open !== props.open) emit("update:open", open);
}
function toggle() {
  setOpen(!props.open);
}

// Keyboard focus lives inside the panel while it is open, so Escape and the arrow keys reach it even when the
// button was clicked with the mouse (WebKit does not focus a clicked button).
watch(
  () => props.open,
  async (open) => {
    if (!open) return;
    await nextTick();
    panel.value?.focus({ preventScroll: true });
  }
);

function onKeydown(event: KeyboardEvent) {
  // Escape closes just this panel: the history window would otherwise also fold the attachment preview.
  if (event.key !== "Escape" || !props.open) return;
  event.stopPropagation();
  event.preventDefault();
  setOpen(false);
  void nextTick(() => button.value?.focus());
}

function onWindowPointerDown(event: PointerEvent) {
  if (!props.open) return;
  const target = event.target;
  if (target instanceof Node && root.value?.contains(target)) return;
  setOpen(false);
}
function onWindowBlur() {
  setOpen(false);
}
onMounted(() => {
  window.addEventListener("pointerdown", onWindowPointerDown);
  window.addEventListener("blur", onWindowBlur);
});
onBeforeUnmount(() => {
  window.removeEventListener("pointerdown", onWindowPointerDown);
  window.removeEventListener("blur", onWindowBlur);
});

// The suffix rows form one tab stop; the arrow keys move between them (Space toggles, as on any checkbox).
const lastFocusedExt = ref<string | null>(null);
const tabStop = computed(() => {
  const options = props.facet.options;
  const remembered = lastFocusedExt.value;
  if (remembered !== null && options.some((option) => option.ext === remembered)) return remembered;
  return options.length ? options[0].ext : null;
});
function onListKeydown(event: KeyboardEvent) {
  const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
  if (!keys.includes(event.key)) return;
  const inputs = Array.from(list.value?.querySelectorAll<HTMLInputElement>("input") ?? []);
  if (!inputs.length) return;
  const at = inputs.findIndex((input) => input === document.activeElement);
  let next: number;
  if (event.key === "Home") next = 0;
  else if (event.key === "End") next = inputs.length - 1;
  else if (event.key === "ArrowDown") next = at < 0 ? 0 : Math.min(inputs.length - 1, at + 1);
  else next = at < 0 ? inputs.length - 1 : Math.max(0, at - 1);
  event.preventDefault();
  inputs[next].focus();
}
</script>

<template>
  <div ref="root" class="attach-wrap" @keydown="onKeydown">
    <button
      ref="button"
      class="attach-btn"
      :class="{ active }"
      type="button"
      :title="t('history.attach.tooltip')"
      :aria-label="t('history.attach.title')"
      :aria-expanded="open"
      :aria-controls="open ? panelId : undefined"
      aria-haspopup="dialog"
      @click="toggle"
    >
      <svg class="attach-clip" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" /></svg>
      <span class="attach-btn-label">{{ t("history.attach.button") }}</span>
      <span v-if="chipCount" class="attach-count" aria-hidden="true">{{ chipCount }}</span>
      <svg class="attach-chevron" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6" /></svg>
    </button>

    <Transition name="attach-pop">
      <div
        v-if="open"
        :id="panelId"
        ref="panel"
        class="attach-panel"
        role="dialog"
        :aria-label="t('history.attach.title')"
        tabindex="-1"
      >
        <header class="attach-head">
          <span class="attach-title">{{ t("history.attach.title") }}</span>
          <button class="attach-reset" type="button" :disabled="!active" @click="reset">
            {{ t("history.attach.reset") }}
          </button>
        </header>

        <div class="attach-body">
          <section>
            <div class="attach-label">{{ t("history.attach.source") }}</div>
            <div class="attach-seg" role="radiogroup" :aria-label="t('history.attach.source')">
              <label v-for="source in SOURCES" :key="source" class="attach-seg-item">
                <input
                  type="radio"
                  class="attach-sr"
                  :name="radioName"
                  :value="source"
                  :checked="modelValue.source === source"
                  @change="setSource(source)"
                />
                <span class="attach-seg-text">{{ t(`history.attach.sources.${source}`) }}</span>
              </label>
            </div>
          </section>

          <label class="attach-switch-row" :class="{ locked: implied }">
            <span class="attach-switch-text">{{ t("history.attach.required") }}</span>
            <span class="attach-switch-count">{{ facet.entries }}</span>
            <input
              type="checkbox"
              role="switch"
              class="attach-sr"
              :checked="requiredOn"
              :disabled="implied"
              @change="setRequired"
            />
            <span class="attach-switch" aria-hidden="true"></span>
          </label>

          <section class="attach-exts">
            <div class="attach-label">
              <span>{{ t("history.attach.suffix") }}</span>
              <span class="attach-hint">{{ t("history.attach.suffixHint") }}</span>
            </div>
            <div
              v-if="facet.options.length"
              ref="list"
              class="attach-ext-list"
              role="group"
              :aria-label="t('history.attach.suffix')"
              @keydown="onListKeydown"
            >
              <label
                v-for="option in facet.options"
                :key="option.ext"
                class="attach-ext"
                :class="{ on: modelValue.exts.includes(option.ext), zero: option.count === 0 }"
                :style="{ '--share': share(option.count) }"
              >
                <input
                  type="checkbox"
                  class="attach-sr"
                  :checked="modelValue.exts.includes(option.ext)"
                  :tabindex="option.ext === tabStop ? 0 : -1"
                  @focus="lastFocusedExt = option.ext"
                  @change="toggleExt(option.ext)"
                />
                <span class="attach-check" aria-hidden="true">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5" /></svg>
                </span>
                <span class="attach-ext-name" :class="{ plain: option.ext === NO_EXTENSION }">{{ extLabel(option.ext) }}</span>
                <span class="attach-ext-bar" aria-hidden="true"></span>
                <span class="attach-ext-count">{{ option.count }}</span>
              </label>
            </div>
            <div v-else class="attach-none">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" /></svg>
              <span>{{ t("history.attach.none") }}</span>
            </div>
          </section>
        </div>

        <footer class="attach-foot" role="status">
          {{ t("history.attach.matches", { n: matchCount }) }}
        </footer>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.attach-wrap {
  position: relative;
  flex: 0 0 auto;
  margin-left: 8px;
}

/* Button: the same chrome as the scope and cleanup buttons; accent-tinted while conditions are active. */
.attach-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 8px 0 10px;
  border: var(--hairline) solid var(--control-border);
  border-radius: var(--radius-sm, 8px);
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 12px;
  white-space: nowrap;
  cursor: pointer;
  box-shadow: var(--clickable-shadow);
}
.attach-btn:hover {
  background: var(--control-hover-bg, var(--control-bg));
}
.attach-btn:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring), var(--clickable-shadow);
}
.attach-btn.active {
  border-color: color-mix(in srgb, var(--accent) 66%, var(--border));
  background: color-mix(in srgb, var(--accent) 13%, var(--control-bg));
}
.attach-clip,
.attach-chevron {
  flex: 0 0 auto;
  width: 13px;
  height: 13px;
  color: var(--text-secondary);
}
.attach-btn.active .attach-clip {
  color: var(--accent);
}
.attach-chevron {
  transition: transform 0.15s ease;
}
.attach-btn[aria-expanded="true"] .attach-chevron {
  transform: rotate(180deg);
}
.attach-count {
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--accent);
  color: #fff;
  font-size: 10px;
  font-weight: 700;
  line-height: 16px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}

/* Panel */
.attach-panel {
  position: absolute;
  z-index: 30;
  top: calc(100% + 6px);
  right: 0;
  display: flex;
  flex-direction: column;
  width: min(316px, calc(var(--history-main-width, 100vw) - 28px));
  max-height: min(500px, calc(100vh - 150px));
  overflow: hidden;
  border: var(--hairline) solid var(--border);
  border-radius: var(--radius-lg, 10px);
  background: var(--surface-overlay);
  box-shadow: 0 14px 38px rgba(0, 0, 0, 0.24), 0 2px 8px rgba(0, 0, 0, 0.1);
  transform-origin: top right;
  outline: none;
}
.attach-pop-enter-active,
.attach-pop-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}
.attach-pop-enter-from,
.attach-pop-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.98);
}

.attach-head {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 11px 12px 8px 14px;
}
.attach-title {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
}
.attach-reset {
  padding: 2px 6px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--accent);
  font-size: 12px;
  cursor: pointer;
}
.attach-reset:hover:not(:disabled) {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}
.attach-reset:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
.attach-reset:disabled {
  color: var(--text-tertiary);
  cursor: default;
}

.attach-body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 0 14px 12px;
}
.attach-label {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-bottom: 6px;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
}
.attach-hint {
  margin-left: auto;
  color: var(--text-tertiary);
  font-weight: 400;
}

/* Visually hidden native inputs keep the keyboard and screen-reader behavior of radios and checkboxes. */
.attach-sr {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: 0;
  opacity: 0;
  pointer-events: none;
}

/* Source: a segmented control. */
.attach-seg {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 2px;
  padding: 2px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--text-primary) 8%, transparent);
}
.attach-seg-item {
  position: relative;
  display: block;
  min-width: 0;
}
.attach-seg-text {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 24px;
  padding: 0 4px;
  border-radius: 6px;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease, box-shadow 0.12s ease;
}
.attach-seg-item:hover .attach-seg-text {
  color: var(--text-primary);
}
.attach-sr:checked + .attach-seg-text {
  background: var(--control-bg);
  color: var(--text-primary);
  box-shadow: 0 0 0 var(--hairline) var(--control-border), var(--clickable-shadow);
}
.attach-sr:focus-visible + .attach-seg-text {
  box-shadow: var(--focus-ring), var(--clickable-shadow);
}

/* "Only entries with attachments": a switch, shown on and locked while a source or suffix implies it. */
.attach-switch-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 26px;
  padding: 0 2px;
  font-size: 12.5px;
  cursor: pointer;
  user-select: none;
}
.attach-switch-row.locked {
  cursor: default;
}
.attach-switch-text {
  flex: 1 1 auto;
  min-width: 0;
}
.attach-switch-count {
  color: var(--text-secondary);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.attach-switch {
  position: relative;
  flex: 0 0 auto;
  width: 30px;
  height: 18px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--text-primary) 22%, transparent);
  transition: background 0.15s ease, opacity 0.15s ease;
}
.attach-switch::after {
  content: "";
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
  transition: transform 0.15s ease;
}
.attach-sr:checked + .attach-switch {
  background: var(--accent);
}
.attach-sr:checked + .attach-switch::after {
  transform: translateX(12px);
}
.attach-sr:focus-visible + .attach-switch {
  box-shadow: var(--focus-ring);
}
.attach-sr:disabled + .attach-switch {
  opacity: 0.55;
}

/* Suffixes */
.attach-exts {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.attach-ext-list {
  flex: 1 1 auto;
  min-height: 0;
  margin: 0 -6px;
  padding: 0 6px;
  overflow-y: auto;
}
.attach-ext {
  position: relative;
  display: flex;
  align-items: center;
  gap: 9px;
  min-height: 30px;
  padding: 0 8px;
  border-radius: 6px;
  font-size: 12.5px;
  cursor: pointer;
  user-select: none;
}
.attach-ext:hover {
  background: color-mix(in srgb, var(--text-primary) 6%, transparent);
}
.attach-ext.on {
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}
.attach-ext.zero .attach-ext-name,
.attach-ext.zero .attach-ext-count {
  opacity: 0.6;
}
.attach-check {
  flex: 0 0 auto;
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  border: var(--hairline) solid var(--control-border);
  border-radius: 4px;
  background: var(--control-bg);
  color: #fff;
  transition: background 0.12s ease, border-color 0.12s ease;
}
.attach-check svg {
  width: 11px;
  height: 11px;
  opacity: 0;
  transform: scale(0.7);
  transition: opacity 0.12s ease, transform 0.12s ease;
}
.attach-sr:checked ~ .attach-check {
  border-color: var(--accent);
  background: var(--accent);
}
.attach-sr:checked ~ .attach-check svg {
  opacity: 1;
  transform: none;
}
.attach-sr:focus-visible ~ .attach-check {
  box-shadow: var(--focus-ring);
}
.attach-ext-name {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  font-family: var(--font-mono);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.attach-ext-name.plain {
  color: var(--text-secondary);
  font-family: inherit;
  font-size: 12.5px;
}
/* How common the suffix is among the entries in scope, next to its exact count. */
.attach-ext-bar {
  flex: 0 0 40px;
  height: 4px;
  border-radius: 2px;
  background: color-mix(in srgb, var(--text-primary) 9%, transparent);
  overflow: hidden;
}
.attach-ext-bar::after {
  content: "";
  display: block;
  width: calc(var(--share, 0) * 100%);
  height: 100%;
  border-radius: inherit;
  background: color-mix(in srgb, var(--accent) 55%, transparent);
  transition: background 0.12s ease;
}
.attach-ext.on .attach-ext-bar::after {
  background: var(--accent);
}
.attach-ext-count {
  flex: 0 0 auto;
  min-width: 24px;
  color: var(--text-secondary);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  text-align: right;
}
.attach-none {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 18px 8px 8px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}
.attach-none svg {
  width: 22px;
  height: 22px;
  opacity: 0.55;
}

.attach-foot {
  flex: 0 0 auto;
  padding: 8px 14px;
  border-top: var(--hairline) solid var(--border);
  background: color-mix(in srgb, var(--text-primary) 3%, transparent);
  color: var(--text-secondary);
  font-size: 11.5px;
  font-variant-numeric: tabular-nums;
}

@media (prefers-reduced-motion: reduce) {
  .attach-pop-enter-active,
  .attach-pop-leave-active,
  .attach-chevron,
  .attach-seg-text,
  .attach-switch,
  .attach-switch::after,
  .attach-check,
  .attach-check svg,
  .attach-ext-bar::after {
    transition: none;
  }
}
</style>
