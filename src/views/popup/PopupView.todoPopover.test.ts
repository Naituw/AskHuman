// The navbar todo popover inside the real popup (docs/specs/todo-quick-add.md): which requests offer it,
// what it does to the request's own keys while it is open, and every way it closes.
import { computed, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import { desktopPlatform, primaryShortcutLabel } from "../../lib/platform";
import type { ConfirmRequest, PopupInit, TodoEntry } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, convertFileSrc: (path: string) => path }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import PopupView from "../PopupView.vue";

// jsdom has no CSS.escape, which the in-page find uses to address its segments.
if (typeof globalThis.CSS === "undefined") {
  (globalThis as unknown as { CSS: { escape: (s: string) => string } }).CSS = {
    escape: (s: string) => s.replace(/[^a-zA-Z0-9_-]/g, (c) => `\\${c}`),
  };
}

const PRIMARY: KeyboardEventInit = desktopPlatform === "mac" ? { metaKey: true } : { ctrlKey: true };
// Speech shortcuts are stored with the logical primary modifier ("cmd" is Ctrl on Windows / Linux) and
// matched on the physical key.
const SPEECH_ON_T = "cmd+t";

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

function whatsNext(id: string): PopupInit {
  return { ...base,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: true, outputFormat: "text",
      whatsNext: true, message: { text: "", files: [] },
      questions: [{ message: "What next?", predefinedOptions: [
        { text: "Add tests", recommended: false },
        { text: "End this turn", recommended: false },
      ] }] } },
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

function todo(id: string, text = `todo ${id}`): TodoEntry {
  return { id, text, createdAtMs: 1 };
}

describe("the navbar todo popover", () => {
  const wrappers: VueWrapper[] = [];
  let interaction: (id: string) => PopupInit;
  let overrides: Record<string, unknown>;
  let store: TodoEntry[];
  let nextId: number;
  let speechAvailable: boolean;

  beforeEach(() => {
    vi.clearAllMocks();
    interaction = ask;
    overrides = {};
    store = [];
    nextId = 0;
    speechAvailable = false;
    // Date only: the page keeps ⌘/Ctrl+W inert for a moment after a form appears, and the tests move past it.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    mocks.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
      switch (command) {
        case "popup_init": return { ...interaction(String(args.requestId ?? "solo")), ...overrides };
        case "popup_agent_resolved": return {};
        case "popup_update_state": return { available: false, pending: false };
        case "speech_available": return speechAvailable;
        case "todos_list": return store.map((entry) => ({ ...entry }));
        case "todos_add": {
          const entry = todo(`added-${++nextId}`, String(args.text));
          store.push(entry);
          return { ...entry };
        }
        case "todos_remove":
          store = store.filter((entry) => entry.id !== args.id);
          return true;
        default: return false;
      }
    });
  });

  afterEach(() => {
    wrappers.splice(0).forEach((w) => { try { w.unmount(); } catch { /* already unmounted */ } });
    document.body.innerHTML = "";
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  // `scoped: false` is the standalone popup: one request, no window around it.
  async function mountRequest(id = "a", scoped = false) {
    const close = vi.fn();
    const active = ref(true);
    const blocked = ref(false);
    const scope = scoped
      ? { requestId: id, active, blocked: computed(() => blocked.value), ready: vi.fn(), close, draft: vi.fn(),
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
    return { wrapper, close, active, blocked };
  }

  const calls = (command: string) => mocks.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const todoButton = (w: VueWrapper) => w.find(".nav-actions .nav-btn[title^='Add a project todo']");
  const popover = (w: VueWrapper) => w.find(".todo-popover");
  const popoverInput = (w: VueWrapper) => w.find<HTMLTextAreaElement>(".todo-popover .tp-input");
  const rows = (w: VueWrapper) => w.findAll(".tp-row").map((row) => row.find(".tp-text").text());
  const answerBox = (w: VueWrapper) => w.findAll<HTMLTextAreaElement>("textarea").find((box) => !box.classes().includes("tp-input"))!;
  const footerButton = (w: VueWrapper, label: string) => w.findAll(".footer .btn").find((b) => b.text().includes(label))!;
  const press = (target: EventTarget, init: KeyboardEventInit) =>
    target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  const openWithKey = async (w: VueWrapper) => {
    press(window, { key: "t", ...PRIMARY });
    await flushPromises();
    expect(popover(w).exists()).toBe(true);
  };

  describe("on an ordinary question", () => {
    it("opens from the button, focuses the input and adds todos without closing", async () => {
      const { wrapper } = await mountRequest();
      expect(popover(wrapper).exists()).toBe(false);
      await todoButton(wrapper).trigger("click");
      await flushPromises();

      expect(popover(wrapper).find(".tp-title").text()).toBe("Add to project");
      expect(popover(wrapper).find(".tp-hint").text()).toBe(`${primaryShortcutLabel("enter")} to add`);
      expect(popover(wrapper).find(".tp-empty").exists()).toBe(true);
      expect(document.activeElement).toBe(popoverInput(wrapper).element);

      await popoverInput(wrapper).setValue("  first todo ");
      await popover(wrapper).find(".tp-add").trigger("click");
      await flushPromises();
      expect(calls("todos_add")).toEqual([expect.objectContaining({ project: "/project", text: "first todo" })]);
      expect(popoverInput(wrapper).element.value).toBe("");
      expect(rows(wrapper)).toEqual(["first todo"]);
      expect(document.activeElement).toBe(popoverInput(wrapper).element);

      await popoverInput(wrapper).setValue("second todo");
      press(popoverInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(rows(wrapper)).toEqual(["first todo", "second todo"]);
      expect(popover(wrapper).exists()).toBe(true);
      // The new row is flagged for a moment; the body's todo section only counts it, collapsed.
      expect(wrapper.find(".tp-row.added .tp-text").text()).toBe("second todo");
      expect(wrapper.find(".todo-section .todo-count").text()).toBe("2");
      expect(wrapper.find(".todo-options").exists()).toBe(false);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("keeps the draft for the request's lifetime", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      await popoverInput(wrapper).setValue("half a thought");
      press(popoverInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(false);
      await openWithKey(wrapper);
      expect(popoverInput(wrapper).element.value).toBe("half a thought");
    });

    it("closes with Esc, a click outside, the button or the shortcut and hands the focus back", async () => {
      const { wrapper } = await mountRequest();
      const answer = answerBox(wrapper).element;
      const closers: Array<() => unknown> = [
        () => press(popoverInput(wrapper).element, { key: "Escape" }),
        () => press(popoverInput(wrapper).element, { key: "t", ...PRIMARY }),
        () => wrapper.find(".todo-backdrop").trigger("click"),
        () => todoButton(wrapper).trigger("click"),
      ];
      for (const close of closers) {
        answer.focus();
        await openWithKey(wrapper);
        expect(document.activeElement).toBe(popoverInput(wrapper).element);
        await close();
        await flushPromises();
        expect(popover(wrapper).exists()).toBe(false);
        expect(wrapper.find(".todo-backdrop").exists()).toBe(false);
        expect(document.activeElement).toBe(answer);
      }
      expect(calls("cancel_popup")).toEqual([]);
    });

    it("never lets the popover's Enter or Esc reach the answer", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my answer");
      await openWithKey(wrapper);

      // Enter with nothing typed: ⌘/Ctrl+Enter would submit the answer outside the popover.
      press(popoverInput(wrapper).element, { key: "Enter", ...PRIMARY });
      press(popoverInput(wrapper).element, { key: "Enter" });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
      expect(calls("todos_add")).toEqual([]);
      expect(popover(wrapper).exists()).toBe(true);

      press(popoverInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      expect(wrapper.find(".confirm-overlay").exists()).toBe(false);
      expect(answerBox(wrapper).element.value).toBe("my answer");

      // Closed again, the very same key submits the answer.
      press(answerBox(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(calls("submit_popup")).toHaveLength(1);
    });

    it("sends with a bare Enter when that is the submit key, and Shift+Enter breaks the line", async () => {
      overrides = { popupSubmitKey: "enter" };
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      expect(popover(wrapper).find(".tp-hint").text()).toBe("↵ to add");
      await popoverInput(wrapper).setValue("line one");
      press(popoverInput(wrapper).element, { key: "Enter", shiftKey: true });
      expect(calls("todos_add")).toEqual([]);
      press(popoverInput(wrapper).element, { key: "Enter" });
      await flushPromises();
      expect(calls("todos_add")).toEqual([expect.objectContaining({ text: "line one" })]);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("keeps an add failure in the popover with the draft intact", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      const original = mocks.invoke.getMockImplementation()!;
      mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
        if (command === "todos_add") throw new Error("disk is full");
        return original(command, args);
      });
      await popoverInput(wrapper).setValue("will not stick");
      await popover(wrapper).find(".tp-add").trigger("click");
      await flushPromises();
      expect(popover(wrapper).find(".tp-error").text()).toContain("disk is full");
      expect(popoverInput(wrapper).element.value).toBe("will not stick");
      expect(rows(wrapper)).toEqual([]);
    });
  });

  describe("on a permission card", () => {
    beforeEach(() => { interaction = permission; });

    it("lists the project's todos but leaves the card body alone", async () => {
      store = [todo("a", "check the logs"), todo("b", "bump the version")];
      const { wrapper } = await mountRequest();
      expect(wrapper.find(".todo-section").exists()).toBe(false);
      await todoButton(wrapper).trigger("click");
      await flushPromises();
      expect(rows(wrapper)).toEqual(["check the logs", "bump the version"]);
      expect(wrapper.find(".todo-section").exists()).toBe(false);
    });

    it("never lets Enter or Esc inside the popover approve, deny or warn", async () => {
      const { wrapper } = await mountRequest();
      await wrapper.find(".confirm-option").trigger("click");
      await openWithKey(wrapper);

      press(popoverInput(wrapper).element, { key: "Enter", ...PRIMARY });
      press(popoverInput(wrapper).element, { key: "Enter" });
      await flushPromises();
      expect(calls("submit_confirm_action")).toEqual([]);

      await popoverInput(wrapper).setValue("note for later");
      press(popoverInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(calls("todos_add")).toEqual([expect.objectContaining({ text: "note for later" })]);
      expect(calls("submit_confirm_action")).toEqual([]);

      press(popoverInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(false);
      expect(wrapper.find(".confirm-overlay").exists()).toBe(false);

      // Closed again, Esc warns about leaving and ⌘/Ctrl+Enter approves the selected choice.
      press(window, { key: "Escape" });
      await flushPromises();
      expect(wrapper.find(".confirm-overlay").exists()).toBe(true);
      press(window, { key: "Escape" });
      await flushPromises();
      press(window, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(calls("submit_confirm_action")).toHaveLength(1);
    });
  });

  describe("on a whats-next question", () => {
    beforeEach(() => { interaction = whatsNext; });

    function multiNext(id: string): PopupInit {
      const init = whatsNext(id);
      if (init.interaction?.type === "ask") init.interaction.request.single = false;
      return init;
    }

    it("submits several todos together with a suggested task", async () => {
      interaction = multiNext;
      store = [todo("first", "first task"), todo("second", "second task")];
      const { wrapper } = await mountRequest();
      const options = wrapper.findAll(".option");
      await options[0].trigger("click");
      await options[2].trigger("click");
      await options[1].trigger("click");
      await footerButton(wrapper, "Send").trigger("click");
      await flushPromises();
      expect(calls("submit_popup")).toEqual([expect.objectContaining({
        submission: expect.objectContaining({ answers: [expect.objectContaining({ selectedOptions: ["Add tests"],
          userInput: "first task\n\nsecond task", todoIds: ["first", "second"],
          todoSelections: [{ id: "first", attachments: [] }, { id: "second", attachments: [] }],
        })] }),
      })]);
    });

    it("keeps end exclusive while allowing tasks to be picked together", async () => {
      interaction = multiNext;
      store = [todo("first", "first task"), todo("second", "second task")];
      const { wrapper } = await mountRequest();
      const options = wrapper.findAll(".option");
      await options[0].trigger("click");
      await options[1].trigger("click");
      expect(wrapper.findAll(".option.selected")).toHaveLength(2);
      await options[3].trigger("click");
      expect(wrapper.findAll(".option.selected").map(row => row.find(".label").text())).toEqual(["End this turn"]);
      await options[2].trigger("click");
      expect(wrapper.findAll(".option.selected").map(row => row.find(".label").text().replace(/^TODO/, ""))).toEqual(["second task"]);
    });

    it("shows what the popover adds and deletes among the options", async () => {
      const { wrapper } = await mountRequest();
      // A todo option carries a "TODO" badge in front of its text.
      const labels = () => wrapper.findAll(".option .label").map((label) => label.text().replace(/^TODO/, ""));
      expect(labels()).toEqual(["Add tests", "End this turn"]);

      await openWithKey(wrapper);
      await popoverInput(wrapper).setValue("write docs");
      press(popoverInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(labels()).toEqual(["Add tests", "write docs", "End this turn"]);
      expect(rows(wrapper)).toEqual(["write docs"]);
      // The question is still unanswered.
      expect(calls("submit_popup")).toEqual([]);

      await popover(wrapper).find(".tp-del").trigger("click");
      await popover(wrapper).find(".tp-del-confirm").trigger("click");
      await flushPromises();
      expect(calls("todos_remove")).toEqual([expect.objectContaining({ project: "/project", id: "added-1" })]);
      expect(labels()).toEqual(["Add tests", "End this turn"]);
      expect(rows(wrapper)).toEqual([]);
    });
  });

  describe("the list", () => {
    it("deletes in two steps and a click elsewhere in the popover cancels the first", async () => {
      store = [todo("a", "first"), todo("b", "second")];
      const { wrapper } = await mountRequest();
      await todoButton(wrapper).trigger("click");
      await flushPromises();
      expect(rows(wrapper)).toEqual(["first", "second"]);

      const firstRow = () => wrapper.findAll(".tp-row")[0];
      await firstRow().find(".tp-del").trigger("click");
      expect(firstRow().find(".tp-del-confirm").text()).toBe("Confirm delete");
      expect(calls("todos_remove")).toEqual([]);

      await popover(wrapper).find(".tp-title").trigger("click");
      expect(firstRow().find(".tp-del-confirm").exists()).toBe(false);
      expect(popover(wrapper).exists()).toBe(true);

      await firstRow().find(".tp-del").trigger("click");
      await firstRow().find(".tp-del-confirm").trigger("click");
      await flushPromises();
      expect(calls("todos_remove")).toEqual([expect.objectContaining({ project: "/project", id: "a" })]);
      expect(rows(wrapper)).toEqual(["second"]);
    });

    it("opens the todos window from its link and closes", async () => {
      const { wrapper } = await mountRequest();
      await todoButton(wrapper).trigger("click");
      await flushPromises();
      await popover(wrapper).find(".tp-open").trigger("click");
      await flushPromises();
      expect(calls("open_todos")).toEqual([{ dir: "/project" }]);
      expect(popover(wrapper).exists()).toBe(false);
    });
  });

  describe("on a request without a project", () => {
    beforeEach(() => { overrides = { project: "", projectName: "" }; });

    it("keeps the button for the todos window and ignores the shortcut", async () => {
      const { wrapper } = await mountRequest();
      expect(todoButton(wrapper).exists()).toBe(false);
      const plain = wrapper.find(".nav-actions .nav-btn[title='Project todos']");
      expect(plain.exists()).toBe(true);
      expect(plain.attributes("aria-haspopup")).toBeUndefined();

      await plain.trigger("click");
      await flushPromises();
      expect(calls("open_todos")).toEqual([{ dir: null }]);
      expect(popover(wrapper).exists()).toBe(false);

      press(window, { key: "t", ...PRIMARY });
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(false);
    });
  });

  describe("in the merged window", () => {
    it("closes when its request stops being the active one", async () => {
      const merged = await mountRequest("a", true);
      await openWithKey(merged.wrapper);
      merged.active.value = false;
      await flushPromises();
      expect(popover(merged.wrapper).exists()).toBe(false);
    });

    it("closes when the window is blocked by a dialog", async () => {
      const merged = await mountRequest("a", true);
      await todoButton(merged.wrapper).trigger("click");
      await flushPromises();
      expect(popover(merged.wrapper).exists()).toBe(true);
      merged.blocked.value = true;
      await flushPromises();
      expect(popover(merged.wrapper).exists()).toBe(false);
    });

    it("still lets ⌘/Ctrl+W close the window while open", async () => {
      const merged = await mountRequest("a", true);
      await openWithKey(merged.wrapper);
      press(popoverInput(merged.wrapper).element, { key: "w", ...PRIMARY });
      await flushPromises();
      expect(merged.close).toHaveBeenCalledTimes(1);
    });
  });

  describe("beside the rest of the popup", () => {
    it("closes when the cancel confirmation appears", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("unsent draft");
      await openWithKey(wrapper);
      await footerButton(wrapper, "Cancel").trigger("click");
      await flushPromises();
      expect(wrapper.find(".confirm-overlay").exists()).toBe(true);
      expect(popover(wrapper).exists()).toBe(false);
    });

    it("leaves ⌘/Ctrl+F alone while open, and does not open while the find bar is", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      press(popoverInput(wrapper).element, { key: "f", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".popup-find-bar").exists()).toBe(false);
      expect(popover(wrapper).exists()).toBe(true);
      press(popoverInput(wrapper).element, { key: "Escape" });
      await flushPromises();

      press(window, { key: "f", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".popup-find-bar").exists()).toBe(true);
      press(window, { key: "t", ...PRIMARY });
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(false);
    });

    it("yields ⌘/Ctrl+T to the speech shortcut but still opens from the button", async () => {
      speechAvailable = true;
      overrides = { speechShortcut: SPEECH_ON_T };
      const { wrapper } = await mountRequest();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(false);
      await todoButton(wrapper).trigger("click");
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(true);
    });

    it("opens with ⌘/Ctrl+T when speech input is on another shortcut", async () => {
      speechAvailable = true;
      overrides = { speechShortcut: "cmd+d" };
      const { wrapper } = await mountRequest();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(true);
    });

    it("keeps an image pasted into the popover away from the answer", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      const reachedWindow = vi.fn();
      window.addEventListener("paste", reachedWindow);
      try {
        popoverInput(wrapper).element.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
        expect(reachedWindow).not.toHaveBeenCalled();
        answerBox(wrapper).element.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
        expect(reachedWindow).toHaveBeenCalledTimes(1);
      } finally {
        window.removeEventListener("paste", reachedWindow);
      }
    });

    it("closes when the focus moves to another control, not when it merely leaves the page", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      const focusout = (relatedTarget: EventTarget | null) =>
        popoverInput(wrapper).element.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget }));

      // The window losing focus, or a click on the popover's own blank space / controls.
      focusout(null);
      focusout(popover(wrapper).find(".tp-title").element);
      focusout(popover(wrapper).find(".tp-add").element);
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(true);

      focusout(answerBox(wrapper).element);
      await flushPromises();
      expect(popover(wrapper).exists()).toBe(false);
    });
  });
});
