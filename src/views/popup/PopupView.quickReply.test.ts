// The footer's quick-reply menu inside the real popup (docs/specs/special-replies.md D18–D21): which requests offer it,
// what it does to the request's own keys while it is open, the directive that waits for Send, and the banner of an
// escalated question.
import { computed, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import zh from "../../i18n/zh";
import { desktopPlatform, primaryShortcutLabel } from "../../lib/platform";
import type { AskRequest, ConfirmRequest, PopupInit } from "../../lib/types";
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
const SPEECH_ON_K = "cmd+k";
const MANAGE = "Manage quick replies…";

const base = {
  theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project",
  language: "en", warm: true,
};

type RequestExtras = Partial<AskRequest>;

function ask(id: string, extras: RequestExtras = {}, questions = 1): PopupInit {
  return { ...base,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] },
      questions: Array.from({ length: questions }, (_, i) => ({
        message: `Question ${i + 1} of ${id}`,
        predefinedOptions: [{ text: "Yes", recommended: false }, { text: "No", recommended: false }],
      })), ...extras } },
  } as PopupInit;
}

function whatsNext(id: string): PopupInit {
  return ask(id, {
    single: true, whatsNext: true,
    questions: [{ message: "What next?", predefinedOptions: [
      { text: "Add tests", recommended: false }, { text: "End this turn", recommended: false },
    ] }],
  });
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

describe("the quick-reply menu", () => {
  const wrappers: VueWrapper[] = [];
  let interaction: (id: string) => PopupInit;
  let overrides: Record<string, unknown>;
  let speechAvailable: boolean;
  let awayEndFails: boolean;

  beforeEach(() => {
    vi.clearAllMocks();
    interaction = (id) => ask(id);
    overrides = {};
    speechAvailable = false;
    awayEndFails = false;
    // Date only: the page keeps ⌘/Ctrl+W inert for a moment after a form appears, and the tests move past it.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    // The vertical multi-question layout watches which card is in view.
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
    });
    mocks.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
      switch (command) {
        case "popup_init": return { ...interaction(String(args.requestId ?? "solo")), ...overrides };
        case "popup_agent_resolved": return {};
        case "popup_update_state": return { available: false, pending: false };
        case "speech_available": return speechAvailable;
        case "todos_list": return [];
        case "away_end":
          if (awayEndFails) throw new Error("daemon is restarting");
          return null;
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
  async function mountRequest(id = "a", scoped = false, locale: "en" | "zh" = "en") {
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
      global: { plugins: [createI18n({ legacy: false, locale, messages: { en, zh } })] },
    });
    wrappers.push(wrapper);
    await flushPromises();
    vi.setSystemTime(Date.now() + 1000);
    return { wrapper, close, active, blocked };
  }

  const calls = (command: string) => mocks.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const submitted = () => calls("submit_popup").map((args) => (args as { submission: Record<string, unknown> }).submission);
  const trigger = (w: VueWrapper) => w.find<HTMLButtonElement>(".footer .qr-trigger");
  const menu = (w: VueWrapper) => w.find(".qr-menu");
  const items = (w: VueWrapper) => w.findAll(".qr-item");
  const itemLabels = (w: VueWrapper) =>
    items(w).map((item) => (item.find(".qr-item-text").exists() ? item.find(".qr-item-text").text() : item.text()));
  const bar = (w: VueWrapper) => w.find(".qr-bar");
  const banner = (w: VueWrapper) => w.find(".qr-banner");
  const primary = (w: VueWrapper) => w.find<HTMLButtonElement>(".footer .btn-primary");
  const answerBox = (w: VueWrapper) => w.findAll<HTMLTextAreaElement>("textarea")[0];
  const press = (target: EventTarget, init: KeyboardEventInit) =>
    target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  const openWithKey = async (w: VueWrapper) => {
    press(window, { key: "k", code: "KeyK", ...PRIMARY });
    await flushPromises();
    expect(menu(w).exists()).toBe(true);
  };
  const choose = async (w: VueWrapper, id: "away" | "endAway" | "mute") => {
    await w.find(`.qr-item[data-item='${id}']`).trigger("click");
    await flushPromises();
  };

  describe("on an ordinary question", () => {
    it("opens from the button and lists the two directives and the link to the settings", async () => {
      const { wrapper } = await mountRequest();
      expect(trigger(wrapper).text()).toContain("Quick reply");
      expect(trigger(wrapper).attributes("aria-expanded")).toBe("false");
      expect(menu(wrapper).exists()).toBe(false);

      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(true);
      expect(trigger(wrapper).attributes("aria-expanded")).toBe("true");
      expect(itemLabels(wrapper)).toEqual(["Away mode…", "Stop using AskHuman in this session…", MANAGE]);
      // The first row is highlighted and holds the focus.
      expect(items(wrapper)[0].classes()).toContain("hover");
      expect(document.activeElement).toBe(items(wrapper)[0].element);
      expect(wrapper.find(".qr-head-hint").text()).toBe(primaryShortcutLabel("k"));
    });

    it("slides the menu back inside the window when its button sits left of centre", async () => {
      // jsdom has no layout: report the menu hanging 73px past the window's left edge, as it does in a
      // several-question footer where Previous / Next / Send crowd the button.
      const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
        const menuBox = this.classList.contains("qr-menu");
        return { left: menuBox ? -73 : 0, right: 0, top: 0, bottom: 0, width: menuBox ? 336 : 0, height: 0, x: 0, y: 0, toJSON: () => ({}) };
      });
      try {
        const { wrapper } = await mountRequest();
        await trigger(wrapper).trigger("click");
        await flushPromises();
        expect(menu(wrapper).attributes("style")).toContain("translateX(85px)");

        // Closed and opened again, it is measured again.
        press(window, { key: "Escape" });
        await flushPromises();
        expect(menu(wrapper).exists()).toBe(false);
        rect.mockImplementation(function (this: HTMLElement) {
          const menuBox = this.classList.contains("qr-menu");
          return { left: menuBox ? 40 : 0, right: 0, top: 0, bottom: 0, width: menuBox ? 336 : 0, height: 0, x: 0, y: 0, toJSON: () => ({}) };
        });
        await trigger(wrapper).trigger("click");
        await flushPromises();
        expect(menu(wrapper).attributes("style") ?? "").not.toContain("translateX");
      } finally {
        rect.mockRestore();
      }
    });

    it("opens with the shortcut and moves the highlight with the arrows", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      expect(items(wrapper)[0].classes()).toContain("hover");
      press(window, { key: "ArrowDown" });
      await flushPromises();
      expect(items(wrapper)[1].classes()).toContain("hover");
      expect(document.activeElement).toBe(items(wrapper)[1].element);
      press(window, { key: "ArrowDown" });
      await flushPromises();
      expect(items(wrapper)[2].classes()).toContain("hover");
      press(window, { key: "ArrowDown" });
      await flushPromises();
      expect(items(wrapper)[0].classes()).toContain("hover");
      press(window, { key: "ArrowUp" });
      await flushPromises();
      expect(items(wrapper)[2].classes()).toContain("hover");
    });

    it("arms the highlighted row with Enter and never submits the answer", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my answer");
      await openWithKey(wrapper);
      press(window, { key: "ArrowDown" });
      press(window, { key: "Enter" });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
      expect(bar(wrapper).exists()).toBe(true);
      expect(bar(wrapper).find(".qr-bar-label").text()).toBe("Stop using AskHuman in this session");
      expect(calls("submit_popup")).toEqual([]);
    });

    it("is keyboard-modal: the request's own Enter and Esc do not reach the answer", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my answer");
      await openWithKey(wrapper);

      // A row keeps the native click of its Enter; anything else the page owns is swallowed.
      press(items(wrapper)[0].element, { key: "Enter", ...PRIMARY });
      press(items(wrapper)[0].element, { key: "1", ...PRIMARY });
      press(items(wrapper)[0].element, { key: "w", altKey: true });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
      expect(menu(wrapper).exists()).toBe(true);
      expect(wrapper.findAll(".option.selected, .option.chosen")).toHaveLength(0);

      press(window, { key: "Escape" });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
      expect(wrapper.find(".confirm-overlay").exists()).toBe(false);
      expect(answerBox(wrapper).element.value).toBe("my answer");

      // Closed again, the very same key submits the answer.
      press(answerBox(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(calls("submit_popup")).toHaveLength(1);
    });

    it("closes with Esc, a click outside, the button or the shortcut and hands the focus back", async () => {
      const { wrapper } = await mountRequest();
      const answer = answerBox(wrapper).element;
      const closers: Array<() => unknown> = [
        () => press(window, { key: "Escape" }),
        () => press(window, { key: "k", code: "KeyK", ...PRIMARY }),
        () => wrapper.find(".qr-backdrop").trigger("click"),
        () => trigger(wrapper).trigger("click"),
      ];
      for (const close of closers) {
        answer.focus();
        await openWithKey(wrapper);
        await close();
        await flushPromises();
        expect(menu(wrapper).exists()).toBe(false);
        expect(wrapper.find(".qr-backdrop").exists()).toBe(false);
        expect(document.activeElement).toBe(answer);
      }
      expect(calls("cancel_popup")).toEqual([]);
    });

    it("closes when the focus moves to another control, not when it merely leaves the page", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      const focusout = (relatedTarget: EventTarget | null) =>
        items(wrapper)[0].element.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget }));
      focusout(null);
      focusout(items(wrapper)[1].element);
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(true);
      focusout(answerBox(wrapper).element);
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
    });
  });

  describe("an armed away-mode directive", () => {
    it("waits above the footer and turns Send into what it will do", async () => {
      const { wrapper } = await mountRequest();
      expect(primary(wrapper).text()).toContain("Send");
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");

      expect(menu(wrapper).exists()).toBe(false);
      expect(bar(wrapper).find(".qr-bar-label").text()).toBe("Away mode");
      const scope = bar(wrapper).find<HTMLSelectElement>(".qr-scope");
      expect(scope.element.value).toBe("global");
      expect(scope.findAll("option").map((o) => o.text())).toEqual(["All Agents", "This session only"]);
      expect(bar(wrapper).find<HTMLInputElement>(".qr-note").attributes("placeholder")).toContain("optional");
      expect(primary(wrapper).text()).toContain("Send and go away");
      expect(primary(wrapper).text()).toContain(primaryShortcutLabel("enter"));
      expect(calls("submit_popup")).toEqual([]);
    });

    it("sends the directive with the answer, scope and note included", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("go with option A");
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      await bar(wrapper).find(".qr-scope").setValue("session");
      await bar(wrapper).find(".qr-note").setValue("  leave production alone  ");
      await primary(wrapper).trigger("click");
      await flushPromises();

      expect(submitted()).toEqual([expect.objectContaining({
        answers: [expect.objectContaining({ userInput: "go with option A" })],
        directive: { kind: "away", scope: "session", note: "leave production alone" },
      })]);
    });

    it("defaults to every Agent and omits an empty note", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      await bar(wrapper).find(".qr-note").setValue("   ");
      press(answerBox(wrapper).element, { key: "Enter", ...PRIMARY });
      await flushPromises();
      expect(submitted()).toEqual([expect.objectContaining({ directive: { kind: "away", scope: "global" } })]);
    });

    it("can be taken back with the cross", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      await bar(wrapper).find(".qr-bar-close").trigger("click");
      expect(bar(wrapper).exists()).toBe(false);
      expect(primary(wrapper).text()).toBe(`Send ${primaryShortcutLabel("enter")}`);
      await primary(wrapper).trigger("click");
      await flushPromises();
      expect(submitted()).toHaveLength(1);
      expect(submitted()[0].directive).toBeUndefined();
    });

    it("keeps a typed note and scope when away mode is picked again", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      await bar(wrapper).find(".qr-scope").setValue("session");
      await bar(wrapper).find(".qr-note").setValue("be careful");
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      expect(bar(wrapper).find<HTMLSelectElement>(".qr-scope").element.value).toBe("session");
      expect(bar(wrapper).find<HTMLInputElement>(".qr-note").element.value).toBe("be careful");
    });

    it("does not go out with a cancel", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      const cancel = wrapper.findAll(".footer .btn").find((b) => b.text().includes("Cancel"))!;
      await cancel.trigger("click");
      await flushPromises();
      // No answer went out; a cancel never carries the directive.
      expect(calls("submit_popup")).toEqual([]);
    });
  });

  describe("an armed mute directive", () => {
    it("says what it will do and goes out for this session only", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "mute");
      expect(bar(wrapper).find(".qr-bar-label").text()).toBe("Stop using AskHuman in this session");
      expect(bar(wrapper).find(".qr-bar-text").text()).toContain("permission approvals and error notices stay");
      expect(bar(wrapper).find(".qr-scope").exists()).toBe(false);
      expect(bar(wrapper).find(".qr-note").exists()).toBe(false);
      expect(primary(wrapper).text()).toContain("Send and turn off");
      await primary(wrapper).trigger("click");
      await flushPromises();
      expect(submitted()).toEqual([expect.objectContaining({ directive: { kind: "mute", scope: "session" } })]);
    });

    it("replaces an armed away mode and drops its note", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      await bar(wrapper).find(".qr-note").setValue("lunch");
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "mute");
      await primary(wrapper).trigger("click");
      await flushPromises();
      expect(submitted()).toEqual([expect.objectContaining({ directive: { kind: "mute", scope: "session" } })]);
    });
  });

  describe("on a question with several parts", () => {
    beforeEach(() => { interaction = (id) => ask(id, {}, 3); });

    it("puts one send button where the navigation was and sends the unanswered parts empty", async () => {
      const { wrapper } = await mountRequest();
      const labels = () => wrapper.findAll(".footer .btn").map((b) => b.text());
      expect(labels().some((label) => label.startsWith("Previous"))).toBe(true);
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "mute");
      expect(labels().some((label) => label.startsWith("Previous"))).toBe(false);
      expect(labels().some((label) => label.startsWith("Next"))).toBe(false);
      expect(primary(wrapper).text()).toContain("Send and turn off");
      await primary(wrapper).trigger("click");
      await flushPromises();
      const [submission] = submitted();
      expect(submission.directive).toEqual({ kind: "mute", scope: "session" });
      expect(submission.answers).toHaveLength(3);
    });

    it("brings the navigation back when the directive is taken back", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "mute");
      await bar(wrapper).find(".qr-bar-close").trigger("click");
      expect(wrapper.findAll(".footer .btn").some((b) => b.text().startsWith("Previous"))).toBe(true);
    });
  });

  describe("on a strict-choice question", () => {
    beforeEach(() => { interaction = (id) => ask(id, { selectOnly: true }); });

    it("lets the directive go out before anything is picked", async () => {
      const { wrapper } = await mountRequest();
      expect(primary(wrapper).attributes("disabled")).toBeDefined();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "away");
      expect(primary(wrapper).attributes("disabled")).toBeUndefined();
      await primary(wrapper).trigger("click");
      await flushPromises();
      expect(submitted()).toEqual([expect.objectContaining({ directive: { kind: "away", scope: "global" } })]);
    });
  });

  describe("on a whats-next question", () => {
    beforeEach(() => { interaction = whatsNext; });

    it("offers the same two directives", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(itemLabels(wrapper)).toEqual(["Away mode…", "Stop using AskHuman in this session…", MANAGE]);
    });
  });

  describe("on cards that are not an Agent's own question", () => {
    it("has no button on a Stop confirmation, failure card or relay, and ⌘K does nothing", async () => {
      interaction = (id) => ask(id, { quickReply: false });
      const { wrapper } = await mountRequest();
      expect(trigger(wrapper).exists()).toBe(false);
      press(window, { key: "k", code: "KeyK", ...PRIMARY });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
      expect(bar(wrapper).exists()).toBe(false);
    });

    it("has no button on a permission card", async () => {
      interaction = permission;
      const { wrapper } = await mountRequest();
      expect(trigger(wrapper).exists()).toBe(false);
      press(window, { key: "k", code: "KeyK", ...PRIMARY });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
    });
  });

  describe("on an escalated question", () => {
    beforeEach(() => { interaction = (id) => ask(id, { escalated: "asked" }); });

    it("explains why it came through and offers to end away mode", async () => {
      const { wrapper } = await mountRequest();
      expect(banner(wrapper).find(".qr-banner-text b").text()).toBe("Away mode is on");
      expect(banner(wrapper).text()).toContain("the Agent is asking for you");
      expect(banner(wrapper).find(".qr-banner-action").text()).toBe("End away mode");
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(itemLabels(wrapper)).toEqual(["End away mode", "Stop using AskHuman in this session…", MANAGE]);
    });

    it("says why each kind of escalation came through", async () => {
      for (const [reason, text] of [
        ["repeated", "the Agent kept asking"],
        ["requested", "you asked this session to ask you"],
      ] as const) {
        interaction = (id) => ask(id, { escalated: reason });
        const { wrapper } = await mountRequest(reason);
        expect(banner(wrapper).text()).toContain(text);
      }
    });

    it("ends away mode from the banner and then looks like an ordinary question", async () => {
      const { wrapper } = await mountRequest();
      await banner(wrapper).find(".qr-banner-action").trigger("click");
      await flushPromises();
      expect(calls("away_end")).toHaveLength(1);
      expect(banner(wrapper).exists()).toBe(false);
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(itemLabels(wrapper)).toEqual(["Away mode…", "Stop using AskHuman in this session…", MANAGE]);
      // Ending away mode answers nothing.
      expect(calls("submit_popup")).toEqual([]);
    });

    it("ends away mode from the menu without arming anything", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "endAway");
      expect(calls("away_end")).toHaveLength(1);
      expect(bar(wrapper).exists()).toBe(false);
      expect(banner(wrapper).exists()).toBe(false);
    });

    it("keeps the banner and says so when away mode cannot be ended", async () => {
      awayEndFails = true;
      const { wrapper } = await mountRequest();
      await banner(wrapper).find(".qr-banner-action").trigger("click");
      await flushPromises();
      expect(banner(wrapper).exists()).toBe(true);
      expect(wrapper.find(".qr-banner-error").text()).toContain("daemon is restarting");
    });

    it("is not shown on a request that is not an escalation", async () => {
      interaction = (id) => ask(id);
      const { wrapper } = await mountRequest();
      expect(banner(wrapper).exists()).toBe(false);
    });
  });

  describe("beside the rest of the popup", () => {
    it("closes when the cancel confirmation appears", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("unsent draft");
      await openWithKey(wrapper);
      await wrapper.findAll(".footer .btn").find((b) => b.text().includes("Cancel"))!.trigger("click");
      await flushPromises();
      expect(wrapper.find(".confirm-overlay").exists()).toBe(true);
      expect(menu(wrapper).exists()).toBe(false);
    });

    it("leaves ⌘/Ctrl+F alone while open, and does not open while the find bar is", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      press(window, { key: "f", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".popup-find-bar").exists()).toBe(false);
      expect(menu(wrapper).exists()).toBe(true);
      press(window, { key: "Escape" });
      await flushPromises();

      press(window, { key: "f", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".popup-find-bar").exists()).toBe(true);
      press(window, { key: "k", code: "KeyK", ...PRIMARY });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
    });

    it("yields ⌘/Ctrl+K to the speech shortcut but still opens from the button", async () => {
      speechAvailable = true;
      overrides = { speechShortcut: SPEECH_ON_K };
      const { wrapper } = await mountRequest();
      press(window, { key: "k", code: "KeyK", ...PRIMARY });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(true);
    });

    it("does not open on top of the todo popover, nor the other way round", async () => {
      const { wrapper } = await mountRequest();
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".todo-popover").exists()).toBe(true);
      press(window, { key: "k", code: "KeyK", ...PRIMARY });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(false);
      expect(wrapper.find(".todo-popover").exists()).toBe(true);

      press(window, { key: "Escape" });
      await flushPromises();
      expect(wrapper.find(".todo-popover").exists()).toBe(false);
      await openWithKey(wrapper);
      press(window, { key: "t", code: "KeyT", ...PRIMARY });
      await flushPromises();
      expect(wrapper.find(".todo-popover").exists()).toBe(false);
      expect(menu(wrapper).exists()).toBe(true);
    });

    it("keeps an image pasted into the menu or the note away from the answer", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await flushPromises();
      const reachedWindow = vi.fn();
      window.addEventListener("paste", reachedWindow);
      try {
        menu(wrapper).element.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
        expect(reachedWindow).not.toHaveBeenCalled();
        await choose(wrapper, "away");
        bar(wrapper).find(".qr-note").element.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
        expect(reachedWindow).not.toHaveBeenCalled();
      } finally {
        window.removeEventListener("paste", reachedWindow);
      }
    });

    it("cannot be opened while the answer is being sent", async () => {
      const { wrapper } = await mountRequest();
      let release: () => void = () => {};
      const original = mocks.invoke.getMockImplementation()!;
      mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
        if (command === "submit_popup") await new Promise<void>((resolve) => { release = resolve; });
        return original(command, args);
      });
      void primary(wrapper).trigger("click");
      await flushPromises();
      expect(trigger(wrapper).attributes("disabled")).toBeDefined();
      release();
      await flushPromises();
    });
  });

  describe("in the merged window", () => {
    it("closes when its request stops being the active one", async () => {
      const merged = await mountRequest("a", true);
      await openWithKey(merged.wrapper);
      merged.active.value = false;
      await flushPromises();
      expect(menu(merged.wrapper).exists()).toBe(false);
    });

    it("closes when the window is blocked by a dialog", async () => {
      const merged = await mountRequest("a", true);
      await trigger(merged.wrapper).trigger("click");
      await flushPromises();
      expect(menu(merged.wrapper).exists()).toBe(true);
      merged.blocked.value = true;
      await flushPromises();
      expect(menu(merged.wrapper).exists()).toBe(false);
    });

    it("still lets ⌘/Ctrl+W close the window while open", async () => {
      const merged = await mountRequest("a", true);
      await openWithKey(merged.wrapper);
      press(window, { key: "w", ...PRIMARY });
      await flushPromises();
      expect(merged.close).toHaveBeenCalledTimes(1);
    });
  });

  describe("with the user's quick replies (docs/specs/special-replies.md D18, D27)", () => {
    const REPLIES = ["Continue", "Use your own judgement", "Skip this one"];
    const presetRows = (w: VueWrapper) => w.findAll(".qr-preset");
    const pick = async (w: VueWrapper, index: number) => {
      await w.find(`.qr-item[data-item='preset-${index}']`).trigger("click");
      await flushPromises();
    };
    beforeEach(() => { interaction = (id) => ask(id, { quickReplies: REPLIES }); });

    it("lists them first with their digits, then the directives and the link to the settings", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(itemLabels(wrapper)).toEqual([...REPLIES, "Away mode…", "Stop using AskHuman in this session…", MANAGE]);
      expect(presetRows(wrapper).map((row) => row.find(".qr-key").text())).toEqual(["1", "2", "3"]);
      expect(wrapper.find(".qr-head-hint").text()).toBe("Click to send");
      expect(wrapper.findAll(".qr-sep")).toHaveLength(2);
      // The first preset is highlighted and holds the focus.
      expect(items(wrapper)[0].classes()).toContain("hover");
      expect(document.activeElement).toBe(items(wrapper)[0].element);
    });

    it("sends a clicked preset at once as the reply of the question", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await pick(wrapper, 1);
      expect(menu(wrapper).exists()).toBe(false);
      expect(submitted()).toEqual([expect.objectContaining({
        answers: [expect.objectContaining({ selectedOptions: [], userInput: "Use your own judgement" })],
      })]);
      expect(submitted()[0].directive).toBeUndefined();
    });

    it("picks a preset with its digit while the menu is open", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      press(window, { key: "3" });
      await flushPromises();
      expect(submitted()).toEqual([expect.objectContaining({
        answers: [expect.objectContaining({ userInput: "Skip this one" })],
      })]);
    });

    it("counts to ten with the digits 1…9 and 0", async () => {
      const ten = Array.from({ length: 10 }, (_, i) => `reply ${i + 1}`);
      interaction = (id) => ask(id, { quickReplies: ten });
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      expect(presetRows(wrapper).map((row) => row.find(".qr-key").text())).toEqual(
        ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"],
      );
      press(window, { key: "0" });
      await flushPromises();
      expect(submitted()).toEqual([expect.objectContaining({
        answers: [expect.objectContaining({ userInput: "reply 10" })],
      })]);
    });

    it("swallows a digit that has no preset, a modified one and a digit while closed does nothing", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      press(window, { key: "7" });
      press(window, { key: "1", shiftKey: true });
      await flushPromises();
      expect(menu(wrapper).exists()).toBe(true);
      expect(calls("submit_popup")).toEqual([]);
      press(window, { key: "Escape" });
      await flushPromises();
      press(window, { key: "1" });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
    });

    it("sends the highlighted preset with Enter", async () => {
      const { wrapper } = await mountRequest();
      await openWithKey(wrapper);
      press(window, { key: "ArrowDown" });
      press(window, { key: "Enter" });
      await flushPromises();
      expect(submitted()).toEqual([expect.objectContaining({
        answers: [expect.objectContaining({ userInput: "Use your own judgement" })],
      })]);
    });

    it("keeps what was answered already and gives every other question the reply", async () => {
      interaction = (id) => ask(id, { quickReplies: REPLIES }, 3);
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my own answer");
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.find(".qr-head-hint").text()).toBe("Click to send · fills 2 unanswered");
      await pick(wrapper, 0);
      expect(submitted()).toEqual([expect.objectContaining({
        answers: [
          expect.objectContaining({ userInput: "my own answer" }),
          expect.objectContaining({ userInput: "Continue" }),
          expect.objectContaining({ userInput: "Continue" }),
        ],
      })]);
    });

    it("says so when a single question is left to answer", async () => {
      interaction = (id) => ask(id, { quickReplies: REPLIES }, 2);
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("first done");
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.find(".qr-head-hint").text()).toBe("Click to send · fills 1 unanswered");
    });

    it("greys the presets out once every question has an answer and starts on the directives", async () => {
      const { wrapper } = await mountRequest();
      await answerBox(wrapper).setValue("my answer");
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(presetRows(wrapper).every((row) => row.classes().includes("disabled"))).toBe(true);
      expect(presetRows(wrapper)[0].attributes("aria-disabled")).toBe("true");
      expect(wrapper.find(".qr-head-hint").text()).toBe("Every question has an answer");
      expect(items(wrapper)[3].classes()).toContain("hover");
      expect(document.activeElement).toBe(items(wrapper)[3].element);

      // Neither a click, a digit nor Enter sends anything.
      await pick(wrapper, 0);
      press(window, { key: "1" });
      await flushPromises();
      expect(calls("submit_popup")).toEqual([]);
      expect(menu(wrapper).exists()).toBe(true);

      // The arrows skip the greyed-out rows in both directions.
      press(window, { key: "ArrowUp" });
      await flushPromises();
      expect(items(wrapper)[5].classes()).toContain("hover");
      press(window, { key: "ArrowDown" });
      await flushPromises();
      expect(items(wrapper)[3].classes()).toContain("hover");
    });

    it("counts a picked option as an answer", async () => {
      const { wrapper } = await mountRequest();
      await wrapper.find(".option").trigger("click");
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.find(".qr-head-hint").text()).toBe("Every question has an answer");
    });

    it("sends the armed directive along with the preset", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await choose(wrapper, "mute");
      expect(bar(wrapper).exists()).toBe(true);
      await trigger(wrapper).trigger("click");
      await pick(wrapper, 0);
      expect(submitted()).toEqual([expect.objectContaining({
        answers: [expect.objectContaining({ userInput: "Continue" })],
        directive: { kind: "mute", scope: "session" },
      })]);
    });

    it("opens the settings at the quick-reply card from the last row", async () => {
      const { wrapper } = await mountRequest();
      await trigger(wrapper).trigger("click");
      await wrapper.find(".qr-item[data-item='manage']").trigger("click");
      await flushPromises();
      expect(calls("open_settings")).toEqual([{ tab: "general#quick-replies" }]);
      expect(menu(wrapper).exists()).toBe(false);
      expect(calls("submit_popup")).toEqual([]);
    });

    it("shows no presets on What's next or a strict choice, even if some arrive", async () => {
      interaction = (id) => ask(id, { quickReplies: REPLIES, selectOnly: true });
      const strict = await mountRequest("strict");
      await trigger(strict.wrapper).trigger("click");
      await flushPromises();
      expect(presetRows(strict.wrapper)).toHaveLength(0);
      expect(itemLabels(strict.wrapper)).toEqual(["Away mode…", "Stop using AskHuman in this session…", MANAGE]);

      interaction = (id) => ({
        ...whatsNext(id),
        interaction: { type: "ask", request: { ...(whatsNext(id).interaction as { request: AskRequest }).request, quickReplies: REPLIES } },
      }) as PopupInit;
      const next = await mountRequest("next");
      await trigger(next.wrapper).trigger("click");
      await flushPromises();
      expect(presetRows(next.wrapper)).toHaveLength(0);
    });

    it("reads in Chinese too", async () => {
      interaction = (id) => ask(id, { quickReplies: ["继续", "按你的判断来"] });
      overrides = { language: "zh" };
      const { wrapper } = await mountRequest("a", false, "zh");
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(itemLabels(wrapper)).toEqual(["继续", "按你的判断来", "离开模式…", "本会话不再使用 AskHuman…", "管理快捷回复…"]);
      expect(wrapper.find(".qr-head-hint").text()).toBe("点击即发送");
    });
  });

  describe("in Chinese", () => {
    it("reads the way the prototype does", async () => {
      interaction = (id) => ask(id, { escalated: "asked" });
      overrides = { language: "zh" };
      const { wrapper } = await mountRequest("a", false, "zh");
      expect(trigger(wrapper).text()).toContain("快捷回复");
      expect(banner(wrapper).text()).toContain("离开模式开启中");
      expect(banner(wrapper).find(".qr-banner-action").text()).toBe("结束离开模式");
      await trigger(wrapper).trigger("click");
      await flushPromises();
      expect(itemLabels(wrapper)).toEqual(["结束离开模式", "本会话不再使用 AskHuman…", "管理快捷回复…"]);
    });
  });
});
