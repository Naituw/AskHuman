// A standalone popup (no inbox page around it) tells what became of a file drop in a line under its title bar, and says
// nothing about files that were added: their thumbnails show that (docs/specs/popup-inbox-collapse.md §6.9).
import { nextTick } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInit } from "../../lib/types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), drag: null as null | ((event: { payload: unknown }) => void) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, convertFileSrc: (path: string) => path }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async (handler: (event: { payload: unknown }) => void) => {
  mocks.drag = handler; return () => { mocks.drag = null; };
} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import PopupView from "../PopupView.vue";

const init = {
  theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project", language: "en", warm: true,
  interaction: { type: "ask", request: { id: "solo", isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
    message: { text: "", files: [] }, questions: [{ message: "Question", predefinedOptions: [] }] } },
} as PopupInit;

describe("a standalone popup's notice about a drop", () => {
  let wrapper: VueWrapper;
  beforeEach(() => {
    mocks.drag = null;
    vi.clearAllMocks();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => null });
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_init") return init;
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
  });
  afterEach(() => {
    wrapper?.unmount();
    vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks();
    Reflect.deleteProperty(document, "elementFromPoint");
  });

  async function open() {
    wrapper = mount(PopupView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises(); await nextTick();
  }
  const drop = async (paths: string[]) => {
    mocks.drag!({ payload: { type: "drop", paths, position: { x: 10, y: 10 } } });
    await flushPromises(); await nextTick();
  };
  const notice = () => wrapper.find(".drop-notice");

  it("shows nothing until something is dropped", async () => {
    await open();
    expect(notice().exists()).toBe(false);
  });

  it("warns of a drop that carried no files, as an alert that goes away by itself", async () => {
    await open();
    await drop([]);
    expect(notice().exists()).toBe(true);
    expect(notice().text()).toContain("No files received");
    expect(notice().attributes("data-level")).toBe("warn");
    expect(notice().attributes("role")).toBe("alert");
    await vi.advanceTimersByTimeAsync(6000);
    await nextTick();
    expect(notice().exists()).toBe(false);
  });

  it("says nothing of a file that was added: its thumbnail is what shows it", async () => {
    await open();
    await drop(["/tmp/notes.txt"]);
    expect(notice().exists()).toBe(false);
    expect(wrapper.text()).toContain("notes.txt");
  });

  it("warns of a picture the system would not let it read, and where to look", async () => {
    await open();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_drop_read_image") throw { reason: "permission_denied", where: "other", message: "Operation not permitted (os error 1)" };
      return false;
    });
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    await drop(["/Users/me/Desktop/shot.png"]);
    quiet.mockRestore();
    expect(notice().text()).toContain("No permission to read “shot.png”");
    expect(notice().attributes("data-level")).toBe("warn");
  });
});
