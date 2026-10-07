import { computed, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, (e: { payload: unknown }) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, fn: (e: { payload: unknown }) => void) => {
  mocks.listeners.set(name, fn); return () => mocks.listeners.delete(name);
}) }));
import { useAttachmentPreview, type PreviewLayout } from "./useAttachmentPreview";
const right: PreviewLayout = { revision: 1, side: "right", mainWidth: 560, mainHeight: 620 };
async function flush() { for (let i = 0; i < 20; i++) await Promise.resolve(); }
describe("preview resize paint handshake", () => {
  beforeEach(() => { vi.clearAllMocks(); mocks.listeners.clear(); });
  afterEach(() => vi.unstubAllGlobals());
  it("coalesces a quick reversal and saves the final position after a held delegated resize", async () => {
    const delegate = vi.fn(async (_open: boolean, extent?: number, _finished?: boolean): Promise<PreviewLayout> => ({ ...right, revision: extent === undefined ? 1 : 3, mainWidth: extent ?? 560 }));
    const state = useAttachmentPreview(computed(() => "request-1"), undefined, delegate);
    state.showPreview(0); await flush(); delegate.mockClear();
    let release!: (layout: PreviewLayout) => void;
    delegate.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    state.resizePreview(580, false); await flush();
    for (const extent of [600, 640, 700, 660, 620]) state.resizePreview(extent, false);
    state.resizePreview(620, true); await flush();
    expect(delegate.mock.calls).toEqual([[true, 580, false]]);
    release({ ...right, revision: 2, mainWidth: 580 }); await flush();
    expect(delegate.mock.calls).toEqual([[true, 580, false], [true, 620, true]]);
    expect(state.previewLayout.value.mainWidth).toBe(620);
    state.disposePreview();
  });
  it("uses the same latest-position queue in independent windows", async () => {
    vi.stubGlobal("requestAnimationFrame", (fn: FrameRequestCallback) => { queueMicrotask(() => fn(0)); return 1; });
    mocks.invoke.mockResolvedValue(right);
    const state = useAttachmentPreview(computed(() => "request-1"));
    state.showPreview(0); await flush(); mocks.invoke.mockClear();
    let release!: (layout: PreviewLayout) => void;
    mocks.invoke.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    state.resizePreview(580, false); await flush();
    state.resizePreview(720, false); state.resizePreview(480, true); await flush();
    expect(mocks.invoke.mock.calls).toEqual([["popup_preview_layout", { requestId: "request-1", open: true, mainExtent: 580, finished: false, version: 2 }]]);
    mocks.invoke.mockResolvedValue({ ...right, revision: 3, mainWidth: 480 });
    release({ ...right, revision: 2, mainWidth: 580 }); await flush();
    expect(mocks.invoke.mock.calls[1]).toEqual(["popup_preview_layout", { requestId: "request-1", open: true, mainExtent: 480, finished: true, version: 3 }]);
    expect(state.previewLayout.value.mainWidth).toBe(480);
    state.disposePreview();
  });
  it("drops pending motion and late results when the active request changes", async () => {
    const active = ref(true);
    const delegate = vi.fn(async (_open: boolean, extent?: number, _finished?: boolean) => ({ ...right, mainWidth: extent ?? 560 }));
    const state = useAttachmentPreview(computed(() => "request-1"), active, delegate);
    state.showPreview(0); await flush(); delegate.mockClear();
    let release!: (layout: PreviewLayout) => void;
    delegate.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    state.resizePreview(580, false); await flush(); state.resizePreview(720, true);
    active.value = false; release({ ...right, revision: 2, mainWidth: 580 }); await flush();
    expect(delegate.mock.calls).toEqual([[true, 580, false]]);
    expect(state.previewLayout.value.mainWidth).toBe(560);
    active.value = true; state.resizePreview(600, true); await flush();
    expect(delegate.mock.calls[1]).toEqual([true, 600, true]);
    state.disposePreview();
  });
  it("closes without replaying old motion or accepting its late layout", async () => {
    const delegate = vi.fn(async (open: boolean, extent?: number, _finished?: boolean): Promise<PreviewLayout> => ({ ...right, revision: 3, side: open ? "right" : "closed", mainWidth: extent ?? 560 }));
    const state = useAttachmentPreview(computed(() => "request-1"), undefined, delegate);
    state.showPreview(0); await flush(); delegate.mockClear();
    let release!: (layout: PreviewLayout) => void;
    delegate.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    state.resizePreview(580, false); await flush(); state.resizePreview(720, true);
    state.stopPreview(); release({ ...right, revision: 4, mainWidth: 580 }); await flush();
    expect(delegate.mock.calls).toEqual([[true, 580, false], [false]]);
    expect(state.previewLayout.value.side).toBe("closed");
    state.disposePreview();
  });
  it("keeps the main region fixed through native resize and waits for a painted preparation", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (fn: FrameRequestCallback) => { frames.push(fn); return frames.length; });
    let acknowledge!: (layout: PreviewLayout) => void;
    mocks.invoke.mockImplementation((command: string) => command === "popup_preview_prepare"
      ? Promise.resolve(right) : new Promise<PreviewLayout>(resolve => { acknowledge = resolve; }));
    const state = useAttachmentPreview(computed(() => "request-1"));
    await state.initPreviewLayout();
    state.showPreview(0); await flush();
    expect(state.previewTransition.value).toEqual(right);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    frames.shift()!(0); await flush();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    frames.shift()!(16); await flush();
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    mocks.listeners.get("popup-preview-layout")!({ payload: right });
    expect(state.previewTransition.value?.mainWidth).toBe(560);
    expect(state.previewTransition.value?.mainHeight).toBe(620);
    acknowledge(right); await flush();
    expect(state.previewTransition.value).toBeNull();
    expect(state.previewLayout.value).toEqual(right);
    state.disposePreview();
  });
  it("cannot resize a disposed Popup after a late preparation result", async () => {
    let finish!: (layout: PreviewLayout) => void;
    mocks.invoke.mockImplementation(() => new Promise<PreviewLayout>(resolve => { finish = resolve; }));
    const state = useAttachmentPreview(computed(() => "request-1"));
    state.showPreview(0); await flush(); state.disposePreview(); finish(right); await flush();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(state.previewTransition.value).toBeNull();
  });
});
