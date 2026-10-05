import { computed, effectScope, nextTick, ref } from "vue";
import { describe, expect, it, vi, beforeEach } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(async (_command: string, _args: Record<string, unknown>) => {}) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../lib/platform", () => ({ isMac: true }));
import { useNativeAttachmentPreview } from "./useNativeAttachmentPreview";
describe("embedded native preview coordination", () => {
  let frames: Map<number, FrameRequestCallback>;
  beforeEach(() => {
    vi.clearAllMocks(); frames = new Map(); let id = 0;
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { frames.set(++id, f); return id; });
    vi.stubGlobal("cancelAnimationFrame", (i: number) => frames.delete(i));
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  });
  async function paint() { await nextTick(); const pending = [...frames.values()]; frames.clear(); pending.forEach(f => f(0)); await nextTick(); }
  function setup() {
    const scope = effectScope();
    const index = ref<number | null>(null), active = ref(false), blocked = ref(false), element = ref<HTMLElement | null>(null);
    const result = scope.run(() => useNativeAttachmentPreview({ requestId: computed(() => "r"), index, active, blocked, element,
      bareEnter: computed(() => false), revision: computed(() => 1) }))!;
    const body = document.createElement("div");
    body.getBoundingClientRect = () => ({ x: 566, y: 48, width: 700, height: 652 }) as DOMRect;
    return { index, active, blocked, element, body, result, scope };
  }
  it("hides before switching files and clips the view to the measured body after paint", async () => {
    const s = setup(); s.index.value = 1; s.active.value = true; s.element.value = s.body; await paint();
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_native", expect.objectContaining({ index: 1, rect: { x: 566, y: 48, width: 700, height: 652 } }));
    const first = mocks.invoke.mock.calls[mocks.invoke.mock.calls.length - 1][1] as { version: number };
    s.index.value = 2;
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_native", expect.objectContaining({ index: null }));
    await paint();
    const last = mocks.invoke.mock.calls[mocks.invoke.mock.calls.length - 1][1] as { version: number; index: number };
    expect(last.index).toBe(2); expect(last.version).toBeGreaterThan(first.version);
    s.scope.stop();
  });
  it("removes the native surface synchronously for a root overlay and restores it afterwards", async () => {
    const s = setup(); s.index.value = 0; s.active.value = true; s.element.value = s.body; await paint();
    s.blocked.value = true;
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_native", expect.objectContaining({ index: null }));
    await paint(); expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_native", expect.objectContaining({ index: null }));
    s.blocked.value = false; await paint();
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_native", expect.objectContaining({ index: 0 }));
    s.scope.stop();
  });
  it("ignores a rejected stale show without hiding the newer file", async () => {
    let reject!: (error: Error) => void;
    mocks.invoke.mockImplementationOnce(async () => {}).mockImplementationOnce(async () => {});
    const s = setup(); s.index.value = 0; s.active.value = true;
    mocks.invoke.mockImplementationOnce(() => new Promise<void>((_, r) => { reject = r; }));
    s.element.value = s.body; await paint();
    s.index.value = 1; await paint(); reject(new Error("stale")); await nextTick();
    expect(s.result.nativeFailed.value).toBe(false);
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_native", expect.objectContaining({ index: 1 }));
    s.scope.stop();
  });
});
