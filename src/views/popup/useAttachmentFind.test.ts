import { defineComponent, h, nextTick, ref } from "vue";
import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PreviewContent } from "./useAttachmentContent";
import AttachmentDiffPreview from "./AttachmentDiffPreview.vue";
import { useAttachmentFind } from "./useAttachmentFind";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(async () => {}), handler: null as ((event: { payload: unknown }) => void) | null }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (_name: string, handler: typeof mocks.handler) => { mocks.handler = handler; return () => { mocks.handler = null; }; }) }));
vi.mock("../../lib/platform", () => ({ isMac: true }));
enableAutoUnmount(afterEach);
beforeEach(() => { vi.clearAllMocks(); vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} }); });
afterEach(() => { vi.useRealTimers(); });
function setup(initial: PreviewContent) {
  const content = ref<PreviewContent | null>(initial), index = ref<number | null>(0), raw = ref(false), active = ref(true), enabled = ref(true), loading = ref(false);
  const body = ref<HTMLElement | null>(null), top = ref(0);
  let result!: ReturnType<typeof useAttachmentFind>;
  const component = defineComponent({ setup() {
    result = useAttachmentFind({ requestId: ref("request"), index, content, loading, raw, active, enabled,
      body, nativeBody: ref(null), failed: ref(null), restoreScroll: async () => {}, syncNative: async () => {}, setTop: value => { top.value = value; },
    });
    return () => h("div", { ref: body }, content.value?.kind === "diff" && !raw.value
      ? h(AttachmentDiffPreview, { parsed: content.value.parsed, viewport: body.value, top: top.value, query: result.diffQuery.value, caseSensitive: result.diffCaseSensitive.value, current: result.diffCurrent.value })
      : h("pre", { class: "attachment-preview-text" }, content.value && "text" in content.value ? content.value.text : ""));
  }});
  const wrapper = mount(component, { attachTo: document.body });
  return { result, content, index, raw, active, enabled, loading, body, wrapper };
}
async function settle() { await flushPromises(); for (let i = 0; i < 4; i++) await nextTick(); }

describe("attachment content find", () => {
  it("indexes the entire virtual diff and mounts an offscreen final-line hit on navigation", async () => {
    const rows = Array.from({ length: 2000 }, (_, i) => ({ text: i === 1999 ? "+needle needle" : "+context", kind: "add" as const, old: null, new: String(i + 1) }));
    const { result, wrapper, body } = setup({ kind: "diff", text: "", parsed: { sections: [{ title: "file", rows }], notice: null } });
    expect(wrapper.findAll(".attachment-diff-line").length).toBeLessThan(100);
    result.adapter.search("needle", false, true); await settle();
    expect(result.adapter.state.value.total).toBe(2);
    expect(body.value!.scrollTop).toBeGreaterThan(40000);
    expect(wrapper.get(".popup-find-hit-current").text()).toBe("needle");
    await result.adapter.go(1); await settle(); expect(result.adapter.state.value.current).toBe(1);
    expect(wrapper.findAll(".popup-find-hit-current")).toHaveLength(1);
  });
  it("searches text safely, clears marks on pause, and refuses queued navigation after request deactivation", async () => {
    const { result, body, active } = setup({ kind: "text", text: "alpha <script> alpha" });
    result.adapter.search("alpha", false, true); await settle(); expect(result.adapter.state.value.total).toBe(2);
    expect(body.value!.querySelectorAll("mark")).toHaveLength(2);
    result.adapter.search("<script>", false, true); active.value = false; await settle();
    expect(body.value!.querySelectorAll("mark")).toHaveLength(0);
    expect(body.value!.querySelector("script")).toBeNull();
  });
  it("retains query through file loading, searches the replacement once, and preserves mode reading position", async () => {
    const { result, content, index, loading, raw, body } = setup({ kind: "text", text: "alpha alpha" });
    result.adapter.search("alpha", false, true); await settle();
    index.value = 1; loading.value = true; content.value = null; await settle();
    expect(result.adapter.state.value.status).toBe("loading");
    content.value = { kind: "markdown", text: "alpha", html: "<p>alpha</p>" }; loading.value = false; await settle();
    expect(result.adapter.state.value.total).toBe(1); expect(result.adapter.state.value.current).toBe(0);
    body.value!.scrollTop = 500; raw.value = true; await settle(); expect(body.value!.scrollTop).toBe(500);
  });
  it("separates unsupported capability from no matches", async () => {
    const { result } = setup({ kind: "image", url: "", width: 1, height: 1 });
    result.adapter.search("alpha", false, true); await settle();
    expect(result.adapter.state.value).toEqual({ total: 0, current: -1, status: "unsupported" });
  });
  it("rejects late PDF replies by query generation and request identity", async () => {
    vi.useFakeTimers();
    const { result, active } = setup({ kind: "native" }); await settle();
    result.adapter.search("old", false, true); await nextTick(); await vi.advanceTimersByTimeAsync(110);
    const first = (mocks.invoke.mock.calls[mocks.invoke.mock.calls.length - 1] as unknown as [string, { search: { generation: number } }])[1].search.generation;
    result.adapter.search("new", false, true); await nextTick(); await vi.advanceTimersByTimeAsync(110);
    const latest = (mocks.invoke.mock.calls[mocks.invoke.mock.calls.length - 1] as unknown as [string, { search: { generation: number } }])[1].search.generation;
    const reply = (generation: number, requestId = "request") => mocks.handler?.({ payload: { generation, requestId, index: 0, total: 12, current: 0, status: "ready" } });
    reply(first); expect(result.adapter.state.value.status).toBe("searching");
    reply(latest, "other"); expect(result.adapter.state.value.status).toBe("searching");
    reply(latest); expect(result.adapter.state.value.total).toBe(12);
    active.value = false; await nextTick(); reply(latest); expect(result.adapter.state.value.total).toBe(12);
  });
});
