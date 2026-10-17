import { computed, defineComponent, h, ref, type Ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { FileAttachment, PopupInit } from "../../lib/types";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), drag: vi.fn(async (_handler: unknown) => () => {}), handlers: new Map<string, Set<(event: { payload: unknown }) => void>>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
  if (!mocks.handlers.has(name)) mocks.handlers.set(name, new Set());
  mocks.handlers.get(name)!.add(handler); return () => mocks.handlers.get(name)?.delete(handler);
}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: mocks.drag }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import { usePopupCore, type DropAim, type DropReceiver, type PopupScope } from "./usePopupCore";
function init(id: string): PopupInit {
  return { theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project", language: "en", warm: true,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: "Choose", predefinedOptions: [{ text: "Yes", recommended: true }] }] } } };
}
describe("mounted inbox forms", () => {
  const wrappers: VueWrapper[] = [];
  beforeEach(() => {
    vi.clearAllMocks(); mocks.handlers.clear();
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    mocks.invoke.mockImplementation(async (command: string, args: { requestId?: string }) => {
      if (command === "popup_init") return init(args.requestId!);
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
  });
  afterEach(() => { wrappers.forEach(w => w.unmount()); wrappers.length = 0; vi.unstubAllGlobals(); });
  async function form(id: string, active: Ref<boolean>, overrides: Partial<PopupScope> = {}) {
    let ctx!: ReturnType<typeof usePopupCore>;
    const scope: PopupScope = { requestId: id, active, blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: vi.fn(),
      preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }), ...overrides };
    const wrapper = mount(defineComponent({ setup() { ctx = usePopupCore(scope); return () => h("div"); } }), {
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper); await flushPromises(); return { ctx, scope };
  }
  it("routes the window shortcut to the active request and retains the hidden draft", async () => {
    const activeA = ref(true), activeB = ref(false);
    const a = await form("a", activeA), b = await form("b", activeB);
    a.ctx.inputByQ.value[0] = "draft A";
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1", ctrlKey: true }));
    expect(a.ctx.chosenByQ.value[0]).toEqual(["Yes"]); expect(b.ctx.chosenByQ.value[0]).toEqual([]);
    activeA.value = false; activeB.value = true; await flushPromises();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1", ctrlKey: true }));
    expect(b.ctx.chosenByQ.value[0]).toEqual(["Yes"]); expect(a.ctx.inputByQ.value[0]).toBe("draft A");
    await b.ctx.submit();
    expect(mocks.invoke).toHaveBeenCalledWith("submit_popup", expect.objectContaining({ requestId: "b" }));
  });
  it("keeps the draft and shows an acknowledgement failure without closing the request", async () => {
    const sending = vi.fn();
    const a = await form("a", ref(true), { sending }); a.ctx.inputByQ.value[0] = "unsent";
    mocks.invoke.mockImplementationOnce(async () => { throw new Error("connection unavailable"); });
    await a.ctx.submit();
    expect(a.ctx.inputByQ.value[0]).toBe("unsent"); expect(a.ctx.submitting.value).toBe(false);
    expect(a.ctx.submissionError.value).toContain("connection unavailable");
    expect(sending).toHaveBeenCalledOnce();
  });
  it("blocks answer shortcuts while allowing the successor's native preview to prepare", async () => {
    const blocked = ref(true);
    const a = await form("a", ref(true), { blocked, nativePreviewBlocked: ref(false) });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1", ctrlKey: true }));
    expect(a.ctx.chosenByQ.value[0]).toEqual([]);
    expect(a.ctx.nativePreviewBlocked.value).toBe(false);
    blocked.value = false;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1", ctrlKey: true }));
    expect(a.ctx.chosenByQ.value[0]).toEqual(["Yes"]);
  });
  it("replaces an earlier timeout error with an authoritative late success", async () => {
    const completion = ref<"sent" | null>(null);
    const a = await form("a", ref(true), { completion }); a.ctx.inputByQ.value[0] = "answer";
    mocks.invoke.mockImplementationOnce(async () => { throw "acknowledgement timed out"; });
    await a.ctx.submit(); expect(a.ctx.submissionError.value).toContain("timed out");
    completion.value = "sent"; await flushPromises();
    expect(a.ctx.submissionError.value).toBeNull(); expect(a.ctx.submitting.value).toBe(true);
  });
  it("keeps title enrichment on its request while preserving drafts", async () => {
    const a = await form("a", ref(true)), b = await form("b", ref(false));
    a.ctx.inputByQ.value[0] = "retained draft";
    for (const handler of mocks.handlers.get("popup-session-title")!) handler({ payload: { requestId: "a", title: "Native name" } });
    await flushPromises();
    expect(a.ctx.agentSessionTitle.value).toBe("Native name");
    expect(b.ctx.agentSessionTitle.value).toBe("");
    expect(a.ctx.inputByQ.value[0]).toBe("retained draft");
  });
  it("retains a title delivered while the initialization snapshot is pending", async () => {
    let release!: (value: PopupInit) => void;
    const pending = new Promise<PopupInit>(resolve => { release = resolve; });
    mocks.invoke.mockImplementation(async (command: string) => command === "popup_init" ? pending : {});
    const started = form("a", ref(true));
    await flushPromises();
    for (const handler of mocks.handlers.get("popup-session-title")!) handler({ payload: { requestId: "a", title: "New name" } });
    release({ ...init("a"), agentSessionId: "native-a", agentSessionTitle: "Old snapshot name" });
    const { ctx } = await started;
    await flushPromises();
    expect(ctx.agentSessionId.value).toBe("native-a");
    expect(ctx.agentSessionTitle.value).toBe("New name");
  });
});

describe("an inbox request whose window routes the drops", () => {
  const wrappers: VueWrapper[] = [];
  function askInit(id: string, options: { questions?: number; vertical?: boolean; selectOnly?: boolean; files?: FileAttachment[] } = {}): PopupInit {
    const base = init(id);
    const ask = base.interaction as Extract<PopupInit["interaction"], { type: "ask" }>;
    return { ...base, verticalQuestions: options.vertical ?? false, interaction: { type: "ask", request: { ...ask.request, selectOnly: options.selectOnly ?? false,
      message: { text: "", files: options.files ?? [] },
      questions: Array.from({ length: options.questions ?? 1 }, (_, i) => ({ message: `Question ${i + 1}`, predefinedOptions: [] })) } } };
  }
  function serve(options: Parameters<typeof askInit>[1] = {}) {
    mocks.invoke.mockImplementation(async (command: string, args: { requestId?: string }) => {
      if (command === "popup_init") return askInit(args.requestId!, options);
      if (command === "read_image_data_url") return "data:image/png;base64,AAAA";
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
  }
  beforeEach(() => { vi.clearAllMocks(); mocks.handlers.clear(); serve(); });
  afterEach(() => { wrappers.forEach(w => w.unmount()); wrappers.length = 0; });
  async function routed(options: Parameters<typeof askInit>[1] = {}) {
    serve(options);
    const lend = vi.fn<(receiver: DropReceiver | null) => void>();
    const aim = ref<DropAim | null>(null);
    let ctx!: ReturnType<typeof usePopupCore>;
    const scope: PopupScope = { requestId: "a", active: ref(true), blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: vi.fn(),
      preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }), dropReceiver: lend, dropAim: aim };
    const wrapper = mount(defineComponent({ setup() { ctx = usePopupCore(scope); return () => h("div"); } }), {
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper); await flushPromises();
    const receiver = lend.mock.calls.find(call => call[0])![0]!;
    return { ctx, aim, lend, receiver, wrapper };
  }

  it("leaves the window's drag events to the page, which owns the one listener of the window", async () => {
    await routed();
    expect(mocks.drag).not.toHaveBeenCalled();
  });

  it("still listens for the window's drag events itself when no page routes them", async () => {
    serve();
    const wrapper = mount(defineComponent({ setup() {
      usePopupCore({ requestId: "a", active: ref(true), blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: vi.fn(),
        preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }) });
      return () => h("div");
    } }), { global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } });
    wrappers.push(wrapper); await flushPromises();
    expect(mocks.drag).toHaveBeenCalledOnce();
  });

  it("lends its receiver while mounted and takes it back when unmounted", async () => {
    const { lend, wrapper } = await routed();
    expect(lend).toHaveBeenCalledTimes(1);
    wrapper.unmount(); wrappers.length = 0;
    expect(lend).toHaveBeenLastCalledWith(null);
  });

  it("attaches the dropped files to the question it is told, and reports how many were new", async () => {
    const { ctx, receiver } = await routed({ questions: 3 });
    expect(await receiver.acceptDrop(["/tmp/a.txt", "/tmp/b.txt"], 1)).toBe(2);
    expect(ctx.replyFilesByQ.value[1].map(f => f.name)).toEqual(["a.txt", "b.txt"]);
    expect(ctx.replyFilesByQ.value[0]).toEqual([]);
    expect(await receiver.acceptDrop(["/tmp/a.txt", "/tmp/c.txt"], 1)).toBe(1);
    expect(ctx.replyFilesByQ.value[1].map(f => f.name)).toEqual(["a.txt", "b.txt", "c.txt"]);
    expect(await receiver.acceptDrop(["/tmp/a.txt"], 1)).toBe(0);
  });

  it("reads dropped pictures into the question's pictures, and counts only those it could read", async () => {
    const { ctx, receiver } = await routed();
    expect(await receiver.acceptDrop(["/tmp/shot.png"], 0)).toBe(1);
    expect(ctx.imagesByQ.value[0]).toEqual([{ data: "data:image/png;base64,AAAA", mediaType: "image/png", filename: "shot.png" }]);
    mocks.invoke.mockImplementation(async (command: string) => { if (command === "read_image_data_url") throw new Error("unreadable"); return false; });
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(await receiver.acceptDrop(["/tmp/broken.png"], 0)).toBe(0);
    quiet.mockRestore();
    expect(ctx.imagesByQ.value[0]).toHaveLength(1);
  });

  it("does not add the request's own attachments back as reply files, on any question", async () => {
    const { ctx, receiver } = await routed({ questions: 2, files: [{ path: "/tmp/given.txt", name: "given.txt", size: 1, isImage: false }] });
    expect(await receiver.acceptDrop(["/tmp/given.txt"], 0)).toBe(0);
    expect(await receiver.acceptDrop(["/tmp/given.txt", "/tmp/mine.txt"], 1)).toBe(1);
    expect(ctx.replyFilesByQ.value.map(files => files.map(f => f.name))).toEqual([[], ["mine.txt"]]);
  });

  it("takes nothing for a select-only request", async () => {
    const { ctx, receiver } = await routed({ selectOnly: true });
    expect(await receiver.acceptDrop(["/tmp/a.txt"], 0)).toBe(0);
    expect(ctx.replyFilesByQ.value[0] ?? []).toEqual([]);
  });

  it("makes the card a file was dropped on the current question when the questions are stacked", async () => {
    const { ctx, receiver } = await routed({ questions: 3, vertical: true });
    expect(ctx.current.value).toBe(0);
    await receiver.acceptDrop(["/tmp/a.txt"], 2);
    expect(ctx.current.value).toBe(2);
    ctx.setActive(0, false);
    expect(await receiver.acceptDrop(["/tmp/a.txt"], 2)).toBe(0);
    expect(ctx.current.value).toBe(0);
  });

  it("leaves the current question alone in the sequential layout", async () => {
    const { ctx, receiver } = await routed({ questions: 3, vertical: false });
    expect(receiver.currentQuestion()).toBe(0);
    await receiver.acceptDrop(["/tmp/a.txt"], receiver.currentQuestion());
    expect(ctx.current.value).toBe(0);
    expect(ctx.replyFilesByQ.value[0].map(f => f.name)).toEqual(["a.txt"]);
  });

  it("tells the page about dragging one of its own attachments out, so that is not a drop", async () => {
    const { receiver } = await routed();
    expect(receiver.draggingOut()).toBe(false);
    receiver.endDragOut();
    expect(receiver.draggingOut()).toBe(false);
  });

  it("shows the question the page aims at, on its card", async () => {
    const { ctx, aim } = await routed({ questions: 3, vertical: true });
    expect(ctx.dropTargetQ.value).toBeNull();
    expect(ctx.dropCardLabel.value).toBeUndefined();
    aim.value = { question: 2 };
    expect(ctx.dropTargetQ.value).toBe(2);
    expect(ctx.dropCardLabel.value).toBe("Release to add to question 3");
    expect(ctx.dropHint.value).toBeNull();
    aim.value = null;
    expect(ctx.dropTargetQ.value).toBeNull();
  });

  it("frames the whole answer area when the page aims at the request and not at a card", async () => {
    const { ctx, aim } = await routed({ questions: 3, vertical: false });
    expect(ctx.dropHint.value).toBeNull();
    aim.value = { question: null };
    expect(ctx.dropTargetQ.value).toBeNull();
    expect(ctx.dropHint.value).toBe("Release to add to question 1");
    ctx.setActive(1, false);
    expect(ctx.dropHint.value).toBe("Release to add to question 2");
    aim.value = null;
    expect(ctx.dropHint.value).toBeNull();
  });

  it("names the request rather than a question when there is only one", async () => {
    const { ctx, aim } = await routed({ questions: 1 });
    aim.value = { question: null };
    expect(ctx.dropHint.value).toBe("Release to add to this request");
  });
});
