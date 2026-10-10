import { computed, reactive, shallowRef, watch, type ComputedRef, type Ref } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
interface ImageResource { scope: string; byteLength: number; displayPixels: number }
export interface DiffRow { text: string; kind: "add" | "delete" | "context" | "meta" | "hunk"; old: string | null; new: string | null }
export interface ParsedDiff { sections: { title: string | null; rows: DiffRow[] }[]; notice: string | null }
export type PreviewContent = { kind: "markdown"; text: string; html: string }
  | { kind: "diff"; text: string; parsed: ParsedDiff }
  | { kind: "text"; text: string }
  | { kind: "image"; url: string; width: number; height: number; resource?: ImageResource }
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
const CACHE_PIXELS = 80_000_000;
const CACHE_ENTRIES = 128;
const cache = new Map<string, { content: PreviewContent; bytes: number; pixels: number }>();
const imageOwners = new Map<string, number>();
let budget = 0, pixels = 0;
function retain(content: PreviewContent | null) {
  if (content?.kind !== "image" || !content.resource) return;
  const scope = content.resource.scope;
  imageOwners.set(scope, (imageOwners.get(scope) ?? 0) + 1);
}
function release(content: PreviewContent | null) {
  if (content?.kind !== "image" || !content.resource) return;
  const scope = content.resource.scope;
  const count = imageOwners.get(scope) ?? 0;
  if (count > 1) { imageOwners.set(scope, count - 1); return; }
  imageOwners.delete(scope);
  void invoke("local_image_release_scope", { scope }).catch(() => {});
}
function evict(key: string) {
  const value = cache.get(key);
  if (!value) return;
  budget -= value.bytes; pixels -= value.pixels; cache.delete(key); release(value.content);
}
function evictRequest(id: string) {
  for (const key of cache.keys()) if (key.startsWith(`${id}:`)) evict(key);
}
export function useAttachmentContent(requestId: ComputedRef<string>, index: Ref<number | null>, active?: Readonly<Ref<boolean>>) {
  const previewContent = shallowRef<PreviewContent | null>(null);
  const previewLoading = shallowRef(false);
  const states = new Map<number, ReadingState>();
  let generation = 0, disposed = false;
  function setContent(content: PreviewContent | null) {
    if (previewContent.value === content) return;
    retain(content); release(previewContent.value); previewContent.value = content;
  }
  function readingState(i: number): ReadingState {
    if (!states.has(i)) states.set(i, reactive(newReadingState())); return states.get(i)!;
  }
  const currentReadingState = computed(() => index.value === null ? null : readingState(index.value));
  function cacheContent(i: number, content: PreviewContent) {
    const key = `${requestId.value}:${i}`;
    evict(key);
    const image = content.kind === "image" ? content.resource : undefined;
    // URL length is not image cost: charge binary storage and decoded display pixels separately.
    const bytes = JSON.stringify(content).length * 2 + (image?.byteLength ?? 0);
    const cost = image?.displayPixels ?? 0;
    if (bytes > CACHE_BYTES || cost > CACHE_PIXELS) return;
    while (cache.size && (budget + bytes > CACHE_BYTES || pixels + cost > CACHE_PIXELS || cache.size >= CACHE_ENTRIES)) {
      evict(cache.keys().next().value!);
    }
    retain(content); cache.set(key, { content, bytes, pixels: cost }); budget += bytes; pixels += cost;
  }
  const stopWatch = watch([requestId, index, () => active?.value ?? true], async ([id, i, visible], [oldId]) => {
    const token = ++generation;
    if (id !== oldId) { states.clear(); evictRequest(oldId); }
    setContent(null);
    previewLoading.value = visible && i !== null;
    // Invalidate queued backend work even when closing without another file read.
    if (!visible || i === null) { void invoke("popup_preview_cancel_read", { requestId: id, generation: token }).catch(() => {}); return; }
    const key = `${id}:${i}`;
    const hit = cache.get(key);
    if (hit) {
      cache.delete(key); cache.set(key, hit); setContent(hit.content); previewLoading.value = false;
      void invoke("popup_preview_cancel_read", { requestId: id, generation: token }).catch(() => {}); return;
    }
    let loaded: Loaded | undefined;
    let adopted = false;
    try {
      loaded = await invoke<Loaded>("popup_preview_read", { requestId: id, index: i, generation: token });
      if (disposed || token !== generation || index.value !== i || requestId.value !== id
          || loaded.requestId !== id || loaded.index !== i || loaded.generation !== token) return;
      const content = loaded.content.kind === "image" && loaded.content.resource
        ? { ...loaded.content, url: convertFileSrc(loaded.content.url, "askhuman-image") }
        : loaded.content;
      setContent(content); adopted = true; cacheContent(i, content);
    } catch {
      if (!disposed && token === generation && index.value === i && requestId.value === id)
        setContent({ kind: "unavailable", reason: "readFailed" });
    } finally {
      if (loaded && !adopted) release(loaded.content);
      if (!disposed && token === generation) previewLoading.value = false;
    }
  });
  function disposeContent() {
    disposed = true; generation++; stopWatch(); evictRequest(requestId.value); states.clear(); setContent(null);
  }
  return { previewContent, previewLoading, currentReadingState, readingState, disposeContent };
}
