import { onScopeDispose, ref, watch, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { isMac } from "../../lib/platform";
// Shared across panel remounts, so a delayed update cannot resurrect a closed native view.
let version = 0;
export function useNativeAttachmentPreview(deps: {
  requestId: Ref<string>; index: Ref<number | null>; element: Ref<HTMLElement | null>;
  active: Ref<boolean>; blocked: Ref<boolean>; bareEnter: Ref<boolean>; revision: Ref<number>;
}) {
  const failed = ref(false);
  let observer: ResizeObserver | undefined;
  let frame = 0;
  let disposed = false;
  function update(show: boolean) {
    if (!isMac) return;
    const token = ++version;
    const rect = show ? deps.element.value?.getBoundingClientRect() : null;
    const visible = !!rect && rect.width > 0 && rect.height > 0;
    void invoke("popup_preview_native", {
      requestId: deps.requestId.value, index: visible ? deps.index.value : null, version: token,
      rect: visible ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null,
      bareEnter: deps.bareEnter.value,
    }).catch(() => { if (!disposed && token === version && visible) { failed.value = true; update(false); } });
  }
  function schedule() {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      frame = 0;
      if (!disposed) update(deps.active.value && !deps.blocked.value && !failed.value);
    });
  }
  const stopElement = watch(deps.element, element => {
    observer?.disconnect();
    if (element && isMac) { observer = new ResizeObserver(schedule); observer.observe(element); }
    schedule();
  }, { flush: "post" });
  const stopActive = watch([deps.requestId, deps.index, deps.active], () => {
    failed.value = false;
    // Hide synchronously on a file switch; the replacement DOM is measured after its next paint.
    update(false); schedule();
  }, { flush: "sync" });
  const stopBlocked = watch(deps.blocked, blocked => {
    if (blocked) { cancelAnimationFrame(frame); update(false); } else schedule();
  }, { flush: "sync" });
  const stopLayout = watch([deps.revision, deps.bareEnter], schedule, { flush: "post" });
  onScopeDispose(() => {
    disposed = true; cancelAnimationFrame(frame); observer?.disconnect();
    stopElement(); stopActive(); stopBlocked(); stopLayout(); update(false);
  });
  return { nativeFailed: failed };
}
