// The merged window mounts one real PopupView per request. The Cancel button sits under ONE request, so it must act on
// that request alone, the way a standalone popup's Cancel does. The window's own confirmation (cancel the current
// request / every pending one) belongs to closing the window: the native close button and ⌘/Ctrl+W.
// docs/specs/popup-request-inbox.md §5.
import { computed, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import { desktopPlatform, primaryShortcutLabel } from "../../lib/platform";
import type { ConfirmRequest, PopupInit } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, convertFileSrc: (path: string) => path }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import PopupView from "../PopupView.vue";

const base = {
  theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project",
  language: "en", warm: true,
};

function ask(id: string): PopupInit {
  return { ...base,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Question of ${id}`, predefinedOptions: [] }] } },
  } as PopupInit;
}

function permission(id: string): PopupInit {
  const request: ConfirmRequest = {
    id, title: "Run command?", context: [], detail: { summary: "Needed for verification", bodyMd: "`cargo test`" },
    choices: [
      { id: "allow", label: "Allow once", description: "", role: "primary" },
      { id: "deny", label: "Deny", description: "", role: "destructive" },
    ],
    presentation: { type: "singleSelectSubmit", submitLabel: "Submit" },
    dismissActionId: "deny", createdAtMs: 1, expiresAtMs: 2,
  };
  return { ...base, interaction: { type: "confirm", request } } as PopupInit;
}

describe("the Cancel button of a request in the merged window", () => {
  const wrappers: VueWrapper[] = [];
  let interaction: (id: string) => PopupInit;

  beforeEach(() => {
    vi.clearAllMocks();
    interaction = ask;
    // Date only: the page keeps ⌘/Ctrl+W inert for a moment after a form appears, and the test moves past it.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    mocks.invoke.mockImplementation(async (command: string, args: { requestId?: string }) => {
      if (command === "popup_init") return interaction(args.requestId ?? "solo");
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
  });

  afterEach(() => {
    wrappers.splice(0).forEach((w) => { try { w.unmount(); } catch { /* already unmounted */ } });
    document.body.innerHTML = "";
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  // `withScope: false` is the standalone popup: one request, no window around it.
  async function mountRequest(id: string, withScope = true) {
    const close = vi.fn();
    const scope = withScope
      ? { requestId: id, active: ref(true), blocked: computed(() => false), ready: vi.fn(), close, draft: vi.fn(),
        preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }) } as unknown as PopupScope
      : undefined;
    const wrapper = mount(PopupView, {
      props: scope ? { scope } : {},
      attachTo: document.body,
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper);
    await flushPromises();
    vi.setSystemTime(Date.now() + 1000);
    return { wrapper, close };
  }

  const cancelButton = (wrapper: VueWrapper) => wrapper.findAll(".footer .btn").find((b) => b.text().includes("Cancel"))!;
  const calls = (command: string) => mocks.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const windowClose = () => window.dispatchEvent(new KeyboardEvent("keydown", {
    key: "w", metaKey: desktopPlatform === "mac", ctrlKey: desktopPlatform !== "mac",
  }));

  it("cancels only its own request and never opens the window's close confirmation", async () => {
    const a = await mountRequest("a");
    await cancelButton(a.wrapper).trigger("click");
    await flushPromises();
    expect(calls("cancel_popup")).toEqual([{ requestId: "a" }]);
    expect(a.close).not.toHaveBeenCalled();
  });

  it("asks before discarding an answer, then still cancels only that request", async () => {
    const a = await mountRequest("a");
    await a.wrapper.find("textarea").setValue("unsent draft");
    await cancelButton(a.wrapper).trigger("click");
    await flushPromises();
    expect(a.wrapper.find(".confirm-overlay").exists()).toBe(true);
    expect(calls("cancel_popup")).toEqual([]);
    expect(a.close).not.toHaveBeenCalled();
    await a.wrapper.find(".confirm-overlay .btn-danger").trigger("click");
    await flushPromises();
    expect(calls("cancel_popup")).toEqual([{ requestId: "a" }]);
    expect(a.close).not.toHaveBeenCalled();
  });

  it("leaves the window's confirmation to closing the window with ⌘/Ctrl+W", async () => {
    const a = await mountRequest("a");
    windowClose();
    await flushPromises();
    expect(a.close).toHaveBeenCalledTimes(1);
    expect(calls("cancel_popup")).toEqual([]);
    expect(a.wrapper.find(".confirm-overlay").exists()).toBe(false);
  });

  it("does not advertise the window-close shortcut on the button, unlike a standalone popup", async () => {
    const merged = await mountRequest("a");
    expect(cancelButton(merged.wrapper).find(".sc").exists()).toBe(false);
    merged.wrapper.unmount();
    const alone = await mountRequest("solo", false);
    expect(cancelButton(alone.wrapper).find(".sc").text()).toBe(primaryShortcutLabel("w"));
  });

  it("keeps a standalone popup's ⌘/Ctrl+W as its own Cancel", async () => {
    await mountRequest("solo", false);
    windowClose();
    await flushPromises();
    expect(calls("cancel_popup")).toEqual([{ requestId: undefined }]);
  });

  describe("on a permission request", () => {
    beforeEach(() => { interaction = permission; });

    it("warns about this request only, and denies only this request when confirmed", async () => {
      const p = await mountRequest("p");
      await cancelButton(p.wrapper).trigger("click");
      await flushPromises();
      expect(p.wrapper.find(".confirm-overlay").exists()).toBe(true);
      expect(p.close).not.toHaveBeenCalled();
      expect(calls("submit_confirm_action")).toEqual([]);
      await p.wrapper.find(".confirm-overlay .btn-danger").trigger("click");
      await flushPromises();
      expect(calls("submit_confirm_action")).toEqual([expect.objectContaining({ choiceIndex: 1, requestId: "p" })]);
      expect(p.close).not.toHaveBeenCalled();
    });

    it("still hands ⌘/Ctrl+W to the window", async () => {
      const p = await mountRequest("p");
      windowClose();
      await flushPromises();
      expect(p.close).toHaveBeenCalledTimes(1);
      expect(p.wrapper.find(".confirm-overlay").exists()).toBe(false);
    });
  });
});
