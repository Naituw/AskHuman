// The divider between a window's main region and the attachment preview panel, shared by the Popup and the
// History window: pointer drag with one resize per animation frame, and the style that freezes the main region
// while the window itself is being resized for the panel.
import { onBeforeUnmount } from "vue";
import type { PreviewLayout } from "./useAttachmentPreview";

/** Width of the draggable divider column, in CSS pixels. Keep in step with `DIVIDER` in popup_preview_geometry.rs. */
export const PREVIEW_DIVIDER_WIDTH = 6;

export function usePreviewDivider(deps: {
  /** Current width of the main region. */
  mainWidth: () => number;
  /** `useAttachmentPreview().resizePreview`: the window clamps and publishes the resulting layout. */
  resize: (mainExtent: number, finished?: boolean) => void;
}) {
  let stopDivider: (() => void) | undefined;
  onBeforeUnmount(() => stopDivider?.());
  function beginDivider(event: PointerEvent) {
    if (event.button !== 0) return;
    stopDivider?.();
    const target = event.currentTarget as HTMLElement;
    const start = event.clientX;
    const extent = deps.mainWidth();
    target.setPointerCapture(event.pointerId);
    let frame = 0;
    let latest = extent;
    const move = (e: PointerEvent) => {
      latest = extent + (e.clientX - start);
      if (!frame) frame = requestAnimationFrame(() => { frame = 0; deps.resize(latest, false); });
    };
    const cleanup = () => {
      cancelAnimationFrame(frame);
      target.removeEventListener("pointermove", move);
      target.removeEventListener("lostpointercapture", end);
      stopDivider = undefined;
    };
    const end = () => { cleanup(); deps.resize(latest, true); };
    stopDivider = cleanup;
    target.addEventListener("pointermove", move);
    target.addEventListener("lostpointercapture", end);
  }
  return { beginDivider };
}

/**
 * While the window is resized for the panel the main region keeps the size it had, so its content does not
 * reflow underneath the animation; the frozen frame is the one `popup_preview_prepare` reports.
 */
export function frozenMainStyle(transition: PreviewLayout | null) {
  if (!transition || transition.side === "closed") return undefined;
  return { position: "absolute" as const, width: `${transition.mainWidth}px`, height: `${transition.mainHeight}px`, left: "0", top: "0" };
}
