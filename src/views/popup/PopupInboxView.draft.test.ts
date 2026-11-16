// What the shared window does when a request is closed because its caller went away
// (docs/specs/popup-draft-on-cancel.md D5): it reads the form before the form goes and hands it to the host.
import { defineComponent, h, onBeforeUnmount, onMounted } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { DraftSave, PopupInboxRequest } from "../../lib/types";
import type { PopupScope } from "./usePopupCore";
const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  scopes: new Map<string, PopupScope>(),
  readers: new Map<string, () => DraftSave | null>(),
  readAt: [] as { id: string; formStillMounted: boolean }[],
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  mock.handlers.set(name, handler); return () => mock.handlers.delete(name);
} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFocus: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
// A form that lends the page a reader the way the real one does, and says whether it still existed when it was read.
vi.mock("../PopupView.vue", () => ({ default: defineComponent({ props: ["scope"], setup(props) {
  const id: string = props.scope.requestId;
  mock.scopes.set(id, props.scope);
  onMounted(() => {
    props.scope.ready(id);
    props.scope.draftReader?.(() => {
      mock.readAt.push({ id, formStillMounted: mock.scopes.has(id) });
      return mock.readers.get(id)?.() ?? null;
    });
  });
  onBeforeUnmount(() => { props.scope.draftReader?.(null); mock.scopes.delete(id); });
  return () => h("div", [h("div", { class: "content" }, h("textarea", { "data-form-id": id })), h("div", { class: "footer" }, "Send")]);
} }) }));
import PopupInboxView from "../PopupInboxView.vue";
function request(id: string): PopupInboxRequest {
  return { requestId: id, sequence: id.charCodeAt(0), project: "/project", source: "Codex", lang: "en", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Request ${id}`, predefinedOptions: [] }] } } };
}
function emit(name: string, payload: unknown) { mock.handlers.get(name)!({ payload }); }
const typed: DraftSave = { answers: [{ selectedOptions: ["Yes"], userInput: "half an answer", images: [], files: [] }], annotations: [{ quote: "q", note: "n" }] };

describe("a request closed because its caller went away", () => {
  let wrapper: VueWrapper;
  beforeEach(() => {
    mock.handlers.clear(); mock.invoke.mockReset(); mock.scopes.clear(); mock.readers.clear(); mock.readAt.length = 0;
    vi.stubGlobal("CSS", { escape: (s: string) => s });
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_inbox_init") return { recovered: false, requests: [request("a"), request("b"), request("c")] };
      if (command === "popup_inbox_layout") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0 };
    });
  });
  afterEach(() => { wrapper?.unmount(); vi.useRealTimers(); vi.unstubAllGlobals(); });
  async function start() {
    wrapper = mount(PopupInboxView, { attachTo: document.body, global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    await flushPromises();
  }
  const draftCalls = () => mock.invoke.mock.calls.filter(call => call[0] === "popup_inbox_draft");
  // `a` is on screen, `b` is waiting with its form already built (sending `a` prepares its successor), `c` was never opened.
  async function startWithSuccessorPrepared() {
    await start();
    mock.scopes.get("a")!.sending!();
    await flushPromises();
    expect(mock.scopes.has("b")).toBe(true);
    expect(mock.scopes.has("c")).toBe(false);
  }
  const other: DraftSave = { answers: [{ selectedOptions: [], userInput: "b's text", images: [], files: [] }] };

  it("reads the form while it still exists and hands what it held to the host", async () => {
    await startWithSuccessorPrepared();
    mock.readers.set("b", () => typed);
    emit("popup-inbox-terminal", { requestId: "b", winner: "caller", keepDraft: true });
    expect(draftCalls()).toEqual([["popup_inbox_draft", { requestId: "b", draft: typed }]]);
    expect(mock.readAt).toEqual([{ id: "b", formStillMounted: true }]);
    await flushPromises();
    expect(wrapper.find('[data-inbox-row="b"]').exists()).toBe(false);
    expect(wrapper.find(".inbox-error").exists()).toBe(false);
  });

  it("reads the form on screen too", async () => {
    await start();
    mock.readers.set("a", () => typed);
    emit("popup-inbox-terminal", { requestId: "a", winner: "system", keepDraft: true });
    expect(draftCalls()).toEqual([["popup_inbox_draft", { requestId: "a", draft: typed }]]);
    expect(mock.readAt).toEqual([{ id: "a", formStillMounted: true }]);
  });

  it("closes the request at once and does not wait for the host to keep the draft", async () => {
    await startWithSuccessorPrepared();
    mock.readers.set("b", () => typed);
    // The host never answers: the request still closes, and nothing is shown.
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_inbox_draft") return new Promise(() => {});
      if (command === "popup_inbox_layout") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0 };
    });
    emit("popup-inbox-terminal", { requestId: "b", winner: "caller", keepDraft: true });
    await flushPromises();
    expect(wrapper.find('[data-inbox-row="b"]').exists()).toBe(false);
    expect(wrapper.find(".inbox-error").exists()).toBe(false);
  });

  it("asks nothing of the form for any other ending", async () => {
    await start();
    mock.readers.set("b", () => typed);
    mock.readers.set("c", () => typed);
    emit("popup-inbox-terminal", { requestId: "b", winner: "telegram" });
    emit("popup-inbox-terminal", { requestId: "c", winner: "system" });
    await flushPromises();
    expect(draftCalls()).toEqual([]);
    expect(mock.readAt).toEqual([]);
  });

  it("tells the host there is nothing when the form holds nothing", async () => {
    await startWithSuccessorPrepared();
    mock.readers.set("b", () => null);
    emit("popup-inbox-terminal", { requestId: "b", winner: "system", keepDraft: true });
    expect(draftCalls()).toEqual([["popup_inbox_draft", { requestId: "b", draft: null }]]);
  });

  it("tells the host there is nothing for a request that was never opened", async () => {
    await start();
    expect(mock.scopes.has("c")).toBe(false);
    emit("popup-inbox-terminal", { requestId: "c", winner: "caller", keepDraft: true });
    expect(draftCalls()).toEqual([["popup_inbox_draft", { requestId: "c", draft: null }]]);
  });

  it("does not let a form that cannot be read, or a host that refuses, reach the user", async () => {
    await startWithSuccessorPrepared();
    mock.readers.set("b", () => { throw new Error("form is broken"); });
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "popup_inbox_draft") throw new Error("host refused");
      if (command === "popup_inbox_layout") return { revision: 1, sidebarWidth: 240, mainWidth: 560, mainHeight: 620, previewWidth: 0 };
    });
    emit("popup-inbox-terminal", { requestId: "b", winner: "caller", keepDraft: true });
    await flushPromises();
    expect(draftCalls()).toEqual([["popup_inbox_draft", { requestId: "b", draft: null }]]);
    expect(wrapper.find(".inbox-error").exists()).toBe(false);
    expect(wrapper.find('[data-inbox-row="b"]').exists()).toBe(false);
  });

  it("keeps a request's reader apart from every other request's", async () => {
    await startWithSuccessorPrepared();
    mock.readers.set("a", () => typed);
    mock.readers.set("b", () => other);
    emit("popup-inbox-terminal", { requestId: "b", winner: "caller", keepDraft: true });
    expect(draftCalls()).toEqual([["popup_inbox_draft", { requestId: "b", draft: other }]]);
    // The one that closed took its reader with it; the other still has its own.
    emit("popup-inbox-terminal", { requestId: "a", winner: "caller", keepDraft: true });
    expect(draftCalls()[1]).toEqual(["popup_inbox_draft", { requestId: "a", draft: typed }]);
  });

  it("reads each request once even when its end is announced twice", async () => {
    await startWithSuccessorPrepared();
    mock.readers.set("b", () => typed);
    emit("popup-inbox-terminal", { requestId: "b", winner: "caller", keepDraft: true });
    await flushPromises();
    // The form is gone by now, so a repeat finds nothing to read (the host reports an end only once anyway).
    emit("popup-inbox-terminal", { requestId: "b", winner: "caller", keepDraft: true });
    expect(draftCalls()[1]).toEqual(["popup_inbox_draft", { requestId: "b", draft: null }]);
    expect(mock.readAt).toHaveLength(1);
  });
});
