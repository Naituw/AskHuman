// The history window's list and search for the unsent content of cancelled asks (docs/specs/popup-draft-on-cancel.md D8):
// the "Draft" mark on the row, finding an entry by what was typed in it, and the entry that was never given one.
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryEntry } from "../lib/types";

const mocks = vi.hoisted(() => ({ entries: [] as unknown[] }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: () => ({ close: vi.fn(async () => {}) }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn(async () => {}) }));
vi.mock("../lib/ipc", () => ({
  clearAllHistory: vi.fn(async () => 0),
  deleteHistoryEntries: vi.fn(async () => 0),
  getHistory: vi.fn(async () => mocks.entries),
  getHistoryProjects: vi.fn(async () => []),
  historyInit: vi.fn(async () => ({ theme: "light", lang: "en", project: "", projectName: "" })),
  resolveHistorySessionTitles: vi.fn(async () => []),
  fileIconDataUrl: vi.fn(async () => ""),
  openPath: vi.fn(async () => {}),
  readImageDataUrl: vi.fn(async () => ""),
  showAttachmentMenu: vi.fn(async () => {}),
  localImageCreateScope: vi.fn(async () => "scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
}));
import HistoryView from "./HistoryView.vue";
import { historyInit } from "../lib/ipc";
import { i18n } from "../i18n";

function entry(id: string, over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id, timestampMs: 1_700_000_000_000, project: "", source: "", channel: "caller", action: "cancel", isMarkdown: false,
    message: { text: `message ${id}`, files: [] },
    questions: [{ message: `question ${id}`, predefinedOptions: [{ text: "Yes", recommended: false }, { text: "No", recommended: false }] }],
    answers: [], ...over,
  };
}

// Newest first, as the backend returns them.
const ENTRIES = (): HistoryEntry[] => [
  entry("waiting", { draft: { answers: [{ selectedOptions: ["No"], userInput: "needs the migration rollback plan", images: ["/tmp/askhuman/waiting/rollback.png"], files: ["/tmp/dir/notes.md"] }] } }),
  entry("taken", { draft: { answers: [{ selectedOptions: [], userInput: "already moved to a new question", images: [], files: [] }], claimedBy: "next-request" } }),
  entry("noted", { draft: { answers: [{ selectedOptions: [], userInput: "", images: [], files: [] }], annotations: [{ quote: "the plan", note: "ask about the quarterly budget" }] } }),
  entry("empty-handed"),
  entry("sent", { channel: "popup", action: "send", answers: [{ selectedOptions: ["Yes"], userInput: "go ahead", images: [], files: [] }] }),
];

let wrapper: VueWrapper | undefined;
async function mountHistory() {
  wrapper = mount(HistoryView, { attachTo: document.body, global: { plugins: [i18n] } });
  await flushPromises(); await flushPromises();
  return wrapper;
}
async function settle() { await flushPromises(); await nextTick(); }
const summaries = (view: VueWrapper) => view.findAll(".entry").map((item) => item.find(".entry-summary").text());
async function typeQuery(view: VueWrapper, text: string) {
  await view.get(".search-input").setValue(text);
  await settle();
}

describe("the history window and the unsent content of cancelled asks", () => {
  beforeEach(() => {
    vi.clearAllMocks(); document.body.replaceChildren();
    i18n.global.locale.value = "en";
    mocks.entries = ENTRIES();
  });
  afterEach(() => { wrapper?.unmount(); wrapper = undefined; });

  it("marks the rows that kept a draft, next to the cancellation, and no others", async () => {
    const view = await mountHistory();
    const rows = view.findAll(".entry");
    expect(rows.map((row) => row.find(".draft-badge").exists())).toEqual([true, true, true, false, false]);
    expect(rows[0].find(".draft-badge").text()).toBe("Draft");
    // The cancellation stays what it was: the red badge of whoever cancelled.
    expect(rows[0].find(".badge.cancel").text()).toBe("Caller");
    expect(rows[3].find(".badge.cancel").text()).toBe("Caller");
    expect(rows[4].find(".badge").classes()).not.toContain("cancel");
  });

  it("tells a draft that is waiting from one a new question has taken", async () => {
    const view = await mountHistory();
    const [waiting, taken] = view.findAll(".draft-badge");
    expect(waiting.classes()).not.toContain("loaded");
    expect(waiting.attributes("title")).toBe("Not submitted when the caller went away · Not loaded into a new question");
    expect(taken.classes()).toContain("loaded");
    expect(taken.attributes("title")).toBe("Not submitted when the caller went away · Loaded into a new question");
  });

  it("finds an entry by the words typed in its draft", async () => {
    const view = await mountHistory();
    await typeQuery(view, "rollback");
    expect(summaries(view)).toEqual(["message waiting"]);
    await typeQuery(view, "already moved");
    expect(summaries(view)).toEqual(["message taken"]);
    // The words of a note typed on the Agent's text are the user's own too.
    await typeQuery(view, "quarterly budget");
    expect(summaries(view)).toEqual(["message noted"]);
  });

  it("finds an entry by the options, pictures and files of its draft", async () => {
    const view = await mountHistory();
    await typeQuery(view, "rollback.png");
    expect(summaries(view)).toEqual(["message waiting"]);
    await typeQuery(view, "notes.md");
    expect(summaries(view)).toEqual(["message waiting"]);
  });

  it("lists the entries that kept a draft when searching for the word", async () => {
    const view = await mountHistory();
    await typeQuery(view, "draft");
    expect(summaries(view)).toEqual(["message waiting", "message taken", "message noted"]);
  });

  it("finds no draft text in an entry that has none, and none in a sent one", async () => {
    const view = await mountHistory();
    await typeQuery(view, "go ahead");
    expect(summaries(view)).toEqual(["message sent"]);
    await typeQuery(view, "rollback plan zzz");
    expect(summaries(view)).toEqual([]);
  });

  it("shows the draft in the detail of the entry that was chosen", async () => {
    const view = await mountHistory();
    // The newest entry is the one on screen when the window opens.
    expect(view.get(".draft-line").text()).toContain("Not submitted when the caller went away");
    expect(view.get(".draft-block .reply-text").text()).toBe("needs the migration rollback plan");
    await view.findAll(".entry")[3].trigger("click");
    await settle();
    expect(view.find(".draft-line").exists()).toBe(false);
    expect(view.find(".draft-block").exists()).toBe(false);
    await view.findAll(".entry")[1].trigger("click");
    await settle();
    expect(view.get(".draft-line-state").text()).toBe("Loaded into a new question");
  });

  it("speaks the language of the interface", async () => {
    vi.mocked(historyInit).mockResolvedValueOnce({ theme: "light", lang: "zh", project: "", projectName: "" } as Awaited<ReturnType<typeof historyInit>>);
    const view = await mountHistory();
    const badge = view.get(".draft-badge");
    expect(badge.text()).toBe("草稿");
    expect(badge.attributes("title")).toBe("调用方断开时未提交的内容 · 未载入新提问");
  });
});
