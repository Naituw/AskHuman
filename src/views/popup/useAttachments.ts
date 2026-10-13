// Popup attachments and reply images: activation, keyboard focus and preview routing.
import { computed, ref, watch, type ComputedRef, type Ref } from "vue";
import { fileIconDataUrl, openPath } from "../../lib/ipc";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import type { FileAttachment, ImageAttachment } from "../../lib/types";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useAttachmentContent } from "./useAttachmentContent";
import { useAttachmentPreview, type PreviewDelegate } from "./useAttachmentPreview";
import { useReplyImagePreview } from "./useReplyImagePreview";
import fallbackDragIcon from "../../../src-tauri/icons/32x32.png?inline";

// The native drag plugin requires PNG even when the attached image is another format.
const FALLBACK_ICON = fallbackDragIcon;

export function useAttachments(deps: {
  attachments: ComputedRef<FileAttachment[]>;
  requestId: ComputedRef<string>;
  active?: Readonly<Ref<boolean>>;
  layout?: PreviewDelegate;
  replyImages?: Readonly<Ref<ImageAttachment[][]>>;
  replyImageElement?: (qIndex: number, index: number) => HTMLElement | null;
}) {
  const { attachments } = deps;
  const preview = useAttachmentPreview(deps.requestId, deps.active, deps.layout);
  const isReplyPreview = ref(false);
  const selectedFile = computed(() => isReplyPreview.value ? null : preview.previewIndex.value);
  const content = useAttachmentContent(deps.requestId, selectedFile, deps.active);
  const reply = useReplyImagePreview(deps.replyImages ?? ref([]),
    computed(() => isReplyPreview.value && preview.previewOpen.value && (deps.active?.value ?? true)),
    index => {
      if (!isReplyPreview.value || !preview.previewOpen.value) return;
      if (index === null) stopPreview(); else preview.showPreview(index);
    });
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
    // Respond immediately to the first click; additional clicks in the same gesture do nothing.
    if (Date.now() < suppressClickUntil || (event && event.detail > 1)) return;
    focusedFile.value = index;
    if (selectedFile.value === index) preview.stopPreview();
    else { isReplyPreview.value = false; preview.showPreview(index); }
  }
  function showPreview(index: number) {
    if (isReplyPreview.value) {
      if (reply.questionIndex.value !== null && reply.select(reply.questionIndex.value, index)) preview.showPreview(index);
    } else if (index >= 0 && index < attachments.value.length) preview.showPreview(index);
  }
  function showQuestionPreview(index: number) {
    if (index < 0 || index >= attachments.value.length) return;
    isReplyPreview.value = false; preview.showPreview(index);
  }
  function selectReplyImage(qIndex: number, index: number, event?: MouseEvent) {
    if (event && event.detail > 1) return;
    if (isReplyPreview.value && preview.previewOpen.value && reply.questionIndex.value === qIndex && reply.index.value === index) stopPreview();
    else if (reply.select(qIndex, index)) { isReplyPreview.value = true; preview.showPreview(index); }
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
    const replyTarget = isReplyPreview.value && reply.questionIndex.value !== null
      ? deps.replyImageElement?.(reply.questionIndex.value, reply.index.value)
      : null;
    preview.stopPreview(finalizing);
    if (returnFocus) { if (replyTarget) replyTarget.focus(); else if (i !== null) attRefs.value[i]?.focus(); }
  }
  function onBackgroundClick(_: MouseEvent) { /* Reading remains active while answering. */ }
  function handleAttachmentKey(e: KeyboardEvent): boolean {
    if (e.isComposing || e.keyCode === 229 || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return false;
    if (e.key === "Escape" && preview.previewOpen.value) {
      stopPreview(); e.preventDefault(); return true;
    }
    const target = e.target instanceof Element ? e.target : document.activeElement;
    const replyItem = target?.closest<HTMLElement>(".thumb-preview");
    if (replyItem) {
      const q = Number(replyItem.dataset.questionIndex), i = Number(replyItem.dataset.imageIndex);
      const count = deps.replyImages?.value[q]?.length ?? 0;
      if (!count || !["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp"].includes(e.key)) return false;
      const next = Math.max(0, Math.min(count - 1, i + (["ArrowRight", "ArrowDown"].includes(e.key) ? 1 : -1)));
      if (reply.select(q, next)) {
        isReplyPreview.value = true; preview.showPreview(next);
        deps.replyImageElement?.(q, next)?.focus();
      }
      e.preventDefault(); return true;
    }
    const item = target?.closest(".attachment");
    const toolbar = target?.closest(".attachment-preview-toolbar");
    if (!item && !toolbar) return false;
    const i = item ? Number((item as HTMLElement).dataset.attachmentIndex) : preview.previewIndex.value;
    const count = item || !isReplyPreview.value ? attachments.value.length : reply.items.value.length;
    if (i === null || !Number.isInteger(i) || i < 0 || i >= count) return false;
    focusedFile.value = i;
    if (item && e.key === "Enter") openFile(attachments.value[i]);
    else if (item && e.key === " ") selectFile(i);
    else if (e.key === "ArrowRight" || (item && e.key === "ArrowDown")) {
      const next = Math.min(i + 1, count - 1);
      if (item) { focusedFile.value = next; attRefs.value[next]?.focus(); }
      if (item) showQuestionPreview(next); else showPreview(next);
    } else if (e.key === "ArrowLeft" || (item && e.key === "ArrowUp")) {
      const next = Math.max(i - 1, 0);
      if (item) { focusedFile.value = next; attRefs.value[next]?.focus(); }
      if (item) showQuestionPreview(next); else showPreview(next);
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
      if (!disposed && (deps.active?.value ?? true) && e.payload.requestId === deps.requestId.value) showQuestionPreview(e.payload.index);
    });
    if (disposed) off(); else menuListener = off;
    const copyOff = await listen<{ requestId: string; index: number }>("popup-preview-copy-path", e => {
      if (!disposed && (deps.active?.value ?? true) && e.payload.requestId === deps.requestId.value && attachments.value[e.payload.index])
        void navigator.clipboard.writeText(attachments.value[e.payload.index].path).catch(() => {});
    });
    if (disposed) copyOff(); else { const previous = menuListener; menuListener = () => { previous?.(); copyOff(); }; }
    const errorOff = await listen<{ requestId: string; index: number }>("popup-preview-action-failed", e => {
      if (!disposed && (deps.active?.value ?? true) && e.payload.requestId === deps.requestId.value && e.payload.index === selectedFile.value)
        previewActionError.value = true;
    });
    if (disposed) errorOff(); else { const previous = menuListener; menuListener = () => { previous?.(); errorOff(); }; }
    const browserOff = await listen<{ requestId: string; index: number }>("popup-preview-open-browser", e => {
      if (!disposed && (deps.active?.value ?? true) && e.payload.requestId === deps.requestId.value) void openBrowser(e.payload.index);
    });
    if (disposed) browserOff(); else { const previous = menuListener; menuListener = () => { previous?.(); browserOff(); }; }
  }
  const stopReplyRequest = watch(deps.requestId, () => {
    if (isReplyPreview.value) preview.stopPreview(true);
    reply.reset(); isReplyPreview.value = false;
  }, { flush: "sync" });
  function disposeAttachments() { disposed = true; browserToken++; stopReplyRequest(); reply.dispose(); stopBrowserWatch(); stopErrorWatch(); menuListener?.(); content.disposeContent(); preview.disposePreview(); }
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
    isReplyPreview, selectReplyImage, replyPreviewQuestion: reply.questionIndex,
    selectedReplyImage: computed(() => isReplyPreview.value && preview.previewOpen.value ? reply.index.value : null),
    previewName: computed(() => isReplyPreview.value ? reply.image.value?.filename : previewFile.value?.name),
    previewCount: computed(() => isReplyPreview.value ? reply.items.value.length : attachments.value.length),
    previewKey: computed(() => isReplyPreview.value ? reply.key.value : `question-${selectedFile.value}`),
    panelContent: computed(() => isReplyPreview.value ? reply.content.value : content.previewContent.value),
    panelLoading: computed(() => isReplyPreview.value ? reply.loading.value : content.previewLoading.value),
    panelReadingState: computed(() => isReplyPreview.value ? reply.readingState.value : content.currentReadingState.value),
    thumbs, draggingOut, setAttRef, selectFile, openFile, openBrowser, openPreviewFile, primaryBrowser, browserOpening, browserActionError, browserErrorIndex, browserErrorFile, onBackgroundClick,
    handleAttachmentKey, formatBytes, loadThumbs, loadDragIcons, onAttachmentContextMenu, onAttachmentDragStart,
    initAttachmentPreviewListeners, disposeAttachments,
  };
}
