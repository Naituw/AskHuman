import { computed, reactive, shallowRef, watch, type ComputedRef, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
export interface DiffRow { text: string; kind: "add" | "delete" | "context" | "meta" | "hunk"; old: string | null; new: string | null }
export interface ParsedDiff { sections: { title: string | null; rows: DiffRow[] }[]; notice: string | null }
export type PreviewContent = { kind: "markdown"; text: string; html: string }
  | { kind: "diff"; text: string; parsed: ParsedDiff }
  | { kind: "text"; text: string }
  | { kind: "image"; url: string; width: number; height: number }
  | { kind: "native"; imageCount?: number | null }
  | { kind: "unavailable"; reason: string };
export interface ReadingState {
  raw: boolean;
  scroll: { preview: { top: number; left: number }; raw: { top: number; left: number } };
  image: { zoom: "fit" | number; top: number; left: number };
}
export function newReadingState(): ReadingState {
  return { raw: false, scroll: { preview: { top: 0, left: 0 }, raw: { top: 0, left: 0 } }, image: { zoom: "fit", top: 0, left: 0 } };
}
interface Loaded { requestId: string; index: number; generation: number; content: PreviewContent }
const CACHE_BYTES = 64 * 1024 * 1024;
export function useAttachmentContent(requestId: ComputedRef<string>, index: Ref<number | null>) {
  const previewContent = shallowRef<PreviewContent | null>(null);
  const previewLoading = shallowRef(false);
  const states = new Map<number, ReadingState>();
  const cache = new Map<number, { content: PreviewContent; bytes: number }>();
  let budget = 0, generation = 0, disposed = false;
  function readingState(i: number): ReadingState {
    if (!states.has(i)) states.set(i, reactive(newReadingState())); return states.get(i)!;
  }
  const currentReadingState = computed(() => index.value === null ? null : readingState(index.value));
  function cacheContent(i: number, content: PreviewContent) {
    // Include the raw text, parsed model and base64 payload, counting JS strings as UTF-16.
    const bytes = JSON.stringify(content).length * 2;
    while (cache.size && budget + bytes > CACHE_BYTES) {
      const key = cache.keys().next().value!; budget -= cache.get(key)!.bytes; cache.delete(key);
    }
    if (bytes <= CACHE_BYTES) { cache.set(i, { content, bytes }); budget += bytes; }
  }
  const stopWatch = watch([requestId, index], async ([id, i], [oldId]) => {
    const token = ++generation;
    if (id !== oldId) { states.clear(); cache.clear(); budget = 0; }
    previewContent.value = null;
    previewLoading.value = i !== null;
    // Invalidate queued backend work even when closing without another file read.
    if (i === null) { previewLoading.value = false; void invoke("popup_preview_cancel_read", { requestId: id, generation: token }).catch(() => {}); return; }
    const hit = cache.get(i);
    if (hit) {
      cache.delete(i); cache.set(i, hit); previewContent.value = hit.content; previewLoading.value = false;
      void invoke("popup_preview_cancel_read", { requestId: id, generation: token }).catch(() => {}); return;
    }
    try {
      const loaded = await invoke<Loaded>("popup_preview_read", { requestId: id, index: i, generation: token });
      if (disposed || token !== generation || index.value !== i || requestId.value !== id
          || loaded.requestId !== id || loaded.index !== i || loaded.generation !== token) return;
      cacheContent(i, loaded.content); previewContent.value = loaded.content;
    } catch {
      if (!disposed && token === generation && index.value === i && requestId.value === id)
        previewContent.value = { kind: "unavailable", reason: "readFailed" };
    } finally { if (!disposed && token === generation) previewLoading.value = false; }
  });
  function disposeContent() {
    disposed = true; generation++; stopWatch(); cache.clear(); states.clear(); previewContent.value = null;
  }
  return { previewContent, previewLoading, currentReadingState, readingState, disposeContent };
}
