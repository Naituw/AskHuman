// Attachment preview for the History window (docs/specs/popup-attachment-preview-panel.md §10).
//
// The window has one preview controller for whichever entry is on screen. Its owner id is `history:<entry id>`:
// the backend resolves the files from the history record by that id, so no path ever comes from this side. The
// panel, the reading state and the native system preview are the Popup's own, driven through useAttachments.
import { computed, inject, onBeforeUnmount, onMounted, provide, ref, shallowRef, watch, type InjectionKey, type Ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { createLocalImageLoader } from "../../lib/localImageUrls";
import type { FileAttachment, HistoryEntry, ImageAttachment } from "../../lib/types";
import type { FindScope } from "../popup/attachmentFind";
import type { AttachmentPanelContext } from "../popup/attachmentPanelContext";
import { HISTORY_LAYOUT_EVENT } from "../popup/useAttachmentPreview";
import { useAttachments } from "../popup/useAttachments";

/** Owner ids of history entries start with this; the backend tells them from Popup request ids by it. */
export const HISTORY_OWNER_PREFIX = "history:";

export function historyOwnerId(entryId: string | null | undefined): string {
  return `${HISTORY_OWNER_PREFIX}${entryId ?? ""}`;
}

/** Below this width of the main region the entry list steps aside, leaving the detail and the panel. */
export const LIST_COLLAPSE_BELOW = 560;

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/**
 * Reply images as the preview controller sees them. The history keeps paths, so `data` carries the path and
 * `replyResolve` maps it to a display URL. The objects are kept per (question, position, path): the controller
 * tracks the selected image by identity, so a reload of the same entry must hand back the same objects.
 */
function replyImagesOf(entry: HistoryEntry | null, cache: Map<string, ImageAttachment>): ImageAttachment[][] {
  if (!entry) return [];
  return entry.answers.map((answer, q) => answer.images.map((path, i) => {
    const key = `${q}\u0000${i}\u0000${path}`;
    let image = cache.get(key);
    if (!image) { image = { data: path, mediaType: "", filename: fileName(path) }; cache.set(key, image); }
    return image;
  }));
}

interface NativeKey { requestId: string; index: number; key: string; metaKey: boolean }

export function useHistoryPreview(deps: {
  /** The entry on screen, or null. */
  entry: Readonly<Ref<HistoryEntry | null>>;
  /** A window-level overlay (the delete confirmation) is up: keys and the native view yield to it. */
  blocked: Readonly<Ref<boolean>>;
}) {
  // The preview follows this, not `deps.entry`: it moves to a new entry only after the panel has collapsed.
  const owner = shallowRef<HistoryEntry | null>(deps.entry.value);
  const ownerId = computed(() => historyOwnerId(owner.value?.id));
  const attachments = computed<FileAttachment[]>(() => owner.value?.message.files ?? []);
  const replyCache = new Map<string, ImageAttachment>();
  const replyImages = shallowRef<ImageAttachment[][]>(replyImagesOf(owner.value, replyCache));
  const loader = createLocalImageLoader();
  let disposed = false;

  const attach = useAttachments({
    attachments,
    requestId: ownerId,
    layoutEvent: HISTORY_LAYOUT_EVENT,
    replyImages,
    replyResolve: image => loader.load(image.data),
    replyImageElement: (q, i) => document.querySelector<HTMLElement>(`.thumb-preview[data-question-index="${q}"][data-image-index="${i}"]`),
  });

  // Another entry means another owner id. Collapse first, while the panel still belongs to the old owner, so
  // that the window releases its layout under the id it was opened with; the new entry always starts closed.
  // (A reloaded copy of the same entry just swaps the record: the open preview stays where it is.)
  watch(deps.entry, next => {
    if (next?.id !== owner.value?.id) {
      if (attach.previewOpen.value) attach.stopPreview();
      owner.value = next;
      replyCache.clear();
      loader.reset();
    } else owner.value = next;
    replyImages.value = replyImagesOf(next, replyCache);
  }, { flush: "sync" });

  function onKeydown(e: KeyboardEvent) {
    if (deps.blocked.value) return;
    attach.handleAttachmentKey(e);
  }

  // The native system preview owns the keyboard while it is focused; the backend forwards the few keys the
  // page cares about. Escape closes the panel and ⌘W closes the window, as everywhere else.
  let unlistenNativeKey: UnlistenFn | undefined;
  async function initNativeKeys() {
    const off = await listen<NativeKey>("popup-preview-native-key", event => {
      const key = event.payload;
      if (disposed || deps.blocked.value) return;
      if (key.requestId !== ownerId.value || key.index !== attach.previewIndex.value || attach.previewContent.value?.kind !== "native") return;
      if (key.key === "w" && key.metaKey) void getCurrentWebviewWindow().close();
      else if (key.key === "Escape") attach.handleAttachmentKey(new KeyboardEvent("keydown", { key: "Escape", cancelable: true }));
    });
    if (disposed) off(); else unlistenNativeKey = off;
  }

  onMounted(() => {
    window.addEventListener("keydown", onKeydown);
    attach.initAttachmentPreviewListeners().catch(error => console.error("History preview listeners failed", error));
    initNativeKeys().catch(error => console.error("History preview key listener failed", error));
  });
  onBeforeUnmount(() => {
    disposed = true;
    window.removeEventListener("keydown", onKeydown);
    unlistenNativeKey?.();
    attach.disposeAttachments();
    loader.dispose();
  });

  // The panel renders from this. History has no in-page find yet, so find stays off and inert.
  const panel: AttachmentPanelContext = {
    ...attach,
    ownerId,
    popupActive: ref(true),
    nativePreviewBlocked: deps.blocked,
    submitWithBareEnter: ref(false),
    findSupported: false,
    findActive: ref(false),
    findScope: ref<FindScope>("question"),
    openFind: () => {},
    refreshFind: () => {},
    registerAttachmentFind: () => {},
  };

  return { attach, panel, ownerId };
}

export type HistoryPreview = ReturnType<typeof useHistoryPreview>;

export const HistoryPreviewKey: InjectionKey<HistoryPreview> = Symbol("history-preview");

export function provideHistoryPreview(preview: HistoryPreview): HistoryPreview {
  provide(HistoryPreviewKey, preview);
  return preview;
}

export function useHistoryPreviewContext(): HistoryPreview {
  const preview = inject(HistoryPreviewKey);
  if (!preview) throw new Error("history preview not provided");
  return preview;
}
