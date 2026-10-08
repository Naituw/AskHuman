<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { desktopPlatform } from "../../lib/platform";
import { openPath } from "../../lib/ipc";
import { usePopupContext } from "./context";
import { useNativeAttachmentPreview } from "./useNativeAttachmentPreview";
import AttachmentDiffPreview from "./AttachmentDiffPreview.vue";
import AttachmentImagePreview from "./AttachmentImagePreview.vue";
import AttachmentMarkdownContent from "../../components/AttachmentMarkdownContent.vue";
import FindBar from "./FindBar.vue";
import { useAttachmentFind } from "./useAttachmentFind";
const { t } = useI18n();
const { previewFile, previewIndex, attachments, showPreview, stopPreview, openFile, openPreviewFile, primaryBrowser, browserOpening, browserActionError, browserErrorIndex, showPreviewMenu, revealFile,
  popupActive, previewContent, previewLoading, currentReadingState, onAttachmentDragStart, previewActionError, request, nativePreviewBlocked, previewTransition, previewLayout, submitWithBareEnter, registerNativePreviewSync,
  findActive, findScope, openFind, refreshFind, registerAttachmentFind } = usePopupContext();
const nativeBody = ref<HTMLElement | null>(null);
const { nativeFailed, syncNativePreview } = useNativeAttachmentPreview({
  requestId: computed(() => request.value?.id ?? ""), index: previewIndex, element: nativeBody,
  active: computed(() => popupActive.value && previewContent.value?.kind === "native"),
  blocked: computed(() => nativePreviewBlocked.value || !!previewTransition.value),
  bareEnter: submitWithBareEnter, revision: computed(() => previewLayout.value.revision),
});
registerNativePreviewSync?.(syncNativePreview);
onBeforeUnmount(() => registerNativePreviewSync?.(null));
const body = ref<HTMLElement | null>(null);
const top = ref(0);
const imageFailed = ref(false);
const actionError = ref(false);
const canToggle = computed(() => previewContent.value?.kind === "markdown" || previewContent.value?.kind === "diff");
const raw = computed(() => !!(canToggle.value && currentReadingState.value?.raw));
const reason = computed(() => nativeFailed.value ? "unsupported" : imageFailed.value ? "imageFailed" : previewContent.value?.kind === "unavailable" ? previewContent.value.reason : null);
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
const { adapter, diffQuery, diffCaseSensitive, diffCurrent } = useAttachmentFind({
  requestId: computed(() => request.value?.id ?? ""), index: previewIndex, content: previewContent,
  loading: previewLoading, raw, active: popupActive,
  enabled: computed(() => findActive.value && findScope.value === "attachment"),
  body, nativeBody, failed: reason, restoreScroll, syncNative: syncNativePreview, setTop: value => { top.value = value; },
});
registerAttachmentFind(adapter);
onBeforeUnmount(() => registerAttachmentFind(null));
// Save before changing the active index; the renderer is mounted only for the current file.
watch(previewIndex, (_, old) => {
  if (old !== null && body.value) {
    const state = ctxState(old); state.scroll[state.raw ? "raw" : "preview"] = { top: body.value.scrollTop, left: body.value.scrollLeft };
  }
  imageFailed.value = false; actionError.value = false;
}, { flush: "sync" });
const { readingState: ctxState } = usePopupContext();
watch(previewContent, () => { if (popupActive.value) void restoreScroll(); }, { flush: "post" });
watch(popupActive, value => { if (value) void restoreScroll(); else saveScroll(); }, { flush: "sync" });
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
    <!-- Explicit regions preserve the pointer behavior of controls and preview content. -->
    <header class="attachment-preview-toolbar" data-tauri-drag-region>
      <span class="attachment-preview-title" data-tauri-drag-region>
        <span class="attachment-preview-name" :title="previewFile?.name" data-tauri-drag-region>{{ previewFile?.name }}</span>
        <span class="attachment-preview-mode-slot" data-tauri-drag-region>
          <button v-if="canToggle" type="button" class="attachment-preview-mode" @click="toggleRaw">{{ t(raw ? 'popup.preview.rendered' : 'popup.preview.raw') }}</button>
        </span>
      </span>
      <div class="attachment-preview-actions" data-tauri-drag-region>
        <span class="attachment-preview-count" data-tauri-drag-region>{{ (previewIndex ?? 0) + 1 }} / {{ attachments.length }}</span>
        <button type="button" :disabled="previewIndex === 0" :aria-label="t('popup.prev')" @click="showPreview((previewIndex ?? 0) - 1)">‹</button>
        <button type="button" :disabled="previewIndex === attachments.length - 1" :aria-label="t('popup.next')" @click="showPreview((previewIndex ?? 0) + 1)">›</button>
        <button type="button" class="attachment-preview-find" :title="t('popup.find.attachmentShortcut')" :aria-label="t('popup.find.attachmentShortcut')" @click="openFind(true, 'attachment')">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><circle cx="10" cy="10" r="7"/><path d="m15 15 6 6"/></svg>
        </button>
        <button type="button" class="attachment-preview-open" :disabled="previewLoading || (primaryBrowser && browserOpening)" :aria-busy="primaryBrowser && browserOpening" @click="openPreviewFile()">{{ t(primaryBrowser ? 'popup.preview.openBrowser' : 'popup.preview.open') }}</button>
        <button type="button" :aria-label="t('popup.preview.more')" @click="showPreviewMenu()">⋯</button>
        <button type="button" :aria-label="t('popup.preview.close')" @click="stopPreview()">×</button>
      </div>
    </header>
    <FindBar v-if="findActive && findScope === 'attachment'" />
    <p v-if="previewActionError" class="attachment-preview-notice" role="alert">{{ t('popup.preview.actionFailed') }}</p>
    <p v-if="browserActionError && browserErrorIndex === previewIndex" class="attachment-preview-notice" role="alert">{{ t(`popup.preview.${browserActionError}`) }} <button type="button" @click="previewFile && openFile(previewFile)">{{ t('popup.preview.openOriginal') }}</button></p>
    <div v-if="previewLoading" class="attachment-preview-status" role="status" data-tauri-drag-region>{{ t('common.loading') }}</div>
    <div v-else-if="reason" class="attachment-preview-status" role="status" data-tauri-drag-region>
      <strong>{{ previewFile?.name }}</strong><p>{{ t(`popup.preview.${reason}`) }}</p>
      <div class="attachment-preview-file-actions" data-tauri-drag-region>
        <button type="button" @click="previewFile && openFile(previewFile)">{{ t('popup.preview.open') }}</button>
        <button type="button" @click="reveal()">{{ t(`popup.preview.reveal.${desktopPlatform}`) }}</button>
      </div>
      <p v-if="actionError" role="alert">{{ t('popup.preview.actionFailed') }}</p>
    </div>
    <AttachmentImagePreview v-else-if="previewContent?.kind === 'image' && currentReadingState" :key="previewIndex ?? 0"
      v-bind="previewContent" :name="previewFile?.name ?? ''" :state="currentReadingState.image"
      @drag="previewFile && onAttachmentDragStart(previewFile, $event)" @error="imageFailed = true" />
    <template v-else-if="previewContent?.kind === 'native'">
      <p v-if="previewContent.imageCount" class="attachment-preview-notice">{{ t('popup.preview.multipleImages', { n: previewContent.imageCount }) }}</p>
      <div ref="nativeBody" class="attachment-preview-body attachment-preview-native" :aria-label="t('popup.preview.system')"></div>
    </template>
    <div v-else ref="body" class="attachment-preview-body" tabindex="0" @scroll.passive="saveScroll()" @markdown-content-updated="refreshFind">
      <pre v-if="previewContent && (raw || previewContent.kind === 'text')" class="attachment-preview-text">{{ 'text' in previewContent ? previewContent.text : '' }}</pre>
      <AttachmentMarkdownContent v-else-if="previewContent?.kind === 'markdown'" :key="previewIndex ?? 0" class="attachment-preview-markdown" @click="markdownClick" :html="previewContent.html" />
      <template v-else-if="previewContent?.kind === 'diff'">
        <p v-if="previewContent.parsed.notice" class="attachment-preview-notice">{{ t('popup.preview.diffPlain') }}</p>
        <AttachmentDiffPreview :parsed="previewContent.parsed" :viewport="body" :top="top" :query="diffQuery" :case-sensitive="diffCaseSensitive" :current="diffCurrent" />
      </template>
    </div>
  </aside>
</template>
