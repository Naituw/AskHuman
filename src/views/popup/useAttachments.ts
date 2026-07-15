// 提问附带附件域（AI→人，仅展示）：选中态、缩略图/拖出图标、预览（Quick Look）、
// 键盘导航与右键菜单。
import { nextTick, ref, type ComputedRef } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  closePreview,
  fileIconDataUrl,
  openPath,
  previewAttachments,
  readImageDataUrl,
  showAttachmentMenu,
} from "../../lib/ipc";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import type { FileAttachment } from "../../lib/types";

export function useAttachments(deps: {
  attachments: ComputedRef<FileAttachment[]>;
}) {
  const { attachments } = deps;

  const selectedFile = ref<number | null>(null);
  const thumbs = ref<Record<string, string>>({});
  const dragIcons = ref<Record<string, string>>({});
  const attRefs = ref<HTMLElement[]>([]);
  const previewing = ref(false);
  const draggingOut = ref(false);
  let unlistenIndex: UnlistenFn | null = null;
  let unlistenFocus: UnlistenFn | null = null;
  let restoreAttachmentFocusOnClose = false;

  function setAttRef(el: Element | null, i: number) {
    if (el) attRefs.value[i] = el as HTMLElement;
  }

  function selectFile(index: number) {
    focusAttachment(index);
  }

  function openFile(file: FileAttachment) {
    openPath(file.path).catch(() => {});
  }

  function focusAttachment(index: number) {
    selectedFile.value = index;
    attRefs.value[index]?.focus();
  }

  function previewSelected(index: number) {
    focusAttachment(index);
    previewing.value = true;
    restoreAttachmentFocusOnClose = true;
    previewAttachments(
      attachments.value.map((f) => f.path),
      index
    ).catch(() => {});
  }

  function stopPreview() {
    if (!previewing.value) return;
    previewing.value = false;
    restoreAttachmentFocusOnClose = false;
    closePreview().catch(() => {});
  }

  function onBackgroundClick(e: MouseEvent) {
    if ((e.target as HTMLElement).closest(".attachment")) return;
    if (previewing.value) {
      restoreAttachmentFocusOnClose = false;
      return;
    }
    if (selectedFile.value !== null) selectedFile.value = null;
  }

  function handleAttachmentKey(e: KeyboardEvent): boolean {
    if (!attachments.value.length) return false;
    const i = selectedFile.value;
    if (i === null) return false;
    if (e.key === "Escape" && previewing.value) {
      stopPreview();
    } else if (e.key === "Enter") {
      openFile(attachments.value[i]);
    } else if (e.key === " ") {
      if (previewing.value) stopPreview();
      else previewSelected(i);
    } else if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      if (i < attachments.value.length - 1) focusAttachment(i + 1);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      if (i > 0) focusAttachment(i - 1);
    } else {
      return false;
    }
    e.preventDefault();
    return true;
  }

  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  async function loadThumbs() {
    for (const file of attachments.value) {
      if (!file.isImage || thumbs.value[file.path]) continue;
      try {
        thumbs.value[file.path] = await readImageDataUrl(file.path);
      } catch {
        /* 缩略图加载失败：退回通用图标，不阻断 */
      }
    }
  }

  async function loadDragIcons() {
    for (const file of attachments.value) {
      if (dragIcons.value[file.path]) continue;
      try {
        dragIcons.value[file.path] = await fileIconDataUrl(file.path);
      } catch {
        /* 取图标失败：拖出时回退用缩略图或不带预览 */
      }
    }
  }

  function onAttachmentContextMenu(file: FileAttachment, i: number, e: MouseEvent) {
    e.preventDefault();
    selectFile(i);
    showAttachmentMenu(file.path).catch((err) =>
      console.error("打开右键菜单失败", err)
    );
  }

  function onAttachmentDragStart(file: FileAttachment, e: DragEvent) {
    e.preventDefault();
    const icon = dragIcons.value[file.path] || thumbs.value[file.path] || "";
    draggingOut.value = true;
    startDrag({ item: [file.path], icon }, () => {
      setTimeout(() => (draggingOut.value = false), 300);
    }).catch((err) => {
      draggingOut.value = false;
      console.error("拖出附件失败", err);
    });
  }

  // 首帧后初始化：Quick Look 预览联动（切换选中项 / 预览关闭时找回焦点）。
  async function initAttachmentPreviewListeners() {
    unlistenIndex = await listen<number>("preview-index", (e) => {
      const i = e.payload;
      if (i >= 0 && i < attachments.value.length) {
        selectedFile.value = i;
        nextTick(() => {
          const el = attRefs.value[i];
          el?.focus();
          el?.scrollIntoView({ block: "nearest" });
        });
      }
    });
    unlistenFocus = await listen("preview-closed", () => {
      previewing.value = false;
      const i = selectedFile.value;
      const shouldRestore = restoreAttachmentFocusOnClose;
      restoreAttachmentFocusOnClose = false;
      if (shouldRestore && i !== null) {
        nextTick(() => attRefs.value[i]?.focus());
      }
    });
  }

  function disposeAttachments() {
    stopPreview();
    unlistenIndex?.();
    unlistenFocus?.();
  }

  return {
    selectedFile,
    thumbs,
    draggingOut,
    setAttRef,
    selectFile,
    openFile,
    stopPreview,
    onBackgroundClick,
    handleAttachmentKey,
    formatBytes,
    loadThumbs,
    loadDragIcons,
    onAttachmentContextMenu,
    onAttachmentDragStart,
    initAttachmentPreviewListeners,
    disposeAttachments,
  };
}
