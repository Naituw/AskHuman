<script setup lang="ts">
// A request-scoped find bar below the active pane toolbar.
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const { t } = useI18n();
const {
  findScope,
  findStatus,
  previewOpen,
  previewFile,
  openFind,
  openFile,
  findTotal,
  findQuery,
  findCaseSensitive,
  findCountLabel,
  findNoMatch,
  findInputEl,
  closeFind,
  goFind,
  onFindQueryInput,
  toggleFindCase,
} = usePopupContext();

const inputEl = ref<HTMLInputElement | null>(null);
onMounted(() => {
  findInputEl.value = inputEl.value;
  void nextTick(() => { inputEl.value?.focus({ preventScroll: true }); inputEl.value?.select(); });
});
onBeforeUnmount(() => { if (findInputEl.value === inputEl.value) findInputEl.value = null; });
</script>

<template>
    <div
      class="popup-find-bar"
      role="search"
      :aria-label="t('popup.find.label')"
      @mousedown.stop
      @click.stop
    >
      <select class="popup-find-scope" :value="findScope" :aria-label="t('popup.find.scope')" @change="openFind(false, ($event.target as HTMLSelectElement).value as 'question' | 'attachment')">
        <option value="question">{{ t('popup.find.question') }}</option>
        <option value="attachment" :disabled="!previewOpen">{{ t('popup.find.attachment') }}</option>
      </select>
      <input
        ref="inputEl"
        class="popup-find-input"
        :class="{ empty: findNoMatch }"
        type="search"
        enterkeyhint="search"
        autocomplete="off"
        autocorrect="off"
        spellcheck="false"
        :placeholder="t(findScope === 'attachment' ? 'popup.find.attachmentPlaceholder' : 'popup.find.placeholder')"
        :aria-keyshortcuts="t('popup.find.ariaShortcut')"
        :value="findQuery"
        @input="onFindQueryInput(($event.target as HTMLInputElement).value)"
      />
      <span
        class="popup-find-count"
        :class="{ empty: findNoMatch }"
        aria-live="polite"
      >{{ findCountLabel }}</span>
      <button
        type="button"
        class="popup-find-btn"
        :title="t('popup.find.prev')"
        :aria-label="t('popup.find.prev')"
        :disabled="findStatus !== 'ready' || !findTotal"
        @click="goFind(-1)"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="m18 15-6-6-6 6" />
        </svg>
      </button>
      <button
        type="button"
        class="popup-find-btn"
        :title="t('popup.find.next')"
        :aria-label="t('popup.find.next')"
        :disabled="findStatus !== 'ready' || !findTotal"
        @click="goFind(1)"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>
      <button
        type="button"
        class="popup-find-btn popup-find-case"
        :class="{ active: findCaseSensitive }"
        :title="t('popup.find.caseSensitive')"
        :aria-label="t('popup.find.caseSensitive')"
        :aria-pressed="findCaseSensitive"
        :disabled="!['ready', 'searching'].includes(findStatus)"
        @click="toggleFindCase"
      >
        Aa
      </button>
      <button
        type="button"
        class="popup-find-btn"
        :title="t('popup.find.close')"
        :aria-label="t('popup.find.close')"
        @click="closeFind"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M18 6 6 18" />
          <path d="m6 6 12 12" />
        </svg>
      </button>
      <span v-if="findStatus !== 'ready'" class="popup-find-status" role="status">
        {{ t(`popup.find.status.${findStatus}`) }}
        <button v-if="!['loading', 'searching'].includes(findStatus)" type="button" @click="previewFile && openFile(previewFile)">{{ t('popup.preview.openOriginal') }}</button>
      </span>
    </div>
</template>
