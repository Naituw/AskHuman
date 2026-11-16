// A card of the shared window that comes up with what an abandoned ask of the same session and question was holding, in the
// real popup (docs/specs/popup-draft-on-cancel.md D6/D7): when it asks the host, what ends up in the form, what is never
// overwritten, and the line that says so.
import { computed, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import zh from "../../i18n/zh";
import type { DraftLoad, DraftLoadAnswer, ImageAttachment, PopupInit } from "../../lib/types";
import type { PopupScope, usePopupCore } from "./usePopupCore";

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

const base = {
  theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project",
  language: "en", warm: true,
};
const MESSAGE = "The migration drops three **indexes** and rewrites the cache layer.";

function ask(id: string): PopupInit {
  return { ...base,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      annotatable: true, message: { text: MESSAGE, files: [] },
      questions: [
        { message: "Which one should I take?", predefinedOptions: [{ text: "Yes", recommended: false }, { text: "No", recommended: false }] },
        { message: "Anything else?", predefinedOptions: [] },
      ] } },
  } as PopupInit;
}

function permission(id: string): PopupInit {
  return { ...base, interaction: { type: "confirm", request: {
    id, title: "Run command?", context: [], detail: { summary: "Needed", bodyMd: "`cargo test`" },
    choices: [{ id: "allow", label: "Allow once", description: "", role: "primary" }, { id: "deny", label: "Deny", description: "", role: "destructive" }],
    presentation: { type: "singleSelectSubmit", submitLabel: "Submit" }, dismissActionId: "deny", createdAtMs: 1, expiresAtMs: 2,
  } } } as PopupInit;
}

const png = (name: string): ImageAttachment => ({ data: "data:image/png;base64,iVBORw0KGgo=", mediaType: "image/png", filename: name });
const answer = (patch: Partial<DraftLoadAnswer> = {}): DraftLoadAnswer => ({ selectedOptions: [], userInput: "", images: [], files: [], ...patch });
const draft = (answers: DraftLoadAnswer[], patch: Partial<DraftLoad> = {}): DraftLoad => ({ answers, annotations: [], missingImages: 0, ...patch });

describe("a card that comes up with an abandoned ask's draft", () => {
  const wrappers: VueWrapper[] = [];
  let interaction: () => PopupInit;
  let claim: (requestId: string) => Promise<DraftLoad | null>;

  beforeEach(() => {
    vi.clearAllMocks();
    interaction = () => ask("a");
    claim = async () => null;
    // Date only: the page keeps some keys inert for a moment after a form appears, and the tests move past it.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
    });
    // jsdom has no layout: give a highlighted passage a box.
    Range.prototype.getBoundingClientRect = () => ({
      x: 100, y: 120, left: 100, top: 120, right: 220, bottom: 140, width: 120, height: 20, toJSON: () => ({}),
    }) as DOMRect;
    mocks.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
      switch (command) {
        case "popup_init": return interaction();
        case "popup_draft_claim": return claim(String(args.requestId));
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
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  // `scoped: false` is the standalone popup: one request, no window around it.
  async function mountRequest(options: { scoped?: boolean; locale?: "en" | "zh" } = {}) {
    const { scoped = true, locale = "en" } = options;
    const markDraft = vi.fn();
    const scope = scoped
      ? { requestId: "a", active: ref(true), blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: markDraft,
        preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }) } as unknown as PopupScope
      : undefined;
    const wrapper = mount(PopupView, {
      props: scope ? { scope } : {},
      attachTo: document.body,
      global: { plugins: [createI18n({ legacy: false, locale, messages: { en, zh } })] },
    });
    wrappers.push(wrapper);
    await flushPromises();
    const ctx = (wrapper.vm as unknown as { ctx: ReturnType<typeof usePopupCore> }).ctx;
    return { wrapper, ctx, markDraft };
  }

  const claims = () => mocks.invoke.mock.calls.filter(([name]) => name === "popup_draft_claim").map(([, args]) => args);
  const notice = (w: VueWrapper) => w.find(".draft-banner");
  const answerBoxes = (w: VueWrapper) => w.findAll<HTMLTextAreaElement>("textarea").filter((box) => !box.classes().includes("an-textarea"));

  it("asks the host for its draft once, by its own request id", async () => {
    await mountRequest();
    expect(claims()).toEqual([{ requestId: "a" }]);
  });

  it("fills the form from the draft and says so, without sending anything", async () => {
    claim = async () => draft([
      answer({ selectedOptions: ["No"], userInput: "because it is slow", images: [png("shot.png")], files: ["/tmp/dir/report.txt"] }),
      answer({ userInput: "and one more thing" }),
    ]);
    const { wrapper, ctx } = await mountRequest();
    expect(notice(wrapper).text()).toBe("Loaded what you had not submitted last time");
    expect(notice(wrapper).attributes("role")).toBe("status");
    expect(ctx.chosenByQ.value).toEqual([["No"], []]);
    expect(ctx.inputByQ.value).toEqual(["because it is slow", "and one more thing"]);
    expect(ctx.imagesByQ.value[0]).toEqual([png("shot.png")]);
    expect(ctx.replyFilesByQ.value[0]).toEqual([{ path: "/tmp/dir/report.txt", name: "report.txt" }]);
    expect(wrapper.findAll(".option")[1].classes()).toContain("selected");
    expect(answerBoxes(wrapper)[0].element.value).toBe("because it is slow");
    // Nothing is sent and nothing is focused for the user: the draft is only a head start.
    expect(mocks.invoke).not.toHaveBeenCalledWith("submit_popup", expect.anything());
    expect(mocks.invoke).not.toHaveBeenCalledWith("cancel_popup", expect.anything());
  });

  it("counts as a draft for the sidebar once it is loaded, like typed content", async () => {
    claim = async () => draft([answer({ userInput: "from before" })]);
    const { markDraft } = await mountRequest();
    expect(markDraft).toHaveBeenLastCalledWith("a", true);
  });

  it("shows nothing when there is no draft", async () => {
    const { wrapper, ctx, markDraft } = await mountRequest();
    expect(notice(wrapper).exists()).toBe(false);
    expect(ctx.inputByQ.value).toEqual(["", ""]);
    expect(markDraft).toHaveBeenLastCalledWith("a", false);
  });

  it("stays as it always was when the host cannot answer", async () => {
    claim = async () => { throw new Error("command popup_draft_claim not found"); };
    const { wrapper, ctx } = await mountRequest();
    expect(notice(wrapper).exists()).toBe(false);
    expect(ctx.inputByQ.value).toEqual(["", ""]);
  });

  it("says how many pictures could not come back", async () => {
    claim = async () => draft([answer({ userInput: "text" })], { missingImages: 1 });
    const one = await mountRequest();
    expect(notice(one.wrapper).text()).toBe("Loaded what you had not submitted last time; 1 picture could not be loaded because its file is gone");
    one.wrapper.unmount();
    claim = async () => draft([answer({ userInput: "text" })], { missingImages: 3 });
    const three = await mountRequest();
    expect(notice(three.wrapper).text()).toBe("Loaded what you had not submitted last time; 3 pictures could not be loaded because their files are gone");
  });

  it("speaks the language of the interface", async () => {
    claim = async () => draft([answer({ userInput: "text" })], { missingImages: 2 });
    const { wrapper } = await mountRequest({ locale: "zh" });
    expect(notice(wrapper).text()).toBe("已载入上次未提交的内容；有 2 张图片的文件已被清理，没能带出");
  });

  it("lets the line be dismissed and keeps what was loaded", async () => {
    claim = async () => draft([answer({ userInput: "text" })]);
    const { wrapper, ctx } = await mountRequest();
    await notice(wrapper).find("button").trigger("click");
    expect(notice(wrapper).exists()).toBe(false);
    expect(ctx.inputByQ.value[0]).toBe("text");
  });

  // Spec D7: the draft can arrive after the user has started typing; what they typed is never replaced.
  it("fills only what is empty when the user has typed in the meantime", async () => {
    let arrive!: (value: DraftLoad) => void;
    claim = () => new Promise<DraftLoad | null>((resolve) => { arrive = resolve; });
    const { wrapper, ctx } = await mountRequest();
    ctx.inputByQ.value[0] = "typed while it was on its way";
    ctx.chosenByQ.value[0] = ["Yes"];
    arrive(draft([
      answer({ selectedOptions: ["No"], userInput: "from before", images: [png("a.png")] }),
      answer({ userInput: "second from before" }),
    ]));
    await flushPromises();
    expect(ctx.inputByQ.value).toEqual(["typed while it was on its way", "second from before"]);
    expect(ctx.chosenByQ.value[0]).toEqual(["Yes"]);
    expect(ctx.imagesByQ.value[0]).toEqual([png("a.png")]);
    expect(notice(wrapper).exists()).toBe(true);
  });

  it("places the notes by their quotes and keeps one whose passage is not on the page at the end of the list", async () => {
    claim = async () => draft([answer({ userInput: "text" })], { annotations: [
      { quote: "not anywhere in the text", note: "was this in the old version?" },
      { quote: "rewrites the cache layer", note: "why this one?" },
    ] });
    const { wrapper, ctx } = await mountRequest();
    const notes = ctx.annotationNotes.value;
    expect(notes.map((n) => [n.quote, n.note, n.anchored])).toEqual([
      ["rewrites the cache layer", "why this one?", true],
      ["not anywhere in the text", "was this in the old version?", false],
    ]);
    const marked = Array.from(document.querySelectorAll("mark.an-mark")).map((m) => m.textContent).join("");
    expect(marked).toBe("rewrites the cache layer");
    expect(wrapper.find(".an-chip").exists()).toBe(true);
    // They are what a send would carry, like notes the user wrote.
    expect(ctx.annotationPayload()).toEqual([
      { quote: "rewrites the cache layer", note: "why this one?" },
      { quote: "not anywhere in the text", note: "was this in the old version?" },
    ]);
  });

  it("does not take notes the card cannot show", async () => {
    interaction = () => {
      const init = ask("a");
      const request = (init.interaction as { request: { annotatable: boolean } }).request;
      request.annotatable = false;
      return init;
    };
    claim = async () => draft([answer({ userInput: "text" })], { annotations: [{ quote: "rewrites the cache layer", note: "why?" }] });
    const { ctx } = await mountRequest();
    expect(ctx.inputByQ.value[0]).toBe("text");
    expect(ctx.annotationNotes.value).toEqual([]);
  });

  it("asks nothing for a standalone popup", async () => {
    const { wrapper } = await mountRequest({ scoped: false });
    expect(claims()).toEqual([]);
    expect(notice(wrapper).exists()).toBe(false);
  });

  it("asks nothing for a permission card", async () => {
    interaction = () => permission("a");
    const { wrapper } = await mountRequest();
    expect(claims()).toEqual([]);
    expect(notice(wrapper).exists()).toBe(false);
  });
});
