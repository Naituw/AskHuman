import { computed, nextTick, ref, watch, type Ref, type ComputedRef } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface PreviewLayout {
  revision: number;
  side: "closed" | "right" | "inside";
  mainWidth: number;
  mainHeight: number;
}

export function useAttachmentPreview(requestId: ComputedRef<string>, active?: Readonly<Ref<boolean>>, delegate?: (open: boolean, mainExtent?: number) => Promise<PreviewLayout>) {
  const previewIndex = ref<number | null>(null);
  const previewLayout = ref<PreviewLayout>({ revision: 0, side: "closed", mainWidth: 560, mainHeight: 620 });
  const previewOpen = computed(() => previewIndex.value !== null);
  const previewTransition = ref<PreviewLayout | null>(null);
  let version = 0;
  let queue: Promise<unknown> = Promise.resolve();
  let unlisten: UnlistenFn | undefined;
  let disposed = false;

  function applyLayout(layout: PreviewLayout) {
    if (active && !active.value) return;
    if (!disposed && layout.revision >= previewLayout.value.revision) previewLayout.value = layout;
  }
  function layout(open: boolean, mainExtent?: number) {
    if (delegate) {
      void delegate(open, mainExtent).then(applyLayout).catch(error => console.error("Attachment layout failed", error));
      return;
    }
    const intent = { requestId: requestId.value, open, mainExtent: mainExtent ?? null, version: ++version };
    queue = queue.catch(() => {}).then(async () => {
      if (disposed || (active && !active.value)) return;
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
    if (!finalizing) layout(false);
  }
  async function initPreviewLayout() {
    const off = await listen<PreviewLayout>("popup-preview-layout", e => applyLayout(e.payload));
    if (disposed) off(); else unlisten = off;
  }
  const stopActive = active && !delegate ? watch(active, value => { if (value) layout(previewOpen.value); }) : undefined;
  function disposePreview() {
    stopActive?.();
    disposed = true;
    previewIndex.value = null;
    previewTransition.value = null;
    unlisten?.();
  }
  function resizePreview(mainExtent: number) { if (previewOpen.value) layout(true, mainExtent); }
  return { previewIndex, previewLayout, previewOpen, previewTransition, showPreview, stopPreview, initPreviewLayout, disposePreview, resizePreview };
}
