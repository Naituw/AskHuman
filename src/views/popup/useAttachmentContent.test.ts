import { computed, nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
import { useAttachmentContent } from "./useAttachmentContent";
function deferred() { let resolve!: (value: unknown) => void; const promise = new Promise(r => resolve = r); return { promise, resolve }; }
function loaded(index: number, generation: number, text: string, requestId = "one") { return { requestId, index, generation, content: { kind: "text", text } }; }
describe("attachment reader ownership and reading state", () => {
  beforeEach(() => { invoke.mockReset(); });
  it("discards a slow previous file and preserves the current one", async () => {
    const first = deferred(), second = deferred();
    invoke.mockImplementation((command, args) => command.endsWith("read") && command !== "popup_preview_cancel_read" ? (args.index === 0 ? first.promise : second.promise) : Promise.resolve());
    const index = ref<number | null>(null); const state = useAttachmentContent(computed(() => "one"), index);
    index.value = 0; await nextTick(); index.value = 1; await nextTick();
    second.resolve(loaded(1, 2, "current")); await nextTick();
    expect(state.previewContent.value).toEqual({ kind: "text", text: "current" });
    first.resolve(loaded(0, 1, "old")); await nextTick();
    expect(state.previewContent.value).toEqual({ kind: "text", text: "current" }); state.disposeContent();
  });
  it("closing invalidates pending reads and cannot reopen the content", async () => {
    const pending = deferred(); invoke.mockReturnValue(pending.promise);
    const index = ref<number | null>(null); const state = useAttachmentContent(computed(() => "one"), index);
    index.value = 0; await nextTick(); index.value = null; await nextTick();
    expect(invoke).toHaveBeenLastCalledWith("popup_preview_cancel_read", { requestId: "one", generation: 2 });
    pending.resolve(loaded(0, 1, "late")); await nextTick();
    expect(state.previewContent.value).toBeNull(); expect(state.previewLoading.value).toBe(false); state.disposeContent();
  });
  it("keeps independent source, preview and image state through close and cache reuse", async () => {
    invoke.mockImplementation((command, args) => Promise.resolve(command === "popup_preview_read" ? loaded(args.index, args.generation, "cached") : undefined));
    const index = ref<number | null>(null); const state = useAttachmentContent(computed(() => "one"), index);
    index.value = 0; await nextTick(); await nextTick();
    state.readingState(0).raw = true; state.readingState(0).scroll.raw.top = 700; state.readingState(0).scroll.preview.top = 120;
    state.readingState(0).image.zoom = 2; index.value = null; await nextTick(); index.value = 0; await nextTick();
    expect(state.currentReadingState.value?.raw).toBe(true); expect(state.currentReadingState.value?.scroll.preview.top).toBe(120);
    expect(state.currentReadingState.value?.scroll.raw.top).toBe(700); expect(state.currentReadingState.value?.image.zoom).toBe(2);
    expect(invoke.mock.calls.filter(([c]) => c === "popup_preview_read")).toHaveLength(1); state.disposeContent();
  });
  it("resets cache and reading state for another Popup request", async () => {
    invoke.mockImplementation((command, args) => Promise.resolve(command === "popup_preview_read" ? loaded(args.index, args.generation, args.requestId, args.requestId) : undefined));
    const request = ref("one"), index = ref<number | null>(null); const state = useAttachmentContent(computed(() => request.value), index);
    index.value = 0; await nextTick(); await nextTick(); state.readingState(0).raw = true;
    request.value = "two"; await nextTick(); await nextTick();
    expect(state.currentReadingState.value?.raw).toBe(false); expect(state.previewContent.value).toEqual({ kind: "text", text: "two" }); state.disposeContent();
  });
  it("disposal rejects late results and releases all content", async () => {
    const pending = deferred(); invoke.mockReturnValue(pending.promise);
    const index = ref<number | null>(null); const state = useAttachmentContent(computed(() => "one"), index);
    index.value = 0; await nextTick(); state.disposeContent(); pending.resolve(loaded(0, 1, "late")); await nextTick();
    expect(state.previewContent.value).toBeNull();
  });
});
