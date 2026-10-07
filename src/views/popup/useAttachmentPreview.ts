import { computed, nextTick, ref, watch, type Ref, type ComputedRef } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface PreviewLayout {
  revision: number;
  side: "closed" | "right" | "inside";
  mainWidth: number;
  mainHeight: number;
}
export type PreviewDelegate = (open: boolean, mainExtent?: number, finished?: boolean) => Promise<PreviewLayout>;

export function useAttachmentPreview(requestId: ComputedRef<string>, active?: Readonly<Ref<boolean>>, delegate?: PreviewDelegate) {
  const previewIndex = ref<number | null>(null);
  const previewLayout = ref<PreviewLayout>({ revision: 0, side: "closed", mainWidth: 560, mainHeight: 620 });
  const previewOpen = computed(() => previewIndex.value !== null);
  const previewTransition = ref<PreviewLayout | null>(null);
  let version = 0;
  let queue: Promise<unknown> = Promise.resolve();
  let unlisten: UnlistenFn | undefined;
  let disposed = false;
  let resizeEpoch = 0;
  let resizeQueued = false;
  let latestResize: { requestId: string; mainExtent: number; finished: boolean; epoch: number } | undefined;

  function cancelResize() { resizeEpoch++; latestResize = undefined; }
  function usableResize(intent: NonNullable<typeof latestResize>) {
    return !disposed && previewOpen.value && (!active || active.value)
      && intent.requestId === requestId.value && intent.epoch === resizeEpoch;
  }

  function applyLayout(layout: PreviewLayout) {
    if (active && !active.value) return;
    if (!disposed && layout.revision >= previewLayout.value.revision) previewLayout.value = layout;
  }
  function layout(open: boolean, mainExtent?: number) {
    const intent = { requestId: requestId.value, open, mainExtent: mainExtent ?? null, version: ++version };
    queue = queue.catch(() => {}).then(async () => {
      if (disposed || (active && !active.value)) return;
      if (delegate) { applyLayout(await delegate(open)); return; }
      try {
        for (let attempt = 0; attempt < 3; attempt++) {
          if (mainExtent === undefined) {
            const prepared = await invoke<PreviewLayout>("popup_preview_prepare", intent);
            if (disposed || (active && !active.value)) return;
            previewTransition.value = prepared;
            await nextTick();
            // Two animation frames guarantee the fixed main region has painted before native resize.
            await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
            await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          }
          if (disposed || (active && !active.value)) return;
          try {
            applyLayout(await invoke<PreviewLayout>("popup_preview_layout", intent));
            return;
          } catch (error) {
            if (String(error) !== "preview geometry changed" || attempt === 2) throw error;
          }
        }
      } finally { previewTransition.value = null; }
    });
    queue.catch((error) => console.error("Attachment layout failed", error));
  }
  function showPreview(index: number) {
    const wasOpen = previewOpen.value;
    previewIndex.value = index;
    if (!wasOpen) layout(true);
  }
  function stopPreview(finalizing = false) {
    if (!previewOpen.value) return;
    previewIndex.value = null;
    cancelResize();
    if (!finalizing) layout(false);
  }
  async function initPreviewLayout() {
    const off = await listen<PreviewLayout>("popup-preview-layout", e => applyLayout(e.payload));
    if (disposed) off(); else unlisten = off;
  }
  const stopActive = active ? watch(active, value => {
    if (!value) cancelResize();
    else if (!delegate) layout(previewOpen.value);
  }, { flush: "sync" }) : undefined;
  const stopRequest = watch(requestId, cancelResize, { flush: "sync" });
  function disposePreview() {
    stopActive?.();
    stopRequest();
    cancelResize();
    disposed = true;
    previewIndex.value = null;
    previewTransition.value = null;
    unlisten?.();
  }
  function resizePreview(mainExtent: number, finished = true) {
    if (!previewOpen.value || disposed || (active && !active.value) || !Number.isFinite(mainExtent)) return;
    latestResize = { requestId: requestId.value, mainExtent, finished, epoch: resizeEpoch };
    if (resizeQueued) return;
    resizeQueued = true;
    // One operation owns the queue. Pointer frames replace its next position while IPC is
    // in flight; they must never enqueue a replay of the whole gesture.
    queue = queue.catch(() => {}).then(async () => {
      try {
        while (latestResize) {
          const intent = latestResize;
          latestResize = undefined;
          if (!usableResize(intent)) continue;
          const result = delegate
            ? await delegate(true, intent.mainExtent, intent.finished)
            : await invoke<PreviewLayout>("popup_preview_layout", {
              requestId: intent.requestId, open: true, mainExtent: intent.mainExtent,
              finished: intent.finished, version: ++version,
            });
          if (usableResize(intent)) applyLayout(result);
        }
      } finally { resizeQueued = false; }
    });
    queue.catch(error => console.error("Attachment resize failed", error));
  }
  return { previewIndex, previewLayout, previewOpen, previewTransition, showPreview, stopPreview, initPreviewLayout, disposePreview, resizePreview };
}
