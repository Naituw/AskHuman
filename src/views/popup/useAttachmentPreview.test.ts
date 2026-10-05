import { computed } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
