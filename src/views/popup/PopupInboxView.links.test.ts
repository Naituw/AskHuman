import { mount, flushPromises } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import { markdownReady } from "../../lib/markdown";
import type { PopupInboxRequest } from "../../lib/types";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: vi.fn(), setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import PopupInboxView from "../PopupInboxView.vue";

function request(id: string): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project: `/project-${id}`, source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "[report](docs/report.md:82)", files: [] }, questions: [{ message: `Request ${id}`, predefinedOptions: [] }] } } };
}

describe("file links in real shared popup forms", () => {
  beforeAll(() => markdownReady);
  afterEach(() => vi.unstubAllGlobals());
  it("opens each request's own file and preserves both forms and their drafts", async () => {
    vi.stubGlobal("CSS", { escape: (value: string) => value });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { queueMicrotask(() => callback(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const a = request("a"), b = request("b");
    invoke.mockImplementation(async (command: string, args: { requestId?: string }) => {
      if (command === "popup_inbox_init") return { recovered: false, requests: [a, b] };
      if (command === "popup_init") {
        const show = args.requestId === "a" ? a : b;
        return { ...show, theme: "light", alwaysOnTop: false, sourceName: "Codex", projectName: show.project, language: "en", warm: true };
      }
      if (command === "popup_inbox_layout" || command === "popup_inbox_commit") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0, frame: { width: 806, height: 620 } };
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
    const wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    try {
      await flushPromises();
      const editorA = wrapper.get('[data-inbox-request="a"] textarea');
      await editorA.setValue("draft A");
      const linkA = wrapper.get('[data-inbox-request="a"] .markdown-body a');
      const click = new MouseEvent("click", { bubbles: true, cancelable: true });
      linkA.element.dispatchEvent(click);
      await flushPromises();
      expect(click.defaultPrevented).toBe(true);
      expect(invoke).toHaveBeenCalledWith("open_path", { path: "/project-a/docs/report.md" });
      expect(wrapper.get('[data-inbox-request="a"] textarea').element).toBe(editorA.element);
      expect((editorA.element as HTMLTextAreaElement).value).toBe("draft A");
      expect(wrapper.findAll(".inbox-row")).toHaveLength(2);

      await wrapper.get('[data-inbox-row="b"]').trigger("click");
      await flushPromises();
      const editorB = wrapper.get('[data-inbox-request="b"] textarea');
      await editorB.setValue("draft B");
      await wrapper.get('[data-inbox-request="b"] .markdown-body a').trigger("click");
      expect(invoke).toHaveBeenCalledWith("open_path", { path: "/project-b/docs/report.md" });
      await wrapper.get('[data-inbox-row="a"]').trigger("click");
      await flushPromises();
      expect((editorA.element as HTMLTextAreaElement).value).toBe("draft A");
      expect((editorB.element as HTMLTextAreaElement).value).toBe("draft B");
      expect(wrapper.find(".status-error").exists()).toBe(false);
    } finally { wrapper.unmount(); }
  });
});
