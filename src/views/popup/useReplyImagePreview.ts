import { computed, reactive, ref, shallowRef, watch, type Ref } from "vue";
import type { ImageAttachment } from "../../lib/types";
import { newReadingState, type PreviewContent } from "./useAttachmentContent";

/**
 * `resolve` turns an image into the URL to display. The Popup keeps reply images as data URLs and leaves it
 * out; the History window stores file paths and resolves them to display URLs on demand.
 */
export function useReplyImagePreview(
  imagesByQ: Readonly<Ref<ImageAttachment[][]>>,
  enabled: Readonly<Ref<boolean>>,
  onListChange: (index: number | null) => void,
  resolve?: (image: ImageAttachment) => Promise<string>,
) {
  const questionIndex = ref<number | null>(null);
  const image = shallowRef<ImageAttachment | null>(null);
  const items = computed(() => questionIndex.value === null ? [] : imagesByQ.value[questionIndex.value] ?? []);
  const index = computed(() => image.value ? items.value.indexOf(image.value) : -1);
  const content = shallowRef<PreviewContent | null>(null);
  const loading = ref(false);
  let lastIndex = 0;
  let nextKey = 0;
  let selecting = false;
  let records = new WeakMap<ImageAttachment, {
    key: number; state: ReturnType<typeof newReadingState>; url?: string; width?: number; height?: number;
  }>();
  function record(value: ImageAttachment) {
    let entry = records.get(value);
    if (!entry) { entry = { key: ++nextKey, state: reactive(newReadingState()) }; records.set(value, entry); }
    return entry;
  }
  const readingState = computed(() => image.value ? record(image.value).state : null);
  const key = computed(() => image.value ? `reply-${record(image.value).key}` : "reply");
  function select(q: number, i: number): boolean {
    const value = imagesByQ.value[q]?.[i];
    if (!value) return false;
    selecting = true;
    questionIndex.value = q; image.value = value; lastIndex = i;
    selecting = false;
    return true;
  }
  // Keep the selected image by identity when earlier thumbnails are removed.
  const stopItems = watch(() => [...items.value], values => {
    if (!image.value || selecting) return;
    const current = values.indexOf(image.value);
    if (current >= 0) { lastIndex = current; onListChange(current); return; }
    lastIndex = Math.min(lastIndex, values.length - 1);
    image.value = values[lastIndex] ?? null;
    onListChange(image.value ? lastIndex : null);
  }, { flush: "sync" });
  const stopLoad = watch([image, enabled], ([value, visible], _, cleanup) => {
    content.value = null; loading.value = false;
    if (!value || !visible) return;
    const entry = record(value);
    const show = () => { content.value = { kind: "image", url: entry.url ?? value.data, width: entry.width!, height: entry.height! }; };
    if (entry.width && entry.height && (entry.url || !resolve)) { show(); return; }
    loading.value = true;
    let probe: HTMLImageElement | undefined;
    let cancelled = false;
    cleanup(() => { cancelled = true; if (probe) { probe.onload = null; probe.onerror = null; probe.src = ""; } });
    const failed = () => { if (!cancelled) { loading.value = false; content.value = { kind: "unavailable", reason: "imageFailed" }; } };
    const start = (url: string) => {
      if (cancelled) return;
      const image = probe = new Image();
      image.onload = () => {
        if (cancelled) return;
        loading.value = false;
        if (!image.naturalWidth || !image.naturalHeight) { content.value = { kind: "unavailable", reason: "imageFailed" }; return; }
        entry.url = url; entry.width = image.naturalWidth; entry.height = image.naturalHeight; show();
      };
      image.onerror = failed;
      image.src = url;
    };
    // Without a resolver the probe starts synchronously, exactly as before.
    if (resolve) resolve(value).then(start, failed); else start(value.data);
  }, { flush: "sync" });
  function reset() { image.value = null; questionIndex.value = null; records = new WeakMap(); }
  function dispose() { stopItems(); stopLoad(); reset(); content.value = null; loading.value = false; }
  return { questionIndex, image, items, index, content, loading, readingState, key, select, reset, dispose };
}
