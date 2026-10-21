// What the attachment preview panel needs from the window that hosts it. The Popup provides it from its own
// context (createPopupContext); the History window builds one from its preview controller. The panel itself
// never learns which window it is in, only the owner id it reads, previews natively and searches by.
import { inject, provide, type InjectionKey, type Ref } from "vue";
import type { useAttachments } from "./useAttachments";
import type { AttachmentFindAdapter, FindScope } from "./attachmentFind";

type Attachments = ReturnType<typeof useAttachments>;

export type AttachmentPanelContext = Pick<Attachments,
  | "previewFile" | "previewIndex" | "previewName" | "previewCount" | "previewKey" | "isReplyPreview"
  | "showPreview" | "stopPreview" | "openFile" | "openPreviewFile" | "primaryBrowser" | "browserOpening"
  | "browserActionError" | "browserErrorIndex" | "showPreviewMenu" | "revealFile"
  | "panelContent" | "panelLoading" | "panelReadingState" | "readingState"
  | "onAttachmentDragStart" | "previewActionError" | "previewTransition" | "previewLayout"
> & {
  /** Owner id of the attachments on show: the request id in a Popup, `history:<entry id>` in History. */
  ownerId: Readonly<Ref<string>>;
  popupActive: Readonly<Ref<boolean>>;
  /** A window-level overlay is up: the native preview view floats above the page and has to step aside. */
  nativePreviewBlocked: Readonly<Ref<boolean>>;
  submitWithBareEnter: Readonly<Ref<boolean>>;
  registerNativePreviewSync?: (sync: (() => Promise<void>) | null) => void;
  /** In-page find over the preview. A host without it hides the entry point and never turns it on. */
  findSupported: boolean;
  findActive: Readonly<Ref<boolean>>;
  findScope: Readonly<Ref<FindScope>>;
  openFind: (prefillFromSelection?: boolean, target?: FindScope, nativeSelection?: string) => void;
  refreshFind: () => void;
  registerAttachmentFind: (adapter: AttachmentFindAdapter | null) => void;
};

export const AttachmentPanelKey: InjectionKey<AttachmentPanelContext> = Symbol("attachment-panel-ctx");

export function provideAttachmentPanel(ctx: AttachmentPanelContext): AttachmentPanelContext {
  provide(AttachmentPanelKey, ctx);
  return ctx;
}

export function useAttachmentPanelContext(): AttachmentPanelContext {
  const ctx = inject(AttachmentPanelKey);
  if (!ctx) throw new Error("attachment panel context not provided");
  return ctx;
}
