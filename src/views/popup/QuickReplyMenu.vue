<script setup lang="ts">
// The footer's "Quick reply" button and its menu (docs/specs/special-replies.md D17–D21, D27): the user's
// preset replies on top (one click sends), then "away mode…" (or "end away mode" on an escalated question)
// and "stop using AskHuman in this session…", then the link to the settings. The two directive rows only
// arm something that waits above the footer until Send; the keyboard rules live in useQuickReply
// (⌘/Ctrl+K, digits, ↑↓, Enter, Esc).
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { primaryShortcutLabel } from "../../lib/platform";
import { usePopupContext } from "./context";

const { t } = useI18n();
const {
  quickReplyAvailable,
  quickReplyOpen,
  quickReplyIndex,
  quickReplyItems,
  quickReplyPresetTargets,
  toggleQuickReply,
  chooseQuickReplyItem,
  closeQuickReply,
  submitting,
  isMulti,
} = usePopupContext();

const hasPresets = computed(() => quickReplyItems.value.some((item) => item.kind === "preset"));
/** What the header says on the right: what a click on a preset does, or the shortcut when there are none. */
const headHint = computed(() => {
  if (!hasPresets.value) return primaryShortcutLabel("k");
  const targets = quickReplyPresetTargets.value;
  if (targets === 0) return t("popup.quickReply.presetsDone");
  if (!isMulti.value) return t("popup.quickReply.presetsSend");
  return targets === 1
    ? t("popup.quickReply.presetsSendFillOne")
    : t("popup.quickReply.presetsSendFill", { n: targets });
});

const shortcut = primaryShortcutLabel("k");
const menuEl = ref<HTMLElement | null>(null);

// The menu hangs from the button's right edge. With several questions Previous / Next / Send sit on that
// button's right, so the button is left of centre and a full-width menu would run past the window's left edge:
// slide it back inside (the width already fits the window, see .qr-menu).
const WINDOW_MARGIN = 12;
const shiftX = ref(0);
async function keepMenuInside() {
  shiftX.value = 0;
  await nextTick();
  const box = menuEl.value?.getBoundingClientRect();
  if (!box || box.width === 0) return;
  shiftX.value = Math.max(0, WINDOW_MARGIN - box.left);
}
watch(quickReplyOpen, (open) => {
  if (open) void keepMenuInside();
  else shiftX.value = 0;
});

// Roving focus: the highlighted row owns the focus, so Enter and the screen reader follow the arrows.
watch([quickReplyOpen, quickReplyIndex], async ([open, index]) => {
  if (!open) return;
  await nextTick();
  const row = menuEl.value?.querySelectorAll<HTMLElement>(".qr-item")[index as number];
  row?.focus({ preventScroll: true });
  // A tall menu scrolls inside itself (see .qr-menu); keep the highlighted row in view.
  row?.scrollIntoView?.({ block: "nearest" });
});

// Tab (or a click on another control) moving the focus out of the menu closes it; the window losing
// focus (relatedTarget null) does not.
function onFocusOut(event: FocusEvent) {
  const next = event.relatedTarget;
  if (next instanceof Node && menuEl.value && !menuEl.value.contains(next)) closeQuickReply(false);
}
</script>

<template>
  <div v-if="quickReplyAvailable" class="qr-anchor" @click.stop>
    <div v-if="quickReplyOpen" class="qr-backdrop" @click="closeQuickReply()"></div>
    <button
      class="btn qr-trigger"
      :class="{ open: quickReplyOpen }"
      type="button"
      aria-haspopup="menu"
      :aria-expanded="quickReplyOpen"
      :disabled="submitting"
      :title="t('popup.quickReply.hint', { shortcut })"
      @click="toggleQuickReply()"
    >
      {{ t("popup.quickReply.button") }}
      <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="m7 15 5 5 5-5" />
        <path d="m7 9 5-5 5 5" />
      </svg>
    </button>

    <div
      v-if="quickReplyOpen"
      ref="menuEl"
      class="qr-menu"
      :style="shiftX ? { transform: `translateX(${shiftX}px)` } : undefined"
      role="menu"
      :aria-label="t('popup.quickReply.menu')"
      @paste.stop
      @focusout="onFocusOut"
    >
      <div class="qr-head">
        <span>{{ t("popup.quickReply.menu") }}</span>
        <span class="qr-head-hint">{{ headHint }}</span>
      </div>
      <template v-for="(item, i) in quickReplyItems" :key="item.id">
        <!-- A rule between the groups: presets, directives, the link to the settings. -->
        <div v-if="i > 0 && item.kind !== quickReplyItems[i - 1].kind" class="qr-sep" role="separator"></div>
        <button
          v-if="item.kind === 'preset'"
          class="qr-item qr-preset"
          :class="{ hover: i === quickReplyIndex, disabled: item.disabled }"
          type="button"
          role="menuitem"
          :aria-disabled="item.disabled"
          :data-item="item.id"
          @mousemove="!item.disabled && (quickReplyIndex = i)"
          @click="chooseQuickReplyItem(item)"
        >
          <span class="qr-item-text qr-preset-text" :title="item.text">{{ item.text }}</span>
          <kbd v-if="item.key" class="qr-key">{{ item.key }}</kbd>
        </button>
        <button
          v-else-if="item.kind === 'manage'"
          class="qr-item qr-manage"
          :class="{ hover: i === quickReplyIndex }"
          type="button"
          role="menuitem"
          data-item="manage"
          @mousemove="quickReplyIndex = i"
          @click="chooseQuickReplyItem(item)"
        >
          {{ t("popup.quickReply.manage") }}
        </button>
        <button
          v-else
          class="qr-item qr-directive"
          :class="{ hover: i === quickReplyIndex }"
          type="button"
          role="menuitem"
          :data-item="item.id"
          @mousemove="quickReplyIndex = i"
          @click="chooseQuickReplyItem(item)"
        >
          <svg v-if="item.id === 'mute'" class="qr-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5" />
            <path d="M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7" />
            <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
            <path d="m2 2 20 20" />
          </svg>
          <svg v-else class="qr-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
          </svg>
          <span class="qr-item-body">
            <span class="qr-item-text">{{ t(`popup.quickReply.item.${item.id}`) }}</span>
            <span class="qr-item-sub">{{ t(`popup.quickReply.item.${item.id}Sub`) }}</span>
          </span>
        </button>
      </template>
    </div>
  </div>
</template>
