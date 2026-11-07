// Notes on passages of the Agent's text inside the real popup (docs/specs/message-annotations.md D10–D16): which requests
// offer them, the button beside a selection, the editor and its keys, the highlights, the footer chip and its list, and
// what goes out with the answer.
import { computed, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import { desktopPlatform, primaryShortcutLabel } from "../../lib/platform";
import type { PopupInit } from "../../lib/types";
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

const base = {
  theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project",
  language: "en", warm: true,
};

const MESSAGE =
  "The migration drops three **indexes** and rewrites the cache layer.\n\nRegression tests were added; staging looks 18% slower.";

function ask(id: string, patch: Record<string, unknown> = {}): PopupInit {
  return { ...base,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      annotatable: true, message: { text: MESSAGE, files: [] },
      questions: [{ message: "Which one should I take?", predefinedOptions: [] }], ...patch } },
  } as PopupInit;
}

describe("notes on the Agent's text", () => {
  const wrappers: VueWrapper[] = [];
  let interaction: () => PopupInit;
  let overrides: Record<string, unknown>;
  let scrolled: Element[];

  beforeEach(() => {
    vi.clearAllMocks();
    interaction = () => ask("a");
    overrides = {};
    scrolled = [];
    // Date only: the page keeps ⌘/Ctrl+W inert for a moment after a form appears, and the tests move past it.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    // The reading-progress observer of a long Message needs one.
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
    });
    // jsdom has no layout: give a selection a box, and let a scroll be seen.
    Range.prototype.getBoundingClientRect = () => ({
      x: 100, y: 120, left: 100, top: 120, right: 220, bottom: 140, width: 120, height: 20, toJSON: () => ({}),
    }) as DOMRect;
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { scrolled.push(this); };
    mocks.invoke.mockImplementation(async (command: string) => {
      switch (command) {
        case "popup_init": return { ...interaction(), ...overrides };
        case "popup_agent_resolved": return {};
        case "popup_update_state": return { available: false, pending: false };
        case "speech_available": return false;
        case "todos_list": return [];
        default: return false;
      }
    });
  });

  afterEach(() => {
    wrappers.splice(0).forEach((w) => { try { w.unmount(); } catch { /* already unmounted */ } });
    document.body.innerHTML = "";
    window.getSelection()?.removeAllRanges();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  async function mountPopup(scope?: PopupScope) {
    const wrapper = mount(PopupView, {
      props: scope ? { scope } : {},
      attachTo: document.body,
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper);
    await flushPromises();
    vi.setSystemTime(Date.now() + 1000);
    return wrapper;
  }

  const calls = (command: string) => mocks.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const press = (target: EventTarget, init: KeyboardEventInit) =>
    target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  const root = () => document.querySelector<HTMLElement>("[data-annotation-root]");
  const fab = (w: VueWrapper) => w.find<HTMLButtonElement>(".an-fab");
  const editor = (w: VueWrapper) => w.find(".an-editor");
  const editorInput = (w: VueWrapper) => w.find<HTMLTextAreaElement>(".an-editor .an-textarea");
  const chip = (w: VueWrapper) => w.find<HTMLButtonElement>(".an-chip");
  const rows = (w: VueWrapper) => w.findAll(".an-row");
  const marks = (id?: number) => Array.from(document.querySelectorAll<HTMLElement>(
    id === undefined ? "mark.an-mark" : `mark.an-mark[data-annotation-id="${id}"]`));
  const answerBox = (w: VueWrapper) => w.findAll<HTMLTextAreaElement>("textarea").find((box) => !box.classes().includes("an-textarea"))!;
  const footerButton = (w: VueWrapper, label: string) => w.findAll(".footer .btn").find((b) => b.text().includes(label))!;
  const sourceToggle = (w: VueWrapper) => w.find(".msg-tools .mt-btn:nth-child(2)");

  function textNodeWith(from: HTMLElement, needle: string): { node: Text; index: number } {
    const walker = document.createTreeWalker(from, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const index = (node as Text).data.indexOf(needle);
      if (index >= 0) return { node: node as Text, index };
    }
    throw new Error(`no text node holds "${needle}"`);
  }

  /** Select text the way a mouse drag does: press, the selection forms, release. */
  async function select(from: string, to = from, within: HTMLElement | null = root()) {
    const host = within as HTMLElement;
    const head = textNodeWith(host, from);
    const tail = textNodeWith(host, to);
    const range = document.createRange();
    range.setStart(head.node, head.index);
    range.setEnd(tail.node, tail.index + to.length);
    host.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    document.dispatchEvent(new MouseEvent("pointerup", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 10));
    await flushPromises();
  }

  async function addNote(w: VueWrapper, from: string, note: string, to = from) {
    await select(from, to);
    await fab(w).trigger("click");
    await flushPromises();
    await editorInput(w).setValue(note);
    await editor(w).find(".btn-primary").trigger("click");
    await flushPromises();
  }

  describe("which requests offer them", () => {
    it("lets the Message be annotated", async () => {
      const w = await mountPopup();
      expect(root()?.textContent).toContain("The migration drops three indexes");
      await select("rewrites the cache");
      expect(fab(w).exists()).toBe(true);
      expect(fab(w).text()).toBe("Add annotation");
    });

    it("hides the button again once the selection is gone", async () => {
      const w = await mountPopup();
      await select("indexes");
      expect(fab(w).exists()).toBe(true);
      window.getSelection()?.removeAllRanges();
      document.dispatchEvent(new Event("selectionchange"));
      await flushPromises();
      expect(fab(w).exists()).toBe(false);
    });

    it("offers nothing for a selection outside the Message", async () => {
      const w = await mountPopup();
      const question = w.find(".question-pane .markdown-body, .question-pane .plain-body").element as HTMLElement;
      expect(question.hasAttribute("data-annotation-root")).toBe(false);
      await select("Which one", "Which one", question);
      expect(fab(w).exists()).toBe(false);
    });

    it("offers nothing when the daemon did not mark the request annotatable", async () => {
      interaction = () => ask("a", { annotatable: false });
      const w = await mountPopup();
      expect(root()).toBeNull();
      expect(fab(w).exists()).toBe(false);
      expect(chip(w).exists()).toBe(false);
    });

    it("lets a direct ask's question be annotated when there is no Message", async () => {
      interaction = () => ask("a", {
        message: { text: "", files: [] },
        questions: [{ message: "Should the **index** be added now, given the 18% slowdown?", predefinedOptions: [] }],
      });
      const w = await mountPopup();
      expect(root()?.textContent).toContain("Should the index be added now");
      await select("given the 18% slowdown");
      expect(fab(w).exists()).toBe(true);
    });

    it("leaves a question beside a Message, several questions and What's next alone", async () => {
      interaction = () => ask("a", {
        message: { text: "", files: [] },
        questions: [{ message: "First?", predefinedOptions: [] }, { message: "Second?", predefinedOptions: [] }],
      });
      await mountPopup();
      expect(root()).toBeNull();
      document.body.innerHTML = "";

      interaction = () => ask("a", { message: { text: "", files: [] }, whatsNext: true, single: true,
        questions: [{ message: "What next?", predefinedOptions: [{ text: "Add tests", recommended: false }] }] });
      await mountPopup();
      expect(root()).toBeNull();
    });

    it("still annotates the Message of a What's next request", async () => {
      interaction = () => ask("a", { whatsNext: true, single: true,
        questions: [{ message: "What next?", predefinedOptions: [{ text: "Add tests", recommended: false }] }] });
      const w = await mountPopup();
      await select("staging looks 18% slower");
      expect(fab(w).exists()).toBe(true);
    });

    it("offers nothing on a confirmation card", async () => {
      interaction = () => ({ ...base, interaction: { type: "confirm", request: {
        id: "c", title: "Run command?", context: [], detail: { summary: "Needed", bodyMd: "`cargo test`" },
        choices: [{ id: "allow", label: "Allow once", description: "", role: "primary" }, { id: "deny", label: "Deny", description: "", role: "destructive" }],
        presentation: { type: "singleSelectSubmit", submitLabel: "Submit" }, dismissActionId: "deny", createdAtMs: 1, expiresAtMs: 2,
      } } }) as PopupInit;
      const w = await mountPopup();
      expect(w.find(".an-layer").exists()).toBe(false);
      expect(root()).toBeNull();
    });
  });

  describe("the editor", () => {
    it("opens from the button with the passage on top, focuses the input and outlines the passage", async () => {
      const w = await mountPopup();
      await select("rewrites the cache layer");
      await fab(w).trigger("click");
      await flushPromises();
      expect(fab(w).exists()).toBe(false);
      expect(editor(w).find(".an-quote").text()).toBe("rewrites the cache layer");
      expect(editor(w).find(".an-hint").text()).toBe(`${primaryShortcutLabel("enter")} to add`);
      expect(document.activeElement).toBe(editorInput(w).element);
      expect(Array.from(document.querySelectorAll("mark.an-draft")).map((m) => m.textContent)).toEqual(["rewrites the cache layer"]);
      expect(window.getSelection()?.isCollapsed).toBe(true);
    });

    it("can take the focus the moment it opens, before it has been placed", async () => {
      const w = await mountPopup();
      await select("three");
      // A browser refuses to focus an element that is `visibility: hidden`; jsdom does not, so look at what the input is
      // told while the focus is being given.
      const seen: string[] = [];
      const focus = HTMLElement.prototype.focus;
      HTMLElement.prototype.focus = function (this: HTMLElement, ...args: Parameters<HTMLElement["focus"]>) {
        if (this.classList.contains("an-textarea")) seen.push(getComputedStyle(this).visibility);
        return focus.apply(this, args);
      };
      try {
        await fab(w).trigger("click");
        await flushPromises();
      } finally {
        HTMLElement.prototype.focus = focus;
      }
      expect(seen.length).toBeGreaterThan(0);
      expect(seen).not.toContain("hidden");
      expect(document.activeElement).toBe(editorInput(w).element);
    });

    it("adds the note with the add button and draws a numbered highlight", async () => {
      const w = await mountPopup();
      await addNote(w, "rewrites the cache layer", "Why a rewrite?");
      expect(editor(w).exists()).toBe(false);
      expect(document.querySelector("mark.an-draft")).toBeNull();
      expect(marks().map((m) => m.textContent)).toEqual(["rewrites the cache layer"]);
      expect(marks()[0].getAttribute("data-annotation-n")).toBe("1");
      expect(chip(w).text()).toContain("Annotations");
      expect(chip(w).find(".an-count").text()).toBe("1");
    });

    it("adds with the send shortcut and never lets that Enter send the answer", async () => {
      const w = await mountPopup();
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      await editorInput(w).setValue("Which three?");
      press(editorInput(w).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(marks()).toHaveLength(1);
      expect(editor(w).exists()).toBe(false);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("ignores a bare Enter (a newline) and one inside an IME composition", async () => {
      const w = await mountPopup();
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      await editorInput(w).setValue("Which three?");
      press(editorInput(w).element, { key: "Enter" });
      press(editorInput(w).element, { key: "Enter", keyCode: 229, isComposing: true, ...PRIMARY });
      await flushPromises();
      expect(editor(w).exists()).toBe(true);
      expect(marks()).toHaveLength(0);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("follows the send-key setting: Enter adds and Shift+Enter breaks the line", async () => {
      overrides = { popupSubmitKey: "enter" };
      const w = await mountPopup();
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      expect(editor(w).find(".an-hint").text()).toBe("↵ to add");
      await editorInput(w).setValue("Which three?");
      press(editorInput(w).element, { key: "Enter", shiftKey: true });
      expect(marks()).toHaveLength(0);
      press(editorInput(w).element, { key: "Enter" });
      await flushPromises();
      expect(marks()).toHaveLength(1);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("closes with Esc without touching the rest of the popup", async () => {
      const w = await mountPopup();
      await answerBox(w).setValue("my answer");
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      await editorInput(w).setValue("never mind");
      press(editorInput(w).element, { key: "Escape" });
      await flushPromises();
      expect(editor(w).exists()).toBe(false);
      expect(document.querySelector("mark.an-draft")).toBeNull();
      expect(marks()).toHaveLength(0);
      expect(w.find(".confirm-overlay").exists()).toBe(false);
      expect(calls("cancel_popup")).toEqual([]);
      expect(answerBox(w).element.value).toBe("my answer");
    });

    it("keeps its keys to itself while the focus is inside it", async () => {
      const w = await mountPopup();
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      press(editorInput(w).element, { key: "k", code: "KeyK", ...PRIMARY });
      press(editorInput(w).element, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(w.find(".qr-menu").exists()).toBe(false);
      expect(w.find(".todo-popover").exists()).toBe(false);
      expect(editor(w).exists()).toBe(true);
    });

    it("hands the keyboard back when the focus is in the answer box", async () => {
      const w = await mountPopup();
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      await editorInput(w).setValue("kept");
      answerBox(w).element.focus();
      press(answerBox(w).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      // The answer goes out, and the note still being typed goes with it.
      expect(calls("submit_popup")).toHaveLength(1);
      expect((calls("submit_popup")[0] as { submission: { annotations?: unknown[] } }).submission.annotations).toEqual([
        { quote: "three", note: "kept" },
      ]);
    });

    it("closes an empty editor on a press outside and keeps one with text", async () => {
      const w = await mountPopup();
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      await flushPromises();
      expect(editor(w).exists()).toBe(false);

      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      await editorInput(w).setValue("typed");
      document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      await flushPromises();
      expect(editor(w).exists()).toBe(true);
    });

    it("takes no note without text", async () => {
      const w = await mountPopup();
      await select("three");
      await fab(w).trigger("click");
      await flushPromises();
      expect(editor(w).find(".btn-primary").attributes("disabled")).toBeDefined();
      await editorInput(w).setValue("   ");
      press(editorInput(w).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(marks()).toHaveLength(0);
      expect(editor(w).exists()).toBe(true);
    });
  });

  describe("an existing note", () => {
    it("turns the button into an edit when the selection touches it, and saves the new text", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await select("three", "and rewrites");
      expect(fab(w).text()).toBe("Edit annotation");
      await fab(w).trigger("click");
      await flushPromises();
      expect(editorInput(w).element.value).toBe("Which ones?");
      expect(editor(w).find(".an-quote").text()).toBe("indexes");
      await editorInput(w).setValue("Which three indexes?");
      await editor(w).find(".btn-primary").trigger("click");
      await flushPromises();
      expect(chip(w).find(".an-count").text()).toBe("1");
      expect(marks()).toHaveLength(1);
      await chip(w).trigger("click");
      expect(rows(w)[0].find(".an-row-text").text()).toBe("Which three indexes?");
    });

    it("opens the editor from a click on its highlight and deletes from there", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      marks(1)[0].click();
      await flushPromises();
      expect(editorInput(w).element.value).toBe("Which ones?");
      await editor(w).find(".an-delete").trigger("click");
      await flushPromises();
      expect(marks()).toHaveLength(0);
      expect(chip(w).exists()).toBe(false);
    });

    it("numbers the notes by their place in the text, whatever the order they were written in", async () => {
      const w = await mountPopup();
      await addNote(w, "18% slower", "Slower how?");
      await addNote(w, "indexes", "Which ones?");
      const numbered = marks().map((m) => [m.textContent, m.getAttribute("data-annotation-n")]);
      expect(numbered).toEqual([["indexes", "1"], ["18% slower", "2"]]);
      await chip(w).trigger("click");
      expect(rows(w).map((row) => row.find(".an-num").text())).toEqual(["1", "2"]);
      expect(rows(w).map((row) => row.find(".an-row-text").text())).toEqual(["Which ones?", "Slower how?"]);
    });

    it("stops adding at twenty notes and says so", async () => {
      const w = await mountPopup();
      const ctx = (w.vm as unknown as { ctx: { annotationNotes: { value: unknown[] } } }).ctx;
      for (let i = 0; i < 20; i++) ctx.annotationNotes.value.push({ id: 100 + i, start: 1000 + i, end: 1001 + i, quote: `gone ${i}`, note: `n${i}`, anchored: false });
      await flushPromises();
      await select("rewrites");
      expect(fab(w).text()).toBe("Annotation limit reached (20)");
      expect(fab(w).attributes("disabled")).toBeDefined();
    });
  });

  describe("the footer chip and its list", () => {
    it("exists only with notes and sits between the quick-reply menu and Send", async () => {
      const w = await mountPopup();
      expect(chip(w).exists()).toBe(false);
      await addNote(w, "indexes", "Which ones?");
      const footer = w.find(".footer").element;
      const order = Array.from(footer.children).map((el) => (el.classList.contains("an-anchor") ? "chip" : el.classList.contains("qr-anchor") ? "quick" : el.classList.contains("btn-primary") ? "send" : "other"));
      expect(order.indexOf("quick")).toBeLessThan(order.indexOf("chip"));
      expect(order.indexOf("chip")).toBeLessThan(order.indexOf("send"));
    });

    it("keeps its name and count when a narrow window hides the label", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await addNote(w, "18% slower", "Slower how?");
      // popup.css hides .an-chip-label below 520 px; the accessible name must not depend on it.
      expect(chip(w).attributes("aria-label")).toBe("Annotations 2");
      expect(chip(w).find(".an-chip-label").text()).toBe("Annotations");
      expect(chip(w).find(".an-count").text()).toBe("2");
    });

    it("lists the notes, finds a passage on a click and pulses it", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await addNote(w, "18% slower", "Slower how?");
      await chip(w).trigger("click");
      await flushPromises();
      expect(w.find(".an-list .qr-head").text()).toContain("Annotations · 2");
      expect(rows(w)[0].find(".an-row-quote").text()).toBe("“indexes”");
      scrolled.length = 0;
      await rows(w)[1].trigger("click");
      expect(scrolled.map((el) => el.getAttribute("data-annotation-id"))).toEqual([marks()[1].getAttribute("data-annotation-id")]);
      expect(marks()[1].classList.contains("an-flash")).toBe(true);
      expect(w.find(".an-list").exists()).toBe(true);
    });

    it("edits and deletes a row from its own buttons", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await addNote(w, "18% slower", "Slower how?");
      await chip(w).trigger("click");
      await rows(w)[1].find(".an-icon-btn[aria-label='Delete']").trigger("click");
      await flushPromises();
      expect(rows(w)).toHaveLength(1);
      expect(marks()).toHaveLength(1);
      await rows(w)[0].find(".an-icon-btn[aria-label='Edit']").trigger("click");
      await flushPromises();
      expect(w.find(".an-list").exists()).toBe(false);
      expect(editorInput(w).element.value).toBe("Which ones?");
    });

    it("gives the focus back to where it was after a note edited from the list is done", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      chip(w).element.focus();
      await chip(w).trigger("click");
      await flushPromises();
      await rows(w)[0].find(".an-icon-btn[aria-label='Edit']").trigger("click");
      await flushPromises();
      expect(document.activeElement).toBe(editorInput(w).element);
      press(editorInput(w).element, { key: "Escape" });
      await flushPromises();
      expect(document.activeElement).toBe(chip(w).element);
    });

    it("is driven from the keyboard: arrows, Enter, E, Delete and Esc", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await addNote(w, "18% slower", "Slower how?");
      await chip(w).trigger("click");
      await flushPromises();
      const hovered = () => rows(w).findIndex((row) => row.classes().includes("hover"));
      expect(hovered()).toBe(0);
      expect(document.activeElement).toBe(rows(w)[0].element);
      press(document.activeElement as Element, { key: "ArrowDown" });
      await flushPromises();
      expect(hovered()).toBe(1);
      scrolled.length = 0;
      press(document.activeElement as Element, { key: "Enter" });
      await flushPromises();
      expect(scrolled).toHaveLength(1);
      expect(w.find(".an-list").exists()).toBe(true);
      press(document.activeElement as Element, { key: "ArrowUp" });
      press(document.activeElement as Element, { key: "ArrowUp" });
      await flushPromises();
      expect(hovered()).toBe(1);

      press(document.activeElement as Element, { key: "Delete" });
      await flushPromises();
      expect(rows(w)).toHaveLength(1);
      press(document.activeElement as Element, { key: "e" });
      await flushPromises();
      expect(w.find(".an-list").exists()).toBe(false);
      expect(editorInput(w).element.value).toBe("Which ones?");
      press(editorInput(w).element, { key: "Escape" });
      await flushPromises();

      await chip(w).trigger("click");
      await flushPromises();
      press(document.activeElement as Element, { key: "Escape" });
      await flushPromises();
      expect(w.find(".an-list").exists()).toBe(false);
      expect(calls("cancel_popup")).toEqual([]);
    });

    it("is modal: Enter never sends and the quick-reply shortcut does not open over it", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await chip(w).trigger("click");
      await flushPromises();
      press(document.activeElement as Element, { key: "k", ...PRIMARY });
      await flushPromises();
      expect(w.find(".qr-menu:not(.an-list)").exists()).toBe(false);
      press(document.activeElement as Element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
      expect(w.find(".an-list").exists()).toBe(true);
    });

    it("keeps a note whose passage cannot be found in the list, flagged", async () => {
      const w = await mountPopup();
      await addNote(w, "three", "Which three?", "and rewrites");
      expect(marks()).not.toHaveLength(0);
      // The source view shows the Markdown as written, where the passage is split by the ** around "indexes".
      await sourceToggle(w).trigger("click");
      await flushPromises();
      expect(marks()).toHaveLength(0);
      await chip(w).trigger("click");
      expect(rows(w)).toHaveLength(1);
      expect(rows(w)[0].find(".an-row-lost").text()).toBe("Original text not found");
      press(document.activeElement as Element, { key: "Escape" });
      await flushPromises();
      // It is still sent.
      await footerButton(w, "Send").trigger("click");
      await flushPromises();
      expect((calls("submit_popup")[0] as { submission: { annotations: unknown[] } }).submission.annotations).toEqual([
        { quote: "three indexes and rewrites", note: "Which three?" },
      ]);
    });

    it("finds the passage again when the rendered view comes back", async () => {
      const w = await mountPopup();
      await addNote(w, "three", "Which three?", "and rewrites");
      await sourceToggle(w).trigger("click");
      await flushPromises();
      expect(marks()).toHaveLength(0);
      await sourceToggle(w).trigger("click");
      await flushPromises();
      expect(marks().map((m) => m.textContent).join("")).toBe("three indexes and rewrites");
      await chip(w).trigger("click");
      expect(rows(w)[0].find(".an-row-lost").exists()).toBe(false);
    });

    it("follows a passage into the source view when the Markdown still contains it", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await sourceToggle(w).trigger("click");
      await flushPromises();
      expect(root()?.tagName).toBe("PRE");
      expect(marks().map((m) => m.textContent)).toEqual(["indexes"]);
      await sourceToggle(w).trigger("click");
      await flushPromises();
      expect(root()?.tagName).not.toBe("PRE");
      expect(marks().map((m) => m.textContent)).toEqual(["indexes"]);
    });
  });

  describe("cancelling and closing", () => {
    it("asks before throwing notes away, like an answer", async () => {
      const w = await mountPopup();
      await footerButton(w, "Cancel").trigger("click");
      await flushPromises();
      expect(w.find(".confirm-overlay").exists()).toBe(false);
      expect(calls("cancel_popup")).toHaveLength(1);

      mocks.invoke.mockClear();
      document.body.innerHTML = "";
      const second = await mountPopup();
      await addNote(second, "indexes", "Which ones?");
      await footerButton(second, "Cancel").trigger("click");
      await flushPromises();
      expect(second.find(".confirm-desc").text()).toBe("All entered answers and annotations will be lost.");
      expect(calls("cancel_popup")).toEqual([]);
      await second.find(".confirm-actions .btn-danger").trigger("click");
      await flushPromises();
      expect(calls("cancel_popup")).toHaveLength(1);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("marks the request as holding a draft in the shared window", async () => {
      const draft = vi.fn();
      const scope = { requestId: "a", active: ref(true), blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft,
        preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }) } as unknown as PopupScope;
      const w = await mountPopup(scope);
      expect(draft).toHaveBeenLastCalledWith("a", false);
      await addNote(w, "indexes", "Which ones?");
      expect(draft).toHaveBeenLastCalledWith("a", true);
    });
  });

  describe("what goes out with the answer", () => {
    it("sends the notes in reading order next to the answer", async () => {
      const w = await mountPopup();
      await addNote(w, "18% slower", "Slower how?");
      await addNote(w, "indexes", "Which ones?");
      await answerBox(w).setValue("Take the first.");
      await footerButton(w, "Send").trigger("click");
      await flushPromises();
      const [call] = calls("submit_popup") as Array<{ submission: { answers: Array<{ userInput: string }>; annotations: unknown[] } }>;
      expect(call.submission.answers[0].userInput).toBe("Take the first.");
      expect(call.submission.annotations).toEqual([
        { quote: "indexes", note: "Which ones?" },
        { quote: "18% slower", note: "Slower how?" },
      ]);
    });

    it("lets notes go out alone, with no answer at all", async () => {
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await footerButton(w, "Send").trigger("click");
      await flushPromises();
      const [call] = calls("submit_popup") as Array<{ submission: { answers: Array<{ userInput: string; selectedOptions: string[] }>; annotations: unknown[] } }>;
      expect(call.submission.answers[0].userInput).toBe("");
      expect(call.submission.annotations).toEqual([{ quote: "indexes", note: "Which ones?" }]);
    });

    it("sends no annotations field when there are none", async () => {
      const w = await mountPopup();
      await answerBox(w).setValue("Take the first.");
      await footerButton(w, "Send").trigger("click");
      await flushPromises();
      const [call] = calls("submit_popup") as Array<{ submission: Record<string, unknown> }>;
      expect("annotations" in call.submission).toBe(false);
    });

    it("keeps a note that is still being typed when Send is clicked", async () => {
      const w = await mountPopup();
      await select("indexes");
      await fab(w).trigger("click");
      await flushPromises();
      await editorInput(w).setValue("Still typing");
      await footerButton(w, "Send").trigger("click");
      await flushPromises();
      const [call] = calls("submit_popup") as Array<{ submission: { annotations: unknown[] } }>;
      expect(call.submission.annotations).toEqual([{ quote: "indexes", note: "Still typing" }]);
    });

    it("lets a quick reply carry the notes too", async () => {
      interaction = () => ask("a", { quickReplies: ["Looks good"] });
      const w = await mountPopup();
      await addNote(w, "indexes", "Which ones?");
      await w.find(".footer .qr-trigger:not(.an-chip)").trigger("click");
      await flushPromises();
      await w.find(".qr-item[data-item='preset-0']").trigger("click");
      await flushPromises();
      const [call] = calls("submit_popup") as Array<{ submission: { answers: Array<{ userInput: string }>; annotations: unknown[] } }>;
      expect(call.submission.answers[0].userInput).toBe("Looks good");
      expect(call.submission.annotations).toEqual([{ quote: "indexes", note: "Which ones?" }]);
    });
  });
});
