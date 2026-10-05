<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { desktopPlatform } from "../../lib/platform";
import { openPath } from "../../lib/ipc";
import { usePopupContext } from "./context";
import AttachmentDiffPreview from "./AttachmentDiffPreview.vue";
import AttachmentImagePreview from "./AttachmentImagePreview.vue";
const { t } = useI18n();
const { previewFile, previewIndex, attachments, showPreview, stopPreview, openFile, showPreviewMenu, revealFile,
  previewContent, previewLoading, currentReadingState, onAttachmentDragStart, previewActionError } = usePopupContext();
const body = ref<HTMLElement | null>(null);
const top = ref(0);
const imageFailed = ref(false);
const actionError = ref(false);
const canToggle = computed(() => previewContent.value?.kind === "markdown" || previewContent.value?.kind === "diff");
const raw = computed(() => canToggle.value && currentReadingState.value?.raw);
const reason = computed(() => imageFailed.value ? "imageFailed" : previewContent.value?.kind === "unavailable" ? previewContent.value.reason : null);
let restoreVersion = 0;
function saveScroll(index = previewIndex.value, mode: "raw" | "preview" = raw.value ? "raw" : "preview") {
  if (index === null || !body.value || !currentReadingState.value) return;
  currentReadingState.value.scroll[mode] = { top: body.value.scrollTop, left: body.value.scrollLeft };
  top.value = body.value.scrollTop;
}
async function restoreScroll() {
  const version = ++restoreVersion;
  await nextTick(); await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  if (version !== restoreVersion || !body.value || !currentReadingState.value) return;
  const saved = currentReadingState.value.scroll[raw.value ? "raw" : "preview"];
  body.value.scrollTop = saved.top; body.value.scrollLeft = saved.left; top.value = body.value.scrollTop;
}
function toggleRaw() { saveScroll(); if (currentReadingState.value) currentReadingState.value.raw = !currentReadingState.value.raw; void restoreScroll(); }
// Save before changing the active index; the renderer is mounted only for the current file.
watch(previewIndex, (_, old) => {
  if (old !== null && body.value) {
    const state = ctxState(old); state.scroll[state.raw ? "raw" : "preview"] = { top: body.value.scrollTop, left: body.value.scrollLeft };
  }
  imageFailed.value = false; actionError.value = false;
}, { flush: "sync" });
const { readingState: ctxState } = usePopupContext();
watch(previewContent, () => { void restoreScroll(); }, { flush: "post" });
onBeforeUnmount(() => { saveScroll(); restoreVersion++; });
async function reveal() { actionError.value = false; try { await revealFile(); } catch { actionError.value = true; } }
function markdownClick(event: MouseEvent) {
  const anchor = (event.target as Element)?.closest("a"); if (!anchor) return;
  event.preventDefault(); const href = anchor.getAttribute("href") ?? "";
  if (href.startsWith('#')) {
    let id: string;
    try { id = decodeURIComponent(href.slice(1)); } catch { return; }
    const target = body.value?.querySelector(`[id="${CSS.escape(id)}"]`); target?.scrollIntoView();
  } else if (/^(https?:\/\/|mailto:)/i.test(href)) { void openPath(href).catch(() => {}); }
}
</script>
<template>
  <aside class="attachment-preview" :aria-label="t('popup.preview.title')">
    <header class="attachment-preview-toolbar">
      <span class="attachment-preview-title">
        <span class="attachment-preview-name" :title="previewFile?.name">{{ previewFile?.name }}</span>
        <span class="attachment-preview-mode-slot">
          <button v-if="canToggle" type="button" class="attachment-preview-mode" @click="toggleRaw">{{ t(raw ? 'popup.preview.rendered' : 'popup.preview.raw') }}</button>
        </span>
      </span>
      <div class="attachment-preview-actions">
        <span class="attachment-preview-count">{{ (previewIndex ?? 0) + 1 }} / {{ attachments.length }}</span>
        <button type="button" :disabled="previewIndex === 0" :aria-label="t('popup.prev')" @click="showPreview((previewIndex ?? 0) - 1)">‹</button>
        <button type="button" :disabled="previewIndex === attachments.length - 1" :aria-label="t('popup.next')" @click="showPreview((previewIndex ?? 0) + 1)">›</button>
        <button type="button" @click="previewFile && openFile(previewFile)">{{ t('popup.preview.open') }}</button>
        <button type="button" :aria-label="t('popup.preview.more')" @click="showPreviewMenu()">⋯</button>
        <button type="button" :aria-label="t('popup.preview.close')" @click="stopPreview()">×</button>
      </div>
    </header>
    <p v-if="previewActionError" class="attachment-preview-notice" role="alert">{{ t('popup.preview.actionFailed') }}</p>
    <div v-if="previewLoading" class="attachment-preview-status" role="status">{{ t('common.loading') }}</div>
    <div v-else-if="reason" class="attachment-preview-status" role="status">
      <strong>{{ previewFile?.name }}</strong><p>{{ t(`popup.preview.${reason}`) }}</p>
      <div class="attachment-preview-file-actions">
        <button type="button" @click="previewFile && openFile(previewFile)">{{ t('popup.preview.open') }}</button>
        <button type="button" @click="reveal()">{{ t(`popup.preview.reveal.${desktopPlatform}`) }}</button>
      </div>
      <p v-if="actionError" role="alert">{{ t('popup.preview.actionFailed') }}</p>
    </div>
    <AttachmentImagePreview v-else-if="previewContent?.kind === 'image' && currentReadingState" :key="previewIndex ?? 0"
      v-bind="previewContent" :name="previewFile?.name ?? ''" :state="currentReadingState.image"
      @drag="previewFile && onAttachmentDragStart(previewFile, $event)" @error="imageFailed = true" />
    <div v-else ref="body" class="attachment-preview-body" tabindex="0" @scroll.passive="saveScroll()">
      <pre v-if="previewContent && (raw || previewContent.kind === 'text')" class="attachment-preview-text">{{ 'text' in previewContent ? previewContent.text : '' }}</pre>
      <article v-else-if="previewContent?.kind === 'markdown'" class="attachment-preview-markdown" @click="markdownClick" v-html="previewContent.html"></article>
      <template v-else-if="previewContent?.kind === 'diff'">
        <p v-if="previewContent.parsed.notice" class="attachment-preview-notice">{{ t('popup.preview.diffPlain') }}</p>
        <AttachmentDiffPreview :parsed="previewContent.parsed" :viewport="body" :top="top" />
      </template>
    </div>
  </aside>
</template>
