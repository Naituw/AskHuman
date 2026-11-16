// What a form hands the page when its caller goes away (docs/specs/popup-draft-on-cancel.md D2/D5).
import { computed, defineComponent, h, ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { DraftSave, ImageAttachment, PopupInit } from "../../lib/types";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import { usePopupCore, type PopupScope } from "./usePopupCore";

function ask(id: string, questions: number): PopupInit {
  return { theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project", language: "en", warm: true,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "Some words to note", files: [] }, annotatable: true,
      questions: Array.from({ length: questions }, (_, i) => ({ message: `Question ${i + 1}`,
        predefinedOptions: [{ text: "Yes", recommended: false }, { text: "No", recommended: false }] })) } } };
}
function confirm(id: string): PopupInit {
  return { ...ask(id, 1), interaction: { type: "confirm", request: {
    id, title: "Run command?", context: [], detail: { summary: "Needed", bodyMd: "`cargo test`" },
    choices: [{ id: "allow", label: "Allow once", description: "", role: "primary" }, { id: "deny", label: "Deny", description: "", role: "destructive" }],
    presentation: { type: "singleSelectSubmit", submitLabel: "Submit" }, dismissActionId: "deny", createdAtMs: 1, expiresAtMs: 2,
  } } } as PopupInit;
}
const png = (name: string): ImageAttachment => ({ data: "iVBORw0KGgo=", mediaType: "image/png", filename: name });

describe("the draft a form can hand over", () => {
  const wrappers: VueWrapper[] = [];
  let interaction: () => PopupInit;
  beforeEach(() => {
    vi.clearAllMocks();
    interaction = () => ask("a", 2);
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_init") return interaction();
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
  });
  afterEach(() => { wrappers.forEach(w => w.unmount()); wrappers.length = 0; vi.unstubAllGlobals(); });

  async function form() {
    let ctx!: ReturnType<typeof usePopupCore>;
    const draftReader = vi.fn<(reader: (() => DraftSave | null) | null) => void>();
    const scope: PopupScope = { requestId: "a", active: ref(true), blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: vi.fn(),
      preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }), draftReader };
    const wrapper = mount(defineComponent({ setup() { ctx = usePopupCore(scope); return () => h("div"); } }), {
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper); await flushPromises();
    const reader = draftReader.mock.calls.find(call => call[0])?.[0] ?? null;
    return { ctx, wrapper, draftReader, reader: reader as () => DraftSave | null };
  }

  it("lends the page a reader and takes it back when the form goes", async () => {
    const { wrapper, draftReader } = await form();
    expect(draftReader).toHaveBeenCalledTimes(1);
    expect(draftReader).toHaveBeenLastCalledWith(expect.any(Function));
    wrapper.unmount(); wrappers.length = 0;
    expect(draftReader).toHaveBeenLastCalledWith(null);
  });

  it("reads each question's options, text, pictures and files, and the notes, in question order", async () => {
    const { ctx, reader } = await form();
    ctx.chosenByQ.value[0] = ["No"];
    ctx.inputByQ.value[0] = "because it is slow";
    ctx.imagesByQ.value[1] = [png("a.png"), png("b.png")];
    ctx.replyFilesByQ.value[1] = [{ path: "/tmp/report.txt", name: "report.txt" }];
    ctx.annotationNotes.value.push({ id: 1, start: 0, end: 4, quote: "Some", note: "which one?", anchored: true });
    expect(reader()).toEqual({
      answers: [
        { selectedOptions: ["No"], userInput: "because it is slow", images: [], files: [] },
        { selectedOptions: [], userInput: "", images: [png("a.png"), png("b.png")], files: ["/tmp/report.txt"] },
      ],
      annotations: [{ quote: "Some", note: "which one?" }],
    });
  });

  it("keeps the chosen options in the order the question lists them", async () => {
    const { ctx, reader } = await form();
    ctx.chosenByQ.value[0] = ["No", "Yes"];
    expect(reader()!.answers[0].selectedOptions).toEqual(["Yes", "No"]);
  });

  it("has nothing to hand over until something is typed", async () => {
    const { ctx, reader } = await form();
    expect(reader()).toBeNull();
    ctx.inputByQ.value[0] = "  \n ";
    expect(reader()).toBeNull();
    ctx.inputByQ.value[1] = "now there is";
    expect(reader()!.answers[1].userInput).toBe("now there is");
  });

  it.each([
    ["an option", (ctx: ReturnType<typeof usePopupCore>) => { ctx.chosenByQ.value[1] = ["Yes"]; }],
    ["a picture", (ctx: ReturnType<typeof usePopupCore>) => { ctx.imagesByQ.value[0] = [png("a.png")]; }],
    ["a reply file", (ctx: ReturnType<typeof usePopupCore>) => { ctx.replyFilesByQ.value[0] = [{ path: "/tmp/x", name: "x" }]; }],
    ["a note", (ctx: ReturnType<typeof usePopupCore>) => { ctx.annotationNotes.value.push({ id: 1, start: 0, end: 4, quote: "Some", note: "n", anchored: true }); }],
  ])("counts %s as something typed", async (_name, type) => {
    const { ctx, reader } = await form();
    type(ctx);
    expect(reader()).not.toBeNull();
  });

  it("carries no todos and no instruction, only the answers and the notes", async () => {
    const { ctx, reader } = await form();
    ctx.inputByQ.value[0] = "text";
    ctx.armedDirective.value = { kind: "away", scope: "global", note: "" };
    const draft = reader()!;
    expect(Object.keys(draft)).toEqual(["answers"]);
    for (const answer of draft.answers) expect(Object.keys(answer).sort()).toEqual(["files", "images", "selectedOptions", "userInput"]);
  });

  it("keeps a note that was still being typed, as a send would", async () => {
    const { ctx, reader } = await form();
    ctx.annotationEditor.value = { id: null, anchor: { start: 0, end: 4, quote: "Some" }, quote: "Some", draft: "half-typed note" };
    expect(reader()!.annotations).toEqual([{ quote: "Some", note: "half-typed note" }]);
    expect(ctx.annotationEditor.value).toBeNull();
    // An editor with nothing in it adds nothing, and a card with no other content has no draft.
    ctx.annotationNotes.value = [];
    ctx.annotationEditor.value = { id: null, anchor: { start: 0, end: 4, quote: "Some" }, quote: "Some", draft: "   " };
    expect(reader()).toBeNull();
  });

  it("can be read while the answer is being sent, and never sends it", async () => {
    const { ctx, reader } = await form();
    ctx.inputByQ.value[0] = "on its way";
    ctx.submitting.value = true;
    expect(reader()!.answers[0].userInput).toBe("on its way");
    expect(mocks.invoke).not.toHaveBeenCalledWith("submit_popup", expect.anything());
  });

  it("leaves what is in the form where it is", async () => {
    const { ctx, reader } = await form();
    ctx.chosenByQ.value[0] = ["Yes"];
    ctx.inputByQ.value[0] = "kept in the form";
    reader();
    expect(ctx.chosenByQ.value[0]).toEqual(["Yes"]);
    expect(ctx.inputByQ.value[0]).toBe("kept in the form");
  });

  it("has nothing to keep on a confirmation card", async () => {
    interaction = () => confirm("a");
    const { ctx, reader } = await form();
    expect(ctx.isConfirm.value).toBe(true);
    ctx.confirmComment.value = "a remark";
    expect(reader()).toBeNull();
  });
});
