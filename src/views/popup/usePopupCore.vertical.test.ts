import { computed, defineComponent, h, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInit } from "../../lib/types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: vi.fn(async () => () => {}) }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import { usePopupCore, type PopupScope } from "./usePopupCore";

// `verticalQuestions` is left out of the snapshot on purpose unless a test sets it: that is what a config written
// before the field existed produces, and the front end must then follow the factory default (vertical).
function init(questions: number, verticalQuestions?: boolean): PopupInit {
  const base: PopupInit = {
    theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project", language: "en", warm: true,
    interaction: { type: "ask", request: { id: "a", isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] },
      questions: Array.from({ length: questions }, (_, i) => ({ message: `Question ${i + 1}`, predefinedOptions: [] })) } },
  };
  return verticalQuestions === undefined ? base : { ...base, verticalQuestions };
}

describe("multi-question layout default", () => {
  const wrappers: VueWrapper[] = [];
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
  });
  afterEach(() => { wrappers.forEach(w => w.unmount()); wrappers.length = 0; vi.unstubAllGlobals(); });

  async function open(snapshot: PopupInit) {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_init") return snapshot;
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
    let ctx!: ReturnType<typeof usePopupCore>;
    const scope: PopupScope = { requestId: "a", active: ref(true), blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: vi.fn(),
      preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }) };
    const wrapper = mount(defineComponent({ setup() { ctx = usePopupCore(scope); return () => h("div"); } }), {
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper); await flushPromises();
    return ctx;
  }

  it("lists every question vertically when the snapshot carries no setting", async () => {
    expect((await open(init(2))).verticalMode.value).toBe(true);
  });

  it("lists every question vertically when the setting is on", async () => {
    expect((await open(init(2, true))).verticalMode.value).toBe(true);
  });

  it("keeps the classic one-question-at-a-time view when the setting is explicitly off", async () => {
    expect((await open(init(2, false))).verticalMode.value).toBe(false);
  });

  it("never lays a single question out vertically", async () => {
    expect((await open(init(1))).verticalMode.value).toBe(false);
  });
});
