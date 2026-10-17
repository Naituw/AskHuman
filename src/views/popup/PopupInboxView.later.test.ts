import { mount, flushPromises } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import { markdownReady } from "../../lib/markdown";
import type { PopupInboxRequest } from "../../lib/types";

// "Answer later" through the real popup form (docs/specs/popup-inbox-collapse.md C10, C24, C27): the footer button, the
// navbar's shrink icon and the page's reaction to them, with nothing but the native window mocked.
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: vi.fn(), setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import PopupInboxView from "../PopupInboxView.vue";

function request(id: string): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project: "/project", source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: `Question ${id}`, files: [] }, questions: [{ message: `Request ${id}`, predefinedOptions: [] }] } } };
}

describe("answer later in real shared popup forms", () => {
  beforeAll(() => markdownReady);
  afterEach(() => vi.unstubAllGlobals());

  async function open() {
    vi.stubGlobal("CSS", { escape: (value: string) => value });
    vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { queueMicrotask(() => callback(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const requests = [request("a"), request("b")];
    invoke.mockReset();
    invoke.mockImplementation(async (command: string, args: { requestId?: string; collapsed?: unknown }) => {
      if (command === "popup_inbox_init") return { recovered: false, requests };
      if (command === "popup_init") {
        const show = requests.find(item => item.requestId === args.requestId)!;
        return { ...show, theme: "light", alwaysOnTop: false, sourceName: "Codex", projectName: show.project, language: "en", warm: true };
      }
      if (command === "popup_inbox_layout") return { revision: 1, sidebarWidth: 240, sidebarPreferred: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0,
        frame: { width: 806, height: 620 }, collapsed: !!args.collapsed };
      if (command === "popup_inbox_commit") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0, frame: { width: 806, height: 620 } };
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
    const wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
    invoke.mockClear();
    return wrapper;
  }
  const layouts = () => invoke.mock.calls.filter(([command]) => command === "popup_inbox_layout").map(([, args]) => args as { collapsed?: unknown });

  it("puts the open request aside from its footer and offers to undo it", async () => {
    const wrapper = await open();
    try {
      const laterA = () => wrapper.get('[data-inbox-request="a"] .btn-later');
      expect(laterA().text()).toBe("Answer later");
      await laterA().trigger("click"); await flushPromises();
      // The window moves on to the waiting request; the request put aside keeps its form and shows how to undo.
      expect(wrapper.get('[data-inbox-request="b"]').isVisible()).toBe(true);
      expect(wrapper.get('[data-inbox-request="a"] .btn-later').text()).toBe("Undo answer later");
      expect(wrapper.get('[data-inbox-row="a"]').classes()).toContain("secondary");
      expect(wrapper.get('[data-inbox-request="b"] .btn-later').text()).toBe("Answer later");
      await laterA().trigger("click"); await flushPromises();
      expect(wrapper.get('[data-inbox-request="a"] .btn-later').text()).toBe("Answer later");
      expect(wrapper.get('[data-inbox-row="a"]').classes()).not.toContain("secondary");
      expect(layouts().every(args => !args.collapsed)).toBe(true);
    } finally { wrapper.unmount(); }
  });

  it("shrinks to the small window after the last waiting request is put aside", async () => {
    const wrapper = await open();
    try {
      await wrapper.get('[data-inbox-request="a"] .btn-later').trigger("click"); await flushPromises();
      await wrapper.get('[data-inbox-request="b"] .btn-later').trigger("click"); await flushPromises();
      expect(layouts()[layouts().length - 1].collapsed).toBeTruthy();
      expect(wrapper.attributes("data-view")).toBe("collapsed");
      // From the small window a row opens its request, which keeps its mark; the navbar can shrink it again.
      await wrapper.get('[data-inbox-row="a"]').trigger("click"); await flushPromises();
      expect(wrapper.attributes("data-view")).toBe("expanded");
      expect(wrapper.get('[data-inbox-request="a"] .btn-later').text()).toBe("Undo answer later");
      const shrink = wrapper.get('[data-inbox-request="a"] .nav-shrink');
      expect(shrink.attributes("aria-label")).toBe("Shrink to small window");
      await shrink.trigger("click"); await flushPromises();
      expect(wrapper.attributes("data-view")).toBe("collapsed");
    } finally { wrapper.unmount(); }
  });

  it("does nothing when the navbar's shrink icon is used while a request is waiting", async () => {
    const wrapper = await open();
    try {
      await wrapper.get('[data-inbox-request="a"] .nav-shrink').trigger("click"); await flushPromises();
      expect(layouts()).toEqual([]);
      expect(wrapper.attributes("data-view")).toBe("expanded");
    } finally { wrapper.unmount(); }
  });
});
