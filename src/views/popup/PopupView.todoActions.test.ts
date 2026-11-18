// Complete, delete and add on every todo list of a request page, inside the real popup
// (docs/specs/todo-list-actions.md): the folded section of an ordinary question, the live queue of What's next and of a
// Stop confirmation, the key rules of the add row, ⌘/Ctrl+T and the navbar button that now opens the Todos window.
import { computed, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import { desktopPlatform, primaryShortcutLabel } from "../../lib/platform";
import type { AskRequest, ConfirmRequest, PopupInit, TodoEntry } from "../../lib/types";
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
// Speech shortcuts are stored with the logical primary modifier ("cmd" is Ctrl on Windows / Linux) and matched on the
// physical key.
const SPEECH_ON_T = "cmd+t";
/** The second a completed row keeps its place before the todo is taken. */
const GRACE_MS = 1000;

const base = {
  theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project",
  language: "en", warm: true,
};

function ask(id: string, extras: Partial<AskRequest> = {}): PopupInit {
  return { ...base,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Question of ${id}`, predefinedOptions: [] }], ...extras } },
  } as PopupInit;
}

function whatsNext(id: string): PopupInit {
  return ask(id, { whatsNext: true, single: true,
    questions: [{ message: "What next?", predefinedOptions: [
      { text: "Add tests", recommended: false },
      { text: "End this turn", recommended: false },
    ] }] });
}

/** A Stop confirmation of a project: its todos are the live queue, ahead of "continue" and "end". */
function stopCard(id: string): PopupInit {
  return ask(id, { todoQueue: true, single: true, outputFormat: "json",
    questions: [{ message: "Continue or end?", predefinedOptions: [
      { text: "Continue the conversation", recommended: true },
      { text: "End the conversation", recommended: false },
    ] }] });
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

describe("the todo lists of a request page", () => {
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
    // The page keeps ⌘/Ctrl+W inert for a moment after a form appears (Date), and a completed row waits a second
    // before its todo is taken (timers); the tests move past both by hand.
    vi.useFakeTimers({ toFake: ["Date", "setTimeout", "clearTimeout"] });
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
        case "todos_complete":
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
  const press = (target: EventTarget, init: KeyboardEventInit) =>
    target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  const advance = async (ms: number) => {
    vi.advanceTimersByTime(ms);
    await flushPromises();
  };

  const toggle = (w: VueWrapper) => w.find(".todo-toggle");
  const openSection = async (w: VueWrapper) => {
    await toggle(w).trigger("click");
    await flushPromises();
  };
  // The folded section's rows, and the todo rows of a live queue (the options that carry a todo).
  const sectionRows = (w: VueWrapper) => w.findAll(".todo-options .todo-option");
  const sectionTexts = (w: VueWrapper) => sectionRows(w).map((row) => row.find(".label").text());
  const liveRows = (w: VueWrapper) => w.findAll(".option[data-todo-id]");
  const liveTexts = (w: VueWrapper) => liveRows(w).map((row) => row.find(".label").text().replace(/^TODO/, ""));
  const liveRow = (w: VueWrapper, id: string) => w.find(`.option[data-todo-id='${id}']`);
  // What the options column is made of, in order: an option, a todo, or the add row.
  const layout = (w: VueWrapper) =>
    Array.from(w.find(".options").element.children).map((child) =>
      child.classList.contains("todo-add-wrap") ? "add" : child.getAttribute("data-todo-id") ? "todo" : "option");
  const addInput = (w: VueWrapper) => w.find<HTMLTextAreaElement>(".todo-add-input");
  const addButton = (w: VueWrapper) => w.find("button.todo-add");
  const answerBox = (w: VueWrapper) =>
    w.findAll<HTMLTextAreaElement>("textarea").find((box) => !box.classes().includes("todo-add-input"))!;
  const footerButton = (w: VueWrapper, label: string) => w.findAll(".footer .btn").find((b) => b.text().includes(label))!;
  const submissions = () => calls("submit_popup").map((args) => (args as { submission: { answers: Record<string, unknown>[] } }).submission);
  const navbarTodoButton = (w: VueWrapper) => w.find(".nav-actions .nav-btn[title='Project todos']");

  describe("the folded section of an ordinary question", () => {
    beforeEach(() => { store = [todo("a", "first"), todo("b", "second")]; });

    it("counts the todos folded and gives every row a ✓ and a ✕ once open", async () => {
      const { wrapper } = await mountRequest();
      expect(wrapper.find(".todo-section .todo-count").text()).toBe("2");
      expect(wrapper.find(".todo-options").exists()).toBe(false);

      await openSection(wrapper);
      expect(sectionTexts(wrapper)).toEqual(["first", "second"]);
      for (const row of sectionRows(wrapper)) {
        expect(row.find("button[aria-label='Mark done']").exists()).toBe(true);
        expect(row.find("button[aria-label='Delete']").exists()).toBe(true);
      }
      expect(addButton(wrapper).text()).toBe("Add a todo");
    });

    it("strikes a completed row through, takes the todo after its second and lets Undo give it up", async () => {
      const { wrapper } = await mountRequest();
      await openSection(wrapper);

      await sectionRows(wrapper)[0].find(".todo-act-done").trigger("click");
      expect(sectionRows(wrapper)[0].classes()).toContain("todo-completing");
      expect(sectionRows(wrapper)[0].find(".todo-undo").text()).toBe("Undo");
      expect(calls("todos_complete")).toEqual([]);
      await advance(GRACE_MS - 1);
      expect(calls("todos_complete")).toEqual([]);
      await advance(1);
      expect(calls("todos_complete")).toEqual([expect.objectContaining({ project: "/project", id: "a" })]);
      expect(sectionTexts(wrapper)).toEqual(["second"]);

      // Undo inside the second: nothing is ever taken.
      await sectionRows(wrapper)[0].find(".todo-act-done").trigger("click");
      await sectionRows(wrapper)[0].find(".todo-undo").trigger("click");
      expect(sectionRows(wrapper)[0].classes()).not.toContain("todo-completing");
      await advance(GRACE_MS * 3);
      expect(calls("todos_complete")).toHaveLength(1);
      expect(sectionTexts(wrapper)).toEqual(["second"]);
    });

    it("drops a completed todo from the picks, and sends nothing of it with the answer", async () => {
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      await sectionRows(wrapper)[0].trigger("click");
      await sectionRows(wrapper)[1].trigger("click");
      expect(sectionRows(wrapper).map((row) => row.classes().includes("selected"))).toEqual([true, true]);

      await sectionRows(wrapper)[0].find(".todo-act-done").trigger("click");
      expect(sectionRows(wrapper)[0].classes()).not.toContain("selected");
      // A row on its way out is no choice any more.
      await sectionRows(wrapper)[0].trigger("click");
      expect(sectionRows(wrapper)[0].classes()).not.toContain("selected");

      await footerButton(wrapper, "Send").trigger("click");
      await flushPromises();
      // Sending does not wait for the second: the todo is taken now, and only the other pick goes with the answer.
      expect(calls("todos_complete")).toEqual([expect.objectContaining({ id: "a" })]);
      expect(submissions()).toEqual([expect.objectContaining({ answers: [expect.objectContaining({
        todoIds: ["b"], userInput: "Also look into this todo task: second",
      })] })]);
    });

    it("deletes in two steps and a click elsewhere gives the first up", async () => {
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      const first = () => sectionRows(wrapper)[0];

      await first().find(".todo-act-del").trigger("click");
      expect(first().find(".todo-del-confirm").text()).toBe("Confirm delete");
      expect(calls("todos_remove")).toEqual([]);
      // A click on the title strip is a click elsewhere.
      await wrapper.find("header.navbar").trigger("click");
      expect(first().find(".todo-del-confirm").exists()).toBe(false);
      expect(first().find(".todo-act-del").exists()).toBe(true);

      await first().find(".todo-act-del").trigger("click");
      await first().find(".todo-del-confirm").trigger("click");
      await flushPromises();
      expect(calls("todos_remove")).toEqual([expect.objectContaining({ project: "/project", id: "a" })]);
      expect(sectionTexts(wrapper)).toEqual(["second"]);
      expect(calls("todos_complete")).toEqual([]);
    });

    it("adds todos from the dashed row, one after the other, without closing or answering", async () => {
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      await addButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.find(".todo-add.editing .todo-add-hint").text()).toBe(`${primaryShortcutLabel("enter")} to add`);
      expect(document.activeElement).toBe(addInput(wrapper).element);

      await addInput(wrapper).setValue("  third todo ");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(calls("todos_add")).toEqual([expect.objectContaining({ project: "/project", text: "third todo" })]);
      expect(sectionTexts(wrapper)).toEqual(["first", "second", "third todo"]);
      // The new row is lit for a moment and is not picked; the input is empty, open and still has the caret.
      expect(wrapper.find(".todo-option.todo-added .label").text()).toBe("third todo");
      expect(sectionRows(wrapper)[2].classes()).not.toContain("selected");
      expect(addInput(wrapper).element.value).toBe("");
      expect(document.activeElement).toBe(addInput(wrapper).element);

      await addInput(wrapper).setValue("fourth");
      await wrapper.find(".todo-add.editing").trigger("click");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(sectionTexts(wrapper)).toEqual(["first", "second", "third todo", "fourth"]);
      expect(wrapper.find(".todo-count").text()).toBe("4");
      expect(calls("submit_popup")).toEqual([]);

      await advance(2000);
      expect(wrapper.find(".todo-option.todo-added").exists()).toBe(false);
    });

    it("collapses the input with Esc without closing the window or asking to cancel", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my answer");
      await openSection(wrapper);
      await addButton(wrapper).trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("half a thought");

      press(addInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      expect(addInput(wrapper).exists()).toBe(false);
      expect(addButton(wrapper).exists()).toBe(true);
      expect(wrapper.find(".todo-options").exists()).toBe(true);
      expect(wrapper.find(".confirm-overlay").exists()).toBe(false);
      expect(calls("cancel_popup")).toEqual([]);
      expect(answerBox(wrapper).element.value).toBe("my answer");

      // What was typed is still there when the row opens again.
      await addButton(wrapper).trigger("click");
      await flushPromises();
      expect(addInput(wrapper).element.value).toBe("half a thought");
    });

    it("never lets the add row's Enter or Esc reach the answer", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my answer");
      await openSection(wrapper);
      await addButton(wrapper).trigger("click");
      await flushPromises();

      // Enter with nothing typed: ⌘/Ctrl+Enter would send the answer outside the add row.
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      press(addInput(wrapper).element, { key: "Enter" });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
      expect(calls("todos_add")).toEqual([]);
      expect(addInput(wrapper).exists()).toBe(true);

      press(addInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      expect(wrapper.find(".confirm-overlay").exists()).toBe(false);

      // Closed again, the very same key sends the answer.
      press(answerBox(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(calls("submit_popup")).toHaveLength(1);
    });

    it("sends with a bare Enter when that is the send key, and Shift+Enter breaks the line", async () => {
      overrides = { popupSubmitKey: "enter" };
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      await addButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.find(".todo-add-hint").text()).toBe("↵ to add");
      await addInput(wrapper).setValue("line one");
      press(addInput(wrapper).element, { key: "Enter", shiftKey: true });
      expect(calls("todos_add")).toEqual([]);
      press(addInput(wrapper).element, { key: "Enter" });
      await flushPromises();
      expect(calls("todos_add")).toEqual([expect.objectContaining({ text: "line one" })]);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("leaves a bare Enter on ✓, ✕ and the add row to the button, not to the answer", async () => {
      overrides = { popupSubmitKey: "enter" };
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my answer");
      await openSection(wrapper);
      const buttons = [
        sectionRows(wrapper)[0].find(".todo-act-done"),
        sectionRows(wrapper)[0].find(".todo-act-del"),
        addButton(wrapper),
      ];
      for (const button of buttons) press(button.element, { key: "Enter" });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
      // Anywhere else the same key still sends.
      press(answerBox(wrapper).element, { key: "Enter" });
      await flushPromises();
      expect(calls("submit_popup")).toHaveLength(1);
    });

    it("keeps an add failure under the row with the draft intact", async () => {
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      await addButton(wrapper).trigger("click");
      await flushPromises();
      const original = mocks.invoke.getMockImplementation()!;
      mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
        if (command === "todos_add") throw new Error("disk is full");
        return original(command, args);
      });
      await addInput(wrapper).setValue("will not stick");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".todo-add-error").text()).toContain("disk is full");
      expect(addInput(wrapper).element.value).toBe("will not stick");
      expect(sectionTexts(wrapper)).toEqual(["first", "second"]);
    });

    it("keeps an image pasted into the add row away from the answer", async () => {
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      await addButton(wrapper).trigger("click");
      await flushPromises();
      const reachedWindow = vi.fn();
      window.addEventListener("paste", reachedWindow);
      try {
        addInput(wrapper).element.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
        expect(reachedWindow).not.toHaveBeenCalled();
        answerBox(wrapper).element.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
        expect(reachedWindow).toHaveBeenCalledTimes(1);
      } finally {
        window.removeEventListener("paste", reachedWindow);
      }
    });

    it("takes a todo still in its second when the page goes away", async () => {
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      await sectionRows(wrapper)[0].find(".todo-act-done").trigger("click");
      expect(calls("todos_complete")).toEqual([]);
      wrapper.unmount();
      await flushPromises();
      expect(calls("todos_complete")).toEqual([expect.objectContaining({ project: "/project", id: "a" })]);
    });

    it("keeps ✓ ✕ and the add row on a strict choice, where the rows themselves are not choices", async () => {
      interaction = (id) => ask(id, { selectOnly: true,
        questions: [{ message: "Pick one", predefinedOptions: [{ text: "A", recommended: false }, { text: "B", recommended: false }] }] });
      const { wrapper } = await mountRequest();
      await openSection(wrapper);
      const rows = sectionRows(wrapper);
      expect(rows.map((row) => row.classes().includes("static"))).toEqual([true, true]);
      expect(rows[0].find(".check").exists()).toBe(false);
      await rows[0].trigger("click");
      expect(rows[0].classes()).not.toContain("selected");

      await rows[0].find(".todo-act-done").trigger("click");
      await advance(GRACE_MS);
      expect(calls("todos_complete")).toEqual([expect.objectContaining({ id: "a" })]);
      await addButton(wrapper).trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("third");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(sectionTexts(wrapper)).toEqual(["second", "third"]);
    });
  });

  describe("a project with no todo yet", () => {
    it("offers one accent entry instead of a folded header, and the input opens in its place", async () => {
      const { wrapper } = await mountRequest();
      const entry = wrapper.find("button.todo-entry");
      expect(entry.text()).toBe("Add a todo");
      expect(wrapper.find(".todo-count").exists()).toBe(false);
      expect(wrapper.find(".todo-options").exists()).toBe(false);

      await entry.trigger("click");
      await flushPromises();
      expect(wrapper.find("button.todo-entry").exists()).toBe(false);
      expect(document.activeElement).toBe(addInput(wrapper).element);

      // Esc: back to the entry.
      press(addInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      expect(wrapper.find("button.todo-entry").exists()).toBe(true);
    });

    it("becomes the open list with its input still open after the first todo, and back to the entry once emptied", async () => {
      const { wrapper } = await mountRequest();
      await wrapper.find("button.todo-entry").trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("the first one");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".todo-count").text()).toBe("1");
      expect(sectionTexts(wrapper)).toEqual(["the first one"]);
      expect(addInput(wrapper).exists()).toBe(true);
      expect(document.activeElement).toBe(addInput(wrapper).element);

      press(addInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      await sectionRows(wrapper)[0].find(".todo-act-del").trigger("click");
      await sectionRows(wrapper)[0].find(".todo-del-confirm").trigger("click");
      await flushPromises();
      expect(wrapper.find("button.todo-entry").exists()).toBe(true);
      expect(wrapper.find(".todo-count").exists()).toBe(false);
    });

    it("does not flash the entry before the queue has been read", async () => {
      store = [todo("a", "first")];
      const original = mocks.invoke.getMockImplementation()!;
      let release: () => void = () => {};
      mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
        if (command === "todos_list") await new Promise<void>((resolve) => { release = resolve; });
        return original(command, args);
      });
      const { wrapper } = await mountRequest();
      expect(wrapper.find(".todo-section").exists()).toBe(false);
      release();
      await flushPromises();
      expect(wrapper.find(".todo-section .todo-count").text()).toBe("1");
      expect(wrapper.find("button.todo-entry").exists()).toBe(false);
    });
  });

  describe("on a What's next question", () => {
    beforeEach(() => {
      interaction = whatsNext;
      store = [todo("first", "first task"), todo("second", "second task")];
    });

    it("puts ✓ ✕ on the todo rows only and the add row after the last todo, before the end option", async () => {
      const { wrapper } = await mountRequest();
      expect(layout(wrapper)).toEqual(["option", "todo", "todo", "add", "option"]);
      expect(wrapper.find(".todo-section").exists()).toBe(false);
      expect(wrapper.findAll(".option .todo-acts")).toHaveLength(2);
      expect(wrapper.findAll(".option")[0].find(".todo-acts").exists()).toBe(false);
      expect(wrapper.findAll(".option")[3].find(".todo-acts").exists()).toBe(false);
    });

    it("completes a todo like the folded section does, and a completed pick stops being the answer", async () => {
      const { wrapper } = await mountRequest();
      await liveRow(wrapper, "first").trigger("click");
      expect(wrapper.findAll(".option.selected")).toHaveLength(1);

      await liveRow(wrapper, "first").find(".todo-act-done").trigger("click");
      expect(liveRow(wrapper, "first").classes()).toContain("todo-completing");
      expect(wrapper.findAll(".option.selected")).toHaveLength(0);
      expect(liveRow(wrapper, "first").find(".todo-undo").text()).toBe("Undo");
      await advance(GRACE_MS);
      expect(calls("todos_complete")).toEqual([expect.objectContaining({ project: "/project", id: "first" })]);
      expect(layout(wrapper)).toEqual(["option", "todo", "add", "option"]);
      expect(liveTexts(wrapper)).toEqual(["second task"]);
    });

    it("deletes in two steps and keeps the other pick", async () => {
      const { wrapper } = await mountRequest();
      await liveRow(wrapper, "second").trigger("click");
      await liveRow(wrapper, "first").find(".todo-act-del").trigger("click");
      await liveRow(wrapper, "first").find(".todo-del-confirm").trigger("click");
      await flushPromises();
      expect(calls("todos_remove")).toEqual([expect.objectContaining({ project: "/project", id: "first" })]);
      expect(liveTexts(wrapper)).toEqual(["second task"]);
      expect(liveRow(wrapper, "second").classes()).toContain("selected");
      expect(calls("submit_popup")).toEqual([]);
    });

    it("adds a todo at the end of the queue, lit and not picked, with the pick kept", async () => {
      const { wrapper } = await mountRequest();
      await liveRow(wrapper, "second").trigger("click");
      await addButton(wrapper).trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("write docs");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();

      expect(liveTexts(wrapper)).toEqual(["first task", "second task", "write docs"]);
      expect(layout(wrapper)).toEqual(["option", "todo", "todo", "todo", "add", "option"]);
      expect(liveRow(wrapper, "added-1").classes()).toContain("todo-added");
      expect(wrapper.findAll(".option.selected").map((row) => row.find(".label").text().replace(/^TODO/, "")))
        .toEqual(["second task"]);
      expect(addInput(wrapper).exists()).toBe(true);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("keeps the add row for a project with no todo, before the end option", async () => {
      store = [];
      const { wrapper } = await mountRequest();
      expect(layout(wrapper)).toEqual(["option", "add", "option"]);
      await addButton(wrapper).trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("note it down");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(layout(wrapper)).toEqual(["option", "todo", "add", "option"]);
    });

    it("says where a todo went that the question has no room to list", async () => {
      store = [];
      interaction = (id) => ask(id, { whatsNext: true, single: true,
        questions: [{ message: "What next?", predefinedOptions: [
          ...Array.from({ length: 9 }, (_, i) => ({ text: `Idea ${i + 1}`, recommended: false })),
          { text: "End this turn", recommended: false },
        ] }] });
      const { wrapper } = await mountRequest();
      await addButton(wrapper).trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("no room for me");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".todo-add-note").text()).toBe("Added at position 1 (not shown here)");
      expect(liveRows(wrapper)).toHaveLength(0);
      await advance(6000);
      expect(wrapper.find(".todo-add-note").exists()).toBe(false);
    });

    it("keeps Enter in the add row away from the answer", async () => {
      const { wrapper } = await mountRequest();
      await liveRow(wrapper, "first").trigger("click");
      await addButton(wrapper).trigger("click");
      await flushPromises();
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      press(addInput(wrapper).element, { key: "Escape" });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
      expect(calls("cancel_popup")).toEqual([]);
      expect(wrapper.find(".confirm-overlay").exists()).toBe(false);
    });

    it("still submits several todos together with a suggestion", async () => {
      const multi = (id: string) => {
        const init = whatsNext(id);
        if (init.interaction?.type === "ask") init.interaction.request.single = false;
        return init;
      };
      interaction = multi;
      const { wrapper } = await mountRequest();
      await wrapper.findAll(".option")[0].trigger("click");
      await liveRow(wrapper, "first").trigger("click");
      await liveRow(wrapper, "second").trigger("click");
      await footerButton(wrapper, "Send").trigger("click");
      await flushPromises();
      expect(submissions()).toEqual([expect.objectContaining({ answers: [expect.objectContaining({
        selectedOptions: ["Add tests"], userInput: "first task\n\nsecond task", todoIds: ["first", "second"],
      })] })]);
    });
  });

  describe("on a Stop confirmation", () => {
    beforeEach(() => {
      interaction = stopCard;
      store = [todo("a", "tidy the logs"), todo("b", "bump the version")];
    });

    it("is one live queue: the todos, the add row, then continue and end, with no second list below", async () => {
      const { wrapper } = await mountRequest();
      expect(wrapper.find(".todo-section").exists()).toBe(false);
      expect(layout(wrapper)).toEqual(["todo", "todo", "add", "option", "option"]);
      expect(liveTexts(wrapper)).toEqual(["tidy the logs", "bump the version"]);
      expect(wrapper.findAll(".option")[2].text()).toContain("Continue the conversation");
      expect(wrapper.findAll(".option")[3].text()).toContain("End the conversation");
    });

    it("completes, deletes and adds in place", async () => {
      const { wrapper } = await mountRequest();
      await liveRow(wrapper, "a").find(".todo-act-done").trigger("click");
      await advance(GRACE_MS);
      expect(calls("todos_complete")).toEqual([expect.objectContaining({ id: "a" })]);
      expect(layout(wrapper)).toEqual(["todo", "add", "option", "option"]);

      await liveRow(wrapper, "b").find(".todo-act-del").trigger("click");
      await liveRow(wrapper, "b").find(".todo-del-confirm").trigger("click");
      await flushPromises();
      expect(calls("todos_remove")).toEqual([expect.objectContaining({ id: "b" })]);
      expect(layout(wrapper)).toEqual(["add", "option", "option"]);

      await addButton(wrapper).trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("rotate the keys");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(layout(wrapper)).toEqual(["todo", "add", "option", "option"]);
      expect(liveTexts(wrapper)).toEqual(["rotate the keys"]);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("sends a picked todo as its text with its id, and nothing of it once completed", async () => {
      const { wrapper } = await mountRequest();
      await liveRow(wrapper, "b").trigger("click");
      await footerButton(wrapper, "Send").trigger("click");
      await flushPromises();
      expect(submissions()).toEqual([expect.objectContaining({ answers: [expect.objectContaining({
        selectedOptions: [], userInput: "bump the version", todoIds: ["b"],
        todoSelections: [{ id: "b", attachments: [] }],
      })] })]);
    });

    it("keeps the end option an end option while a todo is added", async () => {
      const { wrapper } = await mountRequest();
      await wrapper.findAll(".option")[3].trigger("click");
      await addButton(wrapper).trigger("click");
      await flushPromises();
      await addInput(wrapper).setValue("one more");
      press(addInput(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(wrapper.findAll(".option.selected").map((row) => row.text())).toEqual([
        expect.stringContaining("End the conversation"),
      ]);
    });
  });

  describe("⌘/Ctrl+T", () => {
    beforeEach(() => { store = [todo("a", "first"), todo("b", "second")]; });

    it("opens the folded section and puts the caret in its add row", async () => {
      const { wrapper } = await mountRequest();
      expect(wrapper.find(".todo-options").exists()).toBe(false);
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".todo-options").exists()).toBe(true);
      expect(document.activeElement).toBe(addInput(wrapper).element);
      expect(calls("open_todos")).toEqual([]);

      // With the caret already there it does nothing more (and types no "t").
      const event = new KeyboardEvent("keydown", { key: "t", code: "KeyT", bubbles: true, cancelable: true, ...PRIMARY });
      addInput(wrapper).element.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(addInput(wrapper).element);
      expect(calls("open_todos")).toEqual([]);
    });

    it("works from the answer box too, and the entry of an empty project", async () => {
      store = [];
      const { wrapper } = await mountRequest();
      answerBox(wrapper).element.focus();
      press(answerBox(wrapper).element, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(document.activeElement).toBe(addInput(wrapper).element);
      expect(wrapper.find("button.todo-entry").exists()).toBe(false);
    });

    it("brings the whole add row into view, not just the textarea, and again as the input grows", async () => {
      // Scrolling the textarea alone leaves the row's lower edge under the footer.
      const scrolled: Element[] = [];
      const original = Element.prototype.scrollIntoView;
      Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { scrolled.push(this); };
      try {
        const { wrapper } = await mountRequest();
        press(window, { key: "t", code: "KeyT", ...PRIMARY });
        await flushPromises();
        const row = wrapper.find(".todo-add-wrap").element;
        expect(scrolled).toContain(row);
        expect(scrolled).not.toContain(addInput(wrapper).element);

        scrolled.length = 0;
        await addInput(wrapper).setValue("one\ntwo\nthree");
        await flushPromises();
        expect(scrolled).toContain(row);
      } finally {
        Element.prototype.scrollIntoView = original;
      }
    });

    it("focuses the add row of a live queue", async () => {
      interaction = whatsNext;
      const { wrapper } = await mountRequest();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(document.activeElement).toBe(addInput(wrapper).element);
      expect(layout(wrapper)).toEqual(["option", "todo", "todo", "add", "option"]);
    });

    it("opens the Todos window on a permission card, which has no list of its own", async () => {
      interaction = permission;
      const { wrapper } = await mountRequest();
      expect(wrapper.find(".todo-section").exists()).toBe(false);
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(calls("open_todos")).toEqual([{ dir: "/project" }]);
      expect(addInput(wrapper).exists()).toBe(false);
    });

    it("opens the Todos window, with no project preselected, for a request without one", async () => {
      overrides = { project: "", projectName: "" };
      const { wrapper } = await mountRequest();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(calls("open_todos")).toEqual([{ dir: null }]);
      expect(addInput(wrapper).exists()).toBe(false);
    });

    it("yields to the speech shortcut", async () => {
      speechAvailable = true;
      overrides = { speechShortcut: SPEECH_ON_T };
      const { wrapper } = await mountRequest();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(addInput(wrapper).exists()).toBe(false);
      expect(calls("open_todos")).toEqual([]);
    });

    it("opens when speech input is on another shortcut", async () => {
      speechAvailable = true;
      overrides = { speechShortcut: "cmd+d" };
      const { wrapper } = await mountRequest();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(addInput(wrapper).exists()).toBe(true);
    });

    it("stays out of the way while the find bar is open", async () => {
      const { wrapper } = await mountRequest();
      press(window, { key: "f", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".popup-find-bar").exists()).toBe(true);
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(addInput(wrapper).exists()).toBe(false);
      expect(calls("open_todos")).toEqual([]);
    });

    it("is not heard by a request that is not the active one of a shared window", async () => {
      const merged = await mountRequest("a", true);
      merged.active.value = false;
      await flushPromises();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(addInput(merged.wrapper).exists()).toBe(false);
      expect(calls("open_todos")).toEqual([]);
    });
  });

  describe("the navbar button", () => {
    it("opens the Todos window with the project preselected instead of a popover", async () => {
      const { wrapper } = await mountRequest();
      await navbarTodoButton(wrapper).trigger("click");
      await flushPromises();
      expect(calls("open_todos")).toEqual([{ dir: "/project" }]);
      expect(wrapper.find(".todo-popover").exists()).toBe(false);
      expect(addInput(wrapper).exists()).toBe(false);
    });

    it("opens it with nothing preselected for a request without a project", async () => {
      overrides = { project: "", projectName: "" };
      const { wrapper } = await mountRequest();
      await navbarTodoButton(wrapper).trigger("click");
      await flushPromises();
      expect(calls("open_todos")).toEqual([{ dir: null }]);
    });

    it("is there on a permission card as well", async () => {
      interaction = permission;
      const { wrapper } = await mountRequest();
      await navbarTodoButton(wrapper).trigger("click");
      await flushPromises();
      expect(calls("open_todos")).toEqual([{ dir: "/project" }]);
    });
  });
});
