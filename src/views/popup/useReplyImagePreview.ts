import { computed, reactive, ref, shallowRef, watch, type Ref } from "vue";
import type { ImageAttachment } from "../../lib/types";
import { newReadingState, type PreviewContent } from "./useAttachmentContent";

export function useReplyImagePreview(
  imagesByQ: Readonly<Ref<ImageAttachment[][]>>,
  enabled: Readonly<Ref<boolean>>,
  onListChange: (index: number | null) => void,
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
    key: number; state: ReturnType<typeof newReadingState>; width?: number; height?: number;
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
    const show = () => { content.value = { kind: "image", url: value.data, width: entry.width!, height: entry.height! }; };
    if (entry.width && entry.height) { show(); return; }
    loading.value = true;
    const probe = new Image();
    let cancelled = false;
    cleanup(() => { cancelled = true; probe.onload = null; probe.onerror = null; probe.src = ""; });
    probe.onload = () => {
      if (cancelled) return;
      loading.value = false;
      if (!probe.naturalWidth || !probe.naturalHeight) { content.value = { kind: "unavailable", reason: "imageFailed" }; return; }
      entry.width = probe.naturalWidth; entry.height = probe.naturalHeight; show();
    };
    probe.onerror = () => { if (!cancelled) { loading.value = false; content.value = { kind: "unavailable", reason: "imageFailed" }; } };
    probe.src = value.data;
  }, { flush: "sync" });
  function reset() { image.value = null; questionIndex.value = null; records = new WeakMap(); }
  function dispose() { stopItems(); stopLoad(); reset(); content.value = null; loading.value = false; }
  return { questionIndex, image, items, index, content, loading, readingState, key, select, reset, dispose };
}
