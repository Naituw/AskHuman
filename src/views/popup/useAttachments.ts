// Question attachments: activation, keyboard focus, original-file actions and drag-out.
import { computed, ref, watch, type ComputedRef } from "vue";
import { fileIconDataUrl, openPath } from "../../lib/ipc";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import type { FileAttachment } from "../../lib/types";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useAttachmentContent } from "./useAttachmentContent";
import { useAttachmentPreview } from "./useAttachmentPreview";
import fallbackDragIcon from "../../../src-tauri/icons/32x32.png?inline";

// The native drag plugin requires PNG even when the attached image is another format.
const FALLBACK_ICON = fallbackDragIcon;

export function useAttachments(deps: {
  attachments: ComputedRef<FileAttachment[]>;
  requestId: ComputedRef<string>;
}) {
  const { attachments } = deps;
  const preview = useAttachmentPreview(deps.requestId);
  const selectedFile = preview.previewIndex;
  const content = useAttachmentContent(deps.requestId, selectedFile);
  let disposed = false;
  let menuListener: UnlistenFn | undefined;
  const focusedFile = ref<number | null>(null);
  const previewActionError = ref(false);
  const browserActionError = ref<string | null>(null);
  const browserErrorIndex = ref<number | null>(null);
  const browserOpening = ref(false);
  let browserToken = 0;
  const stopErrorWatch = watch([deps.requestId, selectedFile], () => {
    previewActionError.value = false; browserActionError.value = null; browserErrorIndex.value = null;
  });
  const stopBrowserWatch = watch(deps.requestId, () => { browserToken++; browserOpening.value = false; }, { flush: "sync" });
  const thumbs = ref<Record<string, string>>({});
  const dragIcons = ref<Record<string, string>>({});
  const attRefs = ref<HTMLElement[]>([]);
  const draggingOut = ref(false);
  let suppressClickUntil = 0;

  function setAttRef(el: Element | null, i: number) {
    if (el) attRefs.value[i] = el as HTMLElement;
  }
  function selectFile(index: number, event?: MouseEvent) {
    if (Date.now() < suppressClickUntil || (event && event.detail > 1)) return;
    focusedFile.value = index;
    if (selectedFile.value === index) preview.stopPreview();
    else preview.showPreview(index);
  }
  function showPreview(index: number) {
    if (index >= 0 && index < attachments.value.length) preview.showPreview(index);
  }
  function openFile(file: FileAttachment) { openPath(file.path).catch(() => {}); }
  const primaryBrowser = computed(() => content.previewContent.value?.kind === "markdown" && !content.currentReadingState.value?.raw);
  async function openBrowser(index = selectedFile.value) {
    if (index === null || !attachments.value[index] || disposed || browserOpening.value) return;
    const id = deps.requestId.value, token = ++browserToken;
    browserActionError.value = null; browserOpening.value = true;
    try {
      await invoke<void>("popup_preview_open_browser", { requestId: id, index });
    } catch (error) {
      if (!disposed && token === browserToken && id === deps.requestId.value) {
        browserErrorIndex.value = index;
        browserActionError.value = ["limit", "encoding", "readFailed", "unsupported"].includes(String(error)) ? String(error) : "browserFailed";
      }
    } finally { if (!disposed && token === browserToken) browserOpening.value = false; }
  }
  function openPreviewFile() {
    if (primaryBrowser.value) void openBrowser();
    else if (selectedFile.value !== null && attachments.value[selectedFile.value]) openFile(attachments.value[selectedFile.value]);
  }
  function stopPreview(finalizing = false) {
    const i = selectedFile.value;
    const returnFocus = !finalizing && document.activeElement?.closest(".attachment-preview");
    preview.stopPreview(finalizing);
    if (returnFocus && i !== null) attRefs.value[i]?.focus();
  }
  function onBackgroundClick(_: MouseEvent) { /* Reading remains active while answering. */ }
  function handleAttachmentKey(e: KeyboardEvent): boolean {
    if (e.isComposing || e.keyCode === 229 || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return false;
    if (e.key === "Escape" && preview.previewOpen.value) {
      stopPreview(); e.preventDefault(); return true;
    }
    const target = e.target instanceof Element ? e.target : document.activeElement;
    const item = target?.closest(".attachment");
    const toolbar = target?.closest(".attachment-preview-toolbar");
    if (!item && !toolbar) return false;
    const i = item ? Number((item as HTMLElement).dataset.attachmentIndex) : selectedFile.value;
    if (i === null || !Number.isInteger(i) || !attachments.value[i]) return false;
    focusedFile.value = i;
    if (item && e.key === "Enter") openFile(attachments.value[i]);
    else if (item && e.key === " ") selectFile(i);
    else if (e.key === "ArrowRight" || (item && e.key === "ArrowDown")) {
      const next = Math.min(i + 1, attachments.value.length - 1);
      if (item) { focusedFile.value = next; attRefs.value[next]?.focus(); }
      showPreview(next);
    } else if (e.key === "ArrowLeft" || (item && e.key === "ArrowUp")) {
      const next = Math.max(i - 1, 0);
      if (item) { focusedFile.value = next; attRefs.value[next]?.focus(); }
      showPreview(next);
    } else return false;
    e.preventDefault(); return true;
  }
  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }
  async function loadThumbs() {
    let budget = 0;
    for (const [index, file] of attachments.value.entries()) {
      if (disposed || budget >= 8 * 1024 * 1024) break;
      if (thumbs.value[file.path]) continue;
      try {
        const url = await invoke<string | null>("popup_preview_thumbnail", { requestId: deps.requestId.value, index });
        if (disposed) break;
        if (url && budget + url.length * 2 <= 8 * 1024 * 1024) { thumbs.value[file.path] = url; budget += url.length * 2; }
      } catch { /* Keep the generic icon. */ }
    }
  }
  async function loadDragIcons() {
    for (const file of attachments.value) {
      if (dragIcons.value[file.path]) continue;
      try { dragIcons.value[file.path] = await fileIconDataUrl(file.path); } catch { /* Use the valid PNG fallback. */ }
    }
  }
  function onAttachmentContextMenu(_file: FileAttachment, i: number, e: MouseEvent) {
    e.preventDefault(); focusedFile.value = i;
    showPreviewMenu(i);
  }
  function showPreviewMenu(index = selectedFile.value) {
    if (index !== null) invoke("popup_preview_menu", { requestId: deps.requestId.value, index }).catch(error => console.error("Attachment menu failed", error));
  }
  function revealFile(index = selectedFile.value) {
    if (index !== null) return invoke<void>("popup_preview_reveal", { requestId: deps.requestId.value, index });
  }
  async function initAttachmentPreviewListeners() {
    await preview.initPreviewLayout();
    const off = await listen<{ requestId: string; index: number }>("popup-preview-show", e => {
      if (!disposed && e.payload.requestId === deps.requestId.value) showPreview(e.payload.index);
    });
    if (disposed) off(); else menuListener = off;
    const copyOff = await listen<{ requestId: string; index: number }>("popup-preview-copy-path", e => {
      if (!disposed && e.payload.requestId === deps.requestId.value && attachments.value[e.payload.index])
        void navigator.clipboard.writeText(attachments.value[e.payload.index].path).catch(() => {});
    });
    if (disposed) copyOff(); else { const previous = menuListener; menuListener = () => { previous?.(); copyOff(); }; }
    const errorOff = await listen<{ requestId: string; index: number }>("popup-preview-action-failed", e => {
      if (!disposed && e.payload.requestId === deps.requestId.value && e.payload.index === selectedFile.value)
        previewActionError.value = true;
    });
    if (disposed) errorOff(); else { const previous = menuListener; menuListener = () => { previous?.(); errorOff(); }; }
    const browserOff = await listen<{ requestId: string; index: number }>("popup-preview-open-browser", e => {
      if (!disposed && e.payload.requestId === deps.requestId.value) void openBrowser(e.payload.index);
    });
    if (disposed) browserOff(); else { const previous = menuListener; menuListener = () => { previous?.(); browserOff(); }; }
  }
  function disposeAttachments() { disposed = true; browserToken++; stopBrowserWatch(); stopErrorWatch(); menuListener?.(); content.disposeContent(); preview.disposePreview(); }
  function onAttachmentDragStart(file: FileAttachment, e: DragEvent) {
    e.preventDefault();
    suppressClickUntil = Date.now() + 500;
    draggingOut.value = true;
    startDrag({ item: [file.path], icon: dragIcons.value[file.path] || FALLBACK_ICON }, () => {
      suppressClickUntil = Date.now() + 300;
      setTimeout(() => (draggingOut.value = false), 300);
    }).catch(error => { draggingOut.value = false; console.error("Attachment drag failed", error); });
  }
  const previewFile = computed(() => selectedFile.value === null ? null : attachments.value[selectedFile.value] ?? null);
  const browserErrorFile = computed(() => browserErrorIndex.value === null ? null : attachments.value[browserErrorIndex.value] ?? null);
  return {
    ...preview, ...content, showPreview, stopPreview, showPreviewMenu, revealFile, selectedFile, focusedFile, previewFile, previewActionError,
    thumbs, draggingOut, setAttRef, selectFile, openFile, openBrowser, openPreviewFile, primaryBrowser, browserOpening, browserActionError, browserErrorIndex, browserErrorFile, onBackgroundClick,
    handleAttachmentKey, formatBytes, loadThumbs, loadDragIcons, onAttachmentContextMenu, onAttachmentDragStart,
    initAttachmentPreviewListeners, disposeAttachments,
  };
}
