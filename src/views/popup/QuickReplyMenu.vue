<script setup lang="ts">
// The footer's "Quick reply" button and its menu (docs/specs/special-replies.md D19–D21): "away mode…"
// (or "end away mode" on an escalated question) and "stop using AskHuman in this session…". The two
// rows only arm a directive that waits above the footer until Send; the keyboard rules live in
// useQuickReply (⌘/Ctrl+K, ↑↓, Enter, Esc).
import { nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { primaryShortcutLabel } from "../../lib/platform";
import { usePopupContext } from "./context";

const { t } = useI18n();
const {
  quickReplyAvailable,
  quickReplyOpen,
  quickReplyIndex,
  quickReplyItems,
  toggleQuickReply,
  chooseQuickReply,
  closeQuickReply,
  submitting,
} = usePopupContext();

const shortcut = primaryShortcutLabel("k");
const menuEl = ref<HTMLElement | null>(null);

// Roving focus: the highlighted row owns the focus, so Enter and the screen reader follow the arrows.
watch([quickReplyOpen, quickReplyIndex], async ([open, index]) => {
  if (!open) return;
  await nextTick();
  menuEl.value?.querySelectorAll<HTMLElement>(".qr-item")[index as number]?.focus({ preventScroll: true });
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
      role="menu"
      :aria-label="t('popup.quickReply.menu')"
      @paste.stop
      @focusout="onFocusOut"
    >
      <div class="qr-head">
        <span>{{ t("popup.quickReply.menu") }}</span>
        <span class="qr-head-hint">{{ shortcut }}</span>
      </div>
      <button
        v-for="(item, i) in quickReplyItems"
        :key="item.id"
        class="qr-item"
        :class="{ hover: i === quickReplyIndex }"
        type="button"
        role="menuitem"
        :data-item="item.id"
        @mousemove="quickReplyIndex = i"
        @click="chooseQuickReply(item.id)"
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
    </div>
  </div>
</template>
