import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryAnswer, HistoryEntry } from "../lib/types";

const mocks = vi.hoisted(() => ({
  entries: [] as unknown[],
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, handler);
    return () => { mocks.listeners.delete(name); };
  }),
}));
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
import { deleteHistoryEntries, historyInit } from "../lib/ipc";
import { i18n } from "../i18n";

interface Spec {
  text?: string;
  /** Files sent with the question. */
  question?: string[];
  /** Images / files the user replied with. */
  images?: string[];
  files?: string[];
}

function entryOf(id: string, spec: Spec = {}): HistoryEntry {
  const answers: HistoryAnswer[] = spec.images || spec.files
    ? [{ selectedOptions: [], userInput: null, images: spec.images ?? [], files: spec.files ?? [] }]
    : [];
  return {
    id, timestampMs: 1_700_000_000_000, project: "", source: "", channel: "popup", action: "send", isMarkdown: false,
    message: {
      text: spec.text ?? `message ${id}`,
      files: (spec.question ?? []).map(path => ({ path, name: path.split("/").pop()!, size: 5, isImage: false })),
    },
    questions: [], answers,
  };
}

// Newest first, as the backend returns them.
const ENTRIES = () => [
  entryOf("both", { text: "alpha report", question: ["/q/spec.md", "/q/diagram.png"], images: ["/r/shot.PNG"] }),
  entryOf("plain", { text: "alpha chat" }),
  entryOf("docs", { text: "beta docs", question: ["/q/a.md", "/q/b.md", "/q/guide.pdf"] }),
  entryOf("reply-only", { text: "gamma", images: ["/r/one.png", "/r/two.png"], files: ["/r/notes"] }),
  entryOf("folder", { text: "delta", question: ["/q/project/"] }),
];

let wrapper: VueWrapper | undefined;
async function mountHistory() {
  wrapper = mount(HistoryView, { attachTo: document.body, global: { plugins: [i18n] } });
  await flushPromises(); await flushPromises();
  return wrapper;
}
async function settle() { await flushPromises(); await nextTick(); }

const ids = (view: VueWrapper) => view.findAll(".entry").map(item => item.find(".entry-summary").text());
async function openPanel(view: VueWrapper) {
  await view.get(".attach-btn").trigger("click");
  await settle();
  return view.get(".attach-panel");
}
const extRows = (view: VueWrapper) => view.findAll(".attach-ext");
const extNames = (view: VueWrapper) => extRows(view).map(row => row.find(".attach-ext-name").text());
const extCounts = (view: VueWrapper) => extRows(view).map(row => row.find(".attach-ext-count").text());
async function pickExt(view: VueWrapper, label: string) {
  const row = extRows(view).find(item => item.find(".attach-ext-name").text() === label);
  if (!row) throw new Error(`no suffix row ${label}; have ${extNames(view).join(", ")}`);
  await row.get("input").trigger("click");
  await settle();
}
async function pickSource(view: VueWrapper, index: 0 | 1 | 2) {
  await view.findAll(".attach-seg-item input")[index].trigger("click");
  await settle();
}
const chipTexts = (view: VueWrapper) => view.findAll(".attach-chip").map(chip => chip.find(".attach-chip-text").text());
async function typeQuery(view: VueWrapper, text: string) {
  await view.get(".search-input").setValue(text);
  await settle();
}

describe("history window attachment filter", () => {
  beforeEach(() => {
    vi.clearAllMocks(); mocks.listeners.clear(); document.body.replaceChildren();
    i18n.global.locale.value = "en";
    mocks.entries = ENTRIES();
  });
  afterEach(() => { wrapper?.unmount(); wrapper = undefined; });

  it("marks entries that carry attachments with a paperclip and their count, in every state", async () => {
    const view = await mountHistory();
    const badges = view.findAll(".entry").map(item => item.find(".entry-attach"));
    expect(badges.map(badge => (badge.exists() ? badge.text() : ""))).toEqual(["3", "", "3", "3", "1"]);
    expect(badges[0].attributes("title")).toBe("Question attachments: 2 · Reply attachments: 1");
    expect(badges[2].attributes("title")).toBe("Question attachments: 3");
    expect(badges[3].attributes("title")).toBe("Reply attachments: 3");
    expect(badges[0].attributes("aria-label")).toBe("Attachments: 3");
    // The time stays on the right edge, after the badge.
    expect(view.findAll(".entry-top")[0].element.lastElementChild?.classList.contains("entry-time")).toBe(true);
  });

  it("starts with the button idle, no tags and no panel", async () => {
    const view = await mountHistory();
    expect(view.get(".attach-btn").classes()).not.toContain("active");
    expect(view.get(".attach-btn").attributes("aria-expanded")).toBe("false");
    expect(view.find(".attach-count").exists()).toBe(false);
    expect(view.find(".attach-panel").exists()).toBe(false);
    expect(view.find(".attach-chips").exists()).toBe(false);
    expect(view.findAll(".entry")).toHaveLength(5);
  });

  it("lists the suffixes in scope by entry count, with no-suffix last, and the matching total", async () => {
    const view = await mountHistory();
    await openPanel(view);
    expect(view.get(".attach-btn").attributes("aria-expanded")).toBe("true");
    // md: both, docs · png: both, reply-only · pdf: docs · no suffix: reply-only (notes), folder
    expect(extNames(view)).toEqual([".md", ".png", ".pdf", "No extension"]);
    expect(extCounts(view)).toEqual(["2", "2", "1", "2"]);
    expect(view.get(".attach-switch-count").text()).toBe("4");
    expect(view.get(".attach-foot").text()).toBe("Matching entries: 5");
  });

  it("filters by a suffix, shows it as a tag and a count on the button, and removes it from the tag", async () => {
    const view = await mountHistory();
    await openPanel(view);
    await pickExt(view, ".pdf");
    expect(ids(view)).toEqual(["beta docs"]);
    expect(chipTexts(view)).toEqual([".pdf"]);
    expect(view.get(".attach-btn").classes()).toContain("active");
    expect(view.get(".attach-count").text()).toBe("1");
    expect(view.get(".attach-foot").text()).toBe("Matching entries: 1");

    await view.get(".attach-chip-x").trigger("click");
    await settle();
    expect(view.findAll(".entry")).toHaveLength(5);
    expect(view.find(".attach-chips").exists()).toBe(false);
    expect(view.get(".attach-btn").classes()).not.toContain("active");
  });

  it("matches any of several suffixes and keeps the other counts and order while selecting", async () => {
    const view = await mountHistory();
    await openPanel(view);
    const before = extNames(view);
    await pickExt(view, ".pdf");
    await pickExt(view, ".png");
    expect(ids(view)).toEqual(["alpha report", "beta docs", "gamma"]);
    expect(chipTexts(view)).toEqual([".pdf", ".png"]);
    expect(extNames(view)).toEqual(before);
    expect(extCounts(view)).toEqual(["2", "2", "1", "2"]);
    expect(extRows(view).map(row => row.classes().includes("on"))).toEqual([false, true, true, false]);
  });

  it("treats a source as 'has attachments' of that kind and recounts the suffixes for it", async () => {
    const view = await mountHistory();
    await openPanel(view);
    await pickSource(view, 2); // Reply
    expect(ids(view)).toEqual(["alpha report", "gamma"]);
    expect(chipTexts(view)).toEqual(["Reply attachments"]);
    expect(extNames(view)).toEqual([".png", "No extension"]);
    expect(extCounts(view)).toEqual(["2", "1"]);
    // The explicit switch is implied by the source: shown on and locked.
    const required = view.get(".attach-switch-row input").element as HTMLInputElement;
    expect(required.checked).toBe(true);
    expect(required.disabled).toBe(true);

    await pickSource(view, 1); // Question
    expect(ids(view)).toEqual(["alpha report", "beta docs", "delta"]);
    expect(extNames(view)).toEqual([".md", ".pdf", ".png", "No extension"]);

    await pickSource(view, 0); // All
    expect(view.findAll(".entry")).toHaveLength(5);
    expect(view.find(".attach-chips").exists()).toBe(false);
  });

  it("restricts a suffix to the chosen source", async () => {
    const view = await mountHistory();
    await openPanel(view);
    await pickSource(view, 1); // Question
    await pickExt(view, ".png");
    // reply-only has .png too, but only as a reply image.
    expect(ids(view)).toEqual(["alpha report"]);
    expect(chipTexts(view)).toEqual(["Question attachments", ".png"]);
  });

  it("offers 'only entries with attachments' on its own and locks it once a suffix is chosen", async () => {
    const view = await mountHistory();
    await openPanel(view);
    const required = () => view.get(".attach-switch-row input").element as HTMLInputElement;
    expect(required().checked).toBe(false);
    expect(required().disabled).toBe(false);

    await view.get(".attach-switch-row input").trigger("click");
    await settle();
    expect(ids(view)).toEqual(["alpha report", "beta docs", "gamma", "delta"]);
    expect(chipTexts(view)).toEqual(["Has attachments"]);

    await pickExt(view, ".md");
    expect(required().checked).toBe(true);
    expect(required().disabled).toBe(true);
    // The explicit switch is implied by the suffix, so it gives up its tag.
    expect(chipTexts(view)).toEqual([".md"]);
    // Dropping the suffix returns to what the user chose explicitly.
    await pickExt(view, ".md");
    expect(chipTexts(view)).toEqual(["Has attachments"]);
    expect(required().disabled).toBe(false);
  });

  it("combines with keywords: the suffixes on offer follow the search", async () => {
    const view = await mountHistory();
    await typeQuery(view, "alpha");
    await openPanel(view);
    // Only "alpha report" and "alpha chat" are in scope.
    expect(extNames(view)).toEqual([".md", ".png"]);
    expect(extCounts(view)).toEqual(["1", "1"]);
    await pickExt(view, ".md");
    expect(ids(view)).toEqual(["alpha report"]);

    // The keyword stays in force when the suffix is dropped again.
    await pickExt(view, ".md");
    expect(ids(view)).toEqual(["alpha report", "alpha chat"]);
  });

  it("keeps a selected suffix listed at zero when the search leaves nothing for it", async () => {
    const view = await mountHistory();
    await openPanel(view);
    await pickExt(view, ".pdf");
    await typeQuery(view, "gamma");
    expect(view.findAll(".entry")).toHaveLength(0);
    expect(extNames(view)).toContain(".pdf");
    const pdf = extRows(view).find(row => row.find(".attach-ext-name").text() === ".pdf")!;
    expect(pdf.find(".attach-ext-count").text()).toBe("0");
    expect(pdf.classes()).toContain("on");
    await pickExt(view, ".pdf");
    expect(ids(view)).toEqual(["gamma"]);
  });

  it("clears only the attachment conditions from the tags row", async () => {
    const view = await mountHistory();
    await typeQuery(view, "alpha");
    await openPanel(view);
    await pickSource(view, 2);
    await pickExt(view, ".png");
    expect(chipTexts(view)).toEqual(["Reply attachments", ".png"]);

    await view.get(".attach-chip-clear").trigger("click");
    await settle();
    expect(view.find(".attach-chips").exists()).toBe(false);
    expect((view.get(".search-input").element as HTMLInputElement).value).toBe("alpha");
    expect(ids(view)).toEqual(["alpha report", "alpha chat"]);
  });

  it("resets the conditions from the panel", async () => {
    const view = await mountHistory();
    await openPanel(view);
    expect((view.get(".attach-reset").element as HTMLButtonElement).disabled).toBe(true);
    await pickExt(view, ".md");
    expect((view.get(".attach-reset").element as HTMLButtonElement).disabled).toBe(false);
    await view.get(".attach-reset").trigger("click");
    await settle();
    expect(view.findAll(".entry")).toHaveLength(5);
    expect(view.find(".attach-chip").exists()).toBe(false);
  });

  it("says so when no entry in scope has attachments", async () => {
    mocks.entries = [entryOf("a"), entryOf("b")];
    const view = await mountHistory();
    await openPanel(view);
    expect(view.find(".attach-ext").exists()).toBe(false);
    expect(view.get(".attach-none").text()).toBe("No entries with attachments in this scope");
    expect(view.get(".attach-switch-count").text()).toBe("0");
  });

  describe("empty results", () => {
    it("explains the empty list and offers to clear the attachment conditions", async () => {
      const view = await mountHistory();
      await openPanel(view);
      await pickExt(view, ".pdf");
      await typeQuery(view, "gamma");
      expect(view.find(".entry").exists()).toBe(false);
      expect(view.get(".empty-title").text()).toBe("No matching entries");
      expect(view.get(".empty-hint").text()).toBe("Loosen the keywords or attachment filters and try again.");

      await view.get(".empty-action").trigger("click");
      await settle();
      expect(ids(view)).toEqual(["gamma"]);
      expect(view.find(".attach-chips").exists()).toBe(false);
    });

    it("keeps the plain keyword hint and no button when only keywords are in play", async () => {
      const view = await mountHistory();
      await typeQuery(view, "no such text");
      expect(view.get(".empty-hint").text()).toBe("Try different keywords.");
      expect(view.find(".empty-action").exists()).toBe(false);
    });
  });

  describe("cleanup", () => {
    it("treats the attachment conditions as a search: only the visible entries, with the conditions named", async () => {
      const view = await mountHistory();
      await openPanel(view);
      await pickExt(view, ".md");
      await view.get(".clear-btn").trigger("click");
      await settle();
      const scopeAction = view.get(".clear-menu button");
      expect(scopeAction.text()).toBe("Delete current search results (2)");

      await scopeAction.trigger("click");
      await settle();
      const context = view.findAll(".delete-context div").map(row => [row.get("dt").text(), row.get("dd").text()]);
      expect(context).toEqual([["Project", expect.any(String)], ["Session", expect.any(String)], ["Keywords", "None"], ["Attachments", ".md"]]);

      await view.get(".btn-danger").trigger("click");
      await settle();
      expect(deleteHistoryEntries).toHaveBeenCalledWith(["both", "docs"]);
    });

    it("names every condition in the confirmation", async () => {
      const view = await mountHistory();
      await openPanel(view);
      await pickSource(view, 2);
      await pickExt(view, ".png");
      await pickExt(view, "No extension");
      await view.get(".clear-btn").trigger("click");
      await settle();
      await view.get(".clear-menu button").trigger("click");
      await settle();
      const attachments = view.findAll(".delete-context div").map(row => row.get("dd").text()).pop();
      expect(attachments).toBe("Reply attachments · .png · No extension");
    });

    it("leaves the confirmation without an attachments row when none is active", async () => {
      const view = await mountHistory();
      await typeQuery(view, "alpha");
      await view.get(".clear-btn").trigger("click");
      await settle();
      await view.get(".clear-menu button").trigger("click");
      await settle();
      expect(view.findAll(".delete-context div")).toHaveLength(3);
    });
  });

  describe("lifecycle", () => {
    it("resets with a new open request, like the keywords do", async () => {
      const view = await mountHistory();
      await typeQuery(view, "alpha");
      await openPanel(view);
      await pickExt(view, ".md");
      expect(view.find(".attach-panel").exists()).toBe(true);

      mocks.listeners.get("history-open-target")!({ payload: { all: true, project: "", target: null } });
      await settle();
      expect(view.find(".attach-panel").exists()).toBe(false);
      expect(view.find(".attach-chips").exists()).toBe(false);
      expect(view.get(".attach-btn").classes()).not.toContain("active");
      expect((view.get(".search-input").element as HTMLInputElement).value).toBe("");
      expect(view.findAll(".entry")).toHaveLength(5);
    });

    it("keeps the conditions when the entries are reloaded behind them", async () => {
      const view = await mountHistory();
      await openPanel(view);
      await pickExt(view, ".pdf");
      mocks.entries = [entryOf("fresh", { text: "new pdf", question: ["/q/new.pdf"] }), ...ENTRIES()];
      mocks.listeners.get("history-updated")!({ payload: null });
      await settle();
      expect(ids(view)).toEqual(["new pdf", "beta docs"]);
    });
  });

  describe("panel behavior", () => {
    it("closes on Escape without letting the window see the key, and returns focus to the button", async () => {
      const view = await mountHistory();
      const seen = vi.fn();
      window.addEventListener("keydown", seen);
      try {
        const panel = await openPanel(view);
        expect(document.activeElement).toBe(panel.element);
        const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
        panel.element.dispatchEvent(event);
        await settle();
        expect(view.find(".attach-panel").exists()).toBe(false);
        expect(seen).not.toHaveBeenCalled();
        expect(document.activeElement).toBe(view.get(".attach-btn").element);
      } finally {
        window.removeEventListener("keydown", seen);
      }
    });

    it("lets Escape through when the panel is closed", async () => {
      const view = await mountHistory();
      const seen = vi.fn();
      window.addEventListener("keydown", seen);
      try {
        view.get(".attach-btn").element.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
        expect(seen).toHaveBeenCalledTimes(1);
      } finally {
        window.removeEventListener("keydown", seen);
      }
    });

    it("closes when the pointer goes down outside, but not inside", async () => {
      const view = await mountHistory();
      const panel = await openPanel(view);
      panel.element.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      await settle();
      expect(view.find(".attach-panel").exists()).toBe(true);

      view.get(".entry").element.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      await settle();
      expect(view.find(".attach-panel").exists()).toBe(false);
    });

    it("closes when the window loses focus and toggles from the button", async () => {
      const view = await mountHistory();
      await openPanel(view);
      window.dispatchEvent(new Event("blur"));
      await settle();
      expect(view.find(".attach-panel").exists()).toBe(false);

      await openPanel(view);
      await view.get(".attach-btn").trigger("click");
      await settle();
      expect(view.find(".attach-panel").exists()).toBe(false);
    });

    it("moves between the suffix rows with the arrow keys and keeps a single tab stop", async () => {
      const view = await mountHistory();
      await openPanel(view);
      const inputs = () => view.findAll(".attach-ext input").map(input => input.element as HTMLInputElement);
      expect(inputs().map(input => input.tabIndex)).toEqual([0, -1, -1, -1]);

      const list = view.get(".attach-ext-list");
      inputs()[0].focus();
      list.element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
      expect(document.activeElement).toBe(inputs()[1]);
      await settle();
      expect(inputs().map(input => input.tabIndex)).toEqual([-1, 0, -1, -1]);

      list.element.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true, cancelable: true }));
      expect(document.activeElement).toBe(inputs()[3]);
      list.element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
      expect(document.activeElement).toBe(inputs()[3]);
      list.element.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true }));
      expect(document.activeElement).toBe(inputs()[0]);
      list.element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));
      expect(document.activeElement).toBe(inputs()[0]);
    });

    it("only renders the suffix rows while the panel is open", async () => {
      const view = await mountHistory();
      expect(view.find(".attach-ext").exists()).toBe(false);
      await openPanel(view);
      expect(extRows(view).length).toBeGreaterThan(0);
      await view.get(".attach-btn").trigger("click");
      await settle();
      expect(view.find(".attach-ext").exists()).toBe(false);
    });
  });

  it("speaks Chinese when the language is Chinese", async () => {
    vi.mocked(historyInit).mockResolvedValueOnce({ theme: "light", lang: "zh", project: "", projectName: "" });
    const view = await mountHistory();
    expect(view.get(".attach-btn-label").text()).toBe("附件");
    await openPanel(view);
    expect(view.get(".attach-title").text()).toBe("附件筛选");
    expect(view.findAll(".attach-seg-text").map(item => item.text())).toEqual(["全部", "提问附件", "回复附件"]);
    expect(extNames(view)).toContain("无后缀");
    expect(view.get(".attach-foot").text()).toBe("命中记录：5 条");
    i18n.global.locale.value = "en";
  });
});
